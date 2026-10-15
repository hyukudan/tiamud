//! Sectioned world runtime storage on SQLite (schema v17, ADR 0035).

use super::sqlite::*;
use super::*;
use rusqlite::Connection;
use std::collections::BTreeSet;
use tiamud_core::{
    CharacterId, CharacterSnapshot, Direction, ItemSelection, PlayerAction, RuntimeSection,
    SessionId, World,
};

fn open(path: &Path) -> Connection {
    let mut connection = Connection::open(path).unwrap();
    initialise(&mut connection).unwrap();
    connection
}

fn recovered(connection: &mut Connection) -> WorldRuntimeSnapshot {
    match load_startup_world_recovery(connection).unwrap() {
        StartupWorldRecovery::Snapshot { snapshot, .. } => snapshot,
        other => panic!("expected a runtime snapshot, got {other:?}"),
    }
}

/// Section names whose stored row was written by the latest runtime commit.
fn sections_written_last(connection: &Connection) -> BTreeSet<String> {
    let manifest: String = connection
        .query_row(
            "SELECT manifest_json FROM world_runtime WHERE singleton = 1",
            [],
            |row| row.get(0),
        )
        .unwrap();
    let revision = runtime_sections::RuntimeManifest::parse(&manifest)
        .unwrap()
        .revision;
    connection
        .prepare("SELECT section FROM world_runtime_sections WHERE revision = ?1")
        .unwrap()
        .query_map([revision as i64], |row| row.get::<_, String>(0))
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap()
}

fn names(sections: RuntimeSectionSet) -> BTreeSet<String> {
    sections
        .iter()
        .map(|section| section.field_name().to_owned())
        .collect()
}

fn demo_world() -> (World, SessionId) {
    let mut world = World::demo();
    let session = SessionId(41);
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
    (world, session)
}

/// Commits exactly what the actor's command/tick path commits: the dirty
/// sections only, then advances the durable baseline on success.
fn commit_sections(
    connection: &mut Connection,
    world: &mut World,
    events: &[DomainEvent],
) -> Result<RuntimeSectionSet, StoreError> {
    let dirty = world.runtime_dirty_sections();
    match commit_action_sections(connection, &[], &world.runtime_snapshot(), dirty, events) {
        Ok(()) => {
            world.mark_durable_commit(true, Vec::new(), false);
            Ok(dirty)
        }
        Err(error) => {
            world.invalidate_durable_baseline();
            Err(error)
        }
    }
}

#[test]
fn only_dirty_sections_are_rewritten_and_recovery_assembles_the_full_snapshot() {
    let directory = tempfile::TempDir::new().unwrap();
    let path = directory.path().join("sectioned.sqlite3");
    let mut connection = open(&path);
    let (mut world, session) = demo_world();

    // Without a baseline every section is written.
    assert_eq!(
        commit_sections(&mut connection, &mut world, &[]).unwrap(),
        RuntimeSectionSet::ALL
    );
    assert_eq!(
        sections_written_last(&connection),
        names(RuntimeSectionSet::ALL)
    );
    assert_eq!(recovered(&mut connection), world.runtime_snapshot());

    // A clean commit (for example one that only records events) rewrites the
    // head and manifest and no section.
    assert_eq!(
        commit_sections(&mut connection, &mut world, &[]).unwrap(),
        RuntimeSectionSet::EMPTY
    );
    assert!(sections_written_last(&connection).is_empty());
    assert_eq!(recovered(&mut connection), world.runtime_snapshot());

    // Taking an item dirties the item sections only.
    world
        .apply(
            session,
            PlayerAction::Take(ItemSelection::one("blue-glass lantern")),
        )
        .unwrap();
    let dirty = commit_sections(&mut connection, &mut world, &[]).unwrap();
    assert!(dirty.contains(RuntimeSection::ItemInstances));
    assert!(!dirty.contains(RuntimeSection::Npcs));
    assert!(!dirty.contains(RuntimeSection::Adventure));
    assert_eq!(sections_written_last(&connection), names(dirty));
    assert_eq!(recovered(&mut connection), world.runtime_snapshot());

    // Moving changes no runtime section at all.
    world
        .apply(session, PlayerAction::Move(Direction::North))
        .unwrap();
    assert!(world.runtime_dirty_sections().is_empty());
}

