use std::collections::{BTreeMap, BTreeSet};

use tiamud_core::{
    AbilityCategory, AbilityCommunicationDefinition, AbilityCommunicationKind,
    AbilityCommunicationScope, AbilityDefinition, AbilityEffectTarget, AbilityPurpose,
    AbilityStatusEffectDefinition, AbilityTargeting, AccountId, BackgroundSelection,
    BaseClassDefinition, CharacterDefaultsDefinition, CharacterId, CharacterProgressionDefinition,
    CharacterSnapshot, GrantDefinition, LevelThresholdDefinition, LineageDefinition,
    MAX_COMMUNICATION_ABILITIES, MAX_COMMUNICATION_BLOCKED_ROOMS, MAX_TELEPATHY_MESSAGE_BYTES,
    MAX_TELEPATHY_TARGET_BYTES, PlayerAction, RoomDefinition, SessionId, World, WorldDefinition,
    WorldError, parse_action,
};

const CLEAR_ROOM: &str = "test:clear-room";
const BLOCKED_ROOM: &str = "test:silent-room";
const TELEPATHY_ID: &str = "test:mind-whisper";
const STUN_ID: &str = "test:self-stun";

fn telepathy_definition() -> AbilityDefinition {
    AbilityDefinition {
        id: TELEPATHY_ID.into(),
        name: "Mind Whisper".into(),
        description: "Send one bounded thought to an online character.".into(),
        category: AbilityCategory::Spell,
        resource_costs: BTreeMap::from([("focus".into(), 3)]),
        cooldown_ticks: 5,
        targeting: AbilityTargeting::SingleAlly,
        damage: None,
        heal: None,
        status_effects: Vec::new(),
        communication: Some(AbilityCommunicationDefinition {
            kind: AbilityCommunicationKind::Telepathy,
            scope: AbilityCommunicationScope::DirectOnline,
            blocked_room_ids: BTreeSet::from([BLOCKED_ROOM.into()]),
        }),
    }
}

fn stun_definition() -> AbilityDefinition {
    AbilityDefinition {
        id: STUN_ID.into(),
        name: "Still Mind".into(),
        description: "Stun the caster for a deterministic test interval.".into(),
        category: AbilityCategory::Utility,
        resource_costs: BTreeMap::new(),
        cooldown_ticks: 0,
        targeting: AbilityTargeting::SelfOnly,
        damage: None,
        heal: None,
        status_effects: vec![AbilityStatusEffectDefinition::Stun {
            target: AbilityEffectTarget::Caster,
            duration_ticks: 2,
        }],
        communication: None,
    }
}

fn second_telepathy_definition() -> AbilityDefinition {
    let mut ability = telepathy_definition();
    ability.id = "test:mind-echo".into();
    ability.name = "Mind Echo".into();
    ability
}

fn progression(ability_ids: &[&str]) -> CharacterProgressionDefinition {
    let mut grants = vec![
        GrantDefinition::Attribute {
            attribute_id: "max_health".into(),
            amount: 20,
        },
        GrantDefinition::ResourceMax {
            resource_id: "focus".into(),
            amount: 10,
        },
    ];
    grants.extend(
        ability_ids
            .iter()
            .map(|ability_id| GrantDefinition::Ability {
                ability_id: (*ability_id).into(),
            }),
    );
    CharacterProgressionDefinition {
        defaults: CharacterDefaultsDefinition {
            lineage_id: "test:human".into(),
            background_id: None,
            base_class_id: "test:listener".into(),
        },
        level_thresholds: vec![LevelThresholdDefinition {
            level: 2,
            total_experience_required: 100,
            point_awards: BTreeMap::new(),
        }],
        lineages: vec![LineageDefinition {
            id: "test:human".into(),
            name: "Human".into(),
            description: "A test lineage.".into(),
            ui_order: 0,
            creation_grants: Vec::new(),
            talent_graph_ids: Vec::new(),
        }],
        backgrounds: Vec::new(),
        base_classes: vec![BaseClassDefinition {
            id: "test:listener".into(),
            name: "Listener".into(),
            description: "A test class.".into(),
            ui_order: 0,
            creation_grants: grants,
            level_grants: Vec::new(),
            talent_graph_ids: Vec::new(),
        }],
        advanced_classes: Vec::new(),
        talent_graphs: Vec::new(),
    }
}

