//! Transactional PostgreSQL schema management and an explicit, offline
//! SQLite-to-PostgreSQL transfer for Tiamud.
//!
//! This crate deliberately does not implement dual-write. The transfer reads
//! one consistent SQLite snapshot and commits all destination rows in one
//! PostgreSQL transaction, and it refuses to merge into a non-empty target.

pub mod runtime_sections;
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
    #[error("the selected Rustls provider does not support TLS 1.2 and TLS 1.3: {0}")]
    TlsProtocolVersions(#[source] rustls::Error),
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
            .map_err(ConnectError::TlsProtocolVersions)?
            .with_root_certificates(roots)
            .with_no_client_auth(),
    );
    Client::connect(database_url, tls).map_err(ConnectError::from)
}

/// Current PostgreSQL persistence contract, kept equal to SQLite.
pub const SCHEMA_VERSION: i32 = 17;

const MIGRATION_LOCK_ID: i64 = 0x5449_414d_5544_0007;
pub(crate) const APPLICATION_TABLES: [&str; 27] = [
    "accounts",
    "characters",
    "auth_sessions",
    "domain_events",
    "world_runtime",
    "world_area_generation",
    "world_area_checkpoints",
    "social_ignores",
    "social_blocks",
    "social_channel_subscriptions",
    "social_mail_usage",
    "social_board_usage",
    "moderation_mutes",
    "account_bans",
    "social_reports",
    "social_mail_messages",
    "social_mail_report_evidence",
    "social_board_posts",
    "social_board_report_evidence",
    "moderation_audit",
    "staff_profiles",
    "staff_capability_assignments",
    "staff_audit",
    "character_adventure",
    "world_adventure",
    "story_saves",
    "world_runtime_sections",
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
    head_json TEXT NOT NULL CHECK (octet_length(head_json) <= 2097152),
    manifest_json TEXT NOT NULL
        CHECK (octet_length(manifest_json) BETWEEN 2 AND 8192),
    last_event_sequence BIGINT
        CHECK (last_event_sequence IS NULL OR last_event_sequence >= 0),
    updated_at BIGINT NOT NULL DEFAULT (floor(extract(epoch FROM now()))::BIGINT)
);

CREATE TABLE world_runtime_sections (
    section TEXT PRIMARY KEY CHECK (section IN (
        'itemInstances', 'itemProvenance', 'npcs', 'adventure',
        'progressionFlags', 'trades', 'lootEncounters', 'groundLoot',
        'partyLootCursors', 'areaResidency'
    )),
    revision BIGINT NOT NULL CHECK (revision > 0),
    section_json TEXT NOT NULL
        CHECK (octet_length(section_json) BETWEEN 1 AND 2097152),
    section_sha256 TEXT NOT NULL CHECK (section_sha256 ~ '^[0-9a-f]{64}$'),
    updated_at BIGINT NOT NULL DEFAULT (floor(extract(epoch FROM now()))::BIGINT)
);

CREATE TABLE world_area_generation (
    singleton BIGINT PRIMARY KEY CHECK (singleton = 1),
    world_id TEXT NOT NULL CHECK (octet_length(world_id) BETWEEN 1 AND 256),
    active_catalog_sha256 TEXT NOT NULL
        CHECK (active_catalog_sha256 ~ '^[0-9a-f]{64}$'),
    previous_catalog_sha256 TEXT
        CHECK (previous_catalog_sha256 IS NULL
               OR previous_catalog_sha256 ~ '^[0-9a-f]{64}$'),
    activation_sequence BIGINT NOT NULL CHECK (activation_sequence > 0),
    updated_at BIGINT NOT NULL DEFAULT (floor(extract(epoch FROM now()))::BIGINT),
    CHECK (previous_catalog_sha256 IS NULL
           OR previous_catalog_sha256 <> active_catalog_sha256)
);

CREATE TABLE world_area_checkpoints (
    generation_sha256 TEXT NOT NULL
        CHECK (generation_sha256 ~ '^[0-9a-f]{64}$'),
    unit_id TEXT NOT NULL CHECK (unit_id ~ '^[0-9a-f]{64}$'),
    residency_epoch BIGINT NOT NULL CHECK (residency_epoch > 0),
    checkpoint_version BIGINT NOT NULL CHECK (checkpoint_version > 0),
    checkpoint_sha256 TEXT NOT NULL
        CHECK (checkpoint_sha256 ~ '^[0-9a-f]{64}$'),
    captured_logical_tick BIGINT NOT NULL CHECK (captured_logical_tick >= 0),
    checkpoint_json TEXT NOT NULL
        CHECK (octet_length(checkpoint_json) BETWEEN 1 AND 2097152),
    created_at BIGINT NOT NULL DEFAULT (floor(extract(epoch FROM now()))::BIGINT),
    updated_at BIGINT NOT NULL DEFAULT (floor(extract(epoch FROM now()))::BIGINT),
    PRIMARY KEY (generation_sha256, unit_id, residency_epoch),
    CHECK (updated_at >= created_at)
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

CREATE TABLE social_mail_usage (
    account_id BIGINT PRIMARY KEY REFERENCES accounts(id) ON DELETE CASCADE,
    hour_started_at BIGINT NOT NULL,
    hour_count INTEGER NOT NULL CHECK (hour_count BETWEEN 0 AND 20),
    day_started_at BIGINT NOT NULL,
    day_count INTEGER NOT NULL CHECK (day_count BETWEEN 0 AND 50),
    updated_at BIGINT NOT NULL,
    CHECK (updated_at >= hour_started_at AND updated_at >= day_started_at)
);

CREATE TABLE social_board_usage (
    account_id BIGINT PRIMARY KEY REFERENCES accounts(id) ON DELETE CASCADE,
    hour_started_at BIGINT NOT NULL,
    hour_count INTEGER NOT NULL CHECK (hour_count BETWEEN 0 AND 5),
    day_started_at BIGINT NOT NULL,
    day_count INTEGER NOT NULL CHECK (day_count BETWEEN 0 AND 20),
    updated_at BIGINT NOT NULL,
    CHECK (updated_at >= hour_started_at AND updated_at >= day_started_at)
);

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
    report_kind TEXT NOT NULL DEFAULT 'character'
        CONSTRAINT social_reports_report_kind_check
        CHECK (report_kind IN ('character', 'mail', 'board')),
    reason TEXT NOT NULL CHECK (octet_length(reason) BETWEEN 1 AND 2048),
    created_at BIGINT NOT NULL,
    status TEXT NOT NULL DEFAULT 'open'
        CHECK (status IN ('open', 'resolved')),
    resolution TEXT
        CHECK (resolution IS NULL OR resolution IN ('dismissed', 'action_taken', 'no_action')),
    resolution_reason TEXT
        CHECK (resolution_reason IS NULL OR octet_length(resolution_reason) BETWEEN 1 AND 2048),
    resolved_by_account_id BIGINT REFERENCES accounts(id),
    resolved_at BIGINT,
    CHECK (
        (status = 'open'
         AND resolution IS NULL
         AND resolution_reason IS NULL
         AND resolved_by_account_id IS NULL
         AND resolved_at IS NULL)
        OR
        (status = 'resolved'
         AND resolution IS NOT NULL
         AND resolution_reason IS NOT NULL
         AND resolved_by_account_id IS NOT NULL
         AND resolved_at IS NOT NULL
         AND resolved_at >= created_at)
    ),
    CHECK (reporter_character_id <> target_character_id)
);
CREATE INDEX social_reports_status_idx
    ON social_reports (status, created_at);
