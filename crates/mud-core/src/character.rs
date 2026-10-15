//! Character gameplay snapshots, authorized script effects and gameplay
//! validation.

use super::*;

impl World {
    pub fn character_gameplay_snapshot(
        &self,
        session: SessionId,
    ) -> Result<CharacterGameplaySnapshot, WorldError> {
        let character = self.character(session)?;
        self.character_gameplay
            .get(&character.id)
            .cloned()
            .ok_or(WorldError::InvalidEntity)
    }

    pub fn authorized_runtime_context(
        &self,
        session: SessionId,
    ) -> Result<AuthorizedRuntimeContext, WorldError> {
        let character = self.character(session)?;
        let room_id = room_checked(&self.entities, character.location)?
            .stable_id
            .clone();
        let gameplay = self
            .character_gameplay
            .get(&character.id)
            .ok_or(WorldError::InvalidEntity)?;
        let actor_quest_states = quest_state_strings(&self.adventure_content, gameplay);
        let actor_content_preferences = gameplay
            .script_flags
            .iter()
            .filter_map(|flag| flag.strip_prefix("preference.").map(str::to_owned))
            .collect();
        let actor_senses = perception_senses(gameplay)
            .into_iter()
            .map(|sense| match sense {
                Sense::Vision => "vision",
                Sense::Hearing => "hearing",
                Sense::Darkvision => "darkvision",
                Sense::Tremorsense => "tremorsense",
            })
            .map(str::to_owned)
            .collect();
        Ok(AuthorizedRuntimeContext {
            character_id: character.id,
            character_name: character.name.clone(),
            room_id,
            actor_level: gameplay.level,
            actor_flags: gameplay.script_flags.clone(),
            world_instance: "primary".into(),
            actor_quest_states,
            actor_content_preferences,
            actor_senses,
            logical_tick: self.logical_tick,
            weather: self.adventure.weather,
        })
    }

    /// Set a bounded, durable authoring flag for the current character. The
    /// flag is not emitted as a domain/view event, preventing secret condition
    /// state from becoming client-visible.
    pub fn set_authorized_script_flag(
        &mut self,
        session: SessionId,
        flag: &str,
    ) -> Result<bool, WorldError> {
        let flag = flag.trim();
        if !valid_script_flag(flag) {
            return Err(WorldError::InvalidScriptFlag);
        }
        let character_id = self.character(session)?.id;
        let gameplay = self
            .character_gameplay
            .get_mut(&character_id)
            .ok_or(WorldError::InvalidEntity)?;
        if gameplay.script_flags.contains(flag) {
            return Ok(false);
        }
        if gameplay.script_flags.len() >= 64 {
            return Err(WorldError::InvalidScriptFlag);
        }
        gameplay.script_flags.insert(flag.to_owned());
        self.mark_authorized_script_change(session)?;
        Ok(true)
    }

    pub fn apply_authorized_script_damage(
        &mut self,
        session: SessionId,
        amount: u32,
    ) -> Result<bool, WorldError> {
        if amount == 0 || amount > 10_000 {
            return Err(WorldError::InvalidScriptEffect);
        }
        let character_id = self.character(session)?.id;
        let tick = self.logical_tick;
        let changed = {
            let gameplay = self
                .character_gameplay
                .get_mut(&character_id)
                .ok_or(WorldError::InvalidEntity)?;
            let next = gameplay
                .vitals
                .health
                .saturating_sub(i32::try_from(amount).unwrap_or(i32::MAX))
                .max(0);
            if next == gameplay.vitals.health {
                false
            } else {
                gameplay.vitals.health = next;
                if next == 0 {
                    gameplay.recovery_at_tick = Some(tick.saturating_add(3));
                }
                true
            }
        };
        if changed {
            self.mark_authorized_script_change(session)?;
        }
        Ok(changed)
    }

