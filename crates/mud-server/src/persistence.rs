use crate::{
    auth::{
        AuthenticatedSession, DEFAULT_SESSION_TTL, MAX_PASSWORD_BYTES, MIN_PASSWORD_BYTES,
        PasswordService, SessionToken, digest_token, validate_password,
    },
    operations::RuntimeMetrics,
};
use async_trait::async_trait;
use rusqlite::{Connection, OptionalExtension, params, params_from_iter, types::Type};
use sha2::{Digest, Sha256};
#[cfg(test)]
use std::sync::atomic::{AtomicBool, AtomicU8, Ordering};
use std::{
    collections::BTreeSet, future::Future, io, ops::ControlFlow, path::Path, sync::Arc, thread,
    time::Instant,
};
use thiserror::Error;
use tiamud_core::{
    AccountId, AccountRole, AccountSnapshot, AreaDynamicCheckpoint, AreaRuntimeResidence, BoardKey,
    CharacterGameplaySnapshot, CharacterId, CharacterSnapshot, DomainEvent,
    DurableCharacterSnapshot, MAX_ITEM_INSTANCES, SocialChannelName, StaffCapability,
    StaffProfileKey, StaffReportResolution, WorldRuntimeSnapshot, valid_board_public_id,
    valid_board_subject, valid_mail_public_id, valid_mail_subject, valid_mail_target,
    valid_mail_visible_text,
};
use tiamud_persistence_sqlite::{SchemaError, migrate};
#[cfg(test)]
use tokio::sync::Notify;
use tokio::{
    sync::{mpsc, oneshot},
    time::timeout,
};

#[cfg(test)]
use crate::auth::{hash_password, verify_password};

/// Backend-agnostic persistence policy shared with the PostgreSQL store.
pub(crate) mod common;
use common::*;
/// Backend-agnostic startup-ledger, runtime-snapshot and area-checkpoint
/// decoding and validation shared with the PostgreSQL store.
mod recovery;
pub(crate) use recovery::*;
/// The SQLite store: its owner thread, command handler and SQL.
mod sqlite;
pub use sqlite::SqliteCharacterStore;
/// Public types of the persisted story save slots (schema v16, ADR 0030).
mod story_slots;
pub use story_slots::*;
#[cfg(test)]
mod story_slot_tests;
#[cfg(test)]
mod tests;

