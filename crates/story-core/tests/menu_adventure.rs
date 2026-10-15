//! Menu-driven adventure (format v2) scenarios over the Spanish noir sample:
//! verb menus, repetition gating, portraits, dialogue trees, opaque tokens,
//! pagination and v1 compatibility.

use serde_json::json;
use tiamud_story_core::{
    Cue, EndingKind, LastResponse, MAX_VIEW_BYTES, PageItem, STORY_PACK_FORMAT_VERSION,
    STORY_STATE_FORMAT_VERSION, StoryError, StoryEvent, StoryInput, StoryPack, StoryState,
    StoryView, Value, resolve_dialogue_option, resolve_menu, step, view, view_pages,
};

const NOIR: &[u8] = include_bytes!("../fixtures/lluvia-de-neon.story.json");
const CRIPTA: &[u8] = include_bytes!("../fixtures/cripta-del-nigromante.story.json");

fn pack() -> StoryPack {
    let pack = StoryPack::from_json_slice(NOIR).expect("fixture parses");
    let report = pack.validate().expect("fixture validates");
    assert!(report.warnings.is_empty(), "{:?}", report.warnings);
    pack
}

/// Finds the view tokens naming `verb`/`target` by authored id.
fn menu_input(
    pack: &StoryPack,
    state: &StoryState,
    verb: &str,
    target: Option<&str>,
) -> Option<StoryInput> {
    let current = view(pack, state).expect("view");
    for offered in current.menu?.verbs {
        let candidates: Vec<Option<String>> = if offered.needs_target {
            offered.targets.into_iter().map(|t| Some(t.token)).collect()
        } else {
            vec![None]
        };
        for candidate in candidates {
            let selection = resolve_menu(pack, state, &offered.token, candidate.as_deref())
                .expect("view tokens resolve");
            if selection.verb == verb && selection.target == target {
                return Some(StoryInput::MenuAction {
                    verb: offered.token.clone(),
                    target: candidate,
                });
            }
        }
    }
    None
}

fn act(
    pack: &StoryPack,
    state: &StoryState,
    verb: &str,
    target: Option<&str>,
) -> (StoryState, Vec<StoryEvent>) {
    let input = menu_input(pack, state, verb, target)
        .unwrap_or_else(|| panic!("{verb} {target:?} is not offered"));
    step(pack, state, input).expect("menu step")
}

fn say(pack: &StoryPack, state: &StoryState, option: &str) -> (StoryState, Vec<StoryEvent>) {
    let current = view(pack, state).expect("view");
    let token = current
        .dialogue
        .expect("dialogue is active")
        .options
        .into_iter()
        .map(|choice| choice.token)
        .find(|token| {
            resolve_dialogue_option(pack, state, token).map(|o| o.id.as_str()) == Ok(option)
        })
        .unwrap_or_else(|| panic!("option {option} unavailable"));
    step(pack, state, StoryInput::DialogueChoice { token }).expect("dialogue step")
}

fn texts(current: &StoryView) -> Vec<String> {
    current
        .blocks
        .iter()
        .map(|block| block.text.resolve("es").unwrap_or_default().to_owned())
        .collect()
}

fn flag(state: &StoryState, var: &str) -> bool {
    state.vars[var] == Value::Bool(true)
}

/// Walks the sample to the alley with every clue gathered.
fn reach_alley(pack: &StoryPack) -> StoryState {
    let state = StoryState::new(pack, 7).unwrap();
    let (state, _) = act(pack, &state, "investigar", Some("mesa"));
    let (state, _) = act(pack, &state, "usar", Some("tarjeta"));
    let (state, _) = act(pack, &state, "moverse", Some("puerta"));
    let (state, _) = act(pack, &state, "hablar", Some("lola"));
    let (state, _) = say(pack, &state, "pedir-cafe");
    let (state, _) = say(pack, &state, "seguir");
    let (state, _) = say(pack, &state, "despedirse");
    let (state, _) = act(pack, &state, "mostrar", Some("tarjeta"));
    let (state, _) = act(pack, &state, "investigar", Some("reservado"));
    let (state, _) = act(pack, &state, "moverse", Some("callejon"));
    assert_eq!(state.node, "callejon");
    state
}

