//! Scenario, determinism and validation tests over the Spanish sample gamebook.

use serde_json::json;
use tiamud_story_core::{
    EndingKind, MAX_CHOICES_PER_NODE, StoryError, StoryEvent, StoryInput, StoryPack, StoryState,
    UndoRing, Value, resolve_choice, step, view,
};

const FIXTURE: &[u8] = include_bytes!("../fixtures/cripta-del-nigromante.story.json");

fn pack() -> StoryPack {
    let pack = StoryPack::from_json_slice(FIXTURE).expect("fixture parses");
    let report = pack.validate().expect("fixture validates");
    assert!(report.warnings.is_empty(), "{:?}", report.warnings);
    pack
}

fn fixture_value() -> serde_json::Value {
    serde_json::from_slice(FIXTURE).expect("fixture json")
}

fn token_for(pack: &StoryPack, state: &StoryState, id: &str) -> Option<String> {
    view(pack, state)
        .expect("view")
        .choices
        .into_iter()
        .map(|choice| choice.token)
        .find(|token| resolve_choice(pack, state, token).map(|c| c.id.as_str()) == Ok(id))
}

fn choose(pack: &StoryPack, state: &StoryState, id: &str) -> (StoryState, Vec<StoryEvent>) {
    let token = token_for(pack, state, id).unwrap_or_else(|| panic!("choice {id} unavailable"));
    step(pack, state, StoryInput::Choose { token }).expect("step")
}

fn int(state: &StoryState, var: &str) -> i64 {
    match state.vars[var] {
        Value::Int(value) => value,
        Value::Bool(_) => panic!("{var} is boolean"),
    }
}

/// Fights from the combat node until it is left; bounded.
fn fight(pack: &StoryPack, mut state: StoryState) -> StoryState {
    for _ in 0..64 {
        if state.node != "combate" || state.ended.is_some() {
            return state;
        }
        state = choose(pack, &state, "atacar").0;
    }
    panic!("combat did not terminate");
}

fn walk_to_combat(pack: &StoryPack, seed: u64) -> StoryState {
    let state = StoryState::new(pack, seed).expect("state");
    let (state, _) = choose(pack, &state, "tirar-destreza");
    let (state, _) = choose(pack, &state, "ir-norte");
    choose(pack, &state, "luchar").0
}

#[test]
fn scripted_victory_path_with_dice_and_conditional_choice() {
    let pack = pack();
    let state = StoryState::new(&pack, 1).expect("state");
    let first = view(&pack, &state).expect("view");
    assert_eq!(first.title.resolve("es"), Some("La cripta del nigromante"));
    assert_eq!(first.choices.len(), 1);
    assert_eq!(first.stats.len(), 2, "only visible stats are shown");

    let (state, events) = choose(&pack, &state, "tirar-destreza");
    assert!(matches!(
        events[0],
        StoryEvent::DiceRolled {
            count: 1,
            sides: 6,
            ..
        }
    ));
    assert!((7..=12).contains(&int(&state, "destreza")));
    assert_eq!(state.node, "vestibulo");

    // The conditional door choice is hidden without the key.
    assert!(token_for(&pack, &state, "abrir-puerta").is_none());
    let (state, events) = choose(&pack, &state, "coger-antorcha");
    assert!(events.contains(&StoryEvent::ItemGained {
        item: "antorcha".to_owned()
    }));
    assert!(token_for(&pack, &state, "coger-antorcha").is_none());

    // Find a seed whose combat is won; dice are seeded so this is reproducible.
    let won = (0..500u64)
        .map(|seed| fight(&pack, walk_to_combat(&pack, seed)))
        .find(|state| state.node == "victoria-esqueleto")
        .expect("some seed wins the fight");
    assert_eq!(won.vars["esqueleto_vencido"], Value::Bool(true));
    assert_eq!(int(&won, "vida_esqueleto"), 0);
    assert_eq!(won.score, 50);

    let (state, _) = choose(&pack, &won, "coger-llave");
    assert!(
        token_for(&pack, &state, "luchar").is_none(),
        "flag hides the fight"
    );
    assert!(token_for(&pack, &state, "ir-al-foso").is_some());
    let (state, _) = choose(&pack, &state, "volver");
    let (state, events) = choose(&pack, &state, "abrir-puerta");
    assert_eq!(state.score, 150);
    assert!(!state.inventory.contains("llave-de-hierro"));
    let ending = state.ended.clone().expect("ended");
    assert_eq!(ending.kind, EndingKind::Victory);
    assert!(events.contains(&StoryEvent::Ended(ending)));
    let final_view = view(&pack, &state).expect("view");
    assert!(final_view.choices.is_empty());

    let stale = step(
        &pack,
        &state,
        StoryInput::Choose {
            token: format!("t{}c0", state.turn),
        },
    );
    assert_eq!(stale, Err(StoryError::Ended));
}

