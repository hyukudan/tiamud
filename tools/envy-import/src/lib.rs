//! A conservative offline reader for sectioned historical EnvyMUD/Merc-family
//! area files, with stock EnvyMUD 2.2 ordering as its audited baseline.
//!
//! It is intentionally an offline conversion tool. It never executes specials,
//! scripts, or any other content found in an area file.

use std::{
    collections::{HashMap, HashSet},
    fmt::Write as _,
};

use serde::Serialize;
use sha2::{Digest, Sha256};
use thiserror::Error;
use world_pack::{
    CURRENT_SCHEMA_VERSION, Direction, Exit, ExitTarget, LegacyBitvector, LegacyRoomData, Room,
    SourceMetadata, WorldPack, WorldPackDiagnostic,
};

mod legacy;

pub use legacy::{
    AreaMetadata, DiagnosticSeverity, ImportDiagnostic, ImportProvenance, ImportResult,
    LegacyContent, LegacyDataLine, LegacyHelp, LegacyMobile, LegacyObject, LegacyProgram,
    LegacyReset, LegacyShop, LegacySpecial, LicenseReviewStatus, ProgramOwner,
};

const MAX_INPUT_BYTES: usize = 32 * 1024 * 1024;
const MAX_STRING_BYTES: usize = 1024 * 1024;
const MAX_LINE_BYTES: usize = 1024 * 1024;
const MAX_ROOMS: usize = 100_000;

