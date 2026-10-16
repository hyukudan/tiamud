use super::*;
use crate::persistence::BOARD_ACCOUNT_CAPACITY;
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeMap,
    env, process,
    time::{Duration, SystemTime, UNIX_EPOCH},
};
use tiamud_core::{
    AdventureRuntimeSnapshot, AreaDynamicCheckpoint, AreaDynamicCheckpointIdentity,
    AreaRuntimeResidence, Direction, ItemBinding, ItemInstanceId, ItemLocationSnapshot,
    ItemProvenance, LootDropId, RuntimeItemInstanceSnapshot, RuntimeItemInstanceSnapshotV3,
    RuntimeItemLocation, WORLD_RUNTIME_SNAPSHOT_VERSION, World, WorldRuntimeSnapshot,
    WorldRuntimeSnapshotV3,
};

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

#[test]
fn postgres_mail_contract_keeps_exact_input_and_storage_bounds() {
    assert_eq!(MAILBOX_CAPACITY, 64);
    assert_eq!(MAILBOX_BYTE_CAPACITY, 128 * 1024);
    assert_eq!(MAIL_PAGE_SIZE, 10);
    assert_eq!(MAIL_MAX_PAGES, 7);
    assert_eq!(MAIL_HOURLY_CAPACITY, 20);
    assert_eq!(MAIL_DAILY_CAPACITY, 50);
    assert_eq!(MAIL_GLOBAL_CAPACITY, 100_000);
    assert_eq!(MAIL_PURGE_LIMIT, 256);
    assert_eq!(
        normalise_mail_text("subject", MAX_MAIL_SUBJECT_BYTES).unwrap(),
        "subject"
    );
    assert!(matches!(
        normalise_mail_text(" subject ", MAX_MAIL_SUBJECT_BYTES),
        Err(StoreError::InvalidMailInput)
    ));
    assert!(matches!(
        normalise_mail_text("bad\nbody", MAX_MAIL_BODY_BYTES),
        Err(StoreError::InvalidMailInput)
    ));
    assert!(matches!(
        normalise_name("A\u{202e}B", "character"),
        Err(StoreError::InvalidName { kind: "character" })
    ));
    assert!(matches!(
        normalise_name("A|B", "character"),
        Err(StoreError::InvalidName { kind: "character" })
    ));
    assert!(valid_mail_public_id("m0123456789abcdef0123456789abcdef"));
    assert!(!valid_mail_public_id("M0123456789abcdef0123456789abcdef"));
}

#[test]
fn postgres_board_contract_keeps_exact_limits_and_validation() {
    assert_eq!(BOARD_ACCOUNT_CAPACITY, 64);
    assert_eq!(BOARD_CAPACITY_PER_BOARD, 1_000);
    assert_eq!(BOARD_BYTE_CAPACITY_PER_BOARD, 4 * 1_024 * 1_024);
    assert_eq!(BOARD_GLOBAL_CAPACITY, 2_000);
    assert_eq!(BOARD_GLOBAL_BYTE_CAPACITY, 8 * 1_024 * 1_024);
    assert_eq!(BOARD_PAGE_SIZE, 10);
    assert_eq!(BOARD_MAX_PAGES, 100);
    assert_eq!(BOARD_HOURLY_CAPACITY, 5);
    assert_eq!(BOARD_DAILY_CAPACITY, 20);
    assert_eq!(BOARD_PURGE_LIMIT, 256);
    assert_eq!(BOARD_RETENTION_SECONDS, 90 * MAIL_DAY_SECONDS);
    assert!(valid_board_public_id("b0123456789abcdef0123456789abcdef"));
    assert!(!valid_board_public_id("B0123456789abcdef0123456789abcdef"));
    assert!(valid_board_subject("Visible subject"));
    assert!(!valid_board_subject("Bad | subject"));
    assert!(matches!(
        normalise_board_text(" padded ", MAX_BOARD_SUBJECT_BYTES),
        Err(StoreError::InvalidBoardInput)
    ));
    assert!(
        check_board_capacity(
            i64::from(BOARD_ACCOUNT_CAPACITY) - 1,
            i64::from(BOARD_CAPACITY_PER_BOARD) - 1,
            BOARD_BYTE_CAPACITY_PER_BOARD - 2,
            BOARD_GLOBAL_CAPACITY - 1,
            BOARD_GLOBAL_BYTE_CAPACITY - 2,
            2,
        )
        .is_ok()
    );
    for result in [
        check_board_capacity(i64::from(BOARD_ACCOUNT_CAPACITY), 0, 0, 0, 0, 2),
        check_board_capacity(0, i64::from(BOARD_CAPACITY_PER_BOARD), 0, 0, 0, 2),
        check_board_capacity(0, 0, BOARD_BYTE_CAPACITY_PER_BOARD - 1, 0, 0, 2),
        check_board_capacity(0, 0, 0, BOARD_GLOBAL_CAPACITY, 0, 2),
        check_board_capacity(0, 0, 0, 0, BOARD_GLOBAL_BYTE_CAPACITY - 1, 2),
    ] {
        assert!(matches!(result, Err(StoreError::BoardCapacity)));
    }
    assert!(
        check_board_report_capacity(
            BOARD_OPEN_REPORT_CAPACITY_PER_ACCOUNT - 1,
            BOARD_REPORT_EVIDENCE_GLOBAL_CAPACITY - 1,
        )
        .is_ok()
    );
    assert!(matches!(
        check_board_report_capacity(
            BOARD_OPEN_REPORT_CAPACITY_PER_ACCOUNT,
            BOARD_REPORT_EVIDENCE_GLOBAL_CAPACITY - 1,
        ),
        Err(StoreError::BoardReportLimit)
    ));
    assert!(matches!(
        check_board_report_capacity(
            BOARD_OPEN_REPORT_CAPACITY_PER_ACCOUNT - 1,
            BOARD_REPORT_EVIDENCE_GLOBAL_CAPACITY,
        ),
        Err(StoreError::BoardEvidenceLimit)
    ));
}

