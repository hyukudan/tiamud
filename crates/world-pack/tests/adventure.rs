//! WorldPack v10 / AreaPack v9 quests, commodities and shops (ADR 0032).

use std::{fs, process::Command};

use serde_json::{Value, json};
use world_pack::{
    AreaPack, CURRENT_SCHEMA_VERSION, CompositionError, MAX_QUEST_STAGES, MAX_QUESTS,
    MAX_SHOP_STOCK_ENTRIES, ValidationError, WorldPackDocument, compose_world_project,
    legacy_adventure_content,
};

const REVIEWED_WORLD: &[u8] = include_bytes!("../../../worlds/lantern-square.world.json");

type Mutation = Box<dyn Fn(&mut Value)>;

fn reviewed() -> Value {
    serde_json::from_slice(REVIEWED_WORLD).unwrap()
}

fn parse(value: &Value) -> Result<WorldPackDocument, world_pack::DocumentToolError> {
    WorldPackDocument::from_json(&serde_json::to_vec(value).unwrap())
}

fn errors(value: &Value) -> Vec<ValidationError> {
    parse(value)
        .unwrap()
        .validate()
        .err()
        .map_or_else(Vec::new, |errors| errors.0)
}

fn has_error(value: &Value, needle: &str) -> bool {
    errors(value).iter().any(|error| {
        matches!(error, ValidationError::InvalidGameplay(message) if message.contains(needle))
    })
}

fn errand() -> Value {
    json!({
        "id": "demo:errand",
        "name": {"en": "Gate Errand", "es": "Recado de la puerta"},
        "summary": {"en": "Visit the gate, then deal with the stalker."},
        "aliases": ["errand"],
        "prerequisite": {"all": [
            {"gte": ["actor.level", 1]},
            {"contains": ["actor.quest_states", "first-hunt:completed"]}
        ]},
        "repeatability": "repeatable",
        "stages": [
            {"id": "walk", "summary": {"en": "Walk to the north gate."},
             "objectives": [{"kind": "visit_room", "roomId": "demo:north-gate"}]},
            {"id": "fight", "objectives": [
                {"kind": "defeat_npc", "npcPrototypeId": "tiamud:gate-stalker", "count": 2},
                {"kind": "collect_item", "itemPrototypeId": "tiamud:road-token", "count": 1},
                {"kind": "talk_to_npc", "npcPrototypeId": "tiamud:gate-stalker"}
            ]}
        ],
        "rewards": {"coins": 3, "experience": 7}
    })
}

#[test]
fn reviewed_world_declares_exactly_the_formerly_hardcoded_content() {
    let document = WorldPackDocument::from_json(REVIEWED_WORLD).unwrap();
    document.validate().unwrap();
    assert_eq!(document.world.schema_version, CURRENT_SCHEMA_VERSION);
    let (commodities, quests, shops) = legacy_adventure_content(&document.world.start_room_id);
    assert_eq!(document.world.gameplay.commodities, commodities);
    assert_eq!(document.world.gameplay.quests, quests);
    assert_eq!(document.world.gameplay.shops, shops);
}

#[test]
fn pre_v10_world_packs_normalize_with_the_implied_legacy_content() {
    let mut legacy = reviewed();
    legacy["schemaVersion"] = json!(9);
    for field in ["commodities", "quests", "shops"] {
        legacy["gameplay"].as_object_mut().unwrap().remove(field);
    }
    let normalized = parse(&legacy).unwrap();
    normalized.validate().unwrap();
    assert_eq!(normalized.world.schema_version, CURRENT_SCHEMA_VERSION);
    let current = WorldPackDocument::from_json(REVIEWED_WORLD).unwrap();
    assert_eq!(
        normalized.deterministic_artifact().unwrap(),
        current.deterministic_artifact().unwrap()
    );

    for field in ["commodities", "quests", "shops"] {
        let mut smuggled = legacy.clone();
        smuggled["gameplay"][field] = json!([]);
        let error = parse(&smuggled).unwrap_err();
        assert!(error.to_string().contains("v10"), "{field}: {error}");
    }
}

