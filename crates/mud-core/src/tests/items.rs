use super::*;

#[test]
fn restored_item_effects_enforce_numeric_caps() {
    let (mut world, session) = phase_two_world(1, 2, 10);
    let mut snapshot = world.character_gameplay_snapshot(session).unwrap();
    snapshot.effects.push(ActiveEffect {
        kind: EffectKind::Strength,
        magnitude: MAX_STATUS_MAGNITUDE + 1,
        remaining_ticks: 1,
    });

    assert!(matches!(
        world.restore_character_gameplay(CharacterId(900), snapshot),
        Err(WorldError::InvalidGameplayDefinition)
    ));
}

#[test]
fn legacy_character_item_state_is_ignored_and_scene_ids_are_opaque() {
    let (world, session) = connected_world();

    let snapshot = world.character_snapshot(session).unwrap();
    assert!(snapshot.item_locations.is_empty());
    let scene = world.scene_for(session).unwrap();
    assert_eq!(scene.items.len(), 1);
    assert_eq!(scene.items[0].label, "blue-glass lantern");
    assert_ne!(scene.items[0].id, "demo:blue-lantern");
    assert!(!scene.items[0].actions[0].command.contains("demo:"));
    assert!(
        scene
            .as_plain_text()
            .contains("You see here: blue-glass lantern.")
    );
}

#[test]
fn take_inventory_and_drop_form_a_shared_world_loop() {
    let (mut world, session) = connected_world();

    let taken = world
        .apply(session, PlayerAction::Take("lantern".into()))
        .unwrap();
    assert_eq!(
        taken.output.as_deref(),
        Some("You take the blue-glass lantern.\r\n")
    );
    assert!(taken.scene.items.is_empty());
    assert!(taken.changed_character.is_none());
    assert!(matches!(
        taken.domain_events.as_slice(),
        [DomainEvent::ItemTaken { .. }]
    ));
    let inventory = world
        .apply(session, PlayerAction::Inventory)
        .unwrap()
        .output
        .unwrap();
    assert_eq!(
        redact_and_validate_opaque_item_refs(&inventory),
        "Capacity: 1/24 slots; 1000/50000 g.\r\nYou are carrying:\r\n- 1 × blue-glass lantern [common; level 1; 1000 g each; 1 stack slot; 1000 g total; policy transferable]\r\n"
    );

    world
        .apply(session, PlayerAction::Move(Direction::North))
        .unwrap();
    let dropped = world
        .apply(session, PlayerAction::Drop("lamp".into()))
        .unwrap();
    assert_ne!(dropped.scene.items[0].id, "demo:blue-lantern");
    assert!(dropped.changed_character.is_none());
    let saved = world.character_snapshot(session).unwrap();

    world.disconnect(session);
    let reconnected = world.connect(SessionId(77), saved).unwrap();
    assert_eq!(reconnected.title, "The North Gate");
    assert_ne!(reconnected.items[0].id, "demo:blue-lantern");
}

#[test]
fn examine_is_read_only_and_works_for_visible_or_carried_items() {
    let (mut world, session) = connected_world();
    let before = world.character_snapshot(session).unwrap();
    let examined = world
        .apply(session, PlayerAction::Examine("blue".into()))
        .unwrap();
    assert!(examined.output.unwrap().contains("Cold blue light"));
    assert!(examined.changed_character.is_none());
    assert_eq!(world.character_snapshot(session).unwrap(), before);

    world
        .apply(session, PlayerAction::Take("lantern".into()))
        .unwrap();
    assert!(
        world
            .apply(session, PlayerAction::Examine("lamp".into()))
            .unwrap()
            .output
            .unwrap()
            .contains("rain-speckled glass")
    );
}

#[test]
fn absent_items_fail_without_mutation() {
    let (mut world, session) = connected_world();
    let before = world.character_snapshot(session).unwrap();
    let revision = world.revision();

    assert!(matches!(
        world.apply(session, PlayerAction::Take("sword".into())),
        Err(WorldError::ItemNotFound(query)) if query == "sword"
    ));
    assert!(matches!(
        world.apply(session, PlayerAction::Drop("lantern".into())),
        Err(WorldError::ItemNotFound(query)) if query == "lantern"
    ));
    assert_eq!(world.character_snapshot(session).unwrap(), before);
    assert_eq!(world.revision(), revision);
}

#[test]
fn ambiguous_item_names_fail_without_mutation() {
    let mut world = World::from_definition(WorldDefinition {
        start_room_id: "test:start".into(),
        item_prototypes: vec![
            item_prototype("test:blue", "blue lantern", "Blue.", &["lantern"]),
            item_prototype("test:brass", "brass lantern", "Brass.", &["lantern"]),
        ],
        initial_item_instances: vec![
            item_instance(
                "test:blue-lantern",
                "test:blue",
                InitialItemLocation::Room("test:start".into()),
            ),
            item_instance(
                "test:brass-lantern",
                "test:brass",
                InitialItemLocation::Room("test:start".into()),
            ),
        ],
        rooms: vec![RoomDefinition {
            stable_id: "test:start".into(),
            title: "Start".into(),
            prose: Vec::new(),
            image: None,
            audio: None,
            exits: Vec::new(),
            item_ids: vec!["test:blue-lantern".into(), "test:brass-lantern".into()],
        }],
    })
    .unwrap();
    let session = SessionId(8);
    world
        .connect(
            session,
            CharacterSnapshot {
                id: CharacterId(8),
                account_id: AccountId(8),
                name: "Chooser".into(),
                room_id: "test:start".into(),
                item_locations: BTreeMap::new(),
            },
        )
        .unwrap();
    let before = world.character_snapshot(session).unwrap();

    assert!(matches!(
        world.apply(session, PlayerAction::Take("lantern".into())),
        Err(WorldError::AmbiguousItem { .. })
    ));
    assert_eq!(world.character_snapshot(session).unwrap(), before);
}

#[test]
fn rejects_invalid_item_definitions_and_room_references() {
    let room = || RoomDefinition {
        stable_id: "test:start".into(),
        title: "Start".into(),
        prose: Vec::new(),
        image: None,
        audio: None,
        exits: Vec::new(),
        item_ids: Vec::new(),
    };
    let item = item_prototype("test:item", "item", "An item.", &[]);

    assert!(matches!(
        World::from_definition(WorldDefinition {
            start_room_id: "test:start".into(),
            item_prototypes: Vec::new(),
            initial_item_instances: vec![item_instance(
                "test:instance",
                "missing",
                InitialItemLocation::Room("test:start".into()),
            )],
            rooms: vec![room()],
        }),
        Err(WorldBuildError::UnknownItemPrototype(id)) if id == "missing"
    ));
    assert!(
        World::from_definition(WorldDefinition {
            start_room_id: "test:start".into(),
            item_prototypes: vec![item.clone()],
            initial_item_instances: Vec::new(),
            rooms: vec![room()],
        })
        .is_ok()
    );
    assert!(matches!(
        World::from_definition(WorldDefinition {
            start_room_id: "test:start".into(),
            item_prototypes: vec![item.clone()],
            initial_item_instances: vec![
                item_instance("duplicate", "test:item", InitialItemLocation::Staged),
                item_instance("duplicate", "test:item", InitialItemLocation::Staged),
            ],
            rooms: vec![room()],
        }),
        Err(WorldBuildError::DuplicateItemId(id)) if id == "duplicate"
    ));

    assert!(matches!(
        World::from_definition_with_staged_items(
            WorldDefinition {
                start_room_id: "test:start".into(),
                item_prototypes: vec![item.clone()],
                initial_item_instances: vec![item_instance(
                    "test:item-instance",
                    "test:item",
                    InitialItemLocation::Room("test:start".into()),
                )],
                rooms: vec![room()],
            },
            vec!["test:item-instance".into()],
        ),
        Err(WorldBuildError::ItemPlacedAndStaged { item_id, .. })
            if item_id == "test:item-instance"
    ));

    let mut weightless = item;
    weightless.weight_grams = 0;
    assert!(
        World::from_definition(WorldDefinition {
            start_room_id: "test:start".into(),
            item_prototypes: vec![weightless],
            initial_item_instances: Vec::new(),
            rooms: vec![room()],
        })
        .is_ok()
    );

    let mut reserved_transfer = item_prototype("test:bound", "bound", "Bound.", &[]);
    reserved_transfer.transfer_policy = ItemTransferPolicy::CharacterBound;
    assert!(
        World::from_definition(WorldDefinition {
            start_room_id: "test:start".into(),
            item_prototypes: vec![reserved_transfer],
            initial_item_instances: Vec::new(),
            rooms: vec![room()],
        })
        .is_ok()
    );
}

