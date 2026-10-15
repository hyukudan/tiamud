//! Logical clock: ticks, effect expiry, NPC respawn/status and combat
//! turns.

use super::*;

impl World {
    /// Advances deterministic world time by one turn and resolves effects,
    /// recovery, respawns and every active combat in stable NPC-ID order.
    pub fn advance_tick(&mut self) -> Result<Vec<DomainEvent>, WorldError> {
        let checkpoint = self.checkpoint();
        let advanced = (|| {
            self.begin_tick()?;
            let mut events = self.cancel_expired_trades()?;
            events.extend(self.expire_ground_loot()?);
            events.extend(self.resolve_tick_after_expiry()?);
            Ok(events)
        })();
        match advanced {
            Ok(events) => Ok(events),
            Err(error) => {
                self.restore_checkpoint(checkpoint);
                Err(error)
            }
        }
    }

    /// Opens a logical tick without resolving effects, respawns or combat.
    /// The actor cancels expired trades and expires ground loot next, then
    /// calls `resolve_tick_after_expiry` exactly once.
    pub fn begin_tick(&mut self) -> Result<(), WorldError> {
        if self.tick_resolution_pending {
            return Err(WorldError::InvalidDomainTransition);
        }
        let next_tick = self
            .logical_tick
            .checked_add(1)
            .ok_or(WorldError::InvalidDomainTransition)?;
        self.logical_tick = next_tick;
        self.tick_resolution_pending = true;
        Ok(())
    }

    /// Resolves the tick opened by `begin_tick`. Expiration must already have
    /// been processed, so combat cannot observe an expired reservation.
    pub fn resolve_tick_after_expiry(&mut self) -> Result<Vec<DomainEvent>, WorldError> {
        if !self.tick_resolution_pending {
            return Err(WorldError::InvalidDomainTransition);
        }
        let checkpoint = self.checkpoint();
        match self.resolve_tick_after_expiry_inner() {
            Ok(events) => Ok(events),
            Err(error) => {
                self.restore_checkpoint(checkpoint);
                Err(error)
            }
        }
    }

    pub(crate) fn resolve_tick_after_expiry_inner(
        &mut self,
    ) -> Result<Vec<DomainEvent>, WorldError> {
        let tick = self.logical_tick;
        let mut events = vec![DomainEvent::Gameplay {
            event: GameplayEvent::TickAdvanced { tick },
        }];
        self.resolve_character_tick(tick, &mut events)?;
        self.resolve_npc_statuses(tick, &mut events)?;
        self.resolve_npc_respawns(tick, &mut events);

        let combatants = self
            .npcs
            .iter()
            .filter_map(|(npc_id, npc)| npc.engaged_character.map(|owner| (npc_id.clone(), owner)))
            .collect::<Vec<_>>();
        for (npc_id, character_id) in combatants {
            self.resolve_combat_turn(tick, &npc_id, character_id, &mut events)?;
        }
        // Periodic effects are resolved at the start of the tick, but statuses
        // that reach zero must remain visible through this tick's combat. This
        // makes an authored duration of N affect exactly N logical turns while
        // leaving statuses applied during combat untouched until the next tick.
        self.expire_processed_effects();
        let defeated = events
            .iter()
            .filter_map(|event| match event {
                DomainEvent::Gameplay {
                    event: GameplayEvent::NpcDefeated { character_id, .. },
                } => Some(*character_id),
                _ => None,
            })
            .collect::<Vec<_>>();
        for character_id in defeated {
            // TODO(progression-signals): replace this legacy kill-side skill
            // increment once WorldPack can author event-triggered GrantSets.
            // Removing it before that hook exists would silently change the
            // existing combat/quest loop rather than make it data-driven.
            if let Some(rank) = self
                .character_gameplay
                .get_mut(&character_id)
                .and_then(|state| state.skills.get_mut("combat"))
            {
                *rank = rank.saturating_add(1).min(MAX_SKILL_VALUE);
            }
            if let Some(progress) = self
                .character_gameplay
                .get_mut(&character_id)
                .and_then(|state| state.quests.get_mut("first-hunt"))
                && progress.status == QuestStatus::Active
            {
                progress.current = progress.current.saturating_add(1).min(progress.target);
                if progress.current >= progress.target {
                    progress.status = QuestStatus::ReadyToClaim;
                }
                events.push(DomainEvent::Adventure {
                    event: AdventureEvent::QuestChanged {
                        character_id,
                        quest_id: "first-hunt".into(),
                        summary: "First Hunt is ready to claim.".into(),
                    },
                });
            }
        }
        if self
            .adventure
            .parties
            .values()
            .any(|party| party.invitations.values().any(|expiry| *expiry < tick))
        {
            self.adventure
                .parties
                .values_mut()
                .for_each(|party| party.invitations.retain(|_, expiry| *expiry >= tick));
        }
        if tick >= self.adventure.next_weather_tick {
            let choices = [
                WeatherKind::Clear,
                WeatherKind::Rain,
                WeatherKind::Wind,
                WeatherKind::Fog,
            ];
            let previous = self.adventure.weather;
            let offset = (self.rng.next_u64() % 3 + 1) as usize;
            let previous_index = choices
                .iter()
                .position(|weather| *weather == previous)
                .unwrap_or(0);
            self.adventure.weather = choices[(previous_index + offset) % choices.len()];
            self.adventure.next_weather_tick = tick.saturating_add(5);
            events.push(DomainEvent::Adventure {
                event: AdventureEvent::WeatherChanged {
                    weather: self.adventure.weather,
                    tick,
                },
            });
        }
        self.revision = self
            .revision
            .checked_add(1)
            .ok_or(WorldError::InvalidDomainTransition)?;
        self.bump_view_revisions_for_events(&events)?;
        self.tick_resolution_pending = false;
        Ok(events)
    }

