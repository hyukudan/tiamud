//! PostgreSQL access to the sectioned world runtime (schema v17, ADR 0035).
//!
//! The storage format, manifest and verification live in
//! [`tiamud_persistence_sqlite::runtime_sections`]; this module only reads and
//! writes the same rows with PostgreSQL statements.

use postgres::GenericClient;
use thiserror::Error;
use tiamud_persistence_sqlite::runtime_sections::{
    RUNTIME_SECTION_NAMES, RuntimeManifest, RuntimeSectionError, RuntimeSectionRow, StoredRuntime,
    full_runtime_rows,
};

#[derive(Debug, Error)]
pub enum RuntimeRowsError {
    #[error(transparent)]
    Postgres(#[from] postgres::Error),
    #[error(transparent)]
    Sections(#[from] RuntimeSectionError),
}

/// Read the stored v17 runtime rows. `for_update` locks the head row (and
/// with it the right to rewrite sections) for the rest of the transaction.
/// Section rows without a head row are an inconsistency, never an empty world.
pub fn load_stored_runtime<C: GenericClient>(
    client: &mut C,
    for_update: bool,
) -> Result<Option<StoredRuntime>, RuntimeRowsError> {
    let head_sql = if for_update {
        "SELECT snapshot_version, head_json, manifest_json, last_event_sequence
         FROM world_runtime WHERE singleton = 1 FOR UPDATE"
    } else {
        "SELECT snapshot_version, head_json, manifest_json, last_event_sequence
         FROM world_runtime WHERE singleton = 1"
    };
    let head = client.query_opt(head_sql, &[])?;
    let limit = RUNTIME_SECTION_NAMES.len() as i64 + 1;
    let sections = client
        .query(
            "SELECT section, revision, section_json, section_sha256
             FROM world_runtime_sections ORDER BY section LIMIT $1",
            &[&limit],
        )?
        .into_iter()
        .map(|row| RuntimeSectionRow {
            name: row.get(0),
            revision: row.get(1),
            json: row.get(2),
            sha256: row.get(3),
        })
        .collect::<Vec<_>>();
    match head {
        None if sections.is_empty() => Ok(None),
        None => {
            Err(RuntimeSectionError::Inconsistent("section rows without a runtime head").into())
        }
        Some(row) => Ok(Some(StoredRuntime {
            snapshot_version: row.get(0),
            head_json: row.get(1),
            manifest_json: row.get(2),
            last_event_sequence: row.get(3),
            sections,
        })),
    }
}

/// Insert stored rows verbatim into an empty runtime (offline transfer). The
/// caller has already verified them.
pub fn insert_stored_runtime<C: GenericClient>(
    client: &mut C,
    stored: &StoredRuntime,
    updated_at: i64,
) -> Result<u64, postgres::Error> {
    for section in &stored.sections {
        client.execute(
            "INSERT INTO world_runtime_sections
                 (section, revision, section_json, section_sha256, updated_at)
             VALUES ($1, $2, $3, $4, $5)",
            &[
                &section.name,
                &section.revision,
                &section.json,
                &section.sha256,
                &updated_at,
            ],
        )?;
    }
    client.execute(
        "INSERT INTO world_runtime
             (singleton, snapshot_version, head_json, manifest_json,
              last_event_sequence, updated_at)
         VALUES (1, $1, $2, $3, $4, $5)",
        &[
            &stored.snapshot_version,
            &stored.head_json,
            &stored.manifest_json,
            &stored.last_event_sequence,
            &updated_at,
        ],
    )?;
    Ok(1 + stored.sections.len() as u64)
}

/// Replace the stored runtime with a complete rewrite of `json`, keeping the
/// manifest revision monotonic. Used by the v17 migration and tests; the
/// server's commit path writes only changed sections.
pub fn write_runtime_document<C: GenericClient>(
    client: &mut C,
    snapshot_version: i64,
    json: &str,
    last_event_sequence: Option<i64>,
) -> Result<(), RuntimeRowsError> {
    let previous_revision = client
        .query_opt(
            "SELECT manifest_json FROM world_runtime WHERE singleton = 1",
            &[],
        )?
        .and_then(|row| RuntimeManifest::parse(&row.get::<_, String>(0)).ok())
        .map_or(0, |manifest| manifest.revision);
    let revision = previous_revision
        .checked_add(1)
        .ok_or(RuntimeSectionError::InvalidManifest)?;
    let (head_json, manifest, rows) = full_runtime_rows(json, revision);
    client.execute("DELETE FROM world_runtime_sections", &[])?;
    for row in &rows {
        client.execute(
            "INSERT INTO world_runtime_sections
                 (section, revision, section_json, section_sha256)
             VALUES ($1, $2, $3, $4)",
            &[&row.name, &row.revision, &row.json, &row.sha256],
        )?;
    }
    client.execute(
        "INSERT INTO world_runtime
             (singleton, snapshot_version, head_json, manifest_json,
              last_event_sequence, updated_at)
         VALUES (1, $1, $2, $3, $4, floor(extract(epoch FROM now()))::BIGINT)
         ON CONFLICT(singleton) DO UPDATE SET
             snapshot_version = excluded.snapshot_version,
             head_json = excluded.head_json,
             manifest_json = excluded.manifest_json,
             last_event_sequence = excluded.last_event_sequence,
             updated_at = excluded.updated_at",
        &[
            &snapshot_version,
            &head_json,
            &manifest.to_json(),
            &last_event_sequence,
        ],
    )?;
    Ok(())
}
