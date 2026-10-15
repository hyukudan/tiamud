//! Script hook points (ADR 0034) exercised with in-process Rust hosts, so the
//! core contract is tested without any Lua dependency.

use std::cell::RefCell;

use serde_json::json;
use tiamud_story_core::{
    Effect, Ending, EndingKind, MAX_SCRIPT_CALLS_PER_STEP, ScriptCall, ScriptFailure, ScriptHost,
    StoryError, StoryInput, StoryPack, StoryState, Value, step, step_scripted, view, view_scripted,
};

fn pack_from(value: serde_json::Value) -> StoryPack {
    StoryPack::from_json_slice(&serde_json::to_vec(&value).unwrap()).unwrap()
}

/// Gamebook with one effect script and one condition script.
fn duel() -> serde_json::Value {
    json!({
        "format_version": 3,
        "id": "duelo",
        "title": { "es": "Duelo" },
        "start": "inicio",
        "variables": [
            { "id": "vigor", "kind": { "int": { "min": 0, "max": 10, "initial": 6 } } },
            { "id": "oculto", "kind": { "int": { "min": 0, "max": 10, "initial": 0 } } },
            { "id": "alerta", "kind": { "bool": { "initial": false } } }
        ],
        "items": [{ "id": "espada", "name": { "es": "Espada" } }],
        "scripts": [
            { "id": "asalto", "kind": "effect", "source": "function run() end",
              "writes": ["vigor", "alerta"], "items": ["espada"], "goto": ["victoria"],
              "score": true, "ends": true },
            { "id": "listo", "kind": "condition", "source": "function check() return true end" }
        ],
        "nodes": [
            {
                "id": "inicio", "text": { "es": "Inicio." },
                "choices": [
                    { "id": "atacar", "label": { "es": "Atacar" },
                      "effects": [{ "script": { "id": "asalto", "args": { "golpe": 2 } } }],
                      "target": "inicio" },
                    { "id": "esperar", "label": { "es": "Esperar" },
                      "when": { "script": { "id": "listo" } }, "target": "inicio" }
                ]
            },
            { "id": "victoria", "text": { "es": "Ganas." },
              "ending": { "id": "victoria", "kind": "victory" } }
        ]
    })
}

/// Host that returns a fixed intent list and records every call.
struct Fixed {
    effects: Vec<Effect>,
    answer: bool,
    seeds: RefCell<Vec<(String, u64)>>,
}

impl Fixed {
    fn new(effects: Vec<Effect>) -> Self {
        Self {
            effects,
            answer: true,
            seeds: RefCell::new(Vec::new()),
        }
    }
}

impl ScriptHost for Fixed {
    fn check(&self, call: &ScriptCall<'_>) -> Result<bool, ScriptFailure> {
        self.seeds
            .borrow_mut()
            .push((call.script.id.clone(), call.seed));
        Ok(self.answer)
    }

    fn run(&self, call: &ScriptCall<'_>) -> Result<Vec<Effect>, ScriptFailure> {
        assert_eq!(call.args.get("golpe"), Some(&Value::Int(2)));
        assert_eq!(call.snapshot.vars.get("vigor"), Some(&Value::Int(6)));
        self.seeds
            .borrow_mut()
            .push((call.script.id.clone(), call.seed));
        Ok(self.effects.clone())
    }
}

struct Failing;

impl ScriptHost for Failing {
    fn check(&self, _: &ScriptCall<'_>) -> Result<bool, ScriptFailure> {
        Err(ScriptFailure::new("listo:1: boom"))
    }

    fn run(&self, _: &ScriptCall<'_>) -> Result<Vec<Effect>, ScriptFailure> {
        Err(ScriptFailure::new("asalto:3: attempt to index a nil value"))
    }
}

fn attack(pack: &StoryPack, host: &dyn ScriptHost) -> Result<StoryState, StoryError> {
    let state = StoryState::new(pack, 42).unwrap();
    let before = state.clone();
    let token = view_scripted(pack, &state, host)?.choices[0].token.clone();
    let result = step_scripted(pack, &state, StoryInput::Choose { token }, host);
    assert_eq!(state, before);
    result.map(|(next, _)| next)
}

