//! Combat, abilities, item use, equipment and character progression.

use super::*;

impl World {
    pub(crate) fn advance_character_progression(
        &mut self,
        character_id: CharacterId,
        character_name: &str,
        room_id: &str,
        events: &mut Vec<DomainEvent>,
    ) -> Result<(), WorldError> {
        let mut state = self
            .character_gameplay
            .get(&character_id)
            .cloned()
            .ok_or(WorldError::InvalidEntity)?;
        loop {
            let Some(threshold) = self
                .progression
                .next_level(state.level, state.experience)
                .cloned()
            else {
                break;
            };
            state = self
                .progression
                .apply_level(&state, &threshold, &self.abilities)?;
            events.push(DomainEvent::Gameplay {
                event: GameplayEvent::CharacterLeveled {
                    character_id,
                    character_name: character_name.to_owned(),
                    level: state.level,
                    room_id: room_id.to_owned(),
                },
            });
        }
        self.character_gameplay.insert(character_id, state);
        Ok(())
    }
}

impl World {
    pub(crate) fn start_combat(
        &mut self,
        session: SessionId,
        query: &str,
    ) -> Result<ActionOutcome, WorldError> {
        let character = self.character(session)?;
        let character_id = character.id;
        let character_name = character.name.clone();
        let gameplay = self
            .character_gameplay
            .get(&character_id)
            .ok_or(WorldError::InvalidEntity)?;
        if gameplay.vitals.health <= 0
            || gameplay.recovery_at_tick.is_some()
            || gameplay
                .statuses
                .iter()
                .any(|status| status.kind == ActiveStatusKind::Stun)
        {
            return Err(WorldError::CombatUnavailable);
        }
        let room_id = room_checked(&self.entities, character.location)?
            .stable_id
            .clone();
        let npc_id = self.resolve_visible_npc(session, query)?;
        {
            let npc = self.npcs.get(&npc_id).ok_or(WorldError::NpcNotFound)?;
            if !npc.alive()
                || npc.spawn_room_id != room_id
                || npc
                    .engaged_character
                    .is_some_and(|engaged| engaged != character_id)
            {
                return Err(WorldError::CombatUnavailable);
            }
        }
        self.freeze_loot_encounter(&npc_id, character_id, &room_id)?;
        let npc = self.npcs.get_mut(&npc_id).ok_or(WorldError::NpcNotFound)?;
        npc.engaged_character = Some(character_id);
        let npc_name = npc.name.clone();
        let event = DomainEvent::Gameplay {
            event: GameplayEvent::CombatStarted {
                character_id,
                character_name,
                npc_id,
                npc_name: npc_name.clone(),
                room_id,
            },
        };
        self.finish_gameplay_action(
            session,
            format!("You engage the {npc_name}.\r\n"),
            vec![event],
        )
    }

    pub(crate) fn use_ability(
        &mut self,
        session: SessionId,
        ability_query: &str,
        target_query: Option<&str>,
    ) -> Result<ActionOutcome, WorldError> {
        let checkpoint = self.checkpoint();
        match self.use_ability_inner(session, ability_query, target_query) {
            Ok(outcome) => Ok(outcome),
            Err(error) => {
                self.restore_checkpoint(checkpoint);
                Err(error)
            }
        }
    }