#[test]
fn repeatable_instances_are_distinct_opaque_and_capacity_failures_are_atomic() {
    let mut prototype = item_prototype(
        "test:repeated",
        "etched token",
        "One concrete token from a reusable design.",
        &["token"],
    );
    prototype.weight_grams = 600;
    prototype.rarity = ItemRarity::Rare;
    prototype.item_level = 7;
    let mut world = World::from_definition(WorldDefinition {
        start_room_id: "test:start".into(),
        item_prototypes: vec![prototype],
        initial_item_instances: vec![
            item_instance(
                "durable:first",
                "test:repeated",
                InitialItemLocation::Room("test:start".into()),
            ),
            item_instance(
                "durable:second",
                "test:repeated",
                InitialItemLocation::Room("test:start".into()),
            ),
        ],
        rooms: vec![RoomDefinition {
            stable_id: "test:start".into(),
            title: "Start".into(),
            prose: Vec::new(),
            image: None,
            audio: None,
            exits: Vec::new(),
            item_ids: Vec::new(),
        }],
    })
    .unwrap();
    let session = SessionId(500);
    world
        .connect(
            session,
            CharacterSnapshot {
                id: CharacterId(500),
                account_id: AccountId(500),
                name: "Carrier".into(),
                room_id: "test:start".into(),
                item_locations: BTreeMap::new(),
            },
        )
        .unwrap();
    let mut gameplay = world.character_gameplay_snapshot(session).unwrap();
    gameplay.inventory_capacity = InventoryCapacity {
        max_slots: 1,
        max_weight_grams: 1_000,
    };
    world
        .restore_character_gameplay(CharacterId(500), gameplay)
        .unwrap();

    let scene = world.scene_for(session).unwrap();
    assert_eq!(scene.items.len(), 2);
    assert_ne!(scene.items[0].id, scene.items[1].id);
    assert!(scene.items.iter().all(|item| item.rarity == "rare"));
    let scene_json = serde_json::to_string(&scene).unwrap();
    assert!(!scene_json.contains("durable:first"));
    assert!(!scene_json.contains("durable:second"));
    assert!(!scene_json.contains("test:repeated"));

    world
        .apply(
            session,
            PlayerAction::Take(scene.items[0].id.clone().into()),
        )
        .unwrap();
    let before = world.runtime_snapshot();
    let revision = world.revision();
    let remaining = world.scene_for(session).unwrap().items[0].id.clone();
    assert!(matches!(
        world.apply(session, PlayerAction::Take(remaining.into())),
        Err(WorldError::InventoryCapacityExceeded)
    ));
    assert_eq!(world.runtime_snapshot(), before);
    assert_eq!(world.revision(), revision);
    let text = world.inventory_as_plain_text(session).unwrap();
    assert!(text.contains("1/1 slots"));
    assert!(text.contains(
        "[rare; level 7; 600 g each; 1 stack slot; 600 g total; policy transferable; ref o"
    ));
    assert!(
        redact_and_validate_opaque_item_refs(&text).contains(
            "[rare; level 7; 600 g each; 1 stack slot; 600 g total; policy transferable]"
        )
    );
}

#[test]
fn unequip_and_equipment_swap_preflight_slot_capacity() {
    let mut bulky = item_prototype("test:bulky", "bulky blade", "Bulky.", &["bulky"]);
    bulky.equipment_slot = Some(EquipmentSlot::Weapon);
    bulky.slot_cost = 2;
    let mut light = item_prototype("test:light", "light blade", "Light.", &["light"]);
    light.equipment_slot = Some(EquipmentSlot::Weapon);
    let mut world = World::from_definition(WorldDefinition {
        start_room_id: "test:start".into(),
        item_prototypes: vec![bulky, light],
        initial_item_instances: vec![
            item_instance(
                "test:bulky-instance",
                "test:bulky",
                InitialItemLocation::Room("test:start".into()),
            ),
            item_instance(
                "test:light-instance",
                "test:light",
                InitialItemLocation::Room("test:start".into()),
            ),
        ],
        rooms: vec![RoomDefinition {
            stable_id: "test:start".into(),
            title: "Start".into(),
            prose: Vec::new(),
            image: None,
            audio: None,
            exits: Vec::new(),
            item_ids: Vec::new(),
        }],
    })
    .unwrap();
    let session = SessionId(501);
    world
        .connect(
            session,
            CharacterSnapshot {
                id: CharacterId(501),
                account_id: AccountId(501),
                name: "Carrier".into(),
                room_id: "test:start".into(),
                item_locations: BTreeMap::new(),
            },
        )
        .unwrap();
    world
        .apply(session, PlayerAction::Take("bulky".into()))
        .unwrap();
    world
        .apply(session, PlayerAction::Equip("bulky".into()))
        .unwrap();
    world
        .apply(session, PlayerAction::Take("light".into()))
        .unwrap();
    let mut gameplay = world.character_gameplay_snapshot(session).unwrap();
    gameplay.inventory_capacity.max_slots = 1;
    world
        .restore_character_gameplay(CharacterId(501), gameplay)
        .unwrap();
    let before = world.runtime_snapshot();
    let gameplay_before = world.character_gameplay_snapshot(session).unwrap();
    assert!(matches!(
        world.apply(session, PlayerAction::Unequip("weapon".into())),
        Err(WorldError::InventoryCapacityExceeded)
    ));
    assert!(matches!(
        world.apply(session, PlayerAction::Equip("light".into())),
        Err(WorldError::InventoryCapacityExceeded)
    ));
    assert_eq!(world.runtime_snapshot(), before);
    assert_eq!(
        world.character_gameplay_snapshot(session).unwrap(),
        gameplay_before
    );
}

