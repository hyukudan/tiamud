use std::{collections::BTreeMap, env, fs, process, time::SystemTime};

use postgres::{Client, NoTls};
use rusqlite::{Connection, params};
use sha2::{Digest, Sha256};
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
             INSERT INTO world_area_generation
                 (singleton, world_id, active_catalog_sha256,
                  previous_catalog_sha256, activation_sequence, updated_at)
             VALUES (1, 'Mundo de Ámbar',
                     'aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa',
                     'bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb',
                     7, 975);
             INSERT INTO character_adventure (character_id, state_json)
             VALUES (10, '{\"coins\":40}');
             INSERT INTO world_adventure (singleton, state_json)
             VALUES (1, '{\"weather\":\"rain\"}');
             INSERT INTO moderation_audit
                 (sequence, actor_account_id, target_account_id, action,
                  details_json, created_at)
             VALUES (3, 1, 2, 'role.set', '{}', 1000);
             INSERT INTO staff_profiles
                 (account_id, profile_key, lore_title, assigned_by_account_id,
                  created_at, updated_at)
             VALUES (1, 'administrator', 'Keeper of Lanterns', 1, 900, 950);
             INSERT INTO staff_capability_assignments
                 (account_id, capability, assigned_by_account_id, created_at)
             VALUES (1, 'reports.resolve', 1, 900);
             INSERT INTO social_reports
                 (id, reporter_character_id, target_character_id, report_kind, reason, created_at,
                  status, resolution, resolution_reason, resolved_by_account_id, resolved_at)
             VALUES (20, 11, 10, 'mail', 'Repeated harassment', 910, 'resolved', 'action_taken',
                     'Reviewed and moderated', 1, 960),
                    (21, 11, 10, 'board', 'Public harassment', 920, 'open', NULL,
                     NULL, NULL, NULL);
             INSERT INTO social_mail_usage
                 (account_id, hour_started_at, hour_count,
                  day_started_at, day_count, updated_at)
             VALUES (1, 0, 1, 0, 1, 900);
             INSERT INTO social_mail_messages
                 (id, public_id, sender_character_id, sender_name,
                  recipient_character_id, subject, body, created_at,
                  expires_at, read_at, report_id)
             VALUES (30, 'm0123456789abcdef0123456789abcdef', 10, 'Aster',
                     11, 'Lanterns', 'Meet beneath the arch.', 900,
                     2592900, 905, 20);
             INSERT INTO social_mail_report_evidence
                 (report_id, sender_name, subject, body, sent_at,
                  created_at, expires_at)
             VALUES (20, 'Aster', 'Lanterns', 'Meet beneath the arch.',
                     900, 910, 7776910);
             INSERT INTO social_board_usage
                 (account_id, hour_started_at, hour_count,
                  day_started_at, day_count, updated_at)
             VALUES (1, 900, 1, 900, 1, 900);
             INSERT INTO social_board_posts
                 (id, public_id, board_key, author_character_id, author_name,
                  subject, body, created_at, expires_at, report_id)
             VALUES (31, 'b0123456789abcdef0123456789abcdef', 'general',
                     10, 'Aster', 'Lanterns', 'A public board entry.',
                     900, 7776900, 21);
             INSERT INTO social_board_report_evidence
                 (report_id, board_key, post_public_id, author_name,
                  subject, body, posted_at, created_at, expires_at)
             VALUES (21, 'general', 'b0123456789abcdef0123456789abcdef',
                     'Aster', 'Lanterns', 'A public board entry.',
                     900, 920, 7776920);
             INSERT INTO staff_audit
                 (sequence, actor_account_id, target_account_id, capability, action,
                  reason, details_json, created_at)
             VALUES (5, 1, 2, 'reports.resolve', 'report.resolve',
                     'Reviewed report', '{\"reportId\":20}', 960);",
        )
        .unwrap();
    let runtime = tiamud_core::World::demo().runtime_snapshot();
    let runtime_json = serde_json::to_string(&runtime).unwrap();
    tiamud_persistence_sqlite::runtime_sections::write_runtime_document_sqlite(
        &sqlite,
        i64::from(runtime.snapshot_version),
        &runtime_json,
        Some(7),
    )
    .unwrap();
    let source_runtime =
        tiamud_persistence_sqlite::runtime_sections::load_stored_runtime_sqlite(&sqlite)
            .unwrap()
            .unwrap();
    let checkpoint = tiamud_core::AreaDynamicCheckpoint {
        checkpoint_version: tiamud_core::AREA_DYNAMIC_CHECKPOINT_VERSION,
        generation_sha256: "a".repeat(64),
        areas: vec![tiamud_core::AreaDynamicCheckpointIdentity {
            area_id: "demo:archive".to_owned(),
            revision: 1,
            bundle_sha256: "d".repeat(64),
            residency_epoch: 7,
        }],
        captured_world_revision: 1,
        captured_logical_tick: 42,
        captured_rng_state: 1,
        captured_next_item_instance_sequence: 1,
        captured_next_loot_drop_id: tiamud_core::LootDropId(1),
        captured_next_reward_open_id: tiamud_core::RewardOpenId(1),
        npcs: BTreeMap::new(),
        item_instances: BTreeMap::new(),
        item_provenance: BTreeMap::new(),
        ground_loot: BTreeMap::new(),
    };
    tiamud_core::validate_area_dynamic_checkpoint(&checkpoint).unwrap();
    let checkpoint_json = serde_json::to_string(&checkpoint).unwrap();
    let checkpoint_sha256 = hex::encode(Sha256::digest(checkpoint_json.as_bytes()));
    sqlite
        .execute(
            "INSERT INTO world_area_checkpoints
                 (generation_sha256, unit_id, residency_epoch,
                  checkpoint_version, checkpoint_sha256, captured_logical_tick,
                  checkpoint_json, created_at, updated_at)
             VALUES (?1, ?2, 7, 1, ?3, 42, ?4, 980, 981)",
            params![
                "a".repeat(64),
                "b".repeat(64),
                checkpoint_sha256,
                checkpoint_json
            ],
        )
        .unwrap();
    let story_state = r#"{"format_version":2,"pack_id":"cripta-del-nigromante","node":"inicio"}"#;
    let story_state_sha256 = hex::encode(Sha256::digest(story_state.as_bytes()));
    sqlite
        .execute(
            "INSERT INTO story_saves
                 (account_id, story_id, slot, pack_id, pack_sha256,
                  state_format_version, state_json, state_sha256,
                  created_at, updated_at)
             VALUES (2, 'cripta-del-nigromante', 0, 'cripta-del-nigromante',
                     ?1, 2, ?2, ?3, 990, 995)",
            params!["c".repeat(64), story_state, story_state_sha256],
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
    // 25 rows plus the ten runtime section rows of schema v17.
    assert_eq!(report.total_rows, 35);
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
        tiamud_persistence_postgres::MigrationReport { from: 17, to: 17 }
    );
    let story_row = postgres
        .query_one(
            "SELECT account_id, story_id, slot, pack_id, pack_sha256,
                    state_format_version, state_json, state_sha256,
                    created_at, updated_at
             FROM story_saves",
            &[],
        )
        .unwrap();
    assert_eq!(story_row.get::<_, i64>(0), 2);
    assert_eq!(story_row.get::<_, String>(1), "cripta-del-nigromante");
    assert_eq!(story_row.get::<_, i16>(2), 0);
    assert_eq!(story_row.get::<_, String>(3), "cripta-del-nigromante");
    assert_eq!(story_row.get::<_, String>(4), "c".repeat(64));
    assert_eq!(story_row.get::<_, i32>(5), 2);
    assert_eq!(
        story_row.get::<_, String>(6),
        story_state,
        "offline transfer must preserve story save bytes exactly"
    );
    assert_eq!(story_row.get::<_, String>(7), story_state_sha256);
    assert_eq!(story_row.get::<_, i64>(8), 990);
    assert_eq!(story_row.get::<_, i64>(9), 995);
    assert!(
        postgres
            .execute("UPDATE story_saves SET slot = 10", &[])
            .is_err(),
        "the PostgreSQL slot bound must match SQLite"
    );
    assert!(
        postgres
            .execute(
                "UPDATE story_saves SET state_json = repeat('x', 65537)",
                &[],
            )
            .is_err(),
        "the PostgreSQL state byte bound must match SQLite"
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
    let mut target_runtime =
        tiamud_persistence_postgres::runtime_sections::load_stored_runtime(&mut postgres, false)
            .unwrap()
            .unwrap();
    target_runtime.sections.sort_by(|a, b| a.name.cmp(&b.name));
    assert_eq!(
        target_runtime, source_runtime,
        "offline transfer must preserve the verified runtime rows byte-for-byte"
    );
    assert_eq!(assembled_runtime(&mut postgres), json_value(&runtime_json));
    // A genuine v15 database has no story_saves table and still keeps the
    // single-document runtime; v15 -> v16 adds story_saves empty and v16 ->
    // v17 sections the runtime, without touching any other authority.
    downgrade_runtime_to_v16(&mut postgres, &runtime_json, Some(7));
    postgres
        .batch_execute(
            "DROP TABLE story_saves;
             UPDATE tiamud_schema SET version = 15 WHERE singleton = 1",
        )
        .unwrap();
    assert_eq!(
        tiamud_persistence_postgres::migrate(&mut postgres).unwrap(),
        tiamud_persistence_postgres::MigrationReport { from: 15, to: 17 }
    );
    assert_eq!(
        assembled_runtime(&mut postgres),
        json_value(&runtime_json),
        "v15 to v17 must preserve the runtime payload"
    );
    assert_eq!(
        postgres
            .query_one("SELECT COUNT(*)::BIGINT FROM story_saves", &[])
            .unwrap()
            .get::<_, i64>(0),
        0
    );
    assert_eq!(
        postgres
            .query_one("SELECT COUNT(*)::BIGINT FROM world_area_checkpoints", &[])
            .unwrap()
            .get::<_, i64>(0),
        1,
        "v15 to v17 must not touch area checkpoints"
    );
    assert_eq!(
        postgres
            .query_one(
                "SELECT world_id, active_catalog_sha256,
                        previous_catalog_sha256, activation_sequence, updated_at
                 FROM world_area_generation WHERE singleton = 1",
                &[],
            )
            .unwrap()
            .get::<_, String>(0),
        "Mundo de Ámbar"
    );
    assert_eq!(
        postgres
            .query_one(
                "SELECT activation_sequence FROM world_area_generation WHERE singleton = 1",
                &[],
            )
            .unwrap()
            .get::<_, i64>(0),
        7
    );
    let checkpoint_row = postgres
        .query_one(
            "SELECT generation_sha256, unit_id, residency_epoch,
                    checkpoint_version, checkpoint_sha256,
                    captured_logical_tick, checkpoint_json, created_at, updated_at
             FROM world_area_checkpoints",
            &[],
        )
        .unwrap();
    assert_eq!(checkpoint_row.get::<_, String>(0), "a".repeat(64));
    assert_eq!(checkpoint_row.get::<_, String>(1), "b".repeat(64));
    assert_eq!(checkpoint_row.get::<_, i64>(2), 7);
    assert_eq!(checkpoint_row.get::<_, i64>(3), 1);
    assert_eq!(checkpoint_row.get::<_, String>(4), checkpoint_sha256);
    assert_eq!(checkpoint_row.get::<_, i64>(5), 42);
    assert_eq!(checkpoint_row.get::<_, String>(6), checkpoint_json);
    assert_eq!(checkpoint_row.get::<_, i64>(7), 980);
    assert_eq!(checkpoint_row.get::<_, i64>(8), 981);
    assert_eq!(
        postgres
            .query_one(
                "SELECT profile_key, lore_title FROM staff_profiles WHERE account_id = 1",
                &[],
            )
            .unwrap()
            .get::<_, String>(0),
        "administrator"
    );
    assert_eq!(
        postgres
            .query_one(
                "SELECT resolution_reason FROM social_reports WHERE id = 20",
                &[],
            )
            .unwrap()
            .get::<_, String>(0),
        "Reviewed and moderated"
    );
    assert_eq!(
        postgres
            .query_one(
                "SELECT messages.sender_name, messages.body, reports.report_kind
                 FROM social_mail_messages AS messages
                 JOIN social_reports AS reports ON reports.id = messages.report_id
                 WHERE messages.id = 30",
                &[],
            )
            .unwrap()
            .get::<_, String>(1),
        "Meet beneath the arch."
    );
    assert_eq!(
        postgres
            .query_one("SELECT report_kind FROM social_reports WHERE id = 20", &[])
            .unwrap()
            .get::<_, String>(0),
        "mail"
    );
    assert_eq!(
        postgres
            .query_one(
                "SELECT posts.body, evidence.body, reports.report_kind
                 FROM social_board_posts AS posts
                 JOIN social_reports AS reports ON reports.id = posts.report_id
                 JOIN social_board_report_evidence AS evidence
                   ON evidence.report_id = reports.id
                 WHERE posts.id = 31",
                &[],
            )
            .unwrap()
            .get::<_, String>(1),
        "A public board entry."
    );
    assert!(
        postgres
            .execute(
                "UPDATE staff_audit SET reason = 'rewritten' WHERE sequence = 5",
                &[],
            )
            .is_err(),
        "staff audit must reject mutation"
    );

    // v16 -> v17 splits the single legacy document into verified sections.
    downgrade_runtime_to_v16(&mut postgres, &runtime_json, Some(7));
    postgres
        .batch_execute("UPDATE tiamud_schema SET version = 16 WHERE singleton = 1")
        .unwrap();
    assert_eq!(
        tiamud_persistence_postgres::migrate(&mut postgres).unwrap(),
        tiamud_persistence_postgres::MigrationReport { from: 16, to: 17 }
    );
    assert_eq!(assembled_runtime(&mut postgres), json_value(&runtime_json));
    assert_eq!(
        postgres
            .query_one("SELECT COUNT(*)::BIGINT FROM world_runtime_sections", &[])
            .unwrap()
            .get::<_, i64>(0),
        10
    );

    downgrade_runtime_to_v16(&mut postgres, &runtime_json, Some(7));
    postgres
        .batch_execute(
            "DROP TABLE story_saves;
             DROP TABLE world_area_checkpoints;
             DROP TABLE world_area_generation;
             UPDATE tiamud_schema SET version = 13 WHERE singleton = 1",
        )
        .unwrap();
    assert_eq!(
        tiamud_persistence_postgres::migrate(&mut postgres).unwrap(),
        tiamud_persistence_postgres::MigrationReport { from: 13, to: 17 }
    );
    assert_eq!(
        assembled_runtime(&mut postgres),
        json_value(&runtime_json),
        "v13 to v17 must not change the runtime snapshot"
    );
    assert_eq!(
        postgres
            .query_one("SELECT COUNT(*)::BIGINT FROM world_area_generation", &[])
            .unwrap()
            .get::<_, i64>(0),
        0,
        "v13 starts without a selected durable generation"
    );
    downgrade_runtime_to_v16(&mut postgres, &runtime_json, Some(7));
    postgres
        .batch_execute(
            "DROP TABLE story_saves;
             DROP TABLE world_area_checkpoints;
             DROP TABLE social_board_report_evidence;
             DROP TABLE world_area_generation;
             DROP TABLE social_board_posts;
             DROP TABLE social_board_usage;
             DROP TABLE social_mail_report_evidence;
             DROP TABLE social_mail_messages;
             DROP TABLE social_mail_usage;
             DROP INDEX social_reports_mail_open_idx;
             ALTER TABLE social_reports DROP COLUMN report_kind;
             ALTER TABLE world_runtime
                 DROP CONSTRAINT world_runtime_snapshot_json_check,
                 ADD CONSTRAINT world_runtime_snapshot_json_check
                     CHECK (octet_length(snapshot_json) <= 1048576);
             UPDATE tiamud_schema SET version = 10 WHERE singleton = 1",
        )
        .unwrap();
    assert_eq!(
        tiamud_persistence_postgres::migrate(&mut postgres).unwrap(),
        tiamud_persistence_postgres::MigrationReport { from: 10, to: 17 }
    );
    assert_eq!(
        assembled_runtime(&mut postgres),
        json_value(&runtime_json),
        "v10 to v17 must preserve the runtime payload"
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
    downgrade_runtime_to_v16(&mut postgres, &runtime_json, Some(7));
    postgres
        .batch_execute(
            "DROP TABLE story_saves;
             DROP TABLE world_area_checkpoints;
             DROP TABLE character_adventure;
             DROP TABLE world_adventure;
             DROP TABLE world_area_generation;
             DROP TABLE social_channel_subscriptions;
             DROP TABLE social_board_report_evidence;
             DROP TABLE social_board_posts;
             DROP TABLE social_board_usage;
             DROP TABLE social_mail_report_evidence;
             DROP TABLE social_mail_messages;
             DROP TABLE social_mail_usage;
             DROP INDEX social_reports_mail_open_idx;
             ALTER TABLE social_reports DROP COLUMN report_kind;
             DROP TABLE staff_audit;
             DROP TABLE staff_capability_assignments;
             DROP TABLE staff_profiles;
             DROP FUNCTION tiamud_reject_staff_audit_mutation();
             UPDATE social_reports
             SET status = 'open', resolution = NULL, resolution_reason = NULL,
                 resolved_by_account_id = NULL, resolved_at = NULL;
             ALTER TABLE social_reports
                 DROP CONSTRAINT social_reports_status_check,
                 DROP COLUMN resolution CASCADE,
                 DROP COLUMN resolution_reason CASCADE,
                 DROP COLUMN resolved_by_account_id CASCADE,
                 DROP COLUMN resolved_at CASCADE,
                 ADD CONSTRAINT social_reports_status_check
                     CHECK (status IN ('open', 'reviewing', 'closed'));
             ALTER TABLE world_runtime DROP COLUMN last_event_sequence;
             UPDATE tiamud_schema SET version = 6 WHERE singleton = 1",
        )
        .unwrap();
    assert_eq!(
        tiamud_persistence_postgres::migrate(&mut postgres).unwrap(),
        tiamud_persistence_postgres::MigrationReport { from: 6, to: 17 }
    );
    assert!(
        postgres
            .query_one("SELECT to_regclass('character_adventure') IS NOT NULL", &[])
            .unwrap()
            .get::<_, bool>(0)
    );
    assert!(
        postgres
            .query_one("SELECT to_regclass('staff_profiles') IS NOT NULL", &[])
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

    postgres
        .execute(
            "UPDATE tiamud_schema SET version = 18 WHERE singleton = 1",
            &[],
        )
        .unwrap();
    assert!(matches!(
        tiamud_persistence_postgres::migrate(&mut postgres),
        Err(tiamud_persistence_postgres::SchemaError::UnsupportedVersion { found: 18 })
    ));
    assert_eq!(
        postgres
            .query_one("SELECT version FROM tiamud_schema WHERE singleton = 1", &[])
            .unwrap()
            .get::<_, i32>(0),
        18,
        "future metadata must remain untouched after rejection"
    );
    postgres
        .execute(
            "UPDATE tiamud_schema SET version = 17 WHERE singleton = 1",
            &[],
        )
        .unwrap();

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

/// Restore the exact v16 single-document `world_runtime` table holding `json`.
fn downgrade_runtime_to_v16(postgres: &mut Client, json: &str, last_event_sequence: Option<i64>) {
    postgres
        .batch_execute(
            "DROP TABLE IF EXISTS world_runtime_sections;
             DROP TABLE world_runtime;
             CREATE TABLE world_runtime (
                 singleton BIGINT PRIMARY KEY CHECK (singleton = 1),
                 snapshot_version BIGINT NOT NULL,
                 snapshot_json TEXT NOT NULL
                     CONSTRAINT world_runtime_snapshot_json_check
                     CHECK (octet_length(snapshot_json) <= 2097152),
                 last_event_sequence BIGINT
                     CHECK (last_event_sequence IS NULL OR last_event_sequence >= 0),
                 updated_at BIGINT NOT NULL
                     DEFAULT (floor(extract(epoch FROM now()))::BIGINT)
             );",
        )
        .unwrap();
    let version = i64::from(tiamud_core::WORLD_RUNTIME_SNAPSHOT_VERSION);
    postgres
        .execute(
            "INSERT INTO world_runtime
                 (singleton, snapshot_version, snapshot_json, last_event_sequence)
             VALUES (1, $1, $2, $3)",
            &[&version, &json, &last_event_sequence],
        )
        .unwrap();
}

/// The verified assembled runtime document of the current schema.
fn assembled_runtime(postgres: &mut Client) -> serde_json::Value {
    let stored =
        tiamud_persistence_postgres::runtime_sections::load_stored_runtime(postgres, false)
            .unwrap()
            .unwrap();
    json_value(
        &tiamud_persistence_sqlite::runtime_sections::assemble_stored_runtime(&stored, usize::MAX)
            .unwrap(),
    )
}

fn json_value(json: &str) -> serde_json::Value {
    serde_json::from_str(json).unwrap()
}