    pub(crate) fn use_ability_inner(
        &mut self,
        session: SessionId,
        ability_query: &str,
        target_query: Option<&str>,
    ) -> Result<ActionOutcome, WorldError> {
        if ability_query.is_empty()
            || ability_query.len() > 128
            || ability_query.chars().any(char::is_control)
        {
            return Err(WorldError::AbilityNotKnown);
        }
        let character = self.character(session)?;
        let character_id = character.id;
        let character_name = character.name.clone();
        let room_id = room_checked(&self.entities, character.location)?
            .stable_id
            .clone();
        let state = self
            .character_gameplay
            .get(&character_id)
            .cloned()
            .ok_or(WorldError::InvalidEntity)?;
        if state.vitals.health <= 0
            || state.recovery_at_tick.is_some()
            || state.ability_global_ready_tick > self.logical_tick
            || state
                .statuses
                .iter()
                .any(|status| status.kind == ActiveStatusKind::Stun)
        {
            return Err(WorldError::AbilityUnavailable);
        }
        let known = state
            .ability_ids
            .iter()
            .filter_map(|id| self.abilities.get(id))
            .filter(|ability| {
                ability.id.eq_ignore_ascii_case(ability_query)
                    || ability.name.eq_ignore_ascii_case(ability_query)
            })
            .cloned()
            .collect::<Vec<_>>();
        let [ability] = known.as_slice() else {
            return Err(WorldError::AbilityNotKnown);
        };
        let ability = ability.clone();
        // Communication abilities have their own server-coordinated action
        // path because social policy, private receiver receptivity and
        // post-commit delivery are not ordinary gameplay effects. Accepting
        // one here would spend its resources and emit AbilityUsed without
        // applying those boundaries.
        if ability.communication.is_some() {
            return Err(WorldError::AbilityUnavailable);
        }
        if state
            .ability_cooldowns
            .get(&ability.id)
            .is_some_and(|ready| *ready > self.logical_tick)
        {
            return Err(WorldError::AbilityUnavailable);
        }
        if ability.resource_costs.iter().any(|(id, cost)| {
            state
                .resources
                .get(id)
                .is_none_or(|pool| pool.current < *cost)
        }) {
            return Err(WorldError::InsufficientResource);
        }

        let npc_target = match ability.targeting {
            AbilityTargeting::SelfOnly => {
                if target_query.is_some() {
                    return Err(WorldError::InvalidAbilityTarget);
                }
                None
            }
            AbilityTargeting::SingleEnemy => {
                let target = target_query.ok_or(WorldError::InvalidAbilityTarget)?;
                let npc_id = self.resolve_visible_npc(session, target)?;
                let npc = self.npcs.get(&npc_id).ok_or(WorldError::NpcNotFound)?;
                if !npc.alive()
                    || npc.spawn_room_id != room_id
                    || npc
                        .engaged_character
                        .is_some_and(|engaged| engaged != character_id)
                {
                    return Err(WorldError::InvalidAbilityTarget);
                }
                Some(npc_id)
            }
            AbilityTargeting::SingleAlly => {
                if target_query.is_some_and(|target| {
                    !target.eq_ignore_ascii_case("self")
                        && !target.eq_ignore_ascii_case(&character_name)
                }) {
                    return Err(WorldError::InvalidAbilityTarget);
                }
                None
            }
        };

        let caster_statuses = ability
            .status_effects
            .iter()
            .filter(|status| status_target(status) == AbilityEffectTarget::Caster)
            .count()
            + if npc_target.is_none() {
                ability
                    .status_effects
                    .iter()
                    .filter(|status| status_target(status) == AbilityEffectTarget::Target)
                    .count()
            } else {
                0
            };
        if state.statuses.len() + caster_statuses > MAX_ACTIVE_STATUSES {
            return Err(WorldError::AbilityUnavailable);
        }
        if let Some(npc_id) = &npc_target {
            let target_statuses = ability
                .status_effects
                .iter()
                .filter(|status| status_target(status) == AbilityEffectTarget::Target)
                .count();
            if self.npcs[npc_id].statuses.len() + target_statuses > MAX_ACTIVE_STATUSES {
                return Err(WorldError::AbilityUnavailable);
            }
            self.freeze_loot_encounter(npc_id, character_id, &room_id)?;
        }

        let npc_name = npc_target
            .as_ref()
            .and_then(|id| self.npcs.get(id).map(|npc| npc.name.clone()));
        let mut events = vec![DomainEvent::Gameplay {
            event: GameplayEvent::AbilityUsed {
                character_id,
                character_name: character_name.clone(),
                ability_id: ability.id.clone(),
                ability_name: ability.name.clone(),
                npc_id: npc_target.clone(),
                npc_name,
                room_id: room_id.clone(),
            },
        }];

        {
            let state = self
                .character_gameplay
                .get_mut(&character_id)
                .ok_or(WorldError::InvalidEntity)?;
            for (resource_id, cost) in &ability.resource_costs {
                let pool = state
                    .resources
                    .get_mut(resource_id)
                    .ok_or(WorldError::InsufficientResource)?;
                pool.current -= *cost;
            }
            if ability.cooldown_ticks > 0 {
                state.ability_cooldowns.insert(
                    ability.id.clone(),
                    self.logical_tick
                        .saturating_add(u64::from(ability.cooldown_ticks)),
                );
            }
            state.ability_global_ready_tick = self.logical_tick.saturating_add(1);
            if npc_target.is_some() {
                state.last_offensive_ability_tick = Some(self.logical_tick);
            }
        }

        if let Some(npc_id) = &npc_target {
            self.npcs
                .get_mut(npc_id)
                .ok_or(WorldError::NpcNotFound)?
                .engaged_character = Some(character_id);
        }

        if let Some(damage) = &ability.damage {
            let amount = ability_scaled_amount(
                damage.base_amount,
                damage.scaling_attribute.as_deref(),
                damage.scaling_percent,
                &state,
            );
            match (damage.target, npc_target.as_deref()) {
                (AbilityEffectTarget::Target, Some(npc_id)) => {
                    let resistance = self.npcs[npc_id]
                        .resistances
                        .get(&damage.damage_type)
                        .copied()
                        .unwrap_or(0);
                    let amount = resisted_amount(amount, resistance);
                    let npc = self.npcs.get_mut(npc_id).ok_or(WorldError::NpcNotFound)?;
                    npc.health = npc.health.saturating_sub(amount).max(0);
                    let remaining_health = npc.health;
                    let npc_name = npc.name.clone();
                    events.push(DomainEvent::Gameplay {
                        event: GameplayEvent::NpcDamaged {
                            character_id,
                            character_name: character_name.clone(),
                            npc_id: npc_id.to_owned(),
                            npc_name: npc_name.clone(),
                            amount,
                            remaining_health,
                            room_id: room_id.clone(),
                        },
                    });
                    if remaining_health == 0 {
                        self.resolve_npc_defeat(
                            self.logical_tick,
                            npc_id,
                            character_id,
                            &character_name,
                            &room_id,
                            &mut events,
                        )?;
                    }
                }
                (AbilityEffectTarget::Caster, _) | (AbilityEffectTarget::Target, None) => {
                    let resistance = state
                        .resistances
                        .get(&damage.damage_type)
                        .copied()
                        .unwrap_or(0);
                    let amount = resisted_amount(amount, resistance);
                    let state = self
                        .character_gameplay
                        .get_mut(&character_id)
                        .ok_or(WorldError::InvalidEntity)?;
                    state.vitals.health = state.vitals.health.saturating_sub(amount).max(0);
                }
            }
        }

        if let Some(heal) = &ability.heal {
            // Ally targeting is intentionally self-only in the first version;
            // PvP/group target authority is a later protocol milestone.
            if heal.target == AbilityEffectTarget::Target && npc_target.is_some() {
                return Err(WorldError::InvalidAbilityTarget);
            }
            let amount = ability_scaled_amount(
                heal.base_amount,
                heal.scaling_attribute.as_deref(),
                heal.scaling_percent,
                &state,
            );
            let state = self
                .character_gameplay
                .get_mut(&character_id)
                .ok_or(WorldError::InvalidEntity)?;
            let previous = state.vitals.health;
            state.vitals.health = state
                .vitals
                .health
                .saturating_add(amount.max(0))
                .min(state.vitals.max_health);
            let healed = state.vitals.health.saturating_sub(previous);
            if healed > 0 {
                events.push(DomainEvent::Gameplay {
                    event: GameplayEvent::CharacterHealed {
                        character_id,
                        character_name: character_name.clone(),
                        amount: healed,
                        health: state.vitals.health,
                        room_id: room_id.clone(),
                    },
                });
            }
        }

        for status in &ability.status_effects {
            let active = active_status(&ability.id, Some(character_id), None, status);
            if status_target(status) == AbilityEffectTarget::Target
                && let Some(npc_id) = &npc_target
            {
                let npc = self.npcs.get_mut(npc_id).ok_or(WorldError::NpcNotFound)?;
                if npc.alive() {
                    npc.statuses.push(active);
                }
            } else {
                self.character_gameplay
                    .get_mut(&character_id)
                    .ok_or(WorldError::InvalidEntity)?
                    .statuses
                    .push(active);
            }
        }

        let collapsed = {
            let state = self
                .character_gameplay
                .get_mut(&character_id)
                .ok_or(WorldError::InvalidEntity)?;
            if state.vitals.health == 0 && state.recovery_at_tick.is_none() {
                state.recovery_at_tick = Some(self.logical_tick.saturating_add(3));
                state.statuses.clear();
                true
            } else {
                false
            }
        };
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
            events.push(DomainEvent::Gameplay {
                event: GameplayEvent::CharacterCollapsed {
                    character_id,
                    character_name: character_name.clone(),
                    recover_at_tick: self.logical_tick.saturating_add(3),
                    room_id: room_id.clone(),
                },
            });
        }

