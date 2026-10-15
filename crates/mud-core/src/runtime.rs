//! Portable runtime records (item locations, NPC runtime state, area
//! dynamic checkpoints) and their semantic validation.

use super::*;

pub const WORLD_RUNTIME_SNAPSHOT_VERSION: u32 = 9;
pub const MAX_LEGACY_WORLD_RUNTIME_SNAPSHOT_BYTES: usize = 1_048_576;
pub const MAX_WORLD_RUNTIME_SNAPSHOT_BYTES: usize = 2_097_152;
pub(crate) const MAX_ACTIVE_EFFECTS: usize = 32;
pub const MAX_ITEM_INSTANCES: usize = 65_536;
pub const MAX_ITEM_INSTANCE_ID_BYTES: usize = 128;

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "camelCase", deny_unknown_fields)]
pub enum RuntimeItemLocation {
    /// Reserved during world construction for a content-defined initial owner.
    /// Staged items are never projected into a room scene.
    Staged,
    Carried {
        character_id: CharacterId,
    },
    Room {
        room_id: String,
    },
    Equipped {
        character_id: CharacterId,
        slot: EquipmentSlot,
    },
    HeldByNpc {
        npc_id: String,
    },
    Contained {
        container_id: ItemInstanceId,
    },
    Consumed,
}

/// Exact pre-container location vocabulary retained only for snapshot v1/v2
/// conversion. Keeping it separate prevents a v2 payload from smuggling a v3
/// `contained` edge.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "camelCase", deny_unknown_fields)]
pub enum RuntimeItemLocationV2 {
    Staged,
    Carried {
        character_id: CharacterId,
    },
    Room {
        room_id: String,
    },
    Equipped {
        character_id: CharacterId,
        slot: EquipmentSlot,
    },
    HeldByNpc {
        npc_id: String,
    },
    Consumed,
}

impl From<RuntimeItemLocationV2> for RuntimeItemLocation {
    fn from(location: RuntimeItemLocationV2) -> Self {
        match location {
            RuntimeItemLocationV2::Staged => Self::Staged,
            RuntimeItemLocationV2::Carried { character_id } => Self::Carried { character_id },
            RuntimeItemLocationV2::Room { room_id } => Self::Room { room_id },
            RuntimeItemLocationV2::Equipped { character_id, slot } => {
                Self::Equipped { character_id, slot }
            }
            RuntimeItemLocationV2::HeldByNpc { npc_id } => Self::HeldByNpc { npc_id },
            RuntimeItemLocationV2::Consumed => Self::Consumed,
        }
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RuntimeItemInstanceSnapshot {
    pub prototype_id: String,
    pub location: RuntimeItemLocation,
    pub binding: ItemBinding,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub item_affixes: Vec<AppliedItemAffix>,
}

/// Exact item shape written by snapshot v6. It cannot opt into v7 affixes.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RuntimeItemInstanceSnapshotV6 {
    pub prototype_id: String,
    pub location: RuntimeItemLocation,
    pub binding: ItemBinding,
}

/// Exact item shape written by inventory phase 3B. It intentionally lacks
/// `binding`, so a v3 payload cannot opt into v4 fields before conversion.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RuntimeItemInstanceSnapshotV3 {
    pub prototype_id: String,
    pub location: RuntimeItemLocation,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct NpcRuntimeSnapshot {
    pub stable_id: String,
    pub name: String,
    pub description: String,
    pub spawn_room_id: String,
    pub level: u32,
    pub health: i32,
    pub max_health: i32,
    pub strength: i32,
    pub defence: i32,
    pub experience_reward: u64,
    pub loot_item_ids: Vec<ItemInstanceId>,
    pub respawn_ticks: u64,
    pub respawn_at_tick: Option<u64>,
    pub engaged_character: Option<CharacterId>,
    #[serde(default, deserialize_with = "deserialize_runtime_resource_pools")]
    pub resources: BTreeMap<String, ResourcePool>,
    #[serde(default)]
    pub resource_regeneration: BTreeMap<String, u32>,
    #[serde(default)]
    pub ability_ids: Vec<String>,
    #[serde(default)]
    pub ability_cooldowns: BTreeMap<String, u64>,
    #[serde(default)]
    pub resistances: BTreeMap<DamageType, u16>,
    #[serde(default, deserialize_with = "deserialize_runtime_statuses")]
    pub statuses: Vec<ActiveStatusEffect>,
}

/// Snapshot-only exact shape. `ResourcePool` is also used by other durable
/// documents, so strictness is applied at this boundary without changing
/// their compatibility contract.
#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct RuntimeResourcePoolSnapshot {
    pub(crate) current: u32,
    pub(crate) max: u32,
}

