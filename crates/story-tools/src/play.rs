//! Interactive offline player for authors.
//!
//! Reads bounded lines from any [`BufRead`] and writes plain text to any
//! [`Write`], so the same loop serves a terminal and tests. Commands:
//! a choice number, `deshacer`/`undo`, `estado`/`state`, `mirar`/`look`,
//! `ayuda`/`help` and `salir`/`quit`. End of input also quits.

use std::fmt::Write as _;
use std::io::{self, BufRead, Write};

use tiamud_story_core::{StoryPack, StoryState, UndoRing, Value, view};

use crate::render::{render_events, render_view};
use crate::script::{ChooseError, UNDO_CAPACITY, choose};

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
        "Órdenes: número de opción, deshacer, estado, mirar, ayuda, salir.\n"
    } else {
        "Commands: choice number, undo, state, look, help, quit.\n"
    }
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
            other => match other.parse::<usize>() {
                Ok(number) => match choose(pack, &state, number) {
                    Ok((next, events)) => {
                        write!(output, "{}", render_events(pack, &events, locale))?;
                        undo.push(std::mem::replace(&mut state, next));
                        write!(
                            output,
                            "{}",
                            render_view(&view(pack, &state).map_err(to_io)?, locale)
                        )?;
                    }
                    Err(ChooseError::Ended) => writeln!(
                        output,
                        "{}",
                        if spanish {
                            "La historia ha terminado: deshacer o salir."
                        } else {
                            "The story has ended: undo or quit."
                        }
                    )?,
                    Err(ChooseError::OutOfRange { .. }) => writeln!(
                        output,
                        "{}",
                        if spanish {
                            "No existe esa opción."
                        } else {
                            "No such choice."
                        }
                    )?,
                    Err(ChooseError::Story(error)) => writeln!(output, "error: {error}")?,
                },
                Err(_) => writeln!(
                    output,
                    "{}",
                    if spanish {
                        "No entiendo. Escribe «ayuda»."
                    } else {
                        "Unknown command. Type \"help\"."
                    }
                )?,
            },
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
