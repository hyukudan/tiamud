//! Table-driven Spanish parser tests (aventuras conversacionales conventions).

mod support;

use support::{answer, parse, pending, run_table};
use tiamud_story_parser::{
    MAX_CANDIDATES, ParseErrorKind, ParseOutcome, Parser, ScopeObject, SimpleScope,
};

fn parser() -> Parser {
    Parser::spanish().expect("shipped Spanish lexicon is valid")
}

fn room() -> SimpleScope {
    SimpleScope::new(vec![
        ScopeObject::new(
            "llave-dorada",
            "la llave dorada",
            &["llave"],
            &["dorada", "oro"],
        ),
        ScopeObject::new(
            "llave-hierro",
            "la llave de hierro",
            &["llave"],
            &["hierro", "oxidada"],
        ),
        ScopeObject::new("puerta", "la puerta", &["puerta"], &["roble"]),
        ScopeObject::new("caja", "la caja", &["caja", "cofre"], &["madera"]),
        ScopeObject::new(
            "lampara",
            "la lámpara",
            &["lámpara", "farol"],
            &["vieja", "latón"],
        ),
        ScopeObject::new(
            "guardia",
            "el guardia",
            &["guardia", "hombre"],
            &["corpulento"],
        ),
        ScopeObject::new("moneda", "una moneda", &["moneda"], &["plata"]),
        ScopeObject::new("mesa", "la mesa", &["mesa"], &[]),
        ScopeObject::new("muñeco", "el muñeco", &["muñeco"], &["trapo"]),
        ScopeObject::new("vela", "la vela", &["vela"], &["cera"]),
    ])
    .with_vocabulary(&["dragón"])
}

#[test]
fn verbs_objects_and_accents() {
    run_table(
        &parser(),
        &room(),
        &[
            ("coge la llave dorada", "take llave-dorada"),
            ("COGE LA LLAVE DORADA", "take llave-dorada"),
            ("coger la llave de hierro", "take llave-hierro"),
            ("coge la dorada", "take llave-dorada"),
            ("toma la lámpara", "take lampara"),
            ("toma la lampara vieja", "take lampara"),
            ("Tomá la LÁMPARA de latón", "take lampara"),
            ("examina el farol", "examine lampara"),
            ("x farol", "examine lampara"),
            ("ex cofre", "examine caja"),
            ("mira", "look"),
            ("m", "look"),
            ("mira la caja", "examine caja"),
            ("mira en la caja", "search caja in"),
            ("mira debajo de la mesa", "look_under mesa under"),
            ("mira bajo la mesa", "look_under mesa under"),
            ("abre la puerta", "open puerta"),
            (
                "abre la puerta con la llave dorada",
                "unlock puerta with llave-dorada",
            ),
            (
                "abre la puerta con la llave de hierro",
                "unlock puerta with llave-hierro",
            ),
            ("abre la puerta con la llave", "? llave-dorada|llave-hierro"),
            ("coge la llave", "? llave-dorada|llave-hierro"),
            ("pon la moneda en la caja", "put_in moneda in caja"),
            ("mete la moneda dentro de la caja", "put_in moneda in caja"),
            ("mete la moneda dentro del cofre", "put_in moneda in caja"),
            ("pon la moneda sobre la mesa", "put_on moneda on mesa"),
            ("pon la moneda encima de la mesa", "put_on moneda on mesa"),
            ("da la moneda al guardia", "give moneda to guardia"),
            (
                "entrega la moneda de plata al hombre",
                "give moneda to guardia",
            ),
            ("habla con el guardia", "talk guardia with"),
            ("habla al guardia", "talk guardia to"),
            ("enciende la lámpara", "switch_on lampara"),
            ("apaga el farol", "switch_off lampara"),
            ("¡coge la vela!", "take vela"),
            ("coge la moneda de plata", "take moneda"),
            (
                "enseña la llave dorada al guardia",
                "show llave-dorada to guardia",
            ),
        ],
    );
}

#[test]
fn enye_is_distinct_from_n() {
    run_table(
        &parser(),
        &room(),
        &[
            ("coge el muñeco", "take muñeco"),
            ("COGE EL MUÑECO", "take muñeco"),
            ("coge el mun\u{303}eco", "take muñeco"),
            ("coge el muneco", "error: No conozco la palabra 'muneco'."),
            (
                "ensena la llave dorada al guardia",
                "error: No conozco la palabra 'ensena'.",
            ),
        ],
    );
}

