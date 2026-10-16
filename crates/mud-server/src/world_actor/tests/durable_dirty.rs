//! Tick persistence with durable dirty tracking (ADR 0027).

use super::*;

fn idle_world(store: &MemoryStore) -> (World, Vec<SessionId>) {
    let mut world = World::demo();
    let sessions = vec![SessionId(301), SessionId(302)];
    connect_tick_character(
        &mut world,
        store,
        sessions[0],
        CharacterId(301),
        "demo:lantern-square",
        "Durable One",
    );
    connect_tick_character(
        &mut world,
        store,
        sessions[1],
        CharacterId(302),
        "demo:north-gate",
        "Durable Two",
    );
    (world, sessions)
}

fn roles(sessions: &[SessionId]) -> HashMap<SessionId, AccountRole> {
    sessions
        .iter()
        .map(|session| (*session, AccountRole::Player))
        .collect()
}

fn commits(store: &MemoryStore) -> usize {
    store.successful_commits.load(Ordering::SeqCst)
}

/// Runs ticks until one is quiet (no weather change), returning the commit
/// count observed for that tick.
async fn quiet_tick_commits(
    world: &mut World,
    store: &MemoryStore,
    roles: &HashMap<SessionId, AccountRole>,
    delivery: &mut DeliveryRouter,
) -> usize {
    let content = RuntimeContent::default();
    for _ in 0..10 {
        let weather_before = world.runtime_snapshot().adventure;
        let before = commits(store);
        assert!(persist_tick(world, &content, roles, store, delivery).await);
        if world.runtime_snapshot().adventure == weather_before {
            assert_eq!(world.runtime_dirty_reasons(), Vec::<&str>::new());
            return commits(store) - before;
        }
    }
    panic!("no quiet tick within ten ticks");
}

#[tokio::test]
async fn clean_tick_writes_nothing_but_still_refreshes_observers() {
    let store = MemoryStore::default();
    let (mut world, sessions) = idle_world(&store);
    let roles = roles(&sessions);
    let mut delivery = DeliveryRouter::new(None);
    let mut inbox = delivery.register(sessions[0]);
    let content = RuntimeContent::default();

    // The first tick has no durable baseline and writes everything.
    assert!(persist_tick(&mut world, &content, &roles, &store, &mut delivery).await);
    assert_eq!(commits(&store), 1);
    assert_eq!(store.characters_written.load(Ordering::SeqCst), 2);
    while inbox.try_recv().is_ok() {}

    let bytes_before = store.bytes_written.load(Ordering::SeqCst);
    assert_eq!(
        quiet_tick_commits(&mut world, &store, &roles, &mut delivery).await,
        0
    );
    assert!(!world.runtime_durably_dirty());
    assert!(world.durable_clock_lags());
    // A quiet tick still advances the clock and refreshes every observer.
    assert!(matches!(
        inbox.try_recv(),
        Ok(crate::delivery::SessionDelivery::World(_))
    ));
    // Weather ticks write the runtime snapshot but no unchanged character.
    let characters_before = store.characters_written.load(Ordering::SeqCst);
    for _ in 0..10 {
        assert!(persist_tick(&mut world, &content, &roles, &store, &mut delivery).await);
    }
    assert_eq!(
        store.characters_written.load(Ordering::SeqCst),
        characters_before
    );
    assert!(store.bytes_written.load(Ordering::SeqCst) >= bytes_before);
}

#[tokio::test]
async fn mutation_is_written_once_and_only_for_the_changed_character() {
    let store = MemoryStore::default();
    let (mut world, sessions) = idle_world(&store);
    let roles = roles(&sessions);
    let mut delivery = DeliveryRouter::new(None);
    let content = RuntimeContent::default();
    assert!(persist_tick(&mut world, &content, &roles, &store, &mut delivery).await);

    // An uncommitted in-memory change (no command commit) is picked up by
    // the next tick, for the changed character only.
    world
        .apply(sessions[0], PlayerAction::Move(Direction::North))
        .unwrap();
    let characters_before = store.characters_written.load(Ordering::SeqCst);
    let runtime_before = store.runtime_snapshots_written.load(Ordering::SeqCst);
    let adventure_before = world.runtime_snapshot().adventure;
    assert!(persist_tick(&mut world, &content, &roles, &store, &mut delivery).await);
    assert_eq!(
        store.characters_written.load(Ordering::SeqCst),
        characters_before + 1
    );
    assert_eq!(
        store
            .state
            .lock()
            .unwrap()
            .characters
            .get(&CharacterId(301))
            .unwrap()
            .room_id,
        "demo:north-gate"
    );
    // Moving does not touch the runtime snapshot: unless the weather changed
    // on that tick, only the character row was written.
    if world.runtime_snapshot().adventure == adventure_before {
        assert_eq!(
            store.runtime_snapshots_written.load(Ordering::SeqCst),
            runtime_before
        );
    }
    // The change is not written again.
    assert_eq!(
        quiet_tick_commits(&mut world, &store, &roles, &mut delivery).await,
        0
    );
}

