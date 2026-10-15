//! Backend-agnostic persistence policy shared by the SQLite and PostgreSQL
//! stores.
//!
//! Everything here is pure: bounds, normalisation, JSON shaping of stored
//! documents, quota/rate-limit and capacity decisions, retention arithmetic and
//! staff-authority decisions. SQL, locking and transaction boundaries stay in
//! each backend so the two dialects keep their exact statements, while the
//! decisions they apply to the rows they read are defined once and pinned by
//! tests that run without PostgreSQL.

use std::{
    collections::BTreeSet,
    time::{SystemTime, UNIX_EPOCH},
};

use tiamud_core::{
    AccountId, AccountRole, CharacterGameplaySnapshot, CharacterSnapshot, DomainEvent,
    DurableCharacterSnapshot, SocialChannelName, StaffCapability, StaffProfileKey,
    WorldRuntimeSnapshot, valid_board_visible_text, valid_mail_visible_text,
};
use tokio::time::Duration;

use tiamud_persistence_sqlite::story_saves;

use super::{
    BOARD_ACCOUNT_CAPACITY, BOARD_CAPACITY_PER_BOARD, MAILBOX_CAPACITY, MAX_SOCIAL_REASON_BYTES,
    ModerationAction, StaffAccess, StaffAuditRecord, StoreError, StoryPackCompatibility,
    StorySaveRecord, StorySaveSummary, StorySaveWrite, StorySlot,
};
use crate::auth::PasswordWorkError;

pub(crate) const MAX_IDENTITY_NAME_BYTES: usize = 64;
pub(crate) const MAX_ACTIVE_SESSIONS: i64 = 16;
pub(crate) const MAX_CHARACTER_JSON_BYTES: usize = 65_536;
pub(crate) const MAX_EVENT_JSON_BYTES: usize = 65_536;
pub(crate) const MAX_RUNTIME_JSON_BYTES: usize = tiamud_core::MAX_WORLD_RUNTIME_SNAPSHOT_BYTES;
pub(crate) const MAX_AUDIT_DETAILS_BYTES: usize = 4_096;
pub(crate) const MAX_STAFF_ACTION_BYTES: usize = 64;
pub(crate) const MAX_STAFF_LORE_TITLE_BYTES: usize = 128;
pub(crate) const STAFF_LIST_LIMIT: i64 = 10;
pub(crate) const MAILBOX_BYTE_CAPACITY: i64 = 128 * 1_024;
pub(crate) const MAIL_GLOBAL_CAPACITY: i64 = 100_000;
pub(crate) const MAIL_HOURLY_CAPACITY: i64 = 20;
pub(crate) const MAIL_DAILY_CAPACITY: i64 = 50;
pub(crate) const MAIL_HOUR_SECONDS: i64 = 60 * 60;
pub(crate) const MAIL_DAY_SECONDS: i64 = 24 * MAIL_HOUR_SECONDS;
pub(crate) const MAIL_RETENTION_SECONDS: i64 = 30 * MAIL_DAY_SECONDS;
pub(crate) const MAIL_REPORT_EVIDENCE_RETENTION_SECONDS: i64 = 90 * MAIL_DAY_SECONDS;
pub(crate) const MAIL_OPEN_REPORT_CAPACITY_PER_ACCOUNT: i64 = 8;
pub(crate) const MAIL_REPORT_EVIDENCE_GLOBAL_CAPACITY: i64 = 10_000;
pub(crate) const MAIL_PURGE_LIMIT: i64 = 256;
pub(crate) const BOARD_BYTE_CAPACITY_PER_BOARD: i64 = 4 * 1_024 * 1_024;
pub(crate) const BOARD_GLOBAL_CAPACITY: i64 = 2_000;
pub(crate) const BOARD_GLOBAL_BYTE_CAPACITY: i64 = 8 * 1_024 * 1_024;
pub(crate) const BOARD_HOURLY_CAPACITY: i64 = 5;
pub(crate) const BOARD_DAILY_CAPACITY: i64 = 20;
pub(crate) const BOARD_RETENTION_SECONDS: i64 = 90 * MAIL_DAY_SECONDS;
pub(crate) const BOARD_REPORT_EVIDENCE_RETENTION_SECONDS: i64 = 90 * MAIL_DAY_SECONDS;
pub(crate) const BOARD_OPEN_REPORT_CAPACITY_PER_ACCOUNT: i64 = 8;
pub(crate) const BOARD_REPORT_EVIDENCE_GLOBAL_CAPACITY: i64 = 10_000;
pub(crate) const BOARD_PURGE_LIMIT: i64 = 256;
pub(crate) const MAX_MODERATION_DURATION_SECONDS: i64 = 365 * 24 * 60 * 60;
pub(crate) const PERSISTENCE_REQUEST_QUEUE_CAPACITY: usize = 256;
pub(crate) const PERSISTENCE_REQUEST_TIMEOUT: Duration = Duration::from_secs(5);

// ---------------------------------------------------------------------------
// Clock and error mapping
// ---------------------------------------------------------------------------

pub(crate) fn unix_timestamp() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
        .try_into()
        .unwrap_or(i64::MAX)
}

pub(crate) fn map_password_work_error(error: PasswordWorkError) -> StoreError {
    match error {
        PasswordWorkError::Overloaded | PasswordWorkError::Timeout => StoreError::RequestTimeout,
        PasswordWorkError::Execution => StoreError::PasswordHashing,
    }
}

// ---------------------------------------------------------------------------
// Text normalisation and bounds
// ---------------------------------------------------------------------------

pub(crate) fn normalise_name(name: &str, kind: &'static str) -> Result<String, StoreError> {
    let name = name.trim();
    if !valid_mail_visible_text(name, MAX_IDENTITY_NAME_BYTES) || name.contains('|') {
        return Err(StoreError::InvalidName { kind });
    }
    Ok(name.to_owned())
}

pub(crate) fn normalise_mail_text(value: &str, maximum: usize) -> Result<String, StoreError> {
    if !valid_mail_visible_text(value, maximum) {
        Err(StoreError::InvalidMailInput)
    } else {
        Ok(value.to_owned())
    }
}

pub(crate) fn normalise_board_text(value: &str, maximum: usize) -> Result<String, StoreError> {
    if valid_board_visible_text(value, maximum) {
        Ok(value.to_owned())
    } else {
        Err(StoreError::InvalidBoardInput)
    }
}

pub(crate) fn normalise_social_text(value: &str, maximum: usize) -> Result<String, StoreError> {
    let value = value.trim();
    if value.is_empty() || value.len() > maximum || value.chars().any(char::is_control) {
        Err(StoreError::InvalidSocialInput)
    } else {
        Ok(value.to_owned())
    }
}

pub(crate) fn normalise_staff_text(value: &str, maximum: usize) -> Result<String, StoreError> {
    let value = value.trim();
    if value.is_empty() || value.len() > maximum || value.chars().any(char::is_control) {
        Err(StoreError::InvalidStaffInput)
    } else {
        Ok(value.to_owned())
    }
}

pub(crate) fn normalise_lore_title(value: Option<&str>) -> Result<Option<String>, StoreError> {
    value
        .map(|value| normalise_staff_text(value, MAX_STAFF_LORE_TITLE_BYTES))
        .transpose()
}