#[test]
fn staged_npc_loot_is_neither_visible_nor_interactable_before_spawn() {
    let mut world = World::from_definition_with_staged_items(
        WorldDefinition {
            start_room_id: "test:start".into(),
            item_prototypes: vec![item_prototype(
                "test:loot",
                "silver fang",
                "A trophy carried by a guardian.",
                &["fang"],
            )],
            initial_item_instances: vec![item_instance(
                "test:loot",
                "test:loot",
                InitialItemLocation::Staged,
            )],
            rooms: vec![RoomDefinition {
                stable_id: "test:start".into(),
                title: "Start".into(),
                prose: Vec::new(),
                image: None,
                audio: None,
                exits: Vec::new(),
                item_ids: Vec::new(),
            }],
        },
        vec!["test:loot".into()],
    )
    .unwrap();
    let session = SessionId(91);
    world
        .connect(
            session,
            CharacterSnapshot {
                id: CharacterId(92),
                account_id: AccountId(93),
                name: "Observer".into(),
                room_id: "test:start".into(),
                item_locations: BTreeMap::new(),
            },
        )
        .unwrap();

    assert!(world.scene_for(session).unwrap().items.is_empty());
    assert!(matches!(
        world.apply(session, PlayerAction::Take("fang".into())),
        Err(WorldError::ItemNotFound(query)) if query == "fang"
    ));
    world
        .spawn_npc(NpcDefinition {
            stable_id: "test:guardian".into(),
            name: "guardian".into(),
            description: "A silent guardian.".into(),
            room_id: "test:start".into(),
            level: 1,
            max_health: 1,
            strength: 1,
            defence: 0,
            experience_reward: 1,
            loot_item_ids: vec![ItemInstanceId("test:loot".into())],
            respawn_ticks: 1,
        })
        .unwrap();
    assert_eq!(
        world.runtime_snapshot().item_instances[&ItemInstanceId("test:loot".into())].location,
        RuntimeItemLocation::HeldByNpc {
            npc_id: "test:guardian".into(),
        }
    );
    world
        .spawn_npc(NpcDefinition {
            stable_id: "test:bystander".into(),
            name: "bystander".into(),
            description: "Another NPC.".into(),
            room_id: "test:start".into(),
            level: 1,
            max_health: 1,
            strength: 1,
            defence: 0,
            experience_reward: 0,
            loot_item_ids: Vec::new(),
            respawn_ticks: 1,
        })
        .unwrap();
    let mut wrong_owner = world.runtime_snapshot();
    wrong_owner
        .item_instances
        .get_mut(&ItemInstanceId("test:loot".into()))
        .unwrap()
        .location = RuntimeItemLocation::HeldByNpc {
        npc_id: "test:bystander".into(),
    };
    assert!(matches!(
        world.restore_runtime_snapshot(wrong_owner),
        Err(WorldError::InvalidRuntimeSnapshot)
    ));
}

#[test]
fn fungible_stacks_and_containers_are_derived_atomic_and_opaque() {
    let mut tonic = item_prototype("test:tonic", "healing tonic", "Restorative.", &["tonic"]);
    tonic.stack_policy = ItemStackPolicy::Fungible;
    tonic.max_stack_units = 2;
    tonic.weight_grams = 100;
    let mut satchel = item_prototype(
        "test:satchel",
        "archive satchel",
        "A bounded satchel.",
        &["satchel"],
    );
    satchel.weight_grams = 500;
    satchel.container = Some(ItemContainerDefinition {
        max_slots: 2,
        max_weight_grams: 500,
    });
    let instances = vec![
        item_instance(
            "test:satchel-instance",
            "test:satchel",
            InitialItemLocation::Room("test:start".into()),
        ),
        item_instance(
            "test:tonic-a",
            "test:tonic",
            InitialItemLocation::Room("test:start".into()),
        ),
        item_instance(
            "test:tonic-b",
            "test:tonic",
            InitialItemLocation::Room("test:start".into()),
        ),
        item_instance(
            "test:tonic-c",
            "test:tonic",
            InitialItemLocation::Room("test:start".into()),
        ),
    ];
    let (mut world, session) = connect_definition(WorldDefinition {
        start_room_id: "test:start".into(),
        item_prototypes: vec![tonic, satchel],
        initial_item_instances: instances,
        rooms: vec![RoomDefinition {
            stable_id: "test:start".into(),
            title: "Start".into(),
            prose: vec!["A test room.".into()],
            image: None,
            audio: None,
            item_ids: Vec::new(),
            exits: Vec::new(),
        }],
    });

    let scene = world.scene_for(session).unwrap();
    let tonic_groups = scene
        .items
        .iter()
        .filter(|item| item.label == "healing tonic")
        .collect::<Vec<_>>();
    assert_eq!(tonic_groups.len(), 2);
    assert_eq!(
        tonic_groups.iter().map(|item| item.quantity).sum::<u16>(),
        3
    );
    assert!(tonic_groups.iter().all(|item| item.id.starts_with('o')));

    let before_take = world.runtime_snapshot();
    let before_take_revision = world.revision();
    let take = world
        .apply(
            session,
            PlayerAction::Take(ItemSelection {
                query: "tonic".into(),
                quantity: 3,
            }),
        )
        .unwrap();
    assert_eq!(world.revision(), before_take_revision + 1);
    assert!(matches!(
        take.domain_events.as_slice(),
        [DomainEvent::ItemsMoved {
            item_ids,
            operation: ContainerMoveOperation::Take,
            container_id: None,
            ..
        }] if item_ids.len() == 3
    ));
    let projected = world
        .project_event(session, &take.domain_events[0])
        .unwrap()
        .unwrap();
    assert_eq!(projected.kind, ViewEventKind::ItemTaken);
    assert!(projected.target_id.is_none());
    let after_take = world.runtime_snapshot();
    world.rollback_domain_events(&take.domain_events).unwrap();
    assert_eq!(world.runtime_snapshot(), before_take);
    let mut forged = take.domain_events[0].clone();
    if let DomainEvent::ItemsMoved { item_ids, .. } = &mut forged {
        item_ids[1] = item_ids[0].clone();
    }
    assert!(matches!(
        world.replay_persisted_event(&forged),
        Err(WorldError::InvalidDomainEvent)
    ));
    assert_eq!(world.runtime_snapshot(), before_take);
    world
        .replay_persisted_event(&take.domain_events[0])
        .unwrap();
    assert_eq!(world.runtime_snapshot(), after_take);
    world
        .apply(session, PlayerAction::Take(ItemSelection::one("satchel")))
        .unwrap();
    let player = world.player_state_view(session).unwrap();
    assert_eq!(player.inventory_capacity.used_slots, 3);
    assert_eq!(player.inventory_capacity.used_weight_grams, 800);

    world
        .apply(
            session,
            PlayerAction::Put {
                selection: ItemSelection {
                    query: "tonic".into(),
                    quantity: 2,
                },
                container: "satchel".into(),
            },
        )
        .unwrap();
    let contents = world
        .apply(
            session,
            PlayerAction::Contents {
                container: "satchel".into(),
                page: 1,
            },
        )
        .unwrap()
        .container_contents
        .unwrap();
    assert!(contents.selector.starts_with('o'));
    assert_eq!(contents.items.len(), 1);
    assert_eq!(contents.items[0].quantity, 2);
    assert_eq!(contents.capacity.used_slots, 1);
    assert_eq!(contents.capacity.used_weight_grams, 200);

    let before_get = world.runtime_snapshot();
    let get = world
        .apply(
            session,
            PlayerAction::Get {
                selection: ItemSelection {
                    query: "tonic".into(),
                    quantity: 2,
                },
                container: "satchel".into(),
            },
        )
        .unwrap();
    assert!(
        matches!(get.domain_events.as_slice(), [DomainEvent::ItemsMoved { item_ids, .. }] if item_ids.len() == 2)
    );
    world.rollback_domain_events(&get.domain_events).unwrap();
    assert_eq!(world.runtime_snapshot(), before_get);
}

