//! PostgreSQL implementation of the transport-independent character store.
//!
//! `postgres::Client` is synchronous, so one dedicated owner thread performs
//! database work behind the same bounded Tokio channel used by SQLite. Argon2
//! runs in a separately bounded blocking service, and no PostgreSQL call runs
//! on an async executor thread.

use std::{
    thread,
    time::{SystemTime, UNIX_EPOCH},
};

use async_trait::async_trait;
use postgres::{Client, GenericClient};
use tiamud_core::{
    AccountId, AccountRole, AccountSnapshot, CharacterGameplaySnapshot, CharacterId,
    CharacterSnapshot, DomainEvent, DurableCharacterSnapshot, WorldRuntimeSnapshot,
};
use tokio::{
    sync::{mpsc, oneshot},
    time::{Duration, timeout},
};

use crate::{
    auth::{
        AuthenticatedSession, DEFAULT_SESSION_TTL, PasswordService, PasswordWorkError,
        SessionToken, digest_token, validate_password,
    },
    persistence::{
        CharacterStore, DOMAIN_EVENT_RETENTION, ModerationAction, PreparedLogin, Request,
        SocialProfile, SocialRelationKind, StoreError,
    },
};

#[cfg(test)]
use crate::persistence::AuditEntry;

const MAX_IDENTITY_NAME_BYTES: usize = 64;
const MAX_ACTIVE_SESSIONS: i64 = 16;
const MAX_CHARACTER_JSON_BYTES: usize = 65_536;
const MAX_EVENT_JSON_BYTES: usize = 65_536;
const MAX_RUNTIME_JSON_BYTES: usize = 1_048_576;
const MAX_SOCIAL_REASON_BYTES: usize = 1_024;
const MAX_REPORT_REASON_BYTES: usize = 2_048;
const MAX_AUDIT_DETAILS_BYTES: usize = 4_096;
const MAX_MODERATION_DURATION_SECONDS: i64 = 365 * 24 * 60 * 60;
const PERSISTENCE_REQUEST_QUEUE_CAPACITY: usize = 256;
const PERSISTENCE_REQUEST_TIMEOUT: Duration = Duration::from_secs(5);

#[derive(Clone)]
pub struct PostgresCharacterStore {
    tx: mpsc::Sender<Request>,
    passwords: PasswordService,
}

impl PostgresCharacterStore {
    pub async fn open(database_url: &str) -> Result<Self, StoreError> {
        Self::open_with_password_service(database_url, PasswordService::default()).await
    }

    async fn open_with_password_service(
        database_url: &str,
        passwords: PasswordService,
    ) -> Result<Self, StoreError> {
        let database_url = database_url.to_owned();
        let (tx, mut rx) = mpsc::channel(PERSISTENCE_REQUEST_QUEUE_CAPACITY);
        let (ready_tx, ready_rx) = oneshot::channel();

        thread::Builder::new()
            .name("postgres-writer".into())
            .spawn(move || {
                let mut connection = match (|| -> Result<Client, StoreError> {
                    let mut connection = tiamud_persistence_postgres::connect(&database_url)?;
                    tiamud_persistence_postgres::migrate(&mut connection)?;
                    Ok(connection)
                })() {
                    Ok(connection) => {
                        let _ = ready_tx.send(Ok(()));
                        connection
                    }
                    Err(error) => {
                        let _ = ready_tx.send(Err(error));
                        return;
                    }
                };

                while let Some(request) = rx.blocking_recv() {
                    match request {
                        Request::Shutdown { reply } => {
                            drop(connection);
                            let _ = reply.send(Ok(()));
                            return;
                        }
                        Request::Health { reply } => {
                            let result = connection
                                .query_one("SELECT 1::BIGINT", &[])
                                .map(|_| ())
                                .map_err(StoreError::from);
                            let _ = reply.send(result);
                        }
                        Request::FinishRegisterAccount {
                            name,
                            password_hash,
                            reply,
                        } => {
                            let _ = reply.send(finish_register_account(
                                &mut connection,
                                &name,
                                &password_hash,
                                unix_timestamp(),
                            ));
                        }
                        Request::PrepareLoginAccount { name, reply } => {
                            let _ = reply.send(prepare_login_account(&mut connection, &name));
                        }
                        Request::FinishLoginAccount { prepared, reply } => {
                            let _ = reply.send(finish_login_account(
                                &mut connection,
                                &prepared,
                                unix_timestamp(),
                            ));
                        }
                        Request::ResolveSession { token, reply } => {
                            let _ = reply.send(resolve_session(
                                &mut connection,
                                &token,
                                unix_timestamp(),
                            ));
                        }
                        Request::RevokeSession { token, reply } => {
                            let _ = reply.send(revoke_session(
                                &mut connection,
                                &token,
                                unix_timestamp(),
                            ));
                        }
                        #[cfg(test)]
                        Request::LoadOrCreateAccount { name, reply } => {
                            let _ = reply.send(load_or_create_account(&mut connection, &name));
                        }
                        Request::ListCharacters { account_id, reply } => {
                            let _ = reply.send(list_characters(&mut connection, account_id));
                        }
                        Request::LoadOrCreateCharacter {
                            account_id,
                            name,
                            start_room,
                            initial_gameplay,
                            reply,
                        } => {
                            let _ = reply.send(load_or_create_character_with_gameplay(
                                &mut connection,
                                account_id,
                                &name,
                                &start_room,
                                &initial_gameplay,
                            ));
                        }
                        Request::LoadCharacter {
                            account_id,
                            character_id,
                            reply,
                        } => {
                            let _ = reply.send(load_character(
                                &mut connection,
                                account_id,
                                character_id,
                            ));
                        }
                        Request::Save { snapshot, reply } => {
                            let _ = reply.send(save(&mut connection, &snapshot));
                        }
                        Request::LoadCharacterGameplay {
                            character_id,
                            reply,
                        } => {
                            let _ =
                                reply.send(load_character_gameplay(&mut connection, character_id));
                        }
                        Request::LoadRuntimeSnapshot { reply } => {
                            let _ = reply.send(load_runtime_snapshot(&mut connection));
                        }
                        Request::LoadDomainEvents { reply } => {
                            let _ = reply.send(load_domain_events(&mut connection));
                        }
                        Request::CommitAction {
                            characters,
                            runtime,
                            events,
                            reply,
                        } => {
                            let _ = reply.send(commit_action(
                                &mut connection,
                                &characters,
                                &runtime,
                                &events,
                            ));
                        }
                        Request::SocialProfile {
                            character_id,
                            reply,
                        } => {
                            let _ = reply.send(social_profile(
                                &mut connection,
                                character_id,
                                unix_timestamp(),
                            ));
                        }
                        Request::SocialTarget { name, reply } => {
                            let _ =
                                reply.send(social_target(&mut connection, &name, unix_timestamp()));
                        }
                        Request::SocialDeliveryAllowed {
                            sender,
                            recipient,
                            reply,
                        } => {
                            let _ = reply.send(social_delivery_allowed(
                                &mut connection,
                                sender,
                                recipient,
                            ));
                        }
                        Request::SetSocialRelation {
                            actor,
                            target,
                            relation,
                            enabled,
                            reply,
                        } => {
                            let _ = reply.send(set_social_relation(
                                &mut connection,
                                actor,
                                target,
                                relation,
                                enabled,
                                unix_timestamp(),
                            ));
                        }
                        Request::ReportCharacter {
                            actor,
                            target,
                            reason,
                            reply,
                        } => {
                            let _ = reply.send(report_character(
                                &mut connection,
                                actor,
                                target,
                                &reason,
                                unix_timestamp(),
                            ));
                        }
                        Request::ModerateCharacter {
                            actor,
                            target,
                            action,
                            reply,
                        } => {
                            let _ = reply.send(moderate_character(
                                &mut connection,
                                actor,
                                target,
                                action,
                                unix_timestamp(),
                            ));
                        }
                        #[cfg(test)]
                        Request::AuditLog { reply } => {
                            let _ = reply.send(audit_log(&mut connection));
                        }
                    }
                }
            })
            .map_err(StoreError::ThreadSpawn)?;

        ready_rx.await.map_err(|_| StoreError::WorkerStopped)??;
        Ok(Self { tx, passwords })
    }

