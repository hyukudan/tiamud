//! Keeps the published StoryPack JSON Schema aligned with the typed loader.
//!
//! No JSON Schema dependency is added: this file implements exactly the
//! keyword subset the schema uses and fails on any other keyword or pattern,
//! so the schema cannot silently grow beyond what is checked here.

use serde_json::{Value, json};
use tiamud_story_core::{MAX_SCRIPT_ARGS, MAX_SCRIPT_GRANTS, MAX_SCRIPTS, StoryPack};
use tiamud_story_tools::{
    STORY_PACK_SCHEMA, STORY_PACK_SCHEMA_V1, STORY_PACK_SCHEMA_V2, format::canonical_json,
};

const ID_PATTERN: &str = "^[a-z0-9][a-z0-9._-]{0,63}$";
const LOCALE_PATTERN: &str = "^[A-Za-z][A-Za-z0-9-]{1,34}$";
const SHA256_PATTERN: &str = "^[0-9a-f]{64}$";

fn matches_pattern(pattern: &str, text: &str) -> bool {
    let bytes = text.as_bytes();
    match pattern {
        ID_PATTERN => {
            (1..=64).contains(&bytes.len())
                && (bytes[0].is_ascii_lowercase() || bytes[0].is_ascii_digit())
                && bytes.iter().all(|b| {
                    b.is_ascii_lowercase() || b.is_ascii_digit() || matches!(b, b'.' | b'_' | b'-')
                })
        }
        LOCALE_PATTERN => {
            (2..=35).contains(&bytes.len())
                && bytes[0].is_ascii_alphabetic()
                && bytes
                    .iter()
                    .all(|b| b.is_ascii_alphanumeric() || *b == b'-')
        }
        SHA256_PATTERN => {
            bytes.len() == 64
                && bytes
                    .iter()
                    .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(b))
        }
        other => panic!("schema uses an unchecked pattern {other:?}"),
    }
}

fn resolve<'a>(root: &'a Value, reference: &str) -> &'a Value {
    let name = reference
        .strip_prefix("#/$defs/")
        .unwrap_or_else(|| panic!("unsupported $ref {reference:?}"));
    &root["$defs"][name]
}

fn type_matches(kind: &str, value: &Value) -> bool {
    match kind {
        "object" => value.is_object(),
        "array" => value.is_array(),
        "string" => value.is_string(),
        "boolean" => value.is_boolean(),
        "integer" => value.is_i64() || value.is_u64(),
        "null" => value.is_null(),
        other => panic!("unsupported type {other:?}"),
    }
}

