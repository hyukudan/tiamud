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
    pub background: &'static str,
    pub present: &'static str,
    pub portrait: &'static str,
    pub actions: &'static str,
    pub nothing: &'static str,
    pub answers: &'static str,
    pub choices: &'static str,
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
    background: "fondo",
    present: "Presentes",
    portrait: "retrato",
    actions: "Acciones",
    nothing: "(nada)",
    answers: "Respuestas",
    choices: "Opciones",
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
    background: "background",
    present: "Present",
    portrait: "portrait",
    actions: "Actions",
    nothing: "(nothing)",
    answers: "Answers",
    choices: "Choices",
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

/// Renders the scene: title, background, text, present characters, the
/// message window (last response and dialogue lines), status line,
/// inventory, then the ending banner, the dialogue answers, or the numbered
/// choices and verb menu with numbered targets.
pub fn render_view(view: &StoryView, locale: &str) -> String {
    let labels = labels(locale);
    let mut out = String::new();
    let title = view.title.resolve(locale).unwrap_or_default();
    let _ = writeln!(out, "== {title} ==");
    if let Some(background) = &view.background {
        let _ = writeln!(out, "[{}: {background}]", labels.background);
    }
    let _ = writeln!(out, "{}", view.text.resolve(locale).unwrap_or_default());
    if !view.portraits.is_empty() {
        let present: Vec<String> = view
            .portraits
            .iter()
            .map(|portrait| {
                let name = portrait.name.resolve(locale).unwrap_or_default();
                match &portrait.asset {
                    Some(asset) => format!("{name} [{}: {asset}]", labels.portrait),
                    None => name.to_owned(),
                }
            })
            .collect();
        let _ = writeln!(out, "{}: {}", labels.present, present.join(", "));
    }
    if !view.blocks.is_empty() {
        out.push('\n');
        for block in &view.blocks {
            let text = block.text.resolve(locale).unwrap_or_default();
            match (&block.speaker, &block.portrait) {
                (Some(speaker), Some(portrait)) => {
                    let speaker = speaker.resolve(locale).unwrap_or_default();
                    let _ = writeln!(out, "» {speaker} [{}: {portrait}]: {text}", labels.portrait);
                }
                (Some(speaker), None) => {
                    let speaker = speaker.resolve(locale).unwrap_or_default();
                    let _ = writeln!(out, "» {speaker}: {text}");
                }
                (None, _) => {
                    let _ = writeln!(out, "» {text}");
                }
            }
        }
    }
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
    } else if let Some(dialogue) = &view.dialogue {
        let _ = writeln!(out, "{}:", labels.answers);
        for (index, option) in dialogue.options.iter().enumerate() {
            let _ = writeln!(
                out,
                "{}) {}",
                index + 1,
                option.label.resolve(locale).unwrap_or_default()
            );
        }
    } else {
        // In menu scenes plain numbers address verbs, so choices take a
        // `c` prefix there.
        let prefix = if view.menu.is_some() { "c" } else { "" };
        if view.menu.is_some() && !view.choices.is_empty() {
            let _ = writeln!(out, "{}:", labels.choices);
        }
        for (index, choice) in view.choices.iter().enumerate() {
            let _ = writeln!(
                out,
                "{prefix}{}) {}",
                index + 1,
                choice.label.resolve(locale).unwrap_or_default()
            );
        }
        if let Some(menu) = &view.menu {
            let _ = writeln!(out, "{}:", labels.actions);
            for (index, verb) in menu.verbs.iter().enumerate() {
                let label = verb.label.resolve(locale).unwrap_or_default();
                if !verb.needs_target {
                    let _ = writeln!(out, "{}) {label}", index + 1);
                    continue;
                }
                let targets: Vec<String> = verb
                    .targets
                    .iter()
                    .enumerate()
                    .map(|(target, item)| {
                        format!(
                            "{}) {}",
                            target + 1,
                            item.label.resolve(locale).unwrap_or_default()
                        )
                    })
                    .collect();
                let targets = if targets.is_empty() {
                    labels.nothing.to_owned()
                } else {
                    targets.join(" · ")
                };
                let _ = writeln!(out, "{}) {label}: {targets}", index + 1);
            }
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
            // Responses and dialogue lines are shown by the next view.
            StoryEvent::NodeEntered { .. }
            | StoryEvent::VariableChanged { .. }
            | StoryEvent::Ended(_)
            | StoryEvent::MenuAnswered { .. }
            | StoryEvent::DialogueEntered { .. }
            | StoryEvent::DialogueExited { .. } => {}
        }
    }
    out
}