        self.finish_gameplay_action(session, format!("You use {}.\r\n", ability.name), events)
    }

    pub(crate) fn use_item(
        &mut self,
        session: SessionId,
        query: &str,
    ) -> Result<ActionOutcome, WorldError> {
        let item_id = self.resolve_accessible_item(session, query, ItemScope::Carried)?;
        self.ensure_items_unreserved(std::iter::once(&item_id))?;
        let definition = self
            .item_gameplay_for(&item_id)
            .cloned()
            .map_err(|_| WorldError::ItemNotUsable)?;
        let heal_amount = self.item_effective_heal_amount(&item_id)?;
        if heal_amount == 0 && definition.effect.is_none() {
            return Err(WorldError::ItemNotUsable);
        }
        let character = self.character(session)?;
        let character_id = character.id;
        let character_name = character.name.clone();
        let room_id = room_checked(&self.entities, character.location)?
            .stable_id
            .clone();
        let item_name = self.item_effective_name(&item_id)?;
        let state = self
            .character_gameplay
            .get_mut(&character_id)
            .ok_or(WorldError::InvalidEntity)?;
        if definition.effect.is_some() && state.effects.len() >= MAX_ACTIVE_EFFECTS {
            return Err(WorldError::GameplayUnavailable);
        }
        let mut events = vec![DomainEvent::Gameplay {
            event: GameplayEvent::ItemUsed {
                character_id,
                character_name: character_name.clone(),
                item_id: item_id.clone(),
                item_name: item_name.clone(),
                room_id: room_id.clone(),
            },
        }];
        let previous_health = state.vitals.health;
        state.vitals.health = state
            .vitals
            .health
            .saturating_add(heal_amount.max(0))
            .min(state.vitals.max_health);
        let healed = state.vitals.health.saturating_sub(previous_health);
        if healed > 0 {
            events.push(DomainEvent::Gameplay {
                event: GameplayEvent::CharacterHealed {
                    character_id,
                    character_name: character_name.clone(),
                    amount: healed,
                    health: state.vitals.health,
                    room_id: room_id.clone(),
                },
            });
        }
        if let Some(effect) = definition.effect {
            state.effects.push(effect.clone());
            events.push(DomainEvent::Gameplay {
                event: GameplayEvent::EffectApplied {
                    character_id,
                    character_name,
                    effect,
                    room_id,
                },
            });
        }
        if definition.consumable {
            let mut prospective = self.item_instances.clone();
            prospective
                .get_mut(&item_id)
                .ok_or(WorldError::InvalidEntity)?
                .location = SharedItemLocation::Consumed;
            self.apply_item_ownership_transitions(
                &mut prospective,
                std::slice::from_ref(&item_id),
            )?;
            validate_item_graph(&prospective, &self.item_prototypes)?;
            self.item_instances = prospective;
        }
        self.finish_gameplay_action(session, format!("You use the {item_name}.\r\n"), events)
    }

    pub(crate) fn equip_item(
        &mut self,
        session: SessionId,
        query: &str,
    ) -> Result<ActionOutcome, WorldError> {
        let item_id = self.resolve_accessible_item(session, query, ItemScope::Carried)?;
        self.ensure_items_unreserved(std::iter::once(&item_id))?;
        let definition = self.item_gameplay_for(&item_id)?;
        let slot = definition
            .equipment_slot
            .ok_or(WorldError::ItemNotEquippable)?;
        let character = self.character(session)?;
        let character_id = character.id;
        let character_name = character.name.clone();
        let room_id = room_checked(&self.entities, character.location)?
            .stable_id
            .clone();
        let item_name = self.item_effective_name(&item_id)?;
        let previous_item_id = self
            .character_gameplay
            .get(&character_id)
            .ok_or(WorldError::InvalidEntity)?
            .equipment
            .get(&slot)
            .cloned();
        let mut prospective = self.item_instances.clone();
        prospective
            .get_mut(&item_id)
            .ok_or(WorldError::InvalidEntity)?
            .location = SharedItemLocation::Equipped(character_id, slot);
        if let Some(previous_item_id) = &previous_item_id {
            prospective
                .get_mut(previous_item_id)
                .ok_or(WorldError::InvalidEntity)?
                .location = SharedItemLocation::Carried(character_id);
        }
        self.validate_prospective_item_locations(&prospective, [character_id])?;
        let mut events = Vec::new();
        if let Some(previous_item_id) = &previous_item_id {
            let previous_name = self.item_effective_name(previous_item_id)?;
            events.push(DomainEvent::Gameplay {
                event: GameplayEvent::ItemUnequipped {
                    character_id,
                    character_name: character_name.clone(),
                    item_id: previous_item_id.clone(),
                    item_name: previous_name,
                    slot,
                    room_id: room_id.clone(),
                },
            });
        }
        self.character_gameplay
            .get_mut(&character_id)
            .ok_or(WorldError::InvalidEntity)?
            .equipment
            .insert(slot, item_id.clone());
        self.item_instances = prospective;
        events.push(DomainEvent::Gameplay {
            event: GameplayEvent::ItemEquipped {
                character_id,
                character_name,
                item_id,
                item_name: item_name.clone(),
                slot,
                room_id,
            },
        });
        self.finish_gameplay_action(session, format!("You equip the {item_name}.\r\n"), events)
    }

    pub(crate) fn unequip_item(
        &mut self,
        session: SessionId,
        query: &str,
    ) -> Result<ActionOutcome, WorldError> {
        let slot = EquipmentSlot::parse(&query.trim().to_ascii_lowercase())
            .ok_or(WorldError::EquipmentSlotEmpty)?;
        let character = self.character(session)?;
        let character_id = character.id;
        let character_name = character.name.clone();
        let room_id = room_checked(&self.entities, character.location)?
            .stable_id
            .clone();
        let item_id = self
            .character_gameplay
            .get(&character_id)
            .ok_or(WorldError::InvalidEntity)?
            .equipment
            .get(&slot)
            .cloned()
            .ok_or(WorldError::EquipmentSlotEmpty)?;
        let item_name = self.item_effective_name(&item_id)?;
        let mut prospective = self.item_instances.clone();
        prospective
            .get_mut(&item_id)
            .ok_or(WorldError::InvalidEntity)?
            .location = SharedItemLocation::Carried(character_id);
        self.validate_prospective_item_locations(&prospective, [character_id])?;
        self.character_gameplay
            .get_mut(&character_id)
            .ok_or(WorldError::InvalidEntity)?
            .equipment
            .remove(&slot);
        self.item_instances = prospective;
        let event = DomainEvent::Gameplay {
            event: GameplayEvent::ItemUnequipped {
                character_id,
                character_name,
                item_id,
                item_name: item_name.clone(),
                slot,
                room_id,
            },
        };
        self.finish_gameplay_action(
            session,
            format!("You unequip the {item_name}.\r\n"),
            vec![event],
        )
    }

    pub(crate) fn finish_gameplay_action(
        &mut self,
        session: SessionId,
        output: String,
        events: Vec<DomainEvent>,
    ) -> Result<ActionOutcome, WorldError> {
        self.revision = self.revision.wrapping_add(1).max(1);
        if let Some(event) = events.first() {
            self.bump_view_revisions(event)?;
        }
        Ok(ActionOutcome {
            scene: self.scene_for(session)?,
            container_contents: None,
            trade_contents: None,
            reward_odds: None,
            output: Some(output),
            changed_character: None,
            domain_events: events,
        })
    }

    pub(crate) fn promote_character(
        &mut self,
        session: SessionId,
        class_id: &str,
    ) -> Result<ActionOutcome, WorldError> {
        let checkpoint = self.checkpoint();
        match self.promote_character_inner(session, class_id) {
            Ok(outcome) => Ok(outcome),
            Err(error) => {
                self.restore_checkpoint(checkpoint);
                Err(error)
            }
        }
    }

    pub(crate) fn promote_character_inner(
        &mut self,
        session: SessionId,
        class_id: &str,
    ) -> Result<ActionOutcome, WorldError> {
        let character = self.character(session)?;
        let character_id = character.id;
        let character_name = character.name.clone();
        let room_id = room_checked(&self.entities, character.location)?
            .stable_id
            .clone();
        let previous = self
            .character_gameplay
            .get(&character_id)
            .cloned()
            .ok_or(WorldError::InvalidEntity)?;
        let from_class_id = previous.active_class_id.clone();
        let next = self.progression.promote(
            &previous,
            class_id,
            &self.world_progression_flags,
            &self.abilities,
        )?;
        let class_name = self.progression.class_view(class_id).label;
        self.character_gameplay.insert(character_id, next);
        let event = DomainEvent::Gameplay {
            event: GameplayEvent::CharacterPromoted {
                character_id,
                character_name,
                from_class_id,
                to_class_id: class_id.to_owned(),
                room_id,
            },
        };
        self.finish_gameplay_action(
            session,
            format!("You advance to the {class_name} class.\r\n"),
            vec![event],
        )
    }

    pub(crate) fn acquire_talent(
        &mut self,
        session: SessionId,
        graph_id: &str,
        node_id: &str,
    ) -> Result<ActionOutcome, WorldError> {
        let checkpoint = self.checkpoint();
        match self.acquire_talent_inner(session, graph_id, node_id) {
            Ok(outcome) => Ok(outcome),
            Err(error) => {
                self.restore_checkpoint(checkpoint);
                Err(error)
            }
        }
    }

    pub(crate) fn acquire_talent_inner(
        &mut self,
        session: SessionId,
        graph_id: &str,
        node_id: &str,
    ) -> Result<ActionOutcome, WorldError> {
        let character = self.character(session)?;
        let character_id = character.id;
        let character_name = character.name.clone();
        let room_id = room_checked(&self.entities, character.location)?
            .stable_id
            .clone();
        let previous = self
            .character_gameplay
            .get(&character_id)
            .cloned()
            .ok_or(WorldError::InvalidEntity)?;
        let next = self.progression.acquire_talent(
            &previous,
            graph_id,
            node_id,
            &self.world_progression_flags,
            &self.abilities,
        )?;
        let rank = next
            .talent_node_ranks
            .get(graph_id)
            .and_then(|nodes| nodes.get(node_id))
            .copied()
            .ok_or(WorldError::InvalidProgressionTransition)?;
        let node_name = self
            .progression
            .talent_views(&next, &self.world_progression_flags)
            .into_iter()
            .find(|graph| graph.id == graph_id)
            .and_then(|graph| graph.nodes.into_iter().find(|node| node.id == node_id))
            .map_or_else(|| humanize_id(node_id), |node| node.label);
        self.character_gameplay.insert(character_id, next);
        let event = DomainEvent::Gameplay {
            event: GameplayEvent::TalentRankAcquired {
                character_id,
                character_name,
                graph_id: graph_id.to_owned(),
                node_id: node_id.to_owned(),
                rank,
                room_id,
            },
        };
        self.finish_gameplay_action(
            session,
            format!("You acquire rank {rank} of {node_name}.\r\n"),
            vec![event],
        )
    }

    pub fn receive_progression_signal(
        &mut self,
        character_id: CharacterId,
        signal_id: &str,
    ) -> Result<bool, WorldError> {
        if !valid_definition_id(signal_id) {
            return Err(WorldError::InvalidProgressionTransition);
        }
        let state = self
            .character_gameplay
            .get_mut(&character_id)
            .ok_or(WorldError::InvalidEntity)?;
        if state.received_signals.contains(signal_id) {
            return Ok(false);
        }
        if state.received_signals.len() >= MAX_RECEIVED_SIGNALS {
            return Err(WorldError::ProgressionCapExceeded);
        }
        state.received_signals.insert(signal_id.to_owned());
        Ok(true)
    }

    pub fn set_world_progression_flag(
        &mut self,
        flag_id: &str,
        enabled: bool,
    ) -> Result<bool, WorldError> {
        if !valid_definition_id(flag_id) {
            return Err(WorldError::InvalidProgressionTransition);
        }
        let changed = if enabled {
            if self.world_progression_flags.len() >= 64
                && !self.world_progression_flags.contains(flag_id)
            {
                return Err(WorldError::ProgressionCapExceeded);
            }
            self.world_progression_flags.insert(flag_id.to_owned())
        } else {
            self.world_progression_flags.remove(flag_id)
        };
        Ok(changed)
    }
}

