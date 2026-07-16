//! Shared, transactional SQLite schema management for Tiamud.

use rusqlite::{Connection, OptionalExtension, Transaction};
use thiserror::Error;

pub const SCHEMA_VERSION: i64 = 9;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct MigrationReport {
    pub from: i64,
    pub to: i64,
}

#[derive(Debug, Error)]
pub enum SchemaError {
    #[error(transparent)]
    Sqlite(#[from] rusqlite::Error),
    #[error("database schema version {found} is newer than supported version {supported}")]
    UnsupportedVersion { found: i64, supported: i64 },
}

/// Bring an existing database (including the empty v0 database and the
/// name-keyed bootstrap schema) to the current schema in one transaction.
pub fn migrate(connection: &mut Connection) -> Result<MigrationReport, SchemaError> {
    let from = connection.query_row("PRAGMA user_version", [], |row| row.get(0))?;
    if from > SCHEMA_VERSION {
        return Err(SchemaError::UnsupportedVersion {
            found: from,
            supported: SCHEMA_VERSION,
        });
    }

    let transaction = connection.transaction()?;
    if table_exists(&transaction, "characters")?
        && !column_exists(&transaction, "characters", "id")?
    {
        migrate_name_keyed_characters(&transaction)?;
    }

    transaction.execute_batch(
        "CREATE TABLE IF NOT EXISTS accounts (
             id INTEGER PRIMARY KEY AUTOINCREMENT,
             name TEXT NOT NULL COLLATE NOCASE UNIQUE,
             created_at INTEGER NOT NULL DEFAULT (unixepoch())
         );
         CREATE TABLE IF NOT EXISTS characters (
             id INTEGER PRIMARY KEY AUTOINCREMENT,
             account_id INTEGER NOT NULL REFERENCES accounts(id) ON DELETE CASCADE,
             name TEXT NOT NULL COLLATE NOCASE,
             room_id TEXT NOT NULL,
             created_at INTEGER NOT NULL DEFAULT (unixepoch()),
             updated_at INTEGER NOT NULL DEFAULT (unixepoch()),
             UNIQUE(account_id, name)
         );
         CREATE INDEX IF NOT EXISTS characters_account_id_idx
             ON characters(account_id, id);",
    )?;

    if !column_exists(&transaction, "accounts", "password_hash")? {
        transaction.execute_batch("ALTER TABLE accounts ADD COLUMN password_hash TEXT;")?;
    }
    if !column_exists(&transaction, "accounts", "auth_state")? {
        transaction.execute_batch(
            "ALTER TABLE accounts ADD COLUMN auth_state TEXT NOT NULL DEFAULT 'guest'
                 CHECK (auth_state IN ('guest', 'registered'));",
        )?;
    }
    if !column_exists(&transaction, "accounts", "role")? {
        transaction.execute_batch(
            "ALTER TABLE accounts ADD COLUMN role TEXT NOT NULL DEFAULT 'player'
                 CHECK (role IN ('player', 'moderator', 'admin'));",
        )?;
    }
    if !column_exists(&transaction, "characters", "item_locations_json")? {
        transaction.execute_batch(
            "ALTER TABLE characters ADD COLUMN item_locations_json TEXT NOT NULL DEFAULT '{}';",
        )?;
    }
    if !column_exists(&transaction, "characters", "gameplay_json")? {
        transaction.execute_batch(
            "ALTER TABLE characters ADD COLUMN gameplay_json TEXT NOT NULL
                 DEFAULT '{\"level\":1,\"experience\":0,\"vitals\":{\"health\":20,\"maxHealth\":20},\"attributes\":{\"strength\":5,\"defence\":2},\"equipment\":{},\"effects\":[],\"recoveryAtTick\":null}'
                 CHECK(length(gameplay_json) <= 65536);",
        )?;
    }