pub const STOCK_SECTION_ORDER: [&str; 10] = [
    "AREA", "HELPS", "RECALL", "MOBILES", "OBJECTS", "ROOMS", "RESETS", "SHOPS", "SPECIALS", "$",
];
const SUPPORTED_EXTENSION_SECTIONS: [&str; 4] = ["MOBPROGS", "OBJPROGS", "ROOMPROGS", "PROGS"];

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum TextEncoding {
    Ascii,
    Utf8,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AuditReport {
    pub input_bytes: usize,
    pub stock_order: bool,
    pub sections: Vec<SectionOccurrence>,
    pub issues: Vec<AuditIssue>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SectionOccurrence {
    pub name: String,
    pub line: usize,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AuditIssue {
    pub code: &'static str,
    pub line: Option<usize>,
    pub message: String,
}

pub fn audit(bytes: &[u8]) -> Result<AuditReport, ImportError> {
    check_input_size(bytes)?;
    let mut sections = Vec::new();
    let mut additional_issues = Vec::new();
    let mut extension_sections = HashSet::new();

    for (index, line) in byte_lines(bytes).enumerate() {
        let trimmed = trim_ascii(line);
        let Some(name) = trimmed.strip_prefix(b"#") else {
            continue;
        };
        if STOCK_SECTION_ORDER
            .iter()
            .any(|known| name == known.as_bytes())
        {
            sections.push(SectionOccurrence {
                name: String::from_utf8_lossy(name).into_owned(),
                line: index + 1,
            });
        } else if SUPPORTED_EXTENSION_SECTIONS
            .iter()
            .any(|known| name == known.as_bytes())
        {
            let name = String::from_utf8_lossy(name).into_owned();
            if !extension_sections.insert(name.clone()) {
                additional_issues.push(AuditIssue {
                    code: "duplicate_extension_section",
                    line: Some(index + 1),
                    message: format!("supported extension section #{name} appears more than once"),
                });
            }
        } else if is_named_section(name) {
            additional_issues.push(AuditIssue {
                code: "unknown_section",
                line: Some(index + 1),
                message: format!(
                    "section #{} is not part of the audited stock or supported extension set",
                    String::from_utf8_lossy(name)
                ),
            });
        }
    }

    let mut issues = Vec::new();
    let mut seen = HashSet::new();
    let mut previous_index = None;
    for section in &sections {
        let index = STOCK_SECTION_ORDER
            .iter()
            .position(|expected| *expected == section.name)
            .expect("only known section names are collected");
        if !seen.insert(section.name.as_str()) {
            issues.push(AuditIssue {
                code: "duplicate_section",
                line: Some(section.line),
                message: format!("section #{} appears more than once", section.name),
            });
        }
        if let Some(previous) = previous_index
            && index <= previous
        {
            issues.push(AuditIssue {
                code: "section_out_of_order",
                line: Some(section.line),
                message: format!("section #{} is not in stock Envy 2.2 order", section.name),
            });
        }
        previous_index = Some(index);
    }

    for expected in STOCK_SECTION_ORDER {
        if !seen.contains(expected) {
            issues.push(AuditIssue {
                code: "missing_section",
                line: None,
                message: format!("stock section #{expected} is missing"),
            });
        }
    }

    issues.extend(additional_issues);
    let stock_order = !issues.iter().any(|issue| {
        matches!(
            issue.code,
            "duplicate_section"
                | "section_out_of_order"
                | "missing_section"
                | "duplicate_extension_section"
        )
    });

    Ok(AuditReport {
        input_bytes: bytes.len(),
        stock_order,
        sections,
        issues,
    })
}

fn is_named_section(name: &[u8]) -> bool {
    !name.is_empty()
        && name
            .iter()
            .all(|byte| byte.is_ascii_uppercase() || byte.is_ascii_digit() || *byte == b'_')
        && name.iter().any(u8::is_ascii_uppercase)
}

pub fn parse_world_pack(
    bytes: &[u8],
    pack_id: &str,
    source_file: Option<String>,
    encoding: TextEncoding,
) -> Result<WorldPack, ImportError> {
    parse_import(bytes, pack_id, source_file, encoding).map(|result| result.world_pack)
}

/// Parses a historical area into a validated WorldPack plus a loss-aware,
/// inert compatibility report.
///
/// The compatibility data is never interpreted as executable code. Callers
/// can use the diagnostics to decide which constructs need manual authoring in
/// Tiamud instead of silently accepting a lossy conversion.
pub fn parse_import(
    bytes: &[u8],
    pack_id: &str,
    source_file: Option<String>,
    encoding: TextEncoding,
) -> Result<ImportResult, ImportError> {
    check_input_size(bytes)?;
    validate_pack_id(pack_id)?;
    validate_text_encoding(bytes, encoding)?;
    let audit = audit(bytes)?;
    if !audit.stock_order {
        return Err(ImportError::AuditFailed(audit.issues));
    }

    let area_range = section_body(bytes, "AREA", "HELPS")?;
    let mut area = Cursor::new(
        &bytes[area_range.clone()],
        line_number(bytes, area_range.start),
    );
    let area_name = decode(area.tilde_string()?, encoding, area.line())?
        .trim()
        .to_owned();

    let rooms_range = section_body(bytes, "ROOMS", "RESETS")?;
    let mut cursor = Cursor::new(
        &bytes[rooms_range.clone()],
        line_number(bytes, rooms_range.start),
    );
    let mut rooms = Vec::new();
    let mut room_programs = Vec::new();
    let mut room_diagnostics = Vec::new();
    let mut external_exit_references = Vec::new();
    let mut seen_room_vnums = HashSet::new();
    let mut room_source_lines = HashMap::new();
    while !cursor.is_finished() {
        cursor.skip_ascii_whitespace();
        if cursor.is_finished() {
            break;
        }
        let header_line = cursor.line();
        let header = cursor.line_bytes()?;
        let vnum = parse_record_header(header, header_line)?;
        if vnum == 0 {
            break;
        }
        if !seen_room_vnums.insert(vnum) {
            return Err(ImportError::Syntax {
                line: header_line,
                message: format!("duplicate room VNUM {vnum}"),
            });
        }
        room_source_lines.insert(vnum, header_line);
        if rooms.len() >= MAX_ROOMS {
            return Err(ImportError::Limit {
                line: header_line,
                message: format!("room count exceeds {MAX_ROOMS}"),
            });
        }

        let title_line = cursor.line();
        let title = decode(cursor.tilde_string()?, encoding, title_line)?
            .trim()
            .to_owned();
        let description_line = cursor.line();
        let description = decode(cursor.tilde_string()?, encoding, description_line)?
            .trim()
            .to_owned();
        let metadata_line_number = cursor.line();
        let (prefix_values, flags, sector) =
            parse_room_metadata(cursor.line_bytes()?, metadata_line_number)?;
        match &flags {
            LegacyBitvector::Numeric(value) if *value != 0 => {
                room_diagnostics.push(import_warning(
                    "room_flag_mask_preserved",
                    "ROOMS",
                    Some(metadata_line_number),
                    Some(vnum),
                    &format!("room flag mask {value} is preserved but not interpreted"),
                    "review the source dialect's room flag table and author equivalent Tiamud behavior explicitly",
                ));
            }
            LegacyBitvector::Alphabetic(value) => {
                room_diagnostics.push(import_warning(
                    "rom_alphabetic_room_bitvector_preserved",
                    "ROOMS",
                    Some(metadata_line_number),
                    Some(vnum),
                    &format!(
                        "ROM-style room bitvector {value:?} is preserved verbatim and was not converted to a numeric mask"
                    ),
                    "review the exact source flag table and author equivalent Tiamud behavior explicitly",
                ));
            }
            LegacyBitvector::Numeric(_) => {}
        }
        if !(0..=10).contains(&sector) {
            room_diagnostics.push(import_warning(
                "unknown_room_sector_descriptor",
                "ROOMS",
                Some(metadata_line_number),
                Some(vnum),
                &format!(
                    "room sector descriptor {sector} is outside the audited stock range 0..=10"
                ),
                "verify the source codebase's sector table before mapping this room's environment",
            ));
        }

        let mut exits = Vec::new();
        let mut seen_directions = HashMap::new();
        loop {
            cursor.skip_blank_lines();
            let directive_line = cursor.line();
            let directive = trim_ascii(cursor.line_bytes()?);
            if directive == b"S" {
                break;
            }
            if directive == b"E" {
                cursor.tilde_string()?;
                cursor.tilde_string()?;
                continue;
            }
            if directive == b"|" {
                continue;
            }
            if directive.starts_with(b">") {
                room_programs.push(legacy::parse_consumed_program_header(
                    directive,
                    directive_line,
                    &mut cursor,
                    encoding,
                    ProgramOwner::Room { vnum },
                )?);
                continue;
            }
            if let Some(number) = directive.strip_prefix(b"D") {
                let direction_number = parse_i64(number, directive_line)?;
                let direction = parse_direction(direction_number, directive_line)?;
                if let Some(first_line) = seen_directions.insert(direction, directive_line) {
                    return Err(ImportError::Syntax {
                        line: directive_line,
                        message: format!(
                            "room {vnum} repeats direction D{direction_number}; first declared on line {first_line}"
                        ),
                    });
                }
                let description_line = cursor.line();
                let exit_description = decode(cursor.tilde_string()?, encoding, description_line)?
                    .trim()
                    .to_owned();
                let keywords_line = cursor.line();
                let keywords = decode(cursor.tilde_string()?, encoding, keywords_line)?
                    .trim()
                    .to_owned();
                let values_line = cursor.line();
                let values = parse_integer_line(cursor.line_bytes()?, values_line)?;
                let Some((&target_vnum, legacy_values)) = values.split_last() else {
                    return Err(ImportError::Syntax {
                        line: values_line,
                        message: "exit data requires a target VNUM".to_owned(),
                    });
                };
                if !legacy_values.is_empty() {
                    room_diagnostics.push(import_warning(
                        "exit_descriptors_preserved",
                        "ROOMS",
                        Some(values_line),
                        Some(vnum),
                        &format!(
                            "exit D{direction_number} descriptor values {legacy_values:?} are preserved but not interpreted"
                        ),
                        "review door flags and key semantics against the source dialect before authoring the exit",
                    ));
                }
                external_exit_references.push((values_line, vnum, target_vnum));
                exits.push(Exit {
                    direction,
                    target: ExitTarget::LegacyVnum { vnum: target_vnum },
                    description: exit_description,
                    keywords,
                    legacy_values: legacy_values.to_vec(),
                });
                continue;
            }
            return Err(ImportError::Syntax {
                line: directive_line,
                message: format!(
                    "unsupported stock room directive {:?}",
                    String::from_utf8_lossy(directive)
                ),
            });
        }

        rooms.push(Room {
            id: format!("{pack_id}:room:{vnum}"),
            legacy_vnum: Some(vnum),
            title,
            description,
            asset_manifest: None,
            exits,
            items: Vec::new(),
            legacy: Some(LegacyRoomData {
                flags,
                sector,
                prefix_values,
            }),
        });
    }

    resolve_local_exits(&mut rooms);
    for (line, source_vnum, target_vnum) in external_exit_references {
        if !seen_room_vnums.contains(&target_vnum) {
            room_diagnostics.push(import_warning(
                "external_or_missing_exit_target",
                "ROOMS",
                Some(line),
                Some(target_vnum),
                &format!(
                    "room {source_vnum} exits to VNUM {target_vnum}, which is not present in this area"
                ),
                "compose the owning area explicitly or repair the target VNUM before publication",
            ));
        }
    }
    let start_room_id = rooms
        .first()
        .map(|room| room.id.clone())
        .ok_or_else(|| ImportError::Validation("the area contains no rooms".to_owned()))?;
    let provenance = ImportProvenance {
        source_file: source_file.clone(),
        source_format: "envymud-area".to_owned(),
        dialect: "envymud-2.2-stock".to_owned(),
        text_encoding: encoding,
        input_bytes: bytes.len(),
        input_sha256: sha256_hex(bytes),
        license_status: LicenseReviewStatus::UndeterminedReviewRequired,
    };
    let mut pack = WorldPack {
        schema_version: CURRENT_SCHEMA_VERSION,
        id: pack_id.to_owned(),
        name: area_name,
        start_room_id,
        source: Some(SourceMetadata {
            format: "envymud-area".to_owned(),
            dialect: Some("envymud-2.2-stock".to_owned()),
            file: source_file,
        }),
        items: Vec::new(),
        rooms,
        gameplay: world_pack::GameplayDefinition::default(),
    };
    let (legacy, mut diagnostics) =
        legacy::parse_and_translate(bytes, pack_id, encoding, &mut pack, room_programs)?;
    diagnostics.extend(room_diagnostics);
    diagnostics.extend(
        audit
            .issues
            .iter()
            .filter(|issue| issue.code == "unknown_section")
            .map(|issue| {
                import_warning(
                    "unknown_section_preserved_or_ignored",
                    "AREA",
                    issue.line,
                    None,
                    &issue.message,
                    "identify the source dialect and add a reviewed importer before relying on this section",
                )
            }),
    );
    diagnostics.push(import_warning(
        "source_license_review_required",
        "AREA",
        None,
        None,
        "the importer cannot determine the copyright holder or license of historical content",
        "record authoritative provenance and obtain a human legal/license decision before distributing imported content",
    ));
    diagnostics.extend(pack.diagnostics().into_iter().map(|diagnostic| match diagnostic {
        WorldPackDiagnostic::DirectedUnreachableRoom {
            room_id,
            start_room_id,
        } => {
            let vnum = pack
                .rooms
                .iter()
                .find(|room| room.id == room_id)
                .and_then(|room| room.legacy_vnum);
            let line = vnum.and_then(|vnum| room_source_lines.get(&vnum).copied());
            import_warning(
                "directed_unreachable_room_preserved",
                "ROOMS",
                line,
                vnum,
                &format!(
                    "room {room_id:?} is not reachable through directed exits from {start_room_id:?}"
                ),
                "review one-way exits, teleportation, scripts, and cross-area travel before deciding whether this room is intentionally isolated",
            )
        }
    }));
    pack.validate()
        .map_err(|error| ImportError::Validation(error.to_string()))?;
    Ok(ImportResult {
        world_pack: pack,
        legacy,
        diagnostics,
        provenance,
    })
}

fn import_warning(
    code: &'static str,
    section: &'static str,
    line: Option<usize>,
    vnum: Option<i64>,
    message: &str,
    recommendation: &str,
) -> ImportDiagnostic {
    ImportDiagnostic {
        severity: DiagnosticSeverity::Warning,
        code,
        section,
        line,
        vnum,
        message: message.to_owned(),
        recommendation: recommendation.to_owned(),
    }
}

fn resolve_local_exits(rooms: &mut [Room]) {
    let room_ids: HashMap<i64, String> = rooms
        .iter()
        .filter_map(|room| room.legacy_vnum.map(|vnum| (vnum, room.id.clone())))
        .collect();
    for room in rooms {
        for exit in &mut room.exits {
            let ExitTarget::LegacyVnum { vnum } = exit.target else {
                continue;
            };
            if let Some(room_id) = room_ids.get(&vnum) {
                exit.target = ExitTarget::Room {
                    room_id: room_id.clone(),
                };
            }
        }
    }
}

fn section_body(
    bytes: &[u8],
    start: &str,
    end: &str,
) -> Result<std::ops::Range<usize>, ImportError> {
    let start_marker = format!("#{start}");
    let end_marker = format!("#{end}");
    let start =
        find_marker_line(bytes, start_marker.as_bytes()).ok_or_else(|| ImportError::Syntax {
            line: 1,
            message: format!("section #{start} was not found"),
        })?;
    let content_start = line_end(bytes, start);
    let relative_end = find_marker_line(&bytes[content_start..], end_marker.as_bytes())
        .ok_or_else(|| ImportError::Syntax {
            line: line_number(bytes, content_start),
            message: format!("section #{end} was not found after #{start}"),
        })?;
    Ok(content_start..content_start + relative_end)
}

fn find_marker_line(bytes: &[u8], marker: &[u8]) -> Option<usize> {
    let mut offset = 0;
    for line in byte_lines(bytes) {
        if trim_ascii(line) == marker {
            return Some(offset);
        }
        offset += line.len();
        if bytes.get(offset) == Some(&b'\r') {
            offset += 1;
        }
        if bytes.get(offset) == Some(&b'\n') {
            offset += 1;
        }
    }
    None
}

fn line_end(bytes: &[u8], start: usize) -> usize {
    let mut index = start;
    while index < bytes.len() && bytes[index] != b'\n' {
        index += 1;
    }
    (index + usize::from(index < bytes.len())).min(bytes.len())
}

fn line_number(bytes: &[u8], offset: usize) -> usize {
    bytes[..offset.min(bytes.len())]
        .iter()
        .filter(|byte| **byte == b'\n')
        .count()
        + 1
}

fn parse_record_header(bytes: &[u8], line: usize) -> Result<i64, ImportError> {
    let trimmed = trim_ascii(bytes);
    let number = trimmed
        .strip_prefix(b"#")
        .ok_or_else(|| ImportError::Syntax {
            line,
            message: "expected a room header such as #3001".to_owned(),
        })?;
    parse_i64(number, line)
}

fn parse_integer_line(bytes: &[u8], line: usize) -> Result<Vec<i64>, ImportError> {
    let text = std::str::from_utf8(trim_ascii(bytes)).map_err(|_| ImportError::Syntax {
        line,
        message: "numeric data contains non-ASCII bytes".to_owned(),
    })?;
    if text.is_empty() {
        return Err(ImportError::Syntax {
            line,
            message: "expected numeric data".to_owned(),
        });
    }
    text.split_ascii_whitespace()
        .map(|part| parse_legacy_number(part.as_bytes(), line))
        .collect()
}

fn parse_room_metadata(
    bytes: &[u8],
    line: usize,
) -> Result<(Vec<i64>, LegacyBitvector, i64), ImportError> {
    let text = std::str::from_utf8(trim_ascii(bytes)).map_err(|_| ImportError::Syntax {
        line,
        message: "room metadata contains non-ASCII bytes".to_owned(),
    })?;
    let tokens = text.split_ascii_whitespace().collect::<Vec<_>>();
    if tokens.len() < 2 {
        return Err(ImportError::Syntax {
            line,
            message: "room metadata requires at least flags and sector values".to_owned(),
        });
    }
    let flags = parse_legacy_bitvector(tokens[tokens.len() - 2], line)?;
    let sector = parse_legacy_number(tokens[tokens.len() - 1].as_bytes(), line)?;
    let prefix_values = tokens[..tokens.len() - 2]
        .iter()
        .map(|value| parse_legacy_number(value.as_bytes(), line))
        .collect::<Result<Vec<_>, _>>()?;
    Ok((prefix_values, flags, sector))
}

pub(crate) fn parse_legacy_bitvector(
    token: &str,
    line: usize,
) -> Result<LegacyBitvector, ImportError> {
    if !token.is_empty() && token.bytes().all(|byte| byte.is_ascii_alphabetic()) {
        Ok(LegacyBitvector::Alphabetic(token.to_owned()))
    } else {
        parse_legacy_number(token.as_bytes(), line).map(LegacyBitvector::Numeric)
    }
}

fn parse_legacy_number(bytes: &[u8], line: usize) -> Result<i64, ImportError> {
    let mut value = 0_i64;
    let mut found_component = false;
    for component in bytes.split(|byte| *byte == b'|') {
        if component.is_empty() {
            return Err(ImportError::Syntax {
                line,
                message: format!("invalid legacy number {:?}", String::from_utf8_lossy(bytes)),
            });
        }
        value = value
            .checked_add(parse_i64(component, line)?)
            .ok_or_else(|| ImportError::Syntax {
                line,
                message: "legacy numeric expression overflows a signed 64-bit integer".to_owned(),
            })?;
        found_component = true;
    }
    if found_component {
        Ok(value)
    } else {
        Err(ImportError::Syntax {
            line,
            message: "expected a legacy number".to_owned(),
        })
    }
}

fn parse_i64(bytes: &[u8], line: usize) -> Result<i64, ImportError> {
    let text = std::str::from_utf8(trim_ascii(bytes)).map_err(|_| ImportError::Syntax {
        line,
        message: "integer contains non-ASCII bytes".to_owned(),
    })?;
    text.parse::<i64>().map_err(|_| ImportError::Syntax {
        line,
        message: format!("invalid integer {text:?}"),
    })
}

fn parse_direction(value: i64, line: usize) -> Result<Direction, ImportError> {
    match value {
        0 => Ok(Direction::North),
        1 => Ok(Direction::East),
        2 => Ok(Direction::South),
        3 => Ok(Direction::West),
        4 => Ok(Direction::Up),
        5 => Ok(Direction::Down),
        _ => Err(ImportError::Syntax {
            line,
            message: format!("unsupported stock direction D{value}"),
        }),
    }
}

fn decode(bytes: &[u8], encoding: TextEncoding, line: usize) -> Result<String, ImportError> {
    match encoding {
        TextEncoding::Ascii => {
            if bytes.iter().any(|byte| !byte.is_ascii()) {
                return Err(ImportError::Encoding {
                    line,
                    message: "non-ASCII input; select UTF-8 explicitly or transcode offline"
                        .to_owned(),
                });
            }
            Ok(String::from_utf8(bytes.to_vec()).expect("ASCII is UTF-8"))
        }
        TextEncoding::Utf8 => std::str::from_utf8(bytes)
            .map(ToOwned::to_owned)
            .map_err(|error| ImportError::Encoding {
                line,
                message: format!("invalid UTF-8: {error}"),
            }),
    }
}

fn validate_text_encoding(bytes: &[u8], encoding: TextEncoding) -> Result<(), ImportError> {
    if bytes.starts_with(&[0xef, 0xbb, 0xbf]) {
        return Err(ImportError::Encoding {
            line: 1,
            message: "UTF-8 BOM detected; remove the BOM so #AREA is the first byte".to_owned(),
        });
    }
    match encoding {
        TextEncoding::Ascii => {
            let Some(offset) = bytes.iter().position(|byte| !byte.is_ascii()) else {
                return Ok(());
            };
            let line = line_number(bytes, offset);
            let line_start = bytes[..offset]
                .iter()
                .rposition(|byte| *byte == b'\n')
                .map_or(0, |position| position + 1);
            let column = offset - line_start + 1;
            let hint = if std::str::from_utf8(bytes).is_ok() {
                "the file is valid UTF-8; select --encoding utf-8 explicitly"
            } else {
                "the file may use a legacy single-byte encoding; transcode it offline to UTF-8"
            };
            Err(ImportError::Encoding {
                line,
                message: format!(
                    "non-ASCII byte 0x{:02x} at byte offset {offset}, column {column}; {hint}",
                    bytes[offset]
                ),
            })
        }
        TextEncoding::Utf8 => std::str::from_utf8(bytes)
            .map(|_| ())
            .map_err(|error| {
                let offset = error.valid_up_to();
                let line = line_number(bytes, offset);
                let line_start = bytes[..offset]
                    .iter()
                    .rposition(|byte| *byte == b'\n')
                    .map_or(0, |position| position + 1);
                let column = offset - line_start + 1;
                ImportError::Encoding {
                    line,
                    message: format!(
                        "invalid UTF-8 at byte offset {offset}, column {column}: {error}; transcode the source offline with an explicit legacy codec"
                    ),
                }
            }),
    }
}

fn sha256_hex(bytes: &[u8]) -> String {
    let digest = Sha256::digest(bytes);
    let mut hex = String::with_capacity(64);
    for byte in digest {
        write!(&mut hex, "{byte:02x}").expect("writing to String cannot fail");
    }
    hex
}

fn validate_pack_id(pack_id: &str) -> Result<(), ImportError> {
    let valid = !pack_id.is_empty()
        && pack_id.bytes().all(|byte| {
            byte.is_ascii_lowercase() || byte.is_ascii_digit() || b"-_".contains(&byte)
        });
    if valid {
        Ok(())
    } else {
        Err(ImportError::InvalidPackId(pack_id.to_owned()))
    }
}

fn check_input_size(bytes: &[u8]) -> Result<(), ImportError> {
    if bytes.len() <= MAX_INPUT_BYTES {
        Ok(())
    } else {
        Err(ImportError::InputTooLarge {
            bytes: bytes.len(),
            maximum: MAX_INPUT_BYTES,
        })
    }
}

fn byte_lines(bytes: &[u8]) -> impl Iterator<Item = &[u8]> {
    bytes.split(|byte| *byte == b'\n').map(|line| {
        if line.last() == Some(&b'\r') {
            &line[..line.len() - 1]
        } else {
            line
        }
    })
}

fn trim_ascii(mut bytes: &[u8]) -> &[u8] {
    while bytes.first().is_some_and(u8::is_ascii_whitespace) {
        bytes = &bytes[1..];
    }
    while bytes.last().is_some_and(u8::is_ascii_whitespace) {
        bytes = &bytes[..bytes.len() - 1];
    }
    bytes
}

struct Cursor<'a> {
    bytes: &'a [u8],
    position: usize,
    line: usize,
}

impl<'a> Cursor<'a> {
    fn new(bytes: &'a [u8], starting_line: usize) -> Self {
        Self {
            bytes,
            position: 0,
            line: starting_line,
        }
    }

    fn line(&self) -> usize {
        self.line
    }

    fn is_finished(&self) -> bool {
        self.position >= self.bytes.len()
    }

    fn skip_ascii_whitespace(&mut self) {
        while let Some(byte) = self.bytes.get(self.position)
            && byte.is_ascii_whitespace()
        {
            if *byte == b'\n' {
                self.line += 1;
            }
            self.position += 1;
        }
    }

    fn skip_blank_lines(&mut self) {
        loop {
            let start = self.position;
            while matches!(self.bytes.get(self.position), Some(b' ' | b'\t' | b'\r')) {
                self.position += 1;
            }
            if self.bytes.get(self.position) == Some(&b'\n') {
                self.position += 1;
                self.line += 1;
            } else {
                self.position = start;
                break;
            }
        }
    }

    fn line_bytes(&mut self) -> Result<&'a [u8], ImportError> {
        if self.is_finished() {
            return Err(ImportError::UnexpectedEof { line: self.line });
        }
        let start = self.position;
        while self.position < self.bytes.len() && self.bytes[self.position] != b'\n' {
            self.position += 1;
            if self.position - start > MAX_LINE_BYTES {
                return Err(ImportError::Limit {
                    line: self.line,
                    message: format!("line exceeds {MAX_LINE_BYTES} bytes"),
                });
            }
        }
        let mut end = self.position;
        if end > start && self.bytes[end - 1] == b'\r' {
            end -= 1;
        }
        if self.position < self.bytes.len() {
            self.position += 1;
            self.line += 1;
        }
        Ok(&self.bytes[start..end])
    }

    fn peek_line_bytes(&self) -> &'a [u8] {
        let start = self.position;
        let mut end = start;
        while end < self.bytes.len() && self.bytes[end] != b'\n' {
            end += 1;
        }
        if end > start && self.bytes[end - 1] == b'\r' {
            end -= 1;
        }
        &self.bytes[start..end]
    }

    fn tilde_string(&mut self) -> Result<&'a [u8], ImportError> {
        let start = self.position;
        let start_line = self.line;
        while self.position < self.bytes.len() {
            match self.bytes[self.position] {
                b'~' => {
                    let end = self.position;
                    self.position += 1;
                    if self.bytes.get(self.position) == Some(&b'\r') {
                        self.position += 1;
                    }
                    if self.bytes.get(self.position) == Some(&b'\n') {
                        self.position += 1;
                        self.line += 1;
                    }
                    return Ok(&self.bytes[start..end]);
                }
                b'\n' => self.line += 1,
                _ => {}
            }
            self.position += 1;
            if self.position - start > MAX_STRING_BYTES {
                return Err(ImportError::Limit {
                    line: start_line,
                    message: format!("tilde string exceeds {MAX_STRING_BYTES} bytes"),
                });
            }
        }
        Err(ImportError::Syntax {
            line: start_line,
            message: "unterminated tilde string".to_owned(),
        })
    }
}