fn valid(root: &Value, schema: &Value, value: &Value) -> bool {
    let schema = schema.as_object().expect("schema must be an object");
    for (keyword, rule) in schema {
        let ok = match keyword.as_str() {
            "$schema" | "$id" | "$defs" | "title" | "description" => true,
            "$ref" => valid(root, resolve(root, rule.as_str().unwrap()), value),
            "type" => type_matches(rule.as_str().unwrap(), value),
            "const" => value == rule,
            "enum" => rule.as_array().unwrap().contains(value),
            "oneOf" => {
                rule.as_array()
                    .unwrap()
                    .iter()
                    .filter(|branch| valid(root, branch, value))
                    .count()
                    == 1
            }
            "pattern" => value
                .as_str()
                .is_none_or(|text| matches_pattern(rule.as_str().unwrap(), text)),
            "minLength" => value
                .as_str()
                .is_none_or(|text| text.chars().count() as u64 >= rule.as_u64().unwrap()),
            "minimum" => value
                .as_i64()
                .is_none_or(|number| number >= rule.as_i64().unwrap()),
            "maximum" => value
                .as_i64()
                .is_none_or(|number| number <= rule.as_i64().unwrap()),
            "minItems" => value
                .as_array()
                .is_none_or(|items| items.len() as u64 >= rule.as_u64().unwrap()),
            "maxItems" => value
                .as_array()
                .is_none_or(|items| items.len() as u64 <= rule.as_u64().unwrap()),
            "items" => value
                .as_array()
                .is_none_or(|items| items.iter().all(|item| valid(root, rule, item))),
            "minProperties" => value
                .as_object()
                .is_none_or(|map| map.len() as u64 >= rule.as_u64().unwrap()),
            "maxProperties" => value
                .as_object()
                .is_none_or(|map| map.len() as u64 <= rule.as_u64().unwrap()),
            "required" => value.as_object().is_none_or(|map| {
                rule.as_array()
                    .unwrap()
                    .iter()
                    .all(|name| map.contains_key(name.as_str().unwrap()))
            }),
            "properties" => value.as_object().is_none_or(|map| {
                rule.as_object().unwrap().iter().all(|(name, property)| {
                    map.get(name)
                        .is_none_or(|member| valid(root, property, member))
                })
            }),
            "propertyNames" => value.as_object().is_none_or(|map| {
                map.keys()
                    .all(|name| valid(root, rule, &Value::String(name.clone())))
            }),
            "additionalProperties" => value.as_object().is_none_or(|map| {
                let declared = schema.get("properties").and_then(Value::as_object);
                map.iter()
                    .filter(|(name, _)| declared.is_none_or(|known| !known.contains_key(*name)))
                    .all(|(_, member)| match rule {
                        Value::Bool(allowed) => *allowed,
                        other => valid(root, other, member),
                    })
            }),
            other => panic!("schema uses an unchecked keyword {other:?}"),
        };
        if !ok {
            return false;
        }
    }
    true
}

fn schema() -> Value {
    serde_json::from_str(STORY_PACK_SCHEMA).unwrap()
}

fn fixture() -> Value {
    serde_json::from_slice(include_bytes!(
        "../../story-core/fixtures/crypt-of-the-necromancer.story.json"
    ))
    .unwrap()
}

fn loader_accepts(value: &Value) -> bool {
    StoryPack::from_json_slice(&serde_json::to_vec(value).unwrap()).is_ok()
}

#[test]
fn reviewed_fixtures_and_canonical_output_satisfy_the_schema() {
    let root = schema();
    let original = fixture();
    assert!(valid(&root, &root, &original));
    let pack = StoryPack::from_json_slice(&serde_json::to_vec(&original).unwrap()).unwrap();
    let canonical: Value = serde_json::from_slice(&canonical_json(&pack).unwrap()).unwrap();
    assert!(valid(&root, &root, &canonical));

    let lints: Value = serde_json::from_slice(include_bytes!("fixtures/lints.story.json")).unwrap();
    assert!(valid(&root, &root, &lints));

    // The v2 menu sample and its canonical form.
    let noir = noir();
    assert!(valid(&root, &root, &noir));
    let pack = StoryPack::from_json_slice(&serde_json::to_vec(&noir).unwrap()).unwrap();
    let canonical: Value = serde_json::from_slice(&canonical_json(&pack).unwrap()).unwrap();
    assert!(valid(&root, &root, &canonical));
    let menu_lints: Value =
        serde_json::from_slice(include_bytes!("fixtures/menu-lints.story.json")).unwrap();
    assert!(valid(&root, &root, &menu_lints));

    // The historical v1 schema still describes v1 documents and rejects v2.
    let v1: Value = serde_json::from_str(STORY_PACK_SCHEMA_V1).unwrap();
    assert!(valid(&v1, &v1, &original));
    assert!(!valid(&v1, &v1, &noir));
}

fn noir() -> Value {
    serde_json::from_slice(include_bytes!(
        "../../story-core/fixtures/neon-rain.story.json"
    ))
    .unwrap()
}

