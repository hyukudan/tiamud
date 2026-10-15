//! Manual hot-path measurement for the world actor.
//!
//! Run with:
//! `cargo test -p tiamud-server --release --locked hot_path_measurement -- --ignored --nocapture`
//!
//! The store is the in-memory test adapter. It serializes exactly the JSON a
//! SQL adapter would write (character gameplay/identity, runtime snapshot and
//! events) so `bytes/commit` reflects the payload, while the reported times
//! exclude real SQL I/O. Numbers are diagnostic, not CI evidence.

use super::*;
use tiamud_core::{RuntimeItemInstanceSnapshot, RuntimeItemLocation};

struct BenchWorld {
    world: World,
    store: MemoryStore,
    sessions: Vec<SessionId>,
    mover: SessionId,
    route: (Direction, Direction),
}

fn bench_world(
    characters: usize,
    staged_items: usize,
    items_per_room: usize,
    carried_per_character: usize,
) -> BenchWorld {
    let loaded = crate::world_loader::default_loaded_world().unwrap();
    let mut world = crate::world_loader::build_loaded_world(&loaded).unwrap();
    let rooms = loaded
        .definition
        .rooms
        .iter()
        .map(|room| room.stable_id.clone())
        .collect::<Vec<_>>();
    // Find a two-way route for the moving session.
    let mut route = None;
    'outer: for room in &loaded.definition.rooms {
        for exit in &room.exits {
            if let Some(target) = loaded
                .definition
                .rooms
                .iter()
                .find(|candidate| candidate.stable_id == exit.target_room_id)
                && let Some(back) = target
                    .exits
                    .iter()
                    .find(|back| back.target_room_id == room.stable_id)
            {
                route = Some((room.stable_id.clone(), exit.direction, back.direction));
                break 'outer;
            }
        }
    }
    let (route_room, there, back) = route.expect("default world has a two-way exit");
    let prototype = loaded
        .definition
        .item_prototypes
        .iter()
        .find(|prototype| {
            prototype.container.is_none()
                && prototype.sealed_loot_table_id.is_none()
                && prototype.transfer_policy == ItemTransferPolicy::Transferable
        })
        .expect("default world has a plain transferable prototype")
        .prototype_id
        .clone();

    let mut snapshot = world.runtime_snapshot();
    let mut sequence = snapshot.next_item_instance_sequence;
    let mut add_item = |snapshot: &mut WorldRuntimeSnapshot, location: RuntimeItemLocation| {
        let id = ItemInstanceId(format!("runtime:{sequence}"));
        sequence += 1;
        snapshot.item_instances.insert(
            id.clone(),
            RuntimeItemInstanceSnapshot {
                prototype_id: prototype.clone(),
                location,
                binding: tiamud_core::ItemBinding::Unbound,
                item_affixes: Vec::new(),
            },
        );
        snapshot.item_provenance.insert(
            id,
            tiamud_core::ItemProvenance::RuntimeCreated { created_tick: 0 },
        );
    };
    // Staged instances model a large world whose items are mostly out of
    // every player's sight: they cost nothing to render but every former
    // full scan walked them.
    for _ in 0..staged_items {
        add_item(&mut snapshot, RuntimeItemLocation::Staged);
    }
    for room in &rooms {
        for _ in 0..items_per_room {
            add_item(
                &mut snapshot,
                RuntimeItemLocation::Room {
                    room_id: room.clone(),
                },
            );
        }
    }
    for character in 0..characters {
        for _ in 0..carried_per_character {
            add_item(
                &mut snapshot,
                RuntimeItemLocation::Carried {
                    character_id: CharacterId(1_000 + character as i64),
                },
            );
        }
    }
    snapshot.next_item_instance_sequence = sequence;
    world.restore_runtime_snapshot(snapshot).unwrap();

    let store = MemoryStore::default();
    let mut sessions = Vec::new();
    for character in 0..characters {
        let session = SessionId(1_000 + character as u64);
        let room = if character == 0 {
            route_room.clone()
        } else {
            rooms[character % rooms.len()].clone()
        };
        let character_id = CharacterId(1_000 + character as i64);
        let snapshot = CharacterSnapshot {
            id: character_id,
            account_id: AccountId(character_id.0),
            name: format!("Bench{character}"),
            room_id: room,
            item_locations: BTreeMap::new(),
        };
        world.connect(session, snapshot.clone()).unwrap();
        let mut state = store.state.lock().unwrap();
        state.characters.insert(character_id, snapshot);
        state
            .gameplay
            .insert(character_id, CharacterGameplaySnapshot::default());
        drop(state);
        sessions.push(session);
    }
    BenchWorld {
        world,
        store,
        mover: sessions[0],
        sessions,
        route: (there, back),
    }
}

fn per_op(total: Duration, count: usize) -> f64 {
    total.as_secs_f64() * 1_000.0 / count as f64
}