#[test]
fn investigation_repeats_advance_only_through_counters_and_flags() {
    let pack = pack();
    let state = StoryState::new(&pack, 1).unwrap();
    let first = view(&pack, &state).unwrap();
    assert_eq!(first.background.as_deref(), Some("fondo-oficina"));
    assert_eq!(
        first.cues,
        vec![Cue::Music {
            asset: "musica-lluvia".to_owned()
        }]
    );
    let menu = first.menu.expect("menu scene");
    let labels: Vec<_> = menu
        .verbs
        .iter()
        .map(|verb| verb.label.resolve("es").unwrap())
        .collect();
    assert_eq!(labels, ["MIRAR", "INVESTIGAR", "USAR", "MOVERSE"]);
    assert!(menu.verbs[2].needs_target && menu.verbs[2].targets.is_empty());
    assert!(first.blocks.is_empty());

    // The way out is gated by a flag, not by repetition.
    let (state, events) = act(&pack, &state, "moverse", Some("puerta"));
    assert_eq!(state.node, "oficina");
    assert!(texts(&view(&pack, &state).unwrap())[0].starts_with("No sabes"));
    assert!(
        !events
            .iter()
            .any(|e| matches!(e, StoryEvent::NodeEntered { .. }))
    );

    // First look, a new detail on the second, then "nothing new".
    let (state, _) = act(&pack, &state, "investigar", Some("ventana"));
    assert!(texts(&view(&pack, &state).unwrap())[0].contains("destello"));
    assert!(!flag(&state, "dron_visto"));
    let (state, _) = act(&pack, &state, "investigar", Some("ventana"));
    assert!(flag(&state, "dron_visto"));
    let (state, _) = act(&pack, &state, "investigar", Some("ventana"));
    assert!(matches!(state.last, Some(LastResponse::Exhausted { .. })));
    assert!(texts(&view(&pack, &state).unwrap())[0].contains("Nada nuevo"));
    assert_eq!(state.menu_uses["oficina/investigar-ventana"], 3);

    // An untargeted hotspot falls back to the verb default.
    let (state, _) = act(&pack, &state, "mirar", Some("mesa"));
    assert!(matches!(state.last, Some(LastResponse::VerbDefault { .. })));

    let (state, events) = act(&pack, &state, "investigar", Some("mesa"));
    assert!(events.contains(&StoryEvent::ItemGained {
        item: "tarjeta".to_owned()
    }));
    let (state, _) = act(&pack, &state, "investigar", Some("mesa"));
    assert!(texts(&view(&pack, &state).unwrap())[0].contains("No hay nada nuevo"));
    assert!(!state.inventory.is_empty());

    let (state, events) = act(&pack, &state, "moverse", Some("puerta"));
    assert_eq!(state.node, "club");
    assert!(events.contains(&StoryEvent::Cue(Cue::Music {
        asset: "musica-club".to_owned()
    })));
    let club = view(&pack, &state).unwrap();
    assert_eq!(club.portraits.len(), 1);
    assert_eq!(
        club.portraits[0].asset.as_deref(),
        Some("retrato-lola-neutral")
    );
    // The response that moved the player is still shown on arrival.
    assert!(texts(&club)[0].contains("gabardina"));
}

#[test]
fn dialogue_tree_with_once_options_return_and_exit() {
    let pack = pack();
    let state = StoryState::new(&pack, 2).unwrap();
    let (state, _) = act(&pack, &state, "investigar", Some("mesa"));
    let (state, _) = act(&pack, &state, "moverse", Some("puerta"));
    let (state, _) = act(&pack, &state, "mostrar", Some("tarjeta"));
    assert!(texts(&view(&pack, &state).unwrap())[0].contains("Primero"));

    let (state, events) = act(&pack, &state, "hablar", Some("lola"));
    assert!(events.contains(&StoryEvent::DialogueEntered {
        dialogue: "charla-lola".to_owned(),
        node: "saludo".to_owned()
    }));
    let talking = view(&pack, &state).unwrap();
    assert!(talking.menu.is_none() && talking.choices.is_empty());
    assert_eq!(talking.dialogue.as_ref().unwrap().options.len(), 3);
    // Response block, then the dialogue line with its speaker.
    assert_eq!(talking.blocks.len(), 2);
    assert_eq!(
        talking.blocks[1].speaker.as_ref().unwrap().resolve("es"),
        Some("Lola")
    );

    // Menu input is refused while the dialogue waits for an answer.
    let stale_menu = StoryInput::MenuAction {
        verb: format!("t{}v0", state.turn),
        target: Some(format!("t{}v0o0", state.turn)),
    };
    assert_eq!(step(&pack, &state, stale_menu), Err(StoryError::InDialogue));

    let (state, _) = say(&pack, &state, "preguntar-iker");
    let iker = view(&pack, &state).unwrap();
    assert_eq!(
        iker.blocks[0].portrait.as_deref(),
        Some("retrato-lola-nerviosa")
    );
    let (state, _) = say(&pack, &state, "volver");
    // The `once` question is gone after returning to the start node.
    assert_eq!(
        view(&pack, &state).unwrap().dialogue.unwrap().options.len(),
        2
    );
    let (state, _) = say(&pack, &state, "pedir-cafe");
    assert!(flag(&state, "lola_confia"));
    let (state, _) = say(&pack, &state, "seguir");
    let options = view(&pack, &state).unwrap().dialogue.unwrap().options;
    assert_eq!(options.len(), 1, "only the goodbye remains");
    let (state, events) = say(&pack, &state, "despedirse");
    assert!(events.contains(&StoryEvent::DialogueExited {
        dialogue: "charla-lola".to_owned()
    }));
    assert!(state.dialogue.is_none());
    assert!(view(&pack, &state).unwrap().menu.is_some());

    // Out of a dialogue, a dialogue token is refused.
    assert_eq!(
        step(
            &pack,
            &state,
            StoryInput::DialogueChoice {
                token: format!("t{}d0", state.turn)
            }
        ),
        Err(StoryError::NotInDialogue)
    );

    let (state, _) = act(&pack, &state, "mostrar", Some("tarjeta"));
    assert!(flag(&state, "sabe_damaso"));
    let (state, _) = act(&pack, &state, "mostrar", Some("tarjeta"));
    assert!(matches!(state.last, Some(LastResponse::Exhausted { .. })));
}

