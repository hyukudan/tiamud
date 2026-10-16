//! Shared story instances through the manager and the instance task
//! (ADR 0033): joining, votes and their window on a paused clock, forged
//! and stale votes, spectators, host-only actions, host migration, seat
//! grace and reconnection, invitations, turns and the host's autosave.

use super::*;
use crate::persistence::SqliteCharacterStore;
use std::path::PathBuf;
use tempfile::TempDir;

const CRIPTA: &str = "cripta-del-nigromante";
const HOST: AccountId = AccountId(1);
const BRUNO: AccountId = AccountId(2);
const CARLA: AccountId = AccountId(3);

fn cripta() -> StoryPack {
    let bytes = std::fs::read(
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../story-core/fixtures/cripta-del-nigromante.story.json"),
    )
    .unwrap();
    StoryPack::from_json_slice(&bytes).unwrap()
}

fn stories(shared: SharedLimits) -> StoryManager {
    StoryManager::new(
        StoryCatalog::from_packs(vec![cripta()], BTreeMap::new()).unwrap(),
        StoryLimits {
            shared,
            ..StoryLimits::default()
        },
    )
}

fn limits() -> SharedLimits {
    SharedLimits {
        max_seats: 2,
        max_spectators: 1,
        vote_seconds: 30,
        grace: Duration::from_secs(60),
    }
}

fn stories_for_replay() -> StoryManager {
    stories(limits())
}

/// Host shares a seeded instance; Bruno joins seated.
async fn table_of_two(stories: &StoryManager) -> (StoryLink, StoryLink, String) {
    let host = stories
        .enter_seeded(HOST, CRIPTA, 7, Some(1))
        .unwrap()
        .share("Aria")
        .await
        .unwrap();
    let code = host.frames().unwrap().borrow().lobby(HOST).code.unwrap();
    let bruno = stories.join(BRUNO, "Bruno", &code, false).await.unwrap();
    (host, bruno, code)
}

fn frame(link: &StoryLink) -> Arc<SharedFrame> {
    link.frames().unwrap().borrow().clone()
}

/// Lets the instance task run its pending ticks.
async fn settle() {
    for _ in 0..8 {
        tokio::task::yield_now().await;
    }
}

#[tokio::test(start_paused = true)]
async fn a_majority_vote_applies_one_action_that_every_participant_sees() {
    let stories = stories(limits());
    let (host, bruno, code) = table_of_two(&stories).await;
    let carla = stories.join(CARLA, "Carla", &code, true).await.unwrap();
    assert_eq!(carla.role(), Some(StorySeatRole::Spectator));
    assert_eq!(bruno.role(), Some(StorySeatRole::Player));
    let mut watcher = carla.frames().unwrap();

    let view = host.view().await.unwrap();
    let actions = view.actions();
    let first = actions[0].clone();
    let rev = view.view_rev;

    // Spectators are read-only.
    assert_eq!(
        carla.input(first.request(), rev).await,
        Err(StoryFailure::Spectator)
    );
    assert_eq!(
        carla.vote(Some(first.leaf_id().to_owned()), rev).await,
        Err(StoryFailure::Spectator)
    );
    // Forged, stale and wrong-shape votes change nothing.
    assert_eq!(
        bruno.vote(Some("c0123456789abcdef".into()), rev).await,
        Err(StoryFailure::UnknownChoice)
    );
    assert_eq!(
        bruno.vote(Some("../etc".into()), rev).await,
        Err(StoryFailure::UnknownChoice)
    );
    assert_eq!(
        bruno.vote(Some(first.leaf_id().to_owned()), rev + 1).await,
        Err(StoryFailure::StaleView)
    );
    assert_eq!(frame(&host).tally(HOST).voted, 0);

    // The host's number counts as a vote; the view does not change yet.
    let after_host = host.input(first.request(), rev).await.unwrap();
    assert_eq!(after_host.view_rev, rev);
    let tally = frame(&bruno).tally(BRUNO);
    assert_eq!((tally.voted, tally.eligible), (1, 2));
    assert_eq!(tally.your_vote, None);
    assert_eq!(
        frame(&host).tally(HOST).your_vote.as_deref(),
        Some(first.leaf_id())
    );
    assert_eq!(tally.remaining_seconds, Some(30));
    assert_eq!(
        tally.choices,
        [protocol::StoryTallyEntry {
            choice_id: first.leaf_id().to_owned(),
            votes: 1
        }]
    );

    // Bruno votes too: everyone seated voted, the decision applies once.
    let decided = bruno
        .vote(Some(first.leaf_id().to_owned()), rev)
        .await
        .unwrap();
    assert_eq!(decided.view_rev, rev + 1);
    assert_eq!(decided.turn, 1);
    watcher.changed().await.unwrap();
    let seen = watcher.borrow_and_update().clone();
    assert_eq!(seen.view.view_rev, rev + 1);
    assert_eq!(seen.tally(CARLA).voted, 0);
    // A late vote for the closed view is stale.
    assert_eq!(
        bruno.vote(Some(first.leaf_id().to_owned()), rev).await,
        Err(StoryFailure::StaleView)
    );

    // Only the host undoes; others see `canUndo: false`.
    assert!(seen.view_for(HOST, seen.view.clone()).can_undo);
    assert!(!seen.view_for(BRUNO, seen.view.clone()).can_undo);
    assert_eq!(bruno.undo().await, Err(StoryFailure::HostOnly));
    assert_eq!(host.undo().await.unwrap().turn, 0);
    // Save slots belong to the host.
    assert_eq!(bruno.save(1).await, Err(StoryFailure::HostOnly));
    assert_eq!(carla.slots().await, Err(StoryFailure::HostOnly));
    assert_eq!(bruno.restore(1).await, Err(StoryFailure::HostOnly));
    assert_eq!(host.slots().await, Err(StoryFailure::SlotsUnavailable));
    // Host-only management.
    assert_eq!(bruno.kick("Carla").await, Err(StoryFailure::HostOnly));
    assert_eq!(
        bruno.set_mode(StoryDecisionMode::Host, None).await,
        Err(StoryFailure::HostOnly)
    );
    assert_eq!(bruno.invite("Diego").await, Err(StoryFailure::HostOnly));
    assert!(
        stories
            .prometheus()
            .contains("tiamud_story_shared_decisions_total 1")
    );
}

