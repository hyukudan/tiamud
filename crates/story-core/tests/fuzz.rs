//! Deterministic, loop-based property tests. The workspace has no proptest
//! dependency, so every case is driven by a fixed-seed local generator and is
//! exactly reproducible.

use tiamud_story_core::{
    Choice, CmpOp, Condition, Cue, DiceSpec, Effect, Ending, EndingKind, ItemDecl, LocalizedText,
    Node, RollTest, StoryError, StoryInput, StoryPack, StoryState, Value, VariableDecl,
    VariableKind, step, view,
};

const FIXTURE: &[u8] = include_bytes!("../fixtures/cripta-del-nigromante.story.json");

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
fn walk(pack: &StoryPack, seed: u64, rng: &mut Gen, steps: usize) -> StoryState {
    let mut state = StoryState::new(pack, seed).expect("initial state");
    assert_bounded(pack, &state);
    for _ in 0..steps {
        let current = view(pack, &state).expect("view");
        if current.choices.is_empty() {
            break;
        }
        let token = rng.pick(&current.choices).token.clone();
        let (next, _) = step(pack, &state, StoryInput::Choose { token })
            .expect("valid pack and token never fail");
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
        match rng.below(4) {
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

fn gen_pack(rng: &mut Gen) -> StoryPack {
    let text = |s: &str| LocalizedText::single("es", s);
    let node_count = 1 + rng.below(6);
    let nodes = (0..node_count)
        .map(|index| Node {
            id: if rng.chance(3) {
                "n0".to_owned()
            } else {
                format!("n{index}")
            },
            text: text("Texto de prueba"),
            ending: rng.chance(20).then(|| Ending {
                id: format!("e{index}"),
                kind: EndingKind::Victory,
            }),
            choices: (0..rng.below(4))
                .map(|choice| Choice {
                    id: format!("c{choice}"),
                    label: text("Opción"),
                    when: rng.chance(40).then(|| gen_condition(rng, 0)),
                    effects: (0..rng.below(5)).map(|_| gen_effect(rng, 0)).collect(),
                    target: rng.chance(70).then(|| rng.pick(NODE_POOL).to_string()),
                })
                .collect(),
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
    StoryPack {
        format_version: 1,
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
    let (mut valid, mut invalid) = (0, 0);
    for _ in 0..4000 {
        let pack = gen_pack(&mut rng);
        match pack.validate() {
            Ok(_) => {
                valid += 1;
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
                    for choice in current.choices {
                        let _ = step(
                            &pack,
                            &state,
                            StoryInput::Choose {
                                token: choice.token,
                            },
                        );
                    }
                }
            }
        }
    }
    assert!(
        valid > 100 && invalid > 100,
        "valid={valid} invalid={invalid}"
    );
}
