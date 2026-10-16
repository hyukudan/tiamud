//! Format v3 parser adventures: the reviewed Spanish fixture played through
//! free text, an English pack, compatibility with v1/v2 and bounds.

use tiamud_story_core::{
    EndingKind, Location, MAX_COMPATIBLE_REVISIONS, MAX_PROMPT_BYTES, MAX_REPLIES, MAX_VIEW_BYTES,
    MetaRequest, PageItem, Reply, StoryError, StoryInput, StoryPack, StoryState, StoryView, step,
    step_command, view, view_pages,
};

const FARO: &[u8] = include_bytes!("../fixtures/parser/faro-de-punta-gris.story.json");
const CRIPTA: &[u8] = include_bytes!("../fixtures/cripta-del-nigromante.story.json");
const NOIR: &[u8] = include_bytes!("../fixtures/lluvia-de-neon.story.json");

/// The reviewed walkthrough of the fixture, one line per input.
const WALKTHROUGH: &[&str] = &[
    "examina la barca",
    "coge la medalla",
    "n",
    "habla con remedios",
    "dale la medalla a remedios",
    "e",
    "abre el cofre",
    "coge la lámpara y enciéndela",
    "o",
    "abre la puerta con la llave",
    "abre la puerta. n",
    "lee el cuaderno",
    "sube",
    "coge la aceitera",
    "sube",
    "llena el depósito con la aceitera",
    "enciende el fanal",
];

fn faro() -> StoryPack {
    let pack = StoryPack::from_json_slice(FARO).expect("fixture parses");
    let report = pack.validate().expect("fixture validates");
    assert!(report.warnings.is_empty(), "{:?}", report.warnings);
    pack
}

fn blocks(current: &StoryView, locale: &str) -> String {
    current
        .blocks
        .iter()
        .map(|block| block.text.resolve(locale).unwrap_or_default().to_owned())
        .collect::<Vec<_>>()
        .join("\n")
}

fn command(text: &str) -> StoryInput {
    StoryInput::Command {
        text: text.to_owned(),
        language: None,
    }
}

/// Applies one line and returns the next state and its rendered transcript.
fn say(pack: &StoryPack, state: &StoryState, text: &str) -> (StoryState, String) {
    let (next, _) =
        step(pack, state, command(text)).unwrap_or_else(|error| panic!("{text:?}: {error}"));
    next.check_against(pack).expect("bounded state");
    let current = view(pack, &next).expect("view");
    (next, blocks(&current, "es"))
}

fn play(pack: &StoryPack, lines: &[&str]) -> StoryState {
    let mut state = StoryState::new(pack, 7).expect("initial state");
    for line in lines {
        state = say(pack, &state, line).0;
    }
    state
}

fn at(state: &StoryState, obj: &str) -> Location {
    state.objects[obj].at.clone()
}

#[test]
fn walkthrough_lights_the_lighthouse_with_full_score() {
    let pack = faro();
    let start = StoryState::new(&pack, 7).unwrap();
    let opening = view(&pack, &start).unwrap();
    assert_eq!(opening.text.resolve("es"), Some("Playa de guijarros"));
    assert!(blocks(&opening, "es").starts_with("La tormenta te sorprendió"));
    assert!(
        opening
            .prompt
            .as_ref()
            .is_some_and(|prompt| !prompt.question)
    );

    let state = play(&pack, WALKTHROUGH);
    let ending = state.ended.clone().expect("the story ended");
    assert_eq!(ending.id, "faro-encendido");
    assert_eq!(ending.kind, EndingKind::Victory);
    assert_eq!(state.node, "amanecer");
    assert_eq!(state.score, 30);
    // Each executed command is a turn: the two-command lines count twice.
    assert_eq!(state.turn, 19);
    let last = view(&pack, &state).unwrap();
    assert!(last.prompt.is_none());
    assert!(
        last.text
            .resolve("es")
            .unwrap()
            .starts_with("La mecha prende")
    );
    assert_eq!(step(&pack, &state, command("mira")), Err(StoryError::Ended));
}