fn world_with(abilities: Vec<AbilityDefinition>) -> World {
    let ability_ids = abilities
        .iter()
        .map(|ability| ability.id.as_str())
        .collect::<Vec<_>>();
    let mut world = World::from_definition(WorldDefinition {
        start_room_id: CLEAR_ROOM.into(),
        item_prototypes: Vec::new(),
        initial_item_instances: Vec::new(),
        rooms: vec![
            RoomDefinition {
                stable_id: CLEAR_ROOM.into(),
                title: "Clear room".into(),
                prose: vec!["Thought travels freely here.".into()],
                image: None,
                audio: None,
                item_ids: Vec::new(),
                exits: Vec::new(),
            },
            RoomDefinition {
                stable_id: BLOCKED_ROOM.into(),
                title: "Silent room".into(),
                prose: vec!["Authored interference fills the room.".into()],
                image: None,
                audio: None,
                item_ids: Vec::new(),
                exits: Vec::new(),
            },
        ],
    })
    .unwrap();
    world
        .configure_progression(progression(&ability_ids), abilities)
        .unwrap();
    world
}

fn connect(world: &mut World, session: u64, character: i64, name: &str, room_id: &str) {
    let mut gameplay = world
        .initial_gameplay(None, BackgroundSelection::Default, None)
        .unwrap();
    gameplay.resources.get_mut("focus").unwrap().current = 10;
    world
        .restore_character_gameplay(CharacterId(character), gameplay)
        .unwrap();
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
}

#[test]
fn parser_accepts_only_the_exact_bounded_telepathy_shape() {
    assert_eq!(
        parse_action("telepathy test:mind-whisper Aria Hold the North Gate!"),
        Ok(PlayerAction::Telepathy {
            ability_id: TELEPATHY_ID.into(),
            target: "Aria".into(),
            message: "Hold the North Gate!".into(),
        })
    );
    assert_eq!(
        parse_action("  TeLePaThY test:mind-whisper Aria Mixed Case  "),
        Ok(PlayerAction::Telepathy {
            ability_id: TELEPATHY_ID.into(),
            target: "Aria".into(),
            message: "Mixed Case".into(),
        })
    );

    for invalid in [
        "telepathy",
        "telepathy test:mind-whisper",
        "telepathy test:mind-whisper Aria",
        "telepathy TEST:mind-whisper Aria hello",
        "telepathy test:mind-whisper Aria bad\u{7f}message",
        "telepathy test:mind-whisper Aria bad\nmessage",
    ] {
        assert!(parse_action(invalid).is_err(), "{invalid:?}");
    }
    let oversized_id = format!("telepathy {} Aria hello", "a".repeat(65));
    let oversized_target = format!(
        "telepathy {TELEPATHY_ID} {} hello",
        "a".repeat(MAX_TELEPATHY_TARGET_BYTES + 1)
    );
    let oversized_message = format!(
        "telepathy {TELEPATHY_ID} Aria {}",
        "a".repeat(MAX_TELEPATHY_MESSAGE_BYTES + 1)
    );
    assert!(parse_action(&oversized_id).is_err());
    assert!(parse_action(&oversized_target).is_err());
    assert!(parse_action(&oversized_message).is_err());
}

#[test]
fn communication_ability_definition_is_closed_bounded_and_world_referenced() {
    assert_eq!(
        world_with(vec![telepathy_definition()])
            .progression_catalog()
            .base_classes
            .len(),
        1
    );

    let mut invalid = Vec::new();
    let mut martial = telepathy_definition();
    martial.category = AbilityCategory::Martial;
    invalid.push(martial);
    let mut enemy = telepathy_definition();
    enemy.targeting = AbilityTargeting::SingleEnemy;
    invalid.push(enemy);
    let mut free = telepathy_definition();
    free.resource_costs.clear();
    invalid.push(free);
    let mut no_cooldown = telepathy_definition();
    no_cooldown.cooldown_ticks = 0;
    invalid.push(no_cooldown);
    let mut effectful = telepathy_definition();
    effectful.status_effects = vec![AbilityStatusEffectDefinition::Stun {
        target: AbilityEffectTarget::Caster,
        duration_ticks: 1,
    }];
    invalid.push(effectful);
    let mut oversized_id = telepathy_definition();
    oversized_id.id = "a".repeat(65);
    invalid.push(oversized_id);
    let mut too_many_rooms = telepathy_definition();
    too_many_rooms
        .communication
        .as_mut()
        .unwrap()
        .blocked_room_ids = (0..=MAX_COMMUNICATION_BLOCKED_ROOMS)
        .map(|index| format!("test:room-{index}"))
        .collect();
    invalid.push(too_many_rooms);
    let mut unknown_room = telepathy_definition();
    unknown_room
        .communication
        .as_mut()
        .unwrap()
        .blocked_room_ids = BTreeSet::from(["test:missing".into()]);
    invalid.push(unknown_room);

    for ability in invalid {
        let mut world = World::from_definition(WorldDefinition {
            start_room_id: CLEAR_ROOM.into(),
            item_prototypes: Vec::new(),
            initial_item_instances: Vec::new(),
            rooms: vec![
                RoomDefinition {
                    stable_id: CLEAR_ROOM.into(),
                    title: "Clear room".into(),
                    prose: vec!["Safe.".into()],
                    image: None,
                    audio: None,
                    item_ids: Vec::new(),
                    exits: Vec::new(),
                },
                RoomDefinition {
                    stable_id: BLOCKED_ROOM.into(),
                    title: "Silent room".into(),
                    prose: vec!["Safe.".into()],
                    image: None,
                    audio: None,
                    item_ids: Vec::new(),
                    exits: Vec::new(),
                },
            ],
        })
        .unwrap();
        let ability_id = ability.id.clone();
        assert!(matches!(
            world.configure_progression(progression(&[&ability_id]), vec![ability]),
            Err(WorldError::InvalidAbilityDefinition)
        ));
    }
}

