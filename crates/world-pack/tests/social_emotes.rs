use std::fs;

use serde_json::{Value, json};
use world_pack::{
    AREA_PACK_SCHEMA_VERSION, AreaPack, CURRENT_SCHEMA_VERSION, CompositionError,
    MAX_SOCIAL_EMOTES, SocialEmoteTargeting, ValidationError, WorldPackDocument,
    compose_world_project,
};

fn has_gameplay_error(errors: &[ValidationError], needle: &str) -> bool {
    errors.iter().any(
        |error| matches!(error, ValidationError::InvalidGameplay(message) if message.contains(needle)),
    )
}

fn optional_smile() -> Value {
    json!({
        "id": "demo:smile",
        "command": "smile",
        "name": "Sonreír",
        "audience": "room",
        "targeting": "optional_character",
        "templates": {
            "untargeted": {
                "actor": "Sonríes.",
                "others": "{actor} sonríe."
            },
            "targeted": {
                "actor": "Sonríes a {target}.",
                "target": "{actor} te sonríe.",
                "others": "{actor} sonríe a {target}."
            }
        }
    })
}

fn world(emotes: Vec<Value>) -> Value {
    json!({
        "schemaVersion": CURRENT_SCHEMA_VERSION,
        "id": "demo",
        "name": "Demo",
        "startRoomId": "demo:start",
        "rooms": [{
            "id": "demo:start",
            "title": "Inicio",
            "description": "Una sala de prueba."
        }],
        "gameplay": { "socialEmotes": emotes }
    })
}

fn parse_world(value: &Value) -> Result<WorldPackDocument, world_pack::DocumentToolError> {
    WorldPackDocument::from_json(&serde_json::to_vec(value).unwrap())
}

#[test]
fn social_emote_contract_round_trips_and_validates() {
    let document = parse_world(&world(vec![optional_smile()])).unwrap();
    document.validate().unwrap();
    let emote = &document.world.gameplay.social_emotes[0];
    assert_eq!(emote.command, "smile");
    assert_eq!(emote.targeting, SocialEmoteTargeting::OptionalCharacter);

    let artifact = document.deterministic_artifact().unwrap();
    let reparsed = WorldPackDocument::from_json(&artifact).unwrap();
    assert_eq!(reparsed, document);
}

#[test]
fn social_emotes_are_canonical_and_visible_in_semantic_diff() {
    let smile = optional_smile();
    let mut wave = optional_smile();
    wave["id"] = json!("demo:wave");
    wave["command"] = json!("wave");
    wave["name"] = json!("Saludar");

    let first = parse_world(&world(vec![wave.clone(), smile.clone()])).unwrap();
    let reordered = parse_world(&world(vec![smile, wave])).unwrap();
    assert_eq!(
        first.deterministic_artifact().unwrap(),
        reordered.deterministic_artifact().unwrap()
    );

    let mut changed = reordered.clone();
    changed.world.gameplay.social_emotes[0]
        .templates
        .untargeted
        .as_mut()
        .unwrap()
        .actor = "Sonríes con calma.".to_owned();
    let diff = reordered.diff(&changed).unwrap();
    assert_eq!(diff.social_emotes.changed, ["demo:smile"]);
    assert!(diff.to_string().contains("social emotes: ~ demo:smile"));
}

#[test]
fn targeting_requires_the_exact_template_shape() {
    let cases = [
        ("none", true, false),
        ("optional_character", true, true),
        ("required_character", false, true),
    ];
    for (targeting, untargeted, targeted) in cases {
        let mut emote = optional_smile();
        emote["targeting"] = json!(targeting);
        if !untargeted {
            emote["templates"]
                .as_object_mut()
                .unwrap()
                .remove("untargeted");
        }
        if !targeted {
            emote["templates"]
                .as_object_mut()
                .unwrap()
                .remove("targeted");
        }
        parse_world(&world(vec![emote]))
            .unwrap()
            .validate()
            .unwrap();
    }

    let mut missing = optional_smile();
    missing["templates"]
        .as_object_mut()
        .unwrap()
        .remove("targeted");
    let error = parse_world(&world(vec![missing]))
        .unwrap()
        .validate()
        .unwrap_err();
    assert!(has_gameplay_error(&error.0, "targeting mode"), "{error:?}");
}