    pub(crate) fn resolve_character_tick(
        &mut self,
        tick: u64,
        events: &mut Vec<DomainEvent>,
    ) -> Result<(), WorldError> {
        let connected = self.connected_sessions();
        for session in connected {
            let character = self.character(session)?;
            let character_id = character.id;
            let character_name = character.name.clone();
            let current_room_id = room_checked(&self.entities, character.location)?
                .stable_id
                .clone();
            // A quiet character must not take a mutable borrow: copy-on-write
            // state and durable dirty tracking treat every mutable access as
            // a change. The predicate mirrors every mutation below exactly.
            if self
                .character_gameplay
                .get(&character_id)
                .ok_or(WorldError::InvalidEntity)
                .map(|state| character_tick_is_noop(state, tick))?
            {
                continue;
            }
            let resource_regeneration = self
                .character_gameplay
                .get(&character_id)
                .map(|state| state.resource_regeneration.clone())
                .unwrap_or_default();
            let state = self
                .character_gameplay
                .get_mut(&character_id)
                .ok_or(WorldError::InvalidEntity)?;
            let due_flags = state
                .scheduled_script_flags
                .iter()
                .filter(|scheduled| scheduled.due_tick <= tick)
                .map(|scheduled| scheduled.flag.clone())
                .collect::<Vec<_>>();
            state
                .scheduled_script_flags
                .retain(|scheduled| scheduled.due_tick > tick);
            for flag in due_flags {
                if state.script_flags.len() < 64 {
                    state.script_flags.insert(flag);
                }
            }
            for (resource_id, amount) in &resource_regeneration {
                if let Some(pool) = state.resources.get_mut(resource_id) {
                    pool.current = pool.current.saturating_add(*amount).min(pool.max);
                }
            }
            state
                .ability_cooldowns
                .retain(|_, ready_tick| *ready_tick > tick);
            let mut recovered_at_start = false;
            if state
                .recovery_at_tick
                .is_some_and(|recovery| recovery <= tick)
            {
                state.recovery_at_tick = None;
                state.vitals.health = state.vitals.max_health;
                let character_entity = *self
                    .sessions
                    .get(&session)
                    .ok_or(WorldError::UnknownSession)?;
                match self.entities.get_mut(character_entity) {
                    Some(Entity::Character(character)) => character.location = self.start_room,
                    _ => return Err(WorldError::InvalidEntity),
                }
                recovered_at_start = true;
                events.push(DomainEvent::Gameplay {
                    event: GameplayEvent::CharacterRecovered {
                        character_id,
                        character_name: character_name.clone(),
                        room_id: self.start_room_stable_id.clone(),
                    },
                });
            }
            let room_id = if recovered_at_start {
                self.start_room_stable_id.clone()
            } else {
                current_room_id
            };
            let mut retained = Vec::new();
            for mut effect in std::mem::take(&mut state.effects) {
                if effect.kind == EffectKind::Regeneration && state.vitals.health > 0 {
                    let previous = state.vitals.health;
                    state.vitals.health = state
                        .vitals
                        .health
                        .saturating_add(effect.magnitude.max(0))
                        .min(state.vitals.max_health);
                    let amount = state.vitals.health.saturating_sub(previous);
                    if amount > 0 {
                        events.push(DomainEvent::Gameplay {
                            event: GameplayEvent::CharacterHealed {
                                character_id,
                                character_name: character_name.clone(),
                                amount,
                                health: state.vitals.health,
                                room_id: room_id.clone(),
                            },
                        });
                    }
                }
                effect.remaining_ticks -= 1;
                if effect.remaining_ticks == 0 {
                    events.push(DomainEvent::Gameplay {
                        event: GameplayEvent::EffectExpired {
                            character_id,
                            character_name: character_name.clone(),
                            kind: effect.kind,
                            room_id: room_id.clone(),
                        },
                    });
                }
                retained.push(effect);
            }
            state.effects = retained;
            let mut retained_statuses = Vec::new();
            let mut defeating_npc_id = None;
            for mut status in std::mem::take(&mut state.statuses) {
                if state.vitals.health == 0 {
                    break;
                }
                if state.vitals.health > 0 {
                    match status.kind {
                        ActiveStatusKind::Regeneration => {
                            let previous = state.vitals.health;
                            state.vitals.health = state
                                .vitals
                                .health
                                .saturating_add(status.magnitude.max(0))
                                .min(state.vitals.max_health);
                            let amount = state.vitals.health.saturating_sub(previous);
                            if amount > 0 {
                                events.push(DomainEvent::Gameplay {
                                    event: GameplayEvent::CharacterHealed {
                                        character_id,
                                        character_name: character_name.clone(),
                                        amount,
                                        health: state.vitals.health,
                                        room_id: room_id.clone(),
                                    },
                                });
                            }
                        }
                        ActiveStatusKind::DamageOverTime => {
                            let resistance = status
                                .damage_type
                                .and_then(|kind| state.resistances.get(&kind).copied())
                                .unwrap_or(0);
                            let amount = resisted_amount(status.magnitude.max(0), resistance);
                            state.vitals.health = state.vitals.health.saturating_sub(amount).max(0);
                            if state.vitals.health == 0 {
                                defeating_npc_id = status.source_npc_id.clone();
                            }
                        }
                        ActiveStatusKind::AttributeModifier | ActiveStatusKind::Stun => {}
                    }
                }
                status.remaining_ticks -= 1;
                if state.vitals.health > 0 {
                    retained_statuses.push(status);
                }
            }
            state.statuses = retained_statuses;
            let collapsed = state.vitals.health == 0 && state.recovery_at_tick.is_none();
            let recover_at_tick = tick.saturating_add(3);
            if collapsed {
                state.recovery_at_tick = Some(recover_at_tick);
                state.statuses.clear();
            }
            if collapsed {
                let mut released_npcs = Vec::new();
                for npc in self.npcs.values_mut() {
                    if npc.engaged_character == Some(character_id) {
                        npc.engaged_character = None;
                        released_npcs.push(npc.stable_id.clone());
                    }
                }
                for npc_id in released_npcs {
                    self.loot_encounters.remove(&npc_id);
                }
                if let Some(npc_id) = defeating_npc_id
                    && let Some(npc) = self.npcs.get(&npc_id)
                {
                    events.push(DomainEvent::Gameplay {
                        event: GameplayEvent::CharacterDefeated {
                            character_id,
                            character_name: character_name.clone(),
                            npc_id,
                            npc_name: npc.name.clone(),
                            recover_at_tick,
                            room_id,
                        },
                    });
                } else {
                    events.push(DomainEvent::Gameplay {
                        event: GameplayEvent::CharacterCollapsed {
                            character_id,
                            character_name: character_name.clone(),
                            recover_at_tick,
                            room_id,
                        },
                    });
                }
            }
        }
        Ok(())
    }

