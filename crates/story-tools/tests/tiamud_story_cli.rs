use std::{
    fs,
    io::Write,
    path::PathBuf,
    process::{Command, Output, Stdio},
};

use tiamud_story_core::StoryPack;

fn fixture(name: &str) -> String {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures")
        .join(name)
        .to_string_lossy()
        .into_owned()
}

fn cripta() -> String {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../story-core/fixtures/crypt-of-the-necromancer.story.json")
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
        "valid StoryPack \"crypt-of-the-necromancer\" (format v1): 10 node(s), 13 choice(s), 5 variable(s), 2 item(s); 0 warning(s), 0 lint(s)\n"
    );

    let output = run(&["validate", &noir(), "--deny-warnings"]);
    assert!(output.status.success(), "{}", stdout(&output));
    assert_eq!(
        stdout(&output),
        "valid StoryPack \"neon-rain\" (format v2): 5 node(s), 0 choice(s), 5 variable(s), 2 item(s), 6 verb(s), 14 menu entr(y/ies), 2 character(s), 2 dialogue(s); 0 warning(s), 0 lint(s)\n"
    );
}

fn noir() -> String {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../story-core/fixtures/neon-rain.story.json")
        .to_string_lossy()
        .into_owned()
}

#[test]
fn reports_menu_dialogue_and_portrait_lints() {
    let pack = fixture("menu-lints.story.json");
    let output = run(&["validate", &pack]);
    assert!(output.status.success(), "{}", stderr(&output));
    let report = stdout(&output);
    for expected in [
        "lint: nodes[0].menu.verbs[3] (sala/rezar): menu verb condition can never hold",
        "lint: nodes[0].menu.entries[0] (sala/mirar-caja): menu entry can never be selected",
        "lint: nodes[0].menu.entries[1] (sala/usar-fantasma): menu entry can never be selected",
        "lint: nodes[0].menu.entries[2] (sala/mirar-mesa).responses[1] (sombra): response is shadowed",
        "lint: nodes[0].menu.entries[2] (sala/mirar-mesa).exhausted: \"nothing new\" text can never be shown",
        "lint: nodes[0].menu.entries[4] (sala/usar-llave).responses[0] (abrir): response condition can never hold",
        "lint: dialogues[0] (charla).nodes[0] (hola).options[1] (secreto): option condition can never hold",
        "lint: dialogues[0] (charla).nodes[1] (perdido): dialogue node is unreachable from the dialogue start",
        "lint: dialogues[1] (huerfano): dialogue is never started by any response",
        "lint: verbs[2].default[0]: expression \"risa\" is not declared in the portraits of \"eva\"",
        "lint: characters[0].default_expression: expression \"sonrisa\" is not declared",
        "lint: nodes[0].present[0]: expression \"furiosa\" is not declared",
        "lint: nodes[0].menu.entries[3].responses[0].blocks[0]: \"eva\" has no portrait for its default expression \"sonrisa\"",
    ] {
        assert!(
            report.contains(expected),
            "missing {expected:?} in\n{report}"
        );
    }
    let output = run(&["validate", &pack, "--deny-warnings"]);
    assert!(!output.status.success());
}

