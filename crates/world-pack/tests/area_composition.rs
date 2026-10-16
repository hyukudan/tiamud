use std::{fs, path::Path, process::Command};

use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use world_pack::{
    AreaPack, CompositionError, MAX_AREA_FILE_BYTES, MAX_ITEM_ALIAS_BYTES,
    MAX_PROJECT_MANIFEST_BYTES, WorldPackDocument, compose_world_project,
};

fn run(arguments: &[&str]) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_tiamud-world"))
        .args(arguments)
        .output()
        .unwrap()
}

fn write_json(path: &Path, value: &Value) {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).unwrap();
    }
    fs::write(path, serde_json::to_vec(value).unwrap()).unwrap();
}

fn room(id: &str, target: Option<&str>) -> Value {
    let exits = target.map_or_else(Vec::new, |target| {
        vec![json!({
            "direction": if id == "a:start" { "north" } else { "south" },
            "target": { "kind": "room", "room_id": target }
        })]
    });
    json!({
        "id": id,
        "title": id,
        "description": format!("Room {id}."),
        "exits": exits
    })
}

fn area(id: &str, depends_on: &[&str], rooms: Vec<Value>) -> Value {
    json!({
        "schemaVersion": 1,
        "id": id,
        "dependsOn": depends_on,
        "rooms": rooms
    })
}

fn project(areas: Vec<(&str, &str)>) -> Value {
    json!({
        "schemaVersion": 1,
        "id": "demo",
        "name": "Demo",
        "startRoomId": "a:start",
        "rngSeed": 7,
        "areas": areas.into_iter().map(|(id, path)| json!({"id": id, "path": path})).collect::<Vec<_>>()
    })
}

fn write_two_area_project(root: &Path, reverse: bool) -> std::path::PathBuf {
    write_json(
        &root.join("areas/a.json"),
        &area("a", &["b"], vec![room("a:start", Some("b:room"))]),
    );
    write_json(
        &root.join("areas/b.json"),
        &area("b", &["a"], vec![room("b:room", Some("a:start"))]),
    );
    let entries = if reverse {
        vec![("b", "areas/b.json"), ("a", "areas/a.json")]
    } else {
        vec![("a", "areas/a.json"), ("b", "areas/b.json")]
    };
    let manifest = root.join("world.project.json");
    write_json(&manifest, &project(entries));
    manifest
}

#[test]
fn composes_cross_area_cycles_and_is_order_independent() {
    let first = tempfile::tempdir().unwrap();
    let second = tempfile::tempdir().unwrap();
    let first_project = write_two_area_project(first.path(), false);
    let second_project = write_two_area_project(second.path(), true);

    let first = compose_world_project(first_project).unwrap();
    let second = compose_world_project(second_project).unwrap();
    assert_eq!(first.artifact, second.artifact);
    assert_eq!(first.report, second.report);
    assert_eq!(first.report.cross_area_references.len(), 2);
    assert_eq!(first.document.world.rooms.len(), 2);
}

#[test]
fn progression_references_require_declared_area_dependencies() {
    let temp = tempfile::tempdir().unwrap();
    let progression_area = json!({
        "schemaVersion": 2,
        "id": "a",
        "rooms": [room("a:start", None)],
        "levelThresholds": [{"level":2,"totalExperienceRequired":100}],
        "lineages": [{
            "id":"demo:mortal","name":"Mortal","description":"A mortal.","uiOrder":10
        }],
        "baseClasses": [{
            "id":"demo:warden","name":"Warden","description":"A warden.","uiOrder":10,
            "creationGrants":[
                {"kind":"attribute","attributeId":"max_health","amount":10},
                {"kind":"ability","abilityId":"demo:strike"}
            ]
        }]
    });
    let abilities_area = json!({
        "schemaVersion": 2,
        "id": "b",
        "abilities": [{
            "id":"demo:strike","name":"Strike","description":"A strike.",
            "category":"martial","targeting":"single_enemy",
            "damage":{"target":"target","damageType":"physical","baseAmount":1}
        }]
    });
    write_json(&temp.path().join("a.json"), &progression_area);
    write_json(&temp.path().join("b.json"), &abilities_area);
    let manifest = temp.path().join("world.project.json");
    write_json(
        &manifest,
        &json!({
            "schemaVersion":2,"id":"demo","name":"Demo","startRoomId":"a:start",
            "defaultLineageId":"demo:mortal","defaultBaseClassId":"demo:warden",
            "areas":[{"id":"a","path":"a.json"},{"id":"b","path":"b.json"}]
        }),
    );
    assert!(matches!(
        compose_world_project(&manifest),
        Err(CompositionError::UndeclaredDependency {
            kind: world_pack::AreaCrossReferenceKind::ProgressionReference,
            ..
        })
    ));

    let mut declared = progression_area;
    declared["dependsOn"] = json!(["b"]);
    write_json(&temp.path().join("a.json"), &declared);
    compose_world_project(&manifest).unwrap();
}

#[test]
fn item_media_cross_area_edges_target_the_reusable_prototype() {
    let temp = tempfile::tempdir().unwrap();
    let sprite = b"reviewed test sprite";
    let sprite_hash = hex::encode(Sha256::digest(sprite));
    let sprite_path = format!("assets/{sprite_hash}/sprite-idle-640.webp");
    let absolute_sprite_path = temp.path().join(&sprite_path);
    fs::create_dir_all(absolute_sprite_path.parent().unwrap()).unwrap();
    fs::write(&absolute_sprite_path, sprite).unwrap();
    let media_area = json!({
        "schemaVersion": 3,
        "id": "a",
        "rooms": [room("a:start", None)],
        "entityMedia": [{
            "kind": "item",
            "subjectId": "b:lantern",
            "decorative": true,
            "footprint": {"widthMilli": 100, "heightMilli": 100},
            "layer": "foreground",
            "poses": {"idle": {
                "webp": sprite_path
            }}
        }]
    });
    let prototype_area = json!({
        "schemaVersion": 3,
        "id": "b",
        "itemPrototypes": [{
            "id": "b:lantern",
            "name": "Lantern",
            "description": "A reusable lantern prototype.",
            "weightGrams": 0,
            "slotCost": 1,
            "rarity": "common",
            "itemLevel": 1,
            "stackPolicy": "unique",
            "transferPolicy": "transferable"
        }]
    });
    write_json(&temp.path().join("a.json"), &media_area);
    write_json(&temp.path().join("b.json"), &prototype_area);
    let manifest = temp.path().join("world.project.json");
    write_json(
        &manifest,
        &json!({
            "schemaVersion": 2,
            "id": "demo",
            "name": "Demo",
            "startRoomId": "a:start",
            "areas": [{"id":"a","path":"a.json"},{"id":"b","path":"b.json"}]
        }),
    );

    assert!(matches!(
        compose_world_project(&manifest),
        Err(CompositionError::UndeclaredDependency {
            kind: world_pack::AreaCrossReferenceKind::EntityMedia,
            ..
        })
    ));

    let mut declared = media_area;
    declared["dependsOn"] = json!(["b"]);
    write_json(&temp.path().join("a.json"), &declared);
    let composed = compose_world_project(&manifest).unwrap();
    assert!(
        composed
            .report
            .cross_area_references
            .iter()
            .any(|reference| {
                reference.kind == world_pack::AreaCrossReferenceKind::EntityMedia
                    && reference.target_id == "b:lantern"
            })
    );
}