    async fn request<T>(
        &self,
        request: Request,
        receive: oneshot::Receiver<Result<T, StoreError>>,
    ) -> Result<T, StoreError> {
        self.tx
            .send_timeout(request, PERSISTENCE_REQUEST_TIMEOUT)
            .await
            .map_err(|error| match error {
                mpsc::error::SendTimeoutError::Timeout(_) => StoreError::RequestTimeout,
                mpsc::error::SendTimeoutError::Closed(_) => StoreError::WorkerStopped,
            })?;
        timeout(PERSISTENCE_REQUEST_TIMEOUT, receive)
            .await
            .map_err(|_| StoreError::RequestTimeout)?
            .map_err(|_| StoreError::WorkerStopped)?
    }

    /// Bound admission, then await the definitive result of an accepted
    /// mutation so a local deadline cannot race a later database commit.
    async fn request_commit<T>(
        &self,
        request: Request,
        receive: oneshot::Receiver<Result<T, StoreError>>,
    ) -> Result<T, StoreError> {
        self.tx
            .send_timeout(request, PERSISTENCE_REQUEST_TIMEOUT)
            .await
            .map_err(|error| match error {
                mpsc::error::SendTimeoutError::Timeout(_) => StoreError::RequestTimeout,
                mpsc::error::SendTimeoutError::Closed(_) => StoreError::WorkerStopped,
            })?;
        receive.await.map_err(|_| StoreError::WorkerStopped)?
    }
}

#[async_trait]
impl CharacterStore for PostgresCharacterStore {
    fn backend_name(&self) -> &'static str {
        "postgresql"
    }

    fn queue_depth(&self) -> usize {
        self.tx.max_capacity().saturating_sub(self.tx.capacity())
    }

    async fn health_check(&self) -> Result<(), StoreError> {
        let (reply, receive) = oneshot::channel();
        self.request(Request::Health { reply }, receive).await
    }

    async fn shutdown(&self) -> Result<(), StoreError> {
        let (reply, receive) = oneshot::channel();
        self.request_commit(Request::Shutdown { reply }, receive)
            .await
    }

    async fn register_account(
        &self,
        name: &str,
        password: &str,
    ) -> Result<AuthenticatedSession, StoreError> {
        let name = normalise_name(name, "account")?;
        if !validate_password(password) {
            return Err(StoreError::InvalidPassword);
        }
        let password_hash = self
            .passwords
            .hash(password.to_owned())
            .await
            .map_err(map_password_work_error)?;
        let (reply, receive) = oneshot::channel();
        self.request_commit(
            Request::FinishRegisterAccount {
                name,
                password_hash,
                reply,
            },
            receive,
        )
        .await
    }

    async fn login_account(
        &self,
        name: &str,
        password: &str,
    ) -> Result<AuthenticatedSession, StoreError> {
        let name = normalise_name(name, "account")?;
        if !validate_password(password) {
            return Err(StoreError::InvalidPassword);
        }
        let (reply, receive) = oneshot::channel();
        let prepared = self
            .request(Request::PrepareLoginAccount { name, reply }, receive)
            .await?;
        let Some(prepared) = prepared else {
            self.passwords
                .dummy(password.to_owned())
                .await
                .map_err(map_password_work_error)?;
            return Err(StoreError::InvalidCredentials);
        };
        let valid = self
            .passwords
            .verify(password.to_owned(), prepared.encoded_hash.clone())
            .await
            .map_err(map_password_work_error)?;
        if !valid {
            return Err(StoreError::InvalidCredentials);
        }
        let (reply, receive) = oneshot::channel();
        self.request_commit(Request::FinishLoginAccount { prepared, reply }, receive)
            .await
    }

    async fn resolve_session(&self, token: &str) -> Result<AccountSnapshot, StoreError> {
        let (reply, receive) = oneshot::channel();
        self.request(
            Request::ResolveSession {
                token: token.to_owned(),
                reply,
            },
            receive,
        )
        .await
    }

    async fn revoke_session(&self, token: &str) -> Result<(), StoreError> {
        let (reply, receive) = oneshot::channel();
        self.request_commit(
            Request::RevokeSession {
                token: token.to_owned(),
                reply,
            },
            receive,
        )
        .await
    }

    #[cfg(test)]
    async fn load_or_create_account(&self, name: &str) -> Result<AccountSnapshot, StoreError> {
        let (reply, receive) = oneshot::channel();
        self.request_commit(
            Request::LoadOrCreateAccount {
                name: name.to_owned(),
                reply,
            },
            receive,
        )
        .await
    }

    async fn list_characters(
        &self,
        account_id: AccountId,
    ) -> Result<Vec<CharacterSnapshot>, StoreError> {
        let (reply, receive) = oneshot::channel();
        self.request(Request::ListCharacters { account_id, reply }, receive)
            .await
    }

    #[cfg(test)]
    async fn load_or_create_character(
        &self,
        account_id: AccountId,
        name: &str,
        start_room: &str,
    ) -> Result<CharacterSnapshot, StoreError> {
        let (reply, receive) = oneshot::channel();
        self.request_commit(
            Request::LoadOrCreateCharacter {
                account_id,
                name: name.to_owned(),
                start_room: start_room.to_owned(),
                initial_gameplay: Box::new(CharacterGameplaySnapshot::default()),
                reply,
            },
            receive,
        )
        .await
    }

    async fn load_or_create_character_with_gameplay(
        &self,
        account_id: AccountId,
        name: &str,
        start_room: &str,
        initial_gameplay: CharacterGameplaySnapshot,
    ) -> Result<CharacterSnapshot, StoreError> {
        let (reply, receive) = oneshot::channel();
        self.request_commit(
            Request::LoadOrCreateCharacter {
                account_id,
                name: name.to_owned(),
                start_room: start_room.to_owned(),
                initial_gameplay: Box::new(initial_gameplay),
                reply,
            },
            receive,
        )
        .await
    }

    async fn load_character(
        &self,
        account_id: AccountId,
        character_id: CharacterId,
    ) -> Result<CharacterSnapshot, StoreError> {
        let (reply, receive) = oneshot::channel();
        self.request(
            Request::LoadCharacter {
                account_id,
                character_id,
                reply,
            },
            receive,
        )
        .await
    }

    async fn save(&self, snapshot: CharacterSnapshot) -> Result<(), StoreError> {
        let (reply, receive) = oneshot::channel();
        self.request_commit(Request::Save { snapshot, reply }, receive)
            .await
    }

    async fn load_character_gameplay(
        &self,
        character_id: CharacterId,
    ) -> Result<CharacterGameplaySnapshot, StoreError> {
        let (reply, receive) = oneshot::channel();
        self.request(
            Request::LoadCharacterGameplay {
                character_id,
                reply,
            },
            receive,
        )
        .await
    }

    async fn load_runtime_snapshot(&self) -> Result<Option<WorldRuntimeSnapshot>, StoreError> {
        let (reply, receive) = oneshot::channel();
        self.request(Request::LoadRuntimeSnapshot { reply }, receive)
            .await
    }

    async fn load_domain_events(&self) -> Result<Vec<DomainEvent>, StoreError> {
        let (reply, receive) = oneshot::channel();
        self.request(Request::LoadDomainEvents { reply }, receive)
            .await
    }

    async fn social_profile(&self, character_id: CharacterId) -> Result<SocialProfile, StoreError> {
        let (reply, receive) = oneshot::channel();
        self.request(
            Request::SocialProfile {
                character_id,
                reply,
            },
            receive,
        )
        .await
    }

    async fn social_target(&self, name: &str) -> Result<SocialProfile, StoreError> {
        let (reply, receive) = oneshot::channel();
        self.request(
            Request::SocialTarget {
                name: name.to_owned(),
                reply,
            },
            receive,
        )
        .await
    }

    async fn social_delivery_allowed(
        &self,
        sender: CharacterId,
        recipient: CharacterId,
    ) -> Result<bool, StoreError> {
        let (reply, receive) = oneshot::channel();
        self.request(
            Request::SocialDeliveryAllowed {
                sender,
                recipient,
                reply,
            },
            receive,
        )
        .await
    }

    async fn set_social_relation(
        &self,
        actor: CharacterId,
        target: CharacterId,
        relation: SocialRelationKind,
        enabled: bool,
    ) -> Result<(), StoreError> {
        let (reply, receive) = oneshot::channel();
        self.request_commit(
            Request::SetSocialRelation {
                actor,
                target,
                relation,
                enabled,
                reply,
            },
            receive,
        )
        .await
    }

    async fn report_character(
        &self,
        actor: CharacterId,
        target: CharacterId,
        reason: &str,
    ) -> Result<(), StoreError> {
        let (reply, receive) = oneshot::channel();
        self.request_commit(
            Request::ReportCharacter {
                actor,
                target,
                reason: reason.to_owned(),
                reply,
            },
            receive,
        )
        .await
    }

    async fn moderate_character(
        &self,
        actor: CharacterId,
        target: CharacterId,
        action: ModerationAction,
    ) -> Result<(), StoreError> {
        let (reply, receive) = oneshot::channel();
        self.request_commit(
            Request::ModerateCharacter {
                actor,
                target,
                action,
                reply,
            },
            receive,
        )
        .await
    }

    #[cfg(test)]
    async fn audit_log(&self) -> Result<Vec<AuditEntry>, StoreError> {
        let (reply, receive) = oneshot::channel();
        self.request(Request::AuditLog { reply }, receive).await
    }

    async fn commit_action(
        &self,
        characters: Vec<DurableCharacterSnapshot>,
        runtime: WorldRuntimeSnapshot,
        events: Vec<DomainEvent>,
    ) -> Result<(), StoreError> {
        let (reply, receive) = oneshot::channel();
        self.request_commit(
            Request::CommitAction {
                characters,
                runtime,
                events,
                reply,
            },
            receive,
        )
        .await
    }
}