#[test]
fn accusation_with_evidence_wins_and_without_loses() {
    let pack = pack();
    let alley = reach_alley(&pack);
    assert!(alley.inventory.contains("mechero"));
    let (state, _) = act(&pack, &alley, "hablar", Some("damaso"));
    let options = view(&pack, &state).unwrap().dialogue.unwrap().options;
    assert_eq!(options.len(), 3, "the bluff is hidden when evidence exists");
    let (won, events) = say(&pack, &state, "acusar");
    assert_eq!(won.ended.as_ref().unwrap().kind, EndingKind::Victory);
    assert_eq!(won.score, 100);
    assert!(won.dialogue.is_none());
    assert!(events.contains(&StoryEvent::Cue(Cue::Music {
        asset: "musica-final".to_owned()
    })));
    let end = view(&pack, &won).unwrap();
    assert!(end.menu.is_none() && end.dialogue.is_none() && end.choices.is_empty());

    // Same alley without watching the video: only the bluff is possible.
    let state = StoryState::new(&pack, 3).unwrap();
    let (state, _) = act(&pack, &state, "investigar", Some("mesa"));
    let (state, _) = act(&pack, &state, "moverse", Some("puerta"));
    let (state, _) = act(&pack, &state, "hablar", Some("lola"));
    let (state, _) = say(&pack, &state, "pedir-cafe");
    let (state, _) = say(&pack, &state, "seguir");
    let (state, _) = say(&pack, &state, "despedirse");
    let (state, _) = act(&pack, &state, "mostrar", Some("tarjeta"));
    let (state, _) = act(&pack, &state, "moverse", Some("callejon"));
    let (state, _) = act(&pack, &state, "hablar", Some("damaso"));
    let (lost, _) = say(&pack, &state, "farol");
    assert_eq!(lost.ended.as_ref().unwrap().kind, EndingKind::Defeat);
}

#[test]
fn menu_tokens_are_opaque_bound_to_turn_and_verb() {
    let pack = pack();
    let state = StoryState::new(&pack, 4).unwrap();
    let current = view(&pack, &state).unwrap();
    let menu = current.menu.unwrap();
    let mirar = &menu.verbs[0];
    // Tokens never carry authored ids or labels.
    for token in std::iter::once(&mirar.token).chain(mirar.targets.iter().map(|t| &t.token)) {
        assert!(!token.contains("mirar") && !token.contains("mesa"));
    }
    let missing = StoryInput::MenuAction {
        verb: mirar.token.clone(),
        target: None,
    };
    assert_eq!(step(&pack, &state, missing), Err(StoryError::BadTarget));
    let foreign = StoryInput::MenuAction {
        verb: menu.verbs[1].token.clone(),
        target: Some(mirar.targets[0].token.clone()),
    };
    assert_eq!(step(&pack, &state, foreign), Err(StoryError::BadTarget));
    let unknown = StoryInput::MenuAction {
        verb: mirar.token.clone(),
        target: Some(format!("t{}v0o99", state.turn)),
    };
    assert_eq!(step(&pack, &state, unknown), Err(StoryError::UnknownChoice));
    let wrong_kind = StoryInput::MenuAction {
        verb: format!("t{}c0", state.turn),
        target: None,
    };
    assert_eq!(
        step(&pack, &state, wrong_kind),
        Err(StoryError::MalformedToken)
    );

    let good = StoryInput::MenuAction {
        verb: mirar.token.clone(),
        target: Some(mirar.targets[1].token.clone()),
    };
    let (next, _) = step(&pack, &state, good.clone()).unwrap();
    assert_eq!(next.turn, 1);
    // The same tokens are stale one turn later.
    assert_eq!(step(&pack, &next, good), Err(StoryError::StaleChoice));
}

