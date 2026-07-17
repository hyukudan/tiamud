//! Offline integrity checks and consistent backups for Tiamud operators.

use std::{
    fs::{self, File},
    io::{self, Read},
    path::{Path, PathBuf},
};

use rusqlite::{Connection, OpenFlags, OptionalExtension, TransactionBehavior, params};
use tempfile::Builder;
use thiserror::Error;
use tiamud_core::{StaffCapability, StaffProfileKey};
use tiamud_persistence_sqlite::{
    MigrationReport, SCHEMA_VERSION, SchemaError, migrate as migrate_schema,
};
use world_pack::{DocumentToolError, ValidationErrors, WorldPack, WorldPackDocument};

/// Database schema understood by the current Tiamud server.
///
/// Keep this in sync with `tiamud-server`'s persistence schema. Versions below
/// this value remain supported because the server can migrate them on startup.
pub const SUPPORTED_DATABASE_SCHEMA_VERSION: i64 = SCHEMA_VERSION;
const MAX_WORLD_PACK_BYTES: usize = 16 * 1024 * 1024;
const MAX_RUNTIME_SNAPSHOT_BYTES: usize = 1024 * 1024;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CheckReport {
    pub database_schema_version: i64,
    pub world_pack_id: String,
    pub room_count: usize,
    pub item_count: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BackupReport {
    pub output: PathBuf,
    pub database_schema_version: i64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RestoreReport {
    pub database: PathBuf,
    pub database_schema_version: i64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MigrateReport {
    pub database: PathBuf,
    pub backup: PathBuf,
    pub migration: MigrationReport,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RecoverAdminReport {
    pub database: PathBuf,
    pub account_id: i64,
    pub account_name: String,
}

#[derive(Debug, Error)]
pub enum AdminError {
    #[error("could not open SQLite database {path:?}: {source}")]
    OpenDatabase {
        path: PathBuf,
        #[source]
        source: rusqlite::Error,
    },
    #[error("could not open SQLite database {path:?} for migration: {source}")]
    OpenDatabaseForMigration {
        path: PathBuf,
        #[source]
        source: rusqlite::Error,
    },
    #[error("SQLite operation failed for {path:?}: {source}")]
    Database {
        path: PathBuf,
        #[source]
        source: rusqlite::Error,
    },
    #[error("database {path:?} failed integrity_check: {problems:?}")]
    Integrity {
        path: PathBuf,
        problems: Vec<String>,
    },
    #[error("database {path:?} has foreign-key violations: {violations:?}")]
    ForeignKeys {
        path: PathBuf,
        violations: Vec<String>,
    },
    #[error(
        "database {path:?} has unsupported schema version {found}; supported versions are 0 through {supported}"
    )]
    UnsupportedDatabaseSchema {
        path: PathBuf,
        found: i64,
        supported: i64,
    },
    #[error("database {path:?} contains a world runtime snapshot larger than {maximum} bytes")]
    RuntimeSnapshotTooLarge { path: PathBuf, maximum: usize },
    #[error("database {0:?} contains an invalid world runtime snapshot")]
    InvalidRuntimeSnapshot(PathBuf),
    #[error("could not open WorldPack {path:?}: {source}")]
    OpenWorldPack {
        path: PathBuf,
        #[source]
        source: io::Error,
    },
    #[error("could not read WorldPack {path:?}: {source}")]
    ReadWorldPack {
        path: PathBuf,
        #[source]
        source: io::Error,
    },
    #[error("WorldPack {path:?} contains {size} bytes; the limit is {max} bytes")]
    WorldPackTooLarge {
        path: PathBuf,
        size: usize,
        max: usize,
    },
    #[error("WorldPack {path:?} is not valid JSON: {source}")]
    WorldPackJson {
        path: PathBuf,
        #[source]
        source: serde_json::Error,
    },
    #[error("WorldPack {path:?} cannot be normalized: {source}")]
    WorldPackNormalization {
        path: PathBuf,
        #[source]
        source: DocumentToolError,
    },
    #[error("WorldPack {path:?} is invalid: {source}")]
    WorldPackValidation {
        path: PathBuf,
        #[source]
        source: ValidationErrors,
    },
    #[error("backup output {0:?} already exists; refusing to overwrite it")]
    OutputExists(PathBuf),
    #[error("backup output {0:?} must name a file")]
    InvalidOutput(PathBuf),
    #[error("backup path {0:?} is not valid UTF-8 and cannot be passed to SQLite")]
    NonUtf8Path(PathBuf),
    #[error("could not create backup parent directory {path:?}: {source}")]
    CreateParent {
        path: PathBuf,
        #[source]
        source: io::Error,
    },
    #[error("could not create a private staging directory in {path:?}: {source}")]
    CreateStagingDirectory {
        path: PathBuf,
        #[source]
        source: io::Error,
    },
    #[error("could not publish backup at {path:?}: {source}")]
    PublishBackup {
        path: PathBuf,
        #[source]
        source: io::Error,
    },
    #[error("could not copy verified backup {source_path:?} to staging {destination:?}: {source}")]
    CopyBackup {
        source_path: PathBuf,
        destination: PathBuf,
        #[source]
        source: io::Error,
    },
    #[error("could not flush staged database {path:?}: {source}")]
    FlushStagedDatabase {
        path: PathBuf,
        #[source]
        source: io::Error,
    },
    #[error("database migration failed for {path:?}: {source}")]
    Migration {
        path: PathBuf,
        #[source]
        source: SchemaError,
    },
    #[error(
        "database {path:?} uses schema v{found}; admin recovery requires current schema v{required}"
    )]
    AdminRecoverySchema {
        path: PathBuf,
        found: i64,
        required: i64,
    },
    #[error(
        "database {0:?} already has an administrator or implementor profile; use in-game staff assignment instead"
    )]
    AdminAlreadyExists(PathBuf),
    #[error("account {account:?} does not exist in database {path:?}")]
    AccountNotFound { path: PathBuf, account: String },
    #[error("account {account:?} is not registered and cannot become an administrator")]
    AccountNotRegistered { account: String },
}