#[test]
fn losing_combat_reaches_defeat() {
    let pack = pack();
    let lost = (0..500u64)
        .map(|seed| fight(&pack, walk_to_combat(&pack, seed)))
        .find(|state| state.node == "muerte")
        .expect("some seed loses the fight");
    assert_eq!(
        lost.ended.as_ref().map(|e| e.kind),
        Some(EndingKind::Defeat)
    );
    assert_eq!(int(&lost, "resistencia"), 0, "endurance is clamped at zero");
}

#[test]
fn tokens_are_bound_to_the_view() {
    let pack = pack();
    let state = StoryState::new(&pack, 9).expect("state");
    let (next, _) = choose(&pack, &state, "tirar-destreza");
    let old = StoryInput::Choose {
        token: "t0c0".to_owned(),
    };
    assert_eq!(step(&pack, &next, old), Err(StoryError::StaleChoice));
    // Vestibule offers two available choices; ordinal 2 would be hidden.
    let input = StoryInput::Choose {
        token: format!("t{}c2", next.turn),
    };
    assert_eq!(step(&pack, &next, input), Err(StoryError::UnknownChoice));
    for choice in view(&pack, &next).expect("view").choices {
        assert!(!choice.token.contains("antorcha") && !choice.token.contains("norte"));
    }
}

#[test]
fn same_seed_and_inputs_are_deterministic_and_resumable() {
    let pack = pack();
    let run = |seed: u64| {
        let mut state = walk_to_combat(&pack, seed);
        let mut log = Vec::new();
        for _ in 0..10 {
            if state.node != "combate" {
                break;
            }
            let (next, events) = choose(&pack, &state, "atacar");
            log.push((next.clone(), events));
            state = next;
        }
        log
    };
    assert_eq!(run(42), run(42));
    let differs = (0..20u64).any(|seed| run(seed) != run(42));
    assert!(differs, "different seeds produce different rolls");

    // Saving and restoring mid-run continues identically.
    let state = walk_to_combat(&pack, 42);
    let bytes = state.to_json().expect("serialize");
    let restored = StoryState::from_json_slice(&bytes, &pack).expect("restore");
    assert_eq!(restored, state);
    assert_eq!(
        choose(&pack, &restored, "atacar"),
        choose(&pack, &state, "atacar")
    );
}

#[test]
fn undo_ring_restores_previous_states_within_capacity() {
    let pack = pack();
    assert!(UndoRing::new(0).is_err());
    assert!(UndoRing::new(tiamud_story_core::MAX_UNDO_CAPACITY + 1).is_err());
    let mut ring = UndoRing::new(2).expect("ring");
    let s0 = StoryState::new(&pack, 3).expect("state");
    let (s1, _) = choose(&pack, &s0, "tirar-destreza");
    let (s2, _) = choose(&pack, &s1, "ir-norte");
    let (s3, _) = choose(&pack, &s2, "volver");
    assert!(ring.push(s0.clone()).is_none());
    assert!(ring.push(s1.clone()).is_none());
    assert_eq!(ring.push(s2.clone()), Some(s0), "oldest is evicted");
    assert_eq!(ring.len(), 2);
    assert_ne!(s3, s2);
    assert_eq!(ring.pop(), Some(s2));
    assert_eq!(ring.pop(), Some(s1));
    assert!(ring.pop().is_none());
    assert!(ring.is_empty());
}

#[test]
fn state_mismatches_are_rejected() {
    let pack = pack();
    let base = StoryState::new(&pack, 5).expect("state");
    let token = || StoryInput::Choose {
        token: "t0c0".to_owned(),
    };

    let mut state = base.clone();
    state.vars.insert("resistencia".to_owned(), Value::Int(99));
    assert!(matches!(
        step(&pack, &state, token()),
        Err(StoryError::StateMismatch(_))
    ));

    let mut state = base.clone();
    state
        .vars
        .insert("resistencia".to_owned(), Value::Bool(true));
    assert!(matches!(
        step(&pack, &state, token()),
        Err(StoryError::StateMismatch(_))
    ));

    let mut state = base.clone();
    state.inventory.insert("espada".to_owned());
    assert!(matches!(
        step(&pack, &state, token()),
        Err(StoryError::StateMismatch(_))
    ));

    let mut state = base.clone();
    state.node = "nowhere".to_owned();
    assert!(matches!(
        view(&pack, &state),
        Err(StoryError::StateMismatch(_))
    ));

    let mut state = base.clone();
    state.pack_id = "other".to_owned();
    assert!(matches!(
        step(&pack, &state, token()),
        Err(StoryError::StateMismatch(_))
    ));

    let mut state = base.clone();
    state.turn = tiamud_story_core::MAX_TURNS;
    state.vars.insert("destreza".to_owned(), Value::Int(1));
    let input = StoryInput::Choose {
        token: format!("t{}c0", state.turn),
    };
    assert_eq!(step(&pack, &state, input), Err(StoryError::TurnLimit));

    let mut bytes = base.to_json().expect("json");
    bytes.truncate(bytes.len() - 1);
    assert!(matches!(
        StoryState::from_json_slice(&bytes, &pack),
        Err(StoryError::Malformed(_))
    ));
    let mut raw: serde_json::Value = serde_json::from_slice(&base.to_json().unwrap()).unwrap();
    raw["extra"] = json!(1);
    assert!(matches!(
        StoryState::from_json_slice(raw.to_string().as_bytes(), &pack),
        Err(StoryError::Malformed(_))
    ));
}