CREATE INDEX social_reports_mail_open_idx
    ON social_reports (report_kind, status, reporter_character_id);

CREATE TABLE social_mail_messages (
    id BIGSERIAL PRIMARY KEY,
    public_id TEXT NOT NULL UNIQUE
        CHECK (public_id ~ '^m[0-9a-f]{32}$'),
    sender_character_id BIGINT REFERENCES characters(id) ON DELETE SET NULL,
    sender_name TEXT NOT NULL
        CHECK (octet_length(sender_name) BETWEEN 1 AND 64),
    recipient_character_id BIGINT NOT NULL
        REFERENCES characters(id) ON DELETE CASCADE,
    subject TEXT NOT NULL CHECK (octet_length(subject) BETWEEN 1 AND 128),
    body TEXT NOT NULL CHECK (octet_length(body) BETWEEN 1 AND 2048),
    created_at BIGINT NOT NULL,
    expires_at BIGINT NOT NULL,
    read_at BIGINT,
    report_id BIGINT UNIQUE REFERENCES social_reports(id) ON DELETE SET NULL,
    CHECK (expires_at = created_at + 2592000),
    CHECK (read_at IS NULL OR read_at BETWEEN created_at AND expires_at),
    CHECK (sender_character_id IS NULL OR sender_character_id <> recipient_character_id)
);
CREATE INDEX social_mail_messages_recipient_idx
    ON social_mail_messages (recipient_character_id, created_at DESC, id DESC);
CREATE INDEX social_mail_messages_expires_idx
    ON social_mail_messages (expires_at, id);

CREATE TABLE social_mail_report_evidence (
    report_id BIGINT PRIMARY KEY REFERENCES social_reports(id) ON DELETE CASCADE,
    sender_name TEXT NOT NULL
        CHECK (octet_length(sender_name) BETWEEN 1 AND 64),
    subject TEXT NOT NULL CHECK (octet_length(subject) BETWEEN 1 AND 128),
    body TEXT NOT NULL CHECK (octet_length(body) BETWEEN 1 AND 2048),
    sent_at BIGINT NOT NULL,
    created_at BIGINT NOT NULL,
    expires_at BIGINT NOT NULL,
    CHECK (created_at >= sent_at),
    CHECK (expires_at = created_at + 7776000)
);
CREATE INDEX social_mail_report_evidence_expires_idx
    ON social_mail_report_evidence (expires_at, report_id);

CREATE TABLE social_board_posts (
    id BIGSERIAL PRIMARY KEY,
    public_id TEXT NOT NULL UNIQUE
        CHECK (public_id ~ '^b[0-9a-f]{32}$'),
    board_key TEXT NOT NULL CHECK (board_key IN ('general', 'newbie')),
    author_character_id BIGINT NOT NULL
        REFERENCES characters(id) ON DELETE CASCADE,
    author_name TEXT NOT NULL
        CHECK (octet_length(author_name) BETWEEN 1 AND 64),
    subject TEXT NOT NULL
        CHECK (octet_length(subject) BETWEEN 1 AND 128 AND position('|' IN subject) = 0),
    body TEXT NOT NULL CHECK (octet_length(body) BETWEEN 1 AND 4096),
    created_at BIGINT NOT NULL,
    expires_at BIGINT NOT NULL,
    report_id BIGINT UNIQUE REFERENCES social_reports(id) ON DELETE SET NULL,
    CHECK (expires_at = created_at + 7776000)
);
CREATE INDEX social_board_posts_board_idx
    ON social_board_posts (board_key, created_at DESC, id DESC);
CREATE INDEX social_board_posts_author_idx
    ON social_board_posts (author_character_id, created_at DESC, id DESC);
CREATE INDEX social_board_posts_expires_idx
    ON social_board_posts (expires_at, id);

CREATE TABLE social_board_report_evidence (
    report_id BIGINT PRIMARY KEY REFERENCES social_reports(id) ON DELETE CASCADE,
    board_key TEXT NOT NULL CHECK (board_key IN ('general', 'newbie')),
    post_public_id TEXT NOT NULL CHECK (post_public_id ~ '^b[0-9a-f]{32}$'),
    author_name TEXT NOT NULL
        CHECK (octet_length(author_name) BETWEEN 1 AND 64),
    subject TEXT NOT NULL CHECK (octet_length(subject) BETWEEN 1 AND 128),
    body TEXT NOT NULL CHECK (octet_length(body) BETWEEN 1 AND 4096),
    posted_at BIGINT NOT NULL,
    created_at BIGINT NOT NULL,
    expires_at BIGINT NOT NULL,
    CHECK (created_at >= posted_at),
    CHECK (expires_at = created_at + 7776000)
);
CREATE INDEX social_board_report_evidence_expires_idx
    ON social_board_report_evidence (expires_at, report_id);

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

CREATE TABLE staff_profiles (
    account_id BIGINT PRIMARY KEY REFERENCES accounts(id) ON DELETE CASCADE,
    profile_key TEXT NOT NULL
        CHECK (profile_key IN ('helper', 'builder', 'moderator', 'administrator', 'implementor')),
    lore_title TEXT
        CHECK (lore_title IS NULL OR octet_length(lore_title) BETWEEN 1 AND 128),
    assigned_by_account_id BIGINT NOT NULL REFERENCES accounts(id),
    created_at BIGINT NOT NULL,
    updated_at BIGINT NOT NULL,
    CHECK (updated_at >= created_at)
);
CREATE INDEX staff_profiles_profile_idx
    ON staff_profiles (profile_key, account_id);

