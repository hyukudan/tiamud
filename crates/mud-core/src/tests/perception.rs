use super::*;

#[test]
fn taking_an_item_is_observed_globally() {
    let (mut world, first) = connected_world();
    let second = SessionId(17);
    world
        .connect(
            second,
            CharacterSnapshot {
                id: CharacterId(202),
                account_id: AccountId(20),
                name: "Second".into(),
                room_id: world.start_room_id().to_owned(),
                item_locations: BTreeMap::new(),
            },
        )
        .unwrap();

    world
        .apply(first, PlayerAction::Take("lantern".into()))
        .unwrap();
    assert!(world.scene_for(first).unwrap().items.is_empty());
    assert!(world.scene_for(second).unwrap().items.is_empty());
    assert_eq!(
        world.inventory_as_plain_text(second).unwrap(),
        "Capacity: 0/24 slots; 0/50000 g.\r\nYou are carrying nothing.\r\n"
    );
}

#[test]
fn one_domain_event_projects_differently_by_perception() {
    let (mut world, actor) = connected_world();
    let peer = SessionId(17);
    world
        .connect(
            peer,
            CharacterSnapshot {
                id: CharacterId(202),
                account_id: AccountId(20),
                name: "Witness".into(),
                room_id: world.start_room_id().to_owned(),
                item_locations: BTreeMap::new(),
            },
        )
        .unwrap();
    let distant = SessionId(18);
    world
        .connect(
            distant,
            CharacterSnapshot {
                id: CharacterId(303),
                account_id: AccountId(30),
                name: "Archivist".into(),
                room_id: "demo:archive".into(),
                item_locations: BTreeMap::new(),
            },
        )
        .unwrap();

    let actor_scene = world.scene_for(actor).unwrap();
    let peer_scene = world.scene_for(peer).unwrap();
    assert_ne!(actor_scene.view_id, peer_scene.view_id);
    assert_ne!(actor_scene.items[0].id, peer_scene.items[0].id);
    assert!(!actor_scene.items[0].id.contains("demo:"));
    assert!(!actor_scene.items[0].actions[1].command.contains("demo:"));
    let distant_revision = world.scene_for(distant).unwrap().revision;

    let action = parse_action(&actor_scene.items[0].actions[1].command).unwrap();
    let outcome = world.apply(actor, action).unwrap();
    let event = outcome.domain_events.first().unwrap();
    let actor_view = world.project_event(actor, event).unwrap().unwrap();
    let peer_view = world.project_event(peer, event).unwrap().unwrap();

    assert_eq!(actor_view.text, "You take the blue-glass lantern.");
    assert_eq!(peer_view.text, "Tester takes the blue-glass lantern.");
    assert_ne!(actor_view.actor_id, peer_view.actor_id);
    assert_ne!(actor_view.item_id, peer_view.item_id);
    assert_opaque_id(&actor_view.actor_id);
    assert!(!actor_view.item_id.as_deref().unwrap().contains("demo:"));
    assert!(world.project_event(distant, event).unwrap().is_none());
    assert_eq!(world.scene_for(actor).unwrap().revision, 2);
    assert_eq!(world.scene_for(peer).unwrap().revision, 2);
    assert_eq!(world.scene_for(distant).unwrap().revision, distant_revision);
}

#[test]
fn a_dropped_item_becomes_visible_to_other_characters() {
    let (mut world, actor) = connected_world();
    let peer = SessionId(17);
    world
        .connect(
            peer,
            CharacterSnapshot {
                id: CharacterId(202),
                account_id: AccountId(20),
                name: "Witness".into(),
                room_id: world.start_room_id().to_owned(),
                item_locations: BTreeMap::new(),
            },
        )
        .unwrap();
    world
        .apply(actor, PlayerAction::Take("lantern".into()))
        .unwrap();
    world
        .apply(actor, PlayerAction::Move(Direction::North))
        .unwrap();
    world
        .apply(actor, PlayerAction::Drop("lantern".into()))
        .unwrap();

    assert_eq!(world.scene_for(actor).unwrap().items.len(), 1);
    assert!(world.scene_for(peer).unwrap().items.is_empty());
    world
        .apply(peer, PlayerAction::Move(Direction::North))
        .unwrap();
    assert_eq!(world.scene_for(peer).unwrap().items.len(), 1);
    assert_eq!(
        world.scene_for(peer).unwrap().items[0].label,
        "blue-glass lantern"
    );
}

