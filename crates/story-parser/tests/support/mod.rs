//! Shared helpers: compact outcome rendering for table-driven tests.

#![allow(dead_code)]

use tiamud_story_parser::{Command, ParseOutcome, Parser, Pending, Scope};

/// Renders a command as `action [obj] [prep] [obj2] [dir] [all] [-except…]`,
/// prefixed with `!` for meta verbs.
pub fn command(command: &Command) -> String {
    let mut parts = vec![format!(
        "{}{}",
        if command.meta { "!" } else { "" },
        command.action
    )];
    if let Some(obj) = &command.obj {
        parts.push(obj.to_string());
    }
    if let Some(prep) = &command.prep {
        parts.push(prep.to_string());
    }
    if let Some(obj2) = &command.obj2 {
        parts.push(obj2.to_string());
    }
    if let Some(direction) = &command.direction {
        parts.push(direction.to_string());
    }
    if command.all {
        parts.push("all".to_owned());
    }
    for except in &command.except {
        parts.push(format!("-{except}"));
    }
    parts.join(" ")
}

/// Renders one outcome: a command, `? a|b` for ambiguity, `?? question` for
/// an incomplete command or `error: message`.
pub fn outcome(outcome: &ParseOutcome) -> String {
    match outcome {
        ParseOutcome::Command(c) => command(c),
        ParseOutcome::Ambiguous { candidates, .. } => format!(
            "? {}",
            candidates
                .iter()
                .map(|c| c.id.to_string())
                .collect::<Vec<_>>()
                .join("|")
        ),
        ParseOutcome::Incomplete { question, .. } => format!("?? {question}"),
        ParseOutcome::Error(error) => format!("error: {}", error.message),
    }
}

pub fn line(outcomes: &[ParseOutcome]) -> String {
    outcomes.iter().map(outcome).collect::<Vec<_>>().join(" ; ")
}

pub fn parse(parser: &Parser, scope: &dyn Scope, input: &str) -> String {
    line(&parser.parse_line(input, scope))
}

/// Parses `input`, which must end in a question, and returns its pending state.
pub fn pending(parser: &Parser, scope: &dyn Scope, input: &str) -> Pending {
    let outcomes = parser.parse_line(input, scope);
    match outcomes.last() {
        Some(ParseOutcome::Ambiguous { pending, .. })
        | Some(ParseOutcome::Incomplete { pending, .. }) => pending.clone(),
        other => panic!("expected a question for {input:?}, got {other:?}"),
    }
}

pub fn answer(parser: &Parser, scope: &dyn Scope, question: &str, reply: &str) -> String {
    let pending = pending(parser, scope, question);
    line(&parser.answer(&pending, reply, scope))
}

/// Runs a table and reports every mismatch at once.
pub fn run_table(parser: &Parser, scope: &dyn Scope, cases: &[(&str, &str)]) {
    let mut failures = Vec::new();
    for (input, expected) in cases {
        let got = parse(parser, scope, input);
        if got != *expected {
            failures.push(format!(
                "{input:?}\n   expected: {expected}\n   got:      {got}"
            ));
        }
    }
    assert!(
        failures.is_empty(),
        "{} of {} cases failed:\n{}",
        failures.len(),
        cases.len(),
        failures.join("\n")
    );
}
