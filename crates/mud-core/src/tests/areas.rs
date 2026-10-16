use super::*;

#[test]
fn deferred_portal_preflight_is_idempotent_secret_safe_and_non_mutating() {
    let generation = area_generation();
    let request = deferred_portal(
        &generation,
        "area-a:start",
        Direction::North,
        "area-b:start",
        "area-b",
        2,
    );
    let initial_batch = PreparedCoreAreaBatch {
        generation_id: generation.clone(),
        areas: vec![prepared_area(
            "area-a",
            1,
            vec![prepared_room(
                "area-a:start",
                [(Direction::North, "area-b:start")],
            )],
        )],
        deferred_exits: vec![request.clone()],
    };
    let duplicate_batch = initial_batch.clone();
    let (mut world, installed) =
        World::from_initial_prepared_areas("area-a:start", initial_batch).unwrap();
    assert_eq!(installed.installed_area_ids, vec!["area-a"]);
    assert_eq!(world.area_generation(), Some(&generation));
    assert_eq!(world.active_area_count(), 1);
    assert!(world.active_area_matches("area-a", 1, &format!("{:064x}", 1)));

    // A coalesced duplicate completion is a strict no-op even while its
    // outgoing portal remains deferred.
    assert_eq!(
        world.install_prepared_areas(duplicate_batch).unwrap(),
        CoreAreaInstallOutcome {
            installed_areas: 0,
            installed_area_ids: Vec::new(),
            created_dynamic_state: false,
        }
    );

    let session = SessionId(71);
    world
        .connect(
            session,
            CharacterSnapshot {
                id: CharacterId(71),
                account_id: AccountId(71),
                name: "Portal Tester".into(),
                room_id: "area-a:start".into(),
                item_locations: BTreeMap::new(),
            },
        )
        .unwrap();
    let before_character = world.character_snapshot(session).unwrap();
    let before_revision = world.revision();
    let before_view_revision = world.observer(session).unwrap().revision;
    let scene = world.scene_for(session).unwrap();
    assert_eq!(scene.exits.len(), 1);
    assert!(!scene.exits[0].id.contains("area-b"));
    let player = world.player_state_view(session).unwrap();
    assert_eq!(player.map.paths.len(), 1);
    assert_eq!(player.map.paths[0].to, None);
    assert_eq!(
        world
            .deferred_area_load_request(session, Direction::North)
            .unwrap(),
        Some(request.clone())
    );
    assert!(matches!(
        world.apply(session, PlayerAction::Move(Direction::North)),
        Err(WorldError::AreaLoadRequired(actual)) if actual == request
    ));
    assert_eq!(world.character_snapshot(session).unwrap(), before_character);
    assert_eq!(world.revision(), before_revision);
    assert_eq!(
        world.observer(session).unwrap().revision,
        before_view_revision
    );
    assert_eq!(world.shout_audience(session).unwrap().len(), 1);

    world
        .configure_secret_exit("area-a:start", Direction::North, 100)
        .unwrap();
    assert!(world.scene_for(session).unwrap().exits.is_empty());
    assert_eq!(
        world
            .deferred_area_load_request(session, Direction::North)
            .unwrap(),
        None
    );
}

#[test]
fn snapshot_v7_requires_an_explicit_catalog_manifest_but_monolithic_upgrade_is_empty() {
    let (world, _) = connected_world();
    let legacy = runtime_snapshot_json_for_version(&world.runtime_snapshot(), 7);
    let json = serde_json::to_string(&legacy).unwrap();
    let exact = decode_world_runtime_snapshot_v7(7, &json).unwrap();
    let monolithic = decode_world_runtime_snapshot(7, &json).unwrap();
    assert_eq!(monolithic.snapshot_version, WORLD_RUNTIME_SNAPSHOT_VERSION);
    assert!(monolithic.area_residency.is_empty());

    let manifest = BTreeMap::from([(
        "area-a".into(),
        AreaRuntimeResidence::Resident {
            revision: 1,
            bundle_sha256: "a".repeat(64),
            residency_epoch: 1,
        },
    )]);
    let catalog = WorldRuntimeSnapshot::upgrade_v7(exact.clone(), manifest.clone()).unwrap();
    assert_eq!(catalog.area_residency, manifest);
    let retired = BTreeMap::from([(
        "area-a".into(),
        AreaRuntimeResidence::Retired {
            revision: 1,
            bundle_sha256: "a".repeat(64),
            residency_epoch: 2,
            checkpoint_version: AREA_DYNAMIC_CHECKPOINT_VERSION,
            checkpoint_sha256: "b".repeat(64),
        },
    )]);
    assert!(WorldRuntimeSnapshot::upgrade_v7(exact, retired).is_err());
}