pub(crate) fn validate_ability_definition(ability: &AbilityDefinition) -> Result<(), WorldError> {
    let scaling_valid = |attribute: &Option<String>, percent: u16| {
        percent <= 1_000
            && attribute.is_some() == (percent > 0)
            && attribute.as_deref().is_none_or(valid_stat_id)
    };
    let status_valid = |status: &AbilityStatusEffectDefinition| match status {
        AbilityStatusEffectDefinition::AttributeModifier {
            attribute,
            magnitude,
            duration_ticks,
            ..
        } => {
            valid_stat_id(attribute)
                && attribute != "max_health"
                && *magnitude != 0
                && magnitude.unsigned_abs() <= MAX_STATUS_MAGNITUDE as u32
                && (1..=MAX_STATUS_TICKS).contains(duration_ticks)
        }
        AbilityStatusEffectDefinition::Regeneration {
            magnitude,
            duration_ticks,
            ..
        }
        | AbilityStatusEffectDefinition::DamageOverTime {
            magnitude,
            duration_ticks,
            ..
        } => {
            (1..=MAX_STATUS_MAGNITUDE as u32).contains(magnitude)
                && (1..=MAX_STATUS_TICKS).contains(duration_ticks)
        }
        AbilityStatusEffectDefinition::Stun { duration_ticks, .. } => {
            (1..=MAX_STATUS_TICKS).contains(duration_ticks)
        }
    };
    let effect_target_valid = |target: AbilityEffectTarget| {
        !(ability.targeting == AbilityTargeting::SelfOnly && target == AbilityEffectTarget::Target)
    };
    let target_semantics_valid = ability.damage.as_ref().is_none_or(|damage| {
        effect_target_valid(damage.target)
            && !(damage.target == AbilityEffectTarget::Target
                && ability.targeting == AbilityTargeting::SingleAlly)
    }) && ability.heal.as_ref().is_none_or(|heal| {
        effect_target_valid(heal.target)
            && !(heal.target == AbilityEffectTarget::Target
                && ability.targeting == AbilityTargeting::SingleEnemy)
    }) && ability
        .status_effects
        .iter()
        .all(|status| effect_target_valid(status_target(status)));
    let has_gameplay_effect =
        ability.damage.is_some() || ability.heal.is_some() || !ability.status_effects.is_empty();
    let communication_valid = match &ability.communication {
        None => has_gameplay_effect,
        Some(communication) => {
            ability.id.len() <= MAX_TELEPATHY_ABILITY_ID_BYTES
                && matches!(
                    ability.category,
                    AbilityCategory::Spell | AbilityCategory::Utility
                )
                && ability.targeting == AbilityTargeting::SingleAlly
                && !ability.resource_costs.is_empty()
                && ability.cooldown_ticks > 0
                && !has_gameplay_effect
                && communication.blocked_room_ids.len() <= MAX_COMMUNICATION_BLOCKED_ROOMS
                && communication
                    .blocked_room_ids
                    .iter()
                    .all(|room_id| valid_definition_id(room_id))
                && matches!(communication.kind, AbilityCommunicationKind::Telepathy)
                && matches!(communication.scope, AbilityCommunicationScope::DirectOnline)
        }
    };
    let valid = valid_definition_id(&ability.id)
        && safe_definition_text(&ability.name, 128)
        && safe_definition_text(&ability.description, 2_048)
        && ability.resource_costs.len() <= MAX_ABILITY_RESOURCE_COSTS
        && ability
            .resource_costs
            .iter()
            .all(|(id, amount)| valid_stat_id(id) && *amount > 0 && *amount <= MAX_RESOURCE_VALUE)
        && ability.cooldown_ticks <= MAX_COOLDOWN_TICKS
        && ability.status_effects.len() <= MAX_ABILITY_EFFECTS
        && target_semantics_valid
        && communication_valid
        && ability.damage.as_ref().is_none_or(|damage| {
            (1..=1_000_000).contains(&damage.base_amount)
                && scaling_valid(&damage.scaling_attribute, damage.scaling_percent)
        })
        && ability.heal.as_ref().is_none_or(|heal| {
            (1..=1_000_000).contains(&heal.base_amount)
                && scaling_valid(&heal.scaling_attribute, heal.scaling_percent)
        })
        && ability.status_effects.iter().all(status_valid);
    valid
        .then_some(())
        .ok_or(WorldError::InvalidAbilityDefinition)
}

