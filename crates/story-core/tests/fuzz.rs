//! Deterministic, loop-based property tests. The workspace has no proptest
//! dependency, so every case is driven by a fixed-seed local generator and is
//! exactly reproducible.

use tiamud_story_core::{
    CharacterDecl, Choice, CmpOp, Condition, Cue, Dialogue, DialogueNext, DialogueNode,
    DialogueOption, DiceSpec, Effect, Ending, EndingKind, Hotspot, ItemDecl, LocalizedText,
    MAX_VIEW_BYTES, Menu, MenuDefault, MenuEntry, MenuVerb, Node, Presence, Response, RollTest,
    StoryError, StoryInput, StoryPack, StoryState, StoryView, TargetKind, TextBlock, Value,
    VariableDecl, VariableKind, VerbDecl, step, view, view_pages,
};

const FIXTURE: &[u8] = include_bytes!("../fixtures/crypt-of-the-necromancer.story.json");
const NOIR: &[u8] = include_bytes!("../fixtures/neon-rain.story.json");

struct Gen(u64);

impl Gen {
    fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }
    fn below(&mut self, n: usize) -> usize {
        if n == 0 {
            0
        } else {
            (self.next() % n as u64) as usize
        }
    }
    fn chance(&mut self, percent: u64) -> bool {
        self.next() % 100 < percent
    }
    fn pick<'a, T>(&mut self, items: &'a [T]) -> &'a T {
        &items[self.below(items.len())]
    }
}

/// Every state the runtime returns must satisfy the pack bounds.
fn assert_bounded(pack: &StoryPack, state: &StoryState) {
    state
        .check_against(pack)
        .expect("state stays within pack bounds");
    for decl in &pack.variables {
        if let (VariableKind::Int { min, max, .. }, Some(Value::Int(value))) =
            (&decl.kind, state.vars.get(&decl.id))
        {
            assert!(value >= min && value <= max);
        }
    }
    assert!(state.inventory.len() <= pack.items.len());
    assert!(state.visited.len() <= pack.nodes.len());
}

/// Random walk over available choices; on a validated pack every step with a
/// view token must succeed.
/// Every input the view offers: choices, then menu verb/target pairs, then
/// dialogue options.
fn actions(current: &StoryView) -> Vec<StoryInput> {
    let mut out: Vec<StoryInput> = current
        .choices
        .iter()
        .map(|choice| StoryInput::Choose {
            token: choice.token.clone(),
        })
        .collect();
    if let Some(menu) = &current.menu {
        for verb in &menu.verbs {
            if verb.needs_target {
                out.extend(verb.targets.iter().map(|target| StoryInput::MenuAction {
                    verb: verb.token.clone(),
                    target: Some(target.token.clone()),
                }));
            } else {
                out.push(StoryInput::MenuAction {
                    verb: verb.token.clone(),
                    target: None,
                });
            }
        }
    }
    if let Some(dialogue) = &current.dialogue {
        out.extend(
            dialogue
                .options
                .iter()
                .map(|option| StoryInput::DialogueChoice {
                    token: option.token.clone(),
                }),
        );
    }
    out
}

fn assert_pages_bounded(pack: &StoryPack, state: &StoryState) {
    // Locales only change which text is measured; both shipped ones are.
    for locale in ["en", "es"] {
        let pages = view_pages(pack, state, locale).expect("validated packs paginate");
        assert!(!pages.is_empty());
        for page in &pages {
            assert!(serde_json::to_vec(page).unwrap().len() <= MAX_VIEW_BYTES);
            assert_eq!(usize::from(page.pages), pages.len());
        }
    }
}

/// Random walk over available inputs; on a validated pack every step with a
/// view token must succeed.
fn walk(pack: &StoryPack, seed: u64, rng: &mut Gen, steps: usize) -> StoryState {
    let mut state = StoryState::new(pack, seed).expect("initial state");
    assert_bounded(pack, &state);
    for _ in 0..steps {
        let current = view(pack, &state).expect("view");
        let inputs = actions(&current);
        if inputs.is_empty() {
            break;
        }
        let input = rng.pick(&inputs).clone();
        let (next, _) = step(pack, &state, input).expect("valid pack and token never fail");
        assert_pages_bounded(pack, &next);
        assert_eq!(next.turn, state.turn + 1);
        assert_bounded(pack, &next);
        state = next;
    }
    state
}