    pub(crate) fn resolve_npc_respawns(&mut self, tick: u64, events: &mut Vec<DomainEvent>) {
        if !self
            .npcs
            .values()
            .any(|npc| npc.respawn_at_tick.is_some_and(|respawn| respawn <= tick))
        {
            return;
        }
        for npc in self.npcs.values_mut() {
            if npc.respawn_at_tick.is_some_and(|respawn| respawn <= tick) {
                self.loot_encounters.remove(&npc.stable_id);
                npc.respawn_at_tick = None;
                npc.health = npc.max_health;
                npc.engaged_character = None;
                npc.statuses.clear();
                npc.ability_cooldowns.clear();
                for pool in npc.resources.values_mut() {
                    pool.current = pool.max;
                }
                events.push(DomainEvent::Gameplay {
                    event: GameplayEvent::NpcRespawned {
                        npc_id: npc.stable_id.clone(),
                        npc_name: npc.name.clone(),
                        room_id: npc.spawn_room_id.clone(),
                    },
                });
            }
        }
    }

    pub(crate) fn resolve_npc_statuses(
        &mut self,
        tick: u64,
        events: &mut Vec<DomainEvent>,
    ) -> Result<(), WorldError> {
        let mut defeated = Vec::new();
        if self.npcs.values().all(|npc| npc_tick_is_noop(npc, tick)) {
            return Ok(());
        }
        for (npc_id, npc) in &mut self.npcs {
            if !npc.alive() {
                continue;
            }
            for (resource_id, amount) in &npc.resource_regeneration {
                if let Some(pool) = npc.resources.get_mut(resource_id) {
                    pool.current = pool.current.saturating_add(*amount).min(pool.max);
                }
            }
            npc.ability_cooldowns
                .retain(|_, ready_tick| *ready_tick > tick);
            let mut retained = Vec::new();
            let mut defeating_source = None;
            for mut status in std::mem::take(&mut npc.statuses) {
                if npc.health == 0 {
                    break;
                }
                match status.kind {
                    ActiveStatusKind::Regeneration => {
                        npc.health = npc
                            .health
                            .saturating_add(status.magnitude.max(0))
                            .min(npc.max_health);
                    }
                    ActiveStatusKind::DamageOverTime => {
                        let resistance = status
                            .damage_type
                            .and_then(|kind| npc.resistances.get(&kind).copied())
                            .unwrap_or(0);
                        let amount = resisted_amount(status.magnitude.max(0), resistance);
                        npc.health = npc.health.saturating_sub(amount).max(0);
                        if npc.health == 0 {
                            defeating_source = status.source_character_id;
                        }
                    }
                    ActiveStatusKind::AttributeModifier | ActiveStatusKind::Stun => {}
                }
                status.remaining_ticks -= 1;
                if npc.health > 0 {
                    retained.push(status);
                }
            }
            npc.statuses = retained;
            if npc.health == 0 {
                defeated.push((npc_id.clone(), defeating_source));
            }
        }

        for (npc_id, source) in defeated {
            if let Some(character_id) = source
                && self.character_gameplay.contains_key(&character_id)
            {
                let character_name = if let Some(session) = self.session_for_character(character_id)
                {
                    self.character(session)?.name.clone()
                } else {
                    "A departed adventurer".to_owned()
                };
                let room_id = self
                    .npcs
                    .get(&npc_id)
                    .ok_or(WorldError::NpcNotFound)?
                    .spawn_room_id
                    .clone();
                self.resolve_npc_defeat(
                    tick,
                    &npc_id,
                    character_id,
                    &character_name,
                    &room_id,
                    events,
                )?;
            } else if self.npcs.contains_key(&npc_id) {
                self.loot_encounters.remove(&npc_id);
                let (npc_name, room_id, loot) = {
                    let npc = self.npcs.get_mut(&npc_id).ok_or(WorldError::NpcNotFound)?;
                    npc.engaged_character = None;
                    npc.statuses.clear();
                    npc.ability_cooldowns.clear();
                    npc.respawn_at_tick = Some(tick.saturating_add(npc.respawn_ticks));
                    (
                        npc.name.clone(),
                        npc.spawn_room_id.clone(),
                        npc.loot_item_ids.clone(),
                    )
                };
                for item_id in loot {
                    if matches!(
                        self.item_location(&item_id),
                        Some(SharedItemLocation::HeldByNpc(owner)) if owner == &npc_id
                    ) {
                        self.set_item_location(
                            &item_id,
                            SharedItemLocation::Room(room_id.clone()),
                        )?;
                        let item_name = self.item_effective_name(&item_id)?;
                        events.push(DomainEvent::Gameplay {
                            event: GameplayEvent::LootDropped {
                                npc_id: npc_id.clone(),
                                npc_name: npc_name.clone(),
                                item_id,
                                item_name,
                                room_id: room_id.clone(),
                            },
                        });
                    }
                }
            }
        }
        Ok(())
    }

