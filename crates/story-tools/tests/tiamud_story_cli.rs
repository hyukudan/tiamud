use std::{
    fs,
    io::Write,
    path::PathBuf,
    process::{Command, Output, Stdio},
};

fn fixture(name: &str) -> String {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures")
        .join(name)
        .to_string_lossy()
        .into_owned()
}

fn cripta() -> String {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../story-core/fixtures/cripta-del-nigromante.story.json")
        .to_string_lossy()
        .into_owned()
}

fn run(arguments: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_tiamud-story"))
        .args(arguments)
        .output()
        .unwrap()
}

fn run_with_stdin(arguments: &[&str], stdin: &[u8]) -> Output {
    let mut child = Command::new(env!("CARGO_BIN_EXE_tiamud-story"))
        .args(arguments)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    child.stdin.take().unwrap().write_all(stdin).unwrap();
    child.wait_with_output().unwrap()
}

fn stdout(output: &Output) -> String {
    String::from_utf8(output.stdout.clone()).unwrap()
}

fn stderr(output: &Output) -> String {
    String::from_utf8(output.stderr.clone()).unwrap()
}

#[test]
fn validates_reviewed_fixture_without_findings() {
    let pack = cripta();
    let output = run(&["validate", &pack, "--deny-warnings"]);
    assert!(output.status.success(), "{}", stderr(&output));
    assert_eq!(
        stdout(&output),
        "valid StoryPack \"cripta-del-nigromante\": 10 node(s), 13 choice(s), 5 variable(s), 2 item(s); 0 warning(s), 0 lint(s)\n"
    );
}

#[test]
fn rejects_malformed_and_invalid_packs() {
    let temp = tempfile::tempdir().unwrap();

    let syntax = temp.path().join("syntax.json");
    fs::write(&syntax, b"{ \"format_version\": 1, ").unwrap();
    let output = run(&["validate", syntax.to_str().unwrap()]);
    assert!(!output.status.success());
    assert!(stderr(&output).contains("malformed story document"));

    let unknown = temp.path().join("unknown.json");
    let mut value: serde_json::Value =
        serde_json::from_slice(&fs::read(cripta()).unwrap()).unwrap();
    value["nodes"][0]["sorpresa"] = serde_json::json!(true);
    fs::write(&unknown, serde_json::to_vec(&value).unwrap()).unwrap();
    let output = run(&["validate", unknown.to_str().unwrap()]);
    assert!(!output.status.success());
    assert!(stderr(&output).contains("unknown field `sorpresa`"));

    let dangling = temp.path().join("dangling.json");
    let mut value: serde_json::Value =
        serde_json::from_slice(&fs::read(cripta()).unwrap()).unwrap();
    value["nodes"][0]["choices"][0]["target"] = serde_json::json!("no-existe");
    fs::write(&dangling, serde_json::to_vec(&value).unwrap()).unwrap();
    let output = run(&["validate", dangling.to_str().unwrap()]);
    assert!(!output.status.success());
    let message = stderr(&output);
    assert!(message.contains("is invalid"), "{message}");
    assert!(
        message.contains("nodes[0].choices[0].target: unknown node"),
        "{message}"
    );

    let missing = temp.path().join("missing.json");
    let output = run(&["validate", missing.to_str().unwrap()]);
    assert!(!output.status.success());
    assert!(stderr(&output).contains("could not read"));
}

#[test]
fn reports_core_warnings_and_authoring_lints() {
    let pack = fixture("lints.story.json");
    let output = run(&["validate", &pack]);
    assert!(output.status.success(), "{}", stderr(&output));
    let report = stdout(&output);
    for expected in [
        "warning: nodes[3] (callejon): dead end: node has no choices and no ending",
        "warning: nodes[6] (huerfano): node is unreachable from the start node",
        "lint: nodes[0].choices[0] (inicio/forzar): choice condition can never be satisfied",
        "lint: nodes[0].choices[1] (inicio/pagar): choice condition can never be satisfied",
        "lint: nodes[0].choices[2] (inicio/rezar): choice condition can never be satisfied",
        "lint: nodes[0].choices[3] (inicio/abrir): choice condition can never be satisfied",
        "lint: nodes[0].choices[4] (inicio/amuleto): choice condition always holds",
        "lint: nodes[0].choices[5] (inicio/entrenar).effects[1]: `if` condition can never hold",
        "lint: nodes[0].choices[6] (inicio/saltar).effects[0]: roll test can never succeed (total 1..=6, needs 20)",
        "lint: nodes[1] (bucle): soft-lock: no ending can be reached from this node",
        "lint: nodes[2] (bucle-2): soft-lock: no ending can be reached from this node",
        "lint: nodes[4] (tesoro): node is only reachable through choices or branches that can never be taken",
        "2 warning(s), 10 lint(s)",
    ] {
        assert!(
            report.contains(expected),
            "missing {expected:?} in\n{report}"
        );
    }

    let output = run(&["validate", &pack, "--deny-warnings"]);
    assert!(!output.status.success());
    assert!(stderr(&output).contains("12 warning(s) reported and --deny-warnings is set"));
}