#[test]
fn area_retirement_is_partitioned_restart_safe_and_rehydrates_without_global_rewind() {
    let (mut world, rehydrate_batch, unit, _) = retirement_fixture();
    let carried_id = ItemInstanceId("area-b:carried-instance".into());
    let chest_id = ItemInstanceId("area-b:chest-instance".into());
    let child_id = ItemInstanceId("area-b:child-instance".into());
    let npc_item_id = ItemInstanceId("area-b:npc-token-instance".into());
    let npc = world.npcs.get_mut("area-b:keeper").unwrap();
    npc.health = 0;
    npc.respawn_at_tick = Some(1);

    let prepared = world.prepare_area_retirement(unit.clone()).unwrap();
    assert!(prepared.checkpoint.item_instances.contains_key(&chest_id));
    assert!(prepared.checkpoint.item_instances.contains_key(&child_id));
    assert!(
        prepared
            .checkpoint
            .item_instances
            .contains_key(&npc_item_id)
    );
    assert!(!prepared.checkpoint.item_instances.contains_key(&carried_id));
    assert!(prepared.checkpoint.ground_loot.contains_key(&chest_id));
    assert_eq!(prepared.checkpoint.npcs["area-b:keeper"].health, 0);
    let checkpoint = prepared.checkpoint.clone();
    let checkpoint_sha256 = canonical_sha256(&checkpoint).unwrap();
    let before_retirement = (
        world.revision,
        world.logical_tick,
        world.rng.state,
        world.next_item_instance_sequence,
        world.next_loot_drop_id,
        world.next_reward_open_id,
    );
    world
        .apply_prepared_area_retirement(prepared, checkpoint_sha256.clone())
        .unwrap();
    assert_eq!(
        (
            world.revision,
            world.logical_tick,
            world.rng.state,
            world.next_item_instance_sequence,
            world.next_loot_drop_id,
            world.next_reward_open_id,
        ),
        before_retirement
    );
    assert!(!world.area_is_active("area-b"));
    assert!(!world.stable_rooms.contains_key("area-b:start"));
    assert!(!world.npcs.contains_key("area-b:keeper"));
    assert!(!world.item_instances.contains_key(&chest_id));
    assert!(!world.item_instances.contains_key(&child_id));
    assert!(matches!(
        world.item_location(&carried_id),
        Some(SharedItemLocation::Carried(CharacterId(612)))
    ));
    let retired_snapshot = world.runtime_snapshot();
    assert!(matches!(
        retired_snapshot.area_residency.get("area-b"),
        Some(AreaRuntimeResidence::Retired { checkpoint_sha256: digest, .. })
            if digest == &checkpoint_sha256
    ));

    let (mut restarted, _, restart_unit, session) = retirement_fixture();
    restarted.disconnect(session);
    restarted
        .set_item_location(&carried_id, SharedItemLocation::Room("area-b:start".into()))
        .unwrap();
    let before_failed_restore = restarted.runtime_snapshot();
    assert!(
        restarted
            .restore_runtime_snapshot(retired_snapshot.clone())
            .is_err()
    );
    assert_eq!(restarted.runtime_snapshot(), before_failed_restore);
    restarted
        .restore_runtime_snapshot_with_retired_areas(retired_snapshot.clone(), vec![restart_unit])
        .unwrap();
    assert!(!restarted.area_is_active("area-b"));
    assert!(!restarted.stable_rooms.contains_key("area-b:start"));
    assert!(!restarted.item_instances.contains_key(&chest_id));
    assert!(matches!(
        restarted.item_location(&carried_id),
        Some(SharedItemLocation::Carried(CharacterId(612)))
    ));

    world.advance_tick().unwrap();
    let before_rehydrate = (
        world.revision,
        world.logical_tick,
        world.rng.state,
        world.next_item_instance_sequence,
        world.next_loot_drop_id,
        world.next_reward_open_id,
    );
    let outcome = world
        .rehydrate_retired_areas(rehydrate_batch, checkpoint, &checkpoint_sha256)
        .unwrap();
    assert_eq!(outcome.installed_area_ids, vec!["area-b"]);
    assert!(outcome.created_dynamic_state);
    assert_eq!(
        (
            world.revision,
            world.logical_tick,
            world.rng.state,
            world.next_item_instance_sequence,
            world.next_loot_drop_id,
            world.next_reward_open_id,
        ),
        before_rehydrate
    );
    assert!(world.area_is_active("area-b"));
    assert!(world.item_instances.contains_key(&chest_id));
    assert!(matches!(
        world.item_location(&child_id),
        Some(SharedItemLocation::Contained(container)) if container == &chest_id
    ));
    assert!(world.ground_loot.contains_key(&chest_id));
    assert!(matches!(
        world.item_location(&carried_id),
        Some(SharedItemLocation::Carried(CharacterId(612)))
    ));
    let npc = &world.npcs["area-b:keeper"];
    assert_eq!(npc.health, npc.max_health);
    assert_eq!(npc.respawn_at_tick, None);
}