#[test]
fn replies_follow_the_library_and_the_rules() {
    let pack = faro();
    let state = StoryState::new(&pack, 1).unwrap();
    let (state, text) = say(&pack, &state, "examina la barca");
    assert!(text.contains("Dentro de la barca varada ves una medalla de plata."));
    let (state, text) = say(&pack, &state, "coge la medalla y póntela");
    assert_eq!(
        text,
        "Coges la medalla de plata.\nTe pones la medalla de plata.\nLa plata está fría. No es tuya, y lo sabes."
    );
    let (state, text) = say(&pack, &state, "i");
    assert_eq!(text, "Llevas puesto: una medalla de plata.");
    let (state, text) = say(&pack, &state, "deja la medalla");
    assert_eq!(text, "Primero tendrás que quitarte la medalla de plata.");
    let (state, text) = say(&pack, &state, "quítate la medalla");
    assert_eq!(text, "Te quitas la medalla de plata.");
    let (state, text) = say(&pack, &state, "oeste");
    assert_eq!(text, "Por ahí solo hay roca y mar.", "overridden message");
    let (state, text) = say(&pack, &state, "n");
    assert!(text.ends_with("Puedes ver Remedios."));
    let (state, text) = say(&pack, &state, "n");
    assert_eq!(text, "La puerta del faro está cerrada.");
    let (state, text) = say(&pack, &state, "abre la puerta");
    assert_eq!(text, "La puerta del faro está cerrada con llave.");
    let (state, text) = say(&pack, &state, "enseña la medalla a remedios");
    assert!(text.starts_with("—¡Mi medalla!"));
    let (state, text) = say(&pack, &state, "coge la pescadora");
    assert_eq!(text, "Remedios no se deja coger.");
    let (state, text) = say(&pack, &state, "dale la medalla a la pescadora");
    assert!(text.starts_with("Remedios coge la medalla"));
    assert_eq!(at(&state, "llave"), Location::Carried);
    assert_eq!(at(&state, "medalla"), Location::Nowhere);
    let (_, text) = say(&pack, &state, "habla con la anciana");
    assert!(text.starts_with("—Sube y enciende el fanal"));
}

#[test]
fn darkness_hides_things_until_the_lamp_is_lit() {
    let pack = faro();
    let base = play(
        &pack,
        &[
            "examina la barca",
            "coge la medalla",
            "n",
            "dale la medalla a remedios",
            "e",
            "abre el cofre",
            "coge la lámpara",
            "o",
            "abre la puerta con la llave",
            "abre la puerta",
            "n",
        ],
    );
    let (dark, text) = say(&pack, &base, "sube");
    assert_eq!(text, "Está oscuro como boca de lobo. No ves nada.");
    assert_eq!(
        view(&pack, &dark).unwrap().text.resolve("es"),
        Some("Oscuridad")
    );
    let (dark, text) = say(&pack, &dark, "coge la aceitera");
    assert_eq!(text, "No ves eso por aquí.");
    let (dark, text) = say(&pack, &dark, "sube");
    assert!(text.starts_with("Tanteas el primer peldaño"));
    assert_eq!(dark.node, "escalera");
    let (lit, text) = say(&pack, &dark, "enciende la lámpara");
    assert!(text.starts_with("Enciendes la lámpara de aceite.\nEscalera de caracol"));
    assert!(text.ends_with("Puedes ver una aceitera de latón."));
    let (dark_again, text) = say(&pack, &lit, "apaga la lámpara");
    assert_eq!(text, "Apagas la lámpara de aceite.\nTodo queda a oscuras.");
    let (back, _) = say(&pack, &dark_again, "baja");
    assert_eq!(back.node, "vestibulo");
}

#[test]
fn questions_survive_serialization_and_accept_answers() {
    let pack = faro();
    let mut state = play(&pack, &WALKTHROUGH[..15]);
    assert_eq!(state.node, "linterna");
    let (asked, text) = say(&pack, &state, "enciende la lampara");
    assert_eq!(
        text,
        "¿Cuál quieres decir: la lámpara de aceite o el fanal?"
    );
    assert_eq!(asked.turn, state.turn, "a question does not take a turn");
    assert!(asked.pending.is_some());
    let shown = view(&pack, &asked).unwrap();
    assert!(shown.prompt.as_ref().is_some_and(|prompt| prompt.question));
    // A saved and restored state answers the same question.
    let restored = StoryState::from_json_slice(&asked.to_json().unwrap(), &pack).unwrap();
    assert_eq!(restored, asked);
    let (answered, text) = say(&pack, &restored, "el fanal");
    assert!(text.ends_with("El fanal está seco."));
    assert!(answered.pending.is_none());
    // Answering with an ordinal works too.
    let (_, text) = say(&pack, &asked, "segunda");
    assert!(text.ends_with("El fanal está seco."));
    // A new command instead of an answer drops the question.
    let (moved, _) = say(&pack, &asked, "baja");
    assert_eq!(moved.node, "escalera");
    assert!(moved.pending.is_none());
    // Missing objects are asked for, then filled, possibly twice.
    state = say(&pack, &state, "pon").0;
    assert!(state.pending.is_some());
    let (asked_again, text) = say(&pack, &state, "la aceitera");
    assert_eq!(text, "¿En qué quieres poner la aceitera de latón?");
    assert_eq!(asked_again.pending.as_ref().unwrap().answers.len(), 1);
    let restored = StoryState::from_json_slice(&asked_again.to_json().unwrap(), &pack).unwrap();
    let (filled, text) = say(&pack, &restored, "en el deposito");
    assert!(text.starts_with("Vacías la aceitera"));
    assert_eq!(filled.score, state.score + 10);
    // An extension verb with only its object gets the rule for that key.
    let (_, text) = say(&pack, &state, "llena el depósito");
    assert_eq!(text, "¿Con qué? Necesitas aceite.");
}