#[test]
fn container_text_and_actions_keep_capacity_stack_and_weight_facts() {
    let mut satchel = item_prototype(
        "test:satchel",
        "archive satchel",
        "A bounded satchel.",
        &["satchel"],
    );
    satchel.weight_grams = 500;
    satchel.equipment_slot = Some(EquipmentSlot::Trinket);
    satchel.container = Some(ItemContainerDefinition {
        max_slots: 4,
        max_weight_grams: 1_000,
    });
    let mut box_item = item_prototype("test:box", "map box", "A smaller container.", &["box"]);
    box_item.weight_grams = 100;
    box_item.container = Some(ItemContainerDefinition {
        max_slots: 2,
        max_weight_grams: 500,
    });
    let mut tonic = item_prototype("test:tonic", "healing tonic", "Restorative.", &["tonic"]);
    tonic.weight_grams = 100;
    let (mut world, session) = connect_definition(WorldDefinition {
        start_room_id: "test:start".into(),
        item_prototypes: vec![satchel, box_item, tonic],
        initial_item_instances: vec![
            item_instance(
                "test:satchel-instance",
                "test:satchel",
                InitialItemLocation::Room("test:start".into()),
            ),
            item_instance(
                "test:box-instance",
                "test:box",
                InitialItemLocation::Container(ItemInstanceId("test:satchel-instance".into())),
            ),
            item_instance(
                "test:tonic-instance",
                "test:tonic",
                InitialItemLocation::Container(ItemInstanceId("test:box-instance".into())),
            ),
        ],
        rooms: vec![RoomDefinition {
            stable_id: "test:start".into(),
            title: "Start".into(),
            prose: vec!["A test room.".into()],
            image: None,
            audio: None,
            item_ids: Vec::new(),
            exits: Vec::new(),
        }],
    });

    let scene_satchel = world
        .scene_for(session)
        .unwrap()
        .items
        .into_iter()
        .find(|item| item.label == "archive satchel")
        .unwrap();
    assert_eq!(scene_satchel.quantity, 1);
    assert_eq!(scene_satchel.stack_slots, 1);
    assert_eq!(scene_satchel.total_weight_grams, 700);
    assert!(
        scene_satchel
            .actions
            .iter()
            .any(|action| action.label == "Contents"
                && action.command == format!("contents {}", scene_satchel.id))
    );

    world
        .apply(session, PlayerAction::Take(ItemSelection::one("satchel")))
        .unwrap();
    let carried = world.inventory_as_plain_text(session).unwrap();
    assert!(carried.contains("1 × archive satchel"));
    assert!(carried.contains("1 stack slot"));
    assert!(carried.contains("700 g total"));
    assert!(carried.contains("container 1/4 slots; 200/1000 g"));

    world
        .apply(session, PlayerAction::Equip("satchel".into()))
        .unwrap();
    let equipped = world.inventory_as_plain_text(session).unwrap();
    assert!(equipped.contains("trinket: 1 × archive satchel"));
    assert!(equipped.contains("1 stack slot"));
    assert!(equipped.contains("container 1/4 slots; 200/1000 g"));

    let contents = world
        .apply(
            session,
            PlayerAction::Contents {
                container: "satchel".into(),
                page: 1,
            },
        )
        .unwrap();
    let text = contents.output.unwrap();
    assert!(text.contains("1 × map box"));
    assert!(text.contains("1 stack slot"));
    assert!(text.contains("200 g total"));
    assert!(text.contains("container 1/2 slots; 100/500 g"));
}

#[test]
fn container_cycles_and_capacity_fail_without_partial_mutation() {
    let mut bag = item_prototype("test:bag", "canvas bag", "A bag.", &["bag"]);
    bag.container = Some(ItemContainerDefinition {
        max_slots: 2,
        max_weight_grams: 10_000,
    });
    let (mut world, session) = connect_definition(WorldDefinition {
        start_room_id: "test:start".into(),
        item_prototypes: vec![bag],
        initial_item_instances: vec![
            item_instance(
                "test:bag-a",
                "test:bag",
                InitialItemLocation::Room("test:start".into()),
            ),
            item_instance(
                "test:bag-b",
                "test:bag",
                InitialItemLocation::Room("test:start".into()),
            ),
        ],
        rooms: vec![RoomDefinition {
            stable_id: "test:start".into(),
            title: "Start".into(),
            prose: vec!["A test room.".into()],
            image: None,
            audio: None,
            item_ids: Vec::new(),
            exits: Vec::new(),
        }],
    });
    let ids = world.scene_for(session).unwrap().items;
    world
        .apply(session, PlayerAction::Take(ids[0].id.clone().into()))
        .unwrap();
    world
        .apply(session, PlayerAction::Take(ids[1].id.clone().into()))
        .unwrap();
    let carried = world.player_state_view(session).unwrap().inventory;
    world
        .apply(
            session,
            PlayerAction::Put {
                selection: ItemSelection::one(carried[0].id.clone()),
                container: carried[1].id.clone(),
            },
        )
        .unwrap();
    let before = world.runtime_snapshot();
    let nested = world
        .apply(
            session,
            PlayerAction::Contents {
                container: carried[1].id.clone(),
                page: 1,
            },
        )
        .unwrap()
        .container_contents
        .unwrap()
        .items[0]
        .id
        .clone();
    assert!(
        world
            .apply(
                session,
                PlayerAction::Put {
                    selection: ItemSelection::one(carried[1].id.clone()),
                    container: nested,
                },
            )
            .is_err()
    );
    assert_eq!(world.runtime_snapshot(), before);
}

#[test]
fn fungible_equipment_uses_prospective_stack_capacity() {
    let mut blade = item_prototype("test:blade", "thin blade", "A blade.", &["blade"]);
    blade.stack_policy = ItemStackPolicy::Fungible;
    blade.max_stack_units = 10;
    blade.equipment_slot = Some(EquipmentSlot::Weapon);
    let (mut world, session) = connect_definition(WorldDefinition {
        start_room_id: "test:start".into(),
        item_prototypes: vec![blade],
        initial_item_instances: vec![
            item_instance(
                "test:blade-a",
                "test:blade",
                InitialItemLocation::Room("test:start".into()),
            ),
            item_instance(
                "test:blade-b",
                "test:blade",
                InitialItemLocation::Room("test:start".into()),
            ),
        ],
        rooms: vec![RoomDefinition {
            stable_id: "test:start".into(),
            title: "Start".into(),
            prose: vec!["A test room.".into()],
            image: None,
            audio: None,
            item_ids: Vec::new(),
            exits: Vec::new(),
        }],
    });
    world
        .character_gameplay
        .get_mut(&CharacterId(701))
        .unwrap()
        .inventory_capacity
        .max_slots = 1;
    world
        .apply(
            session,
            PlayerAction::Take(ItemSelection {
                query: "blade".into(),
                quantity: 2,
            }),
        )
        .unwrap();
    let selector = world.player_state_view(session).unwrap().inventory[0]
        .id
        .clone();
    world.apply(session, PlayerAction::Equip(selector)).unwrap();
    world
        .apply(session, PlayerAction::Unequip("weapon".into()))
        .unwrap();
    let player = world.player_state_view(session).unwrap();
    assert_eq!(player.inventory.len(), 1);
    assert_eq!(player.inventory[0].quantity, 2);
    assert_eq!(player.inventory_capacity.used_slots, 1);
}

