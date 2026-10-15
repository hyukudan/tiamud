//! Sectioned storage of the world runtime snapshot (schema v17, ADR 0035).
//!
//! The portable `WorldRuntimeSnapshot` is still one logical JSON object with
//! the same versioned DTO. Storage splits it into:
//!
//! - one row per *section* (a closed list of large top-level members) in
//!   `world_runtime_sections`, each with its own SHA-256 and the manifest
//!   revision that last wrote it; and
//! - the `world_runtime` singleton, which keeps the snapshot version, the
//!   *head* document (every remaining member, i.e. the small scalars), the
//!   event watermark and a *manifest* listing every stored section with its
//!   byte length, digest and revision.
//!
//! A runtime commit rewrites the head and the manifest, plus only the sections
//! whose content may have changed, in the same transaction as everything else
//! it writes. Recovery assembles the exact object back and fails closed when
//! the manifest and the section rows disagree in any way.
//!
//! This module is text-level and version-agnostic so the SQLite migration,
//! the PostgreSQL migration and transfer, the admin tool and the server store
//! share one definition.

use rusqlite::{Connection, OptionalExtension, params};
use serde_json::{Map, Value};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};
use thiserror::Error;

/// Top-level snapshot members stored as independent rows, in canonical order.
/// Every other member stays in the head document.
pub const RUNTIME_SECTION_NAMES: [&str; 10] = [
    "itemInstances",
    "itemProvenance",
    "npcs",
    "adventure",
    "progressionFlags",
    "trades",
    "lootEncounters",
    "groundLoot",
    "partyLootCursors",
    "areaResidency",
];

/// Byte ceiling of one stored section or head document. The assembled
/// snapshot keeps its own (version-dependent) ceiling, enforced by callers.
pub const MAX_RUNTIME_SECTION_BYTES: usize = 2 * 1024 * 1024;
/// Byte ceiling of the manifest document.
pub const MAX_RUNTIME_MANIFEST_BYTES: usize = 8 * 1024;
/// Format of the manifest document.
pub const RUNTIME_MANIFEST_FORMAT: u64 = 1;

#[derive(Debug, Error)]
pub enum RuntimeSectionError {
    #[error(transparent)]
    Sqlite(#[from] rusqlite::Error),
    #[error("the world runtime manifest is malformed")]
    InvalidManifest,
    #[error("the world runtime sections disagree with their manifest: {0}")]
    Inconsistent(&'static str),
    #[error("the assembled world runtime exceeds {maximum} bytes")]
    TooLarge { maximum: usize },
}

/// One manifest entry: what the stored section row must contain.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RuntimeManifestEntry {
    /// Manifest revision of the commit that last wrote this section.
    pub revision: u64,
    pub bytes: u64,
    pub sha256: String,
}

/// The manifest stored in `world_runtime.manifest_json`.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct RuntimeManifest {
    /// Monotonic revision, advanced by every runtime write.
    pub revision: u64,
    pub sections: BTreeMap<String, RuntimeManifestEntry>,
}

/// One stored section row.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RuntimeSectionRow {
    pub name: String,
    pub revision: i64,
    pub json: String,
    pub sha256: String,
}

/// The stored runtime rows of one database, before assembly.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct StoredRuntime {
    pub snapshot_version: i64,
    pub head_json: String,
    pub manifest_json: String,
    pub last_event_sequence: Option<i64>,
    pub sections: Vec<RuntimeSectionRow>,
}

/// Lowercase hexadecimal SHA-256 of `bytes`.
pub fn sha256_hex(bytes: &[u8]) -> String {
    hex::encode(Sha256::digest(bytes))
}

fn is_section_name(name: &str) -> bool {
    RUNTIME_SECTION_NAMES.contains(&name)
}

