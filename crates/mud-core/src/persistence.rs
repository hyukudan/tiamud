//! Portable runtime snapshot export and restore.

use super::*;

impl World {
    pub fn runtime_snapshot(&self) -> WorldRuntimeSnapshot {
        WorldRuntimeSnapshot {
            snapshot_version: WORLD_RUNTIME_SNAPSHOT_VERSION,
            revision: self.revision,
            logical_tick: self.logical_tick,
            rng_state: self.rng.state,
            next_item_instance_sequence: self.next_item_instance_sequence,
            item_instances: self
                .item_instances
                .iter()
                .map(|(item_id, instance)| {
                    let location = match &instance.location {
                        SharedItemLocation::Staged => RuntimeItemLocation::Staged,
                        SharedItemLocation::Carried(character_id) => RuntimeItemLocation::Carried {
                            character_id: *character_id,
                        },
                        SharedItemLocation::Room(room_id) => RuntimeItemLocation::Room {
                            room_id: room_id.clone(),
                        },
                        SharedItemLocation::Equipped(character_id, slot) => {
                            RuntimeItemLocation::Equipped {
                                character_id: *character_id,
                                slot: *slot,
                            }
                        }
                        SharedItemLocation::HeldByNpc(npc_id) => RuntimeItemLocation::HeldByNpc {
                            npc_id: npc_id.clone(),
                        },
                        SharedItemLocation::Contained(container_id) => {
                            RuntimeItemLocation::Contained {
                                container_id: container_id.clone(),
                            }
                        }
                        SharedItemLocation::Consumed => RuntimeItemLocation::Consumed,
                    };
                    (
                        item_id.clone(),
                        RuntimeItemInstanceSnapshot {
                            prototype_id: instance.prototype_id.clone(),
                            location,
                            binding: instance.binding,
                            item_affixes: instance.item_affixes.clone(),
                        },
                    )
                })
                .collect(),
            npcs: self
                .npcs
                .iter()
                .map(|(npc_id, npc)| {
                    (
                        npc_id.clone(),
                        NpcRuntimeSnapshot {
                            stable_id: npc.stable_id.clone(),
                            name: npc.name.clone(),
                            description: npc.description.clone(),
                            spawn_room_id: npc.spawn_room_id.clone(),
                            level: npc.level,
                            health: npc.health,
                            max_health: npc.max_health,
                            strength: npc.strength,
                            defence: npc.defence,
                            experience_reward: npc.experience_reward,
                            loot_item_ids: npc.loot_item_ids.clone(),
                            respawn_ticks: npc.respawn_ticks,
                            respawn_at_tick: npc.respawn_at_tick,
                            engaged_character: npc.engaged_character,
                            resources: npc.resources.clone(),
                            resource_regeneration: npc.resource_regeneration.clone(),
                            ability_ids: npc.ability_ids.clone(),
                            ability_cooldowns: npc.ability_cooldowns.clone(),
                            resistances: npc.resistances.clone(),
                            statuses: npc.statuses.clone(),
                        },
                    )
                })
                .collect(),
            adventure: self.adventure.clone(),
            progression_flags: self.world_progression_flags.clone(),
            next_trade_id: self.next_trade_id,
            trades: self.trades.clone(),
            next_loot_drop_id: self.next_loot_drop_id,
            item_provenance: self.item_provenance.clone(),
            loot_encounters: self.loot_encounters.clone(),
            ground_loot: self.ground_loot.clone(),
            party_loot_cursors: self.party_loot_cursors.clone(),
            next_reward_open_id: self.next_reward_open_id,
            area_residency: self
                .active_areas
                .iter()
                .map(|(area_id, active)| {
                    (
                        area_id.clone(),
                        AreaRuntimeResidence::Resident {
                            revision: active.revision,
                            bundle_sha256: active.bundle_sha256.clone(),
                            residency_epoch: active.residency_epoch,
                        },
                    )
                })
                .chain(self.retired_areas.iter().map(|(area_id, retired)| {
                    (
                        area_id.clone(),
                        AreaRuntimeResidence::Retired {
                            revision: retired.revision,
                            bundle_sha256: retired.bundle_sha256.clone(),
                            residency_epoch: retired.residency_epoch,
                            checkpoint_version: AREA_DYNAMIC_CHECKPOINT_VERSION,
                            checkpoint_sha256: retired.checkpoint_sha256.clone(),
                        },
                    )
                }))
                .collect(),
        }
    }