impl From<RuntimeResourcePoolSnapshot> for ResourcePool {
    fn from(pool: RuntimeResourcePoolSnapshot) -> Self {
        Self {
            current: pool.current,
            max: pool.max,
        }
    }
}

pub(crate) fn deserialize_runtime_resource_pools<'de, D>(
    deserializer: D,
) -> Result<BTreeMap<String, ResourcePool>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    BTreeMap::<String, RuntimeResourcePoolSnapshot>::deserialize(deserializer).map(|pools| {
        pools
            .into_iter()
            .map(|(id, pool)| (id, pool.into()))
            .collect()
    })
}

/// Snapshot-only exact shape for status records. Optional fields retain the
/// same defaults as `ActiveStatusEffect`, while unknown fields fail closed.
#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct RuntimeActiveStatusSnapshot {
    pub(crate) source_ability_id: String,
    #[serde(default)]
    pub(crate) source_character_id: Option<CharacterId>,
    #[serde(default)]
    pub(crate) source_npc_id: Option<String>,
    pub(crate) kind: ActiveStatusKind,
    pub(crate) magnitude: i32,
    pub(crate) remaining_ticks: u32,
    #[serde(default)]
    pub(crate) attribute: Option<String>,
    #[serde(default)]
    pub(crate) damage_type: Option<DamageType>,
}

impl From<RuntimeActiveStatusSnapshot> for ActiveStatusEffect {
    fn from(status: RuntimeActiveStatusSnapshot) -> Self {
        Self {
            source_ability_id: status.source_ability_id,
            source_character_id: status.source_character_id,
            source_npc_id: status.source_npc_id,
            kind: status.kind,
            magnitude: status.magnitude,
            remaining_ticks: status.remaining_ticks,
            attribute: status.attribute,
            damage_type: status.damage_type,
        }
    }
}

pub(crate) fn deserialize_runtime_statuses<'de, D>(
    deserializer: D,
) -> Result<Vec<ActiveStatusEffect>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    Vec::<RuntimeActiveStatusSnapshot>::deserialize(deserializer)
        .map(|statuses| statuses.into_iter().map(Into::into).collect())
}

/// Strict, portable dynamic state owned by one retired area unit.
///
/// The global RNG and monotonic allocators are barriers only: rehydration must
/// compare them for provenance but never restore them, because other resident
/// areas may have advanced after this checkpoint was committed.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AreaDynamicCheckpoint {
    pub checkpoint_version: u32,
    pub generation_sha256: String,
    pub areas: Vec<AreaDynamicCheckpointIdentity>,
    pub captured_world_revision: u64,
    pub captured_logical_tick: u64,
    pub captured_rng_state: u64,
    pub captured_next_item_instance_sequence: u64,
    pub captured_next_loot_drop_id: LootDropId,
    pub captured_next_reward_open_id: RewardOpenId,
    pub npcs: BTreeMap<String, NpcRuntimeSnapshot>,
    pub item_instances: BTreeMap<ItemInstanceId, RuntimeItemInstanceSnapshot>,
    pub item_provenance: BTreeMap<ItemInstanceId, ItemProvenance>,
    pub ground_loot: BTreeMap<ItemInstanceId, GroundLootReservation>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AreaDynamicCheckpointIdentity {
    pub area_id: String,
    pub revision: u64,
    pub bundle_sha256: String,
    pub residency_epoch: u64,
}

pub(crate) fn valid_area_residency_manifest(
    manifest: &BTreeMap<String, AreaRuntimeResidence>,
) -> bool {
    manifest.len() <= MAX_RUNTIME_AREA_RESIDENCY_ENTRIES
        && manifest.iter().all(|(area_id, state)| {
            valid_definition_id(area_id)
                && state.revision() > 0
                && valid_digest_sha256(state.bundle_sha256())
                && state.residency_epoch() > 0
                && match state {
                    AreaRuntimeResidence::Resident { .. } => true,
                    AreaRuntimeResidence::Retired {
                        checkpoint_version,
                        checkpoint_sha256,
                        ..
                    } => {
                        *checkpoint_version == AREA_DYNAMIC_CHECKPOINT_VERSION
                            && valid_digest_sha256(checkpoint_sha256)
                    }
                }
        })
}