CREATE TABLE staff_capability_assignments (
    account_id BIGINT NOT NULL REFERENCES accounts(id) ON DELETE CASCADE,
    capability TEXT NOT NULL CHECK (capability IN (
        'reports.view',
        'reports.resolve',
        'moderation.mute',
        'moderation.ban',
        'staff.assign',
        'staff.inspect',
        'staff.where',
        'staff.goto',
        'staff.transfer',
        'staff.restore',
        'staff.announce',
        'staff.audit'
    )),
    assigned_by_account_id BIGINT NOT NULL REFERENCES accounts(id),
    created_at BIGINT NOT NULL,
    PRIMARY KEY (account_id, capability)
);
CREATE INDEX staff_capabilities_capability_idx
    ON staff_capability_assignments (capability, account_id);

CREATE TABLE staff_audit (
    sequence BIGSERIAL PRIMARY KEY,
    actor_account_id BIGINT NOT NULL REFERENCES accounts(id),
    target_account_id BIGINT REFERENCES accounts(id),
    capability TEXT NOT NULL CHECK (capability IN (
        'reports.view',
        'reports.resolve',
        'moderation.mute',
        'moderation.ban',
        'staff.assign',
        'staff.inspect',
        'staff.where',
        'staff.goto',
        'staff.transfer',
        'staff.restore',
        'staff.announce',
        'staff.audit'
    )),
    action TEXT NOT NULL CHECK (octet_length(action) BETWEEN 1 AND 64),
    reason TEXT NOT NULL CHECK (octet_length(reason) BETWEEN 1 AND 1024),
    details_json TEXT NOT NULL CHECK (octet_length(details_json) <= 4096),
    created_at BIGINT NOT NULL
);
CREATE INDEX staff_audit_actor_idx
    ON staff_audit (actor_account_id, sequence);
CREATE FUNCTION tiamud_reject_staff_audit_mutation() RETURNS trigger
LANGUAGE plpgsql AS $$
BEGIN
    RAISE EXCEPTION 'staff_audit is append-only';
END;
$$;
CREATE TRIGGER staff_audit_no_update
    BEFORE UPDATE ON staff_audit
    FOR EACH ROW EXECUTE FUNCTION tiamud_reject_staff_audit_mutation();
CREATE TRIGGER staff_audit_no_delete
    BEFORE DELETE ON staff_audit
    FOR EACH ROW EXECUTE FUNCTION tiamud_reject_staff_audit_mutation();

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

CREATE TABLE story_saves (
    account_id BIGINT NOT NULL REFERENCES accounts(id) ON DELETE CASCADE,
    story_id TEXT NOT NULL CHECK (story_id ~ '^[a-z0-9][a-z0-9._-]{0,63}$'),
    slot SMALLINT NOT NULL CHECK (slot BETWEEN 0 AND 9),
    pack_id TEXT NOT NULL CHECK (pack_id ~ '^[a-z0-9][a-z0-9._-]{0,63}$'),
    pack_sha256 TEXT NOT NULL CHECK (pack_sha256 ~ '^[0-9a-f]{64}$'),
    state_format_version INTEGER NOT NULL
        CHECK (state_format_version BETWEEN 1 AND 255),
    state_json TEXT NOT NULL
        CHECK (octet_length(state_json) BETWEEN 2 AND 65536),
    state_sha256 TEXT NOT NULL CHECK (state_sha256 ~ '^[0-9a-f]{64}$'),
    created_at BIGINT NOT NULL,
    updated_at BIGINT NOT NULL,
    PRIMARY KEY (account_id, story_id, slot),
    CHECK (updated_at >= created_at)
);

INSERT INTO tiamud_schema (singleton, version) VALUES (1, 17);
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

const UPGRADE_V9_TO_V10_SQL: &str = r#"
ALTER TABLE social_reports
    ADD COLUMN resolution TEXT,
    ADD COLUMN resolution_reason TEXT,
    ADD COLUMN resolved_by_account_id BIGINT REFERENCES accounts(id),
    ADD COLUMN resolved_at BIGINT;
UPDATE social_reports SET status = 'open' WHERE status <> 'open';
ALTER TABLE social_reports
    DROP CONSTRAINT IF EXISTS social_reports_status_check,
    ADD CONSTRAINT social_reports_status_check
        CHECK (status IN ('open', 'resolved')),
    ADD CONSTRAINT social_reports_resolution_check CHECK (
        (status = 'open'
         AND resolution IS NULL
         AND resolution_reason IS NULL
         AND resolved_by_account_id IS NULL
         AND resolved_at IS NULL)
        OR
        (status = 'resolved'
         AND resolution IS NOT NULL
         AND resolution IN ('dismissed', 'action_taken', 'no_action')
         AND resolution_reason IS NOT NULL
         AND octet_length(resolution_reason) BETWEEN 1 AND 2048
         AND resolved_by_account_id IS NOT NULL
         AND resolved_at IS NOT NULL
         AND resolved_at >= created_at)
    );

CREATE TABLE staff_profiles (
    account_id BIGINT PRIMARY KEY REFERENCES accounts(id) ON DELETE CASCADE,
    profile_key TEXT NOT NULL
        CHECK (profile_key IN ('helper', 'builder', 'moderator', 'administrator', 'implementor')),
    lore_title TEXT
        CHECK (lore_title IS NULL OR octet_length(lore_title) BETWEEN 1 AND 128),
    assigned_by_account_id BIGINT NOT NULL REFERENCES accounts(id),
    created_at BIGINT NOT NULL,
    updated_at BIGINT NOT NULL,
    CHECK (updated_at >= created_at)
);
CREATE INDEX staff_profiles_profile_idx
    ON staff_profiles (profile_key, account_id);

CREATE TABLE staff_capability_assignments (
    account_id BIGINT NOT NULL REFERENCES accounts(id) ON DELETE CASCADE,
    capability TEXT NOT NULL CHECK (capability IN (
        'reports.view', 'reports.resolve', 'moderation.mute', 'moderation.ban',
        'staff.assign', 'staff.inspect', 'staff.where', 'staff.goto',
        'staff.transfer', 'staff.restore', 'staff.announce', 'staff.audit'
    )),
    assigned_by_account_id BIGINT NOT NULL REFERENCES accounts(id),
    created_at BIGINT NOT NULL,
    PRIMARY KEY (account_id, capability)
);
CREATE INDEX staff_capabilities_capability_idx
    ON staff_capability_assignments (capability, account_id);