fn normalise_name(name: &str, kind: &'static str) -> Result<String, StoreError> {
    let name = name.trim();
    if name.is_empty() || name.len() > MAX_IDENTITY_NAME_BYTES || name.chars().any(char::is_control)
    {
        return Err(StoreError::InvalidName { kind });
    }
    Ok(name.to_owned())
}

fn map_password_work_error(error: PasswordWorkError) -> StoreError {
    match error {
        PasswordWorkError::Overloaded | PasswordWorkError::Timeout => StoreError::RequestTimeout,
        PasswordWorkError::Execution => StoreError::PasswordHashing,
    }
}

fn unix_timestamp() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
        .try_into()
        .unwrap_or(i64::MAX)
}

fn finish_register_account(
    connection: &mut Client,
    name: &str,
    password_hash: &str,
    now: i64,
) -> Result<AuthenticatedSession, StoreError> {
    let mut transaction = connection.transaction()?;
    // Serialises first-account bootstrap and cross-process duplicate checks.
    transaction.batch_execute("LOCK TABLE accounts IN SHARE ROW EXCLUSIVE MODE")?;
    if transaction
        .query_opt(
            "SELECT 1::BIGINT FROM accounts WHERE lower(name) = lower($1) LIMIT 1",
            &[&name],
        )?
        .is_some()
    {
        return Err(StoreError::AccountAlreadyExists);
    }
    let role = if transaction
        .query_one(
            "SELECT COUNT(*)::BIGINT FROM accounts WHERE auth_state = 'registered'",
            &[],
        )?
        .get::<_, i64>(0)
        == 0
    {
        AccountRole::Admin
    } else {
        AccountRole::Player
    };
    let id: i64 = transaction
        .query_one(
            "INSERT INTO accounts (name, password_hash, auth_state, role)
             VALUES ($1, $2, 'registered', $3) RETURNING id",
            &[&name, &password_hash, &role_as_database(role)],
        )?
        .get(0);
    let account = AccountSnapshot {
        id: AccountId(id),
        name: name.to_owned(),
    };
    if role == AccountRole::Admin {
        append_audit(
            &mut transaction,
            account.id,
            Some(account.id),
            "bootstrap_admin",
            serde_json::json!({"source": "first_registered_account"}),
            now,
        )?;
    }
    let session = issue_session(&mut transaction, account, now)?;
    transaction.commit()?;
    Ok(session)
}

fn prepare_login_account<C: GenericClient>(
    connection: &mut C,
    name: &str,
) -> Result<Option<PreparedLogin>, StoreError> {
    let prepared = connection
        .query_opt(
            "SELECT id, password_hash FROM accounts
         WHERE lower(name) = lower($1) AND auth_state = 'registered'",
            &[&name],
        )?
        .and_then(|row| {
            row.get::<_, Option<String>>(1)
                .map(|encoded_hash| PreparedLogin {
                    account_id: AccountId(row.get(0)),
                    encoded_hash,
                })
        });
    Ok(prepared)
}

fn finish_login_account(
    connection: &mut Client,
    prepared: &PreparedLogin,
    now: i64,
) -> Result<AuthenticatedSession, StoreError> {
    let mut transaction = connection.transaction()?;
    let current = transaction.query_opt(
        "SELECT name, password_hash FROM accounts
         WHERE id = $1 AND auth_state = 'registered'",
        &[&prepared.account_id.0],
    )?;
    let Some(row) = current else {
        return Err(StoreError::InvalidCredentials);
    };
    let Some(encoded_hash) = row.get::<_, Option<String>>(1) else {
        return Err(StoreError::InvalidCredentials);
    };
    if encoded_hash != prepared.encoded_hash {
        return Err(StoreError::InvalidCredentials);
    }
    let account = AccountSnapshot {
        id: prepared.account_id,
        name: row.get(0),
    };
    if account_is_banned(&mut transaction, account.id, now)? {
        return Err(StoreError::AccountBanned);
    }

    let session = issue_session(&mut transaction, account, now)?;
    transaction.commit()?;
    Ok(session)
}