#[test]
fn batch_item_projection_filters_hidden_units_and_private_container_selectors() {
    let mut tonic = item_prototype("test:tonic", "healing tonic", "Restorative.", &["tonic"]);
    tonic.stack_policy = ItemStackPolicy::Fungible;
    tonic.max_stack_units = 20;
    let mut satchel = item_prototype(
        "test:satchel",
        "archive satchel",
        "A bounded satchel.",
        &["satchel"],
    );
    satchel.container = Some(ItemContainerDefinition {
        max_slots: 20,
        max_weight_grams: 20_000,
    });
    let (mut world, actor) = connect_definition(WorldDefinition {
        start_room_id: "test:start".into(),
        item_prototypes: vec![tonic, satchel],
        initial_item_instances: vec![
            item_instance(
                "test:tonic-visible",
                "test:tonic",
                InitialItemLocation::Room("test:start".into()),
            ),
            item_instance(
                "test:tonic-hidden",
                "test:tonic",
                InitialItemLocation::Room("test:start".into()),
            ),
            item_instance(
                "test:satchel-instance",
                "test:satchel",
                InitialItemLocation::Room("test:start".into()),
            ),
        ],
        rooms: vec![RoomDefinition {
            stable_id: "test:start".into(),
            title: "Start".into(),
            prose: vec!["A test room.".into()],
            image: None,
            audio: None,
            item_ids: Vec::new(),
            exits: Vec::new(),
        }],
    });
    world
        .configure_hidden_item("test:tonic-hidden", 20)
        .unwrap();
    world.set_observer_role(actor, AccountRole::Admin).unwrap();
    let peer = SessionId(72);
    world
        .connect(
            peer,
            CharacterSnapshot {
                id: CharacterId(702),
                account_id: AccountId(72),
                name: "Witness".into(),
                room_id: "test:start".into(),
                item_locations: BTreeMap::new(),
            },
        )
        .unwrap();
    let peer_visible_tonic = world
        .scene_for(peer)
        .unwrap()
        .items
        .into_iter()
        .find(|item| item.label == "healing tonic")
        .unwrap()
        .id;

    let take = world
        .apply(
            actor,
            PlayerAction::Take(ItemSelection {
                query: "tonic".into(),
                quantity: 2,
            }),
        )
        .unwrap();
    let actor_event = world
        .project_event(actor, &take.domain_events[0])
        .unwrap()
        .unwrap();
    let peer_event = world
        .project_event(peer, &take.domain_events[0])
        .unwrap()
        .unwrap();
    assert!(actor_event.text.contains("2 × healing tonic"));
    assert!(peer_event.text.contains("1 × healing tonic"));
    assert_eq!(
        peer_event.item_id.as_deref(),
        Some(peer_visible_tonic.as_str())
    );

    world
        .apply(actor, PlayerAction::Take(ItemSelection::one("satchel")))
        .unwrap();
    let put = world
        .apply(
            actor,
            PlayerAction::Put {
                selection: ItemSelection::one("tonic"),
                container: "satchel".into(),
            },
        )
        .unwrap();
    let peer_put = world
        .project_event(peer, &put.domain_events[0])
        .unwrap()
        .unwrap();
    assert!(peer_put.text.contains("puts 1 × healing tonic"));
    assert!(peer_put.target_id.is_none());
    assert!(peer_put.item_id.is_none());
}

#[test]
fn batch_replay_supports_offline_owners_but_rejects_foreign_container_roots() {
    let mut tonic = item_prototype("test:tonic", "healing tonic", "Restorative.", &["tonic"]);
    tonic.stack_policy = ItemStackPolicy::Fungible;
    tonic.max_stack_units = 20;
    let mut satchel = item_prototype(
        "test:satchel",
        "archive satchel",
        "A bounded satchel.",
        &["satchel"],
    );
    satchel.container = Some(ItemContainerDefinition {
        max_slots: 20,
        max_weight_grams: 20_000,
    });
    let definition = WorldDefinition {
        start_room_id: "test:start".into(),
        item_prototypes: vec![tonic, satchel],
        initial_item_instances: vec![
            item_instance(
                "test:tonic-a",
                "test:tonic",
                InitialItemLocation::Room("test:start".into()),
            ),
            item_instance(
                "test:tonic-b",
                "test:tonic",
                InitialItemLocation::Room("test:start".into()),
            ),
            item_instance(
                "test:satchel-instance",
                "test:satchel",
                InitialItemLocation::Room("test:start".into()),
            ),
        ],
        rooms: vec![RoomDefinition {
            stable_id: "test:start".into(),
            title: "Start".into(),
            prose: vec!["A test room.".into()],
            image: None,
            audio: None,
            item_ids: Vec::new(),
            exits: Vec::new(),
        }],
    };
    let offline_owner = CharacterId(900);
    let offline_take = DomainEvent::ItemsMoved {
        actor_id: offline_owner,
        actor_name: "Offline owner".into(),
        item_ids: vec!["test:tonic-a".into(), "test:tonic-b".into()],
        item_name: "healing tonic".into(),
        container_id: None,
        operation: ContainerMoveOperation::Take,
        room_id: "test:start".into(),
    };
    let mut offline_world = World::from_definition(definition.clone()).unwrap();
    offline_world.replay_persisted_event(&offline_take).unwrap();
    assert_eq!(
        offline_world.runtime_item_owner_ids(),
        BTreeSet::from([offline_owner])
    );

    let (mut world, actor) = connect_definition(definition);
    let victim = SessionId(73);
    world
        .connect(
            victim,
            CharacterSnapshot {
                id: CharacterId(703),
                account_id: AccountId(73),
                name: "Other owner".into(),
                room_id: "test:start".into(),
                item_locations: BTreeMap::new(),
            },
        )
        .unwrap();
    world
        .apply(actor, PlayerAction::Take(ItemSelection::one("tonic")))
        .unwrap();
    world
        .apply(victim, PlayerAction::Take(ItemSelection::one("satchel")))
        .unwrap();
    let forged = DomainEvent::ItemsMoved {
        actor_id: CharacterId(701),
        actor_name: "Stack Tester".into(),
        item_ids: vec!["test:tonic-a".into()],
        item_name: "healing tonic".into(),
        container_id: Some("test:satchel-instance".into()),
        operation: ContainerMoveOperation::Put,
        room_id: "test:start".into(),
    };
    let before = world.runtime_snapshot();
    assert!(matches!(
        world.replay_persisted_event(&forged),
        Err(WorldError::InvalidDomainEvent)
    ));
    assert_eq!(world.runtime_snapshot(), before);
}

#[test]
fn rollback_revalidates_inventory_capacity_before_publishing_locations() {
    let tonic = item_prototype("test:tonic", "healing tonic", "Restorative.", &["tonic"]);
    let mut satchel = item_prototype(
        "test:satchel",
        "archive satchel",
        "A bounded satchel.",
        &["satchel"],
    );
    satchel.container = Some(ItemContainerDefinition {
        max_slots: 20,
        max_weight_grams: 20_000,
    });
    let (mut world, actor) = connect_definition(WorldDefinition {
        start_room_id: "test:start".into(),
        item_prototypes: vec![tonic, satchel],
        initial_item_instances: vec![
            item_instance(
                "test:tonic-a",
                "test:tonic",
                InitialItemLocation::Room("test:start".into()),
            ),
            item_instance(
                "test:satchel-instance",
                "test:satchel",
                InitialItemLocation::Room("test:start".into()),
            ),
        ],
        rooms: vec![RoomDefinition {
            stable_id: "test:start".into(),
            title: "Start".into(),
            prose: vec!["A test room.".into()],
            image: None,
            audio: None,
            item_ids: Vec::new(),
            exits: Vec::new(),
        }],
    });
    world
        .apply(actor, PlayerAction::Take(ItemSelection::one("tonic")))
        .unwrap();
    world
        .apply(actor, PlayerAction::Take(ItemSelection::one("satchel")))
        .unwrap();
    let put = world
        .apply(
            actor,
            PlayerAction::Put {
                selection: ItemSelection::one("tonic"),
                container: "satchel".into(),
            },
        )
        .unwrap();
    world
        .character_gameplay
        .get_mut(&CharacterId(701))
        .unwrap()
        .inventory_capacity
        .max_slots = 1;
    let before = world.runtime_snapshot();
    assert!(matches!(
        world.rollback_domain_events(&put.domain_events),
        Err(WorldError::InventoryCapacityExceeded)
    ));
    assert_eq!(world.runtime_snapshot(), before);
}

