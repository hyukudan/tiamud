//! Item instances, stacks, containers, inventory capacity, movement and
//! the item ownership graph.

use super::*;

impl World {
    pub(crate) fn inventory_usage(
        &self,
        character_id: CharacterId,
    ) -> Result<InventoryCapacityView, WorldError> {
        let capacity = &self
            .character_gameplay
            .get(&character_id)
            .ok_or(WorldError::InvalidEntity)?
            .inventory_capacity;
        let (used_slots, used_weight_grams) = self.inventory_totals(character_id)?;
        Ok(InventoryCapacityView {
            used_slots: u16::try_from(used_slots).map_err(|_| WorldError::InvalidEntity)?,
            max_slots: capacity.max_slots,
            used_weight_grams: u32::try_from(used_weight_grams)
                .map_err(|_| WorldError::InvalidEntity)?,
            max_weight_grams: capacity.max_weight_grams,
        })
    }

    pub(crate) fn inventory_totals(
        &self,
        character_id: CharacterId,
    ) -> Result<(u32, u64), WorldError> {
        character_inventory_totals(character_id, &self.item_instances, &self.item_prototypes)
    }

    pub(crate) fn inventory_usage_for_capacity(
        &self,
        character_id: CharacterId,
        capacity: &InventoryCapacity,
    ) -> Result<bool, WorldError> {
        let (slots, weight) = self.inventory_totals(character_id)?;
        Ok(
            slots <= u32::from(capacity.max_slots)
                && weight <= u64::from(capacity.max_weight_grams),
        )
    }

    pub(crate) fn validate_prospective_item_locations(
        &self,
        instances: &BTreeMap<ItemInstanceId, ItemInstance>,
        character_ids: impl IntoIterator<Item = CharacterId>,
    ) -> Result<(), WorldError> {
        let graph = build_item_graph_index(instances, &self.item_prototypes)?;
        validate_container_capacities(instances, &self.item_prototypes, &graph)?;
        validate_item_bindings(instances, &self.item_prototypes)?;
        for character_id in character_ids {
            let capacity = &self
                .character_gameplay
                .get(&character_id)
                .ok_or(WorldError::InvalidEntity)?
                .inventory_capacity;
            let (slots, weight) = character_inventory_totals_with_graph(
                character_id,
                instances,
                &self.item_prototypes,
                &graph,
            )?;
            if slots > u32::from(capacity.max_slots)
                || weight > u64::from(capacity.max_weight_grams)
            {
                return Err(WorldError::InventoryCapacityExceeded);
            }
        }
        Ok(())
    }

    pub(crate) fn apply_item_ownership_transitions(
        &self,
        prospective: &mut BTreeMap<ItemInstanceId, ItemInstance>,
        roots: &[ItemInstanceId],
    ) -> Result<(), WorldError> {
        let current_graph = build_item_graph_index(&self.item_instances, &self.item_prototypes)?;
        let mut visited = BTreeSet::new();
        for root in roots {
            let old_owner = item_effective_owner(&self.item_instances, root)?;
            let new_owner = item_effective_owner(prospective, root)?;
            let consumed = matches!(
                prospective.get(root).map(|instance| &instance.location),
                Some(SharedItemLocation::Consumed)
            );
            let subtree = current_graph.subtree_ids(root)?;
            if consumed {
                if old_owner.is_none() || subtree.len() != 1 {
                    return Err(WorldError::ItemTransferForbidden);
                }
                visited.extend(subtree);
                continue;
            }
            match (old_owner, new_owner) {
                (None, Some(character_id)) => {
                    for item_id in subtree {
                        if !visited.insert(item_id.clone()) {
                            continue;
                        }
                        let instance = prospective
                            .get_mut(&item_id)
                            .ok_or(WorldError::InvalidEntity)?;
                        let prototype = self
                            .item_prototypes
                            .get(&instance.prototype_id)
                            .ok_or(WorldError::InvalidEntity)?;
                        if prototype.transfer_policy == ItemTransferPolicy::CharacterBound {
                            match instance.binding {
                                ItemBinding::Unbound => {
                                    instance.binding = ItemBinding::Character { character_id };
                                }
                                ItemBinding::Character {
                                    character_id: bound,
                                } if bound == character_id => {}
                                ItemBinding::Character { .. } => {
                                    return Err(WorldError::ItemTransferForbidden);
                                }
                            }
                        }
                    }
                }
                (Some(previous), Some(next)) if previous != next => {
                    if !current_graph.subtree_is_transferable(root)? {
                        return Err(WorldError::ItemTransferForbidden);
                    }
                    visited.extend(subtree);
                }
                (Some(_), None) => {
                    if !current_graph.subtree_is_transferable(root)? {
                        return Err(WorldError::ItemTransferForbidden);
                    }
                    visited.extend(subtree);
                }
                _ => {
                    visited.extend(subtree);
                }
            }
        }
        validate_item_bindings(prospective, &self.item_prototypes)
            .map_err(|_| WorldError::ItemTransferForbidden)
    }

    pub(crate) fn reconcile_rollback_bindings(
        &self,
        prospective: &mut BTreeMap<ItemInstanceId, ItemInstance>,
        roots: &[ItemInstanceId],
    ) -> Result<(), WorldError> {
        let graph = build_item_graph_index(prospective, &self.item_prototypes)?;
        let mut visited = BTreeSet::new();
        for root in roots {
            for item_id in graph.subtree_ids(root)? {
                if !visited.insert(item_id.clone()) {
                    continue;
                }
                let owner = item_effective_owner(prospective, &item_id)?;
                let instance = prospective
                    .get_mut(&item_id)
                    .ok_or(WorldError::InvalidEntity)?;
                let prototype = self
                    .item_prototypes
                    .get(&instance.prototype_id)
                    .ok_or(WorldError::InvalidEntity)?;
                if prototype.transfer_policy != ItemTransferPolicy::CharacterBound {
                    continue;
                }
                match owner {
                    Some(character_id) => {
                        if instance.binding != (ItemBinding::Character { character_id }) {
                            return Err(WorldError::InvalidDomainTransition);
                        }
                    }
                    None if !matches!(instance.location, SharedItemLocation::Consumed) => {
                        instance.binding = ItemBinding::Unbound;
                    }
                    None => {}
                }
            }
        }
        validate_item_bindings(prospective, &self.item_prototypes)
            .map_err(|_| WorldError::InvalidDomainTransition)
    }

