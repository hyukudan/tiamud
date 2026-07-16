use std::{fs, path::Path, process::Command};

use serde_json::{Value, json};
use world_pack::{
    AreaPack, CompositionError, MAX_AREA_FILE_BYTES, MAX_PROJECT_MANIFEST_BYTES, WorldPackDocument,
    compose_world_project,
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
    assert_eq!(composed.report.cross_area_references.len(), 6);
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
            "schemaVersion": 1,
            "id": "a",
            "items": [{"id":"shared:id","name":"Item","description":"Item."}],
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
        json!({"schemaVersion":3,"id":"a","rooms":[room("a:start", None)]}),
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
    assert!(help.contains("AreaPack v2"));
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
    let aliases = vec!["x"; 1_400_000];
    write_json(
        &expanded.path().join("a.json"),
        &json!({
            "schemaVersion":1,
            "id":"a",
            "items":[{"id":"a:item","name":"Item","description":"Item.","aliases":aliases}],
            "rooms":[{
                "id":"a:start","title":"Start","description":"Start.","items":["a:item"]
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