#[derive(Debug, Error)]
pub enum ImportError {
    #[error("input is {bytes} bytes; maximum accepted size is {maximum}")]
    InputTooLarge { bytes: usize, maximum: usize },
    #[error("invalid pack id {0:?}; use lowercase ASCII letters, digits, '-' or '_'")]
    InvalidPackId(String),
    #[error("stock Envy 2.2 audit failed with {} issue(s)", .0.len())]
    AuditFailed(Vec<AuditIssue>),
    #[error("syntax error on line {line}: {message}")]
    Syntax { line: usize, message: String },
    #[error("encoding error on line {line}: {message}")]
    Encoding { line: usize, message: String },
    #[error("limit exceeded on line {line}: {message}")]
    Limit { line: usize, message: String },
    #[error("unexpected end of file on line {line}")]
    UnexpectedEof { line: usize },
    #[error("generated WorldPack is invalid: {0}")]
    Validation(String),
}

#[cfg(test)]
mod tests {
    use super::*;

    const STOCK_AREA: &[u8] = br#"#AREA
{ 1 10 } Synthetic Test Area~
#HELPS
0 $~
#0
#RECALL
100
#MOBILES
#0
#OBJECTS
#0
#ROOMS
#100
The First Room~
A deliberately synthetic room.
Nothing here came from a historical area file.~
0 4
D0
A narrow way leads north.~
gate~
1 -1 101
S
#101
The Second Room~
Another synthetic room.~
0 0 2
D2
The first room lies south.~
~
0 -1 100
E
sign~
A sign made only for this test.~
S
#0
#RESETS
S
#SHOPS
0
#SPECIALS
S
#$
"#;

