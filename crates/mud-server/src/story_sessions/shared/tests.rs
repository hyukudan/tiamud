//! Pure tests of the shared-instance table with an injected clock.

use super::*;

const HOST: AccountId = AccountId(1);
const BRUNO: AccountId = AccountId(2);
const CARLA: AccountId = AccountId(3);
const DIEGO: AccountId = AccountId(4);

fn limits() -> SharedLimits {
    SharedLimits {
        max_seats: 3,
        max_spectators: 1,
        vote_seconds: 30,
        grace: Duration::from_secs(60),
    }
}

fn seated() -> (Table, Instant) {
    let now = Instant::now();
    let mut table = Table::new(HOST, "Aria", limits(), "ABCD2345".into(), 1);
    table.join(BRUNO, "Bruno", false).unwrap();
    table.join(CARLA, "Carla", false).unwrap();
    (table, now)
}

fn all(_: AccountId) -> bool {
    true
}

#[test]
fn join_codes_are_normalized_and_bounded() {
    assert_eq!(join_code(0), "22222222");
    assert_eq!(join_code(u64::MAX).len(), STORY_JOIN_CODE_BYTES);
    assert_eq!(normalize_join_code(" abcd2345 "), Some("ABCD2345".into()));
    for rejected in [
        "ABCD234",
        "ABCD23456",
        "ABCD234O",
        "ABCD2341",
        "ABCD 345",
        "",
    ] {
        assert_eq!(normalize_join_code(rejected), None, "{rejected:?}");
    }
    assert!(valid_vote_seconds(30) && valid_vote_seconds(120));
    assert!(!valid_vote_seconds(29) && !valid_vote_seconds(121));
}

#[test]
fn seats_and_spectators_are_bounded_and_spectators_cannot_act() {
    let (mut table, now) = seated();
    assert_eq!(
        table.join(DIEGO, "Diego", false),
        Err(StoryFailure::SeatsFull)
    );
    table.join(DIEGO, "Diego", true).unwrap();
    assert_eq!(
        table.join(AccountId(5), "Eva", true),
        Err(StoryFailure::SeatsFull)
    );
    // Rejoining is a no-op.
    table.join(BRUNO, "Bruno", false).unwrap();
    assert_eq!(table.seats().len(), 3);
    assert_eq!(table.role(DIEGO), Some(StorySeatRole::Spectator));
    assert_eq!(table.route(DIEGO), Err(StoryFailure::Spectator));
    assert_eq!(
        table.vote(DIEGO, Some(0), now),
        Err(StoryFailure::Spectator)
    );
    assert_eq!(table.require_host(DIEGO), Err(StoryFailure::HostOnly));
    assert_eq!(
        table.set_mode(BRUNO, StoryDecisionMode::Host, None, now),
        Err(StoryFailure::HostOnly)
    );
    assert_eq!(table.route(AccountId(99)), Err(StoryFailure::NotInStory));
    // A very long name is cut on a character boundary.
    let mut long = Table::new(HOST, &"ñ".repeat(100), limits(), "X".into(), 1);
    assert!(long.seats()[0].name.len() <= MAX_MEMBER_NAME_BYTES);
    long.join(BRUNO, "b", true).unwrap();
    let lobby = long.lobby(BRUNO);
    assert_eq!(lobby.code, None);
    assert_eq!(long.lobby(HOST).code.as_deref(), Some("X"));
}

#[test]
fn vote_closes_early_when_every_connected_seat_voted() {
    let (mut table, now) = seated();
    assert_eq!(table.route(BRUNO), Ok(Route::Vote));
    assert_eq!(table.vote(HOST, Some(1), now), Ok(None));
    assert_eq!(table.remaining_seconds(now), Some(30));
    // Changing a vote before the close is allowed.
    assert_eq!(table.vote(HOST, Some(0), now), Ok(None));
    assert_eq!(table.vote(BRUNO, Some(2), now), Ok(None));
    let tally = table.tally(BRUNO, 1, &["a".into(), "b".into(), "c".into()], now);
    assert_eq!(tally.voted, 2);
    assert_eq!(tally.eligible, 3);
    assert_eq!(tally.your_vote.as_deref(), Some("c"));
    assert_eq!(
        tally
            .choices
            .iter()
            .map(|entry| (entry.choice_id.as_str(), entry.votes))
            .collect::<Vec<_>>(),
        [("a", 1), ("c", 1)]
    );
    // The last vote closes; the 1-1-1 tie goes to the host's vote.
    assert_eq!(table.vote(CARLA, Some(1), now), Ok(Some(0)));
}

