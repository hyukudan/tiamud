use std::collections::BTreeMap;

use tiamud_core::{
    AccountId, CharacterId, CharacterSnapshot, MAX_SOCIAL_EMOTE_COMMAND_BYTES,
    MAX_SOCIAL_EMOTE_ID_BYTES, MAX_SOCIAL_EMOTE_NAME_BYTES, MAX_SOCIAL_EMOTE_RENDER_BYTES,
    MAX_SOCIAL_EMOTE_TARGET_BYTES, MAX_SOCIAL_EMOTES, PlayerAction, RoomDefinition, SessionId,
    SocialAction, SocialEmoteAudience, SocialEmoteDefinition, SocialEmoteTargetedTemplates,
    SocialEmoteTargeting, SocialEmoteTemplates, SocialEmoteUntargetedTemplates, World,
    WorldDefinition, WorldError, parse_action,
};

const ROOM: &str = "test:room";
const REMOTE_ROOM: &str = "test:remote";

fn untargeted(id: &str, command: &str, name: &str) -> SocialEmoteDefinition {
    SocialEmoteDefinition {
        id: id.into(),
        command: command.into(),
        name: name.into(),
        audience: SocialEmoteAudience::Room,
        targeting: SocialEmoteTargeting::None,
        templates: SocialEmoteTemplates {
            untargeted: Some(SocialEmoteUntargetedTemplates {
                actor: "You wave.".into(),
                others: "{actor} waves.".into(),
            }),
            targeted: None,
        },
    }
}

fn optional_smile() -> SocialEmoteDefinition {
    SocialEmoteDefinition {
        id: "test:smile".into(),
        command: "smile".into(),
        name: "Smile".into(),
        audience: SocialEmoteAudience::Room,
        targeting: SocialEmoteTargeting::OptionalCharacter,
        templates: SocialEmoteTemplates {
            untargeted: Some(SocialEmoteUntargetedTemplates {
                actor: "You smile.".into(),
                others: "{actor} smiles.".into(),
            }),
            targeted: Some(SocialEmoteTargetedTemplates {
                actor: "You smile at {target}.".into(),
                target: "{actor} smiles at you.".into(),
                others: "{actor} smiles at {target}.".into(),
            }),
        },
    }
}

fn required_bow() -> SocialEmoteDefinition {
    SocialEmoteDefinition {
        id: "test:bow".into(),
        command: "bow".into(),
        name: "Bow".into(),
        audience: SocialEmoteAudience::Room,
        targeting: SocialEmoteTargeting::RequiredCharacter,
        templates: SocialEmoteTemplates {
            untargeted: None,
            targeted: Some(SocialEmoteTargetedTemplates {
                actor: "You bow to {target}.".into(),
                target: "{actor} bows to you.".into(),
                others: "{actor} bows to {target}.".into(),
            }),
        },
    }
}

fn world_with(definitions: Vec<SocialEmoteDefinition>) -> World {
    let mut world = World::from_definition(WorldDefinition {
        start_room_id: ROOM.into(),
        item_prototypes: Vec::new(),
        initial_item_instances: Vec::new(),
        rooms: vec![
            RoomDefinition {
                stable_id: ROOM.into(),
                title: "Room".into(),
                prose: vec!["A shared room.".into()],
                image: None,
                audio: None,
                item_ids: Vec::new(),
                exits: Vec::new(),
            },
            RoomDefinition {
                stable_id: REMOTE_ROOM.into(),
                title: "Remote".into(),
                prose: vec!["A separate room.".into()],
                image: None,
                audio: None,
                item_ids: Vec::new(),
                exits: Vec::new(),
            },
        ],
    })
    .unwrap();
    world.configure_social_emotes(definitions).unwrap();
    world
}

