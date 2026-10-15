use super::*;

#[test]
fn movement_changes_the_stable_room_reference() {
    let (mut world, session) = connected_world();
    let outcome = world
        .apply(session, PlayerAction::Move(Direction::North))
        .unwrap();
    assert_ne!(outcome.scene.view_id, "demo:north-gate");
    assert_eq!(outcome.scene.title, "The North Gate");
    assert_eq!(
        outcome.changed_character.unwrap().room_id,
        "demo:north-gate"
    );
}

#[test]
fn failed_movement_does_not_change_state() {
    let (mut world, session) = connected_world();
    let before = world.character_snapshot(session).unwrap();
    assert!(matches!(
        world.apply(session, PlayerAction::Move(Direction::West)),
        Err(WorldError::NoExit)
    ));
    assert_eq!(world.character_snapshot(session).unwrap(), before);
}

#[test]
fn durable_character_identity_survives_world_changes() {
    let (mut world, session) = connected_world();
    world
        .apply(session, PlayerAction::Move(Direction::North))
        .unwrap();

    let snapshot = world.character_snapshot(session).unwrap();
    assert_eq!(snapshot.id, CharacterId(101));
    assert_eq!(snapshot.account_id, AccountId(10));
    assert_eq!(snapshot.name, "Tester");
}

#[test]
fn a_character_can_reconnect_after_disconnect() {
    let (mut world, first_session) = connected_world();
    let first_scene = world.scene_for(first_session).unwrap();
    let snapshot = world.character_snapshot(first_session).unwrap();
    world.disconnect(first_session);

    let scene = world.connect(SessionId(9), snapshot.clone()).unwrap();
    assert_ne!(scene.view_id, "demo:lantern-square");
    assert_ne!(scene.view_id, first_scene.view_id);
    assert_ne!(scene.items[0].id, first_scene.items[0].id);
    assert_eq!(scene.title, "Lantern Square");

    let mut restarted = World::demo();
    let restarted_scene = restarted.connect(SessionId(1), snapshot).unwrap();
    assert_ne!(restarted_scene.view_id, first_scene.view_id);
    assert_ne!(restarted_scene.items[0].id, first_scene.items[0].id);
}

#[test]
fn a_character_cannot_be_connected_by_two_sessions_at_once() {
    let (mut world, first_session) = connected_world();
    let snapshot = world.character_snapshot(first_session).unwrap();

    assert!(matches!(
        world.connect(SessionId(9), snapshot),
        Err(WorldError::CharacterAlreadyConnected)
    ));
    assert_eq!(
        world.character_snapshot(first_session).unwrap().id,
        CharacterId(101)
    );
}

#[test]
fn rollback_cannot_replace_a_sessions_durable_identity() {
    let (mut world, session) = connected_world();
    let mut foreign = world.character_snapshot(session).unwrap();
    foreign.id = CharacterId(999);
    let revision = world.revision();

    assert!(matches!(
        world.restore_character(session, &foreign, revision),
        Err(WorldError::InvalidEntity)
    ));
    assert_eq!(
        world.character_snapshot(session).unwrap().id,
        CharacterId(101)
    );
}

#[test]
fn builds_a_world_from_a_transport_independent_definition() {
    let image = SceneImage {
        alt: String::new(),
        localized_alt: BTreeMap::new(),
        manifest: Some("/assets/aaaaaaaa/manifest.json".into()),
        avif: None,
        webp: None,
        jpeg: None,
        width: None,
        height: None,
    };
    let mut world = World::from_definition(WorldDefinition {
        start_room_id: "test:start".into(),
        item_prototypes: Vec::new(),
        initial_item_instances: Vec::new(),
        rooms: vec![
            RoomDefinition {
                stable_id: "test:start".into(),
                title: "Start".into(),
                prose: vec!["A complete textual scene.".into()],
                image: Some(image.clone()),
                audio: None,
                exits: vec![ExitDefinition {
                    direction: Direction::North,
                    target_room_id: "test:north".into(),
                }],
                item_ids: Vec::new(),
            },
            RoomDefinition {
                stable_id: "test:north".into(),
                title: "North".into(),
                prose: vec!["The northern room.".into()],
                image: None,
                audio: None,
                exits: Vec::new(),
                item_ids: Vec::new(),
            },
        ],
    })
    .unwrap();
    let session = SessionId(42);
    let scene = world
        .connect(
            session,
            CharacterSnapshot {
                id: CharacterId(7),
                account_id: AccountId(3),
                name: "Builder".into(),
                room_id: "test:start".into(),
                item_locations: BTreeMap::new(),
            },
        )
        .unwrap();

    assert_eq!(scene.image, Some(image));
    assert_eq!(scene.exits[0].command, "north");
    assert_eq!(
        world
            .apply(session, PlayerAction::Move(Direction::North))
            .unwrap()
            .scene
            .title,
        "North"
    );
}

