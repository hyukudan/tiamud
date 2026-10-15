//! Durable dirty tracking for the server's commit boundary.
//!
//! The world remembers what its single backend last confirmed: copy-on-write
//! handles of every value that the portable runtime snapshot is built from,
//! the small scalar counters, and the exact durable aggregate last written
//! for each connected character. The server uses this to skip a tick commit
//! when nothing durable changed and to write only changed characters.
//!
//! Correctness rests on three rules:
//!
//! - every mutable access to a tracked map goes through `DerefMut` on a
//!   copy-on-write handle, and while the baseline holds a reference that
//!   access produces a new allocation, so pointer identity with the baseline
//!   proves "unchanged" and anything else is treated as dirty;
//! - the baseline is only advanced after the backend confirmed a commit with
//!   exactly the values it records;
//! - any failed or ambiguous commit, snapshot restore or lifecycle rollback
//!   drops the baseline, so the next commit rewrites everything.
//!
//! The logical clock and world revision are deliberately not dirtying on
//! their own. A clean tick therefore leaves the durable clock behind the
//! in-memory one; [`DURABLE_CLOCK_HEARTBEAT_TICKS`] bounds that lag, and
//! ADR 0027 records the trade-off.

use super::*;

/// Maximum number of logical ticks the durable runtime snapshot may lag the
/// in-memory clock when nothing else changed. A crash can rewind the clock by
/// at most this many ticks; graceful shutdown flushes it.
pub const DURABLE_CLOCK_HEARTBEAT_TICKS: u64 = 60;

/// One independently stored part of the portable runtime snapshot (schema
/// v17, ADR 0035). Each variant is exactly one top-level member of the
/// serialized `WorldRuntimeSnapshot`; the remaining small scalar members form
/// the head document that every runtime commit rewrites.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub enum RuntimeSection {
    ItemInstances,
    ItemProvenance,
    Npcs,
    Adventure,
    ProgressionFlags,
    Trades,
    LootEncounters,
    GroundLoot,
    PartyLootCursors,
    AreaResidency,
}

impl RuntimeSection {
    /// Every section in canonical storage order.
    pub const ALL: [Self; 10] = [
        Self::ItemInstances,
        Self::ItemProvenance,
        Self::Npcs,
        Self::Adventure,
        Self::ProgressionFlags,
        Self::Trades,
        Self::LootEncounters,
        Self::GroundLoot,
        Self::PartyLootCursors,
        Self::AreaResidency,
    ];

    /// The camelCase member name of this section in the serialized snapshot,
    /// which is also its durable storage key.
    pub const fn field_name(self) -> &'static str {
        match self {
            Self::ItemInstances => "itemInstances",
            Self::ItemProvenance => "itemProvenance",
            Self::Npcs => "npcs",
            Self::Adventure => "adventure",
            Self::ProgressionFlags => "progressionFlags",
            Self::Trades => "trades",
            Self::LootEncounters => "lootEncounters",
            Self::GroundLoot => "groundLoot",
            Self::PartyLootCursors => "partyLootCursors",
            Self::AreaResidency => "areaResidency",
        }
    }

    const fn bit(self) -> u16 {
        1 << self as u16
    }
}

/// A set of [`RuntimeSection`]s. "Unknown" must always be represented by
/// [`RuntimeSectionSet::ALL`]: a section missing from a dirty set is only
/// skipped when the durable baseline proves it unchanged.
#[derive(Clone, Copy, Debug, Default, Eq, Hash, PartialEq)]
pub struct RuntimeSectionSet(u16);

impl RuntimeSectionSet {
    pub const EMPTY: Self = Self(0);
    pub const ALL: Self = Self((1 << RuntimeSection::ALL.len()) - 1);

    pub const fn contains(self, section: RuntimeSection) -> bool {
        self.0 & section.bit() != 0
    }

    pub fn insert(&mut self, section: RuntimeSection) {
        self.0 |= section.bit();
    }

    pub const fn is_empty(self) -> bool {
        self.0 == 0
    }

    pub const fn is_all(self) -> bool {
        self.0 == Self::ALL.0
    }

    pub const fn len(self) -> usize {
        self.0.count_ones() as usize
    }

    pub fn iter(self) -> impl Iterator<Item = RuntimeSection> {
        RuntimeSection::ALL
            .into_iter()
            .filter(move |section| self.contains(*section))
    }
}

impl FromIterator<RuntimeSection> for RuntimeSectionSet {
    fn from_iter<I: IntoIterator<Item = RuntimeSection>>(iter: I) -> Self {
        let mut set = Self::EMPTY;
        for section in iter {
            set.insert(section);
        }
        set
    }
}

#[derive(Clone, Default)]
pub(crate) struct DurableBaseline {
    runtime: Option<RuntimeBaseline>,
    /// Exact aggregate last confirmed for a character. Absent means unknown,
    /// which always forces the next write.
    characters: HashMap<CharacterId, DurableCharacterSnapshot>,
    /// Handles of the character inputs at the last reconciliation of every
    /// connected character. Equal handles prove no character changed.
    character_gate: Option<CharacterGate>,
}