    pub fn apply_authorized_script_heal(
        &mut self,
        session: SessionId,
        amount: u32,
    ) -> Result<bool, WorldError> {
        if amount == 0 || amount > 10_000 {
            return Err(WorldError::InvalidScriptEffect);
        }
        let character_id = self.character(session)?.id;
        let changed = {
            let gameplay = self
                .character_gameplay
                .get_mut(&character_id)
                .ok_or(WorldError::InvalidEntity)?;
            let next = gameplay
                .vitals
                .health
                .saturating_add(i32::try_from(amount).unwrap_or(i32::MAX))
                .min(gameplay.vitals.max_health);
            if next == gameplay.vitals.health {
                false
            } else {
                gameplay.vitals.health = next;
                if next > 0 {
                    gameplay.recovery_at_tick = None;
                }
                true
            }
        };
        if changed {
            self.mark_authorized_script_change(session)?;
        }
        Ok(changed)
    }

    pub fn give_authorized_script_item(
        &mut self,
        session: SessionId,
        item_id: &str,
    ) -> Result<Option<DomainEvent>, WorldError> {
        let character = self.character(session)?;
        let actor_id = character.id;
        let actor_name = character.name.clone();
        let room_id = room_checked(&self.entities, character.location)?
            .stable_id
            .clone();
        let item_id = ItemInstanceId(item_id.to_owned());
        let item = self
            .item_prototype_for(&item_id)
            .map_err(|_| WorldError::ItemNotFound(item_id.0.clone()))?;
        let item_name = item.name.clone();
        match self.item_location(&item_id) {
            Some(SharedItemLocation::Carried(owner)) if *owner == actor_id => return Ok(None),
            Some(SharedItemLocation::Room(owner_room)) if owner_room == &room_id => {}
            _ => return Err(WorldError::GameplayUnavailable),
        }
        let mut prospective = self.item_instances.clone();
        prospective
            .get_mut(&item_id)
            .ok_or(WorldError::InvalidEntity)?
            .location = SharedItemLocation::Carried(actor_id);
        self.apply_item_ownership_transitions(&mut prospective, std::slice::from_ref(&item_id))?;
        self.validate_prospective_item_locations(&prospective, [actor_id])?;
        self.item_instances = prospective;
        self.revision = self.revision.wrapping_add(1).max(1);
        let event = DomainEvent::ItemTaken {
            actor_id,
            actor_name,
            item_id,
            item_name,
            room_id,
        };
        self.bump_view_revisions(&event)?;
        Ok(Some(event))
    }

    /// Schedule a durable authoring flag. The generic Lua event encodes this
    /// as `<delay-ticks>:<flag>`; due flags are applied before the timer hook on
    /// the corresponding deterministic world tick.
    pub fn schedule_authorized_script_flag(
        &mut self,
        session: SessionId,
        delay_ticks: u64,
        flag: &str,
    ) -> Result<bool, WorldError> {
        let flag = flag.trim();
        if delay_ticks == 0 || delay_ticks > 86_400 || !valid_script_flag(flag) {
            return Err(WorldError::InvalidScriptEffect);
        }
        let character_id = self.character(session)?.id;
        let due_tick = self.logical_tick.saturating_add(delay_ticks);
        let changed = {
            let gameplay = self
                .character_gameplay
                .get_mut(&character_id)
                .ok_or(WorldError::InvalidEntity)?;
            if gameplay.script_flags.contains(flag)
                || gameplay
                    .scheduled_script_flags
                    .iter()
                    .any(|scheduled| scheduled.flag == flag)
            {
                false
            } else {
                if gameplay.script_flags.len() + gameplay.scheduled_script_flags.len() >= 64
                    || gameplay.scheduled_script_flags.len() >= 32
                {
                    return Err(WorldError::InvalidScriptEffect);
                }
                gameplay.scheduled_script_flags.push(ScheduledScriptFlag {
                    flag: flag.to_owned(),
                    due_tick,
                });
                gameplay.scheduled_script_flags.sort_by(|left, right| {
                    (left.due_tick, &left.flag).cmp(&(right.due_tick, &right.flag))
                });
                true
            }
        };
        if changed {
            self.mark_authorized_script_change(session)?;
        }
        Ok(changed)
    }

    pub(crate) fn mark_authorized_script_change(
        &mut self,
        session: SessionId,
    ) -> Result<(), WorldError> {
        self.revision = self.revision.wrapping_add(1).max(1);
        let observer = self
            .observer_views
            .get_mut(&session)
            .ok_or(WorldError::InvalidEntity)?;
        let previous = observer.revision;
        observer.revision = observer.revision.wrapping_add(1).max(1);
        self.last_view_bump = Some((self.revision, vec![(session, previous)]));
        Ok(())
    }

