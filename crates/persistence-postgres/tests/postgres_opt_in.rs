use std::{env, fs, process, time::SystemTime};

use postgres::{Client, NoTls};
use rusqlite::Connection;
use tempfile::TempDir;

/// Opt-in because the normal test suite must not require a PostgreSQL service.
/// The configured database needs CREATE privilege for a temporary schema.
#[test]
fn migrates_a_representative_current_database_when_postgres_is_available() {
    let Ok(url) = env::var("TIAMUD_TEST_POSTGRES_URL") else {
        return;
    };
    let nonce = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let schema = format!("tiamud_test_{}_{}", process::id(), nonce);

    let mut postgres = Client::connect(&url, NoTls).unwrap();
    postgres
        .batch_execute(&format!(
            "CREATE SCHEMA {schema}; SET search_path TO {schema}"
        ))
        .unwrap();

    let temporary = TempDir::new().unwrap();
    let sqlite_path = temporary.path().join("state.sqlite3");
    let mut sqlite = Connection::open(&sqlite_path).unwrap();
    tiamud_persistence_sqlite::migrate(&mut sqlite).unwrap();
    sqlite
        .execute_batch(
            "PRAGMA foreign_keys = ON;
             INSERT INTO accounts
                 (id, name, auth_state, role)
             VALUES
                 (1, 'Keeper', 'registered', 'admin'),
                 (2, 'Player', 'registered', 'player');
             INSERT INTO characters
                 (id, account_id, name, room_id, item_locations_json, gameplay_json)
             VALUES
                 (10, 1, 'Aster', 'demo:square', '{}', '{}'),
                 (11, 2, 'Bryn', 'demo:square', '{}', '{}');
             INSERT INTO social_ignores
                 (owner_character_id, target_character_id)
             VALUES (10, 11);
             INSERT INTO social_channel_subscriptions
                 (character_id, channel, created_at)
             VALUES (10, 'general', 900);
             INSERT INTO domain_events (sequence, event_json)
             VALUES (7, '{\"type\":\"test\"}');
             INSERT INTO world_runtime
                 (singleton, snapshot_version, snapshot_json, last_event_sequence)
             VALUES (1, 4, '{\"tick\":4}', 7);
             INSERT INTO character_adventure (character_id, state_json)
             VALUES (10, '{\"coins\":40}');
             INSERT INTO world_adventure (singleton, state_json)
             VALUES (1, '{\"weather\":\"rain\"}');
             INSERT INTO moderation_audit
                 (sequence, actor_account_id, target_account_id, action,
                  details_json, created_at)
             VALUES (3, 1, 2, 'role.set', '{}', 1000);",
        )
        .unwrap();
    let gameplay_json = r#"{"lineageId":"tiamud:mortal","backgroundId":"tiamud:city-guard","baseClassId":"tiamud:vanguard","activeClassId":"tiamud:bastion","classHistory":["tiamud:vanguard","tiamud:bastion"],"progressionPoints":{"talent":0},"talentNodeRanks":{"tiamud:martial-talents":{"tiamud:guarded-stance":1}},"tags":["tiamud:founder"],"receivedSignals":["tiamud:heard-call"]}"#;
    sqlite
        .execute(
            "UPDATE characters SET gameplay_json = ?1 WHERE id = 10",
            [gameplay_json],
        )
        .unwrap();
    drop(sqlite);

    let report =
        tiamud_persistence_postgres::copy_sqlite_to_postgres(&sqlite_path, &mut postgres).unwrap();
    assert_eq!(report.total_rows, 11);
    assert_eq!(
        postgres
            .query_one("SELECT COUNT(*)::BIGINT FROM characters", &[])
            .unwrap()
            .get::<_, i64>(0),
        2
    );
    assert_eq!(
        postgres
            .query_one("SELECT gameplay_json FROM characters WHERE id = 10", &[])
            .unwrap()
            .get::<_, String>(0),
        gameplay_json,
        "offline transfer must preserve the native v3 snapshot byte-for-byte"
    );
    assert_eq!(
        postgres
            .query_one("SELECT nextval('domain_events_sequence_seq')", &[])
            .unwrap()
            .get::<_, i64>(0),
        8
    );
    assert_eq!(
        postgres
            .query_one("SELECT COUNT(*)::BIGINT FROM character_adventure", &[])
            .unwrap()
            .get::<_, i64>(0),
        1
    );
    assert_eq!(
        postgres
            .query_one("SELECT COUNT(*)::BIGINT FROM world_adventure", &[])
            .unwrap()
            .get::<_, i64>(0),
        1
    );
    assert_eq!(
        postgres
            .query_one(
                "SELECT channel FROM social_channel_subscriptions WHERE character_id = 10",
                &[],
            )
            .unwrap()
            .get::<_, String>(0),
        "general"
    );
    assert_eq!(
        tiamud_persistence_postgres::migrate(&mut postgres).unwrap(),
        tiamud_persistence_postgres::MigrationReport { from: 9, to: 9 }
    );
    assert_eq!(
        postgres
            .query_one(
                "SELECT last_event_sequence FROM world_runtime WHERE singleton = 1",
                &[],
            )
            .unwrap()
            .get::<_, Option<i64>>(0),
        Some(7)
    );
    assert!(matches!(
        tiamud_persistence_postgres::copy_sqlite_to_postgres(&sqlite_path, &mut postgres),
        Err(tiamud_persistence_postgres::TransferError::TargetNotEmpty {
            table: "accounts",
            rows: 2
        })
    ));
    postgres
        .batch_execute(
            "DROP TABLE character_adventure;
             DROP TABLE world_adventure;
             DROP TABLE social_channel_subscriptions;
             ALTER TABLE world_runtime DROP COLUMN last_event_sequence;
             UPDATE tiamud_schema SET version = 6 WHERE singleton = 1",
        )
        .unwrap();
    assert_eq!(
        tiamud_persistence_postgres::migrate(&mut postgres).unwrap(),
        tiamud_persistence_postgres::MigrationReport { from: 6, to: 9 }
    );
    assert!(
        postgres
            .query_one("SELECT to_regclass('character_adventure') IS NOT NULL", &[])
            .unwrap()
            .get::<_, bool>(0)
    );
    assert!(
        postgres
            .query_one(
                "SELECT to_regclass('social_channel_subscriptions') IS NOT NULL",
                &[],
            )
            .unwrap()
            .get::<_, bool>(0)
    );
    assert_eq!(
        postgres
            .query_one(
                "SELECT last_event_sequence FROM world_runtime WHERE singleton = 1",
                &[],
            )
            .unwrap()
            .get::<_, Option<i64>>(0),
        None
    );
    assert_eq!(
        postgres
            .query_one("SELECT COUNT(*)::BIGINT FROM domain_events", &[])
            .unwrap()
            .get::<_, i64>(0),
        1
    );

    postgres.batch_execute("SET search_path TO public").unwrap();
    postgres
        .batch_execute(&format!("DROP SCHEMA {schema} CASCADE"))
        .unwrap();
    fs::remove_file(sqlite_path).ok();
}

