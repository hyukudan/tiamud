//! Replay of persisted domain events and rollback.

use super::*;

impl World {
    /// Replays one committed event while rebuilding shared world state.
    /// Character movement is intentionally ignored because durable character
    /// snapshots are the authority for locations.
    pub fn replay_persisted_event(&mut self, event: &DomainEvent) -> Result<(), WorldError> {
        match event {
            DomainEvent::Gameplay {
                event:
                    GameplayEvent::LootGenerated { .. }
                    | GameplayEvent::LootRightsChanged { .. }
                    | GameplayEvent::LootGenerationSuppressed { .. }
                    | GameplayEvent::LootDespawned { .. }
                    | GameplayEvent::LootEncounterCancelled { .. }
                    | GameplayEvent::RewardOpened { .. },
            } => Err(WorldError::InvalidDomainTransition),
            DomainEvent::CharacterMoved { .. }
            | DomainEvent::CharacterRelocated { .. }
            | DomainEvent::Gameplay { .. }
            | DomainEvent::Adventure { .. } => Ok(()),
            DomainEvent::ItemTaken {
                actor_id,
                actor_name,
                item_id,
                item_name,
                room_id,
            } => {
                self.validate_persisted_item_event(
                    *actor_id, actor_name, item_id, item_name, room_id,
                )?;
                if self.item_location(item_id) != Some(&SharedItemLocation::Room(room_id.clone())) {
                    return Err(WorldError::InvalidDomainTransition);
                }
                let mut prospective = self.item_instances.clone();
                prospective
                    .get_mut(item_id)
                    .ok_or(WorldError::InvalidDomainTransition)?
                    .location = SharedItemLocation::Carried(*actor_id);
                self.apply_item_ownership_transitions(
                    &mut prospective,
                    std::slice::from_ref(item_id),
                )?;
                if self.character_gameplay.contains_key(actor_id) {
                    self.validate_prospective_item_locations(&prospective, [*actor_id])?;
                } else {
                    validate_item_graph(&prospective, &self.item_prototypes)?;
                }
                self.item_instances = prospective;
                self.revision = self.revision.wrapping_add(1).max(1);
                Ok(())
            }
            DomainEvent::ItemDropped {
                actor_id,
                actor_name,
                item_id,
                item_name,
                room_id,
            } => {
                self.validate_persisted_item_event(
                    *actor_id, actor_name, item_id, item_name, room_id,
                )?;
                if self.item_location(item_id) != Some(&SharedItemLocation::Carried(*actor_id)) {
                    return Err(WorldError::InvalidDomainTransition);
                }
                let mut prospective = self.item_instances.clone();
                prospective
                    .get_mut(item_id)
                    .ok_or(WorldError::InvalidDomainTransition)?
                    .location = SharedItemLocation::Room(room_id.clone());
                self.apply_item_ownership_transitions(
                    &mut prospective,
                    std::slice::from_ref(item_id),
                )?;
                if self.character_gameplay.contains_key(actor_id) {
                    self.validate_prospective_item_locations(&prospective, [*actor_id])?;
                } else {
                    validate_item_graph(&prospective, &self.item_prototypes)?;
                }
                self.item_instances = prospective;
                self.revision = self.revision.wrapping_add(1).max(1);
                Ok(())
            }
            DomainEvent::ItemsMoved {
                actor_id,
                item_ids,
                container_id,
                operation,
                room_id,
                ..
            } => {
                self.validate_items_moved_event(event)?;
                let mut prospective = self.item_instances.clone();
                for item_id in item_ids {
                    let expected = match operation {
                        ContainerMoveOperation::Take => SharedItemLocation::Room(room_id.clone()),
                        ContainerMoveOperation::Drop | ContainerMoveOperation::Put => {
                            SharedItemLocation::Carried(*actor_id)
                        }
                        ContainerMoveOperation::Get => SharedItemLocation::Contained(
                            container_id
                                .clone()
                                .ok_or(WorldError::InvalidDomainTransition)?,
                        ),
                    };
                    let destination = match operation {
                        ContainerMoveOperation::Take | ContainerMoveOperation::Get => {
                            SharedItemLocation::Carried(*actor_id)
                        }
                        ContainerMoveOperation::Drop => SharedItemLocation::Room(room_id.clone()),
                        ContainerMoveOperation::Put => SharedItemLocation::Contained(
                            container_id
                                .clone()
                                .ok_or(WorldError::InvalidDomainTransition)?,
                        ),
                    };
                    let instance = prospective
                        .get_mut(item_id)
                        .ok_or(WorldError::InvalidDomainTransition)?;
                    if instance.location != expected {
                        return Err(WorldError::InvalidDomainTransition);
                    }
                    instance.location = destination;
                }
                self.apply_item_ownership_transitions(&mut prospective, item_ids)?;
                // A cold event replay deliberately supports an offline owner:
                // the durable character aggregate is loaded and checked by
                // the startup ownership barrier. When the aggregate is
                // already present, retain the stronger immediate capacity
                // check as well.
                if self.character_gameplay.contains_key(actor_id) {
                    self.validate_prospective_item_locations(&prospective, [*actor_id])?;
                } else {
                    validate_item_graph(&prospective, &self.item_prototypes)?;
                }
                self.item_instances = prospective;
                self.revision = self.revision.wrapping_add(1).max(1);
                Ok(())
            }
            DomainEvent::ItemsGiven {
                giver_id,
                recipient_id,
                item_ids,
                room_id: _,
                ..
            } => {
                self.validate_items_given_event(event)?;
                let mut prospective = self.item_instances.clone();
                for item_id in item_ids {
                    let instance = prospective
                        .get_mut(item_id)
                        .ok_or(WorldError::InvalidDomainTransition)?;
                    if instance.location != SharedItemLocation::Carried(*giver_id) {
                        return Err(WorldError::InvalidDomainTransition);
                    }
                    instance.location = SharedItemLocation::Carried(*recipient_id);
                }
                self.apply_item_ownership_transitions(&mut prospective, item_ids)?;
                let affected = [*giver_id, *recipient_id]
                    .into_iter()
                    .filter(|id| self.character_gameplay.contains_key(id))
                    .collect::<Vec<_>>();
                if affected.is_empty() {
                    validate_item_graph(&prospective, &self.item_prototypes)?;
                } else {
                    self.validate_prospective_item_locations(&prospective, affected)?;
                }
                self.item_instances = prospective;
                self.revision = self.revision.wrapping_add(1).max(1);
                Ok(())
            }
            DomainEvent::TradeRequested { trade } => {
                if trade.id != self.next_trade_id
                    || self.trades.contains_key(&trade.id)
                    || self.trade_for_character(trade.requester).is_some()
                    || self.trade_for_character(trade.recipient).is_some()
                {
                    return Err(WorldError::InvalidDomainTransition);
                }
                let next = TradeId(
                    trade
                        .id
                        .0
                        .checked_add(1)
                        .ok_or(WorldError::InvalidDomainTransition)?,
                );
                let mut candidate_trades = self.trades.clone();
                candidate_trades.insert(trade.id, trade.clone());
                validate_trade_state(
                    next,
                    &candidate_trades,
                    &self.item_instances,
                    &self.item_prototypes,
                    &self.stable_rooms,
                )?;
                self.next_trade_id = next;
                self.trades.insert(trade.id, trade.clone());
                self.revision = self
                    .revision
                    .checked_add(1)
                    .ok_or(WorldError::InvalidDomainTransition)?;
                Ok(())
            }
            DomainEvent::TradeOpened { previous, current } => {
                if self.trades.get(&previous.id) != Some(previous)
                    || !valid_trade_open_transition(previous, current)
                {
                    return Err(WorldError::InvalidDomainTransition);
                }
                self.replay_trade_state_change(current)
            }
            DomainEvent::TradeOfferSet { previous, current } => {
                if self.trades.get(&previous.id) != Some(previous)
                    || !valid_trade_offer_transition(previous, current)
                {
                    return Err(WorldError::InvalidDomainTransition);
                }
                self.replay_trade_state_change(current)
            }
            DomainEvent::TradeInvalidated { previous, current } => {
                if self.trades.get(&previous.id) != Some(previous)
                    || !valid_trade_invalidation_transition(previous, current)
                {
                    return Err(WorldError::InvalidDomainTransition);
                }
                self.replay_trade_state_change(current)
            }
            DomainEvent::TradeConfirmed {
                participant,
                confirmation,
            } => {
                let trade = self
                    .trade_for_character(*participant)
                    .cloned()
                    .ok_or(WorldError::InvalidDomainTransition)?;
                if trade.phase != TradePhase::Negotiating
                    || !trade_has_assets(&trade)
                    || confirmation.state_revision != trade.state_revision
                    || trade.confirmations.contains_key(participant)
                    || (!trade.confirmations.is_empty())
                    || self
                        .trade_basis_sha256_if_participants_loaded(&trade)?
                        .is_some_and(|basis| basis != confirmation.basis_sha256)
                {
                    return Err(WorldError::InvalidDomainTransition);
                }
                let mut current = trade;
                current
                    .confirmations
                    .insert(*participant, confirmation.clone());
                self.replay_trade_state_change(&current)
            }
            DomainEvent::TradeCancelled { previous_trade, .. } => {
                if self.trades.get(&previous_trade.id) != Some(previous_trade) {
                    return Err(WorldError::InvalidDomainTransition);
                }
                self.trades.remove(&previous_trade.id);
                self.revision = self
                    .revision
                    .checked_add(1)
                    .ok_or(WorldError::InvalidDomainTransition)?;
                Ok(())
            }
            DomainEvent::TradeCompleted { previous_trade } => {
                let current = self
                    .trades
                    .get(&previous_trade.id)
                    .cloned()
                    .ok_or(WorldError::InvalidDomainTransition)?;
                let participants =
                    BTreeSet::from([previous_trade.requester, previous_trade.recipient]);
                let canonical_basis = self.trade_basis_sha256_if_participants_loaded(&current)?;
                let confirmations_valid = previous_trade.confirmations.len() == 2
                    && previous_trade
                        .confirmations
                        .keys()
                        .copied()
                        .collect::<BTreeSet<_>>()
                        == participants
                    && previous_trade.confirmations.values().all(|confirmation| {
                        confirmation.state_revision == previous_trade.state_revision
                            && canonical_basis
                                .is_none_or(|basis| confirmation.basis_sha256 == basis)
                    })
                    && previous_trade
                        .confirmations
                        .values()
                        .map(|confirmation| confirmation.basis_sha256)
                        .collect::<BTreeSet<_>>()
                        .len()
                        == 1;
                let mut expected_current = previous_trade.clone();
                expected_current.confirmations = current.confirmations.clone();
                if current.phase != TradePhase::Negotiating
                    || !trade_has_assets(previous_trade)
                    || current.confirmations.len() != 1
                    || current != expected_current
                    || !confirmations_valid
                    || current
                        .confirmations
                        .iter()
                        .any(|(participant, confirmation)| {
                            previous_trade.confirmations.get(participant) != Some(confirmation)
                        })
                {
                    return Err(WorldError::InvalidDomainTransition);
                }
                let mut completed_candidate = self.trades.clone();
                completed_candidate.insert(previous_trade.id, previous_trade.clone());
                validate_trade_state(
                    self.next_trade_id,
                    &completed_candidate,
                    &self.item_instances,
                    &self.item_prototypes,
                    &self.stable_rooms,
                )?;
                if let (Some(requester), Some(recipient)) = (
                    self.character_gameplay.get(&previous_trade.requester),
                    self.character_gameplay.get(&previous_trade.recipient),
                ) {
                    let requester_offer = previous_trade
                        .offers
                        .get(&previous_trade.requester)
                        .ok_or(WorldError::InvalidDomainTransition)?;
                    let recipient_offer = previous_trade
                        .offers
                        .get(&previous_trade.recipient)
                        .ok_or(WorldError::InvalidDomainTransition)?;
                    requester
                        .coins
                        .checked_sub(requester_offer.coins)
                        .and_then(|coins| coins.checked_add(recipient_offer.coins))
                        .ok_or(WorldError::InvalidDomainTransition)?;
                    recipient
                        .coins
                        .checked_sub(recipient_offer.coins)
                        .and_then(|coins| coins.checked_add(requester_offer.coins))
                        .ok_or(WorldError::InvalidDomainTransition)?;
                }
                let mut prospective = self.item_instances.clone();
                for (owner, destination) in [
                    (previous_trade.requester, previous_trade.recipient),
                    (previous_trade.recipient, previous_trade.requester),
                ] {
                    for root in previous_trade
                        .offers
                        .get(&owner)
                        .into_iter()
                        .flat_map(|offer| &offer.lines)
                        .flat_map(|line| &line.item_ids)
                    {
                        let instance = prospective
                            .get_mut(root)
                            .ok_or(WorldError::InvalidDomainTransition)?;
                        if instance.location != SharedItemLocation::Carried(owner) {
                            return Err(WorldError::InvalidDomainTransition);
                        }
                        instance.location = SharedItemLocation::Carried(destination);
                    }
                }
                let affected = [previous_trade.requester, previous_trade.recipient]
                    .into_iter()
                    .filter(|id| self.character_gameplay.contains_key(id))
                    .collect::<Vec<_>>();
                if affected.is_empty() {
                    validate_item_graph(&prospective, &self.item_prototypes)?;
                } else {
                    self.validate_prospective_item_locations(&prospective, affected)?;
                }
                self.item_instances = prospective;
                self.trades.remove(&previous_trade.id);
                self.revision = self
                    .revision
                    .checked_add(1)
                    .ok_or(WorldError::InvalidDomainTransition)?;
                Ok(())
            }
        }
    }

