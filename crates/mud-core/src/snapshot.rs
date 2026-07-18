//! World runtime snapshot DTOs and the versioned upgrade chain.
//!
//! These types are the durable persistence surface for the world actor. They
//! were extracted verbatim from `lib.rs`; `use super::*;` gives them the crate's
//! shared types and helpers, and `lib.rs` re-exports them via `pub use snapshot::*;`.

use super::*;

/// Exact snapshot shape written before per-area durable authority existed.
/// Upgrading v7 is intentionally not implicit: a catalog runtime must supply
/// the exact resident area manifest, while a monolithic runtime supplies an
/// empty manifest.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct WorldRuntimeSnapshotV7 {
    pub snapshot_version: u32,
    pub revision: u64,
    pub logical_tick: u64,
    pub rng_state: u64,
    pub next_item_instance_sequence: u64,
    pub item_instances: BTreeMap<ItemInstanceId, RuntimeItemInstanceSnapshot>,
    pub npcs: BTreeMap<String, NpcRuntimeSnapshot>,
    #[serde(default)]
    pub adventure: AdventureRuntimeSnapshot,
    #[serde(default)]
    pub progression_flags: BTreeSet<String>,
    pub next_trade_id: TradeId,
    pub trades: BTreeMap<TradeId, TradeSnapshot>,
    pub next_loot_drop_id: LootDropId,
    pub item_provenance: BTreeMap<ItemInstanceId, ItemProvenance>,
    pub loot_encounters: BTreeMap<String, LootEncounterV8>,
    pub ground_loot: BTreeMap<ItemInstanceId, GroundLootReservation>,
    pub party_loot_cursors: BTreeMap<u64, u64>,
    pub next_reward_open_id: RewardOpenId,
}

/// Portable shared-world state. It deliberately excludes sessions and opaque
/// observer IDs, which are transport lifetime state and are rebuilt on start.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct WorldRuntimeSnapshot {
    pub snapshot_version: u32,
    pub revision: u64,
    pub logical_tick: u64,
    pub rng_state: u64,
    pub next_item_instance_sequence: u64,
    pub item_instances: BTreeMap<ItemInstanceId, RuntimeItemInstanceSnapshot>,
    pub npcs: BTreeMap<String, NpcRuntimeSnapshot>,
    #[serde(default)]
    pub adventure: AdventureRuntimeSnapshot,
    #[serde(default)]
    pub progression_flags: BTreeSet<String>,
    pub next_trade_id: TradeId,
    pub trades: BTreeMap<TradeId, TradeSnapshot>,
    pub next_loot_drop_id: LootDropId,
    pub item_provenance: BTreeMap<ItemInstanceId, ItemProvenance>,
    pub loot_encounters: BTreeMap<String, LootEncounter>,
    pub ground_loot: BTreeMap<ItemInstanceId, GroundLootReservation>,
    pub party_loot_cursors: BTreeMap<u64, u64>,
    pub next_reward_open_id: RewardOpenId,
    pub area_residency: BTreeMap<String, AreaRuntimeResidence>,
}

/// Exact snapshot shape produced through additive area loading (v8). It carries
/// the area residency manifest of v8 but predates the v9 group-distribution
/// loot fields, so its encounters use [`LootEncounterV8`] and reject any v9
/// field via `deny_unknown_fields`.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct WorldRuntimeSnapshotV8 {
    pub snapshot_version: u32,
    pub revision: u64,
    pub logical_tick: u64,
    pub rng_state: u64,
    pub next_item_instance_sequence: u64,
    pub item_instances: BTreeMap<ItemInstanceId, RuntimeItemInstanceSnapshot>,
    pub npcs: BTreeMap<String, NpcRuntimeSnapshot>,
    #[serde(default)]
    pub adventure: AdventureRuntimeSnapshot,
    #[serde(default)]
    pub progression_flags: BTreeSet<String>,
    pub next_trade_id: TradeId,
    pub trades: BTreeMap<TradeId, TradeSnapshot>,
    pub next_loot_drop_id: LootDropId,
    pub item_provenance: BTreeMap<ItemInstanceId, ItemProvenance>,
    pub loot_encounters: BTreeMap<String, LootEncounterV8>,
    pub ground_loot: BTreeMap<ItemInstanceId, GroundLootReservation>,
    pub party_loot_cursors: BTreeMap<u64, u64>,
    pub next_reward_open_id: RewardOpenId,
    pub area_residency: BTreeMap<String, AreaRuntimeResidence>,
}

