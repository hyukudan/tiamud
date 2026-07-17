use std::path::{Path, PathBuf};

use postgres::{Client, GenericClient};
use rusqlite::{Connection, OpenFlags, OptionalExtension};
use thiserror::Error;

use crate::{APPLICATION_TABLES, SCHEMA_VERSION, SchemaError, migrate_in_transaction};

const TABLES: [&str; 17] = APPLICATION_TABLES;
const MAX_RUNTIME_SNAPSHOT_BYTES: usize = 1024 * 1024;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TableCount {
    pub table: &'static str,
    pub rows: u64,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CopyReport {
    pub sqlite_schema_version: i64,
    pub postgres_schema_version: i32,
    pub tables: Vec<TableCount>,
    pub total_rows: u64,
}

#[derive(Debug, Error)]
pub enum TransferError {
    #[error("could not open SQLite source {path:?}: {source}")]
    OpenSource {
        path: PathBuf,
        #[source]
        source: rusqlite::Error,
    },
    #[error("SQLite source operation failed: {0}")]
    Sqlite(#[from] rusqlite::Error),
    #[error(transparent)]
    Schema(#[from] SchemaError),
    #[error("PostgreSQL operation failed: {0}")]
    Postgres(#[from] postgres::Error),
    #[error(
        "SQLite source schema is v{found}; migrate it explicitly to v{required} before transferring"
    )]
    UnsupportedSourceVersion { found: i64, required: i64 },
    #[error("SQLite source failed integrity_check: {problems:?}")]
    SourceIntegrity { problems: Vec<String> },
    #[error("SQLite source has {violations} foreign-key violation(s)")]
    SourceForeignKeys { violations: usize },
    #[error("SQLite source world runtime snapshot exceeds {maximum} bytes")]
    RuntimeSnapshotTooLarge { maximum: usize },
    #[error("SQLite source contains an invalid world runtime snapshot")]
    InvalidRuntimeSnapshot,
    #[error(
        "PostgreSQL target table {table} contains {rows} row(s); refusing to merge or overwrite"
    )]
    TargetNotEmpty { table: &'static str, rows: i64 },
}

/// Copy one consistent, current SQLite snapshot into an empty PostgreSQL schema.
///
/// The caller owns the PostgreSQL connection, which also makes it possible to
/// select a dedicated schema through `search_path`. Schema creation and all row
/// inserts share one destination transaction. No source writes and no
/// dual-write phase occur.
pub fn copy_sqlite_to_postgres(
    sqlite_path: impl AsRef<Path>,
    postgres: &mut Client,
) -> Result<CopyReport, TransferError> {
    let sqlite_path = sqlite_path.as_ref();
    let source = Connection::open_with_flags(
        sqlite_path,
        OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )
    .map_err(|source| TransferError::OpenSource {
        path: sqlite_path.to_owned(),
        source,
    })?;
    source.execute_batch("PRAGMA query_only = ON; PRAGMA busy_timeout = 5000; BEGIN DEFERRED;")?;
    validate_source(&source)?;

    let mut target = postgres.transaction()?;
    migrate_in_transaction(&mut target)?;
    lock_target_tables(&mut target)?;
    require_empty_target(&mut target)?;

    let mut tables = Vec::with_capacity(TABLES.len());
    tables.push(TableCount {
        table: "accounts",
        rows: copy_accounts(&source, &mut target)?,
    });
    tables.push(TableCount {
        table: "characters",
        rows: copy_characters(&source, &mut target)?,
    });
    tables.push(TableCount {
        table: "auth_sessions",
        rows: copy_auth_sessions(&source, &mut target)?,
    });
    tables.push(TableCount {
        table: "domain_events",
        rows: copy_domain_events(&source, &mut target)?,
    });
    tables.push(TableCount {
        table: "world_runtime",
        rows: copy_world_runtime(&source, &mut target)?,
    });
    tables.push(TableCount {
        table: "social_ignores",
        rows: copy_social_relation(&source, &mut target, "social_ignores")?,
    });
    tables.push(TableCount {
        table: "social_blocks",
        rows: copy_social_relation(&source, &mut target, "social_blocks")?,
    });
    tables.push(TableCount {
        table: "social_channel_subscriptions",
        rows: copy_social_channel_subscriptions(&source, &mut target)?,
    });
    tables.push(TableCount {
        table: "moderation_mutes",
        rows: copy_moderation_decisions(&source, &mut target, "moderation_mutes")?,
    });
    tables.push(TableCount {
        table: "account_bans",
        rows: copy_moderation_decisions(&source, &mut target, "account_bans")?,
    });
    tables.push(TableCount {
        table: "social_reports",
        rows: copy_social_reports(&source, &mut target)?,
    });
    tables.push(TableCount {
        table: "moderation_audit",
        rows: copy_moderation_audit(&source, &mut target)?,
    });
    tables.push(TableCount {
        table: "staff_profiles",
        rows: copy_staff_profiles(&source, &mut target)?,
    });
    tables.push(TableCount {
        table: "staff_capability_assignments",
        rows: copy_staff_capability_assignments(&source, &mut target)?,
    });
    tables.push(TableCount {
        table: "staff_audit",
        rows: copy_staff_audit(&source, &mut target)?,
    });
    tables.push(TableCount {
        table: "character_adventure",
        rows: copy_character_adventure(&source, &mut target)?,
    });
    tables.push(TableCount {
        table: "world_adventure",
        rows: copy_world_adventure(&source, &mut target)?,
    });

    reset_sequences(&mut target)?;
    source.execute_batch("COMMIT")?;
    target.commit()?;

    let total_rows = tables.iter().map(|table| table.rows).sum();
    Ok(CopyReport {
        sqlite_schema_version: tiamud_persistence_sqlite::SCHEMA_VERSION,
        postgres_schema_version: SCHEMA_VERSION,
        tables,
        total_rows,
    })
}

fn lock_target_tables<C: GenericClient>(target: &mut C) -> Result<(), postgres::Error> {
    // This prevents a live PostgreSQL writer from racing the empty-target check
    // and accidentally turning an offline copy into a merge.
    target.batch_execute(
        "LOCK TABLE accounts, characters, auth_sessions, domain_events,
                    world_runtime, social_ignores, social_blocks,
                    social_channel_subscriptions,
                    moderation_mutes, account_bans, social_reports,
                    moderation_audit, staff_profiles,
                    staff_capability_assignments, staff_audit,
                    character_adventure, world_adventure
         IN ACCESS EXCLUSIVE MODE",
    )
}

fn validate_source(source: &Connection) -> Result<(), TransferError> {
    let version = source.query_row("PRAGMA user_version", [], |row| row.get::<_, i64>(0))?;
    let required = tiamud_persistence_sqlite::SCHEMA_VERSION;
    if version != required {
        return Err(TransferError::UnsupportedSourceVersion {
            found: version,
            required,
        });
    }

    let mut statement = source.prepare("PRAGMA integrity_check")?;
    let problems = statement
        .query_map([], |row| row.get::<_, String>(0))?
        .collect::<Result<Vec<_>, _>>()?;
    if problems.as_slice() != ["ok"] {
        return Err(TransferError::SourceIntegrity { problems });
    }
    drop(statement);

    let violations = source
        .query_row("SELECT COUNT(*) FROM pragma_foreign_key_check", [], |row| {
            row.get::<_, i64>(0)
        })?
        .try_into()
        .unwrap_or(usize::MAX);
    if violations != 0 {
        return Err(TransferError::SourceForeignKeys { violations });
    }
    Ok(())
}

fn require_empty_target<C: GenericClient>(target: &mut C) -> Result<(), TransferError> {
    for &table in &TABLES {
        // Identifiers come exclusively from the static manifest above.
        let sql = format!("SELECT COUNT(*)::BIGINT FROM {table}");
        let rows: i64 = target.query_one(&sql, &[])?.get(0);
        if rows != 0 {
            return Err(TransferError::TargetNotEmpty { table, rows });
        }
    }
    Ok(())
}

fn copy_accounts<C: GenericClient>(
    source: &Connection,
    target: &mut C,
) -> Result<u64, TransferError> {
    let mut statement = source.prepare(
        "SELECT id, name, created_at, password_hash, auth_state, role
         FROM accounts ORDER BY id",
    )?;
    let mut rows = statement.query([])?;
    let mut count = 0;
    while let Some(row) = rows.next()? {
        let id: i64 = row.get(0)?;
        let name: String = row.get(1)?;
        let created_at: i64 = row.get(2)?;
        let password_hash: Option<String> = row.get(3)?;
        let auth_state: String = row.get(4)?;
        let role: String = row.get(5)?;
        target.execute(
            "INSERT INTO accounts
                 (id, name, created_at, password_hash, auth_state, role)
             VALUES ($1, $2, $3, $4, $5, $6)",
            &[&id, &name, &created_at, &password_hash, &auth_state, &role],
        )?;
        count += 1;
    }
    Ok(count)
}

fn copy_characters<C: GenericClient>(
    source: &Connection,
    target: &mut C,
) -> Result<u64, TransferError> {
    let mut statement = source.prepare(
        "SELECT id, account_id, name, room_id, created_at, updated_at,
                item_locations_json, gameplay_json
         FROM characters ORDER BY id",
    )?;
    let mut rows = statement.query([])?;
    let mut count = 0;
    while let Some(row) = rows.next()? {
        let id: i64 = row.get(0)?;
        let account_id: i64 = row.get(1)?;
        let name: String = row.get(2)?;
        let room_id: String = row.get(3)?;
        let created_at: i64 = row.get(4)?;
        let updated_at: i64 = row.get(5)?;
        let item_locations_json: String = row.get(6)?;
        let gameplay_json: String = row.get(7)?;
        target.execute(
            "INSERT INTO characters
                 (id, account_id, name, room_id, created_at, updated_at,
                  item_locations_json, gameplay_json)
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8)",
            &[
                &id,
                &account_id,
                &name,
                &room_id,
                &created_at,
                &updated_at,
                &item_locations_json,
                &gameplay_json,
            ],
        )?;
        count += 1;
    }
    Ok(count)
}

