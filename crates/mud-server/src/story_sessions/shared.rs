//! Shared story instances (ADR 0033, phase S7 of ADR 0025).
//!
//! A [`Table`] is the pure, clock-injected membership and decision state of
//! one shared instance: seats, spectators, the decision mode, the votes of
//! the current view and the round-robin turn. The instance task owns it (it
//! stays single-writer) and asks it which action, if any, to apply; the
//! ruleset itself never changes. Every collection is bounded: at most
//! [`MAX_STORY_SEATS`] seats, [`MAX_STORY_SPECTATORS`] spectators, one vote per
//! seat and [`MAX_BANNED`] removed accounts.
//!
//! The host is always the longest-seated player (`seats[0]`): when the host
//! leaves or its seat lapses, the next seat in join order inherits the role.

use std::{
    collections::{BTreeMap, VecDeque},
    time::Duration,
};

use super::StoryFailure;
use tiamud_core::AccountId;
use tiamud_protocol::{
    self as protocol, MAX_STORY_SEATS, MAX_STORY_SPECTATORS, MAX_STORY_VOTE_SECONDS,
    MIN_STORY_VOTE_SECONDS, STORY_JOIN_CODE_BYTES, StoryDecisionMode, StorySeatRole,
};
use tokio::time::Instant;

/// Smallest configurable number of seats (the host plus one).
pub const MIN_SEATS: usize = 2;
pub const DEFAULT_MAX_SEATS: usize = MAX_STORY_SEATS as usize;
pub const DEFAULT_MAX_SPECTATORS: usize = MAX_STORY_SPECTATORS as usize;
pub const DEFAULT_VOTE_SECONDS: u16 = 60;
/// How long a disconnected player's seat is held.
pub const DEFAULT_GRACE_SECONDS: u64 = 120;
pub const MIN_GRACE_SECONDS: u64 = 30;
pub const MAX_GRACE_SECONDS: u64 = 600;
/// Removed (kicked) accounts remembered per instance; the oldest is forgotten.
pub const MAX_BANNED: usize = 64;
/// Longest participant name kept in a lobby (character names are shorter).
pub const MAX_MEMBER_NAME_BYTES: usize = 64;
/// Alphabet of join codes: no `0/O`, `1/I` look-alikes.
pub const JOIN_CODE_ALPHABET: &[u8; 32] = b"23456789ABCDEFGHJKLMNPQRSTUVWXYZ";

/// Server-wide shared-instance limits (environment, validated at startup).
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SharedLimits {
    pub max_seats: usize,
    pub max_spectators: usize,
    /// Default vote and turn window of a new shared instance.
    pub vote_seconds: u16,
    pub grace: Duration,
}

impl Default for SharedLimits {
    fn default() -> Self {
        Self {
            max_seats: DEFAULT_MAX_SEATS,
            max_spectators: DEFAULT_MAX_SPECTATORS,
            vote_seconds: DEFAULT_VOTE_SECONDS,
            grace: Duration::from_secs(DEFAULT_GRACE_SECONDS),
        }
    }
}

/// Normalizes typed join code input; `None` when it cannot be a code.
pub fn normalize_join_code(code: &str) -> Option<String> {
    let code = code.trim().to_ascii_uppercase();
    (code.len() == STORY_JOIN_CODE_BYTES
        && code.bytes().all(|byte| JOIN_CODE_ALPHABET.contains(&byte)))
    .then_some(code)
}

/// A join code from 40 bits of `word`.
pub fn join_code(mut word: u64) -> String {
    let mut code = String::with_capacity(STORY_JOIN_CODE_BYTES);
    for _ in 0..STORY_JOIN_CODE_BYTES {
        code.push(char::from(JOIN_CODE_ALPHABET[(word & 31) as usize]));
        word >>= 5;
    }
    code
}