#[test]
fn real_three_area_project_has_no_semantic_or_canonical_drift() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let composed = compose_world_project(root.join("worlds/lantern-square.project.json")).unwrap();
    let existing = fs::read(root.join("worlds/lantern-square.world.json")).unwrap();
    let canonical_existing = WorldPackDocument::from_json(&existing)
        .unwrap()
        .deterministic_artifact()
        .unwrap();

    assert_eq!(composed.artifact, canonical_existing);
    assert_eq!(composed.report.areas.len(), 3);
    assert_eq!(composed.report.cross_area_references.len(), 7);
}

#[test]
fn communication_blocked_rooms_require_declared_area_dependencies() {
    let temp = tempfile::tempdir().unwrap();
    let communication_area = json!({
        "schemaVersion": 8,
        "id": "a",
        "rooms": [room("a:start", None)],
        "abilities": [{
            "id": "a:whisper",
            "name": "Whisper",
            "description": "A bounded authored whisper.",
            "category": "utility",
            "resourceCosts": {"focus": 1},
            "cooldownTicks": 1,
            "targeting": "single_ally",
            "communication": {
                "kind": "telepathy",
                "scope": "direct_online",
                "blockedRoomIds": ["b:silence"]
            }
        }]
    });
    let blocked_area = json!({
        "schemaVersion": 8,
        "id": "b",
        "rooms": [room("b:silence", None)]
    });
    write_json(&temp.path().join("a.json"), &communication_area);
    write_json(&temp.path().join("b.json"), &blocked_area);
    let manifest = temp.path().join("world.project.json");
    write_json(&manifest, &project(vec![("a", "a.json"), ("b", "b.json")]));

    assert!(matches!(
        compose_world_project(&manifest),
        Err(CompositionError::UndeclaredDependency {
            kind: world_pack::AreaCrossReferenceKind::AbilityBlockedRoom,
            ..
        })
    ));

    let mut declared = communication_area;
    declared["dependsOn"] = json!(["b"]);
    write_json(&temp.path().join("a.json"), &declared);
    let composed = compose_world_project(&manifest).unwrap();
    assert!(
        composed
            .report
            .cross_area_references
            .iter()
            .any(|reference| {
                reference.kind == world_pack::AreaCrossReferenceKind::AbilityBlockedRoom
                    && reference.source_id == "a:whisper"
                    && reference.target_id == "b:silence"
            })
    );
}

#[test]
fn native_area_item_text_bounds_are_enforced_by_composition() {
    let temp = tempfile::tempdir().unwrap();
    let area = json!({
        "schemaVersion": 4,
        "id": "a",
        "itemPrototypes": [{
            "id": "a:lantern",
            "name": "Lantern",
            "description": "A bounded lantern.",
            "aliases": ["a".repeat(MAX_ITEM_ALIAS_BYTES + 1)],
            "weightGrams": 100,
            "slotCost": 1,
            "rarity": "common",
            "itemLevel": 1,
            "stackPolicy": "unique",
            "maxStackUnits": 1,
            "transferPolicy": "transferable"
        }],
        "rooms": [room("a:start", None)]
    });
    write_json(&temp.path().join("a.json"), &area);
    let manifest = temp.path().join("world.project.json");
    write_json(
        &manifest,
        &json!({
            "schemaVersion": 2,
            "id": "demo",
            "name": "Demo",
            "startRoomId": "a:start",
            "areas": [{"id": "a", "path": "a.json"}]
        }),
    );

    let error = compose_world_project(&manifest).unwrap_err();
    let CompositionError::InvalidWorld(errors) = error else {
        panic!("unexpected composition error: {error}");
    };
    assert!(
        errors
            .0
            .iter()
            .any(|error| error.to_string().contains("alias"))
    );

    let mut legacy_area = area;
    legacy_area["schemaVersion"] = json!(3);
    legacy_area["itemPrototypes"][0]
        .as_object_mut()
        .unwrap()
        .remove("maxStackUnits");
    write_json(&temp.path().join("a.json"), &legacy_area);
    let error = compose_world_project(&manifest).unwrap_err();
    let CompositionError::InvalidWorld(errors) = error else {
        panic!("unexpected normalized AreaPack error: {error}");
    };
    assert!(
        errors
            .0
            .iter()
            .any(|error| error.to_string().contains("alias"))
    );
}

#[test]
fn normalized_area_item_ids_match_the_runtime_identifier_contract() {
    let temp = tempfile::tempdir().unwrap();
    let area = json!({
        "schemaVersion": 3,
        "id": "a",
        "itemPrototypes": [{
            "id": "A:Lantern",
            "name": "Lantern",
            "description": "A legacy modular lantern.",
            "weightGrams": 100,
            "slotCost": 1,
            "rarity": "common",
            "itemLevel": 1,
            "stackPolicy": "unique",
            "transferPolicy": "transferable"
        }],
        "rooms": [room("a:start", None)]
    });
    write_json(&temp.path().join("a.json"), &area);
    let manifest = temp.path().join("world.project.json");
    write_json(
        &manifest,
        &json!({
            "schemaVersion": 2,
            "id": "demo",
            "name": "Demo",
            "startRoomId": "a:start",
            "areas": [{"id": "a", "path": "a.json"}]
        }),
    );

    let error = compose_world_project(&manifest).unwrap_err();
    let CompositionError::InvalidWorld(errors) = error else {
        panic!("unexpected normalized AreaPack error: {error}");
    };
    assert!(errors.0.iter().any(|error| {
        error
            .to_string()
            .contains("lowercase namespaced identifier")
    }));
}

