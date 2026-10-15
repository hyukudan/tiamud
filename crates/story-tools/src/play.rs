//! Interactive offline player for authors.
//!
//! Reads bounded lines from any [`BufRead`] and writes plain text to any
//! [`Write`], so the same loop serves a terminal and tests. Commands:
//! a choice number, `deshacer`/`undo`, `estado`/`state`, `mirar`/`look`,
//! `ayuda`/`help` and `salir`/`quit`. End of input also quits.
//!
//! Menu scenes (format v2) add: `N` for a verb without target, `N M` for verb
//! `N` on target `M`, `cN` for a gamebook choice, and the displayed words
//! themselves, for example `mirar mesa` or `investigar ventana`. Words are
//! matched case- and accent-insensitively against the labels the view
//! offers, exactly as a Telnet client would type them; a lone `mirar` still
//! redraws the scene. While a dialogue waits, a number answers it.

use std::fmt::Write as _;
use std::io::{self, BufRead, Write};

use tiamud_story_core::{StoryPack, StoryState, StoryView, UndoRing, Value, view};

use crate::render::{render_events, render_view};
use crate::script::{ChooseError, Stepped, UNDO_CAPACITY, choose, menu_by_number, say};

/// Longest accepted input line in bytes; longer lines are discarded whole.
pub const MAX_INPUT_BYTES: usize = 256;
/// Most input lines one session reads before stopping.
pub const MAX_COMMANDS: usize = 100_000;

/// How a session finished.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlayOutcome {
    pub final_state: StoryState,
    pub commands: usize,
}

enum Line {
    Text(Vec<u8>),
    TooLong,
    Eof,
}

/// Reads one `\n`-terminated line of at most `limit` bytes. Overlong lines
/// are consumed to their end and reported without being buffered.
fn read_line_bounded<R: BufRead>(input: &mut R, limit: usize) -> io::Result<Line> {
    let mut line = Vec::new();
    let mut overflow = false;
    let mut any = false;
    loop {
        let available = match input.fill_buf() {
            Ok(available) => available,
            Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
            Err(error) => return Err(error),
        };
        if available.is_empty() {
            return Ok(match (any, overflow) {
                (false, _) => Line::Eof,
                (true, true) => Line::TooLong,
                (true, false) => Line::Text(line),
            });
        }
        any = true;
        let (chunk, used, done) = match available.iter().position(|byte| *byte == b'\n') {
            Some(position) => (&available[..position], position + 1, true),
            None => (available, available.len(), false),
        };
        if !overflow {
            if line.len().saturating_add(chunk.len()) > limit {
                overflow = true;
                line.clear();
            } else {
                line.extend_from_slice(chunk);
            }
        }
        input.consume(used);
        if done {
            return Ok(if overflow {
                Line::TooLong
            } else {
                Line::Text(line)
            });
        }
    }
}

fn help(spanish: bool) -> &'static str {
    if spanish {
        "Órdenes: número de opción, deshacer, estado, mirar, ayuda, salir.\n\
         Menús: N (verbo), N M (verbo y objetivo), cN (opción), o las palabras: «mirar mesa».\n"
    } else {
        "Commands: choice number, undo, state, look, help, quit.\n\
         Menus: N (verb), N M (verb and target), cN (choice), or the words: \"look desk\".\n"
    }
}

enum Rejected {
    Unknown,
    Choose(ChooseError),
}

impl From<ChooseError> for Rejected {
    fn from(error: ChooseError) -> Self {
        Self::Choose(error)
    }
}

fn choose_message(error: &ChooseError, spanish: bool) -> String {
    let text = match (error, spanish) {
        (ChooseError::Ended, true) => "La historia ha terminado: deshacer o salir.",
        (ChooseError::Ended, false) => "The story has ended: undo or quit.",
        (ChooseError::OutOfRange { .. } | ChooseError::NotOffered, true) => "No existe esa opción.",
        (ChooseError::OutOfRange { .. } | ChooseError::NotOffered, false) => "No such choice.",
        (ChooseError::NoMenu, true) => "Esta escena no tiene menú de verbos.",
        (ChooseError::NoMenu, false) => "This scene has no verb menu.",
        (ChooseError::NoDialogue, true) => "Nadie espera tu respuesta.",
        (ChooseError::NoDialogue, false) => "Nobody is waiting for an answer.",
        (ChooseError::NeedsTarget, true) => "Ese verbo necesita un objetivo: N M.",
        (ChooseError::NeedsTarget, false) => "That verb needs a target: N M.",
        (ChooseError::UnexpectedTarget, true) => "Ese verbo no lleva objetivo.",
        (ChooseError::UnexpectedTarget, false) => "That verb takes no target.",
        (ChooseError::Story(error), _) => return format!("error: {error}"),
    };
    text.to_owned()
}