#[test]
fn postgres_runtime_decoder_accepts_current_and_upgrades_strict_v3_v4_v6() {
    let mut current = World::demo().runtime_snapshot();
    let item_id = current.item_instances.keys().next().unwrap().clone();
    let item = current.item_instances.get_mut(&item_id).unwrap();
    item.location = RuntimeItemLocation::Carried {
        character_id: CharacterId(77),
    };
    item.binding = ItemBinding::Character {
        character_id: CharacterId(77),
    };
    let current_json = serde_json::to_value(&current).unwrap();
    assert_eq!(
        current_json["itemInstances"][item_id.0.as_str()]["binding"],
        serde_json::json!({"kind":"character", "characterId":77})
    );
    let decoded = decode_runtime_snapshot(
        i64::from(WORLD_RUNTIME_SNAPSHOT_VERSION),
        &current_json.to_string(),
    )
    .unwrap();
    assert_eq!(
        decoded.item_instances[&item_id].binding,
        ItemBinding::Character {
            character_id: CharacterId(77)
        }
    );

    let legacy = WorldRuntimeSnapshotV3 {
        snapshot_version: 3,
        revision: 5,
        logical_tick: 6,
        rng_state: 7,
        next_item_instance_sequence: 8,
        item_instances: BTreeMap::from([(
            ItemInstanceId("legacy:item".into()),
            RuntimeItemInstanceSnapshotV3 {
                prototype_id: "legacy:item".into(),
                location: RuntimeItemLocation::Room {
                    room_id: "legacy:room".into(),
                },
            },
        )]),
        npcs: BTreeMap::new(),
        adventure: AdventureRuntimeSnapshot::default(),
        progression_flags: Default::default(),
    };
    let legacy_json = serde_json::to_string(&legacy).unwrap();
    let upgraded = decode_runtime_snapshot(3, &legacy_json).unwrap();
    assert_eq!(
        upgraded.item_instances[&ItemInstanceId("legacy:item".into())].binding,
        ItemBinding::Unbound
    );
    assert!(decode_runtime_snapshot(4, &legacy_json).is_err());

    let mut smuggled = serde_json::to_value(legacy).unwrap();
    smuggled["itemInstances"]["legacy:item"]["binding"] =
        serde_json::json!({"kind":"character", "characterId":77});
    assert!(decode_runtime_snapshot(3, &smuggled.to_string()).is_err());

    let mut v4 = current_json;
    v4["snapshotVersion"] = serde_json::json!(4);
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
        v4.as_object_mut().unwrap().remove(field);
    }
    for item in v4["itemInstances"].as_object_mut().unwrap().values_mut() {
        item.as_object_mut().unwrap().remove("itemAffixes");
    }
    decode_runtime_snapshot(4, &v4.to_string()).unwrap();
    v4["trades"] = serde_json::json!({});
    assert!(decode_runtime_snapshot(4, &v4.to_string()).is_err());

    let mut v6 = serde_json::to_value(&current).unwrap();
    v6["snapshotVersion"] = serde_json::json!(6);
    v6.as_object_mut().unwrap().remove("nextRewardOpenId");
    v6.as_object_mut().unwrap().remove("areaResidency");
    for item in v6["itemInstances"].as_object_mut().unwrap().values_mut() {
        item.as_object_mut().unwrap().remove("itemAffixes");
    }
    let upgraded = decode_runtime_snapshot(6, &v6.to_string()).unwrap();
    assert_eq!(upgraded.next_reward_open_id, tiamud_core::RewardOpenId(1));
    v6["itemInstances"][item_id.0.as_str()]["itemAffixes"] = serde_json::json!([]);
    assert!(decode_runtime_snapshot(6, &v6.to_string()).is_err());
}

fn scoped_url(base: &str, schema: &str) -> String {
    let separator = if base.contains('?') { '&' } else { '?' };
    format!("{base}{separator}options=-csearch_path%3D{schema}")
}

async fn admin_batch(base_url: &str, sql: String) {
    let base_url = base_url.to_owned();
    tokio::task::spawn_blocking(move || {
        tiamud_persistence_postgres::connect(&base_url)
            .unwrap()
            .batch_execute(&sql)
            .unwrap();
    })
    .await
    .unwrap();
}

/// Replace the stored runtime of `schema` with a complete rewrite of `json`
/// as v17 sections, exactly as the v16 -> v17 migration would.
async fn admin_write_runtime_document(base_url: &str, schema: &str, version: i64, json: String) {
    let url = scoped_url(base_url, schema);
    tokio::task::spawn_blocking(move || {
        let mut client = tiamud_persistence_postgres::connect(&url).unwrap();
        tiamud_persistence_postgres::runtime_sections::write_runtime_document(
            &mut client,
            version,
            &json,
            None,
        )
        .unwrap();
    })
    .await
    .unwrap();
}

async fn admin_count(base_url: &str, sql: String) -> i64 {
    let base_url = base_url.to_owned();
    tokio::task::spawn_blocking(move || {
        tiamud_persistence_postgres::connect(&base_url)
            .unwrap()
            .query_one(&sql, &[])
            .unwrap()
            .get(0)
    })
    .await
    .unwrap()
}

#[tokio::test]
async fn postgres_startup_recovery_is_atomic_and_snapshot_authoritative() {
    let Ok(base_url) = env::var("TIAMUD_TEST_POSTGRES_URL") else {
        return;
    };
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let schema = format!("tiamud_recovery_test_{}_{}", process::id(), nonce);
    admin_batch(&base_url, format!("CREATE SCHEMA {schema}")).await;
    let store = PostgresCharacterStore::open(&scoped_url(&base_url, &schema))
        .await
        .unwrap();

    let empty = store.load_startup_world_recovery().await.unwrap();
    assert_eq!(empty, StartupWorldRecovery::Empty);
    assert!(!empty.requires_full_generation());

    let event = DomainEvent::CharacterMoved {
        actor_id: CharacterId(1),
        actor_name: "Aria".into(),
        from_room_id: "room:a".into(),
        to_room_id: "room:b".into(),
        direction: Direction::North,
    };
    let event_json = serde_json::to_string(&event).unwrap().replace('\'', "''");
    admin_batch(
        &base_url,
        format!("INSERT INTO {schema}.domain_events (event_json) VALUES ('{event_json}')"),
    )
    .await;
    let ledger = store.load_startup_world_recovery().await.unwrap();
    assert_eq!(ledger, StartupWorldRecovery::Ledger(vec![event]));
    assert!(ledger.requires_full_generation());

    let snapshot = World::demo().runtime_snapshot();
    store
        .commit_action(Vec::new(), snapshot.clone(), Vec::new())
        .await
        .unwrap();
    assert_eq!(
        store.load_startup_world_recovery().await.unwrap(),
        StartupWorldRecovery::Snapshot {
            durable_version: tiamud_core::WORLD_RUNTIME_SNAPSHOT_VERSION,
            snapshot: snapshot.clone(),
        }
    );

    let mut legacy = serde_json::to_value(snapshot).unwrap();
    let object = legacy.as_object_mut().unwrap();
    object.insert("snapshotVersion".into(), serde_json::json!(7));
    object.remove("areaResidency");
    admin_write_runtime_document(
        &base_url,
        &schema,
        7,
        serde_json::to_string(&legacy).unwrap(),
    )
    .await;
    let StartupWorldRecovery::Snapshot {
        durable_version,
        snapshot,
    } = store.load_startup_world_recovery().await.unwrap()
    else {
        panic!("expected durable snapshot authority");
    };
    assert_eq!(durable_version, 7);
    assert!(snapshot.area_residency.is_empty());

    // A present but invalid snapshot must never fall through to the valid
    // retained ledger observed by the same repeatable-read transaction.
    admin_write_runtime_document(&base_url, &schema, 9, "{}".to_owned()).await;
    assert!(matches!(
        store.load_startup_world_recovery().await,
        Err(StoreError::InvalidRuntimeSnapshot)
    ));

    // Sectioned storage fails closed too: a valid snapshot whose section row
    // no longer matches the manifest digest, and orphan sections.
    store
        .commit_action(Vec::new(), World::demo().runtime_snapshot(), Vec::new())
        .await
        .unwrap();
    admin_batch(
        &base_url,
        format!(
            "UPDATE {schema}.world_runtime_sections
             SET section_json = '[1]' WHERE section = 'npcs'"
        ),
    )
    .await;
    assert!(matches!(
        store.load_startup_world_recovery().await,
        Err(StoreError::InvalidRuntimeSnapshot)
    ));
    admin_batch(&base_url, format!("DELETE FROM {schema}.world_runtime")).await;
    assert!(matches!(
        store.load_startup_world_recovery().await,
        Err(StoreError::InvalidRuntimeSnapshot)
    ));

    admin_batch(
        &base_url,
        format!(
            "DELETE FROM {schema}.world_runtime_sections;
             DELETE FROM {schema}.world_runtime;
             DELETE FROM {schema}.domain_events;
             INSERT INTO {schema}.domain_events (sequence, event_json)
             VALUES (1, '{event_json}'), (3, '{event_json}')"
        ),
    )
    .await;
    assert!(matches!(
        store.load_startup_world_recovery().await,
        Err(StoreError::InvalidStartupLedger)
    ));

    admin_batch(
        &base_url,
        format!(
            "DELETE FROM {schema}.domain_events;
             INSERT INTO {schema}.domain_events (sequence, event_json)
             SELECT sequence, '{{}}'
             FROM generate_series(1, {}) AS generated(sequence)",
            DOMAIN_EVENT_RETENTION + 1
        ),
    )
    .await;
    assert!(matches!(
        store.load_startup_world_recovery().await,
        Err(StoreError::InvalidStartupLedger)
    ));

    drop(store);
    tokio::time::sleep(Duration::from_millis(25)).await;
    admin_batch(&base_url, format!("DROP SCHEMA {schema} CASCADE")).await;
}