CREATE TABLE staff_audit (
    sequence BIGSERIAL PRIMARY KEY,
    actor_account_id BIGINT NOT NULL REFERENCES accounts(id),
    target_account_id BIGINT REFERENCES accounts(id),
    capability TEXT NOT NULL CHECK (capability IN (
        'reports.view', 'reports.resolve', 'moderation.mute', 'moderation.ban',
        'staff.assign', 'staff.inspect', 'staff.where', 'staff.goto',
        'staff.transfer', 'staff.restore', 'staff.announce', 'staff.audit'
    )),
    action TEXT NOT NULL CHECK (octet_length(action) BETWEEN 1 AND 64),
    reason TEXT NOT NULL CHECK (octet_length(reason) BETWEEN 1 AND 1024),
    details_json TEXT NOT NULL CHECK (octet_length(details_json) <= 4096),
    created_at BIGINT NOT NULL
);
CREATE INDEX staff_audit_actor_idx
    ON staff_audit (actor_account_id, sequence);
CREATE FUNCTION tiamud_reject_staff_audit_mutation() RETURNS trigger
LANGUAGE plpgsql AS $$
BEGIN
    RAISE EXCEPTION 'staff_audit is append-only';
END;
$$;
CREATE TRIGGER staff_audit_no_update
    BEFORE UPDATE ON staff_audit
    FOR EACH ROW EXECUTE FUNCTION tiamud_reject_staff_audit_mutation();
CREATE TRIGGER staff_audit_no_delete
    BEFORE DELETE ON staff_audit
    FOR EACH ROW EXECUTE FUNCTION tiamud_reject_staff_audit_mutation();

INSERT INTO staff_profiles
    (account_id, profile_key, lore_title, assigned_by_account_id, created_at, updated_at)
SELECT id,
       CASE role WHEN 'admin' THEN 'administrator' ELSE 'moderator' END,
       NULL, id, floor(extract(epoch FROM now()))::BIGINT,
       floor(extract(epoch FROM now()))::BIGINT
FROM accounts
WHERE role IN ('moderator', 'admin')
ON CONFLICT (account_id) DO NOTHING;

INSERT INTO staff_capability_assignments
    (account_id, capability, assigned_by_account_id, created_at)
SELECT accounts.id, capabilities.capability, accounts.id,
       floor(extract(epoch FROM now()))::BIGINT
FROM accounts
CROSS JOIN (VALUES
    ('reports.view'),
    ('reports.resolve'),
    ('moderation.mute'),
    ('moderation.ban'),
    ('staff.assign'),
    ('staff.inspect'),
    ('staff.where'),
    ('staff.goto'),
    ('staff.transfer'),
    ('staff.restore'),
    ('staff.announce'),
    ('staff.audit')
) AS capabilities(capability)
WHERE accounts.role = 'admin'
   OR (accounts.role = 'moderator'
       AND capabilities.capability IN (
           'reports.view', 'reports.resolve', 'moderation.mute', 'moderation.ban'
       ))
ON CONFLICT (account_id, capability) DO NOTHING;

UPDATE tiamud_schema
SET version = 10, updated_at = floor(extract(epoch FROM now()))::BIGINT
WHERE singleton = 1 AND version = 9;
"#;

const UPGRADE_V10_TO_V11_SQL: &str = r#"
ALTER TABLE world_runtime
    DROP CONSTRAINT world_runtime_snapshot_json_check,
    ADD CONSTRAINT world_runtime_snapshot_json_check
        CHECK (octet_length(snapshot_json) <= 2097152);
UPDATE tiamud_schema
SET version = 11, updated_at = floor(extract(epoch FROM now()))::BIGINT
WHERE singleton = 1 AND version = 10;
"#;

const UPGRADE_V11_TO_V12_SQL: &str = r#"
ALTER TABLE social_reports
    ADD COLUMN report_kind TEXT NOT NULL DEFAULT 'character'
        CHECK (report_kind IN ('character', 'mail'));
CREATE INDEX social_reports_mail_open_idx
    ON social_reports (report_kind, status, reporter_character_id);

CREATE TABLE social_mail_usage (
    account_id BIGINT PRIMARY KEY REFERENCES accounts(id) ON DELETE CASCADE,
    hour_started_at BIGINT NOT NULL,
    hour_count INTEGER NOT NULL CHECK (hour_count BETWEEN 0 AND 20),
    day_started_at BIGINT NOT NULL,
    day_count INTEGER NOT NULL CHECK (day_count BETWEEN 0 AND 50),
    updated_at BIGINT NOT NULL,
    CHECK (updated_at >= hour_started_at AND updated_at >= day_started_at)
);

CREATE TABLE social_mail_messages (
    id BIGSERIAL PRIMARY KEY,
    public_id TEXT NOT NULL UNIQUE
        CHECK (public_id ~ '^m[0-9a-f]{32}$'),
    sender_character_id BIGINT REFERENCES characters(id) ON DELETE SET NULL,
    sender_name TEXT NOT NULL
        CHECK (octet_length(sender_name) BETWEEN 1 AND 64),
    recipient_character_id BIGINT NOT NULL
        REFERENCES characters(id) ON DELETE CASCADE,
    subject TEXT NOT NULL CHECK (octet_length(subject) BETWEEN 1 AND 128),
    body TEXT NOT NULL CHECK (octet_length(body) BETWEEN 1 AND 2048),
    created_at BIGINT NOT NULL,
    expires_at BIGINT NOT NULL,
    read_at BIGINT,
    report_id BIGINT UNIQUE REFERENCES social_reports(id) ON DELETE SET NULL,
    CHECK (expires_at = created_at + 2592000),
    CHECK (read_at IS NULL OR read_at BETWEEN created_at AND expires_at),
    CHECK (sender_character_id IS NULL OR sender_character_id <> recipient_character_id)
);
CREATE INDEX social_mail_messages_recipient_idx
    ON social_mail_messages (recipient_character_id, created_at DESC, id DESC);
CREATE INDEX social_mail_messages_expires_idx
    ON social_mail_messages (expires_at, id);

CREATE TABLE social_mail_report_evidence (
    report_id BIGINT PRIMARY KEY REFERENCES social_reports(id) ON DELETE CASCADE,
    sender_name TEXT NOT NULL
        CHECK (octet_length(sender_name) BETWEEN 1 AND 64),
    subject TEXT NOT NULL CHECK (octet_length(subject) BETWEEN 1 AND 128),
    body TEXT NOT NULL CHECK (octet_length(body) BETWEEN 1 AND 2048),
    sent_at BIGINT NOT NULL,
    created_at BIGINT NOT NULL,
    expires_at BIGINT NOT NULL,
    CHECK (created_at >= sent_at),
    CHECK (expires_at = created_at + 7776000)
);
CREATE INDEX social_mail_report_evidence_expires_idx
    ON social_mail_report_evidence (expires_at, report_id);