#[test]
fn area_retirement_prunes_discovered_rooms_so_projection_never_dangles() {
    let (mut world, _rehydrate_batch, unit, session) = retirement_fixture();
    // The observer previously visited the retired unit's room, so its map
    // memory references a room that retirement removes from `stable_rooms`.
    world
        .observer_views
        .get_mut(&session)
        .expect("connected observer")
        .discovered_rooms
        .insert("area-b:start".into());
    // Both projections resolve `discovered_rooms` strictly and would fail
    // with `InvalidEntity` if the retired room lingered in the map.
    assert!(world.player_state_view(session).is_ok());

    let prepared = world.prepare_area_retirement(unit).unwrap();
    let checkpoint_sha256 = canonical_sha256(&prepared.checkpoint).unwrap();
    world
        .apply_prepared_area_retirement(prepared, checkpoint_sha256)
        .unwrap();

    assert!(!world.stable_rooms.contains_key("area-b:start"));
    assert!(
        !world
            .observer_views
            .get(&session)
            .expect("observer survives retirement")
            .discovered_rooms
            .contains("area-b:start"),
        "retired room must be pruned from the observer's map memory"
    );
    // The dangling reference is gone: the full player projection succeeds.
    world.player_state_view(session).unwrap();
    world.scene_for(session).unwrap();
}

#[test]
fn area_retirement_pins_non_convertible_state_without_mutation() {
    let (mut world, _, unit, session) = retirement_fixture();
    let before = world.runtime_snapshot();
    let b_room = world.stable_rooms["area-b:start"];
    if let Entity::Character(character) = world
        .entities
        .get_mut(world.sessions[&session])
        .expect("connected character")
    {
        character.location = b_room;
    }
    assert_eq!(
        world.prepare_area_retirement(unit.clone()).unwrap_err(),
        AreaRetirementError::Busy(AreaRetirementPin::ConnectedCharacter)
    );
    if let Entity::Character(character) = world
        .entities
        .get_mut(world.sessions[&session])
        .expect("connected character")
    {
        character.location = world.stable_rooms["area-a:start"];
    }
    world
        .npcs
        .get_mut("area-b:keeper")
        .unwrap()
        .engaged_character = Some(CharacterId(612));
    let combat_before = world.runtime_snapshot();
    assert_eq!(
        world.prepare_area_retirement(unit.clone()).unwrap_err(),
        AreaRetirementError::Busy(AreaRetirementPin::Combat)
    );
    // Runtime snapshots deliberately exclude connected positions, but all
    // durable state is byte-for-byte unchanged by both failed preflights.
    assert_ne!(combat_before, before);
    assert_eq!(world.runtime_snapshot(), combat_before);

    world
        .npcs
        .get_mut("area-b:keeper")
        .unwrap()
        .engaged_character = None;
    world.trades.insert(
        TradeId(1),
        TradeSnapshot {
            id: TradeId(1),
            requester: CharacterId(612),
            recipient: CharacterId(613),
            room_id: "area-b:start".into(),
            phase: TradePhase::Negotiating,
            created_tick: 0,
            opened_tick: Some(0),
            expires_at_tick: TRADE_NEGOTIATION_TICKS,
            state_revision: 1,
            offers: BTreeMap::from([
                (CharacterId(612), TradeOffer::default()),
                (CharacterId(613), TradeOffer::default()),
            ]),
            confirmations: BTreeMap::new(),
        },
    );
    let trade_before = world.runtime_snapshot();
    assert_eq!(
        world.prepare_area_retirement(unit.clone()).unwrap_err(),
        AreaRetirementError::Busy(AreaRetirementPin::Trade)
    );
    assert_eq!(world.runtime_snapshot(), trade_before);
    world.trades.clear();

    let stable_before_restore = world.runtime_snapshot();
    let mut late_invalid = stable_before_restore.clone();
    if let Some(AreaRuntimeResidence::Resident {
        residency_epoch, ..
    }) = late_invalid.area_residency.get_mut("area-b")
    {
        *residency_epoch = 99;
    }
    late_invalid
        .item_instances
        .get_mut(&ItemInstanceId("area-b:carried-instance".into()))
        .unwrap()
        .prototype_id = "missing:prototype".into();
    assert!(world.restore_runtime_snapshot(late_invalid).is_err());
    assert_eq!(world.runtime_snapshot(), stable_before_restore);
}

