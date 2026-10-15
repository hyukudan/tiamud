use argon2::{
    Algorithm, Argon2, Params, Version,
    password_hash::{PasswordHash, PasswordHasher, PasswordVerifier, SaltString},
};
use rand_core::{OsRng, RngCore};
use serde::Serialize;
use sha2::{Digest, Sha256};
use std::{fmt, sync::Arc, time::Duration};
use tiamud_core::AccountSnapshot;
use tokio::{sync::Semaphore, task::spawn_blocking, time::timeout};

pub const MIN_PASSWORD_BYTES: usize = 8;
pub const MAX_PASSWORD_BYTES: usize = 1_024;
pub const SESSION_TOKEN_BYTES: usize = 32;
pub const DEFAULT_SESSION_TTL: Duration = Duration::from_secs(30 * 24 * 60 * 60);
pub(crate) const DEFAULT_PASSWORD_CONCURRENCY: usize = 2;
const MAX_PENDING_PASSWORD_WORK: usize = 256;
const PASSWORD_WORK_TIMEOUT: Duration = Duration::from_secs(5);

#[derive(Clone)]
pub(crate) struct PasswordService {
    concurrency: Arc<Semaphore>,
    pending: Arc<Semaphore>,
    backend: Arc<dyn PasswordBackend>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum PasswordWorkError {
    Overloaded,
    Timeout,
    Execution,
}

pub(crate) trait PasswordBackend: Send + Sync {
    fn hash(&self, password: &str) -> Result<String, ()>;
    fn verify(&self, password: &str, encoded_hash: &str) -> Result<bool, ()>;
}

#[derive(Debug)]
struct Argon2Backend;

impl PasswordBackend for Argon2Backend {
    fn hash(&self, password: &str) -> Result<String, ()> {
        hash_password(password).map_err(|_| ())
    }

    fn verify(&self, password: &str, encoded_hash: &str) -> Result<bool, ()> {
        verify_password(password, encoded_hash).map_err(|_| ())
    }
}

enum PasswordJob {
    Hash(String),
    Verify {
        password: String,
        encoded_hash: String,
    },
    Dummy(String),
}

enum PasswordJobResult {
    Hash(String),
    Verify(bool),
    Dummy,
}

impl PasswordService {
    pub(crate) fn new(max_concurrency: usize) -> Self {
        Self::with_backend(max_concurrency, Arc::new(Argon2Backend))
    }

    pub(crate) fn with_backend(max_concurrency: usize, backend: Arc<dyn PasswordBackend>) -> Self {
        Self {
            concurrency: Arc::new(Semaphore::new(max_concurrency.max(1))),
            pending: Arc::new(Semaphore::new(MAX_PENDING_PASSWORD_WORK)),
            backend,
        }
    }

    pub(crate) async fn hash(&self, password: String) -> Result<String, PasswordWorkError> {
        match self.run(PasswordJob::Hash(password)).await? {
            PasswordJobResult::Hash(encoded) => Ok(encoded),
            PasswordJobResult::Verify(_) | PasswordJobResult::Dummy => {
                Err(PasswordWorkError::Execution)
            }
        }
    }

    pub(crate) async fn verify(
        &self,
        password: String,
        encoded_hash: String,
    ) -> Result<bool, PasswordWorkError> {
        match self
            .run(PasswordJob::Verify {
                password,
                encoded_hash,
            })
            .await?
        {
            PasswordJobResult::Verify(valid) => Ok(valid),
            PasswordJobResult::Hash(_) | PasswordJobResult::Dummy => {
                Err(PasswordWorkError::Execution)
            }
        }
    }

    /// Spend the same dominant Argon2 work for an identity with no usable hash.
    pub(crate) async fn dummy(&self, password: String) -> Result<(), PasswordWorkError> {
        match self.run(PasswordJob::Dummy(password)).await? {
            PasswordJobResult::Dummy => Ok(()),
            PasswordJobResult::Hash(_) | PasswordJobResult::Verify(_) => {
                Err(PasswordWorkError::Execution)
            }
        }
    }