pub(crate) fn validate_active_status(
    status: &ActiveStatusEffect,
    abilities: &BTreeMap<String, AbilityDefinition>,
) -> bool {
    validate_active_status_shape(status)
        && abilities
            .get(&status.source_ability_id)
            .is_some_and(|ability| {
                ability
                    .status_effects
                    .iter()
                    .any(|effect| active_status_matches_definition(status, effect))
            })
}

pub(crate) fn validate_active_status_shape(status: &ActiveStatusEffect) -> bool {
    let attributed = status.source_character_id.is_some() ^ status.source_npc_id.is_some();
    let valid_source = valid_definition_id(&status.source_ability_id)
        && status.source_character_id.is_none_or(|id| id.0 > 0)
        && status
            .source_npc_id
            .as_deref()
            .is_none_or(valid_definition_id);
    let valid_shape = match status.kind {
        ActiveStatusKind::AttributeModifier => {
            status.magnitude != 0
                && status.magnitude.unsigned_abs() <= MAX_STATUS_MAGNITUDE as u32
                && status.attribute.as_deref().is_some_and(valid_stat_id)
                && status.damage_type.is_none()
        }
        ActiveStatusKind::Regeneration => {
            (1..=MAX_STATUS_MAGNITUDE).contains(&status.magnitude)
                && status.attribute.is_none()
                && status.damage_type.is_none()
        }
        ActiveStatusKind::DamageOverTime => {
            (1..=MAX_STATUS_MAGNITUDE).contains(&status.magnitude)
                && status.attribute.is_none()
                && status.damage_type.is_some()
        }
        ActiveStatusKind::Stun => {
            status.magnitude == 0 && status.attribute.is_none() && status.damage_type.is_none()
        }
    };
    attributed
        && valid_source
        && valid_shape
        && (1..=MAX_STATUS_TICKS).contains(&status.remaining_ticks)
}