    pub fn durable_character_snapshot(
        &self,
        session: SessionId,
    ) -> Result<DurableCharacterSnapshot, WorldError> {
        Ok(DurableCharacterSnapshot {
            character: self.character_snapshot(session)?,
            gameplay: self.character_gameplay_snapshot(session)?,
        })
    }

    pub fn validate_character_gameplay_snapshot(
        &self,
        character_id: CharacterId,
        snapshot: &CharacterGameplaySnapshot,
    ) -> Result<(), WorldError> {
        // Saves are native v3 snapshots. Unknown content references fail
        // closed instead of being guessed or replayed additively.
        if !snapshot
            .ability_ids
            .iter()
            .all(|id| valid_definition_id(id))
            || !snapshot
                .ability_cooldowns
                .keys()
                .all(|id| valid_definition_id(id))
            || !snapshot.statuses.iter().all(validate_active_status_shape)
        {
            return Err(WorldError::InvalidGameplayDefinition);
        }
        let unique_equipment = snapshot
            .equipment
            .values()
            .collect::<std::collections::HashSet<_>>()
            .len()
            == snapshot.equipment.len();
        let unique_abilities = snapshot
            .ability_ids
            .iter()
            .collect::<std::collections::HashSet<_>>()
            .len()
            == snapshot.ability_ids.len();
        let valid = self
            .progression
            .validate_snapshot(snapshot, &self.abilities)
            && snapshot.level > 0
            && snapshot.level <= MAX_PROGRESSION_LEVEL
            && snapshot.vitals.max_health > 0
            && snapshot.vitals.max_health <= MAX_STAT_VALUE_I32
            && (0..=snapshot.vitals.max_health).contains(&snapshot.vitals.health)
            && snapshot.attributes.strength >= 0
            && snapshot.attributes.strength <= MAX_STAT_VALUE_I32
            && snapshot.attributes.defence >= 0
            && snapshot.attributes.defence <= MAX_STAT_VALUE_I32
            && snapshot.attributes.additional.len() + 2 <= MAX_STATS
            && snapshot.attributes.additional.iter().all(|(id, value)| {
                valid_stat_id(id)
                    && !matches!(id.as_str(), "strength" | "defence" | "max_health")
                    && (0..=MAX_STAT_VALUE_I32).contains(value)
            })
            && snapshot.resources.len() <= MAX_RESOURCE_POOLS
            && snapshot.resources.iter().all(|(id, pool)| {
                valid_stat_id(id)
                    && pool.max > 0
                    && pool.max <= MAX_RESOURCE_VALUE
                    && pool.current <= pool.max
            })
            && snapshot.resource_regeneration.len() <= snapshot.resources.len()
            && snapshot.resource_regeneration.iter().all(|(id, amount)| {
                snapshot.resources.contains_key(id) && *amount <= MAX_RESOURCE_VALUE
            })
            && snapshot.ability_ids.len() <= MAX_KNOWN_ABILITIES
            && unique_abilities
            && snapshot
                .ability_ids
                .iter()
                .all(|id| self.abilities.contains_key(id))
            && snapshot.ability_cooldowns.len() <= MAX_KNOWN_ABILITIES
            && snapshot.ability_cooldowns.iter().all(|(id, ready_tick)| {
                snapshot.ability_ids.contains(id)
                    && *ready_tick
                        <= self
                            .logical_tick
                            .saturating_add(u64::from(MAX_COOLDOWN_TICKS))
            })
            && snapshot.ability_global_ready_tick <= self.logical_tick.saturating_add(1)
            && snapshot
                .last_offensive_ability_tick
                .is_none_or(|tick| tick <= self.logical_tick)
            && snapshot.resistances.len() <= MAX_RESISTANCES
            && snapshot
                .resistances
                .values()
                .all(|value| *value <= MAX_RESISTANCE_PERCENT)
            && snapshot.statuses.len() <= MAX_ACTIVE_STATUSES
            && snapshot.statuses.iter().all(|status| {
                validate_active_status(status, &self.abilities)
                    && status
                        .source_npc_id
                        .as_ref()
                        .is_none_or(|npc_id| self.npcs.contains_key(npc_id))
            })
            && ((snapshot.vitals.health == 0) == snapshot.recovery_at_tick.is_some())
            && snapshot.skills.len() <= MAX_STATS
            && snapshot
                .skills
                .iter()
                .all(|(id, rank)| valid_stat_id(id) && *rank <= MAX_SKILL_VALUE)
            && unique_equipment
            && snapshot.equipment.iter().all(|(slot, item_id)| {
                self.item_instances.contains_key(item_id)
                    && self
                        .item_gameplay_for(item_id)
                        .ok()
                        .is_some_and(|item| item.equipment_slot == Some(*slot))
                    && matches!(
                        self.item_location(item_id),
                        Some(SharedItemLocation::Equipped(owner, equipped_slot))
                            if *owner == character_id && *equipped_slot == *slot
                    )
            })
            && snapshot.effects.iter().all(|effect| {
                (1..=MAX_STATUS_TICKS).contains(&effect.remaining_ticks)
                    && (1..=MAX_STATUS_MAGNITUDE).contains(&effect.magnitude)
            })
            && snapshot.effects.len() <= MAX_ACTIVE_EFFECTS;
        let adventure_valid = valid_character_adventure_state(&self.adventure_content, snapshot)
            && snapshot.script_flags.len() <= 64
            && snapshot
                .script_flags
                .iter()
                .all(|flag| valid_script_flag(flag))
            && snapshot.scheduled_script_flags.len() <= 32
            && snapshot
                .scheduled_script_flags
                .iter()
                .all(|scheduled| scheduled.due_tick > 0 && valid_script_flag(&scheduled.flag))
            && snapshot.script_flags.len() + snapshot.scheduled_script_flags.len() <= 64;
        let equipment_locations_exact =
            self.item_instances
                .iter()
                .all(|(item_id, instance)| match &instance.location {
                    SharedItemLocation::Equipped(owner, slot) if *owner == character_id => {
                        snapshot.equipment.get(slot) == Some(item_id)
                    }
                    _ => true,
                });
        let capacity_valid = snapshot.inventory_capacity.max_slots > 0
            && snapshot.inventory_capacity.max_slots <= 1_000
            && snapshot.inventory_capacity.max_weight_grams <= 1_000_000
            && self.inventory_usage_for_capacity(character_id, &snapshot.inventory_capacity)?;
        if !valid || !adventure_valid || !equipment_locations_exact || !capacity_valid {
            return Err(WorldError::InvalidGameplayDefinition);
        }
        Ok(())
    }