pub fn valid_vote_seconds(seconds: u16) -> bool {
    (MIN_STORY_VOTE_SECONDS..=MAX_STORY_VOTE_SECONDS).contains(&seconds)
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Member {
    pub account: AccountId,
    pub name: String,
    /// When the member was last seen without a connected session.
    absent_since: Option<Instant>,
}

impl Member {
    fn new(account: AccountId, name: &str) -> Self {
        let mut cut = name.len().min(MAX_MEMBER_NAME_BYTES);
        while !name.is_char_boundary(cut) {
            cut -= 1;
        }
        Self {
            account,
            name: name[..cut].to_owned(),
            absent_since: None,
        }
    }

    pub const fn connected(&self) -> bool {
        self.absent_since.is_none()
    }
}

/// How an input of a participant is handled in the current mode.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Route {
    /// Applied immediately (host in host mode, turn holder in turns mode).
    Apply,
    /// Counted as a vote for the addressed action.
    Vote,
}

/// Effects of removing a participant.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct Removal {
    pub removed: bool,
    /// No seated player remains: the instance closes.
    pub closed: bool,
}

/// Effects of one clock tick.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct Tick {
    /// Action ordinal whose vote closed and must be applied now.
    pub winner: Option<usize>,
    /// Accounts whose seat lapsed or whose spectator session ended.
    pub removed: Vec<AccountId>,
    pub closed: bool,
    /// Something public changed and the frame must be republished.
    pub changed: bool,
}

/// Pure membership and decision state of one shared instance.
#[derive(Clone, Debug)]
pub struct Table {
    limits: SharedLimits,
    mode: StoryDecisionMode,
    vote_seconds: u16,
    code: String,
    /// Seats in join order; `seats[0]` is the host.
    seats: Vec<Member>,
    spectators: Vec<Member>,
    /// Last host, kept after the final seat empties (for the autosave).
    last_host: AccountId,
    banned: VecDeque<AccountId>,
    /// Seat → ordinal of the voted action of `vote_view`.
    votes: BTreeMap<AccountId, usize>,
    vote_view: u32,
    /// Vote window (vote mode) or turn deadline (turns mode).
    deadline: Option<Instant>,
    /// Turn holder in turns mode.
    turn: Option<AccountId>,
    /// Bumped on every membership, role, presence or rule change.
    pub lobby_rev: u64,
    /// Bumped on every vote, deadline or view change.
    pub tally_rev: u64,
}

impl Table {
    pub fn new(
        host: AccountId,
        host_name: &str,
        limits: SharedLimits,
        code: String,
        view_rev: u32,
    ) -> Self {
        let vote_seconds = limits.vote_seconds;
        Self {
            limits,
            mode: StoryDecisionMode::Vote,
            vote_seconds,
            code,
            seats: vec![Member::new(host, host_name)],
            spectators: Vec::new(),
            last_host: host,
            banned: VecDeque::new(),
            votes: BTreeMap::new(),
            vote_view: view_rev,
            deadline: None,
            turn: None,
            lobby_rev: 1,
            tally_rev: 1,
        }
    }

    pub fn code(&self) -> &str {
        &self.code
    }

    pub const fn mode(&self) -> StoryDecisionMode {
        self.mode
    }

    /// The current host, or the last one once every seat emptied.
    pub fn host(&self) -> AccountId {
        self.seats
            .first()
            .map_or(self.last_host, |seat| seat.account)
    }

    pub fn seats(&self) -> &[Member] {
        &self.seats
    }

    pub fn members(&self) -> impl Iterator<Item = &Member> {
        self.seats.iter().chain(&self.spectators)
    }

    pub fn is_banned(&self, account: AccountId) -> bool {
        self.banned.contains(&account)
    }

    pub fn role(&self, account: AccountId) -> Option<StorySeatRole> {
        if self.host() == account && !self.seats.is_empty() {
            Some(StorySeatRole::Host)
        } else if self.seats.iter().any(|seat| seat.account == account) {
            Some(StorySeatRole::Player)
        } else if self.spectators.iter().any(|seat| seat.account == account) {
            Some(StorySeatRole::Spectator)
        } else {
            None
        }
    }

