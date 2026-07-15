use std::{fs, process::Command};

const REVIEWED_WORLD: &[u8] = include_bytes!("../../../worlds/lantern-square.world.json");

fn run(arguments: &[&str]) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_tiamud-world"))
        .args(arguments)
        .output()
        .unwrap()
}

#[test]
fn cli_validates_builds_diffs_previews_and_tests_current_world_pack() {
    let temp = tempfile::tempdir().unwrap();
    let input = temp.path().join("input.json");
    let first = temp.path().join("first.json");
    let second = temp.path().join("second.json");
    fs::write(&input, REVIEWED_WORLD).unwrap();

    let output = run(&["validate", input.to_str().unwrap()]);
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );

    for path in [&first, &second] {
        let output = run(&[
            "build",
            input.to_str().unwrap(),
            "--output",
            path.to_str().unwrap(),
        ]);
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
    }
    assert_eq!(fs::read(&first).unwrap(), fs::read(&second).unwrap());

    let output = run(&["diff", first.to_str().unwrap(), second.to_str().unwrap()]);
    assert!(output.status.success());
    assert_eq!(
        String::from_utf8(output.stdout).unwrap(),
        "no differences\n"
    );

    let output = run(&[
        "preview",
        first.to_str().unwrap(),
        "--room",
        "demo:lantern-square",
        "--format",
        "ansi",
    ]);
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let preview = String::from_utf8(output.stdout).unwrap();
    assert!(preview.contains("Lantern Square"));
    assert!(preview.contains("Exits:"));
    assert!(preview.contains("Items:"));
    assert!(!preview.contains("demo:lantern-square"));

    let output = run(&["test", first.to_str().unwrap()]);
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(
        String::from_utf8(output.stdout).unwrap(),
        "content tests passed: 1/1\n"
    );
}

#[test]
fn build_refuses_to_overwrite_without_force() {
    let temp = tempfile::tempdir().unwrap();
    let input = temp.path().join("input.json");
    let output_path = temp.path().join("built.json");
    fs::write(&input, REVIEWED_WORLD).unwrap();
    fs::write(&output_path, b"keep me").unwrap();

    let output = run(&[
        "build",
        input.to_str().unwrap(),
        "--output",
        output_path.to_str().unwrap(),
    ]);
    assert!(!output.status.success());
    assert_eq!(fs::read(&output_path).unwrap(), b"keep me");
}

#[test]
fn validate_checks_localized_alt_against_the_reviewed_manifest() {
    let temp = tempfile::tempdir().unwrap();
    let input = temp.path().join("mismatched-alt.json");
    let mut document: serde_json::Value = serde_json::from_slice(REVIEWED_WORLD).unwrap();
    document["roomMedia"][0]["imageAlt"]["en"] =
        serde_json::Value::String("An unreviewed replacement.".into());
    fs::write(&input, serde_json::to_vec(&document).unwrap()).unwrap();

    let output = run(&["validate", input.to_str().unwrap()]);
    assert!(!output.status.success());
    assert!(
        String::from_utf8_lossy(&output.stderr)
            .contains("English imageAlt must exactly match the reviewed manifest alt")
    );
}

#[cfg(feature = "editorial-ai")]
#[test]
fn editorial_cli_is_fail_closed_and_records_review_without_publishing() {
    let temp = tempfile::tempdir().unwrap();
    let prompt = temp.path().join("prompt.txt");
    let disabled_output = temp.path().join("disabled.json");
    fs::write(&prompt, "Draft a room description.").unwrap();

    let output = run(&[
        "ai-draft",
        "--task",
        "draft-description",
        "--input",
        prompt.to_str().unwrap(),
        "--output",
        disabled_output.to_str().unwrap(),
    ]);
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("editorial AI is disabled"));
    assert!(!disabled_output.exists());

    let draft = temp.path().join("draft.json");
    let reviewed = temp.path().join("reviewed.json");
    let registry = temp.path().join("registry");
    let fixture = serde_json::json!({
        "schemaVersion": 1,
        "kind": "tiamud.editorial-review",
        "state": "draft",
        "suggestion": {
            "provider": "fixture",
            "model": "deterministic",
            "task": "draft_description",
            "text": "Rain silvered the quiet square.",
            "provenance": "deterministic CLI fixture"
        },
        "inputSha256": "a".repeat(64)
    });
    fs::write(&draft, serde_json::to_vec_pretty(&fixture).unwrap()).unwrap();

    let output = run(&[
        "ai-review",
        draft.to_str().unwrap(),
        "--reviewer",
        "Editor",
        "--reviewed-at",
        "2026-07-15T00:00:00Z",
        "--decision",
        "accept",
        "--output",
        reviewed.to_str().unwrap(),
    ]);
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(
        serde_json::from_slice::<serde_json::Value>(&fs::read(&reviewed).unwrap()).unwrap()["suggestion"]
            ["humanReview"]["accepted"],
        true
    );

    let output = run(&[
        "publish",
        reviewed.to_str().unwrap(),
        "--registry",
        registry.to_str().unwrap(),
    ]);
    assert!(!output.status.success());
    assert!(!registry.exists());
}