#[test]
fn templates_reject_unsafe_text_placeholders_and_oversized_rendering() {
    let cases = [
        ("untargeted", "actor", "{actor} sonríes."),
        ("untargeted", "others", "Alguien sonríe."),
        ("targeted", "actor", "Sonríes a {actor}."),
        ("targeted", "target", "{target} te sonríe."),
        ("targeted", "others", "{actor} sonríe."),
        ("targeted", "others", "{actor} sonríe a {victim}."),
        ("targeted", "others", "{actor} <b>{target}</b>"),
        ("targeted", "others", "{actor} & {target}"),
        ("untargeted", "actor", "Sonríes.\nOtra línea."),
        ("untargeted", "actor", " Sonríes."),
        ("untargeted", "actor", "Sonríes. "),
    ];
    for (section, field, value) in cases {
        let mut emote = optional_smile();
        emote["templates"][section][field] = json!(value);
        assert!(
            parse_world(&world(vec![emote]))
                .unwrap()
                .validate()
                .is_err(),
            "accepted {section}.{field}: {value:?}"
        );
    }

    let mut oversized = optional_smile();
    oversized["templates"]["targeted"]["others"] =
        json!(format!("{{actor}} {} {{target}}", "x".repeat(390)));
    assert!(
        parse_world(&world(vec![oversized]))
            .unwrap()
            .validate()
            .is_err()
    );
}

#[test]
fn ids_commands_names_counts_and_nested_fields_are_closed() {
    let mut duplicate = optional_smile();
    duplicate["id"] = json!("demo:grin");
    let error = parse_world(&world(vec![optional_smile(), duplicate]))
        .unwrap()
        .validate()
        .unwrap_err();
    assert!(
        has_gameplay_error(&error.0, "command \"smile\""),
        "{error:?}"
    );

    for (field, value) in [
        ("id", format!("demo:{}", "a".repeat(64))),
        ("command", "Uppercase".to_owned()),
        ("command", format!("a{}", "b".repeat(32))),
        ("name", "ñ".repeat(33)),
        ("name", " Sonreír".to_owned()),
        ("name", "Sonreír ".to_owned()),
        ("name", "Son<reír".to_owned()),
        ("name", "Sonreír & saludar".to_owned()),
    ] {
        let mut emote = optional_smile();
        emote[field] = json!(value);
        assert!(
            parse_world(&world(vec![emote]))
                .unwrap()
                .validate()
                .is_err(),
            "accepted invalid {field}"
        );
    }

    let mut too_many = Vec::new();
    for index in 0..=MAX_SOCIAL_EMOTES {
        let mut emote = optional_smile();
        emote["id"] = json!(format!("demo:emote-{index}"));
        emote["command"] = json!(format!("emote-{index}"));
        too_many.push(emote);
    }
    assert!(parse_world(&world(too_many)).unwrap().validate().is_err());

    let mut unknown = optional_smile();
    unknown["templates"]["targeted"]["future"] = json!(true);
    assert!(parse_world(&world(vec![unknown])).is_err());
}

#[test]
fn worldpack_v2_through_v8_normalize_empty_and_reject_social_emote_smuggling() {
    for version in 2..CURRENT_SCHEMA_VERSION {
        let mut legacy = world(Vec::new());
        legacy["schemaVersion"] = json!(version);
        legacy["gameplay"]
            .as_object_mut()
            .unwrap()
            .remove("socialEmotes");
        let normalized = parse_world(&legacy).unwrap();
        assert_eq!(normalized.world.schema_version, CURRENT_SCHEMA_VERSION);
        assert!(normalized.world.gameplay.social_emotes.is_empty());

        legacy["gameplay"]["socialEmotes"] = json!([]);
        let error = parse_world(&legacy).unwrap_err();
        assert!(
            error.to_string().contains("socialEmotes"),
            "v{version}: {error}"
        );
    }
}