fn issue_session<C: GenericClient>(
    connection: &mut C,
    account: AccountSnapshot,
    now: i64,
) -> Result<AuthenticatedSession, StoreError> {
    let ttl = i64::try_from(DEFAULT_SESSION_TTL.as_secs()).unwrap_or(i64::MAX);
    let expires_at = now.checked_add(ttl).ok_or(StoreError::InvalidSession)?;

    connection.execute(
        "DELETE FROM auth_sessions
         WHERE account_id = $1 AND (expires_at <= $2 OR revoked_at IS NOT NULL)",
        &[&account.id.0, &now],
    )?;
    let active: i64 = connection
        .query_one(
            "SELECT COUNT(*)::BIGINT FROM auth_sessions WHERE account_id = $1",
            &[&account.id.0],
        )?
        .get(0);
    if active >= MAX_ACTIVE_SESSIONS {
        let remove_count = active - MAX_ACTIVE_SESSIONS + 1;
        connection.execute(
            "DELETE FROM auth_sessions WHERE token_hash IN (
                 SELECT token_hash FROM auth_sessions
                 WHERE account_id = $1
                 ORDER BY created_at, token_hash LIMIT $2
             )",
            &[&account.id.0, &remove_count],
        )?;
    }

    let token = SessionToken::generate();
    connection.execute(
        "INSERT INTO auth_sessions
             (token_hash, account_id, created_at, expires_at)
         VALUES ($1, $2, $3, $4)",
        &[&token.digest().as_slice(), &account.id.0, &now, &expires_at],
    )?;
    Ok(AuthenticatedSession {
        account,
        token,
        expires_at,
    })
}

fn resolve_session<C: GenericClient>(
    connection: &mut C,
    token: &str,
    now: i64,
) -> Result<AccountSnapshot, StoreError> {
    let token_hash = digest_token(token).ok_or(StoreError::InvalidSession)?;
    let row = connection
        .query_opt(
            "SELECT accounts.id, accounts.name, auth_sessions.expires_at,
                    auth_sessions.revoked_at
             FROM auth_sessions
             JOIN accounts ON accounts.id = auth_sessions.account_id
             WHERE auth_sessions.token_hash = $1",
            &[&token_hash.as_slice()],
        )?
        .ok_or(StoreError::InvalidSession)?;
    let account = AccountSnapshot {
        id: AccountId(row.get(0)),
        name: row.get(1),
    };
    let expires_at: i64 = row.get(2);
    let revoked_at: Option<i64> = row.get(3);
    if revoked_at.is_some() {
        return Err(StoreError::SessionRevoked);
    }
    if expires_at <= now {
        return Err(StoreError::SessionExpired);
    }
    if account_is_banned(connection, account.id, now)? {
        return Err(StoreError::AccountBanned);
    }
    Ok(account)
}

fn account_is_banned<C: GenericClient>(
    connection: &mut C,
    account_id: AccountId,
    now: i64,
) -> Result<bool, StoreError> {
    Ok(connection
        .query_one(
            "SELECT EXISTS(
                 SELECT 1 FROM account_bans
                 WHERE account_id = $1 AND revoked_at IS NULL
                   AND (expires_at IS NULL OR expires_at > $2)
             )",
            &[&account_id.0, &now],
        )?
        .get(0))
}

fn revoke_session<C: GenericClient>(
    connection: &mut C,
    token: &str,
    now: i64,
) -> Result<(), StoreError> {
    let token_hash = digest_token(token).ok_or(StoreError::InvalidSession)?;
    let updated = connection.execute(
        "UPDATE auth_sessions SET revoked_at = COALESCE(revoked_at, $1)
         WHERE token_hash = $2",
        &[&now, &token_hash.as_slice()],
    )?;
    if updated == 0 {
        return Err(StoreError::InvalidSession);
    }
    Ok(())
}

fn ensure_account_exists<C: GenericClient>(
    connection: &mut C,
    account_id: AccountId,
) -> Result<(), StoreError> {
    if connection
        .query_opt(
            "SELECT 1::BIGINT FROM accounts WHERE id = $1",
            &[&account_id.0],
        )?
        .is_some()
    {
        Ok(())
    } else {
        Err(StoreError::AccountNotFound(account_id))
    }
}

#[cfg(test)]
fn load_or_create_account(
    connection: &mut Client,
    name: &str,
) -> Result<AccountSnapshot, StoreError> {
    let name = normalise_name(name, "account")?;
    connection.execute(
        "INSERT INTO accounts (name, auth_state)
         VALUES ($1, 'guest') ON CONFLICT DO NOTHING",
        &[&name],
    )?;
    connection
        .query_opt(
            "SELECT id, name FROM accounts
             WHERE lower(name) = lower($1) AND auth_state = 'guest'",
            &[&name],
        )?
        .map(|row| AccountSnapshot {
            id: AccountId(row.get(0)),
            name: row.get(1),
        })
        .ok_or(StoreError::AccountAlreadyExists)
}

fn list_characters<C: GenericClient>(
    connection: &mut C,
    account_id: AccountId,
) -> Result<Vec<CharacterSnapshot>, StoreError> {
    ensure_account_exists(connection, account_id)?;
    connection
        .query(
            "SELECT id, account_id, name, room_id, item_locations_json
             FROM characters WHERE account_id = $1 ORDER BY lower(name), id",
            &[&account_id.0],
        )?
        .iter()
        .map(character_from_row)
        .collect()
}

fn load_or_create_character_with_gameplay<C: GenericClient>(
    connection: &mut C,
    account_id: AccountId,
    name: &str,
    start_room: &str,
    initial_gameplay: &CharacterGameplaySnapshot,
) -> Result<CharacterSnapshot, StoreError> {
    ensure_account_exists(connection, account_id)?;
    let name = normalise_name(name, "character")?;
    let gameplay_json = bounded_json(
        initial_gameplay,
        "character gameplay",
        MAX_CHARACTER_JSON_BYTES,
    )?;
    connection.execute(
        "INSERT INTO characters (account_id, name, room_id, gameplay_json)
         VALUES ($1, $2, $3, $4) ON CONFLICT DO NOTHING",
        &[&account_id.0, &name, &start_room, &gameplay_json],
    )?;
    connection
        .query_opt(
            "SELECT id, account_id, name, room_id, item_locations_json
             FROM characters
             WHERE account_id = $1 AND lower(name) = lower($2)",
            &[&account_id.0, &name],
        )?
        .as_ref()
        .map(character_from_row)
        .transpose()?
        .ok_or(StoreError::AccountAlreadyExists)
}

fn load_character<C: GenericClient>(
    connection: &mut C,
    account_id: AccountId,
    character_id: CharacterId,
) -> Result<CharacterSnapshot, StoreError> {
    ensure_account_exists(connection, account_id)?;
    connection
        .query_opt(
            "SELECT id, account_id, name, room_id, item_locations_json
             FROM characters WHERE account_id = $1 AND id = $2",
            &[&account_id.0, &character_id.0],
        )?
        .as_ref()
        .map(character_from_row)
        .transpose()?
        .ok_or(StoreError::CharacterNotFound {
            account_id,
            character_id,
        })
}