#[test]
fn acquisition_binds_recursively_and_restricted_subtrees_cannot_escape_owner() {
    let (mut world, actor) = connect_definition(policy_definition());
    let actor_id = CharacterId(701);
    world
        .connect(
            SessionId(72),
            CharacterSnapshot {
                id: CharacterId(702),
                account_id: AccountId(72),
                name: "Aria".into(),
                room_id: "test:start".into(),
                item_locations: BTreeMap::new(),
            },
        )
        .unwrap();
    let clasp_id = ItemInstanceId("policy:clasp-instance".into());
    let token_id = ItemInstanceId("policy:token-instance".into());
    assert_eq!(
        world.runtime_snapshot().item_instances[&clasp_id].binding,
        ItemBinding::Unbound
    );

    let original_max_weight = world
        .character_gameplay
        .get(&actor_id)
        .unwrap()
        .inventory_capacity
        .max_weight_grams;
    world
        .character_gameplay
        .get_mut(&actor_id)
        .unwrap()
        .inventory_capacity
        .max_weight_grams = 1_000;
    let before_failed_script_grant = world.runtime_snapshot();
    assert!(matches!(
        world.give_authorized_script_item(actor, "policy:bag-instance"),
        Err(WorldError::InventoryCapacityExceeded)
    ));
    assert_eq!(world.runtime_snapshot(), before_failed_script_grant);
    assert_eq!(
        world.runtime_snapshot().item_instances[&clasp_id].binding,
        ItemBinding::Unbound
    );
    world
        .character_gameplay
        .get_mut(&actor_id)
        .unwrap()
        .inventory_capacity
        .max_weight_grams = original_max_weight;

    let take_bag = world
        .apply(actor, PlayerAction::Take(ItemSelection::one("bag")))
        .unwrap();
    assert_eq!(
        world.runtime_snapshot().item_instances[&clasp_id].binding,
        ItemBinding::Character {
            character_id: actor_id
        }
    );
    assert_eq!(world.runtime_item_owner_ids(), BTreeSet::from([actor_id]));
    let bag = world
        .player_state_view(actor)
        .unwrap()
        .inventory
        .into_iter()
        .find(|item| item.label == "traveller bag")
        .unwrap();
    assert_eq!(bag.transfer_policy, ItemTransferPolicy::Transferable);
    assert!(!bag.bound_to_you);
    assert!(!bag.can_give);
    assert!(!bag.actions.iter().any(|action| action.label == "Drop"));

    let before_forbidden = world.runtime_snapshot();
    let forged_restricted_give = DomainEvent::ItemsGiven {
        giver_id: actor_id,
        giver_name: "Stack Tester".into(),
        recipient_id: CharacterId(702),
        recipient_name: "Aria".into(),
        item_ids: vec!["policy:bag-instance".into()],
        item_name: "traveller bag".into(),
        room_id: "test:start".into(),
    };
    assert!(matches!(
        world.replay_persisted_event(&forged_restricted_give),
        Err(WorldError::InvalidDomainEvent)
    ));
    assert_eq!(world.runtime_snapshot(), before_forbidden);
    assert!(matches!(
        world.apply(actor, PlayerAction::Drop(ItemSelection::one("bag"))),
        Err(WorldError::ItemTransferForbidden)
    ));
    assert!(matches!(
        world.apply(
            actor,
            PlayerAction::Give {
                selection: ItemSelection::one("bag"),
                recipient: "Aria".into(),
            }
        ),
        Err(WorldError::ItemTransferForbidden)
    ));
    assert_eq!(world.runtime_snapshot(), before_forbidden);

    world
        .apply(actor, PlayerAction::Take(ItemSelection::one("token")))
        .unwrap();
    assert_eq!(
        world.runtime_snapshot().item_instances[&token_id].binding,
        ItemBinding::Unbound
    );
    let restricted = world.runtime_snapshot();
    assert!(matches!(
        world.apply(actor, PlayerAction::Drop(ItemSelection::one("token"))),
        Err(WorldError::ItemTransferForbidden)
    ));
    assert!(matches!(
        world.apply(
            actor,
            PlayerAction::Give {
                selection: ItemSelection::one("token"),
                recipient: "Aria".into(),
            }
        ),
        Err(WorldError::ItemTransferForbidden)
    ));
    assert!(matches!(
        world.apply(
            actor,
            PlayerAction::Put {
                selection: ItemSelection::one("token"),
                container: "chest".into(),
            }
        ),
        Err(WorldError::ItemTransferForbidden)
    ));
    assert_eq!(world.runtime_snapshot(), restricted);

    world
        .configure_item_gameplay(ItemGameplayDefinition {
            item_id: "policy:token".into(),
            equipment_slot: None,
            attack_bonus: 0,
            heal_amount: 1,
            consumable: true,
            effect: None,
        })
        .unwrap();
    world
        .apply(actor, PlayerAction::Use("token".into()))
        .unwrap();
    let consumed_token_snapshot = world.runtime_snapshot();
    let consumed_token = &consumed_token_snapshot.item_instances[&token_id];
    assert_eq!(consumed_token.location, RuntimeItemLocation::Consumed);
    assert_eq!(consumed_token.binding, ItemBinding::Unbound);
    validate_item_graph(&world.item_instances, &world.item_prototypes).unwrap();
    let mut restored_consumed = World::from_definition(policy_definition()).unwrap();
    restored_consumed
        .restore_runtime_snapshot(consumed_token_snapshot)
        .unwrap();

    world
        .apply(actor, PlayerAction::Take(ItemSelection::one("chest")))
        .unwrap();
    world
        .apply(
            actor,
            PlayerAction::Put {
                selection: ItemSelection::one("bag"),
                container: "chest".into(),
            },
        )
        .unwrap();
    world
        .apply(
            actor,
            PlayerAction::Get {
                selection: ItemSelection::one("bag"),
                container: "chest".into(),
            },
        )
        .unwrap();
    assert_eq!(
        world.runtime_snapshot().item_instances[&clasp_id].binding,
        ItemBinding::Character {
            character_id: actor_id
        }
    );

    world
        .configure_item_gameplay(ItemGameplayDefinition {
            item_id: "policy:scripted".into(),
            equipment_slot: None,
            attack_bonus: 0,
            heal_amount: 1,
            consumable: true,
            effect: None,
        })
        .unwrap();
    let scripted_event = world
        .give_authorized_script_item(actor, "policy:scripted-instance")
        .unwrap()
        .unwrap();
    assert!(matches!(scripted_event, DomainEvent::ItemTaken { .. }));
    world
        .apply(actor, PlayerAction::Use("sigil".into()))
        .unwrap();
    let consumed_snapshot = world.runtime_snapshot();
    let scripted =
        &consumed_snapshot.item_instances[&ItemInstanceId("policy:scripted-instance".into())];
    assert_eq!(scripted.location, RuntimeItemLocation::Consumed);
    assert_eq!(
        scripted.binding,
        ItemBinding::Character {
            character_id: actor_id
        }
    );
    assert!(world.runtime_item_owner_ids().contains(&actor_id));

    let mut replay = World::from_definition(policy_definition()).unwrap();
    replay
        .replay_persisted_event(&take_bag.domain_events[0])
        .unwrap();
    assert_eq!(
        replay.runtime_snapshot().item_instances[&clasp_id].binding,
        ItemBinding::Character {
            character_id: actor_id
        }
    );
    replay
        .rollback_domain_events(&take_bag.domain_events)
        .unwrap();
    assert_eq!(
        replay.runtime_snapshot().item_instances[&clasp_id].binding,
        ItemBinding::Unbound
    );
}