/// Check a database without creating or migrating it, then validate its WorldPack.
pub fn check(
    database: impl AsRef<Path>,
    world_pack: impl AsRef<Path>,
) -> Result<CheckReport, AdminError> {
    let database = database.as_ref();
    let connection = open_read_only(database)?;
    let database_schema_version = validate_database(&connection, database)?;
    let pack = load_world_pack(world_pack.as_ref())?;

    Ok(CheckReport {
        database_schema_version,
        world_pack_id: pack.id,
        room_count: pack.rooms.len(),
        item_count: pack.initial_item_instances.len(),
    })
}

/// Create, verify, and atomically publish a consistent SQLite snapshot.
pub fn backup(
    database: impl AsRef<Path>,
    output: impl AsRef<Path>,
) -> Result<BackupReport, AdminError> {
    let database = database.as_ref();
    let output = output.as_ref();
    let file_name = output
        .file_name()
        .filter(|name| !name.is_empty())
        .ok_or_else(|| AdminError::InvalidOutput(output.to_owned()))?;
    if output.exists() {
        return Err(AdminError::OutputExists(output.to_owned()));
    }

    let parent = output
        .parent()
        .filter(|path| !path.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    fs::create_dir_all(parent).map_err(|source| AdminError::CreateParent {
        path: parent.to_owned(),
        source,
    })?;
    if output.exists() {
        return Err(AdminError::OutputExists(output.to_owned()));
    }

    let staging_directory = Builder::new()
        .prefix(".tiamud-backup-")
        .tempdir_in(parent)
        .map_err(|source| AdminError::CreateStagingDirectory {
            path: parent.to_owned(),
            source,
        })?;
    let staged = staging_directory.path().join(file_name);
    let staged_sqlite_path = staged
        .to_str()
        .ok_or_else(|| AdminError::NonUtf8Path(staged.clone()))?;

    let connection = open_read_only(database)?;
    connection
        .execute("VACUUM INTO ?1", params![staged_sqlite_path])
        .map_err(|source| AdminError::Database {
            path: database.to_owned(),
            source,
        })?;
    drop(connection);

    let snapshot = open_read_only(&staged)?;
    let database_schema_version = validate_database(&snapshot, &staged)?;
    drop(snapshot);

    // hard_link publishes the already verified file atomically and, unlike
    // rename, cannot replace a destination created concurrently.
    fs::hard_link(&staged, output).map_err(|source| {
        if source.kind() == io::ErrorKind::AlreadyExists {
            AdminError::OutputExists(output.to_owned())
        } else {
            AdminError::PublishBackup {
                path: output.to_owned(),
                source,
            }
        }
    })?;

    Ok(BackupReport {
        output: output.to_owned(),
        database_schema_version,
    })
}

/// Verify a previously created SQLite backup without modifying it.
pub fn verify_backup(backup: impl AsRef<Path>) -> Result<BackupReport, AdminError> {
    let backup = backup.as_ref();
    let connection = open_read_only(backup)?;
    let database_schema_version = validate_database(&connection, backup)?;
    Ok(BackupReport {
        output: backup.to_owned(),
        database_schema_version,
    })
}

/// Restore a verified backup to a new database path. Existing destinations are
/// never overwritten; operators can preserve or move the previous database
/// explicitly before publishing a rollback.
pub fn restore(
    backup: impl AsRef<Path>,
    database: impl AsRef<Path>,
) -> Result<RestoreReport, AdminError> {
    let backup = backup.as_ref();
    let database = database.as_ref();
    let verified = verify_backup(backup)?;
    let file_name = database
        .file_name()
        .filter(|name| !name.is_empty())
        .ok_or_else(|| AdminError::InvalidOutput(database.to_owned()))?;
    if database.exists() {
        return Err(AdminError::OutputExists(database.to_owned()));
    }
    let parent = database
        .parent()
        .filter(|path| !path.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    fs::create_dir_all(parent).map_err(|source| AdminError::CreateParent {
        path: parent.to_owned(),
        source,
    })?;

    let staging_directory = Builder::new()
        .prefix(".tiamud-restore-")
        .tempdir_in(parent)
        .map_err(|source| AdminError::CreateStagingDirectory {
            path: parent.to_owned(),
            source,
        })?;
    let staged = staging_directory.path().join(file_name);
    fs::copy(backup, &staged).map_err(|source| AdminError::CopyBackup {
        source_path: backup.to_owned(),
        destination: staged.clone(),
        source,
    })?;
    File::open(&staged)
        .and_then(|file| file.sync_all())
        .map_err(|source| AdminError::FlushStagedDatabase {
            path: staged.clone(),
            source,
        })?;

    let staged_connection = open_read_only(&staged)?;
    let staged_version = validate_database(&staged_connection, &staged)?;
    drop(staged_connection);
    if staged_version != verified.database_schema_version {
        return Err(AdminError::UnsupportedDatabaseSchema {
            path: staged,
            found: staged_version,
            supported: verified.database_schema_version,
        });
    }
    publish_without_overwrite(&staged, database)?;

    Ok(RestoreReport {
        database: database.to_owned(),
        database_schema_version: staged_version,
    })
}

/// Create a verified pre-migration backup and then apply the same transactional
/// schema migrator used by `tiamud-server`.
pub fn migrate(
    database: impl AsRef<Path>,
    backup_output: impl AsRef<Path>,
) -> Result<MigrateReport, AdminError> {
    let database = database.as_ref();
    let backup_output = backup_output.as_ref();
    backup(database, backup_output)?;

    let mut connection = Connection::open_with_flags(
        database,
        OpenFlags::SQLITE_OPEN_READ_WRITE | OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )
    .map_err(|source| AdminError::OpenDatabaseForMigration {
        path: database.to_owned(),
        source,
    })?;
    connection
        .execute_batch("PRAGMA foreign_keys = ON; PRAGMA busy_timeout = 5000;")
        .map_err(|source| database_error(database, source))?;
    let migration = migrate_schema(&mut connection).map_err(|source| AdminError::Migration {
        path: database.to_owned(),
        source,
    })?;
    validate_database(&connection, database)?;

    Ok(MigrateReport {
        database: database.to_owned(),
        backup: backup_output.to_owned(),
        migration,
    })
}

/// Recover an installation that deliberately or accidentally has no operational admin.
///
/// This is an offline break-glass operation: it refuses to replace normal
/// staff assignment while any administrator or implementor profile exists. It
/// installs one exact Implementor bundle and records both the canonical staff
/// audit and the retained legacy moderation audit in the same transaction.
pub fn recover_admin(
    database: impl AsRef<Path>,
    account_name: &str,
) -> Result<RecoverAdminReport, AdminError> {
    let database = database.as_ref();
    let mut connection = Connection::open_with_flags(
        database,
        OpenFlags::SQLITE_OPEN_READ_WRITE | OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )
    .map_err(|source| AdminError::OpenDatabaseForMigration {
        path: database.to_owned(),
        source,
    })?;
    connection
        .execute_batch("PRAGMA foreign_keys = ON; PRAGMA busy_timeout = 5000;")
        .map_err(|source| database_error(database, source))?;
    let version = validate_database(&connection, database)?;
    if version != SCHEMA_VERSION {
        return Err(AdminError::AdminRecoverySchema {
            path: database.to_owned(),
            found: version,
            required: SCHEMA_VERSION,
        });
    }

    let transaction = connection
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .map_err(|source| database_error(database, source))?;
    let has_admin = transaction
        .query_row(
            "SELECT EXISTS(
                 SELECT 1 FROM staff_profiles
                 WHERE profile_key IN ('administrator', 'implementor')
             )",
            [],
            |row| row.get::<_, bool>(0),
        )
        .map_err(|source| database_error(database, source))?;
    if has_admin {
        return Err(AdminError::AdminAlreadyExists(database.to_owned()));
    }

    let account = transaction
        .query_row(
            "SELECT id, name, auth_state FROM accounts WHERE name = ?1 COLLATE NOCASE",
            [account_name],
            |row| {
                Ok((
                    row.get::<_, i64>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                ))
            },
        )
        .optional()
        .map_err(|source| database_error(database, source))?
        .ok_or_else(|| AdminError::AccountNotFound {
            path: database.to_owned(),
            account: account_name.to_owned(),
        })?;
    if account.2 != "registered" {
        return Err(AdminError::AccountNotRegistered { account: account.1 });
    }

    transaction
        .execute(
            "UPDATE accounts SET role = 'admin' WHERE id = ?1",
            [account.0],
        )
        .map_err(|source| database_error(database, source))?;
    let now = transaction
        .query_row("SELECT unixepoch()", [], |row| row.get::<_, i64>(0))
        .map_err(|source| database_error(database, source))?;
    transaction
        .execute(
            "INSERT INTO staff_profiles
                 (account_id, profile_key, lore_title, assigned_by_account_id,
                  created_at, updated_at)
             VALUES (?1, ?2, NULL, ?1, ?3, ?3)
             ON CONFLICT(account_id) DO UPDATE SET
                 profile_key = excluded.profile_key,
                 lore_title = NULL,
                 assigned_by_account_id = excluded.assigned_by_account_id,
                 updated_at = excluded.updated_at",
            params![account.0, StaffProfileKey::Implementor.as_str(), now],
        )
        .map_err(|source| database_error(database, source))?;
    transaction
        .execute(
            "DELETE FROM staff_capability_assignments WHERE account_id = ?1",
            [account.0],
        )
        .map_err(|source| database_error(database, source))?;
    for capability in StaffCapability::ALL {
        transaction
            .execute(
                "INSERT INTO staff_capability_assignments
                     (account_id, capability, assigned_by_account_id, created_at)
                 VALUES (?1, ?2, ?1, ?3)",
                params![account.0, capability.as_str(), now],
            )
            .map_err(|source| database_error(database, source))?;
    }
    transaction
        .execute(
            "INSERT INTO moderation_audit
                 (actor_account_id, target_account_id, action, details_json, created_at)
             VALUES (?1, ?1, 'recover_admin',
                     '{\"profile\":\"implementor\",\"source\":\"tiamud-admin_offline_recovery\"}', ?2)",
            params![account.0, now],
        )
        .map_err(|source| database_error(database, source))?;
    transaction
        .execute(
            "INSERT INTO staff_audit
                 (actor_account_id, target_account_id, capability, action,
                  reason, details_json, created_at)
             VALUES (?1, ?1, ?2, 'recover_admin',
                     'Offline break-glass recovery',
                     '{\"profile\":\"implementor\",\"source\":\"tiamud-admin_offline_recovery\"}', ?3)",
            params![account.0, StaffCapability::StaffAssign.as_str(), now],
        )
        .map_err(|source| database_error(database, source))?;
    transaction
        .commit()
        .map_err(|source| database_error(database, source))?;

    Ok(RecoverAdminReport {
        database: database.to_owned(),
        account_id: account.0,
        account_name: account.1,
    })
}

fn publish_without_overwrite(staged: &Path, output: &Path) -> Result<(), AdminError> {
    fs::hard_link(staged, output).map_err(|source| {
        if source.kind() == io::ErrorKind::AlreadyExists {
            AdminError::OutputExists(output.to_owned())
        } else {
            AdminError::PublishBackup {
                path: output.to_owned(),
                source,
            }
        }
    })
}

fn open_read_only(path: &Path) -> Result<Connection, AdminError> {
    let connection = Connection::open_with_flags(
        path,
        OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )
    .map_err(|source| AdminError::OpenDatabase {
        path: path.to_owned(),
        source,
    })?;
    // SQLITE_OPEN_READ_ONLY prevents writes to the source while still allowing
    // `VACUUM INTO` to create a separate snapshot database.
    connection
        .execute_batch("PRAGMA busy_timeout = 5000;")
        .map_err(|source| AdminError::Database {
            path: path.to_owned(),
            source,
        })?;
    Ok(connection)
}

fn validate_database(connection: &Connection, path: &Path) -> Result<i64, AdminError> {
    let mut statement = connection
        .prepare("PRAGMA integrity_check")
        .map_err(|source| database_error(path, source))?;
    let results = statement
        .query_map([], |row| row.get::<_, String>(0))
        .map_err(|source| database_error(path, source))?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|source| database_error(path, source))?;
    if results.len() != 1 || results[0] != "ok" {
        return Err(AdminError::Integrity {
            path: path.to_owned(),
            problems: results,
        });
    }
    drop(statement);

    let mut statement = connection
        .prepare("PRAGMA foreign_key_check")
        .map_err(|source| database_error(path, source))?;
    let violations = statement
        .query_map([], |row| {
            let table = row.get::<_, String>(0)?;
            let row_id = row.get::<_, Option<i64>>(1)?;
            let parent = row.get::<_, String>(2)?;
            let constraint = row.get::<_, i64>(3)?;
            Ok(format!(
                "table={table:?}, rowid={row_id:?}, parent={parent:?}, constraint={constraint}"
            ))
        })
        .map_err(|source| database_error(path, source))?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|source| database_error(path, source))?;
    if !violations.is_empty() {
        return Err(AdminError::ForeignKeys {
            path: path.to_owned(),
            violations,
        });
    }
    drop(statement);

    let version = connection
        .query_row("PRAGMA user_version", [], |row| row.get::<_, i64>(0))
        .map_err(|source| database_error(path, source))?;
    if !(0..=SUPPORTED_DATABASE_SCHEMA_VERSION).contains(&version) {
        return Err(AdminError::UnsupportedDatabaseSchema {
            path: path.to_owned(),
            found: version,
            supported: SUPPORTED_DATABASE_SCHEMA_VERSION,
        });
    }
    validate_runtime_snapshot(connection, path)?;
    Ok(version)
}