fn character_from_row(row: &postgres::Row) -> Result<CharacterSnapshot, StoreError> {
    let item_locations_json: String = row.get(4);
    Ok(CharacterSnapshot {
        id: CharacterId(row.get(0)),
        account_id: AccountId(row.get(1)),
        name: row.get(2),
        room_id: row.get(3),
        item_locations: serde_json::from_str(&item_locations_json)?,
    })
}

fn save<C: GenericClient>(
    connection: &mut C,
    snapshot: &CharacterSnapshot,
) -> Result<(), StoreError> {
    let name = normalise_name(&snapshot.name, "character")?;
    let item_locations_json = serde_json::to_string(&snapshot.item_locations)?;
    let updated = connection.execute(
        "UPDATE characters
         SET name = $1, room_id = $2, item_locations_json = $3,
             updated_at = floor(extract(epoch FROM now()))::BIGINT
         WHERE id = $4 AND account_id = $5",
        &[
            &name,
            &snapshot.room_id,
            &item_locations_json,
            &snapshot.id.0,
            &snapshot.account_id.0,
        ],
    )?;
    if updated == 0 {
        return Err(StoreError::CharacterNotFound {
            account_id: snapshot.account_id,
            character_id: snapshot.id,
        });
    }
    Ok(())
}

fn load_character_gameplay<C: GenericClient>(
    connection: &mut C,
    character_id: CharacterId,
) -> Result<CharacterGameplaySnapshot, StoreError> {
    let row = connection
        .query_opt(
            "SELECT gameplay_json FROM characters WHERE id = $1",
            &[&character_id.0],
        )?
        .ok_or(StoreError::CharacterGameplayNotFound(character_id))?;
    let json: String = row.get(0);
    serde_json::from_str(&json).map_err(StoreError::from)
}

fn load_runtime_snapshot<C: GenericClient>(
    connection: &mut C,
) -> Result<Option<WorldRuntimeSnapshot>, StoreError> {
    connection
        .query_opt(
            "SELECT snapshot_json FROM world_runtime WHERE singleton = 1",
            &[],
        )?
        .map(|row| {
            let json: String = row.get(0);
            serde_json::from_str(&json).map_err(StoreError::from)
        })
        .transpose()
}

fn load_domain_events<C: GenericClient>(
    connection: &mut C,
) -> Result<Vec<DomainEvent>, StoreError> {
    connection
        .query(
            "SELECT event_json FROM domain_events ORDER BY sequence",
            &[],
        )?
        .into_iter()
        .map(|row| {
            let json: String = row.get(0);
            serde_json::from_str(&json).map_err(StoreError::from)
        })
        .collect()
}

fn commit_action(
    connection: &mut Client,
    characters: &[DurableCharacterSnapshot],
    runtime: &WorldRuntimeSnapshot,
    events: &[DomainEvent],
) -> Result<(), StoreError> {
    let gameplay_json = characters
        .iter()
        .map(|snapshot| {
            bounded_json(
                &snapshot.gameplay,
                "character gameplay",
                MAX_CHARACTER_JSON_BYTES,
            )
        })
        .collect::<Result<Vec<_>, _>>()?;
    let adventure_json = characters
        .iter()
        .map(|snapshot| {
            bounded_json(
                &serde_json::json!({
                    "coins": snapshot.gameplay.coins,
                    "commodities": snapshot.gameplay.commodities,
                    "quests": snapshot.gameplay.quests,
                }),
                "character adventure",
                MAX_CHARACTER_JSON_BYTES,
            )
        })
        .collect::<Result<Vec<_>, _>>()?;
    let runtime_json = bounded_json(runtime, "world runtime", MAX_RUNTIME_JSON_BYTES)?;
    let world_adventure_json = bounded_json(
        &runtime.adventure,
        "world adventure",
        MAX_RUNTIME_JSON_BYTES,
    )?;
    let event_json = events
        .iter()
        .map(|event| bounded_json(event, "domain event", MAX_EVENT_JSON_BYTES))
        .collect::<Result<Vec<_>, _>>()?;

    let mut transaction = connection.transaction()?;
    for ((snapshot, gameplay_json), adventure_json) in
        characters.iter().zip(gameplay_json).zip(adventure_json)
    {
        save(&mut transaction, &snapshot.character)?;
        let updated = transaction.execute(
            "UPDATE characters
             SET gameplay_json = $1,
                 updated_at = floor(extract(epoch FROM now()))::BIGINT
             WHERE id = $2 AND account_id = $3",
            &[
                &gameplay_json,
                &snapshot.character.id.0,
                &snapshot.character.account_id.0,
            ],
        )?;
        if updated == 0 {
            return Err(StoreError::CharacterNotFound {
                account_id: snapshot.character.account_id,
                character_id: snapshot.character.id,
            });
        }
        transaction.execute(
            "INSERT INTO character_adventure (character_id, state_json, updated_at)
             VALUES ($1, $2, floor(extract(epoch FROM now()))::BIGINT)
             ON CONFLICT(character_id) DO UPDATE SET
                 state_json = excluded.state_json,
                 updated_at = excluded.updated_at",
            &[&snapshot.character.id.0, &adventure_json],
        )?;
    }
    transaction.execute(
        "INSERT INTO world_adventure (singleton, state_json, updated_at)
         VALUES (1, $1, floor(extract(epoch FROM now()))::BIGINT)
         ON CONFLICT(singleton) DO UPDATE SET
             state_json = excluded.state_json,
             updated_at = excluded.updated_at",
        &[&world_adventure_json],
    )?;
    for json in event_json {
        transaction.execute(
            "INSERT INTO domain_events (event_json) VALUES ($1)",
            &[&json],
        )?;
    }
    let last_event_sequence: i64 = transaction
        .query_one(
            "SELECT GREATEST(
                 COALESCE((SELECT MAX(sequence) FROM domain_events), 0),
                 COALESCE((SELECT last_event_sequence FROM world_runtime
                           WHERE singleton = 1), 0)
             )",
            &[],
        )?
        .get(0);
    let snapshot_version = i64::from(runtime.snapshot_version);
    transaction.execute(
        "INSERT INTO world_runtime
             (singleton, snapshot_version, snapshot_json,
              last_event_sequence, updated_at)
         VALUES (1, $1, $2, $3, floor(extract(epoch FROM now()))::BIGINT)
         ON CONFLICT(singleton) DO UPDATE SET
             snapshot_version = excluded.snapshot_version,
             snapshot_json = excluded.snapshot_json,
             last_event_sequence = excluded.last_event_sequence,
             updated_at = excluded.updated_at",
        &[&snapshot_version, &runtime_json, &last_event_sequence],
    )?;
    transaction.execute(
        // Prepared parameters are initially `unknown` to PostgreSQL. Keep the
        // casts on both arithmetic operands so PostgreSQL 18 can resolve `-`.
        "DELETE FROM domain_events
         WHERE sequence <= ($1::BIGINT - $2::BIGINT)
           AND sequence <= $1::BIGINT",
        &[&last_event_sequence, &DOMAIN_EVENT_RETENTION],
    )?;
    transaction.commit()?;
    Ok(())
}