#[test]
fn combat_view_events_are_opaque_and_perception_scoped() {
    let (mut world, session) = phase_two_world(13, 2, 20);
    world
        .apply(session, PlayerAction::Move(Direction::North))
        .unwrap();
    world
        .apply(session, PlayerAction::Attack("guardian".into()))
        .unwrap();
    let events = world.advance_tick().unwrap();
    let damage = events
        .iter()
        .find(|event| {
            matches!(
                event,
                DomainEvent::Gameplay {
                    event: GameplayEvent::NpcDamaged { .. }
                }
            )
        })
        .unwrap();
    let view = world.project_event(session, damage).unwrap().unwrap();
    assert_ne!(view.actor_id, "900");
    assert!(!view.target_id.as_deref().unwrap().contains("game:"));
    assert!(!view.text.contains("game:"));
}

#[test]
fn private_narration_uses_only_the_authorized_observers_opaque_identity() {
    let (world, session) = phase_two_world(13, 2, 20);
    let view = world
        .project_private_narration(session, "Only you hear the archive whisper.")
        .unwrap();

    assert_eq!(view.kind, ViewEventKind::Gameplay);
    assert_eq!(view.text, "Only you hear the archive whisper.");
    assert_opaque_id(&view.actor_id);
    assert!(view.target_id.is_none());
    assert!(view.item_id.is_none());
    assert!(matches!(
        world.project_private_narration(session, "\u{1b}[31munsafe"),
        Err(WorldError::InvalidScriptEffect)
    ));
}

#[test]
fn internal_clock_ticks_refresh_but_do_not_narrate_observer_views() {
    let (mut world, session) = phase_two_world(13, 2, 20);
    let before = world.scene_for(session).unwrap().revision;

    let events = world.advance_tick().unwrap();
    let tick = events
        .iter()
        .find(|event| {
            matches!(
                event,
                DomainEvent::Gameplay {
                    event: GameplayEvent::TickAdvanced { .. }
                }
            )
        })
        .unwrap();

    assert_eq!(world.project_event(session, tick).unwrap(), None);
    assert!(world.scene_for(session).unwrap().revision > before);
}