#[test]
fn multi_area_retirement_uses_one_epoch_above_the_unit_maximum() {
    let (mut world, _, mut unit, _) = retirement_fixture();
    world
        .install_prepared_areas(PreparedCoreAreaBatch {
            generation_id: area_generation(),
            areas: vec![prepared_area(
                "area-c",
                1,
                vec![prepared_room("area-c:start", [])],
            )],
            deferred_exits: Vec::new(),
        })
        .unwrap();
    world
        .active_areas
        .get_mut("area-b")
        .unwrap()
        .residency_epoch = 3;
    world
        .active_areas
        .get_mut("area-c")
        .unwrap()
        .residency_epoch = 8;
    unit.areas.push(AreaRuntimeOwnership {
        area_id: "area-c".into(),
        revision: 1,
        bundle_sha256: format!("{:064x}", 1),
        rooms: BTreeSet::from(["area-c:start".into()]),
        initial_item_instances: BTreeSet::new(),
        npc_spawns: BTreeSet::new(),
    });
    unit.areas
        .sort_by(|left, right| left.area_id.cmp(&right.area_id));

    let prepared = world.prepare_area_retirement(unit).unwrap();
    assert!(
        prepared
            .checkpoint
            .areas
            .iter()
            .all(|area| area.residency_epoch == 9)
    );
    let digest = canonical_sha256(&prepared.checkpoint).unwrap();
    world
        .apply_prepared_area_retirement(prepared, digest)
        .unwrap();
    let snapshot = world.runtime_snapshot();
    assert!(matches!(
        snapshot.area_residency.get("area-b"),
        Some(AreaRuntimeResidence::Retired {
            residency_epoch: 9,
            ..
        })
    ));
    assert!(matches!(
        snapshot.area_residency.get("area-c"),
        Some(AreaRuntimeResidence::Retired {
            residency_epoch: 9,
            ..
        })
    ));
}

#[test]
fn initial_area_configuration_pins_abilities_before_lazy_npcs_install() {
    let generation = area_generation();
    let mut area = prepared_area("area-a", 1, vec![prepared_room("area-a:start", [])]);
    area.npcs.push(NpcDefinition {
        stable_id: "area-a:sentinel".into(),
        name: "sentinel".into(),
        description: "A sentinel with a pinned global ability.".into(),
        room_id: "area-a:start".into(),
        level: 1,
        max_health: 10,
        strength: 1,
        defence: 0,
        experience_reward: 0,
        loot_item_ids: Vec::new(),
        respawn_ticks: 10,
    });
    area.npc_ability_sets.push(NpcAbilitySetDefinition {
        npc_id: "area-a:sentinel".into(),
        resource_pools: BTreeMap::new(),
        resource_regeneration: BTreeMap::new(),
        resistances: BTreeMap::new(),
        ability_ids: vec!["global:jab".into()],
    });
    let (world, outcome) = World::from_initial_prepared_areas_with_configuration(
        "area-a:start",
        PreparedCoreAreaBatch {
            generation_id: generation,
            areas: vec![area],
            deferred_exits: Vec::new(),
        },
        |world| {
            world.configure_progression(
                builtin_progression_definition(),
                vec![AbilityDefinition {
                    id: "global:jab".into(),
                    name: "Jab".into(),
                    description: "A pinned global strike.".into(),
                    category: AbilityCategory::Martial,
                    resource_costs: BTreeMap::new(),
                    cooldown_ticks: 0,
                    targeting: AbilityTargeting::SingleEnemy,
                    damage: Some(AbilityDamageDefinition {
                        target: AbilityEffectTarget::Target,
                        damage_type: DamageType::Physical,
                        base_amount: 1,
                        scaling_attribute: None,
                        scaling_percent: 0,
                    }),
                    heal: None,
                    status_effects: Vec::new(),
                    communication: None,
                }],
            )
        },
    )
    .unwrap();
    assert!(outcome.created_dynamic_state);
    assert_eq!(world.revision(), 1);
    assert_eq!(
        world.npcs["area-a:sentinel"].ability_ids,
        vec!["global:jab"]
    );
}