    #[test]
    fn audit_accepts_stock_section_order() {
        let report = audit(STOCK_AREA).unwrap();
        assert!(report.stock_order);
        assert!(report.issues.is_empty());
        assert_eq!(report.sections.len(), STOCK_SECTION_ORDER.len());
    }

    #[test]
    fn audit_reports_out_of_order_sections() {
        let text = String::from_utf8(STOCK_AREA.to_vec()).unwrap();
        let text = text.replace("#MOBILES\n#0\n#OBJECTS", "#OBJECTS\n#0\n#MOBILES");

        let report = audit(text.as_bytes()).unwrap();

        assert!(!report.stock_order);
        assert!(
            report
                .issues
                .iter()
                .any(|issue| issue.code == "section_out_of_order")
        );
    }

    #[test]
    fn parses_rooms_and_resolves_local_exits() {
        let pack = parse_world_pack(
            STOCK_AREA,
            "synthetic",
            Some("synthetic.are".to_owned()),
            TextEncoding::Ascii,
        )
        .unwrap();

        assert_eq!(pack.name, "{ 1 10 } Synthetic Test Area");
        assert_eq!(pack.rooms.len(), 2);
        assert_eq!(pack.rooms[0].legacy_vnum, Some(100));
        assert_eq!(pack.rooms[0].title, "The First Room");
        assert_eq!(
            pack.rooms[0].legacy.as_ref().unwrap().flags,
            LegacyBitvector::Numeric(0)
        );
        assert_eq!(pack.rooms[0].legacy.as_ref().unwrap().sector, 4);
        assert_eq!(
            pack.rooms[0].exits[0].target,
            ExitTarget::Room {
                room_id: "synthetic:room:101".to_owned()
            }
        );
        assert_eq!(
            pack.rooms[1].legacy.as_ref().unwrap().prefix_values,
            vec![0]
        );
    }