    pub(crate) fn replay_trade_state_change(
        &mut self,
        current: &TradeSnapshot,
    ) -> Result<(), WorldError> {
        let mut candidate_trades = self.trades.clone();
        candidate_trades.insert(current.id, current.clone());
        validate_trade_state(
            self.next_trade_id,
            &candidate_trades,
            &self.item_instances,
            &self.item_prototypes,
            &self.stable_rooms,
        )?;
        self.trades.insert(current.id, current.clone());
        self.revision = self
            .revision
            .checked_add(1)
            .ok_or(WorldError::InvalidDomainTransition)?;
        Ok(())
    }

    pub(crate) fn trade_basis_sha256_if_participants_loaded(
        &self,
        trade: &TradeSnapshot,
    ) -> Result<Option<[u8; 32]>, WorldError> {
        if trade
            .participants()
            .into_iter()
            .all(|participant| self.character_gameplay.contains_key(&participant))
        {
            self.trade_basis_sha256(trade).map(Some)
        } else {
            Ok(None)
        }
    }

    /// Reverses uncommitted shared-item events. Validation runs against a copy
    /// first, so a malformed batch cannot partially mutate the live world.
    pub fn rollback_domain_events(&mut self, events: &[DomainEvent]) -> Result<(), WorldError> {
        let mut restored = self.item_instances.clone();
        let mut item_event_count = 0_u64;
        let mut affected_characters = BTreeSet::new();
        for event in events.iter().rev() {
            match event {
                DomainEvent::CharacterMoved { .. }
                | DomainEvent::CharacterRelocated { .. }
                | DomainEvent::Gameplay { .. }
                | DomainEvent::Adventure { .. }
                | DomainEvent::TradeRequested { .. }
                | DomainEvent::TradeOpened { .. }
                | DomainEvent::TradeOfferSet { .. }
                | DomainEvent::TradeConfirmed { .. }
                | DomainEvent::TradeInvalidated { .. }
                | DomainEvent::TradeCancelled { .. }
                | DomainEvent::TradeCompleted { .. } => {}
                DomainEvent::ItemTaken {
                    actor_id,
                    actor_name,
                    item_id,
                    item_name,
                    room_id,
                } => {
                    self.validate_persisted_item_event(
                        *actor_id, actor_name, item_id, item_name, room_id,
                    )?;
                    match restored.get_mut(item_id) {
                        Some(item) if item.location == SharedItemLocation::Carried(*actor_id) => {
                            item.location = SharedItemLocation::Room(room_id.clone());
                        }
                        _ => return Err(WorldError::InvalidDomainTransition),
                    }
                    self.reconcile_rollback_bindings(&mut restored, std::slice::from_ref(item_id))?;
                    affected_characters.insert(*actor_id);
                    item_event_count += 1;
                }
                DomainEvent::ItemDropped {
                    actor_id,
                    actor_name,
                    item_id,
                    item_name,
                    room_id,
                } => {
                    self.validate_persisted_item_event(
                        *actor_id, actor_name, item_id, item_name, room_id,
                    )?;
                    match restored.get_mut(item_id) {
                        Some(item)
                            if item.location == SharedItemLocation::Room(room_id.clone()) =>
                        {
                            item.location = SharedItemLocation::Carried(*actor_id);
                        }
                        _ => return Err(WorldError::InvalidDomainTransition),
                    }
                    self.reconcile_rollback_bindings(&mut restored, std::slice::from_ref(item_id))?;
                    affected_characters.insert(*actor_id);
                    item_event_count += 1;
                }
                DomainEvent::ItemsMoved {
                    actor_id,
                    item_ids,
                    container_id,
                    operation,
                    room_id,
                    ..
                } => {
                    self.validate_items_moved_event(event)?;
                    for item_id in item_ids {
                        let expected = match operation {
                            ContainerMoveOperation::Take | ContainerMoveOperation::Get => {
                                SharedItemLocation::Carried(*actor_id)
                            }
                            ContainerMoveOperation::Drop => {
                                SharedItemLocation::Room(room_id.clone())
                            }
                            ContainerMoveOperation::Put => SharedItemLocation::Contained(
                                container_id
                                    .clone()
                                    .ok_or(WorldError::InvalidDomainTransition)?,
                            ),
                        };
                        let destination = match operation {
                            ContainerMoveOperation::Take => {
                                SharedItemLocation::Room(room_id.clone())
                            }
                            ContainerMoveOperation::Drop | ContainerMoveOperation::Put => {
                                SharedItemLocation::Carried(*actor_id)
                            }
                            ContainerMoveOperation::Get => SharedItemLocation::Contained(
                                container_id
                                    .clone()
                                    .ok_or(WorldError::InvalidDomainTransition)?,
                            ),
                        };
                        let instance = restored
                            .get_mut(item_id)
                            .ok_or(WorldError::InvalidDomainTransition)?;
                        if instance.location != expected {
                            return Err(WorldError::InvalidDomainTransition);
                        }
                        instance.location = destination;
                    }
                    self.reconcile_rollback_bindings(&mut restored, item_ids)?;
                    affected_characters.insert(*actor_id);
                    item_event_count += 1;
                }
                DomainEvent::ItemsGiven {
                    giver_id,
                    recipient_id,
                    item_ids,
                    ..
                } => {
                    self.validate_items_given_event(event)?;
                    for item_id in item_ids {
                        let instance = restored
                            .get_mut(item_id)
                            .ok_or(WorldError::InvalidDomainTransition)?;
                        if instance.location != SharedItemLocation::Carried(*recipient_id) {
                            return Err(WorldError::InvalidDomainTransition);
                        }
                        instance.location = SharedItemLocation::Carried(*giver_id);
                    }
                    self.reconcile_rollback_bindings(&mut restored, item_ids)?;
                    affected_characters.insert(*giver_id);
                    affected_characters.insert(*recipient_id);
                    item_event_count += 1;
                }
            }
        }
        if !affected_characters.is_empty() {
            let loaded = affected_characters
                .iter()
                .copied()
                .filter(|id| self.character_gameplay.contains_key(id))
                .collect::<Vec<_>>();
            if loaded.is_empty() {
                validate_item_graph(&restored, &self.item_prototypes)?;
            } else {
                self.validate_prospective_item_locations(&restored, loaded)?;
            }
        }
        self.item_instances = restored;
        if item_event_count > 0 {
            self.restore_last_view_bump();
            self.revision = self.revision.saturating_sub(item_event_count).max(1);
        }
        Ok(())
    }
}