fn is_digest(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

impl RuntimeManifest {
    pub fn to_json(&self) -> String {
        let sections = self
            .sections
            .iter()
            .map(|(name, entry)| {
                (
                    name.clone(),
                    serde_json::json!({
                        "bytes": entry.bytes,
                        "revision": entry.revision,
                        "sha256": entry.sha256,
                    }),
                )
            })
            .collect::<Map<_, _>>();
        serde_json::json!({
            "format": RUNTIME_MANIFEST_FORMAT,
            "revision": self.revision,
            "sections": sections,
        })
        .to_string()
    }

    /// Strict parse: closed keys, closed section names, lowercase digests,
    /// entry revisions within the manifest revision and bounded sizes.
    pub fn parse(json: &str) -> Result<Self, RuntimeSectionError> {
        if json.len() > MAX_RUNTIME_MANIFEST_BYTES {
            return Err(RuntimeSectionError::InvalidManifest);
        }
        let value: Value =
            serde_json::from_str(json).map_err(|_| RuntimeSectionError::InvalidManifest)?;
        let object = value
            .as_object()
            .ok_or(RuntimeSectionError::InvalidManifest)?;
        if object.len() != 3
            || object.get("format").and_then(Value::as_u64) != Some(RUNTIME_MANIFEST_FORMAT)
        {
            return Err(RuntimeSectionError::InvalidManifest);
        }
        let revision = object
            .get("revision")
            .and_then(Value::as_u64)
            .filter(|revision| *revision > 0 && i64::try_from(*revision).is_ok())
            .ok_or(RuntimeSectionError::InvalidManifest)?;
        let entries = object
            .get("sections")
            .and_then(Value::as_object)
            .ok_or(RuntimeSectionError::InvalidManifest)?;
        let mut sections = BTreeMap::new();
        for (name, entry) in entries {
            let entry = entry
                .as_object()
                .filter(|entry| entry.len() == 3)
                .ok_or(RuntimeSectionError::InvalidManifest)?;
            let entry_revision = entry
                .get("revision")
                .and_then(Value::as_u64)
                .filter(|entry_revision| (1..=revision).contains(entry_revision))
                .ok_or(RuntimeSectionError::InvalidManifest)?;
            let bytes = entry
                .get("bytes")
                .and_then(Value::as_u64)
                .filter(|bytes| *bytes <= MAX_RUNTIME_SECTION_BYTES as u64)
                .ok_or(RuntimeSectionError::InvalidManifest)?;
            let sha256 = entry
                .get("sha256")
                .and_then(Value::as_str)
                .filter(|sha256| is_digest(sha256))
                .ok_or(RuntimeSectionError::InvalidManifest)?;
            if !is_section_name(name) {
                return Err(RuntimeSectionError::InvalidManifest);
            }
            sections.insert(
                name.clone(),
                RuntimeManifestEntry {
                    revision: entry_revision,
                    bytes,
                    sha256: sha256.to_owned(),
                },
            );
        }
        Ok(Self { revision, sections })
    }
}

/// A complete runtime document split into its head and section members.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SplitRuntimeDocument {
    pub head_json: String,
    pub sections: Vec<(String, String)>,
}

/// Split a complete stored snapshot document. A document that is not a JSON
/// object is kept verbatim as the head with no sections, so a corrupt legacy
/// row stays corrupt (and fails closed at recovery) instead of being repaired
/// or dropped by a migration.
pub fn split_runtime_document(json: &str) -> SplitRuntimeDocument {
    let Ok(Value::Object(mut object)) = serde_json::from_str::<Value>(json) else {
        return SplitRuntimeDocument {
            head_json: json.to_owned(),
            sections: Vec::new(),
        };
    };
    let sections = RUNTIME_SECTION_NAMES
        .iter()
        .filter_map(|name| {
            object
                .remove(*name)
                .map(|value| ((*name).to_owned(), value.to_string()))
        })
        .collect();
    SplitRuntimeDocument {
        head_json: Value::Object(object).to_string(),
        sections,
    }
}

/// Byte length of the object assembled from `head_json` and sections of the
/// given names and lengths, or `None` on overflow.
pub fn assembled_document_len<'a>(
    head_json: &str,
    sections: impl IntoIterator<Item = (&'a str, u64)>,
) -> Option<u64> {
    let head = head_json.trim();
    let mut total = u64::try_from(head.len()).ok()?;
    let mut members_in_head = head != "{}";
    for (name, bytes) in sections {
        // `,` (when a member precedes) + `"name":` + value.
        let separator = u64::from(members_in_head);
        let name = u64::try_from(name.len()).ok()?.checked_add(3)?;
        total = total
            .checked_add(separator)?
            .checked_add(name)?
            .checked_add(bytes)?;
        members_in_head = true;
    }
    Some(total)
}