/// Folds case and Spanish accents so typed words match displayed labels.
fn fold(text: &str) -> String {
    text.chars()
        .flat_map(char::to_lowercase)
        .map(|c| match c {
            'á' | 'à' | 'ä' | 'â' => 'a',
            'é' | 'è' | 'ë' | 'ê' => 'e',
            'í' | 'ì' | 'ï' | 'î' => 'i',
            'ó' | 'ò' | 'ö' | 'ô' => 'o',
            'ú' | 'ù' | 'ü' | 'û' => 'u',
            'ñ' => 'n',
            other => other,
        })
        .collect::<String>()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

/// Turns one typed line into an input on the current view. Only labels and
/// numbers the view itself offers are matched; no authored id is exposed.
fn interpret(
    pack: &StoryPack,
    state: &StoryState,
    current: &StoryView,
    text: &str,
) -> Result<Stepped, Rejected> {
    let words: Vec<&str> = text.split_whitespace().collect();
    let numbers: Option<Vec<usize>> = words.iter().map(|word| word.parse().ok()).collect();
    if current.dialogue.is_some() {
        return match numbers.as_deref() {
            Some([number]) => Ok(say(pack, state, *number)?),
            _ => Err(Rejected::Unknown),
        };
    }
    if current.menu.is_none() {
        return match numbers.as_deref() {
            Some([number]) => Ok(choose(pack, state, *number)?),
            _ => Err(Rejected::Unknown),
        };
    }
    if let [word] = words.as_slice()
        && let Some(number) = word.strip_prefix('c').and_then(|rest| rest.parse().ok())
    {
        return Ok(choose(pack, state, number)?);
    }
    match numbers.as_deref() {
        Some([verb]) => return Ok(menu_by_number(pack, state, *verb, None)?),
        Some([verb, target]) => return Ok(menu_by_number(pack, state, *verb, Some(*target))?),
        Some(_) => return Err(Rejected::Unknown),
        None => {}
    }
    // Words: the first is a verb label, the rest a target label.
    let menu = current.menu.as_ref().ok_or(Rejected::Unknown)?;
    let (first, rest) = words.split_first().ok_or(Rejected::Unknown)?;
    let first = fold(first);
    let wanted = fold(&rest.join(" "));
    let locale_labels = |label: &tiamud_story_core::LocalizedText| -> Vec<String> {
        label.0.values().map(|value| fold(value)).collect()
    };
    let (verb_index, verb) = menu
        .verbs
        .iter()
        .enumerate()
        .find(|(_, verb)| locale_labels(&verb.label).contains(&first))
        .ok_or(Rejected::Unknown)?;
    if !verb.needs_target {
        return if wanted.is_empty() {
            Ok(menu_by_number(pack, state, verb_index + 1, None)?)
        } else {
            Err(ChooseError::UnexpectedTarget.into())
        };
    }
    if wanted.is_empty() {
        return Err(ChooseError::NeedsTarget.into());
    }
    let exact: Vec<usize> = (0..verb.targets.len())
        .filter(|index| locale_labels(&verb.targets[*index].label).contains(&wanted))
        .collect();
    let partial: Vec<usize> = (0..verb.targets.len())
        .filter(|index| {
            locale_labels(&verb.targets[*index].label)
                .iter()
                .any(|label| label.contains(&wanted))
        })
        .collect();
    let target = match (exact.as_slice(), partial.as_slice()) {
        ([only], _) | ([], [only]) => *only,
        _ => return Err(ChooseError::NotOffered.into()),
    };
    Ok(menu_by_number(
        pack,
        state,
        verb_index + 1,
        Some(target + 1),
    )?)
}

/// Author-facing dump of the full state, hidden variables included.
fn render_state(state: &StoryState, seed: u64) -> String {
    let mut out = String::new();
    let _ = writeln!(out, "node: {}", state.node);
    let _ = writeln!(
        out,
        "turn: {} · score: {} · seed: {seed}",
        state.turn, state.score
    );
    for (var, value) in &state.vars {
        let value = match value {
            Value::Int(value) => value.to_string(),
            Value::Bool(value) => value.to_string(),
        };
        let _ = writeln!(out, "var {var} = {value}");
    }
    let items: Vec<&str> = state.inventory.iter().map(String::as_str).collect();
    let _ = writeln!(out, "items: [{}]", items.join(", "));
    let _ = writeln!(out, "visited: {} node(s)", state.visited.len());
    if let Some(ending) = &state.ended {
        let _ = writeln!(out, "ended: {} ({:?})", ending.id, ending.kind);
    }
    out
}

/// Runs an interactive session until `salir`/`quit`, end of input or the
/// command limit. The pack must already be validated.
pub fn play<R: BufRead, W: Write>(
    pack: &StoryPack,
    seed: u64,
    locale: &str,
    mut input: R,
    mut output: W,
) -> io::Result<PlayOutcome> {
    let spanish = !locale
        .split('-')
        .next()
        .is_some_and(|language| language.eq_ignore_ascii_case("en"));
    let to_io = |error: tiamud_story_core::StoryError| io::Error::other(error.to_string());
    let mut state = StoryState::new(pack, seed).map_err(to_io)?;
    let mut undo = UndoRing::new(UNDO_CAPACITY).map_err(to_io)?;
    let mut commands = 0usize;
    write!(output, "{}", help(spanish))?;
    write!(
        output,
        "{}",
        render_view(&view(pack, &state).map_err(to_io)?, locale)
    )?;
    loop {
        write!(output, "> ")?;
        output.flush()?;
        if commands >= MAX_COMMANDS {
            writeln!(output, "\n(command limit reached)")?;
            break;
        }
        let line = read_line_bounded(&mut input, MAX_INPUT_BYTES)?;
        commands += 1;
        let text = match line {
            Line::Eof => {
                writeln!(output)?;
                break;
            }
            Line::TooLong => {
                writeln!(
                    output,
                    "{}",
                    if spanish {
                        "Entrada demasiado larga."
                    } else {
                        "Input too long."
                    }
                )?;
                continue;
            }
            Line::Text(bytes) => String::from_utf8_lossy(&bytes).trim().to_lowercase(),
        };
        match text.as_str() {
            "" => {}
            "salir" | "quit" | "q" => break,
            "ayuda" | "help" | "?" => write!(output, "{}", help(spanish))?,
            "estado" | "state" => write!(output, "{}", render_state(&state, seed))?,
            "mirar" | "look" => write!(
                output,
                "{}",
                render_view(&view(pack, &state).map_err(to_io)?, locale)
            )?,
            "deshacer" | "undo" => match undo.pop() {
                Some(previous) => {
                    state = previous;
                    write!(
                        output,
                        "{}",
                        render_view(&view(pack, &state).map_err(to_io)?, locale)
                    )?;
                }
                None => writeln!(
                    output,
                    "{}",
                    if spanish {
                        "No hay nada que deshacer."
                    } else {
                        "Nothing to undo."
                    }
                )?,
            },
            other => {
                let current = view(pack, &state).map_err(to_io)?;
                match interpret(pack, &state, &current, other) {
                    Ok((next, events)) => {
                        write!(output, "{}", render_events(pack, &events, locale))?;
                        undo.push(std::mem::replace(&mut state, next));
                        write!(
                            output,
                            "{}",
                            render_view(&view(pack, &state).map_err(to_io)?, locale)
                        )?;
                    }
                    Err(Rejected::Unknown) => writeln!(
                        output,
                        "{}",
                        if spanish {
                            "No entiendo. Escribe «ayuda»."
                        } else {
                            "Unknown command. Type \"help\"."
                        }
                    )?,
                    Err(Rejected::Choose(error)) => {
                        writeln!(output, "{}", choose_message(&error, spanish))?
                    }
                }
            }
        }
    }
    output.flush()?;
    Ok(PlayOutcome {
        final_state: state,
        commands,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn overlong_lines_are_discarded_whole() {
        let mut input = io::Cursor::new(format!("{}\n7\n", "x".repeat(MAX_INPUT_BYTES + 1)));
        assert!(matches!(
            read_line_bounded(&mut input, MAX_INPUT_BYTES).unwrap(),
            Line::TooLong
        ));
        match read_line_bounded(&mut input, MAX_INPUT_BYTES).unwrap() {
            Line::Text(text) => assert_eq!(text, b"7"),
            _ => panic!("expected a line"),
        }
        assert!(matches!(
            read_line_bounded(&mut input, MAX_INPUT_BYTES).unwrap(),
            Line::Eof
        ));
    }

    #[test]
    fn final_line_without_newline_is_read() {
        let mut input = io::Cursor::new("salir");
        match read_line_bounded(&mut input, MAX_INPUT_BYTES).unwrap() {
            Line::Text(text) => assert_eq!(text, b"salir"),
            _ => panic!("expected a line"),
        }
    }
}