    pub fn inventory_as_plain_text(&self, session: SessionId) -> Result<String, WorldError> {
        let character = self.character(session)?;
        let observer_salt = self.observer(session)?.salt;
        let item_graph = build_item_graph_index(&self.item_instances, &self.item_prototypes)?;
        let carried_ids = self
            .item_instances
            .iter()
            .filter_map(|(item_id, instance)| {
                matches!(&instance.location, SharedItemLocation::Carried(owner) if *owner == character.id)
                    .then_some(item_id)
            })
            .cloned()
            .collect::<Vec<_>>();
        let mut item_rows = derive_stack_groups(
            &self.item_instances,
            &self.item_prototypes,
            carried_ids.iter(),
        )?
        .into_iter()
        .map(|group| {
            let item = self
                .item_prototypes
                .get(&group.prototype_id)
                .ok_or(WorldError::InvalidEntity)?;
            let first = group.members.first().ok_or(WorldError::InvalidEntity)?;
            let total_weight = group.members.iter().try_fold(0_u64, |total, item_id| {
                total
                    .checked_add(item_graph.subtree_weight(item_id)?)
                    .ok_or(WorldError::InvalidEntity)
            })?;
            let container_capacity = item
                .container
                .as_ref()
                .map(|_| {
                    container_usage(
                        first,
                        &self.item_instances,
                        &self.item_prototypes,
                        &item_graph,
                    )
                })
                .transpose()?;
            let affix_details = self.item_affix(first).map_or_else(String::new, |affix| {
                format!(
                    "; affix {} (attack {:+}, healing {:+})",
                    affix.label, affix.attack_bonus_delta, affix.heal_amount_delta
                )
            });
            Ok((
                self.item_effective_name(first)?,
                item.rarity,
                item.item_level,
                item.weight_grams,
                group.members.len(),
                stack_slots(item, group.members.len())?,
                total_weight,
                opaque_id(observer_salt, "item", first.as_str()),
                container_capacity,
                item.transfer_policy,
                matches!(
                    group.binding,
                    ItemBinding::Character {
                        character_id: owner
                    } if owner == character.id
                ),
                affix_details,
            ))
        })
        .collect::<Result<Vec<_>, WorldError>>()?;
        item_rows.sort_unstable_by(|left, right| {
            left.0.cmp(&right.0).then_with(|| left.7.cmp(&right.7))
        });
        let gameplay = self
            .character_gameplay
            .get(&character.id)
            .ok_or(WorldError::InvalidEntity)?;
        let usage = self.inventory_usage(character.id)?;
        let mut output = String::new();
        output.push_str(&format!(
            "Capacity: {}/{} slots; {}/{} g.\r\n",
            usage.used_slots,
            gameplay.inventory_capacity.max_slots,
            usage.used_weight_grams,
            gameplay.inventory_capacity.max_weight_grams
        ));
        if item_rows.is_empty() && gameplay.equipment.is_empty() {
            output.push_str("You are carrying nothing.\r\n");
            return Ok(output);
        }
        if !item_rows.is_empty() {
            output.push_str("You are carrying:\r\n");
            for (
                name,
                rarity,
                level,
                unit_weight,
                quantity,
                slots,
                total_weight,
                reference,
                container_capacity,
                transfer_policy,
                bound_to_you,
                affix_details,
            ) in item_rows
            {
                let stack_slots = if slots == 1 {
                    "stack slot"
                } else {
                    "stack slots"
                };
                let container = container_capacity.map_or_else(String::new, |capacity| {
                    format!(
                        "; container {}/{} slots; {}/{} g",
                        capacity.used_slots,
                        capacity.max_slots,
                        capacity.used_weight_grams,
                        capacity.max_weight_grams,
                    )
                });
                output.push_str(&format!(
                    "- {quantity} × {name} [{}; level {level}; {unit_weight} g each; {slots} {stack_slots}; {total_weight} g total; policy {}{}{affix_details}{container}; ref {reference}]\r\n",
                    rarity.as_str(),
                    transfer_policy.as_str(),
                    if bound_to_you { "; bound to you" } else { "" },
                ));
            }
        }
        if !gameplay.equipment.is_empty() {
            output.push_str("Equipped:\r\n");
            for (slot, item_id) in &gameplay.equipment {
                let item = self.item_prototype_for(item_id)?;
                let total_weight = item_graph.subtree_weight(item_id)?;
                let slots = stack_slots(item, 1)?;
                let stack_slots = if slots == 1 {
                    "stack slot"
                } else {
                    "stack slots"
                };
                let container = item
                    .container
                    .as_ref()
                    .map(|_| {
                        container_usage(
                            item_id,
                            &self.item_instances,
                            &self.item_prototypes,
                            &item_graph,
                        )
                    })
                    .transpose()?
                    .map_or_else(String::new, |capacity| {
                        format!(
                            "; container {}/{} slots; {}/{} g",
                            capacity.used_slots,
                            capacity.max_slots,
                            capacity.used_weight_grams,
                            capacity.max_weight_grams,
                        )
                    });
                let name = self.item_effective_name(item_id)?;
                let affix_details = self.item_affix(item_id).map_or_else(String::new, |affix| {
                    format!(
                        "; affix {} (attack {:+}, healing {:+})",
                        affix.label, affix.attack_bonus_delta, affix.heal_amount_delta
                    )
                });
                output.push_str(&format!(
                    "- {}: 1 × {} [{}; level {}; {} g each; {slots} {stack_slots}; {total_weight} g total; policy {}{}{affix_details}{container}; ref {}]\r\n",
                    slot.as_str(),
                    name,
                    item.rarity.as_str(),
                    item.item_level,
                    item.weight_grams,
                    item.transfer_policy.as_str(),
                    if matches!(
                        self.item_instances
                            .get(item_id)
                            .ok_or(WorldError::InvalidEntity)?
                            .binding,
                        ItemBinding::Character { character_id: owner } if owner == character.id
                    ) {
                        "; bound to you"
                    } else {
                        ""
                    },
                    opaque_id(observer_salt, "item", item_id.as_str())
                ));
            }
        }
        Ok(output)
    }
}

impl World {
    pub(crate) fn move_item(
        &mut self,
        session: SessionId,
        selection: &ItemSelection,
        take: bool,
    ) -> Result<ActionOutcome, WorldError> {
        let scope = if take {
            ItemScope::CurrentRoom
        } else {
            ItemScope::Carried
        };
        let all_candidates = self.item_candidates(session, scope, None)?;
        let mut candidates = all_candidates.clone();
        if take {
            let character_id = self.character(session)?.id;
            candidates.retain(|item_id| {
                self.can_take_ground_loot(character_id, item_id)
                    .unwrap_or(false)
            });
        }
        let item_ids = match self.resolve_item_selection(session, selection, candidates) {
            Ok(item_ids) => item_ids,
            Err(_)
                if take
                    && self
                        .resolve_item_selection(session, selection, all_candidates)
                        .is_ok() =>
            {
                return Err(WorldError::LootUnavailable);
            }
            Err(error) => return Err(error),
        };
        self.ensure_items_unreserved(&item_ids)?;
        let first_item_id = item_ids.first().ok_or(WorldError::InvalidEntity)?;
        let item = self.item_prototype_for(first_item_id)?;
        let item_name = item.name.clone();
        let room_id = room_checked(&self.entities, self.character(session)?.location)?
            .stable_id
            .clone();
        let character = self.character(session)?;
        let actor_id = character.id;
        let actor_name = character.name.clone();
        let destination = if take {
            SharedItemLocation::Carried(actor_id)
        } else {
            SharedItemLocation::Room(room_id.clone())
        };
        let mut prospective = self.item_instances.clone();
        for item_id in &item_ids {
            prospective
                .get_mut(item_id)
                .ok_or(WorldError::InvalidEntity)?
                .location = destination.clone();
        }
        self.apply_item_ownership_transitions(&mut prospective, &item_ids)?;
        self.validate_prospective_item_locations(&prospective, [actor_id])?;
        self.item_instances = prospective;
        if take {
            for item_id in &item_ids {
                self.ground_loot.remove(item_id);
            }
        }
        self.revision = self.revision.wrapping_add(1).max(1);
        let event = if let [item_id] = item_ids.as_slice() {
            if take {
                DomainEvent::ItemTaken {
                    actor_id,
                    actor_name: actor_name.clone(),
                    item_id: item_id.clone(),
                    item_name: item_name.clone(),
                    room_id: room_id.clone(),
                }
            } else {
                DomainEvent::ItemDropped {
                    actor_id,
                    actor_name: actor_name.clone(),
                    item_id: item_id.clone(),
                    item_name: item_name.clone(),
                    room_id: room_id.clone(),
                }
            }
        } else {
            DomainEvent::ItemsMoved {
                actor_id,
                actor_name,
                item_ids,
                item_name: item_name.clone(),
                container_id: None,
                operation: if take {
                    ContainerMoveOperation::Take
                } else {
                    ContainerMoveOperation::Drop
                },
                room_id,
            }
        };
        let events = vec![event];
        self.bump_view_revisions_for_events(&events)?;
        Ok(ActionOutcome {
            scene: self.scene_for(session)?,
            container_contents: None,
            trade_contents: None,
            reward_odds: None,
            output: Some(match (take, selection.quantity) {
                (true, 1) => format!("You take the {item_name}.\r\n"),
                (false, 1) => format!("You drop the {item_name}.\r\n"),
                (true, quantity) => format!("You take {quantity} × {item_name}.\r\n"),
                (false, quantity) => format!("You drop {quantity} × {item_name}.\r\n"),
            }),
            changed_character: None,
            domain_events: events,
        })
    }