#[test]
fn menu_play_is_deterministic_and_resumable_from_json() {
    let pack = pack();
    let a = reach_alley(&pack);
    let b = reach_alley(&pack);
    assert_eq!(a, b);
    let bytes = a.to_json().unwrap();
    let restored = StoryState::from_json_slice(&bytes, &pack).unwrap();
    assert_eq!(restored, a);
    assert_eq!(view(&pack, &restored).unwrap(), view(&pack, &a).unwrap());

    // Tampered counters, spent sets and positions are rejected.
    let raw: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    let tampered = [
        ("menu_uses", json!({ "oficina/no-existe": 1 })),
        ("menu_uses", json!({ "oficina/investigar-mesa": 10_000 })),
        (
            "spent_responses",
            json!(["oficina/investigar-mesa/fantasma"]),
        ),
        ("spent_options", json!(["charla-lola/saludo/a/b"])),
        (
            "dialogue",
            json!({ "dialogue": "charla-lola", "node": "nada" }),
        ),
        ("last", json!({ "verb_default": { "verb": "volar" } })),
    ];
    for (field, value) in tampered {
        let mut raw = raw.clone();
        raw[field] = value;
        assert!(
            matches!(
                StoryState::from_json_slice(raw.to_string().as_bytes(), &pack),
                Err(StoryError::StateMismatch(_))
            ),
            "{field} tampering must be rejected"
        );
    }
}

#[test]
fn v1_packs_and_states_still_load_and_upgrade() {
    let mut cripta = StoryPack::from_json_slice(CRIPTA).unwrap();
    assert_eq!(cripta.format_version, 1);
    cripta.validate().unwrap();
    let state = StoryState::new(&cripta, 5).unwrap();
    let mut raw: serde_json::Value = serde_json::from_slice(&state.to_json().unwrap()).unwrap();
    raw["format_version"] = json!(1);
    let loaded = StoryState::from_json_slice(raw.to_string().as_bytes(), &cripta).unwrap();
    assert_eq!(loaded.format_version, STORY_STATE_FORMAT_VERSION);
    raw["menu_uses"] = json!({ "x/y": 1 });
    assert!(matches!(
        StoryState::from_json_slice(raw.to_string().as_bytes(), &cripta),
        Err(StoryError::StateMismatch(_))
    ));

    assert!(cripta.upgrade());
    assert_eq!(cripta.format_version, STORY_PACK_FORMAT_VERSION);
    assert!(!cripta.upgrade());
    cripta.validate().unwrap();

    // A document that still says v1 may not use v2 features.
    let mut value: serde_json::Value = serde_json::from_slice(CRIPTA).unwrap();
    value["nodes"][0]["background"] = json!("fondo-cripta");
    let pack = StoryPack::from_json_slice(value.to_string().as_bytes()).unwrap();
    let errors = pack.validate().unwrap_err();
    assert!(errors.to_string().contains("format v2 feature"));
}

type Mutation = Box<dyn Fn(&mut serde_json::Value)>;