#[test]
fn quest_and_shop_contract_round_trips_and_is_canonical() {
    let mut value = reviewed();
    value["gameplay"]["quests"]
        .as_array_mut()
        .unwrap()
        .insert(0, errand());
    let document = parse(&value).unwrap();
    document.validate().unwrap();
    let first = document.deterministic_artifact().unwrap();
    let reparsed = WorldPackDocument::from_json(&first).unwrap();
    assert_eq!(reparsed.deterministic_artifact().unwrap(), first);
    let ids = reparsed
        .world
        .gameplay
        .quests
        .iter()
        .map(|quest| quest.id.as_str())
        .collect::<Vec<_>>();
    assert_eq!(ids, ["demo:errand", "first-hunt"]);
    // Stage and objective order is semantic and preserved.
    assert_eq!(reparsed.world.gameplay.quests[0].stages[0].id, "walk");
    assert_eq!(
        reparsed.world.gameplay.quests[0].stages[1].objectives.len(),
        3
    );

    let diff = WorldPackDocument::from_json(REVIEWED_WORLD)
        .unwrap()
        .diff(&reparsed)
        .unwrap();
    assert_eq!(diff.quests.added, ["demo:errand"]);
    assert!(diff.to_string().contains("quests"));
}

#[test]
fn validator_rejects_dangling_references_and_unbounded_shapes() {
    let cases: Vec<(Mutation, &str)> = vec![
        (
            Box::new(|value| {
                value["gameplay"]["quests"][0]["stages"][0]["objectives"][0] =
                    json!({"kind": "defeat_npc", "npcPrototypeId": "demo:ghost", "count": 1});
            }),
            "unknown NPC prototype",
        ),
        (
            Box::new(|value| {
                value["gameplay"]["quests"][0]["stages"][0]["objectives"][0] =
                    json!({"kind": "collect_item", "itemPrototypeId": "demo:ghost", "count": 1});
            }),
            "unknown item prototype",
        ),
        (
            Box::new(|value| {
                value["gameplay"]["quests"][0]["stages"][0]["objectives"][0] =
                    json!({"kind": "visit_room", "roomId": "demo:ghost"});
            }),
            "unknown room",
        ),
        (
            Box::new(|value| {
                value["gameplay"]["quests"][0]["stages"][0]["objectives"][0] =
                    json!({"kind": "defeat_npc", "count": 0});
            }),
            "count must be",
        ),
        (
            Box::new(|value| {
                let stage = value["gameplay"]["quests"][0]["stages"][0].clone();
                value["gameplay"]["quests"][0]["stages"] = Value::Array(
                    (0..=MAX_QUEST_STAGES)
                        .map(|index| {
                            let mut stage = stage.clone();
                            stage["id"] = json!(format!("stage-{index}"));
                            stage
                        })
                        .collect(),
                );
            }),
            "stages",
        ),
        (
            Box::new(|value| {
                value["gameplay"]["quests"][0]["prerequisite"] =
                    json!({"eq": ["world.weather", "rain"]});
            }),
            "prerequisite may only use",
        ),
        (
            Box::new(|value| {
                value["gameplay"]["quests"][0]["aliases"] = json!(["first-hunt"]);
            }),
            "ambiguous",
        ),
        (
            Box::new(|value| {
                value["gameplay"]["quests"][0]["rewards"]["coins"] = json!(1_000_001);
            }),
            "rewards are limited",
        ),
        (
            Box::new(|value| {
                value["gameplay"]["shops"][0]["stock"][0]["sellPrice"] = json!(13);
            }),
            "sellPrice <= buyPrice",
        ),
        (
            Box::new(|value| {
                value["gameplay"]["shops"][0]["stock"][0]["commodityId"] = json!("demo:ghost");
            }),
            "unknown commodity",
        ),
        (
            Box::new(|value| {
                value["gameplay"]["shops"][0]["stock"][1]["stockId"] = json!("tonic");
            }),
            "stockId",
        ),
        (
            Box::new(|value| {
                value["gameplay"]["shops"][0]["stock"][0]["initialStock"] = json!(101);
            }),
            "initialStock <= maxStock",
        ),
        (
            Box::new(|value| {
                value["gameplay"]["shops"][0]["roomId"] = json!("demo:ghost");
            }),
            "unknown room",
        ),
        (
            Box::new(|value| {
                value["gameplay"]["shops"][0]["keeperNpcSpawnId"] = json!("tiamud:gate-stalker-1");
            }),
            "keeper",
        ),
        (
            Box::new(|value| {
                value["gameplay"]["shops"][0]["stock"][0]["restock"] =
                    json!({"kind": "interval", "everyTicks": 0, "amount": 1, "ceiling": 1});
            }),
            "interval restock",
        ),
        (
            Box::new(|value| {
                let entry = value["gameplay"]["shops"][0]["stock"][0].clone();
                value["gameplay"]["shops"][0]["stock"] = Value::Array(
                    (0..=MAX_SHOP_STOCK_ENTRIES)
                        .map(|index| {
                            let mut entry = entry.clone();
                            entry["stockId"] = json!(format!("stock-{index}"));
                            entry
                        })
                        .collect(),
                );
            }),
            "stock entries",
        ),
        (
            Box::new(|value| {
                let shop = value["gameplay"]["shops"][0].clone();
                let mut second = shop;
                second["id"] = json!("demo:second-market");
                second["stock"][0]["stockId"] = json!("second-tonic");
                second["stock"][1]["stockId"] = json!("second-ration");
                value["gameplay"]["shops"]
                    .as_array_mut()
                    .unwrap()
                    .push(second);
            }),
            "more than one shop",
        ),
        (
            Box::new(|value| {
                value["gameplay"]["commodities"][0]["maxOwned"] = json!(0);
            }),
            "maxOwned",
        ),
    ];
    for (mutate, needle) in cases {
        let mut value = reviewed();
        mutate(&mut value);
        assert!(
            has_error(&value, needle),
            "expected {needle:?}: {:?}",
            errors(&value)
        );
    }

    let quest = reviewed()["gameplay"]["quests"][0].clone();
    let mut too_many = reviewed();
    too_many["gameplay"]["quests"] = Value::Array(
        (0..=MAX_QUESTS)
            .map(|index| {
                let mut quest = quest.clone();
                quest["id"] = json!(format!("demo:quest-{index}"));
                quest["aliases"] = json!([]);
                quest
            })
            .collect(),
    );
    assert!(has_error(&too_many, "quests are allowed"));

    let mut unknown = reviewed();
    unknown["gameplay"]["quests"][0]["stages"][0]["objectives"][0]["future"] = json!(true);
    assert!(parse(&unknown).is_err());
}

