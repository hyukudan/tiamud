use super::sqlite::*;
use super::*;
use crate::auth::PasswordBackend;
use std::{
    collections::BTreeMap,
    sync::{
        Condvar, Mutex,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    },
    time::{Duration, SystemTime, UNIX_EPOCH},
};
use tiamud_core::{
    ActiveEffect, AreaDynamicCheckpointIdentity, BackgroundSelection, Direction, EffectKind,
    EquipmentSlot, ItemBinding, ItemInstanceId, ItemLocationSnapshot, PlayerAction,
    RuntimeItemInstanceSnapshotV2, RuntimeItemInstanceSnapshotV3, RuntimeItemLocation,
    RuntimeItemLocationV2, SessionId, World, WorldError, WorldRuntimeSnapshotV1,
    WorldRuntimeSnapshotV2, WorldRuntimeSnapshotV3,
};

async fn account(store: &SqliteCharacterStore, name: &str) -> AccountSnapshot {
    store.load_or_create_account(name).await.unwrap()
}

fn area_generation(
    world_id: &str,
    active: char,
    previous: Option<char>,
    activation_sequence: u64,
) -> DurableAreaGeneration {
    DurableAreaGeneration {
        world_id: world_id.into(),
        active_catalog_sha256: active.to_string().repeat(64),
        previous_catalog_sha256: previous.map(|digest| digest.to_string().repeat(64)),
        activation_sequence,
    }
}

fn area_checkpoint(runtime: &WorldRuntimeSnapshot) -> DurableAreaCheckpoint {
    let checkpoint = AreaDynamicCheckpoint {
        checkpoint_version: tiamud_core::AREA_DYNAMIC_CHECKPOINT_VERSION,
        generation_sha256: "a".repeat(64),
        areas: vec![AreaDynamicCheckpointIdentity {
            area_id: "demo:archive".into(),
            revision: 1,
            bundle_sha256: "d".repeat(64),
            residency_epoch: 2,
        }],
        captured_world_revision: runtime.revision,
        captured_logical_tick: runtime.logical_tick,
        captured_rng_state: runtime.rng_state,
        captured_next_item_instance_sequence: runtime.next_item_instance_sequence,
        captured_next_loot_drop_id: runtime.next_loot_drop_id,
        captured_next_reward_open_id: runtime.next_reward_open_id,
        npcs: BTreeMap::new(),
        item_instances: BTreeMap::new(),
        item_provenance: BTreeMap::new(),
        ground_loot: BTreeMap::new(),
    };
    tiamud_core::validate_area_dynamic_checkpoint(&checkpoint).unwrap();
    let checkpoint_json = serde_json::to_string(&checkpoint).unwrap();
    DurableAreaCheckpoint {
        generation_sha256: checkpoint.generation_sha256.clone(),
        unit_id: area_checkpoint_unit_id(&checkpoint),
        residency_epoch: checkpoint.areas[0].residency_epoch,
        checkpoint_version: checkpoint.checkpoint_version,
        checkpoint_sha256: hex::encode(Sha256::digest(checkpoint_json.as_bytes())),
        captured_logical_tick: checkpoint.captured_logical_tick,
        checkpoint_json,
    }
}

fn retired_runtime(
    mut runtime: WorldRuntimeSnapshot,
    durable: &DurableAreaCheckpoint,
) -> WorldRuntimeSnapshot {
    runtime.area_residency.insert(
        "demo:archive".into(),
        AreaRuntimeResidence::Retired {
            revision: 1,
            bundle_sha256: "d".repeat(64),
            residency_epoch: durable.residency_epoch,
            checkpoint_version: durable.checkpoint_version,
            checkpoint_sha256: durable.checkpoint_sha256.clone(),
        },
    );
    runtime
}

fn initial_resident_runtime(mut runtime: WorldRuntimeSnapshot) -> WorldRuntimeSnapshot {
    runtime.area_residency.insert(
        "demo:archive".into(),
        AreaRuntimeResidence::Resident {
            revision: 1,
            bundle_sha256: "d".repeat(64),
            residency_epoch: 1,
        },
    );
    runtime
}

fn rehydrated_runtime(mut runtime: WorldRuntimeSnapshot) -> WorldRuntimeSnapshot {
    runtime.area_residency.insert(
        "demo:archive".into(),
        AreaRuntimeResidence::Resident {
            revision: 1,
            bundle_sha256: "d".repeat(64),
            residency_epoch: 3,
        },
    );
    runtime
}

struct BlockingVerifyBackend {
    verify_started: AtomicUsize,
    released: AtomicBool,
    lock: Mutex<()>,
    wake: Condvar,
}

impl BlockingVerifyBackend {
    fn new() -> Self {
        Self {
            verify_started: AtomicUsize::new(0),
            released: AtomicBool::new(false),
            lock: Mutex::new(()),
            wake: Condvar::new(),
        }
    }

    fn release(&self) {
        self.released.store(true, Ordering::SeqCst);
        self.wake.notify_all();
    }
}

impl PasswordBackend for BlockingVerifyBackend {
    fn hash(&self, password: &str) -> Result<String, ()> {
        Ok(format!("test:{password}"))
    }

    fn verify(&self, password: &str, encoded_hash: &str) -> Result<bool, ()> {
        self.verify_started.fetch_add(1, Ordering::SeqCst);
        let mut guard = self.lock.lock().unwrap();
        while !self.released.load(Ordering::SeqCst) {
            guard = self.wake.wait(guard).unwrap();
        }
        Ok(encoded_hash == format!("test:{password}"))
    }
}

#[tokio::test]
async fn shutdown_is_a_fifo_barrier_and_closes_the_owner_thread() {
    let store = SqliteCharacterStore::open(":memory:").await.unwrap();
    store
        .register_account("Shutdown Owner", "correct horse battery staple")
        .await
        .unwrap();
    store.shutdown().await.unwrap();
    assert!(matches!(
        store.health_check().await,
        Err(StoreError::WorkerStopped)
    ));
}