#[test]
fn rejects_invalid_world_definition_references() {
    let missing_default = World::from_definition(WorldDefinition {
        start_room_id: "missing".into(),
        item_prototypes: Vec::new(),
        initial_item_instances: Vec::new(),
        rooms: vec![RoomDefinition {
            stable_id: "only".into(),
            title: "Only".into(),
            prose: Vec::new(),
            image: None,
            audio: None,
            exits: Vec::new(),
            item_ids: Vec::new(),
        }],
    });
    assert!(matches!(
        missing_default,
        Err(WorldBuildError::UnknownStartRoom(room)) if room == "missing"
    ));

    let missing_exit = World::from_definition(WorldDefinition {
        start_room_id: "only".into(),
        item_prototypes: Vec::new(),
        initial_item_instances: Vec::new(),
        rooms: vec![RoomDefinition {
            stable_id: "only".into(),
            title: "Only".into(),
            prose: Vec::new(),
            image: None,
            audio: None,
            exits: vec![ExitDefinition {
                direction: Direction::Down,
                target_room_id: "void".into(),
            }],
            item_ids: Vec::new(),
        }],
    });
    assert!(matches!(
        missing_exit,
        Err(WorldBuildError::UnknownExitTarget { target_room_id, .. }) if target_room_id == "void"
    ));

    let duplicate_direction = World::from_definition(WorldDefinition {
        start_room_id: "only".into(),
        item_prototypes: Vec::new(),
        initial_item_instances: Vec::new(),
        rooms: vec![RoomDefinition {
            stable_id: "only".into(),
            title: "Only".into(),
            prose: Vec::new(),
            image: None,
            audio: None,
            exits: vec![
                ExitDefinition {
                    direction: Direction::North,
                    target_room_id: "only".into(),
                },
                ExitDefinition {
                    direction: Direction::North,
                    target_room_id: "only".into(),
                },
            ],
            item_ids: Vec::new(),
        }],
    });
    assert!(matches!(
        duplicate_direction,
        Err(WorldBuildError::DuplicateExitDirection {
            direction: Direction::North,
            ..
        })
    ));
}

#[test]
fn rollback_restores_movement_and_observer_revision() {
    let (mut world, session) = connected_world();
    let before = world.character_snapshot(session).unwrap();
    let view_revision = world.scene_for(session).unwrap().revision;
    let revision = world.revision();
    world
        .apply(session, PlayerAction::Move(Direction::North))
        .unwrap();

    world.restore_character(session, &before, revision).unwrap();

    assert_eq!(world.character_snapshot(session).unwrap(), before);
    assert_eq!(world.revision(), revision);
    assert_eq!(world.scene_for(session).unwrap().revision, view_revision);
    assert_eq!(world.scene_for(session).unwrap().items.len(), 1);
    assert_eq!(
        world.inventory_as_plain_text(session).unwrap(),
        "Capacity: 0/24 slots; 0/50000 g.\r\nYou are carrying nothing.\r\n"
    );
}

#[test]
fn replay_reconstructs_take_and_drop_with_the_owner_disconnected() {
    let owner = CharacterId(404);
    let taken = DomainEvent::ItemTaken {
        actor_id: owner,
        actor_name: "Offline Hero".into(),
        item_id: "demo:blue-lantern".into(),
        item_name: "blue-glass lantern".into(),
        room_id: "demo:lantern-square".into(),
    };
    let dropped = DomainEvent::ItemDropped {
        actor_id: owner,
        actor_name: "Offline Hero".into(),
        item_id: "demo:blue-lantern".into(),
        item_name: "blue-glass lantern".into(),
        room_id: "demo:north-gate".into(),
    };

    let mut carried_world = World::demo();
    carried_world.replay_persisted_event(&taken).unwrap();
    let owner_session = SessionId(404);
    carried_world
        .connect(
            owner_session,
            CharacterSnapshot {
                id: owner,
                account_id: AccountId(40),
                name: "Offline Hero".into(),
                room_id: "demo:archive".into(),
                item_locations: BTreeMap::new(),
            },
        )
        .unwrap();
    let inventory = carried_world
        .inventory_as_plain_text(owner_session)
        .unwrap();
    assert_eq!(
        redact_and_validate_opaque_item_refs(&inventory),
        "Capacity: 1/24 slots; 1000/50000 g.\r\nYou are carrying:\r\n- 1 × blue-glass lantern [common; level 1; 1000 g each; 1 stack slot; 1000 g total; policy transferable]\r\n"
    );

    let mut dropped_world = World::demo();
    dropped_world.replay_persisted_event(&taken).unwrap();
    dropped_world.replay_persisted_event(&dropped).unwrap();
    let witness = SessionId(405);
    let scene = dropped_world
        .connect(
            witness,
            CharacterSnapshot {
                id: CharacterId(405),
                account_id: AccountId(40),
                name: "Witness".into(),
                room_id: "demo:north-gate".into(),
                item_locations: BTreeMap::new(),
            },
        )
        .unwrap();
    assert_eq!(scene.items.len(), 1);
    assert_eq!(scene.items[0].label, "blue-glass lantern");
    assert_eq!(dropped_world.revision(), 3);
}