/// Rebuild the complete snapshot document from the stored rows, verifying the
/// manifest against every row (exact name set, revision, byte length and
/// SHA-256) and refusing any assembled document above `maximum` bytes.
pub fn assemble_runtime_document(
    head_json: &str,
    manifest: &RuntimeManifest,
    rows: &[RuntimeSectionRow],
    maximum: usize,
) -> Result<String, RuntimeSectionError> {
    if rows.len() > RUNTIME_SECTION_NAMES.len() {
        return Err(RuntimeSectionError::Inconsistent("too many section rows"));
    }
    let mut seen = BTreeSet::new();
    for row in rows {
        if !seen.insert(row.name.as_str()) {
            return Err(RuntimeSectionError::Inconsistent("duplicate section row"));
        }
        let entry = manifest
            .sections
            .get(&row.name)
            .ok_or(RuntimeSectionError::Inconsistent("unlisted section row"))?;
        if u64::try_from(row.revision).ok() != Some(entry.revision) {
            return Err(RuntimeSectionError::Inconsistent("section revision"));
        }
        if row.json.len() as u64 != entry.bytes {
            return Err(RuntimeSectionError::Inconsistent("section length"));
        }
        if row.sha256 != entry.sha256 || sha256_hex(row.json.as_bytes()) != entry.sha256 {
            return Err(RuntimeSectionError::Inconsistent("section digest"));
        }
    }
    if seen.len() != manifest.sections.len() {
        return Err(RuntimeSectionError::Inconsistent("missing section row"));
    }
    if head_json.len() > MAX_RUNTIME_SECTION_BYTES {
        return Err(RuntimeSectionError::TooLarge {
            maximum: MAX_RUNTIME_SECTION_BYTES,
        });
    }
    if manifest.sections.is_empty() {
        // Nothing to splice: the head is the whole document. Its decoding
        // (and rejection) belongs to the caller, exactly as for a legacy row.
        if head_json.len() > maximum {
            return Err(RuntimeSectionError::TooLarge { maximum });
        }
        return Ok(head_json.to_owned());
    }
    // Splicing requires an object head that does not already define any
    // section member, and section values that are each one complete JSON
    // value, so no row can inject or shadow members.
    let head = head_json.trim();
    let head_object = serde_json::from_str::<Map<String, Value>>(head)
        .map_err(|_| RuntimeSectionError::Inconsistent("head is not an object"))?;
    if head_object.keys().any(|name| is_section_name(name)) {
        return Err(RuntimeSectionError::Inconsistent("head defines a section"));
    }
    let mut ordered = rows.iter().collect::<Vec<_>>();
    ordered.sort_by_key(|row| {
        RUNTIME_SECTION_NAMES
            .iter()
            .position(|name| *name == row.name)
            .unwrap_or(usize::MAX)
    });
    let total = assembled_document_len(
        head,
        ordered
            .iter()
            .map(|row| (row.name.as_str(), row.json.len() as u64)),
    )
    .ok_or(RuntimeSectionError::TooLarge { maximum })?;
    if total > maximum as u64 {
        return Err(RuntimeSectionError::TooLarge { maximum });
    }
    for row in &ordered {
        serde_json::from_str::<Value>(&row.json)
            .map_err(|_| RuntimeSectionError::Inconsistent("section is not one JSON value"))?;
    }
    let mut assembled = String::with_capacity(total as usize);
    assembled.push_str(&head[..head.len() - 1]);
    let mut members = !head_object.is_empty();
    for row in ordered {
        if members {
            assembled.push(',');
        }
        assembled.push('"');
        assembled.push_str(&row.name);
        assembled.push_str("\":");
        assembled.push_str(&row.json);
        members = true;
    }
    assembled.push('}');
    debug_assert_eq!(assembled.len() as u64, total);
    Ok(assembled)
}