    #[test]
    fn parses_historical_pipe_separated_number_expressions() {
        assert_eq!(parse_legacy_number(b"1|64|256", 1).unwrap(), 321);
        assert!(parse_legacy_number(b"1||2", 1).is_err());
    }

    #[test]
    fn ascii_mode_rejects_non_ascii_room_text() {
        let input = String::from_utf8(STOCK_AREA.to_vec())
            .unwrap()
            .replace("The First Room", "La habitaci\u{00f3}n");

        let error =
            parse_world_pack(input.as_bytes(), "synthetic", None, TextEncoding::Ascii).unwrap_err();

        let ImportError::Encoding { line, message } = error else {
            panic!("expected encoding error");
        };
        assert!(line > 1);
        assert!(message.contains("0xc3"));
        assert!(message.contains("column"));
        assert!(message.contains("valid UTF-8"));
    }

    #[test]
    fn encoding_preflight_rejects_bom_and_invalid_utf8_actionably() {
        let mut bom = vec![0xef, 0xbb, 0xbf];
        bom.extend_from_slice(STOCK_AREA);
        let error = parse_import(&bom, "bom", None, TextEncoding::Utf8).unwrap_err();
        assert!(error.to_string().contains("BOM"));

        let mut invalid = STOCK_AREA.to_vec();
        let offset = invalid
            .windows("The First Room".len())
            .position(|window| window == b"The First Room")
            .unwrap();
        invalid[offset] = 0xff;
        let error = parse_import(&invalid, "invalid", None, TextEncoding::Utf8).unwrap_err();
        let message = error.to_string();
        assert!(message.contains("byte offset"));
        assert!(message.contains("transcode"));
    }