#[test]
fn replay_rejects_invalid_metadata_and_transitions_atomically() {
    let mut world = World::demo();
    let invalid_name = DomainEvent::ItemTaken {
        actor_id: CharacterId(404),
        actor_name: "Offline Hero".into(),
        item_id: "demo:blue-lantern".into(),
        item_name: "forged name".into(),
        room_id: "demo:lantern-square".into(),
    };
    let invalid_transition = DomainEvent::ItemDropped {
        actor_id: CharacterId(404),
        actor_name: "Offline Hero".into(),
        item_id: "demo:blue-lantern".into(),
        item_name: "blue-glass lantern".into(),
        room_id: "demo:lantern-square".into(),
    };
    let unknown_item = DomainEvent::ItemTaken {
        actor_id: CharacterId(404),
        actor_name: "Offline Hero".into(),
        item_id: "demo:forged-item".into(),
        item_name: "blue-glass lantern".into(),
        room_id: "demo:lantern-square".into(),
    };
    let unknown_room = DomainEvent::ItemTaken {
        actor_id: CharacterId(404),
        actor_name: "Offline Hero".into(),
        item_id: "demo:blue-lantern".into(),
        item_name: "blue-glass lantern".into(),
        room_id: "demo:forged-room".into(),
    };

    for invalid in [&invalid_name, &unknown_item, &unknown_room] {
        assert!(matches!(
            world.replay_persisted_event(invalid),
            Err(WorldError::InvalidDomainEvent)
        ));
    }
    assert!(matches!(
        world.replay_persisted_event(&invalid_transition),
        Err(WorldError::InvalidDomainTransition)
    ));
    assert_eq!(world.revision(), 1);
    let session = SessionId(406);
    assert_eq!(
        world
            .connect(
                session,
                CharacterSnapshot {
                    id: CharacterId(406),
                    account_id: AccountId(40),
                    name: "Witness".into(),
                    room_id: "demo:lantern-square".into(),
                    item_locations: BTreeMap::new(),
                },
            )
            .unwrap()
            .items
            .len(),
        1
    );
}

#[test]
fn rollback_domain_events_restores_shared_state_and_view_revisions() {
    let (mut world, actor) = connected_world();
    let peer = SessionId(407);
    world
        .connect(
            peer,
            CharacterSnapshot {
                id: CharacterId(407),
                account_id: AccountId(40),
                name: "Witness".into(),
                room_id: "demo:lantern-square".into(),
                item_locations: BTreeMap::new(),
            },
        )
        .unwrap();
    let domain_revision = world.revision();
    let actor_revision = world.scene_for(actor).unwrap().revision;
    let peer_revision = world.scene_for(peer).unwrap().revision;
    let outcome = world
        .apply(actor, PlayerAction::Take("lantern".into()))
        .unwrap();
    assert!(world.scene_for(peer).unwrap().items.is_empty());

    world
        .rollback_domain_events(&outcome.domain_events)
        .unwrap();

    assert_eq!(world.revision(), domain_revision);
    assert_eq!(world.scene_for(actor).unwrap().revision, actor_revision);
    assert_eq!(world.scene_for(peer).unwrap().revision, peer_revision);
    assert_eq!(world.scene_for(actor).unwrap().items.len(), 1);
    assert_eq!(world.scene_for(peer).unwrap().items.len(), 1);
    assert_eq!(
        world.inventory_as_plain_text(actor).unwrap(),
        "Capacity: 0/24 slots; 0/50000 g.\r\nYou are carrying nothing.\r\n"
    );
}

#[test]
fn invalid_rollback_batch_does_not_partially_mutate_state() {
    let (mut world, actor) = connected_world();
    let outcome = world
        .apply(actor, PlayerAction::Take("lantern".into()))
        .unwrap();
    let revision = world.revision();
    let view_revision = world.scene_for(actor).unwrap().revision;
    let mut forged = outcome.domain_events.clone();
    if let DomainEvent::ItemTaken { item_name, .. } = &mut forged[0] {
        *item_name = "forged name".into();
    }

    assert!(matches!(
        world.rollback_domain_events(&forged),
        Err(WorldError::InvalidDomainEvent)
    ));
    assert_eq!(world.revision(), revision);
    assert_eq!(world.scene_for(actor).unwrap().revision, view_revision);
    let inventory = world.inventory_as_plain_text(actor).unwrap();
    assert_eq!(
        redact_and_validate_opaque_item_refs(&inventory),
        "Capacity: 1/24 slots; 1000/50000 g.\r\nYou are carrying:\r\n- 1 × blue-glass lantern [common; level 1; 1000 g each; 1 stack slot; 1000 g total; policy transferable]\r\n"
    );
}