#[tokio::test]
async fn failed_tick_commit_keeps_state_dirty_and_is_retried_exactly_once() {
    let store = MemoryStore::default();
    let (mut world, sessions) = idle_world(&store);
    let roles = roles(&sessions);
    let mut delivery = DeliveryRouter::new(None);
    let content = RuntimeContent::default();
    let mut gameplay = world.character_gameplay_snapshot(sessions[0]).unwrap();
    gameplay.vitals.health = gameplay.vitals.max_health - 5;
    gameplay.effects.push(ActiveEffect {
        kind: EffectKind::Regeneration,
        magnitude: 1,
        remaining_ticks: 3,
    });
    world
        .restore_character_gameplay(CharacterId(301), gameplay.clone())
        .unwrap();
    assert!(persist_tick(&mut world, &content, &roles, &store, &mut delivery).await);
    let healed = |store: &MemoryStore| {
        store
            .state
            .lock()
            .unwrap()
            .domain_events
            .iter()
            .filter(|event| {
                matches!(
                    event,
                    DomainEvent::Gameplay {
                        event: GameplayEvent::CharacterHealed { .. }
                    }
                )
            })
            .count()
    };
    let health = |world: &World| {
        world
            .character_gameplay_snapshot(sessions[0])
            .unwrap()
            .vitals
            .health
    };
    let stored_health = |store: &MemoryStore| {
        store
            .state
            .lock()
            .unwrap()
            .gameplay
            .get(&CharacterId(301))
            .unwrap()
            .vitals
            .health
    };
    assert_eq!(healed(&store), 1);
    assert_eq!(stored_health(&store), health(&world));
    let before_failure = (world.runtime_snapshot(), health(&world));

    store.fail_saves.store(true, Ordering::SeqCst);
    assert!(!persist_tick(&mut world, &content, &roles, &store, &mut delivery).await);
    // The failed tick is rolled back exactly and nothing is published.
    assert_eq!((world.runtime_snapshot(), health(&world)), before_failure);
    assert_eq!(healed(&store), 1);

    store.fail_saves.store(false, Ordering::SeqCst);
    assert!(persist_tick(&mut world, &content, &roles, &store, &mut delivery).await);
    // The retried tick applies and persists the heal exactly once.
    assert_eq!(healed(&store), 2);
    assert_eq!(health(&world), before_failure.1 + 1);
    assert_eq!(stored_health(&store), health(&world));
    assert_eq!(
        store.state.lock().unwrap().runtime.as_ref().unwrap(),
        &world.runtime_snapshot()
    );
}

#[tokio::test]
async fn clean_ticks_flush_the_clock_within_the_heartbeat_and_on_shutdown() {
    let store = MemoryStore::default();
    let (mut world, sessions) = idle_world(&store);
    let roles = roles(&sessions);
    let mut delivery = DeliveryRouter::new(None);
    let content = RuntimeContent::default();
    let stored_tick = |store: &MemoryStore| {
        store
            .state
            .lock()
            .unwrap()
            .runtime
            .as_ref()
            .unwrap()
            .logical_tick
    };
    for _ in 0..(tiamud_core::DURABLE_CLOCK_HEARTBEAT_TICKS * 2) {
        assert!(persist_tick(&mut world, &content, &roles, &store, &mut delivery).await);
        let lag = world.runtime_snapshot().logical_tick - stored_tick(&store);
        assert!(lag < tiamud_core::DURABLE_CLOCK_HEARTBEAT_TICKS);
    }
    // A failed commit drops the baseline; the next commit rewrites everything.
    world.invalidate_durable_baseline();
    assert!(world.runtime_durably_dirty());
    assert_eq!(world.durable_character_changes().unwrap().len(), 2);
    assert!(persist_tick(&mut world, &content, &roles, &store, &mut delivery).await);
    assert!(!world.runtime_durably_dirty());

    let commits_before = commits(&store);
    quiet_tick_commits(&mut world, &store, &roles, &mut delivery).await;
    assert!(world.durable_clock_lags());
    flush_durable_state_for_shutdown(&mut world, &store).await;
    assert_eq!(stored_tick(&store), world.runtime_snapshot().logical_tick);
    assert!(!world.durable_clock_lags());
    assert!(commits(&store) > commits_before);
    // Nothing left to flush: a second shutdown flush writes nothing.
    let commits_after = commits(&store);
    flush_durable_state_for_shutdown(&mut world, &store).await;
    assert_eq!(commits(&store), commits_after);
}