#[tokio::test(start_paused = true)]
async fn the_vote_window_closes_on_the_clock_and_votes_can_change() {
    let stories = stories(limits());
    let (host, bruno, _) = table_of_two(&stories).await;
    // Reach the first view with two actions, deciding alone in host mode.
    host.set_mode(StoryDecisionMode::Host, None).await.unwrap();
    let mut view = host.view().await.unwrap();
    let mut played = Vec::new();
    while view.actions().len() < 2 {
        let action = view.actions()[0].clone();
        played.push(action.label().to_owned());
        view = host.input(action.request(), view.view_rev).await.unwrap();
    }
    host.set_mode(StoryDecisionMode::Vote, None).await.unwrap();
    let actions = view.actions();
    let rev = view.view_rev;
    bruno
        .vote(Some(actions[0].leaf_id().to_owned()), rev)
        .await
        .unwrap();
    // Changing the vote before the close is allowed; withdrawing too.
    bruno
        .vote(Some(actions[1].leaf_id().to_owned()), rev)
        .await
        .unwrap();
    bruno.vote(None, rev).await.unwrap();
    assert_eq!(frame(&host).tally(HOST).remaining_seconds, None);
    bruno
        .vote(Some(actions[1].leaf_id().to_owned()), rev)
        .await
        .unwrap();

    tokio::time::sleep(Duration::from_secs(29)).await;
    settle().await;
    assert_eq!(frame(&host).view.view_rev, rev, "still open");
    tokio::time::sleep(Duration::from_secs(2)).await;
    settle().await;
    // The host abstained: Bruno's single vote decides.
    let closed = frame(&host);
    assert_eq!(closed.view.view_rev, rev + 1);
    let expected = {
        let replay = stories_for_replay();
        let link = replay
            .enter_seeded(AccountId(50), CRIPTA, 7, Some(1))
            .unwrap();
        let mut first = link.view().await.unwrap();
        for label in &played {
            let action = first
                .actions()
                .into_iter()
                .find(|action| action.label() == label)
                .unwrap();
            first = link.input(action.request(), first.view_rev).await.unwrap();
        }
        let second = first.actions()[1].clone();
        link.input(second.request(), first.view_rev).await.unwrap()
    };
    assert_eq!(closed.view.title, expected.title);
    assert_eq!(closed.view.turn, expected.turn);
}