#[test]
fn lines_stop_at_the_first_failure_and_use_fresh_scope() {
    let pack = faro();
    let state = play(&pack, &["n", "e"]);
    let (state, text) = say(&pack, &state, "coge el cofre y abre el cofre");
    assert_eq!(text, "No puedes llevarte el cofre de marinero.");
    assert!(!state.objects["cofre"].open);
    // The lamp only becomes visible after the chest opens in the same line.
    let (state, text) = say(&pack, &state, "abre el cofre, coge la lámpara");
    assert_eq!(
        text,
        "Abres el cofre de marinero.\nDentro del cofre de marinero ves una lámpara de aceite.\nCoges la lámpara de aceite."
    );
    assert_eq!(at(&state, "lampara"), Location::Carried);
    let (state, text) = say(&pack, &state, "xyzzy");
    assert_eq!(text, "No conozco la palabra 'xyzzy'.");
    let (state, text) = say(&pack, &state, "examínala");
    assert!(text.starts_with("Una lámpara de aceite de las de antes"));
    let (state, text) = say(&pack, &state, "mete la lámpara en el cofre");
    assert_eq!(text, "Metes la lámpara de aceite en el cofre de marinero.");
    let (state, text) = say(&pack, &state, "cierra el cofre y coge la lámpara");
    assert_eq!(text, "Cierras el cofre de marinero.\nNo ves eso por aquí.");
    let (_, text) = say(&pack, &state, "coge todo");
    assert_eq!(text, "No hay nada que coger.");
}

#[test]
fn meta_commands_are_reported_to_the_host() {
    let pack = faro();
    let state = play(&pack, &["coge la medalla", "examina la barca"]);
    let stepped = step_command(&pack, &state, "puntos", None).unwrap();
    assert_eq!(
        stepped.meta,
        vec![MetaRequest {
            action: "score".to_owned()
        }]
    );
    assert_eq!(stepped.state.turn, state.turn);
    assert_eq!(
        blocks(&view(&pack, &stepped.state).unwrap(), "es"),
        "Llevas 0 de 30 puntos en 2 turnos."
    );
    for (line, action) in [
        ("deshacer", "undo"),
        ("guardar", "save"),
        ("cargar", "restore"),
        ("fin", "quit"),
        ("reiniciar", "restart"),
        ("ayuda", "help"),
    ] {
        let stepped = step_command(&pack, &state, line, None).unwrap();
        assert_eq!(stepped.meta.len(), 1, "{line}");
        assert_eq!(stepped.meta[0].action, action, "{line}");
        assert_eq!(stepped.state.turn, state.turn, "{line}");
    }
    // A meta command ends the line; earlier commands still ran.
    let stepped = step_command(&pack, &state, "mira. deshacer. n", None).unwrap();
    assert_eq!(stepped.meta[0].action, "undo");
    assert_eq!(stepped.state.node, "playa");
    assert_eq!(stepped.state.turn, state.turn + 1);
}

#[test]
fn identical_inputs_produce_identical_states_and_events() {
    let pack = faro();
    let run = || {
        let mut state = StoryState::new(&pack, 99).unwrap();
        let mut all_events = Vec::new();
        for line in WALKTHROUGH {
            let (next, events) = step(&pack, &state, command(line)).unwrap();
            all_events.push(events);
            state = next;
        }
        (state, all_events)
    };
    assert_eq!(run(), run());
    // Serializing mid-game and continuing gives the same result.
    let mut state = play(&pack, &WALKTHROUGH[..8]);
    state = StoryState::from_json_slice(&state.to_json().unwrap(), &pack).unwrap();
    for line in &WALKTHROUGH[8..] {
        state = say(&pack, &state, line).0;
    }
    assert_eq!(state, play(&pack, WALKTHROUGH));
}