/// Number of snapshot-covered domain events retained for diagnostics and
/// compatibility. Older covered events are compacted transactionally.
pub(super) const DOMAIN_EVENT_RETENTION: i64 = 10_000;
pub const MAX_SOCIAL_REASON_BYTES: usize = 1_024;
pub const MAX_REPORT_REASON_BYTES: usize = 2_048;
pub const MAX_MAIL_SUBJECT_BYTES: usize = 128;
pub const MAX_MAIL_BODY_BYTES: usize = 2_048;
pub const MAILBOX_CAPACITY: u32 = 64;
pub const MAIL_PAGE_SIZE: u32 = 10;
pub const MAIL_MAX_PAGES: u32 = 7;
pub const BOARD_PAGE_SIZE: u32 = 10;
pub const BOARD_MAX_PAGES: u32 = 100;
pub const BOARD_ACCOUNT_CAPACITY: u32 = 64;
pub const BOARD_CAPACITY_PER_BOARD: u32 = 1_000;
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
    #[error("invalid story save input")]
    InvalidStorySave,
    #[error("story save state exceeds the {maximum} byte limit")]
    StorySaveTooLarge { maximum: usize },
    #[error("the account's story save capacity is exhausted")]
    StorySaveCapacity,
    #[error("the stored story save failed its integrity check")]
    StorySaveCorrupt,
    #[error("the story save belongs to an incompatible story pack revision")]
    StorySavePackMismatch,
    #[error("this character store does not implement story save slots")]
    StorySaveUnsupported,
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

    /// Atomically insert or overwrite one story save slot (ADR 0030).
    ///
    /// Validates identity, slot, pack digest, format version, byte bounds and
    /// that the document's `format_version`/`pack_id` match the metadata,
    /// computes the SHA-256 of the exact bytes and enforces the per-account
    /// caps in the same transaction. Overwriting keeps `created_at`.
    async fn save_story_slot(
        &self,
        _write: StorySaveWrite,
    ) -> Result<StorySaveSummary, StoreError> {
        Err(StoreError::StorySaveUnsupported)
    }

    /// Load one slot. `Ok(None)` means the slot is empty. A row whose bytes,
    /// digest or metadata disagree yields `StorySaveCorrupt`; a row for a pack
    /// revision `pack` does not accept yields `StorySavePackMismatch`. The
    /// caller must still re-validate the state against the pack.
    async fn load_story_slot(
        &self,
        _account_id: AccountId,
        _story_id: &str,
        _slot: StorySlot,
        _pack: StoryPackCompatibility,
    ) -> Result<Option<StorySaveRecord>, StoreError> {
        Err(StoreError::StorySaveUnsupported)
    }

    /// List the account's slots for one story in slot order (at most ten),
    /// reading metadata only.
    async fn list_story_slots(
        &self,
        _account_id: AccountId,
        _story_id: &str,
    ) -> Result<Vec<StorySaveSummary>, StoreError> {
        Err(StoreError::StorySaveUnsupported)
    }

    /// Delete one slot; returns whether a row existed. Account deletion
    /// removes every slot through the `ON DELETE CASCADE` foreign key.
    async fn delete_story_slot(
        &self,
        _account_id: AccountId,
        _story_id: &str,
        _slot: StorySlot,
    ) -> Result<bool, StoreError> {
        Err(StoreError::StorySaveUnsupported)
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

    async fn save_story_slot(&self, write: StorySaveWrite) -> Result<StorySaveSummary, StoreError> {
        self.transaction(self.inner.save_story_slot(write)).await
    }

    async fn load_story_slot(
        &self,
        account_id: AccountId,
        story_id: &str,
        slot: StorySlot,
        pack: StoryPackCompatibility,
    ) -> Result<Option<StorySaveRecord>, StoreError> {
        self.inner
            .load_story_slot(account_id, story_id, slot, pack)
            .await
    }

    async fn list_story_slots(
        &self,
        account_id: AccountId,
        story_id: &str,
    ) -> Result<Vec<StorySaveSummary>, StoreError> {
        self.inner.list_story_slots(account_id, story_id).await
    }

    async fn delete_story_slot(
        &self,
        account_id: AccountId,
        story_id: &str,
        slot: StorySlot,
    ) -> Result<bool, StoreError> {
        self.transaction(self.inner.delete_story_slot(account_id, story_id, slot))
            .await
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
    SaveStorySlot {
        write: StorySaveWrite,
        reply: oneshot::Sender<Result<StorySaveSummary, StoreError>>,
    },
    LoadStorySlot {
        account_id: AccountId,
        story_id: String,
        slot: StorySlot,
        pack: StoryPackCompatibility,
        reply: oneshot::Sender<Result<Option<StorySaveRecord>, StoreError>>,
    },
    ListStorySlots {
        account_id: AccountId,
        story_id: String,
        reply: oneshot::Sender<Result<Vec<StorySaveSummary>, StoreError>>,
    },
    DeleteStorySlot {
        account_id: AccountId,
        story_id: String,
        slot: StorySlot,
        reply: oneshot::Sender<Result<bool, StoreError>>,
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
    fn save_story_slot(&mut self, write: StorySaveWrite) -> Result<StorySaveSummary, StoreError>;
    fn load_story_slot(
        &mut self,
        account_id: AccountId,
        story_id: String,
        slot: StorySlot,
        pack: StoryPackCompatibility,
    ) -> Result<Option<StorySaveRecord>, StoreError>;
    fn list_story_slots(
        &mut self,
        account_id: AccountId,
        story_id: String,
    ) -> Result<Vec<StorySaveSummary>, StoreError>;
    fn delete_story_slot(
        &mut self,
        account_id: AccountId,
        story_id: String,
        slot: StorySlot,
    ) -> Result<bool, StoreError>;
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
        Request::SaveStorySlot { write, reply } => {
            let _ = reply.send(handler.save_story_slot(write));
        }
        Request::LoadStorySlot {
            account_id,
            story_id,
            slot,
            pack,
            reply,
        } => {
            let _ = reply.send(handler.load_story_slot(account_id, story_id, slot, pack));
        }
        Request::ListStorySlots {
            account_id,
            story_id,
            reply,
        } => {
            let _ = reply.send(handler.list_story_slots(account_id, story_id));
        }
        Request::DeleteStorySlot {
            account_id,
            story_id,
            slot,
            reply,
        } => {
            let _ = reply.send(handler.delete_story_slot(account_id, story_id, slot));
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

#[derive(Clone)]
pub(super) struct PreparedLogin {
    pub(super) account_id: AccountId,
    pub(super) encoded_hash: String,
}