    pub(crate) fn give_item(
        &mut self,
        session: SessionId,
        selection: &ItemSelection,
        recipient_query: &str,
    ) -> Result<ActionOutcome, WorldError> {
        let recipient = self.resolve_give_recipient(session, recipient_query)?;
        let giver = self.character(session)?;
        let giver_id = giver.id;
        let giver_name = giver.name.clone();
        let room_id = room_checked(&self.entities, giver.location)?
            .stable_id
            .clone();
        let candidates = self.item_candidates(session, ItemScope::Carried, None)?;
        let item_ids = self.resolve_item_selection(session, selection, candidates)?;
        self.ensure_items_unreserved(&item_ids)?;
        let first_item_id = item_ids.first().ok_or(WorldError::InvalidEntity)?;
        let item_name = self.item_effective_name(first_item_id)?;
        let mut prospective = self.item_instances.clone();
        for item_id in &item_ids {
            prospective
                .get_mut(item_id)
                .ok_or(WorldError::InvalidEntity)?
                .location = SharedItemLocation::Carried(recipient.character_id);
        }
        self.apply_item_ownership_transitions(&mut prospective, &item_ids)?;
        self.validate_prospective_item_locations(&prospective, [giver_id, recipient.character_id])?;
        self.item_instances = prospective;
        self.revision = self.revision.wrapping_add(1).max(1);
        let event = DomainEvent::ItemsGiven {
            giver_id,
            giver_name,
            recipient_id: recipient.character_id,
            recipient_name: recipient.name.clone(),
            item_ids,
            item_name: item_name.clone(),
            room_id,
        };
        self.bump_view_revisions(&event)?;
        Ok(ActionOutcome {
            scene: self.scene_for(session)?,
            container_contents: None,
            trade_contents: None,
            reward_odds: None,
            output: Some(if selection.quantity == 1 {
                format!("You give the {item_name} to {}.\r\n", recipient.name)
            } else {
                format!(
                    "You give {} × {item_name} to {}.\r\n",
                    selection.quantity, recipient.name
                )
            }),
            changed_character: None,
            domain_events: vec![event],
        })
    }

    pub(crate) fn item_candidates(
        &self,
        session: SessionId,
        scope: ItemScope,
        direct_container: Option<&ItemInstanceId>,
    ) -> Result<Vec<ItemInstanceId>, WorldError> {
        let character = self.character(session)?;
        let room_id = &room_checked(&self.entities, character.location)?.stable_id;
        self.item_instances
            .iter()
            .map(|(item_id, instance)| {
                let accessible = if let Some(container_id) = direct_container {
                    matches!(&instance.location, SharedItemLocation::Contained(parent) if parent == container_id)
                } else {
                    match scope {
                        ItemScope::Carried => {
                            matches!(&instance.location, SharedItemLocation::Carried(owner) if *owner == character.id)
                        }
                        ItemScope::CurrentRoom => {
                            matches!(&instance.location, SharedItemLocation::Room(item_room) if item_room == room_id)
                        }
                        ItemScope::Accessible => {
                            matches!(&instance.location, SharedItemLocation::Carried(owner) if *owner == character.id)
                                || matches!(&instance.location, SharedItemLocation::Room(item_room) if item_room == room_id)
                        }
                    }
                };
                let visible = match &instance.location {
                    SharedItemLocation::Room(item_room) => {
                        self.item_visible_to(session, item_id, item_room)?
                    }
                    SharedItemLocation::Carried(owner) => *owner == character.id,
                    SharedItemLocation::Contained(parent) => direct_container == Some(parent),
                    SharedItemLocation::Staged
                    | SharedItemLocation::Equipped(_, _)
                    | SharedItemLocation::HeldByNpc(_)
                    | SharedItemLocation::Consumed => false,
                };
                Ok((accessible && visible).then_some(item_id.clone()))
            })
            .collect::<Result<Vec<_>, WorldError>>()
            .map(|items| items.into_iter().flatten().collect())
    }

    pub(crate) fn resolve_item_selection(
        &self,
        session: SessionId,
        selection: &ItemSelection,
        mut candidates: Vec<ItemInstanceId>,
    ) -> Result<Vec<ItemInstanceId>, WorldError> {
        if selection.quantity == 0 || selection.quantity > MAX_ITEM_SELECTION_QUANTITY {
            return Err(WorldError::GameplayUnavailable);
        }
        candidates.sort();
        let observer_salt = self.observer(session)?.salt;
        let query = selection.query.trim().to_ascii_lowercase();
        if query.is_empty() {
            return Err(WorldError::ItemNotFound(query));
        }

        if let Some(exact_id) = candidates
            .iter()
            .find(|item_id| opaque_id(observer_salt, "item", item_id.as_str()) == query)
        {
            let instance = self
                .item_instances
                .get(exact_id)
                .ok_or(WorldError::InvalidEntity)?;
            let prototype = self
                .item_prototypes
                .get(&instance.prototype_id)
                .ok_or(WorldError::InvalidEntity)?;
            let eligible = if prototype.stack_policy == ItemStackPolicy::Fungible {
                derive_stack_groups(
                    &self.item_instances,
                    &self.item_prototypes,
                    candidates.iter().filter(|candidate| {
                        self.item_instances
                            .get(*candidate)
                            .is_some_and(|candidate_instance| {
                                candidate_instance.prototype_id == instance.prototype_id
                            })
                    }),
                )?
                .into_iter()
                .find(|group| group.members.contains(exact_id))
                .ok_or_else(|| WorldError::ItemNotFound(query.clone()))?
                .members
            } else {
                vec![exact_id.clone()]
            };
            return take_selected_quantity(eligible, selection.quantity, &query);
        }

        let mut representatives = Vec::new();
        let mut represented_fungible = BTreeSet::new();
        for item_id in &candidates {
            let instance = self
                .item_instances
                .get(item_id)
                .ok_or(WorldError::InvalidEntity)?;
            let prototype = self
                .item_prototypes
                .get(&instance.prototype_id)
                .ok_or(WorldError::InvalidEntity)?;
            if prototype.stack_policy == ItemStackPolicy::Unique
                || represented_fungible.insert(instance.prototype_id.clone())
            {
                representatives.push(item_id);
            }
        }
        let resolved = resolve_item_query(
            &self.item_instances,
            &self.item_prototypes,
            representatives.into_iter(),
            &query,
            observer_salt,
        )?;
        let instance = self
            .item_instances
            .get(&resolved)
            .ok_or(WorldError::InvalidEntity)?;
        let prototype = self
            .item_prototypes
            .get(&instance.prototype_id)
            .ok_or(WorldError::InvalidEntity)?;
        let eligible = if prototype.stack_policy == ItemStackPolicy::Fungible {
            candidates
                .into_iter()
                .filter(|candidate| {
                    self.item_instances
                        .get(candidate)
                        .is_some_and(|candidate_instance| {
                            candidate_instance.prototype_id == instance.prototype_id
                        })
                })
                .collect()
        } else {
            vec![resolved]
        };
        take_selected_quantity(eligible, selection.quantity, &query)
    }

    pub(crate) fn item_accessible_as_container(
        &self,
        session: SessionId,
        item_id: &ItemInstanceId,
    ) -> Result<bool, WorldError> {
        let character = self.character(session)?;
        let room_id = &room_checked(&self.entities, character.location)?.stable_id;
        let mut current = item_id;
        for _ in 0..=MAX_CONTAINER_DEPTH {
            let instance = self
                .item_instances
                .get(current)
                .ok_or(WorldError::InvalidEntity)?;
            match &instance.location {
                SharedItemLocation::Carried(owner) | SharedItemLocation::Equipped(owner, _) => {
                    return Ok(*owner == character.id);
                }
                SharedItemLocation::Room(root_room) => {
                    return Ok(root_room == room_id
                        && self.item_visible_to(session, current, root_room)?);
                }
                SharedItemLocation::Contained(parent) => current = parent,
                SharedItemLocation::Staged
                | SharedItemLocation::HeldByNpc(_)
                | SharedItemLocation::Consumed => return Ok(false),
            }
        }
        Err(WorldError::InvalidEntity)
    }