#[test]
fn native_area_instance_ids_share_the_runtime_identifier_contract() {
    let compose_with_id = |instance_id: &str| {
        let temp = tempfile::tempdir().unwrap();
        write_json(
            &temp.path().join("a.json"),
            &json!({
                "schemaVersion": 4,
                "id": "a",
                "itemPrototypes": [{
                    "id": "a:lantern",
                    "name": "Lantern",
                    "description": "A native modular lantern.",
                    "weightGrams": 100,
                    "slotCost": 1,
                    "rarity": "common",
                    "itemLevel": 1,
                    "stackPolicy": "unique",
                    "maxStackUnits": 1,
                    "transferPolicy": "transferable"
                }],
                "initialItemInstances": [{
                    "id": instance_id,
                    "prototypeId": "a:lantern",
                    "location": {"kind": "staged"}
                }],
                "rooms": [room("a:start", None)]
            }),
        );
        let manifest = temp.path().join("world.project.json");
        write_json(
            &manifest,
            &json!({
                "schemaVersion": 2,
                "id": "demo",
                "name": "Demo",
                "startRoomId": "a:start",
                "areas": [{"id": "a", "path": "a.json"}]
            }),
        );
        compose_world_project(manifest)
    };

    for valid_id in ["runtime:1".to_owned(), format!("i{}", "a".repeat(127))] {
        compose_with_id(&valid_id).unwrap();
    }

    for invalid_id in [
        "runtime:".to_owned(),
        "runtime:0".to_owned(),
        "runtime:01".to_owned(),
        "runtime:-1".to_owned(),
        "runtime:+1".to_owned(),
        "runtime:18446744073709551616".to_owned(),
        format!("i{}", "a".repeat(128)),
    ] {
        let error = compose_with_id(&invalid_id).unwrap_err();
        let CompositionError::InvalidWorld(errors) = error else {
            panic!("unexpected AreaPack error for {invalid_id:?}: {error}");
        };
        assert!(
            errors
                .0
                .iter()
                .any(|error| { error.to_string().contains("canonical runtime id") })
        );
    }
}

#[test]
fn duplicate_area_ids_and_paths_fail_closed() {
    let duplicate_id = tempfile::tempdir().unwrap();
    let manifest = duplicate_id.path().join("world.project.json");
    write_json(&manifest, &project(vec![("a", "a.json"), ("a", "b.json")]));
    assert!(matches!(
        compose_world_project(&manifest),
        Err(CompositionError::DuplicateAreaId(id)) if id == "a"
    ));

    let duplicate_path = tempfile::tempdir().unwrap();
    let manifest = duplicate_path.path().join("world.project.json");
    write_json(
        &manifest,
        &project(vec![("a", "same.json"), ("b", "same.json")]),
    );
    assert!(matches!(
        compose_world_project(&manifest),
        Err(CompositionError::DuplicateAreaPath(_))
    ));
}

#[test]
fn duplicate_global_entity_ids_fail_even_across_entity_kinds() {
    let temp = tempfile::tempdir().unwrap();
    write_json(
        &temp.path().join("a.json"),
        &json!({
            "schemaVersion": 3,
            "id": "a",
            "itemPrototypes": [{
                "id":"shared:id","name":"Item","description":"Item.",
                "weightGrams":0,"slotCost":1,"rarity":"common","itemLevel":1,
                "stackPolicy":"unique","transferPolicy":"transferable"
            }],
            "initialItemInstances": [{
                "id":"shared:id","prototypeId":"shared:id","location":{"kind":"staged"}
            }],
            "rooms": [room("a:start", None)]
        }),
    );
    write_json(
        &temp.path().join("b.json"),
        &json!({
            "schemaVersion": 1,
            "id": "b",
            "scripts": [{
                "id":"shared:id", "roomId":"a:start", "hooks":["on_look"],
                "capabilities":[], "source":"function on_look(ctx) end"
            }]
        }),
    );
    let manifest = temp.path().join("world.project.json");
    write_json(&manifest, &project(vec![("a", "a.json"), ("b", "b.json")]));
    assert!(matches!(
        compose_world_project(manifest),
        Err(CompositionError::DuplicateGlobalEntityId { id, .. }) if id == "shared:id"
    ));
}

#[test]
fn prototype_and_instance_share_a_typed_id_across_areas() {
    let temp = tempfile::tempdir().unwrap();
    write_json(
        &temp.path().join("a.json"),
        &json!({
            "schemaVersion": 3,
            "id": "a",
            "itemPrototypes": [{
                "id":"shared:lantern","name":"Lantern","description":"A lantern.",
                "weightGrams":0,"slotCost":1,"rarity":"common","itemLevel":1,
                "stackPolicy":"unique","transferPolicy":"transferable"
            }],
            "rooms": [room("a:start", None)]
        }),
    );
    write_json(
        &temp.path().join("b.json"),
        &json!({
            "schemaVersion": 3,
            "id": "b",
            "dependsOn": ["a"],
            "initialItemInstances": [{
                "id":"shared:lantern",
                "prototypeId":"shared:lantern",
                "location":{"kind":"staged"}
            }]
        }),
    );
    let manifest = temp.path().join("world.project.json");
    write_json(
        &manifest,
        &json!({
            "schemaVersion":2,"id":"demo","name":"Demo","startRoomId":"a:start",
            "areas":[{"id":"a","path":"a.json"},{"id":"b","path":"b.json"}]
        }),
    );

    let composed = compose_world_project(manifest).unwrap();
    assert_eq!(
        composed.document.world.item_prototypes[0].id,
        "shared:lantern"
    );
    assert_eq!(
        composed.document.world.initial_item_instances[0].id,
        "shared:lantern"
    );
    assert!(
        composed
            .report
            .cross_area_references
            .iter()
            .any(|reference| {
                reference.kind == world_pack::AreaCrossReferenceKind::ItemInstancePrototype
                    && reference.from_area == "b"
                    && reference.to_area == "a"
            })
    );
}