fn validate_runtime_snapshot(connection: &Connection, path: &Path) -> Result<(), AdminError> {
    let has_table = connection
        .query_row(
            "SELECT EXISTS(
                 SELECT 1 FROM sqlite_master
                 WHERE type = 'table' AND name = 'world_runtime'
             )",
            [],
            |row| row.get::<_, bool>(0),
        )
        .map_err(|source| database_error(path, source))?;
    if !has_table {
        return Ok(());
    }
    let snapshot = connection
        .query_row(
            "SELECT snapshot_version, snapshot_json
             FROM world_runtime WHERE singleton = 1",
            [],
            |row| Ok((row.get::<_, i64>(0)?, row.get::<_, String>(1)?)),
        )
        .optional()
        .map_err(|source| database_error(path, source))?;
    let Some((version, json)) = snapshot else {
        return Ok(());
    };
    if json.len() > MAX_RUNTIME_SNAPSHOT_BYTES {
        return Err(AdminError::RuntimeSnapshotTooLarge {
            path: path.to_owned(),
            maximum: MAX_RUNTIME_SNAPSHOT_BYTES,
        });
    }
    let version =
        u32::try_from(version).map_err(|_| AdminError::InvalidRuntimeSnapshot(path.to_owned()))?;
    let snapshot = tiamud_core::decode_world_runtime_snapshot(version, &json)
        .map_err(|_| AdminError::InvalidRuntimeSnapshot(path.to_owned()))?;
    tiamud_core::validate_world_runtime_snapshot_semantics(&snapshot)
        .map_err(|_| AdminError::InvalidRuntimeSnapshot(path.to_owned()))?;
    Ok(())
}

