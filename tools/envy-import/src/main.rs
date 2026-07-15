use std::{env, fs, path::Path};

use envy_import::{ImportDiagnostic, TextEncoding, audit, parse_import};

fn main() {
    if let Err(error) = run() {
        eprintln!("error: {error}");
        std::process::exit(1);
    }
}

fn run() -> Result<(), String> {
    let args: Vec<String> = env::args().skip(1).collect();
    match args.as_slice() {
        [command, path] if command == "audit" => {
            let bytes = fs::read(path).map_err(|error| format!("cannot read {path:?}: {error}"))?;
            let report = audit(&bytes).map_err(|error| error.to_string())?;
            println!(
                "{}",
                serde_json::to_string_pretty(&report).map_err(|error| error.to_string())?
            );
            if !report.stock_order {
                return Err("area does not follow the audited stock section order".to_owned());
            }
            Ok(())
        }
        [command, path, options @ ..] if command == "convert" || command == "convert-report" => {
            let mut pack_id = path_stem(path)?;
            let mut encoding = TextEncoding::Ascii;
            let mut index = 0;
            while index < options.len() {
                match options[index].as_str() {
                    "--id" => {
                        index += 1;
                        pack_id = options
                            .get(index)
                            .ok_or_else(|| "--id requires a value".to_owned())?
                            .clone();
                    }
                    "--encoding" => {
                        index += 1;
                        encoding = match options.get(index).map(String::as_str) {
                            Some("ascii") => TextEncoding::Ascii,
                            Some("utf-8") => TextEncoding::Utf8,
                            Some(value) => {
                                return Err(format!(
                                    "unsupported encoding {value:?}; use ascii or utf-8"
                                ));
                            }
                            None => return Err("--encoding requires a value".to_owned()),
                        };
                    }
                    option => return Err(format!("unknown option {option:?}")),
                }
                index += 1;
            }

            let bytes = fs::read(path).map_err(|error| format!("cannot read {path:?}: {error}"))?;
            if command == "convert-report" {
                let report = parse_import(&bytes, &pack_id, Some(path.clone()), encoding)
                    .map_err(|error| error.to_string())?;
                println!(
                    "{}",
                    serde_json::to_string_pretty(&report).map_err(|error| error.to_string())?
                );
            } else {
                let report = parse_import(&bytes, &pack_id, Some(path.clone()), encoding)
                    .map_err(|error| error.to_string())?;
                emit_diagnostics(&report.diagnostics);
                println!(
                    "{}",
                    serde_json::to_string_pretty(&report.world_pack)
                        .map_err(|error| error.to_string())?
                );
            }
            Ok(())
        }
        _ => Err(usage()),
    }
}

fn emit_diagnostics(diagnostics: &[ImportDiagnostic]) {
    for diagnostic in diagnostics {
        let line = diagnostic
            .line
            .map(|line| format!(" line {line}"))
            .unwrap_or_default();
        let vnum = diagnostic
            .vnum
            .map(|vnum| format!(" VNUM {vnum}"))
            .unwrap_or_default();
        eprintln!(
            "{:?} [{}{}{}] {} Recommendation: {}",
            diagnostic.severity,
            diagnostic.section,
            line,
            vnum,
            diagnostic.message,
            diagnostic.recommendation
        );
    }
}

fn path_stem(path: &str) -> Result<String, String> {
    let stem = Path::new(path)
        .file_stem()
        .and_then(|stem| stem.to_str())
        .ok_or_else(|| format!("cannot derive a pack id from {path:?}; pass --id"))?;
    Ok(stem
        .to_ascii_lowercase()
        .chars()
        .map(|character| {
            if character.is_ascii_lowercase() || character.is_ascii_digit() {
                character
            } else {
                '-'
            }
        })
        .collect())
}

fn usage() -> String {
    "usage:\n  envy-import audit <area.are>\n  envy-import convert <area.are> [--id pack-id] [--encoding ascii|utf-8]\n  envy-import convert-report <area.are> [--id pack-id] [--encoding ascii|utf-8]".to_owned()
}
