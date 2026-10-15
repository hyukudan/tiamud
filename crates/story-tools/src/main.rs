use std::{
    fs::{self, OpenOptions},
    io::{self, Write},
    path::{Path, PathBuf},
    process::ExitCode,
};

use clap::{Parser, Subcommand};
use thiserror::Error;
use tiamud_story_core::{MAX_ID_BYTES, StoryState, view};
use tiamud_story_tools::{
    STORY_PACK_SCHEMA, ToolError,
    format::canonical_json,
    lint::lint,
    load_pack,
    play::play,
    read_bounded,
    render::render_view,
    script::{MAX_SCRIPT_BYTES, ScriptError, StoryScript, run_script},
};

#[derive(Debug, Parser)]
#[command(
    name = "tiamud-story",
    version,
    about = "Validate, lint, preview, test, format and play Tiamud StoryPacks offline"
)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Debug, Subcommand)]
enum Command {
    /// Validate a StoryPack and report core warnings plus authoring lints.
    Validate {
        pack: PathBuf,
        /// Exit with failure when any warning or lint is reported.
        #[arg(long)]
        deny_warnings: bool,
    },
    /// Render one node as a player would see it, with numbered choices.
    Preview {
        pack: PathBuf,
        /// Node to render; defaults to the start node. Conditions are
        /// evaluated against the initial state.
        #[arg(long)]
        node: Option<String>,
        #[arg(long, default_value = "es")]
        locale: String,
        #[arg(long, default_value_t = 0)]
        seed: u64,
    },
    /// Replay a JSON test script with a fixed seed and check its expectations.
    Test {
        pack: PathBuf,
        #[arg(long)]
        script: PathBuf,
    },
    /// Play a StoryPack interactively on standard input and output.
    Play {
        pack: PathBuf,
        #[arg(long, default_value_t = 0)]
        seed: u64,
        #[arg(long, default_value = "es")]
        locale: String,
    },
    /// Print the canonical form of a StoryPack, check it, or write it.
    Fmt {
        pack: PathBuf,
        /// Fail when the file is not already canonical; write nothing.
        #[arg(long, conflicts_with = "output")]
        check: bool,
        /// Write the canonical form to this path instead of standard output.
        #[arg(short, long)]
        output: Option<PathBuf>,
        /// Replace an existing output file.
        #[arg(long, requires = "output")]
        force: bool,
    },
    /// Print the published JSON Schema of StoryPack format v1.
    Schema,
}