#[test]
fn random_walks_on_fixture_respect_bounds_and_end() {
    let pack = StoryPack::from_json_slice(FIXTURE).unwrap();
    pack.validate().unwrap();
    let mut rng = Gen(1);
    let mut endings = std::collections::BTreeSet::new();
    for seed in 0..300 {
        let state = walk(&pack, seed, &mut rng, 400);
        if let Some(ending) = state.ended {
            endings.insert(ending.id);
        }
    }
    assert!(
        endings.len() >= 2,
        "random play reaches several endings: {endings:?}"
    );
}

#[test]
fn random_walks_on_menu_fixture_respect_bounds_pages_and_end() {
    let pack = StoryPack::from_json_slice(NOIR).unwrap();
    pack.validate().unwrap();
    let mut rng = Gen(7);
    let mut endings = std::collections::BTreeSet::new();
    for seed in 0..60 {
        let state = walk(&pack, seed, &mut rng, 150);
        if let Some(ending) = state.ended {
            endings.insert(ending.id);
        }
        if seed < 10 {
            let again = walk(&pack, seed, &mut Gen(seed), 40);
            assert_eq!(again, walk(&pack, seed, &mut Gen(seed), 40));
        }
    }
    assert_eq!(
        endings.len(),
        2,
        "random menu play reaches both endings: {endings:?}"
    );
}

#[test]
fn random_walks_are_deterministic() {
    let pack = StoryPack::from_json_slice(FIXTURE).unwrap();
    for seed in 0..50 {
        let a = walk(&pack, seed, &mut Gen(seed ^ 0xABCD), 200);
        let b = walk(&pack, seed, &mut Gen(seed ^ 0xABCD), 200);
        assert_eq!(a, b);
    }
}

#[test]
fn arbitrary_tokens_never_panic() {
    let pack = StoryPack::from_json_slice(FIXTURE).unwrap();
    let state = StoryState::new(&pack, 11).unwrap();
    let mut rng = Gen(2);
    let alphabet = b"tc0123456789-+ x\xff";
    for _ in 0..5000 {
        let len = rng.below(40);
        let bytes: Vec<u8> = (0..len).map(|_| *rng.pick(alphabet)).collect();
        let token = String::from_utf8_lossy(&bytes).into_owned();
        let valid = token == "t0c0";
        match step(&pack, &state, StoryInput::Choose { token }) {
            Ok((next, _)) => {
                assert!(valid);
                assert_bounded(&pack, &next);
            }
            Err(error) => {
                assert!(!valid);
                assert!(matches!(
                    error,
                    StoryError::MalformedToken
                        | StoryError::StaleChoice
                        | StoryError::UnknownChoice
                ));
            }
        }
    }
}

#[test]
fn mutated_pack_documents_never_panic() {
    let mut rng = Gen(3);
    let mut accepted = 0;
    for _ in 0..3000 {
        let mut bytes = FIXTURE.to_vec();
        for _ in 0..=rng.below(4) {
            match rng.below(4) {
                0 => {
                    let at = rng.below(bytes.len());
                    bytes[at] = rng.next() as u8;
                }
                1 => {
                    let at = rng.below(bytes.len());
                    bytes.truncate(at);
                }
                2 => {
                    let at = rng.below(bytes.len() + 1);
                    let byte = *rng.pick(b"{}[]\",:-0123456789 truefalsn");
                    bytes.insert(at, byte);
                }
                _ => {
                    if !bytes.is_empty() {
                        let at = rng.below(bytes.len());
                        bytes.remove(at);
                    }
                }
            }
            if bytes.is_empty() {
                break;
            }
        }
        let Ok(pack) = StoryPack::from_json_slice(&bytes) else {
            continue;
        };
        if pack.validate().is_ok() {
            accepted += 1;
            walk(&pack, rng.next(), &mut rng, 30);
        } else if let Ok(state) = StoryState::new(&pack, 0) {
            // Unvalidated packs may fail, but never panic or break bounds.
            let token = format!("t{}c0", state.turn);
            if let Ok((next, _)) = step(&pack, &state, StoryInput::Choose { token }) {
                let _ = next.check_against(&pack);
            }
        }
    }
    assert!(accepted > 0, "some benign mutations still validate");
}