#[test]
fn unsafe_missing_and_symlink_escaping_area_paths_are_rejected() {
    for unsafe_path in ["../outside.json", "/tmp/outside.json", "C:/outside.json"] {
        let temp = tempfile::tempdir().unwrap();
        let manifest = temp.path().join("world.project.json");
        write_json(&manifest, &project(vec![("a", unsafe_path)]));
        assert!(matches!(
            compose_world_project(manifest),
            Err(CompositionError::UnsafeAreaPath { .. })
        ));
    }

    let missing = tempfile::tempdir().unwrap();
    let manifest = missing.path().join("world.project.json");
    write_json(&manifest, &project(vec![("a", "missing.json")]));
    assert!(matches!(
        compose_world_project(manifest),
        Err(CompositionError::Inspect { .. })
    ));

    #[cfg(unix)]
    {
        use std::os::unix::fs::symlink;
        let project_root = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        write_json(
            &outside.path().join("a.json"),
            &area("a", &[], vec![room("a:start", None)]),
        );
        symlink(
            outside.path().join("a.json"),
            project_root.path().join("a.json"),
        )
        .unwrap();
        let manifest = project_root.path().join("world.project.json");
        write_json(&manifest, &project(vec![("a", "a.json")]));
        assert!(matches!(
            compose_world_project(manifest),
            Err(CompositionError::AreaPathEscape { .. })
        ));
    }
}

#[test]
fn schema_identity_dependencies_and_unknown_fields_are_strict() {
    let cases = [
        json!({"schemaVersion":2,"id":"demo","name":"Demo","startRoomId":"a:start","areas":[]}),
        json!({"schemaVersion":1,"id":"demo","name":"Demo","startRoomId":"a:start","areas":[],"unknown":true}),
    ];
    for value in cases {
        let temp = tempfile::tempdir().unwrap();
        let manifest = temp.path().join("world.project.json");
        write_json(&manifest, &value);
        assert!(compose_world_project(manifest).is_err());
    }

    for bad_area in [
        json!({"schemaVersion":10,"id":"a","rooms":[room("a:start", None)]}),
        json!({"schemaVersion":1,"id":"a","rooms":[room("a:start", None)],"unknown":true}),
        json!({"schemaVersion":1,"id":"wrong","rooms":[room("a:start", None)]}),
        json!({"schemaVersion":1,"id":"a","dependsOn":["missing"],"rooms":[room("a:start", None)]}),
        json!({"schemaVersion":1,"id":"a","dependsOn":["a"],"rooms":[room("a:start", None)]}),
    ] {
        let temp = tempfile::tempdir().unwrap();
        write_json(&temp.path().join("a.json"), &bad_area);
        let manifest = temp.path().join("world.project.json");
        write_json(&manifest, &project(vec![("a", "a.json")]));
        assert!(compose_world_project(manifest).is_err());
    }
}

#[test]
fn area_v3_normalizes_to_current_but_rejects_v4_schema_smuggling() {
    let legacy = json!({
        "schemaVersion": 3,
        "id": "a",
        "itemPrototypes": [{
            "id": "a:item",
            "name": "Item",
            "description": "A legacy unique item.",
            "weightGrams": 0,
            "slotCost": 1,
            "rarity": "common",
            "itemLevel": 1,
            "stackPolicy": "unique",
            "transferPolicy": "transferable"
        }],
        "initialItemInstances": [{
            "id": "a:item-1",
            "prototypeId": "a:item",
            "location": {"kind": "staged"}
        }]
    });
    let normalized: AreaPack = serde_json::from_value(legacy.clone()).unwrap();
    assert_eq!(
        normalized.schema_version,
        world_pack::AREA_PACK_SCHEMA_VERSION
    );
    assert_eq!(normalized.item_prototypes[0].max_stack_units, 1);
    assert!(normalized.item_prototypes[0].container.is_none());

    let mut smuggled = legacy.clone();
    smuggled["itemPrototypes"][0]["maxStackUnits"] = json!(20);
    assert!(serde_json::from_value::<AreaPack>(smuggled).is_err());

    for transfer_policy in ["character_bound", "non_transferable"] {
        let mut smuggled = legacy.clone();
        smuggled["itemPrototypes"][0]["transferPolicy"] = json!(transfer_policy);
        assert!(serde_json::from_value::<AreaPack>(smuggled).is_err());
    }

    let mut smuggled = legacy;
    smuggled["initialItemInstances"][0]["location"] = json!({
        "kind": "container",
        "containerInstanceId": "a:item-1"
    });
    assert!(serde_json::from_value::<AreaPack>(smuggled).is_err());
}

#[test]
fn area_v1_and_v2_only_normalize_transferable_items() {
    for schema_version in [1, 2] {
        let legacy = json!({
            "schemaVersion": schema_version,
            "id": "a",
            "items": [{
                "id": "a:item",
                "name": "Item",
                "description": "A legacy item."
            }],
            "rooms": [{
                "id": "a:start",
                "title": "Start",
                "description": "Start.",
                "items": ["a:item"]
            }]
        });
        let normalized: AreaPack = serde_json::from_value(legacy.clone()).unwrap();
        assert_eq!(
            normalized.item_prototypes[0].transfer_policy,
            world_pack::ItemTransferPolicy::Transferable
        );

        let mut smuggled = legacy;
        smuggled["items"][0]["transferPolicy"] = json!("character_bound");
        assert!(serde_json::from_value::<AreaPack>(smuggled).is_err());
    }
}

#[test]
fn area_v4_normalizes_to_v7_and_accepts_all_transfer_policies() {
    for transfer_policy in ["transferable", "character_bound", "non_transferable"] {
        let area: AreaPack = serde_json::from_value(json!({
            "schemaVersion": 4,
            "id": "a",
            "itemPrototypes": [{
                "id": "a:item",
                "name": "Item",
                "description": "A native item.",
                "weightGrams": 0,
                "slotCost": 1,
                "rarity": "common",
                "itemLevel": 1,
                "stackPolicy": "unique",
                "maxStackUnits": 1,
                "transferPolicy": transfer_policy
            }],
            "initialItemInstances": [{
                "id": "a:item-1",
                "prototypeId": "a:item",
                "location": {"kind": "staged"}
            }]
        }))
        .unwrap();

        assert_eq!(area.schema_version, world_pack::AREA_PACK_SCHEMA_VERSION);
        assert_eq!(area.item_prototypes.len(), 1);
    }
}

#[test]
fn area_v1_through_v4_reject_repeatable_loot_schema_smuggling() {
    for schema_version in [1, 2, 3, 4] {
        for field in ["lootTables", "lootSources"] {
            let mut value = json!({
                "schemaVersion": schema_version,
                "id": "a",
                "rooms": [room("a:start", None)]
            });
            value[field] = json!([]);
            let error = serde_json::from_value::<AreaPack>(value).unwrap_err();
            assert!(error.to_string().contains("cannot declare"), "{error}");
        }
    }

    let normalized: AreaPack = serde_json::from_value(json!({
        "schemaVersion": 4,
        "id": "a",
        "rooms": [room("a:start", None)]
    }))
    .unwrap();
    assert_eq!(
        normalized.schema_version,
        world_pack::AREA_PACK_SCHEMA_VERSION
    );
    assert!(normalized.loot_tables.is_empty());
    assert!(normalized.loot_sources.is_empty());
}