/// Rejects moderation expiries that are not strictly in the future or exceed
/// the maximum moderation duration. `None` is a permanent sanction.
pub(crate) fn validate_expiry(expires_at: Option<i64>, now: i64) -> Result<(), StoreError> {
    if expires_at.is_some_and(|expires_at| {
        expires_at <= now || expires_at.saturating_sub(now) > MAX_MODERATION_DURATION_SECONDS
    }) {
        Err(StoreError::InvalidSocialInput)
    } else {
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// Stored enumerations
// ---------------------------------------------------------------------------

pub(crate) fn role_from_database(value: &str) -> Result<AccountRole, StoreError> {
    match value {
        "player" => Ok(AccountRole::Player),
        "moderator" => Ok(AccountRole::Moderator),
        "admin" => Ok(AccountRole::Admin),
        _ => Err(StoreError::InvalidSocialInput),
    }
}

pub(crate) fn role_as_database(role: AccountRole) -> &'static str {
    match role {
        AccountRole::Player => "player",
        AccountRole::Moderator => "moderator",
        AccountRole::Admin => "admin",
    }
}

pub(crate) fn social_channel_from_database(value: &str) -> Result<SocialChannelName, StoreError> {
    match value {
        "general" => Ok(SocialChannelName::General),
        "newbie" => Ok(SocialChannelName::Newbie),
        "staff" => Ok(SocialChannelName::Staff),
        _ => Err(StoreError::InvalidSocialInput),
    }
}

// ---------------------------------------------------------------------------
// JSON shaping of stored documents
// ---------------------------------------------------------------------------

pub(crate) fn bounded_json<T: serde::Serialize>(
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

/// `characters.gameplay_json` for one character.
pub(crate) fn encode_character_gameplay(
    gameplay: &CharacterGameplaySnapshot,
) -> Result<String, StoreError> {
    bounded_json(gameplay, "character gameplay", MAX_CHARACTER_JSON_BYTES)
}

/// `character_adventure.state_json`: the adventure projection of gameplay.
pub(crate) fn encode_character_adventure(
    gameplay: &CharacterGameplaySnapshot,
) -> Result<String, StoreError> {
    bounded_json(
        &serde_json::json!({
            "coins": gameplay.coins,
            "commodities": gameplay.commodities,
            "quests": gameplay.quests,
        }),
        "character adventure",
        MAX_CHARACTER_JSON_BYTES,
    )
}

/// The validated name and `item_locations_json` written by a character save.
pub(crate) fn encode_character_row(
    snapshot: &CharacterSnapshot,
) -> Result<(String, String), StoreError> {
    let name = normalise_name(&snapshot.name, "character")?;
    let item_locations_json = serde_json::to_string(&snapshot.item_locations)?;
    Ok((name, item_locations_json))
}

/// `world_runtime.snapshot_json` and `world_adventure.state_json` for one
/// runtime snapshot, in that order.
pub(crate) fn encode_runtime_documents(
    runtime: &WorldRuntimeSnapshot,
) -> Result<(String, String), StoreError> {
    let runtime_json = bounded_json(runtime, "world runtime", MAX_RUNTIME_JSON_BYTES)?;
    let world_adventure_json = bounded_json(
        &runtime.adventure,
        "world adventure",
        MAX_RUNTIME_JSON_BYTES,
    )?;
    Ok((runtime_json, world_adventure_json))
}

/// Every document an action commit writes, encoded and bounded before the
/// backend opens its transaction so an oversized or invalid document never
/// starts one.
pub(crate) struct EncodedActionCommit {
    pub(crate) gameplay_json: Vec<String>,
    pub(crate) adventure_json: Vec<String>,
    pub(crate) runtime_json: String,
    pub(crate) world_adventure_json: String,
    pub(crate) event_json: Vec<String>,
    pub(crate) staff_audit: Option<(PreparedStaffAudit, i64)>,
}

pub(crate) fn encode_action_commit(
    characters: &[DurableCharacterSnapshot],
    runtime: &WorldRuntimeSnapshot,
    events: &[DomainEvent],
    staff_audit: Option<(StaffAuditRecord, i64)>,
) -> Result<EncodedActionCommit, StoreError> {
    let gameplay_json = characters
        .iter()
        .map(|snapshot| encode_character_gameplay(&snapshot.gameplay))
        .collect::<Result<Vec<_>, _>>()?;
    let adventure_json = characters
        .iter()
        .map(|snapshot| encode_character_adventure(&snapshot.gameplay))
        .collect::<Result<Vec<_>, _>>()?;
    let (runtime_json, world_adventure_json) = encode_runtime_documents(runtime)?;
    let event_json = events
        .iter()
        .map(|event| bounded_json(event, "domain event", MAX_EVENT_JSON_BYTES))
        .collect::<Result<Vec<_>, _>>()?;
    let staff_audit = staff_audit
        .map(|(audit, now)| prepare_staff_audit(audit).map(|audit| (audit, now)))
        .transpose()?;
    Ok(EncodedActionCommit {
        gameplay_json,
        adventure_json,
        runtime_json,
        world_adventure_json,
        event_json,
        staff_audit,
    })
}

/// `moderation_audit.details_json`, after bounding the action name and the
/// serialized details.
pub(crate) fn encode_moderation_audit_details(
    action: &str,
    details: &serde_json::Value,
) -> Result<String, StoreError> {
    let details_json = serde_json::to_string(details)?;
    if action.is_empty() || action.len() > 64 || details_json.len() > MAX_AUDIT_DETAILS_BYTES {
        return Err(StoreError::InvalidSocialInput);
    }
    Ok(details_json)
}

// ---------------------------------------------------------------------------
// Rate limits, quotas and capacity
// ---------------------------------------------------------------------------

/// One account's rolling hourly and daily send counters, as stored in
/// `social_mail_usage` / `social_board_usage`.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct QuotaWindow {
    pub(crate) hour_started_at: i64,
    pub(crate) hour_count: i64,
    pub(crate) day_started_at: i64,
    pub(crate) day_count: i64,
}

impl QuotaWindow {
    /// The window a first send starts from when no usage row exists.
    pub(crate) fn empty(now: i64) -> Self {
        Self {
            hour_started_at: now,
            hour_count: 0,
            day_started_at: now,
            day_count: 0,
        }
    }

    /// Restarts each period that has elapsed, or whose start lies in the
    /// future (clock rollback), at `now` with a zero count.
    pub(crate) fn rolled(self, now: i64) -> Self {
        let hour_expired = now < self.hour_started_at
            || now.saturating_sub(self.hour_started_at) >= MAIL_HOUR_SECONDS;
        let day_expired = now < self.day_started_at
            || now.saturating_sub(self.day_started_at) >= MAIL_DAY_SECONDS;
        Self {
            hour_started_at: if hour_expired {
                now
            } else {
                self.hour_started_at
            },
            hour_count: if hour_expired { 0 } else { self.hour_count },
            day_started_at: if day_expired {
                now
            } else {
                self.day_started_at
            },
            day_count: if day_expired { 0 } else { self.day_count },
        }
    }

    /// Whether one more send would exceed either period's capacity.
    pub(crate) fn exhausted(self, hourly_capacity: i64, daily_capacity: i64) -> bool {
        self.hour_count >= hourly_capacity || self.day_count >= daily_capacity
    }
}

/// Increments a bounded quota counter for storage as a 32-bit `INTEGER`;
/// `None` fails closed.
pub(crate) fn next_quota_count(count: i64) -> Option<i32> {
    count
        .checked_add(1)
        .and_then(|next| i32::try_from(next).ok())
}

/// Whether a new message must be suppressed because the recipient mailbox or
/// the global mail store is full.
pub(crate) fn mail_delivery_exceeds_capacity(
    mailbox_count: i64,
    mailbox_bytes: i64,
    added_bytes: i64,
    global_count: i64,
) -> bool {
    mailbox_count >= i64::from(MAILBOX_CAPACITY)
        || mailbox_bytes.saturating_add(added_bytes) > MAILBOX_BYTE_CAPACITY
        || global_count >= MAIL_GLOBAL_CAPACITY
}

pub(crate) fn check_mail_report_quota(open_reports: i64) -> Result<(), StoreError> {
    if open_reports >= MAIL_OPEN_REPORT_CAPACITY_PER_ACCOUNT {
        Err(StoreError::MailReportLimit)
    } else {
        Ok(())
    }
}

pub(crate) fn check_mail_evidence_capacity(evidence_count: i64) -> Result<(), StoreError> {
    if evidence_count >= MAIL_REPORT_EVIDENCE_GLOBAL_CAPACITY {
        Err(StoreError::MailEvidenceLimit)
    } else {
        Ok(())
    }
}

pub(crate) fn check_board_capacity(
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

pub(crate) fn check_board_report_capacity(
    open_reports: i64,
    evidence_count: i64,
) -> Result<(), StoreError> {
    if open_reports >= BOARD_OPEN_REPORT_CAPACITY_PER_ACCOUNT {
        Err(StoreError::BoardReportLimit)
    } else if evidence_count >= BOARD_REPORT_EVIDENCE_GLOBAL_CAPACITY {
        Err(StoreError::BoardEvidenceLimit)
    } else {
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// Story save slots (schema v16, ADR 0030)
// ---------------------------------------------------------------------------

/// A validated slot write ready for either backend's upsert.
pub(crate) struct PreparedStorySave {
    pub(crate) account_id: AccountId,
    pub(crate) story_id: String,
    pub(crate) slot: i64,
    pub(crate) pack_id: String,
    pub(crate) pack_sha256: String,
    pub(crate) state_format_version: i64,
    pub(crate) state_json: String,
    pub(crate) state_sha256: String,
    pub(crate) state_bytes: i64,
}

/// Validates a write against the shared row contract and computes the digest
/// of its exact bytes. Oversized documents get their own typed error.
pub(crate) fn prepare_story_save(write: StorySaveWrite) -> Result<PreparedStorySave, StoreError> {
    if write.account_id.0 <= 0 {
        return Err(StoreError::InvalidStorySave);
    }
    let slot = i64::from(write.slot.get());
    story_saves::validate_story_save_identity(
        &write.story_id,
        slot,
        &write.pack_id,
        &write.pack_sha256,
    )
    .map_err(|_| StoreError::InvalidStorySave)?;
    if write.state_json.len() > story_saves::MAX_STORY_SAVE_STATE_BYTES {
        return Err(StoreError::StorySaveTooLarge {
            maximum: story_saves::MAX_STORY_SAVE_STATE_BYTES,
        });
    }
    let state_sha256 = story_saves::story_state_sha256(&write.state_json);
    let state_format_version = i64::from(write.state_format_version);
    story_saves::validate_story_save_document(
        &write.pack_id,
        state_format_version,
        &write.state_json,
        &state_sha256,
    )
    .map_err(|_| StoreError::InvalidStorySave)?;
    let state_bytes = row_count(write.state_json.len());
    Ok(PreparedStorySave {
        account_id: write.account_id,
        story_id: write.story_id,
        slot,
        pack_id: write.pack_id,
        pack_sha256: write.pack_sha256,
        state_format_version,
        state_json: write.state_json,
        state_sha256,
        state_bytes,
    })
}

/// Validates the `(account, story)` part of a lookup.
pub(crate) fn validate_story_lookup(
    account_id: AccountId,
    story_id: &str,
) -> Result<(), StoreError> {
    if account_id.0 <= 0 || !story_saves::valid_story_identifier(story_id) {
        return Err(StoreError::InvalidStorySave);
    }
    Ok(())
}

/// Validates the caller's pack revision and its bounded compatibility set.
pub(crate) fn validate_story_pack_compatibility(
    pack: &StoryPackCompatibility,
) -> Result<(), StoreError> {
    if !story_saves::valid_story_identifier(&pack.pack_id)
        || !story_saves::valid_sha256_hex(&pack.pack_sha256)
        || pack.compatible_pack_sha256.len() > story_saves::MAX_COMPATIBLE_STORY_PACK_REVISIONS
        || !pack
            .compatible_pack_sha256
            .iter()
            .all(|digest| story_saves::valid_sha256_hex(digest))
    {
        return Err(StoreError::InvalidStorySave);
    }
    Ok(())
}

/// Maps the per-account cap decision to the store error.
pub(crate) fn check_story_save_account_capacity(
    stories_with_saves: i64,
    story_has_saves: bool,
    account_bytes: i64,
    replaced_bytes: i64,
    added_bytes: i64,
) -> Result<(), StoreError> {
    story_saves::check_story_save_capacity(
        stories_with_saves,
        story_has_saves,
        account_bytes,
        replaced_bytes,
        added_bytes,
    )
    .map_err(|_| StoreError::StorySaveCapacity)
}

/// One stored row as either backend reads it, widened to `i64`.
pub(crate) struct StorySaveRow {
    pub(crate) story_id: String,
    pub(crate) slot: i64,
    pub(crate) pack_id: String,
    pub(crate) pack_sha256: String,
    pub(crate) state_format_version: i64,
    pub(crate) state_json: String,
    pub(crate) state_sha256: String,
    pub(crate) created_at: i64,
    pub(crate) updated_at: i64,
}

/// Re-checks a stored row completely. Any disagreement between bytes,
/// digest, metadata or bounds is corruption, never a panic.
pub(crate) fn decode_story_save_row(row: StorySaveRow) -> Result<StorySaveRecord, StoreError> {
    story_saves::validate_story_save_row(
        &row.story_id,
        row.slot,
        &row.pack_id,
        &row.pack_sha256,
        row.state_format_version,
        &row.state_json,
        &row.state_sha256,
        row.created_at,
        row.updated_at,
    )
    .map_err(|_| StoreError::StorySaveCorrupt)?;
    let summary = decode_story_save_summary(
        row.story_id,
        row.slot,
        row.pack_id,
        row.pack_sha256,
        row.state_format_version,
        row_count(row.state_json.len()),
        row.state_sha256,
        row.created_at,
        row.updated_at,
    )?;
    Ok(StorySaveRecord {
        summary,
        state_json: row.state_json,
    })
}

/// Decodes listing metadata (no document) and fails closed on bad columns.
#[allow(clippy::too_many_arguments)]
pub(crate) fn decode_story_save_summary(
    story_id: String,
    slot: i64,
    pack_id: String,
    pack_sha256: String,
    state_format_version: i64,
    state_bytes: i64,
    state_sha256: String,
    created_at: i64,
    updated_at: i64,
) -> Result<StorySaveSummary, StoreError> {
    let slot = story_saves::validate_story_save_identity(&story_id, slot, &pack_id, &pack_sha256)
        .ok()
        .and_then(StorySlot::new)
        .ok_or(StoreError::StorySaveCorrupt)?;
    let state_format_version = u32::try_from(state_format_version)
        .ok()
        .filter(|version| {
            (story_saves::MIN_STORY_STATE_FORMAT_VERSION
                ..=story_saves::MAX_STORY_STATE_FORMAT_VERSION)
                .contains(version)
        })
        .ok_or(StoreError::StorySaveCorrupt)?;
    let state_bytes = u32::try_from(state_bytes)
        .ok()
        .filter(|bytes| {
            (story_saves::MIN_STORY_SAVE_STATE_BYTES..=story_saves::MAX_STORY_SAVE_STATE_BYTES)
                .contains(&(*bytes as usize))
        })
        .ok_or(StoreError::StorySaveCorrupt)?;
    if !story_saves::valid_sha256_hex(&state_sha256) || created_at < 0 || updated_at < created_at {
        return Err(StoreError::StorySaveCorrupt);
    }
    Ok(StorySaveSummary {
        story_id,
        slot,
        pack_id,
        pack_sha256,
        state_format_version,
        state_bytes,
        state_sha256,
        created_at,
        updated_at,
    })
}

/// Fails closed unless the slot was written with the caller's pack id and
/// either the exact current digest or a digest the caller declares
/// compatible. There is no implicit migration between pack revisions.
pub(crate) fn check_story_pack_compatibility(
    summary: &StorySaveSummary,
    pack: &StoryPackCompatibility,
) -> Result<(), StoreError> {
    if summary.pack_id == pack.pack_id
        && (summary.pack_sha256 == pack.pack_sha256
            || pack.compatible_pack_sha256.contains(&summary.pack_sha256))
    {
        Ok(())
    } else {
        Err(StoreError::StorySavePackMismatch)
    }
}

// ---------------------------------------------------------------------------
// Retention
// ---------------------------------------------------------------------------

/// Converts a bounded row count to the SQL row-count type without panicking.
pub(crate) fn row_count(length: usize) -> i64 {
    i64::try_from(length).unwrap_or(i64::MAX)
}

/// Rows a bounded purge may still delete from its second table after
/// `deleted` rows were removed from the first.
pub(crate) fn purge_remaining(limit: i64, deleted: usize) -> i64 {
    limit.saturating_sub(row_count(deleted))
}

// ---------------------------------------------------------------------------
// Moderation and staff authority
// ---------------------------------------------------------------------------

/// A validated moderation request: the capability it requires, its audit
/// action name, its normalised reason and the audit details document.
pub(crate) struct PreparedModeration {
    pub(crate) required_capability: StaffCapability,
    pub(crate) action_name: &'static str,
    pub(crate) reason: String,
    pub(crate) details: serde_json::Value,
}

pub(crate) fn prepare_moderation(
    action: &ModerationAction,
    now: i64,
) -> Result<PreparedModeration, StoreError> {
    let (required_capability, action_name, reason, details) = match action {
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
    Ok(PreparedModeration {
        required_capability,
        action_name,
        reason,
        details,
    })
}

pub(crate) struct PreparedStaffAudit {
    pub(crate) actor_account_id: AccountId,
    pub(crate) target_account_id: Option<AccountId>,
    pub(crate) capability: StaffCapability,
    pub(crate) action: String,
    pub(crate) reason: String,
    pub(crate) details_json: String,
}

pub(crate) fn prepare_staff_audit(
    record: StaffAuditRecord,
) -> Result<PreparedStaffAudit, StoreError> {
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

/// Parses a stored `staff_profiles.profile_key`.
pub(crate) fn parse_staff_profile(
    profile: Option<&str>,
) -> Result<Option<StaffProfileKey>, StoreError> {
    profile
        .map(|value| {
            StaffProfileKey::parse(value)
                .ok_or_else(|| StoreError::InvalidStaffState(format!("profile {value:?}")))
        })
        .transpose()
}

/// Assembles one account's staff access from its stored role, parsed profile,
/// lore title and capability rows, failing closed on unknown values or on
/// capabilities held without a profile.
pub(crate) fn staff_access_from_rows(
    account_id: AccountId,
    role: &str,
    profile: Option<StaffProfileKey>,
    lore_title: Option<String>,
    capabilities: impl IntoIterator<Item = String>,
) -> Result<StaffAccess, StoreError> {
    let capabilities = capabilities
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
        role: role_from_database(role)?,
        profile,
        lore_title,
        capabilities,
    })
}

/// Checks that `audit` is authorised: the expected capability and target
/// match, the actor holds the capability through a profile, and any target
/// ranks strictly below the actor. `load_access` reads one account's staff
/// access inside the caller's transaction; it is called for the actor and then,
/// only when the earlier checks pass, for the target.
pub(crate) fn validate_staff_authority_with(
    audit: &PreparedStaffAudit,
    expected_capability: Option<StaffCapability>,
    target_account: Option<AccountId>,
    mut load_access: impl FnMut(AccountId) -> Result<StaffAccess, StoreError>,
) -> Result<StaffAccess, StoreError> {
    if expected_capability.is_some_and(|expected| expected != audit.capability)
        || audit.target_account_id != target_account
    {
        return Err(StoreError::StaffForbidden);
    }
    let actor = load_access(audit.actor_account_id)?;
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
        let target = load_access(target_account)?;
        if target
            .profile
            .is_some_and(|profile| profile.authority_rank() >= actor_profile.authority_rank())
        {
            return Err(StoreError::StaffHierarchy);
        }
    }
    Ok(actor)
}

/// Pins a report-resolution audit to the report's target account. A caller
/// that named a different target is refused.
pub(crate) fn bind_report_audit_target(
    audit: &mut PreparedStaffAudit,
    target_account: AccountId,
) -> Result<(), StoreError> {
    if audit
        .target_account_id
        .is_some_and(|account_id| account_id != target_account)
    {
        return Err(StoreError::StaffForbidden);
    }
    audit.target_account_id = Some(target_account);
    Ok(())
}

/// An actor may assign only a profile ranked strictly below its own and only
/// when it already holds every default capability of that profile.
pub(crate) fn authorize_profile_assignment(
    actor: &StaffAccess,
    profile: Option<StaffProfileKey>,
) -> Result<(), StoreError> {
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
    Ok(())
}

/// Granting a capability requires a staff target and an actor that holds the
/// capability itself; revoking has no further requirement.
pub(crate) fn authorize_capability_change(
    actor: &StaffAccess,
    target: &StaffAccess,
    capability: StaffCapability,
    enabled: bool,
) -> Result<(), StoreError> {
    if enabled && (target.profile.is_none() || !actor.capabilities.contains(&capability)) {
        return Err(StoreError::StaffForbidden);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn access(
        account: i64,
        profile: Option<StaffProfileKey>,
        capabilities: &[StaffCapability],
    ) -> StaffAccess {
        StaffAccess {
            account_id: AccountId(account),
            role: AccountRole::Player,
            profile,
            lore_title: None,
            capabilities: capabilities.iter().copied().collect(),
        }
    }

    fn audit(actor: i64, target: Option<i64>, capability: StaffCapability) -> PreparedStaffAudit {
        PreparedStaffAudit {
            actor_account_id: AccountId(actor),
            target_account_id: target.map(AccountId),
            capability,
            action: "test".to_owned(),
            reason: "reason".to_owned(),
            details_json: "{}".to_owned(),
        }
    }

    fn profiles_by_rank() -> Vec<StaffProfileKey> {
        let mut profiles = StaffProfileKey::ALL.to_vec();
        profiles.sort_by_key(|profile| profile.authority_rank());
        profiles
    }

    #[test]
    fn text_normalisers_trim_bound_and_reject_controls() {
        assert_eq!(normalise_name("  Ada ", "character").unwrap(), "Ada");
        assert!(matches!(
            normalise_name("A|b", "character"),
            Err(StoreError::InvalidName { kind: "character" })
        ));
        assert!(normalise_name(&"a".repeat(MAX_IDENTITY_NAME_BYTES + 1), "x").is_err());
        assert!(normalise_name("   ", "x").is_err());

        assert_eq!(normalise_social_text(" why ", 8).unwrap(), "why");
        assert!(matches!(
            normalise_social_text("a\u{7}", 8),
            Err(StoreError::InvalidSocialInput)
        ));
        assert!(normalise_social_text("123456789", 8).is_err());
        assert!(normalise_social_text("  ", 8).is_err());

        assert_eq!(normalise_staff_text(" ok ", 8).unwrap(), "ok");
        assert!(matches!(
            normalise_staff_text("", 8),
            Err(StoreError::InvalidStaffInput)
        ));
        assert_eq!(normalise_lore_title(None).unwrap(), None);
        assert!(normalise_lore_title(Some(&"t".repeat(MAX_STAFF_LORE_TITLE_BYTES + 1))).is_err());
        assert_eq!(
            normalise_lore_title(Some(" Keeper ")).unwrap().as_deref(),
            Some("Keeper")
        );

        // Mail and board text are not trimmed: they are stored verbatim.
        assert_eq!(normalise_mail_text("hi", 2).unwrap(), "hi");
        assert!(matches!(
            normalise_mail_text("hi!", 2),
            Err(StoreError::InvalidMailInput)
        ));
        assert_eq!(normalise_board_text("hi", 2).unwrap(), "hi");
        assert!(matches!(
            normalise_board_text("hi!", 2),
            Err(StoreError::InvalidBoardInput)
        ));
    }

    fn story_summary(pack_id: &str, pack_sha: char) -> StorySaveSummary {
        decode_story_save_summary(
            "cripta".into(),
            1,
            pack_id.into(),
            pack_sha.to_string().repeat(64),
            2,
            64,
            "e".repeat(64),
            10,
            10,
        )
        .unwrap()
    }

    #[test]
    fn story_save_policy_prepares_writes_and_fails_closed_on_rows() {
        let json = r#"{"format_version":2,"pack_id":"cripta"}"#.to_owned();
        let prepared = prepare_story_save(StorySaveWrite {
            account_id: AccountId(1),
            story_id: "cripta".into(),
            slot: StorySlot::AUTOSAVE,
            pack_id: "cripta".into(),
            pack_sha256: "a".repeat(64),
            state_format_version: 2,
            state_json: json.clone(),
        })
        .unwrap();
        assert_eq!(prepared.slot, 0);
        assert_eq!(prepared.state_bytes, row_count(json.len()));
        assert_eq!(
            prepared.state_sha256,
            story_saves::story_state_sha256(&json)
        );
        assert!(matches!(
            prepare_story_save(StorySaveWrite {
                account_id: AccountId(0),
                story_id: "cripta".into(),
                slot: StorySlot::AUTOSAVE,
                pack_id: "cripta".into(),
                pack_sha256: "a".repeat(64),
                state_format_version: 2,
                state_json: json.clone(),
            }),
            Err(StoreError::InvalidStorySave)
        ));

        let row = |state_json: String, state_sha256: String| StorySaveRow {
            story_id: "cripta".into(),
            slot: 4,
            pack_id: "cripta".into(),
            pack_sha256: "a".repeat(64),
            state_format_version: 2,
            state_json,
            state_sha256,
            created_at: 5,
            updated_at: 6,
        };
        let digest = story_saves::story_state_sha256(&json);
        let record = decode_story_save_row(row(json.clone(), digest.clone())).unwrap();
        assert_eq!(record.summary.slot.get(), 4);
        assert_eq!(record.summary.state_bytes as usize, json.len());
        assert!(matches!(
            decode_story_save_row(row(json.replace('2', "3"), digest.clone())),
            Err(StoreError::StorySaveCorrupt)
        ));
        let mut bad_slot = row(json.clone(), digest);
        bad_slot.slot = 12;
        assert!(matches!(
            decode_story_save_row(bad_slot),
            Err(StoreError::StorySaveCorrupt)
        ));
        assert!(matches!(
            decode_story_save_summary(
                "cripta".into(),
                1,
                "cripta".into(),
                "a".repeat(64),
                2,
                1,
                "e".repeat(64),
                10,
                10,
            ),
            Err(StoreError::StorySaveCorrupt)
        ));
    }

    #[test]
    fn story_pack_compatibility_requires_same_pack_and_declared_digest() {
        let current = StoryPackCompatibility {
            pack_id: "cripta".into(),
            pack_sha256: "a".repeat(64),
            compatible_pack_sha256: ["b".repeat(64)].into_iter().collect(),
        };
        validate_story_pack_compatibility(&current).unwrap();
        assert!(check_story_pack_compatibility(&story_summary("cripta", 'a'), &current).is_ok());
        assert!(check_story_pack_compatibility(&story_summary("cripta", 'b'), &current).is_ok());
        assert!(matches!(
            check_story_pack_compatibility(&story_summary("cripta", 'c'), &current),
            Err(StoreError::StorySavePackMismatch)
        ));
        assert!(matches!(
            check_story_pack_compatibility(&story_summary("otra", 'a'), &current),
            Err(StoreError::StorySavePackMismatch)
        ));
        let mut malformed = current.clone();
        malformed.compatible_pack_sha256.insert("B".repeat(64));
        assert!(matches!(
            validate_story_pack_compatibility(&malformed),
            Err(StoreError::InvalidStorySave)
        ));
        assert!(matches!(
            check_story_save_account_capacity(
                story_saves::MAX_STORIES_WITH_SAVES_PER_ACCOUNT as i64,
                false,
                0,
                0,
                1
            ),
            Err(StoreError::StorySaveCapacity)
        ));
    }

    #[test]
    fn moderation_expiry_is_future_and_bounded() {
        let now = 1_000;
        assert!(validate_expiry(None, now).is_ok());
        assert!(validate_expiry(Some(now + 1), now).is_ok());
        assert!(validate_expiry(Some(now + MAX_MODERATION_DURATION_SECONDS), now).is_ok());
        assert!(validate_expiry(Some(now), now).is_err());
        assert!(validate_expiry(Some(now - 1), now).is_err());
        assert!(validate_expiry(Some(now + MAX_MODERATION_DURATION_SECONDS + 1), now).is_err());
        assert!(validate_expiry(Some(i64::MAX), i64::MIN).is_err());
    }

    #[test]
    fn stored_enumerations_round_trip_and_reject_unknown_values() {
        for role in [
            AccountRole::Player,
            AccountRole::Moderator,
            AccountRole::Admin,
        ] {
            assert_eq!(role_from_database(role_as_database(role)).unwrap(), role);
        }
        assert!(role_from_database("Admin").is_err());
        assert_eq!(
            social_channel_from_database("general").unwrap(),
            SocialChannelName::General
        );
        assert_eq!(
            social_channel_from_database("newbie").unwrap(),
            SocialChannelName::Newbie
        );
        assert_eq!(
            social_channel_from_database("staff").unwrap(),
            SocialChannelName::Staff
        );
        assert!(social_channel_from_database("ooc").is_err());
    }

    #[test]
    fn password_work_errors_map_to_timeout_or_hashing() {
        assert!(matches!(
            map_password_work_error(PasswordWorkError::Overloaded),
            StoreError::RequestTimeout
        ));
        assert!(matches!(
            map_password_work_error(PasswordWorkError::Timeout),
            StoreError::RequestTimeout
        ));
        assert!(matches!(
            map_password_work_error(PasswordWorkError::Execution),
            StoreError::PasswordHashing
        ));
    }

    #[test]
    fn bounded_json_refuses_documents_over_the_limit() {
        assert_eq!(bounded_json(&"ab", "kind", 4).unwrap(), "\"ab\"");
        assert!(matches!(
            bounded_json(&"abc", "kind", 4),
            Err(StoreError::SnapshotTooLarge {
                kind: "kind",
                maximum: 4
            })
        ));
    }

    #[test]
    fn character_adventure_document_has_the_stored_shape() {
        let gameplay = CharacterGameplaySnapshot::default();
        let adventure: serde_json::Value =
            serde_json::from_str(&encode_character_adventure(&gameplay).unwrap()).unwrap();
        let mut keys = adventure
            .as_object()
            .unwrap()
            .keys()
            .cloned()
            .collect::<Vec<_>>();
        keys.sort();
        assert_eq!(keys, ["coins", "commodities", "quests"]);
        assert_eq!(adventure["coins"], serde_json::json!(gameplay.coins));
        assert_eq!(
            encode_character_gameplay(&gameplay).unwrap(),
            serde_json::to_string(&gameplay).unwrap()
        );
    }

    #[test]
    fn action_commit_encoding_validates_staff_audit_before_any_write() {
        let runtime = tiamud_core::World::demo().runtime_snapshot();
        let encoded = encode_action_commit(&[], &runtime, &[], None).unwrap();
        assert!(encoded.gameplay_json.is_empty());
        assert!(encoded.adventure_json.is_empty());
        assert!(encoded.event_json.is_empty());
        assert!(encoded.staff_audit.is_none());
        assert_eq!(
            (encoded.runtime_json, encoded.world_adventure_json),
            encode_runtime_documents(&runtime).unwrap()
        );

        let self_targeted = StaffAuditRecord {
            actor_account_id: AccountId(1),
            target_account_id: Some(AccountId(1)),
            capability: StaffCapability::StaffAssign,
            action: "assign".to_owned(),
            reason: "reason".to_owned(),
            details: serde_json::json!({}),
        };
        assert!(matches!(
            encode_action_commit(&[], &runtime, &[], Some((self_targeted, 7))),
            Err(StoreError::StaffHierarchy)
        ));
    }

    #[test]
    fn moderation_audit_details_are_bounded() {
        let details = serde_json::json!({"reportId": 4});
        assert_eq!(
            encode_moderation_audit_details("report", &details).unwrap(),
            r#"{"reportId":4}"#
        );
        assert!(encode_moderation_audit_details("", &details).is_err());
        assert!(encode_moderation_audit_details(&"a".repeat(65), &details).is_err());
        let large = serde_json::json!({"text": "x".repeat(MAX_AUDIT_DETAILS_BYTES)});
        assert!(matches!(
            encode_moderation_audit_details("report", &large),
            Err(StoreError::InvalidSocialInput)
        ));
    }

    #[test]
    fn quota_window_rolls_elapsed_or_future_periods() {
        let now = 10 * MAIL_DAY_SECONDS;
        let stored = QuotaWindow {
            hour_started_at: now - 10,
            hour_count: 3,
            day_started_at: now - MAIL_HOUR_SECONDS,
            day_count: 9,
        };
        // Neither period elapsed.
        assert_eq!(stored.rolled(now), stored);
        // The hour elapsed, the day did not.
        let hour_elapsed = QuotaWindow {
            hour_started_at: now - MAIL_HOUR_SECONDS,
            ..stored
        };
        assert_eq!(
            hour_elapsed.rolled(now),
            QuotaWindow {
                hour_started_at: now,
                hour_count: 0,
                ..hour_elapsed
            }
        );
        // Both elapsed.
        let day_elapsed = QuotaWindow {
            hour_started_at: now - MAIL_DAY_SECONDS,
            day_started_at: now - MAIL_DAY_SECONDS,
            ..stored
        };
        assert_eq!(day_elapsed.rolled(now), QuotaWindow::empty(now));
        // A start in the future (clock rollback) restarts the period.
        let future = QuotaWindow {
            hour_started_at: now + 1,
            day_started_at: now + 1,
            ..stored
        };
        assert_eq!(future.rolled(now), QuotaWindow::empty(now));
        // Saturating arithmetic: an ancient start never overflows.
        let ancient = QuotaWindow {
            hour_started_at: i64::MIN,
            day_started_at: i64::MIN,
            ..stored
        };
        assert_eq!(ancient.rolled(i64::MAX), QuotaWindow::empty(i64::MAX));
    }

    #[test]
    fn quota_window_exhaustion_matches_mail_and_board_capacities() {
        let now = 0;
        let below = QuotaWindow {
            hour_count: MAIL_HOURLY_CAPACITY - 1,
            day_count: MAIL_DAILY_CAPACITY - 1,
            ..QuotaWindow::empty(now)
        };
        assert!(!below.exhausted(MAIL_HOURLY_CAPACITY, MAIL_DAILY_CAPACITY));
        let hourly = QuotaWindow {
            hour_count: MAIL_HOURLY_CAPACITY,
            ..below
        };
        assert!(hourly.exhausted(MAIL_HOURLY_CAPACITY, MAIL_DAILY_CAPACITY));
        let daily = QuotaWindow {
            day_count: MAIL_DAILY_CAPACITY,
            ..below
        };
        assert!(daily.exhausted(MAIL_HOURLY_CAPACITY, MAIL_DAILY_CAPACITY));
        let board = QuotaWindow {
            hour_count: BOARD_HOURLY_CAPACITY,
            ..QuotaWindow::empty(now)
        };
        assert!(board.exhausted(BOARD_HOURLY_CAPACITY, BOARD_DAILY_CAPACITY));
        assert!(!QuotaWindow::empty(now).exhausted(BOARD_HOURLY_CAPACITY, BOARD_DAILY_CAPACITY));
    }

    #[test]
    fn next_quota_count_fails_closed_outside_i32() {
        assert_eq!(next_quota_count(0), Some(1));
        assert_eq!(next_quota_count(i64::from(i32::MAX) - 1), Some(i32::MAX));
        assert_eq!(next_quota_count(i64::from(i32::MAX)), None);
        assert_eq!(next_quota_count(i64::MAX), None);
    }

    #[test]
    fn mail_capacity_decisions_are_exact_at_their_limits() {
        let capacity = i64::from(MAILBOX_CAPACITY);
        assert!(!mail_delivery_exceeds_capacity(capacity - 1, 0, 10, 0));
        assert!(mail_delivery_exceeds_capacity(capacity, 0, 10, 0));
        assert!(!mail_delivery_exceeds_capacity(
            0,
            MAILBOX_BYTE_CAPACITY - 10,
            10,
            0
        ));
        assert!(mail_delivery_exceeds_capacity(
            0,
            MAILBOX_BYTE_CAPACITY - 10,
            11,
            0
        ));
        assert!(mail_delivery_exceeds_capacity(0, i64::MAX, i64::MAX, 0));
        assert!(!mail_delivery_exceeds_capacity(
            0,
            0,
            0,
            MAIL_GLOBAL_CAPACITY - 1
        ));
        assert!(mail_delivery_exceeds_capacity(
            0,
            0,
            0,
            MAIL_GLOBAL_CAPACITY
        ));

        assert!(check_mail_report_quota(MAIL_OPEN_REPORT_CAPACITY_PER_ACCOUNT - 1).is_ok());
        assert!(matches!(
            check_mail_report_quota(MAIL_OPEN_REPORT_CAPACITY_PER_ACCOUNT),
            Err(StoreError::MailReportLimit)
        ));
        assert!(check_mail_evidence_capacity(MAIL_REPORT_EVIDENCE_GLOBAL_CAPACITY - 1).is_ok());
        assert!(matches!(
            check_mail_evidence_capacity(MAIL_REPORT_EVIDENCE_GLOBAL_CAPACITY),
            Err(StoreError::MailEvidenceLimit)
        ));
    }

    #[test]
    fn board_capacity_decisions_are_exact_at_their_limits() {
        assert!(check_board_capacity(0, 0, 0, 0, 0, 0).is_ok());
        let account = i64::from(BOARD_ACCOUNT_CAPACITY);
        let board = i64::from(BOARD_CAPACITY_PER_BOARD);
        assert!(check_board_capacity(account - 1, 0, 0, 0, 0, 0).is_ok());
        for refused in [
            check_board_capacity(account, 0, 0, 0, 0, 0),
            check_board_capacity(0, board, 0, 0, 0, 0),
            check_board_capacity(0, 0, BOARD_BYTE_CAPACITY_PER_BOARD, 0, 0, 1),
            check_board_capacity(0, 0, 0, BOARD_GLOBAL_CAPACITY, 0, 0),
            check_board_capacity(0, 0, 0, 0, BOARD_GLOBAL_BYTE_CAPACITY, 1),
            check_board_capacity(0, 0, i64::MAX, 0, 0, i64::MAX),
        ] {
            assert!(matches!(refused, Err(StoreError::BoardCapacity)));
        }
        assert!(check_board_capacity(0, 0, BOARD_BYTE_CAPACITY_PER_BOARD - 1, 0, 0, 1).is_ok());

        assert!(check_board_report_capacity(0, 0).is_ok());
        assert!(matches!(
            check_board_report_capacity(BOARD_OPEN_REPORT_CAPACITY_PER_ACCOUNT, 0),
            Err(StoreError::BoardReportLimit)
        ));
        assert!(matches!(
            check_board_report_capacity(0, BOARD_REPORT_EVIDENCE_GLOBAL_CAPACITY),
            Err(StoreError::BoardEvidenceLimit)
        ));
        // The per-account limit is reported before the global evidence limit.
        assert!(matches!(
            check_board_report_capacity(
                BOARD_OPEN_REPORT_CAPACITY_PER_ACCOUNT,
                BOARD_REPORT_EVIDENCE_GLOBAL_CAPACITY
            ),
            Err(StoreError::BoardReportLimit)
        ));
    }

    #[test]
    fn retention_constants_and_purge_budget() {
        assert_eq!(MAIL_RETENTION_SECONDS, 30 * 24 * 60 * 60);
        assert_eq!(MAIL_REPORT_EVIDENCE_RETENTION_SECONDS, 90 * 24 * 60 * 60);
        assert_eq!(BOARD_RETENTION_SECONDS, 90 * 24 * 60 * 60);
        assert_eq!(BOARD_REPORT_EVIDENCE_RETENTION_SECONDS, 90 * 24 * 60 * 60);
        assert_eq!(purge_remaining(MAIL_PURGE_LIMIT, 0), MAIL_PURGE_LIMIT);
        assert_eq!(purge_remaining(BOARD_PURGE_LIMIT, 200), 56);
        assert_eq!(purge_remaining(BOARD_PURGE_LIMIT, 256), 0);
        assert_eq!(
            purge_remaining(BOARD_PURGE_LIMIT, usize::MAX),
            256 - i64::MAX
        );
        assert_eq!(row_count(usize::MAX), i64::MAX);
        assert_eq!(row_count(3), 3);
    }

    #[test]
    fn moderation_requests_map_to_capability_action_and_details() {
        let now = 100;
        let mute = prepare_moderation(
            &ModerationAction::Mute {
                expires_at: Some(now + 60),
                reason: " spam ".to_owned(),
            },
            now,
        )
        .unwrap();
        assert_eq!(mute.required_capability, StaffCapability::ModerationMute);
        assert_eq!(mute.action_name, "mute");
        assert_eq!(mute.reason, "spam");
        assert_eq!(
            mute.details,
            serde_json::json!({"expiresAt": now + 60, "reason": "spam"})
        );

        let unban = prepare_moderation(
            &ModerationAction::Unban {
                reason: "appeal".to_owned(),
            },
            now,
        )
        .unwrap();
        assert_eq!(unban.required_capability, StaffCapability::ModerationBan);
        assert_eq!(unban.action_name, "unban");
        assert_eq!(unban.details, serde_json::json!({"reason": "appeal"}));

        let role = prepare_moderation(
            &ModerationAction::SetRole {
                role: AccountRole::Moderator,
                reason: "trusted".to_owned(),
            },
            now,
        )
        .unwrap();
        assert_eq!(role.required_capability, StaffCapability::StaffAssign);
        assert_eq!(role.action_name, "set_role");
        assert_eq!(
            role.details,
            serde_json::json!({"role": "moderator", "reason": "trusted"})
        );

        assert!(matches!(
            prepare_moderation(
                &ModerationAction::Ban {
                    expires_at: Some(now),
                    reason: "late".to_owned(),
                },
                now,
            ),
            Err(StoreError::InvalidSocialInput)
        ));
        assert!(matches!(
            prepare_moderation(
                &ModerationAction::Unmute {
                    reason: "\u{7}".to_owned(),
                },
                now,
            ),
            Err(StoreError::InvalidStaffInput)
        ));
    }

    #[test]
    fn staff_audit_preparation_bounds_and_refuses_self_targets() {
        let record =
            |target: Option<i64>, action: &str, details: serde_json::Value| StaffAuditRecord {
                actor_account_id: AccountId(1),
                target_account_id: target.map(AccountId),
                capability: StaffCapability::StaffAssign,
                action: action.to_owned(),
                reason: " because ".to_owned(),
                details,
            };
        let prepared = prepare_staff_audit(record(Some(2), " assign ", serde_json::json!({})))
            .expect("valid audit");
        assert_eq!(prepared.action, "assign");
        assert_eq!(prepared.reason, "because");
        assert_eq!(prepared.details_json, "{}");
        assert!(matches!(
            prepare_staff_audit(record(Some(1), "assign", serde_json::json!({}))),
            Err(StoreError::StaffHierarchy)
        ));
        assert!(matches!(
            prepare_staff_audit(record(
                None,
                &"a".repeat(MAX_STAFF_ACTION_BYTES + 1),
                serde_json::json!({})
            )),
            Err(StoreError::InvalidStaffInput)
        ));
        assert!(matches!(
            prepare_staff_audit(record(
                None,
                "assign",
                serde_json::json!({"x": "y".repeat(MAX_AUDIT_DETAILS_BYTES)})
            )),
            Err(StoreError::InvalidStaffInput)
        ));
    }

    #[test]
    fn staff_access_rows_fail_closed() {
        let profile = profiles_by_rank()[0];
        assert_eq!(
            parse_staff_profile(Some(profile.as_str())).unwrap(),
            Some(profile)
        );
        assert_eq!(parse_staff_profile(None).unwrap(), None);
        assert!(matches!(
            parse_staff_profile(Some("emperor")),
            Err(StoreError::InvalidStaffState(_))
        ));

        let capability = StaffCapability::StaffAssign;
        let built = staff_access_from_rows(
            AccountId(5),
            "admin",
            Some(profile),
            Some("Keeper".to_owned()),
            [capability.as_str().to_owned()],
        )
        .unwrap();
        assert_eq!(built.role, AccountRole::Admin);
        assert!(built.capabilities.contains(&capability));
        assert!(matches!(
            staff_access_from_rows(
                AccountId(5),
                "player",
                None,
                None,
                [capability.as_str().to_owned()]
            ),
            Err(StoreError::InvalidStaffState(_))
        ));
        assert!(matches!(
            staff_access_from_rows(
                AccountId(5),
                "player",
                Some(profile),
                None,
                ["nonsense".to_owned()]
            ),
            Err(StoreError::InvalidStaffState(_))
        ));
        assert!(matches!(
            staff_access_from_rows(AccountId(5), "root", None, None, Vec::new()),
            Err(StoreError::InvalidSocialInput)
        ));
    }

    #[test]
    fn staff_authority_requires_capability_profile_and_strict_rank() {
        let ranked = profiles_by_rank();
        let low = ranked[0];
        let high = *ranked.last().unwrap();
        assert!(low.authority_rank() < high.authority_rank());
        let capability = StaffCapability::ReportsResolve;
        let actor = access(1, Some(high), &[capability]);
        let lower_target = access(2, Some(low), &[]);
        let equal_target = access(3, Some(high), &[]);
        let player_target = access(4, None, &[]);
        let load = |account: AccountId| -> Result<StaffAccess, StoreError> {
            Ok(match account.0 {
                1 => actor.clone(),
                2 => lower_target.clone(),
                3 => equal_target.clone(),
                4 => player_target.clone(),
                other => return Err(StoreError::AccountNotFound(AccountId(other))),
            })
        };

        for target in [2, 4] {
            let checked = validate_staff_authority_with(
                &audit(1, Some(target), capability),
                Some(capability),
                Some(AccountId(target)),
                load,
            )
            .unwrap();
            assert_eq!(checked.account_id, AccountId(1));
        }
        assert!(matches!(
            validate_staff_authority_with(
                &audit(1, Some(3), capability),
                Some(capability),
                Some(AccountId(3)),
                load
            ),
            Err(StoreError::StaffHierarchy)
        ));
        // Wrong expected capability or a mismatched target never reads rows.
        let refuse_reads = |_: AccountId| -> Result<StaffAccess, StoreError> {
            panic!("authorisation must fail before reading staff access")
        };
        assert!(matches!(
            validate_staff_authority_with(
                &audit(1, Some(2), capability),
                Some(StaffCapability::StaffAssign),
                Some(AccountId(2)),
                refuse_reads
            ),
            Err(StoreError::StaffForbidden)
        ));
        assert!(matches!(
            validate_staff_authority_with(
                &audit(1, Some(2), capability),
                Some(capability),
                Some(AccountId(4)),
                refuse_reads
            ),
            Err(StoreError::StaffForbidden)
        ));
        // An actor without the capability, or without a profile, is refused.
        let without = access(1, Some(high), &[]);
        assert!(matches!(
            validate_staff_authority_with(&audit(1, None, capability), None, None, |_| Ok(
                without.clone()
            )),
            Err(StoreError::StaffForbidden)
        ));
        let profileless = access(1, None, &[capability]);
        assert!(matches!(
            validate_staff_authority_with(&audit(1, None, capability), None, None, |_| Ok(
                profileless.clone()
            )),
            Err(StoreError::StaffForbidden)
        ));
    }

    #[test]
    fn report_audit_target_binds_or_refuses_mismatch() {
        let mut unbound = audit(1, None, StaffCapability::ReportsResolve);
        bind_report_audit_target(&mut unbound, AccountId(9)).unwrap();
        assert_eq!(unbound.target_account_id, Some(AccountId(9)));
        let mut matching = audit(1, Some(9), StaffCapability::ReportsResolve);
        bind_report_audit_target(&mut matching, AccountId(9)).unwrap();
        let mut mismatched = audit(1, Some(8), StaffCapability::ReportsResolve);
        assert!(matches!(
            bind_report_audit_target(&mut mismatched, AccountId(9)),
            Err(StoreError::StaffForbidden)
        ));
    }

    #[test]
    fn profile_and_capability_assignment_respect_actor_authority() {
        let ranked = profiles_by_rank();
        let low = ranked[0];
        let high = *ranked.last().unwrap();
        let all = StaffCapability::ALL;
        let actor = access(1, Some(high), &all);
        assert!(authorize_profile_assignment(&actor, None).is_ok());
        assert!(authorize_profile_assignment(&actor, Some(low)).is_ok());
        assert!(matches!(
            authorize_profile_assignment(&actor, Some(high)),
            Err(StoreError::StaffHierarchy)
        ));
        let missing_defaults = access(1, Some(high), &[]);
        if !low.default_capabilities().is_empty() {
            assert!(matches!(
                authorize_profile_assignment(&missing_defaults, Some(low)),
                Err(StoreError::StaffForbidden)
            ));
        }
        assert!(matches!(
            authorize_profile_assignment(&access(1, None, &[]), None),
            Err(StoreError::StaffForbidden)
        ));

        let capability = StaffCapability::ReportsResolve;
        let staff_target = access(2, Some(low), &[]);
        let player_target = access(3, None, &[]);
        assert!(authorize_capability_change(&actor, &staff_target, capability, true).is_ok());
        assert!(matches!(
            authorize_capability_change(&actor, &player_target, capability, true),
            Err(StoreError::StaffForbidden)
        ));
        assert!(matches!(
            authorize_capability_change(&missing_defaults, &staff_target, capability, true),
            Err(StoreError::StaffForbidden)
        ));
        assert!(
            authorize_capability_change(&missing_defaults, &player_target, capability, false)
                .is_ok()
        );
    }
}