#[test]
fn hostile_lines_stay_bounded() {
    let pack = faro();
    let state = StoryState::new(&pack, 3).unwrap();
    let long = "coge ".repeat(200);
    let (next, text) = say(&pack, &state, &long);
    assert_eq!(text, "Esa orden es demasiado larga.");
    assert_eq!(next.turn, 0);
    let (_, text) = say(&pack, &state, "mira. mira. mira. mira. mira");
    assert_eq!(text, "Demasiadas órdenes a la vez; prueba de una en una.");
    let (_, text) = say(&pack, &state, "\u{7}\u{0}mira\r");
    assert!(text.starts_with("Playa de guijarros"));
    let (_, text) = say(&pack, &state, "");
    assert_eq!(text, "¿Perdón?");
    // Pages stay under the frame budget even with long replies.
    let state = play(&pack, &["mira. examina la barca. coge la medalla. i"]);
    for page in view_pages(&pack, &state, "es").unwrap() {
        assert!(serde_json::to_vec(&page).unwrap().len() <= MAX_VIEW_BYTES);
    }
    assert!(state.transcript.len() <= MAX_REPLIES);
}

#[test]
fn corrupted_parser_state_is_rejected() {
    let pack = faro();
    let state = play(&pack, &["coge la medalla"]);
    let mut bad = state.clone();
    bad.pronoun = Some("fantasma".to_owned());
    assert!(bad.check_against(&pack).is_err());
    let mut bad = state.clone();
    bad.objects.get_mut("lampara").unwrap().at = Location::In("lampara".to_owned());
    assert!(bad.check_against(&pack).is_err());
    let mut bad = state.clone();
    bad.objects.get_mut("cofre").unwrap().at = Location::In("barca".to_owned());
    bad.objects.get_mut("barca").unwrap().at = Location::In("cofre".to_owned());
    assert!(bad.check_against(&pack).is_err());
    let mut bad = state.clone();
    bad.objects.get_mut("redes").unwrap().worn = true;
    assert!(bad.check_against(&pack).is_err());
    let mut bad = state.clone();
    bad.transcript.push(Reply::Rule {
        rule: "inventada".to_owned(),
    });
    assert!(bad.check_against(&pack).is_err());
    let mut bad = state.clone();
    bad.objects.remove("lente");
    assert!(bad.check_against(&pack).is_err());
    let mut bad = state;
    bad.format_version = 2;
    assert!(bad.check_against(&pack).is_err(), "v2 state with v3 fields");
}

#[test]
fn v1_and_v2_packs_have_no_parser_and_keep_their_states() {
    for bytes in [CRIPTA, NOIR] {
        let pack = StoryPack::from_json_slice(bytes).unwrap();
        pack.validate().unwrap();
        let state = StoryState::new(&pack, 1).unwrap();
        assert!(state.objects.is_empty() && state.transcript.is_empty());
        assert_eq!(
            step(&pack, &state, command("mira")),
            Err(StoryError::NoParser)
        );
        assert!(view(&pack, &state).unwrap().prompt.is_none());
        // States written by the previous format version still load.
        let mut json: serde_json::Value =
            serde_json::from_slice(&state.to_json().unwrap()).unwrap();
        json["format_version"] = serde_json::json!(2);
        let old = StoryState::from_json_slice(&serde_json::to_vec(&json).unwrap(), &pack).unwrap();
        assert_eq!(
            old.format_version,
            tiamud_story_core::STORY_STATE_FORMAT_VERSION
        );
    }
}

#[test]
fn world_features_need_format_v3() {
    let mut json: serde_json::Value = serde_json::from_slice(FARO).unwrap();
    json["format_version"] = serde_json::json!(2);
    let pack = StoryPack::from_json_slice(&serde_json::to_vec(&json).unwrap()).unwrap();
    let errors = pack.validate().unwrap_err();
    assert!(errors.errors.iter().any(|issue| issue.path == "world"));
    assert!(
        errors
            .errors
            .iter()
            .any(|issue| issue.path == "nodes[0].room")
    );
    let mut upgraded = pack.clone();
    assert!(upgraded.upgrade());
    upgraded.validate().unwrap();
}