#[test]
fn area_v1_through_v5_normalize_without_inventing_rewards_and_reject_smuggling() {
    for schema_version in [1, 2, 3, 4, 5] {
        let legacy = json!({
            "schemaVersion": schema_version,
            "id": "a",
            "rooms": [room("a:start", None)]
        });
        let normalized: AreaPack = serde_json::from_value(legacy.clone()).unwrap();
        assert_eq!(
            normalized.schema_version,
            world_pack::AREA_PACK_SCHEMA_VERSION
        );
        assert!(normalized.affixes.is_empty());
        assert!(normalized.affix_pools.is_empty());

        for field in ["affixes", "affixPools"] {
            let mut smuggled = legacy.clone();
            smuggled[field] = json!([]);
            let error = serde_json::from_value::<AreaPack>(smuggled).unwrap_err();
            assert!(error.to_string().contains("cannot declare v6"), "{error}");
        }

        let mut sealed = legacy.clone();
        let field = if schema_version <= 2 {
            "items"
        } else {
            "itemPrototypes"
        };
        sealed[field] = json!([{"sealedLootTableId":"a:rewards"}]);
        let error = serde_json::from_value::<AreaPack>(sealed).unwrap_err();
        assert!(
            error.to_string().contains("cannot declare v6")
                || error
                    .to_string()
                    .contains("unknown field \"sealedLootTableId\""),
            "{error}"
        );
    }

    let affix_reference = json!({
        "schemaVersion": 5,
        "id": "a",
        "rooms": [room("a:start", None)],
        "lootTables": [{
            "id":"a:rewards",
            "guaranteed":[{
                "id":"a:reward","itemPrototypeId":"a:item","quantity":1,
                "affixPoolId":"a:affixes"
            }]
        }]
    });
    let error = serde_json::from_value::<AreaPack>(affix_reference).unwrap_err();
    assert!(error.to_string().contains("cannot declare v6"), "{error}");
}

#[test]
fn area_v1_through_v6_normalize_without_inventing_communication_and_reject_smuggling() {
    for schema_version in 1..=6 {
        let legacy = json!({
            "schemaVersion": schema_version,
            "id": "a",
            "rooms": [room("a:start", None)],
            "abilities": [{
                "id": "a:strike",
                "name": "Strike",
                "description": "An ordinary legacy ability.",
                "category": "martial",
                "targeting": "single_enemy",
                "damage": {
                    "target": "target",
                    "damageType": "physical",
                    "baseAmount": 1
                }
            }]
        });
        let normalized: AreaPack = serde_json::from_value(legacy.clone()).unwrap();
        assert_eq!(
            normalized.schema_version,
            world_pack::AREA_PACK_SCHEMA_VERSION
        );
        assert_eq!(normalized.abilities.len(), 1);
        assert!(normalized.abilities[0].communication.is_none());

        let mut smuggled = legacy;
        smuggled["abilities"] = json!([{
            "id": "a:whisper",
            "name": "Whisper",
            "description": "A later communication effect.",
            "category": "utility",
            "resourceCosts": {"focus": 1},
            "cooldownTicks": 1,
            "targeting": "single_ally",
            "communication": {
                "kind": "telepathy",
                "scope": "direct_online",
                "blockedRoomIds": []
            }
        }]);
        let error = serde_json::from_value::<AreaPack>(smuggled).unwrap_err();
        assert!(error.to_string().contains("cannot declare v7"), "{error}");
    }
}

#[test]
fn area_v8_rejects_unknown_nested_communication_fields() {
    let value = json!({
        "schemaVersion": 8,
        "id": "a",
        "abilities": [{
            "id": "a:whisper",
            "name": "Whisper",
            "description": "A bounded authored whisper.",
            "category": "utility",
            "resourceCosts": {"focus": 1},
            "cooldownTicks": 1,
            "targeting": "single_ally",
            "communication": {
                "kind": "telepathy",
                "scope": "direct_online",
                "blockedRoomId": []
            }
        }]
    });
    let error = serde_json::from_value::<AreaPack>(value).unwrap_err();
    assert!(error.to_string().contains("blockedRoomId"), "{error}");
    assert!(
        error.to_string().contains("$.abilities[0].communication"),
        "{error}"
    );
}

#[test]
fn loot_ownership_and_cross_area_dependencies_are_complete_and_deterministic() {
    let temp = tempfile::tempdir().unwrap();
    let source_area = json!({
        "schemaVersion": 5,
        "id": "a",
        "dependsOn": ["b"],
        "rooms": [room("a:start", None)],
        "lootSources": [{
            "kind": "npc_spawn",
            "id": "a:source",
            "npcSpawnId": "b:wolf-1",
            "lootTableId": "b:drops",
            "rights": "party_round_robin"
        }]
    });
    let table_area = json!({
        "schemaVersion": 5,
        "id": "b",
        "dependsOn": ["c"],
        "rooms": [room("b:room", None)],
        "npcPrototypes": [{
            "id":"b:wolf","name":"Wolf","description":"A wolf.","level":1,
            "maxHealth":10,"strength":2,"defence":0,"experienceReward":10,"respawnTicks":2
        }],
        "npcSpawns": [{"id":"b:wolf-1","prototypeId":"b:wolf","roomId":"b:room"}],
        "lootTables": [{
            "id": "b:drops",
            "guaranteed": [{"id":"b:fixed","itemPrototypeId":"c:token","quantity":1}],
            "pools": [{
                "id":"b:pool","rolls":1,"entries":[
                    {"kind":"item","id":"b:item","weight":1,"itemPrototypeId":"c:token","minQuantity":1,"maxQuantity":1}
                ]
            }]
        }]
    });
    let item_area = json!({
        "schemaVersion": 5,
        "id": "c",
        "itemPrototypes": [{
            "id":"c:token","name":"Token","description":"A token.","weightGrams":1,
            "slotCost":1,"rarity":"common","itemLevel":1,"stackPolicy":"unique",
            "maxStackUnits":1,"transferPolicy":"transferable"
        }]
    });
    write_json(&temp.path().join("a.json"), &source_area);
    write_json(&temp.path().join("b.json"), &table_area);
    write_json(&temp.path().join("c.json"), &item_area);
    let manifest = temp.path().join("world.project.json");
    write_json(
        &manifest,
        &project(vec![("a", "a.json"), ("b", "b.json"), ("c", "c.json")]),
    );

    let composed = compose_world_project(&manifest).unwrap();
    assert_eq!(composed.document.world.gameplay.loot_tables.len(), 1);
    assert_eq!(composed.document.world.gameplay.loot_sources.len(), 1);
    let a = composed
        .report
        .areas
        .iter()
        .find(|area| area.id == "a")
        .unwrap();
    let b = composed
        .report
        .areas
        .iter()
        .find(|area| area.id == "b")
        .unwrap();
    assert_eq!(a.ownership.loot_sources, ["a:source"]);
    assert_eq!(b.ownership.loot_tables, ["b:drops"]);
    for kind in [
        world_pack::AreaCrossReferenceKind::LootGuaranteedPrototype,
        world_pack::AreaCrossReferenceKind::LootEntryPrototype,
        world_pack::AreaCrossReferenceKind::LootSourceNpcSpawn,
        world_pack::AreaCrossReferenceKind::LootSourceTable,
    ] {
        assert!(
            composed
                .report
                .cross_area_references
                .iter()
                .any(|reference| reference.kind == kind),
            "missing {kind:?}"
        );
    }

    let reparsed = WorldPackDocument::from_json(&composed.artifact).unwrap();
    assert_eq!(
        reparsed.deterministic_artifact().unwrap(),
        composed.artifact
    );
}