#[test]
fn validation_rejects_inconsistent_menus_and_dialogues() {
    let base: serde_json::Value = serde_json::from_slice(NOIR).unwrap();
    let cases: Vec<(Mutation, &str)> = vec![
        (
            Box::new(|v| v["nodes"][0]["menu"]["entries"][1]["target"] = json!("sofa")),
            "not a hotspot of this node",
        ),
        (
            Box::new(|v| v["nodes"][1]["menu"]["entries"][1]["target"] = json!("damaso")),
            "not a character present",
        ),
        (
            Box::new(|v| v["nodes"][0]["menu"]["entries"][4]["target"] = json!("pistola")),
            "undeclared item",
        ),
        (
            Box::new(|v| v["nodes"][2]["menu"]["entries"][0]["verb"] = json!("mirar")),
            "not offered by this menu",
        ),
        (
            Box::new(|v| v["nodes"][0]["menu"]["verbs"][0]["verb"] = json!("volar")),
            "undeclared verb",
        ),
        (
            Box::new(|v| v["nodes"][0]["menu"]["entries"][2]["target"] = json!("mesa")),
            "already answers this verb and target",
        ),
        (
            Box::new(|v| {
                v["nodes"][1]["menu"]["entries"][1]["responses"][0]["dialogue"] = json!("nada")
            }),
            "unknown dialogue",
        ),
        (
            Box::new(|v| v["dialogues"][0]["start"] = json!("nada")),
            "unknown dialogue node",
        ),
        (
            Box::new(|v| {
                v["dialogues"][0]["nodes"][1]["options"][0]["next"] = json!({"goto": "nada"})
            }),
            "unknown dialogue node",
        ),
        (
            Box::new(|v| v["dialogues"][0]["nodes"][1]["options"] = json!([])),
            "expected 1..=16 options",
        ),
        (
            Box::new(|v| v["dialogues"][0]["nodes"][0]["lines"][0]["speaker"] = json!("nadie")),
            "undeclared character",
        ),
        (
            Box::new(|v| v["characters"][0]["portraits"]["neutral"] = json!("https://x/y.png")),
            "reviewed asset id",
        ),
        (
            Box::new(|v| v["nodes"][0]["background"] = json!("../fondo.png")),
            "reviewed asset id",
        ),
        (
            Box::new(|v| {
                v["nodes"][0]["menu"]["entries"][2]["responses"][0]["when"] =
                    json!({"uses": {"node": "oficina", "entry": "nada", "op": "eq", "value": 0}})
            }),
            "unknown menu entry",
        ),
        (
            Box::new(|v| v["nodes"][3]["menu"] = json!({"verbs": [{"verb": "mirar"}]})),
            "ending node cannot offer a menu",
        ),
        (
            Box::new(|v| v["nodes"][0]["menu"]["entries"][0]["responses"] = json!([])),
            "expected 1..=8 responses",
        ),
    ];
    for (mutate, needle) in cases {
        let mut value = base.clone();
        mutate(&mut value);
        let pack = StoryPack::from_json_slice(value.to_string().as_bytes()).expect("parses");
        let errors = pack.validate().expect_err(needle);
        assert!(
            errors.errors.iter().any(|e| e.message.contains(needle)),
            "expected {needle:?} in {errors}"
        );
    }
}

#[test]
fn large_views_paginate_under_the_frame_budget_without_truncation() {
    let mut value: serde_json::Value = serde_json::from_slice(NOIR).unwrap();
    // Worst-case escaping: quotes double in JSON.
    let long = "\"".repeat(4096);
    let block = json!({ "speaker": "lola", "text": { "es": long } });
    value["nodes"][0]["text"] = json!({ "es": "\\".repeat(4096) });
    value["nodes"][0]["menu"]["entries"][0]["responses"][0]["blocks"] =
        json!(vec![block.clone(); 8]);
    let hotspots: Vec<_> = (0..32)
        .map(|index| json!({ "id": format!("h{index}"), "label": { "es": "é".repeat(128) } }))
        .collect();
    value["nodes"][0]["hotspots"] = json!(hotspots);
    value["nodes"][0]["menu"]["entries"] = json!([value["nodes"][0]["menu"]["entries"][0].clone()]);
    value["nodes"][0]["menu"]["entries"][0]["target"] = json!("h0");
    let pack = StoryPack::from_json_slice(value.to_string().as_bytes()).unwrap();
    pack.validate().expect("still a valid pack");

    let state = StoryState::new(&pack, 0).unwrap();
    let (state, _) = act(&pack, &state, "mirar", Some("h0"));
    let pages = view_pages(&pack, &state, "es").unwrap();
    assert!(pages.len() > 8, "{} pages", pages.len());
    let mut blocks = 0;
    let mut targets = 0;
    for (index, page) in pages.iter().enumerate() {
        assert!(serde_json::to_vec(page).unwrap().len() <= MAX_VIEW_BYTES);
        assert_eq!(usize::from(page.page), index + 1);
        assert_eq!(usize::from(page.pages), pages.len());
        assert_eq!(page.turn, state.turn);
        for item in &page.items {
            match item {
                PageItem::Block { text, .. } => {
                    assert_eq!(text.len(), 4096, "blocks are never truncated");
                    blocks += 1;
                }
                PageItem::Target { .. } => targets += 1,
                _ => {}
            }
        }
    }
    assert_eq!(blocks, 8);
    // Every verb with hotspot targets lists all 32 hotspots.
    assert_eq!(targets, 3 * 32);
    assert!(matches!(pages[0].items[0], PageItem::Scene { .. }));
}
