use std::collections::{HashMap, HashSet};

use serde::Serialize;
use world_pack::{
    InitialItemInstance, InitialItemLocation, ItemPrototype, ItemRarity, ItemStackPolicy,
    ItemTransferPolicy, LegacyBitvector, WorldPack,
};

use super::{
    Cursor, ImportError, TextEncoding, decode, line_number, parse_integer_line,
    parse_record_header, section_body, trim_ascii,
};

const MAX_MOBILES: usize = 100_000;
const MAX_OBJECTS: usize = 100_000;
const MAX_RESETS: usize = 500_000;
const MAX_SHOPS: usize = 100_000;
const MAX_SPECIALS: usize = 100_000;
const MAX_PROGRAMS: usize = 100_000;
const MAX_HELPS: usize = 100_000;

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ImportResult {
    pub world_pack: WorldPack,
    pub legacy: LegacyContent,
    pub diagnostics: Vec<ImportDiagnostic>,
    pub provenance: ImportProvenance,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ImportProvenance {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source_file: Option<String>,
    pub source_format: String,
    pub dialect: String,
    pub text_encoding: TextEncoding,
    pub input_bytes: usize,
    pub input_sha256: String,
    pub license_status: LicenseReviewStatus,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum LicenseReviewStatus {
    UndeterminedReviewRequired,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LegacyContent {
    pub area: AreaMetadata,
    pub helps: Vec<LegacyHelp>,
    pub mobiles: Vec<LegacyMobile>,
    pub objects: Vec<LegacyObject>,
    pub resets: Vec<LegacyReset>,
    pub shops: Vec<LegacyShop>,
    pub specials: Vec<LegacySpecial>,
    pub programs: Vec<LegacyProgram>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LegacyHelp {
    pub level: i64,
    pub line: usize,
    pub keywords: String,
    pub text: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AreaMetadata {
    pub raw_name: String,
    /// Values inside the historical `{ ... }` prefix. Their meaning varies by
    /// codebase, so the importer does not label them as levels or VNUM bounds.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub braced_values: Vec<i64>,
    pub display_name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub recall_vnum: Option<i64>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum DiagnosticSeverity {
    Warning,
    Info,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ImportDiagnostic {
    pub severity: DiagnosticSeverity,
    pub code: &'static str,
    pub section: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub line: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub vnum: Option<i64>,
    pub message: String,
    pub recommendation: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LegacyMobile {
    pub vnum: i64,
    pub line: usize,
    pub keywords: String,
    pub short_description: String,
    pub long_description: String,
    pub description: String,
    /// Dialects disagree on the exact shape after the four strings. Values are
    /// retained line-by-line instead of assigning potentially wrong meanings.
    pub attribute_lines: Vec<LegacyDataLine>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub programs: Vec<LegacyProgram>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LegacyObject {
    pub vnum: i64,
    pub line: usize,
    pub keywords: String,
    pub short_description: String,
    pub room_description: String,
    pub action_description: String,
    pub item_type: i64,
    pub extra_flags: LegacyBitvector,
    pub wear_flags: LegacyBitvector,
    pub values: Vec<String>,
    /// Remaining data lines are dialect-specific and intentionally inert.
    pub attribute_lines: Vec<LegacyDataLine>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub programs: Vec<LegacyProgram>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LegacyReset {
    pub command: String,
    pub line: usize,
    pub if_flag: i64,
    pub arguments: Vec<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub world_pack_item_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LegacyShop {
    pub keeper_vnum: i64,
    pub line: usize,
    pub buy_types: Vec<i64>,
    pub profit_buy: i64,
    pub profit_sell: i64,
    pub open_hour: i64,
    pub close_hour: i64,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub trailing_values: Vec<i64>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LegacySpecial {
    pub target_kind: String,
    pub target_vnum: i64,
    pub procedure: String,
    pub line: usize,
    pub executable: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LegacyProgram {
    pub owner: ProgramOwner,
    pub trigger: String,
    pub argument: String,
    pub body: String,
    pub line: usize,
    pub executable: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ProgramOwner {
    Mobile {
        vnum: i64,
    },
    Object {
        vnum: i64,
    },
    Room {
        vnum: i64,
    },
    ExternalSection {
        section: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        owner_vnum: Option<i64>,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LegacyDataLine {
    pub raw: String,
    pub tokens: Vec<String>,
}

pub(super) fn parse_and_translate(
    bytes: &[u8],
    pack_id: &str,
    encoding: TextEncoding,
    pack: &mut WorldPack,
    room_programs: Vec<LegacyProgram>,
) -> Result<(LegacyContent, Vec<ImportDiagnostic>), ImportError> {
    let area = parse_area_metadata(bytes, encoding)?;
    let helps = parse_helps(bytes, encoding)?;
    let mobiles = parse_mobiles(bytes, encoding)?;
    let objects = parse_objects(bytes, encoding)?;
    let mut resets = parse_resets(bytes)?;
    let shops = parse_shops(bytes)?;
    let specials = parse_specials(bytes, encoding)?;
    let mut programs = room_programs;
    for mobile in &mobiles {
        programs.extend(mobile.programs.iter().cloned());
    }
    for object in &objects {
        programs.extend(object.programs.iter().cloned());
    }
    programs.extend(parse_extension_program_sections(bytes, encoding)?);
    if programs.len() > MAX_PROGRAMS {
        return Err(ImportError::Limit {
            line: 1,
            message: format!("program count exceeds {MAX_PROGRAMS}"),
        });
    }

    let mut diagnostics = Vec::new();
    translate_object_resets(pack_id, &objects, &mut resets, pack, &mut diagnostics);
    append_descriptor_diagnostics(&objects, &mut diagnostics);
    append_reference_diagnostics(
        ReferenceContext {
            area: &area,
            mobiles: &mobiles,
            objects: &objects,
            resets: &resets,
            shops: &shops,
            specials: &specials,
            programs: &programs,
            pack,
        },
        &mut diagnostics,
    );
    append_loss_diagnostics(
        &helps,
        &mobiles,
        &objects,
        &resets,
        &shops,
        &specials,
        &programs,
        &mut diagnostics,
    );

    Ok((
        LegacyContent {
            area,
            helps,
            mobiles,
            objects,
            resets,
            shops,
            specials,
            programs,
        },
        diagnostics,
    ))
}

fn parse_helps(bytes: &[u8], encoding: TextEncoding) -> Result<Vec<LegacyHelp>, ImportError> {
    let range = section_body(bytes, "HELPS", "RECALL")?;
    let mut cursor = Cursor::new(&bytes[range.clone()], line_number(bytes, range.start));
    let mut helps = Vec::new();
    while !cursor.is_finished() {
        cursor.skip_ascii_whitespace();
        if cursor.is_finished() {
            break;
        }
        let line = cursor.line();
        let header = decode(cursor.tilde_string()?, encoding, line)?;
        let header = header.trim();
        if header == "#0" || header.is_empty() {
            break;
        }
        let mut fields = header.splitn(2, char::is_whitespace);
        let level_text = fields.next().unwrap_or_default();
        let keywords = fields.next().unwrap_or_default().trim();
        let level = super::parse_legacy_number(level_text.as_bytes(), line)?;
        if keywords == "$" {
            break;
        }
        if keywords.is_empty() {
            return Err(ImportError::Syntax {
                line,
                message: "help header requires a level and tilde-terminated keywords".to_owned(),
            });
        }
        if helps.len() >= MAX_HELPS {
            return Err(ImportError::Limit {
                line,
                message: format!("help count exceeds {MAX_HELPS}"),
            });
        }
        let text_line = cursor.line();
        let text = decode(cursor.tilde_string()?, encoding, text_line)?
            .trim()
            .to_owned();
        helps.push(LegacyHelp {
            level,
            line,
            keywords: keywords.to_owned(),
            text,
        });
    }
    Ok(helps)
}

fn parse_area_metadata(bytes: &[u8], encoding: TextEncoding) -> Result<AreaMetadata, ImportError> {
    let area_range = section_body(bytes, "AREA", "HELPS")?;
    let mut area_cursor = Cursor::new(
        &bytes[area_range.clone()],
        line_number(bytes, area_range.start),
    );
    let area_line = area_cursor.line();
    let raw_name = decode(area_cursor.tilde_string()?, encoding, area_line)?
        .trim()
        .to_owned();
    let (braced_values, display_name) = parse_area_header(&raw_name);

    let recall_range = section_body(bytes, "RECALL", "MOBILES")?;
    let mut recall_cursor = Cursor::new(
        &bytes[recall_range.clone()],
        line_number(bytes, recall_range.start),
    );
    recall_cursor.skip_ascii_whitespace();
    let recall_vnum = if recall_cursor.is_finished() {
        None
    } else {
        let line = recall_cursor.line();
        parse_integer_line(recall_cursor.line_bytes()?, line)?
            .first()
            .copied()
    };

    Ok(AreaMetadata {
        raw_name,
        braced_values,
        display_name,
        recall_vnum,
    })
}

fn parse_area_header(name: &str) -> (Vec<i64>, String) {
    let Some(open) = name.find('{') else {
        return (Vec::new(), name.trim().to_owned());
    };
    let Some(relative_close) = name[open + 1..].find('}') else {
        return (Vec::new(), name.trim().to_owned());
    };
    let close = open + 1 + relative_close;
    let values = name[open + 1..close]
        .split_ascii_whitespace()
        .filter_map(|value| value.parse::<i64>().ok())
        .collect::<Vec<_>>();
    (values, name[close + 1..].trim().to_owned())
}

fn parse_mobiles(bytes: &[u8], encoding: TextEncoding) -> Result<Vec<LegacyMobile>, ImportError> {
    let range = section_body(bytes, "MOBILES", "OBJECTS")?;
    let mut cursor = Cursor::new(&bytes[range.clone()], line_number(bytes, range.start));
    let mut records = Vec::new();
    let mut seen = HashSet::new();
    while let Some((vnum, record_line)) = next_record(&mut cursor)? {
        check_count(records.len(), MAX_MOBILES, record_line, "mobile")?;
        if !seen.insert(vnum) {
            return Err(ImportError::Syntax {
                line: record_line,
                message: format!("duplicate mobile VNUM {vnum}"),
            });
        }
        let keywords = next_text(&mut cursor, encoding)?;
        let short_description = next_text(&mut cursor, encoding)?;
        let long_description = next_text(&mut cursor, encoding)?;
        let description = next_text(&mut cursor, encoding)?;
        let (attribute_lines, programs) =
            parse_attributes_and_programs(&mut cursor, encoding, ProgramOwner::Mobile { vnum })?;
        if attribute_lines.len() < 3 {
            return Err(ImportError::Syntax {
                line: record_line,
                message: format!(
                    "mobile {vnum} has {} attribute line(s); expected at least 3 for a Merc-family record",
                    attribute_lines.len()
                ),
            });
        }
        records.push(LegacyMobile {
            vnum,
            line: record_line,
            keywords,
            short_description,
            long_description,
            description,
            attribute_lines,
            programs,
        });
    }
    Ok(records)
}

fn parse_objects(bytes: &[u8], encoding: TextEncoding) -> Result<Vec<LegacyObject>, ImportError> {
    let range = section_body(bytes, "OBJECTS", "ROOMS")?;
    let mut cursor = Cursor::new(&bytes[range.clone()], line_number(bytes, range.start));
    let mut records = Vec::new();
    let mut seen = HashSet::new();
    while let Some((vnum, record_line)) = next_record(&mut cursor)? {
        check_count(records.len(), MAX_OBJECTS, record_line, "object")?;
        if !seen.insert(vnum) {
            return Err(ImportError::Syntax {
                line: record_line,
                message: format!("duplicate object VNUM {vnum}"),
            });
        }
        let keywords = next_text(&mut cursor, encoding)?;
        let short_description = next_text(&mut cursor, encoding)?;
        let room_description = next_text(&mut cursor, encoding)?;
        let action_description = next_text(&mut cursor, encoding)?;
        let (mut attribute_lines, programs) =
            parse_attributes_and_programs(&mut cursor, encoding, ProgramOwner::Object { vnum })?;
        if attribute_lines.len() < 3 {
            return Err(ImportError::Syntax {
                line: record_line,
                message: format!(
                    "object {vnum} has {} data line(s); expected at least 3 for a Merc-family record",
                    attribute_lines.len()
                ),
            });
        }
        let header = attribute_lines.remove(0);
        if header.tokens.len() < 3 {
            return Err(ImportError::Syntax {
                line: record_line,
                message: format!("object {vnum} type/flag line requires at least 3 values"),
            });
        }
        let item_type = parse_data_number(&header.tokens[0], record_line)?;
        let extra_flags = super::parse_legacy_bitvector(&header.tokens[1], record_line)?;
        let wear_flags = super::parse_legacy_bitvector(&header.tokens[2], record_line)?;
        let values = attribute_lines.remove(0).tokens;
        records.push(LegacyObject {
            vnum,
            line: record_line,
            keywords,
            short_description,
            room_description,
            action_description,
            item_type,
            extra_flags,
            wear_flags,
            values,
            attribute_lines,
            programs,
        });
    }
    Ok(records)
}

fn next_record(cursor: &mut Cursor<'_>) -> Result<Option<(i64, usize)>, ImportError> {
    cursor.skip_ascii_whitespace();
    if cursor.is_finished() {
        return Ok(None);
    }
    let line = cursor.line();
    let header = cursor.line_bytes()?;
    let vnum = parse_record_header(header, line)?;
    if vnum == 0 {
        cursor.skip_ascii_whitespace();
        if !cursor.is_finished() {
            return Err(ImportError::Syntax {
                line: cursor.line(),
                message: "data appears after the #0 record terminator".to_owned(),
            });
        }
        Ok(None)
    } else {
        Ok(Some((vnum, line)))
    }
}

fn next_text(cursor: &mut Cursor<'_>, encoding: TextEncoding) -> Result<String, ImportError> {
    let line = cursor.line();
    decode(cursor.tilde_string()?, encoding, line).map(|value| value.trim().to_owned())
}

fn parse_attributes_and_programs(
    cursor: &mut Cursor<'_>,
    encoding: TextEncoding,
    owner: ProgramOwner,
) -> Result<(Vec<LegacyDataLine>, Vec<LegacyProgram>), ImportError> {
    let mut attributes = Vec::new();
    let mut programs = Vec::new();
    loop {
        cursor.skip_blank_lines();
        if cursor.is_finished() {
            break;
        }
        let line = cursor.line();
        let next = trim_ascii(cursor.peek_line_bytes());
        if next.starts_with(b"#") {
            break;
        }
        if next == b"|" {
            cursor.line_bytes()?;
            continue;
        }
        if next.starts_with(b">") {
            programs.push(parse_inline_program(cursor, encoding, owner.clone())?);
            if programs.len() > MAX_PROGRAMS {
                return Err(ImportError::Limit {
                    line,
                    message: format!("program count exceeds {MAX_PROGRAMS}"),
                });
            }
            continue;
        }
        let raw = cursor.line_bytes()?;
        let decoded = decode(trim_ascii(raw), encoding, line)?;
        let tokens = decoded
            .split_ascii_whitespace()
            .map(ToOwned::to_owned)
            .collect::<Vec<_>>();
        if tokens.is_empty() {
            return Err(ImportError::Syntax {
                line,
                message: "empty legacy attribute line".to_owned(),
            });
        }
        attributes.push(LegacyDataLine {
            raw: decoded.to_owned(),
            tokens,
        });
    }
    Ok((attributes, programs))
}

fn parse_data_number(token: &str, line: usize) -> Result<i64, ImportError> {
    super::parse_legacy_number(token.as_bytes(), line)
}

fn parse_inline_program(
    cursor: &mut Cursor<'_>,
    encoding: TextEncoding,
    owner: ProgramOwner,
) -> Result<LegacyProgram, ImportError> {
    let line = cursor.line();
    let header = decode(cursor.tilde_string()?, encoding, line)?;
    let header = header.trim().trim_start_matches('>').trim();
    let mut parts = header.splitn(2, char::is_whitespace);
    let trigger = parts.next().unwrap_or_default().to_owned();
    if trigger.is_empty() {
        return Err(ImportError::Syntax {
            line,
            message: "program trigger must not be empty".to_owned(),
        });
    }
    let argument = parts.next().unwrap_or_default().trim().to_owned();
    let body_line = cursor.line();
    let body = decode(cursor.tilde_string()?, encoding, body_line)?.to_owned();
    Ok(LegacyProgram {
        owner,
        trigger,
        argument,
        body,
        line,
        executable: false,
    })
}

pub(super) fn parse_consumed_program_header(
    raw_header: &[u8],
    line: usize,
    cursor: &mut Cursor<'_>,
    encoding: TextEncoding,
    owner: ProgramOwner,
) -> Result<LegacyProgram, ImportError> {
    let header = decode(raw_header, encoding, line)?;
    let Some(header) = header.trim().strip_suffix('~') else {
        return Err(ImportError::Syntax {
            line,
            message: "program header must end with '~' on its directive line".to_owned(),
        });
    };
    let header = header.trim_start_matches('>').trim();
    let mut parts = header.splitn(2, char::is_whitespace);
    let trigger = parts.next().unwrap_or_default().to_owned();
    if trigger.is_empty() {
        return Err(ImportError::Syntax {
            line,
            message: "program trigger must not be empty".to_owned(),
        });
    }
    let argument = parts.next().unwrap_or_default().trim().to_owned();
    let body_line = cursor.line();
    let body = decode(cursor.tilde_string()?, encoding, body_line)?.to_owned();
    Ok(LegacyProgram {
        owner,
        trigger,
        argument,
        body,
        line,
        executable: false,
    })
}

fn parse_resets(bytes: &[u8]) -> Result<Vec<LegacyReset>, ImportError> {
    let range = section_body(bytes, "RESETS", "SHOPS")?;
    let mut cursor = Cursor::new(&bytes[range.clone()], line_number(bytes, range.start));
    let mut records = Vec::new();
    let mut terminated = false;
    while !cursor.is_finished() {
        let line = cursor.line();
        let raw = trim_ascii(cursor.line_bytes()?);
        if raw.is_empty() || raw.starts_with(b"*") {
            continue;
        }
        let command = raw[0] as char;
        if command == 'S' {
            terminated = true;
            break;
        }
        check_count(records.len(), MAX_RESETS, line, "reset")?;
        let text = std::str::from_utf8(raw).map_err(|_| ImportError::Syntax {
            line,
            message: "reset contains non-ASCII bytes".to_owned(),
        })?;
        let mut fields = text.split_ascii_whitespace();
        let command_text = fields.next().expect("non-empty reset line");
        if command_text.len() != 1 || !command.is_ascii_alphabetic() {
            return Err(ImportError::Syntax {
                line,
                message: format!("invalid reset command {command_text:?}"),
            });
        }
        let values = fields
            .map(|value| super::parse_legacy_number(value.as_bytes(), line))
            .collect::<Result<Vec<_>, _>>()?;
        let Some((&if_flag, arguments)) = values.split_first() else {
            return Err(ImportError::Syntax {
                line,
                message: format!("reset command {command} requires an if-flag"),
            });
        };
        records.push(LegacyReset {
            command: command.to_ascii_uppercase().to_string(),
            line,
            if_flag,
            arguments: arguments.to_vec(),
            world_pack_item_id: None,
        });
    }
    if !terminated {
        return Err(ImportError::Syntax {
            line: cursor.line(),
            message: "#RESETS is missing its S terminator".to_owned(),
        });
    }
    Ok(records)
}

fn parse_shops(bytes: &[u8]) -> Result<Vec<LegacyShop>, ImportError> {
    let range = section_body(bytes, "SHOPS", "SPECIALS")?;
    let mut cursor = Cursor::new(&bytes[range.clone()], line_number(bytes, range.start));
    let mut shops = Vec::new();
    let mut seen_keepers = HashSet::new();
    let mut terminated = false;
    while !cursor.is_finished() {
        cursor.skip_blank_lines();
        if cursor.is_finished() {
            break;
        }
        let line = cursor.line();
        let values = parse_integer_line(cursor.line_bytes()?, line)?;
        if values == [0] {
            terminated = true;
            break;
        }
        check_count(shops.len(), MAX_SHOPS, line, "shop")?;
        if values.len() < 10 {
            return Err(ImportError::Syntax {
                line,
                message: format!(
                    "shop record has {} values; expected keeper, five buy types, margins and hours",
                    values.len()
                ),
            });
        }
        if !seen_keepers.insert(values[0]) {
            return Err(ImportError::Syntax {
                line,
                message: format!("duplicate shop definition for keeper VNUM {}", values[0]),
            });
        }
        shops.push(LegacyShop {
            keeper_vnum: values[0],
            line,
            buy_types: values[1..6].to_vec(),
            profit_buy: values[6],
            profit_sell: values[7],
            open_hour: values[8],
            close_hour: values[9],
            trailing_values: values[10..].to_vec(),
        });
    }
    if !terminated {
        return Err(ImportError::Syntax {
            line: cursor.line(),
            message: "#SHOPS is missing its 0 terminator".to_owned(),
        });
    }
    Ok(shops)
}

fn parse_specials(bytes: &[u8], encoding: TextEncoding) -> Result<Vec<LegacySpecial>, ImportError> {
    let range = section_until_extension_or(bytes, "SPECIALS", "$")?;
    let mut cursor = Cursor::new(&bytes[range.clone()], line_number(bytes, range.start));
    let mut specials = Vec::new();
    let mut seen_targets = HashSet::new();
    let mut terminated = false;
    while !cursor.is_finished() {
        let line = cursor.line();
        let raw = trim_ascii(cursor.line_bytes()?);
        if raw.is_empty() || raw.starts_with(b"*") {
            continue;
        }
        if raw == b"S" {
            terminated = true;
            break;
        }
        check_count(specials.len(), MAX_SPECIALS, line, "special")?;
        let decoded = decode(raw, encoding, line)?;
        let fields = decoded.split_ascii_whitespace().collect::<Vec<_>>();
        if fields.len() != 3 || !matches!(fields[0], "M" | "O" | "R") {
            return Err(ImportError::Syntax {
                line,
                message: "special requires `M|O|R <vnum> <procedure-name>`".to_owned(),
            });
        }
        let target_vnum = super::parse_legacy_number(fields[1].as_bytes(), line)?;
        let target_kind = fields[0].as_bytes()[0] as char;
        if !seen_targets.insert((target_kind, target_vnum)) {
            return Err(ImportError::Syntax {
                line,
                message: format!(
                    "duplicate special assignment for {} VNUM {target_vnum}",
                    fields[0]
                ),
            });
        }
        let procedure = fields[2];
        if !procedure
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || b"_-".contains(&byte))
        {
            return Err(ImportError::Syntax {
                line,
                message: format!("unsafe special procedure identifier {procedure:?}"),
            });
        }
        specials.push(LegacySpecial {
            target_kind: fields[0].to_owned(),
            target_vnum,
            procedure: procedure.to_owned(),
            line,
            executable: false,
        });
    }
    if !terminated {
        return Err(ImportError::Syntax {
            line: cursor.line(),
            message: "#SPECIALS is missing its S terminator".to_owned(),
        });
    }
    Ok(specials)
}

fn translate_object_resets(
    pack_id: &str,
    objects: &[LegacyObject],
    resets: &mut [LegacyReset],
    pack: &mut WorldPack,
    diagnostics: &mut Vec<ImportDiagnostic>,
) {
    let object_by_vnum = objects
        .iter()
        .map(|object| (object.vnum, object))
        .collect::<HashMap<_, _>>();
    let room_by_vnum = pack
        .rooms
        .iter()
        .enumerate()
        .filter_map(|(index, room)| room.legacy_vnum.map(|vnum| (vnum, index)))
        .collect::<HashMap<_, _>>();
    let mut ordinals = HashMap::<i64, usize>::new();
    let mut emitted_prototypes = HashSet::new();

    for reset in resets.iter_mut().filter(|reset| reset.command == "O") {
        if reset.arguments.len() < 3 {
            diagnostics.push(warning(
                "malformed_object_reset",
                "RESETS",
                Some(reset.line),
                reset.arguments.first().copied(),
                "object reset does not contain object, maximum-count and room VNUMs",
                "repair the O reset in the source area before importing it",
            ));
            continue;
        }
        let object_vnum = reset.arguments[0];
        let room_vnum = reset.arguments[2];
        if reset.if_flag != 0 {
            diagnostics.push(warning(
                "conditional_object_reset_not_converted",
                "RESETS",
                Some(reset.line),
                Some(object_vnum),
                "conditional O reset was preserved but not placed unconditionally",
                "author an explicit spawn condition in Tiamud after reviewing the preceding reset",
            ));
            continue;
        }
        let Some(object) = object_by_vnum.get(&object_vnum) else {
            diagnostics.push(warning(
                "missing_object_prototype",
                "RESETS",
                Some(reset.line),
                Some(object_vnum),
                "O reset references an object prototype not present in this area",
                "import the referenced area or repair the object VNUM before conversion",
            ));
            continue;
        };
        let Some(&room_index) = room_by_vnum.get(&room_vnum) else {
            diagnostics.push(warning(
                "external_object_room",
                "RESETS",
                Some(reset.line),
                Some(room_vnum),
                "O reset targets a room not present in this WorldPack",
                "move the reset into the owning pack or place the item manually after composing packs",
            ));
            continue;
        };
        let ordinal = ordinals.entry(object_vnum).or_default();
        *ordinal += 1;
        let prototype_id = format!("{pack_id}:object:{object_vnum}");
        let item_id = format!("{pack_id}:object:{object_vnum}:{}", *ordinal);
        let name = usable_name(&object.short_description, &object.keywords, object_vnum);
        let description =
            usable_description(&object.room_description, &object.action_description, &name);
        let aliases = object
            .keywords
            .split_ascii_whitespace()
            .filter(|alias| !alias.eq_ignore_ascii_case(&name))
            .map(ToOwned::to_owned)
            .collect::<Vec<_>>();
        if emitted_prototypes.insert(object_vnum) {
            pack.item_prototypes.push(ItemPrototype {
                id: prototype_id.clone(),
                name,
                description,
                aliases,
                equipment_slot: None,
                attack_bonus: 0,
                heal_amount: 0,
                consumable: false,
                effect: None,
                weight_grams: 0,
                slot_cost: 1,
                rarity: ItemRarity::Common,
                item_level: 1,
                stack_policy: ItemStackPolicy::Unique,
                max_stack_units: 1,
                container: None,
                transfer_policy: ItemTransferPolicy::Transferable,
                sealed_loot_table_id: None,
            });
        }
        pack.initial_item_instances.push(InitialItemInstance {
            id: item_id.clone(),
            prototype_id,
            location: InitialItemLocation::Room {
                room_id: pack.rooms[room_index].id.clone(),
            },
        });
        reset.world_pack_item_id = Some(item_id);
    }
}

fn usable_name(short: &str, keywords: &str, vnum: i64) -> String {
    if !short.trim().is_empty() {
        short.trim().to_owned()
    } else if let Some(keyword) = keywords.split_ascii_whitespace().next() {
        keyword.to_owned()
    } else {
        format!("legacy object {vnum}")
    }
}

fn usable_description(room: &str, action: &str, name: &str) -> String {
    if !room.trim().is_empty() {
        room.trim().to_owned()
    } else if !action.trim().is_empty() {
        action.trim().to_owned()
    } else {
        format!("An imported {name}.")
    }
}

fn append_descriptor_diagnostics(
    objects: &[LegacyObject],
    diagnostics: &mut Vec<ImportDiagnostic>,
) {
    for object in objects {
        if !is_audited_item_type(object.item_type) {
            diagnostics.push(warning(
                "unknown_object_type_descriptor",
                "OBJECTS",
                Some(object.line),
                Some(object.vnum),
                &format!(
                    "object type descriptor {} is not in the audited stock Envy/Merc set",
                    object.item_type
                ),
                "verify the source codebase's item type table before mapping this prototype",
            ));
        }
        append_bitvector_diagnostic(
            diagnostics,
            object,
            "extra",
            &object.extra_flags,
            "object_extra_flag_mask_preserved",
        );
        append_bitvector_diagnostic(
            diagnostics,
            object,
            "wear",
            &object.wear_flags,
            "object_wear_flag_mask_preserved",
        );
    }
}

fn append_bitvector_diagnostic(
    diagnostics: &mut Vec<ImportDiagnostic>,
    object: &LegacyObject,
    field: &str,
    bitvector: &LegacyBitvector,
    numeric_code: &'static str,
) {
    match bitvector {
        LegacyBitvector::Numeric(0) => {}
        LegacyBitvector::Numeric(value) => diagnostics.push(warning(
            numeric_code,
            "OBJECTS",
            Some(object.line),
            Some(object.vnum),
            &format!("object {field} flag mask {value} is preserved but not interpreted"),
            "review the source dialect's flag table and author equivalent behavior explicitly",
        )),
        LegacyBitvector::Alphabetic(value) => diagnostics.push(warning(
            "rom_alphabetic_object_bitvector_preserved",
            "OBJECTS",
            Some(object.line),
            Some(object.vnum),
            &format!(
                "ROM-style object {field} bitvector {value:?} is preserved verbatim and was not converted to a numeric mask"
            ),
            "review the exact source flag table before authoring equivalent item behavior",
        )),
    }
}

fn is_audited_item_type(value: i64) -> bool {
    matches!(
        value,
        1 | 2
            | 3
            | 4
            | 5
            | 8
            | 9
            | 10
            | 12
            | 13
            | 15
            | 16
            | 17
            | 18
            | 19
            | 20
            | 22
            | 23
            | 24
            | 25
            | 26
    )
}

struct ReferenceContext<'a> {
    area: &'a AreaMetadata,
    mobiles: &'a [LegacyMobile],
    objects: &'a [LegacyObject],
    resets: &'a [LegacyReset],
    shops: &'a [LegacyShop],
    specials: &'a [LegacySpecial],
    programs: &'a [LegacyProgram],
    pack: &'a WorldPack,
}

fn append_reference_diagnostics(
    context: ReferenceContext<'_>,
    diagnostics: &mut Vec<ImportDiagnostic>,
) {
    let mobile_vnums = context
        .mobiles
        .iter()
        .map(|mobile| mobile.vnum)
        .collect::<HashSet<_>>();
    let object_vnums = context
        .objects
        .iter()
        .map(|object| object.vnum)
        .collect::<HashSet<_>>();
    let room_vnums = context
        .pack
        .rooms
        .iter()
        .filter_map(|room| room.legacy_vnum)
        .collect::<HashSet<_>>();

    if let Some(recall_vnum) = context.area.recall_vnum
        && !room_vnums.contains(&recall_vnum)
    {
        diagnostics.push(warning(
            "external_or_missing_recall_room",
            "RECALL",
            None,
            Some(recall_vnum),
            "recall references a room VNUM not present in this area",
            "compose the owning area or select an explicit Tiamud recall destination",
        ));
    }

    for reset in context.resets {
        match reset.command.as_str() {
            "M" => {
                check_reset_reference(
                    reset,
                    0,
                    "mobile",
                    &mobile_vnums,
                    diagnostics,
                );
                check_reset_reference(reset, 2, "room", &room_vnums, diagnostics);
            }
            "P" => {
                check_reset_reference(reset, 0, "object", &object_vnums, diagnostics);
                check_reset_reference(
                    reset,
                    2,
                    "container object",
                    &object_vnums,
                    diagnostics,
                );
                diagnostics.push(warning(
                    "container_reset_not_converted",
                    "RESETS",
                    Some(reset.line),
                    reset.arguments.first().copied(),
                    "P reset container placement is preserved inert and was not converted",
                    "author an explicit WorldPack v9 container instance after reviewing reset limits and ordering",
                ));
            }
            "G" | "E" => {
                check_reset_reference(reset, 0, "object", &object_vnums, diagnostics);
            }
            "D" | "R" => {
                check_reset_reference(reset, 0, "room", &room_vnums, diagnostics);
            }
            "O" => {}
            command => diagnostics.push(warning(
                "unknown_reset_command",
                "RESETS",
                Some(reset.line),
                reset.arguments.first().copied(),
                &format!(
                    "reset command {command:?} is outside the audited stock command set M/O/P/G/E/D/R"
                ),
                "identify the source dialect and implement a reviewed inert translation before relying on this reset",
            )),
        }
    }

    for shop in context.shops {
        if !mobile_vnums.contains(&shop.keeper_vnum) {
            diagnostics.push(warning(
                "external_or_missing_shop_keeper",
                "SHOPS",
                Some(shop.line),
                Some(shop.keeper_vnum),
                "shop keeper VNUM is not present in this area's mobile prototypes",
                "compose the owning mobile prototype or repair the keeper VNUM before authoring the shop",
            ));
        }
    }

    for special in context.specials {
        let target_exists = match special.target_kind.as_str() {
            "M" => mobile_vnums.contains(&special.target_vnum),
            "O" => object_vnums.contains(&special.target_vnum),
            "R" => room_vnums.contains(&special.target_vnum),
            _ => false,
        };
        if !target_exists {
            diagnostics.push(warning(
                "external_or_missing_special_target",
                "SPECIALS",
                Some(special.line),
                Some(special.target_vnum),
                &format!(
                    "special target {} VNUM {} is not present in this area",
                    special.target_kind, special.target_vnum
                ),
                "compose the owning prototype or repair the target VNUM; the procedure remains inert",
            ));
        }
    }

    for program in context.programs {
        let ProgramOwner::ExternalSection {
            section,
            owner_vnum,
        } = &program.owner
        else {
            continue;
        };
        let Some(owner_vnum) = *owner_vnum else {
            diagnostics.push(warning(
                "unscoped_external_program",
                "PROGRAMS",
                Some(program.line),
                None,
                &format!("external {section} record has no active owner VNUM"),
                "associate the record with an explicit prototype before rewriting it in Tiamud",
            ));
            continue;
        };
        let owner_exists = match section.as_str() {
            "MOBPROGS" => mobile_vnums.contains(&owner_vnum),
            "OBJPROGS" => object_vnums.contains(&owner_vnum),
            "ROOMPROGS" => room_vnums.contains(&owner_vnum),
            "PROGS" => true,
            _ => false,
        };
        if !owner_exists {
            diagnostics.push(warning(
                "external_or_missing_program_owner",
                "PROGRAMS",
                Some(program.line),
                Some(owner_vnum),
                &format!("{section} owner VNUM {owner_vnum} is not present in this area"),
                "compose the owning prototype or repair the owner VNUM before rewriting this inert program",
            ));
        }
    }
}

fn check_reset_reference(
    reset: &LegacyReset,
    argument_index: usize,
    target_kind: &str,
    local_vnums: &HashSet<i64>,
    diagnostics: &mut Vec<ImportDiagnostic>,
) {
    let Some(target_vnum) = reset.arguments.get(argument_index).copied() else {
        diagnostics.push(warning(
            "malformed_reset_reference",
            "RESETS",
            Some(reset.line),
            reset.arguments.first().copied(),
            &format!(
                "{} reset has no argument {} for its {target_kind} reference",
                reset.command,
                argument_index + 1
            ),
            "repair the reset argument list in the source area before importing it",
        ));
        return;
    };
    if !local_vnums.contains(&target_vnum) {
        diagnostics.push(warning(
            "external_or_missing_reset_target",
            "RESETS",
            Some(reset.line),
            Some(target_vnum),
            &format!(
                "{} reset references {target_kind} VNUM {target_vnum}, which is not present in this area",
                reset.command
            ),
            "compose the owning area or repair the referenced VNUM before translating this reset",
        ));
    }
}

#[allow(clippy::too_many_arguments)]
fn append_loss_diagnostics(
    helps: &[LegacyHelp],
    mobiles: &[LegacyMobile],
    objects: &[LegacyObject],
    resets: &[LegacyReset],
    shops: &[LegacyShop],
    specials: &[LegacySpecial],
    programs: &[LegacyProgram],
    diagnostics: &mut Vec<ImportDiagnostic>,
) {
    for help in helps {
        diagnostics.push(warning(
            "help_preserved_not_converted",
            "HELPS",
            Some(help.line),
            None,
            "historical help text is preserved as inert compatibility data and is not published automatically",
            "review its authorship and license, then rewrite or approve it in Tiamud's localized help content",
        ));
    }

    for mobile in mobiles {
        diagnostics.push(warning(
            "mobile_preserved_not_converted",
            "MOBILES",
            Some(mobile.line),
            Some(mobile.vnum),
            "mobile prototype is preserved as inert legacy data; this conservative importer does not translate NPC statistics",
            "author the NPC in Tiamud's gameplay model and compare its attributes with this record",
        ));
    }

    let converted_object_vnums = resets
        .iter()
        .filter(|reset| reset.world_pack_item_id.is_some())
        .filter_map(|reset| reset.arguments.first().copied())
        .collect::<HashSet<_>>();
    for object in objects {
        if !converted_object_vnums.contains(&object.vnum) {
            diagnostics.push(warning(
                "object_prototype_not_placed",
                "OBJECTS",
                Some(object.line),
                Some(object.vnum),
                "object prototype has no safely translatable unconditional room placement",
                "add an explicit Tiamud placement or spawn rule after reviewing its legacy resets",
            ));
        }
    }

    for reset in resets {
        if reset.world_pack_item_id.is_some() {
            continue;
        }
        if reset.command == "O" && reset.arguments.len() >= 3 {
            continue;
        }
        diagnostics.push(warning(
            "reset_preserved_not_converted",
            "RESETS",
            Some(reset.line),
            reset.arguments.first().copied(),
            &format!(
                "{} reset is preserved as inert data because this importer cannot translate its semantics safely",
                reset.command
            ),
            "recreate the reset with Tiamud NPC, equipment, container, door or spawn authoring primitives",
        ));
    }

    let mobile_vnums = mobiles
        .iter()
        .map(|mobile| mobile.vnum)
        .collect::<HashSet<_>>();
    for shop in shops {
        let keeper_note = if mobile_vnums.contains(&shop.keeper_vnum) {
            "keeper mobile was found"
        } else {
            "keeper mobile is external or missing"
        };
        diagnostics.push(warning(
            "shop_preserved_not_converted",
            "SHOPS",
            Some(shop.line),
            Some(shop.keeper_vnum),
            &format!("shop is inert; {keeper_note}, and the importer does not publish historical shop policy"),
            "author a Tiamud shop and verify buy types, margins and opening hours manually",
        ));
    }

    for special in specials {
        diagnostics.push(warning(
            "special_never_executed",
            "SPECIALS",
            Some(special.line),
            Some(special.target_vnum),
            &format!(
                "historical procedure {:?} was recorded as an identifier and was not executed or linked",
                special.procedure
            ),
            "replace it with a reviewed, bounded Tiamud behavior; do not load the historical binary symbol",
        ));
    }

    for program in programs {
        diagnostics.push(warning(
            "program_never_executed",
            "PROGRAMS",
            Some(program.line),
            owner_vnum(&program.owner),
            &format!(
                "historical {:?} trigger was retained as text and was not parsed as commands or executed",
                program.trigger
            ),
            "rewrite the behavior using Tiamud's reviewed bounded scripting API and add content tests",
        ));
    }
}

fn owner_vnum(owner: &ProgramOwner) -> Option<i64> {
    match owner {
        ProgramOwner::Mobile { vnum }
        | ProgramOwner::Object { vnum }
        | ProgramOwner::Room { vnum } => Some(*vnum),
        ProgramOwner::ExternalSection { owner_vnum, .. } => *owner_vnum,
    }
}

fn warning(
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

fn parse_extension_program_sections(
    bytes: &[u8],
    encoding: TextEncoding,
) -> Result<Vec<LegacyProgram>, ImportError> {
    let mut programs = Vec::new();
    for section in ["MOBPROGS", "OBJPROGS", "ROOMPROGS", "PROGS"] {
        let Some((body_start, body_end, starting_line)) = optional_section_body(bytes, section)
        else {
            continue;
        };
        let mut cursor = Cursor::new(&bytes[body_start..body_end], starting_line);
        let mut current_vnum = None;
        while !cursor.is_finished() {
            cursor.skip_blank_lines();
            if cursor.is_finished() {
                break;
            }
            let line = cursor.line();
            let raw = trim_ascii(cursor.line_bytes()?);
            if raw.is_empty() || raw == b"S" || raw.starts_with(b"*") {
                continue;
            }
            if let Some(number) = raw.strip_prefix(b"#") {
                let vnum = super::parse_i64(number, line)?;
                current_vnum = (vnum != 0).then_some(vnum);
                continue;
            }
            let owner = ProgramOwner::ExternalSection {
                section: section.to_owned(),
                owner_vnum: current_vnum,
            };
            if raw.starts_with(b">") {
                programs.push(parse_consumed_program_header(
                    raw,
                    line,
                    &mut cursor,
                    encoding,
                    owner,
                )?);
                continue;
            }
            let text = decode(raw, encoding, line)?.to_owned();
            programs.push(LegacyProgram {
                owner,
                trigger: "external_record".to_owned(),
                argument: String::new(),
                body: text,
                line,
                executable: false,
            });
            if programs.len() > MAX_PROGRAMS {
                return Err(ImportError::Limit {
                    line,
                    message: format!("program count exceeds {MAX_PROGRAMS}"),
                });
            }
        }
    }
    Ok(programs)
}

fn optional_section_body(bytes: &[u8], name: &str) -> Option<(usize, usize, usize)> {
    let marker = format!("#{name}");
    let marker_start = super::find_marker_line(bytes, marker.as_bytes())?;
    let body_start = super::line_end(bytes, marker_start);
    let body_end = next_named_section_offset(bytes, body_start).unwrap_or(bytes.len());
    Some((body_start, body_end, line_number(bytes, body_start)))
}

fn section_until_extension_or(
    bytes: &[u8],
    start: &str,
    fallback_end: &str,
) -> Result<std::ops::Range<usize>, ImportError> {
    let marker = format!("#{start}");
    let marker_start =
        super::find_marker_line(bytes, marker.as_bytes()).ok_or_else(|| ImportError::Syntax {
            line: 1,
            message: format!("section #{start} was not found"),
        })?;
    let body_start = super::line_end(bytes, marker_start);
    let fallback_marker = format!("#{fallback_end}");
    let fallback_relative =
        super::find_marker_line(&bytes[body_start..], fallback_marker.as_bytes()).ok_or_else(
            || ImportError::Syntax {
                line: line_number(bytes, body_start),
                message: format!("section #{fallback_end} was not found after #{start}"),
            },
        )?;
    let fallback = body_start + fallback_relative;
    let extension = next_named_section_offset(bytes, body_start).unwrap_or(fallback);
    Ok(body_start..extension.min(fallback))
}

fn next_named_section_offset(bytes: &[u8], start: usize) -> Option<usize> {
    let mut offset = start;
    for line in super::byte_lines(&bytes[start..]) {
        let trimmed = trim_ascii(line);
        if trimmed.starts_with(b"#")
            && trimmed
                .get(1)
                .is_some_and(|byte| byte.is_ascii_uppercase() || *byte == b'$')
        {
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

fn check_count(
    current: usize,
    maximum: usize,
    line: usize,
    record_name: &str,
) -> Result<(), ImportError> {
    if current < maximum {
        Ok(())
    } else {
        Err(ImportError::Limit {
            line,
            message: format!("{record_name} count exceeds {maximum}"),
        })
    }
}