#[test]
fn menu_scenes_preview_test_and_play_like_a_telnet_player() {
    let pack = noir();
    let output = run(&["preview", &pack, "--locale", "es"]);
    assert!(output.status.success(), "{}", stderr(&output));
    let scene = stdout(&output);
    assert!(scene.starts_with("== Lluvia de neón ==\n[fondo: fondo-oficina]\n"));
    assert!(scene.contains(
        "Acciones:\n1) MIRAR: 1) Escritorio · 2) Ventana · 3) Terminal · 4) Puerta\n2) INVESTIGAR: "
    ));
    assert!(scene.contains("3) USAR: (nada)\n"));
    assert!(!scene.contains("investigar-mesa") && !scene.contains("pista_tarjeta"));

    let output = run(&["preview", &pack, "--node", "club"]);
    assert!(output.status.success(), "{}", stderr(&output));
    let scene = stdout(&output);
    assert!(scene.contains("Present: Lola [portrait: retrato-lola-neutral]"));
    assert!(scene.contains("2) TALK: 1) Lola\n"));

    let output = run(&[
        "preview",
        &pack,
        "--node",
        "club",
        "--dialogue",
        "charla-lola/iker",
        "--locale",
        "es",
    ]);
    assert!(output.status.success(), "{}", stderr(&output));
    let scene = stdout(&output);
    assert!(scene.contains(
        "» Lola [retrato: retrato-lola-nerviosa]: —No conozco a ningún Iker. Y baja la voz.\n"
    ));
    assert!(scene.contains("Respuestas:\n1) Cambiar de tema\n"));
    assert!(!scene.contains("Acciones:"));
    let output = run(&["preview", &pack, "--dialogue", "nada"]);
    assert!(!output.status.success());

    let script = fixture("neon-rain-victory.script.json");
    let output = run(&["test", &pack, "--script", &script]);
    assert!(output.status.success(), "{}", stderr(&output));
    assert_eq!(
        stdout(&output),
        "story test passed: 30 step(s), 16 choice(s), 13 expectation(s); final node \"callejon\" at turn 15\n"
    );

    let input = "investigar ventana\n2 2\nINVESTIGAR VENTANA\nmirar nevera\nusar\n7\n2 1\nusar tarjeta\nmoverse puerta\nhablar lola\nmirar barra\n2\n1\n3\n1\nestado\nsalir\n";
    let output = run_with_stdin(
        &["play", &pack, "--seed", "5", "--locale", "es"],
        input.as_bytes(),
    );
    assert!(output.status.success(), "{}", stderr(&output));
    let transcript = stdout(&output);
    for expected in [
        "» El cristal está sucio de hollín.",
        "» Te fijas mejor: un dron de vigilancia",
        "» El dron sigue ahí. Nada nuevo.",
        "No existe esa opción.",
        "Ese verbo necesita un objetivo: N M.",
        "» Entre facturas sin pagar encuentras una tarjeta",
        "[Obtienes: Tarjeta de memoria]",
        "[sonido: sfx-pista]",
        "» Insertas la tarjeta.",
        "[música: musica-club]",
        "» Lola [retrato: retrato-lola-neutral]: —Otra vez la pasma.",
        "Respuestas:\n1) Preguntar por Iker Soto\n2) Pedir un café solo\n3) Despedirte\n",
        "» Lola [retrato: retrato-lola-sonriente]: —Por fin alguien que no pide sintéticos.",
        "var lola_confia = true",
    ] {
        assert!(
            transcript.contains(expected),
            "missing {expected:?} in\n{transcript}"
        );
    }
    // A verb typed while the dialogue waits is not understood.
    assert!(transcript.contains("No entiendo."));
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
    // Without `--locale` the pack's default language (English) is shown.
    let output = run(&["preview", &pack]);
    assert!(output.status.success(), "{}", stderr(&output));
    let scene = stdout(&output);
    assert!(scene.starts_with("== Crypt of the Necromancer ==\n"));
    assert!(scene.contains("Turn 0 · Score 0 · Skill 0 · Stamina 12"));
    assert!(scene.contains("1) Roll one die and add 6 to set your skill\n"));
    assert!(!scene.contains("tirar-destreza"));
    assert!(!scene.contains("vida_esqueleto"));

    let output = run(&["preview", &pack, "--locale", "es"]);
    assert!(output.status.success(), "{}", stderr(&output));
    let scene = stdout(&output);
    assert!(scene.starts_with("== La cripta del nigromante ==\n"));
    assert!(scene.contains("Turno 0 · Puntuación 0 · Destreza 0 · Resistencia 12"));
    assert!(scene.contains("1) Tira un dado y suma 6 para fijar tu destreza\n"));

    let output = run(&["preview", &pack, "--node", "pasillo"]);
    assert!(output.status.success(), "{}", stderr(&output));
    let scene = stdout(&output);
    assert!(scene.contains("1) An armed skeleton rises: fight\n2) Go back to the entrance hall\n"));
    assert!(scene.contains("(1 choice(s) hidden by conditions in the initial state)"));

    let output = run(&["preview", &pack, "--node", "muerte", "--locale", "es"]);
    assert!(output.status.success());
    assert!(stdout(&output).contains("*** FIN (derrota) ***"));
    let output = run(&["preview", &pack, "--node", "muerte"]);
    assert!(output.status.success());
    assert!(stdout(&output).contains("*** THE END (defeat) ***"));

    let output = run(&["preview", &pack, "--node", "nowhere"]);
    assert!(!output.status.success());
    assert!(stderr(&output).contains("unknown node \"nowhere\""));
}