UPDATE tiamud_schema
SET version = 12, updated_at = floor(extract(epoch FROM now()))::BIGINT
WHERE singleton = 1 AND version = 11;
"#;

const UPGRADE_V12_TO_V13_SQL: &str = r#"
ALTER TABLE social_reports
    DROP CONSTRAINT social_reports_report_kind_check,
    ADD CONSTRAINT social_reports_report_kind_check
        CHECK (report_kind IN ('character', 'mail', 'board'));

CREATE TABLE social_board_usage (
    account_id BIGINT PRIMARY KEY REFERENCES accounts(id) ON DELETE CASCADE,
    hour_started_at BIGINT NOT NULL,
    hour_count INTEGER NOT NULL CHECK (hour_count BETWEEN 0 AND 5),
    day_started_at BIGINT NOT NULL,
    day_count INTEGER NOT NULL CHECK (day_count BETWEEN 0 AND 20),
    updated_at BIGINT NOT NULL,
    CHECK (updated_at >= hour_started_at AND updated_at >= day_started_at)
);

CREATE TABLE social_board_posts (
    id BIGSERIAL PRIMARY KEY,
    public_id TEXT NOT NULL UNIQUE
        CHECK (public_id ~ '^b[0-9a-f]{32}$'),
    board_key TEXT NOT NULL CHECK (board_key IN ('general', 'newbie')),
    author_character_id BIGINT NOT NULL
        REFERENCES characters(id) ON DELETE CASCADE,
    author_name TEXT NOT NULL
        CHECK (octet_length(author_name) BETWEEN 1 AND 64),
    subject TEXT NOT NULL
        CHECK (octet_length(subject) BETWEEN 1 AND 128 AND position('|' IN subject) = 0),
    body TEXT NOT NULL CHECK (octet_length(body) BETWEEN 1 AND 4096),
    created_at BIGINT NOT NULL,
    expires_at BIGINT NOT NULL,
    report_id BIGINT UNIQUE REFERENCES social_reports(id) ON DELETE SET NULL,
    CHECK (expires_at = created_at + 7776000)
);
CREATE INDEX social_board_posts_board_idx
    ON social_board_posts (board_key, created_at DESC, id DESC);
CREATE INDEX social_board_posts_author_idx
    ON social_board_posts (author_character_id, created_at DESC, id DESC);
CREATE INDEX social_board_posts_expires_idx
    ON social_board_posts (expires_at, id);

CREATE TABLE social_board_report_evidence (
    report_id BIGINT PRIMARY KEY REFERENCES social_reports(id) ON DELETE CASCADE,
    board_key TEXT NOT NULL CHECK (board_key IN ('general', 'newbie')),
    post_public_id TEXT NOT NULL CHECK (post_public_id ~ '^b[0-9a-f]{32}$'),
    author_name TEXT NOT NULL
        CHECK (octet_length(author_name) BETWEEN 1 AND 64),
    subject TEXT NOT NULL CHECK (octet_length(subject) BETWEEN 1 AND 128),
    body TEXT NOT NULL CHECK (octet_length(body) BETWEEN 1 AND 4096),
    posted_at BIGINT NOT NULL,
    created_at BIGINT NOT NULL,
    expires_at BIGINT NOT NULL,
    CHECK (created_at >= posted_at),
    CHECK (expires_at = created_at + 7776000)
);
CREATE INDEX social_board_report_evidence_expires_idx
    ON social_board_report_evidence (expires_at, report_id);

UPDATE tiamud_schema
SET version = 13, updated_at = floor(extract(epoch FROM now()))::BIGINT
WHERE singleton = 1 AND version = 12;
"#;

const UPGRADE_V13_TO_V14_SQL: &str = r#"
CREATE TABLE world_area_generation (
    singleton BIGINT PRIMARY KEY CHECK (singleton = 1),
    world_id TEXT NOT NULL CHECK (octet_length(world_id) BETWEEN 1 AND 256),
    active_catalog_sha256 TEXT NOT NULL
        CHECK (active_catalog_sha256 ~ '^[0-9a-f]{64}$'),
    previous_catalog_sha256 TEXT
        CHECK (previous_catalog_sha256 IS NULL
               OR previous_catalog_sha256 ~ '^[0-9a-f]{64}$'),
    activation_sequence BIGINT NOT NULL CHECK (activation_sequence > 0),
    updated_at BIGINT NOT NULL DEFAULT (floor(extract(epoch FROM now()))::BIGINT),
    CHECK (previous_catalog_sha256 IS NULL
           OR previous_catalog_sha256 <> active_catalog_sha256)
);
UPDATE tiamud_schema
SET version = 14, updated_at = floor(extract(epoch FROM now()))::BIGINT
WHERE singleton = 1 AND version = 13;
"#;

const UPGRADE_V14_TO_V15_SQL: &str = r#"
CREATE TABLE world_area_checkpoints (
    generation_sha256 TEXT NOT NULL
        CHECK (generation_sha256 ~ '^[0-9a-f]{64}$'),
    unit_id TEXT NOT NULL CHECK (unit_id ~ '^[0-9a-f]{64}$'),
    residency_epoch BIGINT NOT NULL CHECK (residency_epoch > 0),
    checkpoint_version BIGINT NOT NULL CHECK (checkpoint_version > 0),
    checkpoint_sha256 TEXT NOT NULL
        CHECK (checkpoint_sha256 ~ '^[0-9a-f]{64}$'),
    captured_logical_tick BIGINT NOT NULL CHECK (captured_logical_tick >= 0),
    checkpoint_json TEXT NOT NULL
        CHECK (octet_length(checkpoint_json) BETWEEN 1 AND 2097152),
    created_at BIGINT NOT NULL DEFAULT (floor(extract(epoch FROM now()))::BIGINT),
    updated_at BIGINT NOT NULL DEFAULT (floor(extract(epoch FROM now()))::BIGINT),
    PRIMARY KEY (generation_sha256, unit_id, residency_epoch),
    CHECK (updated_at >= created_at)
);
UPDATE tiamud_schema
SET version = 15, updated_at = floor(extract(epoch FROM now()))::BIGINT
WHERE singleton = 1 AND version = 14;
"#;

