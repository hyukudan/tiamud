//! Public types of the persisted story save slots (schema v16, ADR 0030).
//!
//! The store keeps a bounded, opaque state document per
//! `(account, story, slot)` and proves its integrity with the SHA-256 of the
//! exact stored bytes. It never interprets the narrative state: the caller
//! (the S2 story instance manager) re-validates the returned bytes against
//! the loaded StoryPack with `StoryState::from_json_slice` before use.
//! Bounds, validation and capacity decisions live in [`super::common`] and in
//! `tiamud_persistence_sqlite::story_saves`, shared by both backends and the
//! offline mover.

use std::collections::BTreeSet;

use tiamud_core::AccountId;
use tiamud_persistence_sqlite::story_saves::{MAX_STORY_SLOT, STORY_AUTOSAVE_SLOT};

/// A bounded save slot: `0` is the autosave and `1..=9` are manual slots.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct StorySlot(u8);

#[allow(dead_code)] // Wired by the S2 story instance manager.
impl StorySlot {
    pub const AUTOSAVE: Self = Self(STORY_AUTOSAVE_SLOT);

    /// Returns the slot when it is inside the schema bound `0..=9`.
    pub const fn new(slot: u8) -> Option<Self> {
        if slot <= MAX_STORY_SLOT {
            Some(Self(slot))
        } else {
            None
        }
    }

    pub const fn get(self) -> u8 {
        self.0
    }

    pub const fn is_autosave(self) -> bool {
        self.0 == STORY_AUTOSAVE_SLOT
    }
}

/// One atomic slot write. The store computes the SHA-256 itself; the caller
/// only supplies the exact bytes `StoryState::to_json` produced and the
/// identity of the pack revision the state was produced with.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct StorySaveWrite {
    pub account_id: AccountId,
    pub story_id: String,
    pub slot: StorySlot,
    pub pack_id: String,
    /// Lowercase hexadecimal SHA-256 of the exact StoryPack bytes in use.
    pub pack_sha256: String,
    pub state_format_version: u32,
    pub state_json: String,
}

/// Metadata of one stored slot. Listing reads only these columns, never the
/// state document.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct StorySaveSummary {
    pub story_id: String,
    pub slot: StorySlot,
    pub pack_id: String,
    pub pack_sha256: String,
    pub state_format_version: u32,
    pub state_bytes: u32,
    pub state_sha256: String,
    pub created_at: i64,
    pub updated_at: i64,
}

/// A loaded slot whose stored bytes matched their SHA-256 and metadata and
/// whose pack revision is compatible with the caller's pack.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct StorySaveRecord {
    pub summary: StorySaveSummary,
    pub state_json: String,
}

/// The pack revision a slot is about to be loaded into.
///
/// Loading fails closed with `StoreError::StorySavePackMismatch` unless the
/// slot's pack id equals `pack_id` and its digest equals `pack_sha256` or is
/// listed in `compatible_pack_sha256` (at most 16 entries). StoryPack has no
/// compatibility declaration yet, so today callers pass an empty set; ADR
/// 0030 reserves the declaration for a later StoryPack format.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct StoryPackCompatibility {
    pub pack_id: String,
    pub pack_sha256: String,
    pub compatible_pack_sha256: BTreeSet<String>,
}
