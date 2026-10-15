//! Transport-independent world model for Tiamud.

mod ability;
mod actions;
mod adventure;
mod areas;
mod character;
mod cow;
mod definitions;
mod durability;
mod emote;
mod events;
mod gameplay;
mod items;
mod loot;
pub mod loot_distribution;
mod model;
mod parse;
mod perception;
mod persistence;
mod progression;
mod projection;
mod replay;
mod runtime;
mod sessions;
mod snapshot;
mod tick;
mod trade;
mod validation;
mod views;
mod world;

pub use ability::*;
pub use actions::*;
use adventure::*;
pub use areas::*;
use cow::*;
pub use definitions::*;
pub use durability::DURABLE_CLOCK_HEARTBEAT_TICKS;
use durability::DurableBaseline;
pub use emote::*;
pub use events::*;
use gameplay::*;
use items::*;
pub use loot::*;
use model::*;
pub use parse::*;
use perception::*;
pub use progression::*;
use projection::*;
pub use runtime::*;
pub use snapshot::*;
pub use trade::*;
pub use validation::*;
pub use views::*;
pub use world::*;

use rand_core::{OsRng, RngCore};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use slotmap::{SlotMap, new_key_type};
use std::borrow::Borrow;
use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet, VecDeque};
use std::fmt::Write as _;
use thiserror::Error;

/// `MAX_STAT_VALUE` (the stat cap) expressed once as `i32`. The cap sits far
/// below `i32::MAX`, so the conversion is lossless; the const assertion below
/// enforces that invariant at compile time.
const MAX_STAT_VALUE_I32: i32 = MAX_STAT_VALUE as i32;
const _: () = assert!(MAX_STAT_VALUE <= i32::MAX as u32);

/// Zero-based page index for a one-based `page` selector. Callers that must
/// reject `page == 0` still do so explicitly before calling this; the
/// `saturating_sub` only keeps the index computation itself underflow-free.
fn page_index(page: u16) -> usize {
    usize::from(page.saturating_sub(1))
}

new_key_type! {
    /// Internal, generational entity identifier. Never exposed as a persistent ID.
    struct EntityId;
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct SessionId(pub u64);

/// Durable database identity for an account.
///
/// Unlike [`SessionId`], this value survives reconnects and server restarts.
#[derive(Clone, Copy, Debug, Deserialize, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(transparent)]
pub struct AccountId(pub i64);

/// Durable database identity for a character.
///
/// Character names are presentation data and are not used as identity.
#[derive(Clone, Copy, Debug, Deserialize, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(transparent)]
pub struct CharacterId(pub i64);

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct AccountSnapshot {
    pub id: AccountId,
    pub name: String,
}

#[cfg(test)]
mod tests;