#[test]
fn random_operation_sequences_recover_exactly_across_crashes() {
    for seed in [3_u64, 17, 9_001] {
        let directory = tempfile::TempDir::new().unwrap();
        let path = directory.path().join("random.sqlite3");
        let mut connection = open(&path);
        let (mut world, session) = demo_world();
        let mut state = seed;
        let mut next = || {
            state = state
                .wrapping_mul(6_364_136_223_846_793_005)
                .wrapping_add(1_442_695_040_888_963_407);
            (state >> 33) % 8
        };
        let mut committed = None;
        for _ in 0..60 {
            let mut events = Vec::new();
            match next() {
                0 => {
                    let _ = world.apply(
                        session,
                        PlayerAction::Take(ItemSelection::one("blue-glass lantern")),
                    );
                }
                1 => {
                    let _ = world.apply(
                        session,
                        PlayerAction::Drop(ItemSelection::one("blue-glass lantern")),
                    );
                }
                2 => {
                    let _ = world.apply(session, PlayerAction::Move(Direction::North));
                }
                3 => {
                    let _ = world.apply(session, PlayerAction::Move(Direction::South));
                }
                4 | 5 => events = world.advance_tick().unwrap(),
                6 => {
                    // Crash: drop the connection without any shutdown and
                    // recover from the file. The last confirmed commit is the
                    // authority; in-memory state that was never committed is
                    // not.
                    drop(connection);
                    connection = open(&path);
                    if let Some(expected) = &committed {
                        assert_eq!(&recovered(&mut connection), expected);
                    }
                    continue;
                }
                _ => {}
            }
            commit_sections(&mut connection, &mut world, &events).unwrap();
            let snapshot = world.runtime_snapshot();
            assert_eq!(recovered(&mut connection), snapshot, "seed {seed}");
            committed = Some(snapshot);
        }
        // The assembled document is the full snapshot, member for member.
        let (_, assembled) =
            runtime_sections::read_runtime_document_sqlite(&connection, usize::MAX)
                .unwrap()
                .unwrap();
        assert_eq!(
            serde_json::from_str::<serde_json::Value>(&assembled).unwrap(),
            serde_json::to_value(committed.unwrap()).unwrap()
        );
    }
}

#[test]
fn a_failed_sectioned_commit_is_atomic_and_the_retry_rewrites_everything() {
    let directory = tempfile::TempDir::new().unwrap();
    let path = directory.path().join("failure.sqlite3");
    let mut connection = open(&path);
    let (mut world, session) = demo_world();
    commit_sections(&mut connection, &mut world, &[]).unwrap();
    let before = world.runtime_snapshot();
    let rows_before = runtime_sections::load_stored_runtime_sqlite(&connection)
        .unwrap()
        .unwrap();

    world
        .apply(
            session,
            PlayerAction::Take(ItemSelection::one("blue-glass lantern")),
        )
        .unwrap();
    connection
        .execute_batch(
            "CREATE TRIGGER reject_runtime_head
             BEFORE UPDATE ON world_runtime
             BEGIN
                 SELECT RAISE(ABORT, 'injected runtime failure');
             END;",
        )
        .unwrap();
    assert!(commit_sections(&mut connection, &mut world, &[test_event()]).is_err());
    // Nothing of the failed transaction is visible: not the section rows
    // written before the head, not the event.
    assert_eq!(
        runtime_sections::load_stored_runtime_sqlite(&connection)
            .unwrap()
            .unwrap(),
        rows_before
    );
    assert_eq!(recovered(&mut connection), before);
    assert_eq!(
        connection
            .query_row("SELECT COUNT(*) FROM domain_events", [], |row| row
                .get::<_, i64>(0))
            .unwrap(),
        0
    );

    // The failure dropped the baseline, so the retry rewrites every section
    // exactly once, with the same in-memory state.
    connection
        .execute_batch("DROP TRIGGER reject_runtime_head")
        .unwrap();
    assert_eq!(world.runtime_dirty_sections(), RuntimeSectionSet::ALL);
    assert_eq!(
        commit_sections(&mut connection, &mut world, &[test_event()]).unwrap(),
        RuntimeSectionSet::ALL
    );
    assert_eq!(recovered(&mut connection), world.runtime_snapshot());
    assert_eq!(
        connection
            .query_row("SELECT COUNT(*) FROM domain_events", [], |row| row
                .get::<_, i64>(0))
            .unwrap(),
        1
    );
}