#[test]
fn areapack_v1_through_v7_normalize_empty_and_reject_social_emote_smuggling() {
    for version in 1..AREA_PACK_SCHEMA_VERSION {
        let legacy = json!({"schemaVersion": version, "id": "demo"});
        let normalized: AreaPack = serde_json::from_value(legacy.clone()).unwrap();
        assert_eq!(normalized.schema_version, AREA_PACK_SCHEMA_VERSION);
        assert!(normalized.social_emotes.is_empty());

        let mut smuggled = legacy;
        smuggled["socialEmotes"] = json!([]);
        let error = serde_json::from_value::<AreaPack>(smuggled).unwrap_err();
        assert!(
            error.to_string().contains("socialEmotes"),
            "v{version}: {error}"
        );
    }
}

#[test]
fn previous_telepathy_schemas_survive_the_emote_upgrade() {
    let ability = json!({
        "id": "demo:whisper",
        "name": "Susurro mental",
        "description": "Una comunicación revisada.",
        "category": "utility",
        "resourceCosts": {"focus": 1},
        "cooldownTicks": 1,
        "targeting": "single_ally",
        "communication": {
            "kind": "telepathy",
            "scope": "direct_online",
            "blockedRoomIds": []
        }
    });

    let mut previous_world = world(Vec::new());
    previous_world["schemaVersion"] = json!(8);
    previous_world["gameplay"]
        .as_object_mut()
        .unwrap()
        .remove("socialEmotes");
    previous_world["gameplay"]["abilities"] = json!([ability.clone()]);
    let normalized = parse_world(&previous_world).unwrap();
    normalized.validate().unwrap();
    assert!(
        normalized.world.gameplay.abilities[0]
            .communication
            .is_some()
    );
    assert!(normalized.world.gameplay.social_emotes.is_empty());

    let previous_area = json!({
        "schemaVersion": 7,
        "id": "demo",
        "abilities": [ability]
    });
    let normalized: AreaPack = serde_json::from_value(previous_area).unwrap();
    assert!(normalized.abilities[0].communication.is_some());
    assert!(normalized.social_emotes.is_empty());
}

#[test]
fn composition_is_deterministic_and_rejects_duplicate_commands_across_areas() {
    let temp = tempfile::tempdir().unwrap();
    let area = |id: &str, emote_id: &str| {
        let mut emote = optional_smile();
        emote["id"] = json!(emote_id);
        json!({
            "schemaVersion": AREA_PACK_SCHEMA_VERSION,
            "id": id,
            "rooms": [{
                "id": format!("{id}:start"),
                "title": id,
                "description": "Una sala de composición."
            }],
            "socialEmotes": [emote]
        })
    };
    fs::write(
        temp.path().join("a.json"),
        serde_json::to_vec_pretty(&area("a", "a:smile")).unwrap(),
    )
    .unwrap();
    fs::write(
        temp.path().join("b.json"),
        serde_json::to_vec_pretty(&area("b", "b:smile")).unwrap(),
    )
    .unwrap();
    let manifest = json!({
        "schemaVersion": 2,
        "id": "demo",
        "name": "Demo",
        "startRoomId": "a:start",
        "areas": [
            {"id": "a", "path": "a.json"},
            {"id": "b", "path": "b.json"}
        ]
    });
    let manifest_path = temp.path().join("world.project.json");
    fs::write(
        &manifest_path,
        serde_json::to_vec_pretty(&manifest).unwrap(),
    )
    .unwrap();
    let error = compose_world_project(&manifest_path).unwrap_err();
    assert!(
        matches!(&error, CompositionError::InvalidWorld(errors) if has_gameplay_error(&errors.0, "command \"smile\"")),
        "{error:?}"
    );

    let mut second = area("b", "b:wave");
    second["socialEmotes"][0]["command"] = json!("wave");
    fs::write(
        temp.path().join("b.json"),
        serde_json::to_vec_pretty(&second).unwrap(),
    )
    .unwrap();
    let first = compose_world_project(&manifest_path).unwrap();
    let second = compose_world_project(&manifest_path).unwrap();
    assert_eq!(first.artifact, second.artifact);
    assert_eq!(first.report.artifact_sha256, second.report.artifact_sha256);
    assert_eq!(first.report.areas[0].ownership.social_emotes, ["a:smile"]);
    assert_eq!(first.report.areas[1].ownership.social_emotes, ["b:wave"]);
}