pub(crate) fn active_status_matches_definition(
    status: &ActiveStatusEffect,
    definition: &AbilityStatusEffectDefinition,
) -> bool {
    match definition {
        AbilityStatusEffectDefinition::AttributeModifier {
            attribute,
            magnitude,
            duration_ticks,
            ..
        } => {
            status.kind == ActiveStatusKind::AttributeModifier
                && status.magnitude == *magnitude
                && status.attribute.as_deref() == Some(attribute.as_str())
                && status.damage_type.is_none()
                && status.remaining_ticks <= *duration_ticks
        }
        AbilityStatusEffectDefinition::Regeneration {
            magnitude,
            duration_ticks,
            ..
        } => {
            status.kind == ActiveStatusKind::Regeneration
                && status.magnitude == i32::try_from(*magnitude).unwrap_or(i32::MAX)
                && status.attribute.is_none()
                && status.damage_type.is_none()
                && status.remaining_ticks <= *duration_ticks
        }
        AbilityStatusEffectDefinition::DamageOverTime {
            damage_type,
            magnitude,
            duration_ticks,
            ..
        } => {
            status.kind == ActiveStatusKind::DamageOverTime
                && status.magnitude == i32::try_from(*magnitude).unwrap_or(i32::MAX)
                && status.attribute.is_none()
                && status.damage_type == Some(*damage_type)
                && status.remaining_ticks <= *duration_ticks
        }
        AbilityStatusEffectDefinition::Stun { duration_ticks, .. } => {
            status.kind == ActiveStatusKind::Stun
                && status.magnitude == 0
                && status.attribute.is_none()
                && status.damage_type.is_none()
                && status.remaining_ticks <= *duration_ticks
        }
    }
}