    pub(crate) fn ground_loot_root_for(
        &self,
        item_id: &ItemInstanceId,
    ) -> Result<Option<ItemInstanceId>, WorldError> {
        let mut current = item_id;
        for _ in 0..=MAX_CONTAINER_DEPTH {
            if self.ground_loot.contains_key(current) {
                return Ok(Some(current.clone()));
            }
            let instance = self
                .item_instances
                .get(current)
                .ok_or(WorldError::InvalidEntity)?;
            match &instance.location {
                SharedItemLocation::Contained(parent) => current = parent,
                _ => return Ok(None),
            }
        }
        Err(WorldError::InvalidEntity)
    }

    pub(crate) fn loot_reservation_for_item(
        &self,
        item_id: &ItemInstanceId,
    ) -> Result<Option<&GroundLootReservation>, WorldError> {
        self.ground_loot_root_for(item_id)
            .map(|root| root.and_then(|root| self.ground_loot.get(&root)))
    }

    pub(crate) fn can_take_ground_loot(
        &self,
        character_id: CharacterId,
        item_id: &ItemInstanceId,
    ) -> Result<bool, WorldError> {
        Ok(self
            .loot_reservation_for_item(item_id)?
            .is_none_or(|reservation| {
                self.logical_tick >= reservation.claim_until_tick
                    || reservation.beneficiary.is_none()
                    || reservation.beneficiary == Some(character_id)
            }))
    }

    pub(crate) fn scene_loot_access(
        &self,
        character_id: CharacterId,
        item_id: &ItemInstanceId,
    ) -> Result<Option<SceneLootAccessView>, WorldError> {
        let Some(reservation) = self.loot_reservation_for_item(item_id)? else {
            return Ok(None);
        };
        if self.logical_tick >= reservation.claim_until_tick || reservation.beneficiary.is_none() {
            return Ok(None);
        }
        Ok(Some(if reservation.beneficiary == Some(character_id) {
            SceneLootAccessView {
                access: LootAccessView::Yours,
                expires_in_ticks: Some(
                    reservation
                        .claim_until_tick
                        .saturating_sub(self.logical_tick),
                ),
            }
        } else {
            SceneLootAccessView {
                access: LootAccessView::Reserved,
                expires_in_ticks: None,
            }
        }))
    }

    pub(crate) fn resolve_accessible_container(
        &self,
        session: SessionId,
        query: &str,
    ) -> Result<ItemInstanceId, WorldError> {
        let character_id = self.character(session)?.id;
        let candidates = self
            .item_instances
            .iter()
            .map(|(item_id, instance)| {
                let is_container = self
                    .item_prototypes
                    .get(&instance.prototype_id)
                    .is_some_and(|prototype| prototype.container.is_some());
                let reserved_descendant =
                    matches!(&instance.location, SharedItemLocation::Contained(_))
                        && !self.can_take_ground_loot(character_id, item_id)?;
                self.item_accessible_as_container(session, item_id)
                    .map(|accessible| {
                        (is_container && accessible && !reserved_descendant).then_some(item_id)
                    })
            })
            .collect::<Result<Vec<_>, WorldError>>()?;
        resolve_item_query(
            &self.item_instances,
            &self.item_prototypes,
            candidates.into_iter().flatten(),
            query,
            self.observer(session)?.salt,
        )
    }

    pub(crate) fn inspect_container(
        &self,
        session: SessionId,
        query: &str,
        page: u16,
    ) -> Result<ActionOutcome, WorldError> {
        if page == 0 {
            return Err(WorldError::GameplayUnavailable);
        }
        let container_id = self.resolve_accessible_container(session, query)?;
        let character_id = self.character(session)?.id;
        if !self.can_take_ground_loot(character_id, &container_id)? {
            return Err(WorldError::LootUnavailable);
        }
        let observer = self.observer(session)?;
        let item_graph = build_item_graph_index(&self.item_instances, &self.item_prototypes)?;
        let reserved_item_ids = reserved_item_ids_with_graph(&self.trades, &item_graph)?;
        let container_reserved = reserved_item_ids.contains(&container_id);
        let selector = opaque_id(observer.salt, "item", container_id.as_str());
        let direct_ids =
            self.item_candidates(session, ItemScope::Accessible, Some(&container_id))?;
        let groups = derive_stack_groups(
            &self.item_instances,
            &self.item_prototypes,
            direct_ids.iter(),
        )?;
        let total_pages = groups.len().max(1).div_ceil(CONTAINER_PAGE_SIZE);
        let page_index = page_index(page);
        if page_index >= total_pages {
            return Err(WorldError::GameplayUnavailable);
        }
        let start = page_index
            .checked_mul(CONTAINER_PAGE_SIZE)
            .ok_or(WorldError::GameplayUnavailable)?;
        let items = groups
            .into_iter()
            .skip(start)
            .take(CONTAINER_PAGE_SIZE)
            .map(|group| {
                let prototype = self
                    .item_prototypes
                    .get(&group.prototype_id)
                    .ok_or(WorldError::InvalidEntity)?;
                let first = group.members.first().ok_or(WorldError::InvalidEntity)?;
                let item_selector = opaque_id(observer.salt, "item", first.as_str());
                let quantity =
                    u16::try_from(group.members.len()).map_err(|_| WorldError::InvalidEntity)?;
                let reserved_for_trade = container_reserved
                    || group
                        .members
                        .iter()
                        .any(|item_id| reserved_item_ids.contains(item_id));
                let total_weight_grams =
                    group.members.iter().try_fold(0_u64, |total, item_id| {
                        total
                            .checked_add(item_graph.subtree_weight(item_id)?)
                            .ok_or(WorldError::InvalidEntity)
                    })?;
                let mut actions = vec![SceneAction {
                    label: "Examine".into(),
                    command: format!("examine {item_selector}"),
                }];
                if !reserved_for_trade {
                    actions.push(SceneAction {
                        label: "Get".into(),
                        command: format!("get {item_selector} from {selector}"),
                    });
                }
                Ok(PlayerItemView {
                    id: item_selector.clone(),
                    label: self.item_effective_name(first)?,
                    description: prototype.description.clone(),
                    rarity: prototype.rarity.as_str().into(),
                    item_level: prototype.item_level,
                    weight_grams: prototype.weight_grams,
                    quantity,
                    available_quantity: if reserved_for_trade { 0 } else { quantity },
                    reserved_for_trade: if reserved_for_trade { quantity } else { 0 },
                    stack_slots: stack_slots(prototype, group.members.len())?,
                    total_weight_grams,
                    transfer_policy: prototype.transfer_policy,
                    bound_to_you: matches!(
                        group.binding,
                        ItemBinding::Character {
                            character_id: owner
                        } if owner == character_id
                    ),
                    can_give: false,
                    can_trade: false,
                    affix: self.item_affix_view(first),
                    container_capacity: prototype
                        .container
                        .as_ref()
                        .map(|_| {
                            container_usage(
                                first,
                                &self.item_instances,
                                &self.item_prototypes,
                                &item_graph,
                            )
                        })
                        .transpose()?,
                    actions,
                })
            })
            .collect::<Result<Vec<_>, WorldError>>()?;
        let capacity = container_usage(
            &container_id,
            &self.item_instances,
            &self.item_prototypes,
            &item_graph,
        )?;
        let total_pages = u16::try_from(total_pages).map_err(|_| WorldError::InvalidEntity)?;
        let mut output = format!(
            "Container capacity: {}/{} slots; {}/{} g. Page {page}/{total_pages}.\r\n",
            capacity.used_slots,
            capacity.max_slots,
            capacity.used_weight_grams,
            capacity.max_weight_grams,
        );
        if items.is_empty() {
            output.push_str("It is empty.\r\n");
        } else {
            for item in &items {
                let stack_slots = if item.stack_slots == 1 {
                    "stack slot"
                } else {
                    "stack slots"
                };
                let container =
                    item.container_capacity
                        .as_ref()
                        .map_or_else(String::new, |capacity| {
                            format!(
                                "; container {}/{} slots; {}/{} g",
                                capacity.used_slots,
                                capacity.max_slots,
                                capacity.used_weight_grams,
                                capacity.max_weight_grams,
                            )
                        });
                let reservation = if item.reserved_for_trade == 0 {
                    String::new()
                } else {
                    format!(
                        "; {} reserved for trade; {} available",
                        item.reserved_for_trade, item.available_quantity
                    )
                };
                output.push_str(&format!(
                    "- {} × {} [{}; level {}; {} g each; {} {stack_slots}; {} g total; policy {}{}{}{container}; ref {}]\r\n",
                    item.quantity,
                    item.label,
                    item.rarity,
                    item.item_level,
                    item.weight_grams,
                    item.stack_slots,
                    item.total_weight_grams,
                    item.transfer_policy.as_str(),
                    if item.bound_to_you { "; bound to you" } else { "" },
                    reservation,
                    item.id,
                ));
            }
        }
        Ok(ActionOutcome {
            scene: self.scene_for(session)?,
            container_contents: Some(ContainerContentsView {
                selector,
                items,
                page,
                total_pages,
                capacity,
            }),
            trade_contents: None,
            reward_odds: None,
            output: Some(output),
            changed_character: None,
            domain_events: Vec::new(),
        })
    }