pub fn validate_area_dynamic_checkpoint(
    checkpoint: &AreaDynamicCheckpoint,
) -> Result<(), WorldError> {
    if serde_json::to_vec(checkpoint)
        .map_err(|_| WorldError::InvalidRuntimeSnapshot)?
        .len()
        > MAX_WORLD_RUNTIME_SNAPSHOT_BYTES
        || checkpoint.checkpoint_version != AREA_DYNAMIC_CHECKPOINT_VERSION
        || !valid_digest_sha256(&checkpoint.generation_sha256)
        || checkpoint.areas.is_empty()
        || checkpoint.areas.len() > MAX_RUNTIME_AREA_RESIDENCY_ENTRIES
        || checkpoint
            .areas
            .windows(2)
            .any(|pair| pair[0].area_id >= pair[1].area_id)
        || checkpoint.areas.iter().any(|area| {
            !valid_definition_id(&area.area_id)
                || area.revision == 0
                || !valid_digest_sha256(&area.bundle_sha256)
                || area.residency_epoch == 0
        })
        || checkpoint.item_instances.len() > MAX_ITEM_INSTANCES
        || checkpoint
            .item_instances
            .keys()
            .ne(checkpoint.item_provenance.keys())
        || checkpoint.npcs.values().any(|npc| {
            npc.engaged_character.is_some()
                || !npc.statuses.is_empty()
                || !npc.ability_cooldowns.is_empty()
        })
    {
        return Err(WorldError::InvalidRuntimeSnapshot);
    }

    let portable = WorldRuntimeSnapshot {
        snapshot_version: WORLD_RUNTIME_SNAPSHOT_VERSION,
        revision: checkpoint.captured_world_revision,
        logical_tick: checkpoint.captured_logical_tick,
        rng_state: checkpoint.captured_rng_state,
        next_item_instance_sequence: checkpoint.captured_next_item_instance_sequence,
        item_instances: checkpoint.item_instances.clone(),
        npcs: checkpoint.npcs.clone(),
        adventure: AdventureRuntimeSnapshot::default(),
        progression_flags: BTreeSet::new(),
        next_trade_id: TradeId(1),
        trades: BTreeMap::new(),
        next_loot_drop_id: checkpoint.captured_next_loot_drop_id,
        item_provenance: checkpoint.item_provenance.clone(),
        loot_encounters: BTreeMap::new(),
        ground_loot: checkpoint.ground_loot.clone(),
        party_loot_cursors: BTreeMap::new(),
        next_reward_open_id: checkpoint.captured_next_reward_open_id,
        area_residency: BTreeMap::new(),
    };
    validate_world_runtime_snapshot_semantics(&portable)
}