/// Exact snapshot shape produced by repeatable-loot phase 4A. It cannot opt
/// into affixes, sealed provenance or the reward-open counter introduced by
/// v7.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct WorldRuntimeSnapshotV6 {
    pub snapshot_version: u32,
    pub revision: u64,
    pub logical_tick: u64,
    pub rng_state: u64,
    pub next_item_instance_sequence: u64,
    pub item_instances: BTreeMap<ItemInstanceId, RuntimeItemInstanceSnapshotV6>,
    pub npcs: BTreeMap<String, NpcRuntimeSnapshot>,
    #[serde(default)]
    pub adventure: AdventureRuntimeSnapshot,
    #[serde(default)]
    pub progression_flags: BTreeSet<String>,
    pub next_trade_id: TradeId,
    pub trades: BTreeMap<TradeId, TradeSnapshot>,
    pub next_loot_drop_id: LootDropId,
    pub item_provenance: BTreeMap<ItemInstanceId, ItemProvenanceV6>,
    pub loot_encounters: BTreeMap<String, LootEncounterV8>,
    pub ground_loot: BTreeMap<ItemInstanceId, GroundLootReservation>,
    pub party_loot_cursors: BTreeMap<u64, u64>,
}

/// Exact snapshot shape produced by inventory phase 3D. It intentionally has
/// no phase-4 loot fields; v5 inputs receive conservative legacy provenance.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct WorldRuntimeSnapshotV5 {
    pub snapshot_version: u32,
    pub revision: u64,
    pub logical_tick: u64,
    pub rng_state: u64,
    pub next_item_instance_sequence: u64,
    pub item_instances: BTreeMap<ItemInstanceId, RuntimeItemInstanceSnapshotV6>,
    pub npcs: BTreeMap<String, NpcRuntimeSnapshot>,
    #[serde(default)]
    pub adventure: AdventureRuntimeSnapshot,
    #[serde(default)]
    pub progression_flags: BTreeSet<String>,
    pub next_trade_id: TradeId,
    pub trades: BTreeMap<TradeId, TradeSnapshot>,
}

/// Exact snapshot shape produced by inventory phase 3C. It intentionally has
/// no trade fields; v4 payloads may only obtain an empty trade state through
/// the explicit v4 -> v5 conversion.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct WorldRuntimeSnapshotV4 {
    pub snapshot_version: u32,
    pub revision: u64,
    pub logical_tick: u64,
    pub rng_state: u64,
    pub next_item_instance_sequence: u64,
    pub item_instances: BTreeMap<ItemInstanceId, RuntimeItemInstanceSnapshotV6>,
    pub npcs: BTreeMap<String, NpcRuntimeSnapshot>,
    #[serde(default)]
    pub adventure: AdventureRuntimeSnapshot,
    #[serde(default)]
    pub progression_flags: BTreeSet<String>,
}

/// Exact snapshot shape produced by inventory phase 3B.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct WorldRuntimeSnapshotV3 {
    pub snapshot_version: u32,
    pub revision: u64,
    pub logical_tick: u64,
    pub rng_state: u64,
    pub next_item_instance_sequence: u64,
    pub item_instances: BTreeMap<ItemInstanceId, RuntimeItemInstanceSnapshotV3>,
    pub npcs: BTreeMap<String, NpcRuntimeSnapshot>,
    #[serde(default)]
    pub adventure: AdventureRuntimeSnapshot,
    #[serde(default)]
    pub progression_flags: BTreeSet<String>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RuntimeItemInstanceSnapshotV2 {
    pub prototype_id: String,
    pub location: RuntimeItemLocationV2,
}

