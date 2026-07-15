//! A conservative reader for stock EnvyMUD 2.2 area files.
//!
//! It is intentionally an offline conversion tool. It never executes specials,
//! scripts, or any other content found in an area file.

use std::collections::{HashMap, HashSet};

use serde::Serialize;
use thiserror::Error;
use world_pack::{
    CURRENT_SCHEMA_VERSION, Direction, Exit, ExitTarget, LegacyRoomData, Room, SourceMetadata,
    WorldPack,
};

const MAX_INPUT_BYTES: usize = 32 * 1024 * 1024;
const MAX_STRING_BYTES: usize = 1024 * 1024;
const MAX_LINE_BYTES: usize = 1024 * 1024;
const MAX_ROOMS: usize = 100_000;

pub const STOCK_SECTION_ORDER: [&str; 10] = [
    "AREA", "HELPS", "RECALL", "MOBILES", "OBJECTS", "ROOMS", "RESETS", "SHOPS", "SPECIALS", "$",
];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
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

    Ok(AuditReport {
        input_bytes: bytes.len(),
        stock_order: issues.is_empty(),
        sections,
        issues,
    })
}

pub fn parse_world_pack(
    bytes: &[u8],
    pack_id: &str,
    source_file: Option<String>,
    encoding: TextEncoding,
) -> Result<WorldPack, ImportError> {
    check_input_size(bytes)?;
    validate_pack_id(pack_id)?;
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
        let metadata = parse_integer_line(cursor.line_bytes()?, metadata_line_number)?;
        if metadata.len() < 2 {
            return Err(ImportError::Syntax {
                line: metadata_line_number,
                message: "room metadata requires at least flags and sector values".to_owned(),
            });
        }
        let flags = metadata[metadata.len() - 2];
        let sector = metadata[metadata.len() - 1];
        let prefix_values = metadata[..metadata.len() - 2].to_vec();

        let mut exits = Vec::new();
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
            if let Some(number) = directive.strip_prefix(b"D") {
                let direction_number = parse_i64(number, directive_line)?;
                let direction = parse_direction(direction_number, directive_line)?;
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
            exits,
            legacy: Some(LegacyRoomData {
                flags,
                sector,
                prefix_values,
            }),
        });
    }

    resolve_local_exits(&mut rooms);
    let pack = WorldPack {
        schema_version: CURRENT_SCHEMA_VERSION,
        id: pack_id.to_owned(),
        name: area_name,
        source: Some(SourceMetadata {
            format: "envymud-area".to_owned(),
            dialect: Some("envymud-2.2-stock".to_owned()),
            file: source_file,
        }),
        rooms,
    };
    pack.validate()
        .map_err(|error| ImportError::Validation(error.to_string()))?;
    Ok(pack)
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
        assert_eq!(pack.rooms[0].legacy.as_ref().unwrap().flags, 0);
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

        assert!(matches!(error, ImportError::Encoding { .. }));
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
}
