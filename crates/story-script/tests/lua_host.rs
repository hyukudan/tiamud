//! End-to-end tests of the Lua story host: the Spanish fixture, determinism,
//! sandbox quotas, grants, failure semantics and a bounded fuzz.

use serde_json::json;
use tiamud_scripting::ScriptLimits;
use tiamud_story_core::{
    StoryError, StoryEvent, StoryInput, StoryPack, StoryState, Value, step, step_scripted, view,
    view_scripted,
};
use tiamud_story_script::{LuaStoryHost, story_script_limits, unused_scripts, validate_pack};

fn fixture() -> StoryPack {
    StoryPack::from_json_slice(include_bytes!("../fixtures/duel-at-the-pass.story.json")).unwrap()
}

/// Takes the choice with this label in the current scripted view.
fn choose(
    pack: &StoryPack,
    state: &StoryState,
    host: &LuaStoryHost,
    label: &str,
) -> Result<(StoryState, Vec<StoryEvent>), StoryError> {
    let current = view_scripted(pack, state, host)?;
    let token = current
        .choices
        .iter()
        .find(|choice| choice.label.resolve("es") == Some(label))
        .map(|choice| choice.token.clone())
        .unwrap_or_else(|| panic!("no choice {label:?} in {:?}", current.choices));
    step_scripted(pack, state, StoryInput::Choose { token }, host)
}

fn int(state: &StoryState, var: &str) -> i64 {
    match state.vars.get(var) {
        Some(Value::Int(value)) => *value,
        other => panic!("{var} is {other:?}"),
    }
}

/// Plays the duel to the end with the given seed, attacking every round.
fn duel(seed: u64) -> (StoryState, Vec<Vec<StoryEvent>>) {
    let pack = fixture();
    let host = LuaStoryHost::new();
    let mut state = StoryState::new(&pack, seed).unwrap();
    let mut log = Vec::new();
    let (next, events) =
        choose(&pack, &state, &host, "Desenvainar (tira un dado y suma 6)").unwrap();
    state = next;
    log.push(events);
    for _ in 0..64 {
        if state.ended.is_some() {
            break;
        }
        let (next, events) = choose(&pack, &state, &host, "Atacar con la espada").unwrap();
        state = next;
        log.push(events);
    }
    (state, log)
}

/// Minimal v3 pack with one script and one choice that runs or tests it.
fn scripted_pack(kind: &str, source: &str, extra: serde_json::Value) -> StoryPack {
    let mut script = json!({ "id": "guion", "kind": kind, "source": source });
    if let (Some(script), Some(extra)) = (script.as_object_mut(), extra.as_object()) {
        for (key, value) in extra {
            script.insert(key.clone(), value.clone());
        }
    }
    let choice = if kind == "effect" {
        json!({
            "id": "probar", "label": { "es": "Probar" },
            "effects": [{ "script": { "id": "guion" } }], "target": "inicio"
        })
    } else {
        json!({
            "id": "probar", "label": { "es": "Probar" },
            "when": { "script": { "id": "guion" } }, "target": "fin"
        })
    };
    let pack = json!({
        "format_version": 3,
        "id": "prueba-guion",
        "title": { "es": "Prueba" },
        "start": "inicio",
        "variables": [
            { "id": "vida", "kind": { "int": { "min": 0, "max": 10, "initial": 5 } } },
            { "id": "secreto", "kind": { "int": { "min": 0, "max": 10, "initial": 1 } } },
            { "id": "alerta", "kind": { "bool": { "initial": false } } }
        ],
        "items": [{ "id": "llave", "name": { "es": "Llave" } }],
        "scripts": [script],
        "nodes": [
            {
                "id": "inicio", "text": { "es": "Inicio." },
                "choices": [choice, {
                    "id": "terminar", "label": { "es": "Terminar" }, "target": "fin"
                }]
            },
            {
                "id": "fin", "text": { "es": "Fin." },
                "ending": { "id": "fin", "kind": "neutral" }
            }
        ]
    });
    let pack = StoryPack::from_json_slice(&serde_json::to_vec(&pack).unwrap()).unwrap();
    validate_pack(&pack).unwrap_or_else(|errors| panic!("{errors}"));
    pack
}

/// Runs the pack's first choice and returns the error, asserting that the
/// step failed closed (the input state is untouched and reusable).
fn effect_error(pack: &StoryPack, host: &LuaStoryHost) -> StoryError {
    let state = StoryState::new(pack, 7).unwrap();
    let before = state.clone();
    let token = view_scripted(pack, &state, host).unwrap().choices[0]
        .token
        .clone();
    let error = step_scripted(pack, &state, StoryInput::Choose { token }, host).unwrap_err();
    assert_eq!(state, before, "a failed step must not change the state");
    // The same state still accepts a different, script-free input.
    let token = view_scripted(pack, &state, host).unwrap().choices[1]
        .token
        .clone();
    step_scripted(pack, &state, StoryInput::Choose { token }, host).unwrap();
    error
}

