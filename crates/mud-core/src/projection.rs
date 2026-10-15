//! Perception-scoped projection: scenes, player state and event
//! narration with opaque identifiers.

use super::*;

impl World {
    pub fn scene_for(&self, session: SessionId) -> Result<SceneView, WorldError> {
        let current_room = room_checked(&self.entities, self.character(session)?.location)?;
        let character_id = self.character(session)?.id;
        let observer = self.observer(session)?;
        let visual_scene = self.can_receive_visual_scene(session)?;
        let item_graph = build_item_graph_index(&self.item_instances, &self.item_prototypes)?;
        let mut exits = Vec::new();
        for (direction, destination) in &current_room.exits {
            if !self.exit_visible_to(session, &current_room.stable_id, *direction)? {
                continue;
            }
            let target_view_id = match destination {
                RoomExit::Resident(destination) => {
                    let destination = room_checked(&self.entities, *destination)?;
                    opaque_id(observer.salt, "room", &destination.stable_id)
                }
                RoomExit::Deferred(_) => opaque_id(
                    observer.salt,
                    "deferred-exit",
                    &format!("{}:{}", current_room.stable_id, direction.as_str()),
                ),
            };
            exits.push(SceneExit {
                id: target_view_id,
                label: direction.as_str().to_owned(),
                command: direction.as_str().to_owned(),
            });
        }
        let visible_item_ids = self
            .item_instances
            .iter()
            .filter_map(|(item_id, instance)| match &instance.location {
                SharedItemLocation::Room(room_id) if room_id == &current_room.stable_id => {
                    Some(item_id)
                }
                SharedItemLocation::Staged
                | SharedItemLocation::Carried(_)
                | SharedItemLocation::Room(_)
                | SharedItemLocation::Equipped(_, _)
                | SharedItemLocation::HeldByNpc(_)
                | SharedItemLocation::Contained(_)
                | SharedItemLocation::Consumed => None,
            })
            .map(|item_id| {
                self.item_visible_to(session, item_id, &current_room.stable_id)
                    .map(|visible| visible.then_some(item_id))
            })
            .collect::<Result<Vec<_>, WorldError>>()?
            .into_iter()
            .flatten()
            .cloned()
            .collect::<Vec<_>>();
        let mut item_buckets = BTreeMap::<(u8, Option<u64>), Vec<ItemInstanceId>>::new();
        for item_id in visible_item_ids {
            let key = match self.scene_loot_access(character_id, &item_id)? {
                None => (0, None),
                Some(SceneLootAccessView {
                    access: LootAccessView::Yours,
                    expires_in_ticks,
                }) => (1, expires_in_ticks),
                Some(SceneLootAccessView {
                    access: LootAccessView::Reserved,
                    expires_in_ticks,
                }) => (2, expires_in_ticks),
            };
            item_buckets.entry(key).or_default().push(item_id);
        }
        let mut visible_groups = Vec::new();
        for ((access, expires_in_ticks), item_ids) in item_buckets {
            let loot = match access {
                0 => None,
                1 => Some(SceneLootAccessView {
                    access: LootAccessView::Yours,
                    expires_in_ticks,
                }),
                2 => Some(SceneLootAccessView {
                    access: LootAccessView::Reserved,
                    expires_in_ticks,
                }),
                _ => return Err(WorldError::InvalidEntity),
            };
            for group in
                derive_stack_groups(&self.item_instances, &self.item_prototypes, item_ids.iter())?
            {
                visible_groups.push((group, loot.clone()));
            }
        }
        let mut items = visible_groups
            .into_iter()
            .map(|(group, loot)| {
                let item = self
                    .item_prototypes
                    .get(&group.prototype_id)
                    .ok_or(WorldError::InvalidEntity)?;
                let selector = group.members.first().ok_or(WorldError::InvalidEntity)?;
                let view_item_id = opaque_id(observer.salt, "item", selector.as_str());
                let quantity =
                    u16::try_from(group.members.len()).map_err(|_| WorldError::InvalidEntity)?;
                let total_weight_grams =
                    group.members.iter().try_fold(0_u64, |total, item_id| {
                        total
                            .checked_add(item_graph.subtree_weight(item_id)?)
                            .ok_or(WorldError::InvalidEntity)
                    })?;
                let mut actions = vec![SceneAction {
                    label: "Examine".into(),
                    command: format!("examine {view_item_id}"),
                }];
                if !matches!(
                    loot.as_ref().map(|loot| loot.access),
                    Some(LootAccessView::Reserved)
                ) {
                    actions.push(SceneAction {
                        label: "Take".into(),
                        command: format!("take {view_item_id}"),
                    });
                }
                if matches!(
                    loot.as_ref().map(|loot| loot.access),
                    Some(LootAccessView::Yours)
                ) {
                    actions.push(SceneAction {
                        label: "Pass".into(),
                        command: format!("loot pass {view_item_id}"),
                    });
                }
                if item.container.is_some()
                    && !matches!(
                        loot.as_ref().map(|loot| loot.access),
                        Some(LootAccessView::Reserved)
                    )
                {
                    actions.push(SceneAction {
                        label: "Contents".into(),
                        command: format!("contents {view_item_id}"),
                    });
                }
                Ok(SceneItem {
                    id: view_item_id.clone(),
                    label: self.item_effective_name(selector)?,
                    description: item.description.clone(),
                    rarity: item.rarity.as_str().into(),
                    item_level: item.item_level,
                    weight_grams: item.weight_grams,
                    quantity,
                    stack_slots: stack_slots(item, group.members.len())?,
                    total_weight_grams,
                    transfer_policy: item.transfer_policy,
                    bound_to_you: false,
                    affix: self.item_affix_view(selector),
                    container_capacity: item
                        .container
                        .as_ref()
                        .map(|_| {
                            container_usage(
                                selector,
                                &self.item_instances,
                                &self.item_prototypes,
                                &item_graph,
                            )
                        })
                        .transpose()?,
                    loot,
                    actions,
                })
            })
            .collect::<Result<Vec<_>, WorldError>>()?;
        items.sort_by(|left, right| {
            left.label
                .cmp(&right.label)
                .then_with(|| left.id.cmp(&right.id))
        });
        let npcs = self
            .npcs
            .values()
            .filter(|npc| npc.alive() && npc.spawn_room_id == current_room.stable_id)
            .map(|npc| {
                self.npc_visible_to(session, &npc.stable_id, &current_room.stable_id)
                    .map(|visible| visible.then_some(npc))
            })
            .collect::<Result<Vec<_>, WorldError>>()?
            .into_iter()
            .flatten()
            .map(|npc| {
                let view_npc_id = opaque_id(observer.salt, "npc", &npc.stable_id);
                SceneNpc {
                    id: view_npc_id.clone(),
                    label: npc.name.clone(),
                    description: npc.description.clone(),
                    health: npc.health,
                    max_health: npc.max_health,
                    actions: vec![SceneAction {
                        label: "Attack".into(),
                        command: format!("attack {view_npc_id}"),
                    }],
                }
            })
            .collect();
        Ok(SceneView {
            view_id: opaque_id(observer.salt, "room", &current_room.stable_id),
            revision: observer.revision,
            title: current_room.title.clone(),
            prose: if visual_scene {
                current_room.prose.clone()
            } else {
                vec![
                    "Darkness or impaired vision conceals the room's visual details. No visible object, creature, or exit can be identified."
                        .into(),
                ]
            },
            image: visual_scene.then(|| current_room.image.clone()).flatten(),
            audio: self
                .can_receive_audio_scene(session)?
                .then(|| current_room.audio.clone())
                .flatten(),
            exits,
            items,
            npcs,
            weather: WeatherView {
                condition: weather_label(self.adventure.weather).into(),
                day: self.logical_tick / 24 + 1,
                hour: self.logical_tick % 24,
                next_change_in_ticks: self
                    .adventure
                    .next_weather_tick
                    .saturating_sub(self.logical_tick),
            },
        })
    }