const UPGRADE_V15_TO_V16_SQL: &str = r#"
CREATE TABLE story_saves (
    account_id BIGINT NOT NULL REFERENCES accounts(id) ON DELETE CASCADE,
    story_id TEXT NOT NULL CHECK (story_id ~ '^[a-z0-9][a-z0-9._-]{0,63}$'),
    slot SMALLINT NOT NULL CHECK (slot BETWEEN 0 AND 9),
    pack_id TEXT NOT NULL CHECK (pack_id ~ '^[a-z0-9][a-z0-9._-]{0,63}$'),
    pack_sha256 TEXT NOT NULL CHECK (pack_sha256 ~ '^[0-9a-f]{64}$'),
    state_format_version INTEGER NOT NULL
        CHECK (state_format_version BETWEEN 1 AND 255),
    state_json TEXT NOT NULL
        CHECK (octet_length(state_json) BETWEEN 2 AND 65536),
    state_sha256 TEXT NOT NULL CHECK (state_sha256 ~ '^[0-9a-f]{64}$'),
    created_at BIGINT NOT NULL,
    updated_at BIGINT NOT NULL,
    PRIMARY KEY (account_id, story_id, slot),
    CHECK (updated_at >= created_at)
);
UPDATE tiamud_schema
SET version = 16, updated_at = floor(extract(epoch FROM now()))::BIGINT
WHERE singleton = 1 AND version = 15;
"#;

/// Structural part of v16 -> v17 (ADR 0035). The legacy single document is
/// read before this runs and written back as verified sections after it, all
/// in the migration transaction.
const UPGRADE_V16_TO_V17_SQL: &str = r#"
DROP TABLE world_runtime;
CREATE TABLE world_runtime (
    singleton BIGINT PRIMARY KEY CHECK (singleton = 1),
    snapshot_version BIGINT NOT NULL,
    head_json TEXT NOT NULL CHECK (octet_length(head_json) <= 2097152),
    manifest_json TEXT NOT NULL
        CHECK (octet_length(manifest_json) BETWEEN 2 AND 8192),
    last_event_sequence BIGINT
        CHECK (last_event_sequence IS NULL OR last_event_sequence >= 0),
    updated_at BIGINT NOT NULL DEFAULT (floor(extract(epoch FROM now()))::BIGINT)
);
CREATE TABLE world_runtime_sections (
    section TEXT PRIMARY KEY CHECK (section IN (
        'itemInstances', 'itemProvenance', 'npcs', 'adventure',
        'progressionFlags', 'trades', 'lootEncounters', 'groundLoot',
        'partyLootCursors', 'areaResidency'
    )),
    revision BIGINT NOT NULL CHECK (revision > 0),
    section_json TEXT NOT NULL
        CHECK (octet_length(section_json) BETWEEN 1 AND 2097152),
    section_sha256 TEXT NOT NULL CHECK (section_sha256 ~ '^[0-9a-f]{64}$'),
    updated_at BIGINT NOT NULL DEFAULT (floor(extract(epoch FROM now()))::BIGINT)
);
UPDATE tiamud_schema
SET version = 17, updated_at = floor(extract(epoch FROM now()))::BIGINT
WHERE singleton = 1 AND version = 16;
"#;