/// Validates every portable invariant that can be established without a
/// particular WorldPack. Offline movers and recovery tools use this same
/// authority after version-directed decoding; `World::restore_runtime_snapshot`
/// adds only the authored room/prototype/policy/capacity checks.
pub fn validate_world_runtime_snapshot_semantics(
    snapshot: &WorldRuntimeSnapshot,
) -> Result<(), WorldError> {
    if serde_json::to_vec(snapshot)
        .map_err(|_| WorldError::InvalidRuntimeSnapshot)?
        .len()
        > MAX_WORLD_RUNTIME_SNAPSHOT_BYTES
    {
        return Err(WorldError::InvalidRuntimeSnapshot);
    }
    if snapshot.snapshot_version != WORLD_RUNTIME_SNAPSHOT_VERSION
        || snapshot.revision == 0
        || snapshot.rng_state == 0
        || snapshot.next_item_instance_sequence == 0
        || snapshot.item_instances.len() > MAX_ITEM_INSTANCES
        || snapshot.progression_flags.len() > 64
        || !valid_area_residency_manifest(&snapshot.area_residency)
        || !snapshot
            .progression_flags
            .iter()
            .all(|flag| valid_definition_id(flag))
    {
        return Err(WorldError::InvalidRuntimeSnapshot);
    }
    let runtime_sequence_valid = snapshot
        .item_instances
        .keys()
        .filter_map(runtime_item_sequence)
        .max()
        .is_none_or(|sequence| sequence < snapshot.next_item_instance_sequence);
    if !runtime_sequence_valid {
        return Err(WorldError::InvalidRuntimeSnapshot);
    }
    if snapshot.next_loot_drop_id.0 == 0
        || snapshot.next_reward_open_id.0 == 0
        || snapshot.item_provenance.len() != snapshot.item_instances.len()
        || snapshot
            .item_provenance
            .keys()
            .ne(snapshot.item_instances.keys())
        || snapshot.loot_encounters.len() > MAX_LOOT_ENCOUNTERS
        || snapshot.ground_loot.len() > MAX_GROUND_LOOT_RESERVATIONS
    {
        return Err(WorldError::InvalidRuntimeSnapshot);
    }

    let mut children = BTreeMap::<ItemInstanceId, Vec<ItemInstanceId>>::new();
    for (item_id, instance) in &snapshot.item_instances {
        let location_valid = match &instance.location {
            RuntimeItemLocation::Staged | RuntimeItemLocation::Consumed => true,
            RuntimeItemLocation::Carried { character_id }
            | RuntimeItemLocation::Equipped { character_id, .. } => character_id.0 > 0,
            RuntimeItemLocation::Room { room_id } => valid_definition_id(room_id),
            RuntimeItemLocation::HeldByNpc { npc_id } => valid_definition_id(npc_id),
            RuntimeItemLocation::Contained { container_id } => {
                if container_id == item_id
                    || !valid_item_instance_id(container_id.as_str())
                    || !snapshot.item_instances.contains_key(container_id)
                {
                    false
                } else {
                    children
                        .entry(container_id.clone())
                        .or_default()
                        .push(item_id.clone());
                    true
                }
            }
        };
        if !valid_item_instance_id(item_id.as_str())
            || !valid_definition_id(&instance.prototype_id)
            || !location_valid
            || matches!(instance.binding, ItemBinding::Character { character_id } if character_id.0 <= 0)
            || instance.item_affixes.len() > MAX_AFFIXES_PER_ITEM
            || instance.item_affixes.iter().any(|affix| {
                !valid_definition_id(&affix.affix_id)
                    || !valid_digest_sha256(&affix.definition_digest_sha256)
                    || !valid_definition_id(&affix.pool_id)
                    || !valid_digest_sha256(&affix.pool_digest_sha256)
                    || !valid_digest_sha256(&affix.record_digest_sha256)
                    || !safe_definition_text(&affix.label, 64)
                    || !(0..=MAX_AFFIX_MODIFIER).contains(&affix.attack_bonus_delta)
                    || !(0..=MAX_AFFIX_MODIFIER).contains(&affix.heal_amount_delta)
                    || (affix.attack_bonus_delta == 0 && affix.heal_amount_delta == 0)
            })
            || !instance.item_affixes.windows(2).all(|pair| {
                (pair[0].position, pair[0].affix_id.as_str())
                    < (pair[1].position, pair[1].affix_id.as_str())
            })
        {
            return Err(WorldError::InvalidRuntimeSnapshot);
        }
        for affix in &instance.item_affixes {
            let definition = AffixDefinition {
                id: affix.affix_id.clone(),
                label: affix.label.clone(),
                position: affix.position,
                attack_bonus_delta: affix.attack_bonus_delta,
                heal_amount_delta: affix.heal_amount_delta,
            };
            if canonical_sha256(&definition).map_err(|_| WorldError::InvalidRuntimeSnapshot)?
                != affix.definition_digest_sha256
                || applied_affix_record_sha256(affix)
                    .map_err(|_| WorldError::InvalidRuntimeSnapshot)?
                    != affix.record_digest_sha256
            {
                return Err(WorldError::InvalidRuntimeSnapshot);
            }
        }
    }
    for item_id in snapshot.item_instances.keys() {
        let mut current = item_id;
        let mut visited = BTreeSet::new();
        let mut depth = 0_usize;
        while let Some(RuntimeItemInstanceSnapshot {
            location: RuntimeItemLocation::Contained { container_id },
            ..
        }) = snapshot.item_instances.get(current)
        {
            if !visited.insert(current.clone()) {
                return Err(WorldError::InvalidRuntimeSnapshot);
            }
            depth = depth
                .checked_add(1)
                .ok_or(WorldError::InvalidRuntimeSnapshot)?;
            if depth > MAX_CONTAINER_DEPTH {
                return Err(WorldError::InvalidRuntimeSnapshot);
            }
            current = container_id;
        }
    }

    let mut greatest_drop_id = 0_u64;
    let mut greatest_open_id = 0_u64;
    let mut open_sources =
        BTreeMap::<RewardOpenId, (ItemInstanceId, String, String, String, String, u64)>::new();
    let mut source_opens = BTreeMap::<ItemInstanceId, RewardOpenId>::new();
    let mut reward_units = BTreeSet::new();
    let mut open_unit_counts = BTreeMap::<RewardOpenId, usize>::new();
    let mut quantity_indexes =
        BTreeMap::<(RewardOpenId, String, Option<String>, u8), BTreeSet<u16>>::new();
    for (item_id, provenance) in &snapshot.item_provenance {
        match provenance {
            ItemProvenance::AuthoredInitial => {}
            ItemProvenance::LegacySnapshot { source_version } => {
                if !(1..=5).contains(source_version) {
                    return Err(WorldError::InvalidRuntimeSnapshot);
                }
            }
            ItemProvenance::RuntimeCreated { created_tick } => {
                if *created_tick > snapshot.logical_tick {
                    return Err(WorldError::InvalidRuntimeSnapshot);
                }
            }
            ItemProvenance::NpcLoot {
                drop_id,
                npc_spawn_id,
                loot_table_id,
                table_digest_sha256,
                entry_id,
                pool_id,
                sealed_loot_table_id,
                roll_index,
                quantity_index,
                generated_tick,
            } => {
                greatest_drop_id = greatest_drop_id.max(drop_id.0);
                if drop_id.0 == 0
                    || !valid_definition_id(npc_spawn_id)
                    || !valid_definition_id(loot_table_id)
                    || !valid_definition_id(entry_id)
                    || pool_id.as_ref().is_some_and(|id| !valid_definition_id(id))
                    || sealed_loot_table_id
                        .as_ref()
                        .is_some_and(|id| !valid_definition_id(id))
                    || !valid_digest_sha256(table_digest_sha256)
                    || *roll_index >= 8
                    || *quantity_index >= 32
                    || *generated_tick > snapshot.logical_tick
                {
                    return Err(WorldError::InvalidRuntimeSnapshot);
                }
            }
            ItemProvenance::SealedReward {
                open_id,
                source_item_id,
                source_npc_loot_digest_sha256,
                sealed_prototype_id,
                loot_table_id,
                table_digest_sha256,
                entry_id,
                pool_id,
                roll_index,
                quantity_index,
                generated_tick,
            } => {
                greatest_open_id = greatest_open_id.max(open_id.0);
                if open_id.0 == 0
                    || source_item_id == item_id
                    || !valid_item_instance_id(source_item_id.as_str())
                    || !valid_digest_sha256(source_npc_loot_digest_sha256)
                    || !valid_definition_id(sealed_prototype_id)
                    || !valid_definition_id(loot_table_id)
                    || !valid_digest_sha256(table_digest_sha256)
                    || !valid_definition_id(entry_id)
                    || pool_id.as_ref().is_some_and(|id| !valid_definition_id(id))
                    || *roll_index >= 8
                    || usize::from(*quantity_index) >= MAX_SEALED_REWARD_UNITS
                    || *generated_tick > snapshot.logical_tick
                {
                    return Err(WorldError::InvalidRuntimeSnapshot);
                }
                let source_key = (
                    source_item_id.clone(),
                    source_npc_loot_digest_sha256.clone(),
                    sealed_prototype_id.clone(),
                    loot_table_id.clone(),
                    table_digest_sha256.clone(),
                    *generated_tick,
                );
                if open_sources
                    .insert(*open_id, source_key.clone())
                    .is_some_and(|previous| previous != source_key)
                    || source_opens
                        .insert(source_item_id.clone(), *open_id)
                        .is_some_and(|previous| previous != *open_id)
                    || !reward_units.insert((
                        *open_id,
                        entry_id.clone(),
                        pool_id.clone(),
                        *roll_index,
                        *quantity_index,
                    ))
                {
                    return Err(WorldError::InvalidRuntimeSnapshot);
                }
                *open_unit_counts.entry(*open_id).or_default() += 1;
                quantity_indexes
                    .entry((*open_id, entry_id.clone(), pool_id.clone(), *roll_index))
                    .or_default()
                    .insert(*quantity_index);
            }
        }
    }
    if open_unit_counts
        .values()
        .any(|count| !(1..=MAX_SEALED_REWARD_UNITS).contains(count))
        || quantity_indexes
            .values()
            .any(|indexes| indexes.iter().copied().ne((0_u16..).take(indexes.len())))
    {
        return Err(WorldError::InvalidRuntimeSnapshot);
    }
    for (_, (source_item_id, expected_digest, sealed_prototype_id, _, _, _)) in open_sources {
        let Some(source_instance) = snapshot.item_instances.get(&source_item_id) else {
            continue;
        };
        let source_provenance = snapshot
            .item_provenance
            .get(&source_item_id)
            .ok_or(WorldError::InvalidRuntimeSnapshot)?;
        if !matches!(source_instance.location, RuntimeItemLocation::Consumed)
            || source_instance.prototype_id != sealed_prototype_id
            || !matches!(source_provenance, ItemProvenance::NpcLoot { .. })
            || canonical_sha256(source_provenance)
                .map_err(|_| WorldError::InvalidRuntimeSnapshot)?
                != expected_digest
        {
            return Err(WorldError::InvalidRuntimeSnapshot);
        }
    }
    for (npc_id, encounter) in &snapshot.loot_encounters {
        let npc = snapshot
            .npcs
            .get(npc_id)
            .ok_or(WorldError::InvalidRuntimeSnapshot)?;
        if npc_id != &encounter.npc_spawn_id
            || npc.engaged_character != Some(encounter.initiator)
            || encounter.initiator.0 <= 0
            || encounter.started_tick > snapshot.logical_tick
            || !valid_definition_id(&encounter.room_id)
            || encounter.room_id != npc.spawn_room_id
            || encounter.party_id == Some(0)
            || encounter.cohort.is_empty()
            || encounter.cohort.len() > 6
            || !encounter.cohort.contains(&encounter.initiator)
            || encounter.cohort.iter().any(|id| id.0 <= 0)
            || !encounter.cohort.windows(2).all(|pair| pair[0] < pair[1])
            // v9 group-distribution fields fail closed. Declarations only exist
            // under `NeedGreed` and assignments only under `MasterLoot`; every
            // per-root key is a valid instance id and every declared/assigned
            // identity belongs to the frozen cohort.
            || (encounter.distribution != loot_distribution::LootDistributionMode::NeedGreed
                && !encounter.loot_declarations.is_empty())
            || (encounter.distribution != loot_distribution::LootDistributionMode::MasterLoot
                && !encounter.master_assignments.is_empty())
            || encounter.loot_declarations.len() > MAX_GROUND_LOOT_RESERVATIONS
            || encounter.master_assignments.len() > MAX_GROUND_LOOT_RESERVATIONS
            || encounter
                .loot_declarations
                .iter()
                .any(|(root_id, declarations)| {
                    !valid_item_instance_id(root_id.as_str())
                        || declarations.is_empty()
                        || declarations.len() > encounter.cohort.len()
                        || declarations
                            .keys()
                            .any(|member| !encounter.cohort.contains(member))
                })
            || encounter.master_assignments.iter().any(|(root_id, member)| {
                !valid_item_instance_id(root_id.as_str())
                    || !encounter.cohort.contains(member)
            })
        {
            return Err(WorldError::InvalidRuntimeSnapshot);
        }
    }
    for (root_id, reservation) in &snapshot.ground_loot {
        greatest_drop_id = greatest_drop_id.max(reservation.drop_id.0);
        let instance = snapshot
            .item_instances
            .get(root_id)
            .ok_or(WorldError::InvalidRuntimeSnapshot)?;
        if root_id != &reservation.root_item_id
            || reservation.drop_id.0 == 0
            || instance.location
                != (RuntimeItemLocation::Room {
                    room_id: reservation.room_id.clone(),
                })
            || !valid_definition_id(&reservation.room_id)
            || reservation.cohort.is_empty()
            || reservation.cohort.len() > 6
            || reservation.cohort.iter().any(|id| id.0 <= 0)
            || !reservation.cohort.windows(2).all(|pair| pair[0] < pair[1])
            || reservation.beneficiary.is_some_and(|id| {
                !reservation.cohort.contains(&id) || reservation.passed.contains(&id)
            })
            || (snapshot.logical_tick >= reservation.claim_until_tick
                && reservation.beneficiary.is_some())
            || !reservation
                .passed
                .iter()
                .all(|id| reservation.cohort.contains(id))
            || reservation.claim_until_tick
                != reservation
                    .created_tick
                    .checked_add(LOOT_EXCLUSIVE_TICKS)
                    .ok_or(WorldError::InvalidRuntimeSnapshot)?
            || reservation.despawn_at_tick
                != reservation
                    .created_tick
                    .checked_add(LOOT_DESPAWN_TICKS)
                    .ok_or(WorldError::InvalidRuntimeSnapshot)?
            || snapshot.logical_tick >= reservation.despawn_at_tick
        {
            return Err(WorldError::InvalidRuntimeSnapshot);
        }
    }
    if greatest_drop_id >= snapshot.next_loot_drop_id.0
        || greatest_open_id >= snapshot.next_reward_open_id.0
        || snapshot
            .party_loot_cursors
            .iter()
            .any(|(party_id, cursor)| {
                *party_id == 0 || *cursor == 0 || !snapshot.adventure.parties.contains_key(party_id)
            })
    {
        return Err(WorldError::InvalidRuntimeSnapshot);
    }

    let mut party_members = BTreeSet::new();
    let adventure_valid = snapshot.adventure.next_party_id > 0
        && snapshot.adventure.parties.iter().all(|(party_id, party)| {
            *party_id == party.id
                && party.id < snapshot.adventure.next_party_id
                && !party.members.is_empty()
                && party.members.len() <= 6
                && party.members.iter().all(|member| member.0 > 0)
                && party.members.contains(&party.leader)
                && party.members.windows(2).all(|pair| pair[0] < pair[1])
                && party
                    .members
                    .iter()
                    .all(|member| party_members.insert(*member))
                && party.invitations.len() <= 12
                && party
                    .invitations
                    .keys()
                    .all(|invitee| invitee.0 > 0 && !party.members.contains(invitee))
        })
        && snapshot.adventure.shop_stock.len()
            <= MAX_ADVENTURE_SHOPS * MAX_ADVENTURE_SHOP_STOCK_ENTRIES
        && snapshot
            .adventure
            .shop_stock
            .iter()
            .all(|(id, stock)| valid_definition_id(id) && *stock <= MAX_ADVENTURE_SHOP_STOCK)
        && snapshot.adventure.next_weather_tick > 0;
    let npc_shape_valid = snapshot.npcs.iter().all(|(npc_id, npc)| {
        npc_id == &npc.stable_id
            && valid_definition_id(npc_id)
            && npc
                .engaged_character
                .is_none_or(|character| character.0 > 0)
            && npc
                .loot_item_ids
                .iter()
                .all(|item_id| valid_item_instance_id(item_id.as_str()))
    });
    if !adventure_valid || !npc_shape_valid {
        return Err(WorldError::InvalidRuntimeSnapshot);
    }

    validate_portable_trade_state(snapshot, &children)
}

