//! Deterministic, loop-based fuzz/property tests. The workspace has no
//! proptest dependency; every case is driven by a fixed-seed local generator
//! and is exactly reproducible.

mod support;

use tiamud_story_parser::{
    MAX_CANDIDATES, MAX_COMMANDS, MAX_EXCEPT, MAX_INPUT_BYTES, ParseOutcome, Parser, Pending,
    Scope, ScopeObject, SimpleScope,
};

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
    fn pick<'a, T>(&mut self, items: &'a [T]) -> &'a T {
        &items[self.below(items.len())]
    }
}

fn scope() -> SimpleScope {
    SimpleScope::new(vec![
        ScopeObject::new(
            "llave-dorada",
            "la llave dorada",
            &["llave", "key"],
            &["dorada", "golden"],
        ),
        ScopeObject::new(
            "llave-hierro",
            "la llave de hierro",
            &["llave", "key"],
            &["hierro", "iron"],
        ),
        ScopeObject::new("caja", "la caja", &["caja", "box"], &["madera"]),
        ScopeObject::new("guardia", "el guardia", &["guardia", "troll"], &[]),
        ScopeObject::new("moneda", "una moneda", &["moneda", "coin"], &["plata"]),
        ScopeObject::new("muñeco", "el muñeco", &["muñeco"], &[]),
    ])
    .with_last("moneda")
}

/// Every outcome list respects the documented bounds and only names objects
/// from the scope.
fn assert_bounded(outcomes: &[ParseOutcome], scope: &SimpleScope) {
    assert!(!outcomes.is_empty() && outcomes.len() <= MAX_COMMANDS);
    let ids: Vec<&str> = scope.objects().iter().map(|o| o.id.as_str()).collect();
    for (position, outcome) in outcomes.iter().enumerate() {
        if position + 1 < outcomes.len() {
            assert!(
                matches!(outcome, ParseOutcome::Command(_)),
                "only the last outcome may stop the line"
            );
        }
        match outcome {
            ParseOutcome::Command(command) => {
                for id in command
                    .obj
                    .iter()
                    .chain(&command.obj2)
                    .chain(&command.except)
                {
                    assert!(ids.contains(&id.as_str()), "{id} not in scope");
                }
                assert!(command.except.len() <= MAX_EXCEPT);
                assert!(command.except.is_empty() || command.all);
                assert!(!(command.all && command.obj.is_some()));
            }
            ParseOutcome::Ambiguous {
                candidates,
                question,
                pending,
            } => {
                assert!((2..=MAX_CANDIDATES).contains(&candidates.len()));
                assert!(question.len() <= 4096);
                assert_eq!(pending.candidates().len(), candidates.len());
            }
            ParseOutcome::Incomplete { question, .. } => assert!(question.len() <= 1024),
            ParseOutcome::Error(error) => assert!(error.message.len() <= 1024),
        }
    }
}

fn last_pending(outcomes: &[ParseOutcome]) -> Option<Pending> {
    match outcomes.last()? {
        ParseOutcome::Ambiguous { pending, .. } | ParseOutcome::Incomplete { pending, .. } => {
            Some(pending.clone())
        }
        _ => None,
    }
}

#[test]
fn random_bytes_never_panic_and_stay_bounded() {
    let parsers = [Parser::spanish().unwrap(), Parser::english().unwrap()];
    let scope = scope();
    let mut rng = Gen(0x5EED);
    for round in 0..4000 {
        let len = rng.below(MAX_INPUT_BYTES + 64);
        let bytes: Vec<u8> = (0..len).map(|_| rng.next() as u8).collect();
        let parser = &parsers[round % 2];
        let first = parser.parse_line_bytes(&bytes, &scope);
        assert_bounded(&first, &scope);
        assert_eq!(
            first,
            parser.parse_line_bytes(&bytes, &scope),
            "deterministic"
        );
        let lossy = String::from_utf8_lossy(&bytes);
        assert_bounded(&parser.parse_line(&lossy, &scope), &scope);
    }
}