#[test]
fn v2_shapes_the_loader_rejects_are_rejected_by_the_schema() {
    let root = schema();
    type Mutation = Box<dyn Fn(&mut Value)>;
    let mutations: Vec<Mutation> = vec![
        Box::new(|pack| pack["verbs"][0]["targets"] = json!("place")),
        Box::new(|pack| pack["verbs"][0]["icono"] = json!("ojo")),
        Box::new(|pack| {
            pack["verbs"][0].as_object_mut().unwrap().remove("default");
        }),
        Box::new(|pack| pack["characters"][0]["portraits"] = json!(["retrato"])),
        Box::new(|pack| pack["nodes"][0]["menu"]["entries"][0]["peso"] = json!(1)),
        Box::new(|pack| pack["nodes"][0]["menu"]["verbs"][0] = json!("mirar")),
        Box::new(|pack| pack["nodes"][0]["menu"]["entries"][0]["responses"][0]["once"] = json!(1)),
        Box::new(|pack| pack["nodes"][0]["hotspots"][0]["x"] = json!(10)),
        Box::new(|pack| {
            pack["nodes"][1]["present"][0]
                .as_object_mut()
                .unwrap()
                .remove("character");
        }),
        Box::new(|pack| pack["nodes"][0]["cues"] = json!(["play_music"])),
        Box::new(|pack| pack["dialogues"][0]["nodes"][1]["options"][0]["next"] = json!("continue")),
        Box::new(|pack| {
            pack["dialogues"][0]["nodes"][0]["lines"][0]["mood"] = json!("triste");
        }),
        Box::new(|pack| {
            pack["nodes"][0]["menu"]["entries"][2]["responses"][0]["when"] =
                json!({ "uses": { "node": "oficina", "entry": "x", "op": "about", "value": 1 } })
        }),
    ];
    for (index, mutate) in mutations.iter().enumerate() {
        let mut pack = noir();
        mutate(&mut pack);
        assert!(
            !loader_accepts(&pack),
            "v2 mutation {index} should be rejected by the loader"
        );
        assert!(
            !valid(&root, &root, &pack),
            "v2 mutation {index} should be rejected by the schema"
        );
    }
}

#[test]
fn every_effect_and_condition_form_satisfies_the_schema_and_the_loader() {
    let root = schema();
    let mut pack = fixture();
    pack["nodes"][0]["choices"][0]["when"] = json!({
        "all": [
            { "any": [{ "visited": "inicio" }, { "flag": "esqueleto_vencido" }] },
            { "not": { "has_item": "antorcha" } },
            { "var_cmp": { "var": "destreza", "op": "le", "value": 12 } }
        ]
    });
    pack["nodes"][0]["choices"][0]["effects"] = json!([
        { "set": { "var": "esqueleto_vencido", "value": false } },
        { "set": { "var": "destreza", "value": 3 } },
        { "add": { "var": "destreza", "delta": -1 } },
        { "give": "antorcha" },
        { "take": "antorcha" },
        { "roll": { "dice": { "count": 2, "sides": 6, "modifier": -1 }, "modifier_var": "destreza", "store": "tirada", "test": { "at_least": 7, "success": "vestibulo", "failure": "vestibulo" } } },
        { "if": { "when": { "flag": "esqueleto_vencido" }, "then": [{ "goto": "vestibulo" }], "otherwise": [{ "score": -5 }] } },
        { "cue": { "music": { "asset": "musica-combate" } } },
        { "cue": { "sfx": { "asset": "sfx-golpe" } } },
        { "cue": "stop_music" },
        { "score": 10 },
        { "end": { "id": "atajo", "kind": "neutral" } }
    ]);
    assert!(loader_accepts(&pack));
    assert!(valid(&root, &root, &pack));
}