fn copy_auth_sessions<C: GenericClient>(
    source: &Connection,
    target: &mut C,
) -> Result<u64, TransferError> {
    let mut statement = source.prepare(
        "SELECT token_hash, account_id, created_at, expires_at, revoked_at
         FROM auth_sessions ORDER BY created_at, token_hash",
    )?;
    let mut rows = statement.query([])?;
    let mut count = 0;
    while let Some(row) = rows.next()? {
        let token_hash: Vec<u8> = row.get(0)?;
        let account_id: i64 = row.get(1)?;
        let created_at: i64 = row.get(2)?;
        let expires_at: i64 = row.get(3)?;
        let revoked_at: Option<i64> = row.get(4)?;
        target.execute(
            "INSERT INTO auth_sessions
                 (token_hash, account_id, created_at, expires_at, revoked_at)
             VALUES ($1, $2, $3, $4, $5)",
            &[
                &token_hash,
                &account_id,
                &created_at,
                &expires_at,
                &revoked_at,
            ],
        )?;
        count += 1;
    }
    Ok(count)
}

fn copy_domain_events<C: GenericClient>(
    source: &Connection,
    target: &mut C,
) -> Result<u64, TransferError> {
    let mut statement = source
        .prepare("SELECT sequence, event_json, created_at FROM domain_events ORDER BY sequence")?;
    let mut rows = statement.query([])?;
    let mut count = 0;
    while let Some(row) = rows.next()? {
        let sequence: i64 = row.get(0)?;
        let event_json: String = row.get(1)?;
        let created_at: i64 = row.get(2)?;
        target.execute(
            "INSERT INTO domain_events (sequence, event_json, created_at)
             VALUES ($1, $2, $3)",
            &[&sequence, &event_json, &created_at],
        )?;
        count += 1;
    }
    Ok(count)
}