fn database_error(path: &Path, source: rusqlite::Error) -> AdminError {
    AdminError::Database {
        path: path.to_owned(),
        source,
    }
}

fn load_world_pack(path: &Path) -> Result<WorldPack, AdminError> {
    let mut file = File::open(path).map_err(|source| AdminError::OpenWorldPack {
        path: path.to_owned(),
        source,
    })?;
    let mut bytes = Vec::new();
    file.by_ref()
        .take((MAX_WORLD_PACK_BYTES + 1) as u64)
        .read_to_end(&mut bytes)
        .map_err(|source| AdminError::ReadWorldPack {
            path: path.to_owned(),
            source,
        })?;
    if bytes.len() > MAX_WORLD_PACK_BYTES {
        return Err(AdminError::WorldPackTooLarge {
            path: path.to_owned(),
            size: bytes.len(),
            max: MAX_WORLD_PACK_BYTES,
        });
    }

    let document = WorldPackDocument::from_json(&bytes).map_err(|source| match source {
        DocumentToolError::Json(source) => AdminError::WorldPackJson {
            path: path.to_owned(),
            source,
        },
        DocumentToolError::InvalidDocument(source) => AdminError::WorldPackValidation {
            path: path.to_owned(),
            source,
        },
        source => AdminError::WorldPackNormalization {
            path: path.to_owned(),
            source,
        },
    })?;
    document
        .validate()
        .map_err(|source| AdminError::WorldPackValidation {
            path: path.to_owned(),
            source,
        })?;
    Ok(document.world)
}

#[cfg(test)]
mod tests {
    use super::*;
    use rusqlite::params;
    use std::{collections::BTreeSet, io::Write};
    use tempfile::TempDir;

    fn create_database(path: &Path) {
        let connection = Connection::open(path).unwrap();
        connection
            .execute_batch(
                "PRAGMA foreign_keys = ON;
                 CREATE TABLE accounts (id INTEGER PRIMARY KEY, name TEXT NOT NULL);
                 CREATE TABLE characters (
                     id INTEGER PRIMARY KEY,
                     account_id INTEGER NOT NULL REFERENCES accounts(id),
                     name TEXT NOT NULL
                 );
                 INSERT INTO accounts (id, name) VALUES (1, 'keeper');
                 INSERT INTO characters (id, account_id, name) VALUES (10, 1, 'Aster');
                 PRAGMA user_version = 3;",
            )
            .unwrap();
    }