    pub(crate) fn apply_runtime_area_residency_manifest(
        &mut self,
        manifest: &BTreeMap<String, AreaRuntimeResidence>,
    ) -> Result<(), WorldError> {
        if manifest.len() != self.active_areas.len() + self.retired_areas.len() {
            return Err(WorldError::InvalidRuntimeSnapshot);
        }
        for (area_id, state) in manifest {
            match state {
                AreaRuntimeResidence::Resident {
                    revision,
                    bundle_sha256,
                    residency_epoch,
                } => {
                    let active = self
                        .active_areas
                        .get_mut(area_id)
                        .ok_or(WorldError::InvalidRuntimeSnapshot)?;
                    if active.revision != *revision || active.bundle_sha256 != *bundle_sha256 {
                        return Err(WorldError::InvalidRuntimeSnapshot);
                    }
                    active.residency_epoch = *residency_epoch;
                }
                AreaRuntimeResidence::Retired {
                    revision,
                    bundle_sha256,
                    residency_epoch,
                    checkpoint_version,
                    checkpoint_sha256,
                } => {
                    if *checkpoint_version != AREA_DYNAMIC_CHECKPOINT_VERSION {
                        return Err(WorldError::InvalidRuntimeSnapshot);
                    }
                    let retired = self
                        .retired_areas
                        .get_mut(area_id)
                        .ok_or(WorldError::InvalidRuntimeSnapshot)?;
                    if retired.revision != *revision
                        || retired.bundle_sha256 != *bundle_sha256
                        || retired.checkpoint_sha256 != *checkpoint_sha256
                    {
                        return Err(WorldError::InvalidRuntimeSnapshot);
                    }
                    retired.residency_epoch = *residency_epoch;
                }
            }
        }
        Ok(())
    }

    /// Restores a v8 snapshot that names retired areas without ever exposing
    /// the freshly authored instances from those areas. Callers must provide
    /// catalog-proved retirement units covering every retired manifest entry.
    pub fn restore_runtime_snapshot_with_retired_areas(
        &mut self,
        snapshot: WorldRuntimeSnapshot,
        units: Vec<AreaRetirementUnit>,
    ) -> Result<(), WorldError> {
        validate_world_runtime_snapshot_semantics(&snapshot)?;
        let retired_manifest = snapshot
            .area_residency
            .iter()
            .filter_map(|(area_id, state)| match state {
                AreaRuntimeResidence::Retired {
                    revision,
                    bundle_sha256,
                    checkpoint_sha256,
                    ..
                } => Some((
                    area_id.clone(),
                    (*revision, bundle_sha256.clone(), checkpoint_sha256.clone()),
                )),
                AreaRuntimeResidence::Resident { .. } => None,
            })
            .collect::<BTreeMap<_, _>>();
        let supplied_ids = units
            .iter()
            .flat_map(|unit| unit.areas.iter().map(|area| area.area_id.clone()))
            .collect::<Vec<_>>();
        if retired_manifest.is_empty()
            || supplied_ids.len() != retired_manifest.len()
            || supplied_ids.iter().collect::<BTreeSet<_>>().len() != supplied_ids.len()
            || supplied_ids
                .iter()
                .any(|area_id| !retired_manifest.contains_key(area_id))
        {
            return Err(WorldError::InvalidRuntimeSnapshot);
        }

        // The caller reconstructs and supplies the historical retirement
        // order. Sorting here would be incorrect: an earlier unit can remove
        // an inbound boundary that makes a later inverse closure evictable.
        let rollback = self.lifecycle_checkpoint();
        let restored = (|| {
            for mut unit in units {
                let mut checkpoint_sha256 = None;
                for area in &unit.areas {
                    let (revision, bundle_sha256, area_checkpoint_sha256) = retired_manifest
                        .get(&area.area_id)
                        .ok_or(WorldError::InvalidRuntimeSnapshot)?;
                    if area.revision != *revision || area.bundle_sha256 != *bundle_sha256 {
                        return Err(WorldError::InvalidRuntimeSnapshot);
                    }
                    match &checkpoint_sha256 {
                        Some(existing) if existing != area_checkpoint_sha256 => {
                            return Err(WorldError::InvalidRuntimeSnapshot);
                        }
                        None => checkpoint_sha256 = Some(area_checkpoint_sha256.clone()),
                        Some(_) => {}
                    }
                }
                unit.boundary_exits.retain(|request| {
                    self.stable_rooms.contains_key(&request.source_room_id)
                        && self.stable_rooms.contains_key(&request.target_room_id)
                });
                let prepared = self
                    .prepare_area_retirement(unit)
                    .map_err(|_| WorldError::InvalidRuntimeSnapshot)?;
                let authored_checkpoint_sha256 = canonical_sha256(&prepared.checkpoint)
                    .map_err(|_| WorldError::InvalidRuntimeSnapshot)?;
                self.apply_prepared_area_retirement(prepared, authored_checkpoint_sha256)
                    .map_err(|_| WorldError::InvalidRuntimeSnapshot)?;
                let durable_checkpoint_sha256 =
                    checkpoint_sha256.ok_or(WorldError::InvalidRuntimeSnapshot)?;
                for (area_id, retired) in &mut self.retired_areas {
                    if retired_manifest
                        .get(area_id)
                        .is_some_and(|(_, _, digest)| digest == &durable_checkpoint_sha256)
                    {
                        retired.checkpoint_sha256 = durable_checkpoint_sha256.clone();
                    }
                }
            }
            self.restore_runtime_snapshot_inner(snapshot)
        })();
        if let Err(error) = restored {
            self.restore_lifecycle_checkpoint(rollback);
            return Err(error);
        }
        Ok(())
    }