#[test]
fn scripted_tests_pass_and_fail_deterministically() {
    let pack = cripta();
    let passing = fixture("crypt-victory.script.json");
    let output = run(&["test", &pack, "--script", &passing]);
    assert!(output.status.success(), "{}", stderr(&output));
    assert_eq!(
        stdout(&output),
        "story test passed: 17 step(s), 9 choice(s), 7 expectation(s); final node \"vestibulo\" at turn 8\n"
    );

    let failing = fixture("crypt-fail.script.json");
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
    let output = run_with_stdin(&["play", &pack, "--seed", "3", "--locale", "es"], &input);
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

    // English is the pack's default language.
    let output = run_with_stdin(&["play", &pack], b"help\n");
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
    assert_eq!(schema["title"], "Tiamud StoryPack v3");
}

#[test]
fn fmt_upgrade_rewrites_v1_packs_as_v3_losslessly() {
    let temp = tempfile::tempdir().unwrap();
    let upgraded = temp.path().join("upgraded.story.json");
    let output = run(&[
        "fmt",
        &cripta(),
        "--upgrade",
        "--output",
        upgraded.to_str().unwrap(),
    ]);
    assert!(output.status.success(), "{}", stderr(&output));
    let before = StoryPack::from_json_slice(&fs::read(cripta()).unwrap()).unwrap();
    let mut after = StoryPack::from_json_slice(&fs::read(&upgraded).unwrap()).unwrap();
    assert_eq!(after.format_version, 3);
    after.format_version = 1;
    assert_eq!(before, after, "only the version changes");
    let output = run(&["validate", upgraded.to_str().unwrap(), "--deny-warnings"]);
    assert!(output.status.success(), "{}", stdout(&output));
    assert!(stdout(&output).contains("(format v3)"));
    let output = run(&["fmt", &cripta(), "--upgrade", "--check"]);
    assert!(!output.status.success(), "--upgrade conflicts with --check");
}

fn faro() -> String {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../story-core/fixtures/parser/grey-point-lighthouse.story.json")
        .to_string_lossy()
        .into_owned()
}