#[derive(Debug, Error)]
enum CliError {
    #[error(transparent)]
    Tool(#[from] ToolError),
    #[error("{0} warning(s) reported and --deny-warnings is set")]
    Warnings(usize),
    #[error("node must be a valid id of at most {MAX_ID_BYTES} bytes")]
    BadNode,
    #[error("unknown node {0:?}")]
    UnknownNode(String),
    #[error("locale must be 2..=35 ASCII letters, digits or '-'")]
    BadLocale,
    #[error("story error: {0}")]
    Story(#[from] tiamud_story_core::StoryError),
    #[error("script {path:?}: {source}")]
    Script { path: PathBuf, source: ScriptError },
    #[error("{0:?} is not in canonical form; run `tiamud-story fmt` to rewrite it")]
    NotCanonical(PathBuf),
    #[error("output {0:?} already exists and will not be replaced")]
    OutputExists(PathBuf),
    #[error("could not write {path:?}: {source}")]
    Write { path: PathBuf, source: io::Error },
    #[error("could not serialize the pack: {0}")]
    Serialize(#[from] serde_json::Error),
    #[error("terminal I/O failed: {0}")]
    Io(#[from] io::Error),
}

fn main() -> ExitCode {
    match run(Cli::parse()) {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("error: {error}");
            ExitCode::FAILURE
        }
    }
}

fn check_locale(locale: &str) -> Result<(), CliError> {
    let valid = (2..=35).contains(&locale.len())
        && locale.starts_with(|c: char| c.is_ascii_alphabetic())
        && locale
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-');
    if valid {
        Ok(())
    } else {
        Err(CliError::BadLocale)
    }
}

fn run(cli: Cli) -> Result<(), CliError> {
    match cli.command {
        Command::Validate {
            pack: path,
            deny_warnings,
        } => {
            let (pack, report) = load_pack(&path)?;
            let lints = lint(&pack);
            for warning in &report.warnings {
                match node_id_for_path(&pack, &warning.path) {
                    Some(id) => println!("warning: {} ({id}): {}", warning.path, warning.message),
                    None => println!("warning: {warning}"),
                }
            }
            if report.truncated {
                println!("warning: (more core warnings omitted)");
            }
            for item in &lints.lints {
                println!("lint: {item}");
            }
            if lints.truncated {
                println!("lint: (more lints omitted)");
            }
            let choices: usize = pack.nodes.iter().map(|node| node.choices.len()).sum();
            let findings = report.warnings.len() + lints.lints.len();
            println!(
                "valid StoryPack {:?}: {} node(s), {} choice(s), {} variable(s), {} item(s); {} warning(s), {} lint(s)",
                pack.id,
                pack.nodes.len(),
                choices,
                pack.variables.len(),
                pack.items.len(),
                report.warnings.len(),
                lints.lints.len(),
            );
            if deny_warnings && (findings > 0 || report.truncated || lints.truncated) {
                return Err(CliError::Warnings(findings));
            }
        }
        Command::Preview {
            pack: path,
            node,
            locale,
            seed,
        } => {
            check_locale(&locale)?;
            let (pack, _) = load_pack(&path)?;
            let mut state = StoryState::new(&pack, seed)?;
            if let Some(node_id) = node {
                if node_id.is_empty() || node_id.len() > MAX_ID_BYTES {
                    return Err(CliError::BadNode);
                }
                let target = pack
                    .nodes
                    .iter()
                    .find(|candidate| candidate.id == node_id)
                    .ok_or(CliError::UnknownNode(node_id))?;
                state.node = target.id.clone();
                state.visited.insert(target.id.clone());
                state.ended = target.ending.clone();
            }
            let scene = view(&pack, &state)?;
            print!("{}", render_view(&scene, &locale));
            if scene.ending.is_none() {
                let authored = pack
                    .nodes
                    .iter()
                    .find(|candidate| candidate.id == state.node)
                    .map_or(0, |current| current.choices.len());
                let hidden = authored.saturating_sub(scene.choices.len());
                if hidden > 0 {
                    println!("({hidden} choice(s) hidden by conditions in the initial state)");
                }
            }
        }
        Command::Test {
            pack: path,
            script: script_path,
        } => {
            let (pack, _) = load_pack(&path)?;
            let bytes = read_bounded(&script_path, MAX_SCRIPT_BYTES)?;
            let script_error = |source| CliError::Script {
                path: script_path.clone(),
                source,
            };
            let script = StoryScript::from_json_slice(&bytes).map_err(script_error)?;
            let report = run_script(&pack, &script).map_err(script_error)?;
            println!(
                "story test passed: {} step(s), {} choice(s), {} expectation(s); final node {:?} at turn {}",
                report.steps,
                report.choices,
                report.expectations,
                report.final_state.node,
                report.final_state.turn,
            );
        }
        Command::Play {
            pack: path,
            seed,
            locale,
        } => {
            check_locale(&locale)?;
            let (pack, _) = load_pack(&path)?;
            let stdin = io::stdin();
            let stdout = io::stdout();
            play(&pack, seed, &locale, stdin.lock(), stdout.lock())?;
        }
        Command::Fmt {
            pack: path,
            check,
            output,
            force,
        } => {
            let (pack, _) = load_pack(&path)?;
            let canonical = canonical_json(&pack)?;
            if check {
                let current = read_bounded(&path, tiamud_story_core::MAX_PACK_BYTES)?;
                if current != canonical {
                    return Err(CliError::NotCanonical(path));
                }
                println!("{path:?} is canonical");
            } else if let Some(output) = output {
                write_atomically(&output, &canonical, force)?;
                println!("wrote canonical StoryPack to {output:?}");
            } else {
                io::stdout().write_all(&canonical)?;
            }
        }
        Command::Schema => print!("{STORY_PACK_SCHEMA}"),
    }
    Ok(())
}

/// Authored id of the node a core warning path such as `nodes[3]` names.
fn node_id_for_path<'a>(pack: &'a tiamud_story_core::StoryPack, path: &str) -> Option<&'a str> {
    let index = path.strip_prefix("nodes[")?.split_once(']')?.0;
    let node = pack.nodes.get(index.parse::<usize>().ok()?)?;
    Some(node.id.as_str())
}

/// Writes through a sibling temporary file and renames it into place, so a
/// reader never observes a partial pack.
fn write_atomically(path: &Path, bytes: &[u8], force: bool) -> Result<(), CliError> {
    let write_error = |source| CliError::Write {
        path: path.to_path_buf(),
        source,
    };
    if !force && fs::symlink_metadata(path).is_ok() {
        return Err(CliError::OutputExists(path.to_path_buf()));
    }
    let file_name = path
        .file_name()
        .ok_or_else(|| write_error(io::Error::other("output has no file name")))?;
    let mut temporary_name = std::ffi::OsString::from(".");
    temporary_name.push(file_name);
    temporary_name.push(format!(".{}.tmp", std::process::id()));
    let temporary = path.with_file_name(temporary_name);
    let result = (|| {
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temporary)?;
        file.write_all(bytes)?;
        file.sync_all()?;
        if force {
            fs::rename(&temporary, path)
        } else {
            // Fails instead of replacing a file created after the check above.
            fs::hard_link(&temporary, path).and_then(|()| fs::remove_file(&temporary))
        }
    })();
    if result.is_err() {
        let _ = fs::remove_file(&temporary);
    }
    result.map_err(write_error)
}