#[test]
fn communication_catalog_has_a_separate_cap() {
    let exact = (0..MAX_COMMUNICATION_ABILITIES)
        .map(|index| {
            let mut ability = telepathy_definition();
            ability.id = format!("test:exact-mind-{index}");
            ability.name = format!("Exact Mind {index}");
            ability
        })
        .collect::<Vec<_>>();
    let mut exact_world = world_with(Vec::new());
    assert!(
        exact_world
            .configure_progression(progression(&[]), exact)
            .is_ok()
    );

    let abilities = (0..=MAX_COMMUNICATION_ABILITIES)
        .map(|index| {
            let mut ability = telepathy_definition();
            ability.id = format!("test:mind-{index}");
            ability.name = format!("Mind {index}");
            ability
        })
        .collect::<Vec<_>>();
    let ability_ids = abilities
        .iter()
        .map(|ability| ability.id.as_str())
        .collect::<Vec<_>>();
    let mut world = World::from_definition(WorldDefinition {
        start_room_id: CLEAR_ROOM.into(),
        item_prototypes: Vec::new(),
        initial_item_instances: Vec::new(),
        rooms: vec![
            RoomDefinition {
                stable_id: CLEAR_ROOM.into(),
                title: "Clear room".into(),
                prose: vec!["Safe.".into()],
                image: None,
                audio: None,
                item_ids: Vec::new(),
                exits: Vec::new(),
            },
            RoomDefinition {
                stable_id: BLOCKED_ROOM.into(),
                title: "Silent room".into(),
                prose: vec!["Safe.".into()],
                image: None,
                audio: None,
                item_ids: Vec::new(),
                exits: Vec::new(),
            },
        ],
    })
    .unwrap();
    assert!(matches!(
        world.configure_progression(progression(&ability_ids), abilities),
        Err(WorldError::InvalidAbilityDefinition)
    ));
}

#[test]
fn apply_consumes_resources_and_timers_atomically_without_a_domain_event() {
    let mut world = world_with(vec![
        telepathy_definition(),
        second_telepathy_definition(),
        stun_definition(),
    ]);
    connect(&mut world, 1, 1, "Aria", CLEAR_ROOM);
    connect(&mut world, 2, 2, "Borin", CLEAR_ROOM);

    let before = world.character_gameplay_snapshot(SessionId(1)).unwrap();
    let revision = world.revision();
    let plan = world
        .telepathy_preflight(SessionId(1), TELEPATHY_ID, "Borin", "Hold the gate.")
        .unwrap();
    assert_eq!(plan.sender_character_id, CharacterId(1));
    assert_eq!(plan.sender_name, "Aria");
    assert_eq!(plan.sender_room_id, CLEAR_ROOM);
    assert_eq!(plan.ability_id, TELEPATHY_ID);
    assert_eq!(
        world.character_gameplay_snapshot(SessionId(1)).unwrap(),
        before
    );
    assert!(
        world
            .telepathy_receptive(SessionId(2), TELEPATHY_ID)
            .unwrap()
    );

    assert_eq!(
        world
            .apply_telepathy(SessionId(1), TELEPATHY_ID, "Borin", "Hold the gate.")
            .unwrap(),
        plan
    );
    let after = world.character_gameplay_snapshot(SessionId(1)).unwrap();
    assert_eq!(after.resources["focus"].current, 7);
    assert_eq!(after.ability_cooldowns[TELEPATHY_ID], 5);
    assert_eq!(after.ability_global_ready_tick, 1);
    assert!(world.revision() > revision);

    let before_global_rejection = after.clone();
    assert!(matches!(
        world.telepathy_preflight(SessionId(1), "test:mind-echo", "Borin", "A second thought."),
        Err(WorldError::AbilityUnavailable)
    ));
    assert_eq!(
        world.character_gameplay_snapshot(SessionId(1)).unwrap(),
        before_global_rejection
    );

    let runtime_after = world.runtime_snapshot();
    let repeated = world.character_gameplay_snapshot(SessionId(1)).unwrap();
    assert!(matches!(
        world.apply_telepathy(SessionId(1), TELEPATHY_ID, "Borin", "Again."),
        Err(WorldError::AbilityUnavailable)
    ));
    assert_eq!(
        world.character_gameplay_snapshot(SessionId(1)).unwrap(),
        repeated
    );
    assert_eq!(world.runtime_snapshot(), runtime_after);
}

