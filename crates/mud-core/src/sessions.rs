//! Session connection lifecycle and staff world actions.

use super::*;

impl World {
    pub fn connect(
        &mut self,
        session: SessionId,
        snapshot: CharacterSnapshot,
    ) -> Result<SceneView, WorldError> {
        if self.sessions.iter().any(|(connected_session, entity)| {
            *connected_session != session
                && matches!(
                    self.entities.get(*entity),
                    Some(Entity::Character(character)) if character.id == snapshot.id
                )
        }) {
            return Err(WorldError::CharacterAlreadyConnected);
        }
        let location = self
            .stable_rooms
            .get(&snapshot.room_id)
            .copied()
            .ok_or(WorldError::InvalidEntity)?;
        room_checked(&self.entities, location)?;
        let mut salt = [0_u8; 32];
        OsRng
            .try_fill_bytes(&mut salt)
            .map_err(|_| WorldError::InvalidEntity)?;
        self.character_gameplay.entry(snapshot.id).or_default();
        let character = self.entities.insert(Entity::Character(Character {
            id: snapshot.id,
            account_id: snapshot.account_id,
            name: snapshot.name,
            location,
        }));
        if let Some(previous) = self.sessions.insert(session, character) {
            self.entities.remove(previous);
        }
        self.observer_views.insert(
            session,
            ObserverView {
                salt,
                revision: 1,
                discovered_rooms: BTreeSet::from([snapshot.room_id]),
                role: AccountRole::Player,
            },
        );
        self.scene_for(session)
    }

    /// Assigns authorization metadata before any view leaves the server.
    /// Administrative observers bypass concealment but still receive opaque
    /// entity identifiers.
    pub fn set_observer_role(
        &mut self,
        session: SessionId,
        role: AccountRole,
    ) -> Result<(), WorldError> {
        let observer = self
            .observer_views
            .get_mut(&session)
            .ok_or(WorldError::UnknownSession)?;
        if observer.role != role {
            observer.role = role;
            observer.revision = observer.revision.wrapping_add(1).max(1);
        }
        Ok(())
    }

    pub fn disconnect(&mut self, session: SessionId) {
        self.observer_views.remove(&session);
        if let Some(character) = self.sessions.remove(&session) {
            let disconnected_character_id =
                self.entities
                    .get(character)
                    .and_then(|entity| match entity {
                        Entity::Character(character) => Some(character.id),
                        _ => None,
                    });
            if let Some(character_id) = disconnected_character_id {
                self.last_trade_request_ticks.remove(&character_id);
                // Combat is connection-scoped in v1: an NPC immediately stops
                // attacking a departed character. Remove that character's
                // pending effects as well, so damage-over-time cannot award
                // progress only in memory after the durable character is no
                // longer part of the tick commit.
                let mut abandoned_encounters = Vec::new();
                for npc in self.npcs.values_mut() {
                    if npc.engaged_character == Some(character_id) {
                        npc.engaged_character = None;
                        abandoned_encounters.push(npc.stable_id.clone());
                    }
                    npc.statuses
                        .retain(|status| status.source_character_id != Some(character_id));
                }
                for npc_id in abandoned_encounters {
                    self.loot_encounters.remove(&npc_id);
                }
            }
            self.entities.remove(character);
        }
    }

    pub fn character_snapshot(&self, session: SessionId) -> Result<CharacterSnapshot, WorldError> {
        let character = self.character(session)?;
        let current_room = room_checked(&self.entities, character.location)?;
        Ok(CharacterSnapshot {
            id: character.id,
            account_id: character.account_id,
            name: character.name.clone(),
            room_id: current_room.stable_id.clone(),
            item_locations: BTreeMap::new(),
        })
    }