fn english_pack() -> StoryPack {
    let json = serde_json::json!({
        "format_version": 3,
        "id": "cellar",
        "title": { "en": "The Cellar" },
        "start": "kitchen",
        "nodes": [
            {
                "id": "kitchen",
                "text": { "en": "A cold kitchen. Stairs lead down." },
                "room": {
                    "name": { "en": "Kitchen" },
                    "exits": [{ "direction": "down", "to": "cellar", "door": "trapdoor" }]
                }
            },
            {
                "id": "cellar",
                "text": { "en": "A damp cellar full of empty barrels." },
                "room": {
                    "name": { "en": "Cellar" },
                    "dark": true,
                    "exits": [{ "direction": "up", "to": "kitchen", "door": "trapdoor" }]
                }
            }
        ],
        "world": {
            "lexicons": [{ "id": "en" }],
            "objects": [
                {
                    "id": "trapdoor", "name": { "en": "trapdoor" },
                    "nouns": { "en": ["trapdoor", "hatch"] },
                    "description": { "en": "A wooden trapdoor in the floor." },
                    "location": { "room": "kitchen" }, "found_in": ["cellar"],
                    "scenery": true, "openable": true, "locked": true, "key": "brass-key"
                },
                {
                    "id": "box", "name": { "en": "tin box" },
                    "nouns": { "en": ["box"] }, "adjectives": { "en": ["tin"] },
                    "description": { "en": "A dented tin box." },
                    "location": { "room": "kitchen" },
                    "container": true, "openable": true, "portable": true
                },
                {
                    "id": "brass-key", "name": { "en": "brass key" },
                    "nouns": { "en": ["key"] }, "adjectives": { "en": ["brass"] },
                    "description": { "en": "A small brass key." },
                    "location": { "in": "box" }, "portable": true
                },
                {
                    "id": "iron-key", "name": { "en": "iron key" },
                    "nouns": { "en": ["key"] }, "adjectives": { "en": ["iron"] },
                    "description": { "en": "A heavy iron key." },
                    "location": { "room": "kitchen" }, "portable": true
                },
                {
                    "id": "candle", "name": { "en": "candle" },
                    "nouns": { "en": ["candle"] },
                    "description": { "en": "A stub of tallow candle." },
                    "location": "carried", "portable": true, "lightable": true
                },
                {
                    "id": "wine", "name": { "en": "old wine" },
                    "nouns": { "en": ["wine", "bottle"] },
                    "description": { "en": "A dusty bottle." },
                    "location": { "room": "cellar" }, "portable": true
                }
            ],
            "rules": [
                {
                    "id": "drink-wine", "phase": "instead", "action": "drink", "obj": "wine",
                    "effects": [{ "end": { "id": "tipsy", "kind": "neutral" } }],
                    "text": { "en": "You drink the whole bottle and fall asleep." }
                }
            ]
        }
    });
    let pack = StoryPack::from_json_slice(&serde_json::to_vec(&json).unwrap()).unwrap();
    let report = pack.validate().expect("english pack validates");
    assert!(report.warnings.is_empty(), "{:?}", report.warnings);
    pack
}

fn say_en(pack: &StoryPack, state: &StoryState, text: &str) -> (StoryState, String) {
    let (next, _) = step(
        pack,
        state,
        StoryInput::Command {
            text: text.to_owned(),
            language: Some("en-GB".to_owned()),
        },
    )
    .unwrap_or_else(|error| panic!("{text:?}: {error}"));
    next.check_against(pack).unwrap();
    let text = blocks(&view(pack, &next).unwrap(), "en");
    (next, text)
}