#[tokio::test(start_paused = true)]
async fn the_host_migrates_and_the_instance_ends_with_the_last_seat() {
    let stories = stories(limits());
    let (host, bruno, code) = table_of_two(&stories).await;
    let mut watcher = bruno.frames().unwrap();
    assert!(!host.leave().await.unwrap().autosaved);
    watcher.changed().await.unwrap();
    let after = watcher.borrow_and_update().clone();
    assert_eq!(after.table.host(), BRUNO);
    assert_eq!(after.lobby(BRUNO).code.as_deref(), Some(code.as_str()));
    assert_eq!(
        after.removal(HOST),
        Some(protocol::StoryLeftReason::Evicted)
    );
    assert!(stories.attach(HOST).is_none());
    // The former host can rejoin with the code, now as a player.
    let back = stories.join(HOST, "Aria", &code, false).await.unwrap();
    assert_eq!(back.role(), Some(StorySeatRole::Player));
    back.leave().await.unwrap();

    bruno.leave().await.unwrap();
    assert_eq!(stories.live_instances(), 0);
    let last = watcher.borrow_and_update().clone();
    assert_eq!(last.removal(BRUNO), Some(protocol::StoryLeftReason::Ended));
    assert!(watcher.changed().await.is_err(), "the frames end");
    assert_eq!(
        stories.join(CARLA, "Carla", &code, false).await.err(),
        Some(StoryFailure::UnknownCode)
    );
}

#[tokio::test(start_paused = true)]
async fn a_disconnected_seat_is_held_for_the_grace_period() {
    let stories = stories(limits());
    let (host, bruno, _) = table_of_two(&stories).await;
    drop(bruno);
    tokio::time::sleep(Duration::from_secs(1)).await;
    settle().await;
    let lobby = frame(&host).lobby(HOST);
    assert!(!lobby.seats[1].connected, "held while disconnected");
    // Reconnecting within the grace period resumes the same seat.
    let bruno = stories.attach(BRUNO).unwrap();
    tokio::time::sleep(Duration::from_secs(1)).await;
    settle().await;
    assert!(frame(&host).lobby(HOST).seats[1].connected);
    assert_eq!(bruno.role(), Some(StorySeatRole::Player));

    // Away longer than the grace period: the seat lapses.
    drop(bruno);
    tokio::time::sleep(Duration::from_secs(62)).await;
    settle().await;
    assert_eq!(frame(&host).lobby(HOST).seats.len(), 1);
    assert!(stories.attach(BRUNO).is_none());
    assert!(
        stories
            .prometheus()
            .contains("tiamud_story_shared_seat_lapses_total 1")
    );

    // The host's own lapse ends the instance.
    drop(host);
    tokio::time::sleep(Duration::from_secs(62)).await;
    settle().await;
    assert_eq!(stories.live_instances(), 0);
}

#[tokio::test(start_paused = true)]
async fn joining_is_bounded_and_kicked_players_stay_out() {
    let stories = stories(limits());
    let (host, bruno, code) = table_of_two(&stories).await;
    // Seats are full (2); one spectator place.
    assert_eq!(
        stories.join(CARLA, "Carla", &code, false).await.err(),
        Some(StoryFailure::SeatsFull)
    );
    assert!(
        stories.attach(CARLA).is_none(),
        "a refused join leaves no route"
    );
    let carla = stories.join(CARLA, "Carla", &code, true).await.unwrap();
    assert_eq!(
        stories.join(AccountId(4), "Diego", &code, true).await.err(),
        Some(StoryFailure::SeatsFull)
    );
    // Already in a story.
    assert_eq!(
        stories.enter(BRUNO, CRIPTA).map(|link| link.is_shared()),
        Ok(true),
        "re-entering the same story resumes the shared seat"
    );
    assert_eq!(
        stories
            .join(BRUNO, "Bruno", &code, false)
            .await
            .map(|link| link.role()),
        Ok(Some(StorySeatRole::Player))
    );
    // Kicking: the host cannot kick itself; unknown names are typed.
    assert_eq!(host.kick("Aria").await, Err(StoryFailure::UnknownMember));
    assert_eq!(host.kick("Nadie").await, Err(StoryFailure::UnknownMember));
    host.kick("carla").await.unwrap();
    assert_eq!(
        frame(&host).removal(CARLA),
        Some(protocol::StoryLeftReason::Kicked)
    );
    drop(carla);
    assert_eq!(
        stories.join(CARLA, "Carla", &code, true).await.err(),
        Some(StoryFailure::UnknownCode)
    );
    // Unknown and malformed codes look the same; attempts are rate limited.
    let eve = AccountId(5);
    assert_eq!(
        stories.join(eve, "Eva", "ZZZZZZZZ", false).await.err(),
        Some(StoryFailure::UnknownCode)
    );
    assert_eq!(
        stories.join(eve, "Eva", "not-a-code", false).await.err(),
        Some(StoryFailure::UnknownCode)
    );
    for _ in 0..3 {
        let _ = stories.join(eve, "Eva", "ZZZZZZZZ", false).await;
    }
    assert_eq!(
        stories.join(eve, "Eva", &code, true).await.err(),
        Some(StoryFailure::RateLimited)
    );
    drop(bruno);
}

