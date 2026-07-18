use crate::{
    auth::{
        AuthenticatedSession, DEFAULT_SESSION_TTL, MAX_PASSWORD_BYTES, MIN_PASSWORD_BYTES,
        PasswordService, PasswordWorkError, SessionToken, digest_token, validate_password,
    },
    operations::RuntimeMetrics,
};
use async_trait::async_trait;
use rusqlite::{Connection, OptionalExtension, params, params_from_iter, types::Type};
use sha2::{Digest, Sha256};
#[cfg(test)]
use std::sync::atomic::{AtomicBool, AtomicU8, Ordering};
use std::{
    collections::BTreeSet,
    future::Future,
    io,
    ops::ControlFlow,
    path::Path,
    sync::Arc,
    thread,
    time::{Instant, SystemTime, UNIX_EPOCH},
};
use thiserror::Error;
use tiamud_core::{
    AccountId, AccountRole, AccountSnapshot, AreaDynamicCheckpoint, AreaRuntimeResidence, BoardKey,
    CharacterGameplaySnapshot, CharacterId, CharacterSnapshot, DomainEvent,
    DurableCharacterSnapshot, MAX_ITEM_INSTANCES, SocialChannelName, StaffCapability,
    StaffProfileKey, StaffReportResolution, WorldRuntimeSnapshot, valid_board_public_id,
    valid_board_subject, valid_board_visible_text, valid_mail_public_id, valid_mail_subject,
    valid_mail_target, valid_mail_visible_text,
};
use tiamud_persistence_sqlite::{SchemaError, migrate};
#[cfg(test)]
use tokio::sync::Notify;
use tokio::{
    sync::{mpsc, oneshot},
    time::{Duration, timeout},
};

#[cfg(test)]
use crate::auth::{hash_password, verify_password};

const MAX_IDENTITY_NAME_BYTES: usize = 64;
const MAX_ACTIVE_SESSIONS: i64 = 16;
const MAX_CHARACTER_JSON_BYTES: usize = 65_536;
const MAX_EVENT_JSON_BYTES: usize = 65_536;
const MAX_RUNTIME_JSON_BYTES: usize = tiamud_core::MAX_WORLD_RUNTIME_SNAPSHOT_BYTES;
const MAX_DURABLE_AREA_CHECKPOINTS: usize = tiamud_core::MAX_RUNTIME_AREA_RESIDENCY_ENTRIES;
/// Snapshot v6 introduced the 2 MiB physical budget. Keep that admission
/// boundary stable when the current runtime format advances to v7 and beyond;
/// only the exact v1-v5 DTOs remain on the historical 1 MiB budget.
const LAST_ONE_MIB_RUNTIME_SNAPSHOT_VERSION: u32 = 5;
/// Number of snapshot-covered domain events retained for diagnostics and
/// compatibility. Older covered events are compacted transactionally.
pub(super) const DOMAIN_EVENT_RETENTION: i64 = 10_000;
pub const MAX_SOCIAL_REASON_BYTES: usize = 1_024;
pub const MAX_REPORT_REASON_BYTES: usize = 2_048;
const MAX_AUDIT_DETAILS_BYTES: usize = 4_096;
const MAX_STAFF_ACTION_BYTES: usize = 64;
const MAX_STAFF_LORE_TITLE_BYTES: usize = 128;
const STAFF_LIST_LIMIT: i64 = 10;
pub const MAX_MAIL_SUBJECT_BYTES: usize = 128;
pub const MAX_MAIL_BODY_BYTES: usize = 2_048;
pub const MAILBOX_CAPACITY: u32 = 64;
pub const MAIL_PAGE_SIZE: u32 = 10;
pub const MAIL_MAX_PAGES: u32 = 7;
const MAILBOX_BYTE_CAPACITY: i64 = 128 * 1_024;
const MAIL_GLOBAL_CAPACITY: i64 = 100_000;
const MAIL_HOURLY_CAPACITY: i64 = 20;
const MAIL_DAILY_CAPACITY: i64 = 50;
const MAIL_HOUR_SECONDS: i64 = 60 * 60;
const MAIL_DAY_SECONDS: i64 = 24 * MAIL_HOUR_SECONDS;
const MAIL_RETENTION_SECONDS: i64 = 30 * MAIL_DAY_SECONDS;
const MAIL_REPORT_EVIDENCE_RETENTION_SECONDS: i64 = 90 * MAIL_DAY_SECONDS;
const MAIL_OPEN_REPORT_CAPACITY_PER_ACCOUNT: i64 = 8;
const MAIL_REPORT_EVIDENCE_GLOBAL_CAPACITY: i64 = 10_000;
const MAIL_PURGE_LIMIT: i64 = 256;
pub const BOARD_PAGE_SIZE: u32 = 10;
pub const BOARD_MAX_PAGES: u32 = 100;
pub const BOARD_ACCOUNT_CAPACITY: u32 = 64;
pub const BOARD_CAPACITY_PER_BOARD: u32 = 1_000;
const BOARD_BYTE_CAPACITY_PER_BOARD: i64 = 4 * 1_024 * 1_024;
const BOARD_GLOBAL_CAPACITY: i64 = 2_000;
const BOARD_GLOBAL_BYTE_CAPACITY: i64 = 8 * 1_024 * 1_024;
const BOARD_HOURLY_CAPACITY: i64 = 5;
const BOARD_DAILY_CAPACITY: i64 = 20;
const BOARD_RETENTION_SECONDS: i64 = 90 * MAIL_DAY_SECONDS;
const BOARD_REPORT_EVIDENCE_RETENTION_SECONDS: i64 = 90 * MAIL_DAY_SECONDS;
const BOARD_OPEN_REPORT_CAPACITY_PER_ACCOUNT: i64 = 8;
const BOARD_REPORT_EVIDENCE_GLOBAL_CAPACITY: i64 = 10_000;
const BOARD_PURGE_LIMIT: i64 = 256;
const PERSISTENCE_REQUEST_QUEUE_CAPACITY: usize = 256;
const PERSISTENCE_REQUEST_TIMEOUT: Duration = Duration::from_secs(5);

/// Startup first reads characters referenced as runtime item owners under the
/// runtime snapshot budget, then the bounded set of extra trade participants
/// under its separate exact aggregate budget.
pub(super) const MAX_STARTUP_ITEM_OWNERS: usize = MAX_ITEM_INSTANCES;
pub(super) const MAX_STARTUP_OWNER_GAMEPLAY_BYTES: usize =
    tiamud_core::MAX_LEGACY_WORLD_RUNTIME_SNAPSHOT_BYTES;
pub(super) const MAX_STARTUP_TRADE_PARTICIPANTS: usize = 128;
pub(super) const MAX_STARTUP_TRADE_GAMEPLAY_BYTES: usize =
    MAX_STARTUP_TRADE_PARTICIPANTS * MAX_CHARACTER_JSON_BYTES;
/// Maximum distinct durable character identities that one restored runtime may
/// reference without loading their gameplay aggregates. This covers both the
/// maximum 2,048 ground-loot reservations and 2,048 active loot encounters,
/// each with cohorts of six, while keeping startup admission independent from
/// snapshot contents and from gameplay-document budgets.
pub(super) const MAX_STARTUP_CHARACTER_EXISTENCE_IDS: usize = (2_048 + 2_048) * 6;
const SQLITE_CHARACTER_BATCH_SIZE: usize = 900;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct StartupGameplayLoadLimits {
    maximum_characters: usize,
    maximum_bytes: usize,
    kind: &'static str,
}

impl StartupGameplayLoadLimits {
    pub const ITEM_OWNERS: Self = Self {
        maximum_characters: MAX_STARTUP_ITEM_OWNERS,
        maximum_bytes: MAX_STARTUP_OWNER_GAMEPLAY_BYTES,
        kind: "startup owner gameplay",
    };

    pub const TRADE_PARTICIPANTS: Self = Self {
        maximum_characters: MAX_STARTUP_TRADE_PARTICIPANTS,
        maximum_bytes: MAX_STARTUP_TRADE_GAMEPLAY_BYTES,
        kind: "startup trade participant gameplay",
    };

    pub const fn maximum_characters(self) -> usize {
        self.maximum_characters
    }

    pub const fn maximum_bytes(self) -> usize {
        self.maximum_bytes
    }

    pub const fn kind(self) -> &'static str {
        self.kind
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DurableCharacterGameplay {
    pub character_id: CharacterId,
    pub gameplay: CharacterGameplaySnapshot,
}

/// One atomically observed durable authority for world startup.
///
/// A runtime snapshot is authoritative whenever present, even if the retained
/// diagnostic ledger is non-empty. Without a snapshot, a non-empty ledger is
/// the recovery authority; only the absence of both permits a partial initial
/// area load. `durable_version` preserves the exact stored encoding even when
/// the decoded snapshot has been normalized to the current representation.
#[derive(Clone, Debug, Eq, PartialEq)]
#[allow(clippy::large_enum_variant)]
pub enum StartupWorldRecovery {
    Empty,
    Snapshot {
        durable_version: u32,
        snapshot: WorldRuntimeSnapshot,
    },
    Ledger(Vec<DomainEvent>),
}

/// Durable, host-independent identity of the catalog generation selected by
/// the world authority. Registry paths are deliberately excluded: the exact
/// content-addressed path is derived from the configured registry root.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DurableAreaGeneration {
    pub world_id: String,
    pub active_catalog_sha256: String,
    pub previous_catalog_sha256: Option<String>,
    pub activation_sequence: u64,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DurableAreaCheckpoint {
    pub generation_sha256: String,
    pub unit_id: String,
    pub residency_epoch: u64,
    pub checkpoint_version: u32,
    pub checkpoint_sha256: String,
    pub captured_logical_tick: u64,
    pub checkpoint_json: String,
}

impl StartupWorldRecovery {
    pub const fn requires_full_generation(&self) -> bool {
        !matches!(self, Self::Empty)
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SocialProfile {
    pub character_id: CharacterId,
    pub account_id: AccountId,
    pub character_name: String,
    pub role: AccountRole,
    pub muted: bool,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum MailSendResult {
    Stored,
    Suppressed,
    Unavailable,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MailMessageSummary {
    pub public_id: String,
    pub sender_name: String,
    pub subject: String,
    pub created_at: i64,
    pub expires_at: i64,
    pub read: bool,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MailPage {
    pub page: u32,
    pub total_pages: u32,
    pub total_messages: u32,
    pub unread_count: u32,
    pub capacity: u32,
    pub messages: Vec<MailMessageSummary>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MailMessage {
    pub summary: MailMessageSummary,
    pub body: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MailReportEvidence {
    pub sender_name: String,
    pub subject: String,
    pub body: String,
    pub sent_at: i64,
    pub created_at: i64,
    pub expires_at: i64,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct BoardPostSummary {
    pub public_id: String,
    pub board: BoardKey,
    pub author_name: String,
    pub subject: String,
    pub created_at: i64,
    pub expires_at: i64,
    pub can_delete: bool,
    pub can_report: bool,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct BoardPage {
    pub board: BoardKey,
    pub page: u32,
    pub total_pages: u32,
    pub total_posts: u32,
    pub capacity: u32,
    pub posts: Vec<BoardPostSummary>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct BoardEntry {
    pub summary: BoardPostSummary,
    pub body: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct BoardReportEvidence {
    pub board: BoardKey,
    pub post_public_id: String,
    pub author_name: String,
    pub subject: String,
    pub body: String,
    pub posted_at: i64,
    pub created_at: i64,
    pub expires_at: i64,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SocialRelationKind {
    Ignore,
    Block,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ModerationAction {
    Mute {
        expires_at: Option<i64>,
        reason: String,
    },
    Unmute {
        reason: String,
    },
    Ban {
        expires_at: Option<i64>,
        reason: String,
    },
    Unban {
        reason: String,
    },
    SetRole {
        role: AccountRole,
        reason: String,
    },
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct StaffAccess {
    pub account_id: AccountId,
    pub role: AccountRole,
    pub profile: Option<StaffProfileKey>,
    pub lore_title: Option<String>,
    pub capabilities: BTreeSet<StaffCapability>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct StaffReport {
    pub id: i64,
    pub reporter_character_id: CharacterId,
    pub reporter_name: String,
    pub target_character_id: CharacterId,
    pub target_account_id: AccountId,
    pub target_name: String,
    pub reason: String,
    pub created_at: i64,
    pub mail_evidence: Option<MailReportEvidence>,
    pub board_evidence: Option<BoardReportEvidence>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct StaffAuditEntry {
    pub sequence: i64,
    pub actor_account_id: AccountId,
    pub target_account_id: Option<AccountId>,
    pub capability: StaffCapability,
    pub action: String,
    pub reason: String,
    pub details_json: String,
    pub created_at: i64,
}

#[derive(Clone, Debug, PartialEq)]
pub struct StaffAuditRecord {
    pub actor_account_id: AccountId,
    pub target_account_id: Option<AccountId>,
    pub capability: StaffCapability,
    pub action: String,
    pub reason: String,
    pub details: serde_json::Value,
}

#[cfg(test)]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AuditEntry {
    pub sequence: i64,
    pub actor_account_id: AccountId,
    pub target_account_id: Option<AccountId>,
    pub action: String,
    pub details_json: String,
    pub created_at: i64,
}

#[derive(Debug, Error)]
pub enum StoreError {
    #[error("SQLite error: {0}")]
    Sqlite(#[from] rusqlite::Error),
    #[error("PostgreSQL error: {0}")]
    Postgres(#[from] postgres::Error),
    #[error("could not connect to PostgreSQL: {0}")]
    PostgresConnect(#[from] tiamud_persistence_postgres::ConnectError),
    #[error("could not initialise the PostgreSQL schema: {0}")]
    PostgresSchema(#[from] tiamud_persistence_postgres::SchemaError),
    #[error("invalid persisted character state: {0}")]
    CharacterState(#[from] serde_json::Error),
    #[error("the persisted world runtime snapshot is invalid or unsupported")]
    InvalidRuntimeSnapshot,
    #[error("the persisted startup domain-event ledger is incomplete or exceeds its limit")]
    InvalidStartupLedger,
    #[error("the durable area generation is invalid")]
    InvalidAreaGeneration,
    #[error("the durable area generation changed concurrently")]
    AreaGenerationConflict,
    #[error("this character store does not implement durable area generations")]
    AreaGenerationUnsupported,
    #[error("the durable area checkpoint is invalid")]
    InvalidAreaCheckpoint,
    #[error("the durable area checkpoint changed concurrently")]
    AreaCheckpointConflict,
    #[error("this character store does not implement durable area checkpoints")]
    AreaCheckpointUnsupported,
    #[error("serialized {kind} exceeds the {maximum} byte persistence limit")]
    SnapshotTooLarge { kind: &'static str, maximum: usize },
    #[error("durable character count exceeds the {maximum} aggregate startup limit")]
    TooManyCharacterAggregates { maximum: usize },
    #[error("durable character identity count exceeds the {maximum} startup limit")]
    TooManyCharacterIdentities { maximum: usize },
    #[error("the persistence worker stopped unexpectedly")]
    WorkerStopped,
    #[error("the persistence worker did not answer within the bounded timeout")]
    RequestTimeout,
    #[error("could not start the persistence worker: {0}")]
    ThreadSpawn(#[source] io::Error),
    #[error(
        "{kind} name must be 1-{MAX_IDENTITY_NAME_BYTES} bytes of safe visible text and cannot contain |"
    )]
    InvalidName { kind: &'static str },
    #[error("account {0:?} does not exist")]
    AccountNotFound(AccountId),
    #[error("an account with that name already exists")]
    AccountAlreadyExists,
    #[error("password must be {MIN_PASSWORD_BYTES}-{MAX_PASSWORD_BYTES} bytes")]
    InvalidPassword,
    #[error("invalid account name or password")]
    InvalidCredentials,
    #[error("the session token is invalid")]
    InvalidSession,
    #[error("the session has expired")]
    SessionExpired,
    #[error("the session has been revoked")]
    SessionRevoked,
    #[error("password hashing failed")]
    PasswordHashing,
    #[error("character {character_id:?} does not belong to account {account_id:?}")]
    CharacterNotFound {
        account_id: AccountId,
        character_id: CharacterId,
    },
    #[error("character gameplay state for {0:?} does not exist")]
    CharacterGameplayNotFound(CharacterId),
    #[error("character identity {0:?} is not a positive durable ID")]
    InvalidCharacterIdentity(CharacterId),
    #[error("character identity {0:?} does not exist")]
    CharacterIdentityNotFound(CharacterId),
    #[error("no character matches that social target")]
    SocialTargetNotFound,
    #[error("that social target is ambiguous")]
    AmbiguousSocialTarget,
    #[error("a social action cannot target the acting character")]
    SocialSelfTarget,
    #[error("that account is not authorized for this moderation action")]
    SocialForbidden,
    #[error("chat is currently muted for this account")]
    ChatMuted,
    #[error("chat rate limit exceeded")]
    ChatRateLimited,
    #[error("that account is banned")]
    AccountBanned,
    #[error("invalid or oversized social input")]
    InvalidSocialInput,
    #[error("invalid or oversized mail input")]
    InvalidMailInput,
    #[error("mail send quota exceeded")]
    MailRateLimited,
    #[error("that mail message is not available")]
    MailNotFound,
    #[error("that mail message has already been reported")]
    MailAlreadyReported,
    #[error("the account has too many open mail reports")]
    MailReportLimit,
    #[error("mail report evidence capacity is exhausted")]
    MailEvidenceLimit,
    #[error("invalid or oversized board input")]
    InvalidBoardInput,
    #[error("board publication quota exceeded")]
    BoardRateLimited,
    #[error("board publication capacity is exhausted")]
    BoardCapacity,
    #[error("that board post is not available")]
    BoardPostUnavailable,
    #[error("that board post has already been reported")]
    BoardAlreadyReported,
    #[error("the account has too many open board reports")]
    BoardReportLimit,
    #[error("board report evidence capacity is exhausted")]
    BoardEvidenceLimit,
    #[error("invalid or oversized staff input")]
    InvalidStaffInput,
    #[error("persisted staff state is invalid: {0}")]
    InvalidStaffState(String),
    #[error("that account does not hold the required staff capability")]
    StaffForbidden,
    #[error("the staff hierarchy does not permit that target or assignment")]
    StaffHierarchy,
    #[error("staff report {0} is not available for resolution")]
    StaffReportUnavailable(i64),
    #[error("database schema version {found} is newer than supported version {supported}")]
    UnsupportedSchemaVersion { found: i64, supported: i64 },
    #[error("database schema migration left foreign-key violations")]
    SchemaForeignKeyViolations,
}

/// Mutable identity and character state behind the transport-independent actor.
#[async_trait]
pub trait CharacterStore: Send + Sync {
    fn backend_name(&self) -> &'static str {
        "custom"
    }

    fn queue_depth(&self) -> usize {
        0
    }

    /// Verify that the backing worker and database can answer a minimal query.
    async fn health_check(&self) -> Result<(), StoreError> {
        Ok(())
    }

    /// Drains requests accepted before this barrier and closes the backend
    /// owner. Implementations without an owner thread may use the default.
    async fn shutdown(&self) -> Result<(), StoreError> {
        Ok(())
    }

    async fn register_account(
        &self,
        name: &str,
        password: &str,
    ) -> Result<AuthenticatedSession, StoreError>;

    async fn login_account(
        &self,
        name: &str,
        password: &str,
    ) -> Result<AuthenticatedSession, StoreError>;

    async fn resolve_session(&self, token: &str) -> Result<AccountSnapshot, StoreError>;

    async fn revoke_session(&self, token: &str) -> Result<(), StoreError>;

    #[cfg(test)]
    async fn load_or_create_account(&self, name: &str) -> Result<AccountSnapshot, StoreError>;

    async fn list_characters(
        &self,
        account_id: AccountId,
    ) -> Result<Vec<CharacterSnapshot>, StoreError>;

    #[cfg(test)]
    async fn load_or_create_character(
        &self,
        account_id: AccountId,
        name: &str,
        start_room: &str,
    ) -> Result<CharacterSnapshot, StoreError>;

    /// Create a character with a caller-selected initial gameplay sheet.
    ///
    /// Production stores insert identity and the bounded initial gameplay JSON
    /// atomically. An existing character with the same account/name is
    /// returned unchanged.
    async fn load_or_create_character_with_gameplay(
        &self,
        account_id: AccountId,
        name: &str,
        start_room: &str,
        initial_gameplay: CharacterGameplaySnapshot,
    ) -> Result<CharacterSnapshot, StoreError>;

    async fn load_character(
        &self,
        account_id: AccountId,
        character_id: CharacterId,
    ) -> Result<CharacterSnapshot, StoreError>;

    #[allow(dead_code)]
    async fn save(&self, snapshot: CharacterSnapshot) -> Result<(), StoreError>;

    async fn load_character_gameplay(
        &self,
        character_id: CharacterId,
    ) -> Result<CharacterGameplaySnapshot, StoreError>;

    /// Ensure every distinct positive identity exists without loading gameplay
    /// JSON. Startup uses this for durable references that require identity
    /// integrity but not a live character aggregate.
    async fn ensure_character_ids_exist_batch(
        &self,
        character_ids: Vec<CharacterId>,
    ) -> Result<(), StoreError>;

    /// Load only the durable gameplay aggregates requested by the restored
    /// runtime, using set-based batches and the closed startup scope rather
    /// than one query per character.
    async fn load_character_gameplay_batch(
        &self,
        character_ids: Vec<CharacterId>,
        limits: StartupGameplayLoadLimits,
    ) -> Result<Vec<DurableCharacterGameplay>, StoreError>;

    #[allow(dead_code)]
    async fn load_runtime_snapshot(&self) -> Result<Option<WorldRuntimeSnapshot>, StoreError>;

    /// Observe the startup snapshot/ledger choice through one backend
    /// transaction. Implementations must fail closed on an invalid snapshot
    /// and must never fall back to the ledger when a snapshot row exists.
    async fn load_startup_world_recovery(&self) -> Result<StartupWorldRecovery, StoreError>;

    /// True when at least one durable character aggregate exists, even if the
    /// shared runtime snapshot and event ledger are still empty.
    async fn has_durable_character_authority(&self) -> Result<bool, StoreError>;

    async fn load_area_generation(&self) -> Result<Option<DurableAreaGeneration>, StoreError> {
        Ok(None)
    }

    async fn bind_area_generation_if_absent(
        &self,
        _generation: DurableAreaGeneration,
    ) -> Result<DurableAreaGeneration, StoreError> {
        Err(StoreError::AreaGenerationUnsupported)
    }

    /// Atomically advances the durable generation pointer and the global
    /// runtime snapshot. The compare-and-swap precondition prevents two
    /// operator activations from both publishing.
    async fn commit_area_generation_activation(
        &self,
        _runtime: WorldRuntimeSnapshot,
        _expected_current: DurableAreaGeneration,
        _next: DurableAreaGeneration,
    ) -> Result<(), StoreError> {
        Err(StoreError::AreaGenerationUnsupported)
    }

    async fn load_area_checkpoints(&self) -> Result<Vec<DurableAreaCheckpoint>, StoreError> {
        Ok(Vec::new())
    }

    async fn commit_area_retirement(
        &self,
        _runtime: WorldRuntimeSnapshot,
        _expected_generation: DurableAreaGeneration,
        _checkpoint: DurableAreaCheckpoint,
    ) -> Result<(), StoreError> {
        Err(StoreError::AreaCheckpointUnsupported)
    }

    async fn commit_area_rehydration(
        &self,
        _runtime: WorldRuntimeSnapshot,
        _expected_generation: DurableAreaGeneration,
        _checkpoint: DurableAreaCheckpoint,
    ) -> Result<(), StoreError> {
        Err(StoreError::AreaCheckpointUnsupported)
    }

    /// Load the retained event suffix in durable sequence order. Before the
    /// first runtime snapshot this is the complete recovery ledger; once a
    /// snapshot exists, that snapshot is authoritative and covers the
    /// persisted `last_event_sequence` watermark.
    #[allow(dead_code)]
    async fn load_domain_events(&self) -> Result<Vec<DomainEvent>, StoreError>;

    async fn social_profile(&self, character_id: CharacterId) -> Result<SocialProfile, StoreError>;

    async fn social_target(&self, name: &str) -> Result<SocialProfile, StoreError>;

    /// Returns false when recipient-side ignore or either-side block policy
    /// suppresses delivery. Callers deliberately do not reveal which policy
    /// matched to the sender.
    async fn social_delivery_allowed(
        &self,
        sender: CharacterId,
        recipient: CharacterId,
    ) -> Result<bool, StoreError>;

    async fn set_social_relation(
        &self,
        actor: CharacterId,
        target: CharacterId,
        relation: SocialRelationKind,
        enabled: bool,
    ) -> Result<(), StoreError>;

    /// Return the character's durable subscriptions in closed channel order.
    async fn social_channel_subscriptions(
        &self,
        _character_id: CharacterId,
    ) -> Result<Vec<SocialChannelName>, StoreError> {
        Ok(Vec::new())
    }

    /// Return durable subscribers ordered by character identity so channel
    /// fan-out does not require one database query per live session.
    async fn social_channel_subscribers(
        &self,
        _channel: SocialChannelName,
    ) -> Result<Vec<CharacterId>, StoreError> {
        Ok(Vec::new())
    }

    /// Atomically add or remove one durable closed-channel subscription.
    async fn set_social_channel_subscription(
        &self,
        _character_id: CharacterId,
        _channel: SocialChannelName,
        _subscribed: bool,
    ) -> Result<(), StoreError> {
        Ok(())
    }

    async fn send_mail(
        &self,
        sender: CharacterId,
        target_name: &str,
        public_id: &str,
        subject: &str,
        body: &str,
    ) -> Result<MailSendResult, StoreError>;

    async fn mailbox_page(&self, recipient: CharacterId, page: u32)
    -> Result<MailPage, StoreError>;

    async fn read_mail(
        &self,
        recipient: CharacterId,
        public_id: &str,
    ) -> Result<MailMessage, StoreError>;

    async fn delete_mail(&self, recipient: CharacterId, public_id: &str) -> Result<(), StoreError>;

    async fn report_mail(
        &self,
        recipient: CharacterId,
        public_id: &str,
        reason: &str,
    ) -> Result<i64, StoreError>;

    async fn purge_expired_mail(&self) -> Result<u64, StoreError>;

    async fn list_board_posts(
        &self,
        actor: CharacterId,
        board: BoardKey,
        page: u32,
    ) -> Result<BoardPage, StoreError>;

    async fn read_board_post(
        &self,
        actor: CharacterId,
        public_id: &str,
    ) -> Result<BoardEntry, StoreError>;

    async fn publish_board_post(
        &self,
        actor: CharacterId,
        board: BoardKey,
        public_id: &str,
        subject: &str,
        body: &str,
    ) -> Result<BoardEntry, StoreError>;

    async fn delete_board_post(
        &self,
        actor: CharacterId,
        public_id: &str,
    ) -> Result<(), StoreError>;

    async fn report_board_post(
        &self,
        actor: CharacterId,
        public_id: &str,
        reason: &str,
    ) -> Result<i64, StoreError>;

    async fn purge_expired_boards(&self) -> Result<u64, StoreError>;

    async fn report_character(
        &self,
        actor: CharacterId,
        target: CharacterId,
        reason: &str,
    ) -> Result<(), StoreError>;

    async fn moderate_character(
        &self,
        actor: CharacterId,
        target: CharacterId,
        action: ModerationAction,
    ) -> Result<(), StoreError>;

    async fn staff_access(&self, character_id: CharacterId) -> Result<StaffAccess, StoreError>;

    async fn staff_reports(&self, audit: StaffAuditRecord) -> Result<Vec<StaffReport>, StoreError>;

    async fn resolve_staff_report(
        &self,
        report_id: i64,
        resolution: StaffReportResolution,
        resolution_reason: &str,
        audit: StaffAuditRecord,
    ) -> Result<(), StoreError>;

    async fn set_staff_profile(
        &self,
        target_account: AccountId,
        profile: Option<StaffProfileKey>,
        lore_title: Option<&str>,
        audit: StaffAuditRecord,
    ) -> Result<(), StoreError>;

    async fn set_staff_capability(
        &self,
        target_account: AccountId,
        capability: StaffCapability,
        enabled: bool,
        audit: StaffAuditRecord,
    ) -> Result<(), StoreError>;

    async fn staff_audit_entries(
        &self,
        audit: StaffAuditRecord,
    ) -> Result<Vec<StaffAuditEntry>, StoreError>;

    async fn append_staff_audit(&self, audit: StaffAuditRecord) -> Result<(), StoreError>;

    #[cfg(test)]
    async fn audit_log(&self) -> Result<Vec<AuditEntry>, StoreError>;

    /// Atomically persist the character aggregate and shared-world events
    /// produced by one accepted action.
    async fn commit_action(
        &self,
        characters: Vec<DurableCharacterSnapshot>,
        runtime: WorldRuntimeSnapshot,
        events: Vec<DomainEvent>,
    ) -> Result<(), StoreError>;

    async fn commit_staff_action(
        &self,
        characters: Vec<DurableCharacterSnapshot>,
        runtime: WorldRuntimeSnapshot,
        events: Vec<DomainEvent>,
        audit: StaffAuditRecord,
    ) -> Result<(), StoreError>;
}

/// Adds bounded operational timing to every database transaction while
/// retaining one authoritative backend. Read-only queries deliberately bypass
/// the transaction counters so the exported metric keeps its stated meaning.
pub struct MeasuredCharacterStore {
    inner: Arc<dyn CharacterStore>,
    metrics: Arc<RuntimeMetrics>,
}

impl MeasuredCharacterStore {
    pub fn new(inner: Arc<dyn CharacterStore>, metrics: Arc<RuntimeMetrics>) -> Self {
        Self { inner, metrics }
    }

    async fn transaction<T>(
        &self,
        operation: impl Future<Output = Result<T, StoreError>>,
    ) -> Result<T, StoreError> {
        let started = Instant::now();
        let result = operation.await;
        self.metrics
            .database_transaction_finished(started.elapsed(), result.is_ok());
        result
    }
}

#[async_trait]
impl CharacterStore for MeasuredCharacterStore {
    fn backend_name(&self) -> &'static str {
        self.inner.backend_name()
    }

    fn queue_depth(&self) -> usize {
        self.inner.queue_depth()
    }

    async fn health_check(&self) -> Result<(), StoreError> {
        self.inner.health_check().await
    }

    async fn shutdown(&self) -> Result<(), StoreError> {
        self.inner.shutdown().await
    }

    async fn register_account(
        &self,
        name: &str,
        password: &str,
    ) -> Result<AuthenticatedSession, StoreError> {
        self.transaction(self.inner.register_account(name, password))
            .await
    }

    async fn staff_access(&self, character_id: CharacterId) -> Result<StaffAccess, StoreError> {
        self.inner.staff_access(character_id).await
    }

    async fn staff_reports(&self, audit: StaffAuditRecord) -> Result<Vec<StaffReport>, StoreError> {
        self.transaction(self.inner.staff_reports(audit)).await
    }

    async fn resolve_staff_report(
        &self,
        report_id: i64,
        resolution: StaffReportResolution,
        resolution_reason: &str,
        audit: StaffAuditRecord,
    ) -> Result<(), StoreError> {
        self.transaction(self.inner.resolve_staff_report(
            report_id,
            resolution,
            resolution_reason,
            audit,
        ))
        .await
    }

    async fn set_staff_profile(
        &self,
        target_account: AccountId,
        profile: Option<StaffProfileKey>,
        lore_title: Option<&str>,
        audit: StaffAuditRecord,
    ) -> Result<(), StoreError> {
        self.transaction(
            self.inner
                .set_staff_profile(target_account, profile, lore_title, audit),
        )
        .await
    }

    async fn set_staff_capability(
        &self,
        target_account: AccountId,
        capability: StaffCapability,
        enabled: bool,
        audit: StaffAuditRecord,
    ) -> Result<(), StoreError> {
        self.transaction(self.inner.set_staff_capability(
            target_account,
            capability,
            enabled,
            audit,
        ))
        .await
    }

    async fn staff_audit_entries(
        &self,
        audit: StaffAuditRecord,
    ) -> Result<Vec<StaffAuditEntry>, StoreError> {
        self.transaction(self.inner.staff_audit_entries(audit))
            .await
    }

    async fn append_staff_audit(&self, audit: StaffAuditRecord) -> Result<(), StoreError> {
        self.transaction(self.inner.append_staff_audit(audit)).await
    }

    async fn login_account(
        &self,
        name: &str,
        password: &str,
    ) -> Result<AuthenticatedSession, StoreError> {
        self.transaction(self.inner.login_account(name, password))
            .await
    }

    async fn resolve_session(&self, token: &str) -> Result<AccountSnapshot, StoreError> {
        self.inner.resolve_session(token).await
    }

    async fn revoke_session(&self, token: &str) -> Result<(), StoreError> {
        self.transaction(self.inner.revoke_session(token)).await
    }

    #[cfg(test)]
    async fn load_or_create_account(&self, name: &str) -> Result<AccountSnapshot, StoreError> {
        self.transaction(self.inner.load_or_create_account(name))
            .await
    }

    async fn list_characters(
        &self,
        account_id: AccountId,
    ) -> Result<Vec<CharacterSnapshot>, StoreError> {
        self.inner.list_characters(account_id).await
    }

    #[cfg(test)]
    async fn load_or_create_character(
        &self,
        account_id: AccountId,
        name: &str,
        start_room: &str,
    ) -> Result<CharacterSnapshot, StoreError> {
        self.transaction(
            self.inner
                .load_or_create_character(account_id, name, start_room),
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
        self.transaction(self.inner.load_or_create_character_with_gameplay(
            account_id,
            name,
            start_room,
            initial_gameplay,
        ))
        .await
    }

    async fn load_character(
        &self,
        account_id: AccountId,
        character_id: CharacterId,
    ) -> Result<CharacterSnapshot, StoreError> {
        self.inner.load_character(account_id, character_id).await
    }

    async fn save(&self, snapshot: CharacterSnapshot) -> Result<(), StoreError> {
        self.transaction(self.inner.save(snapshot)).await
    }

    async fn load_character_gameplay(
        &self,
        character_id: CharacterId,
    ) -> Result<CharacterGameplaySnapshot, StoreError> {
        self.inner.load_character_gameplay(character_id).await
    }

    async fn ensure_character_ids_exist_batch(
        &self,
        character_ids: Vec<CharacterId>,
    ) -> Result<(), StoreError> {
        self.inner
            .ensure_character_ids_exist_batch(character_ids)
            .await
    }

    async fn load_character_gameplay_batch(
        &self,
        character_ids: Vec<CharacterId>,
        limits: StartupGameplayLoadLimits,
    ) -> Result<Vec<DurableCharacterGameplay>, StoreError> {
        self.inner
            .load_character_gameplay_batch(character_ids, limits)
            .await
    }

    async fn load_runtime_snapshot(&self) -> Result<Option<WorldRuntimeSnapshot>, StoreError> {
        self.inner.load_runtime_snapshot().await
    }

    async fn load_startup_world_recovery(&self) -> Result<StartupWorldRecovery, StoreError> {
        self.inner.load_startup_world_recovery().await
    }

    async fn has_durable_character_authority(&self) -> Result<bool, StoreError> {
        self.inner.has_durable_character_authority().await
    }

    async fn load_area_generation(&self) -> Result<Option<DurableAreaGeneration>, StoreError> {
        self.inner.load_area_generation().await
    }

    async fn bind_area_generation_if_absent(
        &self,
        generation: DurableAreaGeneration,
    ) -> Result<DurableAreaGeneration, StoreError> {
        self.transaction(self.inner.bind_area_generation_if_absent(generation))
            .await
    }

    async fn commit_area_generation_activation(
        &self,
        runtime: WorldRuntimeSnapshot,
        expected_current: DurableAreaGeneration,
        next: DurableAreaGeneration,
    ) -> Result<(), StoreError> {
        self.transaction(self.inner.commit_area_generation_activation(
            runtime,
            expected_current,
            next,
        ))
        .await
    }

    async fn load_area_checkpoints(&self) -> Result<Vec<DurableAreaCheckpoint>, StoreError> {
        self.inner.load_area_checkpoints().await
    }

    async fn commit_area_retirement(
        &self,
        runtime: WorldRuntimeSnapshot,
        expected_generation: DurableAreaGeneration,
        checkpoint: DurableAreaCheckpoint,
    ) -> Result<(), StoreError> {
        self.transaction(self.inner.commit_area_retirement(
            runtime,
            expected_generation,
            checkpoint,
        ))
        .await
    }

    async fn commit_area_rehydration(
        &self,
        runtime: WorldRuntimeSnapshot,
        expected_generation: DurableAreaGeneration,
        checkpoint: DurableAreaCheckpoint,
    ) -> Result<(), StoreError> {
        self.transaction(self.inner.commit_area_rehydration(
            runtime,
            expected_generation,
            checkpoint,
        ))
        .await
    }

    async fn load_domain_events(&self) -> Result<Vec<DomainEvent>, StoreError> {
        self.inner.load_domain_events().await
    }

    async fn social_profile(&self, character_id: CharacterId) -> Result<SocialProfile, StoreError> {
        self.inner.social_profile(character_id).await
    }

    async fn social_target(&self, name: &str) -> Result<SocialProfile, StoreError> {
        self.inner.social_target(name).await
    }

    async fn social_delivery_allowed(
        &self,
        sender: CharacterId,
        recipient: CharacterId,
    ) -> Result<bool, StoreError> {
        self.inner.social_delivery_allowed(sender, recipient).await
    }

    async fn set_social_relation(
        &self,
        actor: CharacterId,
        target: CharacterId,
        relation: SocialRelationKind,
        enabled: bool,
    ) -> Result<(), StoreError> {
        self.transaction(
            self.inner
                .set_social_relation(actor, target, relation, enabled),
        )
        .await
    }

    async fn social_channel_subscriptions(
        &self,
        character_id: CharacterId,
    ) -> Result<Vec<SocialChannelName>, StoreError> {
        self.inner.social_channel_subscriptions(character_id).await
    }

    async fn social_channel_subscribers(
        &self,
        channel: SocialChannelName,
    ) -> Result<Vec<CharacterId>, StoreError> {
        self.inner.social_channel_subscribers(channel).await
    }

    async fn set_social_channel_subscription(
        &self,
        character_id: CharacterId,
        channel: SocialChannelName,
        subscribed: bool,
    ) -> Result<(), StoreError> {
        self.transaction(self.inner.set_social_channel_subscription(
            character_id,
            channel,
            subscribed,
        ))
        .await
    }

    async fn send_mail(
        &self,
        sender: CharacterId,
        target_name: &str,
        public_id: &str,
        subject: &str,
        body: &str,
    ) -> Result<MailSendResult, StoreError> {
        self.transaction(
            self.inner
                .send_mail(sender, target_name, public_id, subject, body),
        )
        .await
    }

    async fn mailbox_page(
        &self,
        recipient: CharacterId,
        page: u32,
    ) -> Result<MailPage, StoreError> {
        self.inner.mailbox_page(recipient, page).await
    }

    async fn read_mail(
        &self,
        recipient: CharacterId,
        public_id: &str,
    ) -> Result<MailMessage, StoreError> {
        self.transaction(self.inner.read_mail(recipient, public_id))
            .await
    }

    async fn delete_mail(&self, recipient: CharacterId, public_id: &str) -> Result<(), StoreError> {
        self.transaction(self.inner.delete_mail(recipient, public_id))
            .await
    }

    async fn report_mail(
        &self,
        recipient: CharacterId,
        public_id: &str,
        reason: &str,
    ) -> Result<i64, StoreError> {
        self.transaction(self.inner.report_mail(recipient, public_id, reason))
            .await
    }

    async fn purge_expired_mail(&self) -> Result<u64, StoreError> {
        self.transaction(self.inner.purge_expired_mail()).await
    }

    async fn list_board_posts(
        &self,
        actor: CharacterId,
        board: BoardKey,
        page: u32,
    ) -> Result<BoardPage, StoreError> {
        self.inner.list_board_posts(actor, board, page).await
    }

    async fn read_board_post(
        &self,
        actor: CharacterId,
        public_id: &str,
    ) -> Result<BoardEntry, StoreError> {
        self.inner.read_board_post(actor, public_id).await
    }

    async fn publish_board_post(
        &self,
        actor: CharacterId,
        board: BoardKey,
        public_id: &str,
        subject: &str,
        body: &str,
    ) -> Result<BoardEntry, StoreError> {
        self.transaction(
            self.inner
                .publish_board_post(actor, board, public_id, subject, body),
        )
        .await
    }

    async fn delete_board_post(
        &self,
        actor: CharacterId,
        public_id: &str,
    ) -> Result<(), StoreError> {
        self.transaction(self.inner.delete_board_post(actor, public_id))
            .await
    }

    async fn report_board_post(
        &self,
        actor: CharacterId,
        public_id: &str,
        reason: &str,
    ) -> Result<i64, StoreError> {
        self.transaction(self.inner.report_board_post(actor, public_id, reason))
            .await
    }

    async fn purge_expired_boards(&self) -> Result<u64, StoreError> {
        self.transaction(self.inner.purge_expired_boards()).await
    }

    async fn report_character(
        &self,
        actor: CharacterId,
        target: CharacterId,
        reason: &str,
    ) -> Result<(), StoreError> {
        self.transaction(self.inner.report_character(actor, target, reason))
            .await
    }

    async fn moderate_character(
        &self,
        actor: CharacterId,
        target: CharacterId,
        action: ModerationAction,
    ) -> Result<(), StoreError> {
        self.transaction(self.inner.moderate_character(actor, target, action))
            .await
    }

    #[cfg(test)]
    async fn audit_log(&self) -> Result<Vec<AuditEntry>, StoreError> {
        self.inner.audit_log().await
    }

    async fn commit_action(
        &self,
        characters: Vec<DurableCharacterSnapshot>,
        runtime: WorldRuntimeSnapshot,
        events: Vec<DomainEvent>,
    ) -> Result<(), StoreError> {
        self.transaction(self.inner.commit_action(characters, runtime, events))
            .await
    }

    async fn commit_staff_action(
        &self,
        characters: Vec<DurableCharacterSnapshot>,
        runtime: WorldRuntimeSnapshot,
        events: Vec<DomainEvent>,
        audit: StaffAuditRecord,
    ) -> Result<(), StoreError> {
        self.transaction(
            self.inner
                .commit_staff_action(characters, runtime, events, audit),
        )
        .await
    }
}

pub(super) enum Request {
    Shutdown {
        reply: oneshot::Sender<Result<(), StoreError>>,
    },
    Health {
        reply: oneshot::Sender<Result<(), StoreError>>,
    },
    FinishRegisterAccount {
        name: String,
        password_hash: String,
        reply: oneshot::Sender<Result<AuthenticatedSession, StoreError>>,
    },
    PrepareLoginAccount {
        name: String,
        reply: oneshot::Sender<Result<Option<PreparedLogin>, StoreError>>,
    },
    FinishLoginAccount {
        prepared: PreparedLogin,
        reply: oneshot::Sender<Result<AuthenticatedSession, StoreError>>,
    },
    ResolveSession {
        token: String,
        reply: oneshot::Sender<Result<AccountSnapshot, StoreError>>,
    },
    RevokeSession {
        token: String,
        reply: oneshot::Sender<Result<(), StoreError>>,
    },
    #[cfg(test)]
    LoadOrCreateAccount {
        name: String,
        reply: oneshot::Sender<Result<AccountSnapshot, StoreError>>,
    },
    ListCharacters {
        account_id: AccountId,
        reply: oneshot::Sender<Result<Vec<CharacterSnapshot>, StoreError>>,
    },
    LoadOrCreateCharacter {
        account_id: AccountId,
        name: String,
        start_room: String,
        initial_gameplay: Box<CharacterGameplaySnapshot>,
        reply: oneshot::Sender<Result<CharacterSnapshot, StoreError>>,
    },
    LoadCharacter {
        account_id: AccountId,
        character_id: CharacterId,
        reply: oneshot::Sender<Result<CharacterSnapshot, StoreError>>,
    },
    #[allow(dead_code)]
    Save {
        snapshot: CharacterSnapshot,
        reply: oneshot::Sender<Result<(), StoreError>>,
    },
    LoadCharacterGameplay {
        character_id: CharacterId,
        reply: oneshot::Sender<Result<CharacterGameplaySnapshot, StoreError>>,
    },
    LoadCharacterGameplayBatch {
        character_ids: Vec<CharacterId>,
        limits: StartupGameplayLoadLimits,
        reply: oneshot::Sender<Result<Vec<DurableCharacterGameplay>, StoreError>>,
    },
    EnsureCharacterIdsExistBatch {
        character_ids: Vec<CharacterId>,
        reply: oneshot::Sender<Result<(), StoreError>>,
    },
    #[allow(dead_code)]
    LoadRuntimeSnapshot {
        reply: oneshot::Sender<Result<Option<WorldRuntimeSnapshot>, StoreError>>,
    },
    LoadStartupWorldRecovery {
        reply: oneshot::Sender<Result<StartupWorldRecovery, StoreError>>,
    },
    HasDurableCharacterAuthority {
        reply: oneshot::Sender<Result<bool, StoreError>>,
    },
    LoadAreaGeneration {
        reply: oneshot::Sender<Result<Option<DurableAreaGeneration>, StoreError>>,
    },
    BindAreaGenerationIfAbsent {
        generation: DurableAreaGeneration,
        reply: oneshot::Sender<Result<DurableAreaGeneration, StoreError>>,
    },
    CommitAreaGenerationActivation {
        runtime: WorldRuntimeSnapshot,
        expected_current: DurableAreaGeneration,
        next: DurableAreaGeneration,
        reply: oneshot::Sender<Result<(), StoreError>>,
    },
    LoadAreaCheckpoints {
        reply: oneshot::Sender<Result<Vec<DurableAreaCheckpoint>, StoreError>>,
    },
    CommitAreaRetirement {
        runtime: WorldRuntimeSnapshot,
        expected_generation: DurableAreaGeneration,
        checkpoint: DurableAreaCheckpoint,
        reply: oneshot::Sender<Result<(), StoreError>>,
    },
    CommitAreaRehydration {
        runtime: WorldRuntimeSnapshot,
        expected_generation: DurableAreaGeneration,
        checkpoint: DurableAreaCheckpoint,
        reply: oneshot::Sender<Result<(), StoreError>>,
    },
    #[allow(dead_code)]
    LoadDomainEvents {
        reply: oneshot::Sender<Result<Vec<DomainEvent>, StoreError>>,
    },
    CommitAction {
        characters: Vec<DurableCharacterSnapshot>,
        runtime: WorldRuntimeSnapshot,
        events: Vec<DomainEvent>,
        reply: oneshot::Sender<Result<(), StoreError>>,
    },
    CommitStaffAction {
        characters: Vec<DurableCharacterSnapshot>,
        runtime: WorldRuntimeSnapshot,
        events: Vec<DomainEvent>,
        audit: StaffAuditRecord,
        reply: oneshot::Sender<Result<(), StoreError>>,
    },
    SocialProfile {
        character_id: CharacterId,
        reply: oneshot::Sender<Result<SocialProfile, StoreError>>,
    },
    SocialTarget {
        name: String,
        reply: oneshot::Sender<Result<SocialProfile, StoreError>>,
    },
    SocialDeliveryAllowed {
        sender: CharacterId,
        recipient: CharacterId,
        reply: oneshot::Sender<Result<bool, StoreError>>,
    },
    SetSocialRelation {
        actor: CharacterId,
        target: CharacterId,
        relation: SocialRelationKind,
        enabled: bool,
        reply: oneshot::Sender<Result<(), StoreError>>,
    },
    SocialChannelSubscriptions {
        character_id: CharacterId,
        reply: oneshot::Sender<Result<Vec<SocialChannelName>, StoreError>>,
    },
    SocialChannelSubscribers {
        channel: SocialChannelName,
        reply: oneshot::Sender<Result<Vec<CharacterId>, StoreError>>,
    },
    SetSocialChannelSubscription {
        character_id: CharacterId,
        channel: SocialChannelName,
        subscribed: bool,
        reply: oneshot::Sender<Result<(), StoreError>>,
    },
    SendMail {
        sender: CharacterId,
        target_name: String,
        public_id: String,
        subject: String,
        body: String,
        reply: oneshot::Sender<Result<MailSendResult, StoreError>>,
    },
    MailboxPage {
        recipient: CharacterId,
        page: u32,
        reply: oneshot::Sender<Result<MailPage, StoreError>>,
    },
    ReadMail {
        recipient: CharacterId,
        public_id: String,
        reply: oneshot::Sender<Result<MailMessage, StoreError>>,
    },
    DeleteMail {
        recipient: CharacterId,
        public_id: String,
        reply: oneshot::Sender<Result<(), StoreError>>,
    },
    ReportMail {
        recipient: CharacterId,
        public_id: String,
        reason: String,
        reply: oneshot::Sender<Result<i64, StoreError>>,
    },
    PurgeExpiredMail {
        reply: oneshot::Sender<Result<u64, StoreError>>,
    },
    ListBoardPosts {
        actor: CharacterId,
        board: BoardKey,
        page: u32,
        reply: oneshot::Sender<Result<BoardPage, StoreError>>,
    },
    ReadBoardPost {
        actor: CharacterId,
        public_id: String,
        reply: oneshot::Sender<Result<BoardEntry, StoreError>>,
    },
    PublishBoardPost {
        actor: CharacterId,
        board: BoardKey,
        public_id: String,
        subject: String,
        body: String,
        reply: oneshot::Sender<Result<BoardEntry, StoreError>>,
    },
    DeleteBoardPost {
        actor: CharacterId,
        public_id: String,
        reply: oneshot::Sender<Result<(), StoreError>>,
    },
    ReportBoardPost {
        actor: CharacterId,
        public_id: String,
        reason: String,
        reply: oneshot::Sender<Result<i64, StoreError>>,
    },
    PurgeExpiredBoards {
        reply: oneshot::Sender<Result<u64, StoreError>>,
    },
    ReportCharacter {
        actor: CharacterId,
        target: CharacterId,
        reason: String,
        reply: oneshot::Sender<Result<(), StoreError>>,
    },
    ModerateCharacter {
        actor: CharacterId,
        target: CharacterId,
        action: ModerationAction,
        reply: oneshot::Sender<Result<(), StoreError>>,
    },
    StaffAccess {
        character_id: CharacterId,
        reply: oneshot::Sender<Result<StaffAccess, StoreError>>,
    },
    StaffReports {
        audit: StaffAuditRecord,
        reply: oneshot::Sender<Result<Vec<StaffReport>, StoreError>>,
    },
    ResolveStaffReport {
        report_id: i64,
        resolution: StaffReportResolution,
        resolution_reason: String,
        audit: StaffAuditRecord,
        reply: oneshot::Sender<Result<(), StoreError>>,
    },
    SetStaffProfile {
        target_account: AccountId,
        profile: Option<StaffProfileKey>,
        lore_title: Option<String>,
        audit: StaffAuditRecord,
        reply: oneshot::Sender<Result<(), StoreError>>,
    },
    SetStaffCapability {
        target_account: AccountId,
        capability: StaffCapability,
        enabled: bool,
        audit: StaffAuditRecord,
        reply: oneshot::Sender<Result<(), StoreError>>,
    },
    StaffAuditEntries {
        audit: StaffAuditRecord,
        reply: oneshot::Sender<Result<Vec<StaffAuditEntry>, StoreError>>,
    },
    AppendStaffAudit {
        audit: StaffAuditRecord,
        reply: oneshot::Sender<Result<(), StoreError>>,
    },
    #[cfg(test)]
    AuditLog {
        reply: oneshot::Sender<Result<Vec<AuditEntry>, StoreError>>,
    },
}

/// Per-backend command logic for the single shared writer-thread dispatch loop.
///
/// The SQLite and PostgreSQL adapters own byte-for-byte identical message
/// loops that differ only in (a) which borrow of the connection each helper
/// receives and (b) the SQL dialect of the inline `Health` probe. This trait
/// captures every command as one method whose body stays per-backend, so the
/// enum match and the uniform `reply.send(..)` plumbing live in exactly one
/// place ([`dispatch_persistence_request`]).
pub(super) trait PersistenceCommandHandler {
    fn health(&mut self) -> Result<(), StoreError>;
    fn finish_register_account(
        &mut self,
        name: String,
        password_hash: String,
    ) -> Result<AuthenticatedSession, StoreError>;
    fn prepare_login_account(&mut self, name: String) -> Result<Option<PreparedLogin>, StoreError>;
    fn finish_login_account(
        &mut self,
        prepared: PreparedLogin,
    ) -> Result<AuthenticatedSession, StoreError>;
    fn resolve_session(&mut self, token: String) -> Result<AccountSnapshot, StoreError>;
    fn revoke_session(&mut self, token: String) -> Result<(), StoreError>;
    #[cfg(test)]
    fn load_or_create_account(&mut self, name: String) -> Result<AccountSnapshot, StoreError>;
    fn list_characters(
        &mut self,
        account_id: AccountId,
    ) -> Result<Vec<CharacterSnapshot>, StoreError>;
    fn load_or_create_character(
        &mut self,
        account_id: AccountId,
        name: String,
        start_room: String,
        initial_gameplay: Box<CharacterGameplaySnapshot>,
    ) -> Result<CharacterSnapshot, StoreError>;
    fn load_character(
        &mut self,
        account_id: AccountId,
        character_id: CharacterId,
    ) -> Result<CharacterSnapshot, StoreError>;
    fn save(&mut self, snapshot: CharacterSnapshot) -> Result<(), StoreError>;
    fn load_character_gameplay(
        &mut self,
        character_id: CharacterId,
    ) -> Result<CharacterGameplaySnapshot, StoreError>;
    fn load_character_gameplay_batch(
        &mut self,
        character_ids: Vec<CharacterId>,
        limits: StartupGameplayLoadLimits,
    ) -> Result<Vec<DurableCharacterGameplay>, StoreError>;
    fn ensure_character_ids_exist_batch(
        &mut self,
        character_ids: Vec<CharacterId>,
    ) -> Result<(), StoreError>;
    fn load_runtime_snapshot(&mut self) -> Result<Option<WorldRuntimeSnapshot>, StoreError>;
    fn load_startup_world_recovery(&mut self) -> Result<StartupWorldRecovery, StoreError>;
    fn has_durable_character_authority(&mut self) -> Result<bool, StoreError>;
    fn load_area_generation(&mut self) -> Result<Option<DurableAreaGeneration>, StoreError>;
    fn bind_area_generation_if_absent(
        &mut self,
        generation: DurableAreaGeneration,
    ) -> Result<DurableAreaGeneration, StoreError>;
    fn commit_area_generation_activation(
        &mut self,
        runtime: WorldRuntimeSnapshot,
        expected_current: DurableAreaGeneration,
        next: DurableAreaGeneration,
    ) -> Result<(), StoreError>;
    fn load_area_checkpoints(&mut self) -> Result<Vec<DurableAreaCheckpoint>, StoreError>;
    fn commit_area_retirement(
        &mut self,
        runtime: WorldRuntimeSnapshot,
        expected_generation: DurableAreaGeneration,
        checkpoint: DurableAreaCheckpoint,
    ) -> Result<(), StoreError>;
    fn commit_area_rehydration(
        &mut self,
        runtime: WorldRuntimeSnapshot,
        expected_generation: DurableAreaGeneration,
        checkpoint: DurableAreaCheckpoint,
    ) -> Result<(), StoreError>;
    fn load_domain_events(&mut self) -> Result<Vec<DomainEvent>, StoreError>;
    fn commit_action(
        &mut self,
        characters: Vec<DurableCharacterSnapshot>,
        runtime: WorldRuntimeSnapshot,
        events: Vec<DomainEvent>,
    ) -> Result<(), StoreError>;
    fn commit_staff_action(
        &mut self,
        characters: Vec<DurableCharacterSnapshot>,
        runtime: WorldRuntimeSnapshot,
        events: Vec<DomainEvent>,
        audit: StaffAuditRecord,
    ) -> Result<(), StoreError>;
    fn social_profile(&mut self, character_id: CharacterId) -> Result<SocialProfile, StoreError>;
    fn social_target(&mut self, name: String) -> Result<SocialProfile, StoreError>;
    fn social_delivery_allowed(
        &mut self,
        sender: CharacterId,
        recipient: CharacterId,
    ) -> Result<bool, StoreError>;
    fn set_social_relation(
        &mut self,
        actor: CharacterId,
        target: CharacterId,
        relation: SocialRelationKind,
        enabled: bool,
    ) -> Result<(), StoreError>;
    fn social_channel_subscriptions(
        &mut self,
        character_id: CharacterId,
    ) -> Result<Vec<SocialChannelName>, StoreError>;
    fn social_channel_subscribers(
        &mut self,
        channel: SocialChannelName,
    ) -> Result<Vec<CharacterId>, StoreError>;
    fn set_social_channel_subscription(
        &mut self,
        character_id: CharacterId,
        channel: SocialChannelName,
        subscribed: bool,
    ) -> Result<(), StoreError>;
    fn send_mail(
        &mut self,
        sender: CharacterId,
        target_name: String,
        public_id: String,
        subject: String,
        body: String,
    ) -> Result<MailSendResult, StoreError>;
    fn mailbox_page(&mut self, recipient: CharacterId, page: u32) -> Result<MailPage, StoreError>;
    fn read_mail(
        &mut self,
        recipient: CharacterId,
        public_id: String,
    ) -> Result<MailMessage, StoreError>;
    fn delete_mail(&mut self, recipient: CharacterId, public_id: String) -> Result<(), StoreError>;
    fn report_mail(
        &mut self,
        recipient: CharacterId,
        public_id: String,
        reason: String,
    ) -> Result<i64, StoreError>;
    fn purge_expired_mail(&mut self) -> Result<u64, StoreError>;
    fn list_board_posts(
        &mut self,
        actor: CharacterId,
        board: BoardKey,
        page: u32,
    ) -> Result<BoardPage, StoreError>;
    fn read_board_post(
        &mut self,
        actor: CharacterId,
        public_id: String,
    ) -> Result<BoardEntry, StoreError>;
    fn publish_board_post(
        &mut self,
        actor: CharacterId,
        board: BoardKey,
        public_id: String,
        subject: String,
        body: String,
    ) -> Result<BoardEntry, StoreError>;
    fn delete_board_post(
        &mut self,
        actor: CharacterId,
        public_id: String,
    ) -> Result<(), StoreError>;
    fn report_board_post(
        &mut self,
        actor: CharacterId,
        public_id: String,
        reason: String,
    ) -> Result<i64, StoreError>;
    fn purge_expired_boards(&mut self) -> Result<u64, StoreError>;
    fn report_character(
        &mut self,
        actor: CharacterId,
        target: CharacterId,
        reason: String,
    ) -> Result<(), StoreError>;
    fn moderate_character(
        &mut self,
        actor: CharacterId,
        target: CharacterId,
        action: ModerationAction,
    ) -> Result<(), StoreError>;
    fn staff_access(&mut self, character_id: CharacterId) -> Result<StaffAccess, StoreError>;
    fn staff_reports(&mut self, audit: StaffAuditRecord) -> Result<Vec<StaffReport>, StoreError>;
    fn resolve_staff_report(
        &mut self,
        report_id: i64,
        resolution: StaffReportResolution,
        resolution_reason: String,
        audit: StaffAuditRecord,
    ) -> Result<(), StoreError>;
    fn set_staff_profile(
        &mut self,
        target_account: AccountId,
        profile: Option<StaffProfileKey>,
        lore_title: Option<String>,
        audit: StaffAuditRecord,
    ) -> Result<(), StoreError>;
    fn set_staff_capability(
        &mut self,
        target_account: AccountId,
        capability: StaffCapability,
        enabled: bool,
        audit: StaffAuditRecord,
    ) -> Result<(), StoreError>;
    fn staff_audit_entries(
        &mut self,
        audit: StaffAuditRecord,
    ) -> Result<Vec<StaffAuditEntry>, StoreError>;
    fn append_staff_audit(&mut self, audit: StaffAuditRecord) -> Result<(), StoreError>;
    #[cfg(test)]
    fn audit_log(&mut self) -> Result<Vec<AuditEntry>, StoreError>;
}

/// Owns the single writer-thread `match request { .. }` shared by every
/// backend. Each arm forwards to the handler and performs the uniform
/// `let _ = reply.send(..)` discard; `Shutdown` returns
/// [`ControlFlow::Break`] so the caller's loop stops and the handler (and its
/// connection) drops at scope end, exactly as the previous per-backend
/// `drop(connection); return;` did.
pub(super) fn dispatch_persistence_request<H: PersistenceCommandHandler>(
    handler: &mut H,
    request: Request,
) -> ControlFlow<()> {
    match request {
        Request::Shutdown { reply } => {
            let _ = reply.send(Ok(()));
            return ControlFlow::Break(());
        }
        Request::Health { reply } => {
            let _ = reply.send(handler.health());
        }
        Request::FinishRegisterAccount {
            name,
            password_hash,
            reply,
        } => {
            let _ = reply.send(handler.finish_register_account(name, password_hash));
        }
        Request::PrepareLoginAccount { name, reply } => {
            let _ = reply.send(handler.prepare_login_account(name));
        }
        Request::FinishLoginAccount { prepared, reply } => {
            let _ = reply.send(handler.finish_login_account(prepared));
        }
        Request::ResolveSession { token, reply } => {
            let _ = reply.send(handler.resolve_session(token));
        }
        Request::RevokeSession { token, reply } => {
            let _ = reply.send(handler.revoke_session(token));
        }
        #[cfg(test)]
        Request::LoadOrCreateAccount { name, reply } => {
            let _ = reply.send(handler.load_or_create_account(name));
        }
        Request::ListCharacters { account_id, reply } => {
            let _ = reply.send(handler.list_characters(account_id));
        }
        Request::LoadOrCreateCharacter {
            account_id,
            name,
            start_room,
            initial_gameplay,
            reply,
        } => {
            let _ = reply.send(handler.load_or_create_character(
                account_id,
                name,
                start_room,
                initial_gameplay,
            ));
        }
        Request::LoadCharacter {
            account_id,
            character_id,
            reply,
        } => {
            let _ = reply.send(handler.load_character(account_id, character_id));
        }
        Request::Save { snapshot, reply } => {
            let _ = reply.send(handler.save(snapshot));
        }
        Request::LoadCharacterGameplay {
            character_id,
            reply,
        } => {
            let _ = reply.send(handler.load_character_gameplay(character_id));
        }
        Request::LoadCharacterGameplayBatch {
            character_ids,
            limits,
            reply,
        } => {
            let _ = reply.send(handler.load_character_gameplay_batch(character_ids, limits));
        }
        Request::EnsureCharacterIdsExistBatch {
            character_ids,
            reply,
        } => {
            let _ = reply.send(handler.ensure_character_ids_exist_batch(character_ids));
        }
        Request::LoadRuntimeSnapshot { reply } => {
            let _ = reply.send(handler.load_runtime_snapshot());
        }
        Request::LoadStartupWorldRecovery { reply } => {
            let _ = reply.send(handler.load_startup_world_recovery());
        }
        Request::HasDurableCharacterAuthority { reply } => {
            let _ = reply.send(handler.has_durable_character_authority());
        }
        Request::LoadAreaGeneration { reply } => {
            let _ = reply.send(handler.load_area_generation());
        }
        Request::BindAreaGenerationIfAbsent { generation, reply } => {
            let _ = reply.send(handler.bind_area_generation_if_absent(generation));
        }
        Request::CommitAreaGenerationActivation {
            runtime,
            expected_current,
            next,
            reply,
        } => {
            let _ = reply.send(handler.commit_area_generation_activation(
                runtime,
                expected_current,
                next,
            ));
        }
        Request::LoadAreaCheckpoints { reply } => {
            let _ = reply.send(handler.load_area_checkpoints());
        }
        Request::CommitAreaRetirement {
            runtime,
            expected_generation,
            checkpoint,
            reply,
        } => {
            let _ = reply.send(handler.commit_area_retirement(
                runtime,
                expected_generation,
                checkpoint,
            ));
        }
        Request::CommitAreaRehydration {
            runtime,
            expected_generation,
            checkpoint,
            reply,
        } => {
            let _ = reply.send(handler.commit_area_rehydration(
                runtime,
                expected_generation,
                checkpoint,
            ));
        }
        Request::LoadDomainEvents { reply } => {
            let _ = reply.send(handler.load_domain_events());
        }
        Request::CommitAction {
            characters,
            runtime,
            events,
            reply,
        } => {
            let _ = reply.send(handler.commit_action(characters, runtime, events));
        }
        Request::CommitStaffAction {
            characters,
            runtime,
            events,
            audit,
            reply,
        } => {
            let _ = reply.send(handler.commit_staff_action(characters, runtime, events, audit));
        }
        Request::SocialProfile {
            character_id,
            reply,
        } => {
            let _ = reply.send(handler.social_profile(character_id));
        }
        Request::SocialTarget { name, reply } => {
            let _ = reply.send(handler.social_target(name));
        }
        Request::SocialDeliveryAllowed {
            sender,
            recipient,
            reply,
        } => {
            let _ = reply.send(handler.social_delivery_allowed(sender, recipient));
        }
        Request::SetSocialRelation {
            actor,
            target,
            relation,
            enabled,
            reply,
        } => {
            let _ = reply.send(handler.set_social_relation(actor, target, relation, enabled));
        }
        Request::SocialChannelSubscriptions {
            character_id,
            reply,
        } => {
            let _ = reply.send(handler.social_channel_subscriptions(character_id));
        }
        Request::SocialChannelSubscribers { channel, reply } => {
            let _ = reply.send(handler.social_channel_subscribers(channel));
        }
        Request::SetSocialChannelSubscription {
            character_id,
            channel,
            subscribed,
            reply,
        } => {
            let _ = reply.send(handler.set_social_channel_subscription(
                character_id,
                channel,
                subscribed,
            ));
        }
        Request::SendMail {
            sender,
            target_name,
            public_id,
            subject,
            body,
            reply,
        } => {
            let _ = reply.send(handler.send_mail(sender, target_name, public_id, subject, body));
        }
        Request::MailboxPage {
            recipient,
            page,
            reply,
        } => {
            let _ = reply.send(handler.mailbox_page(recipient, page));
        }
        Request::ReadMail {
            recipient,
            public_id,
            reply,
        } => {
            let _ = reply.send(handler.read_mail(recipient, public_id));
        }
        Request::DeleteMail {
            recipient,
            public_id,
            reply,
        } => {
            let _ = reply.send(handler.delete_mail(recipient, public_id));
        }
        Request::ReportMail {
            recipient,
            public_id,
            reason,
            reply,
        } => {
            let _ = reply.send(handler.report_mail(recipient, public_id, reason));
        }
        Request::PurgeExpiredMail { reply } => {
            let _ = reply.send(handler.purge_expired_mail());
        }
        Request::ListBoardPosts {
            actor,
            board,
            page,
            reply,
        } => {
            let _ = reply.send(handler.list_board_posts(actor, board, page));
        }
        Request::ReadBoardPost {
            actor,
            public_id,
            reply,
        } => {
            let _ = reply.send(handler.read_board_post(actor, public_id));
        }
        Request::PublishBoardPost {
            actor,
            board,
            public_id,
            subject,
            body,
            reply,
        } => {
            let _ = reply.send(handler.publish_board_post(actor, board, public_id, subject, body));
        }
        Request::DeleteBoardPost {
            actor,
            public_id,
            reply,
        } => {
            let _ = reply.send(handler.delete_board_post(actor, public_id));
        }
        Request::ReportBoardPost {
            actor,
            public_id,
            reason,
            reply,
        } => {
            let _ = reply.send(handler.report_board_post(actor, public_id, reason));
        }
        Request::PurgeExpiredBoards { reply } => {
            let _ = reply.send(handler.purge_expired_boards());
        }
        Request::ReportCharacter {
            actor,
            target,
            reason,
            reply,
        } => {
            let _ = reply.send(handler.report_character(actor, target, reason));
        }
        Request::ModerateCharacter {
            actor,
            target,
            action,
            reply,
        } => {
            let _ = reply.send(handler.moderate_character(actor, target, action));
        }
        Request::StaffAccess {
            character_id,
            reply,
        } => {
            let _ = reply.send(handler.staff_access(character_id));
        }
        Request::StaffReports { audit, reply } => {
            let _ = reply.send(handler.staff_reports(audit));
        }
        Request::ResolveStaffReport {
            report_id,
            resolution,
            resolution_reason,
            audit,
            reply,
        } => {
            let _ = reply.send(handler.resolve_staff_report(
                report_id,
                resolution,
                resolution_reason,
                audit,
            ));
        }
        Request::SetStaffProfile {
            target_account,
            profile,
            lore_title,
            audit,
            reply,
        } => {
            let _ =
                reply.send(handler.set_staff_profile(target_account, profile, lore_title, audit));
        }
        Request::SetStaffCapability {
            target_account,
            capability,
            enabled,
            audit,
            reply,
        } => {
            let _ = reply.send(handler.set_staff_capability(
                target_account,
                capability,
                enabled,
                audit,
            ));
        }
        Request::StaffAuditEntries { audit, reply } => {
            let _ = reply.send(handler.staff_audit_entries(audit));
        }
        Request::AppendStaffAudit { audit, reply } => {
            let _ = reply.send(handler.append_staff_audit(audit));
        }
        #[cfg(test)]
        Request::AuditLog { reply } => {
            let _ = reply.send(handler.audit_log());
        }
    }
    ControlFlow::Continue(())
}

/// SQLite handler: each method reproduces the exact call the writer loop used
/// to inline, including the shared (`&`) vs mutable (`&mut`) connection borrow
/// per command and the SQLite-dialect `Health` probe.
struct SqliteCommandHandler {
    connection: Connection,
}

impl PersistenceCommandHandler for SqliteCommandHandler {
    fn health(&mut self) -> Result<(), StoreError> {
        self.connection
            .query_row("SELECT 1", [], |row| row.get::<_, i64>(0))
            .map(|_| ())
            .map_err(StoreError::from)
    }

    fn finish_register_account(
        &mut self,
        name: String,
        password_hash: String,
    ) -> Result<AuthenticatedSession, StoreError> {
        finish_register_account(
            &mut self.connection,
            &name,
            &password_hash,
            unix_timestamp(),
        )
    }

    fn prepare_login_account(&mut self, name: String) -> Result<Option<PreparedLogin>, StoreError> {
        prepare_login_account(&self.connection, &name)
    }

    fn finish_login_account(
        &mut self,
        prepared: PreparedLogin,
    ) -> Result<AuthenticatedSession, StoreError> {
        finish_login_account(&mut self.connection, &prepared, unix_timestamp())
    }

    fn resolve_session(&mut self, token: String) -> Result<AccountSnapshot, StoreError> {
        resolve_session(&self.connection, &token, unix_timestamp())
    }

    fn revoke_session(&mut self, token: String) -> Result<(), StoreError> {
        revoke_session(&self.connection, &token, unix_timestamp())
    }

    #[cfg(test)]
    fn load_or_create_account(&mut self, name: String) -> Result<AccountSnapshot, StoreError> {
        load_or_create_account(&self.connection, &name)
    }

    fn list_characters(
        &mut self,
        account_id: AccountId,
    ) -> Result<Vec<CharacterSnapshot>, StoreError> {
        list_characters(&self.connection, account_id)
    }

    fn load_or_create_character(
        &mut self,
        account_id: AccountId,
        name: String,
        start_room: String,
        initial_gameplay: Box<CharacterGameplaySnapshot>,
    ) -> Result<CharacterSnapshot, StoreError> {
        load_or_create_character_with_gameplay(
            &self.connection,
            account_id,
            &name,
            &start_room,
            &initial_gameplay,
        )
    }

    fn load_character(
        &mut self,
        account_id: AccountId,
        character_id: CharacterId,
    ) -> Result<CharacterSnapshot, StoreError> {
        load_character(&self.connection, account_id, character_id)
    }

    fn save(&mut self, snapshot: CharacterSnapshot) -> Result<(), StoreError> {
        save(&self.connection, &snapshot)
    }

    fn load_character_gameplay(
        &mut self,
        character_id: CharacterId,
    ) -> Result<CharacterGameplaySnapshot, StoreError> {
        load_character_gameplay(&self.connection, character_id)
    }

    fn load_character_gameplay_batch(
        &mut self,
        character_ids: Vec<CharacterId>,
        limits: StartupGameplayLoadLimits,
    ) -> Result<Vec<DurableCharacterGameplay>, StoreError> {
        load_character_gameplay_batch(&self.connection, character_ids, limits)
    }

    fn ensure_character_ids_exist_batch(
        &mut self,
        character_ids: Vec<CharacterId>,
    ) -> Result<(), StoreError> {
        ensure_character_ids_exist_batch(&self.connection, character_ids)
    }

    fn load_runtime_snapshot(&mut self) -> Result<Option<WorldRuntimeSnapshot>, StoreError> {
        load_runtime_snapshot(&self.connection)
    }

    fn load_startup_world_recovery(&mut self) -> Result<StartupWorldRecovery, StoreError> {
        load_startup_world_recovery(&mut self.connection)
    }

    fn has_durable_character_authority(&mut self) -> Result<bool, StoreError> {
        has_durable_character_authority(&self.connection)
    }

    fn load_area_generation(&mut self) -> Result<Option<DurableAreaGeneration>, StoreError> {
        load_area_generation(&self.connection)
    }

    fn bind_area_generation_if_absent(
        &mut self,
        generation: DurableAreaGeneration,
    ) -> Result<DurableAreaGeneration, StoreError> {
        bind_area_generation_if_absent(&mut self.connection, &generation)
    }

    fn commit_area_generation_activation(
        &mut self,
        runtime: WorldRuntimeSnapshot,
        expected_current: DurableAreaGeneration,
        next: DurableAreaGeneration,
    ) -> Result<(), StoreError> {
        commit_area_generation_activation(&mut self.connection, &runtime, &expected_current, &next)
    }

    fn load_area_checkpoints(&mut self) -> Result<Vec<DurableAreaCheckpoint>, StoreError> {
        load_area_checkpoints(&self.connection)
    }

    fn commit_area_retirement(
        &mut self,
        runtime: WorldRuntimeSnapshot,
        expected_generation: DurableAreaGeneration,
        checkpoint: DurableAreaCheckpoint,
    ) -> Result<(), StoreError> {
        commit_area_retirement(
            &mut self.connection,
            &runtime,
            &expected_generation,
            &checkpoint,
        )
    }

    fn commit_area_rehydration(
        &mut self,
        runtime: WorldRuntimeSnapshot,
        expected_generation: DurableAreaGeneration,
        checkpoint: DurableAreaCheckpoint,
    ) -> Result<(), StoreError> {
        commit_area_rehydration(
            &mut self.connection,
            &runtime,
            &expected_generation,
            &checkpoint,
        )
    }

    fn load_domain_events(&mut self) -> Result<Vec<DomainEvent>, StoreError> {
        load_domain_events(&self.connection)
    }

    fn commit_action(
        &mut self,
        characters: Vec<DurableCharacterSnapshot>,
        runtime: WorldRuntimeSnapshot,
        events: Vec<DomainEvent>,
    ) -> Result<(), StoreError> {
        commit_action(&mut self.connection, &characters, &runtime, &events)
    }

    fn commit_staff_action(
        &mut self,
        characters: Vec<DurableCharacterSnapshot>,
        runtime: WorldRuntimeSnapshot,
        events: Vec<DomainEvent>,
        audit: StaffAuditRecord,
    ) -> Result<(), StoreError> {
        commit_staff_action(
            &mut self.connection,
            &characters,
            &runtime,
            &events,
            audit,
            unix_timestamp(),
        )
    }

    fn social_profile(&mut self, character_id: CharacterId) -> Result<SocialProfile, StoreError> {
        social_profile(&self.connection, character_id, unix_timestamp())
    }

    fn social_target(&mut self, name: String) -> Result<SocialProfile, StoreError> {
        social_target(&self.connection, &name, unix_timestamp())
    }

    fn social_delivery_allowed(
        &mut self,
        sender: CharacterId,
        recipient: CharacterId,
    ) -> Result<bool, StoreError> {
        social_delivery_allowed(&self.connection, sender, recipient)
    }

    fn set_social_relation(
        &mut self,
        actor: CharacterId,
        target: CharacterId,
        relation: SocialRelationKind,
        enabled: bool,
    ) -> Result<(), StoreError> {
        set_social_relation(
            &mut self.connection,
            actor,
            target,
            relation,
            enabled,
            unix_timestamp(),
        )
    }

    fn social_channel_subscriptions(
        &mut self,
        character_id: CharacterId,
    ) -> Result<Vec<SocialChannelName>, StoreError> {
        social_channel_subscriptions(&self.connection, character_id)
    }

    fn social_channel_subscribers(
        &mut self,
        channel: SocialChannelName,
    ) -> Result<Vec<CharacterId>, StoreError> {
        social_channel_subscribers(&self.connection, channel)
    }

    fn set_social_channel_subscription(
        &mut self,
        character_id: CharacterId,
        channel: SocialChannelName,
        subscribed: bool,
    ) -> Result<(), StoreError> {
        set_social_channel_subscription(
            &mut self.connection,
            character_id,
            channel,
            subscribed,
            unix_timestamp(),
        )
    }

    fn send_mail(
        &mut self,
        sender: CharacterId,
        target_name: String,
        public_id: String,
        subject: String,
        body: String,
    ) -> Result<MailSendResult, StoreError> {
        send_mail(
            &mut self.connection,
            sender,
            &target_name,
            &public_id,
            &subject,
            &body,
            unix_timestamp(),
        )
    }

    fn mailbox_page(&mut self, recipient: CharacterId, page: u32) -> Result<MailPage, StoreError> {
        mailbox_page(&self.connection, recipient, page, unix_timestamp())
    }

    fn read_mail(
        &mut self,
        recipient: CharacterId,
        public_id: String,
    ) -> Result<MailMessage, StoreError> {
        read_mail(
            &mut self.connection,
            recipient,
            &public_id,
            unix_timestamp(),
        )
    }

    fn delete_mail(&mut self, recipient: CharacterId, public_id: String) -> Result<(), StoreError> {
        delete_mail(
            &mut self.connection,
            recipient,
            &public_id,
            unix_timestamp(),
        )
    }

    fn report_mail(
        &mut self,
        recipient: CharacterId,
        public_id: String,
        reason: String,
    ) -> Result<i64, StoreError> {
        report_mail(
            &mut self.connection,
            recipient,
            &public_id,
            &reason,
            unix_timestamp(),
        )
    }

    fn purge_expired_mail(&mut self) -> Result<u64, StoreError> {
        purge_expired_mail(&mut self.connection, unix_timestamp())
    }

    fn list_board_posts(
        &mut self,
        actor: CharacterId,
        board: BoardKey,
        page: u32,
    ) -> Result<BoardPage, StoreError> {
        list_board_posts(&self.connection, actor, board, page, unix_timestamp())
    }

    fn read_board_post(
        &mut self,
        actor: CharacterId,
        public_id: String,
    ) -> Result<BoardEntry, StoreError> {
        read_board_post(&self.connection, actor, &public_id, unix_timestamp())
    }

    fn publish_board_post(
        &mut self,
        actor: CharacterId,
        board: BoardKey,
        public_id: String,
        subject: String,
        body: String,
    ) -> Result<BoardEntry, StoreError> {
        publish_board_post(
            &mut self.connection,
            actor,
            board,
            &public_id,
            &subject,
            &body,
            unix_timestamp(),
        )
    }

    fn delete_board_post(
        &mut self,
        actor: CharacterId,
        public_id: String,
    ) -> Result<(), StoreError> {
        delete_board_post(&mut self.connection, actor, &public_id, unix_timestamp())
    }

    fn report_board_post(
        &mut self,
        actor: CharacterId,
        public_id: String,
        reason: String,
    ) -> Result<i64, StoreError> {
        report_board_post(
            &mut self.connection,
            actor,
            &public_id,
            &reason,
            unix_timestamp(),
        )
    }

    fn purge_expired_boards(&mut self) -> Result<u64, StoreError> {
        purge_expired_boards(&mut self.connection, unix_timestamp())
    }

    fn report_character(
        &mut self,
        actor: CharacterId,
        target: CharacterId,
        reason: String,
    ) -> Result<(), StoreError> {
        report_character(
            &mut self.connection,
            actor,
            target,
            &reason,
            unix_timestamp(),
        )
    }

    fn moderate_character(
        &mut self,
        actor: CharacterId,
        target: CharacterId,
        action: ModerationAction,
    ) -> Result<(), StoreError> {
        moderate_character(
            &mut self.connection,
            actor,
            target,
            action,
            unix_timestamp(),
        )
    }

    fn staff_access(&mut self, character_id: CharacterId) -> Result<StaffAccess, StoreError> {
        staff_access(&self.connection, character_id)
    }

    fn staff_reports(&mut self, audit: StaffAuditRecord) -> Result<Vec<StaffReport>, StoreError> {
        staff_reports(&mut self.connection, audit, unix_timestamp())
    }

    fn resolve_staff_report(
        &mut self,
        report_id: i64,
        resolution: StaffReportResolution,
        resolution_reason: String,
        audit: StaffAuditRecord,
    ) -> Result<(), StoreError> {
        resolve_staff_report(
            &mut self.connection,
            report_id,
            resolution,
            &resolution_reason,
            audit,
            unix_timestamp(),
        )
    }

    fn set_staff_profile(
        &mut self,
        target_account: AccountId,
        profile: Option<StaffProfileKey>,
        lore_title: Option<String>,
        audit: StaffAuditRecord,
    ) -> Result<(), StoreError> {
        set_staff_profile(
            &mut self.connection,
            target_account,
            profile,
            lore_title.as_deref(),
            audit,
            unix_timestamp(),
        )
    }

    fn set_staff_capability(
        &mut self,
        target_account: AccountId,
        capability: StaffCapability,
        enabled: bool,
        audit: StaffAuditRecord,
    ) -> Result<(), StoreError> {
        set_staff_capability(
            &mut self.connection,
            target_account,
            capability,
            enabled,
            audit,
            unix_timestamp(),
        )
    }

    fn staff_audit_entries(
        &mut self,
        audit: StaffAuditRecord,
    ) -> Result<Vec<StaffAuditEntry>, StoreError> {
        staff_audit_entries(&mut self.connection, audit, unix_timestamp())
    }

    fn append_staff_audit(&mut self, audit: StaffAuditRecord) -> Result<(), StoreError> {
        append_staff_audit_transaction(&mut self.connection, audit, unix_timestamp())
    }

    #[cfg(test)]
    fn audit_log(&mut self) -> Result<Vec<AuditEntry>, StoreError> {
        audit_log(&self.connection)
    }
}

#[derive(Clone)]
pub(super) struct PreparedLogin {
    pub(super) account_id: AccountId,
    pub(super) encoded_hash: String,
}

/// SQLite adapter with one dedicated owner thread.
///
/// `rusqlite::Connection` never crosses thread boundaries and gameplay tasks do
/// not block a Tokio executor while SQLite is writing.
#[derive(Clone)]
pub struct SqliteCharacterStore {
    tx: mpsc::Sender<Request>,
    passwords: PasswordService,
    #[cfg(test)]
    area_generation_fault: Arc<AtomicU8>,
    #[cfg(test)]
    area_generation_commit_gate: Arc<TestAreaGenerationCommitGate>,
}

#[cfg(test)]
struct TestAreaGenerationCommitGate {
    armed: AtomicBool,
    started: AtomicBool,
    released: AtomicBool,
    changed: Notify,
}

#[cfg(test)]
impl TestAreaGenerationCommitGate {
    fn new() -> Self {
        Self {
            armed: AtomicBool::new(false),
            started: AtomicBool::new(false),
            released: AtomicBool::new(true),
            changed: Notify::new(),
        }
    }

    fn arm(&self) {
        self.started.store(false, Ordering::SeqCst);
        self.released.store(false, Ordering::SeqCst);
        self.armed.store(true, Ordering::SeqCst);
    }

    async fn block_if_armed(&self) {
        if self
            .armed
            .compare_exchange(true, false, Ordering::SeqCst, Ordering::SeqCst)
            .is_err()
        {
            return;
        }
        self.started.store(true, Ordering::SeqCst);
        self.changed.notify_waiters();
        loop {
            let changed = self.changed.notified();
            if self.released.load(Ordering::SeqCst) {
                return;
            }
            changed.await;
        }
    }

    async fn wait_until_started(&self) {
        loop {
            let changed = self.changed.notified();
            if self.started.load(Ordering::SeqCst) {
                return;
            }
            changed.await;
        }
    }

    fn release(&self) {
        self.released.store(true, Ordering::SeqCst);
        self.changed.notify_waiters();
    }
}

impl SqliteCharacterStore {
    pub async fn open(path: impl AsRef<Path>) -> Result<Self, StoreError> {
        Self::open_with_password_service(path, PasswordService::default()).await
    }

    async fn open_with_password_service(
        path: impl AsRef<Path>,
        passwords: PasswordService,
    ) -> Result<Self, StoreError> {
        let path = path.as_ref().to_owned();
        // Backpressure here prevents an accidental caller from building an
        // unbounded heap queue while the database is busy.
        let (tx, mut rx) = mpsc::channel(PERSISTENCE_REQUEST_QUEUE_CAPACITY);
        let (ready_tx, ready_rx) = oneshot::channel();

        thread::Builder::new()
            .name("sqlite-writer".into())
            .spawn(move || {
                let connection = match (|| -> Result<Connection, StoreError> {
                    let mut connection = Connection::open(path)?;
                    initialise(&mut connection)?;
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

                let mut handler = SqliteCommandHandler { connection };
                while let Some(request) = rx.blocking_recv() {
                    if dispatch_persistence_request(&mut handler, request).is_break() {
                        break;
                    }
                }
            })
            .map_err(StoreError::ThreadSpawn)?;

        ready_rx.await.map_err(|_| StoreError::WorkerStopped)??;
        Ok(Self {
            tx,
            passwords,
            #[cfg(test)]
            area_generation_fault: Arc::new(AtomicU8::new(0)),
            #[cfg(test)]
            area_generation_commit_gate: Arc::new(TestAreaGenerationCommitGate::new()),
        })
    }

    #[cfg(test)]
    pub(crate) fn simulate_next_area_generation_commit_ack_loss(&self) {
        self.area_generation_fault.store(1, Ordering::SeqCst);
    }

    #[cfg(test)]
    pub(crate) fn simulate_ambiguous_area_generation_commit_failure(&self) {
        self.area_generation_fault.store(2, Ordering::SeqCst);
    }

    #[cfg(test)]
    pub(crate) fn block_next_area_generation_commit(&self) {
        self.area_generation_commit_gate.arm();
    }

    #[cfg(test)]
    pub(crate) async fn wait_for_blocked_area_generation_commit(&self) {
        self.area_generation_commit_gate.wait_until_started().await;
    }

    #[cfg(test)]
    pub(crate) fn release_blocked_area_generation_commit(&self) {
        self.area_generation_commit_gate.release();
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

    /// Bound admission, but once a state-changing request is accepted, wait
    /// for its definitive result. A response deadline here could report a
    /// timeout while the owner thread subsequently commits the mutation.
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
impl CharacterStore for SqliteCharacterStore {
    fn backend_name(&self) -> &'static str {
        "sqlite"
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

    async fn load_character_gameplay_batch(
        &self,
        character_ids: Vec<CharacterId>,
        limits: StartupGameplayLoadLimits,
    ) -> Result<Vec<DurableCharacterGameplay>, StoreError> {
        let (reply, receive) = oneshot::channel();
        self.request(
            Request::LoadCharacterGameplayBatch {
                character_ids,
                limits,
                reply,
            },
            receive,
        )
        .await
    }

    async fn ensure_character_ids_exist_batch(
        &self,
        character_ids: Vec<CharacterId>,
    ) -> Result<(), StoreError> {
        let (reply, receive) = oneshot::channel();
        self.request(
            Request::EnsureCharacterIdsExistBatch {
                character_ids,
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

    async fn load_startup_world_recovery(&self) -> Result<StartupWorldRecovery, StoreError> {
        let (reply, receive) = oneshot::channel();
        self.request(Request::LoadStartupWorldRecovery { reply }, receive)
            .await
    }

    async fn has_durable_character_authority(&self) -> Result<bool, StoreError> {
        let (reply, receive) = oneshot::channel();
        self.request(Request::HasDurableCharacterAuthority { reply }, receive)
            .await
    }

    async fn load_area_generation(&self) -> Result<Option<DurableAreaGeneration>, StoreError> {
        #[cfg(test)]
        if self
            .area_generation_fault
            .compare_exchange(3, 0, Ordering::SeqCst, Ordering::SeqCst)
            .is_ok()
        {
            return Err(StoreError::WorkerStopped);
        }
        let (reply, receive) = oneshot::channel();
        self.request(Request::LoadAreaGeneration { reply }, receive)
            .await
    }

    async fn bind_area_generation_if_absent(
        &self,
        generation: DurableAreaGeneration,
    ) -> Result<DurableAreaGeneration, StoreError> {
        let (reply, receive) = oneshot::channel();
        self.request_commit(
            Request::BindAreaGenerationIfAbsent { generation, reply },
            receive,
        )
        .await
    }

    async fn commit_area_generation_activation(
        &self,
        runtime: WorldRuntimeSnapshot,
        expected_current: DurableAreaGeneration,
        next: DurableAreaGeneration,
    ) -> Result<(), StoreError> {
        #[cfg(test)]
        self.area_generation_commit_gate.block_if_armed().await;
        #[cfg(test)]
        let injected_fault = self.area_generation_fault.load(Ordering::SeqCst);
        #[cfg(test)]
        if injected_fault == 2 {
            self.area_generation_fault.store(3, Ordering::SeqCst);
            return Err(StoreError::WorkerStopped);
        }
        let (reply, receive) = oneshot::channel();
        let result = self
            .request_commit(
                Request::CommitAreaGenerationActivation {
                    runtime,
                    expected_current,
                    next,
                    reply,
                },
                receive,
            )
            .await;
        #[cfg(test)]
        if injected_fault == 1 && result.is_ok() {
            self.area_generation_fault.store(0, Ordering::SeqCst);
            return Err(StoreError::WorkerStopped);
        }
        result
    }

    async fn load_area_checkpoints(&self) -> Result<Vec<DurableAreaCheckpoint>, StoreError> {
        let (reply, receive) = oneshot::channel();
        self.request(Request::LoadAreaCheckpoints { reply }, receive)
            .await
    }

    async fn commit_area_retirement(
        &self,
        runtime: WorldRuntimeSnapshot,
        expected_generation: DurableAreaGeneration,
        checkpoint: DurableAreaCheckpoint,
    ) -> Result<(), StoreError> {
        let (reply, receive) = oneshot::channel();
        self.request_commit(
            Request::CommitAreaRetirement {
                runtime,
                expected_generation,
                checkpoint,
                reply,
            },
            receive,
        )
        .await
    }

    async fn commit_area_rehydration(
        &self,
        runtime: WorldRuntimeSnapshot,
        expected_generation: DurableAreaGeneration,
        checkpoint: DurableAreaCheckpoint,
    ) -> Result<(), StoreError> {
        let (reply, receive) = oneshot::channel();
        self.request_commit(
            Request::CommitAreaRehydration {
                runtime,
                expected_generation,
                checkpoint,
                reply,
            },
            receive,
        )
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

    async fn social_channel_subscriptions(
        &self,
        character_id: CharacterId,
    ) -> Result<Vec<SocialChannelName>, StoreError> {
        let (reply, receive) = oneshot::channel();
        self.request(
            Request::SocialChannelSubscriptions {
                character_id,
                reply,
            },
            receive,
        )
        .await
    }

    async fn social_channel_subscribers(
        &self,
        channel: SocialChannelName,
    ) -> Result<Vec<CharacterId>, StoreError> {
        let (reply, receive) = oneshot::channel();
        self.request(
            Request::SocialChannelSubscribers { channel, reply },
            receive,
        )
        .await
    }

    async fn set_social_channel_subscription(
        &self,
        character_id: CharacterId,
        channel: SocialChannelName,
        subscribed: bool,
    ) -> Result<(), StoreError> {
        let (reply, receive) = oneshot::channel();
        self.request_commit(
            Request::SetSocialChannelSubscription {
                character_id,
                channel,
                subscribed,
                reply,
            },
            receive,
        )
        .await
    }

    async fn send_mail(
        &self,
        sender: CharacterId,
        target_name: &str,
        public_id: &str,
        subject: &str,
        body: &str,
    ) -> Result<MailSendResult, StoreError> {
        let (reply, receive) = oneshot::channel();
        self.request_commit(
            Request::SendMail {
                sender,
                target_name: target_name.to_owned(),
                public_id: public_id.to_owned(),
                subject: subject.to_owned(),
                body: body.to_owned(),
                reply,
            },
            receive,
        )
        .await
    }

    async fn mailbox_page(
        &self,
        recipient: CharacterId,
        page: u32,
    ) -> Result<MailPage, StoreError> {
        let (reply, receive) = oneshot::channel();
        self.request(
            Request::MailboxPage {
                recipient,
                page,
                reply,
            },
            receive,
        )
        .await
    }

    async fn read_mail(
        &self,
        recipient: CharacterId,
        public_id: &str,
    ) -> Result<MailMessage, StoreError> {
        let (reply, receive) = oneshot::channel();
        self.request_commit(
            Request::ReadMail {
                recipient,
                public_id: public_id.to_owned(),
                reply,
            },
            receive,
        )
        .await
    }

    async fn delete_mail(&self, recipient: CharacterId, public_id: &str) -> Result<(), StoreError> {
        let (reply, receive) = oneshot::channel();
        self.request_commit(
            Request::DeleteMail {
                recipient,
                public_id: public_id.to_owned(),
                reply,
            },
            receive,
        )
        .await
    }

    async fn report_mail(
        &self,
        recipient: CharacterId,
        public_id: &str,
        reason: &str,
    ) -> Result<i64, StoreError> {
        let (reply, receive) = oneshot::channel();
        self.request_commit(
            Request::ReportMail {
                recipient,
                public_id: public_id.to_owned(),
                reason: reason.to_owned(),
                reply,
            },
            receive,
        )
        .await
    }

    async fn purge_expired_mail(&self) -> Result<u64, StoreError> {
        let (reply, receive) = oneshot::channel();
        self.request_commit(Request::PurgeExpiredMail { reply }, receive)
            .await
    }

    async fn list_board_posts(
        &self,
        actor: CharacterId,
        board: BoardKey,
        page: u32,
    ) -> Result<BoardPage, StoreError> {
        let (reply, receive) = oneshot::channel();
        self.request(
            Request::ListBoardPosts {
                actor,
                board,
                page,
                reply,
            },
            receive,
        )
        .await
    }

    async fn read_board_post(
        &self,
        actor: CharacterId,
        public_id: &str,
    ) -> Result<BoardEntry, StoreError> {
        let (reply, receive) = oneshot::channel();
        self.request(
            Request::ReadBoardPost {
                actor,
                public_id: public_id.to_owned(),
                reply,
            },
            receive,
        )
        .await
    }

    async fn publish_board_post(
        &self,
        actor: CharacterId,
        board: BoardKey,
        public_id: &str,
        subject: &str,
        body: &str,
    ) -> Result<BoardEntry, StoreError> {
        let (reply, receive) = oneshot::channel();
        self.request_commit(
            Request::PublishBoardPost {
                actor,
                board,
                public_id: public_id.to_owned(),
                subject: subject.to_owned(),
                body: body.to_owned(),
                reply,
            },
            receive,
        )
        .await
    }

    async fn delete_board_post(
        &self,
        actor: CharacterId,
        public_id: &str,
    ) -> Result<(), StoreError> {
        let (reply, receive) = oneshot::channel();
        self.request_commit(
            Request::DeleteBoardPost {
                actor,
                public_id: public_id.to_owned(),
                reply,
            },
            receive,
        )
        .await
    }

    async fn report_board_post(
        &self,
        actor: CharacterId,
        public_id: &str,
        reason: &str,
    ) -> Result<i64, StoreError> {
        let (reply, receive) = oneshot::channel();
        self.request_commit(
            Request::ReportBoardPost {
                actor,
                public_id: public_id.to_owned(),
                reason: reason.to_owned(),
                reply,
            },
            receive,
        )
        .await
    }

    async fn purge_expired_boards(&self) -> Result<u64, StoreError> {
        let (reply, receive) = oneshot::channel();
        self.request_commit(Request::PurgeExpiredBoards { reply }, receive)
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

    async fn staff_access(&self, character_id: CharacterId) -> Result<StaffAccess, StoreError> {
        let (reply, receive) = oneshot::channel();
        self.request(
            Request::StaffAccess {
                character_id,
                reply,
            },
            receive,
        )
        .await
    }

    async fn staff_reports(&self, audit: StaffAuditRecord) -> Result<Vec<StaffReport>, StoreError> {
        let (reply, receive) = oneshot::channel();
        self.request_commit(Request::StaffReports { audit, reply }, receive)
            .await
    }

    async fn resolve_staff_report(
        &self,
        report_id: i64,
        resolution: StaffReportResolution,
        resolution_reason: &str,
        audit: StaffAuditRecord,
    ) -> Result<(), StoreError> {
        let (reply, receive) = oneshot::channel();
        self.request_commit(
            Request::ResolveStaffReport {
                report_id,
                resolution,
                resolution_reason: resolution_reason.to_owned(),
                audit,
                reply,
            },
            receive,
        )
        .await
    }

    async fn set_staff_profile(
        &self,
        target_account: AccountId,
        profile: Option<StaffProfileKey>,
        lore_title: Option<&str>,
        audit: StaffAuditRecord,
    ) -> Result<(), StoreError> {
        let (reply, receive) = oneshot::channel();
        self.request_commit(
            Request::SetStaffProfile {
                target_account,
                profile,
                lore_title: lore_title.map(str::to_owned),
                audit,
                reply,
            },
            receive,
        )
        .await
    }

    async fn set_staff_capability(
        &self,
        target_account: AccountId,
        capability: StaffCapability,
        enabled: bool,
        audit: StaffAuditRecord,
    ) -> Result<(), StoreError> {
        let (reply, receive) = oneshot::channel();
        self.request_commit(
            Request::SetStaffCapability {
                target_account,
                capability,
                enabled,
                audit,
                reply,
            },
            receive,
        )
        .await
    }

    async fn staff_audit_entries(
        &self,
        audit: StaffAuditRecord,
    ) -> Result<Vec<StaffAuditEntry>, StoreError> {
        let (reply, receive) = oneshot::channel();
        self.request_commit(Request::StaffAuditEntries { audit, reply }, receive)
            .await
    }

    async fn append_staff_audit(&self, audit: StaffAuditRecord) -> Result<(), StoreError> {
        let (reply, receive) = oneshot::channel();
        self.request_commit(Request::AppendStaffAudit { audit, reply }, receive)
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

    async fn commit_staff_action(
        &self,
        characters: Vec<DurableCharacterSnapshot>,
        runtime: WorldRuntimeSnapshot,
        events: Vec<DomainEvent>,
        audit: StaffAuditRecord,
    ) -> Result<(), StoreError> {
        let (reply, receive) = oneshot::channel();
        self.request_commit(
            Request::CommitStaffAction {
                characters,
                runtime,
                events,
                audit,
                reply,
            },
            receive,
        )
        .await
    }
}

fn initialise(connection: &mut Connection) -> Result<(), StoreError> {
    connection.execute_batch(
        "PRAGMA journal_mode = WAL;
         PRAGMA synchronous = FULL;
         PRAGMA foreign_keys = ON;
         PRAGMA busy_timeout = 5000;",
    )?;

    match migrate(connection) {
        Ok(_) => Ok(()),
        Err(SchemaError::Sqlite(error)) => Err(StoreError::Sqlite(error)),
        Err(SchemaError::UnsupportedVersion { found, supported }) => {
            Err(StoreError::UnsupportedSchemaVersion { found, supported })
        }
        Err(SchemaError::ForeignKeyViolations) => Err(StoreError::SchemaForeignKeyViolations),
    }
}

fn normalise_name(name: &str, kind: &'static str) -> Result<String, StoreError> {
    let name = name.trim();
    if !valid_mail_visible_text(name, MAX_IDENTITY_NAME_BYTES) || name.contains('|') {
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

#[cfg(test)]
fn register_account(
    connection: &mut Connection,
    name: &str,
    password: &str,
    now: i64,
) -> Result<AuthenticatedSession, StoreError> {
    let name = normalise_name(name, "account")?;
    if !validate_password(password) {
        return Err(StoreError::InvalidPassword);
    }
    let password_hash = hash_password(password).map_err(|_| StoreError::PasswordHashing)?;
    finish_register_account(connection, &name, &password_hash, now)
}

#[cfg(test)]
fn login_account(
    connection: &mut Connection,
    name: &str,
    password: &str,
    now: i64,
) -> Result<AuthenticatedSession, StoreError> {
    let name = normalise_name(name, "account")?;
    if !validate_password(password) {
        return Err(StoreError::InvalidPassword);
    }
    let Some(prepared) = prepare_login_account(connection, &name)? else {
        let _ = hash_password(password);
        return Err(StoreError::InvalidCredentials);
    };
    if !verify_password(password, &prepared.encoded_hash)
        .map_err(|_| StoreError::PasswordHashing)?
    {
        return Err(StoreError::InvalidCredentials);
    }
    finish_login_account(connection, &prepared, now)
}

fn finish_register_account(
    connection: &mut Connection,
    name: &str,
    password_hash: &str,
    now: i64,
) -> Result<AuthenticatedSession, StoreError> {
    let transaction = connection.transaction()?;
    if transaction.query_row(
        "SELECT EXISTS(SELECT 1 FROM accounts WHERE name = ?1)",
        [name],
        |row| row.get::<_, bool>(0),
    )? {
        return Err(StoreError::AccountAlreadyExists);
    }

    let role = if transaction.query_row(
        "SELECT COUNT(*) FROM accounts WHERE auth_state = 'registered'",
        [],
        |row| row.get::<_, i64>(0),
    )? == 0
    {
        AccountRole::Admin
    } else {
        AccountRole::Player
    };
    transaction.execute(
        "INSERT INTO accounts (name, password_hash, auth_state, role)
         VALUES (?1, ?2, 'registered', ?3)",
        params![name, password_hash, role_as_database(role)],
    )?;
    let account = AccountSnapshot {
        id: AccountId(transaction.last_insert_rowid()),
        name: name.to_owned(),
    };
    if role == AccountRole::Admin {
        transaction.execute(
            "INSERT INTO staff_profiles
                 (account_id, profile_key, lore_title, assigned_by_account_id,
                  created_at, updated_at)
             VALUES (?1, 'implementor', NULL, ?1, ?2, ?2)",
            params![account.id.0, now],
        )?;
        for capability in StaffCapability::ALL {
            transaction.execute(
                "INSERT INTO staff_capability_assignments
                     (account_id, capability, assigned_by_account_id, created_at)
                 VALUES (?1, ?2, ?1, ?3)",
                params![account.id.0, capability.as_str(), now],
            )?;
        }
        append_audit(
            &transaction,
            account.id,
            Some(account.id),
            "bootstrap_admin",
            serde_json::json!({"source": "first_registered_account"}),
            now,
        )?;
        transaction.execute(
            "INSERT INTO staff_audit
                 (actor_account_id, target_account_id, capability, action,
                  reason, details_json, created_at)
             VALUES (?1, ?1, 'staff.assign', 'bootstrap.implementor',
                     'First registered account bootstrap',
                     '{\"source\":\"first_registered_account\"}', ?2)",
            params![account.id.0, now],
        )?;
    }
    let session = issue_session(&transaction, account, now)?;
    transaction.commit()?;
    Ok(session)
}

fn prepare_login_account(
    connection: &Connection,
    name: &str,
) -> Result<Option<PreparedLogin>, StoreError> {
    connection
        .query_row(
            "SELECT id, password_hash FROM accounts
             WHERE name = ?1 AND auth_state = 'registered'",
            [name],
            |row| {
                let Some(encoded_hash) = row.get::<_, Option<String>>(1)? else {
                    return Ok(None);
                };
                Ok(Some(PreparedLogin {
                    account_id: AccountId(row.get(0)?),
                    encoded_hash,
                }))
            },
        )
        .optional()
        .map(|prepared| prepared.flatten())
        .map_err(StoreError::from)
}

fn finish_login_account(
    connection: &mut Connection,
    prepared: &PreparedLogin,
    now: i64,
) -> Result<AuthenticatedSession, StoreError> {
    let transaction = connection.transaction()?;
    let current = transaction
        .query_row(
            "SELECT name, password_hash FROM accounts
             WHERE id = ?1 AND auth_state = 'registered'",
            [prepared.account_id.0],
            |row| Ok((row.get::<_, String>(0)?, row.get::<_, Option<String>>(1)?)),
        )
        .optional()?;
    let Some((name, Some(encoded_hash))) = current else {
        return Err(StoreError::InvalidCredentials);
    };
    if encoded_hash != prepared.encoded_hash {
        return Err(StoreError::InvalidCredentials);
    }
    let account = AccountSnapshot {
        id: prepared.account_id,
        name,
    };
    if account_is_banned(&transaction, account.id, now)? {
        return Err(StoreError::AccountBanned);
    }

    let session = issue_session(&transaction, account, now)?;
    transaction.commit()?;
    Ok(session)
}

fn issue_session(
    connection: &Connection,
    account: AccountSnapshot,
    now: i64,
) -> Result<AuthenticatedSession, StoreError> {
    let ttl = i64::try_from(DEFAULT_SESSION_TTL.as_secs()).unwrap_or(i64::MAX);
    let expires_at = now.checked_add(ttl).ok_or(StoreError::InvalidSession)?;

    connection.execute(
        "DELETE FROM auth_sessions
         WHERE account_id = ?1 AND (expires_at <= ?2 OR revoked_at IS NOT NULL)",
        params![account.id.0, now],
    )?;
    let active: i64 = connection.query_row(
        "SELECT COUNT(*) FROM auth_sessions WHERE account_id = ?1",
        [account.id.0],
        |row| row.get(0),
    )?;
    if active >= MAX_ACTIVE_SESSIONS {
        let remove_count = active - MAX_ACTIVE_SESSIONS + 1;
        connection.execute(
            "DELETE FROM auth_sessions WHERE token_hash IN (
                 SELECT token_hash FROM auth_sessions
                 WHERE account_id = ?1
                 ORDER BY created_at, rowid LIMIT ?2
             )",
            params![account.id.0, remove_count],
        )?;
    }

    let token = SessionToken::generate();
    connection.execute(
        "INSERT INTO auth_sessions
             (token_hash, account_id, created_at, expires_at)
         VALUES (?1, ?2, ?3, ?4)",
        params![token.digest().as_slice(), account.id.0, now, expires_at],
    )?;
    Ok(AuthenticatedSession {
        account,
        token,
        expires_at,
    })
}

fn resolve_session(
    connection: &Connection,
    token: &str,
    now: i64,
) -> Result<AccountSnapshot, StoreError> {
    let token_hash = digest_token(token).ok_or(StoreError::InvalidSession)?;
    let session = connection
        .query_row(
            "SELECT accounts.id, accounts.name, auth_sessions.expires_at,
                    auth_sessions.revoked_at
             FROM auth_sessions
             JOIN accounts ON accounts.id = auth_sessions.account_id
             WHERE auth_sessions.token_hash = ?1",
            [token_hash.as_slice()],
            |row| {
                Ok((
                    AccountSnapshot {
                        id: AccountId(row.get(0)?),
                        name: row.get(1)?,
                    },
                    row.get::<_, i64>(2)?,
                    row.get::<_, Option<i64>>(3)?,
                ))
            },
        )
        .optional()?
        .ok_or(StoreError::InvalidSession)?;

    if session.2.is_some() {
        return Err(StoreError::SessionRevoked);
    }
    if session.1 <= now {
        return Err(StoreError::SessionExpired);
    }
    if account_is_banned(connection, session.0.id, now)? {
        return Err(StoreError::AccountBanned);
    }
    Ok(session.0)
}

fn account_is_banned(
    connection: &Connection,
    account_id: AccountId,
    now: i64,
) -> Result<bool, StoreError> {
    connection
        .query_row(
            "SELECT EXISTS(
                 SELECT 1 FROM account_bans
                 WHERE account_id = ?1 AND revoked_at IS NULL
                   AND (expires_at IS NULL OR expires_at > ?2)
             )",
            params![account_id.0, now],
            |row| row.get(0),
        )
        .map_err(StoreError::from)
}

fn revoke_session(connection: &Connection, token: &str, now: i64) -> Result<(), StoreError> {
    let token_hash = digest_token(token).ok_or(StoreError::InvalidSession)?;
    let updated = connection.execute(
        "UPDATE auth_sessions SET revoked_at = COALESCE(revoked_at, ?1)
         WHERE token_hash = ?2",
        params![now, token_hash.as_slice()],
    )?;
    if updated == 0 {
        return Err(StoreError::InvalidSession);
    }
    Ok(())
}

fn ensure_account_exists(connection: &Connection, account_id: AccountId) -> Result<(), StoreError> {
    if connection.query_row(
        "SELECT EXISTS(SELECT 1 FROM accounts WHERE id = ?1)",
        [account_id.0],
        |row| row.get(0),
    )? {
        Ok(())
    } else {
        Err(StoreError::AccountNotFound(account_id))
    }
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

fn social_channel_from_database(value: &str) -> Result<SocialChannelName, StoreError> {
    match value {
        "general" => Ok(SocialChannelName::General),
        "newbie" => Ok(SocialChannelName::Newbie),
        "staff" => Ok(SocialChannelName::Staff),
        _ => Err(StoreError::InvalidSocialInput),
    }
}

fn ensure_social_character_exists(
    connection: &Connection,
    character_id: CharacterId,
) -> Result<(), StoreError> {
    let exists = connection.query_row(
        "SELECT EXISTS(SELECT 1 FROM characters WHERE id = ?1)",
        [character_id.0],
        |row| row.get::<_, bool>(0),
    )?;
    if exists {
        Ok(())
    } else {
        Err(StoreError::SocialTargetNotFound)
    }
}

fn social_profile(
    connection: &Connection,
    character_id: CharacterId,
    now: i64,
) -> Result<SocialProfile, StoreError> {
    let raw = connection
        .query_row(
            "SELECT characters.id, characters.account_id, characters.name, accounts.role,
                    EXISTS(
                        SELECT 1 FROM moderation_mutes
                        WHERE moderation_mutes.account_id = accounts.id
                          AND moderation_mutes.revoked_at IS NULL
                          AND (moderation_mutes.expires_at IS NULL
                               OR moderation_mutes.expires_at > ?2)
                    )
             FROM characters
             JOIN accounts ON accounts.id = characters.account_id
             WHERE characters.id = ?1",
            params![character_id.0, now],
            |row| {
                Ok((
                    CharacterId(row.get(0)?),
                    AccountId(row.get(1)?),
                    row.get::<_, String>(2)?,
                    row.get::<_, String>(3)?,
                    row.get::<_, bool>(4)?,
                ))
            },
        )
        .optional()?
        .ok_or(StoreError::SocialTargetNotFound)?;
    Ok(SocialProfile {
        character_id: raw.0,
        account_id: raw.1,
        character_name: raw.2,
        role: role_from_database(&raw.3)?,
        muted: raw.4,
    })
}

#[derive(Clone, Debug)]
struct PreparedStaffAudit {
    actor_account_id: AccountId,
    target_account_id: Option<AccountId>,
    capability: StaffCapability,
    action: String,
    reason: String,
    details_json: String,
}

fn staff_access(
    connection: &Connection,
    character_id: CharacterId,
) -> Result<StaffAccess, StoreError> {
    let account_id = connection
        .query_row(
            "SELECT account_id FROM characters WHERE id = ?1",
            [character_id.0],
            |row| row.get::<_, i64>(0).map(AccountId),
        )
        .optional()?
        .ok_or(StoreError::SocialTargetNotFound)?;
    staff_access_by_account(connection, account_id)
}

fn staff_access_by_account(
    connection: &Connection,
    account_id: AccountId,
) -> Result<StaffAccess, StoreError> {
    let (role, profile, lore_title) = connection
        .query_row(
            "SELECT accounts.role, staff_profiles.profile_key, staff_profiles.lore_title
             FROM accounts
             LEFT JOIN staff_profiles ON staff_profiles.account_id = accounts.id
             WHERE accounts.id = ?1",
            [account_id.0],
            |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, Option<String>>(1)?,
                    row.get::<_, Option<String>>(2)?,
                ))
            },
        )
        .optional()?
        .ok_or(StoreError::AccountNotFound(account_id))?;
    let profile = profile
        .as_deref()
        .map(|value| {
            StaffProfileKey::parse(value)
                .ok_or_else(|| StoreError::InvalidStaffState(format!("profile {value:?}")))
        })
        .transpose()?;
    let mut statement = connection.prepare(
        "SELECT capability FROM staff_capability_assignments
         WHERE account_id = ?1 ORDER BY capability",
    )?;
    let raw = statement
        .query_map([account_id.0], |row| row.get::<_, String>(0))?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    let capabilities = raw
        .into_iter()
        .map(|value| {
            StaffCapability::parse(&value)
                .ok_or_else(|| StoreError::InvalidStaffState(format!("capability {value:?}")))
        })
        .collect::<Result<BTreeSet<_>, _>>()?;
    if profile.is_none() && !capabilities.is_empty() {
        return Err(StoreError::InvalidStaffState(format!(
            "account {} has capabilities without a profile",
            account_id.0
        )));
    }
    Ok(StaffAccess {
        account_id,
        role: role_from_database(&role)?,
        profile,
        lore_title,
        capabilities,
    })
}

fn prepare_staff_audit(record: StaffAuditRecord) -> Result<PreparedStaffAudit, StoreError> {
    let action = normalise_staff_text(&record.action, MAX_STAFF_ACTION_BYTES)?;
    let reason = normalise_staff_text(&record.reason, MAX_SOCIAL_REASON_BYTES)?;
    let details_json = serde_json::to_string(&record.details)?;
    if details_json.len() > MAX_AUDIT_DETAILS_BYTES {
        return Err(StoreError::InvalidStaffInput);
    }
    if record.target_account_id == Some(record.actor_account_id) {
        return Err(StoreError::StaffHierarchy);
    }
    Ok(PreparedStaffAudit {
        actor_account_id: record.actor_account_id,
        target_account_id: record.target_account_id,
        capability: record.capability,
        action,
        reason,
        details_json,
    })
}

fn normalise_staff_text(value: &str, maximum: usize) -> Result<String, StoreError> {
    let value = value.trim();
    if value.is_empty() || value.len() > maximum || value.chars().any(char::is_control) {
        Err(StoreError::InvalidStaffInput)
    } else {
        Ok(value.to_owned())
    }
}

fn normalise_lore_title(value: Option<&str>) -> Result<Option<String>, StoreError> {
    value
        .map(|value| normalise_staff_text(value, MAX_STAFF_LORE_TITLE_BYTES))
        .transpose()
}

fn validate_staff_authority(
    connection: &Connection,
    audit: &PreparedStaffAudit,
    expected_capability: Option<StaffCapability>,
    target_account: Option<AccountId>,
) -> Result<StaffAccess, StoreError> {
    if expected_capability.is_some_and(|expected| expected != audit.capability)
        || audit.target_account_id != target_account
    {
        return Err(StoreError::StaffForbidden);
    }
    let actor = staff_access_by_account(connection, audit.actor_account_id)?;
    if !actor.capabilities.contains(&audit.capability) {
        return Err(StoreError::StaffForbidden);
    }
    let Some(actor_profile) = actor.profile else {
        return Err(StoreError::StaffForbidden);
    };
    if let Some(target_account) = target_account {
        if target_account == actor.account_id {
            return Err(StoreError::StaffHierarchy);
        }
        let target = staff_access_by_account(connection, target_account)?;
        if target
            .profile
            .is_some_and(|profile| profile.authority_rank() >= actor_profile.authority_rank())
        {
            return Err(StoreError::StaffHierarchy);
        }
    }
    Ok(actor)
}

fn insert_staff_audit(
    connection: &Connection,
    audit: &PreparedStaffAudit,
    now: i64,
) -> Result<(), StoreError> {
    connection.execute(
        "INSERT INTO staff_audit
             (actor_account_id, target_account_id, capability, action,
              reason, details_json, created_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
        params![
            audit.actor_account_id.0,
            audit.target_account_id.map(|account| account.0),
            audit.capability.as_str(),
            audit.action,
            audit.reason,
            audit.details_json,
            now
        ],
    )?;
    Ok(())
}

fn social_target(
    connection: &Connection,
    name: &str,
    now: i64,
) -> Result<SocialProfile, StoreError> {
    let name = normalise_name(name, "social target")?;
    let mut statement = connection
        .prepare("SELECT id FROM characters WHERE name = ?1 COLLATE NOCASE ORDER BY id LIMIT 2")?;
    let matches = statement
        .query_map([name], |row| row.get::<_, i64>(0))?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    match matches.as_slice() {
        [id] => social_profile(connection, CharacterId(*id), now),
        [] => Err(StoreError::SocialTargetNotFound),
        [_, ..] => Err(StoreError::AmbiguousSocialTarget),
    }
}

fn social_delivery_allowed(
    connection: &Connection,
    sender: CharacterId,
    recipient: CharacterId,
) -> Result<bool, StoreError> {
    if sender == recipient {
        return Ok(true);
    }
    let both_exist: i64 = connection.query_row(
        "SELECT COUNT(*) FROM characters WHERE id IN (?1, ?2)",
        params![sender.0, recipient.0],
        |row| row.get(0),
    )?;
    if both_exist != 2 {
        return Err(StoreError::SocialTargetNotFound);
    }
    let suppressed = connection.query_row(
        "SELECT
             EXISTS(SELECT 1 FROM social_ignores
                    WHERE owner_character_id = ?2 AND target_character_id = ?1)
             OR EXISTS(SELECT 1 FROM social_blocks
                       WHERE owner_character_id = ?2 AND target_character_id = ?1)
             OR EXISTS(SELECT 1 FROM social_blocks
                       WHERE owner_character_id = ?1 AND target_character_id = ?2)",
        params![sender.0, recipient.0],
        |row| row.get::<_, bool>(0),
    )?;
    Ok(!suppressed)
}

fn set_social_relation(
    connection: &mut Connection,
    actor: CharacterId,
    target: CharacterId,
    relation: SocialRelationKind,
    enabled: bool,
    now: i64,
) -> Result<(), StoreError> {
    if actor == target {
        return Err(StoreError::SocialSelfTarget);
    }
    let transaction = connection.transaction()?;
    let actor_profile = social_profile(&transaction, actor, now)?;
    let target_profile = social_profile(&transaction, target, now)?;
    if actor_profile.account_id == target_profile.account_id {
        return Err(StoreError::SocialSelfTarget);
    }
    let table = match relation {
        SocialRelationKind::Ignore => "social_ignores",
        SocialRelationKind::Block => "social_blocks",
    };
    let sql = if enabled {
        format!(
            "INSERT INTO {table} (owner_character_id, target_character_id, created_at)
             VALUES (?1, ?2, ?3) ON CONFLICT DO NOTHING"
        )
    } else {
        format!("DELETE FROM {table} WHERE owner_character_id = ?1 AND target_character_id = ?2")
    };
    if enabled {
        transaction.execute(&sql, params![actor.0, target.0, now])?;
    } else {
        transaction.execute(&sql, params![actor.0, target.0])?;
    }
    let action = match (relation, enabled) {
        (SocialRelationKind::Ignore, true) => "ignore",
        (SocialRelationKind::Ignore, false) => "unignore",
        (SocialRelationKind::Block, true) => "block",
        (SocialRelationKind::Block, false) => "unblock",
    };
    append_audit(
        &transaction,
        actor_profile.account_id,
        Some(target_profile.account_id),
        action,
        serde_json::json!({"targetCharacterId": target.0}),
        now,
    )?;
    transaction.commit()?;
    Ok(())
}

fn social_channel_subscriptions(
    connection: &Connection,
    character_id: CharacterId,
) -> Result<Vec<SocialChannelName>, StoreError> {
    ensure_social_character_exists(connection, character_id)?;
    let mut statement = connection.prepare(
        "SELECT channel FROM social_channel_subscriptions
         WHERE character_id = ?1
         ORDER BY CASE channel
             WHEN 'general' THEN 0
             WHEN 'newbie' THEN 1
             WHEN 'staff' THEN 2
             ELSE 3
         END",
    )?;
    let stored = statement
        .query_map([character_id.0], |row| row.get::<_, String>(0))?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    stored
        .iter()
        .map(|channel| social_channel_from_database(channel))
        .collect()
}

fn social_channel_subscribers(
    connection: &Connection,
    channel: SocialChannelName,
) -> Result<Vec<CharacterId>, StoreError> {
    let mut statement = connection.prepare(
        "SELECT character_id FROM social_channel_subscriptions
         WHERE channel = ?1 ORDER BY character_id",
    )?;
    statement
        .query_map([channel.as_str()], |row| {
            row.get::<_, i64>(0).map(CharacterId)
        })?
        .collect::<rusqlite::Result<Vec<_>>>()
        .map_err(StoreError::from)
}

fn set_social_channel_subscription(
    connection: &mut Connection,
    character_id: CharacterId,
    channel: SocialChannelName,
    subscribed: bool,
    now: i64,
) -> Result<(), StoreError> {
    let transaction = connection.transaction()?;
    ensure_social_character_exists(&transaction, character_id)?;
    if subscribed {
        transaction.execute(
            "INSERT INTO social_channel_subscriptions
                 (character_id, channel, created_at)
             VALUES (?1, ?2, ?3)
             ON CONFLICT(character_id, channel) DO NOTHING",
            params![character_id.0, channel.as_str(), now],
        )?;
    } else {
        transaction.execute(
            "DELETE FROM social_channel_subscriptions
             WHERE character_id = ?1 AND channel = ?2",
            params![character_id.0, channel.as_str()],
        )?;
    }
    transaction.commit()?;
    Ok(())
}

fn normalise_mail_text(value: &str, maximum: usize) -> Result<String, StoreError> {
    if !valid_mail_visible_text(value, maximum) {
        Err(StoreError::InvalidMailInput)
    } else {
        Ok(value.to_owned())
    }
}

fn consume_mail_usage(
    connection: &Connection,
    account_id: AccountId,
    now: i64,
) -> Result<(), StoreError> {
    let stored = connection
        .query_row(
            "SELECT hour_started_at, hour_count, day_started_at, day_count
             FROM social_mail_usage WHERE account_id = ?1",
            [account_id.0],
            |row| {
                Ok((
                    row.get::<_, i64>(0)?,
                    row.get::<_, i64>(1)?,
                    row.get::<_, i64>(2)?,
                    row.get::<_, i64>(3)?,
                ))
            },
        )
        .optional()?;
    let Some((stored_hour_start, stored_hour_count, stored_day_start, stored_day_count)) = stored
    else {
        connection.execute(
            "INSERT INTO social_mail_usage
                 (account_id, hour_started_at, hour_count,
                  day_started_at, day_count, updated_at)
             VALUES (?1, ?2, 1, ?2, 1, ?2)",
            params![account_id.0, now],
        )?;
        return Ok(());
    };
    let hour_expired =
        now < stored_hour_start || now.saturating_sub(stored_hour_start) >= MAIL_HOUR_SECONDS;
    let day_expired =
        now < stored_day_start || now.saturating_sub(stored_day_start) >= MAIL_DAY_SECONDS;
    let hour_start = if hour_expired { now } else { stored_hour_start };
    let day_start = if day_expired { now } else { stored_day_start };
    let hour_count = if hour_expired { 0 } else { stored_hour_count };
    let day_count = if day_expired { 0 } else { stored_day_count };
    if hour_count >= MAIL_HOURLY_CAPACITY || day_count >= MAIL_DAILY_CAPACITY {
        return Err(StoreError::MailRateLimited);
    }
    connection.execute(
        "UPDATE social_mail_usage
         SET hour_started_at = ?1, hour_count = ?2,
             day_started_at = ?3, day_count = ?4, updated_at = ?5
         WHERE account_id = ?6",
        params![
            hour_start,
            hour_count + 1,
            day_start,
            day_count + 1,
            now,
            account_id.0
        ],
    )?;
    Ok(())
}

fn send_mail(
    connection: &mut Connection,
    sender: CharacterId,
    target_name: &str,
    public_id: &str,
    subject: &str,
    body: &str,
    now: i64,
) -> Result<MailSendResult, StoreError> {
    let target_name = normalise_mail_text(target_name, MAX_IDENTITY_NAME_BYTES)?;
    let subject = normalise_mail_text(subject, MAX_MAIL_SUBJECT_BYTES)?;
    let body = normalise_mail_text(body, MAX_MAIL_BODY_BYTES)?;
    if !valid_mail_target(&target_name) || !valid_mail_subject(&subject) {
        return Err(StoreError::InvalidMailInput);
    }
    if !valid_mail_public_id(public_id) {
        return Err(StoreError::InvalidMailInput);
    }
    let transaction = connection.transaction()?;
    let (sender_account_id, sender_name, sender_muted) = transaction
        .query_row(
            "SELECT characters.account_id, characters.name,
                    EXISTS(
                        SELECT 1 FROM moderation_mutes
                        WHERE moderation_mutes.account_id = characters.account_id
                          AND moderation_mutes.revoked_at IS NULL
                          AND (moderation_mutes.expires_at IS NULL
                               OR moderation_mutes.expires_at > ?2)
                    )
             FROM characters WHERE characters.id = ?1",
            params![sender.0, now],
            |row| {
                Ok((
                    AccountId(row.get(0)?),
                    row.get::<_, String>(1)?,
                    row.get::<_, bool>(2)?,
                ))
            },
        )
        .optional()?
        .ok_or(StoreError::SocialTargetNotFound)?;
    consume_mail_usage(&transaction, sender_account_id, now)?;

    if sender_muted {
        transaction.commit()?;
        return Err(StoreError::ChatMuted);
    }
    if !valid_mail_visible_text(&sender_name, MAX_IDENTITY_NAME_BYTES) {
        transaction.commit()?;
        return Ok(MailSendResult::Suppressed);
    }

    let duplicate_public_id = transaction.query_row(
        "SELECT EXISTS(SELECT 1 FROM social_mail_messages WHERE public_id = ?1)",
        [public_id],
        |row| row.get::<_, bool>(0),
    )?;
    let result = if duplicate_public_id {
        MailSendResult::Unavailable
    } else {
        let mut statement = transaction.prepare(
            "SELECT id, account_id FROM characters
             WHERE name = ?1 COLLATE BINARY ORDER BY id LIMIT 2",
        )?;
        let targets = statement
            .query_map([target_name], |row| {
                Ok((CharacterId(row.get(0)?), AccountId(row.get(1)?)))
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        drop(statement);
        match targets.as_slice() {
            [(recipient, recipient_account)] if *recipient != sender => {
                let suppressed_by_policy = transaction.query_row(
                    "SELECT
                         EXISTS(SELECT 1 FROM account_bans
                                WHERE account_id = ?3 AND revoked_at IS NULL
                                  AND (expires_at IS NULL OR expires_at > ?4))
                         OR EXISTS(SELECT 1 FROM social_ignores
                                   WHERE owner_character_id = ?2
                                     AND target_character_id = ?1)
                         OR EXISTS(SELECT 1 FROM social_blocks
                                   WHERE owner_character_id = ?2
                                     AND target_character_id = ?1)
                         OR EXISTS(SELECT 1 FROM social_blocks
                                   WHERE owner_character_id = ?1
                                     AND target_character_id = ?2)",
                    params![sender.0, recipient.0, recipient_account.0, now],
                    |row| row.get::<_, bool>(0),
                )?;
                if suppressed_by_policy {
                    MailSendResult::Suppressed
                } else {
                    let (mailbox_count, mailbox_bytes) = transaction.query_row(
                        "SELECT COUNT(*),
                                COALESCE(SUM(length(CAST(subject AS BLOB))
                                           + length(CAST(body AS BLOB))), 0)
                         FROM social_mail_messages
                         WHERE recipient_character_id = ?1 AND expires_at > ?2",
                        params![recipient.0, now],
                        |row| Ok((row.get::<_, i64>(0)?, row.get::<_, i64>(1)?)),
                    )?;
                    let global_count = transaction.query_row(
                        "SELECT COUNT(*) FROM social_mail_messages WHERE expires_at > ?1",
                        [now],
                        |row| row.get::<_, i64>(0),
                    )?;
                    let requested_bytes =
                        i64::try_from(subject.len().saturating_add(body.len())).unwrap_or(i64::MAX);
                    if mailbox_count >= i64::from(MAILBOX_CAPACITY)
                        || mailbox_bytes.saturating_add(requested_bytes) > MAILBOX_BYTE_CAPACITY
                        || global_count >= MAIL_GLOBAL_CAPACITY
                    {
                        MailSendResult::Suppressed
                    } else {
                        transaction.execute(
                            "INSERT INTO social_mail_messages
                                 (public_id, sender_character_id, sender_name,
                                  recipient_character_id, subject, body,
                                  created_at, expires_at)
                             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
                            params![
                                public_id,
                                sender.0,
                                sender_name,
                                recipient.0,
                                subject,
                                body,
                                now,
                                now.saturating_add(MAIL_RETENTION_SECONDS)
                            ],
                        )?;
                        MailSendResult::Stored
                    }
                }
            }
            _ => MailSendResult::Unavailable,
        }
    };
    transaction.commit()?;
    Ok(result)
}

fn mailbox_page(
    connection: &Connection,
    recipient: CharacterId,
    page: u32,
    now: i64,
) -> Result<MailPage, StoreError> {
    if !(1..=MAIL_MAX_PAGES).contains(&page) {
        return Err(StoreError::InvalidMailInput);
    }
    ensure_social_character_exists(connection, recipient)?;
    let (total, unread) = connection.query_row(
        "SELECT COUNT(*), SUM(CASE WHEN read_at IS NULL THEN 1 ELSE 0 END)
         FROM social_mail_messages
         WHERE recipient_character_id = ?1 AND expires_at > ?2",
        params![recipient.0, now],
        |row| {
            Ok((
                row.get::<_, i64>(0)?,
                row.get::<_, Option<i64>>(1)?.unwrap_or(0),
            ))
        },
    )?;
    let total_messages = u32::try_from(total).map_err(|_| StoreError::InvalidMailInput)?;
    let unread_count = u32::try_from(unread).map_err(|_| StoreError::InvalidMailInput)?;
    let total_pages = total_messages.div_ceil(MAIL_PAGE_SIZE).max(1);
    if total_pages > MAIL_MAX_PAGES {
        return Err(StoreError::InvalidMailInput);
    }
    let offset = i64::from((page - 1).saturating_mul(MAIL_PAGE_SIZE));
    let mut statement = connection.prepare(
        "SELECT public_id, sender_name, subject, created_at, expires_at,
                read_at IS NOT NULL
         FROM social_mail_messages
         WHERE recipient_character_id = ?1 AND expires_at > ?2
         ORDER BY created_at DESC, id DESC
         LIMIT ?3 OFFSET ?4",
    )?;
    let messages = statement
        .query_map(
            params![recipient.0, now, i64::from(MAIL_PAGE_SIZE), offset],
            |row| {
                Ok(MailMessageSummary {
                    public_id: row.get(0)?,
                    sender_name: row.get(1)?,
                    subject: row.get(2)?,
                    created_at: row.get(3)?,
                    expires_at: row.get(4)?,
                    read: row.get(5)?,
                })
            },
        )?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(MailPage {
        page,
        total_pages,
        total_messages,
        unread_count,
        capacity: MAILBOX_CAPACITY,
        messages,
    })
}

fn read_mail(
    connection: &mut Connection,
    recipient: CharacterId,
    public_id: &str,
    now: i64,
) -> Result<MailMessage, StoreError> {
    if !valid_mail_public_id(public_id) {
        return Err(StoreError::InvalidMailInput);
    }
    let transaction = connection.transaction()?;
    ensure_social_character_exists(&transaction, recipient)?;
    let updated = transaction.execute(
        "UPDATE social_mail_messages SET read_at = COALESCE(read_at, ?1)
         WHERE recipient_character_id = ?2 AND public_id = ?3 AND expires_at > ?1",
        params![now, recipient.0, public_id],
    )?;
    if updated != 1 {
        return Err(StoreError::MailNotFound);
    }
    let message = transaction.query_row(
        "SELECT public_id, sender_name, subject, body, created_at, expires_at,
                read_at IS NOT NULL
         FROM social_mail_messages
         WHERE recipient_character_id = ?1 AND public_id = ?2 AND expires_at > ?3",
        params![recipient.0, public_id, now],
        |row| {
            Ok(MailMessage {
                summary: MailMessageSummary {
                    public_id: row.get(0)?,
                    sender_name: row.get(1)?,
                    subject: row.get(2)?,
                    created_at: row.get(4)?,
                    expires_at: row.get(5)?,
                    read: row.get(6)?,
                },
                body: row.get(3)?,
            })
        },
    )?;
    transaction.commit()?;
    Ok(message)
}

fn delete_mail(
    connection: &mut Connection,
    recipient: CharacterId,
    public_id: &str,
    now: i64,
) -> Result<(), StoreError> {
    if !valid_mail_public_id(public_id) {
        return Err(StoreError::InvalidMailInput);
    }
    let transaction = connection.transaction()?;
    ensure_social_character_exists(&transaction, recipient)?;
    let deleted = transaction.execute(
        "DELETE FROM social_mail_messages
         WHERE recipient_character_id = ?1 AND public_id = ?2 AND expires_at > ?3",
        params![recipient.0, public_id, now],
    )?;
    if deleted != 1 {
        return Err(StoreError::MailNotFound);
    }
    transaction.commit()?;
    Ok(())
}

fn report_mail(
    connection: &mut Connection,
    recipient: CharacterId,
    public_id: &str,
    reason: &str,
    now: i64,
) -> Result<i64, StoreError> {
    if !valid_mail_public_id(public_id) {
        return Err(StoreError::InvalidMailInput);
    }
    let reason = normalise_mail_text(reason, MAX_REPORT_REASON_BYTES)?;
    let transaction = connection.transaction()?;
    let reporter = social_profile(&transaction, recipient, now)?;
    let message = transaction
        .query_row(
            "SELECT sender_character_id, sender_name, subject, body,
                    created_at, report_id
             FROM social_mail_messages
             WHERE recipient_character_id = ?1 AND public_id = ?2 AND expires_at > ?3",
            params![recipient.0, public_id, now],
            |row| {
                Ok((
                    row.get::<_, Option<i64>>(0)?.map(CharacterId),
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, String>(3)?,
                    row.get::<_, i64>(4)?,
                    row.get::<_, Option<i64>>(5)?,
                ))
            },
        )
        .optional()?
        .ok_or(StoreError::MailNotFound)?;
    if message.5.is_some() {
        return Err(StoreError::MailAlreadyReported);
    }
    let sender = message.0.ok_or(StoreError::MailNotFound)?;
    let sender_profile = social_profile(&transaction, sender, now)?;
    let open_reports = transaction.query_row(
        "SELECT COUNT(*)
         FROM social_reports AS reports
         JOIN characters AS reporter ON reporter.id = reports.reporter_character_id
         WHERE reporter.account_id = ?1
           AND reports.report_kind = 'mail'
           AND reports.status = 'open'",
        [reporter.account_id.0],
        |row| row.get::<_, i64>(0),
    )?;
    if open_reports >= MAIL_OPEN_REPORT_CAPACITY_PER_ACCOUNT {
        return Err(StoreError::MailReportLimit);
    }
    let evidence_count = transaction.query_row(
        "SELECT COUNT(*) FROM social_mail_report_evidence WHERE expires_at > ?1",
        [now],
        |row| row.get::<_, i64>(0),
    )?;
    if evidence_count >= MAIL_REPORT_EVIDENCE_GLOBAL_CAPACITY {
        return Err(StoreError::MailEvidenceLimit);
    }
    transaction.execute(
        "INSERT INTO social_reports
             (reporter_character_id, target_character_id, report_kind, reason, created_at)
         VALUES (?1, ?2, 'mail', ?3, ?4)",
        params![recipient.0, sender.0, reason, now],
    )?;
    let report_id = transaction.last_insert_rowid();
    transaction.execute(
        "INSERT INTO social_mail_report_evidence
             (report_id, sender_name, subject, body, sent_at, created_at, expires_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
        params![
            report_id,
            message.1,
            message.2,
            message.3,
            message.4,
            now,
            now.saturating_add(MAIL_REPORT_EVIDENCE_RETENTION_SECONDS)
        ],
    )?;
    let linked = transaction.execute(
        "UPDATE social_mail_messages SET report_id = ?1
         WHERE recipient_character_id = ?2 AND public_id = ?3
           AND report_id IS NULL AND expires_at > ?4",
        params![report_id, recipient.0, public_id, now],
    )?;
    if linked != 1 {
        return Err(StoreError::MailAlreadyReported);
    }
    append_audit(
        &transaction,
        reporter.account_id,
        Some(sender_profile.account_id),
        "mail_report",
        serde_json::json!({"reportId": report_id}),
        now,
    )?;
    transaction.commit()?;
    Ok(report_id)
}

fn purge_expired_mail(connection: &mut Connection, now: i64) -> Result<u64, StoreError> {
    let transaction = connection.transaction()?;
    let messages = transaction.execute(
        "DELETE FROM social_mail_messages
         WHERE id IN (
             SELECT id FROM social_mail_messages
             WHERE expires_at <= ?1 ORDER BY expires_at, id LIMIT ?2
         )",
        params![now, MAIL_PURGE_LIMIT],
    )?;
    let remaining = MAIL_PURGE_LIMIT.saturating_sub(i64::try_from(messages).unwrap_or(i64::MAX));
    let evidence = if remaining > 0 {
        transaction.execute(
            "DELETE FROM social_mail_report_evidence
             WHERE report_id IN (
                 SELECT report_id FROM social_mail_report_evidence
                 WHERE expires_at <= ?1 ORDER BY expires_at, report_id LIMIT ?2
             )",
            params![now, remaining],
        )?
    } else {
        0
    };
    transaction.commit()?;
    u64::try_from(messages.saturating_add(evidence)).map_err(|_| StoreError::InvalidMailInput)
}

fn normalise_board_text(value: &str, maximum: usize) -> Result<String, StoreError> {
    if valid_board_visible_text(value, maximum) {
        Ok(value.to_owned())
    } else {
        Err(StoreError::InvalidBoardInput)
    }
}

fn check_board_capacity(
    account_count: i64,
    board_count: i64,
    board_bytes: i64,
    global_count: i64,
    global_bytes: i64,
    added_bytes: i64,
) -> Result<(), StoreError> {
    if account_count >= i64::from(BOARD_ACCOUNT_CAPACITY)
        || board_count >= i64::from(BOARD_CAPACITY_PER_BOARD)
        || board_bytes.saturating_add(added_bytes) > BOARD_BYTE_CAPACITY_PER_BOARD
        || global_count >= BOARD_GLOBAL_CAPACITY
        || global_bytes.saturating_add(added_bytes) > BOARD_GLOBAL_BYTE_CAPACITY
    {
        Err(StoreError::BoardCapacity)
    } else {
        Ok(())
    }
}

fn check_board_report_capacity(open_reports: i64, evidence_count: i64) -> Result<(), StoreError> {
    if open_reports >= BOARD_OPEN_REPORT_CAPACITY_PER_ACCOUNT {
        Err(StoreError::BoardReportLimit)
    } else if evidence_count >= BOARD_REPORT_EVIDENCE_GLOBAL_CAPACITY {
        Err(StoreError::BoardEvidenceLimit)
    } else {
        Ok(())
    }
}

fn consume_board_usage(
    connection: &Connection,
    account_id: AccountId,
    now: i64,
) -> Result<(), StoreError> {
    let stored = connection
        .query_row(
            "SELECT hour_started_at, hour_count, day_started_at, day_count
             FROM social_board_usage WHERE account_id = ?1",
            [account_id.0],
            |row| {
                Ok((
                    row.get::<_, i64>(0)?,
                    row.get::<_, i64>(1)?,
                    row.get::<_, i64>(2)?,
                    row.get::<_, i64>(3)?,
                ))
            },
        )
        .optional()?;
    let Some((stored_hour_start, stored_hour_count, stored_day_start, stored_day_count)) = stored
    else {
        connection.execute(
            "INSERT INTO social_board_usage
                 (account_id, hour_started_at, hour_count,
                  day_started_at, day_count, updated_at)
             VALUES (?1, ?2, 1, ?2, 1, ?2)",
            params![account_id.0, now],
        )?;
        return Ok(());
    };
    let hour_expired =
        now < stored_hour_start || now.saturating_sub(stored_hour_start) >= MAIL_HOUR_SECONDS;
    let day_expired =
        now < stored_day_start || now.saturating_sub(stored_day_start) >= MAIL_DAY_SECONDS;
    let hour_start = if hour_expired { now } else { stored_hour_start };
    let day_start = if day_expired { now } else { stored_day_start };
    let hour_count = if hour_expired { 0 } else { stored_hour_count };
    let day_count = if day_expired { 0 } else { stored_day_count };
    if hour_count >= BOARD_HOURLY_CAPACITY || day_count >= BOARD_DAILY_CAPACITY {
        return Err(StoreError::BoardRateLimited);
    }
    connection.execute(
        "UPDATE social_board_usage
         SET hour_started_at = ?1, hour_count = ?2,
             day_started_at = ?3, day_count = ?4, updated_at = ?5
         WHERE account_id = ?6",
        params![
            hour_start,
            hour_count + 1,
            day_start,
            day_count + 1,
            now,
            account_id.0
        ],
    )?;
    Ok(())
}

fn list_board_posts(
    connection: &Connection,
    actor: CharacterId,
    board: BoardKey,
    page: u32,
    now: i64,
) -> Result<BoardPage, StoreError> {
    if !(1..=BOARD_MAX_PAGES).contains(&page) {
        return Err(StoreError::InvalidBoardInput);
    }
    let actor_profile = social_profile(connection, actor, now)?;
    let total = connection.query_row(
        "SELECT COUNT(*) FROM social_board_posts
         WHERE board_key = ?1 AND expires_at > ?2",
        params![board.as_str(), now],
        |row| row.get::<_, i64>(0),
    )?;
    let total_posts = u32::try_from(total).map_err(|_| StoreError::InvalidBoardInput)?;
    let total_pages = total_posts.div_ceil(BOARD_PAGE_SIZE).max(1);
    if total_pages > BOARD_MAX_PAGES {
        return Err(StoreError::InvalidBoardInput);
    }
    let offset = i64::from((page - 1).saturating_mul(BOARD_PAGE_SIZE));
    let mut statement = connection.prepare(
        "SELECT posts.public_id, posts.author_character_id, authors.account_id,
                posts.author_name, posts.subject, posts.created_at, posts.expires_at,
                posts.report_id
         FROM social_board_posts AS posts
         JOIN characters AS authors ON authors.id = posts.author_character_id
         WHERE posts.board_key = ?1 AND posts.expires_at > ?2
         ORDER BY posts.created_at DESC, posts.id DESC
         LIMIT ?3 OFFSET ?4",
    )?;
    let posts = statement
        .query_map(
            params![board.as_str(), now, i64::from(BOARD_PAGE_SIZE), offset],
            |row| {
                let author_character = CharacterId(row.get(1)?);
                let author_account = AccountId(row.get(2)?);
                let report_id = row.get::<_, Option<i64>>(7)?;
                Ok(BoardPostSummary {
                    public_id: row.get(0)?,
                    board,
                    author_name: row.get(3)?,
                    subject: row.get(4)?,
                    created_at: row.get(5)?,
                    expires_at: row.get(6)?,
                    can_delete: author_character == actor,
                    can_report: author_account != actor_profile.account_id && report_id.is_none(),
                })
            },
        )?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(BoardPage {
        board,
        page,
        total_pages,
        total_posts,
        capacity: BOARD_CAPACITY_PER_BOARD,
        posts,
    })
}

fn read_board_post(
    connection: &Connection,
    actor: CharacterId,
    public_id: &str,
    now: i64,
) -> Result<BoardEntry, StoreError> {
    if !valid_board_public_id(public_id) {
        return Err(StoreError::InvalidBoardInput);
    }
    let actor_profile = social_profile(connection, actor, now)?;
    connection
        .query_row(
            "SELECT posts.board_key, posts.author_character_id, authors.account_id,
                    posts.author_name, posts.subject, posts.body,
                    posts.created_at, posts.expires_at, posts.report_id
             FROM social_board_posts AS posts
             JOIN characters AS authors ON authors.id = posts.author_character_id
             WHERE posts.public_id = ?1 AND posts.expires_at > ?2",
            params![public_id, now],
            |row| {
                let board_key: String = row.get(0)?;
                let board = BoardKey::parse(&board_key).ok_or_else(|| {
                    rusqlite::Error::FromSqlConversionFailure(
                        0,
                        Type::Text,
                        Box::new(io::Error::new(
                            io::ErrorKind::InvalidData,
                            "invalid board key",
                        )),
                    )
                })?;
                let author_character = CharacterId(row.get(1)?);
                let author_account = AccountId(row.get(2)?);
                let report_id = row.get::<_, Option<i64>>(8)?;
                Ok(BoardEntry {
                    summary: BoardPostSummary {
                        public_id: public_id.to_owned(),
                        board,
                        author_name: row.get(3)?,
                        subject: row.get(4)?,
                        created_at: row.get(6)?,
                        expires_at: row.get(7)?,
                        can_delete: author_character == actor,
                        can_report: author_account != actor_profile.account_id
                            && report_id.is_none(),
                    },
                    body: row.get(5)?,
                })
            },
        )
        .optional()?
        .ok_or(StoreError::BoardPostUnavailable)
}

fn publish_board_post(
    connection: &mut Connection,
    actor: CharacterId,
    board: BoardKey,
    public_id: &str,
    subject: &str,
    body: &str,
    now: i64,
) -> Result<BoardEntry, StoreError> {
    if !valid_board_public_id(public_id) || !valid_board_subject(subject) {
        return Err(StoreError::InvalidBoardInput);
    }
    let subject = normalise_board_text(subject, tiamud_core::MAX_BOARD_SUBJECT_BYTES)?;
    let body = normalise_board_text(body, tiamud_core::MAX_BOARD_BODY_BYTES)?;
    let expires_at = now
        .checked_add(BOARD_RETENTION_SECONDS)
        .ok_or(StoreError::InvalidBoardInput)?;
    let transaction = connection.transaction()?;
    let profile = social_profile(&transaction, actor, now)?;
    if profile.muted {
        return Err(StoreError::ChatMuted);
    }
    if account_is_banned(&transaction, profile.account_id, now)? {
        return Err(StoreError::AccountBanned);
    }
    consume_board_usage(&transaction, profile.account_id, now)?;
    let duplicate = transaction.query_row(
        "SELECT EXISTS(SELECT 1 FROM social_board_posts WHERE public_id = ?1)",
        [public_id],
        |row| row.get::<_, bool>(0),
    )?;
    if duplicate {
        return Err(StoreError::InvalidBoardInput);
    }
    let account_count = transaction.query_row(
        "SELECT COUNT(*)
         FROM social_board_posts AS posts
         JOIN characters AS authors ON authors.id = posts.author_character_id
         WHERE authors.account_id = ?1 AND posts.expires_at > ?2",
        params![profile.account_id.0, now],
        |row| row.get::<_, i64>(0),
    )?;
    let (board_count, board_bytes) = transaction.query_row(
        "SELECT COUNT(*),
                COALESCE(SUM(length(CAST(subject AS BLOB))
                             + length(CAST(body AS BLOB))), 0)
         FROM social_board_posts WHERE board_key = ?1 AND expires_at > ?2",
        params![board.as_str(), now],
        |row| Ok((row.get::<_, i64>(0)?, row.get::<_, i64>(1)?)),
    )?;
    let (global_count, global_bytes) = transaction.query_row(
        "SELECT COUNT(*),
                COALESCE(SUM(length(CAST(subject AS BLOB))
                             + length(CAST(body AS BLOB))), 0)
         FROM social_board_posts WHERE expires_at > ?1",
        [now],
        |row| Ok((row.get::<_, i64>(0)?, row.get::<_, i64>(1)?)),
    )?;
    let added_bytes = i64::try_from(subject.len().saturating_add(body.len()))
        .map_err(|_| StoreError::InvalidBoardInput)?;
    if let Err(error) = check_board_capacity(
        account_count,
        board_count,
        board_bytes,
        global_count,
        global_bytes,
        added_bytes,
    ) {
        transaction.commit()?;
        return Err(error);
    }
    transaction.execute(
        "INSERT INTO social_board_posts
             (public_id, board_key, author_character_id, author_name,
              subject, body, created_at, expires_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
        params![
            public_id,
            board.as_str(),
            actor.0,
            profile.character_name,
            subject,
            body,
            now,
            expires_at
        ],
    )?;
    transaction.commit()?;
    Ok(BoardEntry {
        summary: BoardPostSummary {
            public_id: public_id.to_owned(),
            board,
            author_name: profile.character_name,
            subject,
            created_at: now,
            expires_at,
            can_delete: true,
            can_report: false,
        },
        body,
    })
}

fn delete_board_post(
    connection: &mut Connection,
    actor: CharacterId,
    public_id: &str,
    now: i64,
) -> Result<(), StoreError> {
    if !valid_board_public_id(public_id) {
        return Err(StoreError::InvalidBoardInput);
    }
    let transaction = connection.transaction()?;
    social_profile(&transaction, actor, now)?;
    let deleted = transaction.execute(
        "DELETE FROM social_board_posts
         WHERE public_id = ?1 AND author_character_id = ?2 AND expires_at > ?3",
        params![public_id, actor.0, now],
    )?;
    if deleted != 1 {
        return Err(StoreError::BoardPostUnavailable);
    }
    transaction.commit()?;
    Ok(())
}

fn report_board_post(
    connection: &mut Connection,
    actor: CharacterId,
    public_id: &str,
    reason: &str,
    now: i64,
) -> Result<i64, StoreError> {
    if !valid_board_public_id(public_id) {
        return Err(StoreError::InvalidBoardInput);
    }
    let reason = normalise_board_text(reason, tiamud_core::MAX_BOARD_REPORT_REASON_BYTES)?;
    let evidence_expires_at = now
        .checked_add(BOARD_REPORT_EVIDENCE_RETENTION_SECONDS)
        .ok_or(StoreError::InvalidBoardInput)?;
    let transaction = connection.transaction()?;
    let reporter = social_profile(&transaction, actor, now)?;
    let post = transaction
        .query_row(
            "SELECT posts.author_character_id, authors.account_id, posts.board_key,
                    posts.author_name, posts.subject, posts.body,
                    posts.created_at, posts.report_id
             FROM social_board_posts AS posts
             JOIN characters AS authors ON authors.id = posts.author_character_id
             WHERE posts.public_id = ?1 AND posts.expires_at > ?2",
            params![public_id, now],
            |row| {
                Ok((
                    CharacterId(row.get(0)?),
                    AccountId(row.get(1)?),
                    row.get::<_, String>(2)?,
                    row.get::<_, String>(3)?,
                    row.get::<_, String>(4)?,
                    row.get::<_, String>(5)?,
                    row.get::<_, i64>(6)?,
                    row.get::<_, Option<i64>>(7)?,
                ))
            },
        )
        .optional()?
        .ok_or(StoreError::BoardPostUnavailable)?;
    if post.7.is_some() {
        return Err(StoreError::BoardAlreadyReported);
    }
    if post.1 == reporter.account_id {
        return Err(StoreError::BoardPostUnavailable);
    }
    let open_reports = transaction.query_row(
        "SELECT COUNT(*)
         FROM social_reports AS reports
         JOIN characters AS reporter ON reporter.id = reports.reporter_character_id
         WHERE reporter.account_id = ?1 AND reports.report_kind = 'board'
           AND reports.status = 'open'",
        [reporter.account_id.0],
        |row| row.get::<_, i64>(0),
    )?;
    check_board_report_capacity(open_reports, 0)?;
    let evidence_count = transaction.query_row(
        "SELECT COUNT(*) FROM social_board_report_evidence WHERE expires_at > ?1",
        [now],
        |row| row.get::<_, i64>(0),
    )?;
    check_board_report_capacity(open_reports, evidence_count)?;
    transaction.execute(
        "INSERT INTO social_reports
             (reporter_character_id, target_character_id, report_kind, reason, created_at)
         VALUES (?1, ?2, 'board', ?3, ?4)",
        params![actor.0, post.0.0, reason, now],
    )?;
    let report_id = transaction.last_insert_rowid();
    transaction.execute(
        "INSERT INTO social_board_report_evidence
             (report_id, board_key, post_public_id, author_name,
              subject, body, posted_at, created_at, expires_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
        params![
            report_id,
            post.2,
            public_id,
            post.3,
            post.4,
            post.5,
            post.6,
            now,
            evidence_expires_at
        ],
    )?;
    let linked = transaction.execute(
        "UPDATE social_board_posts SET report_id = ?1
         WHERE public_id = ?2 AND report_id IS NULL AND expires_at > ?3",
        params![report_id, public_id, now],
    )?;
    if linked != 1 {
        return Err(StoreError::BoardAlreadyReported);
    }
    append_audit(
        &transaction,
        reporter.account_id,
        Some(post.1),
        "board_report",
        serde_json::json!({"reportId": report_id}),
        now,
    )?;
    transaction.commit()?;
    Ok(report_id)
}

fn purge_expired_boards(connection: &mut Connection, now: i64) -> Result<u64, StoreError> {
    let transaction = connection.transaction()?;
    let posts = transaction.execute(
        "DELETE FROM social_board_posts
         WHERE id IN (
             SELECT id FROM social_board_posts
             WHERE expires_at <= ?1 ORDER BY expires_at, id LIMIT ?2
         )",
        params![now, BOARD_PURGE_LIMIT],
    )?;
    let remaining = BOARD_PURGE_LIMIT.saturating_sub(i64::try_from(posts).unwrap_or(i64::MAX));
    let evidence = if remaining > 0 {
        transaction.execute(
            "DELETE FROM social_board_report_evidence
             WHERE report_id IN (
                 SELECT report_id FROM social_board_report_evidence
                 WHERE expires_at <= ?1 ORDER BY expires_at, report_id LIMIT ?2
             )",
            params![now, remaining],
        )?
    } else {
        0
    };
    transaction.commit()?;
    u64::try_from(posts.saturating_add(evidence)).map_err(|_| StoreError::InvalidBoardInput)
}

fn report_character(
    connection: &mut Connection,
    actor: CharacterId,
    target: CharacterId,
    reason: &str,
    now: i64,
) -> Result<(), StoreError> {
    if actor == target {
        return Err(StoreError::SocialSelfTarget);
    }
    let reason = normalise_social_text(reason, MAX_REPORT_REASON_BYTES)?;
    let transaction = connection.transaction()?;
    let actor_profile = social_profile(&transaction, actor, now)?;
    let target_profile = social_profile(&transaction, target, now)?;
    if actor_profile.account_id == target_profile.account_id {
        return Err(StoreError::SocialSelfTarget);
    }
    transaction.execute(
        "INSERT INTO social_reports
             (reporter_character_id, target_character_id, reason, created_at)
         VALUES (?1, ?2, ?3, ?4)",
        params![actor.0, target.0, reason, now],
    )?;
    append_audit(
        &transaction,
        actor_profile.account_id,
        Some(target_profile.account_id),
        "report",
        serde_json::json!({"reportId": transaction.last_insert_rowid()}),
        now,
    )?;
    transaction.commit()?;
    Ok(())
}

fn staff_reports(
    connection: &mut Connection,
    audit: StaffAuditRecord,
    now: i64,
) -> Result<Vec<StaffReport>, StoreError> {
    let audit = prepare_staff_audit(audit)?;
    let transaction = connection.transaction()?;
    let actor = validate_staff_authority(
        &transaction,
        &audit,
        Some(StaffCapability::ReportsView),
        None,
    )?;
    let actor_rank = actor
        .profile
        .ok_or(StoreError::StaffForbidden)?
        .authority_rank();
    insert_staff_audit(&transaction, &audit, now)?;
    let reports = {
        let mut statement = transaction.prepare(
            "SELECT reports.id,
                    reporter.id, reporter.name,
                    target.id, target.account_id, target.name,
                    reports.reason, reports.created_at,
                    evidence.sender_name, evidence.subject, evidence.body,
                    evidence.sent_at, evidence.created_at, evidence.expires_at,
                    board_evidence.board_key, board_evidence.post_public_id,
                    board_evidence.author_name, board_evidence.subject,
                    board_evidence.body, board_evidence.posted_at,
                    board_evidence.created_at, board_evidence.expires_at
             FROM social_reports AS reports
             JOIN characters AS reporter ON reporter.id = reports.reporter_character_id
             JOIN characters AS target ON target.id = reports.target_character_id
             LEFT JOIN social_mail_report_evidence AS evidence
                    ON evidence.report_id = reports.id AND evidence.expires_at > ?3
             LEFT JOIN social_board_report_evidence AS board_evidence
                    ON board_evidence.report_id = reports.id
                   AND board_evidence.expires_at > ?3
             LEFT JOIN staff_profiles AS target_staff
                    ON target_staff.account_id = target.account_id
             WHERE reports.status = 'open'
               AND CASE target_staff.profile_key
                       WHEN 'helper' THEN 10
                       WHEN 'builder' THEN 20
                       WHEN 'moderator' THEN 30
                       WHEN 'administrator' THEN 40
                       WHEN 'implementor' THEN 50
                       ELSE 0
                   END < ?1
             ORDER BY reports.created_at, reports.id
             LIMIT ?2",
        )?;
        statement
            .query_map(
                params![i64::from(actor_rank), STAFF_LIST_LIMIT, now],
                |row| {
                    let evidence_sender = row.get::<_, Option<String>>(8)?;
                    let board_key = row.get::<_, Option<String>>(14)?;
                    Ok(StaffReport {
                        id: row.get(0)?,
                        reporter_character_id: CharacterId(row.get(1)?),
                        reporter_name: row.get(2)?,
                        target_character_id: CharacterId(row.get(3)?),
                        target_account_id: AccountId(row.get(4)?),
                        target_name: row.get(5)?,
                        reason: row.get(6)?,
                        created_at: row.get(7)?,
                        mail_evidence: match evidence_sender {
                            Some(sender_name) => Some(MailReportEvidence {
                                sender_name,
                                subject: row.get(9)?,
                                body: row.get(10)?,
                                sent_at: row.get(11)?,
                                created_at: row.get(12)?,
                                expires_at: row.get(13)?,
                            }),
                            None => None,
                        },
                        board_evidence: match board_key {
                            Some(board_key) => Some(BoardReportEvidence {
                                board: BoardKey::parse(&board_key).ok_or_else(|| {
                                    rusqlite::Error::FromSqlConversionFailure(
                                        14,
                                        Type::Text,
                                        Box::new(io::Error::new(
                                            io::ErrorKind::InvalidData,
                                            "invalid board key",
                                        )),
                                    )
                                })?,
                                post_public_id: row.get(15)?,
                                author_name: row.get(16)?,
                                subject: row.get(17)?,
                                body: row.get(18)?,
                                posted_at: row.get(19)?,
                                created_at: row.get(20)?,
                                expires_at: row.get(21)?,
                            }),
                            None => None,
                        },
                    })
                },
            )?
            .collect::<rusqlite::Result<Vec<_>>>()?
    };
    transaction.commit()?;
    Ok(reports)
}

fn resolve_staff_report(
    connection: &mut Connection,
    report_id: i64,
    resolution: StaffReportResolution,
    resolution_reason: &str,
    audit: StaffAuditRecord,
    now: i64,
) -> Result<(), StoreError> {
    if report_id <= 0 {
        return Err(StoreError::StaffReportUnavailable(report_id));
    }
    let resolution_reason = normalise_staff_text(resolution_reason, MAX_REPORT_REASON_BYTES)?;
    let mut audit = prepare_staff_audit(audit)?;
    let transaction = connection.transaction()?;
    let target_account = transaction
        .query_row(
            "SELECT target.account_id
             FROM social_reports AS reports
             JOIN characters AS target ON target.id = reports.target_character_id
             WHERE reports.id = ?1 AND reports.status = 'open'",
            [report_id],
            |row| row.get::<_, i64>(0).map(AccountId),
        )
        .optional()?
        .ok_or(StoreError::StaffReportUnavailable(report_id))?;
    if audit
        .target_account_id
        .is_some_and(|account_id| account_id != target_account)
    {
        return Err(StoreError::StaffForbidden);
    }
    audit.target_account_id = Some(target_account);
    validate_staff_authority(
        &transaction,
        &audit,
        Some(StaffCapability::ReportsResolve),
        Some(target_account),
    )?;
    let updated = transaction.execute(
        "UPDATE social_reports
         SET status = 'resolved', resolution = ?1, resolution_reason = ?2,
             resolved_by_account_id = ?3, resolved_at = ?4
         WHERE id = ?5 AND status = 'open'",
        params![
            resolution.as_str(),
            resolution_reason,
            audit.actor_account_id.0,
            now,
            report_id
        ],
    )?;
    if updated != 1 {
        return Err(StoreError::StaffReportUnavailable(report_id));
    }
    if resolution == StaffReportResolution::ActionTaken {
        transaction.execute(
            "DELETE FROM social_mail_messages
             WHERE report_id = ?1 AND expires_at > ?2",
            params![report_id, now],
        )?;
        transaction.execute(
            "DELETE FROM social_board_posts
             WHERE report_id = ?1 AND expires_at > ?2",
            params![report_id, now],
        )?;
    }
    insert_staff_audit(&transaction, &audit, now)?;
    transaction.commit()?;
    Ok(())
}

fn set_staff_profile(
    connection: &mut Connection,
    target_account: AccountId,
    profile: Option<StaffProfileKey>,
    lore_title: Option<&str>,
    audit: StaffAuditRecord,
    now: i64,
) -> Result<(), StoreError> {
    let lore_title = normalise_lore_title(lore_title)?;
    if profile.is_none() && lore_title.is_some() {
        return Err(StoreError::InvalidStaffInput);
    }
    let audit = prepare_staff_audit(audit)?;
    let transaction = connection.transaction()?;
    let actor = validate_staff_authority(
        &transaction,
        &audit,
        Some(StaffCapability::StaffAssign),
        Some(target_account),
    )?;
    let actor_profile = actor.profile.ok_or(StoreError::StaffForbidden)?;
    if profile.is_some_and(|profile| profile.authority_rank() >= actor_profile.authority_rank()) {
        return Err(StoreError::StaffHierarchy);
    }
    if profile.is_some_and(|profile| {
        profile
            .default_capabilities()
            .iter()
            .any(|capability| !actor.capabilities.contains(capability))
    }) {
        return Err(StoreError::StaffForbidden);
    }
    transaction.execute(
        "DELETE FROM staff_capability_assignments WHERE account_id = ?1",
        [target_account.0],
    )?;
    if let Some(profile) = profile {
        transaction.execute(
            "INSERT INTO staff_profiles
                 (account_id, profile_key, lore_title, assigned_by_account_id,
                  created_at, updated_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?5)
             ON CONFLICT(account_id) DO UPDATE SET
                 profile_key = excluded.profile_key,
                 lore_title = excluded.lore_title,
                 assigned_by_account_id = excluded.assigned_by_account_id,
                 updated_at = excluded.updated_at",
            params![
                target_account.0,
                profile.as_str(),
                lore_title,
                audit.actor_account_id.0,
                now
            ],
        )?;
        for capability in profile.default_capabilities() {
            transaction.execute(
                "INSERT INTO staff_capability_assignments
                     (account_id, capability, assigned_by_account_id, created_at)
                 VALUES (?1, ?2, ?3, ?4)",
                params![
                    target_account.0,
                    capability.as_str(),
                    audit.actor_account_id.0,
                    now
                ],
            )?;
        }
    } else {
        transaction.execute(
            "DELETE FROM staff_profiles WHERE account_id = ?1",
            [target_account.0],
        )?;
    }
    insert_staff_audit(&transaction, &audit, now)?;
    transaction.commit()?;
    Ok(())
}

fn set_staff_capability(
    connection: &mut Connection,
    target_account: AccountId,
    capability: StaffCapability,
    enabled: bool,
    audit: StaffAuditRecord,
    now: i64,
) -> Result<(), StoreError> {
    let audit = prepare_staff_audit(audit)?;
    let transaction = connection.transaction()?;
    let actor = validate_staff_authority(
        &transaction,
        &audit,
        Some(StaffCapability::StaffAssign),
        Some(target_account),
    )?;
    let target = staff_access_by_account(&transaction, target_account)?;
    if enabled && (target.profile.is_none() || !actor.capabilities.contains(&capability)) {
        return Err(StoreError::StaffForbidden);
    }
    if enabled {
        transaction.execute(
            "INSERT INTO staff_capability_assignments
                 (account_id, capability, assigned_by_account_id, created_at)
             VALUES (?1, ?2, ?3, ?4)
             ON CONFLICT(account_id, capability) DO UPDATE SET
                 assigned_by_account_id = excluded.assigned_by_account_id,
                 created_at = excluded.created_at",
            params![
                target_account.0,
                capability.as_str(),
                audit.actor_account_id.0,
                now
            ],
        )?;
    } else {
        transaction.execute(
            "DELETE FROM staff_capability_assignments
             WHERE account_id = ?1 AND capability = ?2",
            params![target_account.0, capability.as_str()],
        )?;
    }
    insert_staff_audit(&transaction, &audit, now)?;
    transaction.commit()?;
    Ok(())
}

fn staff_audit_entries(
    connection: &mut Connection,
    audit: StaffAuditRecord,
    now: i64,
) -> Result<Vec<StaffAuditEntry>, StoreError> {
    let audit = prepare_staff_audit(audit)?;
    let transaction = connection.transaction()?;
    validate_staff_authority(
        &transaction,
        &audit,
        Some(StaffCapability::StaffAudit),
        None,
    )?;
    insert_staff_audit(&transaction, &audit, now)?;
    let entries = {
        let mut statement = transaction.prepare(
            "SELECT sequence, actor_account_id, target_account_id, capability,
                    action, reason, details_json, created_at
             FROM staff_audit ORDER BY sequence DESC LIMIT ?1",
        )?;
        let raw = statement
            .query_map([STAFF_LIST_LIMIT], |row| {
                Ok((
                    row.get::<_, i64>(0)?,
                    row.get::<_, i64>(1)?,
                    row.get::<_, Option<i64>>(2)?,
                    row.get::<_, String>(3)?,
                    row.get::<_, String>(4)?,
                    row.get::<_, String>(5)?,
                    row.get::<_, String>(6)?,
                    row.get::<_, i64>(7)?,
                ))
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        raw.into_iter()
            .map(|row| {
                Ok(StaffAuditEntry {
                    sequence: row.0,
                    actor_account_id: AccountId(row.1),
                    target_account_id: row.2.map(AccountId),
                    capability: StaffCapability::parse(&row.3).ok_or_else(|| {
                        StoreError::InvalidStaffState(format!("capability {:?}", row.3))
                    })?,
                    action: row.4,
                    reason: row.5,
                    details_json: row.6,
                    created_at: row.7,
                })
            })
            .collect::<Result<Vec<_>, StoreError>>()?
    };
    transaction.commit()?;
    Ok(entries)
}

fn append_staff_audit_transaction(
    connection: &mut Connection,
    audit: StaffAuditRecord,
    now: i64,
) -> Result<(), StoreError> {
    let audit = prepare_staff_audit(audit)?;
    let transaction = connection.transaction()?;
    validate_staff_authority(&transaction, &audit, None, audit.target_account_id)?;
    insert_staff_audit(&transaction, &audit, now)?;
    transaction.commit()?;
    Ok(())
}

fn moderate_character(
    connection: &mut Connection,
    actor: CharacterId,
    target: CharacterId,
    action: ModerationAction,
    now: i64,
) -> Result<(), StoreError> {
    if actor == target {
        return Err(StoreError::SocialSelfTarget);
    }
    let (required_capability, action_name, reason, details) = match &action {
        ModerationAction::Mute { expires_at, reason } => {
            validate_expiry(*expires_at, now)?;
            let reason = normalise_staff_text(reason, MAX_SOCIAL_REASON_BYTES)?;
            (
                StaffCapability::ModerationMute,
                "mute",
                reason.clone(),
                serde_json::json!({"expiresAt": expires_at, "reason": reason}),
            )
        }
        ModerationAction::Unmute { reason } => {
            let reason = normalise_staff_text(reason, MAX_SOCIAL_REASON_BYTES)?;
            (
                StaffCapability::ModerationMute,
                "unmute",
                reason.clone(),
                serde_json::json!({"reason": reason}),
            )
        }
        ModerationAction::Ban { expires_at, reason } => {
            validate_expiry(*expires_at, now)?;
            let reason = normalise_staff_text(reason, MAX_SOCIAL_REASON_BYTES)?;
            (
                StaffCapability::ModerationBan,
                "ban",
                reason.clone(),
                serde_json::json!({"expiresAt": expires_at, "reason": reason}),
            )
        }
        ModerationAction::Unban { reason } => {
            let reason = normalise_staff_text(reason, MAX_SOCIAL_REASON_BYTES)?;
            (
                StaffCapability::ModerationBan,
                "unban",
                reason.clone(),
                serde_json::json!({"reason": reason}),
            )
        }
        ModerationAction::SetRole { role, reason } => {
            let reason = normalise_staff_text(reason, MAX_SOCIAL_REASON_BYTES)?;
            (
                StaffCapability::StaffAssign,
                "set_role",
                reason.clone(),
                serde_json::json!({"role": role_as_database(*role), "reason": reason}),
            )
        }
    };
    let transaction = connection.transaction()?;
    let actor_profile = social_profile(&transaction, actor, now)?;
    let target_profile = social_profile(&transaction, target, now)?;
    if actor_profile.account_id == target_profile.account_id {
        return Err(StoreError::SocialSelfTarget);
    }
    let staff_audit = prepare_staff_audit(StaffAuditRecord {
        actor_account_id: actor_profile.account_id,
        target_account_id: Some(target_profile.account_id),
        capability: required_capability,
        action: action_name.to_owned(),
        reason: reason.clone(),
        details: details.clone(),
    })?;
    if let Err(error) = validate_staff_authority(
        &transaction,
        &staff_audit,
        Some(required_capability),
        Some(target_profile.account_id),
    ) {
        return Err(match error {
            StoreError::StaffForbidden | StoreError::StaffHierarchy => StoreError::SocialForbidden,
            error => error,
        });
    }

    match action {
        ModerationAction::Mute { expires_at, .. } => {
            revoke_active(
                &transaction,
                "moderation_mutes",
                target_profile.account_id,
                actor_profile.account_id,
                now,
            )?;
            transaction.execute(
                "INSERT INTO moderation_mutes
                     (account_id, imposed_by_account_id, reason, created_at, expires_at)
                 VALUES (?1, ?2, ?3, ?4, ?5)",
                params![
                    target_profile.account_id.0,
                    actor_profile.account_id.0,
                    &reason,
                    now,
                    expires_at
                ],
            )?;
        }
        ModerationAction::Unmute { .. } => {
            revoke_active(
                &transaction,
                "moderation_mutes",
                target_profile.account_id,
                actor_profile.account_id,
                now,
            )?;
        }
        ModerationAction::Ban { expires_at, .. } => {
            revoke_active(
                &transaction,
                "account_bans",
                target_profile.account_id,
                actor_profile.account_id,
                now,
            )?;
            transaction.execute(
                "INSERT INTO account_bans
                     (account_id, imposed_by_account_id, reason, created_at, expires_at)
                 VALUES (?1, ?2, ?3, ?4, ?5)",
                params![
                    target_profile.account_id.0,
                    actor_profile.account_id.0,
                    &reason,
                    now,
                    expires_at
                ],
            )?;
            transaction.execute(
                "UPDATE auth_sessions SET revoked_at = COALESCE(revoked_at, ?1)
                 WHERE account_id = ?2",
                params![now, target_profile.account_id.0],
            )?;
        }
        ModerationAction::Unban { .. } => {
            revoke_active(
                &transaction,
                "account_bans",
                target_profile.account_id,
                actor_profile.account_id,
                now,
            )?;
        }
        ModerationAction::SetRole { role, .. } => {
            transaction.execute(
                "UPDATE accounts SET role = ?1 WHERE id = ?2",
                params![role_as_database(role), target_profile.account_id.0],
            )?;
        }
    }
    append_audit(
        &transaction,
        actor_profile.account_id,
        Some(target_profile.account_id),
        action_name,
        details,
        now,
    )?;
    insert_staff_audit(&transaction, &staff_audit, now)?;
    transaction.commit()?;
    Ok(())
}

const MAX_MODERATION_DURATION_SECONDS: i64 = 365 * 24 * 60 * 60;

fn validate_expiry(expires_at: Option<i64>, now: i64) -> Result<(), StoreError> {
    if expires_at.is_some_and(|expires_at| {
        expires_at <= now || expires_at.saturating_sub(now) > MAX_MODERATION_DURATION_SECONDS
    }) {
        Err(StoreError::InvalidSocialInput)
    } else {
        Ok(())
    }
}

fn revoke_active(
    connection: &Connection,
    table: &str,
    target: AccountId,
    actor: AccountId,
    now: i64,
) -> Result<(), StoreError> {
    debug_assert!(matches!(table, "moderation_mutes" | "account_bans"));
    connection.execute(
        &format!(
            "UPDATE {table}
             SET revoked_at = ?1, revoked_by_account_id = ?2
             WHERE account_id = ?3 AND revoked_at IS NULL
               AND (expires_at IS NULL OR expires_at > ?1)"
        ),
        params![now, actor.0, target.0],
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

fn append_audit(
    connection: &Connection,
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
         VALUES (?1, ?2, ?3, ?4, ?5)",
        params![
            actor.0,
            target.map(|target| target.0),
            action,
            details_json,
            now
        ],
    )?;
    Ok(())
}

#[cfg(test)]
fn audit_log(connection: &Connection) -> Result<Vec<AuditEntry>, StoreError> {
    let mut statement = connection.prepare(
        "SELECT sequence, actor_account_id, target_account_id, action, details_json, created_at
         FROM moderation_audit ORDER BY sequence",
    )?;
    statement
        .query_map([], |row| {
            Ok(AuditEntry {
                sequence: row.get(0)?,
                actor_account_id: AccountId(row.get(1)?),
                target_account_id: row.get::<_, Option<i64>>(2)?.map(AccountId),
                action: row.get(3)?,
                details_json: row.get(4)?,
                created_at: row.get(5)?,
            })
        })?
        .collect::<rusqlite::Result<Vec<_>>>()
        .map_err(StoreError::from)
}

#[cfg(test)]
fn load_or_create_account(
    connection: &Connection,
    name: &str,
) -> Result<AccountSnapshot, StoreError> {
    let name = normalise_name(name, "account")?;
    connection.execute(
        "INSERT INTO accounts (name, auth_state)
         VALUES (?1, 'guest') ON CONFLICT(name) DO NOTHING",
        [&name],
    )?;
    connection
        .query_row(
            "SELECT id, name FROM accounts
             WHERE name = ?1 AND auth_state = 'guest'",
            [&name],
            |row| {
                Ok(AccountSnapshot {
                    id: AccountId(row.get(0)?),
                    name: row.get(1)?,
                })
            },
        )
        .optional()?
        .ok_or(StoreError::AccountAlreadyExists)
}

fn list_characters(
    connection: &Connection,
    account_id: AccountId,
) -> Result<Vec<CharacterSnapshot>, StoreError> {
    ensure_account_exists(connection, account_id)?;
    let mut statement = connection.prepare(
        "SELECT id, account_id, name, room_id, item_locations_json
         FROM characters WHERE account_id = ?1 ORDER BY name COLLATE NOCASE, id",
    )?;
    let snapshots = statement
        .query_map([account_id.0], character_from_row)?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(snapshots)
}

#[cfg(test)]
fn load_or_create_character(
    connection: &Connection,
    account_id: AccountId,
    name: &str,
    start_room: &str,
) -> Result<CharacterSnapshot, StoreError> {
    load_or_create_character_with_gameplay(
        connection,
        account_id,
        name,
        start_room,
        &CharacterGameplaySnapshot::default(),
    )
}

fn load_or_create_character_with_gameplay(
    connection: &Connection,
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
         VALUES (?1, ?2, ?3, ?4)
         ON CONFLICT(account_id, name) DO NOTHING",
        params![account_id.0, name, start_room, gameplay_json],
    )?;
    connection
        .query_row(
            "SELECT id, account_id, name, room_id, item_locations_json
             FROM characters WHERE account_id = ?1 AND name = ?2",
            params![account_id.0, name],
            character_from_row,
        )
        .map_err(StoreError::from)
}

fn load_character(
    connection: &Connection,
    account_id: AccountId,
    character_id: CharacterId,
) -> Result<CharacterSnapshot, StoreError> {
    ensure_account_exists(connection, account_id)?;
    connection
        .query_row(
            "SELECT id, account_id, name, room_id, item_locations_json
             FROM characters WHERE account_id = ?1 AND id = ?2",
            params![account_id.0, character_id.0],
            character_from_row,
        )
        .optional()?
        .ok_or(StoreError::CharacterNotFound {
            account_id,
            character_id,
        })
}

fn character_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<CharacterSnapshot> {
    let item_locations_json = row.get::<_, String>(4)?;
    let item_locations = serde_json::from_str(&item_locations_json).map_err(|error| {
        rusqlite::Error::FromSqlConversionFailure(4, Type::Text, Box::new(error))
    })?;
    Ok(CharacterSnapshot {
        id: CharacterId(row.get(0)?),
        account_id: AccountId(row.get(1)?),
        name: row.get(2)?,
        room_id: row.get(3)?,
        item_locations,
    })
}

fn save(connection: &Connection, snapshot: &CharacterSnapshot) -> Result<(), StoreError> {
    let name = normalise_name(&snapshot.name, "character")?;
    let item_locations_json = serde_json::to_string(&snapshot.item_locations)?;
    let updated = connection.execute(
        "UPDATE characters
         SET name = ?1, room_id = ?2, item_locations_json = ?3, updated_at = unixepoch()
         WHERE id = ?4 AND account_id = ?5",
        params![
            name,
            snapshot.room_id,
            item_locations_json,
            snapshot.id.0,
            snapshot.account_id.0
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

fn load_character_gameplay(
    connection: &Connection,
    character_id: CharacterId,
) -> Result<CharacterGameplaySnapshot, StoreError> {
    let json = connection
        .query_row(
            "SELECT gameplay_json FROM characters WHERE id = ?1",
            [character_id.0],
            |row| row.get::<_, String>(0),
        )
        .optional()?
        .ok_or(StoreError::CharacterGameplayNotFound(character_id))?;
    serde_json::from_str(&json).map_err(StoreError::from)
}

fn load_character_gameplay_batch(
    connection: &Connection,
    character_ids: Vec<CharacterId>,
    limits: StartupGameplayLoadLimits,
) -> Result<Vec<DurableCharacterGameplay>, StoreError> {
    let character_ids = normalise_startup_character_ids(character_ids, limits)?;
    let mut aggregates = Vec::new();
    let mut total_json_bytes = 0_usize;
    for batch in character_ids.chunks(SQLITE_CHARACTER_BATCH_SIZE) {
        let placeholders = std::iter::repeat_n("?", batch.len())
            .collect::<Vec<_>>()
            .join(",");
        let sql = format!(
            "SELECT id, gameplay_json FROM characters WHERE id IN ({placeholders}) ORDER BY id"
        );
        let mut statement = connection.prepare(&sql)?;
        let mut rows = statement.query(params_from_iter(batch.iter().map(|id| id.0)))?;
        while let Some(row) = rows.next()? {
            let character_id = CharacterId(row.get(0)?);
            let json: String = row.get(1)?;
            aggregates.push(decode_startup_gameplay(
                character_id,
                &json,
                &mut total_json_bytes,
                limits,
            )?);
        }
    }
    ensure_all_startup_characters_loaded(&character_ids, &aggregates)?;
    Ok(aggregates)
}

fn ensure_character_ids_exist_batch(
    connection: &Connection,
    character_ids: Vec<CharacterId>,
) -> Result<(), StoreError> {
    let character_ids = normalise_character_identity_ids(character_ids)?;
    let mut found = Vec::with_capacity(character_ids.len());
    for batch in character_ids.chunks(SQLITE_CHARACTER_BATCH_SIZE) {
        let placeholders = std::iter::repeat_n("?", batch.len())
            .collect::<Vec<_>>()
            .join(",");
        let sql = format!("SELECT id FROM characters WHERE id IN ({placeholders}) ORDER BY id");
        let mut statement = connection.prepare(&sql)?;
        let rows = statement.query_map(params_from_iter(batch.iter().map(|id| id.0)), |row| {
            row.get::<_, i64>(0).map(CharacterId)
        })?;
        for row in rows {
            found.push(row?);
        }
    }
    ensure_all_character_ids_exist(&character_ids, &found)
}

pub(super) fn normalise_character_identity_ids(
    mut character_ids: Vec<CharacterId>,
) -> Result<Vec<CharacterId>, StoreError> {
    character_ids.sort_unstable();
    character_ids.dedup();
    if character_ids.len() > MAX_STARTUP_CHARACTER_EXISTENCE_IDS {
        return Err(StoreError::TooManyCharacterIdentities {
            maximum: MAX_STARTUP_CHARACTER_EXISTENCE_IDS,
        });
    }
    if let Some(invalid) = character_ids.iter().find(|id| id.0 <= 0) {
        return Err(StoreError::InvalidCharacterIdentity(*invalid));
    }
    Ok(character_ids)
}

pub(super) fn ensure_all_character_ids_exist(
    requested: &[CharacterId],
    found: &[CharacterId],
) -> Result<(), StoreError> {
    let mut found_ids = BTreeSet::new();
    for character_id in found {
        if character_id.0 <= 0 || !found_ids.insert(*character_id) {
            return Err(StoreError::InvalidRuntimeSnapshot);
        }
    }
    if found_ids.len() > requested.len()
        || found_ids
            .iter()
            .any(|character_id| requested.binary_search(character_id).is_err())
    {
        return Err(StoreError::InvalidRuntimeSnapshot);
    }
    requested
        .iter()
        .find(|character_id| !found_ids.contains(character_id))
        .map_or(Ok(()), |missing| {
            Err(StoreError::CharacterIdentityNotFound(*missing))
        })
}

pub(super) fn normalise_startup_character_ids(
    mut character_ids: Vec<CharacterId>,
    limits: StartupGameplayLoadLimits,
) -> Result<Vec<CharacterId>, StoreError> {
    character_ids.sort_unstable();
    character_ids.dedup();
    if character_ids.len() > limits.maximum_characters() {
        return Err(StoreError::TooManyCharacterAggregates {
            maximum: limits.maximum_characters(),
        });
    }
    if let Some(invalid) = character_ids.iter().find(|id| id.0 <= 0) {
        return Err(StoreError::CharacterGameplayNotFound(*invalid));
    }
    Ok(character_ids)
}

pub(super) fn decode_startup_gameplay(
    character_id: CharacterId,
    json: &str,
    total_json_bytes: &mut usize,
    limits: StartupGameplayLoadLimits,
) -> Result<DurableCharacterGameplay, StoreError> {
    if json.len() > MAX_CHARACTER_JSON_BYTES {
        return Err(StoreError::SnapshotTooLarge {
            kind: "character gameplay",
            maximum: MAX_CHARACTER_JSON_BYTES,
        });
    }
    *total_json_bytes =
        total_json_bytes
            .checked_add(json.len())
            .ok_or(StoreError::SnapshotTooLarge {
                kind: limits.kind(),
                maximum: limits.maximum_bytes(),
            })?;
    if *total_json_bytes > limits.maximum_bytes() {
        return Err(StoreError::SnapshotTooLarge {
            kind: limits.kind(),
            maximum: limits.maximum_bytes(),
        });
    }
    Ok(DurableCharacterGameplay {
        character_id,
        gameplay: serde_json::from_str(json)?,
    })
}

pub(super) fn ensure_all_startup_characters_loaded(
    requested: &[CharacterId],
    aggregates: &[DurableCharacterGameplay],
) -> Result<(), StoreError> {
    let mut loaded = BTreeSet::new();
    for aggregate in aggregates {
        if aggregate.character_id.0 <= 0 || !loaded.insert(aggregate.character_id) {
            return Err(StoreError::InvalidRuntimeSnapshot);
        }
    }
    if loaded.len() != requested.len() {
        return requested
            .iter()
            .find(|character_id| !loaded.contains(character_id))
            .map_or(Err(StoreError::InvalidRuntimeSnapshot), |missing| {
                Err(StoreError::CharacterGameplayNotFound(*missing))
            });
    }
    requested
        .iter()
        .find(|character_id| !loaded.contains(character_id))
        .map_or(Ok(()), |missing| {
            Err(StoreError::CharacterGameplayNotFound(*missing))
        })
}

fn load_runtime_snapshot(
    connection: &Connection,
) -> Result<Option<WorldRuntimeSnapshot>, StoreError> {
    load_durable_runtime_snapshot(connection).map(|snapshot| snapshot.map(|(_, snapshot)| snapshot))
}

fn load_durable_runtime_snapshot(
    connection: &Connection,
) -> Result<Option<(u32, WorldRuntimeSnapshot)>, StoreError> {
    connection
        .query_row(
            "SELECT snapshot_version, snapshot_json FROM world_runtime WHERE singleton = 1",
            [],
            |row| Ok((row.get::<_, i64>(0)?, row.get::<_, String>(1)?)),
        )
        .optional()?
        .map(|(snapshot_version, json)| {
            let durable_version =
                u32::try_from(snapshot_version).map_err(|_| StoreError::InvalidRuntimeSnapshot)?;
            decode_runtime_snapshot(snapshot_version, &json)
                .map(|snapshot| (durable_version, snapshot))
        })
        .transpose()
}

fn has_durable_character_authority(connection: &Connection) -> Result<bool, StoreError> {
    connection
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM characters LIMIT 1)",
            [],
            |row| row.get(0),
        )
        .map_err(StoreError::from)
}

pub(super) fn validate_area_generation(
    generation: &DurableAreaGeneration,
) -> Result<(), StoreError> {
    let valid_world_id = !generation.world_id.is_empty()
        && generation.world_id.len() <= 256
        && generation.world_id.trim() == generation.world_id
        && !generation.world_id.chars().any(|character| {
            character.is_control()
                || matches!(
                    character,
                    '\u{061c}'
                        | '\u{200b}'
                        | '\u{200e}'
                        | '\u{200f}'
                        | '\u{202a}'..='\u{202e}'
                        | '\u{2066}'..='\u{2069}'
                        | '\u{feff}'
                )
        });
    let valid_digest = |digest: &str| {
        digest.len() == 64
            && digest
                .bytes()
                .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    };
    if !valid_world_id
        || !valid_digest(&generation.active_catalog_sha256)
        || generation
            .previous_catalog_sha256
            .as_deref()
            .is_some_and(|digest| {
                !valid_digest(digest) || digest == generation.active_catalog_sha256
            })
        || generation.activation_sequence == 0
        || generation.activation_sequence > i64::MAX as u64
    {
        return Err(StoreError::InvalidAreaGeneration);
    }
    Ok(())
}

pub(super) fn validate_area_generation_transition(
    expected: &DurableAreaGeneration,
    next: &DurableAreaGeneration,
) -> Result<(), StoreError> {
    validate_area_generation(expected)?;
    validate_area_generation(next)?;
    if next.world_id != expected.world_id
        || next.previous_catalog_sha256.as_deref() != Some(expected.active_catalog_sha256.as_str())
        || next.active_catalog_sha256 == expected.active_catalog_sha256
        || next.activation_sequence != expected.activation_sequence.saturating_add(1)
    {
        return Err(StoreError::InvalidAreaGeneration);
    }
    Ok(())
}

fn valid_sha256(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

pub(super) fn area_checkpoint_unit_id(checkpoint: &AreaDynamicCheckpoint) -> String {
    let mut digest = Sha256::new();
    digest.update(b"tiamud-area-retirement-unit-v1\0");
    digest.update(checkpoint.generation_sha256.as_bytes());
    for area in &checkpoint.areas {
        let revision = area.revision.to_string();
        for value in [
            area.area_id.as_str(),
            revision.as_str(),
            area.bundle_sha256.as_str(),
        ] {
            digest.update((value.len() as u64).to_be_bytes());
            digest.update(value.as_bytes());
        }
    }
    hex::encode(digest.finalize())
}

pub(super) fn validate_area_checkpoint(
    durable: &DurableAreaCheckpoint,
) -> Result<AreaDynamicCheckpoint, StoreError> {
    if !valid_sha256(&durable.generation_sha256)
        || !valid_sha256(&durable.unit_id)
        || !valid_sha256(&durable.checkpoint_sha256)
        || durable.residency_epoch == 0
        || durable.residency_epoch > i64::MAX as u64
        || durable.checkpoint_version != tiamud_core::AREA_DYNAMIC_CHECKPOINT_VERSION
        || durable.captured_logical_tick > i64::MAX as u64
        || durable.checkpoint_json.is_empty()
        || durable.checkpoint_json.len() > MAX_RUNTIME_JSON_BYTES
        || hex::encode(Sha256::digest(durable.checkpoint_json.as_bytes()))
            != durable.checkpoint_sha256
    {
        return Err(StoreError::InvalidAreaCheckpoint);
    }
    let checkpoint: AreaDynamicCheckpoint = serde_json::from_str(&durable.checkpoint_json)
        .map_err(|_| StoreError::InvalidAreaCheckpoint)?;
    if serde_json::to_string(&checkpoint).map_err(|_| StoreError::InvalidAreaCheckpoint)?
        != durable.checkpoint_json
        || tiamud_core::validate_area_dynamic_checkpoint(&checkpoint).is_err()
        || area_checkpoint_unit_id(&checkpoint) != durable.unit_id
        || checkpoint.generation_sha256 != durable.generation_sha256
        || checkpoint.checkpoint_version != durable.checkpoint_version
        || checkpoint.captured_logical_tick != durable.captured_logical_tick
        || checkpoint
            .areas
            .iter()
            .any(|area| area.residency_epoch != durable.residency_epoch)
    {
        return Err(StoreError::InvalidAreaCheckpoint);
    }
    Ok(checkpoint)
}

pub(super) fn validate_area_retirement_commit(
    runtime: &WorldRuntimeSnapshot,
    generation: &DurableAreaGeneration,
    durable: &DurableAreaCheckpoint,
) -> Result<AreaDynamicCheckpoint, StoreError> {
    validate_area_generation(generation)?;
    let checkpoint = validate_area_checkpoint(durable)?;
    if generation.active_catalog_sha256 != durable.generation_sha256
        || runtime.snapshot_version != tiamud_core::WORLD_RUNTIME_SNAPSHOT_VERSION
        || tiamud_core::validate_world_runtime_snapshot_semantics(runtime).is_err()
    {
        return Err(StoreError::InvalidAreaCheckpoint);
    }
    let checkpoint_areas = checkpoint
        .areas
        .iter()
        .map(|area| area.area_id.as_str())
        .collect::<BTreeSet<_>>();
    for area in &checkpoint.areas {
        let Some(AreaRuntimeResidence::Retired {
            revision,
            bundle_sha256,
            residency_epoch,
            checkpoint_version,
            checkpoint_sha256,
        }) = runtime.area_residency.get(&area.area_id)
        else {
            return Err(StoreError::InvalidAreaCheckpoint);
        };
        if *revision != area.revision
            || bundle_sha256 != &area.bundle_sha256
            || *residency_epoch != area.residency_epoch
            || *checkpoint_version != durable.checkpoint_version
            || checkpoint_sha256 != &durable.checkpoint_sha256
        {
            return Err(StoreError::InvalidAreaCheckpoint);
        }
    }
    if runtime.area_residency.iter().any(|(area_id, state)| {
        matches!(state, AreaRuntimeResidence::Retired { checkpoint_sha256, .. }
            if checkpoint_sha256 == &durable.checkpoint_sha256)
            && !checkpoint_areas.contains(area_id.as_str())
    }) {
        return Err(StoreError::InvalidAreaCheckpoint);
    }
    Ok(checkpoint)
}

pub(super) fn validate_area_rehydration_commit(
    runtime: &WorldRuntimeSnapshot,
    generation: &DurableAreaGeneration,
    durable: &DurableAreaCheckpoint,
) -> Result<AreaDynamicCheckpoint, StoreError> {
    validate_area_generation(generation)?;
    let checkpoint = validate_area_checkpoint(durable)?;
    if generation.active_catalog_sha256 != durable.generation_sha256
        || runtime.snapshot_version != tiamud_core::WORLD_RUNTIME_SNAPSHOT_VERSION
        || tiamud_core::validate_world_runtime_snapshot_semantics(runtime).is_err()
    {
        return Err(StoreError::InvalidAreaCheckpoint);
    }
    for area in &checkpoint.areas {
        let Some(AreaRuntimeResidence::Resident {
            revision,
            bundle_sha256,
            residency_epoch,
        }) = runtime.area_residency.get(&area.area_id)
        else {
            return Err(StoreError::InvalidAreaCheckpoint);
        };
        if *revision != area.revision
            || bundle_sha256 != &area.bundle_sha256
            || area.residency_epoch.checked_add(1) != Some(*residency_epoch)
        {
            return Err(StoreError::InvalidAreaCheckpoint);
        }
    }
    if runtime.area_residency.values().any(|state| {
        matches!(state, AreaRuntimeResidence::Retired { checkpoint_sha256, .. }
            if checkpoint_sha256 == &durable.checkpoint_sha256)
    }) {
        return Err(StoreError::InvalidAreaCheckpoint);
    }
    Ok(checkpoint)
}

pub(super) fn validate_area_retirement_previous_runtime(
    current: &WorldRuntimeSnapshot,
    next: &WorldRuntimeSnapshot,
    checkpoint: &AreaDynamicCheckpoint,
) -> Result<(), StoreError> {
    let mut expected = next.clone();
    for area in &checkpoint.areas {
        let previous_epoch = area
            .residency_epoch
            .checked_sub(1)
            .filter(|epoch| *epoch > 0)
            .ok_or(StoreError::AreaCheckpointConflict)?;
        expected.area_residency.insert(
            area.area_id.clone(),
            AreaRuntimeResidence::Resident {
                revision: area.revision,
                bundle_sha256: area.bundle_sha256.clone(),
                residency_epoch: previous_epoch,
            },
        );
    }
    for (npc_id, npc) in &checkpoint.npcs {
        if expected.npcs.insert(npc_id.clone(), npc.clone()).is_some() {
            return Err(StoreError::AreaCheckpointConflict);
        }
    }
    for (item_id, item) in &checkpoint.item_instances {
        if expected
            .item_instances
            .insert(item_id.clone(), item.clone())
            .is_some()
        {
            return Err(StoreError::AreaCheckpointConflict);
        }
    }
    for (item_id, provenance) in &checkpoint.item_provenance {
        if expected
            .item_provenance
            .insert(item_id.clone(), provenance.clone())
            .is_some()
        {
            return Err(StoreError::AreaCheckpointConflict);
        }
    }
    for (root_id, reservation) in &checkpoint.ground_loot {
        if expected
            .ground_loot
            .insert(root_id.clone(), reservation.clone())
            .is_some()
        {
            return Err(StoreError::AreaCheckpointConflict);
        }
    }
    if current != &expected {
        return Err(StoreError::AreaCheckpointConflict);
    }
    Ok(())
}

pub(super) fn validate_area_rehydration_previous_runtime(
    current: &WorldRuntimeSnapshot,
    next: &WorldRuntimeSnapshot,
    durable: &DurableAreaCheckpoint,
    checkpoint: &AreaDynamicCheckpoint,
) -> Result<(), StoreError> {
    let mut expected = next.clone();
    for area in &checkpoint.areas {
        expected.area_residency.insert(
            area.area_id.clone(),
            AreaRuntimeResidence::Retired {
                revision: area.revision,
                bundle_sha256: area.bundle_sha256.clone(),
                residency_epoch: area.residency_epoch,
                checkpoint_version: durable.checkpoint_version,
                checkpoint_sha256: durable.checkpoint_sha256.clone(),
            },
        );
    }
    for npc_id in checkpoint.npcs.keys() {
        if expected.npcs.remove(npc_id).is_none() {
            return Err(StoreError::AreaCheckpointConflict);
        }
    }
    for item_id in checkpoint.item_instances.keys() {
        expected.item_instances.remove(item_id);
        expected.item_provenance.remove(item_id);
        expected.ground_loot.remove(item_id);
    }
    for root_id in checkpoint.ground_loot.keys() {
        expected.ground_loot.remove(root_id);
    }
    if current != &expected {
        return Err(StoreError::AreaCheckpointConflict);
    }
    Ok(())
}

fn load_area_generation(
    connection: &Connection,
) -> Result<Option<DurableAreaGeneration>, StoreError> {
    let generation = connection
        .query_row(
            "SELECT world_id, active_catalog_sha256, previous_catalog_sha256,
                    activation_sequence
             FROM world_area_generation WHERE singleton = 1",
            [],
            |row| {
                let sequence = row.get::<_, i64>(3)?;
                Ok(DurableAreaGeneration {
                    world_id: row.get(0)?,
                    active_catalog_sha256: row.get(1)?,
                    previous_catalog_sha256: row.get(2)?,
                    activation_sequence: u64::try_from(sequence).unwrap_or(0),
                })
            },
        )
        .optional()?;
    if let Some(generation) = &generation {
        validate_area_generation(generation)?;
    }
    Ok(generation)
}

fn load_area_checkpoints(
    connection: &Connection,
) -> Result<Vec<DurableAreaCheckpoint>, StoreError> {
    let mut statement = connection.prepare(
        "SELECT generation_sha256, unit_id, residency_epoch,
                checkpoint_version, checkpoint_sha256, captured_logical_tick,
                checkpoint_json
         FROM world_area_checkpoints
         ORDER BY generation_sha256, unit_id, residency_epoch
         LIMIT ?1",
    )?;
    let rows = statement
        .query_map(params![MAX_DURABLE_AREA_CHECKPOINTS as i64 + 1], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, i64>(2)?,
                row.get::<_, i64>(3)?,
                row.get::<_, String>(4)?,
                row.get::<_, i64>(5)?,
                row.get::<_, String>(6)?,
            ))
        })?
        .collect::<Result<Vec<_>, _>>()?;
    if rows.len() > MAX_DURABLE_AREA_CHECKPOINTS {
        return Err(StoreError::InvalidAreaCheckpoint);
    }
    let active_generation = load_area_generation(connection)?;
    rows.into_iter()
        .map(
            |(generation, unit_id, epoch, version, sha256, tick, json)| {
                let durable = DurableAreaCheckpoint {
                    generation_sha256: generation,
                    unit_id,
                    residency_epoch: u64::try_from(epoch).unwrap_or(0),
                    checkpoint_version: u32::try_from(version).unwrap_or(0),
                    checkpoint_sha256: sha256,
                    captured_logical_tick: u64::try_from(tick).unwrap_or(u64::MAX),
                    checkpoint_json: json,
                };
                if active_generation
                    .as_ref()
                    .is_none_or(|active| active.active_catalog_sha256 != durable.generation_sha256)
                {
                    return Err(StoreError::InvalidAreaCheckpoint);
                }
                validate_area_checkpoint(&durable)?;
                Ok(durable)
            },
        )
        .collect()
}

fn persist_area_runtime(
    transaction: &rusqlite::Transaction<'_>,
    runtime: &WorldRuntimeSnapshot,
) -> Result<(), StoreError> {
    let runtime_json = bounded_json(runtime, "world runtime", MAX_RUNTIME_JSON_BYTES)?;
    let adventure_json = bounded_json(
        &runtime.adventure,
        "world adventure",
        MAX_RUNTIME_JSON_BYTES,
    )?;
    transaction.execute(
        "INSERT INTO world_adventure (singleton, state_json, updated_at)
         VALUES (1, ?1, unixepoch())
         ON CONFLICT(singleton) DO UPDATE SET
             state_json = excluded.state_json,
             updated_at = excluded.updated_at",
        [adventure_json],
    )?;
    let last_event_sequence = transaction.query_row(
        "SELECT max(
             COALESCE((SELECT MAX(sequence) FROM domain_events), 0),
             COALESCE((SELECT last_event_sequence FROM world_runtime
                       WHERE singleton = 1), 0)
         )",
        [],
        |row| row.get::<_, i64>(0),
    )?;
    transaction.execute(
        "INSERT INTO world_runtime
             (singleton, snapshot_version, snapshot_json,
              last_event_sequence, updated_at)
         VALUES (1, ?1, ?2, ?3, unixepoch())
         ON CONFLICT(singleton) DO UPDATE SET
             snapshot_version = excluded.snapshot_version,
             snapshot_json = excluded.snapshot_json,
             last_event_sequence = excluded.last_event_sequence,
             updated_at = excluded.updated_at",
        params![runtime.snapshot_version, runtime_json, last_event_sequence],
    )?;
    transaction.execute(
        "DELETE FROM domain_events
         WHERE sequence <= ?1 - ?2 AND sequence <= ?1",
        params![last_event_sequence, DOMAIN_EVENT_RETENTION],
    )?;
    Ok(())
}

fn verify_exact_area_generation(
    transaction: &rusqlite::Transaction<'_>,
    expected: &DurableAreaGeneration,
) -> Result<(), StoreError> {
    let sequence = i64::try_from(expected.activation_sequence)
        .map_err(|_| StoreError::InvalidAreaGeneration)?;
    let changed = transaction.execute(
        "UPDATE world_area_generation SET updated_at = updated_at
         WHERE singleton = 1 AND world_id = ?1
           AND active_catalog_sha256 = ?2
           AND previous_catalog_sha256 IS ?3
           AND activation_sequence = ?4",
        params![
            expected.world_id,
            expected.active_catalog_sha256,
            expected.previous_catalog_sha256,
            sequence,
        ],
    )?;
    if changed != 1 {
        return Err(StoreError::AreaGenerationConflict);
    }
    Ok(())
}

fn commit_area_retirement(
    connection: &mut Connection,
    runtime: &WorldRuntimeSnapshot,
    expected: &DurableAreaGeneration,
    durable: &DurableAreaCheckpoint,
) -> Result<(), StoreError> {
    let checkpoint = validate_area_retirement_commit(runtime, expected, durable)?;
    let transaction = connection.transaction()?;
    verify_exact_area_generation(&transaction, expected)?;
    let current = load_runtime_snapshot(&transaction)?.ok_or(StoreError::AreaCheckpointConflict)?;
    validate_area_retirement_previous_runtime(&current, runtime, &checkpoint)?;
    let inserted = transaction.execute(
        "INSERT INTO world_area_checkpoints
             (generation_sha256, unit_id, residency_epoch,
              checkpoint_version, checkpoint_sha256, captured_logical_tick,
              checkpoint_json, created_at, updated_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, unixepoch(), unixepoch())
         ON CONFLICT(generation_sha256, unit_id, residency_epoch) DO NOTHING",
        params![
            durable.generation_sha256,
            durable.unit_id,
            i64::try_from(durable.residency_epoch).unwrap_or(0),
            i64::from(durable.checkpoint_version),
            durable.checkpoint_sha256,
            i64::try_from(durable.captured_logical_tick).unwrap_or(-1),
            durable.checkpoint_json,
        ],
    )?;
    if inserted != 1 {
        return Err(StoreError::AreaCheckpointConflict);
    }
    persist_area_runtime(&transaction, runtime)?;
    transaction.commit()?;
    Ok(())
}

fn commit_area_rehydration(
    connection: &mut Connection,
    runtime: &WorldRuntimeSnapshot,
    expected: &DurableAreaGeneration,
    durable: &DurableAreaCheckpoint,
) -> Result<(), StoreError> {
    let checkpoint = validate_area_rehydration_commit(runtime, expected, durable)?;
    let transaction = connection.transaction()?;
    verify_exact_area_generation(&transaction, expected)?;
    let current = load_runtime_snapshot(&transaction)?.ok_or(StoreError::AreaCheckpointConflict)?;
    validate_area_rehydration_previous_runtime(&current, runtime, durable, &checkpoint)?;
    let deleted = transaction.execute(
        "DELETE FROM world_area_checkpoints
         WHERE generation_sha256 = ?1 AND unit_id = ?2 AND residency_epoch = ?3
           AND checkpoint_version = ?4 AND checkpoint_sha256 = ?5
           AND captured_logical_tick = ?6 AND checkpoint_json = ?7",
        params![
            durable.generation_sha256,
            durable.unit_id,
            i64::try_from(durable.residency_epoch).unwrap_or(0),
            i64::from(durable.checkpoint_version),
            durable.checkpoint_sha256,
            i64::try_from(durable.captured_logical_tick).unwrap_or(-1),
            durable.checkpoint_json,
        ],
    )?;
    if deleted != 1 {
        return Err(StoreError::AreaCheckpointConflict);
    }
    persist_area_runtime(&transaction, runtime)?;
    transaction.commit()?;
    Ok(())
}

fn bind_area_generation_if_absent(
    connection: &mut Connection,
    generation: &DurableAreaGeneration,
) -> Result<DurableAreaGeneration, StoreError> {
    validate_area_generation(generation)?;
    if generation.activation_sequence != 1 || generation.previous_catalog_sha256.is_some() {
        return Err(StoreError::InvalidAreaGeneration);
    }
    let transaction = connection.transaction()?;
    transaction.execute(
        "INSERT INTO world_area_generation
             (singleton, world_id, active_catalog_sha256,
              previous_catalog_sha256, activation_sequence, updated_at)
         VALUES (1, ?1, ?2, NULL, 1, unixepoch())
         ON CONFLICT(singleton) DO NOTHING",
        params![generation.world_id, generation.active_catalog_sha256],
    )?;
    let actual = load_area_generation(&transaction)?.ok_or(StoreError::InvalidAreaGeneration)?;
    transaction.commit()?;
    Ok(actual)
}

fn commit_area_generation_activation(
    connection: &mut Connection,
    runtime: &WorldRuntimeSnapshot,
    expected: &DurableAreaGeneration,
    next: &DurableAreaGeneration,
) -> Result<(), StoreError> {
    validate_area_generation_transition(expected, next)?;
    let runtime_json = bounded_json(runtime, "world runtime", MAX_RUNTIME_JSON_BYTES)?;
    let world_adventure_json = bounded_json(
        &runtime.adventure,
        "world adventure",
        MAX_RUNTIME_JSON_BYTES,
    )?;
    let expected_sequence = i64::try_from(expected.activation_sequence)
        .map_err(|_| StoreError::InvalidAreaGeneration)?;
    let next_sequence =
        i64::try_from(next.activation_sequence).map_err(|_| StoreError::InvalidAreaGeneration)?;
    let transaction = connection.transaction()?;
    let changed = transaction.execute(
        "UPDATE world_area_generation
         SET active_catalog_sha256 = ?1,
             previous_catalog_sha256 = ?2,
             activation_sequence = ?3,
             updated_at = unixepoch()
         WHERE singleton = 1
           AND world_id = ?4
           AND active_catalog_sha256 = ?5
           AND previous_catalog_sha256 IS ?6
           AND activation_sequence = ?7",
        params![
            next.active_catalog_sha256,
            next.previous_catalog_sha256,
            next_sequence,
            expected.world_id,
            expected.active_catalog_sha256,
            expected.previous_catalog_sha256,
            expected_sequence,
        ],
    )?;
    if changed != 1 {
        return Err(StoreError::AreaGenerationConflict);
    }
    transaction.execute(
        "INSERT INTO world_adventure (singleton, state_json, updated_at)
         VALUES (1, ?1, unixepoch())
         ON CONFLICT(singleton) DO UPDATE SET
             state_json = excluded.state_json,
             updated_at = excluded.updated_at",
        [world_adventure_json],
    )?;
    let last_event_sequence = transaction.query_row(
        "SELECT max(
             COALESCE((SELECT MAX(sequence) FROM domain_events), 0),
             COALESCE((SELECT last_event_sequence FROM world_runtime
                       WHERE singleton = 1), 0)
         )",
        [],
        |row| row.get::<_, i64>(0),
    )?;
    transaction.execute(
        "INSERT INTO world_runtime
             (singleton, snapshot_version, snapshot_json,
              last_event_sequence, updated_at)
         VALUES (1, ?1, ?2, ?3, unixepoch())
         ON CONFLICT(singleton) DO UPDATE SET
             snapshot_version = excluded.snapshot_version,
             snapshot_json = excluded.snapshot_json,
             last_event_sequence = excluded.last_event_sequence,
             updated_at = excluded.updated_at",
        params![runtime.snapshot_version, runtime_json, last_event_sequence],
    )?;
    transaction.execute(
        "DELETE FROM domain_events
         WHERE sequence <= ?1 - ?2
           AND sequence <= ?1",
        params![last_event_sequence, DOMAIN_EVENT_RETENTION],
    )?;
    transaction.commit()?;
    Ok(())
}

fn load_startup_world_recovery(
    connection: &mut Connection,
) -> Result<StartupWorldRecovery, StoreError> {
    let transaction = connection.transaction()?;
    let recovery = match load_durable_runtime_snapshot(&transaction)? {
        Some((durable_version, snapshot)) => StartupWorldRecovery::Snapshot {
            durable_version,
            snapshot,
        },
        None => match load_startup_domain_events(&transaction)? {
            events if events.is_empty() => StartupWorldRecovery::Empty,
            events => StartupWorldRecovery::Ledger(events),
        },
    };
    transaction.commit()?;
    Ok(recovery)
}

fn load_startup_domain_events(connection: &Connection) -> Result<Vec<DomainEvent>, StoreError> {
    let mut statement = connection.prepare(
        "SELECT sequence, event_json
         FROM domain_events
         ORDER BY sequence
         LIMIT ?1",
    )?;
    let rows = statement
        .query_map(params![DOMAIN_EVENT_RETENTION + 1], |row| {
            Ok((row.get::<_, i64>(0)?, row.get::<_, String>(1)?))
        })?
        .collect::<Result<Vec<_>, _>>()?;
    decode_startup_domain_events(rows)
}

pub(super) fn decode_startup_domain_events(
    rows: Vec<(i64, String)>,
) -> Result<Vec<DomainEvent>, StoreError> {
    if rows.len() > DOMAIN_EVENT_RETENTION as usize {
        return Err(StoreError::InvalidStartupLedger);
    }

    let mut expected_sequence = 1_i64;
    let mut events = Vec::with_capacity(rows.len());
    for (sequence, json) in rows {
        if sequence != expected_sequence {
            return Err(StoreError::InvalidStartupLedger);
        }
        if json.len() > MAX_EVENT_JSON_BYTES {
            return Err(StoreError::SnapshotTooLarge {
                kind: "domain event",
                maximum: MAX_EVENT_JSON_BYTES,
            });
        }
        events.push(serde_json::from_str(&json)?);
        expected_sequence += 1;
    }
    Ok(events)
}

pub(super) fn decode_runtime_snapshot(
    durable_version: i64,
    json: &str,
) -> Result<WorldRuntimeSnapshot, StoreError> {
    let durable_version =
        u32::try_from(durable_version).map_err(|_| StoreError::InvalidRuntimeSnapshot)?;
    let maximum = if durable_version <= LAST_ONE_MIB_RUNTIME_SNAPSHOT_VERSION {
        tiamud_core::MAX_LEGACY_WORLD_RUNTIME_SNAPSHOT_BYTES
    } else {
        MAX_RUNTIME_JSON_BYTES
    };
    // `str::len` is the UTF-8 byte length, unlike SQLite's text `length()`.
    if json.len() > maximum {
        return Err(StoreError::SnapshotTooLarge {
            kind: "world runtime",
            maximum,
        });
    }
    let snapshot = tiamud_core::decode_world_runtime_snapshot(durable_version, json)
        .map_err(|_| StoreError::InvalidRuntimeSnapshot)?;
    if snapshot.snapshot_version != tiamud_core::WORLD_RUNTIME_SNAPSHOT_VERSION {
        return Err(StoreError::InvalidRuntimeSnapshot);
    }
    // Admission must cover the normalized current representation too. A valid
    // near-limit legacy payload can expand substantially when later versions
    // add explicit per-instance state. Reject it here, before the decoder hands
    // persistence an inadmissible current representation for restoration.
    bounded_json(&snapshot, "world runtime", MAX_RUNTIME_JSON_BYTES)?;
    Ok(snapshot)
}

fn load_domain_events(connection: &Connection) -> Result<Vec<DomainEvent>, StoreError> {
    let mut statement =
        connection.prepare("SELECT event_json FROM domain_events ORDER BY sequence")?;
    statement
        .query_map([], |row| row.get::<_, String>(0))?
        .map(|json| {
            let json = json?;
            serde_json::from_str(&json).map_err(StoreError::from)
        })
        .collect()
}

fn commit_action(
    connection: &mut Connection,
    characters: &[DurableCharacterSnapshot],
    runtime: &WorldRuntimeSnapshot,
    events: &[DomainEvent],
) -> Result<(), StoreError> {
    commit_action_with_retention(
        connection,
        characters,
        runtime,
        events,
        DOMAIN_EVENT_RETENTION,
        None,
    )
}

fn commit_staff_action(
    connection: &mut Connection,
    characters: &[DurableCharacterSnapshot],
    runtime: &WorldRuntimeSnapshot,
    events: &[DomainEvent],
    audit: StaffAuditRecord,
    now: i64,
) -> Result<(), StoreError> {
    commit_action_with_retention(
        connection,
        characters,
        runtime,
        events,
        DOMAIN_EVENT_RETENTION,
        Some((audit, now)),
    )
}

fn commit_action_with_retention(
    connection: &mut Connection,
    characters: &[DurableCharacterSnapshot],
    runtime: &WorldRuntimeSnapshot,
    events: &[DomainEvent],
    event_retention: i64,
    staff_audit: Option<(StaffAuditRecord, i64)>,
) -> Result<(), StoreError> {
    debug_assert!(event_retention > 0);
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
    let staff_audit = staff_audit
        .map(|(audit, now)| prepare_staff_audit(audit).map(|audit| (audit, now)))
        .transpose()?;

    let transaction = connection.transaction()?;
    if let Some((audit, _)) = &staff_audit {
        validate_staff_authority(&transaction, audit, None, audit.target_account_id)?;
    }
    for ((snapshot, gameplay_json), adventure_json) in
        characters.iter().zip(gameplay_json).zip(adventure_json)
    {
        save(&transaction, &snapshot.character)?;
        let updated = transaction.execute(
            "UPDATE characters SET gameplay_json = ?1, updated_at = unixepoch()
             WHERE id = ?2 AND account_id = ?3",
            params![
                gameplay_json,
                snapshot.character.id.0,
                snapshot.character.account_id.0
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
             VALUES (?1, ?2, unixepoch())
             ON CONFLICT(character_id) DO UPDATE SET
                 state_json = excluded.state_json,
                 updated_at = excluded.updated_at",
            params![snapshot.character.id.0, adventure_json],
        )?;
    }
    transaction.execute(
        "INSERT INTO world_adventure (singleton, state_json, updated_at)
         VALUES (1, ?1, unixepoch())
         ON CONFLICT(singleton) DO UPDATE SET
             state_json = excluded.state_json,
             updated_at = excluded.updated_at",
        [world_adventure_json],
    )?;
    for event_json in event_json {
        transaction.execute(
            "INSERT INTO domain_events (event_json) VALUES (?1)",
            [event_json],
        )?;
    }
    let last_event_sequence = transaction.query_row(
        "SELECT max(
             COALESCE((SELECT MAX(sequence) FROM domain_events), 0),
             COALESCE((SELECT last_event_sequence FROM world_runtime
                       WHERE singleton = 1), 0)
         )",
        [],
        |row| row.get::<_, i64>(0),
    )?;
    transaction.execute(
        "INSERT INTO world_runtime
             (singleton, snapshot_version, snapshot_json,
              last_event_sequence, updated_at)
         VALUES (1, ?1, ?2, ?3, unixepoch())
         ON CONFLICT(singleton) DO UPDATE SET
             snapshot_version = excluded.snapshot_version,
             snapshot_json = excluded.snapshot_json,
             last_event_sequence = excluded.last_event_sequence,
             updated_at = excluded.updated_at",
        params![runtime.snapshot_version, runtime_json, last_event_sequence],
    )?;
    transaction.execute(
        "DELETE FROM domain_events
         WHERE sequence <= ?1 - ?2
           AND sequence <= ?1",
        params![last_event_sequence, event_retention],
    )?;
    if let Some((audit, now)) = &staff_audit {
        insert_staff_audit(&transaction, audit, *now)?;
    }
    transaction.commit()?;
    Ok(())
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
    use crate::auth::PasswordBackend;
    use std::{
        collections::BTreeMap,
        sync::{
            Condvar, Mutex,
            atomic::{AtomicBool, AtomicUsize, Ordering},
        },
        time::{SystemTime, UNIX_EPOCH},
    };
    use tiamud_core::{
        ActiveEffect, AreaDynamicCheckpointIdentity, BackgroundSelection, Direction, EffectKind,
        EquipmentSlot, ItemBinding, ItemInstanceId, ItemLocationSnapshot, PlayerAction,
        RuntimeItemInstanceSnapshotV2, RuntimeItemInstanceSnapshotV3, RuntimeItemLocation,
        RuntimeItemLocationV2, SessionId, World, WorldError, WorldRuntimeSnapshotV1,
        WorldRuntimeSnapshotV2, WorldRuntimeSnapshotV3,
    };

    async fn account(store: &SqliteCharacterStore, name: &str) -> AccountSnapshot {
        store.load_or_create_account(name).await.unwrap()
    }

    fn area_generation(
        world_id: &str,
        active: char,
        previous: Option<char>,
        activation_sequence: u64,
    ) -> DurableAreaGeneration {
        DurableAreaGeneration {
            world_id: world_id.into(),
            active_catalog_sha256: active.to_string().repeat(64),
            previous_catalog_sha256: previous.map(|digest| digest.to_string().repeat(64)),
            activation_sequence,
        }
    }

    fn area_checkpoint(runtime: &WorldRuntimeSnapshot) -> DurableAreaCheckpoint {
        let checkpoint = AreaDynamicCheckpoint {
            checkpoint_version: tiamud_core::AREA_DYNAMIC_CHECKPOINT_VERSION,
            generation_sha256: "a".repeat(64),
            areas: vec![AreaDynamicCheckpointIdentity {
                area_id: "demo:archive".into(),
                revision: 1,
                bundle_sha256: "d".repeat(64),
                residency_epoch: 2,
            }],
            captured_world_revision: runtime.revision,
            captured_logical_tick: runtime.logical_tick,
            captured_rng_state: runtime.rng_state,
            captured_next_item_instance_sequence: runtime.next_item_instance_sequence,
            captured_next_loot_drop_id: runtime.next_loot_drop_id,
            captured_next_reward_open_id: runtime.next_reward_open_id,
            npcs: BTreeMap::new(),
            item_instances: BTreeMap::new(),
            item_provenance: BTreeMap::new(),
            ground_loot: BTreeMap::new(),
        };
        tiamud_core::validate_area_dynamic_checkpoint(&checkpoint).unwrap();
        let checkpoint_json = serde_json::to_string(&checkpoint).unwrap();
        DurableAreaCheckpoint {
            generation_sha256: checkpoint.generation_sha256.clone(),
            unit_id: area_checkpoint_unit_id(&checkpoint),
            residency_epoch: checkpoint.areas[0].residency_epoch,
            checkpoint_version: checkpoint.checkpoint_version,
            checkpoint_sha256: hex::encode(Sha256::digest(checkpoint_json.as_bytes())),
            captured_logical_tick: checkpoint.captured_logical_tick,
            checkpoint_json,
        }
    }

    fn retired_runtime(
        mut runtime: WorldRuntimeSnapshot,
        durable: &DurableAreaCheckpoint,
    ) -> WorldRuntimeSnapshot {
        runtime.area_residency.insert(
            "demo:archive".into(),
            AreaRuntimeResidence::Retired {
                revision: 1,
                bundle_sha256: "d".repeat(64),
                residency_epoch: durable.residency_epoch,
                checkpoint_version: durable.checkpoint_version,
                checkpoint_sha256: durable.checkpoint_sha256.clone(),
            },
        );
        runtime
    }

    fn initial_resident_runtime(mut runtime: WorldRuntimeSnapshot) -> WorldRuntimeSnapshot {
        runtime.area_residency.insert(
            "demo:archive".into(),
            AreaRuntimeResidence::Resident {
                revision: 1,
                bundle_sha256: "d".repeat(64),
                residency_epoch: 1,
            },
        );
        runtime
    }

    fn rehydrated_runtime(mut runtime: WorldRuntimeSnapshot) -> WorldRuntimeSnapshot {
        runtime.area_residency.insert(
            "demo:archive".into(),
            AreaRuntimeResidence::Resident {
                revision: 1,
                bundle_sha256: "d".repeat(64),
                residency_epoch: 3,
            },
        );
        runtime
    }

    struct BlockingVerifyBackend {
        verify_started: AtomicUsize,
        released: AtomicBool,
        lock: Mutex<()>,
        wake: Condvar,
    }

    impl BlockingVerifyBackend {
        fn new() -> Self {
            Self {
                verify_started: AtomicUsize::new(0),
                released: AtomicBool::new(false),
                lock: Mutex::new(()),
                wake: Condvar::new(),
            }
        }

        fn release(&self) {
            self.released.store(true, Ordering::SeqCst);
            self.wake.notify_all();
        }
    }

    impl PasswordBackend for BlockingVerifyBackend {
        fn hash(&self, password: &str) -> Result<String, ()> {
            Ok(format!("test:{password}"))
        }

        fn verify(&self, password: &str, encoded_hash: &str) -> Result<bool, ()> {
            self.verify_started.fetch_add(1, Ordering::SeqCst);
            let mut guard = self.lock.lock().unwrap();
            while !self.released.load(Ordering::SeqCst) {
                guard = self.wake.wait(guard).unwrap();
            }
            Ok(encoded_hash == format!("test:{password}"))
        }
    }

    #[tokio::test]
    async fn shutdown_is_a_fifo_barrier_and_closes_the_owner_thread() {
        let store = SqliteCharacterStore::open(":memory:").await.unwrap();
        store
            .register_account("Shutdown Owner", "correct horse battery staple")
            .await
            .unwrap();
        store.shutdown().await.unwrap();
        assert!(matches!(
            store.health_check().await,
            Err(StoreError::WorkerStopped)
        ));
    }

    #[tokio::test]
    async fn registration_and_login_are_case_insensitive() {
        let store = SqliteCharacterStore::open(":memory:").await.unwrap();
        let registered = store
            .register_account("  Adventurer  ", "correct horse battery staple")
            .await
            .unwrap();
        let logged_in = store
            .login_account("ADVENTURER", "correct horse battery staple")
            .await
            .unwrap();

        assert_eq!(logged_in.account, registered.account);
        assert_ne!(logged_in.token, registered.token);
        assert_eq!(
            store
                .resolve_session(logged_in.token.as_str())
                .await
                .unwrap(),
            registered.account
        );
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn blocked_password_verification_does_not_block_database_health_or_commits() {
        let backend = Arc::new(BlockingVerifyBackend::new());
        let passwords = PasswordService::with_backend(1, backend.clone());
        let store = SqliteCharacterStore::open_with_password_service(":memory:", passwords)
            .await
            .unwrap();
        store
            .register_account("Adventurer", "correct horse battery staple")
            .await
            .unwrap();

        let login_store = store.clone();
        let login = tokio::spawn(async move {
            login_store
                .login_account("adventurer", "correct horse battery staple")
                .await
        });
        timeout(Duration::from_secs(1), async {
            while backend.verify_started.load(Ordering::SeqCst) == 0 {
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap();

        timeout(Duration::from_secs(1), store.health_check())
            .await
            .unwrap()
            .unwrap();
        timeout(
            Duration::from_secs(1),
            store.commit_action(Vec::new(), World::demo().runtime_snapshot(), Vec::new()),
        )
        .await
        .unwrap()
        .unwrap();

        backend.release();
        login.await.unwrap().unwrap();
    }

    #[test]
    fn login_finalization_revalidates_account_state_after_password_work() {
        let mut connection = Connection::open_in_memory().unwrap();
        initialise(&mut connection).unwrap();
        register_account(
            &mut connection,
            "Adventurer",
            "correct horse battery staple",
            1_000,
        )
        .unwrap();
        let prepared = prepare_login_account(&connection, "adventurer")
            .unwrap()
            .unwrap();
        connection
            .execute(
                "UPDATE accounts SET auth_state = 'guest' WHERE id = ?1",
                [prepared.account_id.0],
            )
            .unwrap();

        assert!(matches!(
            finish_login_account(&mut connection, &prepared, 1_001),
            Err(StoreError::InvalidCredentials)
        ));
    }

    #[tokio::test]
    async fn incorrect_password_is_rejected() {
        let store = SqliteCharacterStore::open(":memory:").await.unwrap();
        store
            .register_account("Adventurer", "correct horse battery staple")
            .await
            .unwrap();

        assert!(matches!(
            store
                .login_account("adventurer", "this password is incorrect")
                .await,
            Err(StoreError::InvalidCredentials)
        ));
        assert!(matches!(
            store.load_or_create_account("ADVENTURER").await,
            Err(StoreError::AccountAlreadyExists)
        ));
    }

    #[test]
    fn database_contains_only_argon2_password_and_sha256_token_material() {
        let mut connection = Connection::open_in_memory().unwrap();
        initialise(&mut connection).unwrap();
        let password = "correct horse battery staple";
        let session = register_account(&mut connection, "Secure", password, 1_000).unwrap();

        let encoded_hash: String = connection
            .query_row(
                "SELECT password_hash FROM accounts WHERE id = ?1",
                [session.account.id.0],
                |row| row.get(0),
            )
            .unwrap();
        let stored_token_hash: Vec<u8> = connection
            .query_row("SELECT token_hash FROM auth_sessions", [], |row| row.get(0))
            .unwrap();

        assert!(encoded_hash.starts_with("$argon2id$v=19$"));
        assert!(!encoded_hash.contains(password));
        assert_eq!(stored_token_hash.len(), 32);
        assert_ne!(stored_token_hash, session.token.as_str().as_bytes());
        assert_eq!(stored_token_hash, session.token.digest());
    }

    #[test]
    fn expired_and_revoked_tokens_are_rejected() {
        let mut connection = Connection::open_in_memory().unwrap();
        initialise(&mut connection).unwrap();
        let expired = register_account(
            &mut connection,
            "Expiring",
            "correct horse battery staple",
            1_000,
        )
        .unwrap();

        assert!(matches!(
            resolve_session(&connection, expired.token.as_str(), expired.expires_at),
            Err(StoreError::SessionExpired)
        ));

        let active = login_account(
            &mut connection,
            "expiring",
            "correct horse battery staple",
            2_000,
        )
        .unwrap();
        revoke_session(&connection, active.token.as_str(), 2_001).unwrap();
        assert!(matches!(
            resolve_session(&connection, active.token.as_str(), 2_002),
            Err(StoreError::SessionRevoked)
        ));
    }

    #[test]
    fn authentication_inputs_and_active_sessions_are_bounded() {
        let mut connection = Connection::open_in_memory().unwrap();
        initialise(&mut connection).unwrap();
        assert!(matches!(
            register_account(&mut connection, "Short", "tiny", 1_000),
            Err(StoreError::InvalidPassword)
        ));

        let first = register_account(
            &mut connection,
            "Bounded",
            "correct horse battery staple",
            2_000,
        )
        .unwrap();
        for offset in 1..MAX_ACTIVE_SESSIONS + 2 {
            issue_session(&connection, first.account.clone(), 2_000 + offset).unwrap();
        }
        let count: i64 = connection
            .query_row(
                "SELECT COUNT(*) FROM auth_sessions WHERE account_id = ?1",
                [first.account.id.0],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(count, MAX_ACTIVE_SESSIONS);
        assert!(matches!(
            resolve_session(&connection, first.token.as_str(), 2_100),
            Err(StoreError::InvalidSession)
        ));
    }

    #[test]
    fn schema_v8_adventure_mirrors_are_written_in_the_same_transaction() {
        let mut connection = Connection::open_in_memory().unwrap();
        initialise(&mut connection).unwrap();
        let owner = load_or_create_account(&connection, "Adventure Owner").unwrap();
        let character =
            load_or_create_character(&connection, owner.id, "Market Hero", "demo:lantern-square")
                .unwrap();
        let gameplay = CharacterGameplaySnapshot {
            coins: 28,
            commodities: std::collections::BTreeMap::from([("tonic".into(), 1)]),
            ..CharacterGameplaySnapshot::default()
        };
        let mut runtime = World::demo().runtime_snapshot();
        runtime.adventure.shop_stock.insert("tonic".into(), 19);

        commit_action(
            &mut connection,
            &[DurableCharacterSnapshot {
                character: character.clone(),
                gameplay,
            }],
            &runtime,
            &[],
        )
        .unwrap();

        let character_json: String = connection
            .query_row(
                "SELECT state_json FROM character_adventure WHERE character_id = ?1",
                [character.id.0],
                |row| row.get(0),
            )
            .unwrap();
        let world_json: String = connection
            .query_row(
                "SELECT state_json FROM world_adventure WHERE singleton = 1",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert!(character_json.contains("\"coins\":28"));
        assert!(character_json.contains("\"tonic\":1"));
        assert!(world_json.contains("\"tonic\":19"));
    }

    fn test_domain_event() -> DomainEvent {
        DomainEvent::CharacterMoved {
            actor_id: CharacterId(1),
            actor_name: "Aria".into(),
            from_room_id: "room:a".into(),
            to_room_id: "room:b".into(),
            direction: Direction::North,
        }
    }

    #[test]
    fn startup_world_recovery_distinguishes_empty_and_ledger_authority() {
        let mut connection = Connection::open_in_memory().unwrap();
        initialise(&mut connection).unwrap();

        let empty = load_startup_world_recovery(&mut connection).unwrap();
        assert_eq!(empty, StartupWorldRecovery::Empty);
        assert!(!empty.requires_full_generation());

        let event = test_domain_event();
        connection
            .execute(
                "INSERT INTO domain_events (event_json) VALUES (?1)",
                [serde_json::to_string(&event).unwrap()],
            )
            .unwrap();
        let ledger = load_startup_world_recovery(&mut connection).unwrap();
        assert_eq!(ledger, StartupWorldRecovery::Ledger(vec![event]));
        assert!(ledger.requires_full_generation());
    }

    #[test]
    fn startup_world_recovery_returns_a_valid_snapshot() {
        let mut connection = Connection::open_in_memory().unwrap();
        initialise(&mut connection).unwrap();
        let snapshot = World::demo().runtime_snapshot();
        connection
            .execute(
                "INSERT INTO world_runtime
                     (singleton, snapshot_version, snapshot_json,
                      last_event_sequence, updated_at)
                 VALUES (1, ?1, ?2, 0, unixepoch())",
                params![
                    snapshot.snapshot_version,
                    serde_json::to_string(&snapshot).unwrap()
                ],
            )
            .unwrap();

        let recovery = load_startup_world_recovery(&mut connection).unwrap();
        assert_eq!(
            recovery,
            StartupWorldRecovery::Snapshot {
                durable_version: tiamud_core::WORLD_RUNTIME_SNAPSHOT_VERSION,
                snapshot,
            }
        );
        assert!(recovery.requires_full_generation());
    }

    #[test]
    fn startup_world_recovery_preserves_legacy_durable_snapshot_version() {
        let mut connection = Connection::open_in_memory().unwrap();
        initialise(&mut connection).unwrap();
        let mut legacy = serde_json::to_value(World::demo().runtime_snapshot()).unwrap();
        let object = legacy.as_object_mut().unwrap();
        object.insert("snapshotVersion".into(), serde_json::json!(7));
        object.remove("areaResidency");
        connection
            .execute(
                "INSERT INTO world_runtime
                     (singleton, snapshot_version, snapshot_json,
                      last_event_sequence, updated_at)
                 VALUES (1, 7, ?1, 0, unixepoch())",
                [serde_json::to_string(&legacy).unwrap()],
            )
            .unwrap();

        let StartupWorldRecovery::Snapshot {
            durable_version,
            snapshot,
        } = load_startup_world_recovery(&mut connection).unwrap()
        else {
            panic!("expected durable snapshot authority");
        };
        assert_eq!(durable_version, 7);
        assert!(snapshot.area_residency.is_empty());
        assert_eq!(
            load_runtime_snapshot(&connection).unwrap(),
            Some(snapshot),
            "the public loader remains normalized"
        );
    }

    #[test]
    fn startup_world_recovery_gives_snapshot_precedence_over_the_ledger() {
        let mut connection = Connection::open_in_memory().unwrap();
        initialise(&mut connection).unwrap();
        let snapshot = World::demo().runtime_snapshot();
        connection
            .execute(
                "INSERT INTO world_runtime
                     (singleton, snapshot_version, snapshot_json,
                      last_event_sequence, updated_at)
                 VALUES (1, ?1, ?2, 0, unixepoch())",
                params![
                    snapshot.snapshot_version,
                    serde_json::to_string(&snapshot).unwrap()
                ],
            )
            .unwrap();
        // A retained diagnostic suffix is not consulted when a valid runtime
        // snapshot exists. Make it deliberately undecodable to prove there is
        // no hidden ledger fallback or eager parse.
        connection
            .execute(
                "INSERT INTO domain_events (event_json) VALUES ('not-json')",
                [],
            )
            .unwrap();

        assert_eq!(
            load_startup_world_recovery(&mut connection).unwrap(),
            StartupWorldRecovery::Snapshot {
                durable_version: tiamud_core::WORLD_RUNTIME_SNAPSHOT_VERSION,
                snapshot,
            }
        );
    }

    #[test]
    fn startup_world_recovery_fails_closed_on_an_invalid_snapshot() {
        let mut connection = Connection::open_in_memory().unwrap();
        initialise(&mut connection).unwrap();
        connection
            .execute(
                "INSERT INTO world_runtime
                     (singleton, snapshot_version, snapshot_json,
                      last_event_sequence, updated_at)
                 VALUES (1, ?1, '{}', 0, unixepoch())",
                [tiamud_core::WORLD_RUNTIME_SNAPSHOT_VERSION],
            )
            .unwrap();
        connection
            .execute(
                "INSERT INTO domain_events (event_json) VALUES (?1)",
                [serde_json::to_string(&test_domain_event()).unwrap()],
            )
            .unwrap();

        assert!(matches!(
            load_startup_world_recovery(&mut connection),
            Err(StoreError::InvalidRuntimeSnapshot)
        ));
    }

    #[test]
    fn startup_world_recovery_rejects_a_ledger_gap() {
        let mut connection = Connection::open_in_memory().unwrap();
        initialise(&mut connection).unwrap();
        let event_json = serde_json::to_string(&test_domain_event()).unwrap();
        connection
            .execute(
                "INSERT INTO domain_events (sequence, event_json)
                 VALUES (1, ?1), (3, ?1)",
                [event_json],
            )
            .unwrap();

        assert!(matches!(
            load_startup_world_recovery(&mut connection),
            Err(StoreError::InvalidStartupLedger)
        ));
    }

    #[test]
    fn startup_world_recovery_rejects_more_than_the_retained_event_limit() {
        let mut connection = Connection::open_in_memory().unwrap();
        initialise(&mut connection).unwrap();
        seed_domain_events(&connection, DOMAIN_EVENT_RETENTION + 1);

        assert!(matches!(
            load_startup_world_recovery(&mut connection),
            Err(StoreError::InvalidStartupLedger)
        ));
    }

    #[test]
    fn startup_world_recovery_checks_event_utf8_bytes_before_deserializing() {
        let mut connection = Connection::open_in_memory().unwrap();
        initialise(&mut connection).unwrap();
        let oversized = "é".repeat(MAX_EVENT_JSON_BYTES / 2 + 1);
        assert!(oversized.len() > MAX_EVENT_JSON_BYTES);
        connection
            .execute(
                "INSERT INTO domain_events (event_json) VALUES (?1)",
                [oversized],
            )
            .unwrap();

        assert!(matches!(
            load_startup_world_recovery(&mut connection),
            Err(StoreError::SnapshotTooLarge {
                kind: "domain event",
                maximum: MAX_EVENT_JSON_BYTES,
            })
        ));
    }

    fn seed_domain_events(connection: &Connection, count: i64) {
        connection
            .execute(
                "WITH RECURSIVE counter(value) AS (
                     VALUES(1)
                     UNION ALL
                     SELECT value + 1 FROM counter WHERE value < ?1
                 )
                 INSERT INTO domain_events (event_json)
                 SELECT '{\"type\":\"retained-test\"}' FROM counter",
                [count],
            )
            .unwrap();
    }

    #[test]
    fn runtime_watermark_tracks_events_and_survives_an_empty_commit() {
        let mut connection = Connection::open_in_memory().unwrap();
        initialise(&mut connection).unwrap();
        let mut runtime = World::demo().runtime_snapshot();

        commit_action(&mut connection, &[], &runtime, &[test_domain_event()]).unwrap();
        assert_eq!(
            connection
                .query_row(
                    "SELECT last_event_sequence FROM world_runtime WHERE singleton = 1",
                    [],
                    |row| row.get::<_, i64>(0),
                )
                .unwrap(),
            1
        );

        runtime.logical_tick = 1;
        commit_action(&mut connection, &[], &runtime, &[]).unwrap();
        assert_eq!(
            connection
                .query_row(
                    "SELECT last_event_sequence FROM world_runtime WHERE singleton = 1",
                    [],
                    |row| row.get::<_, i64>(0),
                )
                .unwrap(),
            1
        );
        assert_eq!(
            connection
                .query_row("SELECT COUNT(*) FROM domain_events", [], |row| {
                    row.get::<_, i64>(0)
                })
                .unwrap(),
            1
        );
    }

    #[test]
    fn domain_event_compaction_retains_the_explicit_bounded_suffix() {
        let mut connection = Connection::open_in_memory().unwrap();
        initialise(&mut connection).unwrap();
        seed_domain_events(&connection, DOMAIN_EVENT_RETENTION);

        commit_action(
            &mut connection,
            &[],
            &World::demo().runtime_snapshot(),
            &[test_domain_event()],
        )
        .unwrap();

        let bounds = connection
            .query_row(
                "SELECT COUNT(*), MIN(sequence), MAX(sequence) FROM domain_events",
                [],
                |row| {
                    Ok((
                        row.get::<_, i64>(0)?,
                        row.get::<_, i64>(1)?,
                        row.get::<_, i64>(2)?,
                    ))
                },
            )
            .unwrap();
        assert_eq!(
            bounds,
            (DOMAIN_EVENT_RETENTION, 2, DOMAIN_EVENT_RETENTION + 1)
        );
        assert_eq!(
            connection
                .query_row(
                    "SELECT last_event_sequence FROM world_runtime WHERE singleton = 1",
                    [],
                    |row| row.get::<_, i64>(0),
                )
                .unwrap(),
            DOMAIN_EVENT_RETENTION + 1
        );
    }

    #[test]
    fn domain_event_compaction_stays_bounded_across_many_commits() {
        const TEST_RETENTION: i64 = 10;
        let mut connection = Connection::open_in_memory().unwrap();
        initialise(&mut connection).unwrap();
        let mut runtime = World::demo().runtime_snapshot();

        for sequence in 1..=25 {
            runtime.logical_tick = sequence;
            commit_action_with_retention(
                &mut connection,
                &[],
                &runtime,
                &[test_domain_event()],
                TEST_RETENTION,
                None,
            )
            .unwrap();
        }

        let state = connection
            .query_row(
                "SELECT
                     (SELECT COUNT(*) FROM domain_events),
                     (SELECT MIN(sequence) FROM domain_events),
                     (SELECT MAX(sequence) FROM domain_events),
                     last_event_sequence
                 FROM world_runtime WHERE singleton = 1",
                [],
                |row| {
                    Ok((
                        row.get::<_, i64>(0)?,
                        row.get::<_, i64>(1)?,
                        row.get::<_, i64>(2)?,
                        row.get::<_, i64>(3)?,
                    ))
                },
            )
            .unwrap();
        assert_eq!(state, (TEST_RETENTION, 16, 25, 25));
    }

    #[test]
    fn failed_compaction_rolls_back_event_snapshot_watermark_and_deletion() {
        let mut connection = Connection::open_in_memory().unwrap();
        initialise(&mut connection).unwrap();
        seed_domain_events(&connection, DOMAIN_EVENT_RETENTION);
        let original_runtime = World::demo().runtime_snapshot();
        let original_json = serde_json::to_string(&original_runtime).unwrap();
        connection
            .execute(
                "INSERT INTO world_runtime
                     (singleton, snapshot_version, snapshot_json,
                      last_event_sequence, updated_at)
                 VALUES (1, ?1, ?2, ?3, unixepoch())",
                params![
                    original_runtime.snapshot_version,
                    original_json,
                    DOMAIN_EVENT_RETENTION
                ],
            )
            .unwrap();
        connection
            .execute_batch(
                "CREATE TRIGGER reject_domain_event_compaction
                 BEFORE DELETE ON domain_events
                 BEGIN
                     SELECT RAISE(ABORT, 'injected compaction failure');
                 END;",
            )
            .unwrap();
        let mut replacement = original_runtime;
        replacement.logical_tick = 99;

        assert!(commit_action(&mut connection, &[], &replacement, &[test_domain_event()]).is_err());
        let state = connection
            .query_row(
                "SELECT
                     (SELECT COUNT(*) FROM domain_events),
                     (SELECT MAX(sequence) FROM domain_events),
                     last_event_sequence,
                     snapshot_json
                 FROM world_runtime WHERE singleton = 1",
                [],
                |row| {
                    Ok((
                        row.get::<_, i64>(0)?,
                        row.get::<_, i64>(1)?,
                        row.get::<_, i64>(2)?,
                        row.get::<_, String>(3)?,
                    ))
                },
            )
            .unwrap();
        assert_eq!(
            state,
            (
                DOMAIN_EVENT_RETENTION,
                DOMAIN_EVENT_RETENTION,
                DOMAIN_EVENT_RETENTION,
                original_json
            )
        );
    }

    #[tokio::test]
    async fn sqlite_area_generation_bind_and_activation_are_restart_safe_and_cas_guarded() {
        let temporary = tempfile::TempDir::new().unwrap();
        let database_path = temporary.path().join("area-generation.sqlite3");
        let store = SqliteCharacterStore::open(&database_path).await.unwrap();
        assert_eq!(store.load_area_generation().await.unwrap(), None);

        let initial = area_generation("tiamud:lantern", 'a', None, 1);
        assert_eq!(
            store
                .bind_area_generation_if_absent(initial.clone())
                .await
                .unwrap(),
            initial
        );
        assert_eq!(
            store
                .bind_area_generation_if_absent(area_generation("tiamud:lantern", 'b', None, 1,))
                .await
                .unwrap(),
            initial
        );

        let mut activated_runtime = World::demo().runtime_snapshot();
        activated_runtime.logical_tick = 41;
        let activated = area_generation("tiamud:lantern", 'b', Some('a'), 2);
        store
            .commit_area_generation_activation(
                activated_runtime.clone(),
                initial.clone(),
                activated.clone(),
            )
            .await
            .unwrap();
        assert_eq!(
            store.load_area_generation().await.unwrap(),
            Some(activated.clone())
        );
        assert_eq!(
            store.load_startup_world_recovery().await.unwrap(),
            StartupWorldRecovery::Snapshot {
                durable_version: tiamud_core::WORLD_RUNTIME_SNAPSHOT_VERSION,
                snapshot: activated_runtime.clone(),
            }
        );

        let mut stale_runtime = activated_runtime.clone();
        stale_runtime.logical_tick = 99;
        assert!(matches!(
            store
                .commit_area_generation_activation(
                    stale_runtime,
                    initial.clone(),
                    area_generation("tiamud:lantern", 'c', Some('a'), 2),
                )
                .await,
            Err(StoreError::AreaGenerationConflict)
        ));
        assert_eq!(
            store.load_area_generation().await.unwrap(),
            Some(activated.clone())
        );
        assert_eq!(
            store.load_startup_world_recovery().await.unwrap(),
            StartupWorldRecovery::Snapshot {
                durable_version: tiamud_core::WORLD_RUNTIME_SNAPSHOT_VERSION,
                snapshot: activated_runtime,
            }
        );
        store.shutdown().await.unwrap();

        let reopened = SqliteCharacterStore::open(&database_path).await.unwrap();
        assert_eq!(
            reopened.load_area_generation().await.unwrap(),
            Some(activated)
        );
    }

    #[tokio::test]
    async fn sqlite_area_checkpoint_retirement_and_rehydration_are_exact_and_restart_safe() {
        let temporary = tempfile::TempDir::new().unwrap();
        let database_path = temporary.path().join("area-checkpoint.sqlite3");
        let store = SqliteCharacterStore::open(&database_path).await.unwrap();
        let generation = area_generation("tiamud:lantern", 'a', None, 1);
        store
            .bind_area_generation_if_absent(generation.clone())
            .await
            .unwrap();

        let baseline = initial_resident_runtime(World::demo().runtime_snapshot());
        let stale_checkpoint = area_checkpoint(&baseline);
        let stale_retired = retired_runtime(baseline.clone(), &stale_checkpoint);
        let mut current = baseline;
        current.logical_tick += 1;
        store
            .commit_action(Vec::new(), current.clone(), Vec::new())
            .await
            .unwrap();
        assert!(matches!(
            store
                .commit_area_retirement(stale_retired, generation.clone(), stale_checkpoint,)
                .await,
            Err(StoreError::AreaCheckpointConflict)
        ));
        let checkpoint = area_checkpoint(&current);
        let retired = retired_runtime(current, &checkpoint);
        let mut forged_unit = checkpoint.clone();
        forged_unit.unit_id = "c".repeat(64);
        assert!(matches!(
            store
                .commit_area_retirement(retired.clone(), generation.clone(), forged_unit,)
                .await,
            Err(StoreError::InvalidAreaCheckpoint)
        ));
        assert!(store.load_area_checkpoints().await.unwrap().is_empty());
        store
            .commit_area_retirement(retired.clone(), generation.clone(), checkpoint.clone())
            .await
            .unwrap();
        assert_eq!(
            store.load_area_checkpoints().await.unwrap(),
            vec![checkpoint.clone()]
        );
        assert_eq!(
            store.load_runtime_snapshot().await.unwrap(),
            Some(retired.clone())
        );
        assert!(matches!(
            store
                .commit_area_retirement(retired.clone(), generation.clone(), checkpoint.clone())
                .await,
            Err(StoreError::AreaCheckpointConflict)
        ));

        store.shutdown().await.unwrap();
        let reopened = SqliteCharacterStore::open(&database_path).await.unwrap();
        assert_eq!(
            reopened.load_area_checkpoints().await.unwrap(),
            vec![checkpoint.clone()]
        );
        let stale_resident = rehydrated_runtime(retired.clone());
        let mut current_retired = retired;
        current_retired.logical_tick += 1;
        reopened
            .commit_action(Vec::new(), current_retired.clone(), Vec::new())
            .await
            .unwrap();
        assert!(matches!(
            reopened
                .commit_area_rehydration(stale_resident, generation.clone(), checkpoint.clone(),)
                .await,
            Err(StoreError::AreaCheckpointConflict)
        ));
        assert_eq!(
            reopened.load_area_checkpoints().await.unwrap(),
            vec![checkpoint.clone()]
        );
        let resident = rehydrated_runtime(current_retired);
        reopened
            .commit_area_rehydration(resident.clone(), generation.clone(), checkpoint.clone())
            .await
            .unwrap();
        assert!(reopened.load_area_checkpoints().await.unwrap().is_empty());
        assert_eq!(
            reopened.load_runtime_snapshot().await.unwrap(),
            Some(resident.clone())
        );
        assert!(matches!(
            reopened
                .commit_area_rehydration(resident, generation, checkpoint)
                .await,
            Err(StoreError::AreaCheckpointConflict)
        ));
    }

    #[test]
    fn sqlite_area_checkpoint_snapshot_failures_roll_back_insert_and_delete() {
        let mut connection = Connection::open_in_memory().unwrap();
        initialise(&mut connection).unwrap();
        let generation = area_generation("tiamud:lantern", 'a', None, 1);
        bind_area_generation_if_absent(&mut connection, &generation).unwrap();
        let baseline = initial_resident_runtime(World::demo().runtime_snapshot());
        commit_action(&mut connection, &[], &baseline, &[]).unwrap();
        let checkpoint = area_checkpoint(&baseline);
        let retired = retired_runtime(baseline.clone(), &checkpoint);

        connection
            .execute_batch(
                "CREATE TRIGGER reject_retirement_snapshot
                 BEFORE UPDATE ON world_runtime
                 BEGIN
                     SELECT RAISE(ABORT, 'injected retirement snapshot failure');
                 END;",
            )
            .unwrap();
        assert!(
            commit_area_retirement(&mut connection, &retired, &generation, &checkpoint).is_err()
        );
        assert!(load_area_checkpoints(&connection).unwrap().is_empty());
        assert_eq!(load_runtime_snapshot(&connection).unwrap(), Some(baseline));

        connection
            .execute_batch("DROP TRIGGER reject_retirement_snapshot")
            .unwrap();
        commit_area_retirement(&mut connection, &retired, &generation, &checkpoint).unwrap();
        let resident = rehydrated_runtime(retired.clone());
        connection
            .execute_batch(
                "CREATE TRIGGER reject_rehydration_snapshot
                 BEFORE UPDATE ON world_runtime
                 BEGIN
                     SELECT RAISE(ABORT, 'injected rehydration snapshot failure');
                 END;",
            )
            .unwrap();
        assert!(
            commit_area_rehydration(&mut connection, &resident, &generation, &checkpoint).is_err()
        );
        assert_eq!(
            load_area_checkpoints(&connection).unwrap(),
            vec![checkpoint]
        );
        assert_eq!(load_runtime_snapshot(&connection).unwrap(), Some(retired));
    }

    #[test]
    fn sqlite_area_generation_activation_rolls_back_pointer_with_snapshot_failure() {
        let mut connection = Connection::open_in_memory().unwrap();
        initialise(&mut connection).unwrap();
        let initial = area_generation("Mundo de Ámbar", 'a', None, 1);
        assert_eq!(
            bind_area_generation_if_absent(&mut connection, &initial).unwrap(),
            initial
        );
        let baseline = World::demo().runtime_snapshot();
        commit_action(&mut connection, &[], &baseline, &[]).unwrap();
        connection
            .execute_batch(
                "CREATE TRIGGER reject_generation_snapshot
                 BEFORE UPDATE ON world_runtime
                 BEGIN
                     SELECT RAISE(ABORT, 'injected snapshot failure');
                 END;",
            )
            .unwrap();
        let mut replacement = baseline.clone();
        replacement.logical_tick = 73;

        assert!(
            commit_area_generation_activation(
                &mut connection,
                &replacement,
                &initial,
                &area_generation("Mundo de Ámbar", 'b', Some('a'), 2),
            )
            .is_err()
        );
        assert_eq!(load_area_generation(&connection).unwrap(), Some(initial));
        assert_eq!(load_runtime_snapshot(&connection).unwrap(), Some(baseline));
    }

    #[test]
    fn area_generation_validation_matches_catalog_identity_and_closed_digest_rules() {
        assert!(validate_area_generation(&area_generation("Mundo de Ámbar", 'a', None, 1)).is_ok());
        for invalid in [
            DurableAreaGeneration {
                world_id: " surrounding space ".into(),
                active_catalog_sha256: "a".repeat(64),
                previous_catalog_sha256: None,
                activation_sequence: 1,
            },
            DurableAreaGeneration {
                world_id: "demo:\u{202e}world".into(),
                active_catalog_sha256: "a".repeat(64),
                previous_catalog_sha256: None,
                activation_sequence: 1,
            },
            DurableAreaGeneration {
                world_id: "demo:world".into(),
                active_catalog_sha256: "A".repeat(64),
                previous_catalog_sha256: None,
                activation_sequence: 1,
            },
            area_generation("demo:world", 'a', Some('a'), 2),
            area_generation("demo:world", 'a', None, 0),
        ] {
            assert!(matches!(
                validate_area_generation(&invalid),
                Err(StoreError::InvalidAreaGeneration)
            ));
        }
    }

    #[tokio::test]
    async fn account_ids_are_stable_and_names_are_case_insensitive() {
        let store = SqliteCharacterStore::open(":memory:").await.unwrap();
        let created = account(&store, "  Aria  ").await;
        let loaded = account(&store, "aria").await;

        assert_eq!(loaded.id, created.id);
        assert_eq!(loaded.name, "Aria");
    }

    #[tokio::test]
    async fn character_authority_is_visible_without_a_runtime_snapshot_or_ledger() {
        let store = SqliteCharacterStore::open(":memory:").await.unwrap();
        assert!(!store.has_durable_character_authority().await.unwrap());
        let owner = account(&store, "Legacy Owner").await;
        assert!(!store.has_durable_character_authority().await.unwrap());
        store
            .load_or_create_character(owner.id, "Legacy Hero", "demo:lantern-square")
            .await
            .unwrap();

        assert!(store.has_durable_character_authority().await.unwrap());
        assert_eq!(
            store.load_startup_world_recovery().await.unwrap(),
            StartupWorldRecovery::Empty
        );
    }

    #[tokio::test]
    async fn character_ids_survive_saves_and_case_insensitive_loads() {
        let store = SqliteCharacterStore::open(":memory:").await.unwrap();
        let owner = account(&store, "Owner").await;
        let created = store
            .load_or_create_character(owner.id, "Aria", "demo:lantern-square")
            .await
            .unwrap();

        let loaded_by_name = store
            .load_or_create_character(owner.id, "aria", "ignored")
            .await
            .unwrap();
        assert_eq!(loaded_by_name, created);

        let mut moved = created.clone();
        moved.room_id = "demo:north-gate".into();
        moved
            .item_locations
            .insert("demo:lantern".into(), ItemLocationSnapshot::Carried);
        store.save(moved.clone()).await.unwrap();
        assert_eq!(
            store.load_character(owner.id, created.id).await.unwrap(),
            moved
        );
    }

    #[tokio::test]
    async fn selected_initial_gameplay_is_persisted_once_without_overwriting_existing_characters() {
        let store = SqliteCharacterStore::open(":memory:").await.unwrap();
        let owner = account(&store, "Sheet Owner").await;
        let mut selected = CharacterGameplaySnapshot::default();
        selected.attributes.strength = 11;
        selected.vitals.health = 27;
        selected.vitals.max_health = 27;

        let character = store
            .load_or_create_character_with_gameplay(
                owner.id,
                "Aria",
                "demo:lantern-square",
                selected.clone(),
            )
            .await
            .unwrap();
        assert_eq!(
            store.load_character_gameplay(character.id).await.unwrap(),
            selected
        );

        let mut replacement = CharacterGameplaySnapshot::default();
        replacement.attributes.strength = 99;
        let existing = store
            .load_or_create_character_with_gameplay(owner.id, "aria", "ignored", replacement)
            .await
            .unwrap();
        assert_eq!(existing, character);
        assert_eq!(
            store
                .load_character_gameplay(character.id)
                .await
                .unwrap()
                .attributes
                .strength,
            11
        );
    }

    #[tokio::test]
    async fn character_and_shared_world_events_commit_atomically() {
        let store = SqliteCharacterStore::open(":memory:").await.unwrap();
        let owner = account(&store, "Event Owner").await;
        let mut character = store
            .load_or_create_character(owner.id, "Aria", "room:a")
            .await
            .unwrap();
        character.room_id = "room:b".into();
        let event = DomainEvent::CharacterMoved {
            actor_id: character.id,
            actor_name: character.name.clone(),
            from_room_id: "room:a".into(),
            to_room_id: "room:b".into(),
            direction: Direction::North,
        };

        store
            .commit_action(
                vec![DurableCharacterSnapshot {
                    character: character.clone(),
                    gameplay: CharacterGameplaySnapshot {
                        coins: 28,
                        commodities: std::collections::BTreeMap::from([("tonic".into(), 1)]),
                        ..CharacterGameplaySnapshot::default()
                    },
                }],
                {
                    let mut runtime = World::demo().runtime_snapshot();
                    runtime.adventure.shop_stock.insert("tonic".into(), 19);
                    runtime
                },
                vec![event.clone()],
            )
            .await
            .unwrap();
        assert_eq!(
            store.load_character(owner.id, character.id).await.unwrap(),
            character
        );
        assert_eq!(store.load_domain_events().await.unwrap(), vec![event]);
        assert_eq!(
            store
                .load_character_gameplay(character.id)
                .await
                .unwrap()
                .coins,
            28
        );

        let mut missing = character;
        missing.id = CharacterId(9999);
        let rejected = DomainEvent::ItemTaken {
            actor_id: missing.id,
            actor_name: missing.name.clone(),
            item_id: "item:missing".into(),
            item_name: "missing".into(),
            room_id: "room:b".into(),
        };
        assert!(
            store
                .commit_action(
                    vec![DurableCharacterSnapshot {
                        character: missing,
                        gameplay: CharacterGameplaySnapshot::default(),
                    }],
                    {
                        let mut runtime = World::demo().runtime_snapshot();
                        runtime.logical_tick = 99;
                        runtime.adventure.shop_stock.insert("tonic".into(), 0);
                        runtime
                    },
                    vec![rejected],
                )
                .await
                .is_err()
        );
        assert_eq!(store.load_domain_events().await.unwrap().len(), 1);
        assert_eq!(
            store
                .load_runtime_snapshot()
                .await
                .unwrap()
                .unwrap()
                .logical_tick,
            0
        );
        assert_eq!(
            store
                .load_runtime_snapshot()
                .await
                .unwrap()
                .unwrap()
                .adventure
                .shop_stock["tonic"],
            19
        );
    }

    #[test]
    fn sqlite_runtime_loader_upgrades_v1_items_without_losing_identity_or_location() {
        let mut connection = Connection::open_in_memory().unwrap();
        migrate(&mut connection).unwrap();
        let legacy = WorldRuntimeSnapshotV1 {
            snapshot_version: 1,
            revision: 7,
            logical_tick: 3,
            rng_state: 42,
            item_locations: BTreeMap::from([(
                "legacy:lamp".into(),
                RuntimeItemLocationV2::Room {
                    room_id: "demo:lantern-square".into(),
                },
            )]),
            npcs: BTreeMap::new(),
            adventure: tiamud_core::AdventureRuntimeSnapshot::default(),
            progression_flags: Default::default(),
        };
        connection
            .execute(
                "INSERT INTO world_runtime
                     (singleton, snapshot_version, snapshot_json, updated_at)
                 VALUES (1, 1, ?1, unixepoch())",
                [serde_json::to_string(&legacy).unwrap()],
            )
            .unwrap();

        let upgraded = load_runtime_snapshot(&connection).unwrap().unwrap();
        assert_eq!(
            upgraded.snapshot_version,
            tiamud_core::WORLD_RUNTIME_SNAPSHOT_VERSION
        );
        let item = upgraded
            .item_instances
            .get(&ItemInstanceId("legacy:lamp".into()))
            .unwrap();
        assert_eq!(item.prototype_id, "legacy:lamp");
        assert_eq!(
            item.location,
            RuntimeItemLocation::Room {
                room_id: "demo:lantern-square".into()
            }
        );
        assert_eq!(item.binding, ItemBinding::Unbound);
    }

    #[test]
    fn sqlite_runtime_loader_upgrades_v2_before_validating_current_version() {
        let mut connection = Connection::open_in_memory().unwrap();
        migrate(&mut connection).unwrap();
        let legacy = WorldRuntimeSnapshotV2 {
            snapshot_version: 2,
            revision: 7,
            logical_tick: 3,
            rng_state: 42,
            next_item_instance_sequence: 2,
            item_instances: BTreeMap::from([(
                ItemInstanceId("runtime:1".into()),
                RuntimeItemInstanceSnapshotV2 {
                    prototype_id: "legacy:lamp".into(),
                    location: RuntimeItemLocationV2::Room {
                        room_id: "demo:lantern-square".into(),
                    },
                },
            )]),
            npcs: BTreeMap::new(),
            adventure: tiamud_core::AdventureRuntimeSnapshot::default(),
            progression_flags: Default::default(),
        };
        connection
            .execute(
                "INSERT INTO world_runtime
                     (singleton, snapshot_version, snapshot_json, updated_at)
                 VALUES (1, 2, ?1, unixepoch())",
                [serde_json::to_string(&legacy).unwrap()],
            )
            .unwrap();

        let upgraded = load_runtime_snapshot(&connection).unwrap().unwrap();
        assert_eq!(
            upgraded.snapshot_version,
            tiamud_core::WORLD_RUNTIME_SNAPSHOT_VERSION
        );
        assert_eq!(
            upgraded.item_instances[&ItemInstanceId("runtime:1".into())].location,
            RuntimeItemLocation::Room {
                room_id: "demo:lantern-square".into(),
            }
        );
        assert_eq!(
            upgraded.item_instances[&ItemInstanceId("runtime:1".into())].binding,
            ItemBinding::Unbound
        );
    }

    #[test]
    fn legacy_runtime_upgrade_rejects_a_current_snapshot_above_the_current_budget() {
        let build = |count: usize| WorldRuntimeSnapshotV2 {
            snapshot_version: 2,
            revision: 1,
            logical_tick: 0,
            rng_state: 1,
            next_item_instance_sequence: 1,
            item_instances: (0..count)
                .map(|index| {
                    (
                        ItemInstanceId(format!("legacy:{index}")),
                        RuntimeItemInstanceSnapshotV2 {
                            prototype_id: "p".into(),
                            location: RuntimeItemLocationV2::Staged,
                        },
                    )
                })
                .collect(),
            npcs: BTreeMap::new(),
            adventure: tiamud_core::AdventureRuntimeSnapshot::default(),
            progression_flags: BTreeSet::new(),
        };

        let mut accepted = 0_usize;
        let mut rejected = MAX_ITEM_INSTANCES + 1;
        while accepted + 1 < rejected {
            let candidate = accepted + (rejected - accepted) / 2;
            let bytes = serde_json::to_vec(&build(candidate)).unwrap().len();
            if bytes <= tiamud_core::MAX_LEGACY_WORLD_RUNTIME_SNAPSHOT_BYTES {
                accepted = candidate;
            } else {
                rejected = candidate;
            }
        }
        let legacy_json = serde_json::to_string(&build(accepted)).unwrap();
        assert!(
            legacy_json.len() > tiamud_core::MAX_LEGACY_WORLD_RUNTIME_SNAPSHOT_BYTES - 4_096,
            "fixture should exercise a near-limit legacy snapshot"
        );
        let upgraded = tiamud_core::decode_world_runtime_snapshot(2, &legacy_json).unwrap();
        assert!(
            serde_json::to_vec(&upgraded).unwrap().len()
                > tiamud_core::MAX_WORLD_RUNTIME_SNAPSHOT_BYTES,
            "the fixture must reproduce expansion beyond the current budget"
        );
        assert!(matches!(
            decode_runtime_snapshot(2, &legacy_json),
            Err(StoreError::SnapshotTooLarge {
                kind: "world runtime",
                maximum: tiamud_core::MAX_WORLD_RUNTIME_SNAPSHOT_BYTES,
            })
        ));
    }

    #[test]
    fn sqlite_runtime_loader_upgrades_v3_binding_as_unbound_and_rejects_smuggling() {
        let legacy = WorldRuntimeSnapshotV3 {
            snapshot_version: 3,
            revision: 8,
            logical_tick: 4,
            rng_state: 43,
            next_item_instance_sequence: 3,
            item_instances: BTreeMap::from([(
                ItemInstanceId("runtime:contained".into()),
                RuntimeItemInstanceSnapshotV3 {
                    prototype_id: "legacy:map".into(),
                    location: RuntimeItemLocation::Contained {
                        container_id: ItemInstanceId("runtime:satchel".into()),
                    },
                },
            )]),
            npcs: BTreeMap::new(),
            adventure: tiamud_core::AdventureRuntimeSnapshot::default(),
            progression_flags: Default::default(),
        };

        let mut connection = Connection::open_in_memory().unwrap();
        migrate(&mut connection).unwrap();
        connection
            .execute(
                "INSERT INTO world_runtime
                     (singleton, snapshot_version, snapshot_json, updated_at)
                 VALUES (1, 3, ?1, unixepoch())",
                [serde_json::to_string(&legacy).unwrap()],
            )
            .unwrap();

        let upgraded = load_runtime_snapshot(&connection).unwrap().unwrap();
        let item = &upgraded.item_instances[&ItemInstanceId("runtime:contained".into())];
        assert_eq!(item.binding, ItemBinding::Unbound);
        assert!(matches!(
            item.location,
            RuntimeItemLocation::Contained { .. }
        ));

        let mut smuggled = serde_json::to_value(&legacy).unwrap();
        smuggled["itemInstances"]["runtime:contained"]["binding"] =
            serde_json::json!({"kind":"character", "characterId":7});
        connection
            .execute(
                "UPDATE world_runtime SET snapshot_json = ?1 WHERE singleton = 1",
                [smuggled.to_string()],
            )
            .unwrap();
        assert!(matches!(
            load_runtime_snapshot(&connection),
            Err(StoreError::InvalidRuntimeSnapshot)
        ));
    }

    #[test]
    fn sqlite_runtime_loader_upgrades_strict_v4_and_rejects_trade_smuggling() {
        let mut connection = Connection::open_in_memory().unwrap();
        migrate(&mut connection).unwrap();
        let mut legacy = serde_json::to_value(World::demo().runtime_snapshot()).unwrap();
        legacy["snapshotVersion"] = serde_json::json!(4);
        for field in [
            "nextTradeId",
            "trades",
            "nextLootDropId",
            "itemProvenance",
            "lootEncounters",
            "groundLoot",
            "partyLootCursors",
            "nextRewardOpenId",
            "areaResidency",
        ] {
            legacy.as_object_mut().unwrap().remove(field);
        }
        for item in legacy["itemInstances"]
            .as_object_mut()
            .unwrap()
            .values_mut()
        {
            item.as_object_mut().unwrap().remove("itemAffixes");
        }
        connection
            .execute(
                "INSERT INTO world_runtime
                     (singleton, snapshot_version, snapshot_json, updated_at)
                 VALUES (1, 4, ?1, unixepoch())",
                [legacy.to_string()],
            )
            .unwrap();

        let upgraded = load_runtime_snapshot(&connection).unwrap().unwrap();
        assert_eq!(
            upgraded.snapshot_version,
            tiamud_core::WORLD_RUNTIME_SNAPSHOT_VERSION
        );
        assert_eq!(upgraded.next_trade_id, tiamud_core::TradeId(1));
        assert!(upgraded.trades.is_empty());

        legacy["trades"] = serde_json::json!({});
        connection
            .execute(
                "UPDATE world_runtime SET snapshot_json = ?1 WHERE singleton = 1",
                [legacy.to_string()],
            )
            .unwrap();
        assert!(matches!(
            load_runtime_snapshot(&connection),
            Err(StoreError::InvalidRuntimeSnapshot)
        ));
    }

    #[test]
    fn sqlite_runtime_loader_upgrades_strict_v6_and_rejects_v7_smuggling() {
        let mut legacy = serde_json::to_value(World::demo().runtime_snapshot()).unwrap();
        legacy["snapshotVersion"] = serde_json::json!(6);
        legacy.as_object_mut().unwrap().remove("nextRewardOpenId");
        legacy.as_object_mut().unwrap().remove("areaResidency");
        for item in legacy["itemInstances"]
            .as_object_mut()
            .unwrap()
            .values_mut()
        {
            item.as_object_mut().unwrap().remove("itemAffixes");
        }
        let json = legacy.to_string();
        let upgraded = decode_runtime_snapshot(6, &json).unwrap();
        assert_eq!(
            upgraded.snapshot_version,
            tiamud_core::WORLD_RUNTIME_SNAPSHOT_VERSION
        );
        assert_eq!(upgraded.next_reward_open_id, tiamud_core::RewardOpenId(1));
        assert!(
            upgraded
                .item_instances
                .values()
                .all(|item| item.item_affixes.is_empty())
        );

        let item_id = legacy["itemInstances"]
            .as_object()
            .unwrap()
            .keys()
            .next()
            .unwrap()
            .clone();
        legacy["itemInstances"][item_id]["itemAffixes"] = serde_json::json!([]);
        assert!(matches!(
            decode_runtime_snapshot(6, &legacy.to_string()),
            Err(StoreError::InvalidRuntimeSnapshot)
        ));
    }

    #[test]
    fn sqlite_runtime_loader_rejects_an_unsupported_snapshot_version() {
        let mut connection = Connection::open_in_memory().unwrap();
        migrate(&mut connection).unwrap();
        let legacy = serde_json::json!({
            "snapshotVersion": 99,
            "revision": 1,
            "logicalTick": 0,
            "rngState": 1,
            "nextItemInstanceSequence": 1,
            "itemInstances": {},
            "npcs": {}
        });
        connection
            .execute(
                "INSERT INTO world_runtime
                     (singleton, snapshot_version, snapshot_json, updated_at)
                 VALUES (1, 99, ?1, unixepoch())",
                [legacy.to_string()],
            )
            .unwrap();

        assert!(matches!(
            load_runtime_snapshot(&connection),
            Err(StoreError::InvalidRuntimeSnapshot)
        ));
    }

    #[test]
    fn sqlite_runtime_loader_rejects_column_payload_version_mismatch() {
        let mut connection = Connection::open_in_memory().unwrap();
        migrate(&mut connection).unwrap();
        let runtime = World::demo().runtime_snapshot();
        connection
            .execute(
                "INSERT INTO world_runtime
                     (singleton, snapshot_version, snapshot_json, updated_at)
                 VALUES (1, 1, ?1, unixepoch())",
                [serde_json::to_string(&runtime).unwrap()],
            )
            .unwrap();

        assert!(matches!(
            load_runtime_snapshot(&connection),
            Err(StoreError::InvalidRuntimeSnapshot)
        ));
    }

    #[test]
    fn sqlite_runtime_loader_rejects_multibyte_snapshot_above_byte_limit() {
        let mut connection = Connection::open_in_memory().unwrap();
        migrate(&mut connection).unwrap();
        let oversized = format!(
            "\"{}\"",
            "é".repeat(tiamud_core::MAX_LEGACY_WORLD_RUNTIME_SNAPSHOT_BYTES / 2)
        );
        assert!(oversized.chars().count() <= MAX_RUNTIME_JSON_BYTES);
        assert!(oversized.len() > tiamud_core::MAX_LEGACY_WORLD_RUNTIME_SNAPSHOT_BYTES);
        assert!(oversized.len() <= MAX_RUNTIME_JSON_BYTES);
        connection
            .execute(
                "INSERT INTO world_runtime
                     (singleton, snapshot_version, snapshot_json, updated_at)
                 VALUES (1, 2, ?1, unixepoch())",
                [&oversized],
            )
            .unwrap();

        assert!(matches!(
            load_runtime_snapshot(&connection),
            Err(StoreError::SnapshotTooLarge {
                kind: "world runtime",
                maximum: tiamud_core::MAX_LEGACY_WORLD_RUNTIME_SNAPSHOT_BYTES
            })
        ));
    }

    #[test]
    fn snapshot_v6_and_v7_use_the_two_mib_admission_budget() {
        let between_budgets = format!(
            "\"{}\"",
            "é".repeat(tiamud_core::MAX_LEGACY_WORLD_RUNTIME_SNAPSHOT_BYTES / 2)
        );
        assert!(between_budgets.len() > tiamud_core::MAX_LEGACY_WORLD_RUNTIME_SNAPSHOT_BYTES);
        assert!(between_budgets.len() <= MAX_RUNTIME_JSON_BYTES);

        assert!(matches!(
            decode_runtime_snapshot(5, &between_budgets),
            Err(StoreError::SnapshotTooLarge {
                kind: "world runtime",
                maximum: tiamud_core::MAX_LEGACY_WORLD_RUNTIME_SNAPSHOT_BYTES,
            })
        ));
        assert!(matches!(
            decode_runtime_snapshot(6, &between_budgets),
            Err(StoreError::InvalidRuntimeSnapshot)
        ));
        assert!(matches!(
            decode_runtime_snapshot(7, &between_budgets),
            Err(StoreError::InvalidRuntimeSnapshot)
        ));

        let above_current = format!("\"{}\"", "é".repeat(MAX_RUNTIME_JSON_BYTES / 2));
        assert!(above_current.len() > MAX_RUNTIME_JSON_BYTES);
        assert!(matches!(
            decode_runtime_snapshot(7, &above_current),
            Err(StoreError::SnapshotTooLarge {
                kind: "world runtime",
                maximum: MAX_RUNTIME_JSON_BYTES,
            })
        ));
    }

    #[test]
    fn startup_owner_gameplay_enforces_the_shared_aggregate_byte_budget() {
        let json = serde_json::to_string(&CharacterGameplaySnapshot::default()).unwrap();
        let mut total = MAX_STARTUP_OWNER_GAMEPLAY_BYTES - json.len() + 1;

        assert!(matches!(
            decode_startup_gameplay(
                CharacterId(1),
                &json,
                &mut total,
                StartupGameplayLoadLimits::ITEM_OWNERS,
            ),
            Err(StoreError::SnapshotTooLarge {
                kind: "startup owner gameplay",
                maximum: MAX_STARTUP_OWNER_GAMEPLAY_BYTES
            })
        ));
    }

    #[test]
    fn startup_trade_gameplay_uses_its_separate_exact_budget() {
        let json = serde_json::to_string(&CharacterGameplaySnapshot::default()).unwrap();
        let mut total = MAX_STARTUP_TRADE_GAMEPLAY_BYTES - json.len();
        decode_startup_gameplay(
            CharacterId(1),
            &json,
            &mut total,
            StartupGameplayLoadLimits::TRADE_PARTICIPANTS,
        )
        .unwrap();
        assert_eq!(total, MAX_STARTUP_TRADE_GAMEPLAY_BYTES);

        let mut overflowing_total = MAX_STARTUP_TRADE_GAMEPLAY_BYTES - json.len() + 1;
        assert!(matches!(
            decode_startup_gameplay(
                CharacterId(2),
                &json,
                &mut overflowing_total,
                StartupGameplayLoadLimits::TRADE_PARTICIPANTS,
            ),
            Err(StoreError::SnapshotTooLarge {
                kind: "startup trade participant gameplay",
                maximum: MAX_STARTUP_TRADE_GAMEPLAY_BYTES,
            })
        ));
    }

    #[test]
    fn startup_trade_batch_rejects_excess_and_duplicate_results() {
        let ids = (1..=MAX_STARTUP_TRADE_PARTICIPANTS + 1)
            .map(|id| CharacterId(i64::try_from(id).unwrap()))
            .collect();
        assert!(matches!(
            normalise_startup_character_ids(ids, StartupGameplayLoadLimits::TRADE_PARTICIPANTS),
            Err(StoreError::TooManyCharacterAggregates {
                maximum: MAX_STARTUP_TRADE_PARTICIPANTS,
            })
        ));

        let aggregate = DurableCharacterGameplay {
            character_id: CharacterId(1),
            gameplay: CharacterGameplaySnapshot::default(),
        };
        assert!(matches!(
            ensure_all_startup_characters_loaded(
                &[CharacterId(1)],
                &[aggregate.clone(), aggregate],
            ),
            Err(StoreError::InvalidRuntimeSnapshot)
        ));
    }

    #[test]
    fn startup_identity_scope_deduplicates_and_rejects_invalid_or_excess_ids() {
        assert_eq!(
            normalise_character_identity_ids(vec![
                CharacterId(3),
                CharacterId(1),
                CharacterId(3),
                CharacterId(2),
            ])
            .unwrap(),
            vec![CharacterId(1), CharacterId(2), CharacterId(3)]
        );
        assert!(matches!(
            normalise_character_identity_ids(vec![CharacterId(0)]),
            Err(StoreError::InvalidCharacterIdentity(CharacterId(0)))
        ));

        let excess = (1..=MAX_STARTUP_CHARACTER_EXISTENCE_IDS + 1)
            .map(|id| CharacterId(i64::try_from(id).unwrap()))
            .collect();
        assert!(matches!(
            normalise_character_identity_ids(excess),
            Err(StoreError::TooManyCharacterIdentities {
                maximum: MAX_STARTUP_CHARACTER_EXISTENCE_IDS,
            })
        ));
    }

    #[test]
    fn sqlite_identity_existence_query_crosses_the_parameter_batch_boundary() {
        let mut connection = Connection::open_in_memory().unwrap();
        initialise(&mut connection).unwrap();
        connection
            .execute("INSERT INTO accounts (id, name) VALUES (1, 'Owner')", [])
            .unwrap();
        let transaction = connection.transaction().unwrap();
        {
            let mut statement = transaction
                .prepare(
                    "INSERT INTO characters (id, account_id, name, room_id)
                     VALUES (?1, 1, ?2, 'test:start')",
                )
                .unwrap();
            for id in 1..=SQLITE_CHARACTER_BATCH_SIZE + 1 {
                let id = i64::try_from(id).unwrap();
                statement
                    .execute(params![id, format!("Character {id}")])
                    .unwrap();
            }
        }
        transaction.commit().unwrap();

        let ids = (1..=SQLITE_CHARACTER_BATCH_SIZE + 1)
            .rev()
            .map(|id| CharacterId(i64::try_from(id).unwrap()))
            .collect();
        ensure_character_ids_exist_batch(&connection, ids).unwrap();
        assert!(matches!(
            ensure_character_ids_exist_batch(
                &connection,
                vec![CharacterId(
                    i64::try_from(SQLITE_CHARACTER_BATCH_SIZE + 2).unwrap()
                )],
            ),
            Err(StoreError::CharacterIdentityNotFound(_))
        ));
    }

    #[tokio::test]
    async fn sqlite_store_checks_identity_only_with_dedupe_and_fail_closed_errors() {
        let store = SqliteCharacterStore::open(":memory:").await.unwrap();
        let owner = account(&store, "Identity Owner").await;
        let first = store
            .load_or_create_character(owner.id, "Aria", "demo:lantern-square")
            .await
            .unwrap();
        let second = store
            .load_or_create_character(owner.id, "Bryn", "demo:lantern-square")
            .await
            .unwrap();

        store
            .ensure_character_ids_exist_batch(vec![second.id, first.id, second.id])
            .await
            .unwrap();
        store
            .ensure_character_ids_exist_batch(Vec::new())
            .await
            .unwrap();
        assert!(matches!(
            store
                .ensure_character_ids_exist_batch(vec![CharacterId(0)])
                .await,
            Err(StoreError::InvalidCharacterIdentity(CharacterId(0)))
        ));
        let missing = CharacterId(second.id.0 + 1_000);
        assert!(matches!(
            store.ensure_character_ids_exist_batch(vec![missing]).await,
            Err(StoreError::CharacterIdentityNotFound(character_id)) if character_id == missing
        ));
    }

    #[tokio::test]
    async fn gameplay_and_runtime_snapshots_round_trip_together() {
        let store = SqliteCharacterStore::open(":memory:").await.unwrap();
        let owner = account(&store, "Snapshot Owner").await;
        let character = store
            .load_or_create_character(owner.id, "Aria", "demo:lantern-square")
            .await
            .unwrap();
        let defaults = CharacterGameplaySnapshot::default();
        let gameplay = CharacterGameplaySnapshot {
            level: 4,
            experience: 375,
            resources: std::collections::BTreeMap::from([(
                "tiamud:focus".into(),
                tiamud_core::ResourcePool {
                    current: 7,
                    max: 12,
                },
            )]),
            ability_cooldowns: std::collections::BTreeMap::from([(
                "tiamud:mind-whisper".into(),
                23,
            )]),
            vitals: tiamud_core::Vitals {
                health: 0,
                max_health: defaults.vitals.max_health,
            },
            equipment: std::collections::BTreeMap::from([(
                EquipmentSlot::Weapon,
                ItemInstanceId("tiamud:bronze-sword".to_owned()),
            )]),
            effects: vec![ActiveEffect {
                kind: EffectKind::Strength,
                magnitude: 3,
                remaining_ticks: 2,
            }],
            recovery_at_tick: Some(19),
            coins: 123,
            commodities: std::collections::BTreeMap::from([("tonic".into(), 2)]),
            quests: std::collections::BTreeMap::from([(
                "first-hunt".into(),
                tiamud_core::QuestProgress {
                    status: tiamud_core::QuestStatus::ReadyToClaim,
                    current: 1,
                    target: 1,
                },
            )]),
            script_flags: std::collections::BTreeSet::from(["archive-welcomed".into()]),
            scheduled_script_flags: vec![tiamud_core::ScheduledScriptFlag {
                flag: "archive-follow-up".into(),
                due_tick: 23,
            }],
            ..defaults
        };
        let loaded = crate::world_loader::default_loaded_world().unwrap();
        let mut runtime = crate::world_loader::build_loaded_world(&loaded)
            .unwrap()
            .runtime_snapshot();
        runtime.logical_tick = 17;
        runtime.adventure.next_weather_tick = 20;
        runtime.adventure.shop_stock.insert("tonic".into(), 18);
        let sword = runtime
            .item_instances
            .get_mut(&ItemInstanceId("tiamud:bronze-sword".into()))
            .unwrap();
        sword.location = RuntimeItemLocation::Equipped {
            character_id: character.id,
            slot: EquipmentSlot::Weapon,
        };
        sword.binding = ItemBinding::Unbound;
        let mut restored = crate::world_loader::build_loaded_world(&loaded).unwrap();
        restored.restore_runtime_snapshot(runtime.clone()).unwrap();

        store
            .commit_action(
                vec![DurableCharacterSnapshot {
                    character: character.clone(),
                    gameplay: gameplay.clone(),
                }],
                runtime.clone(),
                Vec::new(),
            )
            .await
            .unwrap();

        assert_eq!(
            store.load_character_gameplay(character.id).await.unwrap(),
            gameplay
        );
        assert_eq!(
            store
                .load_character_gameplay_batch(
                    vec![character.id],
                    StartupGameplayLoadLimits::ITEM_OWNERS,
                )
                .await
                .unwrap(),
            vec![DurableCharacterGameplay {
                character_id: character.id,
                gameplay: gameplay.clone(),
            }]
        );
        assert_eq!(store.load_runtime_snapshot().await.unwrap(), Some(runtime));
    }

    #[tokio::test]
    async fn items_given_event_round_trips_through_sqlite() {
        let store = SqliteCharacterStore::open(":memory:").await.unwrap();
        let event = DomainEvent::ItemsGiven {
            giver_id: CharacterId(11),
            giver_name: "Aria".into(),
            recipient_id: CharacterId(12),
            recipient_name: "Bryn".into(),
            item_ids: vec![ItemInstanceId("runtime:gift-1".into())],
            item_name: "gift".into(),
            room_id: "demo:lantern-square".into(),
        };

        store
            .commit_action(
                Vec::new(),
                World::demo().runtime_snapshot(),
                vec![event.clone()],
            )
            .await
            .unwrap();

        assert_eq!(store.load_domain_events().await.unwrap(), vec![event]);
    }

    #[tokio::test]
    async fn v3_progression_survives_a_sqlite_restart_and_restores_fail_closed() {
        let temporary = tempfile::TempDir::new().unwrap();
        let database_path = temporary.path().join("progression.sqlite3");
        let store = SqliteCharacterStore::open(&database_path).await.unwrap();
        let owner = account(&store, "Progression Owner").await;
        let loaded = crate::world_loader::default_loaded_world().unwrap();
        let mut world = crate::world_loader::build_loaded_world(&loaded).unwrap();
        let mut gameplay = world
            .initial_gameplay(
                Some("tiamud:mortal"),
                BackgroundSelection::Id("tiamud:city-guard"),
                Some("tiamud:vanguard"),
            )
            .unwrap();
        gameplay.level = 5;
        gameplay.experience = 1_600;
        gameplay.progression_points.insert("talent".into(), 1);
        gameplay.tags.insert("tiamud:founder".into());
        let character = store
            .load_or_create_character_with_gameplay(
                owner.id,
                "Iria",
                world.start_room_id(),
                gameplay.clone(),
            )
            .await
            .unwrap();

        world
            .restore_character_gameplay(character.id, gameplay)
            .unwrap();
        world
            .receive_progression_signal(character.id, "tiamud:heard-call")
            .unwrap();
        let session = SessionId(77);
        world.connect(session, character.clone()).unwrap();
        let talent = world
            .apply(
                session,
                PlayerAction::AcquireTalent {
                    graph_id: "tiamud:martial-talents".into(),
                    node_id: "tiamud:guarded-stance".into(),
                },
            )
            .unwrap();
        let promotion = world
            .apply(
                session,
                PlayerAction::Promote {
                    class_id: "tiamud:bastion".into(),
                },
            )
            .unwrap();
        let mut events = talent.domain_events;
        events.extend(promotion.domain_events);
        store
            .commit_action(
                vec![world.durable_character_snapshot(session).unwrap()],
                world.runtime_snapshot(),
                events,
            )
            .await
            .unwrap();
        store.shutdown().await.unwrap();
        drop(store);

        let reopened = SqliteCharacterStore::open(&database_path).await.unwrap();
        let restored = reopened
            .load_character_gameplay(character.id)
            .await
            .unwrap();
        assert_eq!(restored.lineage_id, "tiamud:mortal");
        assert_eq!(restored.background_id.as_deref(), Some("tiamud:city-guard"));
        assert_eq!(restored.base_class_id, "tiamud:vanguard");
        assert_eq!(restored.active_class_id, "tiamud:bastion");
        assert_eq!(
            restored.class_history,
            vec!["tiamud:vanguard".to_owned(), "tiamud:bastion".to_owned()]
        );
        assert_eq!(restored.progression_points["talent"], 0);
        assert_eq!(
            restored.talent_node_ranks["tiamud:martial-talents"]["tiamud:guarded-stance"],
            1
        );
        assert!(restored.tags.contains("tiamud:founder"));
        assert!(restored.received_signals.contains("tiamud:heard-call"));

        let mut restarted_world = crate::world_loader::build_loaded_world(&loaded).unwrap();
        restarted_world
            .restore_character_gameplay(character.id, restored.clone())
            .unwrap();
        restarted_world.connect(session, character.clone()).unwrap();
        assert_eq!(
            restarted_world
                .character_gameplay_snapshot(session)
                .unwrap(),
            restored
        );
        let mut unresolved = restored.clone();
        unresolved.active_class_id = "missing:class".into();
        unresolved.class_history[1] = "missing:class".into();
        assert!(matches!(
            restarted_world.restore_character_gameplay(character.id, unresolved),
            Err(WorldError::InvalidGameplayDefinition)
        ));
        assert_eq!(
            restarted_world
                .character_gameplay_snapshot(session)
                .unwrap(),
            restored,
            "a rejected unresolved definition must not mutate restored state"
        );
        reopened.shutdown().await.unwrap();
    }

    #[tokio::test]
    async fn accounts_can_own_same_named_characters_without_identity_collision() {
        let store = SqliteCharacterStore::open(":memory:").await.unwrap();
        let first = account(&store, "First").await;
        let second = account(&store, "Second").await;
        let first_character = store
            .load_or_create_character(first.id, "Hero", "room:a")
            .await
            .unwrap();
        let second_character = store
            .load_or_create_character(second.id, "Hero", "room:b")
            .await
            .unwrap();

        assert_ne!(first_character.id, second_character.id);
        assert_eq!(first_character.account_id, first.id);
        assert_eq!(second_character.account_id, second.id);
        assert_eq!(store.list_characters(first.id).await.unwrap().len(), 1);
        assert_eq!(store.list_characters(second.id).await.unwrap().len(), 1);
    }

    #[tokio::test]
    async fn character_selection_checks_account_ownership() {
        let store = SqliteCharacterStore::open(":memory:").await.unwrap();
        let owner = account(&store, "Owner").await;
        let stranger = account(&store, "Stranger").await;
        let character = store
            .load_or_create_character(owner.id, "Private", "room:a")
            .await
            .unwrap();

        assert!(matches!(
            store.load_character(stranger.id, character.id).await,
            Err(StoreError::CharacterNotFound { .. })
        ));
    }

    #[tokio::test]
    async fn missing_accounts_and_invalid_names_are_rejected() {
        let store = SqliteCharacterStore::open(":memory:").await.unwrap();
        assert!(matches!(
            store.load_or_create_account(" \n ").await,
            Err(StoreError::InvalidName { kind: "account" })
        ));
        assert!(matches!(
            store.load_or_create_account("A\u{202e}B").await,
            Err(StoreError::InvalidName { kind: "account" })
        ));
        assert!(matches!(
            store.load_or_create_account("A|B").await,
            Err(StoreError::InvalidName { kind: "account" })
        ));
        assert!(matches!(
            store
                .load_or_create_character(AccountId(999), "Hero", "room:a")
                .await,
            Err(StoreError::AccountNotFound(AccountId(999)))
        ));
    }

    #[test]
    fn durable_ids_are_unchanged_after_the_database_is_reopened() {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let path = std::env::temp_dir().join(format!(
            "mud-persistence-{}-{nonce}.sqlite3",
            std::process::id()
        ));

        let (account_id, character_id, login_token, login_account_id) = {
            let mut connection = Connection::open(&path).unwrap();
            initialise(&mut connection).unwrap();
            let account = load_or_create_account(&connection, "Persistent Player").unwrap();
            let character = load_or_create_character(
                &connection,
                account.id,
                "Persistent Hero",
                "demo:lantern-square",
            )
            .unwrap();
            let login = register_account(
                &mut connection,
                "Persistent Login",
                "correct horse battery staple",
                1_000,
            )
            .unwrap();
            (
                account.id,
                character.id,
                login.token.as_str().to_owned(),
                login.account.id,
            )
        };

        {
            let mut connection = Connection::open(&path).unwrap();
            initialise(&mut connection).unwrap();
            let account = load_or_create_account(&connection, "persistent player").unwrap();
            let character = load_character(&connection, account.id, character_id).unwrap();
            assert_eq!(account.id, account_id);
            assert_eq!(character.id, character_id);
            assert_eq!(character.account_id, account_id);
            assert_eq!(
                resolve_session(&connection, &login_token, 1_001)
                    .unwrap()
                    .id,
                login_account_id
            );
        }

        for suffix in ["", "-shm", "-wal"] {
            let _ = std::fs::remove_file(format!("{}{suffix}", path.display()));
        }
    }

    #[test]
    fn migrates_the_name_keyed_bootstrap_schema_without_losing_location() {
        let mut connection = Connection::open_in_memory().unwrap();
        connection
            .execute_batch(
                "CREATE TABLE characters (
                     name TEXT PRIMARY KEY COLLATE NOCASE,
                     room_id TEXT NOT NULL,
                     updated_at INTEGER NOT NULL DEFAULT (unixepoch())
                 );
                 INSERT INTO characters (name, room_id)
                     VALUES ('Legacy Hero', 'demo:north-gate');",
            )
            .unwrap();

        initialise(&mut connection).unwrap();
        let migrated_account = load_or_create_account(&connection, "legacy hero").unwrap();
        let migrated = list_characters(&connection, migrated_account.id).unwrap();

        assert_eq!(migrated.len(), 1);
        assert_eq!(migrated[0].name, "Legacy Hero");
        assert_eq!(migrated[0].room_id, "demo:north-gate");
        assert!(migrated[0].id.0 > 0);
        let migrated_column: bool = connection
            .query_row(
                "SELECT EXISTS(
                     SELECT 1 FROM pragma_table_info('characters') WHERE name = 'account_id'
                 )",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert!(migrated_column);
    }

    #[test]
    fn migrates_v2_accounts_as_explicit_guests_without_losing_rosters() {
        let mut connection = Connection::open_in_memory().unwrap();
        connection
            .execute_batch(
                "CREATE TABLE accounts (
                     id INTEGER PRIMARY KEY AUTOINCREMENT,
                     name TEXT NOT NULL COLLATE NOCASE UNIQUE,
                     created_at INTEGER NOT NULL DEFAULT (unixepoch())
                 );
                 CREATE TABLE characters (
                     id INTEGER PRIMARY KEY AUTOINCREMENT,
                     account_id INTEGER NOT NULL REFERENCES accounts(id) ON DELETE CASCADE,
                     name TEXT NOT NULL COLLATE NOCASE,
                     room_id TEXT NOT NULL,
                     created_at INTEGER NOT NULL DEFAULT (unixepoch()),
                     updated_at INTEGER NOT NULL DEFAULT (unixepoch()),
                     UNIQUE(account_id, name)
                 );
                 INSERT INTO accounts (id, name) VALUES (7, 'Legacy Guest');
                 INSERT INTO characters (id, account_id, name, room_id)
                     VALUES (9, 7, 'Old Hero', 'demo:north-gate');
                 PRAGMA user_version = 2;",
            )
            .unwrap();

        initialise(&mut connection).unwrap();
        let (password_hash, state): (Option<String>, String) = connection
            .query_row(
                "SELECT password_hash, auth_state FROM accounts WHERE id = 7",
                [],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .unwrap();
        let roster = list_characters(&connection, AccountId(7)).unwrap();

        assert_eq!(password_hash, None);
        assert_eq!(state, "guest");
        assert_eq!(roster[0].id, CharacterId(9));
        assert_eq!(roster[0].room_id, "demo:north-gate");
        assert!(matches!(
            login_account(
                &mut connection,
                "legacy guest",
                "correct horse battery staple",
                1_000
            ),
            Err(StoreError::InvalidCredentials)
        ));
    }

    #[test]
    fn sqlite_report_resolution_binds_the_real_target_and_rejects_a_discordant_audit() {
        let mut connection = Connection::open_in_memory().unwrap();
        initialise(&mut connection).unwrap();
        let admin = register_account(
            &mut connection,
            "Admin",
            "correct horse battery staple",
            1_000,
        )
        .unwrap();
        let reporter = register_account(
            &mut connection,
            "Reporter",
            "another correct horse battery",
            1_000,
        )
        .unwrap();
        let target = register_account(
            &mut connection,
            "Target",
            "a third correct horse battery",
            1_000,
        )
        .unwrap();
        let reporter_character =
            load_or_create_character(&connection, reporter.account.id, "Bryn", "room:start")
                .unwrap();
        let target_character =
            load_or_create_character(&connection, target.account.id, "Cyra", "room:start").unwrap();

        report_character(
            &mut connection,
            reporter_character.id,
            target_character.id,
            "harassment in public chat",
            1_100,
        )
        .unwrap();
        let report_id = connection
            .query_row("SELECT MAX(id) FROM social_reports", [], |row| {
                row.get::<_, i64>(0)
            })
            .unwrap();
        resolve_staff_report(
            &mut connection,
            report_id,
            StaffReportResolution::ActionTaken,
            "evidence verified",
            StaffAuditRecord {
                actor_account_id: admin.account.id,
                target_account_id: None,
                capability: StaffCapability::ReportsResolve,
                action: "report_resolve".to_owned(),
                reason: "evidence verified".to_owned(),
                details: serde_json::json!({"report_id": report_id}),
            },
            1_200,
        )
        .unwrap();
        assert_eq!(
            connection
                .query_row(
                    "SELECT status, resolution, resolution_reason,
                            resolved_by_account_id
                     FROM social_reports WHERE id = ?1",
                    [report_id],
                    |row| {
                        Ok((
                            row.get::<_, String>(0)?,
                            row.get::<_, String>(1)?,
                            row.get::<_, String>(2)?,
                            row.get::<_, i64>(3)?,
                        ))
                    },
                )
                .unwrap(),
            (
                "resolved".to_owned(),
                "action_taken".to_owned(),
                "evidence verified".to_owned(),
                admin.account.id.0,
            )
        );
        assert_eq!(
            connection
                .query_row(
                    "SELECT target_account_id FROM staff_audit
                     WHERE action = 'report_resolve'",
                    [],
                    |row| row.get::<_, i64>(0),
                )
                .unwrap(),
            target.account.id.0
        );

        report_character(
            &mut connection,
            reporter_character.id,
            target_character.id,
            "a second incident",
            1_300,
        )
        .unwrap();
        let second_report_id = connection
            .query_row("SELECT MAX(id) FROM social_reports", [], |row| {
                row.get::<_, i64>(0)
            })
            .unwrap();
        let audit_rows_before = connection
            .query_row("SELECT COUNT(*) FROM staff_audit", [], |row| {
                row.get::<_, i64>(0)
            })
            .unwrap();
        assert!(matches!(
            resolve_staff_report(
                &mut connection,
                second_report_id,
                StaffReportResolution::Dismissed,
                "forged target",
                StaffAuditRecord {
                    actor_account_id: admin.account.id,
                    target_account_id: Some(reporter.account.id),
                    capability: StaffCapability::ReportsResolve,
                    action: "report_resolve".to_owned(),
                    reason: "forged target".to_owned(),
                    details: serde_json::json!({"report_id": second_report_id}),
                },
                1_400,
            ),
            Err(StoreError::StaffForbidden)
        ));
        assert_eq!(
            connection
                .query_row(
                    "SELECT status FROM social_reports WHERE id = ?1",
                    [second_report_id],
                    |row| row.get::<_, String>(0),
                )
                .unwrap(),
            "open"
        );
        assert_eq!(
            connection
                .query_row("SELECT COUNT(*) FROM staff_audit", [], |row| {
                    row.get::<_, i64>(0)
                })
                .unwrap(),
            audit_rows_before
        );
    }

    #[tokio::test]
    async fn social_policy_roles_bans_and_audit_survive_restart() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("social.db");
        let store = SqliteCharacterStore::open(&path).await.unwrap();
        let admin = store
            .register_account("Admin", "correct horse battery staple")
            .await
            .unwrap();
        let player = store
            .register_account("Player", "another correct horse battery")
            .await
            .unwrap();
        let third = store
            .register_account("Third", "a third correct horse battery")
            .await
            .unwrap();
        let admin_character = store
            .load_or_create_character(admin.account.id, "Aster", "room:start")
            .await
            .unwrap();
        let player_character = store
            .load_or_create_character(player.account.id, "Bryn", "room:start")
            .await
            .unwrap();
        let third_character = store
            .load_or_create_character(third.account.id, "Cyra", "room:start")
            .await
            .unwrap();

        assert_eq!(
            store.social_profile(admin_character.id).await.unwrap().role,
            AccountRole::Admin
        );
        assert!(matches!(
            store
                .moderate_character(
                    player_character.id,
                    third_character.id,
                    ModerationAction::Mute {
                        expires_at: None,
                        reason: "not authorized".into(),
                    },
                )
                .await,
            Err(StoreError::SocialForbidden)
        ));

        store
            .set_social_relation(
                player_character.id,
                admin_character.id,
                SocialRelationKind::Ignore,
                true,
            )
            .await
            .unwrap();
        assert!(
            !store
                .social_delivery_allowed(admin_character.id, player_character.id)
                .await
                .unwrap()
        );
        store
            .set_social_relation(
                player_character.id,
                third_character.id,
                SocialRelationKind::Block,
                true,
            )
            .await
            .unwrap();
        assert!(
            !store
                .social_delivery_allowed(player_character.id, third_character.id)
                .await
                .unwrap()
        );
        assert!(
            !store
                .social_delivery_allowed(third_character.id, player_character.id)
                .await
                .unwrap()
        );

        store
            .moderate_character(
                admin_character.id,
                player_character.id,
                ModerationAction::SetRole {
                    role: AccountRole::Moderator,
                    reason: "approved moderation assignment".to_owned(),
                },
            )
            .await
            .unwrap();
        assert!(matches!(
            store
                .moderate_character(
                    player_character.id,
                    third_character.id,
                    ModerationAction::Mute {
                        expires_at: None,
                        reason: "legacy role is not authority".into(),
                    },
                )
                .await,
            Err(StoreError::SocialForbidden)
        ));
        store
            .set_staff_profile(
                player.account.id,
                Some(StaffProfileKey::Moderator),
                None,
                StaffAuditRecord {
                    actor_account_id: admin.account.id,
                    target_account_id: Some(player.account.id),
                    capability: StaffCapability::StaffAssign,
                    action: "staff_profile_set".into(),
                    reason: "approved moderation assignment".into(),
                    details: serde_json::json!({
                        "actor_name": "Aster",
                        "target_name": "Bryn",
                        "profile": "moderator",
                    }),
                },
            )
            .await
            .unwrap();
        store
            .moderate_character(
                player_character.id,
                third_character.id,
                ModerationAction::Mute {
                    expires_at: None,
                    reason: "repeated flooding".into(),
                },
            )
            .await
            .unwrap();
        store
            .report_character(
                player_character.id,
                third_character.id,
                "harassment in public chat",
            )
            .await
            .unwrap();
        store
            .moderate_character(
                admin_character.id,
                third_character.id,
                ModerationAction::Ban {
                    expires_at: None,
                    reason: "repeated severe abuse".into(),
                },
            )
            .await
            .unwrap();
        assert!(matches!(
            store
                .login_account("Third", "a third correct horse battery")
                .await,
            Err(StoreError::AccountBanned)
        ));

        drop(store);
        let reopened = SqliteCharacterStore::open(&path).await.unwrap();
        assert_eq!(
            reopened
                .social_profile(player_character.id)
                .await
                .unwrap()
                .role,
            AccountRole::Moderator
        );
        assert!(
            reopened
                .social_profile(third_character.id)
                .await
                .unwrap()
                .muted
        );
        assert!(
            !reopened
                .social_delivery_allowed(admin_character.id, player_character.id)
                .await
                .unwrap()
        );
        assert!(matches!(
            reopened
                .login_account("Third", "a third correct horse battery")
                .await,
            Err(StoreError::AccountBanned)
        ));
        let actions = reopened
            .audit_log()
            .await
            .unwrap()
            .into_iter()
            .map(|entry| entry.action)
            .collect::<Vec<_>>();
        for expected in [
            "bootstrap_admin",
            "ignore",
            "block",
            "set_role",
            "mute",
            "report",
            "ban",
        ] {
            assert!(actions.iter().any(|action| action == expected));
        }
        let verification = Connection::open(&path).unwrap();
        assert_eq!(
            verification
                .query_row(
                    "SELECT reason FROM social_reports WHERE reporter_character_id = ?1",
                    [player_character.id.0],
                    |row| row.get::<_, String>(0),
                )
                .unwrap(),
            "harassment in public chat"
        );
    }

    #[test]
    fn staff_authority_reports_and_audit_are_closed_bounded_and_hierarchical() {
        let mut connection = Connection::open_in_memory().unwrap();
        initialise(&mut connection).unwrap();
        let admin = finish_register_account(&mut connection, "Admin", "hash", 1_000).unwrap();
        let reporter = finish_register_account(&mut connection, "Reporter", "hash", 1_001).unwrap();
        let target = finish_register_account(&mut connection, "Target", "hash", 1_002).unwrap();
        let admin_character =
            load_or_create_character(&connection, admin.account.id, "Aster", "room:start").unwrap();
        let reporter_character =
            load_or_create_character(&connection, reporter.account.id, "Bryn", "room:start")
                .unwrap();
        let target_character =
            load_or_create_character(&connection, target.account.id, "Cyra", "room:start").unwrap();

        let access = staff_access(&connection, admin_character.id).unwrap();
        assert_eq!(access.profile, Some(StaffProfileKey::Implementor));
        assert_eq!(
            access.capabilities,
            StaffCapability::ALL.into_iter().collect()
        );

        for sequence in 0..12 {
            report_character(
                &mut connection,
                reporter_character.id,
                target_character.id,
                &format!("report {sequence}"),
                1_100 + sequence,
            )
            .unwrap();
        }
        report_character(
            &mut connection,
            reporter_character.id,
            admin_character.id,
            "report targeting equal staff authority",
            1_200,
        )
        .unwrap();
        let reports = staff_reports(
            &mut connection,
            StaffAuditRecord {
                actor_account_id: admin.account.id,
                target_account_id: None,
                capability: StaffCapability::ReportsView,
                action: "reports.list".into(),
                reason: "review open reports".into(),
                details: serde_json::json!({}),
            },
            1_201,
        )
        .unwrap();
        assert_eq!(reports.len(), usize::try_from(STAFF_LIST_LIMIT).unwrap());
        assert!(
            reports
                .iter()
                .all(|report| report.target_account_id == target.account.id)
        );

        let report_id = reports[0].id;
        resolve_staff_report(
            &mut connection,
            report_id,
            StaffReportResolution::ActionTaken,
            "moderation action recorded",
            StaffAuditRecord {
                actor_account_id: admin.account.id,
                target_account_id: None,
                capability: StaffCapability::ReportsResolve,
                action: "reports.resolve".into(),
                reason: "review completed".into(),
                details: serde_json::json!({"report_id": report_id}),
            },
            1_202,
        )
        .unwrap();
        let resolution: (String, String, String, i64) = connection
            .query_row(
                "SELECT status, resolution, resolution_reason, resolved_by_account_id
                 FROM social_reports WHERE id = ?1",
                [report_id],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
            )
            .unwrap();
        assert_eq!(
            resolution,
            (
                "resolved".into(),
                "action_taken".into(),
                "moderation action recorded".into(),
                admin.account.id.0,
            )
        );

        connection
            .execute(
                "INSERT INTO staff_profiles
                     (account_id, profile_key, lore_title, assigned_by_account_id,
                      created_at, updated_at)
                 VALUES (?1, 'builder', NULL, ?2, 1202, 1202)",
                params![reporter.account.id.0, admin.account.id.0],
            )
            .unwrap();
        connection
            .execute(
                "INSERT INTO staff_capability_assignments
                     (account_id, capability, assigned_by_account_id, created_at)
                 VALUES (?1, 'staff.where', ?2, 1202)",
                params![reporter.account.id.0, admin.account.id.0],
            )
            .unwrap();

        set_staff_profile(
            &mut connection,
            reporter.account.id,
            Some(StaffProfileKey::Moderator),
            Some("Custodio de la plaza"),
            StaffAuditRecord {
                actor_account_id: admin.account.id,
                target_account_id: Some(reporter.account.id),
                capability: StaffCapability::StaffAssign,
                action: "staff.profile.set".into(),
                reason: "approved moderator assignment".into(),
                details: serde_json::json!({"profile": "moderator"}),
            },
            1_203,
        )
        .unwrap();
        let reporter_access = staff_access(&connection, reporter_character.id).unwrap();
        assert_eq!(reporter_access.profile, Some(StaffProfileKey::Moderator));
        assert_eq!(
            reporter_access.capabilities,
            StaffProfileKey::Moderator
                .default_capabilities()
                .iter()
                .copied()
                .collect()
        );

        assert!(matches!(
            set_staff_profile(
                &mut connection,
                target.account.id,
                Some(StaffProfileKey::Implementor),
                None,
                StaffAuditRecord {
                    actor_account_id: admin.account.id,
                    target_account_id: Some(target.account.id),
                    capability: StaffCapability::StaffAssign,
                    action: "staff.profile.set".into(),
                    reason: "attempt equal profile".into(),
                    details: serde_json::json!({}),
                },
                1_204,
            ),
            Err(StoreError::StaffHierarchy)
        ));
        assert!(matches!(
            set_staff_capability(
                &mut connection,
                target.account.id,
                StaffCapability::StaffWhere,
                true,
                StaffAuditRecord {
                    actor_account_id: reporter.account.id,
                    target_account_id: Some(target.account.id),
                    capability: StaffCapability::StaffAssign,
                    action: "staff.capability.set".into(),
                    reason: "attempt without assignment authority".into(),
                    details: serde_json::json!({"capability": "staff.where"}),
                },
                1_205,
            ),
            Err(StoreError::StaffForbidden)
        ));

        let audit = staff_audit_entries(
            &mut connection,
            StaffAuditRecord {
                actor_account_id: admin.account.id,
                target_account_id: None,
                capability: StaffCapability::StaffAudit,
                action: "staff.audit.list".into(),
                reason: "bounded audit review".into(),
                details: serde_json::json!({}),
            },
            1_206,
        )
        .unwrap();
        assert!(!audit.is_empty());
        assert!(audit.len() <= usize::try_from(STAFF_LIST_LIMIT).unwrap());
        assert!(audit.iter().all(|entry| entry.sequence > 0));
    }

    #[tokio::test]
    async fn closed_channel_subscriptions_are_idempotent_and_survive_restart() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("channel-subscriptions.db");
        let store = SqliteCharacterStore::open(&path).await.unwrap();
        let account = store
            .register_account("Channel Keeper", "correct horse battery staple")
            .await
            .unwrap();
        let character = store
            .load_or_create_character(account.account.id, "Aster", "room:start")
            .await
            .unwrap();

        assert!(
            store
                .social_channel_subscriptions(character.id)
                .await
                .unwrap()
                .is_empty()
        );
        for channel in [
            SocialChannelName::Staff,
            SocialChannelName::General,
            SocialChannelName::Newbie,
            SocialChannelName::General,
        ] {
            store
                .set_social_channel_subscription(character.id, channel, true)
                .await
                .unwrap();
        }
        assert_eq!(
            store
                .social_channel_subscriptions(character.id)
                .await
                .unwrap(),
            vec![
                SocialChannelName::General,
                SocialChannelName::Newbie,
                SocialChannelName::Staff,
            ]
        );
        assert_eq!(
            store
                .social_channel_subscribers(SocialChannelName::General)
                .await
                .unwrap(),
            vec![character.id]
        );
        for _ in 0..2 {
            store
                .set_social_channel_subscription(character.id, SocialChannelName::Newbie, false)
                .await
                .unwrap();
        }
        assert!(matches!(
            store
                .social_channel_subscriptions(CharacterId(i64::MAX))
                .await,
            Err(StoreError::SocialTargetNotFound)
        ));

        store.shutdown().await.unwrap();
        drop(store);
        let reopened = SqliteCharacterStore::open(&path).await.unwrap();
        assert_eq!(
            reopened
                .social_channel_subscriptions(character.id)
                .await
                .unwrap(),
            vec![SocialChannelName::General, SocialChannelName::Staff]
        );
        assert!(
            reopened
                .social_channel_subscribers(SocialChannelName::Newbie)
                .await
                .unwrap()
                .is_empty()
        );
    }

    fn mail_fixture(connection: &Connection) -> (CharacterId, CharacterId, CharacterId) {
        connection
            .execute_batch(
                "INSERT INTO accounts (id, name) VALUES
                     (101, 'Mail Sender'), (102, 'Mail Recipient');
                 INSERT INTO characters (id, account_id, name, room_id) VALUES
                     (1001, 101, 'Aster', 'room:start'),
                     (1002, 102, 'Bryn', 'room:start'),
                     (1003, 101, 'AsterAlt', 'room:start');",
            )
            .unwrap();
        (CharacterId(1001), CharacterId(1002), CharacterId(1003))
    }

    fn mail_id(sequence: u128) -> String {
        format!("m{sequence:032x}")
    }

    fn board_id(sequence: u128) -> String {
        format!("b{sequence:032x}")
    }

    fn seed_board_posts(
        connection: &mut Connection,
        author: CharacterId,
        board: BoardKey,
        first_sequence: u128,
        count: u32,
        subject: &str,
        body: &str,
    ) {
        let created_at = 100;
        let transaction = connection.transaction().unwrap();
        for offset in 0..count {
            transaction
                .execute(
                    "INSERT INTO social_board_posts
                         (public_id, board_key, author_character_id, author_name,
                          subject, body, created_at, expires_at)
                     VALUES (?1, ?2, ?3, 'Seed Author', ?4, ?5, ?6, ?7)",
                    params![
                        board_id(first_sequence + u128::from(offset)),
                        board.as_str(),
                        author.0,
                        subject,
                        body,
                        created_at,
                        created_at + BOARD_RETENTION_SECONDS
                    ],
                )
                .unwrap();
        }
        transaction.commit().unwrap();
    }

    #[test]
    fn board_publish_list_read_delete_and_quota_are_account_bounded() {
        let mut connection = Connection::open_in_memory().unwrap();
        initialise(&mut connection).unwrap();
        let (author, reader, _) = mail_fixture(&connection);
        let entry = publish_board_post(
            &mut connection,
            author,
            BoardKey::General,
            &board_id(1),
            "Lantern meeting",
            "Meet beneath the arch.",
            1_000,
        )
        .unwrap();
        assert!(entry.summary.can_delete);
        assert!(!entry.summary.can_report);
        assert_eq!(entry.summary.expires_at, 1_000 + BOARD_RETENTION_SECONDS);

        let reader_page =
            list_board_posts(&connection, reader, BoardKey::General, 1, 1_001).unwrap();
        assert_eq!(reader_page.total_posts, 1);
        assert!(reader_page.posts[0].can_report);
        assert!(!reader_page.posts[0].can_delete);
        assert!(
            list_board_posts(&connection, reader, BoardKey::General, 100, 1_001)
                .unwrap()
                .posts
                .is_empty()
        );
        assert_eq!(
            read_board_post(&connection, reader, &board_id(1), 1_001)
                .unwrap()
                .body,
            "Meet beneath the arch."
        );
        assert!(matches!(
            delete_board_post(&mut connection, reader, &board_id(1), 1_002),
            Err(StoreError::BoardPostUnavailable)
        ));
        delete_board_post(&mut connection, author, &board_id(1), 1_002).unwrap();

        for sequence in 0_u128..u128::from(BOARD_ACCOUNT_CAPACITY) {
            connection
                .execute(
                    "INSERT INTO social_board_posts
                         (public_id, board_key, author_character_id, author_name,
                          subject, body, created_at, expires_at)
                     VALUES (?1, 'general', ?2, 'Aster', 'Cap', 'Body', ?3, ?4)",
                    params![
                        board_id(100 + sequence),
                        author.0,
                        1_003,
                        1_003 + BOARD_RETENTION_SECONDS
                    ],
                )
                .unwrap();
        }
        assert!(matches!(
            publish_board_post(
                &mut connection,
                author,
                BoardKey::Newbie,
                &board_id(1_000),
                "Account full",
                "A valid attempt still consumes quota.",
                1_003,
            ),
            Err(StoreError::BoardCapacity)
        ));
        assert_eq!(
            connection
                .query_row(
                    "SELECT hour_count FROM social_board_usage WHERE account_id = 101",
                    [],
                    |row| row.get::<_, i64>(0),
                )
                .unwrap(),
            2
        );
        connection
            .execute("DELETE FROM social_board_posts", [])
            .unwrap();

        connection
            .execute(
                "UPDATE social_board_usage
                 SET hour_count = 5, day_count = 5, updated_at = 1002
                 WHERE account_id = 101",
                [],
            )
            .unwrap();
        assert!(matches!(
            publish_board_post(
                &mut connection,
                author,
                BoardKey::Newbie,
                &board_id(2),
                "Rate limited",
                "This attempt is rejected.",
                1_003,
            ),
            Err(StoreError::BoardRateLimited)
        ));
    }

    #[test]
    fn board_reporting_copies_evidence_once_and_purge_is_bounded() {
        let mut connection = Connection::open_in_memory().unwrap();
        initialise(&mut connection).unwrap();
        let (author, reader, same_account_alt) = mail_fixture(&connection);
        publish_board_post(
            &mut connection,
            author,
            BoardKey::Newbie,
            &board_id(10),
            "Reported subject",
            "Evidence body",
            5_000,
        )
        .unwrap();
        assert!(matches!(
            report_board_post(
                &mut connection,
                same_account_alt,
                &board_id(10),
                "Same account",
                5_001,
            ),
            Err(StoreError::BoardPostUnavailable)
        ));
        let report_id = report_board_post(
            &mut connection,
            reader,
            &board_id(10),
            "Public abuse",
            5_002,
        )
        .unwrap();
        assert_eq!(
            connection
                .query_row(
                    "SELECT report_kind FROM social_reports WHERE id = ?1",
                    [report_id],
                    |row| row.get::<_, String>(0),
                )
                .unwrap(),
            "board"
        );
        delete_board_post(&mut connection, author, &board_id(10), 5_003).unwrap();
        assert_eq!(
            connection
                .query_row(
                    "SELECT body FROM social_board_report_evidence WHERE report_id = ?1",
                    [report_id],
                    |row| row.get::<_, String>(0),
                )
                .unwrap(),
            "Evidence body"
        );

        for sequence in 0_u128..300 {
            let created_at = 10_000 + i64::try_from(sequence).unwrap();
            connection
                .execute(
                    "INSERT INTO social_board_posts
                         (public_id, board_key, author_character_id, author_name,
                          subject, body, created_at, expires_at)
                     VALUES (?1, 'general', ?2, 'Aster', 'Old', 'Expired', ?3, ?4)",
                    params![
                        board_id(1_000 + sequence),
                        author.0,
                        created_at,
                        created_at + BOARD_RETENTION_SECONDS
                    ],
                )
                .unwrap();
        }
        let expiry = 10_299 + BOARD_RETENTION_SECONDS;
        assert!(
            list_board_posts(&connection, reader, BoardKey::General, 1, expiry)
                .unwrap()
                .posts
                .is_empty()
        );
        assert_eq!(purge_expired_boards(&mut connection, expiry).unwrap(), 256);
        assert_eq!(purge_expired_boards(&mut connection, expiry).unwrap(), 45);
        assert_eq!(purge_expired_boards(&mut connection, expiry).unwrap(), 0);
    }

    #[test]
    fn board_count_caps_accept_the_exact_board_and_global_boundaries() {
        let mut connection = Connection::open_in_memory().unwrap();
        initialise(&mut connection).unwrap();
        let (publisher, seeded_author, _) = mail_fixture(&connection);
        let now = 20_000;

        seed_board_posts(
            &mut connection,
            seeded_author,
            BoardKey::General,
            10_000,
            BOARD_CAPACITY_PER_BOARD - 1,
            "S",
            "B",
        );
        publish_board_post(
            &mut connection,
            publisher,
            BoardKey::General,
            &board_id(11_000),
            "S",
            "B",
            now,
        )
        .unwrap();
        assert_eq!(
            connection
                .query_row(
                    "SELECT COUNT(*) FROM social_board_posts WHERE board_key = 'general'",
                    [],
                    |row| row.get::<_, i64>(0),
                )
                .unwrap(),
            i64::from(BOARD_CAPACITY_PER_BOARD)
        );
        assert!(matches!(
            publish_board_post(
                &mut connection,
                publisher,
                BoardKey::General,
                &board_id(11_001),
                "S",
                "B",
                now,
            ),
            Err(StoreError::BoardCapacity)
        ));
        assert_eq!(
            connection
                .query_row("SELECT COUNT(*) FROM social_board_posts", [], |row| {
                    row.get::<_, i64>(0)
                })
                .unwrap(),
            i64::from(BOARD_CAPACITY_PER_BOARD)
        );
        assert_eq!(
            connection
                .query_row(
                    "SELECT hour_count FROM social_board_usage WHERE account_id = 101",
                    [],
                    |row| row.get::<_, i64>(0),
                )
                .unwrap(),
            2,
            "a valid capacity rejection consumes exactly one quota unit"
        );

        connection
            .execute_batch("DELETE FROM social_board_posts; DELETE FROM social_board_usage;")
            .unwrap();
        seed_board_posts(
            &mut connection,
            seeded_author,
            BoardKey::General,
            20_000,
            BOARD_CAPACITY_PER_BOARD - 1,
            "S",
            "B",
        );
        seed_board_posts(
            &mut connection,
            seeded_author,
            BoardKey::Newbie,
            21_000,
            BOARD_CAPACITY_PER_BOARD,
            "S",
            "B",
        );
        publish_board_post(
            &mut connection,
            publisher,
            BoardKey::General,
            &board_id(22_000),
            "S",
            "B",
            now,
        )
        .unwrap();
        assert_eq!(
            connection
                .query_row("SELECT COUNT(*) FROM social_board_posts", [], |row| {
                    row.get::<_, i64>(0)
                })
                .unwrap(),
            BOARD_GLOBAL_CAPACITY
        );
        assert!(matches!(
            publish_board_post(
                &mut connection,
                publisher,
                BoardKey::Newbie,
                &board_id(22_001),
                "S",
                "B",
                now,
            ),
            Err(StoreError::BoardCapacity)
        ));
        assert_eq!(
            connection
                .query_row("SELECT COUNT(*) FROM social_board_posts", [], |row| {
                    row.get::<_, i64>(0)
                })
                .unwrap(),
            BOARD_GLOBAL_CAPACITY
        );
    }

    #[test]
    fn board_byte_caps_accept_exact_boundaries_and_reject_one_more_post() {
        let mut connection = Connection::open_in_memory().unwrap();
        initialise(&mut connection).unwrap();
        let (publisher, seeded_author, _) = mail_fixture(&connection);
        let now = 30_000;
        let maximum_subject = "S".repeat(tiamud_core::MAX_BOARD_SUBJECT_BYTES);
        let maximum_body = "B".repeat(tiamud_core::MAX_BOARD_BODY_BYTES);
        let remainder_body = "R".repeat(4_093);

        for (board, first_sequence) in [
            (BoardKey::General, 30_000_u128),
            (BoardKey::Newbie, 40_000_u128),
        ] {
            seed_board_posts(
                &mut connection,
                seeded_author,
                board,
                first_sequence,
                992,
                &maximum_subject,
                &maximum_body,
            );
            seed_board_posts(
                &mut connection,
                seeded_author,
                board,
                first_sequence + 992,
                1,
                "S",
                &remainder_body,
            );
        }

        publish_board_post(
            &mut connection,
            publisher,
            BoardKey::General,
            &board_id(50_000),
            "S",
            "B",
            now,
        )
        .unwrap();
        publish_board_post(
            &mut connection,
            publisher,
            BoardKey::Newbie,
            &board_id(50_001),
            "S",
            "B",
            now,
        )
        .unwrap();
        let (general_bytes, newbie_bytes, global_bytes) = connection
            .query_row(
                "SELECT
                     SUM(CASE WHEN board_key = 'general'
                              THEN length(CAST(subject AS BLOB)) + length(CAST(body AS BLOB))
                              ELSE 0 END),
                     SUM(CASE WHEN board_key = 'newbie'
                              THEN length(CAST(subject AS BLOB)) + length(CAST(body AS BLOB))
                              ELSE 0 END),
                     SUM(length(CAST(subject AS BLOB)) + length(CAST(body AS BLOB)))
                 FROM social_board_posts",
                [],
                |row| {
                    Ok((
                        row.get::<_, i64>(0)?,
                        row.get::<_, i64>(1)?,
                        row.get::<_, i64>(2)?,
                    ))
                },
            )
            .unwrap();
        assert_eq!(general_bytes, BOARD_BYTE_CAPACITY_PER_BOARD);
        assert_eq!(newbie_bytes, BOARD_BYTE_CAPACITY_PER_BOARD);
        assert_eq!(global_bytes, BOARD_GLOBAL_BYTE_CAPACITY);
        let count_before = connection
            .query_row("SELECT COUNT(*) FROM social_board_posts", [], |row| {
                row.get::<_, i64>(0)
            })
            .unwrap();
        assert!(matches!(
            publish_board_post(
                &mut connection,
                publisher,
                BoardKey::General,
                &board_id(50_002),
                "S",
                "B",
                now,
            ),
            Err(StoreError::BoardCapacity)
        ));
        assert_eq!(
            connection
                .query_row("SELECT COUNT(*) FROM social_board_posts", [], |row| {
                    row.get::<_, i64>(0)
                })
                .unwrap(),
            count_before
        );
        assert_eq!(
            connection
                .query_row(
                    "SELECT hour_count FROM social_board_usage WHERE account_id = 101",
                    [],
                    |row| row.get::<_, i64>(0),
                )
                .unwrap(),
            3
        );
    }

    #[test]
    fn board_daily_quota_stops_the_twenty_first_attempt_without_mutation() {
        let mut connection = Connection::open_in_memory().unwrap();
        initialise(&mut connection).unwrap();
        let (publisher, _, _) = mail_fixture(&connection);
        let day_started_at = 40_000;
        for sequence in 0_u128..20 {
            let hour = i64::try_from(sequence / 5).unwrap();
            publish_board_post(
                &mut connection,
                publisher,
                BoardKey::General,
                &board_id(60_000 + sequence),
                "S",
                "B",
                day_started_at + hour * MAIL_HOUR_SECONDS,
            )
            .unwrap();
        }
        let rejected_at = day_started_at + 4 * MAIL_HOUR_SECONDS;
        assert!(matches!(
            publish_board_post(
                &mut connection,
                publisher,
                BoardKey::General,
                &board_id(60_020),
                "S",
                "B",
                rejected_at,
            ),
            Err(StoreError::BoardRateLimited)
        ));
        assert_eq!(
            connection
                .query_row("SELECT COUNT(*) FROM social_board_posts", [], |row| {
                    row.get::<_, i64>(0)
                })
                .unwrap(),
            BOARD_DAILY_CAPACITY
        );
        assert_eq!(
            connection
                .query_row(
                    "SELECT day_count FROM social_board_usage WHERE account_id = 101",
                    [],
                    |row| row.get::<_, i64>(0),
                )
                .unwrap(),
            BOARD_DAILY_CAPACITY,
            "a rate-limited attempt must not increment durable usage"
        );
        assert_eq!(
            connection
                .query_row(
                    "SELECT COUNT(*) FROM social_board_posts WHERE public_id = ?1",
                    [board_id(60_020)],
                    |row| row.get::<_, i64>(0),
                )
                .unwrap(),
            0
        );
    }

    #[test]
    fn board_report_caps_reject_without_linking_or_creating_evidence() {
        let mut connection = Connection::open_in_memory().unwrap();
        initialise(&mut connection).unwrap();
        let (author, reporter, _) = mail_fixture(&connection);
        let now = 50_000;
        seed_board_posts(
            &mut connection,
            author,
            BoardKey::General,
            70_000,
            9,
            "Reported",
            "Evidence",
        );
        for sequence in 0_u128..8 {
            report_board_post(
                &mut connection,
                reporter,
                &board_id(70_000 + sequence),
                "Open report cap",
                now + 1,
            )
            .unwrap();
        }
        assert!(matches!(
            report_board_post(
                &mut connection,
                reporter,
                &board_id(70_008),
                "One report too many",
                now + 1,
            ),
            Err(StoreError::BoardReportLimit)
        ));
        assert_eq!(
            connection
                .query_row(
                    "SELECT COUNT(*) FROM social_reports
                     WHERE report_kind = 'board' AND status = 'open'",
                    [],
                    |row| row.get::<_, i64>(0),
                )
                .unwrap(),
            BOARD_OPEN_REPORT_CAPACITY_PER_ACCOUNT
        );
        assert_eq!(
            connection
                .query_row(
                    "SELECT COUNT(*) FROM social_board_report_evidence",
                    [],
                    |row| row.get::<_, i64>(0),
                )
                .unwrap(),
            BOARD_OPEN_REPORT_CAPACITY_PER_ACCOUNT
        );
        assert_eq!(
            connection
                .query_row(
                    "SELECT COUNT(*) FROM social_board_posts
                     WHERE public_id = ?1 AND report_id IS NULL",
                    [board_id(70_008)],
                    |row| row.get::<_, i64>(0),
                )
                .unwrap(),
            1
        );

        let mut full_evidence = Connection::open_in_memory().unwrap();
        initialise(&mut full_evidence).unwrap();
        let (author, reporter, _) = mail_fixture(&full_evidence);
        full_evidence
            .execute_batch(
                "WITH RECURSIVE sequence(value) AS (
                     VALUES(1) UNION ALL SELECT value + 1 FROM sequence WHERE value < 10000
                 )
                 INSERT INTO social_reports
                     (reporter_character_id, target_character_id, report_kind, reason, created_at)
                 SELECT 1001, 1002, 'board', 'seed evidence capacity', value
                 FROM sequence;
                 INSERT INTO social_board_report_evidence
                     (report_id, board_key, post_public_id, author_name,
                      subject, body, posted_at, created_at, expires_at)
                 SELECT id, 'general', 'bffffffffffffffffffffffffffffffff', 'Seed Author',
                        'Seed', 'Evidence', 0, 100, 7776100
                 FROM social_reports WHERE reason = 'seed evidence capacity';",
            )
            .unwrap();
        seed_board_posts(
            &mut full_evidence,
            author,
            BoardKey::General,
            80_000,
            1,
            "Reported",
            "Evidence",
        );
        assert!(matches!(
            report_board_post(
                &mut full_evidence,
                reporter,
                &board_id(80_000),
                "Global evidence full",
                1_001,
            ),
            Err(StoreError::BoardEvidenceLimit)
        ));
        assert_eq!(
            full_evidence
                .query_row(
                    "SELECT COUNT(*) FROM social_board_report_evidence",
                    [],
                    |row| row.get::<_, i64>(0),
                )
                .unwrap(),
            BOARD_REPORT_EVIDENCE_GLOBAL_CAPACITY
        );
        assert_eq!(
            full_evidence
                .query_row(
                    "SELECT COUNT(*) FROM social_reports WHERE report_kind = 'board'",
                    [],
                    |row| row.get::<_, i64>(0),
                )
                .unwrap(),
            BOARD_REPORT_EVIDENCE_GLOBAL_CAPACITY
        );
        assert_eq!(
            full_evidence
                .query_row(
                    "SELECT COUNT(*) FROM social_board_posts
                     WHERE public_id = ?1 AND report_id IS NULL",
                    [board_id(80_000)],
                    |row| row.get::<_, i64>(0),
                )
                .unwrap(),
            1
        );
    }

    #[test]
    fn mail_send_list_read_delete_and_private_suppression_are_atomic() {
        let mut connection = Connection::open_in_memory().unwrap();
        initialise(&mut connection).unwrap();
        let (sender, recipient, alt) = mail_fixture(&connection);

        assert_eq!(
            send_mail(
                &mut connection,
                sender,
                "Bryn",
                &mail_id(1),
                "Meeting",
                "Meet under the lantern.",
                1_000,
            )
            .unwrap(),
            MailSendResult::Stored
        );
        let page = mailbox_page(&connection, recipient, 1, 1_001).unwrap();
        assert_eq!(page.total_pages, 1);
        assert_eq!(page.total_messages, 1);
        assert_eq!(page.unread_count, 1);
        assert_eq!(page.capacity, MAILBOX_CAPACITY);
        assert_eq!(page.messages[0].public_id, mail_id(1));
        assert!(
            mailbox_page(&connection, sender, 1, 1_001)
                .unwrap()
                .messages
                .is_empty()
        );
        assert!(matches!(
            read_mail(&mut connection, alt, &mail_id(1), 1_001),
            Err(StoreError::MailNotFound)
        ));
        let read = read_mail(&mut connection, recipient, &mail_id(1), 1_001).unwrap();
        assert!(read.summary.read);
        assert_eq!(read.body, "Meet under the lantern.");
        assert_eq!(
            mailbox_page(&connection, recipient, 1, 1_002)
                .unwrap()
                .unread_count,
            0
        );

        assert_eq!(
            send_mail(
                &mut connection,
                sender,
                "Nobody",
                &mail_id(2),
                "Unknown",
                "Still consumes quota.",
                1_003,
            )
            .unwrap(),
            MailSendResult::Unavailable
        );
        assert_eq!(
            send_mail(
                &mut connection,
                sender,
                "Aster",
                &mail_id(3),
                "Self",
                "Still consumes quota.",
                1_004,
            )
            .unwrap(),
            MailSendResult::Unavailable
        );
        connection
            .execute(
                "INSERT INTO social_blocks
                     (owner_character_id, target_character_id, created_at)
                 VALUES (?1, ?2, 1005)",
                params![recipient.0, sender.0],
            )
            .unwrap();
        assert_eq!(
            send_mail(
                &mut connection,
                sender,
                "Bryn",
                &mail_id(4),
                "Blocked",
                "Suppressed without disclosure.",
                1_005,
            )
            .unwrap(),
            MailSendResult::Suppressed
        );
        assert_eq!(
            connection
                .query_row(
                    "SELECT hour_count FROM social_mail_usage WHERE account_id = 101",
                    [],
                    |row| row.get::<_, i64>(0),
                )
                .unwrap(),
            4
        );
        connection
            .execute(
                "DELETE FROM social_blocks
                 WHERE owner_character_id = ?1 AND target_character_id = ?2",
                params![recipient.0, sender.0],
            )
            .unwrap();
        assert_eq!(
            send_mail(
                &mut connection,
                sender,
                "AsterAlt",
                &mail_id(5),
                "For my alt",
                "Same-account alts are allowed.",
                1_006,
            )
            .unwrap(),
            MailSendResult::Stored
        );
        assert!(matches!(
            delete_mail(&mut connection, alt, &mail_id(1), 1_007),
            Err(StoreError::MailNotFound)
        ));
        delete_mail(&mut connection, recipient, &mail_id(1), 1_007).unwrap();
        assert!(
            mailbox_page(&connection, recipient, 1, 1_008)
                .unwrap()
                .messages
                .is_empty()
        );
    }

    #[test]
    fn mail_quota_validation_expiry_and_purge_remain_bounded() {
        let mut connection = Connection::open_in_memory().unwrap();
        initialise(&mut connection).unwrap();
        let (sender, recipient, _) = mail_fixture(&connection);
        connection
            .execute(
                "INSERT INTO social_mail_usage
                     (account_id, hour_started_at, hour_count,
                      day_started_at, day_count, updated_at)
                 VALUES (101, 1000, 20, 1000, 20, 1000)",
                [],
            )
            .unwrap();
        assert!(matches!(
            send_mail(
                &mut connection,
                sender,
                "Nobody",
                &mail_id(10),
                "Rate limited",
                "No target lookup changes the result.",
                1_001,
            ),
            Err(StoreError::MailRateLimited)
        ));
        assert!(matches!(
            send_mail(
                &mut connection,
                sender,
                "Bryn",
                "mNOT-HEX",
                "Invalid",
                "Does not consume quota.",
                1_002,
            ),
            Err(StoreError::InvalidMailInput)
        ));
        assert!(matches!(
            send_mail(
                &mut connection,
                sender,
                "Bryn",
                &mail_id(11),
                " padded subject",
                "No silent trimming.",
                1_002,
            ),
            Err(StoreError::InvalidMailInput)
        ));
        assert_eq!(
            connection
                .query_row(
                    "SELECT hour_count FROM social_mail_usage WHERE account_id = 101",
                    [],
                    |row| row.get::<_, i64>(0),
                )
                .unwrap(),
            20
        );

        connection
            .execute("DELETE FROM social_mail_usage WHERE account_id = 101", [])
            .unwrap();
        assert_eq!(
            send_mail(
                &mut connection,
                sender,
                "bryn",
                &mail_id(13),
                "Exact name",
                "Mail targets preserve the same case rules in both backends.",
                1_050,
            )
            .unwrap(),
            MailSendResult::Unavailable
        );
        assert_eq!(
            mailbox_page(&connection, recipient, 1, 1_050)
                .unwrap()
                .total_messages,
            0
        );
        connection
            .execute(
                "UPDATE characters SET name = 'A' || char(8238) || 'B' WHERE id = ?1",
                [sender.0],
            )
            .unwrap();
        assert_eq!(
            send_mail(
                &mut connection,
                sender,
                "Bryn",
                &mail_id(12),
                "Legacy sender",
                "A legacy bidi identity cannot poison the recipient inbox.",
                1_100,
            )
            .unwrap(),
            MailSendResult::Suppressed
        );
        assert_eq!(
            mailbox_page(&connection, recipient, 1, 1_100)
                .unwrap()
                .total_messages,
            0
        );
        for sequence in 0_u128..300 {
            let created_at = 2_000_i64 + i64::try_from(sequence).unwrap();
            connection
                .execute(
                    "INSERT INTO social_mail_messages
                         (public_id, sender_character_id, sender_name,
                          recipient_character_id, subject, body, created_at, expires_at)
                     VALUES (?1, ?2, 'Aster', ?3, 'Old', 'Expired', ?4, ?5)",
                    params![
                        mail_id(1_000 + sequence),
                        sender.0,
                        recipient.0,
                        created_at,
                        created_at + MAIL_RETENTION_SECONDS
                    ],
                )
                .unwrap();
        }
        let expiry = 2_299 + MAIL_RETENTION_SECONDS;
        assert!(
            mailbox_page(&connection, recipient, 1, expiry)
                .unwrap()
                .messages
                .is_empty()
        );
        assert_eq!(purge_expired_mail(&mut connection, expiry).unwrap(), 256);
        assert_eq!(purge_expired_mail(&mut connection, expiry).unwrap(), 44);
        assert_eq!(purge_expired_mail(&mut connection, expiry).unwrap(), 0);
    }

    #[test]
    fn mail_count_byte_and_page_caps_suppress_without_overfilling() {
        let mut connection = Connection::open_in_memory().unwrap();
        initialise(&mut connection).unwrap();
        let (sender, recipient, _) = mail_fixture(&connection);
        for sequence in 0_u128..u128::from(MAILBOX_CAPACITY) {
            connection
                .execute(
                    "INSERT INTO social_mail_messages
                         (public_id, sender_character_id, sender_name,
                          recipient_character_id, subject, body, created_at, expires_at)
                     VALUES (?1, ?2, 'Aster', ?3, 'S', 'B', ?4, ?5)",
                    params![
                        mail_id(2_000 + sequence),
                        sender.0,
                        recipient.0,
                        10_000 + i64::try_from(sequence).unwrap(),
                        10_000 + i64::try_from(sequence).unwrap() + MAIL_RETENTION_SECONDS
                    ],
                )
                .unwrap();
        }
        let page = mailbox_page(&connection, recipient, MAIL_MAX_PAGES, 11_000).unwrap();
        assert_eq!(page.total_messages, MAILBOX_CAPACITY);
        assert_eq!(page.total_pages, MAIL_MAX_PAGES);
        assert_eq!(page.messages.len(), 4);
        assert_eq!(
            send_mail(
                &mut connection,
                sender,
                "Bryn",
                &mail_id(3_000),
                "Full",
                "The full mailbox suppresses this message.",
                11_001,
            )
            .unwrap(),
            MailSendResult::Suppressed
        );
        assert_eq!(
            mailbox_page(&connection, recipient, 1, 11_002)
                .unwrap()
                .total_messages,
            MAILBOX_CAPACITY
        );

        connection
            .execute("DELETE FROM social_mail_messages", [])
            .unwrap();
        let large_subject = "s".repeat(MAX_MAIL_SUBJECT_BYTES);
        let large_body = "b".repeat(MAX_MAIL_BODY_BYTES);
        for sequence in 0_u128..60 {
            connection
                .execute(
                    "INSERT INTO social_mail_messages
                         (public_id, sender_character_id, sender_name,
                          recipient_character_id, subject, body, created_at, expires_at)
                     VALUES (?1, ?2, 'Aster', ?3, ?4, ?5, ?6, ?7)",
                    params![
                        mail_id(4_000 + sequence),
                        sender.0,
                        recipient.0,
                        large_subject,
                        large_body,
                        20_000 + i64::try_from(sequence).unwrap(),
                        20_000 + i64::try_from(sequence).unwrap() + MAIL_RETENTION_SECONDS
                    ],
                )
                .unwrap();
        }
        assert_eq!(
            send_mail(
                &mut connection,
                sender,
                "Bryn",
                &mail_id(5_000),
                &large_subject,
                &large_body,
                21_000,
            )
            .unwrap(),
            MailSendResult::Suppressed
        );
        assert_eq!(
            mailbox_page(&connection, recipient, 1, 21_001)
                .unwrap()
                .total_messages,
            60
        );
    }

    #[test]
    fn mail_report_copies_bounded_evidence_once_and_survives_hard_delete() {
        let mut connection = Connection::open_in_memory().unwrap();
        initialise(&mut connection).unwrap();
        let (sender, recipient, _) = mail_fixture(&connection);
        assert_eq!(
            send_mail(
                &mut connection,
                sender,
                "Bryn",
                &mail_id(20),
                "Abuse evidence",
                "A body retained only because the recipient reports it.",
                5_000,
            )
            .unwrap(),
            MailSendResult::Stored
        );
        let report_id = report_mail(
            &mut connection,
            recipient,
            &mail_id(20),
            "Please review this message",
            5_001,
        )
        .unwrap();
        assert!(report_id > 0);
        assert_eq!(
            connection
                .query_row(
                    "SELECT report_kind FROM social_reports WHERE id = ?1",
                    [report_id],
                    |row| row.get::<_, String>(0),
                )
                .unwrap(),
            "mail"
        );
        assert!(matches!(
            report_mail(&mut connection, recipient, &mail_id(20), "Duplicate", 5_002,),
            Err(StoreError::MailAlreadyReported)
        ));
        assert_eq!(
            connection
                .query_row("SELECT COUNT(*) FROM social_reports", [], |row| {
                    row.get::<_, i64>(0)
                })
                .unwrap(),
            1,
            "a failed duplicate report rolls back every report row"
        );
        let evidence: (String, String, String) = connection
            .query_row(
                "SELECT sender_name, subject, body
                 FROM social_mail_report_evidence WHERE report_id = ?1",
                [report_id],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .unwrap();
        assert_eq!(evidence.0, "Aster");
        assert_eq!(evidence.1, "Abuse evidence");
        assert_eq!(
            evidence.2,
            "A body retained only because the recipient reports it."
        );
        let audit_details: String = connection
            .query_row(
                "SELECT details_json FROM moderation_audit WHERE action = 'mail_report'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert!(!audit_details.contains("body retained"));

        delete_mail(&mut connection, recipient, &mail_id(20), 5_003).unwrap();
        assert_eq!(
            connection
                .query_row(
                    "SELECT COUNT(*) FROM social_mail_report_evidence WHERE report_id = ?1",
                    [report_id],
                    |row| row.get::<_, i64>(0),
                )
                .unwrap(),
            1
        );
        assert_eq!(
            purge_expired_mail(
                &mut connection,
                5_001 + MAIL_REPORT_EVIDENCE_RETENTION_SECONDS,
            )
            .unwrap(),
            1
        );
    }

    #[test]
    fn open_mail_report_cap_survives_evidence_expiry_and_purge() {
        let mut connection = Connection::open_in_memory().unwrap();
        initialise(&mut connection).unwrap();
        let (sender, recipient, _) = mail_fixture(&connection);
        for sequence in 0..MAIL_OPEN_REPORT_CAPACITY_PER_ACCOUNT {
            let public_id = mail_id(100 + u128::try_from(sequence).unwrap());
            let sent_at = 30_000 + sequence * 2;
            assert_eq!(
                send_mail(
                    &mut connection,
                    sender,
                    "Bryn",
                    &public_id,
                    "Capped report",
                    "Evidence expires without erasing the durable report kind.",
                    sent_at,
                )
                .unwrap(),
                MailSendResult::Stored
            );
            report_mail(
                &mut connection,
                recipient,
                &public_id,
                "Keep this open for the cap regression",
                sent_at + 1,
            )
            .unwrap();
        }

        let purge_at = 31_000 + MAIL_REPORT_EVIDENCE_RETENTION_SECONDS;
        assert_eq!(
            purge_expired_mail(&mut connection, purge_at).unwrap(),
            u64::try_from(MAIL_OPEN_REPORT_CAPACITY_PER_ACCOUNT * 2).unwrap()
        );
        assert_eq!(
            connection
                .query_row(
                    "SELECT COUNT(*) FROM social_mail_report_evidence",
                    [],
                    |row| row.get::<_, i64>(0),
                )
                .unwrap(),
            0,
            "expired bodies must not remain solely to enforce the report cap"
        );
        assert_eq!(
            connection
                .query_row(
                    "SELECT COUNT(*) FROM social_reports
                     WHERE report_kind = 'mail' AND status = 'open'",
                    [],
                    |row| row.get::<_, i64>(0),
                )
                .unwrap(),
            MAIL_OPEN_REPORT_CAPACITY_PER_ACCOUNT
        );

        let ninth_id = mail_id(200);
        assert_eq!(
            send_mail(
                &mut connection,
                sender,
                "Bryn",
                &ninth_id,
                "Ninth report",
                "This message can arrive, but its report must respect the open cap.",
                purge_at + 1,
            )
            .unwrap(),
            MailSendResult::Stored
        );
        assert!(matches!(
            report_mail(
                &mut connection,
                recipient,
                &ninth_id,
                "The durable open cap still applies",
                purge_at + 2,
            ),
            Err(StoreError::MailReportLimit)
        ));
    }

    #[test]
    fn action_taken_resolution_removes_the_live_reported_mail() {
        let mut connection = Connection::open_in_memory().unwrap();
        initialise(&mut connection).unwrap();
        let admin = finish_register_account(&mut connection, "Mail Admin", "hash", 7_000).unwrap();
        let sender_account =
            finish_register_account(&mut connection, "Mail Author", "hash", 7_001).unwrap();
        let recipient_account =
            finish_register_account(&mut connection, "Mail Reader", "hash", 7_002).unwrap();
        let admin_character =
            load_or_create_character(&connection, admin.account.id, "Implementor", "room:start")
                .unwrap();
        let sender = load_or_create_character(
            &connection,
            sender_account.account.id,
            "Aster",
            "room:start",
        )
        .unwrap();
        let recipient = load_or_create_character(
            &connection,
            recipient_account.account.id,
            "Bryn",
            "room:start",
        )
        .unwrap();
        assert_eq!(
            send_mail(
                &mut connection,
                sender.id,
                "Bryn",
                &mail_id(30),
                "Reported",
                "Remove this message when action is taken.",
                7_010,
            )
            .unwrap(),
            MailSendResult::Stored
        );
        let report_id = report_mail(
            &mut connection,
            recipient.id,
            &mail_id(30),
            "Requires moderator action",
            7_011,
        )
        .unwrap();
        let reports = staff_reports(
            &mut connection,
            StaffAuditRecord {
                actor_account_id: admin.account.id,
                target_account_id: None,
                capability: StaffCapability::ReportsView,
                action: "reports.list".into(),
                reason: "review mail evidence".into(),
                details: serde_json::json!({}),
            },
            7_012,
        )
        .unwrap();
        let reported = reports
            .iter()
            .find(|report| report.id == report_id)
            .unwrap();
        assert_eq!(reported.mail_evidence.as_ref().unwrap().subject, "Reported");
        resolve_staff_report(
            &mut connection,
            report_id,
            StaffReportResolution::ActionTaken,
            "mail removed after review",
            StaffAuditRecord {
                actor_account_id: admin.account.id,
                target_account_id: None,
                capability: StaffCapability::ReportsResolve,
                action: "reports.resolve".into(),
                reason: "review completed".into(),
                details: serde_json::json!({"report_id": report_id}),
            },
            7_013,
        )
        .unwrap();
        assert!(matches!(
            read_mail(&mut connection, recipient.id, &mail_id(30), 7_014),
            Err(StoreError::MailNotFound)
        ));
        assert_eq!(
            connection
                .query_row(
                    "SELECT COUNT(*) FROM social_mail_report_evidence WHERE report_id = ?1",
                    [report_id],
                    |row| row.get::<_, i64>(0),
                )
                .unwrap(),
            1,
            "resolution removes live mail but keeps bounded report evidence"
        );

        publish_board_post(
            &mut connection,
            sender.id,
            BoardKey::General,
            &board_id(31),
            "Reported board entry",
            "Remove this post when action is taken.",
            7_020,
        )
        .unwrap();
        let board_report_id = report_board_post(
            &mut connection,
            recipient.id,
            &board_id(31),
            "Requires moderator action",
            7_021,
        )
        .unwrap();
        let reports = staff_reports(
            &mut connection,
            StaffAuditRecord {
                actor_account_id: admin.account.id,
                target_account_id: None,
                capability: StaffCapability::ReportsView,
                action: "reports.list".into(),
                reason: "review board evidence".into(),
                details: serde_json::json!({}),
            },
            7_022,
        )
        .unwrap();
        assert_eq!(
            reports
                .iter()
                .find(|report| report.id == board_report_id)
                .unwrap()
                .board_evidence
                .as_ref()
                .unwrap()
                .subject,
            "Reported board entry"
        );
        resolve_staff_report(
            &mut connection,
            board_report_id,
            StaffReportResolution::ActionTaken,
            "board post removed after review",
            StaffAuditRecord {
                actor_account_id: admin.account.id,
                target_account_id: None,
                capability: StaffCapability::ReportsResolve,
                action: "reports.resolve".into(),
                reason: "review completed".into(),
                details: serde_json::json!({"report_id": board_report_id}),
            },
            7_023,
        )
        .unwrap();
        assert!(matches!(
            read_board_post(&connection, recipient.id, &board_id(31), 7_024),
            Err(StoreError::BoardPostUnavailable)
        ));
        assert_eq!(
            connection
                .query_row(
                    "SELECT COUNT(*) FROM social_board_report_evidence WHERE report_id = ?1",
                    [board_report_id],
                    |row| row.get::<_, i64>(0),
                )
                .unwrap(),
            1,
            "resolution removes the live board post but keeps bounded evidence"
        );
        assert_eq!(
            staff_access(&connection, admin_character.id)
                .unwrap()
                .profile,
            Some(StaffProfileKey::Implementor)
        );
    }

    #[test]
    fn temporary_and_permanent_bans_have_distinct_expiry_semantics() {
        let mut connection = Connection::open_in_memory().unwrap();
        initialise(&mut connection).unwrap();
        let admin = register_account(
            &mut connection,
            "Admin",
            "correct horse battery staple",
            1_000,
        )
        .unwrap();
        let player = register_account(
            &mut connection,
            "Player",
            "another correct horse battery",
            1_000,
        )
        .unwrap();
        let admin_character =
            load_or_create_character(&connection, admin.account.id, "Aster", "room:start").unwrap();
        let player_character =
            load_or_create_character(&connection, player.account.id, "Bryn", "room:start").unwrap();

        moderate_character(
            &mut connection,
            admin_character.id,
            player_character.id,
            ModerationAction::Ban {
                expires_at: Some(1_060),
                reason: "temporary review".into(),
            },
            1_000,
        )
        .unwrap();
        assert!(account_is_banned(&connection, player.account.id, 1_059).unwrap());
        assert!(!account_is_banned(&connection, player.account.id, 1_060).unwrap());

        moderate_character(
            &mut connection,
            admin_character.id,
            player_character.id,
            ModerationAction::Ban {
                expires_at: None,
                reason: "permanent ban".into(),
            },
            2_000,
        )
        .unwrap();
        assert!(account_is_banned(&connection, player.account.id, i64::MAX).unwrap());
    }

    #[test]
    fn refuses_to_open_a_database_from_a_newer_schema() {
        let mut connection = Connection::open_in_memory().unwrap();
        let future = tiamud_persistence_sqlite::SCHEMA_VERSION + 1;
        connection
            .pragma_update(None, "user_version", future)
            .unwrap();

        assert!(matches!(
            initialise(&mut connection),
            Err(StoreError::UnsupportedSchemaVersion {
                found,
                supported: tiamud_persistence_sqlite::SCHEMA_VERSION
            }) if found == future
        ));
        assert_eq!(
            connection
                .query_row("PRAGMA user_version", [], |row| row.get::<_, i64>(0))
                .unwrap(),
            future
        );
    }
}