    pub(crate) fn move_item_to_container(
        &mut self,
        session: SessionId,
        selection: &ItemSelection,
        container_query: &str,
        put: bool,
    ) -> Result<ActionOutcome, WorldError> {
        let container_id = self.resolve_accessible_container(session, container_query)?;
        self.ensure_items_unreserved(std::iter::once(&container_id))?;
        let actor_id = self.character(session)?.id;
        if !self.can_take_ground_loot(actor_id, &container_id)? {
            return Err(WorldError::LootUnavailable);
        }
        let candidates = if put {
            self.item_candidates(session, ItemScope::Carried, None)?
        } else {
            self.item_candidates(session, ItemScope::Accessible, Some(&container_id))?
        };
        let item_ids = self.resolve_item_selection(session, selection, candidates)?;
        self.ensure_items_unreserved(&item_ids)?;
        if !put
            && item_ids.iter().any(|item_id| {
                !self
                    .can_take_ground_loot(actor_id, item_id)
                    .unwrap_or(false)
            })
        {
            return Err(WorldError::LootUnavailable);
        }
        let first_id = item_ids.first().ok_or(WorldError::InvalidEntity)?;
        let item_name = self.item_effective_name(first_id)?;
        let character = self.character(session)?;
        let actor_id = character.id;
        let actor_name = character.name.clone();
        let room_id = room_checked(&self.entities, character.location)?
            .stable_id
            .clone();
        let destination = if put {
            SharedItemLocation::Contained(container_id.clone())
        } else {
            SharedItemLocation::Carried(actor_id)
        };
        let mut prospective = self.item_instances.clone();
        for item_id in &item_ids {
            prospective
                .get_mut(item_id)
                .ok_or(WorldError::InvalidEntity)?
                .location = destination.clone();
        }
        self.apply_item_ownership_transitions(&mut prospective, &item_ids)?;
        self.validate_prospective_item_locations(&prospective, [actor_id])?;
        self.item_instances = prospective;
        self.revision = self.revision.wrapping_add(1).max(1);
        let event = DomainEvent::ItemsMoved {
            actor_id,
            actor_name,
            item_ids,
            item_name: item_name.clone(),
            container_id: Some(container_id),
            operation: if put {
                ContainerMoveOperation::Put
            } else {
                ContainerMoveOperation::Get
            },
            room_id,
        };
        self.bump_view_revisions(&event)?;
        Ok(ActionOutcome {
            scene: self.scene_for(session)?,
            container_contents: None,
            trade_contents: None,
            reward_odds: None,
            output: Some(if put {
                format!(
                    "You put {} × {item_name} in the container.\r\n",
                    selection.quantity
                )
            } else {
                format!(
                    "You get {} × {item_name} from the container.\r\n",
                    selection.quantity
                )
            }),
            changed_character: None,
            domain_events: vec![event],
        })
    }

    pub(crate) fn resolve_accessible_item(
        &self,
        session: SessionId,
        query: &str,
        scope: ItemScope,
    ) -> Result<ItemInstanceId, WorldError> {
        let character = self.character(session)?;
        let room_id = room_checked(&self.entities, character.location)?
            .stable_id
            .as_str();
        let candidates = self
            .item_instances
            .iter()
            .map(|(item_id, instance)| {
                let location = &instance.location;
                let accessible = match scope {
                    ItemScope::Carried => {
                        matches!(location, SharedItemLocation::Carried(owner) if *owner == character.id)
                    }
                    ItemScope::CurrentRoom => {
                        matches!(location, SharedItemLocation::Room(item_room) if item_room == room_id)
                    }
                    ItemScope::Accessible => {
                        matches!(location, SharedItemLocation::Carried(owner) if *owner == character.id)
                            || matches!(location, SharedItemLocation::Room(item_room) if item_room == room_id)
                            || (matches!(location, SharedItemLocation::Contained(_))
                                && self.item_accessible_as_container(session, item_id)?
                                && self.can_take_ground_loot(character.id, item_id)?)
                    }
                };
                let visible = match location {
                    SharedItemLocation::Room(item_room) => self
                        .item_visible_to(session, item_id, item_room)?,
                    SharedItemLocation::Carried(owner) => *owner == character.id,
                    SharedItemLocation::Contained(_) => {
                        scope == ItemScope::Accessible
                            && self.item_accessible_as_container(session, item_id)?
                            && self.can_take_ground_loot(character.id, item_id)?
                    }
                    SharedItemLocation::Staged
                    | SharedItemLocation::Equipped(_, _)
                    | SharedItemLocation::HeldByNpc(_)
                    | SharedItemLocation::Consumed => false,
                };
                Ok((accessible && visible).then_some(item_id))
            })
            .collect::<Result<Vec<_>, WorldError>>()?;
        resolve_item_query(
            &self.item_instances,
            &self.item_prototypes,
            candidates.into_iter().flatten(),
            query,
            self.observer(session)?.salt,
        )
    }
}

#[derive(Clone, Copy, Eq, PartialEq)]
pub(crate) enum ItemScope {
    Carried,
    CurrentRoom,
    Accessible,
}

pub(crate) const MAX_CONTAINER_DEPTH: usize = 4;
pub(crate) const CONTAINER_PAGE_SIZE: usize = 20;

#[derive(Clone, Debug)]
pub(crate) struct ItemStackGroup {
    pub(crate) prototype_id: String,
    pub(crate) binding: ItemBinding,
    pub(crate) members: Vec<ItemInstanceId>,
}

pub(crate) fn derive_stack_groups<'a>(
    instances: &BTreeMap<ItemInstanceId, ItemInstance>,
    prototypes: &HashMap<String, ItemPrototype>,
    item_ids: impl Iterator<Item = &'a ItemInstanceId>,
) -> Result<Vec<ItemStackGroup>, WorldError> {
    let mut by_prototype =
        BTreeMap::<(String, ItemBinding, Vec<AppliedItemAffix>), Vec<ItemInstanceId>>::new();
    for item_id in item_ids {
        let instance = instances.get(item_id).ok_or(WorldError::InvalidEntity)?;
        by_prototype
            .entry((
                instance.prototype_id.clone(),
                instance.binding,
                instance.item_affixes.clone(),
            ))
            .or_default()
            .push(item_id.clone());
    }
    let mut groups = Vec::new();
    for ((prototype_id, binding, _), mut members) in by_prototype {
        let prototype = prototypes
            .get(&prototype_id)
            .ok_or(WorldError::InvalidEntity)?;
        members.sort();
        for chunk in members.chunks(usize::from(prototype.max_stack_units)) {
            groups.push(ItemStackGroup {
                prototype_id: prototype_id.clone(),
                binding,
                members: chunk.to_vec(),
            });
        }
    }
    groups.sort_by(|left, right| left.members[0].cmp(&right.members[0]));
    Ok(groups)
}