    pub fn restore_runtime_snapshot(
        &mut self,
        snapshot: WorldRuntimeSnapshot,
    ) -> Result<(), WorldError> {
        if snapshot
            .area_residency
            .values()
            .any(|state| matches!(state, AreaRuntimeResidence::Retired { .. }))
        {
            return Err(WorldError::InvalidRuntimeSnapshot);
        }
        let rollback = self.lifecycle_checkpoint();
        let restored = self.restore_runtime_snapshot_inner(snapshot);
        if let Err(error) = restored {
            self.restore_lifecycle_checkpoint(rollback);
            return Err(error);
        }
        Ok(())
    }

    pub(crate) fn restore_runtime_snapshot_inner(
        &mut self,
        mut snapshot: WorldRuntimeSnapshot,
    ) -> Result<(), WorldError> {
        validate_world_runtime_snapshot_semantics(&snapshot)?;
        self.apply_runtime_area_residency_manifest(&snapshot.area_residency)?;
        let item_ids_match = snapshot.item_instances.iter().all(|(item_id, instance)| {
            self.item_prototypes.contains_key(&instance.prototype_id)
                && match self.initial_item_prototypes.get(item_id) {
                    Some(prototype_id) => prototype_id == &instance.prototype_id,
                    None => runtime_item_sequence(item_id).is_some(),
                }
        });
        let npc_ids_match = snapshot
            .npcs
            .iter()
            .all(|(npc_id, npc)| npc_id == &npc.stable_id && self.npcs.contains_key(npc_id));
        if !item_ids_match || !npc_ids_match {
            return Err(WorldError::InvalidRuntimeSnapshot);
        }

        // WorldPack remains authoritative for static NPC configuration. The
        // portable snapshot overlays only dynamic state, so adding an NPC or
        // tuning a definition does not get undone on restart.
        let mut npcs = self.npcs.clone();
        for (npc_id, saved) in &snapshot.npcs {
            let authored = self
                .npcs
                .get(npc_id)
                .cloned()
                .ok_or(WorldError::InvalidRuntimeSnapshot)?;
            let max_stat_value = MAX_STAT_VALUE_I32;
            let valid = (1..=MAX_PROGRESSION_LEVEL).contains(&saved.level)
                && (1..=max_stat_value).contains(&saved.max_health)
                && (1..=max_stat_value).contains(&saved.strength)
                && (0..=max_stat_value).contains(&saved.defence)
                && saved.health >= 0
                && saved.health <= max_stat_value
                && saved.engaged_character.is_none_or(|id| id.0 > 0)
                && ((saved.health == 0) == saved.respawn_at_tick.is_some())
                && (saved.health > 0 || saved.engaged_character.is_none())
                && saved.resources.len() <= MAX_RESOURCE_POOLS
                && saved.resources.iter().all(|(id, pool)| {
                    valid_stat_id(id)
                        && pool.max > 0
                        && pool.max <= MAX_RESOURCE_VALUE
                        && pool.current <= pool.max
                })
                && saved.resource_regeneration.len() <= MAX_RESOURCE_POOLS
                && saved
                    .resource_regeneration
                    .iter()
                    .all(|(id, value)| valid_stat_id(id) && *value <= MAX_RESOURCE_VALUE)
                && saved.ability_ids.len() <= MAX_KNOWN_ABILITIES
                && saved.ability_ids.iter().all(|id| valid_definition_id(id))
                && saved.ability_cooldowns.len() <= MAX_KNOWN_ABILITIES
                && saved.ability_cooldowns.iter().all(|(id, ready_tick)| {
                    valid_definition_id(id)
                        && *ready_tick
                            <= snapshot
                                .logical_tick
                                .saturating_add(u64::from(MAX_COOLDOWN_TICKS))
                })
                && saved.resistances.len() <= MAX_RESISTANCES
                && saved
                    .resistances
                    .values()
                    .all(|value| *value <= MAX_RESISTANCE_PERCENT)
                && saved.statuses.len() <= MAX_ACTIVE_STATUSES
                && saved.statuses.iter().all(validate_active_status_shape);
            if !valid {
                return Err(WorldError::InvalidRuntimeSnapshot);
            }
            let legacy_additive_fields_missing = saved.resources.is_empty()
                && saved.resource_regeneration.is_empty()
                && saved.ability_ids.is_empty()
                && saved.ability_cooldowns.is_empty()
                && saved.resistances.is_empty()
                && saved.statuses.is_empty();
            let resources = authored
                .resources
                .iter()
                .map(|(id, configured)| {
                    let current = if legacy_additive_fields_missing {
                        configured.max
                    } else {
                        saved
                            .resources
                            .get(id)
                            .map(|pool| pool.current.min(configured.max))
                            .unwrap_or(configured.max)
                    };
                    (
                        id.clone(),
                        ResourcePool {
                            current,
                            max: configured.max,
                        },
                    )
                })
                .collect();
            let dynamic = npcs
                .get_mut(npc_id)
                .ok_or(WorldError::InvalidRuntimeSnapshot)?;
            dynamic.health = if saved.health == 0 {
                0
            } else {
                saved.health.min(authored.max_health)
            };
            dynamic.respawn_at_tick = saved.respawn_at_tick;
            dynamic.engaged_character = saved.engaged_character;
            dynamic.resources = resources;
            dynamic.ability_cooldowns = saved
                .ability_cooldowns
                .iter()
                .filter(|(id, _)| authored.ability_ids.contains(id))
                .map(|(id, tick)| (id.clone(), *tick))
                .collect();
            dynamic.statuses = saved
                .statuses
                .iter()
                .filter(|status| {
                    self.abilities.contains_key(&status.source_ability_id)
                        && status
                            .source_npc_id
                            .as_ref()
                            .is_none_or(|source| self.npcs.contains_key(source))
                        && validate_active_status(status, &self.abilities)
                })
                .cloned()
                .collect();
        }

        // An older snapshot cannot know about initial instances introduced by
        // a newer WorldPack. Preserve those authored instances from the fresh
        // world, then overlay every durable location from the snapshot. Do not
        // preserve runtime-created instances omitted by the snapshot.
        let mut item_instances = self
            .item_instances
            .iter()
            .filter(|(item_id, _)| self.initial_item_prototypes.contains_key(*item_id))
            .map(|(item_id, instance)| (item_id.clone(), instance.clone()))
            .collect::<BTreeMap<_, _>>();
        for (item_id, saved_instance) in &snapshot.item_instances {
            let location = match &saved_instance.location {
                RuntimeItemLocation::Staged => SharedItemLocation::Staged,
                RuntimeItemLocation::Carried { character_id } => {
                    SharedItemLocation::Carried(*character_id)
                }
                RuntimeItemLocation::Room { room_id } => {
                    if !self.stable_rooms.contains_key(room_id) {
                        return Err(WorldError::InvalidRuntimeSnapshot);
                    }
                    SharedItemLocation::Room(room_id.clone())
                }
                RuntimeItemLocation::Equipped { character_id, slot } => {
                    if self
                        .item_prototypes
                        .get(&saved_instance.prototype_id)
                        .is_none_or(|item| item.equipment_slot != Some(*slot))
                    {
                        return Err(WorldError::InvalidRuntimeSnapshot);
                    }
                    SharedItemLocation::Equipped(*character_id, *slot)
                }
                RuntimeItemLocation::HeldByNpc { npc_id } => {
                    if !npcs.contains_key(npc_id) {
                        return Err(WorldError::InvalidRuntimeSnapshot);
                    }
                    SharedItemLocation::HeldByNpc(npc_id.clone())
                }
                RuntimeItemLocation::Contained { container_id } => {
                    SharedItemLocation::Contained(container_id.clone())
                }
                RuntimeItemLocation::Consumed => SharedItemLocation::Consumed,
            };
            item_instances.insert(
                item_id.clone(),
                ItemInstance {
                    prototype_id: saved_instance.prototype_id.clone(),
                    location,
                    binding: saved_instance.binding,
                    item_affixes: saved_instance.item_affixes.clone(),
                },
            );
        }
        let mut item_provenance = item_instances
            .keys()
            .filter(|item_id| self.initial_item_prototypes.contains_key(*item_id))
            .cloned()
            .map(|item_id| (item_id, ItemProvenance::AuthoredInitial))
            .collect::<BTreeMap<_, _>>();
        for (item_id, provenance) in &snapshot.item_provenance {
            item_provenance.insert(item_id.clone(), provenance.clone());
        }
        if item_provenance.keys().ne(item_instances.keys()) {
            return Err(WorldError::InvalidRuntimeSnapshot);
        }

        if validate_item_graph(&item_instances, &self.item_prototypes).is_err()
            || validate_restored_item_affixes(&item_instances, &self.item_prototypes).is_err()
            || validate_restored_sealed_items(
                &item_instances,
                &item_provenance,
                &snapshot.ground_loot,
                &self.item_prototypes,
            )
            .is_err()
            || npcs.values().any(|npc| {
                npc.loot_item_ids
                    .iter()
                    .any(|item_id| !item_instances.contains_key(item_id))
            })
            || item_instances.iter().any(|(item_id, instance)| {
                matches!(
                    &instance.location,
                    SharedItemLocation::HeldByNpc(owner)
                        if npcs
                            .get(owner)
                            .is_none_or(|npc| !npc.loot_item_ids.contains(item_id))
                )
            })
        {
            return Err(WorldError::InvalidRuntimeSnapshot);
        }
        validate_trade_state(
            snapshot.next_trade_id,
            &snapshot.trades,
            &item_instances,
            &self.item_prototypes,
            &self.stable_rooms,
        )?;

        self.revision = snapshot.revision;
        self.logical_tick = snapshot.logical_tick;
        self.rng = DeterministicRng {
            state: snapshot.rng_state,
        };
        self.npcs = npcs;
        self.item_instances = item_instances;
        self.item_provenance = item_provenance;
        self.next_item_instance_sequence = snapshot
            .next_item_instance_sequence
            .max(self.next_item_instance_sequence);
        snapshot.adventure.next_weather_tick = snapshot
            .adventure
            .next_weather_tick
            .max(snapshot.logical_tick.saturating_add(1));
        self.adventure = snapshot.adventure;
        self.world_progression_flags = snapshot.progression_flags;
        self.next_trade_id = snapshot.next_trade_id;
        self.trades = snapshot.trades;
        self.next_loot_drop_id = snapshot.next_loot_drop_id;
        self.loot_encounters = snapshot.loot_encounters;
        self.ground_loot = snapshot.ground_loot;
        self.party_loot_cursors = snapshot.party_loot_cursors;
        self.next_reward_open_id = snapshot.next_reward_open_id;
        self.tick_resolution_pending = false;
        self.last_view_bump = None;
        Ok(())
    }
}