pub(crate) fn status_target(status: &AbilityStatusEffectDefinition) -> AbilityEffectTarget {
    match status {
        AbilityStatusEffectDefinition::AttributeModifier { target, .. }
        | AbilityStatusEffectDefinition::Regeneration { target, .. }
        | AbilityStatusEffectDefinition::DamageOverTime { target, .. }
        | AbilityStatusEffectDefinition::Stun { target, .. } => *target,
    }
}

pub(crate) fn npc_effect_targets_enemy(
    ability: &AbilityDefinition,
    target: AbilityEffectTarget,
) -> bool {
    ability.targeting == AbilityTargeting::SingleEnemy && target == AbilityEffectTarget::Target
}

pub(crate) fn npc_enemy_status_count(ability: &AbilityDefinition) -> usize {
    ability
        .status_effects
        .iter()
        .filter(|status| npc_effect_targets_enemy(ability, status_target(status)))
        .count()
}

pub(crate) fn npc_self_status_count(ability: &AbilityDefinition) -> usize {
    ability.status_effects.len() - npc_enemy_status_count(ability)
}

pub(crate) fn npc_ability_score(ability: &AbilityDefinition, npc: &Npc) -> i64 {
    let mut score = 0_i64;
    if let Some(damage) = &ability.damage
        && npc_effect_targets_enemy(ability, damage.target)
    {
        score =
            score
                .saturating_add(1_000_000)
                .saturating_add(i64::from(ability_scaled_amount_npc(
                    damage.base_amount,
                    damage.scaling_attribute.as_deref(),
                    damage.scaling_percent,
                    npc,
                )));
    }
    if let Some(heal) = &ability.heal
        && !npc_effect_targets_enemy(ability, heal.target)
        && npc.health < npc.max_health
    {
        let amount = ability_scaled_amount_npc(
            heal.base_amount,
            heal.scaling_attribute.as_deref(),
            heal.scaling_percent,
            npc,
        );
        score = score.saturating_add(500_000).saturating_add(i64::from(
            amount.min(npc.max_health.saturating_sub(npc.health)),
        ));
    }
    for status in &ability.status_effects {
        if npc_effect_targets_enemy(ability, status_target(status)) {
            let value = match status {
                AbilityStatusEffectDefinition::AttributeModifier {
                    magnitude,
                    duration_ticks,
                    ..
                } => i64::from(magnitude.unsigned_abs()) * i64::from(*duration_ticks),
                AbilityStatusEffectDefinition::Regeneration {
                    magnitude,
                    duration_ticks,
                    ..
                }
                | AbilityStatusEffectDefinition::DamageOverTime {
                    magnitude,
                    duration_ticks,
                    ..
                } => i64::from(*magnitude) * i64::from(*duration_ticks),
                AbilityStatusEffectDefinition::Stun { duration_ticks, .. } => {
                    100_000 * i64::from(*duration_ticks)
                }
            };
            score = score.saturating_add(250_000).saturating_add(value);
        } else if !npc
            .statuses
            .iter()
            .any(|active| active.source_ability_id == ability.id)
        {
            score = score.saturating_add(100_000);
        }
    }
    score
}