pub(crate) fn take_selected_quantity(
    mut eligible: Vec<ItemInstanceId>,
    quantity: u16,
    query: &str,
) -> Result<Vec<ItemInstanceId>, WorldError> {
    eligible.sort();
    let quantity = usize::from(quantity);
    if eligible.len() < quantity {
        return Err(WorldError::ItemNotFound(query.to_owned()));
    }
    eligible.truncate(quantity);
    Ok(eligible)
}

pub(crate) fn stack_slots(prototype: &ItemPrototype, quantity: usize) -> Result<u32, WorldError> {
    let max = usize::from(prototype.max_stack_units);
    if max == 0 {
        return Err(WorldError::InvalidEntity);
    }
    let chunks = quantity
        .checked_add(max - 1)
        .ok_or(WorldError::InvalidEntity)?
        / max;
    u32::try_from(chunks)
        .ok()
        .and_then(|chunks| chunks.checked_mul(u32::from(prototype.slot_cost)))
        .ok_or(WorldError::InvalidEntity)
}

#[derive(Clone, Debug)]
pub(crate) struct ItemGraphIndex {
    pub(crate) children: BTreeMap<ItemInstanceId, Vec<ItemInstanceId>>,
    pub(crate) subtree_weights: BTreeMap<ItemInstanceId, u64>,
    pub(crate) subtree_transferable: BTreeMap<ItemInstanceId, bool>,
}

impl ItemGraphIndex {
    pub(crate) fn subtree_weight(&self, item_id: &ItemInstanceId) -> Result<u64, WorldError> {
        self.subtree_weights
            .get(item_id)
            .copied()
            .ok_or(WorldError::InvalidEntity)
    }

    pub(crate) fn subtree_is_transferable(
        &self,
        item_id: &ItemInstanceId,
    ) -> Result<bool, WorldError> {
        self.subtree_transferable
            .get(item_id)
            .copied()
            .ok_or(WorldError::InvalidEntity)
    }

    pub(crate) fn subtree_ids(
        &self,
        root: &ItemInstanceId,
    ) -> Result<Vec<ItemInstanceId>, WorldError> {
        if !self.subtree_weights.contains_key(root) {
            return Err(WorldError::InvalidEntity);
        }
        let mut result = Vec::new();
        let mut pending = vec![root.clone()];
        while let Some(item_id) = pending.pop() {
            if let Some(children) = self.children.get(&item_id) {
                pending.extend(children.iter().rev().cloned());
            }
            result.push(item_id);
        }
        Ok(result)
    }
}

pub(crate) fn build_item_graph_index(
    instances: &BTreeMap<ItemInstanceId, ItemInstance>,
    prototypes: &HashMap<String, ItemPrototype>,
) -> Result<ItemGraphIndex, WorldError> {
    let mut children = BTreeMap::<ItemInstanceId, Vec<ItemInstanceId>>::new();
    let mut subtree_weights = BTreeMap::<ItemInstanceId, u64>::new();
    let mut subtree_transferable = BTreeMap::<ItemInstanceId, bool>::new();
    for (item_id, instance) in instances {
        let prototype = prototypes
            .get(&instance.prototype_id)
            .ok_or(WorldError::InvalidEntity)?;
        subtree_weights.insert(item_id.clone(), u64::from(prototype.weight_grams));
        subtree_transferable.insert(
            item_id.clone(),
            prototype.transfer_policy == ItemTransferPolicy::Transferable
                && instance.binding == ItemBinding::Unbound,
        );
        if let SharedItemLocation::Contained(parent) = &instance.location {
            let parent_instance = instances.get(parent).ok_or(WorldError::InvalidEntity)?;
            if prototypes
                .get(&parent_instance.prototype_id)
                .and_then(|prototype| prototype.container.as_ref())
                .is_none()
            {
                return Err(WorldError::InvalidEntity);
            }
            children
                .entry(parent.clone())
                .or_default()
                .push(item_id.clone());
        }
    }

    // Parent chains are at most four edges, so this is O(N) with a small,
    // explicit constant and detects cycles before any recursive accounting.
    let mut depths = BTreeMap::<ItemInstanceId, usize>::new();
    for item_id in instances.keys() {
        let mut current = item_id;
        let mut visited = BTreeSet::new();
        let mut depth = 0_usize;
        while let Some(ItemInstance {
            location: SharedItemLocation::Contained(parent),
            ..
        }) = instances.get(current)
        {
            if !visited.insert(current.clone()) || parent == current {
                return Err(WorldError::InvalidEntity);
            }
            depth = depth.checked_add(1).ok_or(WorldError::InvalidEntity)?;
            if depth > MAX_CONTAINER_DEPTH {
                return Err(WorldError::InvalidEntity);
            }
            current = parent;
        }
        depths.insert(item_id.clone(), depth);
    }

    // Descendants contribute once to their parent after their own descendants
    // have already been folded in. This avoids rescanning all N instances for
    // every container or root inventory item.
    let mut deepest_first = depths.into_iter().collect::<Vec<_>>();
    deepest_first.sort_by(|(left_id, left_depth), (right_id, right_depth)| {
        right_depth
            .cmp(left_depth)
            .then_with(|| left_id.cmp(right_id))
    });
    for (item_id, depth) in deepest_first {
        if depth == 0 {
            continue;
        }
        let parent = match &instances
            .get(&item_id)
            .ok_or(WorldError::InvalidEntity)?
            .location
        {
            SharedItemLocation::Contained(parent) => parent,
            _ => return Err(WorldError::InvalidEntity),
        };
        let child_weight = subtree_weights
            .get(&item_id)
            .copied()
            .ok_or(WorldError::InvalidEntity)?;
        let parent_weight = subtree_weights
            .get_mut(parent)
            .ok_or(WorldError::InvalidEntity)?;
        *parent_weight = parent_weight
            .checked_add(child_weight)
            .ok_or(WorldError::InvalidEntity)?;
        let child_transferable = subtree_transferable
            .get(&item_id)
            .copied()
            .ok_or(WorldError::InvalidEntity)?;
        let parent_transferable = subtree_transferable
            .get_mut(parent)
            .ok_or(WorldError::InvalidEntity)?;
        *parent_transferable &= child_transferable;
    }

    Ok(ItemGraphIndex {
        children,
        subtree_weights,
        subtree_transferable,
    })
}

pub(crate) fn container_usage(
    container_id: &ItemInstanceId,
    instances: &BTreeMap<ItemInstanceId, ItemInstance>,
    prototypes: &HashMap<String, ItemPrototype>,
    graph: &ItemGraphIndex,
) -> Result<InventoryCapacityView, WorldError> {
    let container_instance = instances
        .get(container_id)
        .ok_or(WorldError::InvalidEntity)?;
    let container_prototype = prototypes
        .get(&container_instance.prototype_id)
        .ok_or(WorldError::InvalidEntity)?;
    let capacity = container_prototype
        .container
        .as_ref()
        .ok_or(WorldError::GameplayUnavailable)?;
    let direct = graph
        .children
        .get(container_id)
        .map(Vec::as_slice)
        .unwrap_or_default();
    let mut quantities = BTreeMap::<(String, ItemBinding), usize>::new();
    let mut used_weight = 0_u64;
    for item_id in direct {
        let instance = instances.get(item_id).ok_or(WorldError::InvalidEntity)?;
        *quantities
            .entry((instance.prototype_id.clone(), instance.binding))
            .or_default() += 1;
        used_weight = used_weight
            .checked_add(graph.subtree_weight(item_id)?)
            .ok_or(WorldError::InvalidEntity)?;
    }
    let mut used_slots = 0_u32;
    for ((prototype_id, _), quantity) in quantities {
        let prototype = prototypes
            .get(&prototype_id)
            .ok_or(WorldError::InvalidEntity)?;
        used_slots = used_slots
            .checked_add(stack_slots(prototype, quantity)?)
            .ok_or(WorldError::InvalidEntity)?;
    }
    Ok(InventoryCapacityView {
        used_slots: u16::try_from(used_slots).map_err(|_| WorldError::InvalidEntity)?,
        max_slots: capacity.max_slots,
        used_weight_grams: u32::try_from(used_weight).map_err(|_| WorldError::InvalidEntity)?,
        max_weight_grams: capacity.max_weight_grams,
    })
}