    /// Seated players only (host included).
    pub fn require_seat(&self, account: AccountId) -> Result<(), StoryFailure> {
        match self.role(account) {
            None => Err(StoryFailure::NotInStory),
            Some(StorySeatRole::Spectator) => Err(StoryFailure::Spectator),
            Some(_) => Ok(()),
        }
    }

    pub fn is_host(&self, account: AccountId) -> bool {
        self.role(account) == Some(StorySeatRole::Host)
    }

    pub fn require_host(&self, account: AccountId) -> Result<(), StoryFailure> {
        match self.role(account) {
            Some(StorySeatRole::Host) => Ok(()),
            Some(_) => Err(StoryFailure::HostOnly),
            None => Err(StoryFailure::NotInStory),
        }
    }

    /// Seats a player (or adds a spectator). A removed account is refused
    /// with the same error as an unknown code; rejoining as a member is a
    /// no-op.
    pub fn join(
        &mut self,
        account: AccountId,
        name: &str,
        spectate: bool,
    ) -> Result<(), StoryFailure> {
        if self.is_banned(account) {
            return Err(StoryFailure::UnknownCode);
        }
        if self.role(account).is_some() {
            return Ok(());
        }
        if spectate {
            if self.spectators.len() >= self.limits.max_spectators {
                return Err(StoryFailure::SeatsFull);
            }
            self.spectators.push(Member::new(account, name));
        } else {
            if self.seats.len() >= self.limits.max_seats {
                return Err(StoryFailure::SeatsFull);
            }
            self.seats.push(Member::new(account, name));
        }
        self.lobby_rev += 1;
        self.tally_rev += 1;
        Ok(())
    }

    /// Removes a participant; the host role migrates to the longest-seated
    /// remaining player. Closing is reported when no seat remains.
    pub fn leave(&mut self, account: AccountId, now: Instant) -> Removal {
        let host = self.host();
        if let Some(index) = self.seats.iter().position(|seat| seat.account == account) {
            self.seats.remove(index);
            self.votes.remove(&account);
            if account == host {
                self.last_host = self.seats.first().map_or(account, |seat| seat.account);
            }
            if self.turn == Some(account) {
                self.turn = self.next_seat_after(index.wrapping_sub(1));
                self.restart_turn(now);
            }
            self.lobby_rev += 1;
            self.tally_rev += 1;
            return Removal {
                removed: true,
                closed: self.seats.is_empty(),
            };
        }
        if let Some(index) = self
            .spectators
            .iter()
            .position(|seat| seat.account == account)
        {
            self.spectators.remove(index);
            self.lobby_rev += 1;
            return Removal {
                removed: true,
                closed: false,
            };
        }
        Removal::default()
    }

    /// Host only: removes a participant by case-insensitive character name
    /// and refuses that account from rejoining this instance.
    pub fn kick(
        &mut self,
        from: AccountId,
        name: &str,
        now: Instant,
    ) -> Result<(AccountId, Removal), StoryFailure> {
        self.require_host(from)?;
        let target = self
            .members()
            .find(|member| member.name.eq_ignore_ascii_case(name.trim()))
            .map(|member| member.account)
            .filter(|account| *account != from)
            .ok_or(StoryFailure::UnknownMember)?;
        if self.banned.len() >= MAX_BANNED {
            self.banned.pop_front();
        }
        self.banned.push_back(target);
        Ok((target, self.leave(target, now)))
    }

    /// Host only: changes the decision mode and, optionally, the window.
    /// Open votes are discarded; turns start with the host.
    pub fn set_mode(
        &mut self,
        from: AccountId,
        mode: StoryDecisionMode,
        vote_seconds: Option<u16>,
        now: Instant,
    ) -> Result<(), StoryFailure> {
        self.require_host(from)?;
        if let Some(seconds) = vote_seconds {
            if !valid_vote_seconds(seconds) {
                return Err(StoryFailure::InvalidSetting);
            }
            self.vote_seconds = seconds;
        }
        self.mode = mode;
        self.votes.clear();
        self.deadline = None;
        self.turn = None;
        if mode == StoryDecisionMode::Turns {
            self.turn = Some(self.host());
            self.restart_turn(now);
        }
        self.lobby_rev += 1;
        self.tally_rev += 1;
        Ok(())
    }

