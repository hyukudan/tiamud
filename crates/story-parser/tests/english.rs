//! Table-driven English parser tests (Infocom/Inform conventions).

mod support;

use support::{answer, parse, run_table};
use tiamud_story_parser::{ParseOutcome, Parser, ScopeObject, SimpleScope};

fn parser() -> Parser {
    Parser::english().expect("shipped English lexicon is valid")
}

fn room() -> SimpleScope {
    SimpleScope::new(vec![
        ScopeObject::new(
            "brass-lamp",
            "the brass lamp",
            &["lamp", "lantern"],
            &["brass"],
        ),
        ScopeObject::new("gold-key", "the golden key", &["key"], &["golden", "gold"]),
        ScopeObject::new("iron-key", "the iron key", &["key"], &["iron", "rusty"]),
        ScopeObject::new("door", "the door", &["door"], &["oak"]),
        ScopeObject::new("box", "the box", &["box", "chest"], &["wooden"]),
        ScopeObject::new("troll", "the troll", &["troll"], &["ugly"]),
        ScopeObject::new("coin", "a coin", &["coin"], &["silver"]),
        ScopeObject::new("table", "the table", &["table"], &[]),
    ])
    .with_vocabulary(&["dragon"])
}

#[test]
fn verbs_and_objects() {
    run_table(
        &parser(),
        &room(),
        &[
            ("take the golden key", "take gold-key"),
            ("TAKE KEY", "? gold-key|iron-key"),
            ("get lamp", "take brass-lamp"),
            ("pick up the lamp", "take brass-lamp"),
            ("Take the Lamp!", "take brass-lamp"),
            ("x lantern", "examine brass-lamp"),
            ("examine the wooden chest", "examine box"),
            ("look", "look"),
            ("l", "look"),
            ("look at the box", "examine box at"),
            ("look in the chest", "search box in"),
            ("look under table", "look_under table under"),
            ("open door", "open door"),
            (
                "open the door with the iron key",
                "unlock door with iron-key",
            ),
            (
                "unlock the oak door with the rusty key",
                "unlock door with iron-key",
            ),
            ("put coin in box", "put_in coin in box"),
            ("put the coin into the chest", "put_in coin in box"),
            ("put coin on table", "put_on coin on table"),
            ("put down the coin", "drop coin"),
            (
                "give the silver coin to the ugly troll",
                "give coin to troll",
            ),
            ("show the coin to the troll", "show coin to troll"),
            ("talk to troll", "talk troll to"),
            ("speak with the troll", "talk troll with"),
            (
                "hit the troll with the lamp",
                "attack troll with brass-lamp",
            ),
            ("turn on the lamp", "switch_on brass-lamp"),
            ("switch off lamp", "switch_off brass-lamp"),
            ("drop all", "drop all"),
            ("take everything except the lamp", "take all -brass-lamp"),
            ("take all but the key", "take all -gold-key -iron-key"),
            (
                "take all except the lamp and the coin",
                "take all -brass-lamp -coin",
            ),
        ],
    );
}

#[test]
fn pronouns() {
    let parser = parser();
    assert_eq!(
        parse(&parser, &room().with_last("coin"), "take it"),
        "take coin"
    );
    assert_eq!(
        parse(&parser, &room().with_last("box"), "put the coin in it"),
        "put_in coin in box"
    );
    assert_eq!(
        parse(&parser, &room(), "take it"),
        "error: I'm not sure what \"it\" refers to."
    );
}

#[test]
fn directions_meta_and_lines() {
    run_table(
        &parser(),
        &room(),
        &[
            ("n", "go north"),
            ("north", "go north"),
            ("go north", "go north"),
            ("go to the north", "go north"),
            ("w", "go west"),
            ("nw", "go northwest"),
            ("sw", "go southwest"),
            ("u", "go up"),
            ("d", "go down"),
            ("in", "go in"),
            ("out", "go out"),
            ("go", "?? Where do you want to go?"),
            ("i", "inventory"),
            ("z", "wait"),
            ("save", "!save"),
            ("restore", "!restore"),
            ("load", "!restore"),
            ("undo", "!undo"),
            ("score", "!score"),
            ("help", "!help"),
            ("q", "!quit"),
            ("restart", "!restart"),
            ("take lamp then n", "take brass-lamp ; go north"),
            ("take lamp and then go north", "take brass-lamp ; go north"),
            ("take lamp and drop it", "take brass-lamp ; drop brass-lamp"),
            (
                "take coin. put it in the box",
                "take coin ; put_in coin in box",
            ),
        ],
    );
}

#[test]
fn errors_and_questions() {
    run_table(
        &parser(),
        &room(),
        &[
            ("xyzzy", "error: I don't know the word \"xyzzy\"."),
            (
                "frobnicate the lamp",
                "error: I don't know the word \"frobnicate\".",
            ),
            ("take the dragon", "error: You can't see any such thing."),
            (
                "lamp",
                "error: That sentence doesn't start with a verb I know.",
            ),
            (
                "examine all",
                "error: You can't use \"all\" with that verb.",
            ),
            ("", "error: I beg your pardon?"),
            ("examine", "?? What do you want to examine?"),
            ("take", "?? What do you want to take?"),
            ("talk", "?? What do you want to talk to?"),
            (
                "unlock door",
                "?? What do you want to unlock the door with?",
            ),
            ("put the coin", "?? What do you want to put a coin in?"),
        ],
    );
}

#[test]
fn ambiguity_answers() {
    let parser = parser();
    let scope = room();
    match parser.parse_line("take key", &scope).as_slice() {
        [ParseOutcome::Ambiguous { question, .. }] => {
            assert_eq!(
                question,
                "Which do you mean, the golden key or the iron key?"
            )
        }
        other => panic!("unexpected {other:?}"),
    }
    for (reply, expected) in [
        ("the golden one", "take gold-key"),
        ("gold", "take gold-key"),
        ("rusty", "take iron-key"),
        ("first", "take gold-key"),
        ("the second one", "take iron-key"),
        ("2", "take iron-key"),
        ("the troll", "error: I don't know which one you mean."),
        ("n", "go north"),
    ] {
        assert_eq!(
            answer(&parser, &scope, "take key", reply),
            expected,
            "{reply:?}"
        );
    }
    assert_eq!(
        answer(&parser, &scope, "unlock door", "with the golden key"),
        "unlock door with gold-key"
    );
    assert_eq!(answer(&parser, &scope, "go", "north"), "go north");
}