impl World {
    pub(crate) fn validate_persisted_item_event(
        &self,
        actor_id: CharacterId,
        actor_name: &str,
        item_id: &ItemInstanceId,
        item_name: &str,
        room_id: &str,
    ) -> Result<(), WorldError> {
        let valid_actor = actor_id.0 > 0 && !actor_name.trim().is_empty();
        let valid_item = self
            .item_prototype_for(item_id)
            .is_ok_and(|item| item.name == item_name && !item_name.trim().is_empty());
        let valid_room = self.stable_rooms.contains_key(room_id);
        if valid_actor && valid_item && valid_room {
            Ok(())
        } else {
            Err(WorldError::InvalidDomainEvent)
        }
    }

    pub(crate) fn validate_items_moved_event(&self, event: &DomainEvent) -> Result<(), WorldError> {
        let DomainEvent::ItemsMoved {
            actor_id,
            actor_name,
            item_ids,
            item_name,
            container_id,
            operation,
            room_id,
        } = event
        else {
            return Err(WorldError::InvalidDomainEvent);
        };
        let container_shape_valid = match operation {
            ContainerMoveOperation::Take | ContainerMoveOperation::Drop => container_id.is_none(),
            ContainerMoveOperation::Put | ContainerMoveOperation::Get => {
                container_id.as_ref().is_some_and(|container_id| {
                    self.item_instances
                        .get(container_id)
                        .and_then(|instance| self.item_prototypes.get(&instance.prototype_id))
                        .is_some_and(|prototype| prototype.container.is_some())
                        && self
                            .container_root_matches_event(container_id, *actor_id, room_id)
                            .unwrap_or(false)
                })
            }
        };
        let unique = item_ids.iter().collect::<BTreeSet<_>>().len() == item_ids.len();
        let first_prototype = item_ids.first().and_then(|item_id| {
            self.item_instances
                .get(item_id)
                .map(|instance| instance.prototype_id.as_str())
        });
        let items_valid = !item_ids.is_empty()
            && item_ids.len() <= usize::from(MAX_ITEM_SELECTION_QUANTITY)
            && unique
            && first_prototype.is_some()
            && item_ids.iter().all(|item_id| {
                self.item_instances
                    .get(item_id)
                    .is_some_and(|instance| Some(instance.prototype_id.as_str()) == first_prototype)
                    && self
                        .item_prototype_for(item_id)
                        .is_ok_and(|prototype| prototype.name == item_name.as_str())
            });
        if actor_id.0 <= 0
            || actor_name.trim().is_empty()
            || item_name.trim().is_empty()
            || !self.stable_rooms.contains_key(room_id)
            || !container_shape_valid
            || !items_valid
        {
            return Err(WorldError::InvalidDomainEvent);
        }
        Ok(())
    }