#[test]
fn random_word_soup_never_panics_and_answers_stay_bounded() {
    const WORDS: &[&str] = &[
        "coge", "cógelo", "ábrela", "dáselo", "dale", "háblale", "abre", "pon", "mete", "mira",
        "habla", "ve", "n", "este", "o", "y", "luego", "todo", "excepto", "menos", "la", "el",
        "al", "del", "de", "en", "con", "a", "dentro", "debajo", "llave", "dorada", "hierro",
        "caja", "moneda", "guardia", "muñeco", "muneco", "ella", "lo", "xyzzy", "1", "2", "9",
        "segunda", "take", "it", "all", "but", "and", "then", "put", "in", "on", "with", "key",
        "golden", "box", "troll", "the", "pick", "up", "go", "north", ",", ".", ";", "!", "¿", "?",
        "ñ", "\u{303}", "LLAVE", "Ábrela", "guardar", "i", "x", "ex", "dentro", "sobre",
    ];
    let parsers = [Parser::spanish().unwrap(), Parser::english().unwrap()];
    let scope = scope();
    let mut rng = Gen(42);
    for round in 0..6000 {
        let parser = &parsers[round % 2];
        let count = 1 + rng.below(20);
        let line: Vec<&str> = (0..count).map(|_| *rng.pick(WORDS)).collect();
        let line = line.join(" ");
        let outcomes = parser.parse_line(&line, &scope);
        assert_bounded(&outcomes, &scope);
        assert_eq!(outcomes, parser.parse_line(&line, &scope), "deterministic");
        // Answer any question with more random soup, up to a bounded depth.
        let mut pending = last_pending(&outcomes);
        for _ in 0..4 {
            let Some(current) = pending.take() else { break };
            let count = 1 + rng.below(6);
            let reply: Vec<&str> = (0..count).map(|_| *rng.pick(WORDS)).collect();
            let reply = reply.join(" ");
            let answered = parser.answer(&current, &reply, &scope);
            assert_bounded(&answered, &scope);
            assert_eq!(answered, parser.answer(&current, &reply, &scope));
            // A pending from one lexicon never panics the other.
            let other = &parsers[(round + 1) % 2];
            assert_bounded(&other.answer(&current, &reply, &scope), &scope);
            pending = last_pending(&answered);
        }
    }
}

#[test]
fn random_lexicon_mutations_are_rejected_or_parse_safely() {
    let base: serde_json::Value =
        serde_json::from_str(Parser::builtin_lexicon_json("es").unwrap()).unwrap();
    let mut rng = Gen(7);
    let scope = scope();
    let junk = [
        serde_json::json!(null),
        serde_json::json!(""),
        serde_json::json!("x y"),
        serde_json::json!([]),
        serde_json::json!(["a"]),
        serde_json::json!({}),
        serde_json::json!(true),
        serde_json::json!(99),
        serde_json::json!("lo"),
        serde_json::json!(["n", "coge"]),
    ];
    let keys = [
        "verbs",
        "prepositions",
        "directions",
        "articles",
        "fillers",
        "pronouns",
        "enclitics",
        "contractions",
        "phrases",
        "all_words",
        "except_words",
        "and_words",
        "then_phrases",
        "ordinals",
        "go_action",
        "language",
    ];
    for _ in 0..500 {
        let mut value = base.clone();
        let key = *rng.pick(&keys);
        let replacement = rng.pick(&junk).clone();
        match value.get_mut(key) {
            Some(serde_json::Value::Array(items)) if !items.is_empty() && rng.below(2) == 0 => {
                let at = rng.below(items.len());
                items[at] = replacement;
            }
            Some(slot) => *slot = replacement,
            None => {}
        }
        let bytes = serde_json::to_vec(&value).unwrap();
        if let Ok(parser) = Parser::from_json_slice(&bytes) {
            for line in ["coge la llave", "n", "cógelo", "pon todo en la caja", "x"] {
                assert_bounded(&parser.parse_line(line, &scope), &scope);
            }
        }
    }
}