fn script_message(error: StoryError) -> String {
    match error {
        StoryError::Script { script, message } => {
            assert_eq!(script, "guion");
            message
        }
        other => panic!("expected a script error, got {other:?}"),
    }
}

#[test]
fn fixture_validates_only_for_a_scripted_runtime() {
    let pack = fixture();
    let report = validate_pack(&pack).unwrap();
    assert!(report.warnings.is_empty(), "{:?}", report.warnings);
    assert!(unused_scripts(&pack).is_empty());
    let errors = pack.validate().unwrap_err();
    assert!(
        errors
            .errors
            .iter()
            .any(|issue| issue.path == "scripts" && issue.message.contains("no script host")),
        "{errors}"
    );
}

#[test]
fn the_duel_is_deterministic_and_reaches_an_ending() {
    let (first, first_log) = duel(11);
    let (second, second_log) = duel(11);
    assert_eq!(first, second);
    assert_eq!(first_log, second_log);
    let ending = first.ended.as_ref().expect("the duel ends");
    assert!(matches!(ending.id.as_str(), "paso-libre" | "sin-bolsa"));
    assert!(int(&first, "ronda") >= 1);
    // Different seeds explore different fights.
    let outcomes: std::collections::BTreeSet<_> = (0..24)
        .map(|seed| {
            let (state, _) = duel(seed);
            (int(&state, "ronda"), state.ended.unwrap().id)
        })
        .collect();
    assert!(outcomes.len() > 1, "{outcomes:?}");
}

#[test]
fn script_dice_advance_the_story_rng() {
    let pack = fixture();
    let host = LuaStoryHost::new();
    let state = StoryState::new(&pack, 3).unwrap();
    let (state, _) = choose(&pack, &state, &host, "Desenvainar (tira un dado y suma 6)").unwrap();
    let (after, _) = choose(&pack, &state, &host, "Atacar con la espada").unwrap();
    assert_ne!(
        after.rng, state.rng,
        "an effect script draws its seed from the story RNG"
    );
    assert_eq!(int(&after, "ronda"), 1);
}

#[test]
fn condition_scripts_gate_choices_in_view_and_step_alike() {
    let pack = fixture();
    let host = LuaStoryHost::new();
    let mut state = StoryState::new(&pack, 5).unwrap();
    state.node = "combate".to_owned();
    state.vars.insert("destreza".to_owned(), Value::Int(12));
    let labels = |state: &StoryState| -> Vec<String> {
        view_scripted(&pack, state, &host)
            .unwrap()
            .choices
            .iter()
            .map(|choice| choice.label.resolve("es").unwrap().to_owned())
            .collect()
    };
    assert!(!labels(&state).contains(&"Huir por la ladera".to_owned()));
    state.vars.insert("ronda".to_owned(), Value::Int(2));
    assert!(labels(&state).contains(&"Huir por la ladera".to_owned()));
    let (fled, _) = choose(&pack, &state, &host, "Huir por la ladera").unwrap();
    assert_eq!(fled.ended.unwrap().id, "huida");
    state.vars.insert("vigor".to_owned(), Value::Int(3));
    assert!(!labels(&state).contains(&"Huir por la ladera".to_owned()));
}

#[test]
fn the_potion_script_takes_only_granted_items() {
    let pack = fixture();
    let host = LuaStoryHost::new();
    let mut state = StoryState::new(&pack, 9).unwrap();
    state.node = "combate".to_owned();
    state.vars.insert("vigor".to_owned(), Value::Int(4));
    let (after, events) = choose(&pack, &state, &host, "Beber la poción de vigor").unwrap();
    assert!(!after.inventory.contains("pocion"));
    assert!(
        (9..=12).contains(&int(&after, "vigor")),
        "{}",
        int(&after, "vigor")
    );
    assert!(events.contains(&StoryEvent::ItemLost {
        item: "pocion".to_owned()
    }));
}