fn copy_world_runtime<C: GenericClient>(
    source: &Connection,
    target: &mut C,
) -> Result<u64, TransferError> {
    let row = source
        .query_row(
            "SELECT singleton, snapshot_version, snapshot_json,
                    last_event_sequence, updated_at
             FROM world_runtime WHERE singleton = 1",
            [],
            |row| {
                Ok((
                    row.get::<_, i64>(0)?,
                    row.get::<_, i64>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, Option<i64>>(3)?,
                    row.get::<_, i64>(4)?,
                ))
            },
        )
        .optional()?;
    let Some((singleton, snapshot_version, snapshot_json, last_event_sequence, updated_at)) = row
    else {
        return Ok(0);
    };
    validate_runtime_snapshot(snapshot_version, &snapshot_json)?;
    target.execute(
        "INSERT INTO world_runtime
             (singleton, snapshot_version, snapshot_json,
              last_event_sequence, updated_at)
         VALUES ($1, $2, $3, $4, $5)",
        &[
            &singleton,
            &snapshot_version,
            &snapshot_json,
            &last_event_sequence,
            &updated_at,
        ],
    )?;
    Ok(1)
}

fn validate_runtime_snapshot(
    snapshot_version: i64,
    snapshot_json: &str,
) -> Result<(), TransferError> {
    if snapshot_json.len() > MAX_RUNTIME_SNAPSHOT_BYTES {
        return Err(TransferError::RuntimeSnapshotTooLarge {
            maximum: MAX_RUNTIME_SNAPSHOT_BYTES,
        });
    }
    let runtime_version =
        u32::try_from(snapshot_version).map_err(|_| TransferError::InvalidRuntimeSnapshot)?;
    let snapshot = tiamud_core::decode_world_runtime_snapshot(runtime_version, snapshot_json)
        .map_err(|_| TransferError::InvalidRuntimeSnapshot)?;
    tiamud_core::validate_world_runtime_snapshot_semantics(&snapshot)
        .map_err(|_| TransferError::InvalidRuntimeSnapshot)?;
    Ok(())
}