#[test]
fn sealed_reward_and_affix_edges_are_owned_and_audited_across_areas() {
    let temp = tempfile::tempdir().unwrap();
    let encounter = json!({
        "schemaVersion": 6,
        "id": "a",
        "dependsOn": ["b"],
        "rooms": [room("a:start", None)],
        "itemPrototypes": [{
            "id":"a:seal","name":"Sealed oath","description":"An earned sealed oath.",
            "consumable":true,"weightGrams":1,"slotCost":1,"rarity":"rare","itemLevel":1,
            "stackPolicy":"unique","maxStackUnits":1,"transferPolicy":"character_bound",
            "sealedLootTableId":"b:rewards"
        }],
        "npcPrototypes": [{
            "id":"a:wolf","name":"Wolf","description":"A wolf.","level":1,
            "maxHealth":10,"strength":2,"defence":0,"experienceReward":10,"respawnTicks":2
        }],
        "npcSpawns": [{"id":"a:wolf-1","prototypeId":"a:wolf","roomId":"a:start"}],
        "lootTables": [{
            "id":"a:drops",
            "guaranteed":[{"id":"a:seal-drop","itemPrototypeId":"a:seal","quantity":1}]
        }],
        "lootSources": [{
            "kind":"npc_spawn","id":"a:source","npcSpawnId":"a:wolf-1",
            "lootTableId":"a:drops","rights":"credited_character"
        }],
        "affixPools": [{
            "id":"a:weapon-affixes","entries":[{
                "kind":"affix","id":"a:charged-roll","weight":1,"affixId":"b:charged"
            }]
        }]
    });
    let rewards = json!({
        "schemaVersion": 6,
        "id": "b",
        "dependsOn": ["a"],
        "affixes": [{
            "id":"b:charged","label":"charged","position":"prefix","attackBonusDelta":1
        }],
        "itemPrototypes": [{
            "id":"b:blade","name":"Storm blade","description":"A storm-marked blade.",
            "equipmentSlot":"weapon","weightGrams":100,"slotCost":1,"rarity":"rare","itemLevel":1,
            "stackPolicy":"unique","maxStackUnits":1,"transferPolicy":"transferable"
        }],
        "lootTables": [{
            "id":"b:rewards",
            "guaranteed":[{
                "id":"b:blade-reward","itemPrototypeId":"b:blade","quantity":1,
                "affixPoolId":"a:weapon-affixes"
            }]
        }]
    });
    write_json(&temp.path().join("a.json"), &encounter);
    write_json(&temp.path().join("b.json"), &rewards);
    let manifest = temp.path().join("world.project.json");
    write_json(&manifest, &project(vec![("a", "a.json"), ("b", "b.json")]));

    let composed = compose_world_project(manifest).unwrap();
    let owner = composed
        .report
        .areas
        .iter()
        .find(|area| area.id == "a")
        .unwrap();
    assert!(owner.ownership.affixes.is_empty());
    assert_eq!(owner.ownership.affix_pools, ["a:weapon-affixes"]);
    let reward_owner = composed
        .report
        .areas
        .iter()
        .find(|area| area.id == "b")
        .unwrap();
    assert_eq!(reward_owner.ownership.affixes, ["b:charged"]);
    for kind in [
        world_pack::AreaCrossReferenceKind::ItemSealedLootTable,
        world_pack::AreaCrossReferenceKind::LootGuaranteedAffixPool,
        world_pack::AreaCrossReferenceKind::AffixPoolAffix,
    ] {
        assert!(
            composed
                .report
                .cross_area_references
                .iter()
                .any(|reference| reference.kind == kind),
            "missing {kind:?}"
        );
    }
}

#[test]
fn contained_instances_require_a_declared_cross_area_dependency() {
    let temp = tempfile::tempdir().unwrap();
    let manifest = temp.path().join("world.project.json");
    write_json(&manifest, &project(vec![("a", "a.json"), ("b", "b.json")]));
    let child_area = json!({
        "schemaVersion": 4,
        "id": "a",
        "itemPrototypes": [{
            "id": "a:map",
            "name": "Map",
            "description": "A folded map.",
            "weightGrams": 100,
            "slotCost": 1,
            "rarity": "common",
            "itemLevel": 1,
            "stackPolicy": "unique",
            "maxStackUnits": 1,
            "transferPolicy": "transferable"
        }],
        "initialItemInstances": [{
            "id": "a:map-1",
            "prototypeId": "a:map",
            "location": {"kind": "container", "containerInstanceId": "b:satchel-1"}
        }],
        "rooms": [room("a:start", None)]
    });
    let container_area = json!({
        "schemaVersion": 4,
        "id": "b",
        "itemPrototypes": [{
            "id": "b:satchel",
            "name": "Satchel",
            "description": "A sturdy satchel.",
            "weightGrams": 200,
            "slotCost": 1,
            "rarity": "common",
            "itemLevel": 1,
            "stackPolicy": "unique",
            "maxStackUnits": 1,
            "container": {"maxSlots": 4, "maxWeightGrams": 1000},
            "transferPolicy": "transferable"
        }],
        "initialItemInstances": [{
            "id": "b:satchel-1",
            "prototypeId": "b:satchel",
            "location": {"kind": "staged"}
        }]
    });
    write_json(&temp.path().join("a.json"), &child_area);
    write_json(&temp.path().join("b.json"), &container_area);
    assert!(matches!(
        compose_world_project(&manifest),
        Err(CompositionError::UndeclaredDependency {
            kind: world_pack::AreaCrossReferenceKind::ItemInstanceContainer,
            ..
        })
    ));

    let mut declared = child_area;
    declared["dependsOn"] = json!(["b"]);
    write_json(&temp.path().join("a.json"), &declared);
    let composed = compose_world_project(&manifest).unwrap();
    assert!(
        composed
            .report
            .cross_area_references
            .iter()
            .any(|reference| {
                reference.kind == world_pack::AreaCrossReferenceKind::ItemInstanceContainer
                    && reference.source_id == "a:map-1"
                    && reference.target_id == "b:satchel-1"
            })
    );
}