    /// Builds a bounded, staff-only diagnostic view. Callers must authorize
    /// `staff.inspect` and audit the access before exposing this text.
    pub fn staff_inspection_as_plain_text(&self, session: SessionId) -> Result<String, WorldError> {
        let character = self.character(session)?;
        let room = room_checked(&self.entities, character.location)?;
        let gameplay = self
            .character_gameplay
            .get(&character.id)
            .ok_or(WorldError::InvalidEntity)?;
        let carried = self
            .item_instances
            .ids_at(&SharedItemLocation::Carried(character.id))
            .count();
        Ok(format!(
            concat!(
                "Character: {}\r\n",
                "Room: {}\r\n",
                "Progression: level {}, XP {}, class {}\r\n",
                "Vitals: {}/{} health; {} resources; {} statuses\r\n",
                "Inventory: {} carried; {} equipped\r\n"
            ),
            character.name,
            room.title,
            gameplay.level,
            gameplay.experience,
            gameplay.active_class_id,
            gameplay.vitals.health,
            gameplay.vitals.max_health,
            gameplay.resources.len(),
            gameplay.statuses.len(),
            carried,
            gameplay.equipment.len(),
        ))
    }

    /// Returns stable operational location data for an already authorized
    /// staff diagnostic. Stable room identity never enters ordinary views.
    pub fn staff_location_as_plain_text(&self, session: SessionId) -> Result<String, WorldError> {
        let character = self.character(session)?;
        let room = room_checked(&self.entities, character.location)?;
        Ok(format!("{} is in {}.\r\n", character.name, room.title))
    }

    /// Relocates the acting staff character to a connected target.
    pub fn staff_goto(
        &mut self,
        actor: SessionId,
        target: SessionId,
    ) -> Result<ActionOutcome, WorldError> {
        self.staff_relocate(actor, target, actor, "You travel to the target.\r\n")
    }

    /// Brings a connected target to the acting staff character.
    pub fn staff_transfer(
        &mut self,
        actor: SessionId,
        target: SessionId,
    ) -> Result<ActionOutcome, WorldError> {
        self.staff_relocate(
            target,
            actor,
            actor,
            "The target is transferred to you.\r\n",
        )
    }

    pub(crate) fn staff_relocate(
        &mut self,
        moving: SessionId,
        destination: SessionId,
        response_session: SessionId,
        output: &str,
    ) -> Result<ActionOutcome, WorldError> {
        let moving_entity = *self
            .sessions
            .get(&moving)
            .ok_or(WorldError::UnknownSession)?;
        let destination_entity = *self
            .sessions
            .get(&destination)
            .ok_or(WorldError::UnknownSession)?;
        let destination_location = self
            .entities
            .get(destination_entity)
            .and_then(|entity| match entity {
                Entity::Character(character) => Some(character.location),
                _ => None,
            })
            .ok_or(WorldError::InvalidEntity)?;
        let to_room_id = room_checked(&self.entities, destination_location)?
            .stable_id
            .clone();
        let (actor_id, actor_name, from_room_id) = self
            .entities
            .get(moving_entity)
            .and_then(|entity| match entity {
                Entity::Character(character) => {
                    Some((character.id, character.name.clone(), character.location))
                }
                _ => None,
            })
            .map(|(id, name, location)| {
                room_checked(&self.entities, location)
                    .map(|room| (id, name, room.stable_id.clone()))
            })
            .transpose()?
            .ok_or(WorldError::InvalidEntity)?;
        if from_room_id == to_room_id {
            return Err(WorldError::GameplayUnavailable);
        }
        match self.entities.get_mut(moving_entity) {
            Some(Entity::Character(character)) => character.location = destination_location,
            _ => return Err(WorldError::InvalidEntity),
        }
        self.revision = self.revision.wrapping_add(1).max(1);
        let event = DomainEvent::CharacterRelocated {
            actor_id,
            actor_name,
            from_room_id,
            to_room_id: to_room_id.clone(),
        };
        let mut events = vec![event];
        let mut released_npcs = Vec::new();
        // Copy-on-write: borrow NPCs mutably only when one is engaged.
        if self
            .npcs
            .values()
            .any(|npc| npc.engaged_character == Some(actor_id))
        {
            for npc in self.npcs.values_mut() {
                if npc.engaged_character == Some(actor_id) {
                    npc.engaged_character = None;
                    released_npcs.push((
                        npc.stable_id.clone(),
                        npc.name.clone(),
                        npc.spawn_room_id.clone(),
                    ));
                }
            }
        }
        for (npc_id, npc_name, room_id) in released_npcs {
            self.loot_encounters.remove(&npc_id);
            events.push(DomainEvent::Gameplay {
                event: GameplayEvent::LootEncounterCancelled {
                    npc_id,
                    npc_name,
                    room_id,
                },
            });
        }
        self.bump_view_revisions_for_events(&events)?;
        self.observer_views
            .get_mut(&moving)
            .ok_or(WorldError::InvalidEntity)?
            .discovered_rooms
            .insert(to_room_id);
        Ok(ActionOutcome {
            scene: self.scene_for(response_session)?,
            container_contents: None,
            trade_contents: None,
            reward_odds: None,
            output: Some(output.to_owned()),
            changed_character: Some(self.character_snapshot(moving)?),
            domain_events: events,
        })
    }