#[test]
fn shapes_the_loader_rejects_are_rejected_by_the_schema() {
    let root = schema();
    type Mutation = Box<dyn Fn(&mut Value)>;
    let mutations: Vec<Mutation> = vec![
        Box::new(|pack| pack["sorpresa"] = json!(1)),
        Box::new(|pack| pack["format_version"] = json!(4)),
        Box::new(|pack| {
            pack.as_object_mut().unwrap().remove("start");
        }),
        Box::new(|pack| pack["variables"][0]["oculta"] = json!(true)),
        Box::new(|pack| pack["variables"][0]["kind"] = json!({ "float": { "initial": 0 } })),
        Box::new(|pack| pack["items"][0]["peso"] = json!(3)),
        Box::new(|pack| pack["nodes"][0]["extra"] = json!("x")),
        Box::new(|pack| pack["nodes"][0]["choices"][0]["peso"] = json!(1)),
        Box::new(|pack| pack["nodes"][0]["choices"][0]["when"] = json!({ "xor": [] })),
        Box::new(|pack| {
            pack["nodes"][0]["choices"][0]["when"] =
                json!({ "var_cmp": { "var": "destreza", "op": "approx", "value": 1 } })
        }),
        Box::new(|pack| pack["nodes"][0]["choices"][0]["effects"] = json!([{ "teleport": "x" }])),
        Box::new(|pack| {
            pack["nodes"][0]["choices"][0]["effects"] =
                json!([{ "roll": { "dice": { "count": 1, "sides": 6, "faces": 2 } } }])
        }),
        Box::new(|pack| {
            pack["nodes"][0]["choices"][0]["effects"] = json!([{ "cue": "play_music" }])
        }),
        Box::new(|pack| pack["nodes"][9]["ending"] = json!({ "id": "muerte", "kind": "tragedy" })),
    ];
    for (index, mutate) in mutations.iter().enumerate() {
        let mut pack = fixture();
        mutate(&mut pack);
        assert!(
            !loader_accepts(&pack),
            "mutation {index} should be rejected by the loader"
        );
        assert!(
            !valid(&root, &root, &pack),
            "mutation {index} should be rejected by the schema"
        );
    }
}

#[test]
fn schema_bounds_match_core_constants() {
    let root = schema();
    let defs = &root["$defs"];
    assert_eq!(
        root["properties"]["nodes"]["maxItems"],
        tiamud_story_core::MAX_NODES
    );
    assert_eq!(
        root["properties"]["variables"]["maxItems"],
        tiamud_story_core::MAX_VARIABLES
    );
    assert_eq!(
        root["properties"]["items"]["maxItems"],
        tiamud_story_core::MAX_ITEMS
    );
    assert_eq!(
        defs["node"]["properties"]["choices"]["maxItems"],
        tiamud_story_core::MAX_CHOICES_PER_NODE
    );
    assert_eq!(
        defs["choice"]["properties"]["effects"]["maxItems"],
        tiamud_story_core::MAX_EFFECTS_PER_CHOICE
    );
    assert_eq!(
        defs["localizedText"]["maxProperties"],
        tiamud_story_core::MAX_LOCALES
    );
    assert_eq!(defs["varInt"]["maximum"], tiamud_story_core::MAX_VAR_ABS);
    assert_eq!(
        defs["dice"]["properties"]["count"]["maximum"],
        tiamud_story_core::MAX_DICE_COUNT
    );
    assert_eq!(
        defs["dice"]["properties"]["sides"]["maximum"],
        tiamud_story_core::MAX_DICE_SIDES
    );
    assert_eq!(
        defs["dice"]["properties"]["modifier"]["maximum"],
        tiamud_story_core::MAX_DICE_MODIFIER
    );
    use tiamud_story_core::{
        MAX_CHARACTERS, MAX_CUES_PER_NODE, MAX_DIALOGUE_NODES, MAX_DIALOGUE_OPTIONS, MAX_DIALOGUES,
        MAX_EFFECTS_PER_CHOICE, MAX_EXPRESSIONS, MAX_HOTSPOTS_PER_NODE, MAX_MENU_ENTRIES_PER_NODE,
        MAX_PRESENT_PER_NODE, MAX_RESPONSES_PER_ENTRY, MAX_TEXT_BLOCKS, MAX_VERBS,
    };
    let props = |def: &str, field: &str| defs[def]["properties"][field].clone();
    assert_eq!(root["properties"]["verbs"]["maxItems"], MAX_VERBS);
    assert_eq!(root["properties"]["characters"]["maxItems"], MAX_CHARACTERS);
    assert_eq!(root["properties"]["dialogues"]["maxItems"], MAX_DIALOGUES);
    assert_eq!(props("node", "cues")["maxItems"], MAX_CUES_PER_NODE);
    assert_eq!(props("node", "hotspots")["maxItems"], MAX_HOTSPOTS_PER_NODE);
    assert_eq!(props("node", "present")["maxItems"], MAX_PRESENT_PER_NODE);
    assert_eq!(props("menu", "verbs")["maxItems"], MAX_VERBS);
    assert_eq!(
        props("menu", "entries")["maxItems"],
        MAX_MENU_ENTRIES_PER_NODE
    );
    assert_eq!(
        props("menuEntry", "responses")["maxItems"],
        MAX_RESPONSES_PER_ENTRY
    );
    assert_eq!(defs["textBlocks"]["maxItems"], MAX_TEXT_BLOCKS);
    assert_eq!(props("response", "blocks")["maxItems"], MAX_TEXT_BLOCKS);
    assert_eq!(props("verb", "default")["maxItems"], MAX_TEXT_BLOCKS);
    assert_eq!(props("dialogueNode", "lines")["maxItems"], MAX_TEXT_BLOCKS);
    assert_eq!(
        props("response", "effects")["maxItems"],
        MAX_EFFECTS_PER_CHOICE
    );
    assert_eq!(
        props("dialogueOption", "effects")["maxItems"],
        MAX_EFFECTS_PER_CHOICE
    );
    assert_eq!(
        props("character", "portraits")["maxProperties"],
        MAX_EXPRESSIONS
    );
    assert_eq!(props("dialogue", "nodes")["maxItems"], MAX_DIALOGUE_NODES);
    assert_eq!(
        props("dialogueNode", "options")["maxItems"],
        MAX_DIALOGUE_OPTIONS
    );
}