#[test]
fn ties_go_to_the_host_vote_or_to_the_lowest_action() {
    let (mut table, now) = seated();
    table.vote(BRUNO, Some(3), now).unwrap();
    table.vote(CARLA, Some(1), now).unwrap();
    // The host abstains: lowest action number wins the tie.
    assert_eq!(table.winner(), Some(1));
    table.vote(HOST, Some(3), now).unwrap();
    assert_eq!(table.winner(), Some(3));
    // The host's vote only breaks ties it is part of.
    let (mut table, now) = table_with_votes(&[(HOST, 0), (BRUNO, 2), (CARLA, 2)]);
    assert_eq!(table.winner(), Some(2));
    table.vote(CARLA, Some(0), now).unwrap();
    assert_eq!(table.winner(), Some(0));
    // Deterministic across repetitions.
    for _ in 0..16 {
        let (table, _) = table_with_votes(&[(BRUNO, 5), (CARLA, 4)]);
        assert_eq!(table.winner(), Some(4));
    }
}

fn table_with_votes(votes: &[(AccountId, usize)]) -> (Table, Instant) {
    let (mut table, now) = seated();
    for (account, ordinal) in votes {
        table.vote(*account, Some(*ordinal), now).unwrap();
    }
    (table, now)
}

#[test]
fn the_window_closes_at_the_deadline_and_abstentions_do_not_block() {
    let (mut table, now) = seated();
    table.vote(BRUNO, Some(2), now).unwrap();
    let early = table.tick(now + Duration::from_secs(29), all);
    assert_eq!(early.winner, None);
    assert_eq!(
        table.remaining_seconds(now + Duration::from_millis(29_001)),
        Some(1)
    );
    let late = table.tick(now + Duration::from_secs(30), all);
    assert_eq!(late.winner, Some(2));
    // Withdrawing every vote clears the window: nothing closes on its own.
    let (mut table, now) = table_with_votes(&[(BRUNO, 1)]);
    table.vote(BRUNO, None, now).unwrap();
    assert_eq!(table.remaining_seconds(now), None);
    assert_eq!(table.tick(now + Duration::from_secs(300), all).winner, None);
    // A new view voids the votes of the old one.
    let (mut table, now) = table_with_votes(&[(BRUNO, 1)]);
    table.on_new_view(2, true, now);
    let tally = table.tally(BRUNO, 2, &["a".into(), "b".into()], now);
    assert_eq!(
        (tally.voted, tally.your_vote, tally.remaining_seconds),
        (0, None, None)
    );
}

#[test]
fn disconnected_seats_do_not_count_and_lapse_after_the_grace() {
    let (mut table, now) = seated();
    table.vote(HOST, Some(1), now).unwrap();
    table.vote(BRUNO, Some(1), now).unwrap();
    // Carla disconnects: the remaining connected seats all voted.
    let tick = table.tick(now, |account| account != CARLA);
    assert_eq!(tick.winner, Some(1));
    assert!(tick.changed);
    assert!(!table.lobby(HOST).seats[2].connected);
    // She reconnects within the grace period and keeps her seat.
    let tick = table.tick(now + Duration::from_secs(59), all);
    assert!(tick.removed.is_empty() && tick.changed);
    assert!(table.lobby(HOST).seats[2].connected);
    // Away for the whole grace period: the seat lapses.
    table.tick(now + Duration::from_secs(100), |account| account != CARLA);
    let tick = table.tick(now + Duration::from_secs(160), |account| account != CARLA);
    assert_eq!(tick.removed, vec![CARLA]);
    assert_eq!(table.role(CARLA), None);
    assert!(!tick.closed);
}

#[test]
fn spectators_leave_with_their_last_session() {
    let (mut table, now) = seated();
    table.join(DIEGO, "Diego", true).unwrap();
    let tick = table.tick(now, |account| account != DIEGO);
    assert_eq!(tick.removed, vec![DIEGO]);
    assert_eq!(table.lobby(HOST).spectators, 0);
}