#[test]
fn without_a_host_scripts_fail_closed() {
    let pack = fixture();
    let host = LuaStoryHost::new();
    let state = StoryState::new(&pack, 1).unwrap();
    let (state, _) = choose(&pack, &state, &host, "Desenvainar (tira un dado y suma 6)").unwrap();
    // The combat scene has a condition script: the plain view refuses it.
    assert_eq!(view(&pack, &state), Err(StoryError::ScriptUnavailable));
    let token = view_scripted(&pack, &state, &host).unwrap().choices[0]
        .token
        .clone();
    let before = state.clone();
    assert_eq!(
        step(&pack, &state, StoryInput::Choose { token }),
        Err(StoryError::ScriptUnavailable)
    );
    assert_eq!(state, before);
}

#[test]
fn scripts_cannot_mutate_state_outside_intents() {
    let pack = scripted_pack(
        "effect",
        r#"
        function run(story, args)
          story.vars.vida = 0
          story.vars.secreto = 9
          story.items.llave = true
          story.node = "fin"
          story.turn = 999
          fx.add("vida", 1)
        end
        "#,
        json!({ "writes": ["vida"] }),
    );
    let host = LuaStoryHost::new();
    let state = StoryState::new(&pack, 1).unwrap();
    let token = view_scripted(&pack, &state, &host).unwrap().choices[0]
        .token
        .clone();
    let (after, _) = step_scripted(&pack, &state, StoryInput::Choose { token }, &host).unwrap();
    assert_eq!(int(&after, "vida"), 6);
    assert_eq!(int(&after, "secreto"), 1);
    assert!(after.inventory.is_empty());
    assert_eq!(after.node, "inicio");
    assert_eq!(after.turn, 1);
}