#[test]
fn mutated_state_documents_never_panic() {
    let pack = StoryPack::from_json_slice(FIXTURE).unwrap();
    let base = walk(&pack, 4, &mut Gen(4), 5).to_json().unwrap();
    let mut rng = Gen(5);
    for _ in 0..3000 {
        let mut bytes = base.clone();
        let at = rng.below(bytes.len());
        bytes[at] = *rng.pick(b"0123456789-\"{}[],:tfx");
        if let Ok(state) = StoryState::from_json_slice(&bytes, &pack) {
            assert_bounded(&pack, &state);
            let _ = view(&pack, &state).expect("checked state has a view");
        }
    }
}

const NODE_POOL: &[&str] = &["n0", "n1", "n2", "n3", "n4", "n5", "missing", "Bad Id"];
const INT_POOL: &[&str] = &["fuerza", "oro", "bandera", "nada"];
const ITEM_POOL: &[&str] = &["llave", "mapa", "fantasma"];

fn gen_condition(rng: &mut Gen, depth: usize) -> Condition {
    let leaf = depth >= 10 || rng.chance(60);
    if leaf {
        match rng.below(5) {
            4 => Condition::Uses {
                node: rng.pick(NODE_POOL).to_string(),
                entry: rng.pick(ENTRY_POOL).to_string(),
                op: CmpOp::Ge,
                value: rng.below(4) as i64,
            },
            0 => Condition::VarCmp {
                var: rng.pick(INT_POOL).to_string(),
                op: *rng.pick(&[
                    CmpOp::Eq,
                    CmpOp::Ne,
                    CmpOp::Lt,
                    CmpOp::Le,
                    CmpOp::Gt,
                    CmpOp::Ge,
                ]),
                value: rng.below(21) as i64 - 10,
            },
            1 => Condition::HasItem(rng.pick(ITEM_POOL).to_string()),
            2 => Condition::Visited(rng.pick(NODE_POOL).to_string()),
            _ => Condition::Flag(rng.pick(INT_POOL).to_string()),
        }
    } else {
        match rng.below(3) {
            0 => Condition::All(
                (0..rng.below(3))
                    .map(|_| gen_condition(rng, depth + 1))
                    .collect(),
            ),
            1 => Condition::Any(
                (0..rng.below(3))
                    .map(|_| gen_condition(rng, depth + 1))
                    .collect(),
            ),
            _ => Condition::Not(Box::new(gen_condition(rng, depth + 1))),
        }
    }
}

fn gen_effect(rng: &mut Gen, depth: usize) -> Effect {
    let wild = |rng: &mut Gen| -> i64 {
        if rng.chance(5) {
            *rng.pick(&[i64::MAX, i64::MIN, 2_000_000_000])
        } else {
            rng.below(41) as i64 - 20
        }
    };
    match rng.below(if depth >= 6 { 9 } else { 10 }) {
        0 => Effect::Set {
            var: rng.pick(INT_POOL).to_string(),
            value: if rng.chance(50) {
                Value::Bool(rng.chance(50))
            } else {
                Value::Int(wild(rng))
            },
        },
        1 => Effect::Add {
            var: rng.pick(INT_POOL).to_string(),
            delta: wild(rng),
        },
        2 => Effect::Give(rng.pick(ITEM_POOL).to_string()),
        3 => Effect::Take(rng.pick(ITEM_POOL).to_string()),
        4 => Effect::Goto(rng.pick(NODE_POOL).to_string()),
        5 => Effect::Roll {
            dice: DiceSpec {
                count: rng.below(20) as u8,
                sides: rng.below(110) as u8,
                modifier: wild(rng),
            },
            modifier_var: rng.chance(50).then(|| rng.pick(INT_POOL).to_string()),
            store: rng.chance(50).then(|| rng.pick(INT_POOL).to_string()),
            test: rng.chance(50).then(|| RollTest {
                at_least: wild(rng),
                success: rng.chance(70).then(|| rng.pick(NODE_POOL).to_string()),
                failure: rng.chance(70).then(|| rng.pick(NODE_POOL).to_string()),
            }),
        },
        6 => Effect::Cue(match rng.below(3) {
            0 => Cue::Music {
                asset: "musica".to_owned(),
            },
            1 => Cue::Sfx {
                asset: if rng.chance(80) {
                    "golpe".to_owned()
                } else {
                    "http://x".to_owned()
                },
            },
            _ => Cue::StopMusic,
        }),
        7 => Effect::Score(wild(rng)),
        8 => Effect::End(Ending {
            id: "fin".to_owned(),
            kind: EndingKind::Neutral,
        }),
        _ => Effect::If {
            when: gen_condition(rng, 0),
            then: (0..rng.below(3))
                .map(|_| gen_effect(rng, depth + 1))
                .collect(),
            otherwise: (0..rng.below(3))
                .map(|_| gen_effect(rng, depth + 1))
                .collect(),
        },
    }
}