#[test]
fn previews_nodes_like_a_player_without_authored_ids() {
    let pack = cripta();
    let output = run(&["preview", &pack]);
    assert!(output.status.success(), "{}", stderr(&output));
    let scene = stdout(&output);
    assert!(scene.starts_with("== La cripta del nigromante ==\n"));
    assert!(scene.contains("Turno 0 · Puntuación 0 · Destreza 0 · Resistencia 12"));
    assert!(scene.contains("1) Tira un dado y suma 6 para fijar tu destreza\n"));
    assert!(!scene.contains("tirar-destreza"));
    assert!(!scene.contains("vida_esqueleto"));

    let output = run(&["preview", &pack, "--node", "pasillo"]);
    assert!(output.status.success(), "{}", stderr(&output));
    let scene = stdout(&output);
    assert!(scene.contains("1) Un esqueleto armado se alza: luchar\n2) Volver al vestíbulo\n"));
    assert!(scene.contains("(1 choice(s) hidden by conditions in the initial state)"));

    let output = run(&["preview", &pack, "--node", "muerte"]);
    assert!(output.status.success());
    assert!(stdout(&output).contains("*** FIN (derrota) ***"));

    let output = run(&["preview", &pack, "--node", "nowhere"]);
    assert!(!output.status.success());
    assert!(stderr(&output).contains("unknown node \"nowhere\""));
}

#[test]
fn scripted_tests_pass_and_fail_deterministically() {
    let pack = cripta();
    let passing = fixture("cripta-victoria.script.json");
    let output = run(&["test", &pack, "--script", &passing]);
    assert!(output.status.success(), "{}", stderr(&output));
    assert_eq!(
        stdout(&output),
        "story test passed: 17 step(s), 9 choice(s), 7 expectation(s); final node \"vestibulo\" at turn 8\n"
    );

    let failing = fixture("cripta-falla.script.json");
    let output = run(&["test", &pack, "--script", &failing]);
    assert!(!output.status.success());
    assert!(
        stderr(&output)
            .contains("step 2: expectation failed: node is \"vestibulo\", expected \"pasillo\""),
        "{}",
        stderr(&output)
    );

    let temp = tempfile::tempdir().unwrap();
    let typo = temp.path().join("typo.json");
    fs::write(
        &typo,
        br#"{"format_version":1,"steps":[{"expect":{"vars":{"destrezza":1}}}]}"#,
    )
    .unwrap();
    let output = run(&["test", &pack, "--script", typo.to_str().unwrap()]);
    assert!(!output.status.success());
    assert!(stderr(&output).contains("step 1: unknown variable \"destrezza\""));

    let out_of_range = temp.path().join("range.json");
    fs::write(
        &out_of_range,
        br#"{"format_version":1,"steps":[{"choose":5}]}"#,
    )
    .unwrap();
    let output = run(&["test", &pack, "--script", out_of_range.to_str().unwrap()]);
    assert!(!output.status.success());
    assert!(stderr(&output).contains("choose 5: no such choice; 1 available"));
}

#[test]
fn interactive_play_supports_undo_state_and_bounded_input() {
    let pack = cripta();
    let mut input = Vec::new();
    input.extend_from_slice(b"1\n");
    input.extend_from_slice(&[b'x'; 4096]);
    input.extend_from_slice(b"\n9\nestado\ndeshacer\nundo\nhola\nsalir\n1\n");
    let output = run_with_stdin(&["play", &pack, "--seed", "3"], &input);
    assert!(output.status.success(), "{}", stderr(&output));
    let transcript = stdout(&output);
    assert!(
        transcript.contains("[Tirada 1d6: 6 +6 = 12]"),
        "{transcript}"
    );
    assert!(transcript.contains("Entrada demasiado larga."));
    assert!(transcript.contains("No existe esa opción."));
    assert!(transcript.contains("node: vestibulo\nturn: 1 · score: 0 · seed: 3\n"));
    assert!(transcript.contains("var vida_esqueleto = 6"));
    assert!(transcript.contains("No hay nada que deshacer."));
    assert!(transcript.contains("No entiendo."));
    // `salir` stops before the trailing choice is read.
    assert_eq!(transcript.matches("Turno 1 ·").count(), 1);

    let output = run_with_stdin(&["play", &pack, "--locale", "en"], b"help\n");
    assert!(output.status.success());
    let transcript = stdout(&output);
    assert!(transcript.contains("Commands: choice number, undo, state, look, help, quit."));
    assert!(transcript.contains("Turn 0 · Score 0"));
}

#[test]
fn fmt_is_idempotent_and_check_detects_drift() {
    let temp = tempfile::tempdir().unwrap();
    let canonical = temp.path().join("canonical.story.json");
    let output = run(&["fmt", &cripta(), "--output", canonical.to_str().unwrap()]);
    assert!(output.status.success(), "{}", stderr(&output));

    let output = run(&["fmt", canonical.to_str().unwrap(), "--check"]);
    assert!(output.status.success(), "{}", stderr(&output));
    let output = run(&["fmt", canonical.to_str().unwrap()]);
    assert_eq!(output.stdout, fs::read(&canonical).unwrap());

    let output = run(&["fmt", &cripta(), "--output", canonical.to_str().unwrap()]);
    assert!(!output.status.success());
    assert!(stderr(&output).contains("already exists"));

    let output = run(&["fmt", &cripta(), "--check"]);
    assert!(!output.status.success());
    assert!(stderr(&output).contains("is not in canonical form"));

    let output = run(&["validate", canonical.to_str().unwrap(), "--deny-warnings"]);
    assert!(output.status.success(), "{}", stderr(&output));
}

#[test]
fn prints_the_published_schema() {
    let output = run(&["schema"]);
    assert!(output.status.success());
    let schema: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(schema["title"], "Tiamud StoryPack v1");
}