/// Parity with the SQLite sectioned-runtime tests (ADR 0035): only dirty
/// sections are rewritten, a failed commit is atomic and the retry rewrites
/// everything, and recovery assembles exactly the committed snapshot.
#[tokio::test]
async fn postgres_sectioned_runtime_rewrites_only_dirty_sections_and_recovers_exactly() {
    let Ok(base_url) = env::var("TIAMUD_TEST_POSTGRES_URL") else {
        return;
    };
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let schema = format!("tiamud_sections_test_{}_{}", process::id(), nonce);
    admin_batch(&base_url, format!("CREATE SCHEMA {schema}")).await;
    let store = PostgresCharacterStore::open(&scoped_url(&base_url, &schema))
        .await
        .unwrap();
    let mut world = World::demo();
    let session = tiamud_core::SessionId(41);
    world
        .connect(
            session,
            CharacterSnapshot {
                id: CharacterId(41),
                account_id: AccountId(41),
                name: "Sectioned".into(),
                room_id: "demo:lantern-square".into(),
                item_locations: Default::default(),
            },
        )
        .unwrap();
    let written_last_sql = format!(
        "SELECT COUNT(*)::BIGINT FROM {schema}.world_runtime_sections
         WHERE revision = (SELECT max(revision) FROM {schema}.world_runtime_sections)"
    );

    assert_eq!(world.runtime_dirty_sections(), RuntimeSectionSet::ALL);
    store
        .commit_action_sections(
            Vec::new(),
            world.runtime_snapshot(),
            world.runtime_dirty_sections(),
            Vec::new(),
        )
        .await
        .unwrap();
    world.mark_durable_commit(true, Vec::new(), false);
    assert_eq!(admin_count(&base_url, written_last_sql.clone()).await, 10);

    world
        .apply(
            session,
            tiamud_core::PlayerAction::Take(tiamud_core::ItemSelection::one("blue-glass lantern")),
        )
        .unwrap();
    let dirty = world.runtime_dirty_sections();
    assert!(!dirty.is_empty() && !dirty.is_all());
    store
        .commit_action_sections(Vec::new(), world.runtime_snapshot(), dirty, Vec::new())
        .await
        .unwrap();
    world.mark_durable_commit(true, Vec::new(), false);
    assert_eq!(
        admin_count(&base_url, written_last_sql.clone()).await,
        dirty.len() as i64
    );
    assert_eq!(
        store.load_startup_world_recovery().await.unwrap(),
        StartupWorldRecovery::Snapshot {
            durable_version: WORLD_RUNTIME_SNAPSHOT_VERSION,
            snapshot: world.runtime_snapshot(),
        }
    );

    // A failed commit leaves no partial section rows behind.
    let committed = world.runtime_snapshot();
    world
        .apply(
            session,
            tiamud_core::PlayerAction::Drop(tiamud_core::ItemSelection::one("blue-glass lantern")),
        )
        .unwrap();
    admin_batch(
        &base_url,
        format!(
            "CREATE FUNCTION {schema}.reject_runtime_head() RETURNS trigger
             LANGUAGE plpgsql AS $$ BEGIN RAISE EXCEPTION 'injected'; END; $$;
             CREATE TRIGGER reject_runtime_head
             BEFORE UPDATE ON {schema}.world_runtime
             FOR EACH ROW EXECUTE FUNCTION {schema}.reject_runtime_head();"
        ),
    )
    .await;
    assert!(
        store
            .commit_action_sections(
                Vec::new(),
                world.runtime_snapshot(),
                world.runtime_dirty_sections(),
                Vec::new(),
            )
            .await
            .is_err()
    );
    world.invalidate_durable_baseline();
    match store.load_startup_world_recovery().await.unwrap() {
        StartupWorldRecovery::Snapshot { snapshot, .. } => assert_eq!(snapshot, committed),
        other => panic!("expected a snapshot, got {other:?}"),
    }
    admin_batch(
        &base_url,
        format!("DROP TRIGGER reject_runtime_head ON {schema}.world_runtime"),
    )
    .await;
    assert_eq!(world.runtime_dirty_sections(), RuntimeSectionSet::ALL);
    store
        .commit_action_sections(
            Vec::new(),
            world.runtime_snapshot(),
            world.runtime_dirty_sections(),
            Vec::new(),
        )
        .await
        .unwrap();
    assert_eq!(admin_count(&base_url, written_last_sql).await, 10);
    match store.load_startup_world_recovery().await.unwrap() {
        StartupWorldRecovery::Snapshot { snapshot, .. } => {
            assert_eq!(snapshot, world.runtime_snapshot())
        }
        other => panic!("expected a snapshot, got {other:?}"),
    }

    drop(store);
    tokio::time::sleep(Duration::from_millis(25)).await;
    admin_batch(&base_url, format!("DROP SCHEMA {schema} CASCADE")).await;
}