const HOTSPOT_POOL: &[&str] = &["mesa", "puerta", "nada"];
const CHARACTER_POOL: &[&str] = &["ana", "bruno", "fantasma"];
const VERB_POOL: &[&str] = &["mirar", "hablar", "usar", "pensar", "volar"];
const TARGET_POOL: &[&str] = &["mesa", "puerta", "ana", "bruno", "llave", "mapa", "nada"];
const DIALOGUE_POOL: &[&str] = &["charla", "nada"];
const DIALOGUE_NODE_POOL: &[&str] = &["d0", "d1", "d2", "dx"];
const ENTRY_POOL: &[&str] = &["e0", "e1", "ex"];

fn text(s: &str) -> LocalizedText {
    LocalizedText::single("es", s)
}

fn gen_blocks(rng: &mut Gen, min: usize) -> Vec<TextBlock> {
    (0..min + rng.below(3))
        .map(|_| {
            let speaker = rng.chance(40).then(|| {
                if rng.chance(3) {
                    "fantasma".to_owned()
                } else {
                    rng.pick(&CHARACTER_POOL[..2]).to_string()
                }
            });
            let expression = (speaker.is_some() && rng.chance(30))
                .then(|| rng.pick(&["neutral", "enfadada", "triste"]).to_string());
            TextBlock {
                speaker,
                expression,
                text: text(if rng.chance(1) { " " } else { "Texto" }),
            }
        })
        .collect()
}

fn rare_condition(rng: &mut Gen) -> Option<Condition> {
    rng.chance(10).then(|| gen_condition(rng, 0))
}

fn rare_effects(rng: &mut Gen) -> Vec<Effect> {
    if rng.chance(30) {
        vec![gen_effect(rng, 0)]
    } else {
        Vec::new()
    }
}

/// Mostly consistent menus with rare deliberate defects, so both accepted
/// and rejected v2 packs are frequent.
fn gen_menu(
    rng: &mut Gen,
    node: usize,
    node_count: usize,
    hotspots: &[Hotspot],
    present: &[Presence],
) -> Menu {
    let mut verbs: Vec<MenuVerb> = Vec::new();
    for verb in &VERB_POOL[..4] {
        if rng.chance(60) {
            verbs.push(MenuVerb {
                verb: verb.to_string(),
                when: rare_condition(rng),
            });
        }
    }
    if rng.chance(3) {
        verbs.push(MenuVerb {
            verb: "volar".to_owned(),
            when: None,
        });
    }
    if verbs.is_empty() && !rng.chance(5) {
        verbs.push(MenuVerb {
            verb: "pensar".to_owned(),
            when: None,
        });
    }
    let offered: Vec<String> = verbs.iter().map(|verb| verb.verb.clone()).collect();
    let mut entries: Vec<MenuEntry> = Vec::new();
    for index in 0..rng.below(5) {
        if offered.is_empty() {
            break;
        }
        let verb = if rng.chance(3) {
            rng.pick(VERB_POOL).to_string()
        } else {
            rng.pick(&offered).clone()
        };
        let candidates: Vec<String> = match verb.as_str() {
            "mirar" => hotspots.iter().map(|hotspot| hotspot.id.clone()).collect(),
            "hablar" => present.iter().map(|p| p.character.clone()).collect(),
            "usar" => vec!["llave".to_owned(), "mapa".to_owned()],
            _ => Vec::new(),
        };
        let target = if rng.chance(3) {
            Some(rng.pick(TARGET_POOL).to_string())
        } else if verb == "pensar" {
            None
        } else if candidates.is_empty() {
            continue;
        } else {
            Some(rng.pick(&candidates).clone())
        };
        if !rng.chance(3)
            && entries
                .iter()
                .any(|entry| entry.verb == verb && entry.target == target)
        {
            continue;
        }
        let response_count = if rng.chance(3) { 0 } else { 1 + rng.below(3) };
        let mut responses = Vec::new();
        for response in 0..response_count {
            // Usually counts this entry's own uses; rarely a missing entry.
            let entry = if rng.chance(3) {
                rng.pick(ENTRY_POOL).to_string()
            } else {
                format!("e{index}")
            };
            responses.push(Response {
                id: format!("r{response}"),
                when: rng.chance(30).then(|| Condition::Uses {
                    node: format!("n{node}"),
                    entry,
                    op: CmpOp::Lt,
                    value: rng.below(3) as i64,
                }),
                once: rng.chance(50),
                blocks: gen_blocks(rng, 1),
                effects: rare_effects(rng),
                target: rng
                    .chance(15)
                    .then(|| format!("n{}", rng.below(node_count))),
                dialogue: rng.chance(30).then(|| {
                    if rng.chance(5) {
                        "nada".to_owned()
                    } else {
                        rng.pick(&DIALOGUE_POOL[..1]).to_string()
                    }
                }),
            });
        }
        let exhausted = if rng.chance(50) {
            gen_blocks(rng, 1)
        } else {
            Vec::new()
        };
        entries.push(MenuEntry {
            id: format!("e{index}"),
            verb,
            target,
            responses,
            exhausted,
        });
    }
    let mut defaults = Vec::new();
    for verb in &offered {
        if rng.chance(30) {
            defaults.push(MenuDefault {
                verb: verb.clone(),
                blocks: gen_blocks(rng, 1),
            });
        }
    }
    Menu {
        verbs,
        entries,
        defaults,
    }
}