    /// Who may type a parser command line (StoryPack v3). Free text is never
    /// put to a vote: in host and vote modes only the host types (the
    /// others still vote on the numbered actions of a view that has them),
    /// in turns mode only the turn holder; spectators never.
    pub fn route_command(&self, from: AccountId) -> Result<(), StoryFailure> {
        match (self.role(from), self.mode) {
            (None, _) => Err(StoryFailure::NotInStory),
            (Some(StorySeatRole::Spectator), _) => Err(StoryFailure::Spectator),
            (Some(StorySeatRole::Host), StoryDecisionMode::Host | StoryDecisionMode::Vote) => {
                Ok(())
            }
            (Some(_), StoryDecisionMode::Host | StoryDecisionMode::Vote) => {
                Err(StoryFailure::HostOnly)
            }
            (Some(_), StoryDecisionMode::Turns) if self.turn == Some(from) => Ok(()),
            (Some(_), StoryDecisionMode::Turns) => Err(StoryFailure::NotYourTurn),
        }
    }

    /// How a story input (`story.choose`, `story.menu`, `story.dialogue` or a
    /// typed number) of `from` is handled.
    pub fn route(&self, from: AccountId) -> Result<Route, StoryFailure> {
        match (self.role(from), self.mode) {
            (None, _) => Err(StoryFailure::NotInStory),
            (Some(StorySeatRole::Spectator), _) => Err(StoryFailure::Spectator),
            (Some(StorySeatRole::Host), StoryDecisionMode::Host) => Ok(Route::Apply),
            (Some(_), StoryDecisionMode::Host) => Err(StoryFailure::HostOnly),
            (Some(_), StoryDecisionMode::Vote) => Ok(Route::Vote),
            (Some(_), StoryDecisionMode::Turns) if self.turn == Some(from) => Ok(Route::Apply),
            (Some(_), StoryDecisionMode::Turns) => Err(StoryFailure::NotYourTurn),
        }
    }

    /// Records (or, with `None`, withdraws) the vote of `from` for action
    /// `ordinal` of the current view. Returns the action to apply now, if
    /// the decision closed: in vote mode when every connected seat voted, in
    /// host mode when the host votes, in turns mode when the turn holder
    /// votes. Other seats' votes in host mode are advisory.
    pub fn vote(
        &mut self,
        from: AccountId,
        ordinal: Option<usize>,
        now: Instant,
    ) -> Result<Option<usize>, StoryFailure> {
        match self.role(from) {
            None => return Err(StoryFailure::NotInStory),
            Some(StorySeatRole::Spectator) => return Err(StoryFailure::Spectator),
            Some(_) => {}
        }
        match self.mode {
            StoryDecisionMode::Turns if self.turn != Some(from) => {
                return Err(StoryFailure::NotYourTurn);
            }
            StoryDecisionMode::Turns => return Ok(ordinal),
            StoryDecisionMode::Host if self.is_host(from) => return Ok(ordinal),
            StoryDecisionMode::Host | StoryDecisionMode::Vote => {}
        }
        let changed = match ordinal {
            Some(ordinal) => self.votes.insert(from, ordinal) != Some(ordinal),
            None => self.votes.remove(&from).is_some(),
        };
        if changed {
            self.tally_rev += 1;
        }
        if self.mode != StoryDecisionMode::Vote {
            return Ok(None);
        }
        if self.votes.is_empty() {
            // Every vote was withdrawn: the window closes until the next one.
            if self.deadline.take().is_some() {
                self.tally_rev += 1;
            }
            return Ok(None);
        }
        if self.deadline.is_none() {
            self.deadline = Some(now + self.window());
            self.tally_rev += 1;
        }
        Ok(self.everyone_voted().then(|| self.winner()).flatten())
    }