#[tokio::test(flavor = "current_thread")]
#[ignore = "manual measurement; run with --ignored --nocapture in release"]
async fn hot_path_measurement() {
    const TICKS: usize = 10;
    const COMMANDS: usize = 20;
    for (characters, staged, per_room, carried) in [(50, 5_000, 5, 10), (200, 5_000, 5, 10)] {
        let BenchWorld {
            mut world,
            store,
            sessions,
            mover,
            route,
        } = bench_world(characters, staged, per_room, carried);
        let roles = sessions
            .iter()
            .map(|session| (*session, AccountRole::Player))
            .collect::<HashMap<_, _>>();
        let content = RuntimeContent::default();
        let mut delivery = DeliveryRouter::new(None);
        let mut inboxes = sessions
            .iter()
            .map(|session| delivery.register(*session))
            .collect::<Vec<_>>();
        let drain = |inboxes: &mut Vec<crate::delivery::DeliveryInbox>| {
            for inbox in inboxes.iter_mut() {
                while inbox.try_recv().is_ok() {}
            }
        };

        // Warm up: one full commit so later ticks start from a durable state.
        assert!(persist_tick(&mut world, &content, &roles, &store, &mut delivery).await);
        drain(&mut inboxes);

        // --- Ticks -------------------------------------------------------
        let commits_before = store.successful_commits.load(Ordering::SeqCst);
        let bytes_before = store.bytes_written.load(Ordering::SeqCst);
        let characters_before = store.characters_written.load(Ordering::SeqCst);
        let runtime_before = store.runtime_snapshots_written.load(Ordering::SeqCst);
        let mut tick_total = Duration::ZERO;
        let mut tick_max = Duration::ZERO;
        for _ in 0..TICKS {
            let started = Instant::now();
            assert!(persist_tick(&mut world, &content, &roles, &store, &mut delivery).await);
            let elapsed = started.elapsed();
            tick_total += elapsed;
            tick_max = tick_max.max(elapsed);
            drain(&mut inboxes);
        }
        let tick_commits = store.successful_commits.load(Ordering::SeqCst) - commits_before;
        let tick_bytes = store.bytes_written.load(Ordering::SeqCst) - bytes_before;
        let tick_characters = store.characters_written.load(Ordering::SeqCst) - characters_before;
        let tick_runtime = store.runtime_snapshots_written.load(Ordering::SeqCst) - runtime_before;

        // --- Checkpoint / scene primitives -------------------------------
        let started = Instant::now();
        for _ in 0..COMMANDS {
            std::hint::black_box(world.checkpoint());
        }
        let checkpoint_ms = per_op(started.elapsed(), COMMANDS);
        let started = Instant::now();
        for session in &sessions {
            std::hint::black_box(world.current_scene(*session).unwrap());
        }
        let scene_ms = per_op(started.elapsed(), sessions.len());
        let started = Instant::now();
        for session in &sessions {
            std::hint::black_box(world.player_state_view(*session).unwrap());
        }
        let player_ms = per_op(started.elapsed(), sessions.len());

        // --- Commands (full actor apply path, durable commit included) ----
        let mut social = SocialRuntime::new();
        let mut command_roles = roles.clone();
        let mut staff = HashMap::new();
        let commits_before = store.successful_commits.load(Ordering::SeqCst);
        let bytes_before = store.bytes_written.load(Ordering::SeqCst);
        let characters_before = store.characters_written.load(Ordering::SeqCst);
        let mut command_total = Duration::ZERO;
        for index in 0..COMMANDS {
            let direction = if index % 2 == 0 { route.0 } else { route.1 };
            let started = Instant::now();
            apply(
                ApplyResources {
                    world: &mut world,
                    runtime_content: &content,
                    social: &mut social,
                    store: &store,
                    session_roles: &mut command_roles,
                    session_staff_accesses: &mut staff,
                    delivery: &mut delivery,
                },
                mover,
                AccountRole::Player,
                PlayerAction::Move(direction),
            )
            .await
            .unwrap();
            command_total += started.elapsed();
            drain(&mut inboxes);
        }
        let command_commits = store.successful_commits.load(Ordering::SeqCst) - commits_before;
        let command_bytes = store.bytes_written.load(Ordering::SeqCst) - bytes_before;
        let command_characters =
            store.characters_written.load(Ordering::SeqCst) - characters_before;

        let items = world.runtime_snapshot().item_instances.len();
        println!(
            "HOTPATH characters={characters} items={items} | tick: mean={:.3}ms max={:.3}ms \
             commits={tick_commits}/{TICKS} runtime_snapshots={tick_runtime} \
             characters_written={tick_characters} bytes/tick={} | checkpoint={:.4}ms \
             scene={:.4}ms player_state={:.4}ms | move command: mean={:.3}ms \
             commits={command_commits}/{COMMANDS} characters_written={command_characters} \
             bytes/commit={}",
            per_op(tick_total, TICKS),
            tick_max.as_secs_f64() * 1_000.0,
            tick_bytes / TICKS,
            checkpoint_ms,
            scene_ms,
            player_ms,
            per_op(command_total, COMMANDS),
            command_bytes / command_commits.max(1),
        );
    }
}
