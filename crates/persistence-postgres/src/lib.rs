//! Transactional PostgreSQL schema management and an explicit, offline
//! SQLite-to-PostgreSQL transfer for Tiamud.
//!
//! This crate deliberately does not implement dual-write. The transfer reads
//! one consistent SQLite snapshot and commits all destination rows in one
//! PostgreSQL transaction, and it refuses to merge into a non-empty target.

mod transfer;

use postgres::{Client, GenericClient};
use rustls::{ClientConfig, RootCertStore};
use std::sync::Arc;
use thiserror::Error;
use tokio_postgres_rustls::MakeRustlsConnect;

pub use transfer::{CopyReport, TableCount, TransferError, copy_sqlite_to_postgres};

#[derive(Debug, Error)]
pub enum ConnectError {
    #[error("the operating system did not provide any usable TLS root certificates")]
    NoTlsRoots,
    #[error("could not add an operating-system TLS root certificate: {0}")]
    TlsRoot(#[source] rustls::Error),
    #[error("could not connect to PostgreSQL: {0}")]
    Postgres(#[from] postgres::Error),
}

/// Connect using Rustls and the operating system trust store. `sslmode` in the
/// PostgreSQL URL remains authoritative, including explicit local plaintext.
pub fn connect(database_url: &str) -> Result<Client, ConnectError> {
    let mut roots = RootCertStore::empty();
    let native_roots = rustls_native_certs::load_native_certs();
    for certificate in native_roots.certs {
        roots.add(certificate).map_err(ConnectError::TlsRoot)?;
    }
    if roots.is_empty() {
        return Err(ConnectError::NoTlsRoots);
    }
    let tls = MakeRustlsConnect::new(
        ClientConfig::builder_with_provider(Arc::new(rustls::crypto::ring::default_provider()))
            .with_protocol_versions(&[&rustls::version::TLS13, &rustls::version::TLS12])
            .expect("the selected Rustls provider must support TLS 1.2 and TLS 1.3")
            .with_root_certificates(roots)
            .with_no_client_auth(),
    );
    Client::connect(database_url, tls).map_err(ConnectError::from)
}

/// Current PostgreSQL persistence contract, kept equal to SQLite.
pub const SCHEMA_VERSION: i32 = 9;

const MIGRATION_LOCK_ID: i64 = 0x5449_414d_5544_0007;
pub(crate) const APPLICATION_TABLES: [&str; 14] = [
    "accounts",
    "characters",
    "auth_sessions",
    "domain_events",
    "world_runtime",
    "social_ignores",
    "social_blocks",
    "social_channel_subscriptions",
    "moderation_mutes",
    "account_bans",
    "social_reports",
    "moderation_audit",
    "character_adventure",
    "world_adventure",
];

/// Complete PostgreSQL representation of the current SQLite schema.
///
/// Text payload constraints use `octet_length`, matching the byte limits
/// enforced by the server before persistence.
pub const SCHEMA_SQL: &str = r#"
CREATE TABLE tiamud_schema (
    singleton SMALLINT PRIMARY KEY CHECK (singleton = 1),
    version INTEGER NOT NULL CHECK (version >= 0),
    updated_at BIGINT NOT NULL DEFAULT (floor(extract(epoch FROM now()))::BIGINT)
);

CREATE TABLE accounts (
    id BIGSERIAL PRIMARY KEY,
    name TEXT NOT NULL CHECK (octet_length(name) BETWEEN 1 AND 64),
    created_at BIGINT NOT NULL DEFAULT (floor(extract(epoch FROM now()))::BIGINT),
    password_hash TEXT,
    auth_state TEXT NOT NULL DEFAULT 'guest'
        CHECK (auth_state IN ('guest', 'registered')),
    role TEXT NOT NULL DEFAULT 'player'
        CHECK (role IN ('player', 'moderator', 'admin'))
);
CREATE UNIQUE INDEX accounts_name_ci_idx ON accounts (lower(name));

CREATE TABLE characters (
    id BIGSERIAL PRIMARY KEY,
    account_id BIGINT NOT NULL REFERENCES accounts(id) ON DELETE CASCADE,
    name TEXT NOT NULL CHECK (octet_length(name) BETWEEN 1 AND 64),
    room_id TEXT NOT NULL,
    created_at BIGINT NOT NULL DEFAULT (floor(extract(epoch FROM now()))::BIGINT),
    updated_at BIGINT NOT NULL DEFAULT (floor(extract(epoch FROM now()))::BIGINT),
    item_locations_json TEXT NOT NULL DEFAULT '{}',
    gameplay_json TEXT NOT NULL
        DEFAULT '{"level":1,"experience":0,"vitals":{"health":20,"maxHealth":20},"attributes":{"strength":5,"defence":2},"equipment":{},"effects":[],"recoveryAtTick":null}'
        CHECK (octet_length(gameplay_json) <= 65536)
);
CREATE UNIQUE INDEX characters_account_name_ci_idx
    ON characters (account_id, lower(name));
CREATE INDEX characters_account_id_idx ON characters (account_id, id);

CREATE TABLE auth_sessions (
    token_hash BYTEA PRIMARY KEY CHECK (octet_length(token_hash) = 32),
    account_id BIGINT NOT NULL REFERENCES accounts(id) ON DELETE CASCADE,
    created_at BIGINT NOT NULL,
    expires_at BIGINT NOT NULL,
    revoked_at BIGINT,
    CHECK (expires_at > created_at)
);
CREATE INDEX auth_sessions_account_idx
    ON auth_sessions (account_id, expires_at);

CREATE TABLE domain_events (
    sequence BIGSERIAL PRIMARY KEY,
    event_json TEXT NOT NULL CHECK (octet_length(event_json) <= 65536),
    created_at BIGINT NOT NULL DEFAULT (floor(extract(epoch FROM now()))::BIGINT)
);

CREATE TABLE world_runtime (
    singleton BIGINT PRIMARY KEY CHECK (singleton = 1),
    snapshot_version BIGINT NOT NULL,
    snapshot_json TEXT NOT NULL CHECK (octet_length(snapshot_json) <= 1048576),
    last_event_sequence BIGINT
        CHECK (last_event_sequence IS NULL OR last_event_sequence >= 0),
    updated_at BIGINT NOT NULL DEFAULT (floor(extract(epoch FROM now()))::BIGINT)
);

CREATE TABLE social_ignores (
    owner_character_id BIGINT NOT NULL REFERENCES characters(id) ON DELETE CASCADE,
    target_character_id BIGINT NOT NULL REFERENCES characters(id) ON DELETE CASCADE,
    created_at BIGINT NOT NULL DEFAULT (floor(extract(epoch FROM now()))::BIGINT),
    PRIMARY KEY (owner_character_id, target_character_id),
    CHECK (owner_character_id <> target_character_id)
);

CREATE TABLE social_blocks (
    owner_character_id BIGINT NOT NULL REFERENCES characters(id) ON DELETE CASCADE,
    target_character_id BIGINT NOT NULL REFERENCES characters(id) ON DELETE CASCADE,
    created_at BIGINT NOT NULL DEFAULT (floor(extract(epoch FROM now()))::BIGINT),
    PRIMARY KEY (owner_character_id, target_character_id),
    CHECK (owner_character_id <> target_character_id)
);

CREATE TABLE social_channel_subscriptions (
    character_id BIGINT NOT NULL REFERENCES characters(id) ON DELETE CASCADE,
    channel TEXT NOT NULL CHECK (channel IN ('general', 'newbie', 'staff')),
    created_at BIGINT NOT NULL DEFAULT (floor(extract(epoch FROM now()))::BIGINT),
    PRIMARY KEY (character_id, channel)
);
CREATE INDEX social_channel_subscriptions_channel_idx
    ON social_channel_subscriptions (channel, character_id);

CREATE TABLE moderation_mutes (
    id BIGSERIAL PRIMARY KEY,
    account_id BIGINT NOT NULL REFERENCES accounts(id) ON DELETE CASCADE,
    imposed_by_account_id BIGINT NOT NULL REFERENCES accounts(id),
    reason TEXT NOT NULL CHECK (octet_length(reason) BETWEEN 1 AND 1024),
    created_at BIGINT NOT NULL,
    expires_at BIGINT,
    revoked_at BIGINT,
    revoked_by_account_id BIGINT REFERENCES accounts(id),
    CHECK (expires_at IS NULL OR expires_at > created_at)
);
CREATE INDEX moderation_mutes_active_idx
    ON moderation_mutes (account_id, revoked_at, expires_at);

CREATE TABLE account_bans (
    id BIGSERIAL PRIMARY KEY,
    account_id BIGINT NOT NULL REFERENCES accounts(id) ON DELETE CASCADE,
    imposed_by_account_id BIGINT NOT NULL REFERENCES accounts(id),
    reason TEXT NOT NULL CHECK (octet_length(reason) BETWEEN 1 AND 1024),
    created_at BIGINT NOT NULL,
    expires_at BIGINT,
    revoked_at BIGINT,
    revoked_by_account_id BIGINT REFERENCES accounts(id),
    CHECK (expires_at IS NULL OR expires_at > created_at)
);
CREATE INDEX account_bans_active_idx
    ON account_bans (account_id, revoked_at, expires_at);

CREATE TABLE social_reports (
    id BIGSERIAL PRIMARY KEY,
    reporter_character_id BIGINT NOT NULL REFERENCES characters(id) ON DELETE CASCADE,
    target_character_id BIGINT NOT NULL REFERENCES characters(id) ON DELETE CASCADE,
    reason TEXT NOT NULL CHECK (octet_length(reason) BETWEEN 1 AND 2048),
    created_at BIGINT NOT NULL,
    status TEXT NOT NULL DEFAULT 'open'
        CHECK (status IN ('open', 'reviewing', 'closed')),
    CHECK (reporter_character_id <> target_character_id)
);
CREATE INDEX social_reports_status_idx
    ON social_reports (status, created_at);

CREATE TABLE moderation_audit (
    sequence BIGSERIAL PRIMARY KEY,
    actor_account_id BIGINT NOT NULL REFERENCES accounts(id),
    target_account_id BIGINT REFERENCES accounts(id),
    action TEXT NOT NULL CHECK (octet_length(action) BETWEEN 1 AND 64),
    details_json TEXT NOT NULL CHECK (octet_length(details_json) <= 4096),
    created_at BIGINT NOT NULL
);
CREATE INDEX moderation_audit_actor_idx
    ON moderation_audit (actor_account_id, sequence);

CREATE TABLE character_adventure (
    character_id BIGINT PRIMARY KEY REFERENCES characters(id) ON DELETE CASCADE,
    state_json TEXT NOT NULL CHECK (octet_length(state_json) <= 65536),
    updated_at BIGINT NOT NULL DEFAULT (floor(extract(epoch FROM now()))::BIGINT)
);

CREATE TABLE world_adventure (
    singleton BIGINT PRIMARY KEY CHECK (singleton = 1),
    state_json TEXT NOT NULL CHECK (octet_length(state_json) <= 1048576),
    updated_at BIGINT NOT NULL DEFAULT (floor(extract(epoch FROM now()))::BIGINT)
);

INSERT INTO tiamud_schema (singleton, version) VALUES (1, 9);
"#;

const UPGRADE_V6_TO_V7_SQL: &str = r#"
CREATE TABLE character_adventure (
    character_id BIGINT PRIMARY KEY REFERENCES characters(id) ON DELETE CASCADE,
    state_json TEXT NOT NULL CHECK (octet_length(state_json) <= 65536),
    updated_at BIGINT NOT NULL DEFAULT (floor(extract(epoch FROM now()))::BIGINT)
);
CREATE TABLE world_adventure (
    singleton BIGINT PRIMARY KEY CHECK (singleton = 1),
    state_json TEXT NOT NULL CHECK (octet_length(state_json) <= 1048576),
    updated_at BIGINT NOT NULL DEFAULT (floor(extract(epoch FROM now()))::BIGINT)
);
UPDATE tiamud_schema
SET version = 7, updated_at = floor(extract(epoch FROM now()))::BIGINT
WHERE singleton = 1 AND version = 6;
"#;

const UPGRADE_V7_TO_V8_SQL: &str = r#"
ALTER TABLE world_runtime ADD COLUMN last_event_sequence BIGINT
    CHECK (last_event_sequence IS NULL OR last_event_sequence >= 0);
UPDATE tiamud_schema
SET version = 8, updated_at = floor(extract(epoch FROM now()))::BIGINT
WHERE singleton = 1 AND version = 7;
"#;

const UPGRADE_V8_TO_V9_SQL: &str = r#"
CREATE TABLE social_channel_subscriptions (
    character_id BIGINT NOT NULL REFERENCES characters(id) ON DELETE CASCADE,
    channel TEXT NOT NULL CHECK (channel IN ('general', 'newbie', 'staff')),
    created_at BIGINT NOT NULL DEFAULT (floor(extract(epoch FROM now()))::BIGINT),
    PRIMARY KEY (character_id, channel)
);
CREATE INDEX social_channel_subscriptions_channel_idx
    ON social_channel_subscriptions (channel, character_id);
UPDATE tiamud_schema
SET version = 9, updated_at = floor(extract(epoch FROM now()))::BIGINT
WHERE singleton = 1 AND version = 8;
"#;

const ENSURE_BOOTSTRAP_ADMIN_SQL: &str = r#"
WITH candidate AS (
    SELECT id FROM accounts
    WHERE auth_state = 'registered'
      AND NOT EXISTS (SELECT 1 FROM accounts WHERE role = 'admin')
    ORDER BY id LIMIT 1
), promoted AS (
    UPDATE accounts
    SET role = 'admin'
    FROM candidate
    WHERE accounts.id = candidate.id
    RETURNING accounts.id
)
INSERT INTO moderation_audit
    (actor_account_id, target_account_id, action, details_json, created_at)
SELECT id, id, 'bootstrap_admin',
       '{"source":"postgres_schema_bootstrap"}',
       floor(extract(epoch FROM now()))::BIGINT
FROM promoted;
"#;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct MigrationReport {
    pub from: i32,
    pub to: i32,
}

#[derive(Debug, Error)]
pub enum SchemaError {
    #[error(transparent)]
    Postgres(#[from] postgres::Error),
    #[error(
        "the PostgreSQL schema contains Tiamud tables but no tiamud_schema metadata; refusing to adopt it"
    )]
    UnmanagedSchema,
    #[error("the PostgreSQL tiamud_schema metadata is missing or malformed")]
    InvalidMetadata,
    #[error(
        "PostgreSQL schema version {found} is unsupported; this backend supports a new v9 schema or migration from v6/v7/v8"
    )]
    UnsupportedVersion { found: i32 },
    #[error("PostgreSQL schema metadata says v9 but required table {table} is missing")]
    MissingTable { table: &'static str },
}