#[tokio::test]
async fn postgres_area_generation_bind_and_activation_are_restart_safe_and_cas_guarded() {
    let Ok(base_url) = env::var("TIAMUD_TEST_POSTGRES_URL") else {
        return;
    };
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let schema = format!("tiamud_generation_test_{}_{}", process::id(), nonce);
    admin_batch(&base_url, format!("CREATE SCHEMA {schema}")).await;
    let scoped_url = scoped_url(&base_url, &schema);
    let store = PostgresCharacterStore::open(&scoped_url).await.unwrap();
    let competing_store = PostgresCharacterStore::open(&scoped_url).await.unwrap();
    assert_eq!(store.load_area_generation().await.unwrap(), None);

    let initial = area_generation("Mundo de Ámbar", 'a', None, 1);
    assert_eq!(
        store
            .bind_area_generation_if_absent(initial.clone())
            .await
            .unwrap(),
        initial
    );
    assert_eq!(
        competing_store
            .bind_area_generation_if_absent(area_generation("Mundo de Ámbar", 'b', None, 1,))
            .await
            .unwrap(),
        initial
    );

    let mut activated_runtime = World::demo().runtime_snapshot();
    activated_runtime.logical_tick = 41;
    let activated = area_generation("Mundo de Ámbar", 'b', Some('a'), 2);
    store
        .commit_area_generation_activation(
            activated_runtime.clone(),
            initial.clone(),
            activated.clone(),
        )
        .await
        .unwrap();
    assert_eq!(
        competing_store.load_area_generation().await.unwrap(),
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
        competing_store
            .commit_area_generation_activation(
                stale_runtime,
                initial,
                area_generation("Mundo de Ámbar", 'c', Some('a'), 2),
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
            snapshot: activated_runtime.clone(),
        }
    );

    admin_batch(
        &base_url,
        format!(
            "CREATE FUNCTION {schema}.reject_generation_snapshot() RETURNS trigger
             LANGUAGE plpgsql AS $$
             BEGIN
                 RAISE EXCEPTION 'injected snapshot failure';
             END;
             $$;
             CREATE TRIGGER reject_generation_snapshot
             BEFORE UPDATE ON {schema}.world_runtime
             FOR EACH ROW EXECUTE FUNCTION {schema}.reject_generation_snapshot()"
        ),
    )
    .await;
    let replacement = {
        let mut runtime = World::demo().runtime_snapshot();
        runtime.logical_tick = 73;
        runtime
    };
    assert!(
        store
            .commit_area_generation_activation(
                replacement,
                activated.clone(),
                area_generation("Mundo de Ámbar", 'c', Some('b'), 3),
            )
            .await
            .is_err()
    );
    assert_eq!(
        store.load_area_generation().await.unwrap(),
        Some(activated.clone()),
        "the pointer update must roll back when snapshot persistence fails"
    );
    assert_eq!(
        store.load_startup_world_recovery().await.unwrap(),
        StartupWorldRecovery::Snapshot {
            durable_version: tiamud_core::WORLD_RUNTIME_SNAPSHOT_VERSION,
            snapshot: activated_runtime,
        }
    );

    drop(competing_store);
    drop(store);
    tokio::time::sleep(Duration::from_millis(25)).await;
    admin_batch(&base_url, format!("DROP SCHEMA {schema} CASCADE")).await;
}

#[tokio::test]
async fn postgres_area_checkpoint_transitions_are_exact_restart_safe_and_atomic() {
    let Ok(base_url) = env::var("TIAMUD_TEST_POSTGRES_URL") else {
        return;
    };
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let schema = format!("tiamud_area_checkpoint_test_{}_{}", process::id(), nonce);
    admin_batch(&base_url, format!("CREATE SCHEMA {schema}")).await;
    let scoped_url = scoped_url(&base_url, &schema);
    let store = PostgresCharacterStore::open(&scoped_url).await.unwrap();
    let generation = area_generation("Mundo de Ámbar", 'a', None, 1);
    store
        .bind_area_generation_if_absent(generation.clone())
        .await
        .unwrap();
    let baseline = initial_resident_runtime(World::demo().runtime_snapshot());
    store
        .commit_action(Vec::new(), baseline.clone(), Vec::new())
        .await
        .unwrap();
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
    let retired = retired_runtime(current.clone(), &checkpoint);

    admin_batch(
        &base_url,
        format!(
            "CREATE FUNCTION {schema}.reject_area_checkpoint_snapshot() RETURNS trigger
             LANGUAGE plpgsql AS $$
             BEGIN
                 RAISE EXCEPTION 'injected area checkpoint snapshot failure';
             END;
             $$;
             CREATE TRIGGER reject_area_checkpoint_snapshot
             BEFORE UPDATE ON {schema}.world_runtime
             FOR EACH ROW EXECUTE FUNCTION {schema}.reject_area_checkpoint_snapshot()"
        ),
    )
    .await;
    assert!(
        store
            .commit_area_retirement(retired.clone(), generation.clone(), checkpoint.clone(),)
            .await
            .is_err()
    );
    assert!(store.load_area_checkpoints().await.unwrap().is_empty());
    assert_eq!(store.load_runtime_snapshot().await.unwrap(), Some(current));

    admin_batch(
        &base_url,
        format!("DROP TRIGGER reject_area_checkpoint_snapshot ON {schema}.world_runtime"),
    )
    .await;
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

    let stale_resident = rehydrated_runtime(retired.clone());
    let mut current_retired = retired;
    current_retired.logical_tick += 1;
    store
        .commit_action(Vec::new(), current_retired.clone(), Vec::new())
        .await
        .unwrap();
    assert!(matches!(
        store
            .commit_area_rehydration(stale_resident, generation.clone(), checkpoint.clone(),)
            .await,
        Err(StoreError::AreaCheckpointConflict)
    ));
    assert_eq!(
        store.load_area_checkpoints().await.unwrap(),
        vec![checkpoint.clone()]
    );
    let resident = rehydrated_runtime(current_retired.clone());
    admin_batch(
        &base_url,
        format!(
            "CREATE TRIGGER reject_area_checkpoint_snapshot
             BEFORE UPDATE ON {schema}.world_runtime
             FOR EACH ROW EXECUTE FUNCTION {schema}.reject_area_checkpoint_snapshot()"
        ),
    )
    .await;
    assert!(
        store
            .commit_area_rehydration(resident.clone(), generation.clone(), checkpoint.clone(),)
            .await
            .is_err()
    );
    assert_eq!(
        store.load_area_checkpoints().await.unwrap(),
        vec![checkpoint.clone()]
    );
    assert_eq!(
        store.load_runtime_snapshot().await.unwrap(),
        Some(current_retired)
    );
    admin_batch(
        &base_url,
        format!("DROP TRIGGER reject_area_checkpoint_snapshot ON {schema}.world_runtime"),
    )
    .await;
    store
        .commit_area_rehydration(resident.clone(), generation.clone(), checkpoint.clone())
        .await
        .unwrap();
    assert!(store.load_area_checkpoints().await.unwrap().is_empty());
    assert_eq!(
        store.load_runtime_snapshot().await.unwrap(),
        Some(resident.clone())
    );
    assert!(matches!(
        store
            .commit_area_rehydration(resident, generation, checkpoint)
            .await,
        Err(StoreError::AreaCheckpointConflict)
    ));

    drop(store);
    tokio::time::sleep(Duration::from_millis(25)).await;
    admin_batch(&base_url, format!("DROP SCHEMA {schema} CASCADE")).await;
}