pub(crate) fn character_inventory_totals(
    character_id: CharacterId,
    instances: &BTreeMap<ItemInstanceId, ItemInstance>,
    prototypes: &HashMap<String, ItemPrototype>,
) -> Result<(u32, u64), WorldError> {
    let graph = build_item_graph_index(instances, prototypes)?;
    character_inventory_totals_with_graph(character_id, instances, prototypes, &graph)
}

pub(crate) fn character_inventory_totals_with_graph(
    character_id: CharacterId,
    instances: &BTreeMap<ItemInstanceId, ItemInstance>,
    prototypes: &HashMap<String, ItemPrototype>,
    graph: &ItemGraphIndex,
) -> Result<(u32, u64), WorldError> {
    let mut carried_quantities = BTreeMap::<(String, ItemBinding), usize>::new();
    let mut weight = 0_u64;
    for (item_id, instance) in instances {
        let carried = matches!(&instance.location, SharedItemLocation::Carried(owner) if *owner == character_id);
        let equipped = matches!(&instance.location, SharedItemLocation::Equipped(owner, _) if *owner == character_id);
        if !carried && !equipped {
            continue;
        }
        if carried {
            *carried_quantities
                .entry((instance.prototype_id.clone(), instance.binding))
                .or_default() += 1;
        }
        weight = weight
            .checked_add(graph.subtree_weight(item_id)?)
            .ok_or(WorldError::InvalidEntity)?;
    }
    let mut slots = 0_u32;
    for ((prototype_id, _), quantity) in carried_quantities {
        let prototype = prototypes
            .get(&prototype_id)
            .ok_or(WorldError::InvalidEntity)?;
        slots = slots
            .checked_add(stack_slots(prototype, quantity)?)
            .ok_or(WorldError::InvalidEntity)?;
    }
    Ok((slots, weight))
}

pub(crate) fn validate_item_graph(
    instances: &BTreeMap<ItemInstanceId, ItemInstance>,
    prototypes: &HashMap<String, ItemPrototype>,
) -> Result<(), WorldError> {
    let graph = build_item_graph_index(instances, prototypes)?;
    validate_container_capacities(instances, prototypes, &graph)?;
    validate_item_bindings(instances, prototypes)
}

pub(crate) fn item_effective_owner(
    instances: &BTreeMap<ItemInstanceId, ItemInstance>,
    item_id: &ItemInstanceId,
) -> Result<Option<CharacterId>, WorldError> {
    let mut current = item_id;
    for _ in 0..=MAX_CONTAINER_DEPTH {
        let instance = instances.get(current).ok_or(WorldError::InvalidEntity)?;
        match &instance.location {
            SharedItemLocation::Carried(character_id)
            | SharedItemLocation::Equipped(character_id, _) => return Ok(Some(*character_id)),
            SharedItemLocation::Contained(parent) => current = parent,
            SharedItemLocation::Staged
            | SharedItemLocation::Room(_)
            | SharedItemLocation::HeldByNpc(_)
            | SharedItemLocation::Consumed => return Ok(None),
        }
    }
    Err(WorldError::InvalidEntity)
}

pub(crate) fn items_by_effective_owner(
    instances: &BTreeMap<ItemInstanceId, ItemInstance>,
) -> Result<BTreeMap<CharacterId, BTreeMap<ItemInstanceId, ItemInstance>>, WorldError> {
    let mut by_owner = BTreeMap::<CharacterId, BTreeMap<ItemInstanceId, ItemInstance>>::new();
    for (item_id, instance) in instances {
        if let Some(owner) = item_effective_owner(instances, item_id)? {
            by_owner
                .entry(owner)
                .or_default()
                .insert(item_id.clone(), instance.clone());
        }
    }
    Ok(by_owner)
}

pub(crate) fn reserved_item_ids_with_graph(
    trades: &BTreeMap<TradeId, TradeSnapshot>,
    graph: &ItemGraphIndex,
) -> Result<BTreeSet<ItemInstanceId>, WorldError> {
    let mut reserved = BTreeSet::new();
    for root in trades
        .values()
        .flat_map(|trade| trade.offers.values())
        .flat_map(|offer| &offer.lines)
        .flat_map(|line| &line.item_ids)
    {
        for item_id in graph.subtree_ids(root)? {
            if !reserved.insert(item_id) {
                return Err(WorldError::InvalidRuntimeSnapshot);
            }
        }
    }
    Ok(reserved)
}

pub(crate) fn character_room_id(
    entities: &SlotMap<EntityId, Entity>,
    character_id: CharacterId,
) -> Option<&str> {
    entities.values().find_map(|entity| match entity {
        Entity::Character(character) if character.id == character_id => entities
            .get(character.location)
            .and_then(|room| match room {
                Entity::Room(room) => Some(room.stable_id.as_str()),
                Entity::Character(_) => None,
            }),
        _ => None,
    })
}

pub(crate) fn validate_item_bindings(
    instances: &BTreeMap<ItemInstanceId, ItemInstance>,
    prototypes: &HashMap<String, ItemPrototype>,
) -> Result<(), WorldError> {
    for (item_id, instance) in instances {
        let prototype = prototypes
            .get(&instance.prototype_id)
            .ok_or(WorldError::InvalidEntity)?;
        let owner = item_effective_owner(instances, item_id)?;
        if owner.is_some_and(|character_id| character_id.0 <= 0) {
            return Err(WorldError::InvalidEntity);
        }
        let valid = match prototype.transfer_policy {
            ItemTransferPolicy::Transferable | ItemTransferPolicy::NonTransferable => {
                instance.binding == ItemBinding::Unbound
            }
            ItemTransferPolicy::CharacterBound => match (owner, instance.binding) {
                (Some(owner), ItemBinding::Character { character_id }) => {
                    owner.0 > 0 && character_id.0 > 0 && owner == character_id
                }
                (None, ItemBinding::Unbound) => {
                    !matches!(instance.location, SharedItemLocation::Consumed)
                }
                (None, ItemBinding::Character { character_id }) => {
                    character_id.0 > 0 && matches!(instance.location, SharedItemLocation::Consumed)
                }
                (Some(_), ItemBinding::Unbound) => false,
            },
        };
        if !valid {
            return Err(WorldError::InvalidEntity);
        }
    }
    Ok(())
}

pub(crate) fn validate_restored_item_affixes(
    instances: &BTreeMap<ItemInstanceId, ItemInstance>,
    prototypes: &HashMap<String, ItemPrototype>,
) -> Result<(), WorldError> {
    for instance in instances.values() {
        let prototype = prototypes
            .get(&instance.prototype_id)
            .ok_or(WorldError::InvalidRuntimeSnapshot)?;
        if !instance.item_affixes.is_empty()
            && (prototype.stack_policy != ItemStackPolicy::Unique
                || prototype.max_stack_units != 1
                || prototype.sealed_loot_table_id.is_some())
        {
            return Err(WorldError::InvalidRuntimeSnapshot);
        }
        for applied in &instance.item_affixes {
            if (applied.attack_bonus_delta != 0 && prototype.equipment_slot.is_none())
                || (applied.heal_amount_delta != 0 && !prototype.consumable)
                || prototype
                    .attack_bonus
                    .checked_add(applied.attack_bonus_delta)
                    .is_none_or(|value| !(0..=MAX_STAT_VALUE_I32).contains(&value))
                || prototype
                    .heal_amount
                    .checked_add(applied.heal_amount_delta)
                    .is_none_or(|value| !(0..=MAX_STAT_VALUE_I32).contains(&value))
            {
                return Err(WorldError::InvalidRuntimeSnapshot);
            }
        }
    }
    Ok(())
}