    /// Return presentation-only joins for entities already visible in the
    /// observer's current semantic scene. It cannot discover hidden, dead,
    /// carried, equipped or out-of-room entities.
    #[doc(hidden)]
    pub fn visible_scene_entity_ids(
        &self,
        session: SessionId,
    ) -> Result<VisibleSceneEntityIds, WorldError> {
        let current_room = room_checked(&self.entities, self.character(session)?.location)?;
        let observer = self.observer(session)?;
        let visible_ids = self
            .item_instances
            .iter()
            .filter_map(|(stable_id, instance)| {
                matches!(&instance.location, SharedItemLocation::Room(room_id) if room_id == &current_room.stable_id)
                    .then_some(stable_id)
            })
            .map(|item_id| {
                self.item_visible_to(session, item_id, &current_room.stable_id)
                    .map(|visible| visible.then_some(item_id.clone()))
            })
            .collect::<Result<Vec<_>, WorldError>>()?
            .into_iter()
            .flatten()
            .collect::<Vec<_>>();
        let mut items = derive_stack_groups(
            &self.item_instances,
            &self.item_prototypes,
            visible_ids.iter(),
        )?
        .into_iter()
        .map(|group| {
            let first = group
                .members
                .first()
                .expect("derived stack groups are non-empty");
            VisibleSceneEntityId {
                stable_id: first.0.clone(),
                prototype_id: group.prototype_id,
                opaque_id: opaque_id(observer.salt, "item", first.as_str()),
            }
        })
        .collect::<Vec<_>>();
        let mut npcs = self
            .npcs
            .values()
            .filter(|npc| npc.alive() && npc.spawn_room_id == current_room.stable_id)
            .map(|npc| {
                self.npc_visible_to(session, &npc.stable_id, &current_room.stable_id)
                    .map(|visible| {
                        visible.then(|| VisibleSceneEntityId {
                            stable_id: npc.stable_id.clone(),
                            prototype_id: npc.stable_id.clone(),
                            opaque_id: opaque_id(observer.salt, "npc", &npc.stable_id),
                        })
                    })
            })
            .collect::<Result<Vec<_>, WorldError>>()?
            .into_iter()
            .flatten()
            .collect::<Vec<_>>();
        items.sort_by(|left, right| left.stable_id.cmp(&right.stable_id));
        npcs.sort_by(|left, right| left.stable_id.cmp(&right.stable_id));
        Ok(VisibleSceneEntityIds { items, npcs })
    }
}

impl World {
    /// Server-internal connected members of the caller's durable party. The
    /// result includes the caller and never exposes party IDs to transports.
    pub fn connected_party_members(
        &self,
        session: SessionId,
    ) -> Result<Vec<ConnectedCharacter>, WorldError> {
        let character = self.character(session)?;
        let party_id = self
            .party_for_member(character.id)
            .ok_or(WorldError::PartyUnavailable)?;
        let party = self
            .adventure
            .parties
            .get(&party_id)
            .ok_or(WorldError::PartyUnavailable)?;
        Ok(self
            .connected_characters()?
            .into_iter()
            .filter(|candidate| party.members.contains(&candidate.character_id))
            .collect())
    }

    pub fn current_scene(&self, session: SessionId) -> Result<SceneView, WorldError> {
        self.scene_for(session)
    }