impl Default for CharacterGameplaySnapshot {
    fn default() -> Self {
        Self {
            lineage_id: "core:human".into(),
            background_id: None,
            base_class_id: "core:wanderer".into(),
            active_class_id: "core:wanderer".into(),
            class_history: vec!["core:wanderer".into()],
            level: 1,
            experience: 0,
            vitals: Vitals {
                health: 20,
                max_health: 20,
            },
            attributes: Attributes {
                strength: 5,
                defence: 2,
                additional: BTreeMap::new(),
            },
            resources: BTreeMap::new(),
            resource_regeneration: BTreeMap::new(),
            ability_ids: Vec::new(),
            ability_cooldowns: BTreeMap::new(),
            ability_global_ready_tick: 0,
            last_offensive_ability_tick: None,
            resistances: BTreeMap::new(),
            statuses: Vec::new(),
            skills: default_skills(),
            progression_points: BTreeMap::new(),
            talent_node_ranks: BTreeMap::new(),
            tags: BTreeSet::new(),
            received_signals: BTreeSet::new(),
            equipment: BTreeMap::new(),
            inventory_capacity: InventoryCapacity::default(),
            effects: Vec::new(),
            recovery_at_tick: None,
            coins: default_coin_balance(),
            commodities: BTreeMap::new(),
            quests: BTreeMap::new(),
            quest_stages: BTreeMap::new(),
            script_flags: BTreeSet::new(),
            scheduled_script_flags: Vec::new(),
        }
    }
}