#[tokio::test]
async fn crash_after_skipped_ticks_recovers_everything_but_a_bounded_clock_lag() {
    let store = MemoryStore::default();
    let (mut world, sessions) = idle_world(&store);
    let roles = roles(&sessions);
    let mut delivery = DeliveryRouter::new(None);
    let content = RuntimeContent::default();
    for index in 0..23 {
        if index == 7 {
            world
                .apply(sessions[1], PlayerAction::Move(Direction::South))
                .unwrap();
        }
        assert!(persist_tick(&mut world, &content, &roles, &store, &mut delivery).await);
    }
    // Simulated crash: no shutdown flush. Recover from what the store holds.
    let state = store.state.lock().unwrap();
    let durable = state.runtime.clone().unwrap();
    let mut recovered = World::demo();
    recovered.restore_runtime_snapshot(durable.clone()).unwrap();
    let mut expected = world.runtime_snapshot();
    let lag = expected.logical_tick - durable.logical_tick;
    assert!(lag < tiamud_core::DURABLE_CLOCK_HEARTBEAT_TICKS);
    expected.logical_tick = durable.logical_tick;
    expected.revision = durable.revision;
    assert_eq!(recovered.runtime_snapshot(), expected);
    for session in &sessions {
        let live = world.durable_character_snapshot(*session).unwrap();
        assert_eq!(
            state.characters.get(&live.character.id),
            Some(&live.character)
        );
        assert_eq!(state.gameplay.get(&live.character.id), Some(&live.gameplay));
    }
}

/// Sectioned runtime commits (ADR 0035): every runtime commit rewrites the
/// sections whose content changed since the previous commit and no other.
/// The memory store additionally proves on every commit that the stored
/// sections assemble to the full snapshot.
#[tokio::test]
async fn runtime_commits_rewrite_only_changed_sections() {
    let store = MemoryStore::default();
    let (mut world, sessions) = idle_world(&store);
    let roles = roles(&sessions);
    let mut delivery = DeliveryRouter::new(None);
    let content = RuntimeContent::default();
    let members = |snapshot: &WorldRuntimeSnapshot| {
        serde_json::to_value(snapshot)
            .unwrap()
            .as_object()
            .unwrap()
            .clone()
    };

    assert!(persist_tick(&mut world, &content, &roles, &store, &mut delivery).await);
    assert_eq!(
        store.last_runtime_sections.lock().unwrap().len(),
        tiamud_core::RuntimeSection::ALL.len()
    );
    let mut committed = members(&world.runtime_snapshot());
    let mut runtime_commits = 0;
    let mut partial_commits = 0;
    for _ in 0..(2 * tiamud_core::DURABLE_CLOCK_HEARTBEAT_TICKS) {
        let before = store.runtime_snapshots_written.load(Ordering::SeqCst);
        assert!(persist_tick(&mut world, &content, &roles, &store, &mut delivery).await);
        if store.runtime_snapshots_written.load(Ordering::SeqCst) == before {
            continue;
        }
        runtime_commits += 1;
        let current = members(&world.runtime_snapshot());
        let written = store
            .last_runtime_sections
            .lock()
            .unwrap()
            .iter()
            .map(|name| (*name).to_owned())
            .collect::<BTreeSet<_>>();
        for section in tiamud_core::RuntimeSection::ALL {
            let name = section.field_name();
            if committed.get(name) != current.get(name) {
                assert!(written.contains(name), "changed {name} was not rewritten");
            } else {
                assert!(!written.contains(name), "unchanged {name} was rewritten");
            }
        }
        if written.len() < tiamud_core::RuntimeSection::ALL.len() {
            partial_commits += 1;
        }
        committed = current;
    }
    // The heartbeat alone forces at least one runtime commit in this window.
    assert!(runtime_commits > 0);
    assert_eq!(partial_commits, runtime_commits);
}