    /// Produce the complete browser/Telnet-neutral player projection. Durable
    /// account, character, room, item and party IDs never cross this boundary.
    pub fn player_state_view(&self, session: SessionId) -> Result<PlayerStateView, WorldError> {
        let character = self.character(session)?;
        let observer = self.observer(session)?;
        let current_room = room_checked(&self.entities, character.location)?;
        let gameplay = self
            .character_gameplay
            .get(&character.id)
            .ok_or(WorldError::InvalidEntity)?;
        let item_graph = build_item_graph_index(&self.item_instances, &self.item_prototypes)?;

        let carried_item_ids = self
            .item_instances
            .iter()
            .filter_map(|(item_id, instance)| {
                matches!(&instance.location, SharedItemLocation::Carried(owner) if *owner == character.id)
                    .then_some(item_id)
            })
            .cloned()
            .collect::<Vec<_>>();
        let reserved_items = self.reserved_item_ids()?;
        let groups = derive_stack_groups(
            &self.item_instances,
            &self.item_prototypes,
            carried_item_ids.iter(),
        )?;
        let mut split_groups = Vec::new();
        for group in groups {
            let (reserved, available): (Vec<_>, Vec<_>) = group
                .members
                .into_iter()
                .partition(|item_id| reserved_items.contains(item_id));
            if !available.is_empty() {
                split_groups.push((
                    ItemStackGroup {
                        prototype_id: group.prototype_id.clone(),
                        binding: group.binding,
                        members: available,
                    },
                    false,
                ));
            }
            if !reserved.is_empty() {
                split_groups.push((
                    ItemStackGroup {
                        prototype_id: group.prototype_id,
                        binding: group.binding,
                        members: reserved,
                    },
                    true,
                ));
            }
        }
        let mut inventory = split_groups
            .into_iter()
            .map(|(group, reserved_group)| {
                let item = self
                    .item_prototypes
                    .get(&group.prototype_id)
                    .ok_or(WorldError::InvalidEntity)?;
                let selector = group.members.first().ok_or(WorldError::InvalidEntity)?;
                let id = opaque_id(observer.salt, "item", selector.as_str());
                let mut actions = vec![SceneAction {
                    label: "Examine".into(),
                    command: format!("examine {id}"),
                }];
                let transferable = group
                    .members
                    .iter()
                    .map(|item_id| item_graph.subtree_is_transferable(item_id))
                    .collect::<Result<Vec<_>, WorldError>>()?
                    .into_iter()
                    .all(|allowed| allowed);
                let (available_quantity, reserved_for_trade) = if reserved_group {
                    (0, group.members.len())
                } else {
                    (group.members.len(), 0)
                };
                if transferable && !reserved_group {
                    actions.push(SceneAction {
                        label: "Drop".into(),
                        command: format!("drop {id}"),
                    });
                }
                {
                    if !reserved_group
                        && (self.item_effective_heal_amount(selector)? != 0
                            || item.effect.is_some())
                    {
                        actions.push(SceneAction {
                            label: "Use".into(),
                            command: format!("use {id}"),
                        });
                    }
                    if !reserved_group && item.equipment_slot.is_some() {
                        actions.push(SceneAction {
                            label: "Equip".into(),
                            command: format!("equip {id}"),
                        });
                    }
                    if !reserved_group && self.item_is_openable_reward(selector, character.id) {
                        actions.push(SceneAction {
                            label: "Abrir".into(),
                            command: format!("open {id}"),
                        });
                        actions.push(SceneAction {
                            label: "Probabilidades".into(),
                            command: format!("reward odds {id} 1"),
                        });
                    }
                }
                Ok(PlayerItemView {
                    id,
                    label: self.item_effective_name(selector)?,
                    description: item.description.clone(),
                    rarity: item.rarity.as_str().into(),
                    item_level: item.item_level,
                    weight_grams: item.weight_grams,
                    quantity: u16::try_from(group.members.len())
                        .map_err(|_| WorldError::InvalidEntity)?,
                    available_quantity: u16::try_from(available_quantity)
                        .map_err(|_| WorldError::InvalidEntity)?,
                    reserved_for_trade: u16::try_from(reserved_for_trade)
                        .map_err(|_| WorldError::InvalidEntity)?,
                    stack_slots: stack_slots(item, group.members.len())?,
                    total_weight_grams: group.members.iter().try_fold(
                        0_u64,
                        |total, item_id| {
                            total
                                .checked_add(item_graph.subtree_weight(item_id)?)
                                .ok_or(WorldError::InvalidEntity)
                        },
                    )?,
                    transfer_policy: item.transfer_policy,
                    bound_to_you: matches!(
                        group.binding,
                        ItemBinding::Character {
                            character_id: owner
                        } if owner == character.id
                    ),
                    can_give: transferable && reserved_for_trade == 0,
                    can_trade: transferable && !reserved_group,
                    affix: self.item_affix_view(selector),
                    container_capacity: item
                        .container
                        .as_ref()
                        .map(|_| {
                            container_usage(
                                selector,
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
        inventory.sort_by(|left, right| left.label.cmp(&right.label));

        let equipment = gameplay
            .equipment
            .iter()
            .map(|(slot, item_id)| {
                let item = self.item_prototype_for(item_id)?;
                Ok(EquippedItemView {
                    id: opaque_id(observer.salt, "item", item_id.as_str()),
                    slot: slot.as_str().into(),
                    label: self.item_effective_name(item_id)?,
                    rarity: item.rarity.as_str().into(),
                    item_level: item.item_level,
                    weight_grams: item.weight_grams,
                    quantity: 1,
                    stack_slots: stack_slots(item, 1)?,
                    total_weight_grams: item_graph.subtree_weight(item_id)?,
                    transfer_policy: item.transfer_policy,
                    bound_to_you: matches!(
                        self.item_instances
                            .get(item_id)
                            .ok_or(WorldError::InvalidEntity)?
                            .binding,
                        ItemBinding::Character {
                            character_id: owner
                        } if owner == character.id
                    ),
                    affix: self.item_affix_view(item_id),
                    container_capacity: item
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
                        .transpose()?,
                    actions: vec![SceneAction {
                        label: "Unequip".into(),
                        command: format!("unequip {}", slot.as_str()),
                    }],
                })
            })
            .collect::<Result<Vec<_>, WorldError>>()?;

        let mut rooms = observer
            .discovered_rooms
            .iter()
            .map(|room_id| {
                let entity = self
                    .stable_rooms
                    .get(room_id)
                    .copied()
                    .ok_or(WorldError::InvalidEntity)?;
                let room = room_checked(&self.entities, entity)?;
                Ok(MapRoomView {
                    id: opaque_id(observer.salt, "room", room_id),
                    title: room.title.clone(),
                    current: room.stable_id == current_room.stable_id,
                })
            })
            .collect::<Result<Vec<_>, WorldError>>()?;
        rooms.sort_by(|left, right| left.title.cmp(&right.title));
        let mut paths = Vec::new();
        for room_id in &observer.discovered_rooms {
            let entity = self
                .stable_rooms
                .get(room_id)
                .copied()
                .ok_or(WorldError::InvalidEntity)?;
            let room = room_checked(&self.entities, entity)?;
            for (direction, destination) in &room.exits {
                if !self.exit_visible_to(session, room_id, *direction)? {
                    continue;
                }
                let destination = destination
                    .resident_target()
                    .map(|destination| room_checked(&self.entities, destination))
                    .transpose()?;
                paths.push(MapPathView {
                    from: opaque_id(observer.salt, "room", room_id),
                    direction: direction.as_str().into(),
                    to: destination
                        .filter(|destination| {
                            observer.discovered_rooms.contains(&destination.stable_id)
                        })
                        .map(|destination| {
                            opaque_id(observer.salt, "room", &destination.stable_id)
                        }),
                });
            }
        }
        paths.sort_by(|left, right| {
            (&left.from, &left.direction, &left.to).cmp(&(&right.from, &right.direction, &right.to))
        });

        let (quest_status, quest_current, quest_target, quest_actions) =
            match gameplay.quests.get("first-hunt") {
                None => (
                    "available",
                    0,
                    1,
                    vec![SceneAction {
                        label: "Accept".into(),
                        command: "quest accept first-hunt".into(),
                    }],
                ),
                Some(progress) => {
                    let (status, actions) = match progress.status {
                        QuestStatus::Active => ("active", Vec::new()),
                        QuestStatus::ReadyToClaim => (
                            "readyToClaim",
                            vec![SceneAction {
                                label: "Claim reward".into(),
                                command: "quest complete first-hunt".into(),
                            }],
                        ),
                        QuestStatus::Completed => ("completed", Vec::new()),
                    };
                    (status, progress.current, progress.target, actions)
                }
            };
        let quests = vec![QuestView {
            id: "first-hunt".into(),
            label: "First Hunt".into(),
            summary: "Defeat one hostile creature; reward 25 coin and 50 experience.".into(),
            status: quest_status.into(),
            current: quest_current,
            target: quest_target,
            actions: quest_actions,
        }];

        let party = self.party_for_member(character.id).map(|party_id| {
            let party = &self.adventure.parties[&party_id];
            let is_leader = party.leader == character.id;
            let members = party
                .members
                .iter()
                .map(|member| PartyMemberView {
                    id: opaque_id(observer.salt, "character", &member.0.to_string()),
                    name: self
                        .connected_character_name(*member)
                        .unwrap_or_else(|| "Absent companion".into()),
                    leader: *member == party.leader,
                    online: self.session_for_character(*member).is_some(),
                })
                .collect();
            let mut actions = vec![SceneAction {
                label: "Leave party".into(),
                command: "party leave".into(),
            }];
            if is_leader {
                actions.push(SceneAction {
                    label: "Disband party".into(),
                    command: "party disband".into(),
                });
            }
            PartyView {
                leader: is_leader,
                members,
                actions,
            }
        });
        let mut party_invites = self
            .adventure
            .parties
            .values()
            .filter_map(|party| {
                let expiry = party.invitations.get(&character.id)?;
                (*expiry >= self.logical_tick).then_some((party.leader, *expiry))
            })
            .filter_map(|(leader, expiry)| {
                self.connected_character_name(leader)
                    .map(|name| (name, expiry))
            })
            .map(|(leader_name, expiry)| PartyInviteView {
                actions: vec![SceneAction {
                    label: "Accept invitation".into(),
                    command: format!("party accept {leader_name}"),
                }],
                leader_name,
                expires_in_ticks: expiry.saturating_sub(self.logical_tick),
            })
            .collect::<Vec<_>>();
        party_invites.sort_by(|left, right| left.leader_name.cmp(&right.leader_name));

        let commodities = commodity_specs()
            .iter()
            .filter_map(|(id, label, _, _)| {
                let quantity = gameplay.commodities.get(*id).copied().unwrap_or(0);
                (quantity > 0).then(|| CommodityView {
                    id: (*id).into(),
                    label: (*label).into(),
                    quantity,
                })
            })
            .collect();
        let shop_available = current_room.stable_id == self.start_room_stable_id;
        let spendable_coins = self.spendable_coins(character.id)?;
        let shop = if shop_available {
            commodity_specs()
                .iter()
                .map(|(id, label, buy_price, sell_price)| {
                    let stock = self.adventure.shop_stock.get(*id).copied().unwrap_or(0);
                    let owned = gameplay.commodities.get(*id).copied().unwrap_or(0);
                    let mut actions = Vec::new();
                    if stock > 0 && spendable_coins >= *buy_price && owned < 99 {
                        actions.push(SceneAction {
                            label: "Buy".into(),
                            command: format!("buy {id}"),
                        });
                    }
                    if owned > 0 {
                        actions.push(SceneAction {
                            label: "Sell".into(),
                            command: format!("sell {id}"),
                        });
                    }
                    ShopItemView {
                        id: (*id).into(),
                        label: (*label).into(),
                        buy_price: *buy_price,
                        sell_price: *sell_price,
                        stock,
                        owned,
                        actions,
                    }
                })
                .collect()
        } else {
            Vec::new()
        };

        let mut online = self
            .connected_characters()?
            .into_iter()
            .map(|connected| OnlineCharacterView {
                id: opaque_id(
                    observer.salt,
                    "character",
                    &connected.character_id.0.to_string(),
                ),
                name: connected.name,
                self_character: connected.character_id == character.id,
                nearby: connected.room_id == current_room.stable_id,
            })
            .collect::<Vec<_>>();
        online.sort_by(|left, right| {
            left.name
                .to_ascii_lowercase()
                .cmp(&right.name.to_ascii_lowercase())
        });

        Ok(PlayerStateView {
            revision: observer.revision,
            character: PlayerCharacterView {
                name: character.name.clone(),
                lineage: self.progression.lineage_view(&gameplay.lineage_id),
                background: gameplay
                    .background_id
                    .as_ref()
                    .map(|id| self.progression.background_view(id)),
                base_class: self.progression.class_view(&gameplay.base_class_id),
                active_class: self.progression.class_view(&gameplay.active_class_id),
                class_history: gameplay
                    .class_history
                    .iter()
                    .map(|id| self.progression.class_view(id))
                    .collect(),
                level: gameplay.level,
                experience: gameplay.experience,
                next_level_experience: self.progression.next_level_experience(gameplay.level),
                health: gameplay.vitals.health,
                max_health: gameplay.vitals.max_health,
                strength: gameplay.attributes.strength,
                defence: gameplay.attributes.defence,
                attributes: std::iter::once(AttributeView {
                    id: "strength".into(),
                    label: "Strength".into(),
                    value: gameplay.attributes.strength,
                })
                .chain(std::iter::once(AttributeView {
                    id: "defence".into(),
                    label: "Defence".into(),
                    value: gameplay.attributes.defence,
                }))
                .chain(
                    gameplay
                        .attributes
                        .additional
                        .iter()
                        .map(|(id, value)| AttributeView {
                            id: id.clone(),
                            label: humanize_id(id),
                            value: *value,
                        }),
                )
                .collect(),
                skills: gameplay
                    .skills
                    .iter()
                    .map(|(id, rank)| SkillView {
                        id: id.clone(),
                        label: skill_label(id),
                        rank: *rank,
                    })
                    .collect(),
                resources: gameplay
                    .resources
                    .iter()
                    .map(|(id, pool)| ResourceView {
                        id: id.clone(),
                        label: humanize_id(id),
                        current: pool.current,
                        max: pool.max,
                    })
                    .collect(),
                abilities: gameplay
                    .ability_ids
                    .iter()
                    .filter_map(|id| self.abilities.get(id))
                    .map(|ability| {
                        let cooldown_remaining = gameplay
                            .ability_cooldowns
                            .get(&ability.id)
                            .copied()
                            .unwrap_or(0)
                            .saturating_sub(self.logical_tick)
                            .max(
                                gameplay
                                    .ability_global_ready_tick
                                    .saturating_sub(self.logical_tick),
                            );
                        let has_resources = ability.resource_costs.iter().all(|(id, cost)| {
                            gameplay
                                .resources
                                .get(id)
                                .is_some_and(|pool| pool.current >= *cost)
                        });
                        let purpose = if ability.communication.is_some() {
                            AbilityPurpose::Telepathy
                        } else {
                            AbilityPurpose::Gameplay
                        };
                        let communication_ready =
                            ability.communication.as_ref().is_none_or(|communication| {
                                !communication
                                    .blocked_room_ids
                                    .contains(&current_room.stable_id)
                            });
                        AbilityView {
                            id: ability.id.clone(),
                            name: ability.name.clone(),
                            description: ability.description.clone(),
                            kind: ability.category.as_str().into(),
                            purpose,
                            resource_costs: ability
                                .resource_costs
                                .iter()
                                .map(|(id, amount)| AbilityResourceCostView {
                                    resource_id: id.clone(),
                                    label: humanize_id(id),
                                    amount: *amount,
                                })
                                .collect(),
                            cooldown_remaining,
                            usable: cooldown_remaining == 0
                                && has_resources
                                && communication_ready
                                && gameplay.vitals.health > 0
                                && gameplay.recovery_at_tick.is_none()
                                && !gameplay
                                    .statuses
                                    .iter()
                                    .any(|status| status.kind == ActiveStatusKind::Stun),
                            targeting: ability.targeting.as_str().into(),
                        }
                    })
                    .collect(),
                resistances: gameplay
                    .resistances
                    .iter()
                    .map(|(damage_type, percent)| ResistanceView {
                        damage_type: *damage_type,
                        percent: *percent,
                    })
                    .collect(),
                statuses: gameplay
                    .statuses
                    .iter()
                    .map(|status| StatusView {
                        id: status.source_ability_id.clone(),
                        label: self
                            .abilities
                            .get(&status.source_ability_id)
                            .map(|ability| ability.name.clone())
                            .unwrap_or_else(|| humanize_id(&status.source_ability_id)),
                        kind: status.kind,
                        magnitude: status.magnitude,
                        remaining_ticks: status.remaining_ticks,
                    })
                    .collect(),
                progression_points: gameplay.progression_points.clone(),
                talents: self
                    .progression
                    .talent_views(gameplay, &self.world_progression_flags),
                promotions: self
                    .progression
                    .promotions(gameplay, &self.world_progression_flags),
                effects: gameplay.effects.clone(),
                recovery_in_ticks: gameplay
                    .recovery_at_tick
                    .map(|tick| tick.saturating_sub(self.logical_tick)),
            },
            inventory,
            equipment,
            inventory_capacity: self.inventory_usage(character.id)?,
            map: DiscoveredMapView {
                current_room_id: opaque_id(observer.salt, "room", &current_room.stable_id),
                rooms,
                paths,
            },
            quests,
            party,
            party_invites,
            economy: EconomyView {
                coins: gameplay.coins,
                reserved_coins: self.reserved_coins(character.id),
                spendable_coins: self.spendable_coins(character.id)?,
                commodities,
                shop_available,
                shop,
            },
            social: SocialView { online },
            trade: self.trade_view_for(session)?,
        })
    }
}

impl World {
    /// Projects an authoritative event according to one observer's current
    /// perception. Domain IDs are replaced with observer-specific opaque IDs.
    pub fn project_event(
        &self,
        session: SessionId,
        event: &DomainEvent,
    ) -> Result<Option<ViewEvent>, WorldError> {
        // The clock changes observer-visible state (weather countdowns, effects,
        // recovery timers), but is not itself player-facing narration.
        if matches!(
            event,
            DomainEvent::Gameplay {
                event: GameplayEvent::TickAdvanced { .. }
            }
        ) {
            return Ok(None);
        }
        if !self.perceives_event(session, event)? {
            return Ok(None);
        }
        let observer = self.observer(session)?;
        let viewer = self.character(session)?;
        let (kind, actor_id, target_id, text, item_id) = match event {
            DomainEvent::CharacterMoved {
                actor_id,
                actor_name,
                direction,
                from_room_id,
                ..
            } => {
                let text = if viewer.id == *actor_id {
                    format!("You go {}.", direction.as_str())
                } else {
                    let viewer_room = room_checked(&self.entities, viewer.location)?
                        .stable_id
                        .as_str();
                    if viewer_room == from_room_id {
                        format!("{actor_name} leaves {}.", direction.as_str())
                    } else {
                        format!("{actor_name} arrives.")
                    }
                };
                (
                    ViewEventKind::CharacterMoved,
                    opaque_id(observer.salt, "character", &actor_id.0.to_string()),
                    None,
                    text,
                    None,
                )
            }
            DomainEvent::CharacterRelocated {
                actor_id,
                actor_name,
                from_room_id,
                ..
            } => {
                let text = if viewer.id == *actor_id {
                    "You are relocated by staff authority.".to_owned()
                } else {
                    let viewer_room = room_checked(&self.entities, viewer.location)?
                        .stable_id
                        .as_str();
                    if viewer_room == from_room_id {
                        format!("{actor_name} vanishes.")
                    } else {
                        format!("{actor_name} appears.")
                    }
                };
                (
                    ViewEventKind::CharacterMoved,
                    opaque_id(observer.salt, "character", &actor_id.0.to_string()),
                    None,
                    text,
                    None,
                )
            }
            DomainEvent::ItemTaken {
                actor_id,
                actor_name,
                item_id,
                item_name,
                ..
            } => {
                let text = if viewer.id == *actor_id {
                    format!("You take the {item_name}.")
                } else {
                    format!("{actor_name} takes the {item_name}.")
                };
                (
                    ViewEventKind::ItemTaken,
                    opaque_id(observer.salt, "character", &actor_id.0.to_string()),
                    None,
                    text,
                    Some(opaque_id(observer.salt, "item", item_id.as_str())),
                )
            }
            DomainEvent::ItemsMoved {
                actor_id,
                actor_name,
                item_ids,
                item_name,
                container_id,
                operation,
                room_id,
            } => {
                let visible_item_ids = self
                    .perceived_moved_item_ids(session, *actor_id, item_ids, *operation, room_id)?;
                let actor_is_viewer = viewer.id == *actor_id;
                let quantity = if actor_is_viewer
                    || matches!(
                        operation,
                        ContainerMoveOperation::Take | ContainerMoveOperation::Drop
                    ) {
                    visible_item_ids.len()
                } else {
                    // Putting/getting is observable room narration, like
                    // equip/use, but another player never receives selectors
                    // for the actor's private inventory or container.
                    item_ids.len()
                };
                let text = match (viewer.id == *actor_id, operation) {
                    (true, ContainerMoveOperation::Take) => {
                        format!("You take {quantity} × {item_name}.")
                    }
                    (true, ContainerMoveOperation::Drop) => {
                        format!("You drop {quantity} × {item_name}.")
                    }
                    (true, ContainerMoveOperation::Put) => {
                        format!("You put {quantity} × {item_name} in the container.")
                    }
                    (true, ContainerMoveOperation::Get) => {
                        format!("You get {quantity} × {item_name} from the container.")
                    }
                    (false, ContainerMoveOperation::Take) => {
                        format!("{actor_name} takes {quantity} × {item_name}.")
                    }
                    (false, ContainerMoveOperation::Drop) => {
                        format!("{actor_name} drops {quantity} × {item_name}.")
                    }
                    (false, ContainerMoveOperation::Put) => {
                        format!("{actor_name} puts {quantity} × {item_name} in a container.")
                    }
                    (false, ContainerMoveOperation::Get) => {
                        format!("{actor_name} gets {quantity} × {item_name} from a container.")
                    }
                };
                (
                    match operation {
                        ContainerMoveOperation::Take => ViewEventKind::ItemTaken,
                        ContainerMoveOperation::Drop => ViewEventKind::ItemDropped,
                        ContainerMoveOperation::Put | ContainerMoveOperation::Get => {
                            ViewEventKind::Gameplay
                        }
                    },
                    opaque_id(observer.salt, "character", &actor_id.0.to_string()),
                    actor_is_viewer
                        .then(|| {
                            container_id.as_ref().map(|container_id| {
                                opaque_id(observer.salt, "item", container_id.as_str())
                            })
                        })
                        .flatten(),
                    text,
                    visible_item_ids
                        .first()
                        .map(|item_id| opaque_id(observer.salt, "item", item_id.as_str())),
                )
            }
            DomainEvent::ItemsGiven {
                giver_id,
                giver_name,
                recipient_id,
                recipient_name,
                item_ids,
                item_name,
                ..
            } => {
                let quantity = item_ids.len();
                let text = if viewer.id == *giver_id {
                    format!("You give {quantity} × {item_name} to {recipient_name}.")
                } else if viewer.id == *recipient_id {
                    format!("{giver_name} gives you {quantity} × {item_name}.")
                } else {
                    format!("{giver_name} gives {quantity} × {item_name} to {recipient_name}.")
                };
                (
                    ViewEventKind::ItemGiven,
                    opaque_id(observer.salt, "character", &giver_id.0.to_string()),
                    Some(opaque_id(
                        observer.salt,
                        "character",
                        &recipient_id.0.to_string(),
                    )),
                    text,
                    (viewer.id == *giver_id || viewer.id == *recipient_id)
                        .then(|| item_ids.first())
                        .flatten()
                        .map(|item_id| opaque_id(observer.salt, "item", item_id.as_str())),
                )
            }
            DomainEvent::TradeRequested { trade } => (
                ViewEventKind::TradeRequested,
                opaque_id(observer.salt, "character", &trade.requester.0.to_string()),
                Some(opaque_id(
                    observer.salt,
                    "character",
                    &trade.recipient.0.to_string(),
                )),
                if viewer.id == trade.recipient {
                    "You receive a trade request.".into()
                } else {
                    "Your trade request is pending.".into()
                },
                None,
            ),
            DomainEvent::TradeOpened { current, .. }
            | DomainEvent::TradeOfferSet { current, .. }
            | DomainEvent::TradeInvalidated { current, .. } => (
                ViewEventKind::TradeChanged,
                opaque_id(observer.salt, "character", &current.requester.0.to_string()),
                Some(opaque_id(
                    observer.salt,
                    "character",
                    &current.recipient.0.to_string(),
                )),
                "The private trade terms change.".into(),
                None,
            ),
            DomainEvent::TradeConfirmed { participant, .. } => (
                ViewEventKind::TradeChanged,
                opaque_id(observer.salt, "character", &participant.0.to_string()),
                None,
                if viewer.id == *participant {
                    "You confirm the current trade terms.".into()
                } else {
                    "The other participant confirms the current trade terms.".into()
                },
                None,
            ),
            DomainEvent::TradeCancelled { previous_trade, .. } => (
                ViewEventKind::TradeCancelled,
                opaque_id(
                    observer.salt,
                    "character",
                    &previous_trade.requester.0.to_string(),
                ),
                Some(opaque_id(
                    observer.salt,
                    "character",
                    &previous_trade.recipient.0.to_string(),
                )),
                "The trade is cancelled.".into(),
                None,
            ),
            DomainEvent::TradeCompleted { previous_trade } => (
                ViewEventKind::TradeCompleted,
                opaque_id(
                    observer.salt,
                    "character",
                    &previous_trade.requester.0.to_string(),
                ),
                Some(opaque_id(
                    observer.salt,
                    "character",
                    &previous_trade.recipient.0.to_string(),
                )),
                if previous_trade.includes(viewer.id) {
                    "Your trade completes atomically.".into()
                } else {
                    "Two characters complete a trade.".into()
                },
                None,
            ),
            DomainEvent::Gameplay { event } => {
                let (actor_id, target_id, text, item_id) =
                    project_gameplay_event(observer.salt, viewer.id, event);
                (ViewEventKind::Gameplay, actor_id, target_id, text, item_id)
            }
            DomainEvent::Adventure { event } => match event {
                AdventureEvent::PartyChanged {
                    character_id,
                    party_id,
                    summary,
                    ..
                } => (
                    ViewEventKind::Adventure,
                    opaque_id(observer.salt, "character", &character_id.0.to_string()),
                    Some(opaque_id(observer.salt, "party", &party_id.to_string())),
                    summary.clone(),
                    None,
                ),
                AdventureEvent::QuestChanged {
                    character_id,
                    summary,
                    ..
                } => (
                    ViewEventKind::Adventure,
                    opaque_id(observer.salt, "character", &character_id.0.to_string()),
                    None,
                    summary.clone(),
                    None,
                ),
                AdventureEvent::EconomyChanged {
                    character_id,
                    commodity_id,
                    summary,
                } => (
                    ViewEventKind::Adventure,
                    opaque_id(observer.salt, "character", &character_id.0.to_string()),
                    None,
                    summary.clone(),
                    commodity_id
                        .as_ref()
                        .map(|id| opaque_id(observer.salt, "commodity", id)),
                ),
                AdventureEvent::WeatherChanged { weather, .. } => (
                    ViewEventKind::Adventure,
                    opaque_id(observer.salt, "world", "weather"),
                    None,
                    format!("The weather turns {}.", weather_label(*weather)),
                    None,
                ),
            },
            DomainEvent::ItemDropped {
                actor_id,
                actor_name,
                item_id,
                item_name,
                ..
            } => {
                let text = if viewer.id == *actor_id {
                    format!("You drop the {item_name}.")
                } else {
                    format!("{actor_name} drops the {item_name}.")
                };
                (
                    ViewEventKind::ItemDropped,
                    opaque_id(observer.salt, "character", &actor_id.0.to_string()),
                    None,
                    text,
                    Some(opaque_id(observer.salt, "item", item_id.as_str())),
                )
            }
        };
        Ok(Some(ViewEvent {
            revision: observer.revision,
            kind,
            text,
            actor_id,
            target_id,
            item_id,
        }))
    }

    /// Build a perception-safe narration event addressed to exactly one
    /// authorized session. The caller must keep the resulting event on that
    /// session's targeted delivery; it is not a domain event and must never be
    /// broadcast or persisted.
    pub fn project_private_narration(
        &self,
        session: SessionId,
        text: &str,
    ) -> Result<ViewEvent, WorldError> {
        let text = text.trim();
        if text.is_empty()
            || text.len() > 1_024
            || text
                .chars()
                .any(|character| character.is_control() && character != '\n')
        {
            return Err(WorldError::InvalidScriptEffect);
        }
        let observer = self.observer(session)?;
        let viewer = self.character(session)?;
        Ok(ViewEvent {
            revision: observer.revision,
            kind: ViewEventKind::Gameplay,
            text: text.to_owned(),
            actor_id: opaque_id(observer.salt, "character", &viewer.id.0.to_string()),
            target_id: None,
            item_id: None,
        })
    }

    pub(crate) fn bump_view_revisions(&mut self, event: &DomainEvent) -> Result<(), WorldError> {
        let affected = self
            .connected_sessions()
            .into_iter()
            .map(|session| Ok((session, self.perceives_event(session, event)?)))
            .collect::<Result<Vec<_>, WorldError>>()?;
        let mut previous = Vec::new();
        for (session, perceived) in affected {
            if perceived {
                let view = self
                    .observer_views
                    .get_mut(&session)
                    .ok_or(WorldError::InvalidEntity)?;
                previous.push((session, view.revision));
                view.revision = view.revision.wrapping_add(1).max(1);
            }
        }
        self.last_view_bump = Some((self.revision, previous));
        Ok(())
    }

    pub(crate) fn bump_view_revisions_for_events(
        &mut self,
        events: &[DomainEvent],
    ) -> Result<(), WorldError> {
        let affected = self
            .connected_sessions()
            .into_iter()
            .map(|session| {
                for event in events {
                    if self.perceives_event(session, event)? {
                        return Ok((session, true));
                    }
                }
                Ok((session, false))
            })
            .collect::<Result<Vec<_>, WorldError>>()?;
        let mut previous = Vec::new();
        for (session, perceived) in affected {
            if perceived {
                let view = self
                    .observer_views
                    .get_mut(&session)
                    .ok_or(WorldError::InvalidEntity)?;
                previous.push((session, view.revision));
                view.revision = view.revision.wrapping_add(1).max(1);
            }
        }
        self.last_view_bump = Some((self.revision, previous));
        Ok(())
    }

    pub(crate) fn restore_last_view_bump(&mut self) {
        let matches_current_revision = self
            .last_view_bump
            .as_ref()
            .is_some_and(|(resulting_revision, _)| *resulting_revision == self.revision);
        if !matches_current_revision {
            return;
        }
        if let Some((_, previous_views)) = self.last_view_bump.take() {
            for (affected_session, previous_revision) in previous_views {
                if let Some(view) = self.observer_views.get_mut(&affected_session) {
                    view.revision = previous_revision;
                }
            }
        }
    }
}

impl World {
    pub(crate) fn perceived_moved_item_ids(
        &self,
        session: SessionId,
        actor_id: CharacterId,
        item_ids: &[ItemInstanceId],
        operation: ContainerMoveOperation,
        room_id: &str,
    ) -> Result<Vec<ItemInstanceId>, WorldError> {
        let viewer = self.character(session)?;
        if viewer.id == actor_id {
            return Ok(item_ids.to_vec());
        }
        let viewer_room = room_checked(&self.entities, viewer.location)?;
        if viewer_room.stable_id != room_id {
            return Ok(Vec::new());
        }
        match operation {
            ContainerMoveOperation::Take | ContainerMoveOperation::Drop => item_ids
                .iter()
                .map(|item_id| {
                    self.item_visible_to(session, item_id, room_id)
                        .map(|visible| visible.then_some(item_id.clone()))
                })
                .collect::<Result<Vec<_>, WorldError>>()
                .map(|items| items.into_iter().flatten().collect()),
            ContainerMoveOperation::Put | ContainerMoveOperation::Get => Ok(Vec::new()),
        }
    }

    pub(crate) fn perceives_event(
        &self,
        session: SessionId,
        event: &DomainEvent,
    ) -> Result<bool, WorldError> {
        let viewer = self.character(session)?;
        let viewer_room = room_checked(&self.entities, viewer.location)?
            .stable_id
            .as_str();
        let perceived = match event {
            DomainEvent::CharacterMoved {
                actor_id,
                from_room_id,
                to_room_id,
                direction,
                ..
            } => {
                viewer.id == *actor_id
                    || viewer_room == to_room_id
                    || (viewer_room == from_room_id
                        && self.exit_visible_to(session, from_room_id, *direction)?)
            }
            DomainEvent::CharacterRelocated {
                actor_id,
                from_room_id,
                to_room_id,
                ..
            } => viewer.id == *actor_id || viewer_room == from_room_id || viewer_room == to_room_id,
            DomainEvent::ItemTaken {
                actor_id,
                item_id,
                room_id,
                ..
            }
            | DomainEvent::ItemDropped {
                actor_id,
                item_id,
                room_id,
                ..
            } => {
                viewer.id == *actor_id
                    || (viewer_room == room_id
                        && self.item_visible_to(session, item_id, room_id)?)
            }
            DomainEvent::ItemsMoved {
                actor_id,
                item_ids,
                operation,
                room_id,
                ..
            } => match operation {
                ContainerMoveOperation::Take | ContainerMoveOperation::Drop => !self
                    .perceived_moved_item_ids(session, *actor_id, item_ids, *operation, room_id)?
                    .is_empty(),
                ContainerMoveOperation::Put | ContainerMoveOperation::Get => {
                    viewer.id == *actor_id || viewer_room == room_id
                }
            },
            DomainEvent::ItemsGiven {
                giver_id,
                recipient_id,
                room_id,
                ..
            } => viewer.id == *giver_id || viewer.id == *recipient_id || viewer_room == room_id,
            DomainEvent::TradeRequested { trade } => trade.includes(viewer.id),
            DomainEvent::TradeOpened { current, .. }
            | DomainEvent::TradeOfferSet { current, .. }
            | DomainEvent::TradeInvalidated { current, .. } => current.includes(viewer.id),
            DomainEvent::TradeConfirmed { participant, .. } => self
                .trade_for_character(*participant)
                .is_some_and(|trade| trade.includes(viewer.id)),
            DomainEvent::TradeCancelled { previous_trade, .. } => {
                previous_trade.includes(viewer.id)
            }
            DomainEvent::TradeCompleted { previous_trade } => {
                previous_trade.includes(viewer.id) || viewer_room == previous_trade.room_id
            }
            DomainEvent::Gameplay { event } => {
                let base = matches!(event, GameplayEvent::TickAdvanced { .. })
                    || (gameplay_event_character(event).is_some_and(|id| id == viewer.id)
                        || gameplay_event_room(event).is_some_and(|room| room == viewer_room));
                let npc_visible = match (gameplay_event_npc(event), gameplay_event_room(event)) {
                    (Some(npc_id), Some(room_id))
                        if gameplay_event_character(event) != Some(viewer.id) =>
                    {
                        self.npc_visible_to(session, npc_id, room_id)?
                    }
                    _ => true,
                };
                let item_visible = match (gameplay_event_item(event), gameplay_event_room(event)) {
                    (Some(item_id), Some(room_id))
                        if gameplay_event_character(event) != Some(viewer.id) =>
                    {
                        self.item_visible_to(session, item_id, room_id)?
                    }
                    _ => true,
                };
                base && npc_visible && item_visible
            }
            DomainEvent::Adventure { event } => match event {
                AdventureEvent::PartyChanged {
                    character_id,
                    audience,
                    ..
                } => *character_id == viewer.id || audience.contains(&viewer.id),
                AdventureEvent::QuestChanged { character_id, .. }
                | AdventureEvent::EconomyChanged { character_id, .. } => *character_id == viewer.id,
                AdventureEvent::WeatherChanged { .. } => true,
            },
        };
        Ok(perceived)
    }
}

pub(crate) fn gameplay_event_room(event: &GameplayEvent) -> Option<&str> {
    match event {
        GameplayEvent::TickAdvanced { .. } => None,
        GameplayEvent::NpcSpawned { room_id, .. }
        | GameplayEvent::CombatStarted { room_id, .. }
        | GameplayEvent::AbilityUsed { room_id, .. }
        | GameplayEvent::NpcAbilityUsed { room_id, .. }
        | GameplayEvent::NpcDamaged { room_id, .. }
        | GameplayEvent::CharacterDamaged { room_id, .. }
        | GameplayEvent::NpcDefeated { room_id, .. }
        | GameplayEvent::LootDropped { room_id, .. }
        | GameplayEvent::LootGenerated { room_id, .. }
        | GameplayEvent::LootRightsChanged { room_id, .. }
        | GameplayEvent::LootGenerationSuppressed { room_id, .. }
        | GameplayEvent::LootDespawned { room_id, .. }
        | GameplayEvent::LootEncounterCancelled { room_id, .. }
        | GameplayEvent::ExperienceGained { room_id, .. }
        | GameplayEvent::CharacterLeveled { room_id, .. }
        | GameplayEvent::CharacterPromoted { room_id, .. }
        | GameplayEvent::TalentRankAcquired { room_id, .. }
        | GameplayEvent::CharacterDefeated { room_id, .. }
        | GameplayEvent::CharacterCollapsed { room_id, .. }
        | GameplayEvent::CharacterRecovered { room_id, .. }
        | GameplayEvent::NpcRespawned { room_id, .. }
        | GameplayEvent::ItemEquipped { room_id, .. }
        | GameplayEvent::ItemUnequipped { room_id, .. }
        | GameplayEvent::ItemUsed { room_id, .. }
        | GameplayEvent::RewardOpened { room_id, .. }
        | GameplayEvent::CharacterHealed { room_id, .. }
        | GameplayEvent::CharacterRestored { room_id, .. }
        | GameplayEvent::EffectApplied { room_id, .. }
        | GameplayEvent::EffectExpired { room_id, .. } => Some(room_id),
    }
}

pub(crate) fn gameplay_event_character(event: &GameplayEvent) -> Option<CharacterId> {
    match event {
        GameplayEvent::CombatStarted { character_id, .. }
        | GameplayEvent::AbilityUsed { character_id, .. }
        | GameplayEvent::NpcAbilityUsed { character_id, .. }
        | GameplayEvent::NpcDamaged { character_id, .. }
        | GameplayEvent::CharacterDamaged { character_id, .. }
        | GameplayEvent::NpcDefeated { character_id, .. }
        | GameplayEvent::ExperienceGained { character_id, .. }
        | GameplayEvent::CharacterLeveled { character_id, .. }
        | GameplayEvent::CharacterPromoted { character_id, .. }
        | GameplayEvent::TalentRankAcquired { character_id, .. }
        | GameplayEvent::CharacterDefeated { character_id, .. }
        | GameplayEvent::CharacterCollapsed { character_id, .. }
        | GameplayEvent::CharacterRecovered { character_id, .. }
        | GameplayEvent::ItemEquipped { character_id, .. }
        | GameplayEvent::ItemUnequipped { character_id, .. }
        | GameplayEvent::ItemUsed { character_id, .. }
        | GameplayEvent::RewardOpened { character_id, .. }
        | GameplayEvent::CharacterHealed { character_id, .. }
        | GameplayEvent::CharacterRestored { character_id, .. }
        | GameplayEvent::EffectApplied { character_id, .. }
        | GameplayEvent::EffectExpired { character_id, .. } => Some(*character_id),
        GameplayEvent::TickAdvanced { .. }
        | GameplayEvent::NpcSpawned { .. }
        | GameplayEvent::LootDropped { .. }
        | GameplayEvent::LootGenerated { .. }
        | GameplayEvent::LootRightsChanged { .. }
        | GameplayEvent::LootGenerationSuppressed { .. }
        | GameplayEvent::LootDespawned { .. }
        | GameplayEvent::LootEncounterCancelled { .. }
        | GameplayEvent::NpcRespawned { .. } => None,
    }
}

pub(crate) fn gameplay_event_npc(event: &GameplayEvent) -> Option<&str> {
    match event {
        GameplayEvent::AbilityUsed { npc_id, .. } => npc_id.as_deref(),
        GameplayEvent::NpcSpawned { npc_id, .. }
        | GameplayEvent::NpcAbilityUsed { npc_id, .. }
        | GameplayEvent::CombatStarted { npc_id, .. }
        | GameplayEvent::NpcDamaged { npc_id, .. }
        | GameplayEvent::CharacterDamaged { npc_id, .. }
        | GameplayEvent::NpcDefeated { npc_id, .. }
        | GameplayEvent::LootDropped { npc_id, .. }
        | GameplayEvent::LootGenerated { npc_id, .. }
        | GameplayEvent::LootGenerationSuppressed { npc_id, .. }
        | GameplayEvent::LootEncounterCancelled { npc_id, .. }
        | GameplayEvent::CharacterDefeated { npc_id, .. }
        | GameplayEvent::NpcRespawned { npc_id, .. } => Some(npc_id),
        GameplayEvent::TickAdvanced { .. }
        | GameplayEvent::ExperienceGained { .. }
        | GameplayEvent::CharacterLeveled { .. }
        | GameplayEvent::CharacterPromoted { .. }
        | GameplayEvent::TalentRankAcquired { .. }
        | GameplayEvent::LootRightsChanged { .. }
        | GameplayEvent::LootDespawned { .. }
        | GameplayEvent::CharacterCollapsed { .. }
        | GameplayEvent::CharacterRecovered { .. }
        | GameplayEvent::ItemEquipped { .. }
        | GameplayEvent::ItemUnequipped { .. }
        | GameplayEvent::ItemUsed { .. }
        | GameplayEvent::RewardOpened { .. }
        | GameplayEvent::CharacterHealed { .. }
        | GameplayEvent::CharacterRestored { .. }
        | GameplayEvent::EffectApplied { .. }
        | GameplayEvent::EffectExpired { .. } => None,
    }
}

pub(crate) fn gameplay_event_item(event: &GameplayEvent) -> Option<&ItemInstanceId> {
    match event {
        GameplayEvent::LootDropped { item_id, .. }
        | GameplayEvent::LootRightsChanged { item_id, .. }
        | GameplayEvent::ItemEquipped { item_id, .. }
        | GameplayEvent::ItemUnequipped { item_id, .. }
        | GameplayEvent::ItemUsed { item_id, .. } => Some(item_id),
        GameplayEvent::RewardOpened { source_item_id, .. } => Some(source_item_id),
        GameplayEvent::TickAdvanced { .. }
        | GameplayEvent::NpcSpawned { .. }
        | GameplayEvent::CombatStarted { .. }
        | GameplayEvent::AbilityUsed { .. }
        | GameplayEvent::NpcAbilityUsed { .. }
        | GameplayEvent::NpcDamaged { .. }
        | GameplayEvent::CharacterDamaged { .. }
        | GameplayEvent::NpcDefeated { .. }
        | GameplayEvent::LootGenerated { .. }
        | GameplayEvent::LootGenerationSuppressed { .. }
        | GameplayEvent::LootDespawned { .. }
        | GameplayEvent::LootEncounterCancelled { .. }
        | GameplayEvent::ExperienceGained { .. }
        | GameplayEvent::CharacterLeveled { .. }
        | GameplayEvent::CharacterPromoted { .. }
        | GameplayEvent::TalentRankAcquired { .. }
        | GameplayEvent::CharacterDefeated { .. }
        | GameplayEvent::CharacterCollapsed { .. }
        | GameplayEvent::CharacterRecovered { .. }
        | GameplayEvent::NpcRespawned { .. }
        | GameplayEvent::CharacterHealed { .. }
        | GameplayEvent::CharacterRestored { .. }
        | GameplayEvent::EffectApplied { .. }
        | GameplayEvent::EffectExpired { .. } => None,
    }
}

pub(crate) fn project_gameplay_event(
    salt: [u8; 32],
    viewer_id: CharacterId,
    event: &GameplayEvent,
) -> (String, Option<String>, String, Option<String>) {
    let character_id = |id: CharacterId| opaque_id(salt, "character", &id.0.to_string());
    let npc_id = |id: &str| opaque_id(salt, "npc", id);
    let item_id = |id: &ItemInstanceId| opaque_id(salt, "item", id.as_str());
    match event {
        GameplayEvent::TickAdvanced { .. } => (
            opaque_id(salt, "world", "clock"),
            None,
            "Time advances.".into(),
            None,
        ),
        GameplayEvent::NpcSpawned {
            npc_id: npc,
            npc_name,
            ..
        }
        | GameplayEvent::NpcRespawned {
            npc_id: npc,
            npc_name,
            ..
        } => (npc_id(npc), None, format!("{npc_name} appears."), None),
        GameplayEvent::CombatStarted {
            character_id: character,
            character_name,
            npc_id: npc,
            npc_name,
            ..
        } => (
            character_id(*character),
            Some(npc_id(npc)),
            if viewer_id == *character {
                format!("You engage {npc_name}.")
            } else {
                format!("{character_name} engages {npc_name}.")
            },
            None,
        ),
        GameplayEvent::AbilityUsed {
            character_id: character,
            character_name,
            ability_name,
            npc_id: npc,
            npc_name,
            ..
        } => (
            character_id(*character),
            npc.as_deref().map(npc_id),
            match (viewer_id == *character, npc_name) {
                (true, Some(target)) => format!("You use {ability_name} on {target}."),
                (true, None) => format!("You use {ability_name}."),
                (false, Some(target)) => {
                    format!("{character_name} uses {ability_name} on {target}.")
                }
                (false, None) => format!("{character_name} uses {ability_name}."),
            },
            None,
        ),
        GameplayEvent::NpcAbilityUsed {
            npc_id: npc,
            npc_name,
            character_id: character,
            character_name,
            ability_name,
            ..
        } => (
            npc_id(npc),
            Some(character_id(*character)),
            if viewer_id == *character {
                format!("{npc_name} uses {ability_name} on you.")
            } else {
                format!("{npc_name} uses {ability_name} on {character_name}.")
            },
            None,
        ),
        GameplayEvent::NpcDamaged {
            character_id: character,
            character_name,
            npc_id: npc,
            npc_name,
            amount,
            remaining_health,
            ..
        } => (
            character_id(*character),
            Some(npc_id(npc)),
            if viewer_id == *character {
                format!("You hit {npc_name} for {amount} ({remaining_health} health).")
            } else {
                format!("{character_name} hits {npc_name} for {amount}.")
            },
            None,
        ),
        GameplayEvent::CharacterDamaged {
            character_id: character,
            character_name,
            npc_id: npc,
            npc_name,
            amount,
            remaining_health,
            ..
        } => (
            npc_id(npc),
            Some(character_id(*character)),
            if viewer_id == *character {
                format!("{npc_name} hits you for {amount} ({remaining_health} health).")
            } else {
                format!("{npc_name} hits {character_name} for {amount}.")
            },
            None,
        ),
        GameplayEvent::NpcDefeated {
            character_id: character,
            character_name,
            npc_id: npc,
            npc_name,
            ..
        } => (
            character_id(*character),
            Some(npc_id(npc)),
            if viewer_id == *character {
                format!("You defeat {npc_name}.")
            } else {
                format!("{character_name} defeats {npc_name}.")
            },
            None,
        ),
        GameplayEvent::LootDropped {
            npc_id: npc,
            npc_name,
            item_id: item,
            item_name,
            ..
        } => (
            npc_id(npc),
            None,
            format!("{npc_name} drops {item_name}."),
            Some(item_id(item)),
        ),
        GameplayEvent::LootGenerated {
            npc_id: npc,
            npc_name,
            item_ids,
            ..
        } => (
            npc_id(npc),
            None,
            format!(
                "{npc_name} yields {} generated loot unit(s).",
                item_ids.len()
            ),
            None,
        ),
        GameplayEvent::LootRightsChanged {
            item_id: item,
            summary,
            ..
        } => (item_id(item), None, summary.clone(), None),
        GameplayEvent::LootGenerationSuppressed {
            npc_id: npc,
            npc_name,
            reason,
            ..
        } => (
            npc_id(npc),
            None,
            format!("Loot from {npc_name} is suppressed: {reason}."),
            None,
        ),
        GameplayEvent::LootDespawned { item_ids, room_id } => (
            opaque_id(salt, "room", room_id),
            None,
            format!("{} unclaimed loot unit(s) fade away.", item_ids.len()),
            None,
        ),
        GameplayEvent::LootEncounterCancelled {
            npc_id: npc,
            npc_name,
            ..
        } => (
            npc_id(npc),
            None,
            format!("The loot encounter with {npc_name} is cancelled."),
            None,
        ),
        GameplayEvent::ExperienceGained {
            character_id: character,
            character_name,
            amount,
            ..
        } => (
            character_id(*character),
            None,
            if viewer_id == *character {
                format!("You gain {amount} experience.")
            } else {
                format!("{character_name} grows more experienced.")
            },
            None,
        ),
        GameplayEvent::CharacterLeveled {
            character_id: character,
            character_name,
            level,
            ..
        } => (
            character_id(*character),
            None,
            if viewer_id == *character {
                format!("You reach level {level}.")
            } else {
                format!("{character_name} grows in power.")
            },
            None,
        ),
        GameplayEvent::CharacterPromoted {
            character_id: character,
            character_name,
            to_class_id,
            ..
        } => (
            character_id(*character),
            None,
            if viewer_id == *character {
                format!("You advance to {}.", humanize_id(to_class_id))
            } else {
                format!("{character_name} advances to a new class.")
            },
            None,
        ),
        GameplayEvent::TalentRankAcquired {
            character_id: character,
            character_name,
            node_id,
            rank,
            ..
        } => (
            character_id(*character),
            None,
            if viewer_id == *character {
                format!("You acquire rank {rank} of {}.", humanize_id(node_id))
            } else {
                format!("{character_name} develops a new talent.")
            },
            None,
        ),
        GameplayEvent::CharacterDefeated {
            character_id: character,
            character_name,
            npc_id: npc,
            npc_name,
            ..
        } => (
            npc_id(npc),
            Some(character_id(*character)),
            if viewer_id == *character {
                format!("{npc_name} defeats you.")
            } else {
                format!("{npc_name} defeats {character_name}.")
            },
            None,
        ),
        GameplayEvent::CharacterCollapsed {
            character_id: character,
            character_name,
            ..
        } => (
            character_id(*character),
            None,
            if viewer_id == *character {
                "You collapse and lose consciousness.".into()
            } else {
                format!("{character_name} collapses.")
            },
            None,
        ),
        GameplayEvent::CharacterRecovered {
            character_id: character,
            character_name,
            ..
        } => (
            character_id(*character),
            None,
            if viewer_id == *character {
                "You recover at the sanctuary.".into()
            } else {
                format!("{character_name} recovers.")
            },
            None,
        ),
        GameplayEvent::ItemEquipped {
            character_id: character,
            character_name,
            item_id: item,
            item_name,
            ..
        }
        | GameplayEvent::ItemUnequipped {
            character_id: character,
            character_name,
            item_id: item,
            item_name,
            ..
        }
        | GameplayEvent::ItemUsed {
            character_id: character,
            character_name,
            item_id: item,
            item_name,
            ..
        } => (
            character_id(*character),
            None,
            if viewer_id == *character {
                format!("You handle {item_name}.")
            } else {
                format!("{character_name} handles {item_name}.")
            },
            Some(item_id(item)),
        ),
        GameplayEvent::RewardOpened {
            character_id: character,
            character_name,
            source_item_id,
            source_item_name,
            item_names,
            ..
        } => (
            character_id(*character),
            None,
            if viewer_id == *character {
                format!(
                    "Abres {source_item_name} y obtienes: {}.",
                    item_names.join(", ")
                )
            } else {
                format!("{character_name} abre {source_item_name}.")
            },
            Some(item_id(source_item_id)),
        ),
        GameplayEvent::CharacterHealed {
            character_id: character,
            character_name,
            amount,
            ..
        } => (
            character_id(*character),
            None,
            if viewer_id == *character {
                format!("You recover {amount} health.")
            } else {
                format!("{character_name} looks healthier.")
            },
            None,
        ),
        GameplayEvent::CharacterRestored {
            character_id: character,
            character_name,
            ..
        } => (
            character_id(*character),
            None,
            if viewer_id == *character {
                "You are fully restored by staff authority.".into()
            } else {
                format!("{character_name} is fully restored.")
            },
            None,
        ),
        GameplayEvent::EffectApplied {
            character_id: character,
            character_name,
            effect,
            ..
        } => (
            character_id(*character),
            None,
            if viewer_id == *character {
                format!("You gain the {:?} effect.", effect.kind)
            } else {
                format!("{character_name} is surrounded by a brief aura.")
            },
            None,
        ),
        GameplayEvent::EffectExpired {
            character_id: character,
            character_name,
            kind,
            ..
        } => (
            character_id(*character),
            None,
            if viewer_id == *character {
                format!("Your {kind:?} effect expires.")
            } else {
                format!("The aura around {character_name} fades.")
            },
            None,
        ),
    }
}

pub(crate) fn opaque_id(observer_salt: [u8; 32], kind: &str, domain_id: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(b"tiamud-view-v2");
    hasher.update(observer_salt);
    hasher.update((kind.len() as u64).to_be_bytes());
    hasher.update(kind.as_bytes());
    hasher.update((domain_id.len() as u64).to_be_bytes());
    hasher.update(domain_id.as_bytes());
    let digest = hasher.finalize();
    let mut prefix = [0_u8; 8];
    prefix.copy_from_slice(&digest[..8]);
    let token = u64::from_be_bytes(prefix);
    format!("o{token:016x}")
}

/// Whether `value` has the exact `o` + 16 lowercase hex shape produced by
/// [`opaque_id`]. Any other string can never equal an opaque selector.
pub(crate) fn is_opaque_id_shaped(value: &str) -> bool {
    let bytes = value.as_bytes();
    bytes.len() == 17
        && bytes[0] == b'o'
        && bytes[1..]
            .iter()
            .all(|byte| matches!(byte, b'0'..=b'9' | b'a'..=b'f'))
}