fn faro() -> Value {
    serde_json::from_slice(include_bytes!(
        "../../story-core/fixtures/parser/grey-point-lighthouse.story.json"
    ))
    .unwrap()
}

#[test]
fn v3_fixtures_satisfy_the_schema_and_older_schemas_reject_them() {
    let root = schema();
    for document in [
        faro(),
        serde_json::from_slice(include_bytes!("fixtures/world-lints.story.json")).unwrap(),
    ] {
        assert!(valid(&root, &root, &document));
        let pack = StoryPack::from_json_slice(&serde_json::to_vec(&document).unwrap()).unwrap();
        let canonical: Value = serde_json::from_slice(&canonical_json(&pack).unwrap()).unwrap();
        assert!(valid(&root, &root, &canonical));
        let v2: Value = serde_json::from_str(STORY_PACK_SCHEMA_V2).unwrap();
        assert!(!valid(&v2, &v2, &document));
    }
    // v1 and v2 documents still satisfy the v3 schema.
    assert!(valid(&root, &root, &fixture()));
    assert!(valid(&root, &root, &noir()));
}

#[test]
fn v3_shapes_the_loader_rejects_are_rejected_by_the_schema() {
    let root = schema();
    type Mutation = Box<dyn Fn(&mut Value)>;
    let mutations: Vec<Mutation> = vec![
        Box::new(|pack| pack["world"]["sorpresa"] = json!(1)),
        Box::new(|pack| pack["world"]["lexicons"][0]["id"] = json!(7)),
        Box::new(|pack| pack["world"]["objects"][0]["location"] = json!("everywhere")),
        Box::new(|pack| pack["world"]["objects"][0]["location"] = json!({ "under": "mesa" })),
        Box::new(|pack| pack["world"]["objects"][0]["portable"] = json!("si")),
        Box::new(|pack| pack["world"]["objects"][0]["gender"] = json!("neutral")),
        Box::new(|pack| pack["world"]["objects"][0]["peso"] = json!(2)),
        Box::new(|pack| {
            pack["world"]["objects"][0]
                .as_object_mut()
                .unwrap()
                .remove("nouns");
        }),
        Box::new(|pack| pack["world"]["rules"][0]["phase"] = json!("during")),
        Box::new(|pack| pack["world"]["messages"]["no_exit"] = json!("texto")),
        Box::new(|pack| pack["world"]["messages"]["inventado"] = json!({ "es": "x" })),
        Box::new(|pack| pack["nodes"][0]["room"]["exits"][0]["puerta"] = json!("x")),
        Box::new(|pack| pack["nodes"][0]["room"]["dark"] = json!(1)),
        Box::new(|pack| {
            pack["world"]["rules"][0]["when"] =
                json!({ "object_is": { "obj": "barca", "flag": "rota" } })
        }),
        Box::new(|pack| {
            pack["world"]["rules"][0]["effects"] =
                json!([{ "move_object": { "obj": "barca", "to": "lejos" } }])
        }),
        Box::new(|pack| {
            pack["world"]["lexicons"][0]["verbs"][0]["patterns"][0]["objeto"] = json!(true)
        }),
    ];
    for (index, mutate) in mutations.iter().enumerate() {
        let mut pack = faro();
        mutate(&mut pack);
        assert!(
            !loader_accepts(&pack),
            "v3 mutation {index} should be rejected by the loader"
        );
        assert!(
            !valid(&root, &root, &pack),
            "v3 mutation {index} should be rejected by the schema"
        );
    }
    // Every v3 condition and effect form is accepted by both.
    let mut pack = faro();
    pack["world"]["rules"][0]["when"] = json!({ "all": [
        { "carried": "medalla" },
        { "object_at": { "obj": "medalla", "at": { "in": "barca" } } },
        { "object_at": { "obj": "medalla", "at": { "room": "playa" } } },
        { "object_at": { "obj": "medalla", "at": "nowhere" } },
        { "object_is": { "obj": "puerta", "flag": "locked" } },
        { "visible": "barca" },
        { "player_in": "playa" },
        { "turn_cmp": { "op": "ge", "value": 3 } }
    ] });
    pack["world"]["rules"][0]["effects"] = json!([
        { "move_object": { "obj": "medalla", "to": "carried" } },
        { "set_object": { "obj": "puerta", "flag": "open", "value": true } }
    ]);
    assert!(loader_accepts(&pack));
    assert!(valid(&root, &root, &pack));
}