    fn window(&self) -> Duration {
        Duration::from_secs(u64::from(self.vote_seconds))
    }

    /// True when every connected seat has a vote (at least one seat voted).
    fn everyone_voted(&self) -> bool {
        let mut present = self.seats.iter().filter(|seat| seat.connected()).peekable();
        present.peek().is_some() && present.all(|seat| self.votes.contains_key(&seat.account))
    }

    /// Majority action. Ties go to the host's vote when it is among the
    /// tied actions, otherwise to the lowest action number.
    pub fn winner(&self) -> Option<usize> {
        let mut counts = BTreeMap::<usize, usize>::new();
        for ordinal in self.votes.values() {
            *counts.entry(*ordinal).or_default() += 1;
        }
        let best = counts.values().copied().max()?;
        let tied = counts
            .iter()
            .filter(|(_, count)| **count == best)
            .map(|(ordinal, _)| *ordinal)
            .collect::<Vec<_>>();
        let host_vote = self.votes.get(&self.host()).copied();
        Some(
            host_vote
                .filter(|vote| tied.contains(vote))
                .unwrap_or(tied[0]),
        )
    }

    /// A new view was published: votes of the old one are void. In turns
    /// mode an applied input passes the turn to the next seat.
    pub fn on_new_view(&mut self, view_rev: u32, turn_taken: bool, now: Instant) {
        self.vote_view = view_rev;
        self.votes.clear();
        if self.mode == StoryDecisionMode::Turns {
            if turn_taken {
                self.advance_turn(now);
            }
        } else {
            self.deadline = None;
        }
        self.tally_rev += 1;
    }

    fn next_seat_after(&self, index: usize) -> Option<AccountId> {
        let count = self.seats.len();
        (1..=count)
            .map(|step| &self.seats[index.wrapping_add(step) % count])
            .find(|seat| seat.connected())
            .or_else(|| self.seats.first())
            .map(|seat| seat.account)
    }

    fn advance_turn(&mut self, now: Instant) {
        let index = self
            .turn
            .and_then(|turn| self.seats.iter().position(|seat| seat.account == turn))
            .unwrap_or(usize::MAX);
        self.turn = self.next_seat_after(index);
        self.restart_turn(now);
        self.lobby_rev += 1;
    }

    fn restart_turn(&mut self, now: Instant) {
        if self.mode == StoryDecisionMode::Turns {
            self.deadline = self.turn.map(|_| now + self.window());
            self.tally_rev += 1;
        }
    }

    /// Applies presence and deadlines. `present` says whether an account has
    /// at least one connected session. Spectators leave as soon as their
    /// last session ends; a seat is held for the grace period.
    pub fn tick(&mut self, now: Instant, present: impl Fn(AccountId) -> bool) -> Tick {
        let mut tick = Tick::default();
        for seat in &mut self.seats {
            match (present(seat.account), seat.absent_since) {
                (true, Some(_)) => {
                    seat.absent_since = None;
                    tick.changed = true;
                }
                (false, None) => {
                    seat.absent_since = Some(now);
                    tick.changed = true;
                }
                (false, Some(since)) if now.duration_since(since) >= self.limits.grace => {
                    tick.removed.push(seat.account);
                }
                _ => {}
            }
        }
        tick.removed.extend(
            self.spectators
                .iter()
                .filter(|spectator| !present(spectator.account))
                .map(|spectator| spectator.account),
        );
        for account in tick.removed.clone() {
            let removal = self.leave(account, now);
            tick.closed |= removal.closed;
            tick.changed = true;
        }
        if tick.closed {
            return tick;
        }
        if tick.changed {
            self.lobby_rev += 1;
            self.tally_rev += 1;
        }
        let expired = self.deadline.is_some_and(|deadline| now >= deadline);
        match self.mode {
            StoryDecisionMode::Vote => {
                if !self.votes.is_empty() && (expired || self.everyone_voted()) {
                    tick.winner = self.winner();
                }
            }
            StoryDecisionMode::Turns => {
                let holder_absent = self.turn.is_some_and(|turn| {
                    !self
                        .seats
                        .iter()
                        .any(|seat| seat.account == turn && seat.connected())
                });
                let anyone = self.seats.iter().any(Member::connected);
                if anyone && (expired || holder_absent || self.turn.is_none()) {
                    let before = self.turn;
                    self.advance_turn(now);
                    tick.changed |= before != self.turn || expired;
                }
            }
            StoryDecisionMode::Host => {}
        }
        tick
    }