/// Verify and assemble one stored runtime.
pub fn assemble_stored_runtime(
    stored: &StoredRuntime,
    maximum: usize,
) -> Result<String, RuntimeSectionError> {
    let manifest = RuntimeManifest::parse(&stored.manifest_json)?;
    assemble_runtime_document(&stored.head_json, &manifest, &stored.sections, maximum)
}

/// The rows a complete rewrite of `json` stores, with a manifest at
/// `revision`.
pub fn full_runtime_rows(
    json: &str,
    revision: u64,
) -> (String, RuntimeManifest, Vec<RuntimeSectionRow>) {
    let split = split_runtime_document(json);
    let row_revision = i64::try_from(revision).unwrap_or(i64::MAX);
    let mut manifest = RuntimeManifest {
        revision,
        sections: BTreeMap::new(),
    };
    let rows = split
        .sections
        .into_iter()
        .map(|(name, json)| {
            let sha256 = sha256_hex(json.as_bytes());
            manifest.sections.insert(
                name.clone(),
                RuntimeManifestEntry {
                    revision,
                    bytes: json.len() as u64,
                    sha256: sha256.clone(),
                },
            );
            RuntimeSectionRow {
                name,
                revision: row_revision,
                json,
                sha256,
            }
        })
        .collect();
    (split.head_json, manifest, rows)
}

// ---------------------------------------------------------------------------
// SQLite access
// ---------------------------------------------------------------------------

fn sqlite_column_exists(
    connection: &Connection,
    table: &str,
    column: &str,
) -> rusqlite::Result<bool> {
    connection.query_row(
        "SELECT EXISTS(SELECT 1 FROM pragma_table_info(?1) WHERE name = ?2)",
        params![table, column],
        |row| row.get(0),
    )
}

/// Read the stored v17 runtime rows. Section rows without a `world_runtime`
/// row are an inconsistency, never an empty world.
pub fn load_stored_runtime_sqlite(
    connection: &Connection,
) -> Result<Option<StoredRuntime>, RuntimeSectionError> {
    let head = connection
        .query_row(
            "SELECT snapshot_version, head_json, manifest_json, last_event_sequence
             FROM world_runtime WHERE singleton = 1",
            [],
            |row| {
                Ok((
                    row.get::<_, i64>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, Option<i64>>(3)?,
                ))
            },
        )
        .optional()?;
    let mut statement = connection.prepare_cached(
        "SELECT section, revision, section_json, section_sha256
         FROM world_runtime_sections ORDER BY section LIMIT ?1",
    )?;
    let sections = statement
        .query_map([RUNTIME_SECTION_NAMES.len() as i64 + 1], |row| {
            Ok(RuntimeSectionRow {
                name: row.get(0)?,
                revision: row.get(1)?,
                json: row.get(2)?,
                sha256: row.get(3)?,
            })
        })?
        .collect::<Result<Vec<_>, _>>()?;
    match head {
        None if sections.is_empty() => Ok(None),
        None => Err(RuntimeSectionError::Inconsistent(
            "section rows without a runtime head",
        )),
        Some((snapshot_version, head_json, manifest_json, last_event_sequence)) => {
            Ok(Some(StoredRuntime {
                snapshot_version,
                head_json,
                manifest_json,
                last_event_sequence,
                sections,
            }))
        }
    }
}

/// Read the complete runtime document of a database at any schema version:
/// the single `snapshot_json` row before v17, the verified assembly after.
/// `maximum` bounds the assembled document.
pub fn read_runtime_document_sqlite(
    connection: &Connection,
    maximum: usize,
) -> Result<Option<(i64, String)>, RuntimeSectionError> {
    if sqlite_column_exists(connection, "world_runtime", "snapshot_json")? {
        return connection
            .query_row(
                "SELECT snapshot_version, snapshot_json FROM world_runtime WHERE singleton = 1",
                [],
                |row| Ok((row.get::<_, i64>(0)?, row.get::<_, String>(1)?)),
            )
            .optional()
            .map_err(Into::into);
    }
    load_stored_runtime_sqlite(connection)?
        .map(|stored| {
            assemble_stored_runtime(&stored, maximum)
                .map(|document| (stored.snapshot_version, document))
        })
        .transpose()
}