#[test]
fn nested_area_typo_is_rejected_without_changing_runtime_reader_compatibility() {
    let temp = tempfile::tempdir().unwrap();
    write_json(
        &temp.path().join("a.json"),
        &json!({
            "schemaVersion":1,
            "id":"a",
            "rooms":[{
                "id":"a:start",
                "titel":"Misspelled title",
                "description":"Start."
            }]
        }),
    );
    let manifest = temp.path().join("world.project.json");
    write_json(&manifest, &project(vec![("a", "a.json")]));
    assert!(matches!(
        compose_world_project(manifest),
        Err(CompositionError::UnknownAreaField { json_path, field })
            if json_path == "$.rooms[0]" && field == "titel"
    ));
    assert!(
        serde_json::from_value::<AreaPack>(json!({
            "schemaVersion":1,
            "id":"a",
            "rooms":[{
                "id":"a:start",
                "titel":"Misspelled title",
                "description":"Start."
            }]
        }))
        .is_err()
    );

    // Runtime WorldPack v2 intentionally keeps accepting future nested fields.
    let runtime = br#"{
      "schemaVersion":2,
      "id":"demo",
      "name":"Demo",
      "startRoomId":"a:start",
      "rooms":[{"id":"a:start","title":"Start","description":"Start.","futureField":true}]
    }"#;
    let document = WorldPackDocument::from_json(runtime).unwrap();
    document.validate().unwrap();
}

#[test]
fn missing_references_and_undeclared_edges_fail_but_unreachable_rooms_are_diagnostic() {
    let missing = tempfile::tempdir().unwrap();
    write_json(
        &missing.path().join("a.json"),
        &area("a", &[], vec![room("a:start", Some("missing:room"))]),
    );
    let manifest = missing.path().join("world.project.json");
    write_json(&manifest, &project(vec![("a", "a.json")]));
    assert!(matches!(
        compose_world_project(manifest),
        Err(CompositionError::InvalidWorld(_))
    ));

    let unreachable = tempfile::tempdir().unwrap();
    write_json(
        &unreachable.path().join("a.json"),
        &area(
            "a",
            &[],
            vec![room("a:start", None), room("a:orphan", None)],
        ),
    );
    let manifest = unreachable.path().join("world.project.json");
    write_json(&manifest, &project(vec![("a", "a.json")]));
    let composed = compose_world_project(manifest).unwrap();
    assert_eq!(composed.document.world.diagnostics().len(), 1);

    let undeclared = tempfile::tempdir().unwrap();
    write_json(
        &undeclared.path().join("a.json"),
        &area("a", &[], vec![room("a:start", Some("b:room"))]),
    );
    write_json(
        &undeclared.path().join("b.json"),
        &area("b", &["a"], vec![room("b:room", Some("a:start"))]),
    );
    let manifest = undeclared.path().join("world.project.json");
    write_json(&manifest, &project(vec![("a", "a.json"), ("b", "b.json")]));
    assert!(matches!(
        compose_world_project(manifest),
        Err(CompositionError::UndeclaredDependency { area_id, target_area, .. })
            if area_id == "a" && target_area == "b"
    ));
}

#[test]
fn assets_are_resolved_from_the_project_root_not_the_area_directory() {
    let temp = tempfile::tempdir().unwrap();
    let digest = "a".repeat(64);
    let relative = format!("assets/{digest}/manifest.json");
    write_json(
        &temp.path().join(&relative),
        &json!({"alt":"Reviewed project-root image."}),
    );
    let mut start = room("a:start", None);
    start["assetManifest"] = Value::String(relative);
    write_json(
        &temp.path().join("areas/a.json"),
        &json!({
            "schemaVersion":1,
            "id":"a",
            "rooms":[start],
            "roomMedia":[{
                "roomId":"a:start",
                "imageAlt":{"en":"Reviewed project-root image."}
            }]
        }),
    );
    let manifest = temp.path().join("world.project.json");
    write_json(&manifest, &project(vec![("a", "areas/a.json")]));
    compose_world_project(manifest).unwrap();
}

#[test]
fn web_assets_resolve_from_a_source_archive_without_git_metadata() {
    let temp = tempfile::tempdir().unwrap();
    fs::write(temp.path().join("Cargo.toml"), "[workspace]\n").unwrap();
    let digest = "b".repeat(64);
    let relative = format!("assets/{digest}/manifest.json");
    write_json(
        &temp.path().join("apps/web/public").join(&relative),
        &json!({"alt":"Reviewed source-archive image."}),
    );
    let mut start = room("a:start", None);
    start["assetManifest"] = Value::String(relative);
    write_json(
        &temp.path().join("worlds/areas/a.json"),
        &json!({
            "schemaVersion":1,
            "id":"a",
            "rooms":[start],
            "roomMedia":[{
                "roomId":"a:start",
                "imageAlt":{"en":"Reviewed source-archive image."}
            }]
        }),
    );
    let manifest = temp.path().join("worlds/world.project.json");
    write_json(&manifest, &project(vec![("a", "areas/a.json")]));

    compose_world_project(manifest).unwrap();
}