#[test]
fn plain_validation_rejects_scripts_and_scripted_validation_accepts_them() {
    let pack = pack_from(duel());
    let errors = pack.validate().unwrap_err();
    assert!(errors.to_string().contains("no script host"), "{errors}");
    let report = pack.validate_scripted().unwrap();
    // The `goto` grant makes the victory node reachable for the lints.
    assert!(report.warnings.is_empty(), "{:?}", report.warnings);
}

#[test]
fn script_declarations_and_references_are_checked() {
    let cases: Vec<(serde_json::Value, &str)> = vec![
        (json!({ "/scripts/0/writes": ["nada"] }), "unknown variable"),
        (json!({ "/scripts/0/items": ["nada"] }), "undeclared item"),
        (json!({ "/scripts/0/goto": ["nada"] }), "unknown node"),
        (json!({ "/scripts/0/objects": ["nada"] }), "unknown object"),
        (
            json!({ "/scripts/0/writes": ["vigor", "vigor"] }),
            "duplicate grant",
        ),
        (json!({ "/scripts/1/score": true }), "take no grants"),
        (json!({ "/scripts/1/id": "asalto" }), "duplicate script id"),
        (json!({ "/scripts/0/source": " " }), "source is empty"),
        (json!({ "/scripts/0/source": "a\u{0}b" }), "NUL"),
        (
            json!({ "/scripts/0/source": "x".repeat(64 * 1024 + 1) }),
            "source exceeds",
        ),
        (json!({ "/scripts/0/id": "Mal" }), "invalid identifier"),
        (
            json!({ "/nodes/0/choices/0/effects/0/script/id": "listo" }),
            "condition script cannot",
        ),
        (
            json!({ "/nodes/0/choices/1/when/script/id": "asalto" }),
            "effect script cannot",
        ),
        (
            json!({ "/nodes/0/choices/1/when/script/id": "nada" }),
            "unknown script",
        ),
        (
            json!({ "/nodes/0/choices/0/effects/0/script/args": {
                "a": 1, "b": 1, "c": 1, "d": 1, "e": 1, "f": 1, "g": 1, "h": 1, "i": 1 } }),
            "at most 8 arguments",
        ),
        (
            json!({ "/nodes/0/choices/0/effects/0/script/args": { "golpe": 2_000_000_000_i64 } }),
            "out of range",
        ),
        (json!({ "/format_version": 2 }), "format v3 feature"),
    ];
    for (patch, expected) in cases {
        let mut document = duel();
        for (pointer, value) in patch.as_object().unwrap() {
            let (parent, key) = pointer.rsplit_once('/').unwrap();
            let parent = document.pointer_mut(parent).unwrap();
            match parent {
                serde_json::Value::Object(map) => {
                    map.insert(key.to_owned(), value.clone());
                }
                serde_json::Value::Array(list) => {
                    list[key.parse::<usize>().unwrap()] = value.clone();
                }
                _ => panic!("{pointer}"),
            }
        }
        let errors = pack_from(document).validate_scripted().unwrap_err();
        assert!(
            errors.to_string().contains(expected),
            "{patch}: expected {expected:?} in {errors}"
        );
    }
    let mut many = duel();
    let script = many["scripts"][1].clone();
    let scripts = many["scripts"].as_array_mut().unwrap();
    for index in 0..40 {
        let mut copy = script.clone();
        copy["id"] = json!(format!("extra-{index}"));
        scripts.push(copy);
    }
    assert!(
        pack_from(many)
            .validate_scripted()
            .unwrap_err()
            .to_string()
            .contains("at most 32 scripts")
    );
}

