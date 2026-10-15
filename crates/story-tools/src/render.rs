//! Plain-text rendering of a [`StoryView`], numbered like a Telnet player
//! would see it. Only view data is rendered: hidden variables, authored ids,
//! unavailable choices and RNG state never appear.

use std::fmt::Write as _;

use tiamud_story_core::{Cue, EndingKind, StoryEvent, StoryPack, StoryView, Value};

/// User-interface words for one display language.
#[derive(Debug)]
pub struct Labels {
    pub turn: &'static str,
    pub score: &'static str,
    pub inventory: &'static str,
    pub empty: &'static str,
    pub the_end: &'static str,
    pub victory: &'static str,
    pub defeat: &'static str,
    pub neutral: &'static str,
    pub roll: &'static str,
    pub success: &'static str,
    pub failure: &'static str,
    pub gained: &'static str,
    pub lost: &'static str,
    pub music: &'static str,
    pub sound: &'static str,
    pub stop_music: &'static str,
    pub yes: &'static str,
    pub no: &'static str,
}

const SPANISH: Labels = Labels {
    turn: "Turno",
    score: "Puntuación",
    inventory: "Inventario",
    empty: "(vacío)",
    the_end: "FIN",
    victory: "victoria",
    defeat: "derrota",
    neutral: "final",
    roll: "Tirada",
    success: "éxito",
    failure: "fallo",
    gained: "Obtienes",
    lost: "Pierdes",
    music: "música",
    sound: "sonido",
    stop_music: "se detiene la música",
    yes: "sí",
    no: "no",
};

const ENGLISH: Labels = Labels {
    turn: "Turn",
    score: "Score",
    inventory: "Inventory",
    empty: "(empty)",
    the_end: "THE END",
    victory: "victory",
    defeat: "defeat",
    neutral: "ending",
    roll: "Roll",
    success: "success",
    failure: "failure",
    gained: "You gain",
    lost: "You lose",
    music: "music",
    sound: "sound",
    stop_music: "music stops",
    yes: "yes",
    no: "no",
};

/// Spanish unless the locale's language is English.
pub fn labels(locale: &str) -> &'static Labels {
    let language = locale.split('-').next().unwrap_or_default();
    if language.eq_ignore_ascii_case("en") {
        &ENGLISH
    } else {
        &SPANISH
    }
}

fn kind_label(labels: &Labels, kind: EndingKind) -> &'static str {
    match kind {
        EndingKind::Victory => labels.victory,
        EndingKind::Defeat => labels.defeat,
        EndingKind::Neutral => labels.neutral,
    }
}

fn value_text(labels: &Labels, value: Value) -> String {
    match value {
        Value::Int(value) => value.to_string(),
        Value::Bool(true) => labels.yes.to_owned(),
        Value::Bool(false) => labels.no.to_owned(),
    }
}

/// Renders the scene: title, text, status line, inventory and either the
/// numbered available choices or the ending banner.
pub fn render_view(view: &StoryView, locale: &str) -> String {
    let labels = labels(locale);
    let mut out = String::new();
    let title = view.title.resolve(locale).unwrap_or_default();
    let _ = writeln!(out, "== {title} ==");
    let _ = writeln!(out, "{}", view.text.resolve(locale).unwrap_or_default());
    out.push('\n');
    let mut status = format!(
        "{} {} · {} {}",
        labels.turn, view.turn, labels.score, view.score
    );
    for stat in &view.stats {
        let _ = write!(
            status,
            " · {} {}",
            stat.label.resolve(locale).unwrap_or_default(),
            value_text(labels, stat.value)
        );
    }
    let _ = writeln!(out, "{status}");
    let inventory: Vec<&str> = view
        .inventory
        .iter()
        .filter_map(|name| name.resolve(locale))
        .collect();
    let _ = writeln!(
        out,
        "{}: {}",
        labels.inventory,
        if inventory.is_empty() {
            labels.empty.to_owned()
        } else {
            inventory.join(", ")
        }
    );
    out.push('\n');
    if let Some(ending) = &view.ending {
        let _ = writeln!(
            out,
            "*** {} ({}) ***",
            labels.the_end,
            kind_label(labels, ending.kind)
        );
    } else {
        for (index, choice) in view.choices.iter().enumerate() {
            let _ = writeln!(
                out,
                "{}) {}",
                index + 1,
                choice.label.resolve(locale).unwrap_or_default()
            );
        }
    }
    out
}

/// Renders the player-relevant consequences of one step: dice, item changes,
/// score and media cues. Variable and node events are left to the next view.
pub fn render_events(pack: &StoryPack, events: &[StoryEvent], locale: &str) -> String {
    let labels = labels(locale);
    let item_name = |id: &str| {
        pack.items
            .iter()
            .find(|item| item.id == id)
            .and_then(|item| item.name.resolve(locale))
            .unwrap_or(id)
            .to_owned()
    };
    let mut out = String::new();
    for event in events {
        match event {
            StoryEvent::DiceRolled {
                count,
                sides,
                rolls,
                total,
                passed,
                ..
            } => {
                let faces: Vec<String> = rolls.iter().map(u8::to_string).collect();
                let _ = write!(out, "[{} {count}d{sides}: {}", labels.roll, faces.join("+"));
                // The bonus folds the fixed modifier and any modifier variable.
                let faces_sum: i128 = rolls.iter().map(|face| i128::from(*face)).sum();
                let bonus = i128::from(*total) - faces_sum;
                if bonus != 0 {
                    let _ = write!(out, " {bonus:+}");
                }
                let _ = write!(out, " = {total}");
                if let Some(passed) = passed {
                    let _ = write!(
                        out,
                        " ({})",
                        if *passed {
                            labels.success
                        } else {
                            labels.failure
                        }
                    );
                }
                out.push_str("]\n");
            }
            StoryEvent::ItemGained { item } => {
                let _ = writeln!(out, "[{}: {}]", labels.gained, item_name(item));
            }
            StoryEvent::ItemLost { item } => {
                let _ = writeln!(out, "[{}: {}]", labels.lost, item_name(item));
            }
            StoryEvent::ScoreChanged { delta, total } => {
                let _ = writeln!(out, "[{} {delta:+} = {total}]", labels.score);
            }
            StoryEvent::Cue(Cue::Music { asset }) => {
                let _ = writeln!(out, "[{}: {asset}]", labels.music);
            }
            StoryEvent::Cue(Cue::Sfx { asset }) => {
                let _ = writeln!(out, "[{}: {asset}]", labels.sound);
            }
            StoryEvent::Cue(Cue::StopMusic) => {
                let _ = writeln!(out, "[{}]", labels.stop_music);
            }
            StoryEvent::NodeEntered { .. }
            | StoryEvent::VariableChanged { .. }
            | StoryEvent::Ended(_) => {}
        }
    }
    out
}