    /// Fully restores a connected character's bounded gameplay resources.
    /// The caller is responsible for capability/hierarchy checks and durable
    /// audit in the same transaction as the resulting snapshots.
    pub fn staff_restore(
        &mut self,
        actor: SessionId,
        target: SessionId,
    ) -> Result<ActionOutcome, WorldError> {
        let target_character = self.character(target)?;
        let character_id = target_character.id;
        let character_name = target_character.name.clone();
        let room_id = room_checked(&self.entities, target_character.location)?
            .stable_id
            .clone();
        let gameplay = self
            .character_gameplay
            .get_mut(&character_id)
            .ok_or(WorldError::InvalidEntity)?;
        gameplay.vitals.health = gameplay.vitals.max_health;
        for resource in gameplay.resources.values_mut() {
            resource.current = resource.max;
        }
        gameplay.statuses.clear();
        gameplay.effects.clear();
        gameplay.ability_cooldowns.clear();
        gameplay.ability_global_ready_tick = self.logical_tick;
        gameplay.recovery_at_tick = None;
        let mut released_npcs = Vec::new();
        // Copy-on-write: borrow NPCs mutably only when one is engaged.
        if self
            .npcs
            .values()
            .any(|npc| npc.engaged_character == Some(character_id))
        {
            for npc in self.npcs.values_mut() {
                if npc.engaged_character == Some(character_id) {
                    npc.engaged_character = None;
                    released_npcs.push(npc.stable_id.clone());
                }
            }
        }
        for npc_id in released_npcs {
            self.loot_encounters.remove(&npc_id);
        }
        self.revision = self.revision.wrapping_add(1).max(1);
        let event = DomainEvent::Gameplay {
            event: GameplayEvent::CharacterRestored {
                character_id,
                character_name,
                health: gameplay.vitals.health,
                room_id,
            },
        };
        self.bump_view_revisions(&event)?;
        Ok(ActionOutcome {
            scene: self.scene_for(actor)?,
            container_contents: None,
            trade_contents: None,
            reward_odds: None,
            output: Some("The target has been fully restored.\r\n".to_owned()),
            changed_character: Some(self.character_snapshot(target)?),
            domain_events: vec![event],
        })
    }

    pub fn restore_character(
        &mut self,
        session: SessionId,
        snapshot: &CharacterSnapshot,
        revision: u64,
    ) -> Result<(), WorldError> {
        let location = self
            .stable_rooms
            .get(&snapshot.room_id)
            .copied()
            .ok_or(WorldError::InvalidEntity)?;
        room_checked(&self.entities, location)?;
        let character_id = *self
            .sessions
            .get(&session)
            .ok_or(WorldError::UnknownSession)?;
        match self.entities.get_mut(character_id) {
            Some(Entity::Character(character))
                if character.id == snapshot.id && character.account_id == snapshot.account_id =>
            {
                character.name.clone_from(&snapshot.name);
                character.location = location;
                self.restore_last_view_bump();
                self.revision = revision;
                Ok(())
            }
            _ => Err(WorldError::InvalidEntity),
        }
    }
}