/// Exact snapshot shape produced by inventory phase 3A.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct WorldRuntimeSnapshotV2 {
    pub snapshot_version: u32,
    pub revision: u64,
    pub logical_tick: u64,
    pub rng_state: u64,
    pub next_item_instance_sequence: u64,
    pub item_instances: BTreeMap<ItemInstanceId, RuntimeItemInstanceSnapshotV2>,
    pub npcs: BTreeMap<String, NpcRuntimeSnapshot>,
    #[serde(default)]
    pub adventure: AdventureRuntimeSnapshot,
    #[serde(default)]
    pub progression_flags: BTreeSet<String>,
}

/// Exact portable shape written before the prototype/instance split.
/// Callers must inspect `snapshot_version` and opt into this conversion rather
/// than allowing missing v2 fields to deserialize as an empty inventory.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct WorldRuntimeSnapshotV1 {
    pub snapshot_version: u32,
    pub revision: u64,
    pub logical_tick: u64,
    pub rng_state: u64,
    pub item_locations: BTreeMap<String, RuntimeItemLocationV2>,
    pub npcs: BTreeMap<String, NpcRuntimeSnapshot>,
    #[serde(default)]
    pub adventure: AdventureRuntimeSnapshot,
    #[serde(default)]
    pub progression_flags: BTreeSet<String>,
}

impl WorldRuntimeSnapshot {
    pub fn upgrade_v1(snapshot: WorldRuntimeSnapshotV1) -> Result<Self, WorldError> {
        let mut upgraded = Self::upgrade_v2(WorldRuntimeSnapshotV2::upgrade_v1(snapshot)?)?;
        for provenance in upgraded.item_provenance.values_mut() {
            *provenance = ItemProvenance::LegacySnapshot { source_version: 1 };
        }
        Ok(upgraded)
    }

    pub fn upgrade_v2(snapshot: WorldRuntimeSnapshotV2) -> Result<Self, WorldError> {
        if snapshot.snapshot_version != 2 {
            return Err(WorldError::InvalidRuntimeSnapshot);
        }
        let item_instances = snapshot
            .item_instances
            .into_iter()
            .map(|(item_id, instance)| {
                (
                    item_id,
                    RuntimeItemInstanceSnapshot {
                        prototype_id: instance.prototype_id,
                        location: instance.location.into(),
                        binding: ItemBinding::Unbound,
                        item_affixes: Vec::new(),
                    },
                )
            })
            .collect::<BTreeMap<_, _>>();
        let item_provenance = legacy_item_provenance(&item_instances, 2);
        Ok(Self {
            snapshot_version: WORLD_RUNTIME_SNAPSHOT_VERSION,
            revision: snapshot.revision,
            logical_tick: snapshot.logical_tick,
            rng_state: snapshot.rng_state,
            next_item_instance_sequence: snapshot.next_item_instance_sequence,
            item_instances,
            npcs: snapshot.npcs,
            adventure: snapshot.adventure,
            progression_flags: snapshot.progression_flags,
            next_trade_id: TradeId(1),
            trades: BTreeMap::new(),
            next_loot_drop_id: LootDropId(1),
            item_provenance,
            loot_encounters: BTreeMap::new(),
            ground_loot: BTreeMap::new(),
            party_loot_cursors: BTreeMap::new(),
            next_reward_open_id: RewardOpenId(1),
            area_residency: BTreeMap::new(),
        })
    }