#[test]
fn granted_intents_apply_like_authored_effects() {
    let pack = pack_from(duel());
    let host = Fixed::new(vec![
        Effect::Add {
            var: "vigor".to_owned(),
            delta: 9,
        },
        Effect::Set {
            var: "alerta".to_owned(),
            value: Value::Bool(true),
        },
        Effect::Give("espada".to_owned()),
        Effect::Score(7),
        Effect::Goto("victoria".to_owned()),
    ]);
    let after = attack(&pack, &host).unwrap();
    assert_eq!(after.vars["vigor"], Value::Int(10), "clamped like `add`");
    assert_eq!(after.vars["alerta"], Value::Bool(true));
    assert!(after.inventory.contains("espada"));
    assert_eq!(after.score, 7);
    assert_eq!(after.node, "victoria");
    assert_eq!(after.ended.unwrap().id, "victoria");
}

#[test]
fn ungranted_or_unsafe_intents_fail_the_step_closed() {
    let pack = pack_from(duel());
    let rejected = [
        Effect::Add {
            var: "oculto".to_owned(),
            delta: 1,
        },
        Effect::Set {
            var: "vigor".to_owned(),
            value: Value::Bool(true),
        },
        Effect::Add {
            var: "vigor".to_owned(),
            delta: i64::MAX,
        },
        Effect::Goto("inicio".to_owned()),
        Effect::Take("nada".to_owned()),
        Effect::Score(10_000_000),
        Effect::End(Ending {
            id: "No Valido".to_owned(),
            kind: EndingKind::Victory,
        }),
        Effect::Cue(tiamud_story_core::Cue::StopMusic),
        Effect::Script(tiamud_story_core::ScriptRef {
            id: "asalto".to_owned(),
            args: Default::default(),
        }),
        Effect::If {
            when: tiamud_story_core::Condition::Flag("alerta".to_owned()),
            then: Vec::new(),
            otherwise: Vec::new(),
        },
    ];
    for effect in rejected {
        let host = Fixed::new(vec![Effect::Score(1), effect.clone()]);
        match attack(&pack, &host) {
            Err(StoryError::Script { script, message }) => {
                assert_eq!(script, "asalto");
                assert!(message.starts_with("effect 2:"), "{message}");
            }
            other => panic!("{effect:?}: {other:?}"),
        }
    }
    let flood = Fixed::new(vec![Effect::Score(1); 33]);
    assert!(matches!(
        attack(&pack, &flood),
        Err(StoryError::Script { .. })
    ));
}

#[test]
fn host_failures_and_missing_hosts_fail_closed() {
    let pack = pack_from(duel());
    let state = StoryState::new(&pack, 1).unwrap();
    assert_eq!(view(&pack, &state), Err(StoryError::ScriptUnavailable));
    match view_scripted(&pack, &state, &Failing) {
        Err(StoryError::Script { script, message }) => {
            assert_eq!(script, "listo");
            assert_eq!(message, "listo:1: boom");
        }
        other => panic!("{other:?}"),
    }
    let host = Fixed::new(Vec::new());
    let token = view_scripted(&pack, &state, &host).unwrap().choices[0]
        .token
        .clone();
    assert_eq!(
        step(
            &pack,
            &state,
            StoryInput::Choose {
                token: token.clone()
            }
        ),
        Err(StoryError::ScriptUnavailable)
    );
    struct RunFails;
    impl ScriptHost for RunFails {
        fn check(&self, _: &ScriptCall<'_>) -> Result<bool, ScriptFailure> {
            Ok(true)
        }
        fn run(&self, _: &ScriptCall<'_>) -> Result<Vec<Effect>, ScriptFailure> {
            Err(ScriptFailure::new("asalto:3: attempt to index a nil value"))
        }
    }
    let error = step_scripted(&pack, &state, StoryInput::Choose { token }, &RunFails).unwrap_err();
    assert_eq!(
        error.to_string(),
        "script \"asalto\" failed: asalto:3: attempt to index a nil value"
    );
}