#[test]
fn area_pack_v9_owns_adventure_content_and_rejects_v8_smuggling() {
    let base =
        json!({"schemaVersion": 9, "id": "demo", "quests": [], "shops": [], "commodities": []});
    let parsed: AreaPack = serde_json::from_value(base.clone()).unwrap();
    assert!(parsed.quests.is_empty());
    for field in ["commodities", "quests", "shops"] {
        let mut legacy = json!({"schemaVersion": 8, "id": "demo"});
        legacy[field] = json!([]);
        let error = serde_json::from_value::<AreaPack>(legacy).unwrap_err();
        assert!(error.to_string().contains("v9"), "{field}: {error}");
    }
}

#[test]
fn composition_rejects_duplicate_quest_ids_across_areas() {
    let temp = tempfile::tempdir().unwrap();
    let quest = reviewed()["gameplay"]["quests"][0].clone();
    for (id, room) in [("a", "a:start"), ("b", "b:start")] {
        let area = json!({
            "schemaVersion": 9,
            "id": id,
            "rooms": [{"id": room, "title": "Room", "description": "A room."}],
            "quests": [quest.clone()]
        });
        fs::write(
            temp.path().join(format!("{id}.json")),
            serde_json::to_vec(&area).unwrap(),
        )
        .unwrap();
    }
    let manifest = temp.path().join("world.project.json");
    fs::write(
        &manifest,
        serde_json::to_vec(&json!({
            "schemaVersion": 2,
            "id": "demo",
            "name": "Demo",
            "startRoomId": "a:start",
            "areas": [{"id": "a", "path": "a.json"}, {"id": "b", "path": "b.json"}]
        }))
        .unwrap(),
    )
    .unwrap();
    assert!(matches!(
        compose_world_project(&manifest),
        Err(CompositionError::DuplicateGlobalEntityId {
            first_kind: "quest",
            ..
        })
    ));
}

#[test]
fn cli_validate_accepts_v10_content_and_rejects_invalid_quests() {
    let temp = tempfile::tempdir().unwrap();
    let valid = temp.path().join("valid.json");
    let invalid = temp.path().join("invalid.json");
    let mut value = reviewed();
    value["gameplay"]["quests"]
        .as_array_mut()
        .unwrap()
        .push(errand());
    fs::write(&valid, serde_json::to_vec(&value).unwrap()).unwrap();
    value["gameplay"]["quests"][1]["stages"][0]["objectives"][0]["roomId"] = json!("demo:nowhere");
    fs::write(&invalid, serde_json::to_vec(&value).unwrap()).unwrap();

    let run = |path: &std::path::Path| {
        Command::new(env!("CARGO_BIN_EXE_tiamud-world"))
            .args(["validate", path.to_str().unwrap()])
            .output()
            .unwrap()
    };
    let output = run(&valid);
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let output = run(&invalid);
    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("validation failed"), "{stderr}");
}