#[tokio::test]
async fn postgres_board_advisory_lock_serializes_concurrent_quota() {
    let Ok(base_url) = env::var("TIAMUD_TEST_POSTGRES_URL") else {
        return;
    };
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let schema = format!("tiamud_board_lock_test_{}_{}", process::id(), nonce);
    admin_batch(&base_url, format!("CREATE SCHEMA {schema}")).await;
    let scoped_url = scoped_url(&base_url, &schema);
    let first_store = PostgresCharacterStore::open(&scoped_url).await.unwrap();
    let account = first_store
        .register_account("Concurrent Board", "correct horse battery staple")
        .await
        .unwrap();
    let character = first_store
        .load_or_create_character(account.account.id, "Quill", "room:start")
        .await
        .unwrap();
    let second_store = PostgresCharacterStore::open(&scoped_url).await.unwrap();

    let (first, second, third, fourth, fifth, sixth) = tokio::join!(
        first_store.publish_board_post(
            character.id,
            BoardKey::General,
            "b00000000000000000000000000000001",
            "Concurrent one",
            "Body",
        ),
        second_store.publish_board_post(
            character.id,
            BoardKey::General,
            "b00000000000000000000000000000002",
            "Concurrent two",
            "Body",
        ),
        first_store.publish_board_post(
            character.id,
            BoardKey::General,
            "b00000000000000000000000000000003",
            "Concurrent three",
            "Body",
        ),
        second_store.publish_board_post(
            character.id,
            BoardKey::General,
            "b00000000000000000000000000000004",
            "Concurrent four",
            "Body",
        ),
        first_store.publish_board_post(
            character.id,
            BoardKey::General,
            "b00000000000000000000000000000005",
            "Concurrent five",
            "Body",
        ),
        second_store.publish_board_post(
            character.id,
            BoardKey::General,
            "b00000000000000000000000000000006",
            "Concurrent six",
            "Body",
        ),
    );
    let results = [first, second, third, fourth, fifth, sixth];
    assert_eq!(results.iter().filter(|result| result.is_ok()).count(), 5);
    assert_eq!(
        results
            .iter()
            .filter(|result| matches!(result, Err(StoreError::BoardRateLimited)))
            .count(),
        1
    );
    assert_eq!(
        admin_count(
            &base_url,
            format!("SELECT COUNT(*)::BIGINT FROM {schema}.social_board_posts"),
        )
        .await,
        5
    );
    assert_eq!(
        admin_count(
            &base_url,
            format!(
                "SELECT hour_count::BIGINT FROM {schema}.social_board_usage
                 WHERE account_id = {}",
                account.account.id.0
            ),
        )
        .await,
        BOARD_HOURLY_CAPACITY
    );

    let now = unix_timestamp();
    admin_batch(
        &base_url,
        format!(
            "UPDATE {schema}.social_board_usage
             SET hour_started_at = {now}, hour_count = 0,
                 day_started_at = {now}, day_count = 20, updated_at = {now}
             WHERE account_id = {}",
            account.account.id.0
        ),
    )
    .await;
    assert!(matches!(
        first_store
            .publish_board_post(
                character.id,
                BoardKey::General,
                "b00000000000000000000000000000007",
                "Daily limit",
                "Body",
            )
            .await,
        Err(StoreError::BoardRateLimited)
    ));
    assert_eq!(
        admin_count(
            &base_url,
            format!(
                "SELECT day_count::BIGINT FROM {schema}.social_board_usage
                 WHERE account_id = {}",
                account.account.id.0
            ),
        )
        .await,
        BOARD_DAILY_CAPACITY
    );
    assert_eq!(
        admin_count(
            &base_url,
            format!("SELECT COUNT(*)::BIGINT FROM {schema}.social_board_posts"),
        )
        .await,
        5,
        "the daily rejection must not create a sixth post"
    );

    first_store.shutdown().await.unwrap();
    second_store.shutdown().await.unwrap();
    drop(first_store);
    drop(second_store);
    tokio::time::sleep(Duration::from_millis(25)).await;
    admin_batch(&base_url, format!("DROP SCHEMA {schema} CASCADE")).await;
}

#[tokio::test]
async fn postgres_characters_only_commit_is_atomic_and_leaves_runtime_alone() {
    let Ok(base_url) = env::var("TIAMUD_TEST_POSTGRES_URL") else {
        return;
    };
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let schema = format!("tiamud_rows_test_{}_{}", process::id(), nonce);
    admin_batch(&base_url, format!("CREATE SCHEMA {schema}")).await;
    let store = PostgresCharacterStore::open(&scoped_url(&base_url, &schema))
        .await
        .unwrap();
    let owner = store
        .register_account("Rows Owner", "correct horse battery staple")
        .await
        .unwrap();
    let mut character = store
        .load_or_create_character(owner.account.id, "Rows", "room:a")
        .await
        .unwrap();
    let mut runtime = tiamud_core::World::demo().runtime_snapshot();
    runtime.logical_tick = 7;
    store
        .commit_action(
            vec![tiamud_core::DurableCharacterSnapshot {
                character: character.clone(),
                gameplay: CharacterGameplaySnapshot::default(),
            }],
            runtime.clone(),
            Vec::new(),
        )
        .await
        .unwrap();
    character.room_id = "room:b".into();
    let changed = tiamud_core::DurableCharacterSnapshot {
        character: character.clone(),
        gameplay: CharacterGameplaySnapshot {
            coins: 4_321,
            ..CharacterGameplaySnapshot::default()
        },
    };
    let mut missing = changed.clone();
    missing.character.id = tiamud_core::CharacterId(999_999);
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
        store
            .load_character(owner.account.id, character.id)
            .await
            .unwrap(),
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
    store.shutdown().await.unwrap();
    admin_batch(&base_url, format!("DROP SCHEMA {schema} CASCADE")).await;
}