fn copy_social_relation<C: GenericClient>(
    source: &Connection,
    target: &mut C,
    table: &'static str,
) -> Result<u64, TransferError> {
    debug_assert!(matches!(table, "social_ignores" | "social_blocks"));
    let select = format!(
        "SELECT owner_character_id, target_character_id, created_at
         FROM {table} ORDER BY owner_character_id, target_character_id"
    );
    let insert = format!(
        "INSERT INTO {table}
             (owner_character_id, target_character_id, created_at)
         VALUES ($1, $2, $3)"
    );
    let mut statement = source.prepare(&select)?;
    let mut rows = statement.query([])?;
    let mut count = 0;
    while let Some(row) = rows.next()? {
        let owner: i64 = row.get(0)?;
        let target_character: i64 = row.get(1)?;
        let created_at: i64 = row.get(2)?;
        target.execute(&insert, &[&owner, &target_character, &created_at])?;
        count += 1;
    }
    Ok(count)
}

fn copy_social_channel_subscriptions<C: GenericClient>(
    source: &Connection,
    target: &mut C,
) -> Result<u64, TransferError> {
    let mut statement = source.prepare(
        "SELECT character_id, channel, created_at
         FROM social_channel_subscriptions
         ORDER BY character_id, channel",
    )?;
    let mut rows = statement.query([])?;
    let mut count = 0;
    while let Some(row) = rows.next()? {
        let character_id: i64 = row.get(0)?;
        let channel: String = row.get(1)?;
        let created_at: i64 = row.get(2)?;
        target.execute(
            "INSERT INTO social_channel_subscriptions
                 (character_id, channel, created_at)
             VALUES ($1, $2, $3)",
            &[&character_id, &channel, &created_at],
        )?;
        count += 1;
    }
    Ok(count)
}