fn gen_dialogue(rng: &mut Gen) -> Dialogue {
    let count = 1 + rng.below(3);
    let mut nodes = Vec::new();
    for index in 0..count {
        let lines = gen_blocks(rng, 1);
        let option_count = if rng.chance(3) { 0 } else { 1 + rng.below(3) };
        let mut options = Vec::new();
        for option in 0..option_count {
            let when = rare_condition(rng);
            let once = rng.chance(50);
            let effects = rare_effects(rng);
            let next = match rng.below(3) {
                0 => DialogueNext::Goto(if rng.chance(3) {
                    rng.pick(DIALOGUE_NODE_POOL).to_string()
                } else {
                    format!("d{}", rng.below(count))
                }),
                1 => DialogueNext::Return,
                _ => DialogueNext::Exit,
            };
            options.push(DialogueOption {
                id: format!("o{option}"),
                label: text("Decir"),
                when,
                once,
                effects,
                next,
            });
        }
        nodes.push(DialogueNode {
            id: format!("d{index}"),
            lines,
            options,
        });
    }
    Dialogue {
        id: "charla".to_owned(),
        start: if rng.chance(3) {
            "dx".to_owned()
        } else {
            "d0".to_owned()
        },
        nodes,
    }
}

fn gen_pack(rng: &mut Gen) -> StoryPack {
    let node_count = 1 + rng.below(6);
    let v2 = rng.chance(60);
    let nodes = (0..node_count)
        .map(|index| {
            let mut hotspots = Vec::new();
            let mut present = Vec::new();
            if v2 {
                let count = rng.below(3);
                for slot_id in HOTSPOT_POOL.iter().take(count) {
                    let id = if rng.chance(3) {
                        rng.pick(HOTSPOT_POOL).to_string()
                    } else {
                        slot_id.to_string()
                    };
                    hotspots.push(Hotspot {
                        id,
                        label: text("Cosa"),
                        when: rare_condition(rng),
                    });
                }
                let count = rng.below(3);
                for slot_character in CHARACTER_POOL.iter().take(count) {
                    let character = if rng.chance(3) {
                        rng.pick(CHARACTER_POOL).to_string()
                    } else {
                        slot_character.to_string()
                    };
                    present.push(Presence {
                        character,
                        expression: rng.chance(30).then(|| "enfadada".to_owned()),
                        when: rare_condition(rng),
                    });
                }
            }
            let menu = (v2 && rng.chance(70))
                .then(|| gen_menu(rng, index, node_count, &hotspots, &present));
            (index, hotspots, present, menu)
        })
        .collect::<Vec<_>>()
        .into_iter()
        .map(|(index, hotspots, present, menu)| {
            // v2 scenes keep few, simple choices and rarely end, so that
            // many generated v2 packs validate.
            let simple = v2;
            Node {
                background: (v2 && rng.chance(30)).then(|| "fondo-calle".to_owned()),
                cues: if v2 && rng.chance(20) {
                    vec![Cue::Music {
                        asset: "musica".to_owned(),
                    }]
                } else {
                    Vec::new()
                },
                hotspots,
                present,
                menu,
                room: None,
                id: if rng.chance(3) {
                    "n0".to_owned()
                } else {
                    format!("n{index}")
                },
                text: text("Texto de prueba"),
                ending: rng.chance(if simple { 2 } else { 20 }).then(|| Ending {
                    id: format!("e{index}"),
                    kind: EndingKind::Victory,
                }),
                choices: (0..rng.below(if simple { 2 } else { 4 }))
                    .map(|choice| Choice {
                        id: format!("c{choice}"),
                        label: text("Opción"),
                        when: rng
                            .chance(if simple { 5 } else { 40 })
                            .then(|| gen_condition(rng, 0)),
                        effects: if simple {
                            rare_effects(rng)
                        } else {
                            (0..rng.below(5)).map(|_| gen_effect(rng, 0)).collect()
                        },
                        target: rng.chance(70).then(|| {
                            if simple {
                                format!("n{}", rng.below(node_count))
                            } else {
                                rng.pick(NODE_POOL).to_string()
                            }
                        }),
                    })
                    .collect(),
            }
        })
        .collect();
    let int_var = |id: &str, rng: &mut Gen| VariableDecl {
        id: id.to_owned(),
        label: None,
        visible: rng.chance(50),
        kind: VariableKind::Int {
            min: -(rng.below(10) as i64),
            max: rng.below(10) as i64,
            initial: if rng.chance(5) { 50 } else { 0 },
        },
    };
    let verb = |id: &str, targets: TargetKind, rng: &mut Gen| {
        let min = usize::from(rng.chance(95));
        VerbDecl {
            id: id.to_owned(),
            label: text(id),
            targets,
            default: gen_blocks(rng, min),
        }
    };
    let character = |id: &str| CharacterDecl {
        id: id.to_owned(),
        name: text(id),
        portraits: [("neutral".to_owned(), format!("retrato-{id}"))]
            .into_iter()
            .collect(),
        default_expression: None,
    };
    StoryPack {
        world: None,
        scripts: Vec::new(),
        compatible_revisions: Vec::new(),
        format_version: if v2 || rng.chance(30) { 2 } else { 1 },
        verbs: if v2 {
            vec![
                verb("mirar", TargetKind::Hotspot, rng),
                verb("hablar", TargetKind::Character, rng),
                verb("usar", TargetKind::Item, rng),
                verb("pensar", TargetKind::None, rng),
            ]
        } else {
            Vec::new()
        },
        characters: if v2 {
            vec![character("ana"), character("bruno")]
        } else {
            Vec::new()
        },
        dialogues: if v2 && rng.chance(70) {
            vec![gen_dialogue(rng)]
        } else {
            Vec::new()
        },
        id: "generada".to_owned(),
        title: text("Generada"),
        start: if rng.chance(5) {
            "missing".to_owned()
        } else {
            "n0".to_owned()
        },
        variables: vec![
            int_var("fuerza", rng),
            int_var("oro", rng),
            VariableDecl {
                id: "bandera".to_owned(),
                label: None,
                visible: false,
                kind: VariableKind::Bool { initial: false },
            },
        ],
        items: vec![
            ItemDecl {
                id: "llave".to_owned(),
                name: text("Llave"),
            },
            ItemDecl {
                id: "mapa".to_owned(),
                name: text("Mapa"),
            },
        ],
        initial_inventory: if rng.chance(10) {
            vec!["fantasma".to_owned()]
        } else {
            vec!["mapa".to_owned()]
        },
        nodes,
    }
}