#[tokio::test(start_paused = true)]
async fn invitations_carry_the_code_to_connected_characters_only() {
    let stories = stories(limits());
    let mut inbox = stories.register_inbox("Bruno").unwrap();
    let solo = stories.enter_seeded(HOST, CRIPTA, 7, Some(1)).unwrap();
    assert_eq!(solo.invite("Bruno").await, Err(StoryFailure::NotShared));
    let host = solo.share("Aria").await.unwrap();
    // Unknown names are accepted silently: no presence oracle.
    host.invite("Nadie").await.unwrap();
    host.invite("bruno").await.unwrap();
    let invitation = inbox.recv().await.unwrap();
    assert_eq!(invitation.host, "Aria");
    assert_eq!(invitation.story_id, CRIPTA);
    assert_eq!(Some(invitation.code.clone()), frame(&host).lobby(HOST).code);
    let bruno = stories
        .join(BRUNO, "Bruno", &invitation.code.to_lowercase(), false)
        .await
        .unwrap();
    assert_eq!(bruno.role(), Some(StorySeatRole::Player));
    // Invitations are rate limited per host.
    let mut limited = false;
    for _ in 0..INVITES_PER_MINUTE {
        limited |= host.invite("Carla").await == Err(StoryFailure::RateLimited);
    }
    assert!(limited);
    // Dropping the inbox unregisters it.
    drop(inbox);
    assert!(stories.inner.inboxes.lock().unwrap().is_empty());
}

#[tokio::test(start_paused = true)]
async fn turns_mode_rotates_who_decides() {
    let stories = stories(limits());
    let (host, bruno, _) = table_of_two(&stories).await;
    assert_eq!(
        host.set_mode(StoryDecisionMode::Turns, Some(10)).await,
        Err(StoryFailure::InvalidSetting)
    );
    host.set_mode(StoryDecisionMode::Turns, None).await.unwrap();
    let view = host.view().await.unwrap();
    let action = view.actions()[0].clone();
    assert_eq!(
        bruno.input(action.request(), view.view_rev).await,
        Err(StoryFailure::NotYourTurn)
    );
    let next = host.input(action.request(), view.view_rev).await.unwrap();
    assert_eq!(next.view_rev, view.view_rev + 1);
    assert_eq!(frame(&host).table.turn_name(), Some("Bruno"));
    let action = next.actions()[0].clone();
    assert_eq!(
        host.input(action.request(), next.view_rev).await,
        Err(StoryFailure::NotYourTurn)
    );
    bruno.input(action.request(), next.view_rev).await.unwrap();
    assert_eq!(frame(&host).table.turn_name(), Some("Aria"));

    // Host mode: only the host's input applies.
    host.set_mode(StoryDecisionMode::Host, None).await.unwrap();
    let view = host.view().await.unwrap();
    let action = view.actions()[0].clone();
    assert_eq!(
        bruno.input(action.request(), view.view_rev).await,
        Err(StoryFailure::HostOnly)
    );
    bruno
        .vote(Some(action.leaf_id().to_owned()), view.view_rev)
        .await
        .unwrap();
    assert_eq!(frame(&host).tally(HOST).voted, 1, "advisory vote");
    assert_eq!(
        host.input(action.request(), view.view_rev)
            .await
            .unwrap()
            .view_rev,
        view.view_rev + 1
    );
}