    pub fn upgrade_v3(snapshot: WorldRuntimeSnapshotV3) -> Result<Self, WorldError> {
        if snapshot.snapshot_version != 3 {
            return Err(WorldError::InvalidRuntimeSnapshot);
        }
        let item_instances = snapshot
            .item_instances
            .into_iter()
            .map(|(item_id, instance)| {
                (
                    item_id,
                    RuntimeItemInstanceSnapshot {
                        prototype_id: instance.prototype_id,
                        location: instance.location,
                        binding: ItemBinding::Unbound,
                        item_affixes: Vec::new(),
                    },
                )
            })
            .collect::<BTreeMap<_, _>>();
        let item_provenance = legacy_item_provenance(&item_instances, 3);
        Ok(Self {
            snapshot_version: WORLD_RUNTIME_SNAPSHOT_VERSION,
            revision: snapshot.revision,
            logical_tick: snapshot.logical_tick,
            rng_state: snapshot.rng_state,
            next_item_instance_sequence: snapshot.next_item_instance_sequence,
            item_instances,
            npcs: snapshot.npcs,
            adventure: snapshot.adventure,
            progression_flags: snapshot.progression_flags,
            next_trade_id: TradeId(1),
            trades: BTreeMap::new(),
            next_loot_drop_id: LootDropId(1),
            item_provenance,
            loot_encounters: BTreeMap::new(),
            ground_loot: BTreeMap::new(),
            party_loot_cursors: BTreeMap::new(),
            next_reward_open_id: RewardOpenId(1),
            area_residency: BTreeMap::new(),
        })
    }

    pub fn upgrade_v4(snapshot: WorldRuntimeSnapshotV4) -> Result<Self, WorldError> {
        if snapshot.snapshot_version != 4 {
            return Err(WorldError::InvalidRuntimeSnapshot);
        }
        let item_provenance = legacy_item_provenance(&snapshot.item_instances, 4);
        let item_instances = upgrade_v6_item_instances(snapshot.item_instances);
        Ok(Self {
            snapshot_version: WORLD_RUNTIME_SNAPSHOT_VERSION,
            revision: snapshot.revision,
            logical_tick: snapshot.logical_tick,
            rng_state: snapshot.rng_state,
            next_item_instance_sequence: snapshot.next_item_instance_sequence,
            item_instances,
            npcs: snapshot.npcs,
            adventure: snapshot.adventure,
            progression_flags: snapshot.progression_flags,
            next_trade_id: TradeId(1),
            trades: BTreeMap::new(),
            next_loot_drop_id: LootDropId(1),
            item_provenance,
            loot_encounters: BTreeMap::new(),
            ground_loot: BTreeMap::new(),
            party_loot_cursors: BTreeMap::new(),
            next_reward_open_id: RewardOpenId(1),
            area_residency: BTreeMap::new(),
        })
    }

    pub fn upgrade_v5(snapshot: WorldRuntimeSnapshotV5) -> Result<Self, WorldError> {
        if snapshot.snapshot_version != 5 {
            return Err(WorldError::InvalidRuntimeSnapshot);
        }
        let item_provenance = legacy_item_provenance(&snapshot.item_instances, 5);
        let item_instances = upgrade_v6_item_instances(snapshot.item_instances);
        Ok(Self {
            snapshot_version: WORLD_RUNTIME_SNAPSHOT_VERSION,
            revision: snapshot.revision,
            logical_tick: snapshot.logical_tick,
            rng_state: snapshot.rng_state,
            next_item_instance_sequence: snapshot.next_item_instance_sequence,
            item_instances,
            npcs: snapshot.npcs,
            adventure: snapshot.adventure,
            progression_flags: snapshot.progression_flags,
            next_trade_id: snapshot.next_trade_id,
            trades: snapshot.trades,
            next_loot_drop_id: LootDropId(1),
            item_provenance,
            loot_encounters: BTreeMap::new(),
            ground_loot: BTreeMap::new(),
            party_loot_cursors: BTreeMap::new(),
            next_reward_open_id: RewardOpenId(1),
            area_residency: BTreeMap::new(),
        })
    }