#[derive(Clone)]
struct RuntimeBaseline {
    item_instances: ItemInstances,
    item_provenance: Shared<BTreeMap<ItemInstanceId, ItemProvenance>>,
    npcs: Shared<BTreeMap<String, Npc>>,
    adventure: Shared<AdventureRuntimeSnapshot>,
    world_progression_flags: Shared<BTreeSet<String>>,
    trades: Shared<BTreeMap<TradeId, TradeSnapshot>>,
    loot_encounters: Shared<BTreeMap<String, LootEncounter>>,
    ground_loot: Shared<BTreeMap<ItemInstanceId, GroundLootReservation>>,
    party_loot_cursors: Shared<BTreeMap<u64, u64>>,
    rng_state: u64,
    next_item_instance_sequence: u64,
    next_trade_id: TradeId,
    next_loot_drop_id: LootDropId,
    next_reward_open_id: RewardOpenId,
    active_areas: BTreeMap<String, ActiveAreaRevision>,
    retired_areas: BTreeMap<String, RetiredAreaRevision>,
    logical_tick: u64,
}

#[derive(Clone)]
struct CharacterGate {
    entities: Shared<SlotMap<EntityId, Entity>>,
    character_gameplay: Shared<HashMap<CharacterId, CharacterGameplaySnapshot>>,
}

impl World {
    fn runtime_baseline(&self) -> RuntimeBaseline {
        RuntimeBaseline {
            item_instances: self.item_instances.clone(),
            item_provenance: self.item_provenance.clone(),
            npcs: self.npcs.clone(),
            adventure: self.adventure.clone(),
            world_progression_flags: self.world_progression_flags.clone(),
            trades: self.trades.clone(),
            loot_encounters: self.loot_encounters.clone(),
            ground_loot: self.ground_loot.clone(),
            party_loot_cursors: self.party_loot_cursors.clone(),
            rng_state: self.rng.state,
            next_item_instance_sequence: self.next_item_instance_sequence,
            next_trade_id: self.next_trade_id,
            next_loot_drop_id: self.next_loot_drop_id,
            next_reward_open_id: self.next_reward_open_id,
            active_areas: self.active_areas.clone(),
            retired_areas: self.retired_areas.clone(),
            logical_tick: self.logical_tick,
        }
    }

    /// True when the portable runtime snapshot may differ from the last one
    /// the backend confirmed, ignoring only the logical clock and revision.
    pub fn runtime_durably_dirty(&self) -> bool {
        !self.runtime_dirty_reasons().is_empty()
    }