fn rejects(mutate: impl FnOnce(&mut serde_json::Value), needle: &str) {
    let mut value = fixture_value();
    mutate(&mut value);
    let pack = StoryPack::from_json_slice(value.to_string().as_bytes()).expect("parses");
    let errors = pack.validate().expect_err("must be rejected");
    assert!(
        errors
            .errors
            .iter()
            .any(|issue| issue.message.contains(needle)),
        "expected {needle:?} in {errors}"
    );
}

#[test]
fn validation_rejects_malformed_packs() {
    rejects(
        |v| v["nodes"][1]["id"] = json!("inicio"),
        "duplicate node id",
    );
    rejects(
        |v| v["nodes"][0]["choices"][0]["target"] = json!("nada"),
        "unknown node",
    );
    rejects(|v| v["start"] = json!("nada"), "start node does not exist");
    rejects(
        |v| v["variables"][0]["kind"]["int"]["initial"] = json!(50),
        "initial value is out of bounds",
    );
    rejects(
        |v| v["variables"][0]["kind"]["int"]["min"] = json!(13),
        "min exceeds max",
    );
    rejects(
        |v| v["variables"][1]["id"] = json!("destreza"),
        "duplicate variable id",
    );
    rejects(
        |v| v["nodes"][1]["choices"][0]["effects"][0] = json!({"give": "espada"}),
        "undeclared item",
    );
    rejects(
        |v| {
            v["nodes"][1]["choices"][0]["effects"][0] =
                json!({"set": {"var": "destreza", "value": true}})
        },
        "does not match the variable type",
    );
    rejects(
        |v| {
            v["nodes"][1]["choices"][0]["effects"][0] =
                json!({"set": {"var": "destreza", "value": 13}})
        },
        "outside the variable bounds",
    );
    rejects(
        |v| v["nodes"][1]["choices"][0]["when"] = json!({"flag": "destreza"}),
        "flag needs a boolean variable",
    );
    rejects(
        |v| {
            v["nodes"][1]["choices"][0]["when"] =
                json!({"var_cmp": {"var": "esqueleto_vencido", "op": "eq", "value": 1}})
        },
        "expected an integer variable",
    );
    rejects(
        |v| {
            v["nodes"][0]["choices"][0]["effects"][0]["roll"]["dice"]["sides"] = json!(1);
        },
        "sides must be",
    );
    rejects(
        |v| {
            v["nodes"][0]["choices"][0]["effects"][0]["roll"]["dice"]["count"] = json!(17);
        },
        "count must be",
    );
    rejects(
        |v| {
            v["nodes"][3]["choices"][0]["effects"][0] =
                json!({"cue": {"music": {"asset": "https://example.invalid/a.ogg"}}});
        },
        "reviewed asset id",
    );
    rejects(
        |v| v["nodes"][9]["choices"] = v["nodes"][0]["choices"].clone(),
        "ending node cannot offer choices",
    );
    rejects(
        |v| v["nodes"][0]["text"] = json!({"es": "x".repeat(5000)}),
        "text must contain",
    );
    rejects(|v| v["nodes"][0]["text"] = json!({}), "at least one locale");
    rejects(|v| v["id"] = json!("Con Espacios"), "invalid identifier");
    rejects(
        |v| {
            let choice = v["nodes"][2]["choices"][2].clone();
            let mut many = Vec::new();
            for index in 0..=MAX_CHOICES_PER_NODE {
                let mut copy = choice.clone();
                copy["id"] = json!(format!("c{index}"));
                many.push(copy);
            }
            v["nodes"][2]["choices"] = json!(many);
        },
        "at most 32 choices",
    );
    rejects(
        |v| {
            let mut condition = json!({"has_item": "antorcha"});
            for _ in 0..9 {
                condition = json!({ "not": condition });
            }
            v["nodes"][1]["choices"][0]["when"] = condition;
        },
        "conditions nest at most",
    );
    rejects(
        |v| {
            let mut effect = json!({"give": "antorcha"});
            for _ in 0..6 {
                effect = json!({"if": {"when": {"has_item": "antorcha"}, "then": [effect]}});
            }
            v["nodes"][1]["choices"][0]["effects"] = json!([effect]);
        },
        "if effects nest at most",
    );
    rejects(
        |v| {
            let many: Vec<_> = (0..65).map(|_| json!({"give": "antorcha"})).collect();
            v["nodes"][1]["choices"][0]["effects"] = json!(many);
        },
        "effects including nested",
    );
}