fn connect(world: &mut World, session: u64, character: i64, name: &str, room_id: &str) {
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
fn parser_accepts_only_the_closed_bounded_emote_shapes() {
    assert_eq!(
        parse_action("emotes"),
        Ok(PlayerAction::Social(SocialAction::EmoteList))
    );
    assert_eq!(
        parse_action("  EmOtE smile  "),
        Ok(PlayerAction::Social(SocialAction::Emote {
            command: "smile".into(),
            target: None,
        }))
    );
    assert_eq!(
        parse_action("EMOTE smile Borin"),
        Ok(PlayerAction::Social(SocialAction::Emote {
            command: "smile".into(),
            target: Some("Borin".into()),
        }))
    );
    assert_eq!(
        parse_action("emote smile <Borin>&Aria"),
        Ok(PlayerAction::Social(SocialAction::Emote {
            command: "smile".into(),
            target: Some("<Borin>&Aria".into()),
        }))
    );

    for invalid in [
        "emote",
        "emote Smile",
        "emote -smile",
        "emote smile Borin\u{7f}",
        "emote smile Borin\nAria",
    ] {
        assert!(parse_action(invalid).is_err(), "{invalid:?}");
    }
    assert!(
        parse_action(&format!(
            "emote {}",
            "a".repeat(MAX_SOCIAL_EMOTE_COMMAND_BYTES + 1)
        ))
        .is_err()
    );
    assert!(
        parse_action(&format!(
            "emote smile {}",
            "a".repeat(MAX_SOCIAL_EMOTE_TARGET_BYTES + 1)
        ))
        .is_err()
    );
}

#[test]
fn catalog_is_bounded_unique_atomic_and_does_not_expose_templates() {
    let exact = (0..MAX_SOCIAL_EMOTES)
        .map(|index| {
            untargeted(
                &format!("test:emote-{index}"),
                &format!("emote-{index}"),
                &format!("Emote {index}"),
            )
        })
        .collect::<Vec<_>>();
    let mut world = world_with(exact);
    assert_eq!(world.social_emotes().len(), MAX_SOCIAL_EMOTES);
    assert!(
        world
            .social_emotes()
            .windows(2)
            .all(|pair| pair[0].command < pair[1].command)
    );
    let encoded = serde_json::to_value(world.social_emotes()).unwrap();
    assert!(encoded.as_array().unwrap()[0].get("templates").is_none());

    let before = world.social_emotes();
    let too_many = (0..=MAX_SOCIAL_EMOTES)
        .map(|index| {
            untargeted(
                &format!("test:overflow-{index}"),
                &format!("overflow-{index}"),
                "Overflow",
            )
        })
        .collect();
    assert!(matches!(
        world.configure_social_emotes(too_many),
        Err(WorldError::InvalidEmoteDefinition)
    ));
    assert_eq!(world.social_emotes(), before);

    let duplicate_id = vec![
        untargeted("test:same", "first", "First"),
        untargeted("test:same", "second", "Second"),
    ];
    let duplicate_command = vec![
        untargeted("test:first", "same", "First"),
        untargeted("test:second", "same", "Second"),
    ];
    for invalid in [duplicate_id, duplicate_command] {
        assert!(matches!(
            world.configure_social_emotes(invalid),
            Err(WorldError::InvalidEmoteDefinition)
        ));
        assert_eq!(world.social_emotes(), before);
    }
}

#[test]
fn definition_matrix_placeholders_and_plain_text_are_fail_closed() {
    let valid = vec![
        untargeted("test:wave", "wave", "Wave"),
        optional_smile(),
        required_bow(),
    ];
    assert_eq!(world_with(valid.clone()).social_emotes().len(), 3);

    let mut invalid = Vec::new();
    let mut wrong_matrix = optional_smile();
    wrong_matrix.templates.targeted = None;
    invalid.push(wrong_matrix);
    let mut wrong_none = untargeted("test:wave", "wave", "Wave");
    wrong_none.templates.targeted = optional_smile().templates.targeted;
    invalid.push(wrong_none);
    let mut wrong_required = required_bow();
    wrong_required.templates.untargeted = optional_smile().templates.untargeted;
    invalid.push(wrong_required);

    for template in [
        "{actor} waves.",
        "{target} waves.",
        "You {unknown}.",
        "You <wave>.",
        "You wave & smile.",
        "You wave.\nThen bow.",
        "You wave.\u{1b}[31m",
    ] {
        let mut emote = untargeted("test:bad-template", "bad-template", "Bad");
        emote.templates.untargeted.as_mut().unwrap().actor = template.into();
        invalid.push(emote);
    }
    let mut doubled_actor = optional_smile();
    doubled_actor.templates.targeted.as_mut().unwrap().others =
        "{actor} greets {actor} and {target}.".into();
    invalid.push(doubled_actor);
    let mut missing_target = optional_smile();
    missing_target.templates.targeted.as_mut().unwrap().actor = "You smile.".into();
    invalid.push(missing_target);
    let mut target_mentions_target = optional_smile();
    target_mentions_target
        .templates
        .targeted
        .as_mut()
        .unwrap()
        .target = "{actor} smiles at {target}.".into();
    invalid.push(target_mentions_target);

    let mut oversized_render = optional_smile();
    oversized_render.templates.targeted.as_mut().unwrap().actor = format!(
        "{}{{target}}",
        "x".repeat(MAX_SOCIAL_EMOTE_RENDER_BYTES - 63)
    );
    invalid.push(oversized_render);
    let mut unsafe_name = optional_smile();
    unsafe_name.name = "Smile<script>".into();
    invalid.push(unsafe_name);
    let mut oversized_name = optional_smile();
    oversized_name.name = "x".repeat(MAX_SOCIAL_EMOTE_NAME_BYTES + 1);
    invalid.push(oversized_name);
    let mut oversized_id = optional_smile();
    oversized_id.id = "x".repeat(MAX_SOCIAL_EMOTE_ID_BYTES + 1);
    invalid.push(oversized_id);
    let mut invalid_command = optional_smile();
    invalid_command.command = "Smile".into();
    invalid.push(invalid_command);

    for definition in invalid {
        let mut world = world_with(Vec::new());
        assert!(matches!(
            world.configure_social_emotes(vec![definition]),
            Err(WorldError::InvalidEmoteDefinition)
        ));
    }

    let mut exact_render = optional_smile();
    exact_render.templates.targeted.as_mut().unwrap().actor = format!(
        "{}{{target}}",
        "x".repeat(MAX_SOCIAL_EMOTE_RENDER_BYTES - MAX_SOCIAL_EMOTE_TARGET_BYTES)
    );
    assert!(
        world_with(Vec::new())
            .configure_social_emotes(vec![exact_render])
            .is_ok()
    );
}

#[test]
fn preflight_and_render_are_local_perspective_specific_and_read_only() {
    let mut world = world_with(vec![
        untargeted("test:wave", "wave", "Wave"),
        optional_smile(),
        required_bow(),
    ]);
    connect(&mut world, 1, 1, "Aria", ROOM);
    connect(&mut world, 2, 2, "Borin", ROOM);
    connect(&mut world, 3, 3, "Cora", ROOM);
    connect(&mut world, 4, 4, "Dain", REMOTE_ROOM);
    let snapshot = world.runtime_snapshot();
    let revision = world.revision();

    let untargeted = world.emote_preflight(SessionId(1), "smile", None).unwrap();
    assert_eq!(untargeted.actor_character_id, CharacterId(1));
    assert_eq!(untargeted.target_session_id, None);
    assert_eq!(
        untargeted.other_session_ids,
        vec![SessionId(2), SessionId(3)]
    );
    assert_eq!(
        world.render_emote(&untargeted).unwrap(),
        tiamud_core::SocialEmoteRender {
            actor: "You smile.".into(),
            target: None,
            others: "Aria smiles.".into(),
        }
    );

    let targeted = world
        .emote_preflight(SessionId(1), "smile", Some("bOrIn"))
        .unwrap();
    assert_eq!(targeted.target_session_id, Some(SessionId(2)));
    assert_eq!(targeted.target_character_id, Some(CharacterId(2)));
    assert_eq!(targeted.other_session_ids, vec![SessionId(3)]);
    assert_eq!(
        world.render_emote(&targeted).unwrap(),
        tiamud_core::SocialEmoteRender {
            actor: "You smile at Borin.".into(),
            target: Some("Aria smiles at you.".into()),
            others: "Aria smiles at Borin.".into(),
        }
    );
    assert_eq!(world.runtime_snapshot(), snapshot);
    assert_eq!(world.revision(), revision);
}

#[test]
fn targeting_rejects_wrong_shape_self_remote_unknown_ambiguous_and_stale_plans() {
    let mut world = world_with(vec![
        untargeted("test:wave", "wave", "Wave"),
        optional_smile(),
        required_bow(),
    ]);
    connect(&mut world, 1, 1, "Aria", ROOM);
    connect(&mut world, 2, 2, "Borin", ROOM);
    connect(&mut world, 3, 3, "Cora", REMOTE_ROOM);

    for result in [
        world.emote_preflight(SessionId(1), "wave", Some("Borin")),
        world.emote_preflight(SessionId(1), "bow", None),
        world.emote_preflight(SessionId(1), "smile", Some("Aria")),
        world.emote_preflight(SessionId(1), "smile", Some("Cora")),
        world.emote_preflight(SessionId(1), "smile", Some("Nobody")),
        world.emote_preflight(SessionId(1), "missing", None),
        world.emote_preflight(SessionId(1), "smile", Some("opaque-deadbeef")),
    ] {
        assert!(result.is_err());
    }

    let stale = world
        .emote_preflight(SessionId(1), "bow", Some("Borin"))
        .unwrap();
    world.disconnect(SessionId(2));
    assert!(world.render_emote(&stale).is_err());

    connect(&mut world, 2, 2, "Borin", ROOM);
    connect(&mut world, 5, 5, "Borin", ROOM);
    assert!(matches!(
        world.emote_preflight(SessionId(1), "smile", Some("Borin")),
        Err(WorldError::EmoteTargetUnavailable)
    ));
}

#[test]
fn placeholder_like_character_names_are_inserted_literally_once() {
    let mut world = world_with(vec![optional_smile()]);
    connect(&mut world, 1, 1, "{target}", ROOM);
    connect(&mut world, 2, 2, "{actor}", ROOM);
    connect(&mut world, 3, 3, "Witness", ROOM);

    let plan = world
        .emote_preflight(SessionId(1), "smile", Some("{actor}"))
        .unwrap();
    assert_eq!(
        world.render_emote(&plan).unwrap(),
        tiamud_core::SocialEmoteRender {
            actor: "You smile at {actor}.".into(),
            target: Some("{target} smiles at you.".into()),
            others: "{target} smiles at {actor}.".into(),
        }
    );
}

#[test]
fn globally_valid_markup_like_names_remain_usable_literal_text() {
    let mut world = world_with(vec![optional_smile()]);
    connect(&mut world, 1, 1, "A&B", ROOM);
    connect(&mut world, 2, 2, "<B>", ROOM);
    connect(&mut world, 3, 3, "Witness", ROOM);

    let plan = world
        .emote_preflight(SessionId(1), "smile", Some("<B>"))
        .unwrap();
    assert_eq!(
        world.render_emote(&plan).unwrap(),
        tiamud_core::SocialEmoteRender {
            actor: "You smile at <B>.".into(),
            target: Some("A&B smiles at you.".into()),
            others: "A&B smiles at <B>.".into(),
        }
    );
}

#[test]
fn core_social_actions_remain_server_routed_and_cannot_create_domain_events() {
    let mut world = world_with(vec![optional_smile()]);
    connect(&mut world, 1, 1, "Aria", ROOM);
    let snapshot = world.runtime_snapshot();
    let revision = world.revision();
    for action in [
        SocialAction::EmoteList,
        SocialAction::Emote {
            command: "smile".into(),
            target: None,
        },
    ] {
        assert!(matches!(
            world.apply(SessionId(1), PlayerAction::Social(action)),
            Err(WorldError::GameplayUnavailable)
        ));
    }
    assert_eq!(world.runtime_snapshot(), snapshot);
    assert_eq!(world.revision(), revision);
}