#[test]
fn seeds_are_deterministic_and_conditions_agree_between_view_and_step() {
    let pack = pack_from(duel());
    let host = Fixed::new(Vec::new());
    let state = StoryState::new(&pack, 9).unwrap();
    let token = view_scripted(&pack, &state, &host).unwrap().choices[1]
        .token
        .clone();
    let (after, _) = step_scripted(&pack, &state, StoryInput::Choose { token }, &host).unwrap();
    let seeds = host.seeds.borrow().clone();
    // One condition evaluation in the view, the same one in the step.
    assert_eq!(seeds.len(), 2);
    assert_eq!(seeds[0], seeds[1]);
    assert_eq!(
        after.rng, state.rng,
        "condition scripts never advance the RNG"
    );

    let host = Fixed::new(Vec::new());
    let token = view_scripted(&pack, &state, &host).unwrap().choices[0]
        .token
        .clone();
    let (after, _) = step_scripted(&pack, &state, StoryInput::Choose { token }, &host).unwrap();
    assert_ne!(
        after.rng, state.rng,
        "effect scripts draw their seed from the RNG"
    );
    let again = Fixed::new(Vec::new());
    let token = view_scripted(&pack, &state, &again).unwrap().choices[0]
        .token
        .clone();
    let (replayed, _) = step_scripted(&pack, &state, StoryInput::Choose { token }, &again).unwrap();
    assert_eq!(after, replayed);
    assert_eq!(*host.seeds.borrow(), *again.seeds.borrow());
}

#[test]
fn the_script_budget_bounds_one_view() {
    let mut document = duel();
    let condition = json!({ "script": { "id": "listo" } });
    let all: Vec<_> = (0..=MAX_SCRIPT_CALLS_PER_STEP)
        .map(|_| condition.clone())
        .collect();
    document["nodes"][0]["choices"][1]["when"] = json!({ "all": all });
    let pack = pack_from(document);
    // `all` holds at most 64 condition nodes; 17 scripts fit statically.
    pack.validate_scripted().unwrap();
    let state = StoryState::new(&pack, 1).unwrap();
    assert_eq!(
        view_scripted(&pack, &state, &Fixed::new(Vec::new())),
        Err(StoryError::ScriptBudget {
            limit: MAX_SCRIPT_CALLS_PER_STEP
        })
    );
}

/// Bounded fuzz: a host that emits random intents (granted or not, valid or
/// not) never makes `step_scripted` panic; every success revalidates against
/// the pack and every failure is a typed error.
#[test]
fn fuzz_random_intents_never_corrupt_state() {
    let pack = pack_from(duel());
    let mut generator = 0x0dd_ba11_u64;
    let mut next = move || {
        generator ^= generator << 13;
        generator ^= generator >> 7;
        generator ^= generator << 17;
        generator
    };
    let vars = ["vigor", "oculto", "alerta", "nada"];
    let nodes = ["inicio", "victoria", "nada"];
    let items = ["espada", "nada"];
    for _ in 0..500 {
        let count = usize::try_from(next() % 40).unwrap();
        let effects: Vec<Effect> = (0..count)
            .map(|_| {
                let pick = |list: &[&str], roll: u64| {
                    list[usize::try_from(roll % list.len() as u64).unwrap()].to_owned()
                };
                let raw = next();
                let number = i64::from_ne_bytes(next().to_ne_bytes()) >> (next() % 64);
                match raw % 8 {
                    0 => Effect::Add {
                        var: pick(&vars, next()),
                        delta: number,
                    },
                    1 => Effect::Set {
                        var: pick(&vars, next()),
                        value: if next() % 2 == 0 {
                            Value::Int(number)
                        } else {
                            Value::Bool(true)
                        },
                    },
                    2 => Effect::Give(pick(&items, next())),
                    3 => Effect::Take(pick(&items, next())),
                    4 => Effect::Goto(pick(&nodes, next())),
                    5 => Effect::Score(number),
                    6 => Effect::End(Ending {
                        id: pick(&nodes, next()),
                        kind: EndingKind::Neutral,
                    }),
                    _ => Effect::Cue(tiamud_story_core::Cue::StopMusic),
                }
            })
            .collect();
        let host = Fixed::new(effects);
        match attack(&pack, &host) {
            Ok(after) => after.check_against(&pack).unwrap(),
            Err(
                StoryError::Script { .. }
                | StoryError::EffectBudget
                | StoryError::ScriptBudget { .. },
            ) => {}
            Err(other) => panic!("unexpected error {other:?}"),
        }
    }
}