#[test]
fn parsing_rejects_unknown_fields_versions_and_oversize() {
    let mut value = fixture_value();
    value["nodes"][0]["sorpresa"] = json!(true);
    assert!(matches!(
        StoryPack::from_json_slice(value.to_string().as_bytes()),
        Err(StoryError::Malformed(_))
    ));
    let mut value = fixture_value();
    value["nodes"][0]["choices"][0]["effects"][0]["roll"]["extra"] = json!(1);
    assert!(matches!(
        StoryPack::from_json_slice(value.to_string().as_bytes()),
        Err(StoryError::Malformed(_))
    ));
    let mut value = fixture_value();
    value["variables"][0]["kind"]["int"]["step"] = json!(1);
    assert!(matches!(
        StoryPack::from_json_slice(value.to_string().as_bytes()),
        Err(StoryError::Malformed(_))
    ));
    for unsupported in [0, 4] {
        let mut value = fixture_value();
        value["format_version"] = json!(unsupported);
        assert_eq!(
            StoryPack::from_json_slice(value.to_string().as_bytes()),
            Err(StoryError::UnsupportedVersion { found: unsupported })
        );
    }
    let huge = vec![b' '; tiamud_story_core::MAX_PACK_BYTES + 1];
    assert!(matches!(
        StoryPack::from_json_slice(&huge),
        Err(StoryError::TooLarge { .. })
    ));
}

#[test]
fn lints_report_unreachable_nodes_and_dead_ends() {
    let mut value = fixture_value();
    value["nodes"]
        .as_array_mut()
        .unwrap()
        .push(json!({"id": "isla", "text": {"es": "Nadie llega aquí."}}));
    let pack = StoryPack::from_json_slice(value.to_string().as_bytes()).expect("parses");
    let report = pack.validate().expect("warnings are not errors");
    let messages: Vec<_> = report.warnings.iter().map(|w| w.message.as_str()).collect();
    assert!(messages.iter().any(|m| m.contains("unreachable")));
    assert!(messages.iter().any(|m| m.contains("dead end")));

    let mut value = fixture_value();
    for node in value["nodes"].as_array_mut().unwrap() {
        node.as_object_mut().unwrap().remove("ending");
    }
    let pack = StoryPack::from_json_slice(value.to_string().as_bytes()).expect("parses");
    let report = pack.validate().expect("warnings only");
    assert!(
        report
            .warnings
            .iter()
            .any(|w| w.message.contains("no ending is reachable"))
    );
}

#[test]
fn unvalidated_extreme_values_saturate_without_panicking() {
    let mut value = fixture_value();
    value["nodes"][0]["choices"][0]["effects"] = json!([
        {"add": {"var": "resistencia", "delta": i64::MAX}},
        {"add": {"var": "destreza", "delta": i64::MIN}},
        {"score": i64::MAX},
        {"roll": {"dice": {"count": 16, "sides": 100, "modifier": i64::MAX}, "store": "tirada"}}
    ]);
    let pack = StoryPack::from_json_slice(value.to_string().as_bytes()).expect("parses");
    assert!(
        pack.validate().is_err(),
        "validation rejects these magnitudes"
    );
    let state = StoryState::new(&pack, 0).expect("state");
    let (next, events) = step(
        &pack,
        &state,
        StoryInput::Choose {
            token: "t0c0".to_owned(),
        },
    )
    .expect("runtime still bounded");
    assert_eq!(int(&next, "resistencia"), 20);
    assert_eq!(int(&next, "destreza"), 0);
    assert_eq!(int(&next, "tirada"), 40);
    assert_eq!(next.score, tiamud_story_core::MAX_SCORE_ABS);
    assert!(
        events
            .iter()
            .any(|e| matches!(e, StoryEvent::VariableChanged { clamped: true, .. }))
    );
    next.check_against(&pack).expect("bounds hold");
}