    pub(crate) fn validate_items_given_event(&self, event: &DomainEvent) -> Result<(), WorldError> {
        let DomainEvent::ItemsGiven {
            giver_id,
            giver_name,
            recipient_id,
            recipient_name,
            item_ids,
            item_name,
            room_id,
        } = event
        else {
            return Err(WorldError::InvalidDomainEvent);
        };
        let unique = item_ids.iter().collect::<BTreeSet<_>>().len() == item_ids.len();
        let first_prototype = item_ids.first().and_then(|item_id| {
            self.item_instances
                .get(item_id)
                .map(|instance| instance.prototype_id.as_str())
        });
        let graph = build_item_graph_index(&self.item_instances, &self.item_prototypes)
            .map_err(|_| WorldError::InvalidDomainEvent)?;
        let items_valid = !item_ids.is_empty()
            && item_ids.len() <= usize::from(MAX_ITEM_SELECTION_QUANTITY)
            && unique
            && first_prototype.is_some()
            && item_ids.iter().all(|item_id| {
                self.item_instances
                    .get(item_id)
                    .is_some_and(|instance| Some(instance.prototype_id.as_str()) == first_prototype)
                    && self
                        .item_prototype_for(item_id)
                        .is_ok_and(|prototype| prototype.name == item_name.as_str())
                    && graph.subtree_is_transferable(item_id).unwrap_or(false)
            });
        if giver_id.0 <= 0
            || recipient_id.0 <= 0
            || giver_id == recipient_id
            || giver_name.trim().is_empty()
            || recipient_name.trim().is_empty()
            || item_name.trim().is_empty()
            || !self.stable_rooms.contains_key(room_id)
            || !items_valid
        {
            return Err(WorldError::InvalidDomainEvent);
        }
        Ok(())
    }

    pub(crate) fn container_root_matches_event(
        &self,
        container_id: &ItemInstanceId,
        actor_id: CharacterId,
        room_id: &str,
    ) -> Result<bool, WorldError> {
        let mut current = container_id;
        for _ in 0..=MAX_CONTAINER_DEPTH {
            let instance = self
                .item_instances
                .get(current)
                .ok_or(WorldError::InvalidEntity)?;
            match &instance.location {
                SharedItemLocation::Carried(owner) | SharedItemLocation::Equipped(owner, _) => {
                    return Ok(*owner == actor_id);
                }
                SharedItemLocation::Room(root_room) => return Ok(root_room == room_id),
                SharedItemLocation::Contained(parent) => current = parent,
                SharedItemLocation::Staged
                | SharedItemLocation::HeldByNpc(_)
                | SharedItemLocation::Consumed => return Ok(false),
            }
        }
        Err(WorldError::InvalidEntity)
    }
}