/// Install or verify the current schema in one PostgreSQL transaction.
///
/// PostgreSQL persistence starts at v6, so an empty destination is installed
/// at the current version, v6/v7/v8 are upgraded to v9, and v9 is verified.
pub fn migrate(client: &mut Client) -> Result<MigrationReport, SchemaError> {
    let mut transaction = client.transaction()?;
    let report = migrate_in_transaction(&mut transaction)?;
    transaction.commit()?;
    Ok(report)
}

pub(crate) fn migrate_in_transaction<C: GenericClient>(
    client: &mut C,
) -> Result<MigrationReport, SchemaError> {
    client.query_one("SELECT pg_advisory_xact_lock($1)", &[&MIGRATION_LOCK_ID])?;

    let metadata_exists: bool = client
        .query_one("SELECT to_regclass('tiamud_schema') IS NOT NULL", &[])?
        .get(0);

    if !metadata_exists {
        if any_application_table_exists(client)? {
            return Err(SchemaError::UnmanagedSchema);
        }
        client.batch_execute(SCHEMA_SQL)?;
        return Ok(MigrationReport {
            from: 0,
            to: SCHEMA_VERSION,
        });
    }

    let rows = client.query("SELECT version FROM tiamud_schema WHERE singleton = 1", &[])?;
    if rows.len() != 1 {
        return Err(SchemaError::InvalidMetadata);
    }
    let found: i32 = rows[0].get(0);
    match found {
        6 => {
            client.batch_execute(UPGRADE_V6_TO_V7_SQL)?;
            client.batch_execute(UPGRADE_V7_TO_V8_SQL)?;
            client.batch_execute(UPGRADE_V8_TO_V9_SQL)?;
        }
        7 => {
            client.batch_execute(UPGRADE_V7_TO_V8_SQL)?;
            client.batch_execute(UPGRADE_V8_TO_V9_SQL)?;
        }
        8 => client.batch_execute(UPGRADE_V8_TO_V9_SQL)?,
        SCHEMA_VERSION => {}
        _ => return Err(SchemaError::UnsupportedVersion { found }),
    }
    for &table in &APPLICATION_TABLES {
        let exists: bool = client
            .query_one("SELECT to_regclass($1::text) IS NOT NULL", &[&table])?
            .get(0);
        if !exists {
            return Err(SchemaError::MissingTable { table });
        }
    }
    // The automatic first-admin promotion belongs to the v6 role migration.
    // Re-running it for an already-current schema would silently undo an
    // operator's deliberate no-admin state after a restart.
    if found == 6 {
        client.batch_execute(ENSURE_BOOTSTRAP_ADMIN_SQL)?;
    }

    Ok(MigrationReport {
        from: found,
        to: SCHEMA_VERSION,
    })
}