fn role_from_database(value: &str) -> Result<AccountRole, StoreError> {
    match value {
        "player" => Ok(AccountRole::Player),
        "moderator" => Ok(AccountRole::Moderator),
        "admin" => Ok(AccountRole::Admin),
        _ => Err(StoreError::InvalidSocialInput),
    }
}

fn role_as_database(role: AccountRole) -> &'static str {
    match role {
        AccountRole::Player => "player",
        AccountRole::Moderator => "moderator",
        AccountRole::Admin => "admin",
    }
}

fn social_profile<C: GenericClient>(
    connection: &mut C,
    character_id: CharacterId,
    now: i64,
) -> Result<SocialProfile, StoreError> {
    let row = connection
        .query_opt(
            "SELECT characters.id, characters.account_id, characters.name, accounts.role,
                    EXISTS(
                        SELECT 1 FROM moderation_mutes
                        WHERE moderation_mutes.account_id = accounts.id
                          AND moderation_mutes.revoked_at IS NULL
                          AND (moderation_mutes.expires_at IS NULL
                               OR moderation_mutes.expires_at > $2)
                    )
             FROM characters
             JOIN accounts ON accounts.id = characters.account_id
             WHERE characters.id = $1",
            &[&character_id.0, &now],
        )?
        .ok_or(StoreError::SocialTargetNotFound)?;
    let role: String = row.get(3);
    Ok(SocialProfile {
        character_id: CharacterId(row.get(0)),
        account_id: AccountId(row.get(1)),
        character_name: row.get(2),
        role: role_from_database(&role)?,
        muted: row.get(4),
    })
}

fn social_target<C: GenericClient>(
    connection: &mut C,
    name: &str,
    now: i64,
) -> Result<SocialProfile, StoreError> {
    let name = normalise_name(name, "social target")?;
    let matches = connection.query(
        "SELECT id FROM characters WHERE lower(name) = lower($1) ORDER BY id LIMIT 2",
        &[&name],
    )?;
    match matches.as_slice() {
        [row] => social_profile(connection, CharacterId(row.get(0)), now),
        [] => Err(StoreError::SocialTargetNotFound),
        [_, ..] => Err(StoreError::AmbiguousSocialTarget),
    }
}

fn social_delivery_allowed<C: GenericClient>(
    connection: &mut C,
    sender: CharacterId,
    recipient: CharacterId,
) -> Result<bool, StoreError> {
    if sender == recipient {
        return Ok(true);
    }
    let both_exist: i64 = connection
        .query_one(
            "SELECT COUNT(*)::BIGINT FROM characters WHERE id = ANY($1)",
            &[&[sender.0, recipient.0].as_slice()],
        )?
        .get(0);
    if both_exist != 2 {
        return Err(StoreError::SocialTargetNotFound);
    }
    let suppressed: bool = connection
        .query_one(
            "SELECT
                 EXISTS(SELECT 1 FROM social_ignores
                        WHERE owner_character_id = $2 AND target_character_id = $1)
                 OR EXISTS(SELECT 1 FROM social_blocks
                           WHERE owner_character_id = $2 AND target_character_id = $1)
                 OR EXISTS(SELECT 1 FROM social_blocks
                           WHERE owner_character_id = $1 AND target_character_id = $2)",
            &[&sender.0, &recipient.0],
        )?
        .get(0);
    Ok(!suppressed)
}

fn set_social_relation(
    connection: &mut Client,
    actor: CharacterId,
    target: CharacterId,
    relation: SocialRelationKind,
    enabled: bool,
    now: i64,
) -> Result<(), StoreError> {
    if actor == target {
        return Err(StoreError::SocialSelfTarget);
    }
    let mut transaction = connection.transaction()?;
    let actor_profile = social_profile(&mut transaction, actor, now)?;
    let target_profile = social_profile(&mut transaction, target, now)?;
    if actor_profile.account_id == target_profile.account_id {
        return Err(StoreError::SocialSelfTarget);
    }
    let table = match relation {
        SocialRelationKind::Ignore => "social_ignores",
        SocialRelationKind::Block => "social_blocks",
    };
    if enabled {
        transaction.execute(
            &format!(
                "INSERT INTO {table}
                     (owner_character_id, target_character_id, created_at)
                 VALUES ($1, $2, $3) ON CONFLICT DO NOTHING"
            ),
            &[&actor.0, &target.0, &now],
        )?;
    } else {
        transaction.execute(
            &format!(
                "DELETE FROM {table}
                 WHERE owner_character_id = $1 AND target_character_id = $2"
            ),
            &[&actor.0, &target.0],
        )?;
    }
    let action = match (relation, enabled) {
        (SocialRelationKind::Ignore, true) => "ignore",
        (SocialRelationKind::Ignore, false) => "unignore",
        (SocialRelationKind::Block, true) => "block",
        (SocialRelationKind::Block, false) => "unblock",
    };
    append_audit(
        &mut transaction,
        actor_profile.account_id,
        Some(target_profile.account_id),
        action,
        serde_json::json!({"targetCharacterId": target.0}),
        now,
    )?;
    transaction.commit()?;
    Ok(())
}

fn report_character(
    connection: &mut Client,
    actor: CharacterId,
    target: CharacterId,
    reason: &str,
    now: i64,
) -> Result<(), StoreError> {
    if actor == target {
        return Err(StoreError::SocialSelfTarget);
    }
    let reason = normalise_social_text(reason, MAX_REPORT_REASON_BYTES)?;
    let mut transaction = connection.transaction()?;
    let actor_profile = social_profile(&mut transaction, actor, now)?;
    let target_profile = social_profile(&mut transaction, target, now)?;
    if actor_profile.account_id == target_profile.account_id {
        return Err(StoreError::SocialSelfTarget);
    }
    let report_id: i64 = transaction
        .query_one(
            "INSERT INTO social_reports
                 (reporter_character_id, target_character_id, reason, created_at)
             VALUES ($1, $2, $3, $4) RETURNING id",
            &[&actor.0, &target.0, &reason, &now],
        )?
        .get(0);
    append_audit(
        &mut transaction,
        actor_profile.account_id,
        Some(target_profile.account_id),
        "report",
        serde_json::json!({"reportId": report_id}),
        now,
    )?;
    transaction.commit()?;
    Ok(())
}