#[test]
fn v3_schema_bounds_match_core_constants() {
    use tiamud_story_core::{
        MAX_EXITS, MAX_EXTENSION_VERBS, MAX_FOUND_IN, MAX_LEXICONS, MAX_OBJECTS, MAX_RULES,
    };
    let root = schema();
    let defs = &root["$defs"];
    let props = |def: &str, field: &str| defs[def]["properties"][field].clone();
    assert_eq!(props("world", "lexicons")["maxItems"], MAX_LEXICONS);
    assert_eq!(props("world", "objects")["maxItems"], MAX_OBJECTS);
    assert_eq!(props("world", "rules")["maxItems"], MAX_RULES);
    assert_eq!(props("room", "exits")["maxItems"], MAX_EXITS);
    assert_eq!(props("object", "found_in")["maxItems"], MAX_FOUND_IN);
    assert_eq!(
        props("lexiconRef", "verbs")["maxItems"],
        MAX_EXTENSION_VERBS
    );
    assert_eq!(
        defs["localizedWords"]["additionalProperties"]["maxItems"],
        tiamud_story_parser::MAX_SCOPE_WORDS_PER_OBJECT
    );
    assert_eq!(
        root["properties"]["format_version"]["enum"],
        json!([1, 2, 3])
    );
}

fn duelo() -> Value {
    serde_json::from_slice(include_bytes!(
        "../../story-script/fixtures/duel-at-the-pass.story.json"
    ))
    .unwrap()
}