#[tokio::test(start_paused = true)]
async fn solo_instances_refuse_shared_operations_and_others_cannot_share() {
    let stories = stories(limits());
    let solo = stories.enter_seeded(HOST, CRIPTA, 7, Some(1)).unwrap();
    let view = solo.view().await.unwrap();
    assert_eq!(
        solo.vote(Some(view.actions()[0].leaf_id().to_owned()), view.view_rev)
            .await,
        Err(StoryFailure::NotShared)
    );
    assert_eq!(solo.kick("x").await, Err(StoryFailure::NotShared));
    assert_eq!(
        solo.set_mode(StoryDecisionMode::Vote, None).await,
        Err(StoryFailure::NotShared)
    );
    assert!(!solo.is_shared());
    assert_eq!(solo.role(), None);
    // Sharing twice is a no-op that keeps the code.
    let shared = solo.share("Aria").await.unwrap();
    let code = frame(&shared).lobby(HOST).code;
    let again = shared.share("Aria").await.unwrap();
    assert_eq!(frame(&again).lobby(HOST).code, code);
    // Capacity: shared instances count against the instance cap.
    assert_eq!(stories.live_instances(), 1);
    assert!(
        stories
            .prometheus()
            .contains("tiamud_story_shared_instances 1")
    );
}

#[tokio::test]
async fn the_autosave_of_a_shared_instance_belongs_to_its_last_host() {
    let directory = TempDir::new().unwrap();
    let store = SqliteCharacterStore::open(directory.path().join("state.sqlite3"))
        .await
        .unwrap();
    let aria = store.load_or_create_account("Aria").await.unwrap().id;
    let bruno_account = store.load_or_create_account("Bruno").await.unwrap().id;
    let store: Arc<dyn CharacterStore> = Arc::new(store);
    let stories = stories(limits());
    stories.attach_store(store.clone());

    let host = stories
        .enter_seeded(aria, CRIPTA, 7, Some(1))
        .unwrap()
        .share("Aria")
        .await
        .unwrap();
    let code = frame(&host).lobby(aria).code.unwrap();
    let bruno = stories
        .join(bruno_account, "Bruno", &code, false)
        .await
        .unwrap();
    let view = host.view().await.unwrap();
    let action = view.actions()[0].clone();
    host.input(action.request(), view.view_rev).await.unwrap();
    bruno.input(action.request(), view.view_rev).await.unwrap();
    // The host saves to its own account; the guest cannot.
    assert_eq!(host.save(2).await.unwrap().slot, 2);
    assert_eq!(bruno.save(2).await, Err(StoryFailure::HostOnly));
    // The host leaves first: no autosave yet (the instance lives on).
    assert!(!host.leave().await.unwrap().autosaved);
    // Bruno, now host and last seat, closes it: the autosave is his.
    assert!(bruno.leave().await.unwrap().autosaved);
    let bruno_slots = store.list_story_slots(bruno_account, CRIPTA).await.unwrap();
    assert!(bruno_slots.iter().any(|slot| slot.slot.is_autosave()));
    let aria_slots = store.list_story_slots(aria, CRIPTA).await.unwrap();
    assert!(aria_slots.iter().all(|slot| !slot.slot.is_autosave()));
    assert_eq!(aria_slots.len(), 1);
}

#[test]
fn shared_configuration_fails_closed() {
    let mut config = story_config(Some("stories".into()), None, None).unwrap();
    apply_shared_story_env(
        &mut config,
        &SharedStoryEnv {
            max_seats: Some("4".into()),
            max_spectators: Some("0".into()),
            vote_seconds: Some("45".into()),
            grace_seconds: Some("300".into()),
        },
    )
    .unwrap();
    assert_eq!(
        config.limits.shared,
        SharedLimits {
            max_seats: 4,
            max_spectators: 0,
            vote_seconds: 45,
            grace: Duration::from_secs(300),
        }
    );
    for (env, error) in [
        (
            SharedStoryEnv {
                max_seats: Some("1".into()),
                ..SharedStoryEnv::default()
            },
            StoryConfigError::InvalidMaxSeats,
        ),
        (
            SharedStoryEnv {
                max_seats: Some("9".into()),
                ..SharedStoryEnv::default()
            },
            StoryConfigError::InvalidMaxSeats,
        ),
        (
            SharedStoryEnv {
                max_spectators: Some("33".into()),
                ..SharedStoryEnv::default()
            },
            StoryConfigError::InvalidMaxSpectators,
        ),
        (
            SharedStoryEnv {
                vote_seconds: Some("29".into()),
                ..SharedStoryEnv::default()
            },
            StoryConfigError::InvalidVoteSeconds,
        ),
        (
            SharedStoryEnv {
                grace_seconds: Some("601".into()),
                ..SharedStoryEnv::default()
            },
            StoryConfigError::InvalidGraceSeconds,
        ),
    ] {
        let mut config = story_config(Some("stories".into()), None, None).unwrap();
        assert_eq!(apply_shared_story_env(&mut config, &env), Err(error));
    }
    let mut disabled = story_config(None, None, None).unwrap();
    assert_eq!(
        apply_shared_story_env(
            &mut disabled,
            &SharedStoryEnv {
                vote_seconds: Some("60".into()),
                ..SharedStoryEnv::default()
            }
        ),
        Err(StoryConfigError::LimitsWithoutDirectory)
    );
    let mut untouched = story_config(None, None, None).unwrap();
    apply_shared_story_env(&mut untouched, &SharedStoryEnv::default()).unwrap();
    assert_eq!(untouched.limits, StoryLimits::default());
}