#[test]
fn failed_preflight_and_direct_world_apply_never_spend_the_ability() {
    let mut world = world_with(vec![telepathy_definition(), stun_definition()]);
    connect(&mut world, 1, 1, "Aria", CLEAR_ROOM);
    connect(&mut world, 2, 2, "Borin", CLEAR_ROOM);
    let mut low = world.character_gameplay_snapshot(SessionId(1)).unwrap();
    low.resources.get_mut("focus").unwrap().current = 2;
    world
        .restore_character_gameplay(CharacterId(1), low)
        .unwrap();
    let before = world.character_gameplay_snapshot(SessionId(1)).unwrap();
    let runtime = world.runtime_snapshot();

    assert!(matches!(
        world.apply_telepathy(SessionId(1), TELEPATHY_ID, "Borin", "Hello."),
        Err(WorldError::InsufficientResource)
    ));
    assert!(matches!(
        world.apply(
            SessionId(1),
            PlayerAction::Telepathy {
                ability_id: TELEPATHY_ID.into(),
                target: "Borin".into(),
                message: "Hello.".into(),
            }
        ),
        Err(WorldError::GameplayUnavailable)
    ));
    assert_eq!(
        world.character_gameplay_snapshot(SessionId(1)).unwrap(),
        before
    );
    assert_eq!(world.runtime_snapshot(), runtime);
}

#[test]
fn communication_ability_cannot_enter_the_generic_ability_path() {
    let mut world = world_with(vec![telepathy_definition()]);
    connect(&mut world, 1, 1, "Aria", CLEAR_ROOM);
    let before_gameplay = world.character_gameplay_snapshot(SessionId(1)).unwrap();
    let before_runtime = world.runtime_snapshot();
    let before_revision = world.revision();

    for target in [None, Some("self".to_owned())] {
        assert!(matches!(
            world.apply(
                SessionId(1),
                PlayerAction::Ability {
                    ability_id: TELEPATHY_ID.into(),
                    target,
                },
            ),
            Err(WorldError::AbilityUnavailable)
        ));
        assert_eq!(
            world.character_gameplay_snapshot(SessionId(1)).unwrap(),
            before_gameplay
        );
        assert_eq!(world.runtime_snapshot(), before_runtime);
        assert_eq!(world.revision(), before_revision);
    }
}