#[test]
fn generated_packs_validate_soundly() {
    // Soundness: whatever validation accepts must run without any runtime
    // pack error, effect-budget error or bound violation.
    let mut rng = Gen(6);
    let (mut valid, mut invalid, mut valid_menus, mut valid_dialogues) = (0, 0, 0, 0);
    for _ in 0..4000 {
        let pack = gen_pack(&mut rng);
        match pack.validate() {
            Ok(_) => {
                valid += 1;
                if pack.nodes.iter().any(|node| node.menu.is_some()) {
                    valid_menus += 1;
                }
                if !pack.dialogues.is_empty() {
                    valid_dialogues += 1;
                }
                for _ in 0..3 {
                    let seed = rng.next();
                    walk(&pack, seed, &mut rng, 25);
                }
                // Round-trip through the JSON format preserves the pack.
                let bytes = serde_json::to_vec(&pack).unwrap();
                assert_eq!(StoryPack::from_json_slice(&bytes).unwrap(), pack);
            }
            Err(errors) => {
                invalid += 1;
                assert!(!errors.errors.is_empty());
                // Unvalidated packs may fail at runtime but must not panic.
                if let Ok(state) = StoryState::new(&pack, rng.next())
                    && let Ok(current) = view(&pack, &state)
                {
                    let _ = view_pages(&pack, &state, "es");
                    for input in actions(&current) {
                        if let Ok((next, _)) = step(&pack, &state, input) {
                            let _ = view(&pack, &next);
                            let _ = view_pages(&pack, &next, "es");
                        }
                    }
                }
            }
        }
    }
    assert!(
        valid > 100 && invalid > 100 && valid_menus > 50 && valid_dialogues > 20,
        "valid={valid} invalid={invalid} menus={valid_menus} dialogues={valid_dialogues}"
    );
}