#[test]
fn hidden_item_policy_targets_one_instance_not_its_reusable_prototype() {
    let mut world = World::from_definition(WorldDefinition {
        start_room_id: "test:start".into(),
        item_prototypes: vec![item_prototype(
            "test:lantern",
            "blue lantern",
            "A cold blue lantern.",
            &["lantern"],
        )],
        initial_item_instances: vec![item_instance(
            "test:lantern:one",
            "test:lantern",
            InitialItemLocation::Room("test:start".into()),
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
    assert!(matches!(
        world.configure_hidden_item("test:lantern", 100),
        Err(WorldError::InvalidGameplayDefinition)
    ));
    world
        .configure_hidden_item("test:lantern:one", 100)
        .unwrap();
    let session = SessionId(777);
    world
        .connect(
            session,
            CharacterSnapshot {
                id: CharacterId(777),
                account_id: AccountId(777),
                name: "Observer".into(),
                room_id: "test:start".into(),
                item_locations: BTreeMap::new(),
            },
        )
        .unwrap();

    assert!(world.scene_for(session).unwrap().items.is_empty());
}

#[test]
fn perception_authorizes_scenes_actions_maps_and_events_before_projection() {
    let (mut world, actor) = phase_two_world(13, 2, 20);
    world.configure_hidden_item("game:sword", 20).unwrap();
    world
        .configure_secret_exit("game:start", Direction::North, 20)
        .unwrap();
    world.configure_hidden_npc("game:guardian", 20).unwrap();
    world
        .configure_room_light("game:arena", LightLevel::Dark)
        .unwrap();
    let arena_entity = world.stable_rooms["game:arena"];
    let arena = room_mut(&mut world.entities, arena_entity);
    arena.image = Some(SceneImage {
        alt: "A moonless arena.".into(),
        localized_alt: BTreeMap::from([("en".into(), "A moonless arena.".into())]),
        manifest: None,
        avif: Some("/assets/arena.avif".into()),
        webp: None,
        jpeg: None,
        width: Some(640),
        height: Some(360),
    });
    arena.audio = Some(SceneAudio {
        channel: "ambience".into(),
        url: format!("/assets/{}/arena.opus", "a".repeat(64)),
        looped: true,
    });

    let start_observer = SessionId(901);
    world
        .connect(
            start_observer,
            CharacterSnapshot {
                id: CharacterId(901),
                account_id: AccountId(91),
                name: "Scout".into(),
                room_id: "game:start".into(),
                item_locations: BTreeMap::new(),
            },
        )
        .unwrap();
    let low_scene = world.scene_for(start_observer).unwrap();
    assert!(low_scene.exits.is_empty());
    assert_eq!(
        low_scene
            .items
            .iter()
            .map(|item| item.label.as_str())
            .collect::<Vec<_>>(),
        vec!["red potion"]
    );
    assert!(matches!(
        world.apply(start_observer, PlayerAction::Move(Direction::North)),
        Err(WorldError::NoExit)
    ));
    assert!(matches!(
        world.apply(start_observer, PlayerAction::Take("sword".into())),
        Err(WorldError::ItemNotFound(_))
    ));
    assert!(
        world
            .player_state_view(start_observer)
            .unwrap()
            .map
            .paths
            .is_empty()
    );

    world.set_observer_role(actor, AccountRole::Admin).unwrap();
    let admin_scene = world.scene_for(actor).unwrap();
    assert_eq!(admin_scene.exits.len(), 1);
    assert!(
        admin_scene
            .items
            .iter()
            .any(|item| item.label == "iron sword")
    );
    assert!(!admin_scene.exits[0].id.contains("game:"));

    let item_event = world
        .apply(actor, PlayerAction::Take("sword".into()))
        .unwrap()
        .domain_events
        .remove(0);
    assert_eq!(
        world.project_event(start_observer, &item_event).unwrap(),
        None
    );
    let move_event = world
        .apply(actor, PlayerAction::Move(Direction::North))
        .unwrap()
        .domain_events
        .remove(0);
    assert_eq!(
        world.project_event(start_observer, &move_event).unwrap(),
        None
    );

    let arena_observer = SessionId(902);
    world
        .connect(
            arena_observer,
            CharacterSnapshot {
                id: CharacterId(902),
                account_id: AccountId(92),
                name: "Listener".into(),
                room_id: "game:arena".into(),
                item_locations: BTreeMap::new(),
            },
        )
        .unwrap();
    let dark_scene = world.scene_for(arena_observer).unwrap();
    assert!(dark_scene.npcs.is_empty());
    assert!(!dark_scene.prose.join(" ").contains("Claw marks"));
    assert!(dark_scene.image.is_none());
    assert!(dark_scene.audio.is_some());

    let combat_event = world
        .apply(actor, PlayerAction::Attack("guardian".into()))
        .unwrap()
        .domain_events
        .remove(0);
    assert_eq!(
        world.project_event(arena_observer, &combat_event).unwrap(),
        None
    );

    let mut perceptive = world.character_gameplay_snapshot(arena_observer).unwrap();
    perceptive.skills.insert("perception".into(), 100);
    perceptive.script_flags.insert("sense.darkvision".into());
    world
        .restore_character_gameplay(CharacterId(902), perceptive.clone())
        .unwrap();
    let darkvision_scene = world.scene_for(arena_observer).unwrap();
    assert_eq!(darkvision_scene.npcs.len(), 1);
    assert!(darkvision_scene.prose.join(" ").contains("Claw marks"));
    assert!(darkvision_scene.image.is_some());

    perceptive.script_flags.insert("condition.blind".into());
    world
        .restore_character_gameplay(CharacterId(902), perceptive.clone())
        .unwrap();
    let blind_scene = world.scene_for(arena_observer).unwrap();
    assert!(blind_scene.npcs.is_empty());
    assert!(blind_scene.image.is_none());
    assert!(blind_scene.audio.is_some());
    perceptive.script_flags.insert("sense.tremorsense".into());
    world
        .restore_character_gameplay(CharacterId(902), perceptive.clone())
        .unwrap();
    assert_eq!(world.scene_for(arena_observer).unwrap().npcs.len(), 1);
    perceptive.script_flags.insert("condition.deaf".into());
    world
        .restore_character_gameplay(CharacterId(902), perceptive)
        .unwrap();
    assert!(world.scene_for(arena_observer).unwrap().audio.is_none());
}

#[test]
fn spatial_voice_is_directed_bounded_secret_safe_and_deafness_only_filters_receivers() {
    let room = |id: &str, exits: Vec<(Direction, &str)>| RoomDefinition {
        stable_id: id.into(),
        title: id.into(),
        prose: vec![format!("Room {id}.")],
        image: None,
        audio: None,
        item_ids: Vec::new(),
        exits: exits
            .into_iter()
            .map(|(direction, target_room_id)| ExitDefinition {
                direction,
                target_room_id: target_room_id.into(),
            })
            .collect(),
    };
    let mut world = World::from_definition(WorldDefinition {
        start_room_id: "voice:r0".into(),
        item_prototypes: Vec::new(),
        initial_item_instances: Vec::new(),
        rooms: vec![
            room("voice:r0", vec![(Direction::East, "voice:r1")]),
            room(
                "voice:r1",
                vec![(Direction::West, "voice:r0"), (Direction::East, "voice:r2")],
            ),
            room(
                "voice:r2",
                vec![(Direction::West, "voice:r1"), (Direction::East, "voice:r3")],
            ),
            room(
                "voice:r3",
                vec![(Direction::West, "voice:r2"), (Direction::East, "voice:r4")],
            ),
            room("voice:r4", vec![(Direction::West, "voice:r3")]),
            // This one-way edge points toward the origin. A directed BFS
            // from r0 must not infer an unauthored reverse connection.
            room("voice:inbound", vec![(Direction::West, "voice:r0")]),
        ],
    })
    .unwrap();
    let connect = |world: &mut World, session: u64, character: i64, name: &str, room_id: &str| {
        world
            .connect(
                SessionId(session),
                CharacterSnapshot {
                    id: CharacterId(character),
                    account_id: AccountId(character),
                    name: name.into(),
                    room_id: room_id.into(),
                    item_locations: BTreeMap::new(),
                },
            )
            .unwrap();
    };
    connect(&mut world, 1, 1, "Actor", "voice:r0");
    connect(&mut world, 2, 2, "Local", "voice:r0");
    connect(&mut world, 3, 3, "Near", "voice:r1");
    connect(&mut world, 4, 4, "Mid", "voice:r2");
    connect(&mut world, 5, 5, "Far", "voice:r3");
    connect(&mut world, 6, 6, "Beyond", "voice:r4");
    connect(&mut world, 7, 7, "Inbound", "voice:inbound");
    connect(&mut world, 8, 8, "Deaf", "voice:r1");
    let mut deaf = world.character_gameplay_snapshot(SessionId(8)).unwrap();
    deaf.script_flags.insert("condition.deaf".into());
    world
        .restore_character_gameplay(CharacterId(8), deaf)
        .unwrap();

    let names = |audience: Vec<ConnectedCharacter>| {
        audience
            .into_iter()
            .map(|candidate| candidate.name)
            .collect::<Vec<_>>()
    };
    assert_eq!(
        names(world.local_voice_audience(SessionId(1)).unwrap()),
        vec!["Actor", "Local"]
    );
    assert_eq!(
        names(world.yell_audience(SessionId(1)).unwrap()),
        vec!["Actor", "Local", "Near"]
    );
    assert_eq!(
        names(world.shout_audience(SessionId(1)).unwrap()),
        vec!["Actor", "Local", "Near", "Mid", "Far"]
    );

    world
        .configure_secret_exit("voice:r1", Direction::East, 1)
        .unwrap();
    assert_eq!(
        names(world.shout_audience(SessionId(1)).unwrap()),
        vec!["Actor", "Local", "Near"]
    );

    // Deafness does not prevent emission or remove the actor's private
    // acknowledgement path: only receivers are filtered.
    assert_eq!(
        names(world.shout_audience(SessionId(8)).unwrap()),
        vec!["Actor", "Local", "Near"]
    );
}

#[test]
fn shout_traversal_accepts_the_exact_structural_room_cap() {
    let directions = [
        Direction::North,
        Direction::East,
        Direction::South,
        Direction::West,
        Direction::Up,
        Direction::Down,
    ];
    let mut exits = (0..MAX_SPATIAL_VOICE_ROOMS)
        .map(|_| Vec::new())
        .collect::<Vec<Vec<ExitDefinition>>>();
    let mut next_room = 1_usize;
    let mut level = vec![0_usize];
    for _ in 0..SHOUT_MAX_ROOM_HOPS {
        let mut next_level = Vec::new();
        for parent in level {
            for direction in directions {
                let child = next_room;
                next_room += 1;
                exits[parent].push(ExitDefinition {
                    direction,
                    target_room_id: format!("cap:r{child}"),
                });
                next_level.push(child);
            }
        }
        level = next_level;
    }
    assert_eq!(next_room, MAX_SPATIAL_VOICE_ROOMS);
    let mut world = World::from_definition(WorldDefinition {
        start_room_id: "cap:r0".into(),
        item_prototypes: Vec::new(),
        initial_item_instances: Vec::new(),
        rooms: exits
            .into_iter()
            .enumerate()
            .map(|(index, exits)| RoomDefinition {
                stable_id: format!("cap:r{index}"),
                title: format!("Cap room {index}"),
                prose: Vec::new(),
                image: None,
                audio: None,
                item_ids: Vec::new(),
                exits,
            })
            .collect(),
    })
    .unwrap();
    for index in 0..MAX_SPATIAL_VOICE_ROOMS {
        let identity = i64::try_from(index + 1).unwrap();
        world
            .connect(
                SessionId(u64::try_from(index + 1).unwrap()),
                CharacterSnapshot {
                    id: CharacterId(identity),
                    account_id: AccountId(identity),
                    name: format!("Listener {index}"),
                    room_id: format!("cap:r{index}"),
                    item_locations: BTreeMap::new(),
                },
            )
            .unwrap();
    }

    assert_eq!(
        world.shout_audience(SessionId(1)).unwrap().len(),
        MAX_SPATIAL_VOICE_ROOMS
    );
}

#[test]
fn spatial_voice_visits_cycles_and_convergent_paths_only_once() {
    let room = |id: &str, exits: Vec<(Direction, &str)>| RoomDefinition {
        stable_id: id.into(),
        title: id.into(),
        prose: Vec::new(),
        image: None,
        audio: None,
        item_ids: Vec::new(),
        exits: exits
            .into_iter()
            .map(|(direction, target)| ExitDefinition {
                direction,
                target_room_id: target.into(),
            })
            .collect(),
    };
    let mut world = World::from_definition(WorldDefinition {
        start_room_id: "diamond:origin".into(),
        item_prototypes: Vec::new(),
        initial_item_instances: Vec::new(),
        rooms: vec![
            room(
                "diamond:origin",
                vec![
                    (Direction::North, "diamond:left"),
                    (Direction::East, "diamond:right"),
                ],
            ),
            room("diamond:left", vec![(Direction::East, "diamond:join")]),
            room("diamond:right", vec![(Direction::North, "diamond:join")]),
            // This edge closes a cycle back to the origin after the two
            // branches converge at the same room.
            room("diamond:join", vec![(Direction::South, "diamond:origin")]),
        ],
    })
    .unwrap();
    for (session, name, room_id) in [
        (1_u64, "Actor", "diamond:origin"),
        (2, "At join", "diamond:join"),
    ] {
        world
            .connect(
                SessionId(session),
                CharacterSnapshot {
                    id: CharacterId(i64::try_from(session).unwrap()),
                    account_id: AccountId(i64::try_from(session).unwrap()),
                    name: name.into(),
                    room_id: room_id.into(),
                    item_locations: BTreeMap::new(),
                },
            )
            .unwrap();
    }

    let audience = world.shout_audience(SessionId(1)).unwrap();
    assert_eq!(
        audience
            .iter()
            .map(|candidate| candidate.name.as_str())
            .collect::<Vec<_>>(),
        vec!["Actor", "At join"]
    );
    assert_eq!(
        audience
            .iter()
            .filter(|candidate| candidate.room_id == "diamond:join")
            .count(),
        1
    );
}

#[test]
fn player_state_is_complete_bounded_and_never_serialises_durable_entity_ids() {
    let (mut world, session) = connected_world();
    let initial = world.player_state_view(session).unwrap();
    assert_eq!(initial.character.name, "Tester");
    assert_eq!(initial.character.health, 20);
    assert_eq!(initial.map.rooms.len(), 1);
    assert!(initial.map.rooms[0].current);
    assert_eq!(initial.quests[0].status, "available");
    assert_eq!(initial.economy.coins, 40);
    assert_eq!(initial.social.online.len(), 1);

    world
        .apply(session, PlayerAction::Take("lantern".into()))
        .unwrap();
    world
        .apply(session, PlayerAction::Move(Direction::North))
        .unwrap();
    let discovered = world.player_state_view(session).unwrap();
    assert_eq!(discovered.inventory.len(), 1);
    assert_eq!(discovered.map.rooms.len(), 2);
    assert_eq!(
        discovered
            .map
            .rooms
            .iter()
            .filter(|room| room.current)
            .count(),
        1
    );
    assert!(
        discovered
            .inventory
            .iter()
            .all(|item| item.id.starts_with('o') && item.id.len() == 17)
    );
    assert!(
        discovered
            .equipment
            .iter()
            .all(|item| item.id.starts_with('o') && item.id.len() == 17)
    );

    let json = serde_json::to_string(&discovered).unwrap();
    assert!(!json.contains("demo:lantern-square"));
    assert!(!json.contains("demo:north-gate"));
    assert!(!json.contains("demo:blue-lantern"));
    assert!(!json.contains("accountId"));
    assert!(!json.contains("characterId"));
    assert!(!json.contains("partyId"));
    assert!(!json.contains("\"id\":101"));
}