#[test]
fn ungranted_or_malformed_intents_are_rejected() {
    let host = LuaStoryHost::new();
    let cases = [
        (
            r#"function run() fx.add("secreto", 1) end"#,
            "not in `writes`",
        ),
        (r#"function run() fx.give("llave") end"#, "not in `items`"),
        (r#"function run() fx.go("fin") end"#, "not in `goto`"),
        (
            r#"function run() fx.score(5) end"#,
            "may not change the score",
        ),
        (
            r#"function run() fx.finish("fin", "victory") end"#,
            "may not end",
        ),
        (
            r#"function run() fx.set("vida", true) end"#,
            "does not match",
        ),
        (
            r#"function run() fx.add("vida", 1.5) end"#,
            "must be an integer",
        ),
        (
            r#"function run() tiamud.emit("cue", "musica", nil) end"#,
            "capability",
        ),
        (
            r#"function run() tiamud.emit("add", "vida", "mucho") end"#,
            "integer value",
        ),
        (r#"function run() error("regla rota") end"#, "regla rota"),
        (
            r#"function run() local x = nil; return x.campo end"#,
            "guion:1:",
        ),
    ];
    for (source, expected) in cases {
        let pack = scripted_pack("effect", source, json!({ "writes": ["vida"] }));
        let message = script_message(effect_error(&pack, &host));
        assert!(message.contains(expected), "{source}: {message}");
    }
}

#[test]
fn condition_scripts_are_pure_and_must_return_booleans() {
    let host = LuaStoryHost::new();
    for (source, expected) in [
        (
            r#"function check() fx.add("vida", 1) return true end"#,
            "capability",
        ),
        ("function check() return 1 end", "must return a boolean"),
        ("function check() end", "must return a boolean"),
    ] {
        let pack = scripted_pack("condition", source, json!({}));
        let state = StoryState::new(&pack, 1).unwrap();
        let error = view_scripted(&pack, &state, &host).unwrap_err();
        assert!(script_message(error).contains(expected), "{source}");
    }
    // Grants on a condition script are a validation error.
    let pack = json!({
        "format_version": 3, "id": "x", "title": { "es": "X" }, "start": "a",
        "variables": [{ "id": "v", "kind": { "bool": { "initial": false } } }],
        "scripts": [{ "id": "c", "kind": "condition", "source": "function check() return true end",
                      "writes": ["v"] }],
        "nodes": [{ "id": "a", "text": { "es": "A" }, "ending": { "id": "a", "kind": "neutral" } }]
    });
    let pack = StoryPack::from_json_slice(&serde_json::to_vec(&pack).unwrap()).unwrap();
    assert!(validate_pack(&pack).is_err());
}

#[test]
fn sandbox_quotas_stop_hostile_scripts() {
    let host = LuaStoryHost::new();
    let cases = [
        ("function run() while true do end end", "quota"),
        (
            "function run() local t = {} for i = 1, 1e9 do t[i] = string.rep('x', 1024) end end",
            "quota",
        ),
        (
            "function run() local s = string.rep('x', 64 * 1024 * 1024) end",
            "memory",
        ),
        (
            r#"function run() for i = 1, 40 do fx.add("vida", 1) end end"#,
            "too many events",
        ),
        (
            r#"function run() fx.give(string.rep("x", 2000)) end"#,
            "too many events or an oversized event field",
        ),
        (
            "function run() local function f() return f() + 1 end f() end",
            "",
        ),
    ];
    for (source, expected) in cases {
        let pack = scripted_pack("effect", source, json!({ "writes": ["vida"] }));
        let message = script_message(effect_error(&pack, &host));
        assert!(message.contains(expected), "{source}: {message}");
    }
    // Tighter limits apply when a runtime asks for them.
    let strict = LuaStoryHost::with_limits(ScriptLimits {
        max_instructions: 500,
        ..story_script_limits()
    });
    let pack = scripted_pack(
        "effect",
        "function run() local n = 0 for i = 1, 10000 do n = n + i end end",
        json!({}),
    );
    assert!(script_message(effect_error(&pack, &strict)).contains("quota"));
}

#[test]
fn the_sandbox_exposes_no_io_clock_or_nondeterminism() {
    let pack = scripted_pack(
        "effect",
        r#"
        function run(story)
          assert(os == nil and io == nil and package == nil and require == nil)
          assert(load == nil and dofile == nil and loadfile == nil and debug == nil)
          assert(print == nil and collectgarbage == nil and next == nil)
          assert(math.random == nil and string.format == nil and string.find == nil)
          assert(tostring({}) == "table" and tostring(print) == "nil")
          local keys = ""
          for name, _ in pairs({ zeta = 1, alfa = 2, [3] = 3, mu = 4, [1] = 5 }) do
            keys = keys .. tostring(name) .. ","
          end
          assert(keys == "1,3,alfa,mu,zeta,", keys)
          local order = 0
          for name, _ in pairs(story.vars) do order = order * 3 + #name end
          fx.set("vida", order % 10)
        end
        "#,
        json!({ "writes": ["vida"] }),
    );
    let host = LuaStoryHost::new();
    let run = || {
        let state = StoryState::new(&pack, 1).unwrap();
        let token = view_scripted(&pack, &state, &host).unwrap().choices[0]
            .token
            .clone();
        step_scripted(&pack, &state, StoryInput::Choose { token }, &host).unwrap()
    };
    let first = run();
    for _ in 0..8 {
        assert_eq!(run(), first);
    }
}

#[test]
fn validation_reports_compile_errors_with_the_script_line() {
    let pack = json!({
        "format_version": 3, "id": "x", "title": { "es": "X" }, "start": "a",
        "scripts": [
            { "id": "roto", "kind": "effect", "source": "function run()\n  if then\nend" },
            { "id": "sin-run", "kind": "effect", "source": "function otra() end" },
            { "id": "bucle", "kind": "condition", "source": "while true do end" }
        ],
        "nodes": [{ "id": "a", "text": { "es": "A" }, "ending": { "id": "a", "kind": "neutral" } }]
    });
    let pack = StoryPack::from_json_slice(&serde_json::to_vec(&pack).unwrap()).unwrap();
    let errors = validate_pack(&pack).unwrap_err();
    let text = errors.to_string();
    assert!(
        text.contains("scripts[0].source") && text.contains("roto:2:"),
        "{text}"
    );
    assert!(
        text.contains("scripts[1].source") && text.contains("`run`"),
        "{text}"
    );
    assert!(
        text.contains("scripts[2].source") && text.contains("quota"),
        "{text}"
    );
    assert_eq!(unused_scripts(&pack), vec!["roto", "sin-run", "bucle"]);
}

#[test]
fn the_per_step_script_budget_fails_the_step_closed() {
    let choices: Vec<_> = (0..20)
        .map(|index| {
            json!({
                "id": format!("c{index}"), "label": { "es": format!("Opción {index}") },
                "when": { "script": { "id": "si" } }, "target": "a"
            })
        })
        .collect();
    let pack = json!({
        "format_version": 3, "id": "x", "title": { "es": "X" }, "start": "a",
        "scripts": [{ "id": "si", "kind": "condition", "source": "function check() return true end" }],
        "nodes": [
            { "id": "a", "text": { "es": "A" }, "choices": choices },
            { "id": "b", "text": { "es": "B" }, "ending": { "id": "b", "kind": "neutral" } }
        ]
    });
    let pack = StoryPack::from_json_slice(&serde_json::to_vec(&pack).unwrap()).unwrap();
    validate_pack(&pack).unwrap();
    let host = LuaStoryHost::new();
    let state = StoryState::new(&pack, 1).unwrap();
    assert_eq!(
        view_scripted(&pack, &state, &host),
        Err(StoryError::ScriptBudget {
            limit: tiamud_story_core::MAX_SCRIPT_CALLS_PER_STEP
        })
    );
}

fn world_pack(objects_grant: serde_json::Value) -> StoryPack {
    let pack = json!({
        "format_version": 3, "id": "faro", "title": { "es": "Faro" }, "start": "sala",
        "variables": [{ "id": "visto", "kind": { "bool": { "initial": false } } }],
        "world": {
            "lexicons": [{ "id": "es" }],
            "objects": [
                { "id": "cofre", "name": { "es": "cofre" }, "nouns": { "es": ["cofre"] },
                  "description": { "es": "Un cofre." }, "location": { "room": "sala" },
                  "container": true, "openable": true },
                { "id": "llave", "name": { "es": "llave" }, "gender": "feminine",
                  "nouns": { "es": ["llave"] }, "description": { "es": "Una llave." },
                  "location": "carried", "portable": true }
            ],
            "rules": [{
                "id": "guardar-al-esperar", "phase": "instead", "action": "wait",
                "effects": [{ "script": { "id": "guardar" } }]
            }]
        },
        "scripts": [{
            "id": "guardar", "kind": "effect", "objects": objects_grant, "writes": ["visto"],
            "source": "function run(story)\n  if story.objects.llave.at == 'carried' and not story.objects.cofre.open then\n    fx.set_object('cofre', 'open', true)\n    fx.move('llave', 'in:cofre')\n    fx.set('visto', true)\n  end\nend"
        }],
        "nodes": [{
            "id": "sala", "text": { "es": "Una sala." }, "room": { "name": { "es": "Sala" } }
        }]
    });
    let pack = StoryPack::from_json_slice(&serde_json::to_vec(&pack).unwrap()).unwrap();
    validate_pack(&pack).unwrap_or_else(|errors| panic!("{errors}"));
    pack
}

#[test]
fn world_objects_are_projected_and_changed_only_when_granted() {
    let host = LuaStoryHost::new();
    let pack = world_pack(json!(["llave", "cofre"]));
    let state = StoryState::new(&pack, 1).unwrap();
    let stepped =
        tiamud_story_core::step_command_scripted(&pack, &state, "espera", Some("es"), &host)
            .unwrap();
    let after = stepped.state;
    assert!(after.objects["cofre"].open);
    assert_eq!(
        after.objects["llave"].at,
        tiamud_story_core::Location::In("cofre".to_owned())
    );
    assert_eq!(after.vars["visto"], Value::Bool(true));

    let pack = world_pack(json!(["cofre"]));
    let state = StoryState::new(&pack, 1).unwrap();
    let error =
        tiamud_story_core::step_command_scripted(&pack, &state, "espera", Some("es"), &host)
            .unwrap_err();
    match error {
        StoryError::Script { script, message } => {
            assert_eq!(script, "guardar");
            assert!(message.contains("not in `objects`"), "{message}");
        }
        other => panic!("{other:?}"),
    }
}

/// Bounded fuzz: random seeds and random choice sequences over the fixture
/// never panic, every offered choice is accepted by `step`, garbage tokens
/// fail without side effects, and replaying a walk reproduces its states.
#[test]
fn fuzz_random_walks_over_the_fixture() {
    let pack = fixture();
    let host = LuaStoryHost::new();
    let mut generator = 0x5eed_u64;
    let mut next = move || {
        generator ^= generator << 13;
        generator ^= generator >> 7;
        generator ^= generator << 17;
        generator
    };
    for _ in 0..24 {
        let seed = next();
        let picks: Vec<u64> = (0..40).map(|_| next()).collect();
        let walk = || {
            let mut state = StoryState::new(&pack, seed).unwrap();
            let mut trail = vec![state.clone()];
            for pick in &picks {
                if state.ended.is_some() {
                    break;
                }
                let current = view_scripted(&pack, &state, &host).unwrap();
                assert!(!current.choices.is_empty());
                let count = u64::try_from(current.choices.len()).unwrap();
                let index = usize::try_from(*pick % count).unwrap();
                let token = current.choices[index].token.clone();
                let (next_state, _) =
                    step_scripted(&pack, &state, StoryInput::Choose { token }, &host).unwrap();
                next_state.check_against(&pack).unwrap();
                state = next_state;
                trail.push(state.clone());
            }
            if state.ended.is_none() {
                let before = state.clone();
                for token in ["t999999c0", "zzz", "", "t0c99"] {
                    let input = StoryInput::Choose {
                        token: token.to_owned(),
                    };
                    assert!(step_scripted(&pack, &state, input, &host).is_err());
                }
                assert_eq!(state, before);
            }
            trail
        };
        assert_eq!(walk(), walk());
    }
}