pub(crate) fn active_status(
    source_ability_id: &str,
    source_character_id: Option<CharacterId>,
    source_npc_id: Option<&str>,
    status: &AbilityStatusEffectDefinition,
) -> ActiveStatusEffect {
    let (kind, magnitude, remaining_ticks, attribute, damage_type) = match status {
        AbilityStatusEffectDefinition::AttributeModifier {
            attribute,
            magnitude,
            duration_ticks,
            ..
        } => (
            ActiveStatusKind::AttributeModifier,
            *magnitude,
            *duration_ticks,
            Some(attribute.clone()),
            None,
        ),
        AbilityStatusEffectDefinition::Regeneration {
            magnitude,
            duration_ticks,
            ..
        } => (
            ActiveStatusKind::Regeneration,
            i32::try_from(*magnitude).unwrap_or(i32::MAX),
            *duration_ticks,
            None,
            None,
        ),
        AbilityStatusEffectDefinition::DamageOverTime {
            damage_type,
            magnitude,
            duration_ticks,
            ..
        } => (
            ActiveStatusKind::DamageOverTime,
            i32::try_from(*magnitude).unwrap_or(i32::MAX),
            *duration_ticks,
            None,
            Some(*damage_type),
        ),
        AbilityStatusEffectDefinition::Stun { duration_ticks, .. } => {
            (ActiveStatusKind::Stun, 0, *duration_ticks, None, None)
        }
    };
    ActiveStatusEffect {
        source_ability_id: source_ability_id.to_owned(),
        source_character_id,
        source_npc_id: source_npc_id.map(str::to_owned),
        kind,
        magnitude,
        remaining_ticks,
        attribute,
        damage_type,
    }
}

pub(crate) fn attribute_value(attributes: &Attributes, id: &str) -> i32 {
    match id {
        "strength" => attributes.strength,
        "defence" => attributes.defence,
        _ => attributes.additional.get(id).copied().unwrap_or(0),
    }
}

pub(crate) fn permanent_gameplay_attribute(gameplay: &CharacterGameplaySnapshot, id: &str) -> i32 {
    match id {
        "max_health" => gameplay.vitals.max_health,
        _ => attribute_value(&gameplay.attributes, id),
    }
}

pub(crate) fn ability_scaled_amount(
    base: u32,
    scaling_attribute: Option<&str>,
    scaling_percent: u16,
    gameplay: &CharacterGameplaySnapshot,
) -> i32 {
    let scaling = scaling_attribute
        .map(|id| {
            let value = if id == "max_health" {
                gameplay.vitals.max_health
            } else {
                effective_gameplay_attribute(gameplay, id)
            };
            i64::from(value.max(0))
        })
        .unwrap_or(0)
        .saturating_mul(i64::from(scaling_percent))
        / 100;
    i64::from(base)
        .saturating_add(scaling)
        .clamp(0, i64::from(i32::MAX)) as i32
}

pub(crate) fn ability_scaled_amount_npc(
    base: u32,
    scaling_attribute: Option<&str>,
    scaling_percent: u16,
    npc: &Npc,
) -> i32 {
    let scaling = scaling_attribute
        .map(|id| {
            let value = if id == "max_health" {
                npc.max_health
            } else {
                effective_npc_attribute(npc, id)
            };
            i64::from(value.max(0))
        })
        .unwrap_or(0)
        .saturating_mul(i64::from(scaling_percent))
        / 100;
    i64::from(base)
        .saturating_add(scaling)
        .clamp(0, i64::from(i32::MAX)) as i32
}

pub(crate) fn effective_gameplay_attribute(gameplay: &CharacterGameplaySnapshot, id: &str) -> i32 {
    gameplay
        .statuses
        .iter()
        .filter(|status| {
            status.kind == ActiveStatusKind::AttributeModifier
                && status.attribute.as_deref() == Some(id)
        })
        .fold(
            attribute_value(&gameplay.attributes, id),
            |value, status| value.saturating_add(status.magnitude),
        )
}

pub(crate) fn effective_npc_attribute(npc: &Npc, id: &str) -> i32 {
    let base = match id {
        "strength" => npc.strength,
        "defence" => npc.defence,
        _ => 0,
    };
    npc.statuses
        .iter()
        .filter(|status| {
            status.kind == ActiveStatusKind::AttributeModifier
                && status.attribute.as_deref() == Some(id)
        })
        .fold(base, |value, status| value.saturating_add(status.magnitude))
}

pub(crate) fn resisted_amount(amount: i32, resistance: u16) -> i32 {
    if amount <= 0 {
        return 0;
    }
    let retained = 100_i64.saturating_sub(i64::from(resistance));
    (i64::from(amount).saturating_mul(retained) / 100).clamp(1, i64::from(i32::MAX)) as i32
}
