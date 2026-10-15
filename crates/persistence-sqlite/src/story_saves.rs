//! Backend-neutral row contract for persisted story save slots (schema v16,
//! ADR 0030).
//!
//! The SQLite and PostgreSQL DDL both encode these bounds as `CHECK`
//! constraints. This module is the single Rust definition of the same bounds
//! and of the row-integrity rule, so the server stores (through
//! `persistence/common.rs`) and the offline SQLite-to-PostgreSQL mover apply
//! exactly one policy. It is pure: no SQL, no I/O and no dependency on the
//! narrative ruleset. Re-validating a state against its StoryPack remains the
//! caller's job (`StoryState::from_json_slice`).

use sha2::{Digest, Sha256};
use thiserror::Error;

/// Slot reserved for the automatic save; manual slots are `1..=MAX_STORY_SLOT`.
pub const STORY_AUTOSAVE_SLOT: u8 = 0;
/// Highest slot number accepted by the schema (`CHECK(slot BETWEEN 0 AND 9)`).
pub const MAX_STORY_SLOT: u8 = 9;
/// Number of slots one account may hold for a single story.
pub const STORY_SLOTS_PER_STORY: usize = MAX_STORY_SLOT as usize + 1;
/// Story and pack identifiers use the StoryPack id grammar and byte bound.
pub const MAX_STORY_IDENTIFIER_BYTES: usize = 64;
/// Smallest accepted state document (`{}`); real states are much larger.
pub const MIN_STORY_SAVE_STATE_BYTES: usize = 2;
/// Exact byte bound of one stored state document.
pub const MAX_STORY_SAVE_STATE_BYTES: usize = 65_536;
/// Inclusive bounds of the stored `StoryState.format_version`.
pub const MIN_STORY_STATE_FORMAT_VERSION: u32 = 1;
pub const MAX_STORY_STATE_FORMAT_VERSION: u32 = 255;
/// Distinct stories for which one account may hold at least one slot.
pub const MAX_STORIES_WITH_SAVES_PER_ACCOUNT: usize = 16;
/// Aggregate state bytes one account may hold across every story and slot.
pub const MAX_STORY_SAVE_BYTES_PER_ACCOUNT: usize = 1_048_576;
/// Pack revisions a caller may declare compatible with the current one.
pub const MAX_COMPATIBLE_STORY_PACK_REVISIONS: usize = 16;

/// Why a story save row (or a row about to be written) is unacceptable.
#[derive(Clone, Copy, Debug, Eq, Error, PartialEq)]
pub enum StorySaveRowError {
    #[error("story or pack identifier is outside the bounded id grammar")]
    InvalidIdentifier,
    #[error("story save slot is outside 0..=9")]
    InvalidSlot,
    #[error("story pack digest is not 64 lowercase hexadecimal characters")]
    InvalidPackDigest,
    #[error("story state format version is outside 1..=255")]
    InvalidFormatVersion,
    #[error("story state document is outside its byte bounds")]
    StateSize,
    #[error("story state document is not a JSON object")]
    MalformedState,
    #[error("story state document disagrees with its row metadata")]
    MetadataMismatch,
    #[error("story state SHA-256 is malformed or does not match the exact bytes")]
    DigestMismatch,
    #[error("story save timestamps are negative or out of order")]
    InvalidTimestamps,
}

/// True for the StoryPack id grammar: 1..=64 bytes, an ASCII lowercase letter
/// or digit first, then lowercase letters, digits, `-`, `_` or `.`.
pub fn valid_story_identifier(id: &str) -> bool {
    let bytes = id.as_bytes();
    if bytes.is_empty() || bytes.len() > MAX_STORY_IDENTIFIER_BYTES {
        return false;
    }
    if !(bytes[0].is_ascii_lowercase() || bytes[0].is_ascii_digit()) {
        return false;
    }
    bytes
        .iter()
        .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || matches!(b, b'-' | b'_' | b'.'))
}