fn copy_moderation_decisions<C: GenericClient>(
    source: &Connection,
    target: &mut C,
    table: &'static str,
) -> Result<u64, TransferError> {
    debug_assert!(matches!(table, "moderation_mutes" | "account_bans"));
    let select = format!(
        "SELECT id, account_id, imposed_by_account_id, reason, created_at,
                expires_at, revoked_at, revoked_by_account_id
         FROM {table} ORDER BY id"
    );
    let insert = format!(
        "INSERT INTO {table}
             (id, account_id, imposed_by_account_id, reason, created_at,
              expires_at, revoked_at, revoked_by_account_id)
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8)"
    );
    let mut statement = source.prepare(&select)?;
    let mut rows = statement.query([])?;
    let mut count = 0;
    while let Some(row) = rows.next()? {
        let id: i64 = row.get(0)?;
        let account_id: i64 = row.get(1)?;
        let imposed_by: i64 = row.get(2)?;
        let reason: String = row.get(3)?;
        let created_at: i64 = row.get(4)?;
        let expires_at: Option<i64> = row.get(5)?;
        let revoked_at: Option<i64> = row.get(6)?;
        let revoked_by: Option<i64> = row.get(7)?;
        target.execute(
            &insert,
            &[
                &id,
                &account_id,
                &imposed_by,
                &reason,
                &created_at,
                &expires_at,
                &revoked_at,
                &revoked_by,
            ],
        )?;
        count += 1;
    }
    Ok(count)
}

fn copy_social_reports<C: GenericClient>(
    source: &Connection,
    target: &mut C,
) -> Result<u64, TransferError> {
    let mut statement = source.prepare(
        "SELECT id, reporter_character_id, target_character_id, reason,
                created_at, status, resolution, resolution_reason,
                resolved_by_account_id, resolved_at
         FROM social_reports ORDER BY id",
    )?;
    let mut rows = statement.query([])?;
    let mut count = 0;
    while let Some(row) = rows.next()? {
        let id: i64 = row.get(0)?;
        let reporter: i64 = row.get(1)?;
        let target_character: i64 = row.get(2)?;
        let reason: String = row.get(3)?;
        let created_at: i64 = row.get(4)?;
        let status: String = row.get(5)?;
        let resolution: Option<String> = row.get(6)?;
        let resolution_reason: Option<String> = row.get(7)?;
        let resolved_by: Option<i64> = row.get(8)?;
        let resolved_at: Option<i64> = row.get(9)?;
        target.execute(
            "INSERT INTO social_reports
                 (id, reporter_character_id, target_character_id, reason,
                  created_at, status, resolution, resolution_reason,
                  resolved_by_account_id, resolved_at)
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10)",
            &[
                &id,
                &reporter,
                &target_character,
                &reason,
                &created_at,
                &status,
                &resolution,
                &resolution_reason,
                &resolved_by,
                &resolved_at,
            ],
        )?;
        count += 1;
    }
    Ok(count)
}

fn copy_moderation_audit<C: GenericClient>(
    source: &Connection,
    target: &mut C,
) -> Result<u64, TransferError> {
    let mut statement = source.prepare(
        "SELECT sequence, actor_account_id, target_account_id, action,
                details_json, created_at
         FROM moderation_audit ORDER BY sequence",
    )?;
    let mut rows = statement.query([])?;
    let mut count = 0;
    while let Some(row) = rows.next()? {
        let sequence: i64 = row.get(0)?;
        let actor: i64 = row.get(1)?;
        let target_account: Option<i64> = row.get(2)?;
        let action: String = row.get(3)?;
        let details_json: String = row.get(4)?;
        let created_at: i64 = row.get(5)?;
        target.execute(
            "INSERT INTO moderation_audit
                 (sequence, actor_account_id, target_account_id, action,
                  details_json, created_at)
             VALUES ($1, $2, $3, $4, $5, $6)",
            &[
                &sequence,
                &actor,
                &target_account,
                &action,
                &details_json,
                &created_at,
            ],
        )?;
        count += 1;
    }
    Ok(count)
}