    pub fn upgrade_v6(snapshot: WorldRuntimeSnapshotV6) -> Result<Self, WorldError> {
        if snapshot.snapshot_version != 6 {
            return Err(WorldError::InvalidRuntimeSnapshot);
        }
        Ok(Self {
            snapshot_version: WORLD_RUNTIME_SNAPSHOT_VERSION,
            revision: snapshot.revision,
            logical_tick: snapshot.logical_tick,
            rng_state: snapshot.rng_state,
            next_item_instance_sequence: snapshot.next_item_instance_sequence,
            item_instances: upgrade_v6_item_instances(snapshot.item_instances),
            npcs: snapshot.npcs,
            adventure: snapshot.adventure,
            progression_flags: snapshot.progression_flags,
            next_trade_id: snapshot.next_trade_id,
            trades: snapshot.trades,
            next_loot_drop_id: snapshot.next_loot_drop_id,
            item_provenance: snapshot
                .item_provenance
                .into_iter()
                .map(|(id, provenance)| (id, provenance.into()))
                .collect(),
            loot_encounters: snapshot
                .loot_encounters
                .into_iter()
                .map(|(npc_id, encounter)| (npc_id, encounter.into()))
                .collect(),
            ground_loot: snapshot.ground_loot,
            party_loot_cursors: snapshot.party_loot_cursors,
            next_reward_open_id: RewardOpenId(1),
            area_residency: BTreeMap::new(),
        })
    }

    pub fn upgrade_v7(
        snapshot: WorldRuntimeSnapshotV7,
        area_residency: BTreeMap<String, AreaRuntimeResidence>,
    ) -> Result<Self, WorldError> {
        if snapshot.snapshot_version != 7
            || !valid_area_residency_manifest(&area_residency)
            || area_residency
                .values()
                .any(|state| matches!(state, AreaRuntimeResidence::Retired { .. }))
        {
            return Err(WorldError::InvalidRuntimeSnapshot);
        }
        let upgraded = Self {
            snapshot_version: WORLD_RUNTIME_SNAPSHOT_VERSION,
            revision: snapshot.revision,
            logical_tick: snapshot.logical_tick,
            rng_state: snapshot.rng_state,
            next_item_instance_sequence: snapshot.next_item_instance_sequence,
            item_instances: snapshot.item_instances,
            npcs: snapshot.npcs,
            adventure: snapshot.adventure,
            progression_flags: snapshot.progression_flags,
            next_trade_id: snapshot.next_trade_id,
            trades: snapshot.trades,
            next_loot_drop_id: snapshot.next_loot_drop_id,
            item_provenance: snapshot.item_provenance,
            loot_encounters: snapshot
                .loot_encounters
                .into_iter()
                .map(|(npc_id, encounter)| (npc_id, encounter.into()))
                .collect(),
            ground_loot: snapshot.ground_loot,
            party_loot_cursors: snapshot.party_loot_cursors,
            next_reward_open_id: snapshot.next_reward_open_id,
            area_residency,
        };
        validate_world_runtime_snapshot_semantics(&upgraded)?;
        Ok(upgraded)
    }

    /// Upgrade an exact snapshot v8 payload to the current v9 shape. v8 already
    /// carries the area residency manifest, so unlike v7 no external identities
    /// are supplied. Every loot encounter normalizes to the `RoundRobin` default
    /// with empty declarations/assignments; the rest of the state is preserved
    /// verbatim and revalidated.
    pub fn upgrade_v8(snapshot: WorldRuntimeSnapshotV8) -> Result<Self, WorldError> {
        if snapshot.snapshot_version != 8 {
            return Err(WorldError::InvalidRuntimeSnapshot);
        }
        let upgraded = Self {
            snapshot_version: WORLD_RUNTIME_SNAPSHOT_VERSION,
            revision: snapshot.revision,
            logical_tick: snapshot.logical_tick,
            rng_state: snapshot.rng_state,
            next_item_instance_sequence: snapshot.next_item_instance_sequence,
            item_instances: snapshot.item_instances,
            npcs: snapshot.npcs,
            adventure: snapshot.adventure,
            progression_flags: snapshot.progression_flags,
            next_trade_id: snapshot.next_trade_id,
            trades: snapshot.trades,
            next_loot_drop_id: snapshot.next_loot_drop_id,
            item_provenance: snapshot.item_provenance,
            loot_encounters: snapshot
                .loot_encounters
                .into_iter()
                .map(|(npc_id, encounter)| (npc_id, encounter.into()))
                .collect(),
            ground_loot: snapshot.ground_loot,
            party_loot_cursors: snapshot.party_loot_cursors,
            next_reward_open_id: snapshot.next_reward_open_id,
            area_residency: snapshot.area_residency,
        };
        validate_world_runtime_snapshot_semantics(&upgraded)?;
        Ok(upgraded)
    }
}