fn moderate_character(
    connection: &mut Client,
    actor: CharacterId,
    target: CharacterId,
    action: ModerationAction,
    now: i64,
) -> Result<(), StoreError> {
    if actor == target {
        return Err(StoreError::SocialSelfTarget);
    }
    let mut transaction = connection.transaction()?;
    let actor_profile = social_profile(&mut transaction, actor, now)?;
    let target_profile = social_profile(&mut transaction, target, now)?;
    if actor_profile.account_id == target_profile.account_id {
        return Err(StoreError::SocialSelfTarget);
    }
    let role_change = matches!(action, ModerationAction::SetRole(_));
    let authorized = if role_change {
        actor_profile.role == AccountRole::Admin
    } else {
        actor_profile.role.can_moderate(target_profile.role)
    };
    if !authorized {
        return Err(StoreError::SocialForbidden);
    }

    let (action_name, details) = match action {
        ModerationAction::Mute { expires_at, reason } => {
            let reason = normalise_social_text(&reason, MAX_SOCIAL_REASON_BYTES)?;
            validate_expiry(expires_at, now)?;
            revoke_active(
                &mut transaction,
                "moderation_mutes",
                target_profile.account_id,
                actor_profile.account_id,
                now,
            )?;
            transaction.execute(
                "INSERT INTO moderation_mutes
                     (account_id, imposed_by_account_id, reason, created_at, expires_at)
                 VALUES ($1, $2, $3, $4, $5)",
                &[
                    &target_profile.account_id.0,
                    &actor_profile.account_id.0,
                    &reason,
                    &now,
                    &expires_at,
                ],
            )?;
            (
                "mute",
                serde_json::json!({"expiresAt": expires_at, "reason": reason}),
            )
        }
        ModerationAction::Unmute => {
            revoke_active(
                &mut transaction,
                "moderation_mutes",
                target_profile.account_id,
                actor_profile.account_id,
                now,
            )?;
            ("unmute", serde_json::json!({}))
        }
        ModerationAction::Ban { expires_at, reason } => {
            let reason = normalise_social_text(&reason, MAX_SOCIAL_REASON_BYTES)?;
            validate_expiry(expires_at, now)?;
            revoke_active(
                &mut transaction,
                "account_bans",
                target_profile.account_id,
                actor_profile.account_id,
                now,
            )?;
            transaction.execute(
                "INSERT INTO account_bans
                     (account_id, imposed_by_account_id, reason, created_at, expires_at)
                 VALUES ($1, $2, $3, $4, $5)",
                &[
                    &target_profile.account_id.0,
                    &actor_profile.account_id.0,
                    &reason,
                    &now,
                    &expires_at,
                ],
            )?;
            transaction.execute(
                "UPDATE auth_sessions SET revoked_at = COALESCE(revoked_at, $1)
                 WHERE account_id = $2",
                &[&now, &target_profile.account_id.0],
            )?;
            (
                "ban",
                serde_json::json!({"expiresAt": expires_at, "reason": reason}),
            )
        }
        ModerationAction::Unban => {
            revoke_active(
                &mut transaction,
                "account_bans",
                target_profile.account_id,
                actor_profile.account_id,
                now,
            )?;
            ("unban", serde_json::json!({}))
        }
        ModerationAction::SetRole(role) => {
            transaction.execute(
                "UPDATE accounts SET role = $1 WHERE id = $2",
                &[&role_as_database(role), &target_profile.account_id.0],
            )?;
            (
                "set_role",
                serde_json::json!({"role": role_as_database(role)}),
            )
        }
    };
    append_audit(
        &mut transaction,
        actor_profile.account_id,
        Some(target_profile.account_id),
        action_name,
        details,
        now,
    )?;
    transaction.commit()?;
    Ok(())
}

fn validate_expiry(expires_at: Option<i64>, now: i64) -> Result<(), StoreError> {
    if expires_at.is_some_and(|expires_at| {
        expires_at <= now || expires_at.saturating_sub(now) > MAX_MODERATION_DURATION_SECONDS
    }) {
        Err(StoreError::InvalidSocialInput)
    } else {
        Ok(())
    }
}

fn revoke_active<C: GenericClient>(
    connection: &mut C,
    table: &str,
    target: AccountId,
    actor: AccountId,
    now: i64,
) -> Result<(), StoreError> {
    debug_assert!(matches!(table, "moderation_mutes" | "account_bans"));
    connection.execute(
        &format!(
            "UPDATE {table}
             SET revoked_at = $1, revoked_by_account_id = $2
             WHERE account_id = $3 AND revoked_at IS NULL
               AND (expires_at IS NULL OR expires_at > $1)"
        ),
        &[&now, &actor.0, &target.0],
    )?;
    Ok(())
}

fn normalise_social_text(value: &str, maximum: usize) -> Result<String, StoreError> {
    let value = value.trim();
    if value.is_empty() || value.len() > maximum || value.chars().any(char::is_control) {
        Err(StoreError::InvalidSocialInput)
    } else {
        Ok(value.to_owned())
    }
}

fn append_audit<C: GenericClient>(
    connection: &mut C,
    actor: AccountId,
    target: Option<AccountId>,
    action: &str,
    details: serde_json::Value,
    now: i64,
) -> Result<(), StoreError> {
    let details_json = serde_json::to_string(&details)?;
    if action.is_empty() || action.len() > 64 || details_json.len() > MAX_AUDIT_DETAILS_BYTES {
        return Err(StoreError::InvalidSocialInput);
    }
    connection.execute(
        "INSERT INTO moderation_audit
             (actor_account_id, target_account_id, action, details_json, created_at)
         VALUES ($1, $2, $3, $4, $5)",
        &[
            &actor.0,
            &target.map(|target| target.0),
            &action,
            &details_json,
            &now,
        ],
    )?;
    Ok(())
}

#[cfg(test)]
fn audit_log(connection: &mut Client) -> Result<Vec<AuditEntry>, StoreError> {
    Ok(connection
        .query(
            "SELECT sequence, actor_account_id, target_account_id, action,
                    details_json, created_at
             FROM moderation_audit ORDER BY sequence",
            &[],
        )?
        .into_iter()
        .map(|row| AuditEntry {
            sequence: row.get(0),
            actor_account_id: AccountId(row.get(1)),
            target_account_id: row.get::<_, Option<i64>>(2).map(AccountId),
            action: row.get(3),
            details_json: row.get(4),
            created_at: row.get(5),
        })
        .collect())
}