#[test]
fn scripted_packs_satisfy_the_schema_and_bad_script_shapes_do_not() {
    let root = schema();
    let original = duelo();
    assert!(valid(&root, &root, &original));
    let pack = StoryPack::from_json_slice(&serde_json::to_vec(&original).unwrap()).unwrap();
    let canonical: Value = serde_json::from_slice(&canonical_json(&pack).unwrap()).unwrap();
    assert!(valid(&root, &root, &canonical));
    // Older schemas know nothing of scripts.
    let v2: Value = serde_json::from_str(STORY_PACK_SCHEMA_V2).unwrap();
    assert!(!valid(&v2, &v2, &original));

    let defs = &root["$defs"];
    assert_eq!(root["properties"]["scripts"]["maxItems"], MAX_SCRIPTS);
    assert_eq!(
        defs["script"]["properties"]["writes"]["maxItems"],
        MAX_SCRIPT_GRANTS
    );
    assert_eq!(
        defs["scriptRef"]["properties"]["args"]["maxProperties"],
        MAX_SCRIPT_ARGS
    );

    type Mutation = Box<dyn Fn(&mut Value)>;
    let mutations: Vec<Mutation> = vec![
        Box::new(|pack| pack["scripts"][0]["kind"] = json!("macro")),
        Box::new(|pack| pack["scripts"][0]["lenguaje"] = json!("lua")),
        Box::new(|pack| pack["scripts"][0]["score"] = json!("si")),
        Box::new(|pack| pack["scripts"][0]["writes"] = json!("vigor")),
        Box::new(|pack| {
            pack["nodes"][1]["choices"][0]["effects"][0]["script"]["args"] = json!({ "arma": "x" })
        }),
        Box::new(|pack| pack["nodes"][1]["choices"][0]["effects"][0]["script"]["peso"] = json!(1)),
        Box::new(|pack| pack["nodes"][1]["choices"][2]["when"] = json!({ "script": "puede_huir" })),
    ];
    for mutate in mutations {
        let mut document = original.clone();
        mutate(&mut document);
        assert!(!valid(&root, &root, &document), "{document}");
        assert!(!loader_accepts(&document), "{document}");
    }
}

#[test]
fn prompt_and_compatible_revisions_match_the_loader() {
    use tiamud_story_core::{MAX_COMPATIBLE_REVISIONS, MAX_PROMPT_BYTES};
    let root = schema();
    assert_eq!(
        root["properties"]["compatible_revisions"]["maxItems"],
        MAX_COMPATIBLE_REVISIONS
    );
    assert_eq!(MAX_PROMPT_BYTES, 64);
    let mut pack = faro();
    pack["world"]["prompt"] = json!({ "es": "¿Qué haces?", "en": "What now?" });
    pack["compatible_revisions"] = json!(["0123456789abcdef".repeat(4)]);
    assert!(loader_accepts(&pack));
    assert!(
        StoryPack::from_json_slice(&serde_json::to_vec(&pack).unwrap())
            .unwrap()
            .validate()
            .is_ok()
    );
    assert!(valid(&root, &root, &pack));
    let canonical: Value = serde_json::from_slice(
        &canonical_json(&StoryPack::from_json_slice(&serde_json::to_vec(&pack).unwrap()).unwrap())
            .unwrap(),
    )
    .unwrap();
    assert!(valid(&root, &root, &canonical));
    let v2: Value = serde_json::from_str(STORY_PACK_SCHEMA_V2).unwrap();
    let mut older = fixture();
    older["compatible_revisions"] = json!(["0123456789abcdef".repeat(4)]);
    assert!(!valid(&v2, &v2, &older));

    for bad in [
        json!(["0123456789ABCDEF".repeat(4)]),
        json!(["abc"]),
        json!([7]),
        json!("0123456789abcdef"),
    ] {
        let mut pack = faro();
        pack["compatible_revisions"] = bad;
        assert!(!valid(&root, &root, &pack));
    }
    let mut pack = faro();
    pack["world"]["prompt"] = json!(">");
    assert!(!loader_accepts(&pack));
    assert!(!valid(&root, &root, &pack));
}