#[tokio::test]
async fn registration_and_login_are_case_insensitive() {
    let store = SqliteCharacterStore::open(":memory:").await.unwrap();
    let registered = store
        .register_account("  Adventurer  ", "correct horse battery staple")
        .await
        .unwrap();
    let logged_in = store
        .login_account("ADVENTURER", "correct horse battery staple")
        .await
        .unwrap();

    assert_eq!(logged_in.account, registered.account);
    assert_ne!(logged_in.token, registered.token);
    assert_eq!(
        store
            .resolve_session(logged_in.token.as_str())
            .await
            .unwrap(),
        registered.account
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn blocked_password_verification_does_not_block_database_health_or_commits() {
    let backend = Arc::new(BlockingVerifyBackend::new());
    let passwords = PasswordService::with_backend(1, backend.clone());
    let store = SqliteCharacterStore::open_with_password_service(":memory:", passwords)
        .await
        .unwrap();
    store
        .register_account("Adventurer", "correct horse battery staple")
        .await
        .unwrap();

    let login_store = store.clone();
    let login = tokio::spawn(async move {
        login_store
            .login_account("adventurer", "correct horse battery staple")
            .await
    });
    timeout(Duration::from_secs(1), async {
        while backend.verify_started.load(Ordering::SeqCst) == 0 {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();

    timeout(Duration::from_secs(1), store.health_check())
        .await
        .unwrap()
        .unwrap();
    timeout(
        Duration::from_secs(1),
        store.commit_action(Vec::new(), World::demo().runtime_snapshot(), Vec::new()),
    )
    .await
    .unwrap()
    .unwrap();

    backend.release();
    login.await.unwrap().unwrap();
}

#[test]
fn login_finalization_revalidates_account_state_after_password_work() {
    let mut connection = Connection::open_in_memory().unwrap();
    initialise(&mut connection).unwrap();
    register_account(
        &mut connection,
        "Adventurer",
        "correct horse battery staple",
        1_000,
    )
    .unwrap();
    let prepared = prepare_login_account(&connection, "adventurer")
        .unwrap()
        .unwrap();
    connection
        .execute(
            "UPDATE accounts SET auth_state = 'guest' WHERE id = ?1",
            [prepared.account_id.0],
        )
        .unwrap();

    assert!(matches!(
        finish_login_account(&mut connection, &prepared, 1_001),
        Err(StoreError::InvalidCredentials)
    ));
}

#[tokio::test]
async fn incorrect_password_is_rejected() {
    let store = SqliteCharacterStore::open(":memory:").await.unwrap();
    store
        .register_account("Adventurer", "correct horse battery staple")
        .await
        .unwrap();

    assert!(matches!(
        store
            .login_account("adventurer", "this password is incorrect")
            .await,
        Err(StoreError::InvalidCredentials)
    ));
    assert!(matches!(
        store.load_or_create_account("ADVENTURER").await,
        Err(StoreError::AccountAlreadyExists)
    ));
}

#[test]
fn database_contains_only_argon2_password_and_sha256_token_material() {
    let mut connection = Connection::open_in_memory().unwrap();
    initialise(&mut connection).unwrap();
    let password = "correct horse battery staple";
    let session = register_account(&mut connection, "Secure", password, 1_000).unwrap();

    let encoded_hash: String = connection
        .query_row(
            "SELECT password_hash FROM accounts WHERE id = ?1",
            [session.account.id.0],
            |row| row.get(0),
        )
        .unwrap();
    let stored_token_hash: Vec<u8> = connection
        .query_row("SELECT token_hash FROM auth_sessions", [], |row| row.get(0))
        .unwrap();

    assert!(encoded_hash.starts_with("$argon2id$v=19$"));
    assert!(!encoded_hash.contains(password));
    assert_eq!(stored_token_hash.len(), 32);
    assert_ne!(stored_token_hash, session.token.as_str().as_bytes());
    assert_eq!(stored_token_hash, session.token.digest());
}

#[test]
fn expired_and_revoked_tokens_are_rejected() {
    let mut connection = Connection::open_in_memory().unwrap();
    initialise(&mut connection).unwrap();
    let expired = register_account(
        &mut connection,
        "Expiring",
        "correct horse battery staple",
        1_000,
    )
    .unwrap();

    assert!(matches!(
        resolve_session(&connection, expired.token.as_str(), expired.expires_at),
        Err(StoreError::SessionExpired)
    ));

    let active = login_account(
        &mut connection,
        "expiring",
        "correct horse battery staple",
        2_000,
    )
    .unwrap();
    revoke_session(&connection, active.token.as_str(), 2_001).unwrap();
    assert!(matches!(
        resolve_session(&connection, active.token.as_str(), 2_002),
        Err(StoreError::SessionRevoked)
    ));
}

#[test]
fn authentication_inputs_and_active_sessions_are_bounded() {
    let mut connection = Connection::open_in_memory().unwrap();
    initialise(&mut connection).unwrap();
    assert!(matches!(
        register_account(&mut connection, "Short", "tiny", 1_000),
        Err(StoreError::InvalidPassword)
    ));

    let first = register_account(
        &mut connection,
        "Bounded",
        "correct horse battery staple",
        2_000,
    )
    .unwrap();
    for offset in 1..MAX_ACTIVE_SESSIONS + 2 {
        issue_session(&connection, first.account.clone(), 2_000 + offset).unwrap();
    }
    let count: i64 = connection
        .query_row(
            "SELECT COUNT(*) FROM auth_sessions WHERE account_id = ?1",
            [first.account.id.0],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(count, MAX_ACTIVE_SESSIONS);
    assert!(matches!(
        resolve_session(&connection, first.token.as_str(), 2_100),
        Err(StoreError::InvalidSession)
    ));
}

#[test]
fn schema_v8_adventure_mirrors_are_written_in_the_same_transaction() {
    let mut connection = Connection::open_in_memory().unwrap();
    initialise(&mut connection).unwrap();
    let owner = load_or_create_account(&connection, "Adventure Owner").unwrap();
    let character =
        load_or_create_character(&connection, owner.id, "Market Hero", "demo:lantern-square")
            .unwrap();
    let gameplay = CharacterGameplaySnapshot {
        coins: 28,
        commodities: std::collections::BTreeMap::from([("tonic".into(), 1)]),
        ..CharacterGameplaySnapshot::default()
    };
    let mut runtime = World::demo().runtime_snapshot();
    runtime.adventure.shop_stock.insert("tonic".into(), 19);

    commit_action(
        &mut connection,
        &[DurableCharacterSnapshot {
            character: character.clone(),
            gameplay,
        }],
        &runtime,
        &[],
    )
    .unwrap();

    let character_json: String = connection
        .query_row(
            "SELECT state_json FROM character_adventure WHERE character_id = ?1",
            [character.id.0],
            |row| row.get(0),
        )
        .unwrap();
    let world_json: String = connection
        .query_row(
            "SELECT state_json FROM world_adventure WHERE singleton = 1",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert!(character_json.contains("\"coins\":28"));
    assert!(character_json.contains("\"tonic\":1"));
    assert!(world_json.contains("\"tonic\":19"));
}

fn test_domain_event() -> DomainEvent {
    DomainEvent::CharacterMoved {
        actor_id: CharacterId(1),
        actor_name: "Aria".into(),
        from_room_id: "room:a".into(),
        to_room_id: "room:b".into(),
        direction: Direction::North,
    }
}

#[test]
fn startup_world_recovery_distinguishes_empty_and_ledger_authority() {
    let mut connection = Connection::open_in_memory().unwrap();
    initialise(&mut connection).unwrap();

    let empty = load_startup_world_recovery(&mut connection).unwrap();
    assert_eq!(empty, StartupWorldRecovery::Empty);
    assert!(!empty.requires_full_generation());

    let event = test_domain_event();
    connection
        .execute(
            "INSERT INTO domain_events (event_json) VALUES (?1)",
            [serde_json::to_string(&event).unwrap()],
        )
        .unwrap();
    let ledger = load_startup_world_recovery(&mut connection).unwrap();
    assert_eq!(ledger, StartupWorldRecovery::Ledger(vec![event]));
    assert!(ledger.requires_full_generation());
}

#[test]
fn startup_world_recovery_returns_a_valid_snapshot() {
    let mut connection = Connection::open_in_memory().unwrap();
    initialise(&mut connection).unwrap();
    let snapshot = World::demo().runtime_snapshot();
    connection
        .execute(
            "INSERT INTO world_runtime
                 (singleton, snapshot_version, snapshot_json,
                  last_event_sequence, updated_at)
             VALUES (1, ?1, ?2, 0, unixepoch())",
            params![
                snapshot.snapshot_version,
                serde_json::to_string(&snapshot).unwrap()
            ],
        )
        .unwrap();

    let recovery = load_startup_world_recovery(&mut connection).unwrap();
    assert_eq!(
        recovery,
        StartupWorldRecovery::Snapshot {
            durable_version: tiamud_core::WORLD_RUNTIME_SNAPSHOT_VERSION,
            snapshot,
        }
    );
    assert!(recovery.requires_full_generation());
}

#[test]
fn startup_world_recovery_preserves_legacy_durable_snapshot_version() {
    let mut connection = Connection::open_in_memory().unwrap();
    initialise(&mut connection).unwrap();
    let mut legacy = serde_json::to_value(World::demo().runtime_snapshot()).unwrap();
    let object = legacy.as_object_mut().unwrap();
    object.insert("snapshotVersion".into(), serde_json::json!(7));
    object.remove("areaResidency");
    connection
        .execute(
            "INSERT INTO world_runtime
                 (singleton, snapshot_version, snapshot_json,
                  last_event_sequence, updated_at)
             VALUES (1, 7, ?1, 0, unixepoch())",
            [serde_json::to_string(&legacy).unwrap()],
        )
        .unwrap();

    let StartupWorldRecovery::Snapshot {
        durable_version,
        snapshot,
    } = load_startup_world_recovery(&mut connection).unwrap()
    else {
        panic!("expected durable snapshot authority");
    };
    assert_eq!(durable_version, 7);
    assert!(snapshot.area_residency.is_empty());
    assert_eq!(
        load_runtime_snapshot(&connection).unwrap(),
        Some(snapshot),
        "the public loader remains normalized"
    );
}

#[test]
fn startup_world_recovery_gives_snapshot_precedence_over_the_ledger() {
    let mut connection = Connection::open_in_memory().unwrap();
    initialise(&mut connection).unwrap();
    let snapshot = World::demo().runtime_snapshot();
    connection
        .execute(
            "INSERT INTO world_runtime
                 (singleton, snapshot_version, snapshot_json,
                  last_event_sequence, updated_at)
             VALUES (1, ?1, ?2, 0, unixepoch())",
            params![
                snapshot.snapshot_version,
                serde_json::to_string(&snapshot).unwrap()
            ],
        )
        .unwrap();
    // A retained diagnostic suffix is not consulted when a valid runtime
    // snapshot exists. Make it deliberately undecodable to prove there is
    // no hidden ledger fallback or eager parse.
    connection
        .execute(
            "INSERT INTO domain_events (event_json) VALUES ('not-json')",
            [],
        )
        .unwrap();

    assert_eq!(
        load_startup_world_recovery(&mut connection).unwrap(),
        StartupWorldRecovery::Snapshot {
            durable_version: tiamud_core::WORLD_RUNTIME_SNAPSHOT_VERSION,
            snapshot,
        }
    );
}

#[test]
fn startup_world_recovery_fails_closed_on_an_invalid_snapshot() {
    let mut connection = Connection::open_in_memory().unwrap();
    initialise(&mut connection).unwrap();
    connection
        .execute(
            "INSERT INTO world_runtime
                 (singleton, snapshot_version, snapshot_json,
                  last_event_sequence, updated_at)
             VALUES (1, ?1, '{}', 0, unixepoch())",
            [tiamud_core::WORLD_RUNTIME_SNAPSHOT_VERSION],
        )
        .unwrap();
    connection
        .execute(
            "INSERT INTO domain_events (event_json) VALUES (?1)",
            [serde_json::to_string(&test_domain_event()).unwrap()],
        )
        .unwrap();

    assert!(matches!(
        load_startup_world_recovery(&mut connection),
        Err(StoreError::InvalidRuntimeSnapshot)
    ));
}

#[test]
fn startup_world_recovery_rejects_a_ledger_gap() {
    let mut connection = Connection::open_in_memory().unwrap();
    initialise(&mut connection).unwrap();
    let event_json = serde_json::to_string(&test_domain_event()).unwrap();
    connection
        .execute(
            "INSERT INTO domain_events (sequence, event_json)
             VALUES (1, ?1), (3, ?1)",
            [event_json],
        )
        .unwrap();

    assert!(matches!(
        load_startup_world_recovery(&mut connection),
        Err(StoreError::InvalidStartupLedger)
    ));
}

#[test]
fn startup_world_recovery_rejects_more_than_the_retained_event_limit() {
    let mut connection = Connection::open_in_memory().unwrap();
    initialise(&mut connection).unwrap();
    seed_domain_events(&connection, DOMAIN_EVENT_RETENTION + 1);

    assert!(matches!(
        load_startup_world_recovery(&mut connection),
        Err(StoreError::InvalidStartupLedger)
    ));
}

#[test]
fn startup_world_recovery_checks_event_utf8_bytes_before_deserializing() {
    let mut connection = Connection::open_in_memory().unwrap();
    initialise(&mut connection).unwrap();
    let oversized = "é".repeat(MAX_EVENT_JSON_BYTES / 2 + 1);
    assert!(oversized.len() > MAX_EVENT_JSON_BYTES);
    connection
        .execute(
            "INSERT INTO domain_events (event_json) VALUES (?1)",
            [oversized],
        )
        .unwrap();

    assert!(matches!(
        load_startup_world_recovery(&mut connection),
        Err(StoreError::SnapshotTooLarge {
            kind: "domain event",
            maximum: MAX_EVENT_JSON_BYTES,
        })
    ));
}

fn seed_domain_events(connection: &Connection, count: i64) {
    connection
        .execute(
            "WITH RECURSIVE counter(value) AS (
                 VALUES(1)
                 UNION ALL
                 SELECT value + 1 FROM counter WHERE value < ?1
             )
             INSERT INTO domain_events (event_json)
             SELECT '{\"type\":\"retained-test\"}' FROM counter",
            [count],
        )
        .unwrap();
}

#[test]
fn runtime_watermark_tracks_events_and_survives_an_empty_commit() {
    let mut connection = Connection::open_in_memory().unwrap();
    initialise(&mut connection).unwrap();
    let mut runtime = World::demo().runtime_snapshot();

    commit_action(&mut connection, &[], &runtime, &[test_domain_event()]).unwrap();
    assert_eq!(
        connection
            .query_row(
                "SELECT last_event_sequence FROM world_runtime WHERE singleton = 1",
                [],
                |row| row.get::<_, i64>(0),
            )
            .unwrap(),
        1
    );

    runtime.logical_tick = 1;
    commit_action(&mut connection, &[], &runtime, &[]).unwrap();
    assert_eq!(
        connection
            .query_row(
                "SELECT last_event_sequence FROM world_runtime WHERE singleton = 1",
                [],
                |row| row.get::<_, i64>(0),
            )
            .unwrap(),
        1
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
fn domain_event_compaction_retains_the_explicit_bounded_suffix() {
    let mut connection = Connection::open_in_memory().unwrap();
    initialise(&mut connection).unwrap();
    seed_domain_events(&connection, DOMAIN_EVENT_RETENTION);

    commit_action(
        &mut connection,
        &[],
        &World::demo().runtime_snapshot(),
        &[test_domain_event()],
    )
    .unwrap();

    let bounds = connection
        .query_row(
            "SELECT COUNT(*), MIN(sequence), MAX(sequence) FROM domain_events",
            [],
            |row| {
                Ok((
                    row.get::<_, i64>(0)?,
                    row.get::<_, i64>(1)?,
                    row.get::<_, i64>(2)?,
                ))
            },
        )
        .unwrap();
    assert_eq!(
        bounds,
        (DOMAIN_EVENT_RETENTION, 2, DOMAIN_EVENT_RETENTION + 1)
    );
    assert_eq!(
        connection
            .query_row(
                "SELECT last_event_sequence FROM world_runtime WHERE singleton = 1",
                [],
                |row| row.get::<_, i64>(0),
            )
            .unwrap(),
        DOMAIN_EVENT_RETENTION + 1
    );
}

#[test]
fn domain_event_compaction_stays_bounded_across_many_commits() {
    const TEST_RETENTION: i64 = 10;
    let mut connection = Connection::open_in_memory().unwrap();
    initialise(&mut connection).unwrap();
    let mut runtime = World::demo().runtime_snapshot();

    for sequence in 1..=25 {
        runtime.logical_tick = sequence;
        commit_action_with_retention(
            &mut connection,
            &[],
            &runtime,
            &[test_domain_event()],
            TEST_RETENTION,
            None,
        )
        .unwrap();
    }

    let state = connection
        .query_row(
            "SELECT
                 (SELECT COUNT(*) FROM domain_events),
                 (SELECT MIN(sequence) FROM domain_events),
                 (SELECT MAX(sequence) FROM domain_events),
                 last_event_sequence
             FROM world_runtime WHERE singleton = 1",
            [],
            |row| {
                Ok((
                    row.get::<_, i64>(0)?,
                    row.get::<_, i64>(1)?,
                    row.get::<_, i64>(2)?,
                    row.get::<_, i64>(3)?,
                ))
            },
        )
        .unwrap();
    assert_eq!(state, (TEST_RETENTION, 16, 25, 25));
}

#[test]
fn failed_compaction_rolls_back_event_snapshot_watermark_and_deletion() {
    let mut connection = Connection::open_in_memory().unwrap();
    initialise(&mut connection).unwrap();
    seed_domain_events(&connection, DOMAIN_EVENT_RETENTION);
    let original_runtime = World::demo().runtime_snapshot();
    let original_json = serde_json::to_string(&original_runtime).unwrap();
    connection
        .execute(
            "INSERT INTO world_runtime
                 (singleton, snapshot_version, snapshot_json,
                  last_event_sequence, updated_at)
             VALUES (1, ?1, ?2, ?3, unixepoch())",
            params![
                original_runtime.snapshot_version,
                original_json,
                DOMAIN_EVENT_RETENTION
            ],
        )
        .unwrap();
    connection
        .execute_batch(
            "CREATE TRIGGER reject_domain_event_compaction
             BEFORE DELETE ON domain_events
             BEGIN
                 SELECT RAISE(ABORT, 'injected compaction failure');
             END;",
        )
        .unwrap();
    let mut replacement = original_runtime;
    replacement.logical_tick = 99;

    assert!(commit_action(&mut connection, &[], &replacement, &[test_domain_event()]).is_err());
    let state = connection
        .query_row(
            "SELECT
                 (SELECT COUNT(*) FROM domain_events),
                 (SELECT MAX(sequence) FROM domain_events),
                 last_event_sequence,
                 snapshot_json
             FROM world_runtime WHERE singleton = 1",
            [],
            |row| {
                Ok((
                    row.get::<_, i64>(0)?,
                    row.get::<_, i64>(1)?,
                    row.get::<_, i64>(2)?,
                    row.get::<_, String>(3)?,
                ))
            },
        )
        .unwrap();
    assert_eq!(
        state,
        (
            DOMAIN_EVENT_RETENTION,
            DOMAIN_EVENT_RETENTION,
            DOMAIN_EVENT_RETENTION,
            original_json
        )
    );
}

#[tokio::test]
async fn sqlite_area_generation_bind_and_activation_are_restart_safe_and_cas_guarded() {
    let temporary = tempfile::TempDir::new().unwrap();
    let database_path = temporary.path().join("area-generation.sqlite3");
    let store = SqliteCharacterStore::open(&database_path).await.unwrap();
    assert_eq!(store.load_area_generation().await.unwrap(), None);

    let initial = area_generation("tiamud:lantern", 'a', None, 1);
    assert_eq!(
        store
            .bind_area_generation_if_absent(initial.clone())
            .await
            .unwrap(),
        initial
    );
    assert_eq!(
        store
            .bind_area_generation_if_absent(area_generation("tiamud:lantern", 'b', None, 1,))
            .await
            .unwrap(),
        initial
    );

    let mut activated_runtime = World::demo().runtime_snapshot();
    activated_runtime.logical_tick = 41;
    let activated = area_generation("tiamud:lantern", 'b', Some('a'), 2);
    store
        .commit_area_generation_activation(
            activated_runtime.clone(),
            initial.clone(),
            activated.clone(),
        )
        .await
        .unwrap();
    assert_eq!(
        store.load_area_generation().await.unwrap(),
        Some(activated.clone())
    );
    assert_eq!(
        store.load_startup_world_recovery().await.unwrap(),
        StartupWorldRecovery::Snapshot {
            durable_version: tiamud_core::WORLD_RUNTIME_SNAPSHOT_VERSION,
            snapshot: activated_runtime.clone(),
        }
    );

    let mut stale_runtime = activated_runtime.clone();
    stale_runtime.logical_tick = 99;
    assert!(matches!(
        store
            .commit_area_generation_activation(
                stale_runtime,
                initial.clone(),
                area_generation("tiamud:lantern", 'c', Some('a'), 2),
            )
            .await,
        Err(StoreError::AreaGenerationConflict)
    ));
    assert_eq!(
        store.load_area_generation().await.unwrap(),
        Some(activated.clone())
    );
    assert_eq!(
        store.load_startup_world_recovery().await.unwrap(),
        StartupWorldRecovery::Snapshot {
            durable_version: tiamud_core::WORLD_RUNTIME_SNAPSHOT_VERSION,
            snapshot: activated_runtime,
        }
    );
    store.shutdown().await.unwrap();

    let reopened = SqliteCharacterStore::open(&database_path).await.unwrap();
    assert_eq!(
        reopened.load_area_generation().await.unwrap(),
        Some(activated)
    );
}

#[tokio::test]
async fn sqlite_area_checkpoint_retirement_and_rehydration_are_exact_and_restart_safe() {
    let temporary = tempfile::TempDir::new().unwrap();
    let database_path = temporary.path().join("area-checkpoint.sqlite3");
    let store = SqliteCharacterStore::open(&database_path).await.unwrap();
    let generation = area_generation("tiamud:lantern", 'a', None, 1);
    store
        .bind_area_generation_if_absent(generation.clone())
        .await
        .unwrap();

    let baseline = initial_resident_runtime(World::demo().runtime_snapshot());
    let stale_checkpoint = area_checkpoint(&baseline);
    let stale_retired = retired_runtime(baseline.clone(), &stale_checkpoint);
    let mut current = baseline;
    current.logical_tick += 1;
    store
        .commit_action(Vec::new(), current.clone(), Vec::new())
        .await
        .unwrap();
    assert!(matches!(
        store
            .commit_area_retirement(stale_retired, generation.clone(), stale_checkpoint,)
            .await,
        Err(StoreError::AreaCheckpointConflict)
    ));
    let checkpoint = area_checkpoint(&current);
    let retired = retired_runtime(current, &checkpoint);
    let mut forged_unit = checkpoint.clone();
    forged_unit.unit_id = "c".repeat(64);
    assert!(matches!(
        store
            .commit_area_retirement(retired.clone(), generation.clone(), forged_unit,)
            .await,
        Err(StoreError::InvalidAreaCheckpoint)
    ));
    assert!(store.load_area_checkpoints().await.unwrap().is_empty());
    store
        .commit_area_retirement(retired.clone(), generation.clone(), checkpoint.clone())
        .await
        .unwrap();
    assert_eq!(
        store.load_area_checkpoints().await.unwrap(),
        vec![checkpoint.clone()]
    );
    assert_eq!(
        store.load_runtime_snapshot().await.unwrap(),
        Some(retired.clone())
    );
    assert!(matches!(
        store
            .commit_area_retirement(retired.clone(), generation.clone(), checkpoint.clone())
            .await,
        Err(StoreError::AreaCheckpointConflict)
    ));

    store.shutdown().await.unwrap();
    let reopened = SqliteCharacterStore::open(&database_path).await.unwrap();
    assert_eq!(
        reopened.load_area_checkpoints().await.unwrap(),
        vec![checkpoint.clone()]
    );
    let stale_resident = rehydrated_runtime(retired.clone());
    let mut current_retired = retired;
    current_retired.logical_tick += 1;
    reopened
        .commit_action(Vec::new(), current_retired.clone(), Vec::new())
        .await
        .unwrap();
    assert!(matches!(
        reopened
            .commit_area_rehydration(stale_resident, generation.clone(), checkpoint.clone(),)
            .await,
        Err(StoreError::AreaCheckpointConflict)
    ));
    assert_eq!(
        reopened.load_area_checkpoints().await.unwrap(),
        vec![checkpoint.clone()]
    );
    let resident = rehydrated_runtime(current_retired);
    reopened
        .commit_area_rehydration(resident.clone(), generation.clone(), checkpoint.clone())
        .await
        .unwrap();
    assert!(reopened.load_area_checkpoints().await.unwrap().is_empty());
    assert_eq!(
        reopened.load_runtime_snapshot().await.unwrap(),
        Some(resident.clone())
    );
    assert!(matches!(
        reopened
            .commit_area_rehydration(resident, generation, checkpoint)
            .await,
        Err(StoreError::AreaCheckpointConflict)
    ));
}

#[test]
fn sqlite_area_checkpoint_snapshot_failures_roll_back_insert_and_delete() {
    let mut connection = Connection::open_in_memory().unwrap();
    initialise(&mut connection).unwrap();
    let generation = area_generation("tiamud:lantern", 'a', None, 1);
    bind_area_generation_if_absent(&mut connection, &generation).unwrap();
    let baseline = initial_resident_runtime(World::demo().runtime_snapshot());
    commit_action(&mut connection, &[], &baseline, &[]).unwrap();
    let checkpoint = area_checkpoint(&baseline);
    let retired = retired_runtime(baseline.clone(), &checkpoint);

    connection
        .execute_batch(
            "CREATE TRIGGER reject_retirement_snapshot
             BEFORE UPDATE ON world_runtime
             BEGIN
                 SELECT RAISE(ABORT, 'injected retirement snapshot failure');
             END;",
        )
        .unwrap();
    assert!(commit_area_retirement(&mut connection, &retired, &generation, &checkpoint).is_err());
    assert!(load_area_checkpoints(&connection).unwrap().is_empty());
    assert_eq!(load_runtime_snapshot(&connection).unwrap(), Some(baseline));

    connection
        .execute_batch("DROP TRIGGER reject_retirement_snapshot")
        .unwrap();
    commit_area_retirement(&mut connection, &retired, &generation, &checkpoint).unwrap();
    let resident = rehydrated_runtime(retired.clone());
    connection
        .execute_batch(
            "CREATE TRIGGER reject_rehydration_snapshot
             BEFORE UPDATE ON world_runtime
             BEGIN
                 SELECT RAISE(ABORT, 'injected rehydration snapshot failure');
             END;",
        )
        .unwrap();
    assert!(commit_area_rehydration(&mut connection, &resident, &generation, &checkpoint).is_err());
    assert_eq!(
        load_area_checkpoints(&connection).unwrap(),
        vec![checkpoint]
    );
    assert_eq!(load_runtime_snapshot(&connection).unwrap(), Some(retired));
}

#[test]
fn sqlite_area_generation_activation_rolls_back_pointer_with_snapshot_failure() {
    let mut connection = Connection::open_in_memory().unwrap();
    initialise(&mut connection).unwrap();
    let initial = area_generation("Mundo de Ámbar", 'a', None, 1);
    assert_eq!(
        bind_area_generation_if_absent(&mut connection, &initial).unwrap(),
        initial
    );
    let baseline = World::demo().runtime_snapshot();
    commit_action(&mut connection, &[], &baseline, &[]).unwrap();
    connection
        .execute_batch(
            "CREATE TRIGGER reject_generation_snapshot
             BEFORE UPDATE ON world_runtime
             BEGIN
                 SELECT RAISE(ABORT, 'injected snapshot failure');
             END;",
        )
        .unwrap();
    let mut replacement = baseline.clone();
    replacement.logical_tick = 73;

    assert!(
        commit_area_generation_activation(
            &mut connection,
            &replacement,
            &initial,
            &area_generation("Mundo de Ámbar", 'b', Some('a'), 2),
        )
        .is_err()
    );
    assert_eq!(load_area_generation(&connection).unwrap(), Some(initial));
    assert_eq!(load_runtime_snapshot(&connection).unwrap(), Some(baseline));
}

#[test]
fn area_generation_validation_matches_catalog_identity_and_closed_digest_rules() {
    assert!(validate_area_generation(&area_generation("Mundo de Ámbar", 'a', None, 1)).is_ok());
    for invalid in [
        DurableAreaGeneration {
            world_id: " surrounding space ".into(),
            active_catalog_sha256: "a".repeat(64),
            previous_catalog_sha256: None,
            activation_sequence: 1,
        },
        DurableAreaGeneration {
            world_id: "demo:\u{202e}world".into(),
            active_catalog_sha256: "a".repeat(64),
            previous_catalog_sha256: None,
            activation_sequence: 1,
        },
        DurableAreaGeneration {
            world_id: "demo:world".into(),
            active_catalog_sha256: "A".repeat(64),
            previous_catalog_sha256: None,
            activation_sequence: 1,
        },
        area_generation("demo:world", 'a', Some('a'), 2),
        area_generation("demo:world", 'a', None, 0),
    ] {
        assert!(matches!(
            validate_area_generation(&invalid),
            Err(StoreError::InvalidAreaGeneration)
        ));
    }
}

#[tokio::test]
async fn account_ids_are_stable_and_names_are_case_insensitive() {
    let store = SqliteCharacterStore::open(":memory:").await.unwrap();
    let created = account(&store, "  Aria  ").await;
    let loaded = account(&store, "aria").await;

    assert_eq!(loaded.id, created.id);
    assert_eq!(loaded.name, "Aria");
}

#[tokio::test]
async fn character_authority_is_visible_without_a_runtime_snapshot_or_ledger() {
    let store = SqliteCharacterStore::open(":memory:").await.unwrap();
    assert!(!store.has_durable_character_authority().await.unwrap());
    let owner = account(&store, "Legacy Owner").await;
    assert!(!store.has_durable_character_authority().await.unwrap());
    store
        .load_or_create_character(owner.id, "Legacy Hero", "demo:lantern-square")
        .await
        .unwrap();

    assert!(store.has_durable_character_authority().await.unwrap());
    assert_eq!(
        store.load_startup_world_recovery().await.unwrap(),
        StartupWorldRecovery::Empty
    );
}

#[tokio::test]
async fn character_ids_survive_saves_and_case_insensitive_loads() {
    let store = SqliteCharacterStore::open(":memory:").await.unwrap();
    let owner = account(&store, "Owner").await;
    let created = store
        .load_or_create_character(owner.id, "Aria", "demo:lantern-square")
        .await
        .unwrap();

    let loaded_by_name = store
        .load_or_create_character(owner.id, "aria", "ignored")
        .await
        .unwrap();
    assert_eq!(loaded_by_name, created);

    let mut moved = created.clone();
    moved.room_id = "demo:north-gate".into();
    moved
        .item_locations
        .insert("demo:lantern".into(), ItemLocationSnapshot::Carried);
    store.save(moved.clone()).await.unwrap();
    assert_eq!(
        store.load_character(owner.id, created.id).await.unwrap(),
        moved
    );
}

#[tokio::test]
async fn selected_initial_gameplay_is_persisted_once_without_overwriting_existing_characters() {
    let store = SqliteCharacterStore::open(":memory:").await.unwrap();
    let owner = account(&store, "Sheet Owner").await;
    let mut selected = CharacterGameplaySnapshot::default();
    selected.attributes.strength = 11;
    selected.vitals.health = 27;
    selected.vitals.max_health = 27;

    let character = store
        .load_or_create_character_with_gameplay(
            owner.id,
            "Aria",
            "demo:lantern-square",
            selected.clone(),
        )
        .await
        .unwrap();
    assert_eq!(
        store.load_character_gameplay(character.id).await.unwrap(),
        selected
    );

    let mut replacement = CharacterGameplaySnapshot::default();
    replacement.attributes.strength = 99;
    let existing = store
        .load_or_create_character_with_gameplay(owner.id, "aria", "ignored", replacement)
        .await
        .unwrap();
    assert_eq!(existing, character);
    assert_eq!(
        store
            .load_character_gameplay(character.id)
            .await
            .unwrap()
            .attributes
            .strength,
        11
    );
}

#[tokio::test]
async fn characters_only_commit_is_atomic_and_leaves_runtime_and_events_alone() {
    let store = SqliteCharacterStore::open(":memory:").await.unwrap();
    let owner = account(&store, "Rows Owner").await;
    let mut character = store
        .load_or_create_character(owner.id, "Rows", "room:a")
        .await
        .unwrap();
    let mut runtime = World::demo().runtime_snapshot();
    runtime.logical_tick = 7;
    let event = DomainEvent::CharacterMoved {
        actor_id: character.id,
        actor_name: character.name.clone(),
        from_room_id: "room:a".into(),
        to_room_id: "room:a".into(),
        direction: Direction::North,
    };
    store
        .commit_action(
            vec![DurableCharacterSnapshot {
                character: character.clone(),
                gameplay: CharacterGameplaySnapshot::default(),
            }],
            runtime.clone(),
            vec![event.clone()],
        )
        .await
        .unwrap();

    character.room_id = "room:b".into();
    let changed = DurableCharacterSnapshot {
        character: character.clone(),
        gameplay: CharacterGameplaySnapshot {
            coins: 4_321,
            ..CharacterGameplaySnapshot::default()
        },
    };
    let mut missing = changed.clone();
    missing.character.id = CharacterId(9_999);
    // One unknown row rolls back the whole characters-only transaction.
    assert!(
        store
            .commit_characters(vec![changed.clone(), missing])
            .await
            .is_err()
    );
    assert_eq!(
        store
            .load_character_gameplay(character.id)
            .await
            .unwrap()
            .coins,
        CharacterGameplaySnapshot::default().coins
    );

    store.commit_characters(vec![changed]).await.unwrap();
    assert_eq!(
        store.load_character(owner.id, character.id).await.unwrap(),
        character
    );
    assert_eq!(
        store
            .load_character_gameplay(character.id)
            .await
            .unwrap()
            .coins,
        4_321
    );
    assert_eq!(store.load_runtime_snapshot().await.unwrap(), Some(runtime));
    assert_eq!(store.load_domain_events().await.unwrap(), vec![event]);
}

#[tokio::test]
async fn character_and_shared_world_events_commit_atomically() {
    let store = SqliteCharacterStore::open(":memory:").await.unwrap();
    let owner = account(&store, "Event Owner").await;
    let mut character = store
        .load_or_create_character(owner.id, "Aria", "room:a")
        .await
        .unwrap();
    character.room_id = "room:b".into();
    let event = DomainEvent::CharacterMoved {
        actor_id: character.id,
        actor_name: character.name.clone(),
        from_room_id: "room:a".into(),
        to_room_id: "room:b".into(),
        direction: Direction::North,
    };

    store
        .commit_action(
            vec![DurableCharacterSnapshot {
                character: character.clone(),
                gameplay: CharacterGameplaySnapshot {
                    coins: 28,
                    commodities: std::collections::BTreeMap::from([("tonic".into(), 1)]),
                    ..CharacterGameplaySnapshot::default()
                },
            }],
            {
                let mut runtime = World::demo().runtime_snapshot();
                runtime.adventure.shop_stock.insert("tonic".into(), 19);
                runtime
            },
            vec![event.clone()],
        )
        .await
        .unwrap();
    assert_eq!(
        store.load_character(owner.id, character.id).await.unwrap(),
        character
    );
    assert_eq!(store.load_domain_events().await.unwrap(), vec![event]);
    assert_eq!(
        store
            .load_character_gameplay(character.id)
            .await
            .unwrap()
            .coins,
        28
    );

    let mut missing = character;
    missing.id = CharacterId(9999);
    let rejected = DomainEvent::ItemTaken {
        actor_id: missing.id,
        actor_name: missing.name.clone(),
        item_id: "item:missing".into(),
        item_name: "missing".into(),
        room_id: "room:b".into(),
    };
    assert!(
        store
            .commit_action(
                vec![DurableCharacterSnapshot {
                    character: missing,
                    gameplay: CharacterGameplaySnapshot::default(),
                }],
                {
                    let mut runtime = World::demo().runtime_snapshot();
                    runtime.logical_tick = 99;
                    runtime.adventure.shop_stock.insert("tonic".into(), 0);
                    runtime
                },
                vec![rejected],
            )
            .await
            .is_err()
    );
    assert_eq!(store.load_domain_events().await.unwrap().len(), 1);
    assert_eq!(
        store
            .load_runtime_snapshot()
            .await
            .unwrap()
            .unwrap()
            .logical_tick,
        0
    );
    assert_eq!(
        store
            .load_runtime_snapshot()
            .await
            .unwrap()
            .unwrap()
            .adventure
            .shop_stock["tonic"],
        19
    );
}

#[test]
fn sqlite_runtime_loader_upgrades_v1_items_without_losing_identity_or_location() {
    let mut connection = Connection::open_in_memory().unwrap();
    migrate(&mut connection).unwrap();
    let legacy = WorldRuntimeSnapshotV1 {
        snapshot_version: 1,
        revision: 7,
        logical_tick: 3,
        rng_state: 42,
        item_locations: BTreeMap::from([(
            "legacy:lamp".into(),
            RuntimeItemLocationV2::Room {
                room_id: "demo:lantern-square".into(),
            },
        )]),
        npcs: BTreeMap::new(),
        adventure: tiamud_core::AdventureRuntimeSnapshot::default(),
        progression_flags: Default::default(),
    };
    connection
        .execute(
            "INSERT INTO world_runtime
                 (singleton, snapshot_version, snapshot_json, updated_at)
             VALUES (1, 1, ?1, unixepoch())",
            [serde_json::to_string(&legacy).unwrap()],
        )
        .unwrap();

    let upgraded = load_runtime_snapshot(&connection).unwrap().unwrap();
    assert_eq!(
        upgraded.snapshot_version,
        tiamud_core::WORLD_RUNTIME_SNAPSHOT_VERSION
    );
    let item = upgraded
        .item_instances
        .get(&ItemInstanceId("legacy:lamp".into()))
        .unwrap();
    assert_eq!(item.prototype_id, "legacy:lamp");
    assert_eq!(
        item.location,
        RuntimeItemLocation::Room {
            room_id: "demo:lantern-square".into()
        }
    );
    assert_eq!(item.binding, ItemBinding::Unbound);
}

#[test]
fn sqlite_runtime_loader_upgrades_v2_before_validating_current_version() {
    let mut connection = Connection::open_in_memory().unwrap();
    migrate(&mut connection).unwrap();
    let legacy = WorldRuntimeSnapshotV2 {
        snapshot_version: 2,
        revision: 7,
        logical_tick: 3,
        rng_state: 42,
        next_item_instance_sequence: 2,
        item_instances: BTreeMap::from([(
            ItemInstanceId("runtime:1".into()),
            RuntimeItemInstanceSnapshotV2 {
                prototype_id: "legacy:lamp".into(),
                location: RuntimeItemLocationV2::Room {
                    room_id: "demo:lantern-square".into(),
                },
            },
        )]),
        npcs: BTreeMap::new(),
        adventure: tiamud_core::AdventureRuntimeSnapshot::default(),
        progression_flags: Default::default(),
    };
    connection
        .execute(
            "INSERT INTO world_runtime
                 (singleton, snapshot_version, snapshot_json, updated_at)
             VALUES (1, 2, ?1, unixepoch())",
            [serde_json::to_string(&legacy).unwrap()],
        )
        .unwrap();

    let upgraded = load_runtime_snapshot(&connection).unwrap().unwrap();
    assert_eq!(
        upgraded.snapshot_version,
        tiamud_core::WORLD_RUNTIME_SNAPSHOT_VERSION
    );
    assert_eq!(
        upgraded.item_instances[&ItemInstanceId("runtime:1".into())].location,
        RuntimeItemLocation::Room {
            room_id: "demo:lantern-square".into(),
        }
    );
    assert_eq!(
        upgraded.item_instances[&ItemInstanceId("runtime:1".into())].binding,
        ItemBinding::Unbound
    );
}

#[test]
fn legacy_runtime_upgrade_rejects_a_current_snapshot_above_the_current_budget() {
    let build = |count: usize| WorldRuntimeSnapshotV2 {
        snapshot_version: 2,
        revision: 1,
        logical_tick: 0,
        rng_state: 1,
        next_item_instance_sequence: 1,
        item_instances: (0..count)
            .map(|index| {
                (
                    ItemInstanceId(format!("legacy:{index}")),
                    RuntimeItemInstanceSnapshotV2 {
                        prototype_id: "p".into(),
                        location: RuntimeItemLocationV2::Staged,
                    },
                )
            })
            .collect(),
        npcs: BTreeMap::new(),
        adventure: tiamud_core::AdventureRuntimeSnapshot::default(),
        progression_flags: BTreeSet::new(),
    };

    let mut accepted = 0_usize;
    let mut rejected = MAX_ITEM_INSTANCES + 1;
    while accepted + 1 < rejected {
        let candidate = accepted + (rejected - accepted) / 2;
        let bytes = serde_json::to_vec(&build(candidate)).unwrap().len();
        if bytes <= tiamud_core::MAX_LEGACY_WORLD_RUNTIME_SNAPSHOT_BYTES {
            accepted = candidate;
        } else {
            rejected = candidate;
        }
    }
    let legacy_json = serde_json::to_string(&build(accepted)).unwrap();
    assert!(
        legacy_json.len() > tiamud_core::MAX_LEGACY_WORLD_RUNTIME_SNAPSHOT_BYTES - 4_096,
        "fixture should exercise a near-limit legacy snapshot"
    );
    let upgraded = tiamud_core::decode_world_runtime_snapshot(2, &legacy_json).unwrap();
    assert!(
        serde_json::to_vec(&upgraded).unwrap().len()
            > tiamud_core::MAX_WORLD_RUNTIME_SNAPSHOT_BYTES,
        "the fixture must reproduce expansion beyond the current budget"
    );
    assert!(matches!(
        decode_runtime_snapshot(2, &legacy_json),
        Err(StoreError::SnapshotTooLarge {
            kind: "world runtime",
            maximum: tiamud_core::MAX_WORLD_RUNTIME_SNAPSHOT_BYTES,
        })
    ));
}

#[test]
fn sqlite_runtime_loader_upgrades_v3_binding_as_unbound_and_rejects_smuggling() {
    let legacy = WorldRuntimeSnapshotV3 {
        snapshot_version: 3,
        revision: 8,
        logical_tick: 4,
        rng_state: 43,
        next_item_instance_sequence: 3,
        item_instances: BTreeMap::from([(
            ItemInstanceId("runtime:contained".into()),
            RuntimeItemInstanceSnapshotV3 {
                prototype_id: "legacy:map".into(),
                location: RuntimeItemLocation::Contained {
                    container_id: ItemInstanceId("runtime:satchel".into()),
                },
            },
        )]),
        npcs: BTreeMap::new(),
        adventure: tiamud_core::AdventureRuntimeSnapshot::default(),
        progression_flags: Default::default(),
    };

    let mut connection = Connection::open_in_memory().unwrap();
    migrate(&mut connection).unwrap();
    connection
        .execute(
            "INSERT INTO world_runtime
                 (singleton, snapshot_version, snapshot_json, updated_at)
             VALUES (1, 3, ?1, unixepoch())",
            [serde_json::to_string(&legacy).unwrap()],
        )
        .unwrap();

    let upgraded = load_runtime_snapshot(&connection).unwrap().unwrap();
    let item = &upgraded.item_instances[&ItemInstanceId("runtime:contained".into())];
    assert_eq!(item.binding, ItemBinding::Unbound);
    assert!(matches!(
        item.location,
        RuntimeItemLocation::Contained { .. }
    ));

    let mut smuggled = serde_json::to_value(&legacy).unwrap();
    smuggled["itemInstances"]["runtime:contained"]["binding"] =
        serde_json::json!({"kind":"character", "characterId":7});
    connection
        .execute(
            "UPDATE world_runtime SET snapshot_json = ?1 WHERE singleton = 1",
            [smuggled.to_string()],
        )
        .unwrap();
    assert!(matches!(
        load_runtime_snapshot(&connection),
        Err(StoreError::InvalidRuntimeSnapshot)
    ));
}

#[test]
fn sqlite_runtime_loader_upgrades_strict_v4_and_rejects_trade_smuggling() {
    let mut connection = Connection::open_in_memory().unwrap();
    migrate(&mut connection).unwrap();
    let mut legacy = serde_json::to_value(World::demo().runtime_snapshot()).unwrap();
    legacy["snapshotVersion"] = serde_json::json!(4);
    for field in [
        "nextTradeId",
        "trades",
        "nextLootDropId",
        "itemProvenance",
        "lootEncounters",
        "groundLoot",
        "partyLootCursors",
        "nextRewardOpenId",
        "areaResidency",
    ] {
        legacy.as_object_mut().unwrap().remove(field);
    }
    for item in legacy["itemInstances"]
        .as_object_mut()
        .unwrap()
        .values_mut()
    {
        item.as_object_mut().unwrap().remove("itemAffixes");
    }
    connection
        .execute(
            "INSERT INTO world_runtime
                 (singleton, snapshot_version, snapshot_json, updated_at)
             VALUES (1, 4, ?1, unixepoch())",
            [legacy.to_string()],
        )
        .unwrap();

    let upgraded = load_runtime_snapshot(&connection).unwrap().unwrap();
    assert_eq!(
        upgraded.snapshot_version,
        tiamud_core::WORLD_RUNTIME_SNAPSHOT_VERSION
    );
    assert_eq!(upgraded.next_trade_id, tiamud_core::TradeId(1));
    assert!(upgraded.trades.is_empty());

    legacy["trades"] = serde_json::json!({});
    connection
        .execute(
            "UPDATE world_runtime SET snapshot_json = ?1 WHERE singleton = 1",
            [legacy.to_string()],
        )
        .unwrap();
    assert!(matches!(
        load_runtime_snapshot(&connection),
        Err(StoreError::InvalidRuntimeSnapshot)
    ));
}

#[test]
fn sqlite_runtime_loader_upgrades_strict_v6_and_rejects_v7_smuggling() {
    let mut legacy = serde_json::to_value(World::demo().runtime_snapshot()).unwrap();
    legacy["snapshotVersion"] = serde_json::json!(6);
    legacy.as_object_mut().unwrap().remove("nextRewardOpenId");
    legacy.as_object_mut().unwrap().remove("areaResidency");
    for item in legacy["itemInstances"]
        .as_object_mut()
        .unwrap()
        .values_mut()
    {
        item.as_object_mut().unwrap().remove("itemAffixes");
    }
    let json = legacy.to_string();
    let upgraded = decode_runtime_snapshot(6, &json).unwrap();
    assert_eq!(
        upgraded.snapshot_version,
        tiamud_core::WORLD_RUNTIME_SNAPSHOT_VERSION
    );
    assert_eq!(upgraded.next_reward_open_id, tiamud_core::RewardOpenId(1));
    assert!(
        upgraded
            .item_instances
            .values()
            .all(|item| item.item_affixes.is_empty())
    );

    let item_id = legacy["itemInstances"]
        .as_object()
        .unwrap()
        .keys()
        .next()
        .unwrap()
        .clone();
    legacy["itemInstances"][item_id]["itemAffixes"] = serde_json::json!([]);
    assert!(matches!(
        decode_runtime_snapshot(6, &legacy.to_string()),
        Err(StoreError::InvalidRuntimeSnapshot)
    ));
}

#[test]
fn sqlite_runtime_loader_rejects_an_unsupported_snapshot_version() {
    let mut connection = Connection::open_in_memory().unwrap();
    migrate(&mut connection).unwrap();
    let legacy = serde_json::json!({
        "snapshotVersion": 99,
        "revision": 1,
        "logicalTick": 0,
        "rngState": 1,
        "nextItemInstanceSequence": 1,
        "itemInstances": {},
        "npcs": {}
    });
    connection
        .execute(
            "INSERT INTO world_runtime
                 (singleton, snapshot_version, snapshot_json, updated_at)
             VALUES (1, 99, ?1, unixepoch())",
            [legacy.to_string()],
        )
        .unwrap();

    assert!(matches!(
        load_runtime_snapshot(&connection),
        Err(StoreError::InvalidRuntimeSnapshot)
    ));
}

#[test]
fn sqlite_runtime_loader_rejects_column_payload_version_mismatch() {
    let mut connection = Connection::open_in_memory().unwrap();
    migrate(&mut connection).unwrap();
    let runtime = World::demo().runtime_snapshot();
    connection
        .execute(
            "INSERT INTO world_runtime
                 (singleton, snapshot_version, snapshot_json, updated_at)
             VALUES (1, 1, ?1, unixepoch())",
            [serde_json::to_string(&runtime).unwrap()],
        )
        .unwrap();

    assert!(matches!(
        load_runtime_snapshot(&connection),
        Err(StoreError::InvalidRuntimeSnapshot)
    ));
}

#[test]
fn sqlite_runtime_loader_rejects_multibyte_snapshot_above_byte_limit() {
    let mut connection = Connection::open_in_memory().unwrap();
    migrate(&mut connection).unwrap();
    let oversized = format!(
        "\"{}\"",
        "é".repeat(tiamud_core::MAX_LEGACY_WORLD_RUNTIME_SNAPSHOT_BYTES / 2)
    );
    assert!(oversized.chars().count() <= MAX_RUNTIME_JSON_BYTES);
    assert!(oversized.len() > tiamud_core::MAX_LEGACY_WORLD_RUNTIME_SNAPSHOT_BYTES);
    assert!(oversized.len() <= MAX_RUNTIME_JSON_BYTES);
    connection
        .execute(
            "INSERT INTO world_runtime
                 (singleton, snapshot_version, snapshot_json, updated_at)
             VALUES (1, 2, ?1, unixepoch())",
            [&oversized],
        )
        .unwrap();

    assert!(matches!(
        load_runtime_snapshot(&connection),
        Err(StoreError::SnapshotTooLarge {
            kind: "world runtime",
            maximum: tiamud_core::MAX_LEGACY_WORLD_RUNTIME_SNAPSHOT_BYTES
        })
    ));
}

#[test]
fn snapshot_v6_and_v7_use_the_two_mib_admission_budget() {
    let between_budgets = format!(
        "\"{}\"",
        "é".repeat(tiamud_core::MAX_LEGACY_WORLD_RUNTIME_SNAPSHOT_BYTES / 2)
    );
    assert!(between_budgets.len() > tiamud_core::MAX_LEGACY_WORLD_RUNTIME_SNAPSHOT_BYTES);
    assert!(between_budgets.len() <= MAX_RUNTIME_JSON_BYTES);

    assert!(matches!(
        decode_runtime_snapshot(5, &between_budgets),
        Err(StoreError::SnapshotTooLarge {
            kind: "world runtime",
            maximum: tiamud_core::MAX_LEGACY_WORLD_RUNTIME_SNAPSHOT_BYTES,
        })
    ));
    assert!(matches!(
        decode_runtime_snapshot(6, &between_budgets),
        Err(StoreError::InvalidRuntimeSnapshot)
    ));
    assert!(matches!(
        decode_runtime_snapshot(7, &between_budgets),
        Err(StoreError::InvalidRuntimeSnapshot)
    ));

    let above_current = format!("\"{}\"", "é".repeat(MAX_RUNTIME_JSON_BYTES / 2));
    assert!(above_current.len() > MAX_RUNTIME_JSON_BYTES);
    assert!(matches!(
        decode_runtime_snapshot(7, &above_current),
        Err(StoreError::SnapshotTooLarge {
            kind: "world runtime",
            maximum: MAX_RUNTIME_JSON_BYTES,
        })
    ));
}

#[test]
fn startup_owner_gameplay_enforces_the_shared_aggregate_byte_budget() {
    let json = serde_json::to_string(&CharacterGameplaySnapshot::default()).unwrap();
    let mut total = MAX_STARTUP_OWNER_GAMEPLAY_BYTES - json.len() + 1;

    assert!(matches!(
        decode_startup_gameplay(
            CharacterId(1),
            &json,
            &mut total,
            StartupGameplayLoadLimits::ITEM_OWNERS,
        ),
        Err(StoreError::SnapshotTooLarge {
            kind: "startup owner gameplay",
            maximum: MAX_STARTUP_OWNER_GAMEPLAY_BYTES
        })
    ));
}

#[test]
fn startup_trade_gameplay_uses_its_separate_exact_budget() {
    let json = serde_json::to_string(&CharacterGameplaySnapshot::default()).unwrap();
    let mut total = MAX_STARTUP_TRADE_GAMEPLAY_BYTES - json.len();
    decode_startup_gameplay(
        CharacterId(1),
        &json,
        &mut total,
        StartupGameplayLoadLimits::TRADE_PARTICIPANTS,
    )
    .unwrap();
    assert_eq!(total, MAX_STARTUP_TRADE_GAMEPLAY_BYTES);

    let mut overflowing_total = MAX_STARTUP_TRADE_GAMEPLAY_BYTES - json.len() + 1;
    assert!(matches!(
        decode_startup_gameplay(
            CharacterId(2),
            &json,
            &mut overflowing_total,
            StartupGameplayLoadLimits::TRADE_PARTICIPANTS,
        ),
        Err(StoreError::SnapshotTooLarge {
            kind: "startup trade participant gameplay",
            maximum: MAX_STARTUP_TRADE_GAMEPLAY_BYTES,
        })
    ));
}

#[test]
fn startup_trade_batch_rejects_excess_and_duplicate_results() {
    let ids = (1..=MAX_STARTUP_TRADE_PARTICIPANTS + 1)
        .map(|id| CharacterId(i64::try_from(id).unwrap()))
        .collect();
    assert!(matches!(
        normalise_startup_character_ids(ids, StartupGameplayLoadLimits::TRADE_PARTICIPANTS),
        Err(StoreError::TooManyCharacterAggregates {
            maximum: MAX_STARTUP_TRADE_PARTICIPANTS,
        })
    ));

    let aggregate = DurableCharacterGameplay {
        character_id: CharacterId(1),
        gameplay: CharacterGameplaySnapshot::default(),
    };
    assert!(matches!(
        ensure_all_startup_characters_loaded(&[CharacterId(1)], &[aggregate.clone(), aggregate],),
        Err(StoreError::InvalidRuntimeSnapshot)
    ));
}

#[test]
fn startup_identity_scope_deduplicates_and_rejects_invalid_or_excess_ids() {
    assert_eq!(
        normalise_character_identity_ids(vec![
            CharacterId(3),
            CharacterId(1),
            CharacterId(3),
            CharacterId(2),
        ])
        .unwrap(),
        vec![CharacterId(1), CharacterId(2), CharacterId(3)]
    );
    assert!(matches!(
        normalise_character_identity_ids(vec![CharacterId(0)]),
        Err(StoreError::InvalidCharacterIdentity(CharacterId(0)))
    ));

    let excess = (1..=MAX_STARTUP_CHARACTER_EXISTENCE_IDS + 1)
        .map(|id| CharacterId(i64::try_from(id).unwrap()))
        .collect();
    assert!(matches!(
        normalise_character_identity_ids(excess),
        Err(StoreError::TooManyCharacterIdentities {
            maximum: MAX_STARTUP_CHARACTER_EXISTENCE_IDS,
        })
    ));
}

#[test]
fn sqlite_identity_existence_query_crosses_the_parameter_batch_boundary() {
    let mut connection = Connection::open_in_memory().unwrap();
    initialise(&mut connection).unwrap();
    connection
        .execute("INSERT INTO accounts (id, name) VALUES (1, 'Owner')", [])
        .unwrap();
    let transaction = connection.transaction().unwrap();
    {
        let mut statement = transaction
            .prepare(
                "INSERT INTO characters (id, account_id, name, room_id)
                 VALUES (?1, 1, ?2, 'test:start')",
            )
            .unwrap();
        for id in 1..=SQLITE_CHARACTER_BATCH_SIZE + 1 {
            let id = i64::try_from(id).unwrap();
            statement
                .execute(params![id, format!("Character {id}")])
                .unwrap();
        }
    }
    transaction.commit().unwrap();

    let ids = (1..=SQLITE_CHARACTER_BATCH_SIZE + 1)
        .rev()
        .map(|id| CharacterId(i64::try_from(id).unwrap()))
        .collect();
    ensure_character_ids_exist_batch(&connection, ids).unwrap();
    assert!(matches!(
        ensure_character_ids_exist_batch(
            &connection,
            vec![CharacterId(
                i64::try_from(SQLITE_CHARACTER_BATCH_SIZE + 2).unwrap()
            )],
        ),
        Err(StoreError::CharacterIdentityNotFound(_))
    ));
}

#[tokio::test]
async fn sqlite_store_checks_identity_only_with_dedupe_and_fail_closed_errors() {
    let store = SqliteCharacterStore::open(":memory:").await.unwrap();
    let owner = account(&store, "Identity Owner").await;
    let first = store
        .load_or_create_character(owner.id, "Aria", "demo:lantern-square")
        .await
        .unwrap();
    let second = store
        .load_or_create_character(owner.id, "Bryn", "demo:lantern-square")
        .await
        .unwrap();

    store
        .ensure_character_ids_exist_batch(vec![second.id, first.id, second.id])
        .await
        .unwrap();
    store
        .ensure_character_ids_exist_batch(Vec::new())
        .await
        .unwrap();
    assert!(matches!(
        store
            .ensure_character_ids_exist_batch(vec![CharacterId(0)])
            .await,
        Err(StoreError::InvalidCharacterIdentity(CharacterId(0)))
    ));
    let missing = CharacterId(second.id.0 + 1_000);
    assert!(matches!(
        store.ensure_character_ids_exist_batch(vec![missing]).await,
        Err(StoreError::CharacterIdentityNotFound(character_id)) if character_id == missing
    ));
}

#[tokio::test]
async fn gameplay_and_runtime_snapshots_round_trip_together() {
    let store = SqliteCharacterStore::open(":memory:").await.unwrap();
    let owner = account(&store, "Snapshot Owner").await;
    let character = store
        .load_or_create_character(owner.id, "Aria", "demo:lantern-square")
        .await
        .unwrap();
    let defaults = CharacterGameplaySnapshot::default();
    let gameplay = CharacterGameplaySnapshot {
        level: 4,
        experience: 375,
        resources: std::collections::BTreeMap::from([(
            "tiamud:focus".into(),
            tiamud_core::ResourcePool {
                current: 7,
                max: 12,
            },
        )]),
        ability_cooldowns: std::collections::BTreeMap::from([("tiamud:mind-whisper".into(), 23)]),
        vitals: tiamud_core::Vitals {
            health: 0,
            max_health: defaults.vitals.max_health,
        },
        equipment: std::collections::BTreeMap::from([(
            EquipmentSlot::Weapon,
            ItemInstanceId("tiamud:bronze-sword".to_owned()),
        )]),
        effects: vec![ActiveEffect {
            kind: EffectKind::Strength,
            magnitude: 3,
            remaining_ticks: 2,
        }],
        recovery_at_tick: Some(19),
        coins: 123,
        commodities: std::collections::BTreeMap::from([("tonic".into(), 2)]),
        quests: std::collections::BTreeMap::from([(
            "first-hunt".into(),
            tiamud_core::QuestProgress {
                status: tiamud_core::QuestStatus::ReadyToClaim,
                current: 1,
                target: 1,
            },
        )]),
        script_flags: std::collections::BTreeSet::from(["archive-welcomed".into()]),
        scheduled_script_flags: vec![tiamud_core::ScheduledScriptFlag {
            flag: "archive-follow-up".into(),
            due_tick: 23,
        }],
        ..defaults
    };
    let loaded = crate::world_loader::default_loaded_world().unwrap();
    let mut runtime = crate::world_loader::build_loaded_world(&loaded)
        .unwrap()
        .runtime_snapshot();
    runtime.logical_tick = 17;
    runtime.adventure.next_weather_tick = 20;
    runtime.adventure.shop_stock.insert("tonic".into(), 18);
    let sword = runtime
        .item_instances
        .get_mut(&ItemInstanceId("tiamud:bronze-sword".into()))
        .unwrap();
    sword.location = RuntimeItemLocation::Equipped {
        character_id: character.id,
        slot: EquipmentSlot::Weapon,
    };
    sword.binding = ItemBinding::Unbound;
    let mut restored = crate::world_loader::build_loaded_world(&loaded).unwrap();
    restored.restore_runtime_snapshot(runtime.clone()).unwrap();

    store
        .commit_action(
            vec![DurableCharacterSnapshot {
                character: character.clone(),
                gameplay: gameplay.clone(),
            }],
            runtime.clone(),
            Vec::new(),
        )
        .await
        .unwrap();

    assert_eq!(
        store.load_character_gameplay(character.id).await.unwrap(),
        gameplay
    );
    assert_eq!(
        store
            .load_character_gameplay_batch(
                vec![character.id],
                StartupGameplayLoadLimits::ITEM_OWNERS,
            )
            .await
            .unwrap(),
        vec![DurableCharacterGameplay {
            character_id: character.id,
            gameplay: gameplay.clone(),
        }]
    );
    assert_eq!(store.load_runtime_snapshot().await.unwrap(), Some(runtime));
}

#[tokio::test]
async fn items_given_event_round_trips_through_sqlite() {
    let store = SqliteCharacterStore::open(":memory:").await.unwrap();
    let event = DomainEvent::ItemsGiven {
        giver_id: CharacterId(11),
        giver_name: "Aria".into(),
        recipient_id: CharacterId(12),
        recipient_name: "Bryn".into(),
        item_ids: vec![ItemInstanceId("runtime:gift-1".into())],
        item_name: "gift".into(),
        room_id: "demo:lantern-square".into(),
    };

    store
        .commit_action(
            Vec::new(),
            World::demo().runtime_snapshot(),
            vec![event.clone()],
        )
        .await
        .unwrap();

    assert_eq!(store.load_domain_events().await.unwrap(), vec![event]);
}

#[tokio::test]
async fn v3_progression_survives_a_sqlite_restart_and_restores_fail_closed() {
    let temporary = tempfile::TempDir::new().unwrap();
    let database_path = temporary.path().join("progression.sqlite3");
    let store = SqliteCharacterStore::open(&database_path).await.unwrap();
    let owner = account(&store, "Progression Owner").await;
    let loaded = crate::world_loader::default_loaded_world().unwrap();
    let mut world = crate::world_loader::build_loaded_world(&loaded).unwrap();
    let mut gameplay = world
        .initial_gameplay(
            Some("tiamud:mortal"),
            BackgroundSelection::Id("tiamud:city-guard"),
            Some("tiamud:vanguard"),
        )
        .unwrap();
    gameplay.level = 5;
    gameplay.experience = 1_600;
    gameplay.progression_points.insert("talent".into(), 1);
    gameplay.tags.insert("tiamud:founder".into());
    let character = store
        .load_or_create_character_with_gameplay(
            owner.id,
            "Iria",
            world.start_room_id(),
            gameplay.clone(),
        )
        .await
        .unwrap();

    world
        .restore_character_gameplay(character.id, gameplay)
        .unwrap();
    world
        .receive_progression_signal(character.id, "tiamud:heard-call")
        .unwrap();
    let session = SessionId(77);
    world.connect(session, character.clone()).unwrap();
    let talent = world
        .apply(
            session,
            PlayerAction::AcquireTalent {
                graph_id: "tiamud:martial-talents".into(),
                node_id: "tiamud:guarded-stance".into(),
            },
        )
        .unwrap();
    let promotion = world
        .apply(
            session,
            PlayerAction::Promote {
                class_id: "tiamud:bastion".into(),
            },
        )
        .unwrap();
    let mut events = talent.domain_events;
    events.extend(promotion.domain_events);
    store
        .commit_action(
            vec![world.durable_character_snapshot(session).unwrap()],
            world.runtime_snapshot(),
            events,
        )
        .await
        .unwrap();
    store.shutdown().await.unwrap();
    drop(store);

    let reopened = SqliteCharacterStore::open(&database_path).await.unwrap();
    let restored = reopened
        .load_character_gameplay(character.id)
        .await
        .unwrap();
    assert_eq!(restored.lineage_id, "tiamud:mortal");
    assert_eq!(restored.background_id.as_deref(), Some("tiamud:city-guard"));
    assert_eq!(restored.base_class_id, "tiamud:vanguard");
    assert_eq!(restored.active_class_id, "tiamud:bastion");
    assert_eq!(
        restored.class_history,
        vec!["tiamud:vanguard".to_owned(), "tiamud:bastion".to_owned()]
    );
    assert_eq!(restored.progression_points["talent"], 0);
    assert_eq!(
        restored.talent_node_ranks["tiamud:martial-talents"]["tiamud:guarded-stance"],
        1
    );
    assert!(restored.tags.contains("tiamud:founder"));
    assert!(restored.received_signals.contains("tiamud:heard-call"));

    let mut restarted_world = crate::world_loader::build_loaded_world(&loaded).unwrap();
    restarted_world
        .restore_character_gameplay(character.id, restored.clone())
        .unwrap();
    restarted_world.connect(session, character.clone()).unwrap();
    assert_eq!(
        restarted_world
            .character_gameplay_snapshot(session)
            .unwrap(),
        restored
    );
    let mut unresolved = restored.clone();
    unresolved.active_class_id = "missing:class".into();
    unresolved.class_history[1] = "missing:class".into();
    assert!(matches!(
        restarted_world.restore_character_gameplay(character.id, unresolved),
        Err(WorldError::InvalidGameplayDefinition)
    ));
    assert_eq!(
        restarted_world
            .character_gameplay_snapshot(session)
            .unwrap(),
        restored,
        "a rejected unresolved definition must not mutate restored state"
    );
    reopened.shutdown().await.unwrap();
}

#[tokio::test]
async fn accounts_can_own_same_named_characters_without_identity_collision() {
    let store = SqliteCharacterStore::open(":memory:").await.unwrap();
    let first = account(&store, "First").await;
    let second = account(&store, "Second").await;
    let first_character = store
        .load_or_create_character(first.id, "Hero", "room:a")
        .await
        .unwrap();
    let second_character = store
        .load_or_create_character(second.id, "Hero", "room:b")
        .await
        .unwrap();

    assert_ne!(first_character.id, second_character.id);
    assert_eq!(first_character.account_id, first.id);
    assert_eq!(second_character.account_id, second.id);
    assert_eq!(store.list_characters(first.id).await.unwrap().len(), 1);
    assert_eq!(store.list_characters(second.id).await.unwrap().len(), 1);
}

#[tokio::test]
async fn character_selection_checks_account_ownership() {
    let store = SqliteCharacterStore::open(":memory:").await.unwrap();
    let owner = account(&store, "Owner").await;
    let stranger = account(&store, "Stranger").await;
    let character = store
        .load_or_create_character(owner.id, "Private", "room:a")
        .await
        .unwrap();

    assert!(matches!(
        store.load_character(stranger.id, character.id).await,
        Err(StoreError::CharacterNotFound { .. })
    ));
}

#[tokio::test]
async fn missing_accounts_and_invalid_names_are_rejected() {
    let store = SqliteCharacterStore::open(":memory:").await.unwrap();
    assert!(matches!(
        store.load_or_create_account(" \n ").await,
        Err(StoreError::InvalidName { kind: "account" })
    ));
    assert!(matches!(
        store.load_or_create_account("A\u{202e}B").await,
        Err(StoreError::InvalidName { kind: "account" })
    ));
    assert!(matches!(
        store.load_or_create_account("A|B").await,
        Err(StoreError::InvalidName { kind: "account" })
    ));
    assert!(matches!(
        store
            .load_or_create_character(AccountId(999), "Hero", "room:a")
            .await,
        Err(StoreError::AccountNotFound(AccountId(999)))
    ));
}

#[test]
fn durable_ids_are_unchanged_after_the_database_is_reopened() {
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let path = std::env::temp_dir().join(format!(
        "mud-persistence-{}-{nonce}.sqlite3",
        std::process::id()
    ));

    let (account_id, character_id, login_token, login_account_id) = {
        let mut connection = Connection::open(&path).unwrap();
        initialise(&mut connection).unwrap();
        let account = load_or_create_account(&connection, "Persistent Player").unwrap();
        let character = load_or_create_character(
            &connection,
            account.id,
            "Persistent Hero",
            "demo:lantern-square",
        )
        .unwrap();
        let login = register_account(
            &mut connection,
            "Persistent Login",
            "correct horse battery staple",
            1_000,
        )
        .unwrap();
        (
            account.id,
            character.id,
            login.token.as_str().to_owned(),
            login.account.id,
        )
    };

    {
        let mut connection = Connection::open(&path).unwrap();
        initialise(&mut connection).unwrap();
        let account = load_or_create_account(&connection, "persistent player").unwrap();
        let character = load_character(&connection, account.id, character_id).unwrap();
        assert_eq!(account.id, account_id);
        assert_eq!(character.id, character_id);
        assert_eq!(character.account_id, account_id);
        assert_eq!(
            resolve_session(&connection, &login_token, 1_001)
                .unwrap()
                .id,
            login_account_id
        );
    }

    for suffix in ["", "-shm", "-wal"] {
        let _ = std::fs::remove_file(format!("{}{suffix}", path.display()));
    }
}

#[test]
fn migrates_the_name_keyed_bootstrap_schema_without_losing_location() {
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

    initialise(&mut connection).unwrap();
    let migrated_account = load_or_create_account(&connection, "legacy hero").unwrap();
    let migrated = list_characters(&connection, migrated_account.id).unwrap();

    assert_eq!(migrated.len(), 1);
    assert_eq!(migrated[0].name, "Legacy Hero");
    assert_eq!(migrated[0].room_id, "demo:north-gate");
    assert!(migrated[0].id.0 > 0);
    let migrated_column: bool = connection
        .query_row(
            "SELECT EXISTS(
                 SELECT 1 FROM pragma_table_info('characters') WHERE name = 'account_id'
             )",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert!(migrated_column);
}

#[test]
fn migrates_v2_accounts_as_explicit_guests_without_losing_rosters() {
    let mut connection = Connection::open_in_memory().unwrap();
    connection
        .execute_batch(
            "CREATE TABLE accounts (
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
             INSERT INTO accounts (id, name) VALUES (7, 'Legacy Guest');
             INSERT INTO characters (id, account_id, name, room_id)
                 VALUES (9, 7, 'Old Hero', 'demo:north-gate');
             PRAGMA user_version = 2;",
        )
        .unwrap();

    initialise(&mut connection).unwrap();
    let (password_hash, state): (Option<String>, String) = connection
        .query_row(
            "SELECT password_hash, auth_state FROM accounts WHERE id = 7",
            [],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .unwrap();
    let roster = list_characters(&connection, AccountId(7)).unwrap();

    assert_eq!(password_hash, None);
    assert_eq!(state, "guest");
    assert_eq!(roster[0].id, CharacterId(9));
    assert_eq!(roster[0].room_id, "demo:north-gate");
    assert!(matches!(
        login_account(
            &mut connection,
            "legacy guest",
            "correct horse battery staple",
            1_000
        ),
        Err(StoreError::InvalidCredentials)
    ));
}

#[test]
fn sqlite_report_resolution_binds_the_real_target_and_rejects_a_discordant_audit() {
    let mut connection = Connection::open_in_memory().unwrap();
    initialise(&mut connection).unwrap();
    let admin = register_account(
        &mut connection,
        "Admin",
        "correct horse battery staple",
        1_000,
    )
    .unwrap();
    let reporter = register_account(
        &mut connection,
        "Reporter",
        "another correct horse battery",
        1_000,
    )
    .unwrap();
    let target = register_account(
        &mut connection,
        "Target",
        "a third correct horse battery",
        1_000,
    )
    .unwrap();
    let reporter_character =
        load_or_create_character(&connection, reporter.account.id, "Bryn", "room:start").unwrap();
    let target_character =
        load_or_create_character(&connection, target.account.id, "Cyra", "room:start").unwrap();

    report_character(
        &mut connection,
        reporter_character.id,
        target_character.id,
        "harassment in public chat",
        1_100,
    )
    .unwrap();
    let report_id = connection
        .query_row("SELECT MAX(id) FROM social_reports", [], |row| {
            row.get::<_, i64>(0)
        })
        .unwrap();
    resolve_staff_report(
        &mut connection,
        report_id,
        StaffReportResolution::ActionTaken,
        "evidence verified",
        StaffAuditRecord {
            actor_account_id: admin.account.id,
            target_account_id: None,
            capability: StaffCapability::ReportsResolve,
            action: "report_resolve".to_owned(),
            reason: "evidence verified".to_owned(),
            details: serde_json::json!({"report_id": report_id}),
        },
        1_200,
    )
    .unwrap();
    assert_eq!(
        connection
            .query_row(
                "SELECT status, resolution, resolution_reason,
                        resolved_by_account_id
                 FROM social_reports WHERE id = ?1",
                [report_id],
                |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, String>(2)?,
                        row.get::<_, i64>(3)?,
                    ))
                },
            )
            .unwrap(),
        (
            "resolved".to_owned(),
            "action_taken".to_owned(),
            "evidence verified".to_owned(),
            admin.account.id.0,
        )
    );
    assert_eq!(
        connection
            .query_row(
                "SELECT target_account_id FROM staff_audit
                 WHERE action = 'report_resolve'",
                [],
                |row| row.get::<_, i64>(0),
            )
            .unwrap(),
        target.account.id.0
    );

    report_character(
        &mut connection,
        reporter_character.id,
        target_character.id,
        "a second incident",
        1_300,
    )
    .unwrap();
    let second_report_id = connection
        .query_row("SELECT MAX(id) FROM social_reports", [], |row| {
            row.get::<_, i64>(0)
        })
        .unwrap();
    let audit_rows_before = connection
        .query_row("SELECT COUNT(*) FROM staff_audit", [], |row| {
            row.get::<_, i64>(0)
        })
        .unwrap();
    assert!(matches!(
        resolve_staff_report(
            &mut connection,
            second_report_id,
            StaffReportResolution::Dismissed,
            "forged target",
            StaffAuditRecord {
                actor_account_id: admin.account.id,
                target_account_id: Some(reporter.account.id),
                capability: StaffCapability::ReportsResolve,
                action: "report_resolve".to_owned(),
                reason: "forged target".to_owned(),
                details: serde_json::json!({"report_id": second_report_id}),
            },
            1_400,
        ),
        Err(StoreError::StaffForbidden)
    ));
    assert_eq!(
        connection
            .query_row(
                "SELECT status FROM social_reports WHERE id = ?1",
                [second_report_id],
                |row| row.get::<_, String>(0),
            )
            .unwrap(),
        "open"
    );
    assert_eq!(
        connection
            .query_row("SELECT COUNT(*) FROM staff_audit", [], |row| {
                row.get::<_, i64>(0)
            })
            .unwrap(),
        audit_rows_before
    );
}

#[tokio::test]
async fn social_policy_roles_bans_and_audit_survive_restart() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("social.db");
    let store = SqliteCharacterStore::open(&path).await.unwrap();
    let admin = store
        .register_account("Admin", "correct horse battery staple")
        .await
        .unwrap();
    let player = store
        .register_account("Player", "another correct horse battery")
        .await
        .unwrap();
    let third = store
        .register_account("Third", "a third correct horse battery")
        .await
        .unwrap();
    let admin_character = store
        .load_or_create_character(admin.account.id, "Aster", "room:start")
        .await
        .unwrap();
    let player_character = store
        .load_or_create_character(player.account.id, "Bryn", "room:start")
        .await
        .unwrap();
    let third_character = store
        .load_or_create_character(third.account.id, "Cyra", "room:start")
        .await
        .unwrap();

    assert_eq!(
        store.social_profile(admin_character.id).await.unwrap().role,
        AccountRole::Admin
    );
    assert!(matches!(
        store
            .moderate_character(
                player_character.id,
                third_character.id,
                ModerationAction::Mute {
                    expires_at: None,
                    reason: "not authorized".into(),
                },
            )
            .await,
        Err(StoreError::SocialForbidden)
    ));

    store
        .set_social_relation(
            player_character.id,
            admin_character.id,
            SocialRelationKind::Ignore,
            true,
        )
        .await
        .unwrap();
    assert!(
        !store
            .social_delivery_allowed(admin_character.id, player_character.id)
            .await
            .unwrap()
    );
    store
        .set_social_relation(
            player_character.id,
            third_character.id,
            SocialRelationKind::Block,
            true,
        )
        .await
        .unwrap();
    assert!(
        !store
            .social_delivery_allowed(player_character.id, third_character.id)
            .await
            .unwrap()
    );
    assert!(
        !store
            .social_delivery_allowed(third_character.id, player_character.id)
            .await
            .unwrap()
    );

    store
        .moderate_character(
            admin_character.id,
            player_character.id,
            ModerationAction::SetRole {
                role: AccountRole::Moderator,
                reason: "approved moderation assignment".to_owned(),
            },
        )
        .await
        .unwrap();
    assert!(matches!(
        store
            .moderate_character(
                player_character.id,
                third_character.id,
                ModerationAction::Mute {
                    expires_at: None,
                    reason: "legacy role is not authority".into(),
                },
            )
            .await,
        Err(StoreError::SocialForbidden)
    ));
    store
        .set_staff_profile(
            player.account.id,
            Some(StaffProfileKey::Moderator),
            None,
            StaffAuditRecord {
                actor_account_id: admin.account.id,
                target_account_id: Some(player.account.id),
                capability: StaffCapability::StaffAssign,
                action: "staff_profile_set".into(),
                reason: "approved moderation assignment".into(),
                details: serde_json::json!({
                    "actor_name": "Aster",
                    "target_name": "Bryn",
                    "profile": "moderator",
                }),
            },
        )
        .await
        .unwrap();
    store
        .moderate_character(
            player_character.id,
            third_character.id,
            ModerationAction::Mute {
                expires_at: None,
                reason: "repeated flooding".into(),
            },
        )
        .await
        .unwrap();
    store
        .report_character(
            player_character.id,
            third_character.id,
            "harassment in public chat",
        )
        .await
        .unwrap();
    store
        .moderate_character(
            admin_character.id,
            third_character.id,
            ModerationAction::Ban {
                expires_at: None,
                reason: "repeated severe abuse".into(),
            },
        )
        .await
        .unwrap();
    assert!(matches!(
        store
            .login_account("Third", "a third correct horse battery")
            .await,
        Err(StoreError::AccountBanned)
    ));

    drop(store);
    let reopened = SqliteCharacterStore::open(&path).await.unwrap();
    assert_eq!(
        reopened
            .social_profile(player_character.id)
            .await
            .unwrap()
            .role,
        AccountRole::Moderator
    );
    assert!(
        reopened
            .social_profile(third_character.id)
            .await
            .unwrap()
            .muted
    );
    assert!(
        !reopened
            .social_delivery_allowed(admin_character.id, player_character.id)
            .await
            .unwrap()
    );
    assert!(matches!(
        reopened
            .login_account("Third", "a third correct horse battery")
            .await,
        Err(StoreError::AccountBanned)
    ));
    let actions = reopened
        .audit_log()
        .await
        .unwrap()
        .into_iter()
        .map(|entry| entry.action)
        .collect::<Vec<_>>();
    for expected in [
        "bootstrap_admin",
        "ignore",
        "block",
        "set_role",
        "mute",
        "report",
        "ban",
    ] {
        assert!(actions.iter().any(|action| action == expected));
    }
    let verification = Connection::open(&path).unwrap();
    assert_eq!(
        verification
            .query_row(
                "SELECT reason FROM social_reports WHERE reporter_character_id = ?1",
                [player_character.id.0],
                |row| row.get::<_, String>(0),
            )
            .unwrap(),
        "harassment in public chat"
    );
}

#[test]
fn staff_authority_reports_and_audit_are_closed_bounded_and_hierarchical() {
    let mut connection = Connection::open_in_memory().unwrap();
    initialise(&mut connection).unwrap();
    let admin = finish_register_account(&mut connection, "Admin", "hash", 1_000).unwrap();
    let reporter = finish_register_account(&mut connection, "Reporter", "hash", 1_001).unwrap();
    let target = finish_register_account(&mut connection, "Target", "hash", 1_002).unwrap();
    let admin_character =
        load_or_create_character(&connection, admin.account.id, "Aster", "room:start").unwrap();
    let reporter_character =
        load_or_create_character(&connection, reporter.account.id, "Bryn", "room:start").unwrap();
    let target_character =
        load_or_create_character(&connection, target.account.id, "Cyra", "room:start").unwrap();

    let access = staff_access(&connection, admin_character.id).unwrap();
    assert_eq!(access.profile, Some(StaffProfileKey::Implementor));
    assert_eq!(
        access.capabilities,
        StaffCapability::ALL.into_iter().collect()
    );

    for sequence in 0..12 {
        report_character(
            &mut connection,
            reporter_character.id,
            target_character.id,
            &format!("report {sequence}"),
            1_100 + sequence,
        )
        .unwrap();
    }
    report_character(
        &mut connection,
        reporter_character.id,
        admin_character.id,
        "report targeting equal staff authority",
        1_200,
    )
    .unwrap();
    let reports = staff_reports(
        &mut connection,
        StaffAuditRecord {
            actor_account_id: admin.account.id,
            target_account_id: None,
            capability: StaffCapability::ReportsView,
            action: "reports.list".into(),
            reason: "review open reports".into(),
            details: serde_json::json!({}),
        },
        1_201,
    )
    .unwrap();
    assert_eq!(reports.len(), usize::try_from(STAFF_LIST_LIMIT).unwrap());
    assert!(
        reports
            .iter()
            .all(|report| report.target_account_id == target.account.id)
    );

    let report_id = reports[0].id;
    resolve_staff_report(
        &mut connection,
        report_id,
        StaffReportResolution::ActionTaken,
        "moderation action recorded",
        StaffAuditRecord {
            actor_account_id: admin.account.id,
            target_account_id: None,
            capability: StaffCapability::ReportsResolve,
            action: "reports.resolve".into(),
            reason: "review completed".into(),
            details: serde_json::json!({"report_id": report_id}),
        },
        1_202,
    )
    .unwrap();
    let resolution: (String, String, String, i64) = connection
        .query_row(
            "SELECT status, resolution, resolution_reason, resolved_by_account_id
             FROM social_reports WHERE id = ?1",
            [report_id],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
        )
        .unwrap();
    assert_eq!(
        resolution,
        (
            "resolved".into(),
            "action_taken".into(),
            "moderation action recorded".into(),
            admin.account.id.0,
        )
    );

    connection
        .execute(
            "INSERT INTO staff_profiles
                 (account_id, profile_key, lore_title, assigned_by_account_id,
                  created_at, updated_at)
             VALUES (?1, 'builder', NULL, ?2, 1202, 1202)",
            params![reporter.account.id.0, admin.account.id.0],
        )
        .unwrap();
    connection
        .execute(
            "INSERT INTO staff_capability_assignments
                 (account_id, capability, assigned_by_account_id, created_at)
             VALUES (?1, 'staff.where', ?2, 1202)",
            params![reporter.account.id.0, admin.account.id.0],
        )
        .unwrap();

    set_staff_profile(
        &mut connection,
        reporter.account.id,
        Some(StaffProfileKey::Moderator),
        Some("Custodio de la plaza"),
        StaffAuditRecord {
            actor_account_id: admin.account.id,
            target_account_id: Some(reporter.account.id),
            capability: StaffCapability::StaffAssign,
            action: "staff.profile.set".into(),
            reason: "approved moderator assignment".into(),
            details: serde_json::json!({"profile": "moderator"}),
        },
        1_203,
    )
    .unwrap();
    let reporter_access = staff_access(&connection, reporter_character.id).unwrap();
    assert_eq!(reporter_access.profile, Some(StaffProfileKey::Moderator));
    assert_eq!(
        reporter_access.capabilities,
        StaffProfileKey::Moderator
            .default_capabilities()
            .iter()
            .copied()
            .collect()
    );

    assert!(matches!(
        set_staff_profile(
            &mut connection,
            target.account.id,
            Some(StaffProfileKey::Implementor),
            None,
            StaffAuditRecord {
                actor_account_id: admin.account.id,
                target_account_id: Some(target.account.id),
                capability: StaffCapability::StaffAssign,
                action: "staff.profile.set".into(),
                reason: "attempt equal profile".into(),
                details: serde_json::json!({}),
            },
            1_204,
        ),
        Err(StoreError::StaffHierarchy)
    ));
    assert!(matches!(
        set_staff_capability(
            &mut connection,
            target.account.id,
            StaffCapability::StaffWhere,
            true,
            StaffAuditRecord {
                actor_account_id: reporter.account.id,
                target_account_id: Some(target.account.id),
                capability: StaffCapability::StaffAssign,
                action: "staff.capability.set".into(),
                reason: "attempt without assignment authority".into(),
                details: serde_json::json!({"capability": "staff.where"}),
            },
            1_205,
        ),
        Err(StoreError::StaffForbidden)
    ));

    let audit = staff_audit_entries(
        &mut connection,
        StaffAuditRecord {
            actor_account_id: admin.account.id,
            target_account_id: None,
            capability: StaffCapability::StaffAudit,
            action: "staff.audit.list".into(),
            reason: "bounded audit review".into(),
            details: serde_json::json!({}),
        },
        1_206,
    )
    .unwrap();
    assert!(!audit.is_empty());
    assert!(audit.len() <= usize::try_from(STAFF_LIST_LIMIT).unwrap());
    assert!(audit.iter().all(|entry| entry.sequence > 0));
}

#[tokio::test]
async fn closed_channel_subscriptions_are_idempotent_and_survive_restart() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("channel-subscriptions.db");
    let store = SqliteCharacterStore::open(&path).await.unwrap();
    let account = store
        .register_account("Channel Keeper", "correct horse battery staple")
        .await
        .unwrap();
    let character = store
        .load_or_create_character(account.account.id, "Aster", "room:start")
        .await
        .unwrap();

    assert!(
        store
            .social_channel_subscriptions(character.id)
            .await
            .unwrap()
            .is_empty()
    );
    for channel in [
        SocialChannelName::Staff,
        SocialChannelName::General,
        SocialChannelName::Newbie,
        SocialChannelName::General,
    ] {
        store
            .set_social_channel_subscription(character.id, channel, true)
            .await
            .unwrap();
    }
    assert_eq!(
        store
            .social_channel_subscriptions(character.id)
            .await
            .unwrap(),
        vec![
            SocialChannelName::General,
            SocialChannelName::Newbie,
            SocialChannelName::Staff,
        ]
    );
    assert_eq!(
        store
            .social_channel_subscribers(SocialChannelName::General)
            .await
            .unwrap(),
        vec![character.id]
    );
    for _ in 0..2 {
        store
            .set_social_channel_subscription(character.id, SocialChannelName::Newbie, false)
            .await
            .unwrap();
    }
    assert!(matches!(
        store
            .social_channel_subscriptions(CharacterId(i64::MAX))
            .await,
        Err(StoreError::SocialTargetNotFound)
    ));

    store.shutdown().await.unwrap();
    drop(store);
    let reopened = SqliteCharacterStore::open(&path).await.unwrap();
    assert_eq!(
        reopened
            .social_channel_subscriptions(character.id)
            .await
            .unwrap(),
        vec![SocialChannelName::General, SocialChannelName::Staff]
    );
    assert!(
        reopened
            .social_channel_subscribers(SocialChannelName::Newbie)
            .await
            .unwrap()
            .is_empty()
    );
}

fn mail_fixture(connection: &Connection) -> (CharacterId, CharacterId, CharacterId) {
    connection
        .execute_batch(
            "INSERT INTO accounts (id, name) VALUES
                 (101, 'Mail Sender'), (102, 'Mail Recipient');
             INSERT INTO characters (id, account_id, name, room_id) VALUES
                 (1001, 101, 'Aster', 'room:start'),
                 (1002, 102, 'Bryn', 'room:start'),
                 (1003, 101, 'AsterAlt', 'room:start');",
        )
        .unwrap();
    (CharacterId(1001), CharacterId(1002), CharacterId(1003))
}

fn mail_id(sequence: u128) -> String {
    format!("m{sequence:032x}")
}

fn board_id(sequence: u128) -> String {
    format!("b{sequence:032x}")
}

fn seed_board_posts(
    connection: &mut Connection,
    author: CharacterId,
    board: BoardKey,
    first_sequence: u128,
    count: u32,
    subject: &str,
    body: &str,
) {
    let created_at = 100;
    let transaction = connection.transaction().unwrap();
    for offset in 0..count {
        transaction
            .execute(
                "INSERT INTO social_board_posts
                     (public_id, board_key, author_character_id, author_name,
                      subject, body, created_at, expires_at)
                 VALUES (?1, ?2, ?3, 'Seed Author', ?4, ?5, ?6, ?7)",
                params![
                    board_id(first_sequence + u128::from(offset)),
                    board.as_str(),
                    author.0,
                    subject,
                    body,
                    created_at,
                    created_at + BOARD_RETENTION_SECONDS
                ],
            )
            .unwrap();
    }
    transaction.commit().unwrap();
}

#[test]
fn board_publish_list_read_delete_and_quota_are_account_bounded() {
    let mut connection = Connection::open_in_memory().unwrap();
    initialise(&mut connection).unwrap();
    let (author, reader, _) = mail_fixture(&connection);
    let entry = publish_board_post(
        &mut connection,
        author,
        BoardKey::General,
        &board_id(1),
        "Lantern meeting",
        "Meet beneath the arch.",
        1_000,
    )
    .unwrap();
    assert!(entry.summary.can_delete);
    assert!(!entry.summary.can_report);
    assert_eq!(entry.summary.expires_at, 1_000 + BOARD_RETENTION_SECONDS);

    let reader_page = list_board_posts(&connection, reader, BoardKey::General, 1, 1_001).unwrap();
    assert_eq!(reader_page.total_posts, 1);
    assert!(reader_page.posts[0].can_report);
    assert!(!reader_page.posts[0].can_delete);
    assert!(
        list_board_posts(&connection, reader, BoardKey::General, 100, 1_001)
            .unwrap()
            .posts
            .is_empty()
    );
    assert_eq!(
        read_board_post(&connection, reader, &board_id(1), 1_001)
            .unwrap()
            .body,
        "Meet beneath the arch."
    );
    assert!(matches!(
        delete_board_post(&mut connection, reader, &board_id(1), 1_002),
        Err(StoreError::BoardPostUnavailable)
    ));
    delete_board_post(&mut connection, author, &board_id(1), 1_002).unwrap();

    for sequence in 0_u128..u128::from(BOARD_ACCOUNT_CAPACITY) {
        connection
            .execute(
                "INSERT INTO social_board_posts
                     (public_id, board_key, author_character_id, author_name,
                      subject, body, created_at, expires_at)
                 VALUES (?1, 'general', ?2, 'Aster', 'Cap', 'Body', ?3, ?4)",
                params![
                    board_id(100 + sequence),
                    author.0,
                    1_003,
                    1_003 + BOARD_RETENTION_SECONDS
                ],
            )
            .unwrap();
    }
    assert!(matches!(
        publish_board_post(
            &mut connection,
            author,
            BoardKey::Newbie,
            &board_id(1_000),
            "Account full",
            "A valid attempt still consumes quota.",
            1_003,
        ),
        Err(StoreError::BoardCapacity)
    ));
    assert_eq!(
        connection
            .query_row(
                "SELECT hour_count FROM social_board_usage WHERE account_id = 101",
                [],
                |row| row.get::<_, i64>(0),
            )
            .unwrap(),
        2
    );
    connection
        .execute("DELETE FROM social_board_posts", [])
        .unwrap();

    connection
        .execute(
            "UPDATE social_board_usage
             SET hour_count = 5, day_count = 5, updated_at = 1002
             WHERE account_id = 101",
            [],
        )
        .unwrap();
    assert!(matches!(
        publish_board_post(
            &mut connection,
            author,
            BoardKey::Newbie,
            &board_id(2),
            "Rate limited",
            "This attempt is rejected.",
            1_003,
        ),
        Err(StoreError::BoardRateLimited)
    ));
}

#[test]
fn board_reporting_copies_evidence_once_and_purge_is_bounded() {
    let mut connection = Connection::open_in_memory().unwrap();
    initialise(&mut connection).unwrap();
    let (author, reader, same_account_alt) = mail_fixture(&connection);
    publish_board_post(
        &mut connection,
        author,
        BoardKey::Newbie,
        &board_id(10),
        "Reported subject",
        "Evidence body",
        5_000,
    )
    .unwrap();
    assert!(matches!(
        report_board_post(
            &mut connection,
            same_account_alt,
            &board_id(10),
            "Same account",
            5_001,
        ),
        Err(StoreError::BoardPostUnavailable)
    ));
    let report_id = report_board_post(
        &mut connection,
        reader,
        &board_id(10),
        "Public abuse",
        5_002,
    )
    .unwrap();
    assert_eq!(
        connection
            .query_row(
                "SELECT report_kind FROM social_reports WHERE id = ?1",
                [report_id],
                |row| row.get::<_, String>(0),
            )
            .unwrap(),
        "board"
    );
    delete_board_post(&mut connection, author, &board_id(10), 5_003).unwrap();
    assert_eq!(
        connection
            .query_row(
                "SELECT body FROM social_board_report_evidence WHERE report_id = ?1",
                [report_id],
                |row| row.get::<_, String>(0),
            )
            .unwrap(),
        "Evidence body"
    );

    for sequence in 0_u128..300 {
        let created_at = 10_000 + i64::try_from(sequence).unwrap();
        connection
            .execute(
                "INSERT INTO social_board_posts
                     (public_id, board_key, author_character_id, author_name,
                      subject, body, created_at, expires_at)
                 VALUES (?1, 'general', ?2, 'Aster', 'Old', 'Expired', ?3, ?4)",
                params![
                    board_id(1_000 + sequence),
                    author.0,
                    created_at,
                    created_at + BOARD_RETENTION_SECONDS
                ],
            )
            .unwrap();
    }
    let expiry = 10_299 + BOARD_RETENTION_SECONDS;
    assert!(
        list_board_posts(&connection, reader, BoardKey::General, 1, expiry)
            .unwrap()
            .posts
            .is_empty()
    );
    assert_eq!(purge_expired_boards(&mut connection, expiry).unwrap(), 256);
    assert_eq!(purge_expired_boards(&mut connection, expiry).unwrap(), 45);
    assert_eq!(purge_expired_boards(&mut connection, expiry).unwrap(), 0);
}

#[test]
fn board_count_caps_accept_the_exact_board_and_global_boundaries() {
    let mut connection = Connection::open_in_memory().unwrap();
    initialise(&mut connection).unwrap();
    let (publisher, seeded_author, _) = mail_fixture(&connection);
    let now = 20_000;

    seed_board_posts(
        &mut connection,
        seeded_author,
        BoardKey::General,
        10_000,
        BOARD_CAPACITY_PER_BOARD - 1,
        "S",
        "B",
    );
    publish_board_post(
        &mut connection,
        publisher,
        BoardKey::General,
        &board_id(11_000),
        "S",
        "B",
        now,
    )
    .unwrap();
    assert_eq!(
        connection
            .query_row(
                "SELECT COUNT(*) FROM social_board_posts WHERE board_key = 'general'",
                [],
                |row| row.get::<_, i64>(0),
            )
            .unwrap(),
        i64::from(BOARD_CAPACITY_PER_BOARD)
    );
    assert!(matches!(
        publish_board_post(
            &mut connection,
            publisher,
            BoardKey::General,
            &board_id(11_001),
            "S",
            "B",
            now,
        ),
        Err(StoreError::BoardCapacity)
    ));
    assert_eq!(
        connection
            .query_row("SELECT COUNT(*) FROM social_board_posts", [], |row| {
                row.get::<_, i64>(0)
            })
            .unwrap(),
        i64::from(BOARD_CAPACITY_PER_BOARD)
    );
    assert_eq!(
        connection
            .query_row(
                "SELECT hour_count FROM social_board_usage WHERE account_id = 101",
                [],
                |row| row.get::<_, i64>(0),
            )
            .unwrap(),
        2,
        "a valid capacity rejection consumes exactly one quota unit"
    );

    connection
        .execute_batch("DELETE FROM social_board_posts; DELETE FROM social_board_usage;")
        .unwrap();
    seed_board_posts(
        &mut connection,
        seeded_author,
        BoardKey::General,
        20_000,
        BOARD_CAPACITY_PER_BOARD - 1,
        "S",
        "B",
    );
    seed_board_posts(
        &mut connection,
        seeded_author,
        BoardKey::Newbie,
        21_000,
        BOARD_CAPACITY_PER_BOARD,
        "S",
        "B",
    );
    publish_board_post(
        &mut connection,
        publisher,
        BoardKey::General,
        &board_id(22_000),
        "S",
        "B",
        now,
    )
    .unwrap();
    assert_eq!(
        connection
            .query_row("SELECT COUNT(*) FROM social_board_posts", [], |row| {
                row.get::<_, i64>(0)
            })
            .unwrap(),
        BOARD_GLOBAL_CAPACITY
    );
    assert!(matches!(
        publish_board_post(
            &mut connection,
            publisher,
            BoardKey::Newbie,
            &board_id(22_001),
            "S",
            "B",
            now,
        ),
        Err(StoreError::BoardCapacity)
    ));
    assert_eq!(
        connection
            .query_row("SELECT COUNT(*) FROM social_board_posts", [], |row| {
                row.get::<_, i64>(0)
            })
            .unwrap(),
        BOARD_GLOBAL_CAPACITY
    );
}

#[test]
fn board_byte_caps_accept_exact_boundaries_and_reject_one_more_post() {
    let mut connection = Connection::open_in_memory().unwrap();
    initialise(&mut connection).unwrap();
    let (publisher, seeded_author, _) = mail_fixture(&connection);
    let now = 30_000;
    let maximum_subject = "S".repeat(tiamud_core::MAX_BOARD_SUBJECT_BYTES);
    let maximum_body = "B".repeat(tiamud_core::MAX_BOARD_BODY_BYTES);
    let remainder_body = "R".repeat(4_093);

    for (board, first_sequence) in [
        (BoardKey::General, 30_000_u128),
        (BoardKey::Newbie, 40_000_u128),
    ] {
        seed_board_posts(
            &mut connection,
            seeded_author,
            board,
            first_sequence,
            992,
            &maximum_subject,
            &maximum_body,
        );
        seed_board_posts(
            &mut connection,
            seeded_author,
            board,
            first_sequence + 992,
            1,
            "S",
            &remainder_body,
        );
    }

    publish_board_post(
        &mut connection,
        publisher,
        BoardKey::General,
        &board_id(50_000),
        "S",
        "B",
        now,
    )
    .unwrap();
    publish_board_post(
        &mut connection,
        publisher,
        BoardKey::Newbie,
        &board_id(50_001),
        "S",
        "B",
        now,
    )
    .unwrap();
    let (general_bytes, newbie_bytes, global_bytes) = connection
        .query_row(
            "SELECT
                 SUM(CASE WHEN board_key = 'general'
                          THEN length(CAST(subject AS BLOB)) + length(CAST(body AS BLOB))
                          ELSE 0 END),
                 SUM(CASE WHEN board_key = 'newbie'
                          THEN length(CAST(subject AS BLOB)) + length(CAST(body AS BLOB))
                          ELSE 0 END),
                 SUM(length(CAST(subject AS BLOB)) + length(CAST(body AS BLOB)))
             FROM social_board_posts",
            [],
            |row| {
                Ok((
                    row.get::<_, i64>(0)?,
                    row.get::<_, i64>(1)?,
                    row.get::<_, i64>(2)?,
                ))
            },
        )
        .unwrap();
    assert_eq!(general_bytes, BOARD_BYTE_CAPACITY_PER_BOARD);
    assert_eq!(newbie_bytes, BOARD_BYTE_CAPACITY_PER_BOARD);
    assert_eq!(global_bytes, BOARD_GLOBAL_BYTE_CAPACITY);
    let count_before = connection
        .query_row("SELECT COUNT(*) FROM social_board_posts", [], |row| {
            row.get::<_, i64>(0)
        })
        .unwrap();
    assert!(matches!(
        publish_board_post(
            &mut connection,
            publisher,
            BoardKey::General,
            &board_id(50_002),
            "S",
            "B",
            now,
        ),
        Err(StoreError::BoardCapacity)
    ));
    assert_eq!(
        connection
            .query_row("SELECT COUNT(*) FROM social_board_posts", [], |row| {
                row.get::<_, i64>(0)
            })
            .unwrap(),
        count_before
    );
    assert_eq!(
        connection
            .query_row(
                "SELECT hour_count FROM social_board_usage WHERE account_id = 101",
                [],
                |row| row.get::<_, i64>(0),
            )
            .unwrap(),
        3
    );
}

#[test]
fn board_daily_quota_stops_the_twenty_first_attempt_without_mutation() {
    let mut connection = Connection::open_in_memory().unwrap();
    initialise(&mut connection).unwrap();
    let (publisher, _, _) = mail_fixture(&connection);
    let day_started_at = 40_000;
    for sequence in 0_u128..20 {
        let hour = i64::try_from(sequence / 5).unwrap();
        publish_board_post(
            &mut connection,
            publisher,
            BoardKey::General,
            &board_id(60_000 + sequence),
            "S",
            "B",
            day_started_at + hour * MAIL_HOUR_SECONDS,
        )
        .unwrap();
    }
    let rejected_at = day_started_at + 4 * MAIL_HOUR_SECONDS;
    assert!(matches!(
        publish_board_post(
            &mut connection,
            publisher,
            BoardKey::General,
            &board_id(60_020),
            "S",
            "B",
            rejected_at,
        ),
        Err(StoreError::BoardRateLimited)
    ));
    assert_eq!(
        connection
            .query_row("SELECT COUNT(*) FROM social_board_posts", [], |row| {
                row.get::<_, i64>(0)
            })
            .unwrap(),
        BOARD_DAILY_CAPACITY
    );
    assert_eq!(
        connection
            .query_row(
                "SELECT day_count FROM social_board_usage WHERE account_id = 101",
                [],
                |row| row.get::<_, i64>(0),
            )
            .unwrap(),
        BOARD_DAILY_CAPACITY,
        "a rate-limited attempt must not increment durable usage"
    );
    assert_eq!(
        connection
            .query_row(
                "SELECT COUNT(*) FROM social_board_posts WHERE public_id = ?1",
                [board_id(60_020)],
                |row| row.get::<_, i64>(0),
            )
            .unwrap(),
        0
    );
}

#[test]
fn board_report_caps_reject_without_linking_or_creating_evidence() {
    let mut connection = Connection::open_in_memory().unwrap();
    initialise(&mut connection).unwrap();
    let (author, reporter, _) = mail_fixture(&connection);
    let now = 50_000;
    seed_board_posts(
        &mut connection,
        author,
        BoardKey::General,
        70_000,
        9,
        "Reported",
        "Evidence",
    );
    for sequence in 0_u128..8 {
        report_board_post(
            &mut connection,
            reporter,
            &board_id(70_000 + sequence),
            "Open report cap",
            now + 1,
        )
        .unwrap();
    }
    assert!(matches!(
        report_board_post(
            &mut connection,
            reporter,
            &board_id(70_008),
            "One report too many",
            now + 1,
        ),
        Err(StoreError::BoardReportLimit)
    ));
    assert_eq!(
        connection
            .query_row(
                "SELECT COUNT(*) FROM social_reports
                 WHERE report_kind = 'board' AND status = 'open'",
                [],
                |row| row.get::<_, i64>(0),
            )
            .unwrap(),
        BOARD_OPEN_REPORT_CAPACITY_PER_ACCOUNT
    );
    assert_eq!(
        connection
            .query_row(
                "SELECT COUNT(*) FROM social_board_report_evidence",
                [],
                |row| row.get::<_, i64>(0),
            )
            .unwrap(),
        BOARD_OPEN_REPORT_CAPACITY_PER_ACCOUNT
    );
    assert_eq!(
        connection
            .query_row(
                "SELECT COUNT(*) FROM social_board_posts
                 WHERE public_id = ?1 AND report_id IS NULL",
                [board_id(70_008)],
                |row| row.get::<_, i64>(0),
            )
            .unwrap(),
        1
    );

    let mut full_evidence = Connection::open_in_memory().unwrap();
    initialise(&mut full_evidence).unwrap();
    let (author, reporter, _) = mail_fixture(&full_evidence);
    full_evidence
        .execute_batch(
            "WITH RECURSIVE sequence(value) AS (
                 VALUES(1) UNION ALL SELECT value + 1 FROM sequence WHERE value < 10000
             )
             INSERT INTO social_reports
                 (reporter_character_id, target_character_id, report_kind, reason, created_at)
             SELECT 1001, 1002, 'board', 'seed evidence capacity', value
             FROM sequence;
             INSERT INTO social_board_report_evidence
                 (report_id, board_key, post_public_id, author_name,
                  subject, body, posted_at, created_at, expires_at)
             SELECT id, 'general', 'bffffffffffffffffffffffffffffffff', 'Seed Author',
                    'Seed', 'Evidence', 0, 100, 7776100
             FROM social_reports WHERE reason = 'seed evidence capacity';",
        )
        .unwrap();
    seed_board_posts(
        &mut full_evidence,
        author,
        BoardKey::General,
        80_000,
        1,
        "Reported",
        "Evidence",
    );
    assert!(matches!(
        report_board_post(
            &mut full_evidence,
            reporter,
            &board_id(80_000),
            "Global evidence full",
            1_001,
        ),
        Err(StoreError::BoardEvidenceLimit)
    ));
    assert_eq!(
        full_evidence
            .query_row(
                "SELECT COUNT(*) FROM social_board_report_evidence",
                [],
                |row| row.get::<_, i64>(0),
            )
            .unwrap(),
        BOARD_REPORT_EVIDENCE_GLOBAL_CAPACITY
    );
    assert_eq!(
        full_evidence
            .query_row(
                "SELECT COUNT(*) FROM social_reports WHERE report_kind = 'board'",
                [],
                |row| row.get::<_, i64>(0),
            )
            .unwrap(),
        BOARD_REPORT_EVIDENCE_GLOBAL_CAPACITY
    );
    assert_eq!(
        full_evidence
            .query_row(
                "SELECT COUNT(*) FROM social_board_posts
                 WHERE public_id = ?1 AND report_id IS NULL",
                [board_id(80_000)],
                |row| row.get::<_, i64>(0),
            )
            .unwrap(),
        1
    );
}

#[test]
fn mail_send_list_read_delete_and_private_suppression_are_atomic() {
    let mut connection = Connection::open_in_memory().unwrap();
    initialise(&mut connection).unwrap();
    let (sender, recipient, alt) = mail_fixture(&connection);

    assert_eq!(
        send_mail(
            &mut connection,
            sender,
            "Bryn",
            &mail_id(1),
            "Meeting",
            "Meet under the lantern.",
            1_000,
        )
        .unwrap(),
        MailSendResult::Stored
    );
    let page = mailbox_page(&connection, recipient, 1, 1_001).unwrap();
    assert_eq!(page.total_pages, 1);
    assert_eq!(page.total_messages, 1);
    assert_eq!(page.unread_count, 1);
    assert_eq!(page.capacity, MAILBOX_CAPACITY);
    assert_eq!(page.messages[0].public_id, mail_id(1));
    assert!(
        mailbox_page(&connection, sender, 1, 1_001)
            .unwrap()
            .messages
            .is_empty()
    );
    assert!(matches!(
        read_mail(&mut connection, alt, &mail_id(1), 1_001),
        Err(StoreError::MailNotFound)
    ));
    let read = read_mail(&mut connection, recipient, &mail_id(1), 1_001).unwrap();
    assert!(read.summary.read);
    assert_eq!(read.body, "Meet under the lantern.");
    assert_eq!(
        mailbox_page(&connection, recipient, 1, 1_002)
            .unwrap()
            .unread_count,
        0
    );

    assert_eq!(
        send_mail(
            &mut connection,
            sender,
            "Nobody",
            &mail_id(2),
            "Unknown",
            "Still consumes quota.",
            1_003,
        )
        .unwrap(),
        MailSendResult::Unavailable
    );
    assert_eq!(
        send_mail(
            &mut connection,
            sender,
            "Aster",
            &mail_id(3),
            "Self",
            "Still consumes quota.",
            1_004,
        )
        .unwrap(),
        MailSendResult::Unavailable
    );
    connection
        .execute(
            "INSERT INTO social_blocks
                 (owner_character_id, target_character_id, created_at)
             VALUES (?1, ?2, 1005)",
            params![recipient.0, sender.0],
        )
        .unwrap();
    assert_eq!(
        send_mail(
            &mut connection,
            sender,
            "Bryn",
            &mail_id(4),
            "Blocked",
            "Suppressed without disclosure.",
            1_005,
        )
        .unwrap(),
        MailSendResult::Suppressed
    );
    assert_eq!(
        connection
            .query_row(
                "SELECT hour_count FROM social_mail_usage WHERE account_id = 101",
                [],
                |row| row.get::<_, i64>(0),
            )
            .unwrap(),
        4
    );
    connection
        .execute(
            "DELETE FROM social_blocks
             WHERE owner_character_id = ?1 AND target_character_id = ?2",
            params![recipient.0, sender.0],
        )
        .unwrap();
    assert_eq!(
        send_mail(
            &mut connection,
            sender,
            "AsterAlt",
            &mail_id(5),
            "For my alt",
            "Same-account alts are allowed.",
            1_006,
        )
        .unwrap(),
        MailSendResult::Stored
    );
    assert!(matches!(
        delete_mail(&mut connection, alt, &mail_id(1), 1_007),
        Err(StoreError::MailNotFound)
    ));
    delete_mail(&mut connection, recipient, &mail_id(1), 1_007).unwrap();
    assert!(
        mailbox_page(&connection, recipient, 1, 1_008)
            .unwrap()
            .messages
            .is_empty()
    );
}

#[test]
fn mail_quota_validation_expiry_and_purge_remain_bounded() {
    let mut connection = Connection::open_in_memory().unwrap();
    initialise(&mut connection).unwrap();
    let (sender, recipient, _) = mail_fixture(&connection);
    connection
        .execute(
            "INSERT INTO social_mail_usage
                 (account_id, hour_started_at, hour_count,
                  day_started_at, day_count, updated_at)
             VALUES (101, 1000, 20, 1000, 20, 1000)",
            [],
        )
        .unwrap();
    assert!(matches!(
        send_mail(
            &mut connection,
            sender,
            "Nobody",
            &mail_id(10),
            "Rate limited",
            "No target lookup changes the result.",
            1_001,
        ),
        Err(StoreError::MailRateLimited)
    ));
    assert!(matches!(
        send_mail(
            &mut connection,
            sender,
            "Bryn",
            "mNOT-HEX",
            "Invalid",
            "Does not consume quota.",
            1_002,
        ),
        Err(StoreError::InvalidMailInput)
    ));
    assert!(matches!(
        send_mail(
            &mut connection,
            sender,
            "Bryn",
            &mail_id(11),
            " padded subject",
            "No silent trimming.",
            1_002,
        ),
        Err(StoreError::InvalidMailInput)
    ));
    assert_eq!(
        connection
            .query_row(
                "SELECT hour_count FROM social_mail_usage WHERE account_id = 101",
                [],
                |row| row.get::<_, i64>(0),
            )
            .unwrap(),
        20
    );

    connection
        .execute("DELETE FROM social_mail_usage WHERE account_id = 101", [])
        .unwrap();
    assert_eq!(
        send_mail(
            &mut connection,
            sender,
            "bryn",
            &mail_id(13),
            "Exact name",
            "Mail targets preserve the same case rules in both backends.",
            1_050,
        )
        .unwrap(),
        MailSendResult::Unavailable
    );
    assert_eq!(
        mailbox_page(&connection, recipient, 1, 1_050)
            .unwrap()
            .total_messages,
        0
    );
    connection
        .execute(
            "UPDATE characters SET name = 'A' || char(8238) || 'B' WHERE id = ?1",
            [sender.0],
        )
        .unwrap();
    assert_eq!(
        send_mail(
            &mut connection,
            sender,
            "Bryn",
            &mail_id(12),
            "Legacy sender",
            "A legacy bidi identity cannot poison the recipient inbox.",
            1_100,
        )
        .unwrap(),
        MailSendResult::Suppressed
    );
    assert_eq!(
        mailbox_page(&connection, recipient, 1, 1_100)
            .unwrap()
            .total_messages,
        0
    );
    for sequence in 0_u128..300 {
        let created_at = 2_000_i64 + i64::try_from(sequence).unwrap();
        connection
            .execute(
                "INSERT INTO social_mail_messages
                     (public_id, sender_character_id, sender_name,
                      recipient_character_id, subject, body, created_at, expires_at)
                 VALUES (?1, ?2, 'Aster', ?3, 'Old', 'Expired', ?4, ?5)",
                params![
                    mail_id(1_000 + sequence),
                    sender.0,
                    recipient.0,
                    created_at,
                    created_at + MAIL_RETENTION_SECONDS
                ],
            )
            .unwrap();
    }
    let expiry = 2_299 + MAIL_RETENTION_SECONDS;
    assert!(
        mailbox_page(&connection, recipient, 1, expiry)
            .unwrap()
            .messages
            .is_empty()
    );
    assert_eq!(purge_expired_mail(&mut connection, expiry).unwrap(), 256);
    assert_eq!(purge_expired_mail(&mut connection, expiry).unwrap(), 44);
    assert_eq!(purge_expired_mail(&mut connection, expiry).unwrap(), 0);
}

#[test]
fn mail_count_byte_and_page_caps_suppress_without_overfilling() {
    let mut connection = Connection::open_in_memory().unwrap();
    initialise(&mut connection).unwrap();
    let (sender, recipient, _) = mail_fixture(&connection);
    for sequence in 0_u128..u128::from(MAILBOX_CAPACITY) {
        connection
            .execute(
                "INSERT INTO social_mail_messages
                     (public_id, sender_character_id, sender_name,
                      recipient_character_id, subject, body, created_at, expires_at)
                 VALUES (?1, ?2, 'Aster', ?3, 'S', 'B', ?4, ?5)",
                params![
                    mail_id(2_000 + sequence),
                    sender.0,
                    recipient.0,
                    10_000 + i64::try_from(sequence).unwrap(),
                    10_000 + i64::try_from(sequence).unwrap() + MAIL_RETENTION_SECONDS
                ],
            )
            .unwrap();
    }
    let page = mailbox_page(&connection, recipient, MAIL_MAX_PAGES, 11_000).unwrap();
    assert_eq!(page.total_messages, MAILBOX_CAPACITY);
    assert_eq!(page.total_pages, MAIL_MAX_PAGES);
    assert_eq!(page.messages.len(), 4);
    assert_eq!(
        send_mail(
            &mut connection,
            sender,
            "Bryn",
            &mail_id(3_000),
            "Full",
            "The full mailbox suppresses this message.",
            11_001,
        )
        .unwrap(),
        MailSendResult::Suppressed
    );
    assert_eq!(
        mailbox_page(&connection, recipient, 1, 11_002)
            .unwrap()
            .total_messages,
        MAILBOX_CAPACITY
    );

    connection
        .execute("DELETE FROM social_mail_messages", [])
        .unwrap();
    let large_subject = "s".repeat(MAX_MAIL_SUBJECT_BYTES);
    let large_body = "b".repeat(MAX_MAIL_BODY_BYTES);
    for sequence in 0_u128..60 {
        connection
            .execute(
                "INSERT INTO social_mail_messages
                     (public_id, sender_character_id, sender_name,
                      recipient_character_id, subject, body, created_at, expires_at)
                 VALUES (?1, ?2, 'Aster', ?3, ?4, ?5, ?6, ?7)",
                params![
                    mail_id(4_000 + sequence),
                    sender.0,
                    recipient.0,
                    large_subject,
                    large_body,
                    20_000 + i64::try_from(sequence).unwrap(),
                    20_000 + i64::try_from(sequence).unwrap() + MAIL_RETENTION_SECONDS
                ],
            )
            .unwrap();
    }
    assert_eq!(
        send_mail(
            &mut connection,
            sender,
            "Bryn",
            &mail_id(5_000),
            &large_subject,
            &large_body,
            21_000,
        )
        .unwrap(),
        MailSendResult::Suppressed
    );
    assert_eq!(
        mailbox_page(&connection, recipient, 1, 21_001)
            .unwrap()
            .total_messages,
        60
    );
}

#[test]
fn mail_report_copies_bounded_evidence_once_and_survives_hard_delete() {
    let mut connection = Connection::open_in_memory().unwrap();
    initialise(&mut connection).unwrap();
    let (sender, recipient, _) = mail_fixture(&connection);
    assert_eq!(
        send_mail(
            &mut connection,
            sender,
            "Bryn",
            &mail_id(20),
            "Abuse evidence",
            "A body retained only because the recipient reports it.",
            5_000,
        )
        .unwrap(),
        MailSendResult::Stored
    );
    let report_id = report_mail(
        &mut connection,
        recipient,
        &mail_id(20),
        "Please review this message",
        5_001,
    )
    .unwrap();
    assert!(report_id > 0);
    assert_eq!(
        connection
            .query_row(
                "SELECT report_kind FROM social_reports WHERE id = ?1",
                [report_id],
                |row| row.get::<_, String>(0),
            )
            .unwrap(),
        "mail"
    );
    assert!(matches!(
        report_mail(&mut connection, recipient, &mail_id(20), "Duplicate", 5_002,),
        Err(StoreError::MailAlreadyReported)
    ));
    assert_eq!(
        connection
            .query_row("SELECT COUNT(*) FROM social_reports", [], |row| {
                row.get::<_, i64>(0)
            })
            .unwrap(),
        1,
        "a failed duplicate report rolls back every report row"
    );
    let evidence: (String, String, String) = connection
        .query_row(
            "SELECT sender_name, subject, body
             FROM social_mail_report_evidence WHERE report_id = ?1",
            [report_id],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .unwrap();
    assert_eq!(evidence.0, "Aster");
    assert_eq!(evidence.1, "Abuse evidence");
    assert_eq!(
        evidence.2,
        "A body retained only because the recipient reports it."
    );
    let audit_details: String = connection
        .query_row(
            "SELECT details_json FROM moderation_audit WHERE action = 'mail_report'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert!(!audit_details.contains("body retained"));

    delete_mail(&mut connection, recipient, &mail_id(20), 5_003).unwrap();
    assert_eq!(
        connection
            .query_row(
                "SELECT COUNT(*) FROM social_mail_report_evidence WHERE report_id = ?1",
                [report_id],
                |row| row.get::<_, i64>(0),
            )
            .unwrap(),
        1
    );
    assert_eq!(
        purge_expired_mail(
            &mut connection,
            5_001 + MAIL_REPORT_EVIDENCE_RETENTION_SECONDS,
        )
        .unwrap(),
        1
    );
}

#[test]
fn open_mail_report_cap_survives_evidence_expiry_and_purge() {
    let mut connection = Connection::open_in_memory().unwrap();
    initialise(&mut connection).unwrap();
    let (sender, recipient, _) = mail_fixture(&connection);
    for sequence in 0..MAIL_OPEN_REPORT_CAPACITY_PER_ACCOUNT {
        let public_id = mail_id(100 + u128::try_from(sequence).unwrap());
        let sent_at = 30_000 + sequence * 2;
        assert_eq!(
            send_mail(
                &mut connection,
                sender,
                "Bryn",
                &public_id,
                "Capped report",
                "Evidence expires without erasing the durable report kind.",
                sent_at,
            )
            .unwrap(),
            MailSendResult::Stored
        );
        report_mail(
            &mut connection,
            recipient,
            &public_id,
            "Keep this open for the cap regression",
            sent_at + 1,
        )
        .unwrap();
    }

    let purge_at = 31_000 + MAIL_REPORT_EVIDENCE_RETENTION_SECONDS;
    assert_eq!(
        purge_expired_mail(&mut connection, purge_at).unwrap(),
        u64::try_from(MAIL_OPEN_REPORT_CAPACITY_PER_ACCOUNT * 2).unwrap()
    );
    assert_eq!(
        connection
            .query_row(
                "SELECT COUNT(*) FROM social_mail_report_evidence",
                [],
                |row| row.get::<_, i64>(0),
            )
            .unwrap(),
        0,
        "expired bodies must not remain solely to enforce the report cap"
    );
    assert_eq!(
        connection
            .query_row(
                "SELECT COUNT(*) FROM social_reports
                 WHERE report_kind = 'mail' AND status = 'open'",
                [],
                |row| row.get::<_, i64>(0),
            )
            .unwrap(),
        MAIL_OPEN_REPORT_CAPACITY_PER_ACCOUNT
    );

    let ninth_id = mail_id(200);
    assert_eq!(
        send_mail(
            &mut connection,
            sender,
            "Bryn",
            &ninth_id,
            "Ninth report",
            "This message can arrive, but its report must respect the open cap.",
            purge_at + 1,
        )
        .unwrap(),
        MailSendResult::Stored
    );
    assert!(matches!(
        report_mail(
            &mut connection,
            recipient,
            &ninth_id,
            "The durable open cap still applies",
            purge_at + 2,
        ),
        Err(StoreError::MailReportLimit)
    ));
}

#[test]
fn action_taken_resolution_removes_the_live_reported_mail() {
    let mut connection = Connection::open_in_memory().unwrap();
    initialise(&mut connection).unwrap();
    let admin = finish_register_account(&mut connection, "Mail Admin", "hash", 7_000).unwrap();
    let sender_account =
        finish_register_account(&mut connection, "Mail Author", "hash", 7_001).unwrap();
    let recipient_account =
        finish_register_account(&mut connection, "Mail Reader", "hash", 7_002).unwrap();
    let admin_character =
        load_or_create_character(&connection, admin.account.id, "Implementor", "room:start")
            .unwrap();
    let sender = load_or_create_character(
        &connection,
        sender_account.account.id,
        "Aster",
        "room:start",
    )
    .unwrap();
    let recipient = load_or_create_character(
        &connection,
        recipient_account.account.id,
        "Bryn",
        "room:start",
    )
    .unwrap();
    assert_eq!(
        send_mail(
            &mut connection,
            sender.id,
            "Bryn",
            &mail_id(30),
            "Reported",
            "Remove this message when action is taken.",
            7_010,
        )
        .unwrap(),
        MailSendResult::Stored
    );
    let report_id = report_mail(
        &mut connection,
        recipient.id,
        &mail_id(30),
        "Requires moderator action",
        7_011,
    )
    .unwrap();
    let reports = staff_reports(
        &mut connection,
        StaffAuditRecord {
            actor_account_id: admin.account.id,
            target_account_id: None,
            capability: StaffCapability::ReportsView,
            action: "reports.list".into(),
            reason: "review mail evidence".into(),
            details: serde_json::json!({}),
        },
        7_012,
    )
    .unwrap();
    let reported = reports
        .iter()
        .find(|report| report.id == report_id)
        .unwrap();
    assert_eq!(reported.mail_evidence.as_ref().unwrap().subject, "Reported");
    resolve_staff_report(
        &mut connection,
        report_id,
        StaffReportResolution::ActionTaken,
        "mail removed after review",
        StaffAuditRecord {
            actor_account_id: admin.account.id,
            target_account_id: None,
            capability: StaffCapability::ReportsResolve,
            action: "reports.resolve".into(),
            reason: "review completed".into(),
            details: serde_json::json!({"report_id": report_id}),
        },
        7_013,
    )
    .unwrap();
    assert!(matches!(
        read_mail(&mut connection, recipient.id, &mail_id(30), 7_014),
        Err(StoreError::MailNotFound)
    ));
    assert_eq!(
        connection
            .query_row(
                "SELECT COUNT(*) FROM social_mail_report_evidence WHERE report_id = ?1",
                [report_id],
                |row| row.get::<_, i64>(0),
            )
            .unwrap(),
        1,
        "resolution removes live mail but keeps bounded report evidence"
    );

    publish_board_post(
        &mut connection,
        sender.id,
        BoardKey::General,
        &board_id(31),
        "Reported board entry",
        "Remove this post when action is taken.",
        7_020,
    )
    .unwrap();
    let board_report_id = report_board_post(
        &mut connection,
        recipient.id,
        &board_id(31),
        "Requires moderator action",
        7_021,
    )
    .unwrap();
    let reports = staff_reports(
        &mut connection,
        StaffAuditRecord {
            actor_account_id: admin.account.id,
            target_account_id: None,
            capability: StaffCapability::ReportsView,
            action: "reports.list".into(),
            reason: "review board evidence".into(),
            details: serde_json::json!({}),
        },
        7_022,
    )
    .unwrap();
    assert_eq!(
        reports
            .iter()
            .find(|report| report.id == board_report_id)
            .unwrap()
            .board_evidence
            .as_ref()
            .unwrap()
            .subject,
        "Reported board entry"
    );
    resolve_staff_report(
        &mut connection,
        board_report_id,
        StaffReportResolution::ActionTaken,
        "board post removed after review",
        StaffAuditRecord {
            actor_account_id: admin.account.id,
            target_account_id: None,
            capability: StaffCapability::ReportsResolve,
            action: "reports.resolve".into(),
            reason: "review completed".into(),
            details: serde_json::json!({"report_id": board_report_id}),
        },
        7_023,
    )
    .unwrap();
    assert!(matches!(
        read_board_post(&connection, recipient.id, &board_id(31), 7_024),
        Err(StoreError::BoardPostUnavailable)
    ));
    assert_eq!(
        connection
            .query_row(
                "SELECT COUNT(*) FROM social_board_report_evidence WHERE report_id = ?1",
                [board_report_id],
                |row| row.get::<_, i64>(0),
            )
            .unwrap(),
        1,
        "resolution removes the live board post but keeps bounded evidence"
    );
    assert_eq!(
        staff_access(&connection, admin_character.id)
            .unwrap()
            .profile,
        Some(StaffProfileKey::Implementor)
    );
}

#[test]
fn temporary_and_permanent_bans_have_distinct_expiry_semantics() {
    let mut connection = Connection::open_in_memory().unwrap();
    initialise(&mut connection).unwrap();
    let admin = register_account(
        &mut connection,
        "Admin",
        "correct horse battery staple",
        1_000,
    )
    .unwrap();
    let player = register_account(
        &mut connection,
        "Player",
        "another correct horse battery",
        1_000,
    )
    .unwrap();
    let admin_character =
        load_or_create_character(&connection, admin.account.id, "Aster", "room:start").unwrap();
    let player_character =
        load_or_create_character(&connection, player.account.id, "Bryn", "room:start").unwrap();

    moderate_character(
        &mut connection,
        admin_character.id,
        player_character.id,
        ModerationAction::Ban {
            expires_at: Some(1_060),
            reason: "temporary review".into(),
        },
        1_000,
    )
    .unwrap();
    assert!(account_is_banned(&connection, player.account.id, 1_059).unwrap());
    assert!(!account_is_banned(&connection, player.account.id, 1_060).unwrap());

    moderate_character(
        &mut connection,
        admin_character.id,
        player_character.id,
        ModerationAction::Ban {
            expires_at: None,
            reason: "permanent ban".into(),
        },
        2_000,
    )
    .unwrap();
    assert!(account_is_banned(&connection, player.account.id, i64::MAX).unwrap());
}

#[test]
fn refuses_to_open_a_database_from_a_newer_schema() {
    let mut connection = Connection::open_in_memory().unwrap();
    let future = tiamud_persistence_sqlite::SCHEMA_VERSION + 1;
    connection
        .pragma_update(None, "user_version", future)
        .unwrap();

    assert!(matches!(
        initialise(&mut connection),
        Err(StoreError::UnsupportedSchemaVersion {
            found,
            supported: tiamud_persistence_sqlite::SCHEMA_VERSION
        }) if found == future
    ));
    assert_eq!(
        connection
            .query_row("PRAGMA user_version", [], |row| row.get::<_, i64>(0))
            .unwrap(),
        future
    );
}