#[test]
fn compose_cli_never_leaves_a_partial_output_and_never_overwrites() {
    let temp = tempfile::tempdir().unwrap();
    let invalid_project = temp.path().join("invalid.project.json");
    let output_path = temp.path().join("world.json");
    fs::write(&invalid_project, b"not JSON").unwrap();

    let output = run(&[
        "compose",
        invalid_project.to_str().unwrap(),
        "--output",
        output_path.to_str().unwrap(),
    ]);
    assert!(!output.status.success());
    assert!(!output_path.exists());
    assert!(fs::read_dir(temp.path()).unwrap().all(|entry| {
        !entry
            .unwrap()
            .file_name()
            .to_string_lossy()
            .ends_with(".part")
    }));

    let project = write_two_area_project(temp.path(), false);
    fs::write(&output_path, b"preserve").unwrap();
    let output = run(&[
        "compose",
        project.to_str().unwrap(),
        "--output",
        output_path.to_str().unwrap(),
    ]);
    assert!(!output.status.success());
    assert_eq!(fs::read(output_path).unwrap(), b"preserve");

    let successful_output = temp.path().join("composed.json");
    let output = run(&[
        "compose",
        project.to_str().unwrap(),
        "--output",
        successful_output.to_str().unwrap(),
    ]);
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(
        fs::read(&successful_output).unwrap(),
        compose_world_project(&project).unwrap().artifact
    );
    assert!(fs::read_dir(temp.path()).unwrap().all(|entry| {
        !entry
            .unwrap()
            .file_name()
            .to_string_lossy()
            .ends_with(".part")
    }));
}

#[test]
fn compose_is_present_in_top_level_and_subcommand_help() {
    let top = run(&["--help"]);
    assert!(top.status.success());
    assert!(String::from_utf8_lossy(&top.stdout).contains("compose"));

    let compose = run(&["compose", "--help"]);
    assert!(compose.status.success());
    let help = String::from_utf8_lossy(&compose.stdout);
    assert!(help.contains("world.project.json") || help.contains("<PROJECT>"));
    assert!(help.contains("--output"));
    assert!(help.contains("AreaPack v9"));
}

#[test]
fn area_file_and_composed_artifact_size_limits_are_enforced() {
    let oversized = tempfile::tempdir().unwrap();
    let area_path = oversized.path().join("a.json");
    fs::write(&area_path, vec![b' '; MAX_AREA_FILE_BYTES as usize + 1]).unwrap();
    let manifest = oversized.path().join("world.project.json");
    write_json(&manifest, &project(vec![("a", "a.json")]));
    assert!(matches!(
        compose_world_project(manifest),
        Err(CompositionError::FileTooLarge { .. })
    ));

    let expanded = tempfile::tempdir().unwrap();
    let prefix_values = vec![0; 2_000_000];
    write_json(
        &expanded.path().join("a.json"),
        &json!({
            "schemaVersion":1,
            "id":"a",
            "rooms":[{
                "id":"a:start","title":"Start","description":"Start.",
                "legacy":{"flags":0,"sector":0,"prefixValues":prefix_values}
            }]
        }),
    );
    assert!(fs::metadata(expanded.path().join("a.json")).unwrap().len() < MAX_AREA_FILE_BYTES);
    let manifest = expanded.path().join("world.project.json");
    write_json(&manifest, &project(vec![("a", "a.json")]));
    assert!(matches!(
        compose_world_project(manifest),
        Err(CompositionError::ComposedWorldTooLarge { .. })
    ));
}

#[test]
fn area_count_and_total_source_byte_limits_are_enforced_before_composition() {
    let too_many = tempfile::tempdir().unwrap();
    let entries = (0..=world_pack::MAX_AREA_COUNT)
        .map(|index| (format!("area-{index}"), format!("areas/area-{index}.json")))
        .collect::<Vec<_>>();
    let manifest = too_many.path().join("world.project.json");
    write_json(
        &manifest,
        &json!({
            "schemaVersion":1,
            "id":"demo",
            "name":"Demo",
            "startRoomId":"area-0:start",
            "areas":entries.into_iter().map(|(id,path)| json!({"id":id,"path":path})).collect::<Vec<_>>()
        }),
    );
    assert!(matches!(
        compose_world_project(manifest),
        Err(CompositionError::InvalidAreaCount { .. })
    ));

    let total = tempfile::tempdir().unwrap();
    for id in ["a", "b"] {
        write_json(
            &total.path().join(format!("{id}.json")),
            &json!({
                "schemaVersion":1,
                "id":id,
                "rooms":[{
                    "id":format!("{id}:start"),
                    "title":"Room",
                    "description":"x".repeat(8 * 1024 * 1024 + 512 * 1024)
                }]
            }),
        );
    }
    let manifest = total.path().join("world.project.json");
    write_json(&manifest, &project(vec![("a", "a.json"), ("b", "b.json")]));
    assert!(matches!(
        compose_world_project(manifest),
        Err(CompositionError::TotalAreaBytesTooLarge { .. })
    ));
}

#[test]
fn manifest_and_image_manifest_reads_are_bounded_and_require_regular_files() {
    let oversized_manifest = tempfile::tempdir().unwrap();
    let manifest = oversized_manifest.path().join("world.project.json");
    fs::write(
        &manifest,
        vec![b' '; MAX_PROJECT_MANIFEST_BYTES as usize + 1],
    )
    .unwrap();
    assert!(matches!(
        compose_world_project(manifest),
        Err(CompositionError::FileTooLarge {
            kind: "project manifest",
            ..
        })
    ));

    #[cfg(unix)]
    assert!(matches!(
        compose_world_project("/dev/zero"),
        Err(CompositionError::NotRegularFile {
            kind: "project manifest",
            ..
        })
    ));

    let oversized_image = tempfile::tempdir().unwrap();
    let digest = "c".repeat(64);
    let relative = format!("assets/{digest}/manifest.json");
    let asset = oversized_image.path().join(&relative);
    fs::create_dir_all(asset.parent().unwrap()).unwrap();
    fs::write(&asset, vec![b' '; 256 * 1024 + 1]).unwrap();
    let mut start = room("a:start", None);
    start["assetManifest"] = Value::String(relative);
    write_json(
        &oversized_image.path().join("a.json"),
        &json!({
            "schemaVersion":1,
            "id":"a",
            "rooms":[start],
            "roomMedia":[{
                "roomId":"a:start",
                "imageAlt":{"en":"Reviewed image."}
            }]
        }),
    );
    let manifest = oversized_image.path().join("world.project.json");
    write_json(&manifest, &project(vec![("a", "a.json")]));
    assert!(matches!(
        compose_world_project(manifest),
        Err(CompositionError::FileTooLarge {
            kind: "image manifest",
            ..
        })
    ));
}