#[test]
fn enclitic_and_standalone_pronouns() {
    let parser = parser();
    let cases: &[(&str, &str, &str)] = &[
        ("lampara", "cógelo", "take lampara"),
        ("puerta", "ábrela", "open puerta"),
        ("caja", "examínalo", "examine caja"),
        ("caja", "EXAMÍNALA", "examine caja"),
        ("moneda", "ponla en la caja", "put_in moneda in caja"),
        ("moneda", "dáselo al guardia", "give moneda to guardia"),
        ("moneda", "dámelo", "?? ¿A qué quieres dar una moneda?"),
        ("guardia", "dale la moneda", "give moneda to guardia"),
        (
            "guardia",
            "enséñale la llave dorada",
            "show llave-dorada to guardia",
        ),
        ("guardia", "háblale", "talk guardia with"),
        ("vela", "coge la", "take vela"),
        ("vela", "cogerla", "take vela"),
        (
            "llave-dorada",
            "abre la puerta con ella",
            "unlock puerta with llave-dorada",
        ),
        ("dragon-ausente", "cógelo", "error: No ves eso por aquí."),
    ];
    let mut failures = Vec::new();
    for (last, input, expected) in cases {
        let got = parse(&parser, &room().with_last(last), input);
        if got != *expected {
            failures.push(format!(
                "{input:?} (last {last}): expected {expected}, got {got}"
            ));
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
    assert_eq!(
        parse(&parser, &room(), "cógela"),
        "error: No sé a qué te refieres con 'la'."
    );
}

#[test]
fn all_and_except() {
    run_table(
        &parser(),
        &room(),
        &[
            ("coge todo", "take all"),
            ("deja todos", "drop all"),
            ("coge todo excepto la lámpara", "take all -lampara"),
            (
                "coge todo menos la llave",
                "take all -llave-dorada -llave-hierro",
            ),
            (
                "coge todo menos la lámpara y la moneda",
                "take all -lampara -moneda",
            ),
            (
                "coge todo salvo la lámpara, la moneda",
                "take all -lampara -moneda",
            ),
            ("mete todo en la caja", "put_in in caja all"),
            (
                "examina todo",
                "error: No puedes usar «todo» con ese verbo.",
            ),
            ("coge todo excepto", "error: No entiendo esa frase."),
            ("coge todo excepto el dragón", "error: No ves eso por aquí."),
        ],
    );
}

#[test]
fn directions_and_movement() {
    run_table(
        &parser(),
        &room(),
        &[
            ("n", "go north"),
            ("norte", "go north"),
            ("S", "go south"),
            ("e", "go east"),
            ("este", "go east"),
            ("o", "go west"),
            ("ne", "go northeast"),
            ("no", "go northwest"),
            ("se", "go southeast"),
            ("so", "go southwest"),
            ("sube", "go up"),
            ("abajo", "go down"),
            ("dentro", "go in"),
            ("sal", "go out"),
            ("ve al norte", "go north"),
            ("ir hacia el sur", "go south"),
            ("camina al noroeste", "go northwest"),
            ("ve", "?? ¿Hacia dónde quieres ir?"),
            ("norte la caja", "error: No entiendo esa frase."),
            (
                "vela",
                "error: Esa frase no empieza por un verbo que conozca.",
            ),
        ],
    );
}

#[test]
fn meta_verbs() {
    run_table(
        &parser(),
        &room(),
        &[
            ("i", "inventory"),
            ("inventario", "inventory"),
            ("z", "wait"),
            ("espera", "wait"),
            ("guardar", "!save"),
            ("cargar", "!restore"),
            ("deshacer", "!undo"),
            ("puntos", "!score"),
            ("puntuación", "!score"),
            ("ayuda", "!help"),
            ("salir", "!quit"),
            ("fin", "!quit"),
            ("reiniciar", "!restart"),
        ],
    );
}

#[test]
fn multi_command_lines() {
    run_table(
        &parser(),
        &room(),
        &[
            ("ne. so", "go northeast ; go southwest"),
            ("n, e y luego s", "go north ; go east ; go south"),
            ("n y después e", "go north ; go east"),
            (
                "coge la llave dorada y ábrela",
                "take llave-dorada ; open llave-dorada",
            ),
            (
                "coge la moneda y ponla en la caja",
                "take moneda ; put_in moneda in caja",
            ),
            (
                "coge la llave dorada, abre la puerta con ella",
                "take llave-dorada ; unlock puerta with llave-dorada",
            ),
            ("mira la vela y luego cógela", "examine vela ; take vela"),
            ("coge todo y n", "take all ; go north"),
            ("coge la llave; n", "? llave-dorada|llave-hierro"),
            ("n; coge la llave", "go north ; ? llave-dorada|llave-hierro"),
            (
                "n. s. e. o. n",
                "error: Demasiadas órdenes a la vez; prueba de una en una.",
            ),
            ("n. s. e. o", "go north ; go south ; go east ; go west"),
            ("n...", "go north"),
        ],
    );
}

#[test]
fn errors_and_bounds() {
    let long = "n ".repeat(150);
    let words = "coge la moneda ".repeat(6);
    let cases = [
        ("", "error: ¿Perdón?"),
        ("   ... ,, ", "error: ¿Perdón?"),
        ("xyzzy", "error: No conozco la palabra 'xyzzy'."),
        ("coge el xyzzy", "error: No conozco la palabra 'xyzzy'."),
        ("saca la moneda", "error: No conozco la palabra 'saca'."),
        ("coge el dragón", "error: No ves eso por aquí."),
        (
            "llave",
            "error: Esa frase no empieza por un verbo que conozca.",
        ),
        (
            "coge la llave y la lámpara",
            "error: No entiendo esa frase.",
        ),
        ("mira la", "error: No sé a qué te refieres con 'la'."),
        ("coge", "?? ¿Qué quieres coger?"),
        ("abre", "?? ¿Qué quieres abrir?"),
        ("abre la puerta con", "?? ¿Con qué quieres abrir la puerta?"),
        ("pon la moneda", "?? ¿En qué quieres poner una moneda?"),
        ("pon en la caja", "?? ¿Qué quieres poner?"),
        ("habla", "?? ¿Con qué quieres hablar?"),
        ("coge el", "?? ¿Qué quieres coger?"),
        (long.as_str(), "error: Esa orden es demasiado larga."),
        (
            words.as_str(),
            "error: Esa orden tiene demasiadas palabras.",
        ),
    ];
    run_table(&parser(), &room(), &cases);
}

#[test]
fn ambiguity_question_and_answers() {
    let parser = parser();
    let scope = room();
    match parser.parse_line("coge la llave", &scope).as_slice() {
        [
            ParseOutcome::Ambiguous {
                candidates,
                question,
                pending,
            },
        ] => {
            assert_eq!(
                question,
                "¿Cuál quieres decir: la llave dorada o la llave de hierro?"
            );
            assert_eq!(candidates.len(), 2);
            assert_eq!(candidates[0].name, "la llave dorada");
            assert_eq!(pending.candidates().len(), 2);
        }
        other => panic!("unexpected {other:?}"),
    }
    for (reply, expected) in [
        ("la dorada", "take llave-dorada"),
        ("dorada", "take llave-dorada"),
        ("la de hierro", "take llave-hierro"),
        ("la llave oxidada", "take llave-hierro"),
        ("2", "take llave-hierro"),
        ("1", "take llave-dorada"),
        ("segunda", "take llave-hierro"),
        ("la primera", "take llave-dorada"),
        ("3", "error: No sé a cuál te refieres."),
        ("la vela", "error: No sé a cuál te refieres."),
        ("xyzzy", "error: No conozco la palabra 'xyzzy'."),
        ("norte", "go north"),
        ("coge la vela", "take vela"),
        ("n. s", "go north ; go south"),
    ] {
        assert_eq!(
            answer(&parser, &scope, "coge la llave", reply),
            expected,
            "reply {reply:?}"
        );
    }
    // The second object is resolved after the ambiguous first one is answered.
    assert_eq!(
        answer(&parser, &scope, "abre la puerta con la llave", "dorada"),
        "unlock puerta with llave-dorada"
    );
}

#[test]
fn incomplete_answers() {
    let parser = parser();
    let scope = room();
    for (question, reply, expected) in [
        ("abre", "la puerta", "open puerta"),
        ("coge", "la lámpara", "take lampara"),
        ("coge", "la llave", "? llave-dorada|llave-hierro"),
        ("pon la moneda", "en la caja", "put_in moneda in caja"),
        ("pon la moneda", "la caja", "put_in moneda in caja"),
        ("pon en la caja", "la moneda", "put_in moneda in caja"),
        (
            "abre la puerta con",
            "la llave dorada",
            "unlock puerta with llave-dorada",
        ),
        ("habla", "con el guardia", "talk guardia with"),
        ("ve", "norte", "go north"),
        ("ve", "al sur", "go south"),
        ("ve", "coge la vela", "take vela"),
        ("coge", "n", "go north"),
        ("coge", "xyzzy", "error: No conozco la palabra 'xyzzy'."),
        ("coge", "el dragón", "error: No ves eso por aquí."),
    ] {
        assert_eq!(
            answer(&parser, &scope, question, reply),
            expected,
            "{question:?} -> {reply:?}"
        );
    }
    // A two-step dialogue: incomplete, then ambiguous, then resolved.
    let first = pending(&parser, &scope, "coge");
    let second = match parser.answer(&first, "la llave", &scope).as_slice() {
        [ParseOutcome::Ambiguous { pending, .. }] => pending.clone(),
        other => panic!("unexpected {other:?}"),
    };
    assert_eq!(
        support::line(&parser.answer(&second, "la oxidada", &scope)),
        "take llave-hierro"
    );
}

#[test]
fn pending_is_bound_to_its_lexicon() {
    let spanish = parser();
    let english = Parser::english().unwrap();
    let pending = pending(&spanish, &room(), "coge la llave");
    match english.answer(&pending, "1", &room()).as_slice() {
        [ParseOutcome::Error(error)] => assert_eq!(error.kind, ParseErrorKind::StalePending),
        other => panic!("unexpected {other:?}"),
    }
}

#[test]
fn candidate_bound_and_vagueness() {
    let parser = parser();
    let coins = |n: usize| {
        SimpleScope::new(
            (0..n)
                .map(|i| ScopeObject::new(&format!("moneda-{i}"), "una moneda", &["moneda"], &[]))
                .collect(),
        )
    };
    match parser
        .parse_line("coge la moneda", &coins(MAX_CANDIDATES))
        .as_slice()
    {
        [ParseOutcome::Ambiguous { candidates, .. }] => {
            assert_eq!(candidates.len(), MAX_CANDIDATES)
        }
        other => panic!("unexpected {other:?}"),
    }
    assert_eq!(
        parse(&parser, &coins(MAX_CANDIDATES + 1), "coge la moneda"),
        "error: Tendrás que ser más concreto."
    );
    // Identical objects are still selectable by number.
    assert_eq!(
        answer(&parser, &coins(3), "coge la moneda", "3"),
        "take moneda-2"
    );
}

#[test]
fn invalid_scope_is_an_error_not_a_truncation() {
    let parser = parser();
    let too_many = SimpleScope::new(
        (0..=tiamud_story_parser::MAX_SCOPE_OBJECTS)
            .map(|i| ScopeObject::new(&format!("o{i}"), "o", &["cosa"], &[]))
            .collect(),
    );
    let bad_word = SimpleScope::new(vec![ScopeObject::new("x", "x", &["dos palabras"], &[])]);
    for scope in [too_many, bad_word] {
        match parser.parse_line("coge la cosa", &scope).as_slice() {
            [ParseOutcome::Error(error)] => assert_eq!(error.kind, ParseErrorKind::InvalidScope),
            other => panic!("unexpected {other:?}"),
        }
    }
}

#[test]
fn stepwise_api_matches_line_api() {
    let parser = parser();
    let scope = room();
    let segments = parser
        .split_line("coge la vela, n y luego mira")
        .expect("splits");
    assert_eq!(segments.len(), 3);
    assert_eq!(segments[0].words(), ["coge", "la", "vela"]);
    let outcomes: Vec<_> = segments
        .iter()
        .map(|segment| parser.parse_segment(segment, &scope))
        .collect();
    assert_eq!(support::line(&outcomes), "take vela ; go north ; look");
    assert_eq!(
        parser
            .split_line("coge todo menos la vela y la mesa")
            .unwrap()[0]
            .words(),
        ["coge", "todo", "menos", "la", "vela", ",", "la", "mesa"]
    );
}