    transaction.execute_batch(
        "CREATE TABLE IF NOT EXISTS auth_sessions (
             token_hash BLOB PRIMARY KEY CHECK(length(token_hash) = 32),
             account_id INTEGER NOT NULL REFERENCES accounts(id) ON DELETE CASCADE,
             created_at INTEGER NOT NULL,
             expires_at INTEGER NOT NULL,
             revoked_at INTEGER,
             CHECK(expires_at > created_at)
         );
         CREATE INDEX IF NOT EXISTS auth_sessions_account_idx
             ON auth_sessions(account_id, expires_at);
         CREATE TABLE IF NOT EXISTS domain_events (
             sequence INTEGER PRIMARY KEY AUTOINCREMENT,
             event_json TEXT NOT NULL CHECK(length(event_json) <= 65536),
             created_at INTEGER NOT NULL DEFAULT (unixepoch())
         );
         CREATE TABLE IF NOT EXISTS world_runtime (
             singleton INTEGER PRIMARY KEY CHECK(singleton = 1),
             snapshot_version INTEGER NOT NULL,
             snapshot_json TEXT NOT NULL CHECK(length(snapshot_json) <= 1048576),
             last_event_sequence INTEGER
                 CHECK(last_event_sequence IS NULL OR last_event_sequence >= 0),
             updated_at INTEGER NOT NULL DEFAULT (unixepoch())
         );
         CREATE TABLE IF NOT EXISTS social_ignores (
             owner_character_id INTEGER NOT NULL REFERENCES characters(id) ON DELETE CASCADE,
             target_character_id INTEGER NOT NULL REFERENCES characters(id) ON DELETE CASCADE,
             created_at INTEGER NOT NULL DEFAULT (unixepoch()),
             PRIMARY KEY(owner_character_id, target_character_id),
             CHECK(owner_character_id != target_character_id)
         );
         CREATE TABLE IF NOT EXISTS social_blocks (
             owner_character_id INTEGER NOT NULL REFERENCES characters(id) ON DELETE CASCADE,
             target_character_id INTEGER NOT NULL REFERENCES characters(id) ON DELETE CASCADE,
             created_at INTEGER NOT NULL DEFAULT (unixepoch()),
             PRIMARY KEY(owner_character_id, target_character_id),
             CHECK(owner_character_id != target_character_id)
         );
         CREATE TABLE IF NOT EXISTS social_channel_subscriptions (
             character_id INTEGER NOT NULL REFERENCES characters(id) ON DELETE CASCADE,
             channel TEXT NOT NULL
                 CHECK(channel IN ('general', 'newbie', 'staff')),
             created_at INTEGER NOT NULL DEFAULT (unixepoch()),
             PRIMARY KEY(character_id, channel)
         );
         CREATE INDEX IF NOT EXISTS social_channel_subscriptions_channel_idx
             ON social_channel_subscriptions(channel, character_id);
         CREATE TABLE IF NOT EXISTS moderation_mutes (
             id INTEGER PRIMARY KEY AUTOINCREMENT,
             account_id INTEGER NOT NULL REFERENCES accounts(id) ON DELETE CASCADE,
             imposed_by_account_id INTEGER NOT NULL REFERENCES accounts(id),
             reason TEXT NOT NULL CHECK(length(reason) BETWEEN 1 AND 1024),
             created_at INTEGER NOT NULL,
             expires_at INTEGER,
             revoked_at INTEGER,
             revoked_by_account_id INTEGER REFERENCES accounts(id),
             CHECK(expires_at IS NULL OR expires_at > created_at)
         );
         CREATE INDEX IF NOT EXISTS moderation_mutes_active_idx
             ON moderation_mutes(account_id, revoked_at, expires_at);
         CREATE TABLE IF NOT EXISTS account_bans (
             id INTEGER PRIMARY KEY AUTOINCREMENT,
             account_id INTEGER NOT NULL REFERENCES accounts(id) ON DELETE CASCADE,
             imposed_by_account_id INTEGER NOT NULL REFERENCES accounts(id),
             reason TEXT NOT NULL CHECK(length(reason) BETWEEN 1 AND 1024),
             created_at INTEGER NOT NULL,
             expires_at INTEGER,
             revoked_at INTEGER,
             revoked_by_account_id INTEGER REFERENCES accounts(id),
             CHECK(expires_at IS NULL OR expires_at > created_at)
         );
         CREATE INDEX IF NOT EXISTS account_bans_active_idx
             ON account_bans(account_id, revoked_at, expires_at);
         CREATE TABLE IF NOT EXISTS social_reports (
             id INTEGER PRIMARY KEY AUTOINCREMENT,
             reporter_character_id INTEGER NOT NULL REFERENCES characters(id) ON DELETE CASCADE,
             target_character_id INTEGER NOT NULL REFERENCES characters(id) ON DELETE CASCADE,
             reason TEXT NOT NULL CHECK(length(reason) BETWEEN 1 AND 2048),
             created_at INTEGER NOT NULL,
             status TEXT NOT NULL DEFAULT 'open'
                 CHECK(status IN ('open', 'reviewing', 'closed')),
             CHECK(reporter_character_id != target_character_id)
         );
         CREATE INDEX IF NOT EXISTS social_reports_status_idx
             ON social_reports(status, created_at);
         CREATE TABLE IF NOT EXISTS moderation_audit (
             sequence INTEGER PRIMARY KEY AUTOINCREMENT,
             actor_account_id INTEGER NOT NULL REFERENCES accounts(id),
             target_account_id INTEGER REFERENCES accounts(id),
             action TEXT NOT NULL CHECK(length(action) BETWEEN 1 AND 64),
             details_json TEXT NOT NULL CHECK(length(details_json) <= 4096),
             created_at INTEGER NOT NULL
         );
         CREATE INDEX IF NOT EXISTS moderation_audit_actor_idx
             ON moderation_audit(actor_account_id, sequence);
         CREATE TABLE IF NOT EXISTS character_adventure (
             character_id INTEGER PRIMARY KEY REFERENCES characters(id) ON DELETE CASCADE,
             state_json TEXT NOT NULL CHECK(length(state_json) <= 65536),
             updated_at INTEGER NOT NULL DEFAULT (unixepoch())
         );
         CREATE TABLE IF NOT EXISTS world_adventure (
             singleton INTEGER PRIMARY KEY CHECK(singleton = 1),
             state_json TEXT NOT NULL CHECK(length(state_json) <= 1048576),
             updated_at INTEGER NOT NULL DEFAULT (unixepoch())
         );
         PRAGMA user_version = 9;",
    )?;
    // A legacy snapshot does not prove which event sequence it covers. Keep a
    // NULL watermark until the next successful runtime commit establishes it;
    // the migration itself must never discard event history.
    if !column_exists(&transaction, "world_runtime", "last_event_sequence")? {
        transaction.execute_batch(
            "ALTER TABLE world_runtime ADD COLUMN last_event_sequence INTEGER
                 CHECK(last_event_sequence IS NULL OR last_event_sequence >= 0);",
        )?;
    }
    if from < 6 {
        let bootstrap_admin = transaction
            .query_row(
                "SELECT id FROM accounts
                 WHERE auth_state = 'registered'
                   AND NOT EXISTS(SELECT 1 FROM accounts WHERE role = 'admin')
                 ORDER BY id LIMIT 1",
                [],
                |row| row.get::<_, i64>(0),
            )
            .optional()?;
        if let Some(account_id) = bootstrap_admin {
            transaction.execute(
                "UPDATE accounts SET role = 'admin' WHERE id = ?1",
                [account_id],
            )?;
            transaction.execute(
                "INSERT INTO moderation_audit
                     (actor_account_id, target_account_id, action, details_json, created_at)
                 VALUES (?1, ?1, 'bootstrap_admin',
                         '{\"source\":\"schema_v6_first_registered_account\"}', unixepoch())",
                [account_id],
            )?;
        }
    }
    transaction.commit()?;

    Ok(MigrationReport {
        from,
        to: SCHEMA_VERSION,
    })
}