    fn create_world_pack(path: &Path) {
        fs::write(
            path,
            r#"{
                "schemaVersion": 3,
                "id": "test-world",
                "name": "Test World",
                "startRoomId": "room:start",
                "items": [{
                    "id": "item:lamp",
                    "name": "lamp",
                    "description": "A steady lamp."
                }],
                "rooms": [{
                    "id": "room:start",
                    "title": "Start",
                    "description": "The beginning.",
                    "items": ["item:lamp"]
                }]
            }"#,
        )
        .unwrap();
    }

    #[test]
    fn check_accepts_a_sound_database_and_world_pack() {
        let temporary = TempDir::new().unwrap();
        let database = temporary.path().join("tiamud.sqlite3");
        let world = temporary.path().join("world.json");
        create_database(&database);
        create_world_pack(&world);

        let report = check(&database, &world).unwrap();

        assert_eq!(report.database_schema_version, 3);
        assert_eq!(report.world_pack_id, "test-world");
        assert_eq!(report.room_count, 1);
        assert_eq!(report.item_count, 1);
    }

    #[test]
    fn check_rejects_a_runtime_snapshot_with_a_discordant_original_version() {
        let temporary = TempDir::new().unwrap();
        let database = temporary.path().join("tiamud.sqlite3");
        let world = temporary.path().join("world.json");
        let mut connection = Connection::open(&database).unwrap();
        migrate_schema(&mut connection).unwrap();
        let runtime = tiamud_core::World::demo().runtime_snapshot();
        connection
            .execute(
                "INSERT INTO world_runtime
                     (singleton, snapshot_version, snapshot_json, updated_at)
                 VALUES (1, 2, ?1, unixepoch())",
                [serde_json::to_string(&runtime).unwrap()],
            )
            .unwrap();
        drop(connection);
        create_world_pack(&world);

        assert!(matches!(
            check(&database, &world),
            Err(AdminError::InvalidRuntimeSnapshot(path)) if path == database
        ));
    }

    #[test]
    fn check_accepts_strict_v3_v4_and_v5_runtime_snapshots() {
        for (case, snapshot_version) in [("v3", 3_i64), ("v4", 4_i64), ("v5", 5_i64)] {
            let temporary = TempDir::new().unwrap();
            let database = temporary.path().join(format!("tiamud-{case}.sqlite3"));
            let world = temporary.path().join("world.json");
            create_database(&database);
            let mut connection = Connection::open(&database).unwrap();
            migrate_schema(&mut connection).unwrap();
            let runtime_json = if snapshot_version == 3 {
                let mut runtime =
                    serde_json::to_value(tiamud_core::World::demo().runtime_snapshot()).unwrap();
                runtime["snapshotVersion"] = serde_json::json!(3);
                runtime.as_object_mut().unwrap().remove("nextTradeId");
                runtime.as_object_mut().unwrap().remove("trades");
                for item in runtime["itemInstances"]
                    .as_object_mut()
                    .unwrap()
                    .values_mut()
                {
                    item.as_object_mut().unwrap().remove("binding");
                }
                runtime.to_string()
            } else if snapshot_version == 4 {
                let mut runtime =
                    serde_json::to_value(tiamud_core::World::demo().runtime_snapshot()).unwrap();
                runtime["snapshotVersion"] = serde_json::json!(4);
                runtime.as_object_mut().unwrap().remove("nextTradeId");
                runtime.as_object_mut().unwrap().remove("trades");
                runtime.to_string()
            } else {
                let mut runtime = tiamud_core::World::demo().runtime_snapshot();
                let item = runtime.item_instances.values_mut().next().unwrap();
                item.location = tiamud_core::RuntimeItemLocation::Carried {
                    character_id: tiamud_core::CharacterId(10),
                };
                serde_json::to_string(&runtime).unwrap()
            };
            let decoded = tiamud_core::decode_world_runtime_snapshot(
                u32::try_from(snapshot_version).unwrap(),
                &runtime_json,
            )
            .unwrap();
            let mut restored = tiamud_core::World::demo();
            restored.restore_runtime_snapshot(decoded).unwrap();
            connection
                .execute(
                    "INSERT INTO world_runtime
                         (singleton, snapshot_version, snapshot_json, updated_at)
                     VALUES (1, ?1, ?2, unixepoch())",
                    params![snapshot_version, runtime_json],
                )
                .unwrap();
            drop(connection);
            create_world_pack(&world);

            check(&database, &world).unwrap();
        }
    }

    #[test]
    fn check_rejects_deserializable_v5_trade_invariant_violations() {
        let mut zero_next_trade = tiamud_core::World::demo().runtime_snapshot();
        zero_next_trade.next_trade_id = tiamud_core::TradeId(0);

        let mut invalid_offer = tiamud_core::World::demo().runtime_snapshot();
        let trade_id = tiamud_core::TradeId(1);
        let requester = tiamud_core::CharacterId(71);
        let recipient = tiamud_core::CharacterId(72);
        invalid_offer.next_trade_id = tiamud_core::TradeId(2);
        invalid_offer.trades.insert(
            trade_id,
            tiamud_core::TradeSnapshot {
                id: trade_id,
                requester,
                recipient,
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

        for (case, runtime) in [
            ("zero-next-trade", zero_next_trade),
            ("invalid-offer", invalid_offer),
        ] {
            let temporary = TempDir::new().unwrap();
            let database = temporary.path().join(format!("{case}.sqlite3"));
            let world = temporary.path().join("world.json");
            create_database(&database);
            let mut connection = Connection::open(&database).unwrap();
            migrate_schema(&mut connection).unwrap();
            let json = serde_json::to_string(&runtime).unwrap();
            serde_json::from_str::<tiamud_core::WorldRuntimeSnapshot>(&json).unwrap();
            connection
                .execute(
                    "INSERT INTO world_runtime
                         (singleton, snapshot_version, snapshot_json, updated_at)
                     VALUES (1, 5, ?1, unixepoch())",
                    [json],
                )
                .unwrap();
            drop(connection);
            create_world_pack(&world);

            assert!(matches!(
                check(&database, &world),
                Err(AdminError::InvalidRuntimeSnapshot(path)) if path == database
            ));
        }
    }

    #[test]
    fn check_rejects_trade_fields_smuggled_into_v4() {
        let temporary = TempDir::new().unwrap();
        let database = temporary.path().join("tiamud.sqlite3");
        let world = temporary.path().join("world.json");
        create_database(&database);
        let mut connection = Connection::open(&database).unwrap();
        migrate_schema(&mut connection).unwrap();
        let mut runtime =
            serde_json::to_value(tiamud_core::World::demo().runtime_snapshot()).unwrap();
        runtime["snapshotVersion"] = serde_json::json!(4);
        connection
            .execute(
                "INSERT INTO world_runtime
                     (singleton, snapshot_version, snapshot_json, updated_at)
                 VALUES (1, 4, ?1, unixepoch())",
                [runtime.to_string()],
            )
            .unwrap();
        drop(connection);
        create_world_pack(&world);

        assert!(matches!(
            check(&database, &world),
            Err(AdminError::InvalidRuntimeSnapshot(path)) if path == database
        ));
    }

    #[test]
    fn check_rejects_binding_smuggled_into_v3() {
        let temporary = TempDir::new().unwrap();
        let database = temporary.path().join("tiamud.sqlite3");
        let world = temporary.path().join("world.json");
        create_database(&database);
        let mut connection = Connection::open(&database).unwrap();
        migrate_schema(&mut connection).unwrap();
        let mut runtime =
            serde_json::to_value(tiamud_core::World::demo().runtime_snapshot()).unwrap();
        runtime["snapshotVersion"] = serde_json::json!(3);
        runtime.as_object_mut().unwrap().remove("nextTradeId");
        runtime.as_object_mut().unwrap().remove("trades");
        let item_id = runtime["itemInstances"]
            .as_object()
            .unwrap()
            .keys()
            .next()
            .unwrap()
            .clone();
        for item in runtime["itemInstances"]
            .as_object_mut()
            .unwrap()
            .values_mut()
        {
            item.as_object_mut().unwrap().remove("binding");
        }
        runtime["itemInstances"][&item_id]["binding"] = serde_json::json!({"kind":"unbound"});
        connection
            .execute(
                "INSERT INTO world_runtime
                     (singleton, snapshot_version, snapshot_json, updated_at)
                 VALUES (1, 3, ?1, unixepoch())",
                [runtime.to_string()],
            )
            .unwrap();
        drop(connection);
        create_world_pack(&world);

        assert!(matches!(
            check(&database, &world),
            Err(AdminError::InvalidRuntimeSnapshot(path)) if path == database
        ));
    }

    #[test]
    fn check_rejects_a_corrupt_database() {
        let temporary = TempDir::new().unwrap();
        let database = temporary.path().join("corrupt.sqlite3");
        let world = temporary.path().join("world.json");
        File::create(&database)
            .unwrap()
            .write_all(b"this is not sqlite")
            .unwrap();
        create_world_pack(&world);

        let error = check(&database, &world).unwrap_err();

        assert!(matches!(error, AdminError::Database { .. }));
    }

    #[test]
    fn check_reports_foreign_key_violations() {
        let temporary = TempDir::new().unwrap();
        let database = temporary.path().join("invalid-fk.sqlite3");
        let world = temporary.path().join("world.json");
        let connection = Connection::open(&database).unwrap();
        connection
            .execute_batch(
                "PRAGMA foreign_keys = OFF;
                 CREATE TABLE parent (id INTEGER PRIMARY KEY);
                 CREATE TABLE child (parent_id INTEGER REFERENCES parent(id));
                 INSERT INTO child (parent_id) VALUES (404);
                 PRAGMA user_version = 3;",
            )
            .unwrap();
        drop(connection);
        create_world_pack(&world);

        let error = check(&database, &world).unwrap_err();

        assert!(matches!(error, AdminError::ForeignKeys { .. }));
    }

    #[test]
    fn check_rejects_a_future_database_version() {
        let temporary = TempDir::new().unwrap();
        let database = temporary.path().join("future.sqlite3");
        let world = temporary.path().join("world.json");
        create_database(&database);
        Connection::open(&database)
            .unwrap()
            .execute_batch("PRAGMA user_version = 999;")
            .unwrap();
        create_world_pack(&world);

        let error = check(&database, &world).unwrap_err();

        assert!(matches!(
            error,
            AdminError::UnsupportedDatabaseSchema { found: 999, .. }
        ));
    }

    #[test]
    fn check_rejects_missing_start_room_and_item_references() {
        let temporary = TempDir::new().unwrap();
        let database = temporary.path().join("tiamud.sqlite3");
        let world = temporary.path().join("world.json");
        create_database(&database);
        fs::write(
            &world,
            r#"{
                "schemaVersion": 4,
                "id": "broken",
                "name": "Broken",
                "startRoomId": "room:missing",
                "initialItemInstances": [{
                    "id": "item:missing",
                    "prototypeId": "prototype:missing",
                    "location": { "kind": "room", "roomId": "room:missing" }
                }],
                "rooms": [{
                    "id": "room:start",
                    "title": "Start",
                    "description": "The beginning."
                }]
            }"#,
        )
        .unwrap();

        let error = check(&database, &world).unwrap_err();

        assert!(matches!(error, AdminError::WorldPackValidation { .. }));
        let message = error.to_string();
        assert!(message.contains("invalid"));
    }

    #[test]
    fn backup_rejects_an_existing_output_without_changing_it() {
        let temporary = TempDir::new().unwrap();
        let database = temporary.path().join("tiamud.sqlite3");
        let output = temporary.path().join("backup.sqlite3");
        create_database(&database);
        fs::write(&output, b"keep me").unwrap();

        let error = backup(&database, &output).unwrap_err();

        assert!(matches!(error, AdminError::OutputExists(_)));
        assert_eq!(fs::read(output).unwrap(), b"keep me");
    }

    #[test]
    fn backup_creates_parents_and_a_restorable_snapshot() {
        let temporary = TempDir::new().unwrap();
        let database = temporary.path().join("tiamud.sqlite3");
        let output = temporary.path().join("nested/backups/tiamud.sqlite3");
        create_database(&database);

        let report = backup(&database, &output).unwrap();

        assert_eq!(report.output, output);
        assert_eq!(report.database_schema_version, 3);
        let restored =
            Connection::open_with_flags(&report.output, OpenFlags::SQLITE_OPEN_READ_ONLY).unwrap();
        let character: String = restored
            .query_row(
                "SELECT name FROM characters WHERE account_id = ?1",
                params![1],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(character, "Aster");
        assert_eq!(
            restored
                .query_row("PRAGMA integrity_check", [], |row| row.get::<_, String>(0))
                .unwrap(),
            "ok"
        );
    }

    #[test]
    fn backup_rejects_a_future_schema_before_publishing() {
        let temporary = TempDir::new().unwrap();
        let database = temporary.path().join("future.sqlite3");
        let output = temporary.path().join("backup.sqlite3");
        create_database(&database);
        Connection::open(&database)
            .unwrap()
            .execute_batch("PRAGMA user_version = 999;")
            .unwrap();

        let error = backup(&database, &output).unwrap_err();

        assert!(matches!(
            error,
            AdminError::UnsupportedDatabaseSchema { found: 999, .. }
        ));
        assert!(!output.exists());
    }

    #[test]
    fn verify_and_restore_publish_an_identical_valid_database() {
        let temporary = TempDir::new().unwrap();
        let database = temporary.path().join("tiamud.sqlite3");
        let snapshot = temporary.path().join("backups/snapshot.sqlite3");
        let restored = temporary.path().join("restored/tiamud.sqlite3");
        create_database(&database);
        backup(&database, &snapshot).unwrap();

        assert_eq!(verify_backup(&snapshot).unwrap().database_schema_version, 3);
        let report = restore(&snapshot, &restored).unwrap();
        assert_eq!(report.database, restored);
        assert_eq!(report.database_schema_version, 3);
        let connection = open_read_only(&report.database).unwrap();
        let name: String = connection
            .query_row("SELECT name FROM characters WHERE id = 10", [], |row| {
                row.get(0)
            })
            .unwrap();
        assert_eq!(name, "Aster");

        assert!(matches!(
            restore(&snapshot, &report.database),
            Err(AdminError::OutputExists(_))
        ));
    }

    #[test]
    fn migrate_creates_a_safety_backup_and_uses_the_server_schema() {
        let temporary = TempDir::new().unwrap();
        let database = temporary.path().join("v2.sqlite3");
        let safety = temporary.path().join("before-migration.sqlite3");
        let connection = Connection::open(&database).unwrap();
        connection
            .execute_batch(
                "PRAGMA foreign_keys = ON;
                 CREATE TABLE accounts (
                     id INTEGER PRIMARY KEY AUTOINCREMENT,
                     name TEXT NOT NULL COLLATE NOCASE UNIQUE,
                     created_at INTEGER NOT NULL DEFAULT (unixepoch())
                 );
                 CREATE TABLE characters (
                     id INTEGER PRIMARY KEY AUTOINCREMENT,
                     account_id INTEGER NOT NULL REFERENCES accounts(id) ON DELETE CASCADE,
                     name TEXT NOT NULL COLLATE NOCASE,
                     room_id TEXT NOT NULL,
                     created_at INTEGER NOT NULL DEFAULT (unixepoch()),
                     updated_at INTEGER NOT NULL DEFAULT (unixepoch()),
                     UNIQUE(account_id, name)
                 );
                 INSERT INTO accounts (id, name) VALUES (1, 'legacy');
                 INSERT INTO characters (id, account_id, name, room_id)
                     VALUES (1, 1, 'Legacy', 'room:start');
                 PRAGMA user_version = 2;",
            )
            .unwrap();
        drop(connection);

        let report = migrate(&database, &safety).unwrap();
        assert_eq!(
            report.migration,
            MigrationReport {
                from: 2,
                to: SCHEMA_VERSION,
            }
        );
        assert_eq!(verify_backup(&safety).unwrap().database_schema_version, 2);
        let migrated = open_read_only(&database).unwrap();
        assert_eq!(
            validate_database(&migrated, &database).unwrap(),
            SCHEMA_VERSION
        );
        let auth_state: String = migrated
            .query_row("SELECT auth_state FROM accounts WHERE id = 1", [], |row| {
                row.get(0)
            })
            .unwrap();
        assert_eq!(auth_state, "guest");
    }

    #[test]
    fn offline_admin_recovery_installs_the_exact_implementor_bundle_and_is_single_use() {
        let temporary = TempDir::new().unwrap();
        let database = temporary.path().join("current.sqlite3");
        let mut connection = Connection::open(&database).unwrap();
        migrate_schema(&mut connection).unwrap();
        connection
            .execute_batch(
                "INSERT INTO accounts (name, password_hash, auth_state, role)
                     VALUES ('Keeper', 'hash', 'registered', 'player');
                 INSERT INTO accounts (name, password_hash, auth_state, role)
                     VALUES ('Legacy Admin', 'hash', 'registered', 'admin');
                 INSERT INTO staff_profiles
                     (account_id, profile_key, lore_title, assigned_by_account_id,
                      created_at, updated_at)
                     SELECT id, 'helper', 'Old title', id, 1, 1
                     FROM accounts WHERE name = 'Keeper';
                 INSERT INTO staff_capability_assignments
                     (account_id, capability, assigned_by_account_id, created_at)
                     SELECT id, 'reports.view', id, 1
                     FROM accounts WHERE name = 'Keeper';",
            )
            .unwrap();
        drop(connection);

        let report = recover_admin(&database, "keeper").unwrap();
        assert_eq!(report.account_name, "Keeper");
        let connection = Connection::open(&database).unwrap();
        assert_eq!(
            connection
                .query_row(
                    "SELECT role FROM accounts WHERE id = ?1",
                    [report.account_id],
                    |row| { row.get::<_, String>(0) }
                )
                .unwrap(),
            "admin"
        );
        assert_eq!(
            connection
                .query_row(
                    "SELECT profile_key, lore_title, assigned_by_account_id
                     FROM staff_profiles WHERE account_id = ?1",
                    [report.account_id],
                    |row| {
                        Ok((
                            row.get::<_, String>(0)?,
                            row.get::<_, Option<String>>(1)?,
                            row.get::<_, i64>(2)?,
                        ))
                    },
                )
                .unwrap(),
            (
                StaffProfileKey::Implementor.as_str().to_owned(),
                None,
                report.account_id,
            )
        );
        let capabilities = {
            let mut statement = connection
                .prepare(
                    "SELECT capability FROM staff_capability_assignments
                     WHERE account_id = ?1 ORDER BY capability",
                )
                .unwrap();
            statement
                .query_map([report.account_id], |row| row.get::<_, String>(0))
                .unwrap()
                .collect::<rusqlite::Result<BTreeSet<_>>>()
                .unwrap()
        };
        assert_eq!(
            capabilities,
            StaffCapability::ALL
                .into_iter()
                .map(|capability| capability.as_str().to_owned())
                .collect::<BTreeSet<_>>()
        );
        assert_eq!(
            connection
                .query_row(
                    "SELECT action FROM moderation_audit WHERE target_account_id = ?1",
                    [report.account_id],
                    |row| row.get::<_, String>(0),
                )
                .unwrap(),
            "recover_admin"
        );
        assert_eq!(
            connection
                .query_row(
                    "SELECT actor_account_id, target_account_id, capability, action,
                            reason, details_json
                     FROM staff_audit WHERE target_account_id = ?1",
                    [report.account_id],
                    |row| {
                        Ok((
                            row.get::<_, i64>(0)?,
                            row.get::<_, Option<i64>>(1)?,
                            row.get::<_, String>(2)?,
                            row.get::<_, String>(3)?,
                            row.get::<_, String>(4)?,
                            row.get::<_, String>(5)?,
                        ))
                    },
                )
                .unwrap(),
            (
                report.account_id,
                Some(report.account_id),
                StaffCapability::StaffAssign.as_str().to_owned(),
                "recover_admin".to_owned(),
                "Offline break-glass recovery".to_owned(),
                "{\"profile\":\"implementor\",\"source\":\"tiamud-admin_offline_recovery\"}"
                    .to_owned(),
            )
        );
        assert!(
            connection
                .execute(
                    "UPDATE staff_audit SET reason = 'rewritten'
                     WHERE target_account_id = ?1",
                    [report.account_id],
                )
                .is_err()
        );
        assert!(
            connection
                .execute(
                    "DELETE FROM staff_audit WHERE target_account_id = ?1",
                    [report.account_id],
                )
                .is_err()
        );
        drop(connection);

        assert!(matches!(
            recover_admin(&database, "Keeper"),
            Err(AdminError::AdminAlreadyExists(_))
        ));
    }

    #[test]
    fn offline_admin_recovery_refuses_an_existing_operational_profile() {
        let temporary = TempDir::new().unwrap();
        let database = temporary.path().join("existing-admin.sqlite3");
        let mut connection = Connection::open(&database).unwrap();
        migrate_schema(&mut connection).unwrap();
        connection
            .execute_batch(
                "INSERT INTO accounts (name, password_hash, auth_state, role)
                     VALUES ('Existing', 'hash', 'registered', 'player');
                 INSERT INTO accounts (name, password_hash, auth_state, role)
                     VALUES ('Candidate', 'hash', 'registered', 'player');
                 INSERT INTO staff_profiles
                     (account_id, profile_key, lore_title, assigned_by_account_id,
                      created_at, updated_at)
                     SELECT id, 'administrator', NULL, id, 1, 1
                     FROM accounts WHERE name = 'Existing';",
            )
            .unwrap();
        drop(connection);

        assert!(matches!(
            recover_admin(&database, "Candidate"),
            Err(AdminError::AdminAlreadyExists(_))
        ));

        let connection = Connection::open(&database).unwrap();
        assert_eq!(
            connection
                .query_row(
                    "SELECT role FROM accounts WHERE name = 'Candidate'",
                    [],
                    |row| row.get::<_, String>(0),
                )
                .unwrap(),
            "player"
        );
        assert_eq!(
            connection
                .query_row("SELECT COUNT(*) FROM staff_audit", [], |row| {
                    row.get::<_, i64>(0)
                })
                .unwrap(),
            0
        );
    }

    #[test]
    fn offline_admin_recovery_rolls_back_if_the_canonical_audit_cannot_commit() {
        let temporary = TempDir::new().unwrap();
        let database = temporary.path().join("failed-recovery.sqlite3");
        let mut connection = Connection::open(&database).unwrap();
        migrate_schema(&mut connection).unwrap();
        connection
            .execute_batch(
                "INSERT INTO accounts (name, password_hash, auth_state, role)
                     VALUES ('Keeper', 'hash', 'registered', 'player');
                 INSERT INTO staff_profiles
                     (account_id, profile_key, lore_title, assigned_by_account_id,
                      created_at, updated_at)
                     SELECT id, 'helper', 'Old title', id, 1, 1
                     FROM accounts WHERE name = 'Keeper';
                 INSERT INTO staff_capability_assignments
                     (account_id, capability, assigned_by_account_id, created_at)
                     SELECT id, 'reports.view', id, 1
                     FROM accounts WHERE name = 'Keeper';
                 CREATE TRIGGER fail_recover_staff_audit
                     BEFORE INSERT ON staff_audit
                     WHEN NEW.action = 'recover_admin'
                 BEGIN
                     SELECT RAISE(ABORT, 'injected staff audit failure');
                 END;",
            )
            .unwrap();
        drop(connection);

        assert!(matches!(
            recover_admin(&database, "Keeper"),
            Err(AdminError::Database { .. })
        ));

        let connection = Connection::open(&database).unwrap();
        assert_eq!(
            connection
                .query_row(
                    "SELECT accounts.role, staff_profiles.profile_key,
                            staff_profiles.lore_title
                     FROM accounts
                     JOIN staff_profiles ON staff_profiles.account_id = accounts.id
                     WHERE accounts.name = 'Keeper'",
                    [],
                    |row| {
                        Ok((
                            row.get::<_, String>(0)?,
                            row.get::<_, String>(1)?,
                            row.get::<_, Option<String>>(2)?,
                        ))
                    },
                )
                .unwrap(),
            (
                "player".to_owned(),
                "helper".to_owned(),
                Some("Old title".to_owned()),
            )
        );
        assert_eq!(
            connection
                .query_row(
                    "SELECT capability FROM staff_capability_assignments",
                    [],
                    |row| row.get::<_, String>(0),
                )
                .unwrap(),
            "reports.view"
        );
        assert_eq!(
            connection
                .query_row("SELECT COUNT(*) FROM moderation_audit", [], |row| {
                    row.get::<_, i64>(0)
                })
                .unwrap(),
            0
        );
        assert_eq!(
            connection
                .query_row("SELECT COUNT(*) FROM staff_audit", [], |row| {
                    row.get::<_, i64>(0)
                })
                .unwrap(),
            0
        );
    }
}
