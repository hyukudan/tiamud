use argon2::{
    Algorithm, Argon2, Params, Version,
    password_hash::{PasswordHash, PasswordHasher, PasswordVerifier, SaltString},
};
use rand_core::{OsRng, RngCore};
use serde::Serialize;
use sha2::{Digest, Sha256};
use std::{fmt, time::Duration};
use tiamud_core::AccountSnapshot;

pub const MIN_PASSWORD_BYTES: usize = 8;
pub const MAX_PASSWORD_BYTES: usize = 1_024;
pub const SESSION_TOKEN_BYTES: usize = 32;
pub const DEFAULT_SESSION_TTL: Duration = Duration::from_secs(30 * 24 * 60 * 60);

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
}