#[test]
fn additive_cross_area_presentation_and_pending_npc_abilities_are_atomic() {
    let generation = area_generation();
    let mut start = prepared_area("area-a", 1, vec![prepared_room("area-a:start", [])]);
    let image = SceneImage {
        alt: "A verified cross-area scene.".into(),
        localized_alt: BTreeMap::from([("es".into(), "Una escena verificada entre áreas.".into())]),
        manifest: Some(format!("/assets/{}/manifest.json", "a".repeat(64))),
        avif: None,
        webp: None,
        jpeg: None,
        width: None,
        height: None,
    };
    start.room_presentations.push(RoomPresentationDefinition {
        room_id: "area-a:start".into(),
        image: Some(image.clone()),
        audio: None,
    });
    let mut rules = prepared_area("area-rules", 1, Vec::new());
    rules.npc_ability_sets.push(NpcAbilitySetDefinition {
        npc_id: "area-npcs:sentinel".into(),
        resource_pools: BTreeMap::new(),
        resource_regeneration: BTreeMap::new(),
        resistances: BTreeMap::new(),
        ability_ids: vec!["global:jab".into()],
    });
    let (mut world, initial) = World::from_initial_prepared_areas_with_configuration(
        "area-a:start",
        PreparedCoreAreaBatch {
            generation_id: generation.clone(),
            areas: vec![start, rules],
            deferred_exits: Vec::new(),
        },
        |world| {
            world.configure_progression(
                builtin_progression_definition(),
                vec![AbilityDefinition {
                    id: "global:jab".into(),
                    name: "Jab".into(),
                    description: "A pinned global strike.".into(),
                    category: AbilityCategory::Martial,
                    resource_costs: BTreeMap::new(),
                    cooldown_ticks: 0,
                    targeting: AbilityTargeting::SingleEnemy,
                    damage: Some(AbilityDamageDefinition {
                        target: AbilityEffectTarget::Target,
                        damage_type: DamageType::Physical,
                        base_amount: 1,
                        scaling_attribute: None,
                        scaling_percent: 0,
                    }),
                    heal: None,
                    status_effects: Vec::new(),
                    communication: None,
                }],
            )
        },
    )
    .unwrap();
    assert!(!initial.created_dynamic_state);
    let room = room_checked(&world.entities, world.stable_rooms["area-a:start"]).unwrap();
    assert_eq!(room.image.as_ref(), Some(&image));
    assert!(
        world
            .pending_npc_ability_sets
            .contains_key("area-npcs:sentinel")
    );

    let mut npcs = prepared_area("area-npcs", 1, Vec::new());
    npcs.npcs.push(NpcDefinition {
        stable_id: "area-npcs:sentinel".into(),
        name: "sentinel".into(),
        description: "A sentinel configured from another area.".into(),
        room_id: "area-a:start".into(),
        level: 1,
        max_health: 10,
        strength: 1,
        defence: 0,
        experience_reward: 0,
        loot_item_ids: Vec::new(),
        respawn_ticks: 10,
    });
    let installed = world
        .install_prepared_areas(PreparedCoreAreaBatch {
            generation_id: generation.clone(),
            areas: vec![npcs],
            deferred_exits: Vec::new(),
        })
        .unwrap();
    assert!(installed.created_dynamic_state);
    assert_eq!(
        world.npcs["area-npcs:sentinel"].ability_ids,
        vec!["global:jab"]
    );
    assert!(world.pending_npc_ability_sets.is_empty());

    let mut loot = prepared_area("area-loot", 1, Vec::new());
    loot.item_prototypes.push(item_prototype(
        "area-loot:token",
        "token",
        "A cross-area NPC possession.",
        &[],
    ));
    loot.initial_item_instances
        .push(InitialItemInstanceDefinition {
            instance_id: ItemInstanceId("area-loot:token-1".into()),
            prototype_id: "area-loot:token".into(),
            location: InitialItemLocation::NpcSpawn("area-npcs:sentinel".into()),
        });
    let installed_loot = world
        .install_prepared_areas(PreparedCoreAreaBatch {
            generation_id: generation.clone(),
            areas: vec![loot],
            deferred_exits: Vec::new(),
        })
        .unwrap();
    assert!(installed_loot.created_dynamic_state);
    assert!(
        world.npcs["area-npcs:sentinel"]
            .loot_item_ids
            .contains(&ItemInstanceId("area-loot:token-1".into()))
    );
    assert_eq!(
        world.item_location(&ItemInstanceId("area-loot:token-1".into())),
        Some(&SharedItemLocation::HeldByNpc("area-npcs:sentinel".into()))
    );

    let checkpoint = world.lifecycle_checkpoint();
    let mut conflict = prepared_area("area-conflict", 1, Vec::new());
    let mut conflicting_image = image;
    conflicting_image.alt = "Conflicting replacement.".into();
    conflict
        .room_presentations
        .push(RoomPresentationDefinition {
            room_id: "area-a:start".into(),
            image: Some(conflicting_image),
            audio: None,
        });
    assert!(matches!(
        world.install_prepared_areas(PreparedCoreAreaBatch {
            generation_id: generation,
            areas: vec![conflict],
            deferred_exits: Vec::new(),
        }),
        Err(WorldError::InvalidAreaLifecycle)
    ));
    let after = world.lifecycle_checkpoint();
    assert_eq!(checkpoint.active_areas, after.active_areas);
    assert_eq!(
        checkpoint.pending_npc_ability_sets,
        after.pending_npc_ability_sets
    );
    let room = room_checked(&world.entities, world.stable_rooms["area-a:start"]).unwrap();
    assert_eq!(
        room.image.as_ref().map(|image| image.alt.as_str()),
        Some("A verified cross-area scene.")
    );
}