fn test_event() -> DomainEvent {
    DomainEvent::CharacterMoved {
        actor_id: CharacterId(41),
        actor_name: "Sectioned".into(),
        from_room_id: "demo:lantern-square".into(),
        to_room_id: "demo:north-gate".into(),
        direction: Direction::North,
    }
}

#[test]
fn corrupt_missing_or_orphan_sections_fail_closed() {
    let corruptions: [(&str, &str); 7] = [
        (
            "tampered bytes",
            "UPDATE world_runtime_sections SET section_json = '[1]' WHERE section = 'npcs'",
        ),
        (
            "tampered digest",
            "UPDATE world_runtime_sections
             SET section_sha256 = '0000000000000000000000000000000000000000000000000000000000000000'
             WHERE section = 'trades'",
        ),
        (
            "missing section",
            "DELETE FROM world_runtime_sections WHERE section = 'itemInstances'",
        ),
        (
            "stale section revision",
            "UPDATE world_runtime_sections SET revision = revision + 1 WHERE section = 'groundLoot'",
        ),
        (
            "malformed manifest",
            "UPDATE world_runtime SET manifest_json = '{}'",
        ),
        (
            "head defines a section",
            "UPDATE world_runtime SET head_json = '{\"npcs\":{}}'",
        ),
        ("orphan sections", "DELETE FROM world_runtime"),
    ];
    for (case, sql) in corruptions {
        let directory = tempfile::TempDir::new().unwrap();
        let path = directory.path().join("corrupt.sqlite3");
        let mut connection = open(&path);
        let (mut world, _) = demo_world();
        commit_sections(&mut connection, &mut world, &[]).unwrap();
        // A valid retained ledger must never become the fallback authority.
        connection
            .execute(
                "INSERT INTO domain_events (event_json) VALUES (?1)",
                [serde_json::to_string(&test_event()).unwrap()],
            )
            .unwrap();
        connection.execute_batch(sql).unwrap();
        assert!(
            matches!(
                load_startup_world_recovery(&mut connection),
                Err(StoreError::InvalidRuntimeSnapshot)
            ),
            "{case}"
        );
    }
}

#[tokio::test]
async fn the_store_api_commits_sections_through_its_owner_thread() {
    let directory = tempfile::TempDir::new().unwrap();
    let path = directory.path().join("store.sqlite3");
    let store = SqliteCharacterStore::open(&path).await.unwrap();
    let (mut world, session) = demo_world();
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
    world
        .apply(
            session,
            PlayerAction::Take(ItemSelection::one("blue-glass lantern")),
        )
        .unwrap();
    store
        .commit_action_sections(
            Vec::new(),
            world.runtime_snapshot(),
            world.runtime_dirty_sections(),
            Vec::new(),
        )
        .await
        .unwrap();
    store.shutdown().await.unwrap();
    drop(store);

    let reopened = SqliteCharacterStore::open(&path).await.unwrap();
    assert_eq!(
        reopened.load_startup_world_recovery().await.unwrap(),
        StartupWorldRecovery::Snapshot {
            durable_version: tiamud_core::WORLD_RUNTIME_SNAPSHOT_VERSION,
            snapshot: world.runtime_snapshot(),
        }
    );
}