    /// Seconds left of the running deadline, rounded up.
    pub fn remaining_seconds(&self, now: Instant) -> Option<u16> {
        let left = self.deadline?.saturating_duration_since(now);
        let seconds = left.as_millis().div_ceil(1_000);
        Some(u16::try_from(seconds).unwrap_or(u16::MAX))
    }

    /// `story.lobby` as seen by `me`; the code is shown to the host only.
    pub fn lobby(&self, me: AccountId) -> protocol::StoryLobby {
        let host = self.host();
        protocol::StoryLobby {
            mode: self.mode,
            vote_seconds: self.vote_seconds,
            you: self.role(me).unwrap_or(StorySeatRole::Spectator),
            code: (self.is_host(me)).then(|| self.code.clone()),
            seats: self
                .seats
                .iter()
                .map(|seat| protocol::StorySeat {
                    name: seat.name.clone(),
                    role: if seat.account == host {
                        StorySeatRole::Host
                    } else {
                        StorySeatRole::Player
                    },
                    connected: seat.connected(),
                    turn: self.mode == StoryDecisionMode::Turns && self.turn == Some(seat.account),
                })
                .collect(),
            spectators: u8::try_from(self.spectators.len()).unwrap_or(u8::MAX),
            max_seats: u8::try_from(self.limits.max_seats).unwrap_or(u8::MAX),
            max_spectators: u8::try_from(self.limits.max_spectators).unwrap_or(u8::MAX),
        }
    }

    /// `story.tally` as seen by `me`. `leaves` are the opaque leaf ids of
    /// the current view's actions, in action order. Only actions with votes
    /// are listed (at most one per seat), in action order.
    pub fn tally(
        &self,
        me: AccountId,
        view_rev: u32,
        leaves: &[String],
        now: Instant,
    ) -> protocol::StoryTally {
        let current = view_rev == self.vote_view;
        let mut counts = BTreeMap::<usize, u8>::new();
        if current {
            for ordinal in self.votes.values() {
                let count = counts.entry(*ordinal).or_default();
                *count = count.saturating_add(1);
            }
        }
        let leaf = |ordinal: usize| leaves.get(ordinal).cloned();
        protocol::StoryTally {
            view_rev,
            mode: self.mode,
            choices: counts
                .into_iter()
                .filter_map(|(ordinal, votes)| {
                    leaf(ordinal).map(|choice_id| protocol::StoryTallyEntry { choice_id, votes })
                })
                .collect(),
            voted: if current {
                u8::try_from(self.votes.len()).unwrap_or(u8::MAX)
            } else {
                0
            },
            eligible: u8::try_from(match self.mode {
                StoryDecisionMode::Vote => {
                    self.seats.iter().filter(|seat| seat.connected()).count()
                }
                _ => 1,
            })
            .unwrap_or(u8::MAX),
            remaining_seconds: self.remaining_seconds(now),
            your_vote: current
                .then(|| self.votes.get(&me).and_then(|ordinal| leaf(*ordinal)))
                .flatten(),
        }
    }

    /// The turn holder's name in turns mode.
    pub fn turn_name(&self) -> Option<&str> {
        let turn = self.turn?;
        self.seats
            .iter()
            .find(|seat| seat.account == turn)
            .map(|seat| seat.name.as_str())
    }
}

#[cfg(test)]
mod tests;