    pub(crate) fn expire_processed_effects(&mut self) {
        if self.character_gameplay.values().any(|state| {
            state
                .effects
                .iter()
                .any(|effect| effect.remaining_ticks == 0)
                || state
                    .statuses
                    .iter()
                    .any(|status| status.remaining_ticks == 0)
        }) {
            for state in self.character_gameplay.values_mut() {
                state.effects.retain(|effect| effect.remaining_ticks > 0);
                state.statuses.retain(|status| status.remaining_ticks > 0);
            }
        }
        if self.npcs.values().any(|npc| {
            npc.statuses
                .iter()
                .any(|status| status.remaining_ticks == 0)
        }) {
            for npc in self.npcs.values_mut() {
                npc.statuses.retain(|status| status.remaining_ticks > 0);
            }
        }
    }

    pub(crate) fn resolve_combat_turn(
        &mut self,
        tick: u64,
        npc_id: &str,
        character_id: CharacterId,
        events: &mut Vec<DomainEvent>,
    ) -> Result<(), WorldError> {
        let Some(session) = self.session_for_character(character_id) else {
            if let Some(npc) = self.npcs.get_mut(npc_id) {
                npc.engaged_character = None;
            }
            self.loot_encounters.remove(npc_id);
            return Ok(());
        };
        let character = self.character(session)?;
        let character_name = character.name.clone();
        let room_id = room_checked(&self.entities, character.location)?
            .stable_id
            .clone();
        let state = self
            .character_gameplay
            .get(&character_id)
            .cloned()
            .ok_or(WorldError::InvalidEntity)?;
        let npc_snapshot = self
            .npcs
            .get(npc_id)
            .cloned()
            .ok_or(WorldError::NpcNotFound)?;
        if !npc_snapshot.alive()
            || npc_snapshot.spawn_room_id != room_id
            || state.vitals.health <= 0
        {
            if let Some(npc) = self.npcs.get_mut(npc_id) {
                npc.engaged_character = None;
            }
            self.loot_encounters.remove(npc_id);
            return Ok(());
        }

        let replaces_normal_attack =
            state.last_offensive_ability_tick == Some(tick.saturating_sub(1));
        let player_stunned = state
            .statuses
            .iter()
            .any(|status| status.kind == ActiveStatusKind::Stun);
        if !replaces_normal_attack && !player_stunned {
            let equipment_bonus = state
                .equipment
                .values()
                .filter_map(|item_id| self.item_effective_attack_bonus(item_id).ok())
                .fold(0_i32, i32::saturating_add);
            let effect_bonus = state
                .effects
                .iter()
                .filter(|effect| effect.kind == EffectKind::Strength)
                .map(|effect| effect.magnitude)
                .fold(0_i32, i32::saturating_add);
            let player_attack = effective_gameplay_attribute(&state, "strength")
                .saturating_add(i32::try_from(state.level).unwrap_or(i32::MAX))
                .saturating_add(equipment_bonus)
                .saturating_add(effect_bonus);
            let player_damage = resisted_amount(
                player_attack
                    .saturating_sub(effective_npc_attribute(&npc_snapshot, "defence"))
                    .saturating_add(self.rng.range_inclusive(0, 2))
                    .max(1),
                npc_snapshot
                    .resistances
                    .get(&DamageType::Physical)
                    .copied()
                    .unwrap_or(0),
            );
            let remaining_npc_health = {
                let npc = self.npcs.get_mut(npc_id).ok_or(WorldError::NpcNotFound)?;
                npc.health = npc.health.saturating_sub(player_damage).max(0);
                npc.health
            };
            events.push(DomainEvent::Gameplay {
                event: GameplayEvent::NpcDamaged {
                    character_id,
                    character_name: character_name.clone(),
                    npc_id: npc_id.to_owned(),
                    npc_name: npc_snapshot.name.clone(),
                    amount: player_damage,
                    remaining_health: remaining_npc_health,
                    room_id: room_id.clone(),
                },
            });
            if remaining_npc_health == 0 {
                self.resolve_npc_defeat(
                    tick,
                    npc_id,
                    character_id,
                    &character_name,
                    &room_id,
                    events,
                )?;
                return Ok(());
            }
        }

        let acting_npc = self
            .npcs
            .get(npc_id)
            .cloned()
            .ok_or(WorldError::NpcNotFound)?;
        if acting_npc
            .statuses
            .iter()
            .any(|status| status.kind == ActiveStatusKind::Stun)
        {
            return Ok(());
        }
        let npc_ability = acting_npc
            .ability_ids
            .iter()
            .filter_map(|id| self.abilities.get(id))
            .filter(|ability| {
                acting_npc
                    .ability_cooldowns
                    .get(&ability.id)
                    .is_none_or(|ready| *ready <= tick)
                    && ability.resource_costs.iter().all(|(id, cost)| {
                        acting_npc
                            .resources
                            .get(id)
                            .is_some_and(|pool| pool.current >= *cost)
                    })
                    && state.statuses.len() + npc_enemy_status_count(ability) <= MAX_ACTIVE_STATUSES
                    && acting_npc.statuses.len() + npc_self_status_count(ability)
                        <= MAX_ACTIVE_STATUSES
                    && npc_ability_score(ability, &acting_npc) > 0
            })
            .max_by(|left, right| {
                npc_ability_score(left, &acting_npc)
                    .cmp(&npc_ability_score(right, &acting_npc))
                    .then_with(|| right.id.cmp(&left.id))
            })
            .cloned();
        let mut player_damage = None;
        if let Some(ability) = &npc_ability {
            let npc = self.npcs.get_mut(npc_id).ok_or(WorldError::NpcNotFound)?;
            for (resource_id, cost) in &ability.resource_costs {
                npc.resources
                    .get_mut(resource_id)
                    .ok_or(WorldError::InsufficientResource)?
                    .current -= *cost;
            }
            if ability.cooldown_ticks > 0 {
                npc.ability_cooldowns.insert(
                    ability.id.clone(),
                    tick.saturating_add(u64::from(ability.cooldown_ticks)),
                );
            }
            events.push(DomainEvent::Gameplay {
                event: GameplayEvent::NpcAbilityUsed {
                    npc_id: npc_id.to_owned(),
                    npc_name: npc_snapshot.name.clone(),
                    character_id,
                    character_name: character_name.clone(),
                    ability_id: ability.id.clone(),
                    ability_name: ability.name.clone(),
                    room_id: room_id.clone(),
                },
            });

            if let Some(damage) = &ability.damage {
                let amount = ability_scaled_amount_npc(
                    damage.base_amount,
                    damage.scaling_attribute.as_deref(),
                    damage.scaling_percent,
                    &acting_npc,
                );
                if npc_effect_targets_enemy(ability, damage.target) {
                    player_damage = Some((amount, damage.damage_type));
                } else {
                    let resistance = self.npcs[npc_id]
                        .resistances
                        .get(&damage.damage_type)
                        .copied()
                        .unwrap_or(0);
                    let amount = resisted_amount(amount, resistance);
                    let npc = self.npcs.get_mut(npc_id).ok_or(WorldError::NpcNotFound)?;
                    npc.health = npc.health.saturating_sub(amount).max(0);
                }
            }
            if let Some(heal) = &ability.heal
                && !npc_effect_targets_enemy(ability, heal.target)
            {
                let amount = ability_scaled_amount_npc(
                    heal.base_amount,
                    heal.scaling_attribute.as_deref(),
                    heal.scaling_percent,
                    &acting_npc,
                );
                let npc = self.npcs.get_mut(npc_id).ok_or(WorldError::NpcNotFound)?;
                npc.health = npc.health.saturating_add(amount).min(npc.max_health);
            }
            if self.npcs.get(npc_id).is_some_and(|npc| npc.health > 0) {
                for status in &ability.status_effects {
                    if !npc_effect_targets_enemy(ability, status_target(status)) {
                        self.npcs
                            .get_mut(npc_id)
                            .ok_or(WorldError::NpcNotFound)?
                            .statuses
                            .push(active_status(&ability.id, None, Some(npc_id), status));
                    }
                }
            }
            if self.npcs.get(npc_id).is_some_and(|npc| npc.health == 0) {
                self.resolve_npc_defeat(
                    tick,
                    npc_id,
                    character_id,
                    &character_name,
                    &room_id,
                    events,
                )?;
                return Ok(());
            }
        } else {
            player_damage = Some((
                effective_npc_attribute(&npc_snapshot, "strength")
                    .saturating_add(i32::try_from(npc_snapshot.level).unwrap_or(i32::MAX))
                    .saturating_sub(effective_gameplay_attribute(&state, "defence"))
                    .saturating_add(self.rng.range_inclusive(0, 2))
                    .max(1),
                DamageType::Physical,
            ));
        }
        let npc_damage = player_damage.map(|(amount, damage_type)| {
            resisted_amount(
                amount,
                state.resistances.get(&damage_type).copied().unwrap_or(0),
            )
        });
        let player_state = self
            .character_gameplay
            .get_mut(&character_id)
            .ok_or(WorldError::InvalidEntity)?;
        if let Some(npc_damage) = npc_damage {
            player_state.vitals.health =
                player_state.vitals.health.saturating_sub(npc_damage).max(0);
        }
        if player_state.vitals.health > 0
            && let Some(ability) = &npc_ability
        {
            for status in &ability.status_effects {
                if npc_effect_targets_enemy(ability, status_target(status)) {
                    player_state.statuses.push(active_status(
                        &ability.id,
                        None,
                        Some(npc_id),
                        status,
                    ));
                }
            }
        }
        let remaining_health = player_state.vitals.health;
        if let Some(npc_damage) = npc_damage {
            events.push(DomainEvent::Gameplay {
                event: GameplayEvent::CharacterDamaged {
                    character_id,
                    character_name: character_name.clone(),
                    npc_id: npc_id.to_owned(),
                    npc_name: npc_snapshot.name.clone(),
                    amount: npc_damage,
                    remaining_health,
                    room_id: room_id.clone(),
                },
            });
        }
        if remaining_health == 0 {
            let recover_at_tick = tick.saturating_add(3);
            player_state.recovery_at_tick = Some(recover_at_tick);
            player_state.statuses.clear();
            if let Some(npc) = self.npcs.get_mut(npc_id) {
                npc.engaged_character = None;
            }
            self.loot_encounters.remove(npc_id);
            events.push(DomainEvent::Gameplay {
                event: GameplayEvent::CharacterDefeated {
                    character_id,
                    character_name,
                    npc_id: npc_id.to_owned(),
                    npc_name: npc_snapshot.name,
                    recover_at_tick,
                    room_id,
                },
            });
        }
        Ok(())
    }
}