    #[test]
    fn audit_surfaces_unknown_and_duplicate_extension_sections() {
        let unknown = String::from_utf8(STOCK_AREA.to_vec())
            .unwrap()
            .replace("#$", "#VENDOR_DATA\nopaque\n#$");
        let report = audit(unknown.as_bytes()).unwrap();
        assert!(report.stock_order);
        assert!(
            report
                .issues
                .iter()
                .any(|issue| issue.code == "unknown_section")
        );
        let imported = parse_import(
            unknown.as_bytes(),
            "unknown-section",
            None,
            TextEncoding::Ascii,
        )
        .unwrap();
        assert!(imported.diagnostics.iter().any(|diagnostic| {
            diagnostic.code == "unknown_section_preserved_or_ignored" && diagnostic.line.is_some()
        }));

        let duplicate_extension = String::from_utf8(STOCK_AREA.to_vec())
            .unwrap()
            .replace("#$", "#MOBPROGS\nS\n#MOBPROGS\nS\n#$");
        let report = audit(duplicate_extension.as_bytes()).unwrap();
        assert!(!report.stock_order);
        assert!(
            report
                .issues
                .iter()
                .any(|issue| issue.code == "duplicate_extension_section")
        );
    }

    #[test]
    fn room_vnum_and_direction_collisions_are_line_specific_errors() {
        let duplicate_room = String::from_utf8(STOCK_AREA.to_vec())
            .unwrap()
            .replace("#101\nThe Second Room", "#100\nThe Second Room");
        let error = parse_import(
            duplicate_room.as_bytes(),
            "duplicate-room",
            None,
            TextEncoding::Ascii,
        )
        .unwrap_err();
        assert!(error.to_string().contains("duplicate room VNUM 100"));

        let duplicate_direction = String::from_utf8(STOCK_AREA.to_vec()).unwrap().replace(
            "1 -1 101\nS",
            "1 -1 101\nD0\nA duplicate road.~\nroad~\n0 -1 101\nS",
        );
        let error = parse_import(
            duplicate_direction.as_bytes(),
            "duplicate-direction",
            None,
            TextEncoding::Ascii,
        )
        .unwrap_err();
        assert!(error.to_string().contains("repeats direction D0"));
        assert!(error.to_string().contains("first declared on line"));
    }

    #[test]
    fn unterminated_room_title_is_rejected() {
        let text = String::from_utf8(STOCK_AREA.to_vec())
            .unwrap()
            .replace("The First Room~", "The First Room");

        let error =
            parse_world_pack(text.as_bytes(), "synthetic", None, TextEncoding::Ascii).unwrap_err();

        assert!(matches!(error, ImportError::Syntax { .. }));
    }

    #[test]
    fn arbitrary_bounded_inputs_never_panic_the_auditor_or_converter() {
        let mut seed = 0x5449_414d_5544_u64;
        for case in 0..512 {
            let length = (next_random(&mut seed) as usize) % 4096;
            let bytes = (0..length)
                .map(|_| next_random(&mut seed) as u8)
                .collect::<Vec<_>>();

            let _ = audit(&bytes);
            let _ = parse_world_pack(
                &bytes,
                &format!("fuzz-{case}"),
                None,
                if case % 2 == 0 {
                    TextEncoding::Ascii
                } else {
                    TextEncoding::Utf8
                },
            );
        }
    }

    fn next_random(seed: &mut u64) -> u64 {
        *seed ^= *seed << 13;
        *seed ^= *seed >> 7;
        *seed ^= *seed << 17;
        *seed
    }
}