#[test]
fn the_host_migrates_to_the_longest_seated_player_and_the_last_seat_closes() {
    let (mut table, now) = seated();
    let removal = table.leave(HOST, now);
    assert!(removal.removed && !removal.closed);
    assert_eq!(table.host(), BRUNO);
    assert_eq!(table.role(BRUNO), Some(StorySeatRole::Host));
    assert_eq!(table.lobby(BRUNO).code.as_deref(), Some("ABCD2345"));
    assert_eq!(table.lobby(CARLA).code, None);
    assert!(table.leave(CARLA, now).removed);
    let removal = table.leave(BRUNO, now);
    assert!(removal.closed);
    // The last host is remembered for the autosave.
    assert_eq!(table.host(), BRUNO);
    assert_eq!(table.leave(BRUNO, now), Removal::default());
    // A lapsed host migrates too.
    let (mut table, now) = seated();
    table.tick(now, |account| account != HOST);
    let tick = table.tick(now + Duration::from_secs(61), |account| account != HOST);
    assert_eq!(tick.removed, vec![HOST]);
    assert_eq!(table.host(), BRUNO);
}

#[test]
fn kicked_players_cannot_rejoin_and_the_host_cannot_kick_itself() {
    let (mut table, now) = seated();
    assert_eq!(table.kick(BRUNO, "Carla", now), Err(StoryFailure::HostOnly));
    assert_eq!(
        table.kick(HOST, "aria", now),
        Err(StoryFailure::UnknownMember)
    );
    assert_eq!(
        table.kick(HOST, "nadie", now),
        Err(StoryFailure::UnknownMember)
    );
    let (kicked, removal) = table.kick(HOST, " carla ", now).unwrap();
    assert_eq!(kicked, CARLA);
    assert!(removal.removed);
    assert!(table.is_banned(CARLA));
    assert_eq!(
        table.join(CARLA, "Carla", false),
        Err(StoryFailure::UnknownCode)
    );
    // The banned list is bounded.
    for index in 0..(MAX_BANNED as i64 + 4) {
        let account = AccountId(100 + index);
        table.join(account, &format!("p{index}"), false).unwrap();
        table.kick(HOST, &format!("p{index}"), now).unwrap();
    }
    assert!(table.banned.len() <= MAX_BANNED);
    assert!(!table.is_banned(CARLA));
}

#[test]
fn host_mode_lets_only_the_host_decide_and_keeps_advisory_votes() {
    let (mut table, now) = seated();
    table
        .set_mode(HOST, StoryDecisionMode::Host, Some(45), now)
        .unwrap();
    assert_eq!(table.route(HOST), Ok(Route::Apply));
    assert_eq!(table.route(BRUNO), Err(StoryFailure::HostOnly));
    assert_eq!(table.vote(BRUNO, Some(1), now), Ok(None));
    assert_eq!(table.vote(CARLA, Some(1), now), Ok(None));
    // Advisory votes never close on their own.
    assert_eq!(table.tick(now + Duration::from_secs(600), all).winner, None);
    assert_eq!(table.vote(HOST, Some(0), now), Ok(Some(0)));
    let lobby = table.lobby(HOST);
    assert_eq!(
        (lobby.mode, lobby.vote_seconds),
        (StoryDecisionMode::Host, 45)
    );
    assert_eq!(
        table.set_mode(HOST, StoryDecisionMode::Vote, Some(10), now),
        Err(StoryFailure::InvalidSetting)
    );
}

#[test]
fn turns_rotate_in_seat_order_and_skip_absent_or_slow_players() {
    let (mut table, now) = seated();
    table
        .set_mode(HOST, StoryDecisionMode::Turns, None, now)
        .unwrap();
    assert_eq!(table.route(HOST), Ok(Route::Apply));
    assert_eq!(table.route(BRUNO), Err(StoryFailure::NotYourTurn));
    assert_eq!(
        table.vote(BRUNO, Some(0), now),
        Err(StoryFailure::NotYourTurn)
    );
    assert_eq!(table.turn_name(), Some("Aria"));
    table.on_new_view(2, true, now);
    assert_eq!(table.turn_name(), Some("Bruno"));
    assert!(table.lobby(CARLA).seats[1].turn);
    // Bruno idles past the window: the turn passes to Carla.
    let tick = table.tick(now + Duration::from_secs(30), all);
    assert!(tick.changed);
    assert_eq!(table.turn_name(), Some("Carla"));
    // Absent players are skipped at once.
    let later = now + Duration::from_secs(31);
    table.tick(later, |account| account != HOST && account != CARLA);
    assert_eq!(table.turn_name(), Some("Bruno"));
    // The turn holder leaving passes the turn too.
    table.leave(BRUNO, later);
    assert_ne!(table.turn_name(), Some("Bruno"));
    // With nobody connected the turn does not spin.
    let before = table.lobby_rev;
    table.tick(later + Duration::from_secs(1), |_| false);
    let after = table.lobby_rev;
    table.tick(later + Duration::from_secs(2), |_| false);
    assert_eq!(table.lobby_rev, after);
    assert!(after >= before);
}