/// True for exactly 64 lowercase hexadecimal characters.
pub fn valid_sha256_hex(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

/// Lowercase hexadecimal SHA-256 of the exact state bytes.
pub fn story_state_sha256(state_json: &str) -> String {
    hex::encode(Sha256::digest(state_json.as_bytes()))
}

/// Converts a database slot to the bounded slot number.
pub fn story_slot_from_database(slot: i64) -> Result<u8, StorySaveRowError> {
    u8::try_from(slot)
        .ok()
        .filter(|slot| *slot <= MAX_STORY_SLOT)
        .ok_or(StorySaveRowError::InvalidSlot)
}

/// Validates the identity part of a save: story, slot and pack revision.
pub fn validate_story_save_identity(
    story_id: &str,
    slot: i64,
    pack_id: &str,
    pack_sha256: &str,
) -> Result<u8, StorySaveRowError> {
    if !valid_story_identifier(story_id) || !valid_story_identifier(pack_id) {
        return Err(StorySaveRowError::InvalidIdentifier);
    }
    let slot = story_slot_from_database(slot)?;
    if !valid_sha256_hex(pack_sha256) {
        return Err(StorySaveRowError::InvalidPackDigest);
    }
    Ok(slot)
}

/// Validates the state document against its metadata without the pack: byte
/// bounds, JSON object shape, `format_version`/`pack_id` agreement with the
/// row and the exact SHA-256 of the stored bytes.
pub fn validate_story_save_document(
    pack_id: &str,
    state_format_version: i64,
    state_json: &str,
    state_sha256: &str,
) -> Result<u32, StorySaveRowError> {
    let format_version = u32::try_from(state_format_version)
        .ok()
        .filter(|version| {
            (MIN_STORY_STATE_FORMAT_VERSION..=MAX_STORY_STATE_FORMAT_VERSION).contains(version)
        })
        .ok_or(StorySaveRowError::InvalidFormatVersion)?;
    if !(MIN_STORY_SAVE_STATE_BYTES..=MAX_STORY_SAVE_STATE_BYTES).contains(&state_json.len()) {
        return Err(StorySaveRowError::StateSize);
    }
    if !valid_sha256_hex(state_sha256) || story_state_sha256(state_json) != state_sha256 {
        return Err(StorySaveRowError::DigestMismatch);
    }
    let document: serde_json::Value =
        serde_json::from_str(state_json).map_err(|_| StorySaveRowError::MalformedState)?;
    let object = document
        .as_object()
        .ok_or(StorySaveRowError::MalformedState)?;
    let document_version = object
        .get("format_version")
        .and_then(serde_json::Value::as_u64);
    let document_pack = object.get("pack_id").and_then(serde_json::Value::as_str);
    if document_version != Some(u64::from(format_version)) || document_pack != Some(pack_id) {
        return Err(StorySaveRowError::MetadataMismatch);
    }
    Ok(format_version)
}

/// Validates one complete stored row: identity, document and timestamps.
#[allow(clippy::too_many_arguments)]
pub fn validate_story_save_row(
    story_id: &str,
    slot: i64,
    pack_id: &str,
    pack_sha256: &str,
    state_format_version: i64,
    state_json: &str,
    state_sha256: &str,
    created_at: i64,
    updated_at: i64,
) -> Result<(), StorySaveRowError> {
    validate_story_save_identity(story_id, slot, pack_id, pack_sha256)?;
    validate_story_save_document(pack_id, state_format_version, state_json, state_sha256)?;
    if created_at < 0 || updated_at < created_at {
        return Err(StorySaveRowError::InvalidTimestamps);
    }
    Ok(())
}

/// Why an account cannot accept one more save.
#[derive(Clone, Copy, Debug, Eq, Error, PartialEq)]
pub enum StorySaveCapacityError {
    #[error("the account already holds saves for the maximum number of stories")]
    TooManyStories,
    #[error("the account's aggregate story save bytes would exceed the limit")]
    TooManyBytes,
}

/// Decides whether an upsert fits the per-account caps.
///
/// `stories_with_saves` counts distinct stories the account holds now;
/// `story_has_saves` says whether the target story is one of them;
/// `account_bytes` is the current aggregate, `replaced_bytes` the size of the
/// slot being overwritten (0 for a new slot) and `added_bytes` the new size.
pub fn check_story_save_capacity(
    stories_with_saves: i64,
    story_has_saves: bool,
    account_bytes: i64,
    replaced_bytes: i64,
    added_bytes: i64,
) -> Result<(), StorySaveCapacityError> {
    if !story_has_saves && stories_with_saves >= MAX_STORIES_WITH_SAVES_PER_ACCOUNT as i64 {
        return Err(StorySaveCapacityError::TooManyStories);
    }
    let after = account_bytes
        .saturating_sub(replaced_bytes)
        .saturating_add(added_bytes);
    if after > MAX_STORY_SAVE_BYTES_PER_ACCOUNT as i64 {
        return Err(StorySaveCapacityError::TooManyBytes);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn state(pack: &str, version: u32) -> String {
        format!("{{\"format_version\":{version},\"pack_id\":\"{pack}\",\"node\":\"inicio\"}}")
    }

    #[test]
    fn identifiers_follow_the_story_pack_grammar() {
        assert!(valid_story_identifier("cripta-del-nigromante"));
        assert!(valid_story_identifier("9.a_b"));
        assert!(valid_story_identifier(&"a".repeat(64)));
        for invalid in ["", "-a", "Sala", "a b", "ñ", "a/b"] {
            assert!(!valid_story_identifier(invalid), "{invalid:?}");
        }
        assert!(!valid_story_identifier(&"a".repeat(65)));
    }

    #[test]
    fn identity_rejects_slots_ids_and_digests_out_of_bounds() {
        let digest = "a".repeat(64);
        assert_eq!(
            validate_story_save_identity("s", 0, "p", &digest),
            Ok(STORY_AUTOSAVE_SLOT)
        );
        assert_eq!(validate_story_save_identity("s", 9, "p", &digest), Ok(9));
        for slot in [-1, 10, i64::MAX, i64::MIN] {
            assert_eq!(
                validate_story_save_identity("s", slot, "p", &digest),
                Err(StorySaveRowError::InvalidSlot)
            );
        }
        assert_eq!(
            validate_story_save_identity("S", 0, "p", &digest),
            Err(StorySaveRowError::InvalidIdentifier)
        );
        for bad in ["A".repeat(64), "a".repeat(63), "g".repeat(64)] {
            assert_eq!(
                validate_story_save_identity("s", 0, "p", &bad),
                Err(StorySaveRowError::InvalidPackDigest)
            );
        }
    }

    #[test]
    fn document_must_match_metadata_size_and_exact_digest() {
        let json = state("p", 2);
        let digest = story_state_sha256(&json);
        assert_eq!(validate_story_save_document("p", 2, &json, &digest), Ok(2));
        assert_eq!(
            validate_story_save_document("q", 2, &json, &digest),
            Err(StorySaveRowError::MetadataMismatch)
        );
        assert_eq!(
            validate_story_save_document("p", 1, &json, &digest),
            Err(StorySaveRowError::MetadataMismatch)
        );
        for version in [0, 256, -1] {
            assert_eq!(
                validate_story_save_document("p", version, &json, &digest),
                Err(StorySaveRowError::InvalidFormatVersion)
            );
        }
        let flipped = json.replace("inicio", "inicia");
        assert_eq!(
            validate_story_save_document("p", 2, &flipped, &digest),
            Err(StorySaveRowError::DigestMismatch)
        );
        assert_eq!(
            validate_story_save_document("p", 2, &json, &digest.to_uppercase()),
            Err(StorySaveRowError::DigestMismatch)
        );
        let array = "[1,2]";
        assert_eq!(
            validate_story_save_document("p", 2, array, &story_state_sha256(array)),
            Err(StorySaveRowError::MalformedState)
        );
        let broken = "{\"format_version\":2";
        assert_eq!(
            validate_story_save_document("p", 2, broken, &story_state_sha256(broken)),
            Err(StorySaveRowError::MalformedState)
        );
        let huge = format!(
            "{{\"format_version\":2,\"pack_id\":\"p\",\"x\":\"{}\"}}",
            "a".repeat(MAX_STORY_SAVE_STATE_BYTES)
        );
        assert_eq!(
            validate_story_save_document("p", 2, &huge, &story_state_sha256(&huge)),
            Err(StorySaveRowError::StateSize)
        );
        assert_eq!(
            validate_story_save_document("p", 2, "", &story_state_sha256("")),
            Err(StorySaveRowError::StateSize)
        );
    }

    #[test]
    fn rows_require_ordered_non_negative_timestamps() {
        let json = state("p", 2);
        let digest = story_state_sha256(&json);
        let pack = "b".repeat(64);
        assert!(validate_story_save_row("s", 3, "p", &pack, 2, &json, &digest, 10, 10).is_ok());
        assert_eq!(
            validate_story_save_row("s", 3, "p", &pack, 2, &json, &digest, 10, 9),
            Err(StorySaveRowError::InvalidTimestamps)
        );
        assert_eq!(
            validate_story_save_row("s", 3, "p", &pack, 2, &json, &digest, -1, 0),
            Err(StorySaveRowError::InvalidTimestamps)
        );
    }

    #[test]
    fn capacity_counts_new_stories_and_replaced_bytes() {
        let max_stories = MAX_STORIES_WITH_SAVES_PER_ACCOUNT as i64;
        let max_bytes = MAX_STORY_SAVE_BYTES_PER_ACCOUNT as i64;
        assert!(check_story_save_capacity(max_stories - 1, false, 0, 0, 10).is_ok());
        assert_eq!(
            check_story_save_capacity(max_stories, false, 0, 0, 10),
            Err(StorySaveCapacityError::TooManyStories)
        );
        assert!(check_story_save_capacity(max_stories, true, 0, 0, 10).is_ok());
        assert!(check_story_save_capacity(1, true, max_bytes - 10, 0, 10).is_ok());
        assert_eq!(
            check_story_save_capacity(1, true, max_bytes - 10, 0, 11),
            Err(StorySaveCapacityError::TooManyBytes)
        );
        assert!(check_story_save_capacity(1, true, max_bytes, 100, 100).is_ok());
        assert!(check_story_save_capacity(1, true, i64::MAX, 0, i64::MAX).is_err());
    }
}