#[test]
fn english_lexicon_plays_the_same_library() {
    let pack = english_pack();
    let state = StoryState::new(&pack, 0).unwrap();
    assert_eq!(
        blocks(&view(&pack, &state).unwrap(), "en"),
        "Kitchen\nA cold kitchen. Stairs lead down.\nYou can see a tin box and an iron key."
    );
    let (state, text) = say_en(&pack, &state, "take the key");
    assert_eq!(text, "You take the iron key.", "only one key is visible");
    let (state, text) = say_en(&pack, &state, "open the box then take the brass key");
    assert_eq!(
        text,
        "You open the tin box.\nIn the tin box you see a brass key.\nYou take the brass key."
    );
    let (state, text) = say_en(&pack, &state, "unlock the hatch with the key");
    assert_eq!(text, "Which do you mean, the brass key or the iron key?");
    let (state, text) = say_en(&pack, &state, "iron");
    assert_eq!(text, "The iron key doesn't fit the lock of the trapdoor.");
    let (state, text) = say_en(
        &pack,
        &state,
        "unlock the hatch with the brass key. open it",
    );
    assert_eq!(
        text,
        "You unlock the trapdoor with the brass key.\nYou open the trapdoor."
    );
    let (state, text) = say_en(&pack, &state, "go down");
    assert_eq!(text, "It is pitch dark. You can't see a thing.");
    let (state, text) = say_en(&pack, &state, "light the candle");
    assert!(text.starts_with("You light the candle.\nCellar\n"));
    assert!(text.ends_with("You can see an old wine."));
    let (state, text) = say_en(&pack, &state, "drop all");
    assert_eq!(
        text,
        "You drop the brass key.\nYou drop the iron key.\nYou drop the candle."
    );
    let (state, text) = say_en(&pack, &state, "inventory");
    assert_eq!(text, "You are carrying nothing.");
    let (state, _) = say_en(&pack, &state, "get the candle");
    let (state, text) = say_en(&pack, &state, "drink wine");
    assert_eq!(text, "You drink the whole bottle and fall asleep.");
    assert_eq!(state.ended.as_ref().map(|e| e.id.as_str()), Some("tipsy"));
}

fn scene_prompt(pack: &StoryPack, state: &StoryState, locale: &str) -> Option<String> {
    view_pages(pack, state, locale).unwrap()[0]
        .items
        .iter()
        .find_map(|item| match item {
            PageItem::Scene { prompt, .. } => prompt.clone(),
            _ => None,
        })
}

#[test]
fn authored_prompt_is_localized_and_bounded() {
    let pack = faro();
    let state = StoryState::new(&pack, 0).unwrap();
    // Without an authored prompt the parser shows `>`.
    assert_eq!(scene_prompt(&pack, &state, "es").as_deref(), Some(">"));

    let mut json: serde_json::Value = serde_json::from_slice(FARO).unwrap();
    json["world"]["prompt"] = serde_json::json!({ "es": "¿Qué haces?", "en": "What now?" });
    let pack = StoryPack::from_json_slice(&serde_json::to_vec(&json).unwrap()).unwrap();
    pack.validate().unwrap();
    assert_eq!(
        scene_prompt(&pack, &state, "en").as_deref(),
        Some("What now?")
    );
    assert_eq!(
        scene_prompt(&pack, &state, "es").as_deref(),
        Some("¿Qué haces?")
    );

    // Too long, multi-line or empty prompts fail validation.
    for bad in [
        serde_json::json!({ "es": "x".repeat(MAX_PROMPT_BYTES + 1) }),
        serde_json::json!({ "es": "uno\ndos" }),
        serde_json::json!({}),
    ] {
        json["world"]["prompt"] = bad;
        let pack = StoryPack::from_json_slice(&serde_json::to_vec(&json).unwrap()).unwrap();
        let errors = pack.validate().unwrap_err();
        assert!(
            errors
                .errors
                .iter()
                .any(|issue| issue.path == "world.prompt")
        );
    }
}

#[test]
fn compatible_revisions_are_bounded_unique_digests_of_format_v3() {
    let digest = "ab".repeat(32);
    let mut json: serde_json::Value = serde_json::from_slice(FARO).unwrap();
    json["compatible_revisions"] = serde_json::json!([digest]);
    let pack = StoryPack::from_json_slice(&serde_json::to_vec(&json).unwrap()).unwrap();
    pack.validate().unwrap();
    assert_eq!(pack.compatible_revisions, vec![digest.clone()]);

    let reject = |value: serde_json::Value, version: u32| {
        let mut json: serde_json::Value = serde_json::from_slice(CRIPTA).unwrap();
        json["format_version"] = serde_json::json!(version);
        json["compatible_revisions"] = value;
        let pack = StoryPack::from_json_slice(&serde_json::to_vec(&json).unwrap()).unwrap();
        let errors = pack.validate().unwrap_err();
        assert!(
            errors
                .errors
                .iter()
                .any(|issue| issue.path.starts_with("compatible_revisions")),
            "{errors:?}"
        );
    };
    reject(serde_json::json!(["AB".repeat(32)]), 3);
    reject(serde_json::json!(["ab".repeat(31)]), 3);
    reject(serde_json::json!([digest, digest]), 3);
    reject(
        serde_json::json!(
            (0..=MAX_COMPATIBLE_REVISIONS)
                .map(|index| format!("{index:064x}"))
                .collect::<Vec<_>>()
        ),
        3,
    );
    // A v1 document may not declare it.
    reject(serde_json::json!([digest]), 1);
}