#[test]
fn parser_worlds_validate_test_and_play_with_free_text() {
    let pack = faro();
    let output = run(&["validate", &pack, "--deny-warnings"]);
    assert!(output.status.success(), "{}", stdout(&output));
    assert_eq!(
        stdout(&output),
        "valid StoryPack \"grey-point-lighthouse\" (format v3): 7 node(s), 0 choice(s), 3 variable(s), 0 item(s), 6 room(s), 14 object(s), 14 rule(s), lexicon(s) en+es; 0 warning(s), 0 lint(s)\n"
    );

    for script in [
        "lighthouse-victory.script.json",
        "lighthouse-victory-es.script.json",
    ] {
        let output = run(&["test", &pack, "--script", &fixture(script)]);
        assert!(output.status.success(), "{script}: {}", stderr(&output));
        assert_eq!(
            stdout(&output),
            "story test passed: 40 step(s), 21 choice(s), 18 expectation(s); final node \"amanecer\" at turn 21\n"
        );
    }

    let temp = tempfile::tempdir().unwrap();
    let wrong = temp.path().join("wrong.json");
    fs::write(
        &wrong,
        r#"{"format_version":3,"language":"es","steps":[{"text":"coge la llave"},{"expect":{"transcript":["Coges"]}},{"text":"examina la barca"},{"expect":{"carried":["medalla"]}}]}"#,
    )
    .unwrap();
    let output = run(&["test", &pack, "--script", wrong.to_str().unwrap()]);
    assert!(!output.status.success());
    assert!(
        stderr(&output).contains("step 2: expectation failed: transcript does not contain \"Coges\"; it reads \"No ves eso por aquí.\""),
        "{}",
        stderr(&output)
    );
    let old = temp.path().join("old.json");
    fs::write(&old, br#"{"format_version":2,"steps":[{"text":"mira"}]}"#).unwrap();
    let output = run(&["test", &pack, "--script", old.to_str().unwrap()]);
    assert!(
        stderr(&output)
            .contains("step 1: text steps and v3 expectations need script format_version 3")
    );
    let output = run(&["test", &cripta(), "--script", wrong.to_str().unwrap()]);
    assert!(stderr(&output).contains("need a pack with a parser world"));

    let input = "examina la barca\ncoge la medalla y póntela\nsur\nguardar\ndeshacer\nestado\nxyzzy\nsalir\nmira\n";
    let output = run_with_stdin(
        &["play", &pack, "--seed", "2", "--locale", "es"],
        input.as_bytes(),
    );
    assert!(output.status.success(), "{}", stderr(&output));
    let transcript = stdout(&output);
    for expected in [
        "Escribe órdenes como en cualquier aventura",
        "» Dentro de la barca varada ves una medalla de plata.",
        "» Coges la medalla de plata.\n» Te pones la medalla de plata.",
        "» Por ahí solo hay roca y mar.",
        "(Sin ranuras de guardado en el modo offline: usa deshacer.)",
        "node: playa\nturn: 3 · score: 0 · seed: 2",
        "» No conozco la palabra 'xyzzy'.",
    ] {
        assert!(
            transcript.contains(expected),
            "missing {expected:?} in\n{transcript}"
        );
    }
    assert!(!transcript.contains("Inventario"), "{transcript}");
}

#[test]
fn reports_parser_world_lints() {
    let output = run(&["validate", &fixture("world-lints.story.json")]);
    assert!(output.status.success(), "{}", stderr(&output));
    let report = stdout(&output);
    for expected in [
        "warning: nodes[2] (sotano): node is unreachable from the start node",
        "warning: world.rules[1]: rule has neither text nor effects",
        "lint: world.objects[1] (moneda-falsa): every es word of this object also names \"moneda\" in the same room",
        "lint: world.objects[2] (llave): object starts nowhere and no effect moves it",
        "lint: world.objects[3] (cofre): its key \"llave\" never enters play and no effect unlocks it",
        "lint: world.objects[4] (reja): object is locked, has no key and no effect unlocks it",
        "lint: world.rules[0] (rezar): the \"en\" lexicon never produces action \"pray\"",
        "2 warning(s), 5 lint(s)",
    ] {
        assert!(
            report.contains(expected),
            "missing {expected:?} in\n{report}"
        );
    }
}

fn duelo() -> String {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../story-script/fixtures/duel-at-the-pass.story.json")
        .to_string_lossy()
        .into_owned()
}

#[test]
fn validates_tests_and_plays_a_pack_with_lua_scripts() {
    let output = run(&["validate", &duelo(), "--deny-warnings"]);
    assert!(output.status.success(), "{}", stderr(&output));
    assert_eq!(
        stdout(&output),
        "valid StoryPack \"duel-at-the-pass\" (format v3): 5 node(s), 4 choice(s), 5 variable(s), 1 item(s), 3 Lua script(s); 0 warning(s), 0 lint(s)\n"
    );
    let output = run(&[
        "test",
        &duelo(),
        "--script",
        &fixture("duel-victory.script.json"),
    ]);
    assert!(output.status.success(), "{}", stderr(&output));
    assert!(stdout(&output).starts_with("story test passed: 11 step(s)"));
    let output = run_with_stdin(&["play", &duelo(), "--seed", "3"], b"1\n1\n");
    assert!(output.status.success(), "{}", stderr(&output));
    assert!(stdout(&output).contains("Attack with your sword"));
}

#[test]
fn reports_script_compile_errors_runtime_failures_and_unused_scripts() {
    let temp = tempfile::tempdir().unwrap();
    let mut pack: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(duelo()).unwrap()).unwrap();
    pack["scripts"][0]["source"] = serde_json::json!("function run()\n  if then\nend");
    let broken = temp.path().join("roto.story.json");
    fs::write(&broken, serde_json::to_vec(&pack).unwrap()).unwrap();
    let output = run(&["validate", broken.to_str().unwrap()]);
    assert!(!output.status.success());
    let message = stderr(&output);
    assert!(
        message.contains("scripts[0].source") && message.contains("asalto:2:"),
        "{message}"
    );

    pack["scripts"][0]["source"] =
        serde_json::json!("function run(story)\n  return story.nada.campo\nend");
    pack["nodes"][1]["choices"][2]["when"] = serde_json::json!({ "has_item": "pocion" });
    let failing = temp.path().join("falla.story.json");
    fs::write(&failing, serde_json::to_vec(&pack).unwrap()).unwrap();
    let output = run(&["validate", failing.to_str().unwrap()]);
    assert!(output.status.success(), "{}", stderr(&output));
    assert!(
        stdout(&output)
            .contains("lint: scripts[1] (puede_huir): script is declared but never referenced"),
        "{}",
        stdout(&output)
    );
    let output = run_with_stdin(&["play", failing.to_str().unwrap()], b"1\n1\n");
    let transcript = format!("{}{}", stdout(&output), stderr(&output));
    assert!(
        transcript.contains("asalto:2: attempt to index a nil value")
            && !transcript.contains("stack traceback"),
        "{transcript}"
    );
}