#[tokio::test]
async fn postgres_store_round_trips_auth_characters_runtime_and_social_policy() {
    let Ok(base_url) = env::var("TIAMUD_TEST_POSTGRES_URL") else {
        return;
    };
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let schema = format!("tiamud_store_test_{}_{}", process::id(), nonce);
    admin_batch(&base_url, format!("CREATE SCHEMA {schema}")).await;

    let store = PostgresCharacterStore::open(&scoped_url(&base_url, &schema))
        .await
        .unwrap();
    store.health_check().await.unwrap();
    assert!(!store.has_durable_character_authority().await.unwrap());

    let keeper = store
        .register_account(" Keeper ", "correct horse battery staple")
        .await
        .unwrap();
    let player = store
        .register_account("Player", "another correct horse battery staple")
        .await
        .unwrap();
    let login = store
        .login_account("KEEPER", "correct horse battery staple")
        .await
        .unwrap();
    assert_eq!(login.account, keeper.account);
    assert_eq!(
        store.resolve_session(login.token.as_str()).await.unwrap(),
        keeper.account
    );

    let keeper_character = store
        .load_or_create_character(keeper.account.id, "Aster", "room:start")
        .await
        .unwrap();
    assert!(store.has_durable_character_authority().await.unwrap());
    let keeper_staff = store.staff_access(keeper_character.id).await.unwrap();
    assert_eq!(keeper_staff.profile, Some(StaffProfileKey::Implementor));
    assert_eq!(
        keeper_staff.capabilities,
        StaffCapability::ALL.into_iter().collect()
    );
    let defaults = CharacterGameplaySnapshot::default();
    let mut selected_gameplay = CharacterGameplaySnapshot {
        lineage_id: "tiamud:mortal".into(),
        background_id: Some("tiamud:city-guard".into()),
        base_class_id: "tiamud:vanguard".into(),
        active_class_id: "tiamud:bastion".into(),
        class_history: vec!["tiamud:vanguard".into(), "tiamud:bastion".into()],
        attributes: tiamud_core::Attributes {
            defence: 9,
            ..defaults.attributes
        },
        resources: BTreeMap::from([(
            "tiamud:focus".into(),
            tiamud_core::ResourcePool {
                current: 7,
                max: 12,
            },
        )]),
        ability_cooldowns: BTreeMap::from([("tiamud:mind-whisper".into(), 23)]),
        ..defaults
    };
    selected_gameplay
        .progression_points
        .insert("talent".into(), 0);
    selected_gameplay.talent_node_ranks.insert(
        "tiamud:martial-talents".into(),
        BTreeMap::from([("tiamud:guarded-stance".into(), 1)]),
    );
    selected_gameplay.tags.insert("tiamud:founder".into());
    selected_gameplay
        .received_signals
        .insert("tiamud:heard-call".into());
    let player_character = store
        .load_or_create_character_with_gameplay(
            player.account.id,
            "Bryn",
            "room:start",
            selected_gameplay.clone(),
        )
        .await
        .unwrap();
    assert_eq!(
        store
            .load_character_gameplay(player_character.id)
            .await
            .unwrap(),
        selected_gameplay
    );
    let all_gameplay = store
        .load_character_gameplay_batch(
            vec![player_character.id, keeper_character.id],
            StartupGameplayLoadLimits::ITEM_OWNERS,
        )
        .await
        .unwrap();
    assert_eq!(all_gameplay.len(), 2);
    assert_eq!(all_gameplay[0].character_id, keeper_character.id);
    assert_eq!(all_gameplay[1].character_id, player_character.id);
    assert_eq!(all_gameplay[1].gameplay, selected_gameplay);
    assert_eq!(
        store.list_characters(keeper.account.id).await.unwrap(),
        vec![keeper_character.clone()]
    );
    assert_eq!(
        store
            .load_character(keeper.account.id, keeper_character.id)
            .await
            .unwrap(),
        keeper_character
    );

    let mut updated_character = keeper_character.clone();
    updated_character.room_id = "room:north".into();
    updated_character
        .item_locations
        .insert("item:lamp".into(), ItemLocationSnapshot::Carried);
    let gameplay = selected_gameplay;
    let runtime = WorldRuntimeSnapshot {
        snapshot_version: WORLD_RUNTIME_SNAPSHOT_VERSION,
        revision: 9,
        logical_tick: 3,
        rng_state: 42,
        next_item_instance_sequence: 1,
        item_instances: BTreeMap::from([(
            ItemInstanceId("item:lamp".into()),
            RuntimeItemInstanceSnapshot {
                prototype_id: "item:lamp".into(),
                location: RuntimeItemLocation::Carried {
                    character_id: keeper_character.id,
                },
                binding: ItemBinding::Character {
                    character_id: keeper_character.id,
                },
                item_affixes: Vec::new(),
            },
        )]),
        next_loot_drop_id: LootDropId(1),
        item_provenance: BTreeMap::from([(
            ItemInstanceId("item:lamp".into()),
            ItemProvenance::AuthoredInitial,
        )]),
        loot_encounters: BTreeMap::new(),
        ground_loot: BTreeMap::new(),
        party_loot_cursors: BTreeMap::new(),
        npcs: BTreeMap::new(),
        adventure: AdventureRuntimeSnapshot::default(),
        progression_flags: Default::default(),
        next_trade_id: tiamud_core::TradeId(1),
        trades: BTreeMap::new(),
        next_reward_open_id: tiamud_core::RewardOpenId(1),
        area_residency: BTreeMap::new(),
    };
    let event = DomainEvent::CharacterMoved {
        actor_id: keeper_character.id,
        actor_name: keeper_character.name.clone(),
        from_room_id: "room:start".into(),
        to_room_id: "room:north".into(),
        direction: Direction::North,
    };
    store
        .commit_action(
            vec![DurableCharacterSnapshot {
                character: updated_character.clone(),
                gameplay: gameplay.clone(),
            }],
            runtime.clone(),
            vec![event.clone()],
        )
        .await
        .unwrap();
    assert_eq!(
        store
            .load_character_gameplay(keeper_character.id)
            .await
            .unwrap(),
        gameplay
    );
    assert_eq!(
        store.load_runtime_snapshot().await.unwrap(),
        Some(runtime.clone())
    );
    assert_eq!(
        store.load_domain_events().await.unwrap(),
        vec![event.clone()]
    );
    assert_eq!(
        admin_count(
            &base_url,
            format!("SELECT last_event_sequence FROM {schema}.world_runtime WHERE singleton = 1"),
        )
        .await,
        1
    );
    assert_eq!(
        admin_count(
            &base_url,
            format!("SELECT COUNT(*)::BIGINT FROM {schema}.character_adventure"),
        )
        .await,
        1
    );
    assert_eq!(
        admin_count(
            &base_url,
            format!("SELECT COUNT(*)::BIGINT FROM {schema}.world_adventure"),
        )
        .await,
        1
    );
    assert_eq!(
        store
            .load_character(keeper.account.id, keeper_character.id)
            .await
            .unwrap(),
        updated_character
    );

    assert_eq!(
        store
            .social_profile(keeper_character.id)
            .await
            .unwrap()
            .role,
        AccountRole::Admin
    );
    assert!(
        store
            .social_channel_subscriptions(player_character.id)
            .await
            .unwrap()
            .is_empty()
    );
    for channel in [SocialChannelName::Newbie, SocialChannelName::General] {
        store
            .set_social_channel_subscription(player_character.id, channel, true)
            .await
            .unwrap();
    }
    store
        .set_social_channel_subscription(keeper_character.id, SocialChannelName::General, true)
        .await
        .unwrap();
    assert_eq!(
        store
            .social_channel_subscriptions(player_character.id)
            .await
            .unwrap(),
        vec![SocialChannelName::General, SocialChannelName::Newbie]
    );
    assert_eq!(
        store
            .social_channel_subscribers(SocialChannelName::General)
            .await
            .unwrap(),
        vec![keeper_character.id, player_character.id]
    );
    let public_id = "m0123456789abcdef0123456789abcdef";
    assert_eq!(
        store
            .send_mail(
                keeper_character.id,
                &player_character.name,
                public_id,
                "Lanterns",
                "Meet beneath the arch.",
            )
            .await
            .unwrap(),
        MailSendResult::Stored
    );
    let page = store.mailbox_page(player_character.id, 1).await.unwrap();
    assert_eq!(page.total_messages, 1);
    assert_eq!(page.unread_count, 1);
    assert_eq!(page.capacity, 64);
    assert_eq!(page.messages[0].public_id, public_id);
    let message = store
        .read_mail(player_character.id, public_id)
        .await
        .unwrap();
    assert_eq!(message.body, "Meet beneath the arch.");
    assert!(message.summary.read);
    let mail_report = store
        .report_mail(player_character.id, public_id, "Unwanted offline contact")
        .await
        .unwrap();
    assert!(mail_report > 0);
    store
        .delete_mail(player_character.id, public_id)
        .await
        .unwrap();
    assert_eq!(
        admin_count(
            &base_url,
            format!("SELECT COUNT(*)::BIGINT FROM {schema}.social_mail_report_evidence"),
        )
        .await,
        1
    );
    let board_public_id = "b0123456789abcdef0123456789abcdef";
    let board_entry = store
        .publish_board_post(
            keeper_character.id,
            BoardKey::General,
            board_public_id,
            "Lantern board",
            "A durable public entry.",
        )
        .await
        .unwrap();
    assert!(board_entry.summary.can_delete);
    let board_page = store
        .list_board_posts(player_character.id, BoardKey::General, 1)
        .await
        .unwrap();
    assert_eq!(board_page.total_posts, 1);
    assert!(board_page.posts[0].can_report);
    assert_eq!(
        store
            .read_board_post(player_character.id, board_public_id)
            .await
            .unwrap()
            .body,
        "A durable public entry."
    );
    store
        .report_board_post(
            player_character.id,
            board_public_id,
            "Public board evidence",
        )
        .await
        .unwrap();
    store
        .delete_board_post(keeper_character.id, board_public_id)
        .await
        .unwrap();
    assert_eq!(
        admin_count(
            &base_url,
            format!("SELECT COUNT(*)::BIGINT FROM {schema}.social_board_report_evidence"),
        )
        .await,
        1
    );

    let actionable_id = "bfedcba9876543210fedcba9876543210";
    store
        .publish_board_post(
            player_character.id,
            BoardKey::Newbie,
            actionable_id,
            "Actionable board entry",
            "Remove this after staff review.",
        )
        .await
        .unwrap();
    let board_report_id = store
        .report_board_post(keeper_character.id, actionable_id, "Requires staff action")
        .await
        .unwrap();
    let reports = store
        .staff_reports(StaffAuditRecord {
            actor_account_id: keeper.account.id,
            target_account_id: None,
            capability: StaffCapability::ReportsView,
            action: "reports.list".into(),
            reason: "review board evidence".into(),
            details: serde_json::json!({}),
        })
        .await
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
        "Actionable board entry"
    );
    store
        .resolve_staff_report(
            board_report_id,
            StaffReportResolution::ActionTaken,
            "board post removed",
            StaffAuditRecord {
                actor_account_id: keeper.account.id,
                target_account_id: None,
                capability: StaffCapability::ReportsResolve,
                action: "reports.resolve".into(),
                reason: "review completed".into(),
                details: serde_json::json!({"reportId": board_report_id}),
            },
        )
        .await
        .unwrap();
    assert!(matches!(
        store
            .read_board_post(keeper_character.id, actionable_id)
            .await,
        Err(StoreError::BoardPostUnavailable)
    ));
    assert_eq!(
        admin_count(
            &base_url,
            format!("SELECT COUNT(*)::BIGINT FROM {schema}.social_board_report_evidence"),
        )
        .await,
        2
    );
    store
        .set_social_relation(
            player_character.id,
            keeper_character.id,
            SocialRelationKind::Ignore,
            true,
        )
        .await
        .unwrap();
    assert!(
        !store
            .social_delivery_allowed(keeper_character.id, player_character.id)
            .await
            .unwrap()
    );
    assert_eq!(
        store
            .send_mail(
                keeper_character.id,
                &player_character.name,
                "mfedcba9876543210fedcba9876543210",
                "Suppressed",
                "This body must not be stored.",
            )
            .await
            .unwrap(),
        MailSendResult::Suppressed
    );
    assert_eq!(
        store
            .mailbox_page(player_character.id, 1)
            .await
            .unwrap()
            .total_messages,
        0
    );
    store
        .moderate_character(
            keeper_character.id,
            player_character.id,
            ModerationAction::Mute {
                expires_at: None,
                reason: "integration test".into(),
            },
        )
        .await
        .unwrap();
    assert!(
        store
            .social_profile(player_character.id)
            .await
            .unwrap()
            .muted
    );
    store
        .report_character(player_character.id, keeper_character.id, "test report")
        .await
        .unwrap();
    assert!(store.audit_log().await.unwrap().len() >= 4);

    admin_batch(
        &base_url,
        format!(
            "INSERT INTO {schema}.domain_events (event_json)
             SELECT '{{\"type\":\"retained-test\"}}'
             FROM generate_series(1, {DOMAIN_EVENT_RETENTION})"
        ),
    )
    .await;
    store
        .commit_action(Vec::new(), runtime.clone(), vec![event.clone()])
        .await
        .unwrap();
    assert_eq!(
        admin_count(
            &base_url,
            format!("SELECT COUNT(*)::BIGINT FROM {schema}.domain_events"),
        )
        .await,
        DOMAIN_EVENT_RETENTION
    );
    let compacted_watermark = DOMAIN_EVENT_RETENTION + 2;
    assert_eq!(
        admin_count(
            &base_url,
            format!("SELECT last_event_sequence FROM {schema}.world_runtime WHERE singleton = 1"),
        )
        .await,
        compacted_watermark
    );

    admin_batch(
        &base_url,
        format!(
            "CREATE FUNCTION {schema}.reject_domain_event_compaction()
             RETURNS trigger LANGUAGE plpgsql AS $$
             BEGIN
                 RAISE EXCEPTION 'injected compaction failure';
             END $$;
             CREATE TRIGGER reject_domain_event_compaction
             BEFORE DELETE ON {schema}.domain_events
             FOR EACH STATEMENT
             EXECUTE FUNCTION {schema}.reject_domain_event_compaction();"
        ),
    )
    .await;
    let mut rejected_runtime = runtime.clone();
    rejected_runtime.logical_tick += 1;
    assert!(
        store
            .commit_action(Vec::new(), rejected_runtime, vec![event])
            .await
            .is_err()
    );
    assert_eq!(
        admin_count(
            &base_url,
            format!("SELECT COUNT(*)::BIGINT FROM {schema}.domain_events"),
        )
        .await,
        DOMAIN_EVENT_RETENTION
    );
    assert_eq!(
        admin_count(
            &base_url,
            format!("SELECT last_event_sequence FROM {schema}.world_runtime WHERE singleton = 1"),
        )
        .await,
        compacted_watermark
    );
    admin_batch(
        &base_url,
        format!(
            "DROP TRIGGER reject_domain_event_compaction ON {schema}.domain_events;
             DROP FUNCTION {schema}.reject_domain_event_compaction();"
        ),
    )
    .await;

    store.revoke_session(login.token.as_str()).await.unwrap();
    assert!(matches!(
        store.resolve_session(login.token.as_str()).await,
        Err(StoreError::SessionRevoked)
    ));

    drop(store);
    tokio::time::sleep(Duration::from_millis(25)).await;
    admin_batch(&base_url, format!("DROP SCHEMA {schema} CASCADE")).await;
}