fn any_application_table_exists<C: GenericClient>(client: &mut C) -> Result<bool, postgres::Error> {
    Ok(client
        .query_one(
            "SELECT EXISTS (
                 SELECT 1
                 FROM unnest($1::text[]) AS candidate(name)
                 WHERE to_regclass(candidate.name) IS NOT NULL
             )",
            &[&APPLICATION_TABLES.as_slice()],
        )?
        .get(0))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn schema_has_every_current_table_and_metadata() {
        for table in [
            "accounts",
            "characters",
            "auth_sessions",
            "domain_events",
            "world_runtime",
            "social_ignores",
            "social_blocks",
            "social_channel_subscriptions",
            "moderation_mutes",
            "account_bans",
            "social_reports",
            "moderation_audit",
            "character_adventure",
            "world_adventure",
        ] {
            assert!(
                SCHEMA_SQL.contains(&format!("CREATE TABLE {table}")),
                "missing {table}"
            );
        }
        assert!(SCHEMA_SQL.contains("VALUES (1, 9)"));
        assert!(SCHEMA_SQL.contains("last_event_sequence BIGINT"));
    }

    #[test]
    fn schema_keeps_security_relevant_limits_and_case_insensitive_identity() {
        assert!(SCHEMA_SQL.contains("octet_length(token_hash) = 32"));
        assert!(SCHEMA_SQL.contains("octet_length(gameplay_json) <= 65536"));
        assert!(SCHEMA_SQL.contains("octet_length(snapshot_json) <= 1048576"));
        assert!(SCHEMA_SQL.contains("octet_length(details_json) <= 4096"));
        assert!(SCHEMA_SQL.contains("accounts (lower(name))"));
        assert!(SCHEMA_SQL.contains("characters (account_id, lower(name))"));
    }

    #[test]
    fn v8_upgrade_adds_an_unknown_watermark_without_compacting_history() {
        assert!(UPGRADE_V7_TO_V8_SQL.contains("ADD COLUMN last_event_sequence"));
        assert!(!UPGRADE_V7_TO_V8_SQL.contains("DELETE FROM domain_events"));
        assert!(!UPGRADE_V7_TO_V8_SQL.contains("NOT NULL"));
    }

    #[test]
    fn v9_upgrade_adds_only_the_closed_channel_subscription_contract() {
        assert!(UPGRADE_V8_TO_V9_SQL.contains("CREATE TABLE social_channel_subscriptions"));
        assert!(UPGRADE_V8_TO_V9_SQL.contains("'general', 'newbie', 'staff'"));
        assert!(UPGRADE_V8_TO_V9_SQL.contains("PRIMARY KEY (character_id, channel)"));
        assert!(UPGRADE_V8_TO_V9_SQL.contains("SET version = 9"));
    }

    #[test]
    fn postgres_contract_tracks_the_sqlite_source_contract() {
        assert_eq!(
            i64::from(SCHEMA_VERSION),
            tiamud_persistence_sqlite::SCHEMA_VERSION
        );
    }
}