pub(crate) fn validate_restored_sealed_items(
    instances: &BTreeMap<ItemInstanceId, ItemInstance>,
    provenance: &BTreeMap<ItemInstanceId, ItemProvenance>,
    ground_loot: &BTreeMap<ItemInstanceId, GroundLootReservation>,
    prototypes: &HashMap<String, ItemPrototype>,
) -> Result<(), WorldError> {
    for (item_id, instance) in instances {
        let prototype = prototypes
            .get(&instance.prototype_id)
            .ok_or(WorldError::InvalidRuntimeSnapshot)?;
        if prototype.sealed_loot_table_id.is_none() {
            continue;
        }
        let ItemProvenance::NpcLoot {
            drop_id,
            sealed_loot_table_id,
            ..
        } = provenance
            .get(item_id)
            .ok_or(WorldError::InvalidRuntimeSnapshot)?
        else {
            return Err(WorldError::InvalidRuntimeSnapshot);
        };
        if sealed_loot_table_id.as_ref() != prototype.sealed_loot_table_id.as_ref()
            || !instance.item_affixes.is_empty()
        {
            return Err(WorldError::InvalidRuntimeSnapshot);
        }
        match &instance.location {
            SharedItemLocation::Room(room_id) => {
                let reservation = ground_loot
                    .get(item_id)
                    .ok_or(WorldError::InvalidRuntimeSnapshot)?;
                if instance.binding != ItemBinding::Unbound
                    || reservation.root_item_id != *item_id
                    || reservation.room_id != *room_id
                    || reservation.drop_id != *drop_id
                {
                    return Err(WorldError::InvalidRuntimeSnapshot);
                }
            }
            SharedItemLocation::Carried(_) | SharedItemLocation::Contained(_) => {
                let owner = item_effective_owner(instances, item_id)
                    .map_err(|_| WorldError::InvalidRuntimeSnapshot)?;
                if !matches!(
                    (owner, instance.binding),
                    (Some(owner), ItemBinding::Character { character_id }) if owner == character_id
                ) {
                    return Err(WorldError::InvalidRuntimeSnapshot);
                }
            }
            SharedItemLocation::Consumed => {
                if !matches!(
                    instance.binding,
                    ItemBinding::Character { character_id } if character_id.0 > 0
                ) {
                    return Err(WorldError::InvalidRuntimeSnapshot);
                }
            }
            SharedItemLocation::Staged
            | SharedItemLocation::Equipped(_, _)
            | SharedItemLocation::HeldByNpc(_) => {
                return Err(WorldError::InvalidRuntimeSnapshot);
            }
        }
    }
    Ok(())
}

pub(crate) fn validate_container_capacities(
    instances: &BTreeMap<ItemInstanceId, ItemInstance>,
    prototypes: &HashMap<String, ItemPrototype>,
    graph: &ItemGraphIndex,
) -> Result<(), WorldError> {
    for (item_id, instance) in instances {
        let prototype = prototypes
            .get(&instance.prototype_id)
            .ok_or(WorldError::InvalidEntity)?;
        if let Some(capacity) = &prototype.container {
            let usage = container_usage(item_id, instances, prototypes, graph)?;
            if usage.used_slots > capacity.max_slots
                || usage.used_weight_grams > capacity.max_weight_grams
            {
                return Err(WorldError::InventoryCapacityExceeded);
            }
        }
    }
    Ok(())
}

pub(crate) fn compile_item_prototype(
    item: ItemPrototypeDefinition,
) -> Result<(String, ItemPrototype), WorldError> {
    let stack_valid = match item.stack_policy {
        ItemStackPolicy::Unique => item.max_stack_units == 1,
        ItemStackPolicy::Fungible => (2..=1_000).contains(&item.max_stack_units),
    };
    let container_valid = item.container.as_ref().is_none_or(|container| {
        item.stack_policy == ItemStackPolicy::Unique
            && item.max_stack_units == 1
            && !item.consumable
            && (1..=1_000).contains(&container.max_slots)
            && container.max_weight_grams <= 1_000_000
    });
    let presentation_valid = valid_definition_id(&item.prototype_id)
        && safe_definition_text(&item.name, 128)
        && safe_definition_multiline_text(&item.description, 2_048)
        && item.aliases.len() <= 32
        && item
            .aliases
            .iter()
            .all(|alias| safe_definition_text(alias, 64));
    let sealed_valid = item.sealed_loot_table_id.as_ref().is_none_or(|table_id| {
        valid_definition_id(table_id)
            && item.stack_policy == ItemStackPolicy::Unique
            && item.max_stack_units == 1
            && item.consumable
            && item.container.is_none()
            && item.equipment_slot.is_none()
            && item.attack_bonus == 0
            && item.heal_amount == 0
            && item.effect.is_none()
            && item.transfer_policy == ItemTransferPolicy::CharacterBound
    });
    if !presentation_valid
        || item.weight_grams > 1_000_000
        || item.slot_cost == 0
        || item.slot_cost > 1_000
        || item.item_level == 0
        || item.item_level > 1_000
        || !stack_valid
        || !container_valid
        || !sealed_valid
    {
        return Err(WorldError::InvalidAreaLifecycle);
    }
    let prototype_id = item.prototype_id;
    Ok((
        prototype_id,
        ItemPrototype {
            name: item.name,
            description: item.description,
            aliases: item.aliases,
            equipment_slot: item.equipment_slot,
            attack_bonus: item.attack_bonus,
            heal_amount: item.heal_amount,
            consumable: item.consumable,
            effect: item.effect,
            weight_grams: item.weight_grams,
            slot_cost: item.slot_cost,
            rarity: item.rarity,
            item_level: item.item_level,
            stack_policy: item.stack_policy,
            max_stack_units: item.max_stack_units,
            container: item.container,
            transfer_policy: item.transfer_policy,
            sealed_loot_table_id: item.sealed_loot_table_id,
        },
    ))
}

pub(crate) fn resolve_item_query<'a>(
    instances: &BTreeMap<ItemInstanceId, ItemInstance>,
    prototypes: &HashMap<String, ItemPrototype>,
    candidates: impl Iterator<Item = &'a ItemInstanceId>,
    query: &str,
    observer_salt: [u8; 32],
) -> Result<ItemInstanceId, WorldError> {
    let query = query.trim().to_ascii_lowercase();
    let candidate_items = candidates
        .map(|item_id| {
            let instance = instances.get(item_id).ok_or(WorldError::InvalidEntity)?;
            let prototype = prototypes
                .get(&instance.prototype_id)
                .ok_or(WorldError::InvalidEntity)?;
            Ok((item_id, prototype))
        })
        .collect::<Result<Vec<_>, _>>()?;

    if let Some((item_id, _)) = candidate_items
        .iter()
        .find(|(item_id, _)| opaque_id(observer_salt, "item", item_id.as_str()) == query)
    {
        return Ok((*item_id).clone());
    }

    let exact = candidate_items
        .iter()
        .copied()
        .filter(|(_, item)| item_matches(item, &query, false))
        .collect::<Vec<_>>();
    if exact.len() == 1 {
        return Ok(exact[0].0.clone());
    }
    if exact.len() > 1 {
        return Err(ambiguous_item_error(query, &exact));
    }

    let prefix = candidate_items
        .iter()
        .copied()
        .filter(|(_, item)| item_matches(item, &query, true))
        .collect::<Vec<_>>();
    match prefix.as_slice() {
        [(item_id, _)] => Ok((*item_id).clone()),
        [] => Err(WorldError::ItemNotFound(query)),
        matches => Err(ambiguous_item_error(query, matches)),
    }
}

pub(crate) fn item_matches(item: &ItemPrototype, query: &str, prefix: bool) -> bool {
    let matches = |term: &str| {
        if prefix {
            term.to_ascii_lowercase().starts_with(query)
        } else {
            term.eq_ignore_ascii_case(query)
        }
    };
    matches(&item.name) || item.aliases.iter().any(|alias| matches(alias))
}

pub(crate) fn ambiguous_item_error(
    query: String,
    items: &[(&ItemInstanceId, &ItemPrototype)],
) -> WorldError {
    let mut names = items
        .iter()
        .map(|(_, item)| item.name.clone())
        .collect::<Vec<_>>();
    names.sort_unstable();
    names.dedup();
    WorldError::AmbiguousItem {
        query,
        matches: names.join(", "),
    }
}