fn table_exists(connection: &Transaction<'_>, table: &str) -> rusqlite::Result<bool> {
    connection.query_row(
        "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type = 'table' AND name = ?1)",
        [table],
        |row| row.get(0),
    )
}

fn column_exists(
    connection: &Transaction<'_>,
    table: &str,
    column: &str,
) -> rusqlite::Result<bool> {
    let sql = format!("PRAGMA table_info({table})");
    let mut statement = connection.prepare(&sql)?;
    let mut rows = statement.query([])?;
    while let Some(row) = rows.next()? {
        if row.get::<_, String>(1)? == column {
            return Ok(true);
        }
    }
    Ok(false)
}

fn migrate_name_keyed_characters(transaction: &Transaction<'_>) -> rusqlite::Result<()> {
    transaction.execute_batch(
        "ALTER TABLE characters RENAME TO legacy_characters;
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
         INSERT INTO accounts (name)
             SELECT name FROM legacy_characters ORDER BY name COLLATE NOCASE;
         INSERT INTO characters (account_id, name, room_id, updated_at)
             SELECT accounts.id, legacy_characters.name, legacy_characters.room_id,
                    legacy_characters.updated_at
             FROM legacy_characters
             JOIN accounts ON accounts.name = legacy_characters.name COLLATE NOCASE
             ORDER BY accounts.id;
         DROP TABLE legacy_characters;",
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn creates_current_schema_from_an_empty_database() {
        let mut connection = Connection::open_in_memory().unwrap();
        let report = migrate(&mut connection).unwrap();

        assert_eq!(report, MigrationReport { from: 0, to: 9 });
        assert_eq!(
            connection
                .query_row("PRAGMA user_version", [], |row| row.get::<_, i64>(0))
                .unwrap(),
            9
        );
        assert!(
            connection
                .prepare("SELECT item_locations_json FROM characters")
                .is_ok()
        );
        assert!(
            connection
                .prepare("SELECT sequence, event_json FROM domain_events")
                .is_ok()
        );
        assert!(
            connection
                .prepare("SELECT gameplay_json FROM characters")
                .is_ok()
        );
        assert!(
            connection
                .prepare("SELECT snapshot_json, last_event_sequence FROM world_runtime")
                .is_ok()
        );
        assert!(connection.prepare("SELECT role FROM accounts").is_ok());
        assert!(
            connection
                .prepare("SELECT sequence, details_json FROM moderation_audit")
                .is_ok()
        );
        assert!(
            connection
                .prepare("SELECT character_id, state_json FROM character_adventure")
                .is_ok()
        );
        assert!(
            connection
                .prepare("SELECT singleton, state_json FROM world_adventure")
                .is_ok()
        );
        assert!(
            connection
                .prepare(
                    "SELECT character_id, channel, created_at
                     FROM social_channel_subscriptions",
                )
                .is_ok()
        );
    }

    #[test]
    fn migrates_the_name_keyed_bootstrap_without_losing_location() {
        let mut connection = Connection::open_in_memory().unwrap();
        connection
            .execute_batch(
                "CREATE TABLE characters (
                     name TEXT PRIMARY KEY COLLATE NOCASE,
                     room_id TEXT NOT NULL,
                     updated_at INTEGER NOT NULL DEFAULT (unixepoch())
                 );
                 INSERT INTO characters (name, room_id)
                     VALUES ('Legacy Hero', 'demo:north-gate');",
            )
            .unwrap();

        migrate(&mut connection).unwrap();
        let saved: (String, String) = connection
            .query_row("SELECT name, room_id FROM characters", [], |row| {
                Ok((row.get(0)?, row.get(1)?))
            })
            .unwrap();
        assert_eq!(saved, ("Legacy Hero".into(), "demo:north-gate".into()));
    }

    #[test]
    fn rejects_a_future_schema_without_mutating_it() {
        let mut connection = Connection::open_in_memory().unwrap();
        connection
            .execute_batch("PRAGMA user_version = 999;")
            .unwrap();

        assert!(matches!(
            migrate(&mut connection),
            Err(SchemaError::UnsupportedVersion { found: 999, .. })
        ));
        assert!(connection.prepare("SELECT * FROM accounts").is_err());
    }

    #[test]
    fn upgrades_v3_with_an_empty_domain_event_ledger() {
        let mut connection = Connection::open_in_memory().unwrap();
        connection
            .execute_batch(
                "CREATE TABLE accounts (
                     id INTEGER PRIMARY KEY,
                     name TEXT NOT NULL UNIQUE,
                     password_hash TEXT,
                     auth_state TEXT NOT NULL DEFAULT 'guest'
                 );
                 CREATE TABLE characters (
                     id INTEGER PRIMARY KEY,
                     account_id INTEGER NOT NULL REFERENCES accounts(id),
                     name TEXT NOT NULL,
                     room_id TEXT NOT NULL,
                     item_locations_json TEXT NOT NULL DEFAULT '{}'
                 );
                 PRAGMA user_version = 3;",
            )
            .unwrap();

        assert_eq!(
            migrate(&mut connection).unwrap(),
            MigrationReport { from: 3, to: 9 }
        );
        assert_eq!(
            connection
                .query_row("SELECT COUNT(*) FROM domain_events", [], |row| {
                    row.get::<_, i64>(0)
                })
                .unwrap(),
            0
        );
    }

    #[test]
    fn upgrades_v4_with_default_gameplay_and_an_empty_runtime() {
        let mut connection = Connection::open_in_memory().unwrap();
        connection
            .execute_batch(
                "CREATE TABLE accounts (
                     id INTEGER PRIMARY KEY,
                     name TEXT NOT NULL UNIQUE,
                     password_hash TEXT,
                     auth_state TEXT NOT NULL DEFAULT 'guest'
                 );
                 CREATE TABLE characters (
                     id INTEGER PRIMARY KEY,
                     account_id INTEGER NOT NULL REFERENCES accounts(id),
                     name TEXT NOT NULL,
                     room_id TEXT NOT NULL,
                     item_locations_json TEXT NOT NULL DEFAULT '{}'
                 );
                 CREATE TABLE domain_events (
                     sequence INTEGER PRIMARY KEY AUTOINCREMENT,
                     event_json TEXT NOT NULL
                 );
                 INSERT INTO accounts (id, name, auth_state)
                     VALUES (1, 'Legacy', 'registered');
                 INSERT INTO characters (id, account_id, name, room_id)
                     VALUES (7, 1, 'Veteran', 'demo:square');
                 PRAGMA user_version = 4;",
            )
            .unwrap();

        assert_eq!(
            migrate(&mut connection).unwrap(),
            MigrationReport { from: 4, to: 9 }
        );
        let gameplay: String = connection
            .query_row(
                "SELECT gameplay_json FROM characters WHERE id = 7",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert!(gameplay.contains("\"level\":1"));
        assert_eq!(
            connection
                .query_row("SELECT COUNT(*) FROM world_runtime", [], |row| row
                    .get::<_, i64>(0))
                .unwrap(),
            0
        );
    }

    #[test]
    fn upgrades_v5_with_social_policy_and_durable_audit_tables() {
        let mut connection = Connection::open_in_memory().unwrap();
        connection
            .execute_batch(
                "CREATE TABLE accounts (
                     id INTEGER PRIMARY KEY,
                     name TEXT NOT NULL UNIQUE,
                     password_hash TEXT,
                     auth_state TEXT NOT NULL DEFAULT 'guest'
                 );
                 CREATE TABLE characters (
                     id INTEGER PRIMARY KEY,
                     account_id INTEGER NOT NULL REFERENCES accounts(id),
                     name TEXT NOT NULL,
                     room_id TEXT NOT NULL,
                     item_locations_json TEXT NOT NULL DEFAULT '{}',
                     gameplay_json TEXT NOT NULL DEFAULT '{}'
                 );
                 CREATE TABLE auth_sessions (
                     token_hash BLOB PRIMARY KEY,
                     account_id INTEGER NOT NULL,
                     created_at INTEGER NOT NULL,
                     expires_at INTEGER NOT NULL,
                     revoked_at INTEGER
                 );
                 CREATE TABLE domain_events (
                     sequence INTEGER PRIMARY KEY AUTOINCREMENT,
                     event_json TEXT NOT NULL
                 );
                 CREATE TABLE world_runtime (
                     singleton INTEGER PRIMARY KEY,
                     snapshot_version INTEGER NOT NULL,
                     snapshot_json TEXT NOT NULL,
                     updated_at INTEGER NOT NULL
                 );
                 INSERT INTO accounts (id, name, auth_state)
                     VALUES (1, 'Legacy', 'registered');
                 INSERT INTO characters (id, account_id, name, room_id)
                     VALUES (7, 1, 'Veteran', 'demo:square');
                 PRAGMA user_version = 5;",
            )
            .unwrap();

        assert_eq!(
            migrate(&mut connection).unwrap(),
            MigrationReport { from: 5, to: 9 }
        );
        assert_eq!(
            connection
                .query_row("SELECT role FROM accounts WHERE id = 1", [], |row| {
                    row.get::<_, String>(0)
                })
                .unwrap(),
            "admin"
        );
        assert_eq!(
            connection
                .query_row("SELECT COUNT(*) FROM moderation_audit", [], |row| {
                    row.get::<_, i64>(0)
                })
                .unwrap(),
            1
        );
    }

    #[test]
    fn upgrades_v6_without_replacing_social_or_gameplay_state() {
        let mut connection = Connection::open_in_memory().unwrap();
        connection
            .execute_batch(
                "CREATE TABLE accounts (
                     id INTEGER PRIMARY KEY,
                     name TEXT NOT NULL UNIQUE,
                     password_hash TEXT,
                     auth_state TEXT NOT NULL DEFAULT 'guest',
                     role TEXT NOT NULL DEFAULT 'player'
                 );
                 CREATE TABLE characters (
                     id INTEGER PRIMARY KEY,
                     account_id INTEGER NOT NULL REFERENCES accounts(id),
                     name TEXT NOT NULL,
                     room_id TEXT NOT NULL,
                     item_locations_json TEXT NOT NULL DEFAULT '{}',
                     gameplay_json TEXT NOT NULL DEFAULT '{}'
                 );
                 CREATE TABLE social_ignores (
                     owner_character_id INTEGER NOT NULL,
                     target_character_id INTEGER NOT NULL,
                     created_at INTEGER NOT NULL,
                     PRIMARY KEY(owner_character_id, target_character_id)
                 );
                 INSERT INTO accounts (id, name, auth_state, role)
                     VALUES (1, 'Legacy', 'registered', 'admin');
                 INSERT INTO characters
                     (id, account_id, name, room_id, gameplay_json)
                     VALUES (7, 1, 'Veteran', 'demo:square', '{\"level\":9}');
                 PRAGMA user_version = 6;",
            )
            .unwrap();

        assert_eq!(
            migrate(&mut connection).unwrap(),
            MigrationReport { from: 6, to: 9 }
        );
        assert!(connection.prepare("SELECT role FROM accounts").is_ok());
        assert!(
            connection
                .prepare("SELECT gameplay_json FROM characters")
                .is_ok()
        );
        assert!(connection.prepare("SELECT * FROM social_ignores").is_ok());
        assert!(
            connection
                .prepare("SELECT * FROM character_adventure")
                .is_ok()
        );
        assert!(connection.prepare("SELECT * FROM world_adventure").is_ok());
        assert_eq!(
            connection
                .query_row(
                    "SELECT gameplay_json FROM characters WHERE id = 7",
                    [],
                    |row| row.get::<_, String>(0),
                )
                .unwrap(),
            "{\"level\":9}"
        );
    }

    #[test]
    fn upgrades_v7_without_compacting_events_when_snapshot_is_absent() {
        let mut connection = Connection::open_in_memory().unwrap();
        connection
            .execute_batch(
                "CREATE TABLE domain_events (
                     sequence INTEGER PRIMARY KEY AUTOINCREMENT,
                     event_json TEXT NOT NULL,
                     created_at INTEGER NOT NULL DEFAULT (unixepoch())
                 );
                 CREATE TABLE world_runtime (
                     singleton INTEGER PRIMARY KEY,
                     snapshot_version INTEGER NOT NULL,
                     snapshot_json TEXT NOT NULL,
                     updated_at INTEGER NOT NULL DEFAULT (unixepoch())
                 );
                 INSERT INTO domain_events (event_json)
                     VALUES ('{\"type\":\"legacy-one\"}'),
                            ('{\"type\":\"legacy-two\"}');
                 PRAGMA user_version = 7;",
            )
            .unwrap();

        assert_eq!(
            migrate(&mut connection).unwrap(),
            MigrationReport { from: 7, to: 9 }
        );
        assert_eq!(
            connection
                .query_row("SELECT COUNT(*) FROM domain_events", [], |row| {
                    row.get::<_, i64>(0)
                })
                .unwrap(),
            2
        );
        assert_eq!(
            connection
                .query_row("SELECT COUNT(*) FROM world_runtime", [], |row| {
                    row.get::<_, i64>(0)
                })
                .unwrap(),
            0
        );
        assert!(
            connection
                .prepare("SELECT last_event_sequence FROM world_runtime")
                .is_ok()
        );
    }

    #[test]
    fn legacy_snapshot_watermark_stays_unknown_until_a_runtime_commit() {
        let mut connection = Connection::open_in_memory().unwrap();
        connection
            .execute_batch(
                "CREATE TABLE domain_events (
                     sequence INTEGER PRIMARY KEY AUTOINCREMENT,
                     event_json TEXT NOT NULL,
                     created_at INTEGER NOT NULL DEFAULT (unixepoch())
                 );
                 CREATE TABLE world_runtime (
                     singleton INTEGER PRIMARY KEY,
                     snapshot_version INTEGER NOT NULL,
                     snapshot_json TEXT NOT NULL,
                     updated_at INTEGER NOT NULL DEFAULT (unixepoch())
                 );
                 INSERT INTO domain_events (event_json)
                     VALUES ('{\"type\":\"legacy\"}');
                 INSERT INTO world_runtime
                     (singleton, snapshot_version, snapshot_json)
                     VALUES (1, 1, '{}');
                 PRAGMA user_version = 7;",
            )
            .unwrap();

        migrate(&mut connection).unwrap();

        assert_eq!(
            connection
                .query_row(
                    "SELECT last_event_sequence FROM world_runtime WHERE singleton = 1",
                    [],
                    |row| row.get::<_, Option<i64>>(0),
                )
                .unwrap(),
            None
        );
        assert_eq!(
            connection
                .query_row("SELECT COUNT(*) FROM domain_events", [], |row| {
                    row.get::<_, i64>(0)
                })
                .unwrap(),
            1
        );
    }

    #[test]
    fn upgrades_v8_by_adding_closed_channel_subscriptions_without_losing_characters() {
        let mut connection = Connection::open_in_memory().unwrap();
        migrate(&mut connection).unwrap();
        connection
            .execute_batch(
                "INSERT INTO accounts (id, name, auth_state, role)
                     VALUES (1, 'Legacy', 'registered', 'player');
                 INSERT INTO characters (id, account_id, name, room_id)
                     VALUES (7, 1, 'Veteran', 'demo:square');
                 DROP TABLE social_channel_subscriptions;
                 PRAGMA user_version = 8;",
            )
            .unwrap();

        assert_eq!(
            migrate(&mut connection).unwrap(),
            MigrationReport { from: 8, to: 9 }
        );
        assert_eq!(
            connection
                .query_row("SELECT name FROM characters WHERE id = 7", [], |row| {
                    row.get::<_, String>(0)
                })
                .unwrap(),
            "Veteran"
        );
        connection
            .execute(
                "INSERT INTO social_channel_subscriptions (character_id, channel)
                 VALUES (7, 'general')",
                [],
            )
            .unwrap();
        assert!(
            connection
                .execute(
                    "INSERT INTO social_channel_subscriptions (character_id, channel)
                     VALUES (7, 'unknown')",
                    [],
                )
                .is_err()
        );
    }

    #[test]
    fn current_schema_never_silently_recreates_an_admin() {
        let mut connection = Connection::open_in_memory().unwrap();
        migrate(&mut connection).unwrap();
        connection
            .execute(
                "INSERT INTO accounts (name, password_hash, auth_state, role)
                 VALUES ('Oldest Player', 'hash', 'registered', 'player')",
                [],
            )
            .unwrap();

        assert_eq!(
            migrate(&mut connection).unwrap(),
            MigrationReport { from: 9, to: 9 }
        );
        assert_eq!(
            connection
                .query_row(
                    "SELECT role FROM accounts WHERE name = 'Oldest Player'",
                    [],
                    |row| row.get::<_, String>(0),
                )
                .unwrap(),
            "player"
        );
        assert_eq!(
            connection
                .query_row("SELECT COUNT(*) FROM moderation_audit", [], |row| {
                    row.get::<_, i64>(0)
                })
                .unwrap(),
            0
        );
    }
}