#[test]
fn give_is_atomic_capacity_bounded_opaque_and_replayable() {
    let definition = give_definition();
    let mut world = World::from_definition(definition.clone()).unwrap();
    let connect = |world: &mut World,
                   session: SessionId,
                   character_id: CharacterId,
                   name: &str,
                   room_id: &str| {
        world
            .connect(
                session,
                CharacterSnapshot {
                    id: character_id,
                    account_id: AccountId(character_id.0),
                    name: name.into(),
                    room_id: room_id.into(),
                    item_locations: BTreeMap::new(),
                },
            )
            .unwrap();
    };
    let giver_session = SessionId(71);
    let recipient_session = SessionId(72);
    let observer_session = SessionId(73);
    let remote_session = SessionId(74);
    connect(
        &mut world,
        giver_session,
        CharacterId(701),
        "Stack Tester",
        "give:start",
    );
    connect(
        &mut world,
        recipient_session,
        CharacterId(702),
        "Aria",
        "give:start",
    );
    connect(
        &mut world,
        observer_session,
        CharacterId(703),
        "Bram",
        "give:start",
    );
    connect(
        &mut world,
        remote_session,
        CharacterId(704),
        "Distant",
        "give:remote",
    );

    let social = world.player_state_view(giver_session).unwrap().social;
    let aria = social
        .online
        .iter()
        .find(|entry| entry.name == "Aria")
        .unwrap();
    let aria_selector = aria.id.clone();
    // Selector hashing is skipped only for queries that cannot be one.
    assert!(is_opaque_id_shaped(&aria_selector));
    assert!(!is_opaque_id_shaped(&aria_selector.to_ascii_uppercase()));
    assert!(!is_opaque_id_shaped("Aria"));
    assert!(matches!(
        world.resolve_give_recipient(giver_session, &aria_selector.to_ascii_uppercase()),
        Err(WorldError::GiveTargetUnavailable)
    ));
    assert!(aria.nearby);
    assert!(
        !social
            .online
            .iter()
            .find(|entry| entry.name == "Distant")
            .unwrap()
            .nearby
    );
    assert_eq!(
        world
            .resolve_give_recipient(giver_session, &aria_selector)
            .unwrap()
            .character_id,
        CharacterId(702)
    );
    assert_eq!(
        world
            .resolve_give_recipient(giver_session, "aRiA")
            .unwrap()
            .character_id,
        CharacterId(702)
    );
    connect(
        &mut world,
        SessionId(75),
        CharacterId(705),
        "ARIA",
        "give:start",
    );
    assert!(matches!(
        world.resolve_give_recipient(giver_session, "aria"),
        Err(WorldError::GiveTargetUnavailable)
    ));
    for invalid in ["Stack Tester", "Distant", "Offline"] {
        assert!(matches!(
            world.resolve_give_recipient(giver_session, invalid),
            Err(WorldError::GiveTargetUnavailable)
        ));
    }

    let take = world
        .apply(
            giver_session,
            PlayerAction::Take(ItemSelection {
                query: "token".into(),
                quantity: 3,
            }),
        )
        .unwrap();
    assert!(world.player_state_view(giver_session).unwrap().inventory[0].can_give);
    world
        .character_gameplay
        .get_mut(&CharacterId(702))
        .unwrap()
        .inventory_capacity
        .max_slots = 1;
    let before_capacity_failure = world.runtime_snapshot();
    assert!(matches!(
        world.apply(
            giver_session,
            PlayerAction::Give {
                selection: ItemSelection {
                    query: "token".into(),
                    quantity: 3,
                },
                recipient: aria_selector.clone(),
            }
        ),
        Err(WorldError::InventoryCapacityExceeded)
    ));
    assert_eq!(world.runtime_snapshot(), before_capacity_failure);

    let given = world
        .apply(
            giver_session,
            PlayerAction::Give {
                selection: ItemSelection {
                    query: "token".into(),
                    quantity: 2,
                },
                recipient: aria_selector,
            },
        )
        .unwrap();
    let event = given.domain_events.first().unwrap();
    assert!(matches!(
        event,
        DomainEvent::ItemsGiven {
            giver_id: CharacterId(701),
            recipient_id: CharacterId(702),
            item_ids,
            ..
        } if item_ids.len() == 2
    ));
    let giver_view = world.project_event(giver_session, event).unwrap().unwrap();
    let recipient_view = world
        .project_event(recipient_session, event)
        .unwrap()
        .unwrap();
    let observer_view = world
        .project_event(observer_session, event)
        .unwrap()
        .unwrap();
    assert_eq!(giver_view.kind, ViewEventKind::ItemGiven);
    assert!(giver_view.text.contains("to Aria"));
    assert!(recipient_view.text.contains("gives you"));
    assert!(observer_view.text.contains("to Aria"));
    assert!(giver_view.item_id.is_some());
    assert!(recipient_view.item_id.is_some());
    assert_ne!(giver_view.item_id, recipient_view.item_id);
    assert!(observer_view.item_id.is_none());
    assert!(
        world
            .project_event(remote_session, event)
            .unwrap()
            .is_none()
    );
    assert_eq!(
        world.player_state_view(giver_session).unwrap().inventory[0].quantity,
        1
    );
    assert_eq!(
        world
            .player_state_view(recipient_session)
            .unwrap()
            .inventory[0]
            .quantity,
        2
    );

    let mut replay = World::from_definition(definition.clone()).unwrap();
    replay
        .replay_persisted_event(&take.domain_events[0])
        .unwrap();
    replay.replay_persisted_event(event).unwrap();
    assert_eq!(
        replay
            .runtime_snapshot()
            .item_instances
            .values()
            .filter(|instance| {
                instance.location
                    == (RuntimeItemLocation::Carried {
                        character_id: CharacterId(702),
                    })
            })
            .count(),
        2
    );
    replay
        .rollback_domain_events(std::slice::from_ref(event))
        .unwrap();
    replay.rollback_domain_events(&take.domain_events).unwrap();
    let initial = World::from_definition(definition)
        .unwrap()
        .runtime_snapshot();
    assert_eq!(replay.runtime_snapshot(), initial);

    world.disconnect(recipient_session);
    assert_eq!(
        world
            .runtime_snapshot()
            .item_instances
            .values()
            .filter(|instance| {
                instance.location
                    == (RuntimeItemLocation::Carried {
                        character_id: CharacterId(702),
                    })
            })
            .count(),
        2
    );
}

#[test]
fn item_graph_index_handles_thousands_of_empty_containers_in_one_pass() {
    let mut container = item_prototype(
        "test:container",
        "empty container",
        "A bounded container.",
        &["container"],
    );
    container.container = Some(ItemContainerDefinition {
        max_slots: 1,
        max_weight_grams: 1,
    });
    let instances = (0..4_096)
        .map(|index| {
            item_instance(
                &format!("test:container-{index}"),
                "test:container",
                InitialItemLocation::Staged,
            )
        })
        .collect();
    let world = World::from_definition(WorldDefinition {
        start_room_id: "test:start".into(),
        item_prototypes: vec![container],
        initial_item_instances: instances,
        rooms: vec![RoomDefinition {
            stable_id: "test:start".into(),
            title: "Start".into(),
            prose: vec!["A test room.".into()],
            image: None,
            audio: None,
            item_ids: Vec::new(),
            exits: Vec::new(),
        }],
    })
    .unwrap();
    let graph = build_item_graph_index(&world.item_instances, &world.item_prototypes).unwrap();
    assert_eq!(graph.subtree_weights.len(), 4_096);
    assert!(graph.children.is_empty());
}