fn copy_staff_profiles<C: GenericClient>(
    source: &Connection,
    target: &mut C,
) -> Result<u64, TransferError> {
    let mut statement = source.prepare(
        "SELECT account_id, profile_key, lore_title, assigned_by_account_id,
                created_at, updated_at
         FROM staff_profiles ORDER BY account_id",
    )?;
    let mut rows = statement.query([])?;
    let mut count = 0;
    while let Some(row) = rows.next()? {
        let account_id: i64 = row.get(0)?;
        let profile_key: String = row.get(1)?;
        let lore_title: Option<String> = row.get(2)?;
        let assigned_by: i64 = row.get(3)?;
        let created_at: i64 = row.get(4)?;
        let updated_at: i64 = row.get(5)?;
        target.execute(
            "INSERT INTO staff_profiles
                 (account_id, profile_key, lore_title, assigned_by_account_id,
                  created_at, updated_at)
             VALUES ($1, $2, $3, $4, $5, $6)",
            &[
                &account_id,
                &profile_key,
                &lore_title,
                &assigned_by,
                &created_at,
                &updated_at,
            ],
        )?;
        count += 1;
    }
    Ok(count)
}

fn copy_staff_capability_assignments<C: GenericClient>(
    source: &Connection,
    target: &mut C,
) -> Result<u64, TransferError> {
    let mut statement = source.prepare(
        "SELECT account_id, capability, assigned_by_account_id, created_at
         FROM staff_capability_assignments
         ORDER BY account_id, capability",
    )?;
    let mut rows = statement.query([])?;
    let mut count = 0;
    while let Some(row) = rows.next()? {
        let account_id: i64 = row.get(0)?;
        let capability: String = row.get(1)?;
        let assigned_by: i64 = row.get(2)?;
        let created_at: i64 = row.get(3)?;
        target.execute(
            "INSERT INTO staff_capability_assignments
                 (account_id, capability, assigned_by_account_id, created_at)
             VALUES ($1, $2, $3, $4)",
            &[&account_id, &capability, &assigned_by, &created_at],
        )?;
        count += 1;
    }
    Ok(count)
}

fn copy_staff_audit<C: GenericClient>(
    source: &Connection,
    target: &mut C,
) -> Result<u64, TransferError> {
    let mut statement = source.prepare(
        "SELECT sequence, actor_account_id, target_account_id, capability,
                action, reason, details_json, created_at
         FROM staff_audit ORDER BY sequence",
    )?;
    let mut rows = statement.query([])?;
    let mut count = 0;
    while let Some(row) = rows.next()? {
        let sequence: i64 = row.get(0)?;
        let actor: i64 = row.get(1)?;
        let target_account: Option<i64> = row.get(2)?;
        let capability: String = row.get(3)?;
        let action: String = row.get(4)?;
        let reason: String = row.get(5)?;
        let details_json: String = row.get(6)?;
        let created_at: i64 = row.get(7)?;
        target.execute(
            "INSERT INTO staff_audit
                 (sequence, actor_account_id, target_account_id, capability,
                  action, reason, details_json, created_at)
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8)",
            &[
                &sequence,
                &actor,
                &target_account,
                &capability,
                &action,
                &reason,
                &details_json,
                &created_at,
            ],
        )?;
        count += 1;
    }
    Ok(count)
}

fn copy_character_adventure<C: GenericClient>(
    source: &Connection,
    target: &mut C,
) -> Result<u64, TransferError> {
    let mut statement = source.prepare(
        "SELECT character_id, state_json, updated_at
         FROM character_adventure ORDER BY character_id",
    )?;
    let mut rows = statement.query([])?;
    let mut count = 0;
    while let Some(row) = rows.next()? {
        let character_id: i64 = row.get(0)?;
        let state_json: String = row.get(1)?;
        let updated_at: i64 = row.get(2)?;
        target.execute(
            "INSERT INTO character_adventure
                 (character_id, state_json, updated_at)
             VALUES ($1, $2, $3)",
            &[&character_id, &state_json, &updated_at],
        )?;
        count += 1;
    }
    Ok(count)
}

fn copy_world_adventure<C: GenericClient>(
    source: &Connection,
    target: &mut C,
) -> Result<u64, TransferError> {
    let row = source
        .query_row(
            "SELECT singleton, state_json, updated_at
             FROM world_adventure WHERE singleton = 1",
            [],
            |row| {
                Ok((
                    row.get::<_, i64>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, i64>(2)?,
                ))
            },
        )
        .optional()?;
    let Some((singleton, state_json, updated_at)) = row else {
        return Ok(0);
    };
    target.execute(
        "INSERT INTO world_adventure (singleton, state_json, updated_at)
         VALUES ($1, $2, $3)",
        &[&singleton, &state_json, &updated_at],
    )?;
    Ok(1)
}