/// Convert the v16 single-document runtime into the v17 sectioned layout,
/// preserving its version, bytes-equivalent content, watermark and
/// `updated_at`. A non-object legacy document is kept verbatim as the head so
/// startup still fails closed on it.
fn upgrade_v16_to_v17<C: GenericClient>(client: &mut C) -> Result<(), SchemaError> {
    let legacy = client
        .query_opt(
            "SELECT snapshot_version, snapshot_json, last_event_sequence, updated_at
             FROM world_runtime WHERE singleton = 1",
            &[],
        )?
        .map(|row| {
            (
                row.get::<_, i64>(0),
                row.get::<_, String>(1),
                row.get::<_, Option<i64>>(2),
                row.get::<_, i64>(3),
            )
        });
    client.batch_execute(UPGRADE_V16_TO_V17_SQL)?;
    if let Some((snapshot_version, json, last_event_sequence, updated_at)) = legacy {
        runtime_sections::write_runtime_document(
            client,
            snapshot_version,
            &json,
            last_event_sequence,
        )?;
        client.execute(
            "UPDATE world_runtime SET updated_at = $1 WHERE singleton = 1",
            &[&updated_at],
        )?;
    }
    Ok(())
}

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
        "PostgreSQL schema version {found} is unsupported; this backend supports a new v17 schema or migration from v6/v7/v8/v9/v10/v11/v12/v13/v14/v15/v16"
    )]
    UnsupportedVersion { found: i32 },
    #[error("PostgreSQL schema metadata says v17 but required table {table} is missing")]
    MissingTable { table: &'static str },
    #[error("could not convert the v16 world runtime into v17 sections: {0}")]
    RuntimeSections(#[from] runtime_sections::RuntimeRowsError),
}

/// Install or verify the current schema in one PostgreSQL transaction.
///
/// PostgreSQL persistence starts at v6, so an empty destination is installed
/// at the current version, v6-v16 are upgraded to v17, and v17 is verified.
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
    // The automatic first-admin promotion belongs only to the v6 role
    // migration. It must precede v10's one-time role-to-capability seed so the
    // promoted account receives the administrator profile and capabilities.
    if found == 6 {
        client.batch_execute(ENSURE_BOOTSTRAP_ADMIN_SQL)?;
    }
    match found {
        6 => {
            client.batch_execute(UPGRADE_V6_TO_V7_SQL)?;
            client.batch_execute(UPGRADE_V7_TO_V8_SQL)?;
            client.batch_execute(UPGRADE_V8_TO_V9_SQL)?;
            client.batch_execute(UPGRADE_V9_TO_V10_SQL)?;
            client.batch_execute(UPGRADE_V10_TO_V11_SQL)?;
            client.batch_execute(UPGRADE_V11_TO_V12_SQL)?;
        }
        7 => {
            client.batch_execute(UPGRADE_V7_TO_V8_SQL)?;
            client.batch_execute(UPGRADE_V8_TO_V9_SQL)?;
            client.batch_execute(UPGRADE_V9_TO_V10_SQL)?;
            client.batch_execute(UPGRADE_V10_TO_V11_SQL)?;
            client.batch_execute(UPGRADE_V11_TO_V12_SQL)?;
        }
        8 => {
            client.batch_execute(UPGRADE_V8_TO_V9_SQL)?;
            client.batch_execute(UPGRADE_V9_TO_V10_SQL)?;
            client.batch_execute(UPGRADE_V10_TO_V11_SQL)?;
            client.batch_execute(UPGRADE_V11_TO_V12_SQL)?;
        }
        9 => {
            client.batch_execute(UPGRADE_V9_TO_V10_SQL)?;
            client.batch_execute(UPGRADE_V10_TO_V11_SQL)?;
            client.batch_execute(UPGRADE_V11_TO_V12_SQL)?;
        }
        10 => {
            client.batch_execute(UPGRADE_V10_TO_V11_SQL)?;
            client.batch_execute(UPGRADE_V11_TO_V12_SQL)?;
        }
        11 => client.batch_execute(UPGRADE_V11_TO_V12_SQL)?,
        12..=16 => {}
        SCHEMA_VERSION => {}
        _ => return Err(SchemaError::UnsupportedVersion { found }),
    }
    if (6..=12).contains(&found) {
        client.batch_execute(UPGRADE_V12_TO_V13_SQL)?;
    }
    if (6..=13).contains(&found) {
        client.batch_execute(UPGRADE_V13_TO_V14_SQL)?;
    }
    if (6..=14).contains(&found) {
        client.batch_execute(UPGRADE_V14_TO_V15_SQL)?;
    }
    if (6..=15).contains(&found) {
        client.batch_execute(UPGRADE_V15_TO_V16_SQL)?;
    }
    if (6..=16).contains(&found) {
        upgrade_v16_to_v17(client)?;
    }
    for &table in &APPLICATION_TABLES {
        let exists: bool = client
            .query_one("SELECT to_regclass($1::text) IS NOT NULL", &[&table])?
            .get(0);
        if !exists {
            return Err(SchemaError::MissingTable { table });
        }
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
            "world_area_generation",
            "world_area_checkpoints",
            "social_ignores",
            "social_blocks",
            "social_channel_subscriptions",
            "social_mail_messages",
            "social_mail_usage",
            "social_mail_report_evidence",
            "social_board_posts",
            "social_board_usage",
            "social_board_report_evidence",
            "moderation_mutes",
            "account_bans",
            "social_reports",
            "moderation_audit",
            "staff_profiles",
            "staff_capability_assignments",
            "staff_audit",
            "character_adventure",
            "world_adventure",
            "story_saves",
            "world_runtime_sections",
        ] {
            assert!(
                SCHEMA_SQL.contains(&format!("CREATE TABLE {table}")),
                "missing {table}"
            );
        }
        assert!(SCHEMA_SQL.contains("VALUES (1, 17)"));
        assert!(SCHEMA_SQL.contains("last_event_sequence BIGINT"));
    }

    #[test]
    fn schema_keeps_security_relevant_limits_and_case_insensitive_identity() {
        assert!(SCHEMA_SQL.contains("octet_length(token_hash) = 32"));
        assert!(SCHEMA_SQL.contains("octet_length(gameplay_json) <= 65536"));
        assert!(SCHEMA_SQL.contains("octet_length(head_json) <= 2097152"));
        assert!(SCHEMA_SQL.contains("octet_length(section_json) BETWEEN 1 AND 2097152"));
        assert!(!SCHEMA_SQL.contains("snapshot_json"));
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
    fn v10_upgrade_adds_closed_staff_authority_and_resolvable_reports() {
        for profile in [
            "helper",
            "builder",
            "moderator",
            "administrator",
            "implementor",
        ] {
            assert!(UPGRADE_V9_TO_V10_SQL.contains(&format!("'{profile}'")));
            assert!(SCHEMA_SQL.contains(&format!("'{profile}'")));
        }
        for capability in [
            "reports.view",
            "reports.resolve",
            "moderation.mute",
            "moderation.ban",
            "staff.assign",
            "staff.inspect",
            "staff.where",
            "staff.goto",
            "staff.transfer",
            "staff.restore",
            "staff.announce",
            "staff.audit",
        ] {
            assert!(UPGRADE_V9_TO_V10_SQL.contains(&format!("'{capability}'")));
            assert!(SCHEMA_SQL.contains(&format!("'{capability}'")));
        }
        assert!(UPGRADE_V9_TO_V10_SQL.contains("status IN ('open', 'resolved')"));
        assert!(SCHEMA_SQL.contains("status IN ('open', 'resolved')"));
        assert!(UPGRADE_V9_TO_V10_SQL.contains("resolved_by_account_id"));
        assert!(SCHEMA_SQL.contains("resolved_by_account_id"));
        assert!(UPGRADE_V9_TO_V10_SQL.contains("staff_audit_no_update"));
        assert!(UPGRADE_V9_TO_V10_SQL.contains("staff_audit_no_delete"));
        assert!(SCHEMA_SQL.contains("staff_audit_no_update"));
        assert!(SCHEMA_SQL.contains("staff_audit_no_delete"));
        assert!(UPGRADE_V9_TO_V10_SQL.contains("accounts.role = 'moderator'"));
        assert!(UPGRADE_V9_TO_V10_SQL.contains("SET version = 10"));
    }

    #[test]
    fn v11_upgrade_replaces_only_the_runtime_snapshot_byte_constraint() {
        assert!(
            UPGRADE_V10_TO_V11_SQL.contains("DROP CONSTRAINT world_runtime_snapshot_json_check")
        );
        assert!(UPGRADE_V10_TO_V11_SQL.contains("CHECK (octet_length(snapshot_json) <= 2097152)"));
        assert!(UPGRADE_V10_TO_V11_SQL.contains("SET version = 11"));
        assert!(!UPGRADE_V10_TO_V11_SQL.contains("DROP TABLE"));
        assert!(!UPGRADE_V10_TO_V11_SQL.contains("DELETE FROM"));
    }

    #[test]
    fn v12_upgrade_adds_bounded_offline_mail_without_snapshot_changes() {
        for table in [
            "social_mail_messages",
            "social_mail_usage",
            "social_mail_report_evidence",
        ] {
            assert!(UPGRADE_V11_TO_V12_SQL.contains(&format!("CREATE TABLE {table}")));
            assert!(SCHEMA_SQL.contains(&format!("CREATE TABLE {table}")));
        }
        for limit in ["20", "50", "128", "2048", "2592000", "7776000"] {
            assert!(UPGRADE_V11_TO_V12_SQL.contains(limit));
            assert!(SCHEMA_SQL.contains(limit));
        }
        assert!(UPGRADE_V11_TO_V12_SQL.contains("SET version = 12"));
        assert!(UPGRADE_V11_TO_V12_SQL.contains("ADD COLUMN report_kind"));
        assert!(UPGRADE_V11_TO_V12_SQL.contains("'character', 'mail'"));
        assert!(!UPGRADE_V11_TO_V12_SQL.contains("world_runtime"));
        assert!(!UPGRADE_V11_TO_V12_SQL.contains("DELETE FROM"));
    }

    #[test]
    fn v13_upgrade_adds_bounded_boards_without_snapshot_changes() {
        for table in [
            "social_board_posts",
            "social_board_usage",
            "social_board_report_evidence",
        ] {
            assert!(UPGRADE_V12_TO_V13_SQL.contains(&format!("CREATE TABLE {table}")));
            assert!(SCHEMA_SQL.contains(&format!("CREATE TABLE {table}")));
        }
        for limit in ["5", "20", "128", "4096", "7776000"] {
            assert!(UPGRADE_V12_TO_V13_SQL.contains(limit));
            assert!(SCHEMA_SQL.contains(limit));
        }
        assert!(UPGRADE_V12_TO_V13_SQL.contains("SET version = 13"));
        assert!(UPGRADE_V12_TO_V13_SQL.contains("'character', 'mail', 'board'"));
        assert!(!UPGRADE_V12_TO_V13_SQL.contains("world_runtime"));
        assert!(!UPGRADE_V12_TO_V13_SQL.contains("DELETE FROM"));
    }

    #[test]
    fn v14_upgrade_adds_only_the_durable_area_generation_pointer() {
        assert!(UPGRADE_V13_TO_V14_SQL.contains("CREATE TABLE world_area_generation"));
        assert!(UPGRADE_V13_TO_V14_SQL.contains("active_catalog_sha256"));
        assert!(UPGRADE_V13_TO_V14_SQL.contains("previous_catalog_sha256"));
        assert!(UPGRADE_V13_TO_V14_SQL.contains("activation_sequence"));
        assert!(UPGRADE_V13_TO_V14_SQL.contains("SET version = 14"));
        assert!(!UPGRADE_V13_TO_V14_SQL.contains("world_runtime"));
        assert!(!UPGRADE_V13_TO_V14_SQL.contains("DELETE FROM"));
    }

    #[test]
    fn v15_upgrade_adds_only_bounded_area_checkpoints() {
        assert!(UPGRADE_V14_TO_V15_SQL.contains("CREATE TABLE world_area_checkpoints"));
        for column in [
            "generation_sha256",
            "unit_id",
            "residency_epoch",
            "checkpoint_version",
            "checkpoint_sha256",
            "captured_logical_tick",
            "checkpoint_json",
            "created_at",
            "updated_at",
        ] {
            assert!(UPGRADE_V14_TO_V15_SQL.contains(column));
            assert!(SCHEMA_SQL.contains(column));
        }
        assert!(UPGRADE_V14_TO_V15_SQL.contains("^[0-9a-f]{64}$"));
        assert!(UPGRADE_V14_TO_V15_SQL.contains("BETWEEN 1 AND 2097152"));
        assert!(
            UPGRADE_V14_TO_V15_SQL
                .contains("PRIMARY KEY (generation_sha256, unit_id, residency_epoch)")
        );
        assert!(UPGRADE_V14_TO_V15_SQL.contains("SET version = 15"));
        assert!(!UPGRADE_V14_TO_V15_SQL.contains("world_runtime"));
        assert!(!UPGRADE_V14_TO_V15_SQL.contains("DELETE FROM"));
    }

    #[test]
    fn v17_upgrade_matches_the_fresh_sectioned_runtime_tables() {
        fn table(sql: &str, name: &str) -> String {
            let start = sql.find(&format!("CREATE TABLE {name} (")).unwrap();
            let end = start + sql[start..].find("\n);").unwrap();
            sql[start..end].to_owned()
        }
        for name in ["world_runtime", "world_runtime_sections"] {
            assert_eq!(
                table(SCHEMA_SQL, name),
                table(UPGRADE_V16_TO_V17_SQL, name),
                "fresh install and v16->v17 upgrade must create the identical {name}"
            );
        }
        assert!(UPGRADE_V16_TO_V17_SQL.contains("SET version = 17"));
        assert!(UPGRADE_V16_TO_V17_SQL.contains("octet_length(manifest_json) BETWEEN 2 AND 8192"));
        assert!(UPGRADE_V16_TO_V17_SQL.contains("section_sha256 ~ '^[0-9a-f]{64}$'"));
        assert!(!UPGRADE_V16_TO_V17_SQL.contains("DELETE FROM domain_events"));
        for name in tiamud_persistence_sqlite::runtime_sections::RUNTIME_SECTION_NAMES {
            assert!(
                UPGRADE_V16_TO_V17_SQL.contains(&format!("'{name}'")),
                "{name}"
            );
        }
    }

    #[test]
    fn v16_upgrade_adds_only_bounded_story_save_slots() {
        assert!(UPGRADE_V15_TO_V16_SQL.contains("CREATE TABLE story_saves"));
        for column in [
            "account_id",
            "story_id",
            "slot",
            "pack_id",
            "pack_sha256",
            "state_format_version",
            "state_json",
            "state_sha256",
            "created_at",
            "updated_at",
        ] {
            assert!(UPGRADE_V15_TO_V16_SQL.contains(column));
            assert!(SCHEMA_SQL.contains(column));
        }
        let story_table = |sql: &str| {
            let start = sql.find("CREATE TABLE story_saves").unwrap();
            let end = start + sql[start..].find(");").unwrap();
            sql[start..end].to_owned()
        };
        assert_eq!(
            story_table(UPGRADE_V15_TO_V16_SQL),
            story_table(SCHEMA_SQL),
            "fresh install and v15->v16 upgrade must create the identical table"
        );
        assert!(UPGRADE_V15_TO_V16_SQL.contains("REFERENCES accounts(id) ON DELETE CASCADE"));
        assert!(UPGRADE_V15_TO_V16_SQL.contains("slot BETWEEN 0 AND 9"));
        assert!(UPGRADE_V15_TO_V16_SQL.contains("BETWEEN 2 AND 65536"));
        assert!(UPGRADE_V15_TO_V16_SQL.contains("BETWEEN 1 AND 255"));
        assert!(UPGRADE_V15_TO_V16_SQL.contains("PRIMARY KEY (account_id, story_id, slot)"));
        assert!(UPGRADE_V15_TO_V16_SQL.contains("SET version = 16"));
        assert!(!UPGRADE_V15_TO_V16_SQL.contains("world_runtime"));
        assert!(!UPGRADE_V15_TO_V16_SQL.contains("DELETE FROM"));
        assert_eq!(
            tiamud_persistence_sqlite::story_saves::MAX_STORY_SAVE_STATE_BYTES,
            65_536
        );
        assert_eq!(tiamud_persistence_sqlite::story_saves::MAX_STORY_SLOT, 9);
    }

    #[test]
    fn postgres_contract_tracks_the_sqlite_source_contract() {
        assert_eq!(
            i64::from(SCHEMA_VERSION),
            tiamud_persistence_sqlite::SCHEMA_VERSION
        );
    }
}