#[test]
fn item_presentation_text_is_bounded_before_it_reaches_transports() {
    let definition_for = |item: ItemPrototypeDefinition| WorldDefinition {
        start_room_id: "test:start".into(),
        item_prototypes: vec![item],
        initial_item_instances: Vec::new(),
        rooms: vec![RoomDefinition {
            stable_id: "test:start".into(),
            title: "Start".into(),
            prose: vec!["A test room.".into()],
            image: None,
            audio: None,
            item_ids: Vec::new(),
            exits: Vec::new(),
        }],
    };
    let mut oversized = item_prototype("test:item", "safe", "Safe.", &["item"]);
    oversized.name = "x".repeat(129);
    assert!(matches!(
        World::from_definition(definition_for(oversized)),
        Err(WorldBuildError::InvalidItemPrototype(_))
    ));

    let mut control = item_prototype("test:item", "safe", "Safe.", &["item"]);
    control.description = "unsafe\u{1b}line".into();
    assert!(matches!(
        World::from_definition(definition_for(control)),
        Err(WorldBuildError::InvalidItemPrototype(_))
    ));

    let mut aliases = item_prototype("test:item", "safe", "Safe.", &["item"]);
    aliases.aliases = (0..33).map(|index| format!("alias-{index}")).collect();
    assert!(matches!(
        World::from_definition(definition_for(aliases)),
        Err(WorldBuildError::InvalidItemPrototype(_))
    ));

    let mut multiline = item_prototype("test:item", "safe", "Safe.", &["item"]);
    multiline.description = "First line.\n\tSecond line.".into();
    assert!(World::from_definition(definition_for(multiline)).is_ok());
}

fn full_scan_location_index(world: &World) -> BTreeMap<SharedItemLocation, Vec<ItemInstanceId>> {
    let mut expected = BTreeMap::<SharedItemLocation, Vec<ItemInstanceId>>::new();
    for (item_id, instance) in world.item_instances.iter() {
        expected
            .entry(instance.location.clone())
            .or_default()
            .push(item_id.clone());
    }
    expected
}

fn assert_location_index_matches_full_scan(world: &World, locations: &[SharedItemLocation]) {
    let expected = full_scan_location_index(world);
    for location in locations.iter().chain(expected.keys()) {
        let indexed = world
            .item_instances
            .ids_at(location)
            .cloned()
            .collect::<Vec<_>>();
        assert_eq!(
            indexed,
            expected.get(location).cloned().unwrap_or_default(),
            "{location:?}"
        );
    }
    let full = build_item_graph_index(&world.item_instances, &world.item_prototypes).unwrap();
    let roots = world
        .item_instances
        .iter()
        .filter(|(_, instance)| !matches!(instance.location, SharedItemLocation::Contained(_)))
        .map(|(item_id, _)| item_id)
        .collect::<Vec<_>>();
    let partial =
        build_item_subgraph_index(&world.item_instances, &world.item_prototypes, roots).unwrap();
    assert_eq!(partial.subtree_weights, full.subtree_weights);
    assert_eq!(partial.subtree_transferable, full.subtree_transferable);
    assert_eq!(partial.children, full.children);
    for character_id in [CharacterId(1), CharacterId(2)] {
        assert_eq!(
            world.inventory_totals(character_id).unwrap(),
            character_inventory_totals_with_graph(
                character_id,
                &world.item_instances,
                &world.item_prototypes,
                &full,
            )
            .unwrap()
        );
    }
}

#[test]
fn item_location_index_matches_full_scan_after_random_operations() {
    let mut tonic = item_prototype("test:tonic", "healing tonic", "Restorative.", &["tonic"]);
    tonic.stack_policy = ItemStackPolicy::Fungible;
    tonic.max_stack_units = 5;
    tonic.weight_grams = 100;
    let mut satchel = item_prototype("test:satchel", "satchel", "A satchel.", &["satchel"]);
    satchel.weight_grams = 500;
    satchel.container = Some(ItemContainerDefinition {
        max_slots: 50,
        max_weight_grams: 100_000,
    });
    let room = |id: &str| RoomDefinition {
        stable_id: id.into(),
        title: id.into(),
        prose: vec!["A test room.".into()],
        image: None,
        audio: None,
        item_ids: Vec::new(),
        exits: Vec::new(),
    };
    let mut instances = Vec::new();
    for index in 0..6 {
        instances.push(item_instance(
            &format!("test:satchel-{index}"),
            "test:satchel",
            InitialItemLocation::Room("test:a".into()),
        ));
    }
    for index in 0..40 {
        instances.push(item_instance(
            &format!("test:tonic-{index:02}"),
            "test:tonic",
            InitialItemLocation::Room(if index % 2 == 0 { "test:a" } else { "test:b" }.into()),
        ));
    }
    let mut world = World::from_definition(WorldDefinition {
        start_room_id: "test:a".into(),
        item_prototypes: vec![tonic, satchel],
        initial_item_instances: instances,
        rooms: vec![room("test:a"), room("test:b")],
    })
    .unwrap();
    let satchels = (0..6)
        .map(|index| ItemInstanceId(format!("test:satchel-{index}")))
        .collect::<Vec<_>>();
    let tonics = (0..40)
        .map(|index| ItemInstanceId(format!("test:tonic-{index:02}")))
        .collect::<Vec<_>>();
    let mut top_level = vec![
        SharedItemLocation::Room("test:a".into()),
        SharedItemLocation::Room("test:b".into()),
        SharedItemLocation::Carried(CharacterId(1)),
        SharedItemLocation::Carried(CharacterId(2)),
        SharedItemLocation::Equipped(CharacterId(1), EquipmentSlot::Weapon),
        SharedItemLocation::HeldByNpc("test:npc".into()),
        SharedItemLocation::Staged,
        SharedItemLocation::Consumed,
    ];
    let mut every_location = top_level.clone();
    every_location.extend(
        satchels
            .iter()
            .map(|satchel| SharedItemLocation::Contained(satchel.clone())),
    );
    let mut state = 0x9e37_79b9_7f4a_7c15_u64;
    let mut next = move |bound: usize| {
        state ^= state << 13;
        state ^= state >> 7;
        state ^= state << 17;
        (state % bound as u64) as usize
    };
    let mut checkpoints = Vec::new();
    assert_location_index_matches_full_scan(&world, &every_location);
    for step in 0..600 {
        match next(10) {
            // Move a tonic anywhere, including into a satchel.
            0..=4 => {
                let tonic = &tonics[next(tonics.len())];
                let destination = every_location[next(every_location.len())].clone();
                world.item_instances.get_mut(tonic).unwrap().location = destination;
            }
            // Containers only move between top-level locations, so the
            // graph stays acyclic and within the depth bound.
            5 | 6 => {
                let satchel = &satchels[next(satchels.len())];
                let destination = top_level[next(top_level.len())].clone();
                world.item_instances.get_mut(satchel).unwrap().location = destination;
            }
            7 => checkpoints.push(world.checkpoint()),
            8 => {
                if let Some(checkpoint) = checkpoints.pop() {
                    world.restore_checkpoint(checkpoint);
                }
            }
            _ => {
                // Prospective-copy pattern used by the mutation helpers.
                let mut prospective = world.item_instances.clone();
                let tonic = &tonics[next(tonics.len())];
                prospective.get_mut(tonic).unwrap().location =
                    SharedItemLocation::Carried(CharacterId(2));
                if next(2) == 0 {
                    world.item_instances = prospective;
                }
            }
        }
        if step % 97 == 0 {
            top_level.rotate_left(1);
        }
        assert_location_index_matches_full_scan(&world, &every_location);
    }
}