fn bounded_json<T: serde::Serialize>(
    value: &T,
    kind: &'static str,
    maximum: usize,
) -> Result<String, StoreError> {
    let json = serde_json::to_string(value)?;
    if json.len() > maximum {
        return Err(StoreError::SnapshotTooLarge { kind, maximum });
    }
    Ok(json)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{collections::BTreeMap, env, process, time::Duration};
    use tiamud_core::{
        AdventureRuntimeSnapshot, Direction, ItemLocationSnapshot, RuntimeItemLocation,
        WORLD_RUNTIME_SNAPSHOT_VERSION,
    };

    fn scoped_url(base: &str, schema: &str) -> String {
        let separator = if base.contains('?') { '&' } else { '?' };
        format!("{base}{separator}options=-csearch_path%3D{schema}")
    }

    async fn admin_batch(base_url: &str, sql: String) {
        let base_url = base_url.to_owned();
        tokio::task::spawn_blocking(move || {
            tiamud_persistence_postgres::connect(&base_url)
                .unwrap()
                .batch_execute(&sql)
                .unwrap();
        })
        .await
        .unwrap();
    }

    async fn admin_count(base_url: &str, sql: String) -> i64 {
        let base_url = base_url.to_owned();
        tokio::task::spawn_blocking(move || {
            tiamud_persistence_postgres::connect(&base_url)
                .unwrap()
                .query_one(&sql, &[])
                .unwrap()
                .get(0)
        })
        .await
        .unwrap()
    }

    #[tokio::test]
    async fn postgres_store_round_trips_auth_characters_runtime_and_social_policy() {
        let Ok(base_url) = env::var("TIAMUD_TEST_POSTGRES_URL") else {
            return;
        };
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let schema = format!("tiamud_store_test_{}_{}", process::id(), nonce);
        admin_batch(&base_url, format!("CREATE SCHEMA {schema}")).await;

        let store = PostgresCharacterStore::open(&scoped_url(&base_url, &schema))
            .await
            .unwrap();
        store.health_check().await.unwrap();

        let keeper = store
            .register_account(" Keeper ", "correct horse battery staple")
            .await
            .unwrap();
        let player = store
            .register_account("Player", "another correct horse battery staple")
            .await
            .unwrap();
        let login = store
            .login_account("KEEPER", "correct horse battery staple")
            .await
            .unwrap();
        assert_eq!(login.account, keeper.account);
        assert_eq!(
            store.resolve_session(login.token.as_str()).await.unwrap(),
            keeper.account
        );

        let keeper_character = store
            .load_or_create_character(keeper.account.id, "Aster", "room:start")
            .await
            .unwrap();
        let mut selected_gameplay = CharacterGameplaySnapshot::default();
        selected_gameplay.attributes.defence = 9;
        let player_character = store
            .load_or_create_character_with_gameplay(
                player.account.id,
                "Bryn",
                "room:start",
                selected_gameplay.clone(),
            )
            .await
            .unwrap();
        assert_eq!(
            store
                .load_character_gameplay(player_character.id)
                .await
                .unwrap(),
            selected_gameplay
        );
        assert_eq!(
            store.list_characters(keeper.account.id).await.unwrap(),
            vec![keeper_character.clone()]
        );
        assert_eq!(
            store
                .load_character(keeper.account.id, keeper_character.id)
                .await
                .unwrap(),
            keeper_character
        );

        let mut updated_character = keeper_character.clone();
        updated_character.room_id = "room:north".into();
        updated_character
            .item_locations
            .insert("item:lamp".into(), ItemLocationSnapshot::Carried);
        let gameplay = CharacterGameplaySnapshot::default();
        let runtime = WorldRuntimeSnapshot {
            snapshot_version: WORLD_RUNTIME_SNAPSHOT_VERSION,
            revision: 9,
            logical_tick: 3,
            rng_state: 42,
            item_locations: BTreeMap::from([(
                "item:lamp".into(),
                RuntimeItemLocation::Carried {
                    character_id: keeper_character.id,
                },
            )]),
            npcs: BTreeMap::new(),
            adventure: AdventureRuntimeSnapshot::default(),
        };
        let event = DomainEvent::CharacterMoved {
            actor_id: keeper_character.id,
            actor_name: keeper_character.name.clone(),
            from_room_id: "room:start".into(),
            to_room_id: "room:north".into(),
            direction: Direction::North,
        };
        store
            .commit_action(
                vec![DurableCharacterSnapshot {
                    character: updated_character.clone(),
                    gameplay: gameplay.clone(),
                }],
                runtime.clone(),
                vec![event.clone()],
            )
            .await
            .unwrap();
        assert_eq!(
            store
                .load_character_gameplay(keeper_character.id)
                .await
                .unwrap(),
            gameplay
        );
        assert_eq!(
            store.load_runtime_snapshot().await.unwrap(),
            Some(runtime.clone())
        );
        assert_eq!(
            store.load_domain_events().await.unwrap(),
            vec![event.clone()]
        );
        assert_eq!(
            admin_count(
                &base_url,
                format!(
                    "SELECT last_event_sequence FROM {schema}.world_runtime WHERE singleton = 1"
                ),
            )
            .await,
            1
        );
        assert_eq!(
            admin_count(
                &base_url,
                format!("SELECT COUNT(*)::BIGINT FROM {schema}.character_adventure"),
            )
            .await,
            1
        );
        assert_eq!(
            admin_count(
                &base_url,
                format!("SELECT COUNT(*)::BIGINT FROM {schema}.world_adventure"),
            )
            .await,
            1
        );
        assert_eq!(
            store
                .load_character(keeper.account.id, keeper_character.id)
                .await
                .unwrap(),
            updated_character
        );

        assert_eq!(
            store
                .social_profile(keeper_character.id)
                .await
                .unwrap()
                .role,
            AccountRole::Admin
        );
        store
            .set_social_relation(
                player_character.id,
                keeper_character.id,
                SocialRelationKind::Ignore,
                true,
            )
            .await
            .unwrap();
        assert!(
            !store
                .social_delivery_allowed(keeper_character.id, player_character.id)
                .await
                .unwrap()
        );
        store
            .moderate_character(
                keeper_character.id,
                player_character.id,
                ModerationAction::Mute {
                    expires_at: None,
                    reason: "integration test".into(),
                },
            )
            .await
            .unwrap();
        assert!(
            store
                .social_profile(player_character.id)
                .await
                .unwrap()
                .muted
        );
        store
            .report_character(player_character.id, keeper_character.id, "test report")
            .await
            .unwrap();
        assert!(store.audit_log().await.unwrap().len() >= 4);

        admin_batch(
            &base_url,
            format!(
                "INSERT INTO {schema}.domain_events (event_json)
                 SELECT '{{\"type\":\"retained-test\"}}'
                 FROM generate_series(1, {DOMAIN_EVENT_RETENTION})"
            ),
        )
        .await;
        store
            .commit_action(Vec::new(), runtime.clone(), vec![event.clone()])
            .await
            .unwrap();
        assert_eq!(
            admin_count(
                &base_url,
                format!("SELECT COUNT(*)::BIGINT FROM {schema}.domain_events"),
            )
            .await,
            DOMAIN_EVENT_RETENTION
        );
        let compacted_watermark = DOMAIN_EVENT_RETENTION + 2;
        assert_eq!(
            admin_count(
                &base_url,
                format!(
                    "SELECT last_event_sequence FROM {schema}.world_runtime WHERE singleton = 1"
                ),
            )
            .await,
            compacted_watermark
        );

        admin_batch(
            &base_url,
            format!(
                "CREATE FUNCTION {schema}.reject_domain_event_compaction()
                 RETURNS trigger LANGUAGE plpgsql AS $$
                 BEGIN
                     RAISE EXCEPTION 'injected compaction failure';
                 END $$;
                 CREATE TRIGGER reject_domain_event_compaction
                 BEFORE DELETE ON {schema}.domain_events
                 FOR EACH STATEMENT
                 EXECUTE FUNCTION {schema}.reject_domain_event_compaction();"
            ),
        )
        .await;
        let mut rejected_runtime = runtime.clone();
        rejected_runtime.logical_tick += 1;
        assert!(
            store
                .commit_action(Vec::new(), rejected_runtime, vec![event])
                .await
                .is_err()
        );
        assert_eq!(
            admin_count(
                &base_url,
                format!("SELECT COUNT(*)::BIGINT FROM {schema}.domain_events"),
            )
            .await,
            DOMAIN_EVENT_RETENTION
        );
        assert_eq!(
            admin_count(
                &base_url,
                format!(
                    "SELECT last_event_sequence FROM {schema}.world_runtime WHERE singleton = 1"
                ),
            )
            .await,
            compacted_watermark
        );
        admin_batch(
            &base_url,
            format!(
                "DROP TRIGGER reject_domain_event_compaction ON {schema}.domain_events;
                 DROP FUNCTION {schema}.reject_domain_event_compaction();"
            ),
        )
        .await;

        store.revoke_session(login.token.as_str()).await.unwrap();
        assert!(matches!(
            store.resolve_session(login.token.as_str()).await,
            Err(StoreError::SessionRevoked)
        ));

        drop(store);
        tokio::time::sleep(Duration::from_millis(25)).await;
        admin_batch(&base_url, format!("DROP SCHEMA {schema} CASCADE")).await;
    }
}