fn upgrade_v6_item_instances(
    instances: BTreeMap<ItemInstanceId, RuntimeItemInstanceSnapshotV6>,
) -> BTreeMap<ItemInstanceId, RuntimeItemInstanceSnapshot> {
    instances
        .into_iter()
        .map(|(id, instance)| {
            (
                id,
                RuntimeItemInstanceSnapshot {
                    prototype_id: instance.prototype_id,
                    location: instance.location,
                    binding: instance.binding,
                    item_affixes: Vec::new(),
                },
            )
        })
        .collect()
}

fn legacy_item_provenance<T>(
    items: &BTreeMap<ItemInstanceId, T>,
    source_version: u32,
) -> BTreeMap<ItemInstanceId, ItemProvenance> {
    items
        .keys()
        .cloned()
        .map(|item_id| (item_id, ItemProvenance::LegacySnapshot { source_version }))
        .collect()
}

impl WorldRuntimeSnapshotV2 {
    pub fn upgrade_v1(snapshot: WorldRuntimeSnapshotV1) -> Result<Self, WorldError> {
        if snapshot.snapshot_version != 1 {
            return Err(WorldError::InvalidRuntimeSnapshot);
        }
        let item_instances = snapshot
            .item_locations
            .into_iter()
            .map(|(legacy_id, location)| {
                (
                    ItemInstanceId(legacy_id.clone()),
                    RuntimeItemInstanceSnapshotV2 {
                        prototype_id: legacy_id,
                        location,
                    },
                )
            })
            .collect();
        Ok(Self {
            snapshot_version: 2,
            revision: snapshot.revision,
            logical_tick: snapshot.logical_tick,
            rng_state: snapshot.rng_state,
            next_item_instance_sequence: 1,
            item_instances,
            npcs: snapshot.npcs,
            adventure: snapshot.adventure,
            progression_flags: snapshot.progression_flags,
        })
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum CompatibleWorldRuntimeSnapshot {
    V9(WorldRuntimeSnapshot),
    V8(WorldRuntimeSnapshotV8),
    V7(WorldRuntimeSnapshotV7),
    V6(WorldRuntimeSnapshotV6),
    V5(WorldRuntimeSnapshotV5),
    V4(WorldRuntimeSnapshotV4),
    V3(WorldRuntimeSnapshotV3),
    V2(WorldRuntimeSnapshotV2),
    V1(WorldRuntimeSnapshotV1),
}

impl<'de> Deserialize<'de> for CompatibleWorldRuntimeSnapshot {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let value = serde_json::Value::deserialize(deserializer)?;
        let version = value
            .get("snapshotVersion")
            .and_then(serde_json::Value::as_u64)
            .ok_or_else(|| serde::de::Error::custom("missing snapshotVersion"))?;
        match version {
            1 => serde_json::from_value(value)
                .map(Self::V1)
                .map_err(serde::de::Error::custom),
            2 => serde_json::from_value(value)
                .map(Self::V2)
                .map_err(serde::de::Error::custom),
            3 => serde_json::from_value(value)
                .map(Self::V3)
                .map_err(serde::de::Error::custom),
            4 => serde_json::from_value(value)
                .map(Self::V4)
                .map_err(serde::de::Error::custom),
            5 => serde_json::from_value(value)
                .map(Self::V5)
                .map_err(serde::de::Error::custom),
            6 => serde_json::from_value(value)
                .map(Self::V6)
                .map_err(serde::de::Error::custom),
            7 => serde_json::from_value(value)
                .map(Self::V7)
                .map_err(serde::de::Error::custom),
            8 => serde_json::from_value(value)
                .map(Self::V8)
                .map_err(serde::de::Error::custom),
            9 => serde_json::from_value(value)
                .map(Self::V9)
                .map_err(serde::de::Error::custom),
            _ => Err(serde::de::Error::custom("unsupported snapshotVersion")),
        }
    }
}

impl CompatibleWorldRuntimeSnapshot {
    pub fn into_current(self) -> Result<WorldRuntimeSnapshot, WorldError> {
        match self {
            Self::V9(snapshot) => Ok(snapshot),
            Self::V8(snapshot) => WorldRuntimeSnapshot::upgrade_v8(snapshot),
            // The generic path is the monolithic compatibility path. Catalog
            // recovery decodes the exact v7 DTO separately and supplies its
            // verified resident manifest to `upgrade_v7`.
            Self::V7(snapshot) => WorldRuntimeSnapshot::upgrade_v7(snapshot, BTreeMap::new()),
            Self::V6(snapshot) => WorldRuntimeSnapshot::upgrade_v6(snapshot),
            Self::V5(snapshot) => WorldRuntimeSnapshot::upgrade_v5(snapshot),
            Self::V4(snapshot) => WorldRuntimeSnapshot::upgrade_v4(snapshot),
            Self::V3(snapshot) => WorldRuntimeSnapshot::upgrade_v3(snapshot),
            Self::V2(snapshot) => WorldRuntimeSnapshot::upgrade_v2(snapshot),
            Self::V1(snapshot) => WorldRuntimeSnapshot::upgrade_v1(snapshot),
        }
    }
}

/// Decode one durable runtime payload only after comparing the storage column
/// with the payload's original version. Normalization never erases evidence of
/// a mismatched column/version pair.
pub fn decode_world_runtime_snapshot(
    durable_version: u32,
    json: &str,
) -> Result<WorldRuntimeSnapshot, WorldError> {
    let byte_limit = if durable_version <= 5 {
        MAX_LEGACY_WORLD_RUNTIME_SNAPSHOT_BYTES
    } else {
        MAX_WORLD_RUNTIME_SNAPSHOT_BYTES
    };
    if json.len() > byte_limit {
        return Err(WorldError::InvalidRuntimeSnapshot);
    }
    let compatible: CompatibleWorldRuntimeSnapshot =
        serde_json::from_str(json).map_err(|_| WorldError::InvalidRuntimeSnapshot)?;
    let payload_version = match &compatible {
        CompatibleWorldRuntimeSnapshot::V9(snapshot) => snapshot.snapshot_version,
        CompatibleWorldRuntimeSnapshot::V8(snapshot) => snapshot.snapshot_version,
        CompatibleWorldRuntimeSnapshot::V7(snapshot) => snapshot.snapshot_version,
        CompatibleWorldRuntimeSnapshot::V6(snapshot) => snapshot.snapshot_version,
        CompatibleWorldRuntimeSnapshot::V5(snapshot) => snapshot.snapshot_version,
        CompatibleWorldRuntimeSnapshot::V4(snapshot) => snapshot.snapshot_version,
        CompatibleWorldRuntimeSnapshot::V3(snapshot) => snapshot.snapshot_version,
        CompatibleWorldRuntimeSnapshot::V2(snapshot) => snapshot.snapshot_version,
        CompatibleWorldRuntimeSnapshot::V1(snapshot) => snapshot.snapshot_version,
    };
    if payload_version != durable_version {
        return Err(WorldError::InvalidRuntimeSnapshot);
    }
    compatible.into_current()
}

/// Decodes the exact legacy v7 DTO without inventing area residency.
/// Catalog recovery must subsequently call [`WorldRuntimeSnapshot::upgrade_v7`]
/// with identities derived from its verified durable generation.
pub fn decode_world_runtime_snapshot_v7(
    durable_version: u32,
    json: &str,
) -> Result<WorldRuntimeSnapshotV7, WorldError> {
    if durable_version != 7 || json.len() > MAX_WORLD_RUNTIME_SNAPSHOT_BYTES {
        return Err(WorldError::InvalidRuntimeSnapshot);
    }
    let snapshot: WorldRuntimeSnapshotV7 =
        serde_json::from_str(json).map_err(|_| WorldError::InvalidRuntimeSnapshot)?;
    if snapshot.snapshot_version != durable_version {
        return Err(WorldError::InvalidRuntimeSnapshot);
    }
    Ok(snapshot)
}
