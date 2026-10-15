use super::*;

#[test]
fn portable_snapshots_resume_combat_loot_xp_equipment_effects_and_rng() {
    let (mut original, session) = phase_two_world(41, 2, 3);
    original
        .apply(session, PlayerAction::Take("potion".into()))
        .unwrap();
    original
        .apply(session, PlayerAction::Use("potion".into()))
        .unwrap();
    original
        .apply(session, PlayerAction::Take("sword".into()))
        .unwrap();
    original
        .apply(session, PlayerAction::Equip("sword".into()))
        .unwrap();
    original
        .apply(session, PlayerAction::Move(Direction::North))
        .unwrap();
    original
        .apply(session, PlayerAction::Attack("guardian".into()))
        .unwrap();
    original.advance_tick().unwrap();

    let runtime_json = serde_json::to_string(&original.runtime_snapshot()).unwrap();
    let durable_json =
        serde_json::to_string(&original.durable_character_snapshot(session).unwrap()).unwrap();
    let runtime: WorldRuntimeSnapshot = serde_json::from_str(&runtime_json).unwrap();
    let durable: DurableCharacterSnapshot = serde_json::from_str(&durable_json).unwrap();

    let (mut resumed, resumed_session) = phase_two_world(999, 2, 3);
    let revision = runtime.revision;
    resumed.restore_runtime_snapshot(runtime).unwrap();
    resumed
        .restore_character(resumed_session, &durable.character, revision)
        .unwrap();
    resumed
        .restore_character_gameplay(CharacterId(900), durable.gameplay)
        .unwrap();

    assert_eq!(resumed.runtime_snapshot(), original.runtime_snapshot());
    assert_eq!(
        resumed
            .character_gameplay_snapshot(resumed_session)
            .unwrap(),
        original.character_gameplay_snapshot(session).unwrap()
    );
    let resumed_inventory = resumed.inventory_as_plain_text(resumed_session).unwrap();
    let original_inventory = original.inventory_as_plain_text(session).unwrap();
    assert_ne!(resumed_inventory, original_inventory);
    assert_eq!(
        redact_and_validate_opaque_item_refs(&resumed_inventory),
        redact_and_validate_opaque_item_refs(&original_inventory)
    );
    assert_eq!(
        resumed.advance_tick().unwrap(),
        original.advance_tick().unwrap()
    );
}

#[test]
fn portable_snapshots_resume_defeat_recovery_and_npc_respawn_ticks() {
    let (mut original, session) = phase_two_world(5, 50, 100);
    original
        .apply(session, PlayerAction::Move(Direction::North))
        .unwrap();
    original
        .apply(session, PlayerAction::Attack("guardian".into()))
        .unwrap();
    original.advance_tick().unwrap();
    assert!(
        original
            .character_gameplay_snapshot(session)
            .unwrap()
            .recovery_at_tick
            .is_some()
    );

    let runtime = original.runtime_snapshot();
    let durable = original.durable_character_snapshot(session).unwrap();
    let (mut resumed, resumed_session) = phase_two_world(999, 50, 100);
    let revision = runtime.revision;
    resumed.restore_runtime_snapshot(runtime).unwrap();
    resumed
        .restore_character(resumed_session, &durable.character, revision)
        .unwrap();
    resumed
        .restore_character_gameplay(CharacterId(900), durable.gameplay)
        .unwrap();

    for _ in 0..4 {
        assert_eq!(
            resumed.advance_tick().unwrap(),
            original.advance_tick().unwrap()
        );
    }
    assert_eq!(
        resumed
            .character_gameplay_snapshot(resumed_session)
            .unwrap(),
        original.character_gameplay_snapshot(session).unwrap()
    );
    assert_eq!(
        resumed.scene_for(resumed_session).unwrap().title,
        "Sanctuary"
    );
}

#[test]
fn checkpoint_restores_all_gameplay_and_observer_state() {
    let (mut world, session) = phase_two_world(17, 2, 20);
    world
        .apply(session, PlayerAction::Take("potion".into()))
        .unwrap();
    let before_runtime = world.runtime_snapshot();
    let before_gameplay = world.character_gameplay_snapshot(session).unwrap();
    let before_scene = world.scene_for(session).unwrap();
    let checkpoint = world.checkpoint();

    world
        .apply(session, PlayerAction::Use("potion".into()))
        .unwrap();
    world.advance_tick().unwrap();
    world.restore_checkpoint(checkpoint);

    assert_eq!(world.runtime_snapshot(), before_runtime);
    assert_eq!(
        world.character_gameplay_snapshot(session).unwrap(),
        before_gameplay
    );
    let restored_scene = world.scene_for(session).unwrap();
    assert_eq!(restored_scene.view_id, before_scene.view_id);
    assert_eq!(restored_scene.revision, before_scene.revision);
    assert_eq!(restored_scene.title, before_scene.title);
    assert_eq!(restored_scene.items, before_scene.items);
    assert_eq!(restored_scene.npcs, before_scene.npcs);
}