#[test]
fn authored_interference_and_private_receiver_state_fail_closed() {
    let mut world = world_with(vec![telepathy_definition(), stun_definition()]);
    connect(&mut world, 1, 1, "Aria", BLOCKED_ROOM);
    connect(&mut world, 2, 2, "Borin", CLEAR_ROOM);
    let before = world.character_gameplay_snapshot(SessionId(1)).unwrap();
    assert!(matches!(
        world.telepathy_preflight(SessionId(1), TELEPATHY_ID, "Borin", "Hello."),
        Err(WorldError::AbilityUnavailable)
    ));
    assert_eq!(
        world.character_gameplay_snapshot(SessionId(1)).unwrap(),
        before
    );
    let view = world.player_state_view(SessionId(1)).unwrap();
    let telepathy = view
        .character
        .abilities
        .iter()
        .find(|ability| ability.id == TELEPATHY_ID)
        .unwrap();
    assert_eq!(telepathy.purpose, AbilityPurpose::Telepathy);
    assert!(!telepathy.usable);
    let ordinary = view
        .character
        .abilities
        .iter()
        .find(|ability| ability.id == STUN_ID)
        .unwrap();
    assert_eq!(ordinary.purpose, AbilityPurpose::Gameplay);

    let mut blocked_receiver = world_with(vec![telepathy_definition(), stun_definition()]);
    connect(&mut blocked_receiver, 1, 1, "Aria", CLEAR_ROOM);
    connect(&mut blocked_receiver, 2, 2, "Borin", BLOCKED_ROOM);
    assert!(
        !blocked_receiver
            .telepathy_receptive(SessionId(2), TELEPATHY_ID)
            .unwrap()
    );

    let mut recovering = blocked_receiver
        .character_gameplay_snapshot(SessionId(1))
        .unwrap();
    recovering.vitals.health = 0;
    recovering.recovery_at_tick = Some(3);
    blocked_receiver
        .restore_character_gameplay(CharacterId(1), recovering)
        .unwrap();
    assert!(
        !blocked_receiver
            .telepathy_receptive(SessionId(1), TELEPATHY_ID)
            .unwrap()
    );

    let mut stunned = world_with(vec![telepathy_definition(), stun_definition()]);
    connect(&mut stunned, 1, 1, "Aria", CLEAR_ROOM);
    stunned
        .apply(
            SessionId(1),
            PlayerAction::Ability {
                ability_id: STUN_ID.into(),
                target: None,
            },
        )
        .unwrap();
    assert!(
        !stunned
            .telepathy_receptive(SessionId(1), TELEPATHY_ID)
            .unwrap()
    );
}

#[test]
fn defeated_recovering_and_stunned_senders_cannot_spend_telepathy() {
    let mut recovering = world_with(vec![telepathy_definition(), stun_definition()]);
    connect(&mut recovering, 1, 1, "Aria", CLEAR_ROOM);
    let mut state = recovering
        .character_gameplay_snapshot(SessionId(1))
        .unwrap();
    state.vitals.health = 0;
    state.recovery_at_tick = Some(3);
    recovering
        .restore_character_gameplay(CharacterId(1), state)
        .unwrap();
    let before = recovering
        .character_gameplay_snapshot(SessionId(1))
        .unwrap();
    assert!(matches!(
        recovering.apply_telepathy(SessionId(1), TELEPATHY_ID, "Borin", "Hello."),
        Err(WorldError::AbilityUnavailable)
    ));
    assert_eq!(
        recovering
            .character_gameplay_snapshot(SessionId(1))
            .unwrap(),
        before
    );

    let mut stunned = world_with(vec![telepathy_definition(), stun_definition()]);
    connect(&mut stunned, 1, 1, "Aria", CLEAR_ROOM);
    stunned
        .apply(
            SessionId(1),
            PlayerAction::Ability {
                ability_id: STUN_ID.into(),
                target: None,
            },
        )
        .unwrap();
    stunned.advance_tick().unwrap();
    let before = stunned.character_gameplay_snapshot(SessionId(1)).unwrap();
    assert!(
        before
            .statuses
            .iter()
            .any(|status| status.source_ability_id == STUN_ID)
    );
    assert_eq!(before.ability_global_ready_tick, stunned.logical_tick());
    assert!(matches!(
        stunned.apply_telepathy(SessionId(1), TELEPATHY_ID, "Borin", "Hello."),
        Err(WorldError::AbilityUnavailable)
    ));
    assert_eq!(
        stunned.character_gameplay_snapshot(SessionId(1)).unwrap(),
        before
    );
}

#[test]
fn structured_inputs_are_revalidated_before_resource_mutation() {
    let mut world = world_with(vec![telepathy_definition()]);
    connect(&mut world, 1, 1, "Aria", CLEAR_ROOM);
    let before = world.character_gameplay_snapshot(SessionId(1)).unwrap();
    let oversized = "x".repeat(MAX_TELEPATHY_MESSAGE_BYTES + 1);
    for (target, message) in [
        (" Borin", "Hello."),
        ("Borin", " Hello."),
        ("Borin", "bad\u{7f}message"),
        ("Borin", "bad\nmessage"),
        ("Borin", oversized.as_str()),
    ] {
        assert!(matches!(
            world.apply_telepathy(SessionId(1), TELEPATHY_ID, target, message),
            Err(WorldError::InvalidTelepathyRequest)
        ));
    }
    assert_eq!(
        world.character_gameplay_snapshot(SessionId(1)).unwrap(),
        before
    );
}