const FARO: &[u8] = include_bytes!("../fixtures/parser/grey-point-lighthouse.story.json");

/// Words, fragments and junk a player (or an attacker) may type.
const TEXT_POOL: &[&str] = &[
    "coge",
    "deja",
    "abre",
    "cierra",
    "examina",
    "x",
    "mira",
    "i",
    "n",
    "s",
    "e",
    "o",
    "sube",
    "baja",
    "sal",
    "entra",
    "enciende",
    "apaga",
    "pon",
    "mete",
    "dale",
    "habla",
    "lee",
    "ponte",
    "quítate",
    "llena",
    "espera",
    "todo",
    "excepto",
    "la",
    "el",
    "con",
    "en",
    "a",
    "y",
    "luego",
    "lámpara",
    "llave",
    "puerta",
    "cofre",
    "medalla",
    "remedios",
    "aceitera",
    "fanal",
    "deposito",
    "barca",
    "cuaderno",
    "lente",
    "cógela",
    "ábrela",
    "examínalo",
    "segunda",
    "1",
    "2",
    "ella",
    "deshacer",
    "puntos",
    "ayuda",
    "guardar",
    ",",
    ".",
    "xyzzy",
    "ñandú",
    "\u{301}",
    "🦀",
    "\t",
    "take",
    "lamp",
    "north",
    "\u{0}",
];

/// Random free text on the parser fixture: every input on a validated pack
/// is accepted (or rejected only because the story ended), every state stays
/// within its bounds and paginates, and replaying the lines is deterministic.
#[test]
fn random_text_commands_stay_bounded_and_deterministic() {
    let pack = StoryPack::from_json_slice(FARO).unwrap();
    pack.validate().unwrap();
    let mut rng = Gen(31);
    let (mut accepted, mut ended) = (0usize, 0usize);
    for _ in 0..40 {
        let seed = rng.next();
        let mut state = StoryState::new(&pack, seed).unwrap();
        let mut lines = Vec::new();
        for _ in 0..120 {
            let words = 1 + rng.below(10);
            let mut line = String::new();
            for _ in 0..words {
                line.push_str(rng.pick::<&str>(TEXT_POOL));
                line.push(if rng.chance(15) { '.' } else { ' ' });
            }
            if rng.chance(2) {
                line = line.repeat(40);
            }
            let language = match rng.below(10) {
                0..=2 => Some("es".to_owned()),
                3 => Some("en".to_owned()),
                4 => Some("xx-YY".to_owned()),
                _ => None,
            };
            let input = StoryInput::Command {
                text: line.clone(),
                language: language.clone(),
            };
            match step(&pack, &state, input) {
                Ok((next, _)) => {
                    accepted += 1;
                    assert_bounded(&pack, &next);
                    assert_pages_bounded(&pack, &next);
                    lines.push((line, language));
                    state = next;
                }
                Err(StoryError::Ended) => {
                    ended += 1;
                    break;
                }
                Err(error) => panic!("{line:?} on a validated pack: {error}"),
            }
        }
        // Replaying the same lines in the same languages reproduces the state.
        let mut replay = StoryState::new(&pack, seed).unwrap();
        for (line, language) in &lines {
            let input = StoryInput::Command {
                text: line.clone(),
                language: language.clone(),
            };
            replay = step(&pack, &replay, input).expect("replay").0;
        }
        assert_eq!(replay, state);
    }
    assert!(accepted > 2000, "accepted={accepted} ended={ended}");
}