#[test]
fn rolls_back_schema_and_rows_when_a_source_value_breaks_a_postgres_limit() {
    let Ok(url) = env::var("TIAMUD_TEST_POSTGRES_URL") else {
        return;
    };
    let nonce = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let schema = format!("tiamud_test_rollback_{}_{}", process::id(), nonce);

    let mut postgres = Client::connect(&url, NoTls).unwrap();
    postgres
        .batch_execute(&format!(
            "CREATE SCHEMA {schema}; SET search_path TO {schema}"
        ))
        .unwrap();

    let temporary = TempDir::new().unwrap();
    let sqlite_path = temporary.path().join("invalid-state.sqlite3");
    let mut sqlite = Connection::open(&sqlite_path).unwrap();
    tiamud_persistence_sqlite::migrate(&mut sqlite).unwrap();
    let oversized_name = "x".repeat(65);
    sqlite
        .execute(
            "INSERT INTO accounts (id, name) VALUES (1, ?1)",
            [&oversized_name],
        )
        .unwrap();
    drop(sqlite);

    let error = tiamud_persistence_postgres::copy_sqlite_to_postgres(&sqlite_path, &mut postgres)
        .unwrap_err();
    assert!(matches!(
        error,
        tiamud_persistence_postgres::TransferError::Postgres(_)
    ));
    assert!(
        postgres
            .query_one("SELECT to_regclass('tiamud_schema') IS NULL", &[])
            .unwrap()
            .get::<_, bool>(0),
        "schema DDL must roll back with the failed copy"
    );

    postgres.batch_execute("SET search_path TO public").unwrap();
    postgres
        .batch_execute(&format!("DROP SCHEMA {schema} CASCADE"))
        .unwrap();
}