    async fn run(&self, job: PasswordJob) -> Result<PasswordJobResult, PasswordWorkError> {
        let pending = Arc::clone(&self.pending);
        let concurrency = Arc::clone(&self.concurrency);
        let backend = Arc::clone(&self.backend);
        // Admission is deliberately non-waiting: a semaphore with
        // `acquire().await` bounds permits but not the number of suspended
        // callers queued behind those permits.
        let pending_permit = pending
            .try_acquire_owned()
            .map_err(|_| PasswordWorkError::Overloaded)?;
        timeout(PASSWORD_WORK_TIMEOUT, async move {
            let concurrency_permit = concurrency
                .acquire_owned()
                .await
                .map_err(|_| PasswordWorkError::Execution)?;
            spawn_blocking(move || {
                // Keep both permits inside the blocking closure. Cancellation of
                // the async caller cannot admit replacement work while Argon2 is
                // still consuming memory and CPU.
                let _pending_permit = pending_permit;
                let _concurrency_permit = concurrency_permit;
                match job {
                    PasswordJob::Hash(password) => backend
                        .hash(&password)
                        .map(PasswordJobResult::Hash)
                        .map_err(|_| PasswordWorkError::Execution),
                    PasswordJob::Verify {
                        password,
                        encoded_hash,
                    } => backend
                        .verify(&password, &encoded_hash)
                        .map(PasswordJobResult::Verify)
                        .map_err(|_| PasswordWorkError::Execution),
                    PasswordJob::Dummy(password) => backend
                        .hash(&password)
                        .map(|_| PasswordJobResult::Dummy)
                        .map_err(|_| PasswordWorkError::Execution),
                }
            })
            .await
            .map_err(|_| PasswordWorkError::Execution)?
        })
        .await
        .map_err(|_| PasswordWorkError::Timeout)?
    }
}

impl Default for PasswordService {
    fn default() -> Self {
        Self::new(DEFAULT_PASSWORD_CONCURRENCY)
    }
}

/// An opaque bearer credential. Its custom `Debug` implementation prevents
/// accidental disclosure in logs and test failure output.
#[derive(Clone, Eq, PartialEq, Serialize)]
#[serde(transparent)]
pub struct SessionToken(String);

impl SessionToken {
    pub fn as_str(&self) -> &str {
        &self.0
    }

    pub(crate) fn generate() -> Self {
        let mut bytes = [0_u8; SESSION_TOKEN_BYTES];
        OsRng.fill_bytes(&mut bytes);
        Self(hex::encode(bytes))
    }

    pub(crate) fn digest(&self) -> [u8; 32] {
        digest_token_bytes(self.0.as_bytes())
    }

    pub(crate) fn parse(value: &str) -> Option<Self> {
        if value.len() != SESSION_TOKEN_BYTES * 2 {
            return None;
        }
        let decoded = hex::decode(value).ok()?;
        if decoded.len() != SESSION_TOKEN_BYTES {
            return None;
        }
        Some(Self(value.to_owned()))
    }
}

impl fmt::Debug for SessionToken {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("SessionToken([REDACTED])")
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AuthenticatedSession {
    pub account: AccountSnapshot,
    pub token: SessionToken,
    pub expires_at: i64,
}

pub(crate) fn validate_password(password: &str) -> bool {
    (MIN_PASSWORD_BYTES..=MAX_PASSWORD_BYTES).contains(&password.len())
}

pub(crate) fn hash_password(password: &str) -> Result<String, argon2::password_hash::Error> {
    let salt = SaltString::generate(&mut OsRng);
    argon2()
        .hash_password(password.as_bytes(), &salt)
        .map(|hash| hash.to_string())
}

pub(crate) fn verify_password(
    password: &str,
    encoded_hash: &str,
) -> Result<bool, argon2::password_hash::Error> {
    let parsed = PasswordHash::new(encoded_hash)?;
    Ok(argon2()
        .verify_password(password.as_bytes(), &parsed)
        .is_ok())
}

pub(crate) fn digest_token(value: &str) -> Option<[u8; 32]> {
    SessionToken::parse(value).map(|token| token.digest())
}

fn digest_token_bytes(value: &[u8]) -> [u8; 32] {
    Sha256::digest(value).into()
}

fn argon2() -> Argon2<'static> {
    Argon2::new(Algorithm::Argon2id, Version::V0x13, Params::default())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{
        Condvar, Mutex,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    };