/// Replace the stored runtime with a complete rewrite of `json`, keeping the
/// manifest revision monotonic. Used by the v17 migration, offline tooling and
/// test fixtures; the server's commit path writes only changed sections.
pub fn write_runtime_document_sqlite(
    connection: &Connection,
    snapshot_version: i64,
    json: &str,
    last_event_sequence: Option<i64>,
) -> Result<(), RuntimeSectionError> {
    let previous_revision = connection
        .query_row(
            "SELECT manifest_json FROM world_runtime WHERE singleton = 1",
            [],
            |row| row.get::<_, String>(0),
        )
        .optional()?
        .and_then(|manifest| RuntimeManifest::parse(&manifest).ok())
        .map_or(0, |manifest| manifest.revision);
    let revision = previous_revision
        .checked_add(1)
        .ok_or(RuntimeSectionError::InvalidManifest)?;
    let (head_json, manifest, rows) = full_runtime_rows(json, revision);
    connection.execute("DELETE FROM world_runtime_sections", [])?;
    for row in &rows {
        connection.execute(
            "INSERT INTO world_runtime_sections
                 (section, revision, section_json, section_sha256, updated_at)
             VALUES (?1, ?2, ?3, ?4, unixepoch())",
            params![row.name, row.revision, row.json, row.sha256],
        )?;
    }
    connection.execute(
        "INSERT INTO world_runtime
             (singleton, snapshot_version, head_json, manifest_json,
              last_event_sequence, updated_at)
         VALUES (1, ?1, ?2, ?3, ?4, unixepoch())
         ON CONFLICT(singleton) DO UPDATE SET
             snapshot_version = excluded.snapshot_version,
             head_json = excluded.head_json,
             manifest_json = excluded.manifest_json,
             last_event_sequence = excluded.last_event_sequence,
             updated_at = excluded.updated_at",
        params![
            snapshot_version,
            head_json,
            manifest.to_json(),
            last_event_sequence
        ],
    )?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rows_of(json: &str) -> (String, RuntimeManifest, Vec<RuntimeSectionRow>) {
        full_runtime_rows(json, 1)
    }

    #[test]
    fn split_and_assemble_round_trip_the_document_semantically() {
        let json = r#"{"snapshotVersion":9,"revision":3,"itemInstances":{"a":{"x":1}},"npcs":{},"adventure":{"flags":[]},"areaResidency":{},"nextTradeId":1}"#;
        let (head, manifest, rows) = rows_of(json);
        assert_eq!(
            head,
            r#"{"nextTradeId":1,"revision":3,"snapshotVersion":9}"#
        );
        assert_eq!(rows.len(), 4);
        let assembled = assemble_runtime_document(&head, &manifest, &rows, 1 << 20).unwrap();
        let original: Value = serde_json::from_str(json).unwrap();
        let rebuilt: Value = serde_json::from_str(&assembled).unwrap();
        assert_eq!(original, rebuilt);
        assert_eq!(
            assembled_document_len(
                &head,
                rows.iter()
                    .map(|row| (row.name.as_str(), row.json.len() as u64))
            ),
            Some(assembled.len() as u64)
        );
        let parsed = RuntimeManifest::parse(&manifest.to_json()).unwrap();
        assert_eq!(parsed, manifest);
    }

    #[test]
    fn empty_head_and_non_object_documents_are_handled_exactly() {
        let (head, manifest, rows) = rows_of(r#"{"npcs":{"a":1}}"#);
        assert_eq!(head, "{}");
        assert_eq!(
            assemble_runtime_document(&head, &manifest, &rows, 1024).unwrap(),
            r#"{"npcs":{"a":1}}"#
        );
        let (head, manifest, rows) = rows_of("\"not an object\"");
        assert!(rows.is_empty() && manifest.sections.is_empty());
        assert_eq!(
            assemble_runtime_document(&head, &manifest, &rows, 1024).unwrap(),
            "\"not an object\""
        );
    }

    #[test]
    fn assembly_fails_closed_on_any_disagreement() {
        let json = r#"{"snapshotVersion":9,"itemInstances":{"a":1},"npcs":{"b":2}}"#;
        let (head, manifest, rows) = rows_of(json);

        let mut missing = rows.clone();
        missing.pop();
        assert!(assemble_runtime_document(&head, &manifest, &missing, 1024).is_err());

        let mut corrupt = rows.clone();
        corrupt[0].json = r#"{"a":2}"#.into();
        assert!(assemble_runtime_document(&head, &manifest, &corrupt, 1024).is_err());

        let mut rehashed = rows.clone();
        rehashed[0].json = r#"{"a":3}"#.into();
        rehashed[0].sha256 = sha256_hex(rehashed[0].json.as_bytes());
        assert!(assemble_runtime_document(&head, &manifest, &rehashed, 1024).is_err());

        let mut stale = rows.clone();
        stale[1].revision = 7;
        assert!(assemble_runtime_document(&head, &manifest, &stale, 1024).is_err());

        let mut extra = rows.clone();
        extra.push(RuntimeSectionRow {
            name: "trades".into(),
            revision: 1,
            json: "{}".into(),
            sha256: sha256_hex(b"{}"),
        });
        assert!(assemble_runtime_document(&head, &manifest, &extra, 1024).is_err());

        let mut duplicate = rows.clone();
        duplicate.push(rows[0].clone());
        assert!(assemble_runtime_document(&head, &manifest, &duplicate, 1024).is_err());

        // A head that already defines a section member cannot shadow it.
        assert!(assemble_runtime_document(r#"{"npcs":{}}"#, &manifest, &rows, 1024).is_err());
        assert!(assemble_runtime_document("[]", &manifest, &rows, 1024).is_err());

        // A crafted row cannot inject members even with a matching digest.
        let injected = r#"{},"snapshotVersion":1"#.to_owned();
        let (head, mut manifest, mut rows) = rows_of(r#"{"snapshotVersion":9,"npcs":{}}"#);
        rows[0].json = injected.clone();
        rows[0].sha256 = sha256_hex(injected.as_bytes());
        manifest.sections.get_mut("npcs").unwrap().bytes = injected.len() as u64;
        manifest.sections.get_mut("npcs").unwrap().sha256 = rows[0].sha256.clone();
        assert!(assemble_runtime_document(&head, &manifest, &rows, 1024).is_err());
    }

    #[test]
    fn assembly_enforces_the_document_ceiling_before_allocating() {
        let json = format!(r#"{{"snapshotVersion":9,"npcs":"{}"}}"#, "x".repeat(100));
        let (head, manifest, rows) = rows_of(&json);
        assert!(matches!(
            assemble_runtime_document(&head, &manifest, &rows, 64),
            Err(RuntimeSectionError::TooLarge { maximum: 64 })
        ));
        assert!(assemble_runtime_document(&head, &manifest, &rows, json.len()).is_ok());
    }

    #[test]
    fn manifest_parsing_is_strict() {
        let valid = RuntimeManifest {
            revision: 2,
            sections: BTreeMap::from([(
                "npcs".to_owned(),
                RuntimeManifestEntry {
                    revision: 2,
                    bytes: 2,
                    sha256: sha256_hex(b"{}"),
                },
            )]),
        };
        assert_eq!(RuntimeManifest::parse(&valid.to_json()).unwrap(), valid);
        for invalid in [
            "",
            "{}",
            r#"{"format":2,"revision":1,"sections":{}}"#,
            r#"{"format":1,"revision":0,"sections":{}}"#,
            r#"{"format":1,"revision":1,"sections":{},"extra":1}"#,
            r#"{"format":1,"revision":1,"sections":{"unknown":{"bytes":2,"revision":1,"sha256":"00"}}}"#,
            &valid
                .to_json()
                .replace("\"revision\":2,\"sha256\"", "\"revision\":3,\"sha256\""),
            &valid
                .to_json()
                .replace(&sha256_hex(b"{}"), &sha256_hex(b"{}").to_uppercase()),
        ] {
            assert!(RuntimeManifest::parse(invalid).is_err(), "{invalid}");
        }
    }
}