    /// Names of the runtime inputs that differ from the durable baseline.
    /// Diagnostic only; `["baseline"]` means nothing is confirmed yet.
    #[doc(hidden)]
    pub fn runtime_dirty_reasons(&self) -> Vec<&'static str> {
        let Some(baseline) = &self.durable.runtime else {
            return vec!["baseline"];
        };
        [
            (
                "item_instances",
                baseline.item_instances.ptr_eq(&self.item_instances),
            ),
            (
                "item_provenance",
                baseline.item_provenance.ptr_eq(&self.item_provenance),
            ),
            ("npcs", baseline.npcs.ptr_eq(&self.npcs)),
            ("adventure", baseline.adventure.ptr_eq(&self.adventure)),
            (
                "progression_flags",
                baseline
                    .world_progression_flags
                    .ptr_eq(&self.world_progression_flags),
            ),
            ("trades", baseline.trades.ptr_eq(&self.trades)),
            (
                "loot_encounters",
                baseline.loot_encounters.ptr_eq(&self.loot_encounters),
            ),
            (
                "ground_loot",
                baseline.ground_loot.ptr_eq(&self.ground_loot),
            ),
            (
                "party_loot_cursors",
                baseline.party_loot_cursors.ptr_eq(&self.party_loot_cursors),
            ),
            ("rng", baseline.rng_state == self.rng.state),
            (
                "next_item_instance_sequence",
                baseline.next_item_instance_sequence == self.next_item_instance_sequence,
            ),
            (
                "next_trade_id",
                baseline.next_trade_id == self.next_trade_id,
            ),
            (
                "next_loot_drop_id",
                baseline.next_loot_drop_id == self.next_loot_drop_id,
            ),
            (
                "next_reward_open_id",
                baseline.next_reward_open_id == self.next_reward_open_id,
            ),
            ("active_areas", baseline.active_areas == self.active_areas),
            (
                "retired_areas",
                baseline.retired_areas == self.retired_areas,
            ),
        ]
        .into_iter()
        .filter_map(|(name, clean)| (!clean).then_some(name))
        .collect()
    }

    /// Runtime snapshot sections that may differ from what the backend last
    /// confirmed. Without a confirmed baseline every section is dirty. Scalar
    /// counters, the clock and the revision live in the head document, which
    /// every runtime commit rewrites, so they are not sections.
    pub fn runtime_dirty_sections(&self) -> RuntimeSectionSet {
        let Some(baseline) = &self.durable.runtime else {
            return RuntimeSectionSet::ALL;
        };
        [
            (
                RuntimeSection::ItemInstances,
                baseline.item_instances.ptr_eq(&self.item_instances),
            ),
            (
                RuntimeSection::ItemProvenance,
                baseline.item_provenance.ptr_eq(&self.item_provenance),
            ),
            (RuntimeSection::Npcs, baseline.npcs.ptr_eq(&self.npcs)),
            (
                RuntimeSection::Adventure,
                baseline.adventure.ptr_eq(&self.adventure),
            ),
            (
                RuntimeSection::ProgressionFlags,
                baseline
                    .world_progression_flags
                    .ptr_eq(&self.world_progression_flags),
            ),
            (RuntimeSection::Trades, baseline.trades.ptr_eq(&self.trades)),
            (
                RuntimeSection::LootEncounters,
                baseline.loot_encounters.ptr_eq(&self.loot_encounters),
            ),
            (
                RuntimeSection::GroundLoot,
                baseline.ground_loot.ptr_eq(&self.ground_loot),
            ),
            (
                RuntimeSection::PartyLootCursors,
                baseline.party_loot_cursors.ptr_eq(&self.party_loot_cursors),
            ),
            (
                RuntimeSection::AreaResidency,
                baseline.active_areas == self.active_areas
                    && baseline.retired_areas == self.retired_areas,
            ),
        ]
        .into_iter()
        .filter_map(|(section, clean)| (!clean).then_some(section))
        .collect()
    }

    /// True when a commit must include the runtime snapshot: either durable
    /// state changed or the confirmed clock lags by the heartbeat bound.
    pub fn runtime_needs_durable_commit(&self) -> bool {
        self.runtime_durably_dirty()
            || self.durable.runtime.as_ref().is_none_or(|baseline| {
                self.logical_tick.saturating_sub(baseline.logical_tick)
                    >= DURABLE_CLOCK_HEARTBEAT_TICKS
                    || self.logical_tick < baseline.logical_tick
            })
    }

    /// True when the in-memory clock differs from the last confirmed one.
    pub fn durable_clock_lags(&self) -> bool {
        self.durable
            .runtime
            .as_ref()
            .is_none_or(|baseline| baseline.logical_tick != self.logical_tick)
    }

    fn character_gate_holds(&self) -> bool {
        self.durable.character_gate.as_ref().is_some_and(|gate| {
            gate.entities.ptr_eq(&self.entities)
                && gate.character_gameplay.ptr_eq(&self.character_gameplay)
        })
    }

    /// Durable aggregates of connected characters that differ from what the
    /// backend last confirmed, in session order. Unknown characters are
    /// always included.
    pub fn durable_character_changes(&self) -> Result<Vec<DurableCharacterSnapshot>, WorldError> {
        let sessions = self.connected_sessions();
        if self.character_gate_holds()
            && sessions.iter().all(|session| {
                self.character(*session)
                    .is_ok_and(|character| self.durable.characters.contains_key(&character.id))
            })
        {
            return Ok(Vec::new());
        }
        let mut changed = Vec::new();
        for session in sessions {
            let snapshot = self.durable_character_snapshot(session)?;
            if self.durable.characters.get(&snapshot.character.id) != Some(&snapshot) {
                changed.push(snapshot);
            }
        }
        Ok(changed)
    }

    /// Records a commit the backend confirmed. `runtime_committed` means the
    /// snapshot built from the current state was part of it; `characters`
    /// are the exact aggregates written; `all_connected_reconciled` means
    /// the caller compared every connected character against the baseline
    /// (via [`World::durable_character_changes`]) immediately before it.
    pub fn mark_durable_commit(
        &mut self,
        runtime_committed: bool,
        characters: Vec<DurableCharacterSnapshot>,
        all_connected_reconciled: bool,
    ) {
        if runtime_committed {
            self.durable.runtime = Some(self.runtime_baseline());
        }
        for snapshot in characters {
            self.durable
                .characters
                .insert(snapshot.character.id, snapshot);
        }
        if all_connected_reconciled {
            let connected = self
                .sessions
                .values()
                .filter_map(|entity| match self.entities.get(*entity) {
                    Some(Entity::Character(character)) => Some(character.id),
                    _ => None,
                })
                .collect::<HashSet<_>>();
            // Forget disconnected characters: whatever happens to their rows
            // while offline, a reconnect starts from an unknown baseline.
            self.durable
                .characters
                .retain(|character_id, _| connected.contains(character_id));
            self.durable.character_gate = Some(CharacterGate {
                entities: self.entities.clone(),
                character_gameplay: self.character_gameplay.clone(),
            });
        } else {
            self.durable.character_gate = None;
        }
    }

    /// Forgets everything the backend confirmed. Required after any failed
    /// or ambiguous commit, so the next commit rewrites the full state.
    pub fn invalidate_durable_baseline(&mut self) {
        self.durable = DurableBaseline::default();
    }
}