    pub fn restore_character_gameplay(
        &mut self,
        character_id: CharacterId,
        snapshot: CharacterGameplaySnapshot,
    ) -> Result<(), WorldError> {
        self.validate_character_gameplay_snapshot(character_id, &snapshot)?;
        self.character_gameplay.insert(character_id, snapshot);
        Ok(())
    }

    pub fn runtime_item_owner_ids(&self) -> BTreeSet<CharacterId> {
        self.item_instances
            .iter()
            .filter_map(|(item_id, instance)| {
                self.item_root_owner(item_id)
                    .ok()
                    .flatten()
                    .or(match instance.binding {
                        ItemBinding::Unbound => None,
                        ItemBinding::Character { character_id } => Some(character_id),
                    })
            })
            .collect()
    }

    pub(crate) fn item_root_owner(
        &self,
        item_id: &ItemInstanceId,
    ) -> Result<Option<CharacterId>, WorldError> {
        let mut current = item_id;
        for _ in 0..=MAX_CONTAINER_DEPTH {
            let instance = self
                .item_instances
                .get(current)
                .ok_or(WorldError::InvalidEntity)?;
            match &instance.location {
                SharedItemLocation::Carried(character_id)
                | SharedItemLocation::Equipped(character_id, _) => {
                    return Ok(Some(*character_id));
                }
                SharedItemLocation::Contained(parent) => current = parent,
                SharedItemLocation::Staged
                | SharedItemLocation::Room(_)
                | SharedItemLocation::HeldByNpc(_)
                | SharedItemLocation::Consumed => return Ok(None),
            }
        }
        Err(WorldError::InvalidEntity)
    }
}