#[test]
fn additive_install_relinks_portals_reports_dynamic_state_and_rolls_back_exactly() {
    let generation = area_generation();
    let first_request = deferred_portal(
        &generation,
        "area-a:start",
        Direction::North,
        "area-b:start",
        "area-b",
        2,
    );
    let (mut world, _) = World::from_initial_prepared_areas(
        "area-a:start",
        PreparedCoreAreaBatch {
            generation_id: generation.clone(),
            areas: vec![prepared_area(
                "area-a",
                1,
                vec![prepared_room(
                    "area-a:start",
                    [(Direction::North, "area-b:start")],
                )],
            )],
            deferred_exits: vec![first_request],
        },
    )
    .unwrap();
    let session = SessionId(72);
    world
        .connect(
            session,
            CharacterSnapshot {
                id: CharacterId(72),
                account_id: AccountId(72),
                name: "Lifecycle Tester".into(),
                room_id: "area-a:start".into(),
                item_locations: BTreeMap::new(),
            },
        )
        .unwrap();
    let second_request = deferred_portal(
        &generation,
        "area-b:start",
        Direction::East,
        "area-c:start",
        "area-c",
        3,
    );
    let before_install_revision = world.revision();
    let installed = world
        .install_prepared_areas(PreparedCoreAreaBatch {
            generation_id: generation.clone(),
            areas: vec![prepared_area(
                "area-b",
                2,
                vec![prepared_room(
                    "area-b:start",
                    [
                        (Direction::South, "area-a:start"),
                        (Direction::East, "area-c:start"),
                    ],
                )],
            )],
            deferred_exits: vec![second_request.clone()],
        })
        .unwrap();
    assert_eq!(installed.installed_area_ids, vec!["area-b"]);
    assert!(!installed.created_dynamic_state);
    assert_eq!(world.revision(), before_install_revision);
    assert_eq!(
        world
            .deferred_area_load_request(session, Direction::North)
            .unwrap(),
        None
    );
    world
        .apply(session, PlayerAction::Move(Direction::North))
        .unwrap();
    assert_eq!(
        world
            .deferred_area_load_request(session, Direction::East)
            .unwrap(),
        Some(second_request)
    );

    let lifecycle_checkpoint = world.lifecycle_checkpoint();
    let mut invalid_c = prepared_area(
        "area-c",
        3,
        vec![prepared_room(
            "area-c:start",
            [(Direction::West, "area-b:start")],
        )],
    );
    let duplicate = item_prototype("area-c:item", "area item", "An item.", &[]);
    invalid_c.item_prototypes = vec![duplicate.clone(), duplicate];
    assert!(matches!(
        world.install_prepared_areas(PreparedCoreAreaBatch {
            generation_id: generation.clone(),
            areas: vec![invalid_c],
            deferred_exits: Vec::new(),
        }),
        Err(WorldError::InvalidAreaLifecycle)
    ));
    assert!(!world.area_is_active("area-c"));
    assert_eq!(
        world
            .deferred_area_load_request(session, Direction::East)
            .unwrap(),
        Some(deferred_portal(
            &generation,
            "area-b:start",
            Direction::East,
            "area-c:start",
            "area-c",
            3,
        ))
    );

    let mut valid_c = prepared_area(
        "area-c",
        3,
        vec![prepared_room(
            "area-c:start",
            [(Direction::West, "area-b:start")],
        )],
    );
    valid_c.item_prototypes = vec![item_prototype(
        "area-c:item",
        "area item",
        "An item installed with its area.",
        &[],
    )];
    valid_c.initial_item_instances = vec![item_instance(
        "area-c:item-instance",
        "area-c:item",
        InitialItemLocation::Room("area-c:start".into()),
    )];
    let revision_before_dynamic_install = world.revision();
    let installed = world
        .install_prepared_areas(PreparedCoreAreaBatch {
            generation_id: generation,
            areas: vec![valid_c],
            deferred_exits: Vec::new(),
        })
        .unwrap();
    assert_eq!(installed.installed_area_ids, vec!["area-c"]);
    assert!(installed.created_dynamic_state);
    assert_eq!(world.revision(), revision_before_dynamic_install);
    assert_eq!(
        world
            .deferred_area_load_request(session, Direction::East)
            .unwrap(),
        None
    );
    let scene = world
        .apply(session, PlayerAction::Move(Direction::East))
        .unwrap()
        .scene;
    assert_eq!(scene.title, "area-c:start");
    assert_eq!(scene.items.len(), 1);

    world.restore_lifecycle_checkpoint(lifecycle_checkpoint);
    assert!(!world.area_is_active("area-c"));
    assert_eq!(
        world.character_snapshot(session).unwrap().room_id,
        "area-b:start"
    );
    assert!(matches!(
        world.apply(session, PlayerAction::Move(Direction::East)),
        Err(WorldError::AreaLoadRequired(_))
    ));
}

#[test]
fn unknown_persisted_rooms_fail_closed_without_relocating() {
    let mut world = World::demo();
    let session = SessionId(11);
    assert!(matches!(
        world.connect(
            session,
            CharacterSnapshot {
                id: CharacterId(102),
                account_id: AccountId(10),
                name: "Old Save".into(),
                room_id: "missing:room".into(),
                item_locations: BTreeMap::new(),
            },
        ),
        Err(WorldError::InvalidEntity)
    ));
    assert!(!world.sessions.contains_key(&session));
    assert!(!world.character_gameplay.contains_key(&CharacterId(102)));
}

#[test]
fn corrupt_exit_references_return_an_error_instead_of_panicking() {
    let (mut world, session) = connected_world();
    let character_id = world.sessions[&session];
    room_mut(&mut world.entities, world.start_room)
        .exits
        .insert(Direction::West, RoomExit::Resident(character_id));

    assert!(matches!(
        world.scene_for(session),
        Err(WorldError::InvalidEntity)
    ));
    assert!(matches!(
        world.apply(session, PlayerAction::Move(Direction::West)),
        Err(WorldError::InvalidEntity)
    ));
    assert_eq!(
        world.character_snapshot(session).unwrap().room_id,
        "demo:lantern-square"
    );
}