#[test]
fn shared_commands_parse_in_both_languages() {
    for (line, expected) in [
        (
            "historia compartir cripta",
            StoryCommand::Host("cripta".into(), None),
        ),
        (
            "story host cripta",
            StoryCommand::Host("cripta".into(), None),
        ),
        (
            "historia unirse abcd2345",
            StoryCommand::Join {
                code: "abcd2345".into(),
                spectate: false,
            },
        ),
        (
            "STORY WATCH ABCD2345",
            StoryCommand::Join {
                code: "abcd2345".into(),
                spectate: true,
            },
        ),
    ] {
        assert_eq!(parse_story_command(line), Some(expected), "{line}");
    }
    for (line, expected) in [
        ("votar 2", StoryInputCommand::Vote(Some(2))),
        ("vote 0", StoryInputCommand::Vote(Some(0))),
        ("vote", StoryInputCommand::Vote(None)),
        ("votar x", StoryInputCommand::Vote(None)),
        ("asientos", StoryInputCommand::Seats),
        ("seats", StoryInputCommand::Seats),
        ("compartir", StoryInputCommand::Share),
        ("invitar Bruno", StoryInputCommand::Invite("Bruno".into())),
        ("kick Carla", StoryInputCommand::Kick("Carla".into())),
        (
            "modo turnos",
            StoryInputCommand::Mode(Some((StoryDecisionMode::Turns, None))),
        ),
        (
            "mode vote 45",
            StoryInputCommand::Mode(Some((StoryDecisionMode::Vote, Some(45)))),
        ),
        (
            "modo anfitrión",
            StoryInputCommand::Mode(Some((StoryDecisionMode::Host, None))),
        ),
        ("modo caos", StoryInputCommand::Mode(None)),
        ("mode vote 99999", StoryInputCommand::Mode(None)),
    ] {
        assert_eq!(parse_story_input(line), expected, "{line}");
    }
}

#[test]
fn tallies_and_lobbies_render_as_text() {
    let now = tokio::time::Instant::now();
    let mut table = Table::new(HOST, "Aria", limits(), "ABCD2345".into(), 1);
    table.join(BRUNO, "Bruno", false).unwrap();
    let view = StoryProjection {
        story_id: CRIPTA.into(),
        title: "Cripta".into(),
        view_rev: 1,
        turn: 0,
        score: 0,
        ending: None,
        can_undo: false,
        items: vec![
            StoryItem::Choice {
                id: "c0000000000000001".into(),
                label: "Norte".into(),
            },
            StoryItem::Choice {
                id: "c0000000000000002".into(),
                label: "Sur".into(),
            },
        ],
    };
    table.vote(BRUNO, Some(1), now).unwrap();
    let frame = SharedFrame {
        view: view.clone(),
        table: table.clone(),
        closed: None,
    };
    let text = render_tally(&frame.tally_at(BRUNO, now), &view).unwrap();
    assert_eq!(text, "Votos (1/2): 2) 1 · cierra en 30 s · tu voto: 2");
    let lobby = render_lobby(&frame.lobby(HOST));
    assert!(lobby.contains("Código para unirse: ABCD2345"));
    assert!(lobby.contains("1. Aria (anfitrión)"));
    assert!(!render_lobby(&frame.lobby(BRUNO)).contains("ABCD2345"));
}