#[test]
fn snapshot_v1_upgrade_event_json_and_allocator_remain_compatible() {
    let world = World::demo();
    let current = world.runtime_snapshot();
    let legacy = WorldRuntimeSnapshotV1 {
        snapshot_version: 1,
        revision: current.revision,
        logical_tick: current.logical_tick,
        rng_state: current.rng_state,
        item_locations: current
            .item_instances
            .iter()
            .map(|(id, instance)| {
                let location = match &instance.location {
                    RuntimeItemLocation::Staged => RuntimeItemLocationV2::Staged,
                    RuntimeItemLocation::Carried { character_id } => {
                        RuntimeItemLocationV2::Carried {
                            character_id: *character_id,
                        }
                    }
                    RuntimeItemLocation::Room { room_id } => RuntimeItemLocationV2::Room {
                        room_id: room_id.clone(),
                    },
                    RuntimeItemLocation::Equipped { character_id, slot } => {
                        RuntimeItemLocationV2::Equipped {
                            character_id: *character_id,
                            slot: *slot,
                        }
                    }
                    RuntimeItemLocation::HeldByNpc { npc_id } => RuntimeItemLocationV2::HeldByNpc {
                        npc_id: npc_id.clone(),
                    },
                    RuntimeItemLocation::Consumed => RuntimeItemLocationV2::Consumed,
                    RuntimeItemLocation::Contained { .. } => {
                        panic!("v1 fixture must not contain nested items")
                    }
                };
                (id.0.clone(), location)
            })
            .collect(),
        npcs: current.npcs,
        adventure: current.adventure,
        progression_flags: current.progression_flags,
    };
    let encoded = serde_json::to_string(&legacy).unwrap();
    let compatible: CompatibleWorldRuntimeSnapshot = serde_json::from_str(&encoded).unwrap();
    let upgraded = compatible.into_current().unwrap();
    assert_eq!(upgraded.snapshot_version, WORLD_RUNTIME_SNAPSHOT_VERSION);
    assert_eq!(upgraded.item_instances.len(), 1);
    assert_eq!(
        upgraded.item_instances[&ItemInstanceId("demo:blue-lantern".into())].prototype_id,
        "demo:blue-lantern"
    );

    let event = DomainEvent::ItemTaken {
        actor_id: CharacterId(1),
        actor_name: "Carrier".into(),
        item_id: ItemInstanceId("demo:blue-lantern".into()),
        item_name: "blue-glass lantern".into(),
        room_id: "demo:lantern-square".into(),
    };
    let event_json = serde_json::to_string(&event).unwrap();
    assert!(
        event_json.contains("\"item_id\":\"demo:blue-lantern\""),
        "{event_json}"
    );
    assert_eq!(
        serde_json::from_str::<DomainEvent>(&event_json).unwrap(),
        event
    );

    let mut collision_prototype = item_prototype(
        "runtime:1",
        "legacy token",
        "Legacy runtime-looking identity.",
        &["token"],
    );
    collision_prototype.weight_grams = 0;
    let mut collision_world = World::from_definition(WorldDefinition {
        start_room_id: "test:start".into(),
        item_prototypes: vec![collision_prototype],
        initial_item_instances: vec![item_instance(
            "runtime:1",
            "runtime:1",
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
    })
    .unwrap();
    let before = collision_world.runtime_snapshot();
    let checkpoint = collision_world.checkpoint();
    assert_eq!(
        collision_world
            .create_staged_item_instance("runtime:1")
            .unwrap(),
        ItemInstanceId("runtime:2".into())
    );
    collision_world.restore_checkpoint(checkpoint);
    assert_eq!(collision_world.runtime_snapshot(), before);
}

#[test]
fn runtime_restore_merges_new_authored_instances_and_rejects_identity_drift() {
    let prototypes = || {
        vec![
            item_prototype("test:token", "token", "A token.", &["token"]),
            item_prototype("test:other", "other", "Another token.", &["other"]),
        ]
    };
    let room = || RoomDefinition {
        stable_id: "test:start".into(),
        title: "Start".into(),
        prose: Vec::new(),
        image: None,
        audio: None,
        exits: Vec::new(),
        item_ids: Vec::new(),
    };
    let build = |include_new: bool| {
        let mut instances = vec![item_instance(
            "test:token:one",
            "test:token",
            InitialItemLocation::Room("test:start".into()),
        )];
        if include_new {
            instances.push(item_instance(
                "test:token:two",
                "test:token",
                InitialItemLocation::Room("test:start".into()),
            ));
        }
        World::from_definition(WorldDefinition {
            start_room_id: "test:start".into(),
            item_prototypes: prototypes(),
            initial_item_instances: instances,
            rooms: vec![room()],
        })
        .unwrap()
    };

    let old_snapshot = build(false).runtime_snapshot();
    let mut upgraded_world = build(true);
    upgraded_world
        .restore_runtime_snapshot(old_snapshot.clone())
        .unwrap();
    assert!(
        upgraded_world
            .runtime_snapshot()
            .item_instances
            .contains_key(&ItemInstanceId("test:token:two".into()))
    );

    let mut retargeted = old_snapshot.clone();
    retargeted
        .item_instances
        .get_mut(&ItemInstanceId("test:token:one".into()))
        .unwrap()
        .prototype_id = "test:other".into();
    assert!(matches!(
        build(true).restore_runtime_snapshot(retargeted),
        Err(WorldError::InvalidRuntimeSnapshot)
    ));

    let mut foreign_id = old_snapshot.clone();
    foreign_id.item_instances.insert(
        ItemInstanceId("phantom:anything".into()),
        RuntimeItemInstanceSnapshot {
            prototype_id: "test:token".into(),
            location: RuntimeItemLocation::Staged,
            binding: ItemBinding::Unbound,
            item_affixes: Vec::new(),
        },
    );
    assert!(matches!(
        build(true).restore_runtime_snapshot(foreign_id),
        Err(WorldError::InvalidRuntimeSnapshot)
    ));

    let mut runtime_authored = World::from_definition(WorldDefinition {
        start_room_id: "test:start".into(),
        item_prototypes: prototypes(),
        initial_item_instances: vec![
            item_instance(
                "test:token:one",
                "test:token",
                InitialItemLocation::Room("test:start".into()),
            ),
            item_instance("runtime:100", "test:token", InitialItemLocation::Staged),
        ],
        rooms: vec![room()],
    })
    .unwrap();
    runtime_authored
        .restore_runtime_snapshot(old_snapshot.clone())
        .unwrap();
    assert_eq!(
        runtime_authored
            .create_staged_item_instance("test:token")
            .unwrap(),
        ItemInstanceId("runtime:101".into())
    );

    let mut stale_sequence = old_snapshot;
    stale_sequence.item_instances.insert(
        ItemInstanceId("runtime:100".into()),
        RuntimeItemInstanceSnapshot {
            prototype_id: "test:token".into(),
            location: RuntimeItemLocation::Staged,
            binding: ItemBinding::Unbound,
            item_affixes: Vec::new(),
        },
    );
    stale_sequence.next_item_instance_sequence = 100;
    assert!(matches!(
        build(true).restore_runtime_snapshot(stale_sequence),
        Err(WorldError::InvalidRuntimeSnapshot)
    ));
}

#[test]
fn weather_timer_is_deterministic_and_survives_runtime_restore() {
    let (mut first, _) = connected_world();
    let (mut second, _) = connected_world();
    first.set_rng_seed(777);
    second.set_rng_seed(777);
    for _ in 0..5 {
        assert_eq!(
            first.advance_tick().unwrap(),
            second.advance_tick().unwrap()
        );
    }
    assert_ne!(
        first.runtime_snapshot().adventure.weather,
        WeatherKind::Rain
    );

    let snapshot = first.runtime_snapshot();
    let (mut restored, _) = connected_world();
    restored.restore_runtime_snapshot(snapshot).unwrap();
    assert_eq!(
        restored.runtime_snapshot().adventure,
        first.runtime_snapshot().adventure
    );
}

#[test]
fn snapshot_decoder_is_version_directed_and_rejects_v2_v3_smuggling() {
    let (world, _) = connected_world();
    let json = serde_json::to_string(&world.runtime_snapshot()).unwrap();
    assert_eq!(
        decode_world_runtime_snapshot(WORLD_RUNTIME_SNAPSHOT_VERSION, &json)
            .unwrap()
            .snapshot_version,
        WORLD_RUNTIME_SNAPSHOT_VERSION
    );
    assert!(decode_world_runtime_snapshot(4, &json).is_err());
    let legacy_v3 = runtime_snapshot_json_for_version(&world.runtime_snapshot(), 3);
    let legacy_v3_json = serde_json::to_string(&legacy_v3).unwrap();
    assert_eq!(
        decode_world_runtime_snapshot(3, &legacy_v3_json)
            .unwrap()
            .snapshot_version,
        WORLD_RUNTIME_SNAPSHOT_VERSION
    );
    let mut v3_binding_smuggling = legacy_v3;
    v3_binding_smuggling["itemInstances"]
        .as_object_mut()
        .unwrap()
        .values_mut()
        .next()
        .unwrap()["binding"] = serde_json::json!({"kind": "unbound"});
    assert!(
        decode_world_runtime_snapshot(3, &serde_json::to_string(&v3_binding_smuggling).unwrap())
            .is_err()
    );
    let mut smuggled: serde_json::Value = serde_json::from_str(&json).unwrap();
    smuggled["snapshotVersion"] = serde_json::json!(2);
    let instance = smuggled["itemInstances"]
        .as_object_mut()
        .unwrap()
        .values_mut()
        .next()
        .unwrap();
    instance["location"] = serde_json::json!({
        "kind": "contained",
        "containerId": "demo:blue-lantern"
    });
    let smuggled = serde_json::to_string(&smuggled).unwrap();
    assert!(decode_world_runtime_snapshot(2, &smuggled).is_err());

    let mut unknown_root: serde_json::Value = serde_json::from_str(&json).unwrap();
    unknown_root["quantity"] = serde_json::json!(1);
    assert!(
        decode_world_runtime_snapshot(6, &serde_json::to_string(&unknown_root).unwrap()).is_err()
    );

    let mut unknown_instance: serde_json::Value = serde_json::from_str(&json).unwrap();
    let instance = unknown_instance["itemInstances"]
        .as_object_mut()
        .unwrap()
        .values_mut()
        .next()
        .unwrap();
    instance["quantity"] = serde_json::json!(1);
    assert!(
        decode_world_runtime_snapshot(6, &serde_json::to_string(&unknown_instance).unwrap())
            .is_err()
    );

    let mut unknown_location: serde_json::Value = serde_json::from_str(&json).unwrap();
    let location = &mut unknown_location["itemInstances"]
        .as_object_mut()
        .unwrap()
        .values_mut()
        .next()
        .unwrap()["location"];
    location["containerId"] = serde_json::json!("demo:blue-lantern");
    assert!(
        decode_world_runtime_snapshot(6, &serde_json::to_string(&unknown_location).unwrap())
            .is_err()
    );

    let legacy_v4 = runtime_snapshot_json_for_version(&world.runtime_snapshot(), 4);
    let upgraded =
        decode_world_runtime_snapshot(4, &serde_json::to_string(&legacy_v4).unwrap()).unwrap();
    assert_eq!(upgraded.snapshot_version, WORLD_RUNTIME_SNAPSHOT_VERSION);
    assert_eq!(upgraded.next_trade_id, TradeId(1));
    assert!(upgraded.trades.is_empty());
    let mut smuggled_v4 = legacy_v4;
    smuggled_v4["trades"] = serde_json::json!({});
    assert!(
        decode_world_runtime_snapshot(4, &serde_json::to_string(&smuggled_v4).unwrap()).is_err()
    );

    let mut missing_trade_fields: serde_json::Value = serde_json::from_str(&json).unwrap();
    missing_trade_fields
        .as_object_mut()
        .unwrap()
        .remove("trades");
    assert!(
        decode_world_runtime_snapshot(6, &serde_json::to_string(&missing_trade_fields).unwrap())
            .is_err()
    );
    let mut nested_trade = runtime_snapshot_json_for_version(&world.runtime_snapshot(), 6);
    nested_trade["nextTradeId"] = serde_json::json!(2);
    nested_trade["trades"]["1"] = serde_json::to_value(TradeSnapshot {
        id: TradeId(1),
        requester: CharacterId(101),
        recipient: CharacterId(102),
        room_id: world.start_room_id().into(),
        phase: TradePhase::Requested,
        created_tick: 0,
        opened_tick: None,
        expires_at_tick: TRADE_REQUEST_TICKS,
        state_revision: 1,
        offers: BTreeMap::new(),
        confirmations: BTreeMap::new(),
    })
    .unwrap();
    assert!(
        decode_world_runtime_snapshot(6, &serde_json::to_string(&nested_trade).unwrap()).is_ok()
    );
    nested_trade["trades"]["1"]["futureTradeField"] = serde_json::json!(true);
    assert!(
        decode_world_runtime_snapshot(6, &serde_json::to_string(&nested_trade).unwrap()).is_err()
    );

    assert!(valid_item_instance_id("runtime:1"));
    assert!(valid_item_instance_id("runtime:18446744073709551615"));
    assert!(valid_item_instance_id(&"i".repeat(128)));
    for invalid_id in [
        "runtime:",
        "runtime:0",
        "runtime:01",
        "runtime:-1",
        "runtime:+1",
        "runtime:18446744073709551616",
        &"i".repeat(129),
    ] {
        assert!(!valid_item_instance_id(invalid_id), "{invalid_id}");
    }
}

#[test]
fn runtime_snapshot_nested_records_reject_unknown_fields_across_versions() {
    let mut world = World::demo();
    let room_id = world.start_room_id().to_owned();
    world
        .spawn_npc(NpcDefinition {
            stable_id: "test:snapshot-npc".into(),
            name: "snapshot sentinel".into(),
            description: "An NPC retained only to exercise the durable snapshot shape.".into(),
            room_id,
            level: 1,
            max_health: 10,
            strength: 1,
            defence: 0,
            experience_reward: 1,
            loot_item_ids: Vec::new(),
            respawn_ticks: 2,
        })
        .unwrap();
    let snapshot = world.runtime_snapshot();
    for version in 1..=WORLD_RUNTIME_SNAPSHOT_VERSION {
        let clean = runtime_snapshot_json_for_version(&snapshot, version);
        let clean_json = serde_json::to_string(&clean).unwrap();
        assert!(
            decode_world_runtime_snapshot(version, &clean_json).is_ok(),
            "valid v{version} fixture must decode"
        );

        let mut npc_smuggling = clean.clone();
        first_runtime_npc(&mut npc_smuggling)["futureNpcField"] = serde_json::json!(true);
        assert!(
            decode_world_runtime_snapshot(version, &serde_json::to_string(&npc_smuggling).unwrap())
                .is_err(),
            "v{version} must reject an unknown NPC field"
        );

        let mut adventure_smuggling = clean.clone();
        adventure_smuggling["adventure"]["futureAdventureField"] = serde_json::json!(true);
        assert!(
            decode_world_runtime_snapshot(
                version,
                &serde_json::to_string(&adventure_smuggling).unwrap()
            )
            .is_err(),
            "v{version} must reject an unknown adventure field"
        );

        let mut party_smuggling = clean.clone();
        party_smuggling["adventure"]["parties"]["77"] = serde_json::json!({
            "id": 77,
            "leader": 701,
            "members": [701],
            "invitations": {},
            "futurePartyField": true
        });
        assert!(
            decode_world_runtime_snapshot(
                version,
                &serde_json::to_string(&party_smuggling).unwrap()
            )
            .is_err(),
            "v{version} must reject an unknown party field"
        );

        let mut resource_smuggling = clean.clone();
        first_runtime_npc(&mut resource_smuggling)["resources"]["test:focus"] = serde_json::json!({
            "current": 1,
            "max": 2,
            "futureResourceField": true
        });
        assert!(
            decode_world_runtime_snapshot(
                version,
                &serde_json::to_string(&resource_smuggling).unwrap()
            )
            .is_err(),
            "v{version} must reject an unknown resource-pool field"
        );

        let mut status_smuggling = clean.clone();
        first_runtime_npc(&mut status_smuggling)["statuses"] = serde_json::json!([{
            "sourceAbilityId": "test:stun",
            "kind": "stun",
            "magnitude": 1,
            "remainingTicks": 1,
            "futureStatusField": true
        }]);
        assert!(
            decode_world_runtime_snapshot(
                version,
                &serde_json::to_string(&status_smuggling).unwrap()
            )
            .is_err(),
            "v{version} must reject an unknown active-status field"
        );

        let mut legacy_defaults = clean;
        legacy_defaults.as_object_mut().unwrap().remove("adventure");
        let npc = first_runtime_npc(&mut legacy_defaults)
            .as_object_mut()
            .unwrap();
        for optional in [
            "resources",
            "resourceRegeneration",
            "abilityIds",
            "abilityCooldowns",
            "resistances",
            "statuses",
        ] {
            npc.remove(optional);
        }
        assert!(
            decode_world_runtime_snapshot(
                version,
                &serde_json::to_string(&legacy_defaults).unwrap()
            )
            .is_ok(),
            "v{version} must retain defaults for absent additive fields"
        );
    }
}

#[test]
fn snapshot_v4_requires_valid_binding_and_rejects_policy_drift() {
    let (mut world, actor) = connect_definition(policy_definition());
    world
        .apply(actor, PlayerAction::Take(ItemSelection::one("bag")))
        .unwrap();
    let snapshot = world.runtime_snapshot();
    let encoded = serde_json::to_string(&snapshot).unwrap();
    assert!(encoded.contains("\"snapshotVersion\":9"));
    assert!(encoded.contains("\"binding\":{\"kind\":\"character\",\"characterId\":701}"));
    let mut restored = World::from_definition(policy_definition()).unwrap();
    restored.restore_runtime_snapshot(snapshot.clone()).unwrap();
    assert!(
        restored
            .runtime_item_owner_ids()
            .contains(&CharacterId(701))
    );

    let clasp_id = ItemInstanceId("policy:clasp-instance".into());
    let mut zero = snapshot.clone();
    zero.item_instances.get_mut(&clasp_id).unwrap().binding = ItemBinding::Character {
        character_id: CharacterId(0),
    };
    assert!(matches!(
        World::from_definition(policy_definition())
            .unwrap()
            .restore_runtime_snapshot(zero),
        Err(WorldError::InvalidRuntimeSnapshot)
    ));
    let mut zero_location = snapshot.clone();
    zero_location
        .item_instances
        .get_mut(&ItemInstanceId("policy:bag-instance".into()))
        .unwrap()
        .location = RuntimeItemLocation::Carried {
        character_id: CharacterId(0),
    };
    assert!(matches!(
        World::from_definition(policy_definition())
            .unwrap()
            .restore_runtime_snapshot(zero_location),
        Err(WorldError::InvalidRuntimeSnapshot)
    ));
    let mut unbound = snapshot.clone();
    unbound.item_instances.get_mut(&clasp_id).unwrap().binding = ItemBinding::Unbound;
    assert!(matches!(
        World::from_definition(policy_definition())
            .unwrap()
            .restore_runtime_snapshot(unbound),
        Err(WorldError::InvalidRuntimeSnapshot)
    ));
    let mut escaped = snapshot.clone();
    escaped.item_instances.get_mut(&clasp_id).unwrap().location = RuntimeItemLocation::Room {
        room_id: "test:start".into(),
    };
    assert!(matches!(
        World::from_definition(policy_definition())
            .unwrap()
            .restore_runtime_snapshot(escaped),
        Err(WorldError::InvalidRuntimeSnapshot)
    ));
    let mut forged_transferable = snapshot.clone();
    forged_transferable
        .item_instances
        .get_mut(&ItemInstanceId("policy:bag-instance".into()))
        .unwrap()
        .binding = ItemBinding::Character {
        character_id: CharacterId(701),
    };
    assert!(matches!(
        World::from_definition(policy_definition())
            .unwrap()
            .restore_runtime_snapshot(forged_transferable),
        Err(WorldError::InvalidRuntimeSnapshot)
    ));
    let mut drifted_definition = policy_definition();
    drifted_definition
        .item_prototypes
        .iter_mut()
        .find(|prototype| prototype.prototype_id == "policy:clasp")
        .unwrap()
        .transfer_policy = ItemTransferPolicy::Transferable;
    assert!(matches!(
        World::from_definition(drifted_definition)
            .unwrap()
            .restore_runtime_snapshot(snapshot),
        Err(WorldError::InvalidRuntimeSnapshot)
    ));

    let mut missing_binding: serde_json::Value = serde_json::from_str(&encoded).unwrap();
    missing_binding["itemInstances"]
        .as_object_mut()
        .unwrap()
        .values_mut()
        .next()
        .unwrap()
        .as_object_mut()
        .unwrap()
        .remove("binding");
    assert!(
        decode_world_runtime_snapshot(4, &serde_json::to_string(&missing_binding).unwrap())
            .is_err()
    );
}

#[test]
fn snapshot_v9_round_trips_and_decodes_at_current_version() {
    let (world, _) = connected_world();
    let snapshot = world.runtime_snapshot();
    assert_eq!(snapshot.snapshot_version, WORLD_RUNTIME_SNAPSHOT_VERSION);
    assert_eq!(WORLD_RUNTIME_SNAPSHOT_VERSION, 9);
    let json = serde_json::to_string(&snapshot).unwrap();
    // A direct serde round-trip is lossless.
    let restored: WorldRuntimeSnapshot = serde_json::from_str(&json).unwrap();
    assert_eq!(restored, snapshot);
    // The version-directed decoder accepts the current payload verbatim.
    let decoded = decode_world_runtime_snapshot(WORLD_RUNTIME_SNAPSHOT_VERSION, &json).unwrap();
    assert_eq!(decoded, snapshot);
    // A column/payload version disagreement still fails closed.
    assert!(decode_world_runtime_snapshot(8, &json).is_err());
}

#[test]
fn snapshot_upgrade_v8_normalizes_loot_distribution_and_preserves_state() {
    // The v8 -> v9 loot-encounter conversion defaults to round-robin with no
    // declarations or assignments.
    let converted: LootEncounter = LootEncounterV8 {
        npc_spawn_id: "game:guardian".into(),
        initiator: CharacterId(910),
        party_id: Some(5),
        cohort: vec![CharacterId(910), CharacterId(911)],
        room_id: "game:arena".into(),
        started_tick: 3,
    }
    .into();
    assert_eq!(
        converted.distribution,
        loot_distribution::LootDistributionMode::RoundRobin
    );
    assert!(converted.loot_declarations.is_empty());
    assert!(converted.master_assignments.is_empty());
    assert_eq!(converted.initiator, CharacterId(910));
    assert_eq!(converted.cohort, vec![CharacterId(910), CharacterId(911)]);

    // An exact v8 payload upgrades to v9 with the rest of the state intact.
    let (world, _) = connected_world();
    let snapshot = world.runtime_snapshot();
    let legacy_v8 = runtime_snapshot_json_for_version(&snapshot, 8);
    let upgraded =
        decode_world_runtime_snapshot(8, &serde_json::to_string(&legacy_v8).unwrap()).unwrap();
    assert_eq!(upgraded.snapshot_version, WORLD_RUNTIME_SNAPSHOT_VERSION);
    assert!(upgraded.loot_encounters.values().all(|encounter| {
        encounter.distribution == loot_distribution::LootDistributionMode::RoundRobin
            && encounter.loot_declarations.is_empty()
            && encounter.master_assignments.is_empty()
    }));
    assert_eq!(upgraded.item_instances, snapshot.item_instances);
    assert_eq!(upgraded.item_provenance, snapshot.item_provenance);
    assert_eq!(upgraded.npcs, snapshot.npcs);
    assert_eq!(upgraded.revision, snapshot.revision);
    assert_eq!(upgraded.rng_state, snapshot.rng_state);
    assert_eq!(upgraded.area_residency, snapshot.area_residency);
    assert_eq!(upgraded, snapshot);
}

#[test]
fn snapshot_v8_rejects_smuggled_v9_loot_distribution_field() {
    let (world, _) = connected_world();
    let snapshot = world.runtime_snapshot();
    // A well-formed v8 loot encounter carrying the v9 `distribution` field.
    let smuggled_encounter = serde_json::json!({
        "npcSpawnId": "ghost:target",
        "initiator": 1,
        "cohort": [1],
        "roomId": "ghost:room",
        "startedTick": 0,
        "distribution": "need_greed",
    });
    // Declared as v8, the field is unknown and deserialization fails closed
    // before any semantic validation runs.
    let mut v8 = runtime_snapshot_json_for_version(&snapshot, 8);
    v8["lootEncounters"]["ghost:target"] = smuggled_encounter.clone();
    assert!(decode_world_runtime_snapshot(8, &serde_json::to_string(&v8).unwrap()).is_err());
    // The same field is legitimate under v9, proving the rejection is
    // version-specific rather than a generic parse failure.
    let mut v9 = serde_json::to_value(&snapshot).unwrap();
    v9["lootEncounters"]["ghost:target"] = smuggled_encounter;
    let decoded = decode_world_runtime_snapshot(
        WORLD_RUNTIME_SNAPSHOT_VERSION,
        &serde_json::to_string(&v9).unwrap(),
    )
    .unwrap();
    assert_eq!(
        decoded.loot_encounters["ghost:target"].distribution,
        loot_distribution::LootDistributionMode::NeedGreed
    );
}

#[test]
fn near_limit_v5_snapshot_upgrades_losslessly_below_the_v6_byte_budget() {
    let (mut world, _) = phase_two_world(13, 1, 10);
    let mut legacy = runtime_snapshot_json_for_version(&world.runtime_snapshot(), 5);
    let mut encoded_len = serde_json::to_vec(&legacy).unwrap().len();
    while encoded_len < MAX_LEGACY_WORLD_RUNTIME_SNAPSHOT_BYTES - 40_000 {
        for _ in 0..100 {
            world.create_staged_item_instance("game:sword").unwrap();
        }
        legacy = runtime_snapshot_json_for_version(&world.runtime_snapshot(), 5);
        encoded_len = serde_json::to_vec(&legacy).unwrap().len();
    }
    while encoded_len < MAX_LEGACY_WORLD_RUNTIME_SNAPSHOT_BYTES - 8_000 {
        world.create_staged_item_instance("game:sword").unwrap();
        legacy = runtime_snapshot_json_for_version(&world.runtime_snapshot(), 5);
        encoded_len = serde_json::to_vec(&legacy).unwrap().len();
    }
    assert!(encoded_len <= MAX_LEGACY_WORLD_RUNTIME_SNAPSHOT_BYTES);
    let legacy_json = serde_json::to_string(&legacy).unwrap();
    let upgraded = decode_world_runtime_snapshot(5, &legacy_json).unwrap();
    assert_eq!(upgraded.snapshot_version, WORLD_RUNTIME_SNAPSHOT_VERSION);
    assert_eq!(
        upgraded.item_provenance.len(),
        upgraded.item_instances.len()
    );
    assert!(upgraded.item_provenance.values().all(|provenance| matches!(
        provenance,
        ItemProvenance::LegacySnapshot { source_version: 5 }
    )));
    assert!(serde_json::to_vec(&upgraded).unwrap().len() <= MAX_WORLD_RUNTIME_SNAPSHOT_BYTES);
}

#[test]
fn snapshot_v6_rejects_affix_and_sealed_provenance_smuggling() {
    let (world, _, _) = sealed_reward_world(11);
    let mut legacy = runtime_snapshot_json_for_version(&world.runtime_snapshot(), 6);
    legacy["itemProvenance"]
        .as_object_mut()
        .unwrap()
        .values_mut()
        .next()
        .unwrap()["sealedLootTableId"] = serde_json::json!("reward:cache-table");
    assert!(decode_world_runtime_snapshot(6, &legacy.to_string()).is_err());
    legacy["itemProvenance"]
        .as_object_mut()
        .unwrap()
        .values_mut()
        .next()
        .unwrap()
        .as_object_mut()
        .unwrap()
        .remove("sealedLootTableId");
    let instance = legacy["itemInstances"]
        .as_object_mut()
        .unwrap()
        .values_mut()
        .next()
        .unwrap();
    instance["itemAffixes"] = serde_json::json!([]);
    assert!(decode_world_runtime_snapshot(6, &legacy.to_string()).is_err());

    let mut forged = world.runtime_snapshot();
    let source = forged.item_instances.keys().next().unwrap().clone();
    forged.item_instances.get_mut(&source).unwrap().item_affixes = vec![AppliedItemAffix {
        affix_id: "reward:forged".into(),
        definition_digest_sha256: "0".repeat(64),
        pool_id: "reward:forged-pool".into(),
        pool_digest_sha256: "1".repeat(64),
        record_digest_sha256: "2".repeat(64),
        label: "forged".into(),
        position: AffixPosition::Prefix,
        attack_bonus_delta: 1,
        heal_amount_delta: 0,
    }];
    assert!(matches!(
        validate_world_runtime_snapshot_semantics(&forged),
        Err(WorldError::InvalidRuntimeSnapshot)
    ));
}

#[test]
fn cold_replay_fails_closed_for_every_snapshot_v7_loot_ledger_event() {
    let (mut world, _) = connected_world();
    let item_id = ItemInstanceId("demo:blue-lantern".into());
    let events = vec![
        GameplayEvent::LootGenerated {
            drop_id: LootDropId(1),
            npc_id: "demo:npc".into(),
            npc_name: "npc".into(),
            item_ids: vec![item_id.clone()],
            room_id: "demo:lantern-square".into(),
        },
        GameplayEvent::LootRightsChanged {
            item_id: item_id.clone(),
            room_id: "demo:lantern-square".into(),
            summary: "changed".into(),
        },
        GameplayEvent::LootGenerationSuppressed {
            drop_id: LootDropId(1),
            npc_id: "demo:npc".into(),
            npc_name: "npc".into(),
            reason: "capacity".into(),
            room_id: "demo:lantern-square".into(),
        },
        GameplayEvent::LootDespawned {
            item_ids: vec![item_id],
            room_id: "demo:lantern-square".into(),
        },
        GameplayEvent::LootEncounterCancelled {
            npc_id: "demo:npc".into(),
            npc_name: "npc".into(),
            room_id: "demo:lantern-square".into(),
        },
        GameplayEvent::RewardOpened {
            open_id: RewardOpenId(1),
            character_id: CharacterId(101),
            character_name: "Tester".into(),
            source_item_id: ItemInstanceId("runtime:1".into()),
            source_item_name: "sealed cache".into(),
            item_ids: vec![ItemInstanceId("runtime:2".into())],
            item_names: vec!["forged reward".into()],
            room_id: "demo:lantern-square".into(),
        },
    ];
    for event in events {
        assert!(matches!(
            world.replay_persisted_event(&DomainEvent::Gameplay { event }),
            Err(WorldError::InvalidDomainTransition)
        ));
    }
}