    /// Liveness bound for spin-waits on a state the test itself drives. It
    /// only turns a genuine hang into a failure; a loaded host may need more
    /// than a second to schedule the blocking password workers.
    const TEST_HANG_GUARD: Duration = Duration::from_secs(30);

    struct BlockingBackend {
        active: AtomicUsize,
        maximum: AtomicUsize,
        started: AtomicUsize,
        released: AtomicBool,
        lock: Mutex<()>,
        wake: Condvar,
    }

    impl BlockingBackend {
        fn new() -> Self {
            Self {
                active: AtomicUsize::new(0),
                maximum: AtomicUsize::new(0),
                started: AtomicUsize::new(0),
                released: AtomicBool::new(false),
                lock: Mutex::new(()),
                wake: Condvar::new(),
            }
        }

        fn block(&self) {
            let active = self.active.fetch_add(1, Ordering::SeqCst) + 1;
            self.maximum.fetch_max(active, Ordering::SeqCst);
            self.started.fetch_add(1, Ordering::SeqCst);
            let mut guard = self.lock.lock().unwrap();
            while !self.released.load(Ordering::SeqCst) {
                guard = self.wake.wait(guard).unwrap();
            }
            self.active.fetch_sub(1, Ordering::SeqCst);
        }

        fn release(&self) {
            self.released.store(true, Ordering::SeqCst);
            self.wake.notify_all();
        }
    }

    impl PasswordBackend for BlockingBackend {
        fn hash(&self, password: &str) -> Result<String, ()> {
            self.block();
            Ok(format!("test:{password}"))
        }

        fn verify(&self, password: &str, encoded_hash: &str) -> Result<bool, ()> {
            self.block();
            Ok(encoded_hash == format!("test:{password}"))
        }
    }

    #[test]
    fn argon2id_hash_does_not_contain_the_password() {
        let password = "correct horse battery staple";
        let encoded = hash_password(password).unwrap();

        assert!(encoded.starts_with("$argon2id$v=19$"));
        assert!(!encoded.contains(password));
        assert!(verify_password(password, &encoded).unwrap());
        assert!(!verify_password("incorrect password", &encoded).unwrap());
    }

    #[test]
    fn bearer_tokens_are_random_parseable_and_redacted() {
        let first = SessionToken::generate();
        let second = SessionToken::generate();

        assert_ne!(first, second);
        assert_eq!(SessionToken::parse(first.as_str()), Some(first.clone()));
        assert_eq!(format!("{first:?}"), "SessionToken([REDACTED])");
        assert!(SessionToken::parse("too-short").is_none());
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn password_service_never_exceeds_its_explicit_concurrency_limit() {
        let backend = Arc::new(BlockingBackend::new());
        let service = PasswordService::with_backend(2, backend.clone());
        let jobs = (0..8)
            .map(|index| {
                let service = service.clone();
                tokio::spawn(async move { service.hash(format!("password-{index}")).await })
            })
            .collect::<Vec<_>>();

        timeout(TEST_HANG_GUARD, async {
            while backend.started.load(Ordering::SeqCst) < 2 {
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap();
        tokio::time::sleep(Duration::from_millis(20)).await;
        assert_eq!(backend.maximum.load(Ordering::SeqCst), 2);
        assert_eq!(backend.started.load(Ordering::SeqCst), 2);

        backend.release();
        for job in jobs {
            job.await.unwrap().unwrap();
        }
        assert_eq!(backend.maximum.load(Ordering::SeqCst), 2);
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn password_service_rejects_work_beyond_its_true_pending_bound() {
        let backend = Arc::new(BlockingBackend::new());
        let service = PasswordService::with_backend(1, backend.clone());
        let jobs = (0..MAX_PENDING_PASSWORD_WORK)
            .map(|index| {
                let service = service.clone();
                tokio::spawn(async move { service.hash(format!("password-{index}")).await })
            })
            .collect::<Vec<_>>();

        timeout(TEST_HANG_GUARD, async {
            while service.pending.available_permits() != 0 {
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap();
        assert_eq!(
            service.hash("one-request-too-many".into()).await,
            Err(PasswordWorkError::Overloaded)
        );

        backend.release();
        for job in jobs {
            job.await.unwrap().unwrap();
        }
    }
}