/// True when `resolve_character_tick` would leave this character unchanged
/// and emit nothing. Must stay in lockstep with that function.
fn character_tick_is_noop(state: &CharacterGameplaySnapshot, tick: u64) -> bool {
    !state
        .scheduled_script_flags
        .iter()
        .any(|scheduled| scheduled.due_tick <= tick)
        && !state
            .resource_regeneration
            .iter()
            .any(|(resource_id, amount)| {
                state.resources.get(resource_id).is_some_and(|pool| {
                    pool.current.saturating_add(*amount).min(pool.max) != pool.current
                })
            })
        && !state
            .ability_cooldowns
            .values()
            .any(|ready_tick| *ready_tick <= tick)
        && state
            .recovery_at_tick
            .is_none_or(|recovery| recovery > tick)
        && state.effects.is_empty()
        && state.statuses.is_empty()
        && !(state.vitals.health == 0 && state.recovery_at_tick.is_none())
}

/// True when `resolve_npc_statuses` would leave this NPC unchanged.
fn npc_tick_is_noop(npc: &Npc, tick: u64) -> bool {
    !npc.alive()
        || (!npc
            .resource_regeneration
            .iter()
            .any(|(resource_id, amount)| {
                npc.resources.get(resource_id).is_some_and(|pool| {
                    pool.current.saturating_add(*amount).min(pool.max) != pool.current
                })
            })
            && !npc
                .ability_cooldowns
                .values()
                .any(|ready_tick| *ready_tick <= tick)
            && npc.statuses.is_empty())
}