fn story_state_json(pack_id: &str, filler: usize) -> String {
    format!(
        "{{\"format_version\":2,\"pack_id\":\"{pack_id}\",\"node\":\"{}\"}}",
        "n".repeat(filler)
    )
}

fn story_write(
    account_id: AccountId,
    story_id: &str,
    slot: u8,
    pack_sha: char,
    state_json: String,
) -> StorySaveWrite {
    StorySaveWrite {
        account_id,
        story_id: story_id.to_owned(),
        slot: StorySlot::new(slot).unwrap(),
        pack_id: story_id.to_owned(),
        pack_sha256: pack_sha.to_string().repeat(64),
        state_format_version: 2,
        state_json,
    }
}

fn story_pack(story_id: &str, pack_sha: char) -> StoryPackCompatibility {
    StoryPackCompatibility {
        pack_id: story_id.to_owned(),
        pack_sha256: pack_sha.to_string().repeat(64),
        compatible_pack_sha256: std::collections::BTreeSet::new(),
    }
}

/// PostgreSQL parity of the SQLite story save slot suite (schema v16,
/// ADR 0030): round trip, overwrite, listing, deletion, caps, corruption,
/// pack revision mismatch and account cascade.
#[tokio::test]
async fn postgres_story_slots_round_trip_cap_detect_corruption_and_cascade() {
    use tiamud_persistence_sqlite::story_saves::{
        MAX_STORIES_WITH_SAVES_PER_ACCOUNT, MAX_STORY_SAVE_BYTES_PER_ACCOUNT,
        MAX_STORY_SAVE_STATE_BYTES, STORY_SLOTS_PER_STORY, story_state_sha256,
    };

    let Ok(base_url) = env::var("TIAMUD_TEST_POSTGRES_URL") else {
        return;
    };
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let schema = format!("tiamud_story_slot_test_{}_{}", process::id(), nonce);
    admin_batch(&base_url, format!("CREATE SCHEMA {schema}")).await;
    let store = PostgresCharacterStore::open(&scoped_url(&base_url, &schema))
        .await
        .unwrap();
    let reader = store.load_or_create_account("Reader").await.unwrap();
    let other = store.load_or_create_account("Other").await.unwrap();
    let slot = |value: u8| StorySlot::new(value).unwrap();

    // Round trip, overwrite and listing.
    let first = story_state_json("cripta", 4);
    let saved = store
        .save_story_slot(story_write(reader.id, "cripta", 3, 'a', first.clone()))
        .await
        .unwrap();
    assert_eq!(saved.state_sha256, story_state_sha256(&first));
    assert_eq!(saved.state_bytes as usize, first.len());
    let loaded = store
        .load_story_slot(reader.id, "cripta", slot(3), story_pack("cripta", 'a'))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(loaded.state_json, first);
    assert_eq!(loaded.summary, saved);
    let second = story_state_json("cripta", 9);
    let overwritten = store
        .save_story_slot(story_write(reader.id, "cripta", 3, 'b', second.clone()))
        .await
        .unwrap();
    assert_eq!(overwritten.created_at, saved.created_at);
    assert!(overwritten.updated_at >= saved.updated_at);
    store
        .save_story_slot(story_write(
            reader.id,
            "cripta",
            0,
            'b',
            story_state_json("cripta", 1),
        ))
        .await
        .unwrap();
    store
        .save_story_slot(story_write(
            other.id,
            "cripta",
            5,
            'b',
            story_state_json("cripta", 1),
        ))
        .await
        .unwrap();
    let listed = store.list_story_slots(reader.id, "cripta").await.unwrap();
    assert_eq!(
        listed
            .iter()
            .map(|save| save.slot.get())
            .collect::<Vec<_>>(),
        vec![0, 3]
    );
    assert_eq!(listed[1], overwritten);
    assert_eq!(
        store
            .load_story_slot(reader.id, "cripta", slot(3), story_pack("cripta", 'b'))
            .await
            .unwrap()
            .unwrap()
            .state_json,
        second
    );

    // Pack revision compatibility fails closed unless declared.
    assert!(matches!(
        store
            .load_story_slot(reader.id, "cripta", slot(3), story_pack("cripta", 'a'))
            .await,
        Err(StoreError::StorySavePackMismatch)
    ));
    let mut declared = story_pack("cripta", 'a');
    declared.compatible_pack_sha256.insert("b".repeat(64));
    assert!(
        store
            .load_story_slot(reader.id, "cripta", slot(3), declared)
            .await
            .unwrap()
            .is_some()
    );

    // Input validation and typed size error.
    assert!(matches!(
        store
            .save_story_slot(story_write(
                reader.id,
                "cripta",
                1,
                'a',
                story_state_json("otra", 1),
            ))
            .await,
        Err(StoreError::InvalidStorySave)
    ));
    assert!(matches!(
        store
            .save_story_slot(story_write(
                reader.id,
                "cripta",
                1,
                'a',
                story_state_json("cripta", MAX_STORY_SAVE_STATE_BYTES),
            ))
            .await,
        Err(StoreError::StorySaveTooLarge { .. })
    ));
    assert!(matches!(
        store
            .save_story_slot(story_write(
                AccountId(404_404),
                "cripta",
                1,
                'a',
                story_state_json("cripta", 1),
            ))
            .await,
        Err(StoreError::AccountNotFound(_))
    ));

    // Delete.
    assert!(
        store
            .delete_story_slot(reader.id, "cripta", slot(0))
            .await
            .unwrap()
    );
    assert!(
        !store
            .delete_story_slot(reader.id, "cripta", slot(0))
            .await
            .unwrap()
    );

    // Story cap.
    let collector = store.load_or_create_account("Collector").await.unwrap();
    for index in 0..MAX_STORIES_WITH_SAVES_PER_ACCOUNT {
        let story = format!("historia-{index}");
        store
            .save_story_slot(story_write(
                collector.id,
                &story,
                1,
                'a',
                story_state_json(&story, 1),
            ))
            .await
            .unwrap();
    }
    assert!(matches!(
        store
            .save_story_slot(story_write(
                collector.id,
                "historia-extra",
                1,
                'a',
                story_state_json("historia-extra", 1),
            ))
            .await,
        Err(StoreError::StorySaveCapacity)
    ));

    // Byte cap across stories; overwriting counts only the difference.
    let hoarder = store.load_or_create_account("Hoarder").await.unwrap();
    let large = |story: &str| story_state_json(story, MAX_STORY_SAVE_STATE_BYTES - 64);
    let fits = MAX_STORY_SAVE_BYTES_PER_ACCOUNT / large("grande-0").len();
    let place = |index: usize| {
        (
            format!("grande-{}", index / STORY_SLOTS_PER_STORY),
            u8::try_from(index % STORY_SLOTS_PER_STORY).unwrap(),
        )
    };
    for index in 0..fits {
        let (story, number) = place(index);
        store
            .save_story_slot(story_write(hoarder.id, &story, number, 'a', large(&story)))
            .await
            .unwrap();
    }
    let (story, number) = place(fits);
    assert!(matches!(
        store
            .save_story_slot(story_write(hoarder.id, &story, number, 'a', large(&story)))
            .await,
        Err(StoreError::StorySaveCapacity)
    ));
    store
        .save_story_slot(story_write(
            hoarder.id,
            "grande-0",
            0,
            'b',
            large("grande-0"),
        ))
        .await
        .unwrap();

    // Same-length tampering is detected by the digest, never a panic.
    admin_batch(
        &base_url,
        format!(
            "UPDATE {schema}.story_saves
             SET state_json = replace(state_json, 'nnnn', 'mmmm')
             WHERE account_id = {} AND story_id = 'cripta' AND slot = 3",
            reader.id.0
        ),
    )
    .await;
    assert!(matches!(
        store
            .load_story_slot(reader.id, "cripta", slot(3), story_pack("cripta", 'b'))
            .await,
        Err(StoreError::StorySaveCorrupt)
    ));

    // Account deletion cascades to every slot of that account only.
    admin_batch(
        &base_url,
        format!("DELETE FROM {schema}.accounts WHERE id = {}", reader.id.0),
    )
    .await;
    assert_eq!(
        admin_count(
            &base_url,
            format!(
                "SELECT COUNT(*)::BIGINT FROM {schema}.story_saves WHERE account_id = {}",
                reader.id.0
            ),
        )
        .await,
        0
    );
    assert_eq!(
        store
            .list_story_slots(other.id, "cripta")
            .await
            .unwrap()
            .len(),
        1
    );

    drop(store);
    tokio::time::sleep(Duration::from_millis(25)).await;
    admin_batch(&base_url, format!("DROP SCHEMA {schema} CASCADE")).await;
}