fn reset_sequences<C: GenericClient>(target: &mut C) -> Result<(), postgres::Error> {
    target.batch_execute(
        "SELECT setval(pg_get_serial_sequence('accounts', 'id')::regclass,
                       COALESCE(MAX(id), 1), COUNT(*) > 0) FROM accounts;
         SELECT setval(pg_get_serial_sequence('characters', 'id')::regclass,
                       COALESCE(MAX(id), 1), COUNT(*) > 0) FROM characters;
         SELECT setval(pg_get_serial_sequence('domain_events', 'sequence')::regclass,
                       COALESCE(MAX(sequence), 1), COUNT(*) > 0) FROM domain_events;
         SELECT setval(pg_get_serial_sequence('moderation_mutes', 'id')::regclass,
                       COALESCE(MAX(id), 1), COUNT(*) > 0) FROM moderation_mutes;
         SELECT setval(pg_get_serial_sequence('account_bans', 'id')::regclass,
                       COALESCE(MAX(id), 1), COUNT(*) > 0) FROM account_bans;
         SELECT setval(pg_get_serial_sequence('social_reports', 'id')::regclass,
                       COALESCE(MAX(id), 1), COUNT(*) > 0) FROM social_reports;
         SELECT setval(pg_get_serial_sequence('moderation_audit', 'sequence')::regclass,
                       COALESCE(MAX(sequence), 1), COUNT(*) > 0) FROM moderation_audit;
         SELECT setval(pg_get_serial_sequence('staff_audit', 'sequence')::regclass,
                       COALESCE(MAX(sequence), 1), COUNT(*) > 0) FROM staff_audit;",
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use rusqlite::params;
    use tempfile::TempDir;

    fn current_source(path: &Path) -> Connection {
        let mut connection = Connection::open(path).unwrap();
        tiamud_persistence_sqlite::migrate(&mut connection).unwrap();
        connection
    }

    #[test]
    fn accepts_a_valid_current_source_snapshot() {
        let temporary = TempDir::new().unwrap();
        let path = temporary.path().join("state.sqlite3");
        let connection = current_source(&path);

        validate_source(&connection).unwrap();
    }

    #[test]
    fn runtime_copy_validates_the_original_version_before_normalizing() {
        let mut runtime = tiamud_core::World::demo().runtime_snapshot();
        let first_item = runtime.item_instances.keys().next().unwrap().clone();
        let item = runtime.item_instances.get_mut(&first_item).unwrap();
        item.binding = tiamud_core::ItemBinding::Character {
            character_id: tiamud_core::CharacterId(41),
        };
        item.location = tiamud_core::RuntimeItemLocation::Carried {
            character_id: tiamud_core::CharacterId(41),
        };
        let json = serde_json::to_string(&runtime).unwrap();
        validate_runtime_snapshot(i64::from(runtime.snapshot_version), &json).unwrap();
        assert!(matches!(
            validate_runtime_snapshot(2, &json),
            Err(TransferError::InvalidRuntimeSnapshot)
        ));

        let mut zero_next_trade = tiamud_core::World::demo().runtime_snapshot();
        zero_next_trade.next_trade_id = tiamud_core::TradeId(0);
        let json = serde_json::to_string(&zero_next_trade).unwrap();
        serde_json::from_str::<tiamud_core::WorldRuntimeSnapshot>(&json).unwrap();
        assert!(matches!(
            validate_runtime_snapshot(5, &json),
            Err(TransferError::InvalidRuntimeSnapshot)
        ));

        let mut invalid_offer = tiamud_core::World::demo().runtime_snapshot();
        let trade_id = tiamud_core::TradeId(1);
        let requester = tiamud_core::CharacterId(71);
        invalid_offer.next_trade_id = tiamud_core::TradeId(2);
        invalid_offer.trades.insert(
            trade_id,
            tiamud_core::TradeSnapshot {
                id: trade_id,
                requester,
                recipient: tiamud_core::CharacterId(72),
                room_id: "demo:lantern-square".into(),
                phase: tiamud_core::TradePhase::Requested,
                created_tick: invalid_offer.logical_tick,
                opened_tick: None,
                expires_at_tick: invalid_offer.logical_tick + tiamud_core::TRADE_REQUEST_TICKS,
                state_revision: 1,
                offers: std::collections::BTreeMap::from([(
                    requester,
                    tiamud_core::TradeOffer::default(),
                )]),
                confirmations: std::collections::BTreeMap::new(),
            },
        );
        let json = serde_json::to_string(&invalid_offer).unwrap();
        serde_json::from_str::<tiamud_core::WorldRuntimeSnapshot>(&json).unwrap();
        assert!(matches!(
            validate_runtime_snapshot(5, &json),
            Err(TransferError::InvalidRuntimeSnapshot)
        ));

        let mut v3 = serde_json::to_value(tiamud_core::World::demo().runtime_snapshot()).unwrap();
        v3["snapshotVersion"] = serde_json::json!(3);
        v3.as_object_mut().unwrap().remove("nextTradeId");
        v3.as_object_mut().unwrap().remove("trades");
        for item in v3["itemInstances"].as_object_mut().unwrap().values_mut() {
            item.as_object_mut().unwrap().remove("binding");
        }
        let v3_json = v3.to_string();
        validate_runtime_snapshot(3, &v3_json).unwrap();
        assert!(matches!(
            validate_runtime_snapshot(4, &v3_json),
            Err(TransferError::InvalidRuntimeSnapshot)
        ));

        let mut v4 = serde_json::to_value(tiamud_core::World::demo().runtime_snapshot()).unwrap();
        v4["snapshotVersion"] = serde_json::json!(4);
        v4.as_object_mut().unwrap().remove("nextTradeId");
        v4.as_object_mut().unwrap().remove("trades");
        validate_runtime_snapshot(4, &v4.to_string()).unwrap();
        v4["trades"] = serde_json::json!({});
        assert!(matches!(
            validate_runtime_snapshot(4, &v4.to_string()),
            Err(TransferError::InvalidRuntimeSnapshot)
        ));

        v3["itemInstances"][first_item.0.as_str()]["binding"] =
            serde_json::json!({"kind":"unbound"});
        assert!(matches!(
            validate_runtime_snapshot(3, &v3.to_string()),
            Err(TransferError::InvalidRuntimeSnapshot)
        ));
    }

    #[test]
    fn rejects_an_old_source_instead_of_mutating_it() {
        let connection = Connection::open_in_memory().unwrap();
        connection.execute_batch("PRAGMA user_version = 5").unwrap();

        let error = validate_source(&connection).unwrap_err();
        assert!(matches!(
            error,
            TransferError::UnsupportedSourceVersion {
                found: 5,
                required: 10
            }
        ));
        assert_eq!(
            connection
                .query_row("PRAGMA user_version", [], |row| row.get::<_, i64>(0))
                .unwrap(),
            5
        );
    }

    #[test]
    fn rejects_foreign_key_violations_before_opening_a_target_transaction() {
        let temporary = TempDir::new().unwrap();
        let path = temporary.path().join("state.sqlite3");
        let connection = current_source(&path);
        connection
            .execute_batch("PRAGMA foreign_keys = OFF")
            .unwrap();
        connection
            .execute(
                "INSERT INTO characters
                     (id, account_id, name, room_id, item_locations_json, gameplay_json)
                 VALUES (99, 404, 'orphan', 'room:start', '{}', '{}')",
                params![],
            )
            .unwrap();

        let error = validate_source(&connection).unwrap_err();
        assert!(matches!(
            error,
            TransferError::SourceForeignKeys { violations: 1 }
        ));
    }

    #[test]
    fn transfer_manifest_is_parent_before_child_and_complete() {
        assert_eq!(TABLES[0], "accounts");
        assert_eq!(TABLES[1], "characters");
        assert_eq!(TABLES.len(), 17);
        assert_eq!(TABLES.last(), Some(&"world_adventure"));
    }
}
