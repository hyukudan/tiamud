//! Transport-independent world model for Tiamud.

mod ability;
mod progression;

pub use ability::*;
pub use progression::*;

use serde::{Deserialize, Serialize};
use slotmap::{SlotMap, new_key_type};
use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::hash::{DefaultHasher, Hash, Hasher};
use thiserror::Error;

new_key_type! {
    /// Internal, generational entity identifier. Never exposed as a persistent ID.
    struct EntityId;
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct SessionId(pub u64);

/// Durable database identity for an account.
///
/// Unlike [`SessionId`], this value survives reconnects and server restarts.
#[derive(Clone, Copy, Debug, Deserialize, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(transparent)]
pub struct AccountId(pub i64);

/// Durable database identity for a character.
///
/// Character names are presentation data and are not used as identity.
#[derive(Clone, Copy, Debug, Deserialize, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(transparent)]
pub struct CharacterId(pub i64);

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct AccountSnapshot {
    pub id: AccountId,
    pub name: String,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Direction {
    North,
    East,
    South,
    West,
    Up,
    Down,
}

impl Direction {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::North => "north",
            Self::East => "east",
            Self::South => "south",
            Self::West => "west",
            Self::Up => "up",
            Self::Down => "down",
        }
    }

    fn parse(value: &str) -> Option<Self> {
        match value {
            "north" | "n" => Some(Self::North),
            "east" | "e" => Some(Self::East),
            "south" | "s" => Some(Self::South),
            "west" | "w" => Some(Self::West),
            "up" | "u" => Some(Self::Up),
            "down" | "d" => Some(Self::Down),
            _ => None,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum PlayerAction {
    Look,
    Move(Direction),
    Inventory,
    Take(String),
    Drop(String),
    Examine(String),
    Attack(String),
    Ability {
        ability_id: String,
        target: Option<String>,
    },
    Promote {
        class_id: String,
    },
    AcquireTalent {
        graph_id: String,
        node_id: String,
    },
    Use(String),
    Equip(String),
    Unequip(String),
    Party(PartyAction),
    Quest(QuestAction),
    Economy(EconomyAction),
    Weather,
    Time,
    Social(SocialAction),
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum PartyAction {
    Create,
    Invite(String),
    Accept(String),
    Revoke(String),
    Kick(String),
    Transfer(String),
    Disband,
    Leave,
    Status,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum QuestAction {
    List,
    Accept(String),
    Status,
    Complete(String),
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum EconomyAction {
    Balance,
    Shop,
    Buy(String),
    Sell(String),
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum AccountRole {
    Player,
    Moderator,
    Admin,
}

impl AccountRole {
    pub fn can_moderate(self, target: Self) -> bool {
        matches!(self, Self::Moderator | Self::Admin) && self > target
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SocialChannelName {
    General,
    Newbie,
    Staff,
}

impl SocialChannelName {
    pub const ALL: [Self; 3] = [Self::General, Self::Newbie, Self::Staff];

    pub const fn as_str(self) -> &'static str {
        match self {
            Self::General => "general",
            Self::Newbie => "newbie",
            Self::Staff => "staff",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        match value.trim().to_ascii_lowercase().as_str() {
            "general" | "gossip" => Some(Self::General),
            "newbie" | "help" => Some(Self::Newbie),
            "staff" | "wiznet" => Some(Self::Staff),
            _ => None,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum SocialAction {
    Say(String),
    PartySay(String),
    Tell {
        target: String,
        message: String,
    },
    Reply(String),
    ChannelList,
    ChannelJoin(SocialChannelName),
    ChannelLeave(SocialChannelName),
    ChannelSay {
        channel: SocialChannelName,
        message: String,
    },
    Who,
    Help(Option<String>),
    Ignore(String),
    Unignore(String),
    Block(String),
    Unblock(String),
    Mute {
        target: String,
        duration_seconds: Option<u64>,
        reason: String,
    },
    Unmute(String),
    Report {
        target: String,
        reason: String,
    },
    Ban {
        target: String,
        duration_seconds: Option<u64>,
        reason: String,
    },
    Unban(String),
    SetRole {
        target: String,
        role: AccountRole,
    },
}

#[derive(Debug, Error, Eq, PartialEq)]
pub enum ParseActionError {
    #[error("Please enter a command.")]
    Empty,
    #[error("Unknown command: {0}")]
    Unknown(String),
    #[error("{command} what?")]
    MissingTarget { command: &'static str },
}

pub fn parse_action(input: &str) -> Result<PlayerAction, ParseActionError> {
    let input = input.trim();
    let normalised = input.to_ascii_lowercase();
    if normalised.is_empty() {
        return Err(ParseActionError::Empty);
    }
    if matches!(normalised.as_str(), "look" | "l") {
        return Ok(PlayerAction::Look);
    }
    if matches!(normalised.as_str(), "inventory" | "inv" | "i") {
        return Ok(PlayerAction::Inventory);
    }
    if normalised == "weather" {
        return Ok(PlayerAction::Weather);
    }
    if normalised == "time" {
        return Ok(PlayerAction::Time);
    }
    if let Some(class_id) = normalised.strip_prefix("promote ").map(str::trim) {
        if class_id.is_empty() {
            return Err(ParseActionError::MissingTarget { command: "promote" });
        }
        return Ok(PlayerAction::Promote {
            class_id: class_id.to_owned(),
        });
    }
    if normalised == "promote" {
        return Err(ParseActionError::MissingTarget { command: "promote" });
    }
    if let Some(rest) = normalised.strip_prefix("talent learn ").map(str::trim) {
        let mut parts = rest.split_whitespace();
        let graph_id = parts.next();
        let node_id = parts.next();
        if graph_id.is_none() || node_id.is_none() || parts.next().is_some() {
            return Err(ParseActionError::MissingTarget { command: "talent" });
        }
        return Ok(PlayerAction::AcquireTalent {
            graph_id: graph_id.expect("checked").to_owned(),
            node_id: node_id.expect("checked").to_owned(),
        });
    }
    if normalised == "balance" {
        return Ok(PlayerAction::Economy(EconomyAction::Balance));
    }
    if normalised == "shop" {
        return Ok(PlayerAction::Economy(EconomyAction::Shop));
    }
    for (command, constructor) in [
        ("buy", EconomyAction::Buy as fn(String) -> EconomyAction),
        ("sell", EconomyAction::Sell),
    ] {
        if normalised == command {
            return Err(ParseActionError::MissingTarget { command });
        }
        if let Some(target) = command_tail_phrase(input, command) {
            return Ok(PlayerAction::Economy(constructor(target)));
        }
    }
    match normalised.as_str() {
        "party create" => return Ok(PlayerAction::Party(PartyAction::Create)),
        "party disband" => return Ok(PlayerAction::Party(PartyAction::Disband)),
        "party leave" => return Ok(PlayerAction::Party(PartyAction::Leave)),
        "party" | "party status" => return Ok(PlayerAction::Party(PartyAction::Status)),
        "quest" | "quests" | "quest list" => {
            return Ok(PlayerAction::Quest(QuestAction::List));
        }
        "quest status" => return Ok(PlayerAction::Quest(QuestAction::Status)),
        _ => {}
    }
    for (command, constructor) in [
        (
            "party invite",
            PartyAction::Invite as fn(String) -> PartyAction,
        ),
        ("party accept", PartyAction::Accept),
        ("party revoke", PartyAction::Revoke),
        ("party kick", PartyAction::Kick),
        ("party transfer", PartyAction::Transfer),
    ] {
        if normalised == command {
            return Err(ParseActionError::MissingTarget { command: "party" });
        }
        if let Some(target) = command_tail_phrase(input, command) {
            return Ok(PlayerAction::Party(constructor(target)));
        }
    }
    for (command, constructor) in [
        (
            "quest accept",
            QuestAction::Accept as fn(String) -> QuestAction,
        ),
        ("quest complete", QuestAction::Complete),
    ] {
        if normalised == command {
            return Err(ParseActionError::MissingTarget { command: "quest" });
        }
        if let Some(target) = command_tail_phrase(input, command) {
            return Ok(PlayerAction::Quest(constructor(target)));
        }
    }
    if normalised == "who" {
        return Ok(PlayerAction::Social(SocialAction::Who));
    }
    if normalised == "help" {
        return Ok(PlayerAction::Social(SocialAction::Help(None)));
    }
    if let Some(topic) = command_tail(input, "help") {
        return Ok(PlayerAction::Social(SocialAction::Help(Some(topic))));
    }
    if let Some(message) = command_tail(input, "say") {
        return Ok(PlayerAction::Social(SocialAction::Say(message)));
    }
    if normalised == "say" {
        return Err(ParseActionError::MissingTarget { command: "say" });
    }
    for command in ["party say", "gsay", "gtell"] {
        if let Some(message) = command_tail_phrase(input, command) {
            return Ok(PlayerAction::Social(SocialAction::PartySay(message)));
        }
        if normalised == command {
            return Err(ParseActionError::MissingTarget {
                command: "party say",
            });
        }
    }
    if let Some(rest) = command_tail(input, "tell") {
        let Some((target, message)) = rest.split_once(char::is_whitespace) else {
            return Err(ParseActionError::MissingTarget { command: "tell" });
        };
        let message = message.trim();
        if message.is_empty() {
            return Err(ParseActionError::MissingTarget { command: "tell" });
        }
        return Ok(PlayerAction::Social(SocialAction::Tell {
            target: target.to_owned(),
            message: message.to_owned(),
        }));
    }
    if normalised == "tell" {
        return Err(ParseActionError::MissingTarget { command: "tell" });
    }
    if let Some(message) = command_tail(input, "reply") {
        return Ok(PlayerAction::Social(SocialAction::Reply(message)));
    }
    if normalised == "reply" {
        return Err(ParseActionError::MissingTarget { command: "reply" });
    }
    if matches!(
        normalised.as_str(),
        "channel" | "channels" | "channel list" | "channels list"
    ) {
        return Ok(PlayerAction::Social(SocialAction::ChannelList));
    }
    for (command, join) in [("channel join", true), ("channel leave", false)] {
        if normalised == command {
            return Err(ParseActionError::MissingTarget { command: "channel" });
        }
        if let Some(channel) = command_tail_phrase(input, command) {
            let channel = SocialChannelName::parse(&channel)
                .ok_or_else(|| ParseActionError::Unknown(input.trim().to_owned()))?;
            return Ok(PlayerAction::Social(if join {
                SocialAction::ChannelJoin(channel)
            } else {
                SocialAction::ChannelLeave(channel)
            }));
        }
    }
    if let Some(rest) = command_tail_phrase(input, "channel say") {
        let Some((channel, message)) = rest.split_once(char::is_whitespace) else {
            return Err(ParseActionError::MissingTarget { command: "channel" });
        };
        let channel = SocialChannelName::parse(channel)
            .ok_or_else(|| ParseActionError::Unknown(input.trim().to_owned()))?;
        let message = message.trim();
        if message.is_empty() {
            return Err(ParseActionError::MissingTarget { command: "channel" });
        }
        return Ok(PlayerAction::Social(SocialAction::ChannelSay {
            channel,
            message: message.to_owned(),
        }));
    }
    if normalised == "channel say" {
        return Err(ParseActionError::MissingTarget { command: "channel" });
    }
    if let Some(rest) = command_tail(input, "channel") {
        let Some((channel, message)) = rest.split_once(char::is_whitespace) else {
            return Err(ParseActionError::MissingTarget { command: "channel" });
        };
        let channel = SocialChannelName::parse(channel)
            .ok_or_else(|| ParseActionError::Unknown(input.trim().to_owned()))?;
        let message = message.trim();
        if message.is_empty() {
            return Err(ParseActionError::MissingTarget { command: "channel" });
        }
        return Ok(PlayerAction::Social(SocialAction::ChannelSay {
            channel,
            message: message.to_owned(),
        }));
    }
    for (command, channel) in [
        ("gossip", SocialChannelName::General),
        ("newbie", SocialChannelName::Newbie),
        ("wiznet", SocialChannelName::Staff),
    ] {
        if let Some(message) = command_tail(input, command) {
            return Ok(PlayerAction::Social(SocialAction::ChannelSay {
                channel,
                message,
            }));
        }
        if normalised == command {
            return Err(ParseActionError::MissingTarget { command: "channel" });
        }
    }
    for (command, constructor) in [
        ("ignore", SocialAction::Ignore as fn(String) -> SocialAction),
        ("unignore", SocialAction::Unignore),
        ("block", SocialAction::Block),
        ("unblock", SocialAction::Unblock),
        ("unmute", SocialAction::Unmute),
        ("unban", SocialAction::Unban),
    ] {
        if normalised == command {
            return Err(ParseActionError::MissingTarget { command });
        }
        if let Some(target) = command_tail(input, command) {
            return Ok(PlayerAction::Social(constructor(target)));
        }
    }
    for (command, is_ban) in [("mute", false), ("ban", true)] {
        if normalised == command {
            return Err(ParseActionError::MissingTarget { command });
        }
        if let Some(rest) = command_tail(input, command) {
            let mut parts = rest.splitn(3, char::is_whitespace);
            let target = parts.next().unwrap_or_default().trim();
            let duration = parts.next().unwrap_or_default().trim();
            let reason = parts.next().unwrap_or_default().trim();
            if target.is_empty() || duration.is_empty() || reason.is_empty() {
                return Err(ParseActionError::MissingTarget { command });
            }
            let duration_seconds = parse_moderation_duration(duration)
                .ok_or_else(|| ParseActionError::Unknown(input.to_owned()))?;
            let action = if is_ban {
                SocialAction::Ban {
                    target: target.to_owned(),
                    duration_seconds,
                    reason: reason.to_owned(),
                }
            } else {
                SocialAction::Mute {
                    target: target.to_owned(),
                    duration_seconds,
                    reason: reason.to_owned(),
                }
            };
            return Ok(PlayerAction::Social(action));
        }
    }
    if let Some(rest) = command_tail(input, "report") {
        let Some((target, reason)) = rest.split_once(char::is_whitespace) else {
            return Err(ParseActionError::MissingTarget { command: "report" });
        };
        let reason = reason.trim();
        if reason.is_empty() {
            return Err(ParseActionError::MissingTarget { command: "report" });
        }
        return Ok(PlayerAction::Social(SocialAction::Report {
            target: target.to_owned(),
            reason: reason.to_owned(),
        }));
    }
    if normalised == "report" {
        return Err(ParseActionError::MissingTarget { command: "report" });
    }
    if let Some(rest) = command_tail(input, "role") {
        let Some((target, role)) = rest.split_once(char::is_whitespace) else {
            return Err(ParseActionError::MissingTarget { command: "role" });
        };
        let role = match role.trim().to_ascii_lowercase().as_str() {
            "player" => AccountRole::Player,
            "moderator" | "mod" => AccountRole::Moderator,
            "admin" => AccountRole::Admin,
            _ => return Err(ParseActionError::Unknown(input.to_owned())),
        };
        return Ok(PlayerAction::Social(SocialAction::SetRole {
            target: target.to_owned(),
            role,
        }));
    }
    if normalised == "role" {
        return Err(ParseActionError::MissingTarget { command: "role" });
    }
    if let Some(direction) = Direction::parse(&normalised) {
        return Ok(PlayerAction::Move(direction));
    }
    if let Some(rest) = normalised.strip_prefix("go ")
        && let Some(direction) = Direction::parse(rest.trim())
    {
        return Ok(PlayerAction::Move(direction));
    }
    for command in ["ability", "skill", "cast"] {
        if normalised == command {
            return Err(ParseActionError::MissingTarget { command: "ability" });
        }
        if let Some(rest) = normalised.strip_prefix(&format!("{command} ")) {
            let mut parts = rest.trim().splitn(2, char::is_whitespace);
            let ability_id = parts.next().unwrap_or_default().trim();
            if ability_id.is_empty() {
                return Err(ParseActionError::MissingTarget { command: "ability" });
            }
            let target = parts
                .next()
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(str::to_owned);
            return Ok(PlayerAction::Ability {
                ability_id: ability_id.to_owned(),
                target,
            });
        }
    }
    for (command, aliases, action) in [
        (
            "take",
            &["take", "get"][..],
            PlayerAction::Take as fn(String) -> PlayerAction,
        ),
        ("drop", &["drop"][..], PlayerAction::Drop),
        ("examine", &["examine", "x"][..], PlayerAction::Examine),
        ("attack", &["attack", "kill"][..], PlayerAction::Attack),
        ("use", &["use"][..], PlayerAction::Use),
        ("equip", &["equip", "wear"][..], PlayerAction::Equip),
        ("unequip", &["unequip", "remove"][..], PlayerAction::Unequip),
    ] {
        for alias in aliases {
            if normalised == *alias {
                return Err(ParseActionError::MissingTarget { command });
            }
            if let Some(target) = normalised.strip_prefix(&format!("{alias} ")) {
                let target = target.trim();
                if target.is_empty() {
                    return Err(ParseActionError::MissingTarget { command });
                }
                return Ok(action(target.to_owned()));
            }
        }
    }
    Err(ParseActionError::Unknown(input.trim().to_owned()))
}

fn command_tail(input: &str, command: &str) -> Option<String> {
    let (head, tail) = input.split_once(char::is_whitespace)?;
    if !head.eq_ignore_ascii_case(command) {
        return None;
    }
    let tail = tail.trim();
    (!tail.is_empty()).then(|| tail.to_owned())
}

fn command_tail_phrase(input: &str, command: &str) -> Option<String> {
    let head = input.get(..command.len())?;
    if !head.eq_ignore_ascii_case(command) {
        return None;
    }
    let rest = input.get(command.len()..)?;
    if !rest.chars().next().is_some_and(char::is_whitespace) {
        return None;
    }
    let tail = rest.trim();
    (!tail.is_empty()).then(|| tail.to_owned())
}

fn parse_moderation_duration(value: &str) -> Option<Option<u64>> {
    if matches!(value.to_ascii_lowercase().as_str(), "permanent" | "perm") {
        return Some(None);
    }
    let (number, multiplier) = match value.as_bytes().last().copied()? {
        b'm' | b'M' => (&value[..value.len() - 1], 60_u64),
        b'h' | b'H' => (&value[..value.len() - 1], 60 * 60),
        b'd' | b'D' => (&value[..value.len() - 1], 24 * 60 * 60),
        _ => (value, 1),
    };
    number
        .parse::<u64>()
        .ok()
        .and_then(|number| number.checked_mul(multiplier))
        .map(Some)
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "kind", content = "roomId", rename_all = "camelCase")]
pub enum ItemLocationSnapshot {
    Carried,
    Room(String),
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CharacterSnapshot {
    pub id: CharacterId,
    pub account_id: AccountId,
    pub name: String,
    pub room_id: String,
    /// Legacy v2 per-character item state. Shared-world runtimes ignore this
    /// field, but retain it so existing persisted characters remain readable.
    #[serde(default)]
    pub item_locations: BTreeMap<String, ItemLocationSnapshot>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ConnectedCharacter {
    pub session_id: SessionId,
    pub character_id: CharacterId,
    pub account_id: AccountId,
    pub name: String,
    pub room_id: String,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SceneImage {
    pub alt: String,
    #[serde(skip_serializing_if = "BTreeMap::is_empty")]
    pub localized_alt: BTreeMap<String, String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub manifest: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub avif: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub webp: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub jpeg: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub width: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub height: Option<u32>,
}

/// Optional, non-authoritative ambient audio attached to a projected scene.
/// URLs always refer to reviewed, immutable assets; transports still require
/// explicit client consent before sending or playing a cue.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SceneAudio {
    pub channel: String,
    pub url: String,
    #[serde(rename = "loop")]
    pub looped: bool,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SceneExit {
    pub id: String,
    pub label: String,
    pub command: String,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SceneAction {
    pub label: String,
    pub command: String,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SceneItem {
    pub id: String,
    pub label: String,
    pub description: String,
    pub actions: Vec<SceneAction>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SceneNpc {
    pub id: String,
    pub label: String,
    pub description: String,
    pub health: i32,
    pub max_health: i32,
    pub actions: Vec<SceneAction>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WeatherView {
    pub condition: String,
    pub day: u64,
    pub hour: u64,
    pub next_change_in_ticks: u64,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SceneView {
    pub view_id: String,
    pub revision: u64,
    pub title: String,
    pub prose: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub image: Option<SceneImage>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub audio: Option<SceneAudio>,
    pub exits: Vec<SceneExit>,
    pub items: Vec<SceneItem>,
    pub npcs: Vec<SceneNpc>,
    pub weather: WeatherView,
}

/// Complete, bounded and perception-safe state for one connected player.
///
/// Every entity identifier in this projection is salted for the transport
/// session. Content identifiers (quest and commodity slugs) are intentionally
/// stable because clients need them to submit their corresponding actions.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PlayerStateView {
    pub revision: u64,
    pub character: PlayerCharacterView,
    pub inventory: Vec<PlayerItemView>,
    pub equipment: Vec<EquippedItemView>,
    pub map: DiscoveredMapView,
    pub quests: Vec<QuestView>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub party: Option<PartyView>,
    pub party_invites: Vec<PartyInviteView>,
    pub economy: EconomyView,
    pub social: SocialView,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PlayerCharacterView {
    pub name: String,
    pub lineage: LineageView,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub background: Option<BackgroundView>,
    pub base_class: ClassView,
    pub active_class: ClassView,
    pub class_history: Vec<ClassView>,
    pub level: u32,
    pub experience: u64,
    pub next_level_experience: u64,
    pub health: i32,
    pub max_health: i32,
    pub strength: i32,
    pub defence: i32,
    pub attributes: Vec<AttributeView>,
    pub skills: Vec<SkillView>,
    pub resources: Vec<ResourceView>,
    pub abilities: Vec<AbilityView>,
    pub resistances: Vec<ResistanceView>,
    pub statuses: Vec<StatusView>,
    pub progression_points: BTreeMap<String, u16>,
    pub talents: Vec<TalentGraphView>,
    pub promotions: Vec<PromotionView>,
    pub effects: Vec<ActiveEffect>,
    pub recovery_in_ticks: Option<u64>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SkillView {
    pub id: String,
    pub label: String,
    pub rank: u16,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PlayerItemView {
    pub id: String,
    pub label: String,
    pub description: String,
    pub actions: Vec<SceneAction>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct EquippedItemView {
    pub id: String,
    pub slot: String,
    pub label: String,
    pub actions: Vec<SceneAction>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DiscoveredMapView {
    pub current_room_id: String,
    pub rooms: Vec<MapRoomView>,
    pub paths: Vec<MapPathView>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MapRoomView {
    pub id: String,
    pub title: String,
    pub current: bool,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MapPathView {
    pub from: String,
    pub direction: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub to: Option<String>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct QuestView {
    pub id: String,
    pub label: String,
    pub summary: String,
    pub status: String,
    pub current: u32,
    pub target: u32,
    pub actions: Vec<SceneAction>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PartyView {
    pub leader: bool,
    pub members: Vec<PartyMemberView>,
    pub actions: Vec<SceneAction>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PartyMemberView {
    pub id: String,
    pub name: String,
    pub leader: bool,
    pub online: bool,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PartyInviteView {
    pub leader_name: String,
    pub expires_in_ticks: u64,
    pub actions: Vec<SceneAction>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct EconomyView {
    pub coins: u64,
    pub commodities: Vec<CommodityView>,
    pub shop_available: bool,
    pub shop: Vec<ShopItemView>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CommodityView {
    pub id: String,
    pub label: String,
    pub quantity: u32,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ShopItemView {
    pub id: String,
    pub label: String,
    pub buy_price: u64,
    pub sell_price: u64,
    pub stock: u32,
    pub owned: u32,
    pub actions: Vec<SceneAction>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SocialView {
    pub online: Vec<OnlineCharacterView>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OnlineCharacterView {
    pub id: String,
    pub name: String,
    pub self_character: bool,
}

impl SceneView {
    pub fn as_plain_text(&self) -> String {
        let mut output = format!("\r\n{}\r\n", self.title);
        for paragraph in &self.prose {
            output.push_str(paragraph);
            output.push_str("\r\n");
        }
        if !self.items.is_empty() {
            output.push_str("You see here: ");
            output.push_str(
                &self
                    .items
                    .iter()
                    .map(|item| item.label.as_str())
                    .collect::<Vec<_>>()
                    .join(", "),
            );
            output.push_str(".\r\n");
        }
        for npc in &self.npcs {
            output.push_str(&format!(
                "{} is here ({}/{} health).\r\n",
                npc.label, npc.health, npc.max_health
            ));
        }
        output.push_str(&format!(
            "Weather: {}; day {}, hour {}.\r\n",
            self.weather.condition, self.weather.day, self.weather.hour
        ));
        if self.exits.is_empty() {
            output.push_str("Exits: none\r\n");
        } else {
            let exits = self
                .exits
                .iter()
                .map(|exit| exit.label.as_str())
                .collect::<Vec<_>>()
                .join(", ");
            output.push_str(&format!("Exits: {exits}\r\n"));
        }
        output
    }
}

/// Transport- and authoring-format-independent input used to construct a world.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct WorldDefinition {
    pub start_room_id: String,
    pub items: Vec<ItemDefinition>,
    pub rooms: Vec<RoomDefinition>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ItemDefinition {
    pub stable_id: String,
    pub name: String,
    pub description: String,
    pub aliases: Vec<String>,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum EquipmentSlot {
    Weapon,
    Body,
    Trinket,
}

impl EquipmentSlot {
    fn parse(value: &str) -> Option<Self> {
        match value {
            "weapon" => Some(Self::Weapon),
            "body" | "armour" | "armor" => Some(Self::Body),
            "trinket" => Some(Self::Trinket),
            _ => None,
        }
    }

    fn as_str(self) -> &'static str {
        match self {
            Self::Weapon => "weapon",
            Self::Body => "body",
            Self::Trinket => "trinket",
        }
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum EffectKind {
    Regeneration,
    Strength,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ActiveEffect {
    pub kind: EffectKind,
    pub magnitude: i32,
    pub remaining_ticks: u32,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Vitals {
    pub health: i32,
    pub max_health: i32,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Attributes {
    pub strength: i32,
    pub defence: i32,
    #[serde(default)]
    pub additional: BTreeMap<String, i32>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CharacterGameplaySnapshot {
    pub lineage_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub background_id: Option<String>,
    pub base_class_id: String,
    pub active_class_id: String,
    pub class_history: Vec<String>,
    pub level: u32,
    pub experience: u64,
    pub vitals: Vitals,
    pub attributes: Attributes,
    #[serde(default)]
    pub resources: BTreeMap<String, ResourcePool>,
    #[serde(default)]
    pub resource_regeneration: BTreeMap<String, u32>,
    #[serde(default)]
    pub ability_ids: Vec<String>,
    /// Absolute logical tick at which an ability becomes available again.
    #[serde(default)]
    pub ability_cooldowns: BTreeMap<String, u64>,
    /// One bounded action slot per world tick prevents command spam from
    /// bypassing deterministic combat turns.
    #[serde(default)]
    pub ability_global_ready_tick: u64,
    /// Used by the combat loop to replace, rather than stack with, the normal
    /// player attack on the following resolved turn.
    #[serde(default)]
    pub last_offensive_ability_tick: Option<u64>,
    #[serde(default)]
    pub resistances: BTreeMap<DamageType, u16>,
    #[serde(default)]
    pub statuses: Vec<ActiveStatusEffect>,
    #[serde(default = "default_skills")]
    pub skills: BTreeMap<String, u16>,
    #[serde(default)]
    pub progression_points: BTreeMap<String, u16>,
    #[serde(default)]
    pub talent_node_ranks: BTreeMap<String, BTreeMap<String, u8>>,
    #[serde(default)]
    pub tags: BTreeSet<String>,
    #[serde(default)]
    pub received_signals: BTreeSet<String>,
    pub equipment: BTreeMap<EquipmentSlot, String>,
    pub effects: Vec<ActiveEffect>,
    pub recovery_at_tick: Option<u64>,
    #[serde(default = "default_coin_balance")]
    pub coins: u64,
    #[serde(default)]
    pub commodities: BTreeMap<String, u32>,
    #[serde(default)]
    pub quests: BTreeMap<String, QuestProgress>,
    /// Server-authorized script state. Flags are never projected directly to
    /// clients; they may only influence declarative content conditions.
    #[serde(default)]
    pub script_flags: BTreeSet<String>,
    #[serde(default)]
    pub scheduled_script_flags: Vec<ScheduledScriptFlag>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ScheduledScriptFlag {
    pub flag: String,
    pub due_tick: u64,
}

fn default_coin_balance() -> u64 {
    40
}

fn default_skills() -> BTreeMap<String, u16> {
    BTreeMap::from([
        ("combat".to_owned(), 10),
        ("perception".to_owned(), 10),
        ("survival".to_owned(), 10),
    ])
}

fn builtin_progression_definition() -> CharacterProgressionDefinition {
    CharacterProgressionDefinition {
        defaults: CharacterDefaultsDefinition {
            lineage_id: "core:human".into(),
            background_id: None,
            base_class_id: "core:wanderer".into(),
        },
        level_thresholds: (2..=MAX_PROGRESSION_LEVEL)
            .map(|level| LevelThresholdDefinition {
                level,
                total_experience_required: u64::from(level - 1) * 100,
                point_awards: BTreeMap::new(),
            })
            .collect(),
        lineages: vec![LineageDefinition {
            id: "core:human".into(),
            name: "Human".into(),
            description: "A versatile mortal lineage.".into(),
            ui_order: 0,
            creation_grants: Vec::new(),
            talent_graph_ids: Vec::new(),
        }],
        backgrounds: Vec::new(),
        base_classes: vec![BaseClassDefinition {
            id: "core:wanderer".into(),
            name: "Wanderer".into(),
            description: "A flexible adventurer with a balanced foundation.".into(),
            ui_order: 0,
            creation_grants: vec![
                GrantDefinition::Attribute {
                    attribute_id: "max_health".into(),
                    amount: 20,
                },
                GrantDefinition::Attribute {
                    attribute_id: "strength".into(),
                    amount: 5,
                },
                GrantDefinition::Attribute {
                    attribute_id: "defence".into(),
                    amount: 2,
                },
                GrantDefinition::Skill {
                    skill_id: "combat".into(),
                    amount: 10,
                },
                GrantDefinition::Skill {
                    skill_id: "exploration".into(),
                    amount: 5,
                },
            ],
            level_grants: (2..=MAX_PROGRESSION_LEVEL)
                .map(|level| LevelGrantDefinition {
                    level,
                    grants: vec![
                        GrantDefinition::Attribute {
                            attribute_id: "max_health".into(),
                            amount: 5,
                        },
                        GrantDefinition::Attribute {
                            attribute_id: "strength".into(),
                            amount: 2,
                        },
                        GrantDefinition::Attribute {
                            attribute_id: "defence".into(),
                            amount: 1,
                        },
                    ],
                })
                .collect(),
            talent_graph_ids: Vec::new(),
        }],
        advanced_classes: Vec::new(),
        talent_graphs: Vec::new(),
    }
}

fn valid_script_flag(flag: &str) -> bool {
    valid_definition_id(flag)
}

fn perception_senses(gameplay: &CharacterGameplaySnapshot) -> BTreeSet<Sense> {
    let mut senses = BTreeSet::from([Sense::Vision, Sense::Hearing]);
    if gameplay.script_flags.contains("condition.blind") {
        senses.remove(&Sense::Vision);
    }
    if gameplay.script_flags.contains("condition.deaf") {
        senses.remove(&Sense::Hearing);
    }
    if gameplay.script_flags.contains("sense.darkvision")
        && !gameplay.script_flags.contains("condition.blind")
    {
        senses.insert(Sense::Darkvision);
    }
    if gameplay.script_flags.contains("sense.tremorsense") {
        senses.insert(Sense::Tremorsense);
    }
    senses
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum QuestStatus {
    Active,
    ReadyToClaim,
    Completed,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct QuestProgress {
    pub status: QuestStatus,
    pub current: u32,
    pub target: u32,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum WeatherKind {
    Clear,
    Rain,
    Wind,
    Fog,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PartySnapshot {
    pub id: u64,
    pub leader: CharacterId,
    pub members: Vec<CharacterId>,
    pub invitations: BTreeMap<CharacterId, u64>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AdventureRuntimeSnapshot {
    pub next_party_id: u64,
    pub parties: BTreeMap<u64, PartySnapshot>,
    pub shop_stock: BTreeMap<String, u32>,
    pub weather: WeatherKind,
    pub next_weather_tick: u64,
}

impl Default for AdventureRuntimeSnapshot {
    fn default() -> Self {
        Self {
            next_party_id: 1,
            parties: BTreeMap::new(),
            shop_stock: BTreeMap::from([("tonic".into(), 20), ("trail-ration".into(), 20)]),
            weather: WeatherKind::Rain,
            next_weather_tick: 5,
        }
    }
}

/// Portable, durable state for one character. Identity/location and gameplay
/// are kept together so persistence cannot publish only half of an action.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DurableCharacterSnapshot {
    pub character: CharacterSnapshot,
    pub gameplay: CharacterGameplaySnapshot,
}

pub const WORLD_RUNTIME_SNAPSHOT_VERSION: u32 = 1;
const MAX_ACTIVE_EFFECTS: usize = 32;

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum RuntimeItemLocation {
    /// Reserved during world construction for a content-defined initial owner.
    /// Staged items are never projected into a room scene.
    Staged,
    Carried {
        character_id: CharacterId,
    },
    Room {
        room_id: String,
    },
    Equipped {
        character_id: CharacterId,
        slot: EquipmentSlot,
    },
    HeldByNpc {
        npc_id: String,
    },
    Consumed,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct NpcRuntimeSnapshot {
    pub stable_id: String,
    pub name: String,
    pub description: String,
    pub spawn_room_id: String,
    pub level: u32,
    pub health: i32,
    pub max_health: i32,
    pub strength: i32,
    pub defence: i32,
    pub experience_reward: u64,
    pub loot_item_ids: Vec<String>,
    pub respawn_ticks: u64,
    pub respawn_at_tick: Option<u64>,
    pub engaged_character: Option<CharacterId>,
    #[serde(default)]
    pub resources: BTreeMap<String, ResourcePool>,
    #[serde(default)]
    pub resource_regeneration: BTreeMap<String, u32>,
    #[serde(default)]
    pub ability_ids: Vec<String>,
    #[serde(default)]
    pub ability_cooldowns: BTreeMap<String, u64>,
    #[serde(default)]
    pub resistances: BTreeMap<DamageType, u16>,
    #[serde(default)]
    pub statuses: Vec<ActiveStatusEffect>,
}

/// Portable shared-world state. It deliberately excludes sessions and opaque
/// observer IDs, which are transport lifetime state and are rebuilt on start.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WorldRuntimeSnapshot {
    pub snapshot_version: u32,
    pub revision: u64,
    pub logical_tick: u64,
    pub rng_state: u64,
    pub item_locations: BTreeMap<String, RuntimeItemLocation>,
    pub npcs: BTreeMap<String, NpcRuntimeSnapshot>,
    #[serde(default)]
    pub adventure: AdventureRuntimeSnapshot,
    #[serde(default)]
    pub progression_flags: BTreeSet<String>,
}

impl Default for CharacterGameplaySnapshot {
    fn default() -> Self {
        Self {
            lineage_id: "core:human".into(),
            background_id: None,
            base_class_id: "core:wanderer".into(),
            active_class_id: "core:wanderer".into(),
            class_history: vec!["core:wanderer".into()],
            level: 1,
            experience: 0,
            vitals: Vitals {
                health: 20,
                max_health: 20,
            },
            attributes: Attributes {
                strength: 5,
                defence: 2,
                additional: BTreeMap::new(),
            },
            resources: BTreeMap::new(),
            resource_regeneration: BTreeMap::new(),
            ability_ids: Vec::new(),
            ability_cooldowns: BTreeMap::new(),
            ability_global_ready_tick: 0,
            last_offensive_ability_tick: None,
            resistances: BTreeMap::new(),
            statuses: Vec::new(),
            skills: default_skills(),
            progression_points: BTreeMap::new(),
            talent_node_ranks: BTreeMap::new(),
            tags: BTreeSet::new(),
            received_signals: BTreeSet::new(),
            equipment: BTreeMap::new(),
            effects: Vec::new(),
            recovery_at_tick: None,
            coins: default_coin_balance(),
            commodities: BTreeMap::new(),
            quests: BTreeMap::new(),
            script_flags: BTreeSet::new(),
            scheduled_script_flags: Vec::new(),
        }
    }
}

/// Whitelisted runtime facts available to the server-side content engine.
/// Stable IDs remain server-internal and this type is deliberately not serializable.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AuthorizedRuntimeContext {
    pub character_id: CharacterId,
    pub character_name: String,
    pub room_id: String,
    pub actor_level: u32,
    pub actor_flags: BTreeSet<String>,
    pub world_instance: String,
    pub actor_quest_states: BTreeSet<String>,
    pub actor_content_preferences: BTreeSet<String>,
    pub actor_senses: BTreeSet<String>,
    pub logical_tick: u64,
    pub weather: WeatherKind,
}

/// Server-internal join key for presentation annotations. This type is
/// intentionally not serializable: stable IDs must never cross a transport
/// boundary, while `opaque_id` is safe only for this observer session.
#[derive(Clone, Debug, Eq, PartialEq)]
#[doc(hidden)]
pub struct VisibleSceneEntityId {
    pub stable_id: String,
    pub opaque_id: String,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
#[doc(hidden)]
pub struct VisibleSceneEntityIds {
    pub items: Vec<VisibleSceneEntityId>,
    pub npcs: Vec<VisibleSceneEntityId>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ItemGameplayDefinition {
    pub item_id: String,
    pub equipment_slot: Option<EquipmentSlot>,
    pub attack_bonus: i32,
    pub heal_amount: i32,
    pub consumable: bool,
    pub effect: Option<ActiveEffect>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct NpcDefinition {
    pub stable_id: String,
    pub name: String,
    pub description: String,
    pub room_id: String,
    pub level: u32,
    pub max_health: i32,
    pub strength: i32,
    pub defence: i32,
    pub experience_reward: u64,
    pub loot_item_ids: Vec<String>,
    pub respawn_ticks: u64,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RoomDefinition {
    pub stable_id: String,
    pub title: String,
    pub prose: Vec<String>,
    pub image: Option<SceneImage>,
    pub audio: Option<SceneAudio>,
    pub exits: Vec<ExitDefinition>,
    pub item_ids: Vec<String>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ExitDefinition {
    pub direction: Direction,
    pub target_room_id: String,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum LightLevel {
    Bright,
    Dim,
    Dark,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Sense {
    Vision,
    Hearing,
    Darkvision,
    Tremorsense,
}

#[derive(Debug, Error, Eq, PartialEq)]
pub enum WorldBuildError {
    #[error("A world must contain at least one room.")]
    EmptyWorld,
    #[error("A room has an empty stable ID.")]
    EmptyRoomId,
    #[error("Room ID {0:?} appears more than once.")]
    DuplicateRoomId(String),
    #[error("Start room {0:?} does not exist.")]
    UnknownStartRoom(String),
    #[error("Room {room_id:?} points to missing room {target_room_id:?}.")]
    UnknownExitTarget {
        room_id: String,
        target_room_id: String,
    },
    #[error("Room {room_id:?} defines direction {direction:?} more than once.")]
    DuplicateExitDirection {
        room_id: String,
        direction: Direction,
    },
    #[error("An item has an empty stable ID.")]
    EmptyItemId,
    #[error("Item ID {0:?} appears more than once.")]
    DuplicateItemId(String),
    #[error("Item {0:?} must have a name.")]
    EmptyItemName(String),
    #[error("Room {room_id:?} points to missing item {item_id:?}.")]
    UnknownRoomItem { room_id: String, item_id: String },
    #[error("Room {room_id:?} lists item {item_id:?} more than once.")]
    DuplicateRoomItem { room_id: String, item_id: String },
    #[error("Item {item_id:?} is placed in both {first_room_id:?} and {second_room_id:?}.")]
    ItemPlacedMoreThanOnce {
        item_id: String,
        first_room_id: String,
        second_room_id: String,
    },
    #[error("Item {0:?} has no initial room.")]
    UnplacedItem(String),
    #[error("Staged item {0:?} is not declared by the world.")]
    UnknownStagedItem(String),
    #[error("Staged item {0:?} appears more than once.")]
    DuplicateStagedItem(String),
    #[error("Item {item_id:?} is both placed in room {room_id:?} and staged for an initial owner.")]
    ItemPlacedAndStaged { item_id: String, room_id: String },
}

#[derive(Clone, Debug)]
struct Item {
    stable_id: String,
    name: String,
    description: String,
    aliases: Vec<String>,
}

#[derive(Clone, Debug)]
struct Room {
    stable_id: String,
    title: String,
    prose: Vec<String>,
    image: Option<SceneImage>,
    audio: Option<SceneAudio>,
    exits: BTreeMap<Direction, EntityId>,
}

#[derive(Clone, Debug)]
struct Character {
    id: CharacterId,
    account_id: AccountId,
    name: String,
    location: EntityId,
}

#[derive(Clone, Debug, Eq, PartialEq)]
enum SharedItemLocation {
    Staged,
    Carried(CharacterId),
    Room(String),
    Equipped(CharacterId, EquipmentSlot),
    HeldByNpc(String),
    Consumed,
}

#[derive(Clone, Debug)]
struct Npc {
    stable_id: String,
    name: String,
    description: String,
    spawn_room_id: String,
    level: u32,
    health: i32,
    max_health: i32,
    strength: i32,
    defence: i32,
    experience_reward: u64,
    loot_item_ids: Vec<String>,
    respawn_ticks: u64,
    respawn_at_tick: Option<u64>,
    engaged_character: Option<CharacterId>,
    resources: BTreeMap<String, ResourcePool>,
    resource_regeneration: BTreeMap<String, u32>,
    ability_ids: Vec<String>,
    ability_cooldowns: BTreeMap<String, u64>,
    resistances: BTreeMap<DamageType, u16>,
    statuses: Vec<ActiveStatusEffect>,
}

impl Npc {
    fn alive(&self) -> bool {
        self.health > 0 && self.respawn_at_tick.is_none()
    }
}

#[derive(Clone, Debug)]
struct DeterministicRng {
    state: u64,
}

impl DeterministicRng {
    fn new(seed: u64) -> Self {
        Self { state: seed.max(1) }
    }

    fn next_u64(&mut self) -> u64 {
        let mut value = self.state;
        value ^= value << 13;
        value ^= value >> 7;
        value ^= value << 17;
        self.state = value;
        value
    }

    fn range_inclusive(&mut self, minimum: i32, maximum: i32) -> i32 {
        debug_assert!(minimum <= maximum);
        minimum + (self.next_u64() % (maximum - minimum + 1) as u64) as i32
    }
}

#[derive(Clone, Debug)]
struct ObserverView {
    salt: u64,
    revision: u64,
    discovered_rooms: BTreeSet<String>,
    role: AccountRole,
}

#[derive(Clone, Debug)]
enum Entity {
    Room(Box<Room>),
    Character(Character),
}

#[derive(Debug, Error)]
pub enum WorldError {
    #[error("No character is attached to this session.")]
    UnknownSession,
    #[error("That character is already connected.")]
    CharacterAlreadyConnected,
    #[error("There is no exit in that direction.")]
    NoExit,
    #[error("The world contains an invalid entity reference.")]
    InvalidEntity,
    #[error("No accessible item matches {0:?}.")]
    ItemNotFound(String),
    #[error("{query:?} is ambiguous; it could mean: {matches}.")]
    AmbiguousItem { query: String, matches: String },
    #[error("A persisted domain event references invalid world data.")]
    InvalidDomainEvent,
    #[error("A persisted domain event is inconsistent with authoritative world state.")]
    InvalidDomainTransition,
    #[error("That gameplay action is not available.")]
    GameplayUnavailable,
    #[error("The gameplay definition is invalid.")]
    InvalidGameplayDefinition,
    #[error("No visible NPC matches that target.")]
    NpcNotFound,
    #[error("That NPC name is ambiguous.")]
    AmbiguousNpc,
    #[error("That item cannot be equipped.")]
    ItemNotEquippable,
    #[error("That item cannot be used.")]
    ItemNotUsable,
    #[error("Nothing is equipped in that slot.")]
    EquipmentSlotEmpty,
    #[error("Combat cannot be started right now.")]
    CombatUnavailable,
    #[error("No lineage matches that identifier.")]
    LineageNotFound,
    #[error("No background matches that identifier.")]
    BackgroundNotFound,
    #[error("No class matches that identifier.")]
    ClassNotFound,
    #[error("That class promotion is not available.")]
    PromotionUnavailable,
    #[error("That talent rank is not available.")]
    TalentUnavailable,
    #[error("There are not enough progression points.")]
    InsufficientProgressionPoints,
    #[error("A progression grant would exceed an engine cap.")]
    ProgressionCapExceeded,
    #[error("The authored progression configuration is invalid.")]
    InvalidProgressionDefinition,
    #[error("The progression transition is invalid.")]
    InvalidProgressionTransition,
    #[error("The authored ability configuration is invalid.")]
    InvalidAbilityDefinition,
    #[error("That ability is unknown or has not been learned.")]
    AbilityNotKnown,
    #[error("That ability cannot be used right now.")]
    AbilityUnavailable,
    #[error("There are not enough resources to use that ability.")]
    InsufficientResource,
    #[error("That ability target is not valid.")]
    InvalidAbilityTarget,
    #[error("The persisted world runtime snapshot is invalid or incompatible.")]
    InvalidRuntimeSnapshot,
    #[error("That party action is not available.")]
    PartyUnavailable,
    #[error("Only the party leader may do that.")]
    PartyForbidden,
    #[error("No connected character matches that party target.")]
    PartyTargetNotFound,
    #[error("That quest action is not available.")]
    QuestUnavailable,
    #[error("That shop action is not available here.")]
    ShopUnavailable,
    #[error("The shop does not carry that commodity.")]
    CommodityNotFound,
    #[error("You do not have enough coin.")]
    InsufficientFunds,
    #[error("You do not own that commodity.")]
    CommodityNotOwned,
    #[error("A script requested an invalid or excessive authorized flag.")]
    InvalidScriptFlag,
    #[error("A script requested an invalid or excessive authorized effect.")]
    InvalidScriptEffect,
}

/// Authoritative facts emitted after a successful world-state transition.
/// These IDs belong to the domain and must never be sent directly to clients;
/// use [`World::project_event`] to obtain an observer-specific [`ViewEvent`].
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum DomainEvent {
    CharacterMoved {
        actor_id: CharacterId,
        actor_name: String,
        from_room_id: String,
        to_room_id: String,
        direction: Direction,
    },
    ItemTaken {
        actor_id: CharacterId,
        actor_name: String,
        item_id: String,
        item_name: String,
        room_id: String,
    },
    ItemDropped {
        actor_id: CharacterId,
        actor_name: String,
        item_id: String,
        item_name: String,
        room_id: String,
    },
    Gameplay {
        event: GameplayEvent,
    },
    Adventure {
        event: AdventureEvent,
    },
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "eventKind", rename_all = "camelCase")]
pub enum AdventureEvent {
    PartyChanged {
        character_id: CharacterId,
        party_id: u64,
        #[serde(default)]
        audience: Vec<CharacterId>,
        summary: String,
    },
    QuestChanged {
        character_id: CharacterId,
        quest_id: String,
        summary: String,
    },
    EconomyChanged {
        character_id: CharacterId,
        commodity_id: Option<String>,
        summary: String,
    },
    WeatherChanged {
        weather: WeatherKind,
        tick: u64,
    },
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "eventKind", rename_all = "camelCase")]
pub enum GameplayEvent {
    TickAdvanced {
        tick: u64,
    },
    NpcSpawned {
        npc_id: String,
        npc_name: String,
        room_id: String,
    },
    CombatStarted {
        character_id: CharacterId,
        character_name: String,
        npc_id: String,
        npc_name: String,
        room_id: String,
    },
    AbilityUsed {
        character_id: CharacterId,
        character_name: String,
        ability_id: String,
        ability_name: String,
        npc_id: Option<String>,
        npc_name: Option<String>,
        room_id: String,
    },
    NpcAbilityUsed {
        npc_id: String,
        npc_name: String,
        character_id: CharacterId,
        character_name: String,
        ability_id: String,
        ability_name: String,
        room_id: String,
    },
    NpcDamaged {
        character_id: CharacterId,
        character_name: String,
        npc_id: String,
        npc_name: String,
        amount: i32,
        remaining_health: i32,
        room_id: String,
    },
    CharacterDamaged {
        character_id: CharacterId,
        character_name: String,
        npc_id: String,
        npc_name: String,
        amount: i32,
        remaining_health: i32,
        room_id: String,
    },
    NpcDefeated {
        character_id: CharacterId,
        character_name: String,
        npc_id: String,
        npc_name: String,
        respawn_at_tick: u64,
        room_id: String,
    },
    LootDropped {
        npc_id: String,
        npc_name: String,
        item_id: String,
        item_name: String,
        room_id: String,
    },
    ExperienceGained {
        character_id: CharacterId,
        character_name: String,
        amount: u64,
        total: u64,
        room_id: String,
    },
    CharacterLeveled {
        character_id: CharacterId,
        character_name: String,
        level: u32,
        room_id: String,
    },
    CharacterPromoted {
        character_id: CharacterId,
        character_name: String,
        from_class_id: String,
        to_class_id: String,
        room_id: String,
    },
    TalentRankAcquired {
        character_id: CharacterId,
        character_name: String,
        graph_id: String,
        node_id: String,
        rank: u8,
        room_id: String,
    },
    CharacterDefeated {
        character_id: CharacterId,
        character_name: String,
        npc_id: String,
        npc_name: String,
        recover_at_tick: u64,
        room_id: String,
    },
    CharacterCollapsed {
        character_id: CharacterId,
        character_name: String,
        recover_at_tick: u64,
        room_id: String,
    },
    CharacterRecovered {
        character_id: CharacterId,
        character_name: String,
        room_id: String,
    },
    NpcRespawned {
        npc_id: String,
        npc_name: String,
        room_id: String,
    },
    ItemEquipped {
        character_id: CharacterId,
        character_name: String,
        item_id: String,
        item_name: String,
        slot: EquipmentSlot,
        room_id: String,
    },
    ItemUnequipped {
        character_id: CharacterId,
        character_name: String,
        item_id: String,
        item_name: String,
        slot: EquipmentSlot,
        room_id: String,
    },
    ItemUsed {
        character_id: CharacterId,
        character_name: String,
        item_id: String,
        item_name: String,
        room_id: String,
    },
    CharacterHealed {
        character_id: CharacterId,
        character_name: String,
        amount: i32,
        health: i32,
        room_id: String,
    },
    EffectApplied {
        character_id: CharacterId,
        character_name: String,
        effect: ActiveEffect,
        room_id: String,
    },
    EffectExpired {
        character_id: CharacterId,
        character_name: String,
        kind: EffectKind,
        room_id: String,
    },
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum ViewEventKind {
    CharacterMoved,
    ItemTaken,
    ItemDropped,
    Gameplay,
    Adventure,
}

/// Perception-safe projection of one domain event for one observer.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ViewEvent {
    pub revision: u64,
    pub kind: ViewEventKind,
    pub text: String,
    pub actor_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub target_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub item_id: Option<String>,
}

#[derive(Clone, Debug)]
pub struct ActionOutcome {
    pub scene: SceneView,
    /// Optional transport-independent, plain-text response to the command.
    pub output: Option<String>,
    /// Present when the action changed persistent character state.
    pub changed_character: Option<CharacterSnapshot>,
    /// Authoritative events to persist and then project to connected views.
    pub domain_events: Vec<DomainEvent>,
}

/// A single-owner world graph. Callers are expected to place it behind an actor.
pub struct World {
    entities: SlotMap<EntityId, Entity>,
    stable_rooms: HashMap<String, EntityId>,
    stable_items: HashMap<String, Item>,
    item_locations: BTreeMap<String, SharedItemLocation>,
    item_gameplay: HashMap<String, ItemGameplayDefinition>,
    room_light: HashMap<String, LightLevel>,
    hidden_items: HashMap<String, u16>,
    hidden_npcs: HashMap<String, u16>,
    secret_exits: HashMap<(String, Direction), u16>,
    npcs: BTreeMap<String, Npc>,
    abilities: BTreeMap<String, AbilityDefinition>,
    progression: ProgressionCatalog,
    progression_catalog: CharacterCreationCatalogView,
    world_progression_flags: BTreeSet<String>,
    character_gameplay: HashMap<CharacterId, CharacterGameplaySnapshot>,
    sessions: HashMap<SessionId, EntityId>,
    observer_views: HashMap<SessionId, ObserverView>,
    next_observer_salt: u64,
    last_view_bump: Option<(u64, Vec<(SessionId, u64)>)>,
    start_room: EntityId,
    start_room_stable_id: String,
    revision: u64,
    logical_tick: u64,
    rng: DeterministicRng,
    adventure: AdventureRuntimeSnapshot,
}

/// Exact in-memory checkpoint used around one durable commit. Unlike the
/// portable snapshots this also contains observer revision bookkeeping, so a
/// failed database transaction is invisible to connected clients.
#[derive(Clone)]
pub struct WorldCheckpoint {
    entities: SlotMap<EntityId, Entity>,
    item_locations: BTreeMap<String, SharedItemLocation>,
    npcs: BTreeMap<String, Npc>,
    character_gameplay: HashMap<CharacterId, CharacterGameplaySnapshot>,
    observer_views: HashMap<SessionId, ObserverView>,
    last_view_bump: Option<(u64, Vec<(SessionId, u64)>)>,
    revision: u64,
    logical_tick: u64,
    rng: DeterministicRng,
    adventure: AdventureRuntimeSnapshot,
    world_progression_flags: BTreeSet<String>,
}

impl World {
    pub fn from_definition(definition: WorldDefinition) -> Result<Self, WorldBuildError> {
        Self::from_definition_with_staged_items(definition, Vec::new())
    }

    /// Builds a world while reserving explicitly named items for initial
    /// ownership assignment (for example, NPC loot). A staged item is not
    /// visible in any room and can only be claimed by an NPC spawn.
    pub fn from_definition_with_staged_items(
        definition: WorldDefinition,
        staged_item_ids: Vec<String>,
    ) -> Result<Self, WorldBuildError> {
        if definition.rooms.is_empty() {
            return Err(WorldBuildError::EmptyWorld);
        }

        let mut entities = SlotMap::with_key();
        let mut stable_rooms = HashMap::with_capacity(definition.rooms.len());
        let mut stable_items = HashMap::with_capacity(definition.items.len());
        for item in definition.items {
            if item.stable_id.trim().is_empty() {
                return Err(WorldBuildError::EmptyItemId);
            }
            if item.name.trim().is_empty() {
                return Err(WorldBuildError::EmptyItemName(item.stable_id));
            }
            let stable_id = item.stable_id;
            if stable_items.contains_key(&stable_id) {
                return Err(WorldBuildError::DuplicateItemId(stable_id));
            }
            stable_items.insert(
                stable_id.clone(),
                Item {
                    stable_id,
                    name: item.name,
                    description: item.description,
                    aliases: item.aliases,
                },
            );
        }
        let mut unresolved_exits = Vec::with_capacity(definition.rooms.len());
        let mut unresolved_room_items = Vec::with_capacity(definition.rooms.len());

        for definition in definition.rooms {
            if definition.stable_id.trim().is_empty() {
                return Err(WorldBuildError::EmptyRoomId);
            }
            if stable_rooms.contains_key(&definition.stable_id) {
                return Err(WorldBuildError::DuplicateRoomId(definition.stable_id));
            }

            let stable_id = definition.stable_id;
            let room_id = entities.insert(Entity::Room(Box::new(Room {
                stable_id: stable_id.clone(),
                title: definition.title,
                prose: definition.prose,
                image: definition.image,
                audio: definition.audio,
                exits: BTreeMap::new(),
            })));
            stable_rooms.insert(stable_id, room_id);
            unresolved_exits.push((room_id, definition.exits));
            unresolved_room_items.push((room_id, definition.item_ids));
        }

        let start_room = stable_rooms
            .get(&definition.start_room_id)
            .copied()
            .ok_or_else(|| WorldBuildError::UnknownStartRoom(definition.start_room_id.clone()))?;

        for (room_id, exits) in unresolved_exits {
            let source_id = room(&entities, room_id).stable_id.clone();
            for exit in exits {
                let destination =
                    stable_rooms
                        .get(&exit.target_room_id)
                        .copied()
                        .ok_or_else(|| WorldBuildError::UnknownExitTarget {
                            room_id: source_id.clone(),
                            target_room_id: exit.target_room_id.clone(),
                        })?;
                let previous = room_mut(&mut entities, room_id)
                    .exits
                    .insert(exit.direction, destination);
                if previous.is_some() {
                    return Err(WorldBuildError::DuplicateExitDirection {
                        room_id: source_id.clone(),
                        direction: exit.direction,
                    });
                }
            }
        }

        let mut item_locations = BTreeMap::new();
        for (room_id, item_ids) in unresolved_room_items {
            let source_id = room(&entities, room_id).stable_id.clone();
            let mut seen = std::collections::HashSet::new();
            for item_id in item_ids {
                if !seen.insert(item_id.clone()) {
                    return Err(WorldBuildError::DuplicateRoomItem {
                        room_id: source_id,
                        item_id,
                    });
                }
                if !stable_items.contains_key(&item_id) {
                    return Err(WorldBuildError::UnknownRoomItem {
                        room_id: source_id,
                        item_id,
                    });
                }
                if let Some(SharedItemLocation::Room(first_room_id)) = item_locations
                    .insert(item_id.clone(), SharedItemLocation::Room(source_id.clone()))
                {
                    return Err(WorldBuildError::ItemPlacedMoreThanOnce {
                        item_id,
                        first_room_id,
                        second_room_id: source_id,
                    });
                }
            }
        }
        let mut staged = std::collections::HashSet::new();
        for item_id in staged_item_ids {
            if !staged.insert(item_id.clone()) {
                return Err(WorldBuildError::DuplicateStagedItem(item_id));
            }
            if !stable_items.contains_key(&item_id) {
                return Err(WorldBuildError::UnknownStagedItem(item_id));
            }
            if let Some(SharedItemLocation::Room(room_id)) = item_locations.get(&item_id) {
                return Err(WorldBuildError::ItemPlacedAndStaged {
                    item_id,
                    room_id: room_id.clone(),
                });
            }
            item_locations.insert(item_id, SharedItemLocation::Staged);
        }
        for item_id in stable_items.keys() {
            if !item_locations.contains_key(item_id) {
                return Err(WorldBuildError::UnplacedItem(item_id.clone()));
            }
        }

        let progression =
            ProgressionCatalog::compile(builtin_progression_definition(), &BTreeMap::new())
                .expect("the built-in progression is valid");
        let progression_catalog = progression.creation_view();

        Ok(Self {
            entities,
            stable_rooms,
            stable_items,
            item_locations,
            item_gameplay: HashMap::new(),
            room_light: HashMap::new(),
            hidden_items: HashMap::new(),
            hidden_npcs: HashMap::new(),
            secret_exits: HashMap::new(),
            npcs: BTreeMap::new(),
            abilities: BTreeMap::new(),
            progression,
            progression_catalog,
            world_progression_flags: BTreeSet::new(),
            character_gameplay: HashMap::new(),
            sessions: HashMap::new(),
            observer_views: HashMap::new(),
            next_observer_salt: 1,
            last_view_bump: None,
            start_room,
            start_room_stable_id: definition.start_room_id,
            revision: 1,
            logical_tick: 0,
            rng: DeterministicRng::new(0x5449_414d_5544),
            adventure: AdventureRuntimeSnapshot::default(),
        })
    }

    pub fn demo() -> Self {
        Self::from_definition(WorldDefinition {
            start_room_id: "demo:lantern-square".into(),
            items: vec![ItemDefinition {
                stable_id: "demo:blue-lantern".into(),
                name: "blue-glass lantern".into(),
                description: "Cold blue light pools inside the rain-speckled glass.".into(),
                aliases: vec!["lantern".into(), "lamp".into()],
            }],
            rooms: vec![
                RoomDefinition {
                    stable_id: "demo:lantern-square".into(),
                    title: "Lantern Square".into(),
                    prose: vec![
                        "Rain has polished the old stones to a dark shine. Beneath the clock tower, a few late travellers gather around a blue-glass lantern.".into(),
                        "The illustration, when available, is atmospheric only; the text remains the complete source of playable information.".into(),
                    ],
                    image: None,
                    audio: None,
                    exits: vec![
                        ExitDefinition {
                            direction: Direction::North,
                            target_room_id: "demo:north-gate".into(),
                        },
                        ExitDefinition {
                            direction: Direction::Down,
                            target_room_id: "demo:archive".into(),
                        },
                    ],
                    item_ids: vec!["demo:blue-lantern".into()],
                },
                RoomDefinition {
                    stable_id: "demo:north-gate".into(),
                    title: "The North Gate".into(),
                    prose: vec![
                        "An iron-bound gate stands open between squat stone towers. The road continues into fields silvered by rain.".into(),
                    ],
                    image: None,
                    audio: None,
                    exits: vec![ExitDefinition {
                        direction: Direction::South,
                        target_room_id: "demo:lantern-square".into(),
                    }],
                    item_ids: Vec::new(),
                },
                RoomDefinition {
                    stable_id: "demo:archive".into(),
                    title: "The Cartographer's Archive".into(),
                    prose: vec![
                        "Shelves of annotated maps surround a broad oak table. A dry stair climbs back toward the square.".into(),
                    ],
                    image: None,
                    audio: None,
                    exits: vec![ExitDefinition {
                        direction: Direction::Up,
                        target_room_id: "demo:lantern-square".into(),
                    }],
                    item_ids: Vec::new(),
                },
            ],
        })
        .expect("the built-in demo world is valid")
    }

    /// Atomically installs one reviewed character progression and ability catalog.
    pub fn configure_progression(
        &mut self,
        definition: CharacterProgressionDefinition,
        abilities: Vec<AbilityDefinition>,
    ) -> Result<(), WorldError> {
        if abilities.len() > MAX_ABILITIES {
            return Err(WorldError::InvalidAbilityDefinition);
        }
        let ability_count = abilities.len();
        let unique_ability_names = abilities
            .iter()
            .map(|ability| ability.name.to_lowercase())
            .collect::<BTreeSet<_>>();
        if unique_ability_names.len() != ability_count {
            return Err(WorldError::InvalidAbilityDefinition);
        }
        let ability_map = abilities
            .into_iter()
            .map(|ability| {
                validate_ability_definition(&ability)?;
                Ok((ability.id.clone(), ability))
            })
            .collect::<Result<BTreeMap<_, _>, WorldError>>()?;
        if ability_map.len() != ability_count || ability_map.len() > MAX_ABILITIES {
            return Err(WorldError::InvalidAbilityDefinition);
        }

        let progression = ProgressionCatalog::compile(definition, &ability_map)?;
        self.progression_catalog = progression.creation_view();
        self.progression = progression;
        self.abilities = ability_map;
        Ok(())
    }

    pub fn progression_catalog(&self) -> &CharacterCreationCatalogView {
        &self.progression_catalog
    }

    pub fn initial_gameplay(
        &self,
        lineage_id: Option<&str>,
        background: BackgroundSelection<'_>,
        base_class_id: Option<&str>,
    ) -> Result<CharacterGameplaySnapshot, WorldError> {
        self.progression
            .initial_gameplay(lineage_id, background, base_class_id, &self.abilities)
    }

    pub fn configure_npc_abilities(
        &mut self,
        definition: NpcAbilitySetDefinition,
    ) -> Result<(), WorldError> {
        let unique_abilities = definition
            .ability_ids
            .iter()
            .collect::<std::collections::HashSet<_>>()
            .len()
            == definition.ability_ids.len();
        let abilities_usable = definition.ability_ids.iter().all(|id| {
            self.abilities.get(id).is_some_and(|ability| {
                ability
                    .resource_costs
                    .keys()
                    .all(|resource| definition.resource_pools.contains_key(resource))
            })
        });
        let npc = self
            .npcs
            .get_mut(&definition.npc_id)
            .ok_or(WorldError::NpcNotFound)?;
        if definition.resource_pools.len() > MAX_RESOURCE_POOLS
            || definition.resource_regeneration.len() > MAX_RESOURCE_POOLS
            || definition.resistances.len() > MAX_RESISTANCES
            || definition.ability_ids.len() > MAX_KNOWN_ABILITIES
            || !unique_abilities
            || !abilities_usable
            || definition
                .resource_pools
                .iter()
                .any(|(id, value)| !valid_stat_id(id) || *value == 0 || *value > MAX_RESOURCE_VALUE)
            || definition.resource_regeneration.iter().any(|(id, value)| {
                !valid_stat_id(id)
                    || !definition.resource_pools.contains_key(id)
                    || *value > MAX_RESOURCE_VALUE
            })
            || definition
                .resistances
                .values()
                .any(|value| *value > MAX_RESISTANCE_PERCENT)
        {
            return Err(WorldError::InvalidAbilityDefinition);
        }
        npc.resources = definition
            .resource_pools
            .into_iter()
            .map(|(id, maximum)| {
                (
                    id,
                    ResourcePool {
                        current: maximum,
                        max: maximum,
                    },
                )
            })
            .collect();
        npc.resource_regeneration = definition.resource_regeneration;
        npc.resistances = definition.resistances;
        npc.ability_ids = definition.ability_ids;
        npc.ability_cooldowns.clear();
        Ok(())
    }

    pub fn start_room_id(&self) -> &str {
        &self.start_room_stable_id
    }

    pub fn revision(&self) -> u64 {
        self.revision
    }

    pub fn logical_tick(&self) -> u64 {
        self.logical_tick
    }

    pub fn set_rng_seed(&mut self, seed: u64) {
        self.rng = DeterministicRng::new(seed);
    }

    pub fn checkpoint(&self) -> WorldCheckpoint {
        WorldCheckpoint {
            entities: self.entities.clone(),
            item_locations: self.item_locations.clone(),
            npcs: self.npcs.clone(),
            character_gameplay: self.character_gameplay.clone(),
            observer_views: self.observer_views.clone(),
            last_view_bump: self.last_view_bump.clone(),
            revision: self.revision,
            logical_tick: self.logical_tick,
            rng: self.rng.clone(),
            adventure: self.adventure.clone(),
            world_progression_flags: self.world_progression_flags.clone(),
        }
    }

    pub fn restore_checkpoint(&mut self, checkpoint: WorldCheckpoint) {
        self.entities = checkpoint.entities;
        self.item_locations = checkpoint.item_locations;
        self.npcs = checkpoint.npcs;
        self.character_gameplay = checkpoint.character_gameplay;
        self.observer_views = checkpoint.observer_views;
        self.last_view_bump = checkpoint.last_view_bump;
        self.revision = checkpoint.revision;
        self.logical_tick = checkpoint.logical_tick;
        self.rng = checkpoint.rng;
        self.adventure = checkpoint.adventure;
        self.world_progression_flags = checkpoint.world_progression_flags;
    }

    pub fn runtime_snapshot(&self) -> WorldRuntimeSnapshot {
        WorldRuntimeSnapshot {
            snapshot_version: WORLD_RUNTIME_SNAPSHOT_VERSION,
            revision: self.revision,
            logical_tick: self.logical_tick,
            rng_state: self.rng.state,
            item_locations: self
                .item_locations
                .iter()
                .map(|(item_id, location)| {
                    let location = match location {
                        SharedItemLocation::Staged => RuntimeItemLocation::Staged,
                        SharedItemLocation::Carried(character_id) => RuntimeItemLocation::Carried {
                            character_id: *character_id,
                        },
                        SharedItemLocation::Room(room_id) => RuntimeItemLocation::Room {
                            room_id: room_id.clone(),
                        },
                        SharedItemLocation::Equipped(character_id, slot) => {
                            RuntimeItemLocation::Equipped {
                                character_id: *character_id,
                                slot: *slot,
                            }
                        }
                        SharedItemLocation::HeldByNpc(npc_id) => RuntimeItemLocation::HeldByNpc {
                            npc_id: npc_id.clone(),
                        },
                        SharedItemLocation::Consumed => RuntimeItemLocation::Consumed,
                    };
                    (item_id.clone(), location)
                })
                .collect(),
            npcs: self
                .npcs
                .iter()
                .map(|(npc_id, npc)| {
                    (
                        npc_id.clone(),
                        NpcRuntimeSnapshot {
                            stable_id: npc.stable_id.clone(),
                            name: npc.name.clone(),
                            description: npc.description.clone(),
                            spawn_room_id: npc.spawn_room_id.clone(),
                            level: npc.level,
                            health: npc.health,
                            max_health: npc.max_health,
                            strength: npc.strength,
                            defence: npc.defence,
                            experience_reward: npc.experience_reward,
                            loot_item_ids: npc.loot_item_ids.clone(),
                            respawn_ticks: npc.respawn_ticks,
                            respawn_at_tick: npc.respawn_at_tick,
                            engaged_character: npc.engaged_character,
                            resources: npc.resources.clone(),
                            resource_regeneration: npc.resource_regeneration.clone(),
                            ability_ids: npc.ability_ids.clone(),
                            ability_cooldowns: npc.ability_cooldowns.clone(),
                            resistances: npc.resistances.clone(),
                            statuses: npc.statuses.clone(),
                        },
                    )
                })
                .collect(),
            adventure: self.adventure.clone(),
            progression_flags: self.world_progression_flags.clone(),
        }
    }

    pub fn restore_runtime_snapshot(
        &mut self,
        mut snapshot: WorldRuntimeSnapshot,
    ) -> Result<(), WorldError> {
        let item_ids_match = snapshot
            .item_locations
            .keys()
            .all(|item_id| self.stable_items.contains_key(item_id));
        let npc_ids_match = snapshot
            .npcs
            .iter()
            .all(|(npc_id, npc)| npc_id == &npc.stable_id && self.npcs.contains_key(npc_id));
        let mut party_members = std::collections::HashSet::new();
        let adventure_valid = snapshot.adventure.next_party_id > 0
            && snapshot.adventure.parties.iter().all(|(party_id, party)| {
                *party_id == party.id
                    && party.id < snapshot.adventure.next_party_id
                    && !party.members.is_empty()
                    && party.members.len() <= 6
                    && party.members.iter().all(|member| member.0 > 0)
                    && party.members.contains(&party.leader)
                    && party.members.windows(2).all(|pair| pair[0] < pair[1])
                    && party
                        .members
                        .iter()
                        .all(|member| party_members.insert(*member))
                    && party.invitations.len() <= 12
                    && party
                        .invitations
                        .keys()
                        .all(|invitee| invitee.0 > 0 && !party.members.contains(invitee))
            })
            && snapshot.adventure.shop_stock.len() == commodity_specs().len()
            && commodity_specs().iter().all(|(id, _, _, _)| {
                snapshot
                    .adventure
                    .shop_stock
                    .get(*id)
                    .is_some_and(|stock| *stock <= 100)
            })
            && snapshot.adventure.next_weather_tick > 0;
        if snapshot.snapshot_version != WORLD_RUNTIME_SNAPSHOT_VERSION
            || snapshot.revision == 0
            || snapshot.rng_state == 0
            || !item_ids_match
            || !npc_ids_match
            || !adventure_valid
            || snapshot.progression_flags.len() > 64
            || !snapshot
                .progression_flags
                .iter()
                .all(|flag| valid_definition_id(flag))
        {
            return Err(WorldError::InvalidRuntimeSnapshot);
        }

        // WorldPack remains authoritative for static NPC configuration. The
        // portable snapshot overlays only dynamic state, so adding an NPC or
        // tuning a definition does not get undone on restart.
        let mut npcs = self.npcs.clone();
        for (npc_id, saved) in &snapshot.npcs {
            let authored = self
                .npcs
                .get(npc_id)
                .cloned()
                .ok_or(WorldError::InvalidRuntimeSnapshot)?;
            let max_stat_value = i32::try_from(MAX_STAT_VALUE).expect("MAX_STAT_VALUE fits in i32");
            let valid = (1..=MAX_PROGRESSION_LEVEL).contains(&saved.level)
                && (1..=max_stat_value).contains(&saved.max_health)
                && (1..=max_stat_value).contains(&saved.strength)
                && (0..=max_stat_value).contains(&saved.defence)
                && saved.health >= 0
                && saved.health <= max_stat_value
                && saved.engaged_character.is_none_or(|id| id.0 > 0)
                && ((saved.health == 0) == saved.respawn_at_tick.is_some())
                && (saved.health > 0 || saved.engaged_character.is_none())
                && saved.resources.len() <= MAX_RESOURCE_POOLS
                && saved.resources.iter().all(|(id, pool)| {
                    valid_stat_id(id)
                        && pool.max > 0
                        && pool.max <= MAX_RESOURCE_VALUE
                        && pool.current <= pool.max
                })
                && saved.resource_regeneration.len() <= MAX_RESOURCE_POOLS
                && saved
                    .resource_regeneration
                    .iter()
                    .all(|(id, value)| valid_stat_id(id) && *value <= MAX_RESOURCE_VALUE)
                && saved.ability_ids.len() <= MAX_KNOWN_ABILITIES
                && saved.ability_ids.iter().all(|id| valid_definition_id(id))
                && saved.ability_cooldowns.len() <= MAX_KNOWN_ABILITIES
                && saved.ability_cooldowns.iter().all(|(id, ready_tick)| {
                    valid_definition_id(id)
                        && *ready_tick
                            <= snapshot
                                .logical_tick
                                .saturating_add(u64::from(MAX_COOLDOWN_TICKS))
                })
                && saved.resistances.len() <= MAX_RESISTANCES
                && saved
                    .resistances
                    .values()
                    .all(|value| *value <= MAX_RESISTANCE_PERCENT)
                && saved.statuses.len() <= MAX_ACTIVE_STATUSES
                && saved.statuses.iter().all(validate_active_status_shape);
            if !valid {
                return Err(WorldError::InvalidRuntimeSnapshot);
            }
            let legacy_additive_fields_missing = saved.resources.is_empty()
                && saved.resource_regeneration.is_empty()
                && saved.ability_ids.is_empty()
                && saved.ability_cooldowns.is_empty()
                && saved.resistances.is_empty()
                && saved.statuses.is_empty();
            let resources = authored
                .resources
                .iter()
                .map(|(id, configured)| {
                    let current = if legacy_additive_fields_missing {
                        configured.max
                    } else {
                        saved
                            .resources
                            .get(id)
                            .map(|pool| pool.current.min(configured.max))
                            .unwrap_or(configured.max)
                    };
                    (
                        id.clone(),
                        ResourcePool {
                            current,
                            max: configured.max,
                        },
                    )
                })
                .collect();
            let dynamic = npcs
                .get_mut(npc_id)
                .ok_or(WorldError::InvalidRuntimeSnapshot)?;
            dynamic.health = if saved.health == 0 {
                0
            } else {
                saved.health.min(authored.max_health)
            };
            dynamic.respawn_at_tick = saved.respawn_at_tick;
            dynamic.engaged_character = saved.engaged_character;
            dynamic.resources = resources;
            dynamic.ability_cooldowns = saved
                .ability_cooldowns
                .iter()
                .filter(|(id, _)| authored.ability_ids.contains(id))
                .map(|(id, tick)| (id.clone(), *tick))
                .collect();
            dynamic.statuses = saved
                .statuses
                .iter()
                .filter(|status| {
                    self.abilities.contains_key(&status.source_ability_id)
                        && status
                            .source_npc_id
                            .as_ref()
                            .is_none_or(|source| self.npcs.contains_key(source))
                        && validate_active_status(status, &self.abilities)
                })
                .cloned()
                .collect();
        }

        let mut item_locations = self.item_locations.clone();
        for (item_id, saved_location) in &snapshot.item_locations {
            let location = match saved_location {
                RuntimeItemLocation::Staged => SharedItemLocation::Staged,
                RuntimeItemLocation::Carried { character_id } => {
                    SharedItemLocation::Carried(*character_id)
                }
                RuntimeItemLocation::Room { room_id } => {
                    if !self.stable_rooms.contains_key(room_id) {
                        return Err(WorldError::InvalidRuntimeSnapshot);
                    }
                    SharedItemLocation::Room(room_id.clone())
                }
                RuntimeItemLocation::Equipped { character_id, slot } => {
                    if self
                        .item_gameplay
                        .get(item_id)
                        .is_none_or(|item| item.equipment_slot != Some(*slot))
                    {
                        return Err(WorldError::InvalidRuntimeSnapshot);
                    }
                    SharedItemLocation::Equipped(*character_id, *slot)
                }
                RuntimeItemLocation::HeldByNpc { npc_id } => {
                    if !npcs.contains_key(npc_id) {
                        return Err(WorldError::InvalidRuntimeSnapshot);
                    }
                    SharedItemLocation::HeldByNpc(npc_id.clone())
                }
                RuntimeItemLocation::Consumed => SharedItemLocation::Consumed,
            };
            item_locations.insert(item_id.clone(), location);
        }

        self.revision = snapshot.revision;
        self.logical_tick = snapshot.logical_tick;
        self.rng = DeterministicRng {
            state: snapshot.rng_state,
        };
        self.npcs = npcs;
        self.item_locations = item_locations;
        snapshot.adventure.next_weather_tick = snapshot
            .adventure
            .next_weather_tick
            .max(snapshot.logical_tick.saturating_add(1));
        self.adventure = snapshot.adventure;
        self.world_progression_flags = snapshot.progression_flags;
        self.last_view_bump = None;
        Ok(())
    }

    pub fn character_gameplay_snapshot(
        &self,
        session: SessionId,
    ) -> Result<CharacterGameplaySnapshot, WorldError> {
        let character = self.character(session)?;
        self.character_gameplay
            .get(&character.id)
            .cloned()
            .ok_or(WorldError::InvalidEntity)
    }

    pub fn authorized_runtime_context(
        &self,
        session: SessionId,
    ) -> Result<AuthorizedRuntimeContext, WorldError> {
        let character = self.character(session)?;
        let room_id = room_checked(&self.entities, character.location)?
            .stable_id
            .clone();
        let gameplay = self
            .character_gameplay
            .get(&character.id)
            .ok_or(WorldError::InvalidEntity)?;
        let actor_quest_states = gameplay
            .quests
            .iter()
            .map(|(id, progress)| {
                let status = match progress.status {
                    QuestStatus::Active => "active",
                    QuestStatus::ReadyToClaim => "ready_to_claim",
                    QuestStatus::Completed => "completed",
                };
                format!("{id}:{status}")
            })
            .collect();
        let actor_content_preferences = gameplay
            .script_flags
            .iter()
            .filter_map(|flag| flag.strip_prefix("preference.").map(str::to_owned))
            .collect();
        let actor_senses = perception_senses(gameplay)
            .into_iter()
            .map(|sense| match sense {
                Sense::Vision => "vision",
                Sense::Hearing => "hearing",
                Sense::Darkvision => "darkvision",
                Sense::Tremorsense => "tremorsense",
            })
            .map(str::to_owned)
            .collect();
        Ok(AuthorizedRuntimeContext {
            character_id: character.id,
            character_name: character.name.clone(),
            room_id,
            actor_level: gameplay.level,
            actor_flags: gameplay.script_flags.clone(),
            world_instance: "primary".into(),
            actor_quest_states,
            actor_content_preferences,
            actor_senses,
            logical_tick: self.logical_tick,
            weather: self.adventure.weather,
        })
    }

    /// Set a bounded, durable authoring flag for the current character. The
    /// flag is not emitted as a domain/view event, preventing secret condition
    /// state from becoming client-visible.
    pub fn set_authorized_script_flag(
        &mut self,
        session: SessionId,
        flag: &str,
    ) -> Result<bool, WorldError> {
        let flag = flag.trim();
        if !valid_script_flag(flag) {
            return Err(WorldError::InvalidScriptFlag);
        }
        let character_id = self.character(session)?.id;
        let gameplay = self
            .character_gameplay
            .get_mut(&character_id)
            .ok_or(WorldError::InvalidEntity)?;
        if gameplay.script_flags.contains(flag) {
            return Ok(false);
        }
        if gameplay.script_flags.len() >= 64 {
            return Err(WorldError::InvalidScriptFlag);
        }
        gameplay.script_flags.insert(flag.to_owned());
        self.mark_authorized_script_change(session)?;
        Ok(true)
    }

    pub fn apply_authorized_script_damage(
        &mut self,
        session: SessionId,
        amount: u32,
    ) -> Result<bool, WorldError> {
        if amount == 0 || amount > 10_000 {
            return Err(WorldError::InvalidScriptEffect);
        }
        let character_id = self.character(session)?.id;
        let tick = self.logical_tick;
        let changed = {
            let gameplay = self
                .character_gameplay
                .get_mut(&character_id)
                .ok_or(WorldError::InvalidEntity)?;
            let next = gameplay
                .vitals
                .health
                .saturating_sub(i32::try_from(amount).unwrap_or(i32::MAX))
                .max(0);
            if next == gameplay.vitals.health {
                false
            } else {
                gameplay.vitals.health = next;
                if next == 0 {
                    gameplay.recovery_at_tick = Some(tick.saturating_add(3));
                }
                true
            }
        };
        if changed {
            self.mark_authorized_script_change(session)?;
        }
        Ok(changed)
    }

    pub fn apply_authorized_script_heal(
        &mut self,
        session: SessionId,
        amount: u32,
    ) -> Result<bool, WorldError> {
        if amount == 0 || amount > 10_000 {
            return Err(WorldError::InvalidScriptEffect);
        }
        let character_id = self.character(session)?.id;
        let changed = {
            let gameplay = self
                .character_gameplay
                .get_mut(&character_id)
                .ok_or(WorldError::InvalidEntity)?;
            let next = gameplay
                .vitals
                .health
                .saturating_add(i32::try_from(amount).unwrap_or(i32::MAX))
                .min(gameplay.vitals.max_health);
            if next == gameplay.vitals.health {
                false
            } else {
                gameplay.vitals.health = next;
                if next > 0 {
                    gameplay.recovery_at_tick = None;
                }
                true
            }
        };
        if changed {
            self.mark_authorized_script_change(session)?;
        }
        Ok(changed)
    }

    pub fn give_authorized_script_item(
        &mut self,
        session: SessionId,
        item_id: &str,
    ) -> Result<Option<DomainEvent>, WorldError> {
        let character = self.character(session)?;
        let actor_id = character.id;
        let actor_name = character.name.clone();
        let room_id = room_checked(&self.entities, character.location)?
            .stable_id
            .clone();
        let item = self
            .stable_items
            .get(item_id)
            .ok_or_else(|| WorldError::ItemNotFound(item_id.to_owned()))?;
        let item_name = item.name.clone();
        match self.item_locations.get(item_id) {
            Some(SharedItemLocation::Carried(owner)) if *owner == actor_id => return Ok(None),
            Some(SharedItemLocation::Room(owner_room)) if owner_room == &room_id => {}
            _ => return Err(WorldError::GameplayUnavailable),
        }
        self.item_locations
            .insert(item_id.to_owned(), SharedItemLocation::Carried(actor_id));
        self.revision = self.revision.wrapping_add(1).max(1);
        let event = DomainEvent::ItemTaken {
            actor_id,
            actor_name,
            item_id: item_id.to_owned(),
            item_name,
            room_id,
        };
        self.bump_view_revisions(&event)?;
        Ok(Some(event))
    }

    /// Schedule a durable authoring flag. The generic Lua event encodes this
    /// as `<delay-ticks>:<flag>`; due flags are applied before the timer hook on
    /// the corresponding deterministic world tick.
    pub fn schedule_authorized_script_flag(
        &mut self,
        session: SessionId,
        delay_ticks: u64,
        flag: &str,
    ) -> Result<bool, WorldError> {
        let flag = flag.trim();
        if delay_ticks == 0 || delay_ticks > 86_400 || !valid_script_flag(flag) {
            return Err(WorldError::InvalidScriptEffect);
        }
        let character_id = self.character(session)?.id;
        let due_tick = self.logical_tick.saturating_add(delay_ticks);
        let changed = {
            let gameplay = self
                .character_gameplay
                .get_mut(&character_id)
                .ok_or(WorldError::InvalidEntity)?;
            if gameplay.script_flags.contains(flag)
                || gameplay
                    .scheduled_script_flags
                    .iter()
                    .any(|scheduled| scheduled.flag == flag)
            {
                false
            } else {
                if gameplay.script_flags.len() + gameplay.scheduled_script_flags.len() >= 64
                    || gameplay.scheduled_script_flags.len() >= 32
                {
                    return Err(WorldError::InvalidScriptEffect);
                }
                gameplay.scheduled_script_flags.push(ScheduledScriptFlag {
                    flag: flag.to_owned(),
                    due_tick,
                });
                gameplay.scheduled_script_flags.sort_by(|left, right| {
                    (left.due_tick, &left.flag).cmp(&(right.due_tick, &right.flag))
                });
                true
            }
        };
        if changed {
            self.mark_authorized_script_change(session)?;
        }
        Ok(changed)
    }

    fn mark_authorized_script_change(&mut self, session: SessionId) -> Result<(), WorldError> {
        self.revision = self.revision.wrapping_add(1).max(1);
        let observer = self
            .observer_views
            .get_mut(&session)
            .ok_or(WorldError::InvalidEntity)?;
        let previous = observer.revision;
        observer.revision = observer.revision.wrapping_add(1).max(1);
        self.last_view_bump = Some((self.revision, vec![(session, previous)]));
        Ok(())
    }

    pub fn durable_character_snapshot(
        &self,
        session: SessionId,
    ) -> Result<DurableCharacterSnapshot, WorldError> {
        Ok(DurableCharacterSnapshot {
            character: self.character_snapshot(session)?,
            gameplay: self.character_gameplay_snapshot(session)?,
        })
    }

    pub fn restore_character_gameplay(
        &mut self,
        character_id: CharacterId,
        snapshot: CharacterGameplaySnapshot,
    ) -> Result<(), WorldError> {
        // Saves are native v3 snapshots. Unknown content references fail
        // closed instead of being guessed or replayed additively.
        if !snapshot
            .ability_ids
            .iter()
            .all(|id| valid_definition_id(id))
            || !snapshot
                .ability_cooldowns
                .keys()
                .all(|id| valid_definition_id(id))
            || !snapshot.statuses.iter().all(validate_active_status_shape)
        {
            return Err(WorldError::InvalidGameplayDefinition);
        }
        let unique_equipment = snapshot
            .equipment
            .values()
            .collect::<std::collections::HashSet<_>>()
            .len()
            == snapshot.equipment.len();
        let unique_abilities = snapshot
            .ability_ids
            .iter()
            .collect::<std::collections::HashSet<_>>()
            .len()
            == snapshot.ability_ids.len();
        let valid = self
            .progression
            .validate_snapshot(&snapshot, &self.abilities)
            && snapshot.level > 0
            && snapshot.level <= MAX_PROGRESSION_LEVEL
            && snapshot.vitals.max_health > 0
            && snapshot.vitals.max_health <= i32::try_from(MAX_STAT_VALUE).unwrap_or(i32::MAX)
            && (0..=snapshot.vitals.max_health).contains(&snapshot.vitals.health)
            && snapshot.attributes.strength >= 0
            && snapshot.attributes.strength <= i32::try_from(MAX_STAT_VALUE).unwrap_or(i32::MAX)
            && snapshot.attributes.defence >= 0
            && snapshot.attributes.defence <= i32::try_from(MAX_STAT_VALUE).unwrap_or(i32::MAX)
            && snapshot.attributes.additional.len() + 2 <= MAX_STATS
            && snapshot.attributes.additional.iter().all(|(id, value)| {
                valid_stat_id(id)
                    && !matches!(id.as_str(), "strength" | "defence" | "max_health")
                    && (0..=i32::try_from(MAX_STAT_VALUE).unwrap_or(i32::MAX)).contains(value)
            })
            && snapshot.resources.len() <= MAX_RESOURCE_POOLS
            && snapshot.resources.iter().all(|(id, pool)| {
                valid_stat_id(id)
                    && pool.max > 0
                    && pool.max <= MAX_RESOURCE_VALUE
                    && pool.current <= pool.max
            })
            && snapshot.resource_regeneration.len() <= snapshot.resources.len()
            && snapshot.resource_regeneration.iter().all(|(id, amount)| {
                snapshot.resources.contains_key(id) && *amount <= MAX_RESOURCE_VALUE
            })
            && snapshot.ability_ids.len() <= MAX_KNOWN_ABILITIES
            && unique_abilities
            && snapshot
                .ability_ids
                .iter()
                .all(|id| self.abilities.contains_key(id))
            && snapshot.ability_cooldowns.len() <= MAX_KNOWN_ABILITIES
            && snapshot.ability_cooldowns.iter().all(|(id, ready_tick)| {
                snapshot.ability_ids.contains(id)
                    && *ready_tick
                        <= self
                            .logical_tick
                            .saturating_add(u64::from(MAX_COOLDOWN_TICKS))
            })
            && snapshot.ability_global_ready_tick <= self.logical_tick.saturating_add(1)
            && snapshot
                .last_offensive_ability_tick
                .is_none_or(|tick| tick <= self.logical_tick)
            && snapshot.resistances.len() <= MAX_RESISTANCES
            && snapshot
                .resistances
                .values()
                .all(|value| *value <= MAX_RESISTANCE_PERCENT)
            && snapshot.statuses.len() <= MAX_ACTIVE_STATUSES
            && snapshot.statuses.iter().all(|status| {
                validate_active_status(status, &self.abilities)
                    && status
                        .source_npc_id
                        .as_ref()
                        .is_none_or(|npc_id| self.npcs.contains_key(npc_id))
            })
            && ((snapshot.vitals.health == 0) == snapshot.recovery_at_tick.is_some())
            && snapshot.skills.len() <= MAX_STATS
            && snapshot
                .skills
                .iter()
                .all(|(id, rank)| valid_stat_id(id) && *rank <= MAX_SKILL_VALUE)
            && unique_equipment
            && snapshot.equipment.iter().all(|(slot, item_id)| {
                self.stable_items.contains_key(item_id)
                    && self
                        .item_gameplay
                        .get(item_id)
                        .is_some_and(|item| item.equipment_slot == Some(*slot))
                    && (matches!(
                        self.item_locations.get(item_id),
                        Some(SharedItemLocation::Carried(owner)) if *owner == character_id
                    ) || matches!(
                        self.item_locations.get(item_id),
                        Some(SharedItemLocation::Equipped(owner, equipped_slot))
                            if *owner == character_id && *equipped_slot == *slot
                    ))
            })
            && snapshot.effects.iter().all(|effect| {
                (1..=MAX_STATUS_TICKS).contains(&effect.remaining_ticks)
                    && (1..=MAX_STATUS_MAGNITUDE).contains(&effect.magnitude)
            })
            && snapshot.effects.len() <= MAX_ACTIVE_EFFECTS;
        let adventure_valid = snapshot.commodities.len() <= commodity_specs().len()
            && snapshot.commodities.iter().all(|(id, amount)| {
                commodity_specs().iter().any(|(known, _, _, _)| known == id) && *amount <= 99
            })
            && snapshot.quests.len() <= 32
            && snapshot.quests.iter().all(|(id, progress)| {
                id == "first-hunt"
                    && progress.target == 1
                    && progress.current <= progress.target
                    && match progress.status {
                        QuestStatus::Active => progress.current < progress.target,
                        QuestStatus::ReadyToClaim | QuestStatus::Completed => {
                            progress.current == progress.target
                        }
                    }
            })
            && snapshot.script_flags.len() <= 64
            && snapshot
                .script_flags
                .iter()
                .all(|flag| valid_script_flag(flag))
            && snapshot.scheduled_script_flags.len() <= 32
            && snapshot
                .scheduled_script_flags
                .iter()
                .all(|scheduled| scheduled.due_tick > 0 && valid_script_flag(&scheduled.flag))
            && snapshot.script_flags.len() + snapshot.scheduled_script_flags.len() <= 64;
        if !valid || !adventure_valid {
            return Err(WorldError::InvalidGameplayDefinition);
        }
        if let Some(previous) = self.character_gameplay.get(&character_id) {
            for previous_item in previous.equipment.values() {
                if !snapshot
                    .equipment
                    .values()
                    .any(|item| item == previous_item)
                    && matches!(
                        self.item_locations.get(previous_item),
                        Some(SharedItemLocation::Equipped(owner, _)) if *owner == character_id
                    )
                {
                    self.item_locations.insert(
                        previous_item.clone(),
                        SharedItemLocation::Carried(character_id),
                    );
                }
            }
        }
        for (slot, item_id) in &snapshot.equipment {
            self.item_locations.insert(
                item_id.clone(),
                SharedItemLocation::Equipped(character_id, *slot),
            );
        }
        self.character_gameplay.insert(character_id, snapshot);
        Ok(())
    }

    pub fn configure_item_gameplay(
        &mut self,
        definition: ItemGameplayDefinition,
    ) -> Result<(), WorldError> {
        let max_stat_value = i32::try_from(MAX_STAT_VALUE).expect("MAX_STAT_VALUE fits in i32");
        let valid = self.stable_items.contains_key(&definition.item_id)
            && (0..=max_stat_value).contains(&definition.attack_bonus)
            && (0..=max_stat_value).contains(&definition.heal_amount)
            && definition.effect.as_ref().is_none_or(|effect| {
                (1..=MAX_STATUS_MAGNITUDE).contains(&effect.magnitude)
                    && (1..=MAX_STATUS_TICKS).contains(&effect.remaining_ticks)
            });
        if !valid {
            return Err(WorldError::InvalidGameplayDefinition);
        }
        self.item_gameplay
            .insert(definition.item_id.clone(), definition);
        Ok(())
    }

    /// Sets the ambient light used by the perception projection. Omitted rooms
    /// are bright by default so existing packs retain their behavior.
    pub fn configure_room_light(
        &mut self,
        room_id: &str,
        light: LightLevel,
    ) -> Result<(), WorldError> {
        if !self.stable_rooms.contains_key(room_id) {
            return Err(WorldError::InvalidGameplayDefinition);
        }
        self.room_light.insert(room_id.to_owned(), light);
        Ok(())
    }

    pub fn configure_hidden_item(
        &mut self,
        item_id: &str,
        difficulty: u16,
    ) -> Result<(), WorldError> {
        if !self.stable_items.contains_key(item_id) || !(1..=100).contains(&difficulty) {
            return Err(WorldError::InvalidGameplayDefinition);
        }
        self.hidden_items.insert(item_id.to_owned(), difficulty);
        Ok(())
    }

    pub fn configure_hidden_npc(
        &mut self,
        npc_id: &str,
        difficulty: u16,
    ) -> Result<(), WorldError> {
        if !self.npcs.contains_key(npc_id) || !(1..=100).contains(&difficulty) {
            return Err(WorldError::InvalidGameplayDefinition);
        }
        self.hidden_npcs.insert(npc_id.to_owned(), difficulty);
        Ok(())
    }

    pub fn configure_secret_exit(
        &mut self,
        room_id: &str,
        direction: Direction,
        difficulty: u16,
    ) -> Result<(), WorldError> {
        let room = self
            .stable_rooms
            .get(room_id)
            .copied()
            .and_then(|id| self.entities.get(id));
        let valid_exit =
            matches!(room, Some(Entity::Room(room)) if room.exits.contains_key(&direction));
        if !valid_exit || !(1..=100).contains(&difficulty) {
            return Err(WorldError::InvalidGameplayDefinition);
        }
        self.secret_exits
            .insert((room_id.to_owned(), direction), difficulty);
        Ok(())
    }

    pub fn spawn_npc(&mut self, definition: NpcDefinition) -> Result<DomainEvent, WorldError> {
        let loot_unique = definition
            .loot_item_ids
            .iter()
            .collect::<std::collections::HashSet<_>>()
            .len()
            == definition.loot_item_ids.len();
        let max_stat_value = i32::try_from(MAX_STAT_VALUE).expect("MAX_STAT_VALUE fits in i32");
        let valid = !definition.stable_id.trim().is_empty()
            && !definition.name.trim().is_empty()
            && !self.npcs.contains_key(&definition.stable_id)
            && self.stable_rooms.contains_key(&definition.room_id)
            && (1..=MAX_PROGRESSION_LEVEL).contains(&definition.level)
            && (1..=max_stat_value).contains(&definition.max_health)
            && (1..=max_stat_value).contains(&definition.strength)
            && (0..=max_stat_value).contains(&definition.defence)
            && definition.respawn_ticks > 0
            && loot_unique
            && definition.loot_item_ids.iter().all(|item_id| {
                self.stable_items.contains_key(item_id)
                    && matches!(
                        self.item_locations.get(item_id),
                        Some(SharedItemLocation::Staged)
                    )
            });
        if !valid {
            return Err(WorldError::InvalidGameplayDefinition);
        }
        for item_id in &definition.loot_item_ids {
            self.item_locations.insert(
                item_id.clone(),
                SharedItemLocation::HeldByNpc(definition.stable_id.clone()),
            );
        }
        let event = DomainEvent::Gameplay {
            event: GameplayEvent::NpcSpawned {
                npc_id: definition.stable_id.clone(),
                npc_name: definition.name.clone(),
                room_id: definition.room_id.clone(),
            },
        };
        self.npcs.insert(
            definition.stable_id.clone(),
            Npc {
                stable_id: definition.stable_id,
                name: definition.name,
                description: definition.description,
                spawn_room_id: definition.room_id,
                level: definition.level,
                health: definition.max_health,
                max_health: definition.max_health,
                strength: definition.strength,
                defence: definition.defence,
                experience_reward: definition.experience_reward,
                loot_item_ids: definition.loot_item_ids,
                respawn_ticks: definition.respawn_ticks,
                respawn_at_tick: None,
                engaged_character: None,
                resources: BTreeMap::new(),
                resource_regeneration: BTreeMap::new(),
                ability_ids: Vec::new(),
                ability_cooldowns: BTreeMap::new(),
                resistances: BTreeMap::new(),
                statuses: Vec::new(),
            },
        );
        self.revision = self.revision.wrapping_add(1).max(1);
        self.bump_view_revisions(&event)?;
        Ok(event)
    }

    /// Advances deterministic world time by one turn and resolves effects,
    /// recovery, respawns and every active combat in stable NPC-ID order.
    pub fn advance_tick(&mut self) -> Result<Vec<DomainEvent>, WorldError> {
        let checkpoint = self.checkpoint();
        match self.advance_tick_inner() {
            Ok(events) => Ok(events),
            Err(error) => {
                self.restore_checkpoint(checkpoint);
                Err(error)
            }
        }
    }

    fn advance_tick_inner(&mut self) -> Result<Vec<DomainEvent>, WorldError> {
        self.logical_tick = self.logical_tick.wrapping_add(1);
        let tick = self.logical_tick;
        let mut events = vec![DomainEvent::Gameplay {
            event: GameplayEvent::TickAdvanced { tick },
        }];
        self.resolve_character_tick(tick, &mut events)?;
        self.resolve_npc_statuses(tick, &mut events)?;
        self.resolve_npc_respawns(tick, &mut events);

        let combatants = self
            .npcs
            .iter()
            .filter_map(|(npc_id, npc)| npc.engaged_character.map(|owner| (npc_id.clone(), owner)))
            .collect::<Vec<_>>();
        for (npc_id, character_id) in combatants {
            self.resolve_combat_turn(tick, &npc_id, character_id, &mut events)?;
        }
        // Periodic effects are resolved at the start of the tick, but statuses
        // that reach zero must remain visible through this tick's combat. This
        // makes an authored duration of N affect exactly N logical turns while
        // leaving statuses applied during combat untouched until the next tick.
        self.expire_processed_effects();
        let defeated = events
            .iter()
            .filter_map(|event| match event {
                DomainEvent::Gameplay {
                    event: GameplayEvent::NpcDefeated { character_id, .. },
                } => Some(*character_id),
                _ => None,
            })
            .collect::<Vec<_>>();
        for character_id in defeated {
            // TODO(progression-signals): replace this legacy kill-side skill
            // increment once WorldPack can author event-triggered GrantSets.
            // Removing it before that hook exists would silently change the
            // existing combat/quest loop rather than make it data-driven.
            if let Some(rank) = self
                .character_gameplay
                .get_mut(&character_id)
                .and_then(|state| state.skills.get_mut("combat"))
            {
                *rank = rank.saturating_add(1).min(MAX_SKILL_VALUE);
            }
            if let Some(progress) = self
                .character_gameplay
                .get_mut(&character_id)
                .and_then(|state| state.quests.get_mut("first-hunt"))
                && progress.status == QuestStatus::Active
            {
                progress.current = progress.current.saturating_add(1).min(progress.target);
                if progress.current >= progress.target {
                    progress.status = QuestStatus::ReadyToClaim;
                }
                events.push(DomainEvent::Adventure {
                    event: AdventureEvent::QuestChanged {
                        character_id,
                        quest_id: "first-hunt".into(),
                        summary: "First Hunt is ready to claim.".into(),
                    },
                });
            }
        }
        self.adventure
            .parties
            .values_mut()
            .for_each(|party| party.invitations.retain(|_, expiry| *expiry >= tick));
        if tick >= self.adventure.next_weather_tick {
            let choices = [
                WeatherKind::Clear,
                WeatherKind::Rain,
                WeatherKind::Wind,
                WeatherKind::Fog,
            ];
            let previous = self.adventure.weather;
            let offset = (self.rng.next_u64() % 3 + 1) as usize;
            let previous_index = choices
                .iter()
                .position(|weather| *weather == previous)
                .unwrap_or(0);
            self.adventure.weather = choices[(previous_index + offset) % choices.len()];
            self.adventure.next_weather_tick = tick.saturating_add(5);
            events.push(DomainEvent::Adventure {
                event: AdventureEvent::WeatherChanged {
                    weather: self.adventure.weather,
                    tick,
                },
            });
        }
        self.revision = self.revision.wrapping_add(1).max(1);
        self.bump_view_revisions_for_events(&events)?;
        Ok(events)
    }

    fn resolve_character_tick(
        &mut self,
        tick: u64,
        events: &mut Vec<DomainEvent>,
    ) -> Result<(), WorldError> {
        let connected = self.connected_sessions();
        for session in connected {
            let character = self.character(session)?;
            let character_id = character.id;
            let character_name = character.name.clone();
            let current_room_id = room_checked(&self.entities, character.location)?
                .stable_id
                .clone();
            let resource_regeneration = self
                .character_gameplay
                .get(&character_id)
                .map(|state| state.resource_regeneration.clone())
                .unwrap_or_default();
            let state = self
                .character_gameplay
                .get_mut(&character_id)
                .ok_or(WorldError::InvalidEntity)?;
            let due_flags = state
                .scheduled_script_flags
                .iter()
                .filter(|scheduled| scheduled.due_tick <= tick)
                .map(|scheduled| scheduled.flag.clone())
                .collect::<Vec<_>>();
            state
                .scheduled_script_flags
                .retain(|scheduled| scheduled.due_tick > tick);
            for flag in due_flags {
                if state.script_flags.len() < 64 {
                    state.script_flags.insert(flag);
                }
            }
            for (resource_id, amount) in &resource_regeneration {
                if let Some(pool) = state.resources.get_mut(resource_id) {
                    pool.current = pool.current.saturating_add(*amount).min(pool.max);
                }
            }
            state
                .ability_cooldowns
                .retain(|_, ready_tick| *ready_tick > tick);
            let mut recovered_at_start = false;
            if state
                .recovery_at_tick
                .is_some_and(|recovery| recovery <= tick)
            {
                state.recovery_at_tick = None;
                state.vitals.health = state.vitals.max_health;
                let character_entity = *self
                    .sessions
                    .get(&session)
                    .ok_or(WorldError::UnknownSession)?;
                match self.entities.get_mut(character_entity) {
                    Some(Entity::Character(character)) => character.location = self.start_room,
                    _ => return Err(WorldError::InvalidEntity),
                }
                recovered_at_start = true;
                events.push(DomainEvent::Gameplay {
                    event: GameplayEvent::CharacterRecovered {
                        character_id,
                        character_name: character_name.clone(),
                        room_id: self.start_room_stable_id.clone(),
                    },
                });
            }
            let room_id = if recovered_at_start {
                self.start_room_stable_id.clone()
            } else {
                current_room_id
            };
            let mut retained = Vec::new();
            for mut effect in std::mem::take(&mut state.effects) {
                if effect.kind == EffectKind::Regeneration && state.vitals.health > 0 {
                    let previous = state.vitals.health;
                    state.vitals.health = state
                        .vitals
                        .health
                        .saturating_add(effect.magnitude.max(0))
                        .min(state.vitals.max_health);
                    let amount = state.vitals.health.saturating_sub(previous);
                    if amount > 0 {
                        events.push(DomainEvent::Gameplay {
                            event: GameplayEvent::CharacterHealed {
                                character_id,
                                character_name: character_name.clone(),
                                amount,
                                health: state.vitals.health,
                                room_id: room_id.clone(),
                            },
                        });
                    }
                }
                effect.remaining_ticks -= 1;
                if effect.remaining_ticks == 0 {
                    events.push(DomainEvent::Gameplay {
                        event: GameplayEvent::EffectExpired {
                            character_id,
                            character_name: character_name.clone(),
                            kind: effect.kind,
                            room_id: room_id.clone(),
                        },
                    });
                }
                retained.push(effect);
            }
            state.effects = retained;
            let mut retained_statuses = Vec::new();
            let mut defeating_npc_id = None;
            for mut status in std::mem::take(&mut state.statuses) {
                if state.vitals.health == 0 {
                    break;
                }
                if state.vitals.health > 0 {
                    match status.kind {
                        ActiveStatusKind::Regeneration => {
                            let previous = state.vitals.health;
                            state.vitals.health = state
                                .vitals
                                .health
                                .saturating_add(status.magnitude.max(0))
                                .min(state.vitals.max_health);
                            let amount = state.vitals.health.saturating_sub(previous);
                            if amount > 0 {
                                events.push(DomainEvent::Gameplay {
                                    event: GameplayEvent::CharacterHealed {
                                        character_id,
                                        character_name: character_name.clone(),
                                        amount,
                                        health: state.vitals.health,
                                        room_id: room_id.clone(),
                                    },
                                });
                            }
                        }
                        ActiveStatusKind::DamageOverTime => {
                            let resistance = status
                                .damage_type
                                .and_then(|kind| state.resistances.get(&kind).copied())
                                .unwrap_or(0);
                            let amount = resisted_amount(status.magnitude.max(0), resistance);
                            state.vitals.health = state.vitals.health.saturating_sub(amount).max(0);
                            if state.vitals.health == 0 {
                                defeating_npc_id = status.source_npc_id.clone();
                            }
                        }
                        ActiveStatusKind::AttributeModifier | ActiveStatusKind::Stun => {}
                    }
                }
                status.remaining_ticks -= 1;
                if state.vitals.health > 0 {
                    retained_statuses.push(status);
                }
            }
            state.statuses = retained_statuses;
            let collapsed = state.vitals.health == 0 && state.recovery_at_tick.is_none();
            let recover_at_tick = tick.saturating_add(3);
            if collapsed {
                state.recovery_at_tick = Some(recover_at_tick);
                state.statuses.clear();
            }
            if collapsed {
                for npc in self.npcs.values_mut() {
                    if npc.engaged_character == Some(character_id) {
                        npc.engaged_character = None;
                    }
                }
                if let Some(npc_id) = defeating_npc_id
                    && let Some(npc) = self.npcs.get(&npc_id)
                {
                    events.push(DomainEvent::Gameplay {
                        event: GameplayEvent::CharacterDefeated {
                            character_id,
                            character_name: character_name.clone(),
                            npc_id,
                            npc_name: npc.name.clone(),
                            recover_at_tick,
                            room_id,
                        },
                    });
                } else {
                    events.push(DomainEvent::Gameplay {
                        event: GameplayEvent::CharacterCollapsed {
                            character_id,
                            character_name: character_name.clone(),
                            recover_at_tick,
                            room_id,
                        },
                    });
                }
            }
        }
        Ok(())
    }

    fn resolve_npc_respawns(&mut self, tick: u64, events: &mut Vec<DomainEvent>) {
        for npc in self.npcs.values_mut() {
            if npc.respawn_at_tick.is_some_and(|respawn| respawn <= tick) {
                npc.respawn_at_tick = None;
                npc.health = npc.max_health;
                npc.engaged_character = None;
                npc.statuses.clear();
                npc.ability_cooldowns.clear();
                for pool in npc.resources.values_mut() {
                    pool.current = pool.max;
                }
                events.push(DomainEvent::Gameplay {
                    event: GameplayEvent::NpcRespawned {
                        npc_id: npc.stable_id.clone(),
                        npc_name: npc.name.clone(),
                        room_id: npc.spawn_room_id.clone(),
                    },
                });
            }
        }
    }

    fn resolve_npc_statuses(
        &mut self,
        tick: u64,
        events: &mut Vec<DomainEvent>,
    ) -> Result<(), WorldError> {
        let mut defeated = Vec::new();
        for (npc_id, npc) in &mut self.npcs {
            if !npc.alive() {
                continue;
            }
            for (resource_id, amount) in &npc.resource_regeneration {
                if let Some(pool) = npc.resources.get_mut(resource_id) {
                    pool.current = pool.current.saturating_add(*amount).min(pool.max);
                }
            }
            npc.ability_cooldowns
                .retain(|_, ready_tick| *ready_tick > tick);
            let mut retained = Vec::new();
            let mut defeating_source = None;
            for mut status in std::mem::take(&mut npc.statuses) {
                if npc.health == 0 {
                    break;
                }
                match status.kind {
                    ActiveStatusKind::Regeneration => {
                        npc.health = npc
                            .health
                            .saturating_add(status.magnitude.max(0))
                            .min(npc.max_health);
                    }
                    ActiveStatusKind::DamageOverTime => {
                        let resistance = status
                            .damage_type
                            .and_then(|kind| npc.resistances.get(&kind).copied())
                            .unwrap_or(0);
                        let amount = resisted_amount(status.magnitude.max(0), resistance);
                        npc.health = npc.health.saturating_sub(amount).max(0);
                        if npc.health == 0 {
                            defeating_source = status.source_character_id;
                        }
                    }
                    ActiveStatusKind::AttributeModifier | ActiveStatusKind::Stun => {}
                }
                status.remaining_ticks -= 1;
                if npc.health > 0 {
                    retained.push(status);
                }
            }
            npc.statuses = retained;
            if npc.health == 0 {
                defeated.push((npc_id.clone(), defeating_source));
            }
        }

        for (npc_id, source) in defeated {
            if let Some(character_id) = source
                && self.character_gameplay.contains_key(&character_id)
            {
                let character_name = if let Some(session) = self.session_for_character(character_id)
                {
                    self.character(session)?.name.clone()
                } else {
                    "A departed adventurer".to_owned()
                };
                let room_id = self
                    .npcs
                    .get(&npc_id)
                    .ok_or(WorldError::NpcNotFound)?
                    .spawn_room_id
                    .clone();
                self.resolve_npc_defeat(
                    tick,
                    &npc_id,
                    character_id,
                    &character_name,
                    &room_id,
                    events,
                )?;
            } else if self.npcs.contains_key(&npc_id) {
                let (npc_name, room_id, loot) = {
                    let npc = self.npcs.get_mut(&npc_id).ok_or(WorldError::NpcNotFound)?;
                    npc.engaged_character = None;
                    npc.statuses.clear();
                    npc.ability_cooldowns.clear();
                    npc.respawn_at_tick = Some(tick.saturating_add(npc.respawn_ticks));
                    (
                        npc.name.clone(),
                        npc.spawn_room_id.clone(),
                        npc.loot_item_ids.clone(),
                    )
                };
                for item_id in loot {
                    if matches!(
                        self.item_locations.get(&item_id),
                        Some(SharedItemLocation::HeldByNpc(owner)) if owner == &npc_id
                    ) {
                        self.item_locations
                            .insert(item_id.clone(), SharedItemLocation::Room(room_id.clone()));
                        let item_name = self
                            .stable_items
                            .get(&item_id)
                            .ok_or(WorldError::InvalidEntity)?
                            .name
                            .clone();
                        events.push(DomainEvent::Gameplay {
                            event: GameplayEvent::LootDropped {
                                npc_id: npc_id.clone(),
                                npc_name: npc_name.clone(),
                                item_id,
                                item_name,
                                room_id: room_id.clone(),
                            },
                        });
                    }
                }
            }
        }
        Ok(())
    }

    fn expire_processed_effects(&mut self) {
        for state in self.character_gameplay.values_mut() {
            state.effects.retain(|effect| effect.remaining_ticks > 0);
            state.statuses.retain(|status| status.remaining_ticks > 0);
        }
        for npc in self.npcs.values_mut() {
            npc.statuses.retain(|status| status.remaining_ticks > 0);
        }
    }

    fn resolve_combat_turn(
        &mut self,
        tick: u64,
        npc_id: &str,
        character_id: CharacterId,
        events: &mut Vec<DomainEvent>,
    ) -> Result<(), WorldError> {
        let Some(session) = self.session_for_character(character_id) else {
            if let Some(npc) = self.npcs.get_mut(npc_id) {
                npc.engaged_character = None;
            }
            return Ok(());
        };
        let character = self.character(session)?;
        let character_name = character.name.clone();
        let room_id = room_checked(&self.entities, character.location)?
            .stable_id
            .clone();
        let state = self
            .character_gameplay
            .get(&character_id)
            .cloned()
            .ok_or(WorldError::InvalidEntity)?;
        let npc_snapshot = self
            .npcs
            .get(npc_id)
            .cloned()
            .ok_or(WorldError::NpcNotFound)?;
        if !npc_snapshot.alive()
            || npc_snapshot.spawn_room_id != room_id
            || state.vitals.health <= 0
        {
            if let Some(npc) = self.npcs.get_mut(npc_id) {
                npc.engaged_character = None;
            }
            return Ok(());
        }

        let replaces_normal_attack =
            state.last_offensive_ability_tick == Some(tick.saturating_sub(1));
        let player_stunned = state
            .statuses
            .iter()
            .any(|status| status.kind == ActiveStatusKind::Stun);
        if !replaces_normal_attack && !player_stunned {
            let equipment_bonus = state
                .equipment
                .values()
                .filter_map(|item_id| self.item_gameplay.get(item_id))
                .map(|item| item.attack_bonus)
                .fold(0_i32, i32::saturating_add);
            let effect_bonus = state
                .effects
                .iter()
                .filter(|effect| effect.kind == EffectKind::Strength)
                .map(|effect| effect.magnitude)
                .fold(0_i32, i32::saturating_add);
            let player_attack = effective_gameplay_attribute(&state, "strength")
                .saturating_add(i32::try_from(state.level).unwrap_or(i32::MAX))
                .saturating_add(equipment_bonus)
                .saturating_add(effect_bonus);
            let player_damage = resisted_amount(
                player_attack
                    .saturating_sub(effective_npc_attribute(&npc_snapshot, "defence"))
                    .saturating_add(self.rng.range_inclusive(0, 2))
                    .max(1),
                npc_snapshot
                    .resistances
                    .get(&DamageType::Physical)
                    .copied()
                    .unwrap_or(0),
            );
            let remaining_npc_health = {
                let npc = self.npcs.get_mut(npc_id).ok_or(WorldError::NpcNotFound)?;
                npc.health = npc.health.saturating_sub(player_damage).max(0);
                npc.health
            };
            events.push(DomainEvent::Gameplay {
                event: GameplayEvent::NpcDamaged {
                    character_id,
                    character_name: character_name.clone(),
                    npc_id: npc_id.to_owned(),
                    npc_name: npc_snapshot.name.clone(),
                    amount: player_damage,
                    remaining_health: remaining_npc_health,
                    room_id: room_id.clone(),
                },
            });
            if remaining_npc_health == 0 {
                self.resolve_npc_defeat(
                    tick,
                    npc_id,
                    character_id,
                    &character_name,
                    &room_id,
                    events,
                )?;
                return Ok(());
            }
        }

        let acting_npc = self
            .npcs
            .get(npc_id)
            .cloned()
            .ok_or(WorldError::NpcNotFound)?;
        if acting_npc
            .statuses
            .iter()
            .any(|status| status.kind == ActiveStatusKind::Stun)
        {
            return Ok(());
        }
        let npc_ability = acting_npc
            .ability_ids
            .iter()
            .filter_map(|id| self.abilities.get(id))
            .filter(|ability| {
                acting_npc
                    .ability_cooldowns
                    .get(&ability.id)
                    .is_none_or(|ready| *ready <= tick)
                    && ability.resource_costs.iter().all(|(id, cost)| {
                        acting_npc
                            .resources
                            .get(id)
                            .is_some_and(|pool| pool.current >= *cost)
                    })
                    && state.statuses.len() + npc_enemy_status_count(ability) <= MAX_ACTIVE_STATUSES
                    && acting_npc.statuses.len() + npc_self_status_count(ability)
                        <= MAX_ACTIVE_STATUSES
                    && npc_ability_score(ability, &acting_npc) > 0
            })
            .max_by(|left, right| {
                npc_ability_score(left, &acting_npc)
                    .cmp(&npc_ability_score(right, &acting_npc))
                    .then_with(|| right.id.cmp(&left.id))
            })
            .cloned();
        let mut player_damage = None;
        if let Some(ability) = &npc_ability {
            let npc = self.npcs.get_mut(npc_id).ok_or(WorldError::NpcNotFound)?;
            for (resource_id, cost) in &ability.resource_costs {
                npc.resources
                    .get_mut(resource_id)
                    .ok_or(WorldError::InsufficientResource)?
                    .current -= *cost;
            }
            if ability.cooldown_ticks > 0 {
                npc.ability_cooldowns.insert(
                    ability.id.clone(),
                    tick.saturating_add(u64::from(ability.cooldown_ticks)),
                );
            }
            events.push(DomainEvent::Gameplay {
                event: GameplayEvent::NpcAbilityUsed {
                    npc_id: npc_id.to_owned(),
                    npc_name: npc_snapshot.name.clone(),
                    character_id,
                    character_name: character_name.clone(),
                    ability_id: ability.id.clone(),
                    ability_name: ability.name.clone(),
                    room_id: room_id.clone(),
                },
            });

            if let Some(damage) = &ability.damage {
                let amount = ability_scaled_amount_npc(
                    damage.base_amount,
                    damage.scaling_attribute.as_deref(),
                    damage.scaling_percent,
                    &acting_npc,
                );
                if npc_effect_targets_enemy(ability, damage.target) {
                    player_damage = Some((amount, damage.damage_type));
                } else {
                    let resistance = self.npcs[npc_id]
                        .resistances
                        .get(&damage.damage_type)
                        .copied()
                        .unwrap_or(0);
                    let amount = resisted_amount(amount, resistance);
                    let npc = self.npcs.get_mut(npc_id).ok_or(WorldError::NpcNotFound)?;
                    npc.health = npc.health.saturating_sub(amount).max(0);
                }
            }
            if let Some(heal) = &ability.heal
                && !npc_effect_targets_enemy(ability, heal.target)
            {
                let amount = ability_scaled_amount_npc(
                    heal.base_amount,
                    heal.scaling_attribute.as_deref(),
                    heal.scaling_percent,
                    &acting_npc,
                );
                let npc = self.npcs.get_mut(npc_id).ok_or(WorldError::NpcNotFound)?;
                npc.health = npc.health.saturating_add(amount).min(npc.max_health);
            }
            if self.npcs.get(npc_id).is_some_and(|npc| npc.health > 0) {
                for status in &ability.status_effects {
                    if !npc_effect_targets_enemy(ability, status_target(status)) {
                        self.npcs
                            .get_mut(npc_id)
                            .ok_or(WorldError::NpcNotFound)?
                            .statuses
                            .push(active_status(&ability.id, None, Some(npc_id), status));
                    }
                }
            }
            if self.npcs.get(npc_id).is_some_and(|npc| npc.health == 0) {
                self.resolve_npc_defeat(
                    tick,
                    npc_id,
                    character_id,
                    &character_name,
                    &room_id,
                    events,
                )?;
                return Ok(());
            }
        } else {
            player_damage = Some((
                effective_npc_attribute(&npc_snapshot, "strength")
                    .saturating_add(i32::try_from(npc_snapshot.level).unwrap_or(i32::MAX))
                    .saturating_sub(effective_gameplay_attribute(&state, "defence"))
                    .saturating_add(self.rng.range_inclusive(0, 2))
                    .max(1),
                DamageType::Physical,
            ));
        }
        let npc_damage = player_damage.map(|(amount, damage_type)| {
            resisted_amount(
                amount,
                state.resistances.get(&damage_type).copied().unwrap_or(0),
            )
        });
        let player_state = self
            .character_gameplay
            .get_mut(&character_id)
            .ok_or(WorldError::InvalidEntity)?;
        if let Some(npc_damage) = npc_damage {
            player_state.vitals.health =
                player_state.vitals.health.saturating_sub(npc_damage).max(0);
        }
        if player_state.vitals.health > 0
            && let Some(ability) = &npc_ability
        {
            for status in &ability.status_effects {
                if npc_effect_targets_enemy(ability, status_target(status)) {
                    player_state.statuses.push(active_status(
                        &ability.id,
                        None,
                        Some(npc_id),
                        status,
                    ));
                }
            }
        }
        let remaining_health = player_state.vitals.health;
        if let Some(npc_damage) = npc_damage {
            events.push(DomainEvent::Gameplay {
                event: GameplayEvent::CharacterDamaged {
                    character_id,
                    character_name: character_name.clone(),
                    npc_id: npc_id.to_owned(),
                    npc_name: npc_snapshot.name.clone(),
                    amount: npc_damage,
                    remaining_health,
                    room_id: room_id.clone(),
                },
            });
        }
        if remaining_health == 0 {
            let recover_at_tick = tick.saturating_add(3);
            player_state.recovery_at_tick = Some(recover_at_tick);
            player_state.statuses.clear();
            if let Some(npc) = self.npcs.get_mut(npc_id) {
                npc.engaged_character = None;
            }
            events.push(DomainEvent::Gameplay {
                event: GameplayEvent::CharacterDefeated {
                    character_id,
                    character_name,
                    npc_id: npc_id.to_owned(),
                    npc_name: npc_snapshot.name,
                    recover_at_tick,
                    room_id,
                },
            });
        }
        Ok(())
    }

    fn resolve_npc_defeat(
        &mut self,
        tick: u64,
        npc_id: &str,
        character_id: CharacterId,
        character_name: &str,
        room_id: &str,
        events: &mut Vec<DomainEvent>,
    ) -> Result<(), WorldError> {
        let (npc_name, reward, respawn_at_tick, loot) = {
            let npc = self.npcs.get_mut(npc_id).ok_or(WorldError::NpcNotFound)?;
            npc.engaged_character = None;
            npc.statuses.clear();
            npc.ability_cooldowns.clear();
            let respawn_at_tick = tick.saturating_add(npc.respawn_ticks);
            npc.respawn_at_tick = Some(respawn_at_tick);
            (
                npc.name.clone(),
                npc.experience_reward,
                respawn_at_tick,
                npc.loot_item_ids.clone(),
            )
        };
        events.push(DomainEvent::Gameplay {
            event: GameplayEvent::NpcDefeated {
                character_id,
                character_name: character_name.to_owned(),
                npc_id: npc_id.to_owned(),
                npc_name: npc_name.clone(),
                respawn_at_tick,
                room_id: room_id.to_owned(),
            },
        });
        for item_id in loot {
            if self.item_locations.get(&item_id)
                == Some(&SharedItemLocation::HeldByNpc(npc_id.to_owned()))
            {
                self.item_locations.insert(
                    item_id.clone(),
                    SharedItemLocation::Room(room_id.to_owned()),
                );
                let item_name = self
                    .stable_items
                    .get(&item_id)
                    .ok_or(WorldError::InvalidEntity)?
                    .name
                    .clone();
                events.push(DomainEvent::Gameplay {
                    event: GameplayEvent::LootDropped {
                        npc_id: npc_id.to_owned(),
                        npc_name: npc_name.clone(),
                        item_id,
                        item_name,
                        room_id: room_id.to_owned(),
                    },
                });
            }
        }
        let total = {
            let state = self
                .character_gameplay
                .get_mut(&character_id)
                .ok_or(WorldError::InvalidEntity)?;
            state.experience = state.experience.saturating_add(reward);
            state.experience
        };
        events.push(DomainEvent::Gameplay {
            event: GameplayEvent::ExperienceGained {
                character_id,
                character_name: character_name.to_owned(),
                amount: reward,
                total,
                room_id: room_id.to_owned(),
            },
        });
        self.advance_character_progression(character_id, character_name, room_id, events)?;
        Ok(())
    }

    fn advance_character_progression(
        &mut self,
        character_id: CharacterId,
        character_name: &str,
        room_id: &str,
        events: &mut Vec<DomainEvent>,
    ) -> Result<(), WorldError> {
        let mut state = self
            .character_gameplay
            .get(&character_id)
            .cloned()
            .ok_or(WorldError::InvalidEntity)?;
        loop {
            let Some(threshold) = self
                .progression
                .next_level(state.level, state.experience)
                .cloned()
            else {
                break;
            };
            state = self
                .progression
                .apply_level(&state, &threshold, &self.abilities)?;
            events.push(DomainEvent::Gameplay {
                event: GameplayEvent::CharacterLeveled {
                    character_id,
                    character_name: character_name.to_owned(),
                    level: state.level,
                    room_id: room_id.to_owned(),
                },
            });
        }
        self.character_gameplay.insert(character_id, state);
        Ok(())
    }

    /// Replays one committed event while rebuilding shared world state.
    /// Character movement is intentionally ignored because durable character
    /// snapshots are the authority for locations.
    pub fn replay_persisted_event(&mut self, event: &DomainEvent) -> Result<(), WorldError> {
        match event {
            DomainEvent::CharacterMoved { .. }
            | DomainEvent::Gameplay { .. }
            | DomainEvent::Adventure { .. } => Ok(()),
            DomainEvent::ItemTaken {
                actor_id,
                actor_name,
                item_id,
                item_name,
                room_id,
            } => {
                self.validate_persisted_item_event(
                    *actor_id, actor_name, item_id, item_name, room_id,
                )?;
                if self.item_locations.get(item_id)
                    != Some(&SharedItemLocation::Room(room_id.clone()))
                {
                    return Err(WorldError::InvalidDomainTransition);
                }
                self.item_locations
                    .insert(item_id.clone(), SharedItemLocation::Carried(*actor_id));
                self.revision = self.revision.wrapping_add(1).max(1);
                Ok(())
            }
            DomainEvent::ItemDropped {
                actor_id,
                actor_name,
                item_id,
                item_name,
                room_id,
            } => {
                self.validate_persisted_item_event(
                    *actor_id, actor_name, item_id, item_name, room_id,
                )?;
                if self.item_locations.get(item_id) != Some(&SharedItemLocation::Carried(*actor_id))
                {
                    return Err(WorldError::InvalidDomainTransition);
                }
                self.item_locations
                    .insert(item_id.clone(), SharedItemLocation::Room(room_id.clone()));
                self.revision = self.revision.wrapping_add(1).max(1);
                Ok(())
            }
        }
    }

    /// Reverses uncommitted shared-item events. Validation runs against a copy
    /// first, so a malformed batch cannot partially mutate the live world.
    pub fn rollback_domain_events(&mut self, events: &[DomainEvent]) -> Result<(), WorldError> {
        let mut restored = self.item_locations.clone();
        let mut item_event_count = 0_u64;
        for event in events.iter().rev() {
            match event {
                DomainEvent::CharacterMoved { .. }
                | DomainEvent::Gameplay { .. }
                | DomainEvent::Adventure { .. } => {}
                DomainEvent::ItemTaken {
                    actor_id,
                    actor_name,
                    item_id,
                    item_name,
                    room_id,
                } => {
                    self.validate_persisted_item_event(
                        *actor_id, actor_name, item_id, item_name, room_id,
                    )?;
                    if restored.get(item_id) != Some(&SharedItemLocation::Carried(*actor_id)) {
                        return Err(WorldError::InvalidDomainTransition);
                    }
                    restored.insert(item_id.clone(), SharedItemLocation::Room(room_id.clone()));
                    item_event_count += 1;
                }
                DomainEvent::ItemDropped {
                    actor_id,
                    actor_name,
                    item_id,
                    item_name,
                    room_id,
                } => {
                    self.validate_persisted_item_event(
                        *actor_id, actor_name, item_id, item_name, room_id,
                    )?;
                    if restored.get(item_id) != Some(&SharedItemLocation::Room(room_id.clone())) {
                        return Err(WorldError::InvalidDomainTransition);
                    }
                    restored.insert(item_id.clone(), SharedItemLocation::Carried(*actor_id));
                    item_event_count += 1;
                }
            }
        }
        self.item_locations = restored;
        if item_event_count > 0 {
            self.restore_last_view_bump();
            self.revision = self.revision.saturating_sub(item_event_count).max(1);
        }
        Ok(())
    }

    pub fn connect(
        &mut self,
        session: SessionId,
        snapshot: CharacterSnapshot,
    ) -> Result<SceneView, WorldError> {
        if self.sessions.iter().any(|(connected_session, entity)| {
            *connected_session != session
                && matches!(
                    self.entities.get(*entity),
                    Some(Entity::Character(character)) if character.id == snapshot.id
                )
        }) {
            return Err(WorldError::CharacterAlreadyConnected);
        }
        let location = self
            .stable_rooms
            .get(&snapshot.room_id)
            .copied()
            .unwrap_or(self.start_room);
        room_checked(&self.entities, location)?;
        self.character_gameplay.entry(snapshot.id).or_default();
        let character = self.entities.insert(Entity::Character(Character {
            id: snapshot.id,
            account_id: snapshot.account_id,
            name: snapshot.name,
            location,
        }));
        if let Some(previous) = self.sessions.insert(session, character) {
            self.entities.remove(previous);
        }
        let salt = self.next_observer_salt;
        self.next_observer_salt = self.next_observer_salt.wrapping_add(1).max(1);
        self.observer_views.insert(
            session,
            ObserverView {
                salt,
                revision: 1,
                discovered_rooms: BTreeSet::from([snapshot.room_id]),
                role: AccountRole::Player,
            },
        );
        self.scene_for(session)
    }

    /// Assigns authorization metadata before any view leaves the server.
    /// Administrative observers bypass concealment but still receive opaque
    /// entity identifiers.
    pub fn set_observer_role(
        &mut self,
        session: SessionId,
        role: AccountRole,
    ) -> Result<(), WorldError> {
        let observer = self
            .observer_views
            .get_mut(&session)
            .ok_or(WorldError::UnknownSession)?;
        if observer.role != role {
            observer.role = role;
            observer.revision = observer.revision.wrapping_add(1).max(1);
        }
        Ok(())
    }

    pub fn disconnect(&mut self, session: SessionId) {
        self.observer_views.remove(&session);
        if let Some(character) = self.sessions.remove(&session) {
            let disconnected_character_id =
                self.entities
                    .get(character)
                    .and_then(|entity| match entity {
                        Entity::Character(character) => Some(character.id),
                        _ => None,
                    });
            if let Some(character_id) = disconnected_character_id {
                // Combat is connection-scoped in v1: an NPC immediately stops
                // attacking a departed character. Remove that character's
                // pending effects as well, so damage-over-time cannot award
                // progress only in memory after the durable character is no
                // longer part of the tick commit.
                for npc in self.npcs.values_mut() {
                    if npc.engaged_character == Some(character_id) {
                        npc.engaged_character = None;
                    }
                    npc.statuses
                        .retain(|status| status.source_character_id != Some(character_id));
                }
            }
            self.entities.remove(character);
        }
    }

    pub fn character_snapshot(&self, session: SessionId) -> Result<CharacterSnapshot, WorldError> {
        let character = self.character(session)?;
        let current_room = room_checked(&self.entities, character.location)?;
        Ok(CharacterSnapshot {
            id: character.id,
            account_id: character.account_id,
            name: character.name.clone(),
            room_id: current_room.stable_id.clone(),
            item_locations: BTreeMap::new(),
        })
    }

    pub fn restore_character(
        &mut self,
        session: SessionId,
        snapshot: &CharacterSnapshot,
        revision: u64,
    ) -> Result<(), WorldError> {
        let location = self
            .stable_rooms
            .get(&snapshot.room_id)
            .copied()
            .ok_or(WorldError::InvalidEntity)?;
        room_checked(&self.entities, location)?;
        let character_id = *self
            .sessions
            .get(&session)
            .ok_or(WorldError::UnknownSession)?;
        match self.entities.get_mut(character_id) {
            Some(Entity::Character(character))
                if character.id == snapshot.id && character.account_id == snapshot.account_id =>
            {
                character.name.clone_from(&snapshot.name);
                character.location = location;
                self.restore_last_view_bump();
                self.revision = revision;
                Ok(())
            }
            _ => Err(WorldError::InvalidEntity),
        }
    }

    pub fn apply(
        &mut self,
        session: SessionId,
        action: PlayerAction,
    ) -> Result<ActionOutcome, WorldError> {
        let gameplay = self
            .character_gameplay
            .get(&self.character(session)?.id)
            .ok_or(WorldError::InvalidEntity)?;
        let defeated = gameplay.recovery_at_tick.is_some();
        let stunned = gameplay
            .statuses
            .iter()
            .any(|status| status.kind == ActiveStatusKind::Stun);
        if defeated
            && !matches!(
                &action,
                PlayerAction::Look
                    | PlayerAction::Inventory
                    | PlayerAction::Examine(_)
                    | PlayerAction::Party(_)
                    | PlayerAction::Quest(_)
                    | PlayerAction::Economy(EconomyAction::Balance)
                    | PlayerAction::Weather
                    | PlayerAction::Time
            )
        {
            return Err(WorldError::GameplayUnavailable);
        }
        if stunned
            && !matches!(
                &action,
                PlayerAction::Look
                    | PlayerAction::Inventory
                    | PlayerAction::Examine(_)
                    | PlayerAction::Weather
                    | PlayerAction::Time
            )
        {
            return Err(WorldError::GameplayUnavailable);
        }
        match action {
            PlayerAction::Look => Ok(ActionOutcome {
                scene: self.scene_for(session)?,
                output: None,
                changed_character: None,
                domain_events: Vec::new(),
            }),
            PlayerAction::Inventory => Ok(ActionOutcome {
                scene: self.scene_for(session)?,
                output: Some(self.inventory_as_plain_text(session)?),
                changed_character: None,
                domain_events: Vec::new(),
            }),
            PlayerAction::Examine(query) => {
                let item_id =
                    self.resolve_accessible_item(session, &query, ItemScope::Accessible)?;
                let item = self
                    .stable_items
                    .get(&item_id)
                    .ok_or(WorldError::InvalidEntity)?;
                Ok(ActionOutcome {
                    scene: self.scene_for(session)?,
                    output: Some(format!("{}\r\n", item.description)),
                    changed_character: None,
                    domain_events: Vec::new(),
                })
            }
            PlayerAction::Take(query) => self.move_item(session, &query, true),
            PlayerAction::Drop(query) => self.move_item(session, &query, false),
            PlayerAction::Attack(query) => self.start_combat(session, &query),
            PlayerAction::Ability { ability_id, target } => {
                self.use_ability(session, &ability_id, target.as_deref())
            }
            PlayerAction::Promote { class_id } => self.promote_character(session, &class_id),
            PlayerAction::AcquireTalent { graph_id, node_id } => {
                self.acquire_talent(session, &graph_id, &node_id)
            }
            PlayerAction::Use(query) => self.use_item(session, &query),
            PlayerAction::Equip(query) => self.equip_item(session, &query),
            PlayerAction::Unequip(query) => self.unequip_item(session, &query),
            PlayerAction::Party(action) => self.apply_party(session, action),
            PlayerAction::Quest(action) => self.apply_quest(session, action),
            PlayerAction::Economy(action) => self.apply_economy(session, action),
            PlayerAction::Weather => Ok(ActionOutcome {
                scene: self.scene_for(session)?,
                output: Some(format!(
                    "The weather is {}.\r\n",
                    weather_label(self.adventure.weather)
                )),
                changed_character: None,
                domain_events: Vec::new(),
            }),
            PlayerAction::Time => Ok(ActionOutcome {
                scene: self.scene_for(session)?,
                output: Some(format!(
                    "It is hour {} of day {} (world tick {}).\r\n",
                    self.logical_tick % 24,
                    self.logical_tick / 24 + 1,
                    self.logical_tick
                )),
                changed_character: None,
                domain_events: Vec::new(),
            }),
            PlayerAction::Social(_) => Err(WorldError::GameplayUnavailable),
            PlayerAction::Move(direction) => {
                let location = self.character(session)?.location;
                let source_room = room_checked(&self.entities, location)?;
                let from_room_id = source_room.stable_id.clone();
                if !self.exit_visible_to(session, &from_room_id, direction)? {
                    return Err(WorldError::NoExit);
                }
                let destination = source_room
                    .exits
                    .get(&direction)
                    .copied()
                    .ok_or(WorldError::NoExit)?;
                // Validate the destination before mutating character state. This
                // turns a corrupt graph edge into a recoverable world error.
                let to_room_id = room_checked(&self.entities, destination)?.stable_id.clone();
                let character_id = *self
                    .sessions
                    .get(&session)
                    .ok_or(WorldError::UnknownSession)?;
                let (actor_id, actor_name) = match self.entities.get_mut(character_id) {
                    Some(Entity::Character(character)) => {
                        character.location = destination;
                        (character.id, character.name.clone())
                    }
                    _ => return Err(WorldError::InvalidEntity),
                };
                self.revision = self.revision.wrapping_add(1).max(1);
                let event = DomainEvent::CharacterMoved {
                    actor_id,
                    actor_name,
                    from_room_id,
                    to_room_id: to_room_id.clone(),
                    direction,
                };
                self.bump_view_revisions(&event)?;
                self.observer_views
                    .get_mut(&session)
                    .ok_or(WorldError::InvalidEntity)?
                    .discovered_rooms
                    .insert(to_room_id);
                Ok(ActionOutcome {
                    scene: self.scene_for(session)?,
                    output: None,
                    changed_character: Some(self.character_snapshot(session)?),
                    domain_events: vec![event],
                })
            }
        }
    }

    pub fn scene_for(&self, session: SessionId) -> Result<SceneView, WorldError> {
        let current_room = room_checked(&self.entities, self.character(session)?.location)?;
        let observer = self.observer(session)?;
        let visual_scene = self.can_receive_visual_scene(session)?;
        let mut exits = Vec::new();
        for (direction, destination) in &current_room.exits {
            if !self.exit_visible_to(session, &current_room.stable_id, *direction)? {
                continue;
            }
            let destination = room_checked(&self.entities, *destination)?;
            exits.push(SceneExit {
                id: opaque_id(observer.salt, "room", &destination.stable_id),
                label: direction.as_str().to_owned(),
                command: direction.as_str().to_owned(),
            });
        }
        let mut items = self
            .item_locations
            .iter()
            .filter_map(|(item_id, location)| match location {
                SharedItemLocation::Room(room_id) if room_id == &current_room.stable_id => {
                    Some(item_id)
                }
                SharedItemLocation::Staged
                | SharedItemLocation::Carried(_)
                | SharedItemLocation::Room(_)
                | SharedItemLocation::Equipped(_, _)
                | SharedItemLocation::HeldByNpc(_)
                | SharedItemLocation::Consumed => None,
            })
            .map(|item_id| {
                self.item_visible_to(session, item_id, &current_room.stable_id)
                    .map(|visible| visible.then_some(item_id))
            })
            .collect::<Result<Vec<_>, WorldError>>()?
            .into_iter()
            .flatten()
            .map(|item_id| {
                let item = self
                    .stable_items
                    .get(item_id)
                    .ok_or(WorldError::InvalidEntity)?;
                let view_item_id = opaque_id(observer.salt, "item", item_id);
                Ok(SceneItem {
                    id: view_item_id.clone(),
                    label: item.name.clone(),
                    description: item.description.clone(),
                    actions: vec![
                        SceneAction {
                            label: "Examine".into(),
                            command: format!("examine {view_item_id}"),
                        },
                        SceneAction {
                            label: "Take".into(),
                            command: format!("take {view_item_id}"),
                        },
                    ],
                })
            })
            .collect::<Result<Vec<_>, WorldError>>()?;
        items.sort_by(|left, right| left.label.cmp(&right.label));
        let npcs = self
            .npcs
            .values()
            .filter(|npc| npc.alive() && npc.spawn_room_id == current_room.stable_id)
            .map(|npc| {
                self.npc_visible_to(session, &npc.stable_id, &current_room.stable_id)
                    .map(|visible| visible.then_some(npc))
            })
            .collect::<Result<Vec<_>, WorldError>>()?
            .into_iter()
            .flatten()
            .map(|npc| {
                let view_npc_id = opaque_id(observer.salt, "npc", &npc.stable_id);
                SceneNpc {
                    id: view_npc_id.clone(),
                    label: npc.name.clone(),
                    description: npc.description.clone(),
                    health: npc.health,
                    max_health: npc.max_health,
                    actions: vec![SceneAction {
                        label: "Attack".into(),
                        command: format!("attack {view_npc_id}"),
                    }],
                }
            })
            .collect();
        Ok(SceneView {
            view_id: opaque_id(observer.salt, "room", &current_room.stable_id),
            revision: observer.revision,
            title: current_room.title.clone(),
            prose: if visual_scene {
                current_room.prose.clone()
            } else {
                vec![
                    "Darkness or impaired vision conceals the room's visual details. No visible object, creature, or exit can be identified."
                        .into(),
                ]
            },
            image: visual_scene.then(|| current_room.image.clone()).flatten(),
            audio: self
                .can_receive_audio_scene(session)?
                .then(|| current_room.audio.clone())
                .flatten(),
            exits,
            items,
            npcs,
            weather: WeatherView {
                condition: weather_label(self.adventure.weather).into(),
                day: self.logical_tick / 24 + 1,
                hour: self.logical_tick % 24,
                next_change_in_ticks: self
                    .adventure
                    .next_weather_tick
                    .saturating_sub(self.logical_tick),
            },
        })
    }

    /// Return presentation-only joins for entities already visible in the
    /// observer's current semantic scene. It cannot discover hidden, dead,
    /// carried, equipped or out-of-room entities.
    #[doc(hidden)]
    pub fn visible_scene_entity_ids(
        &self,
        session: SessionId,
    ) -> Result<VisibleSceneEntityIds, WorldError> {
        let current_room = room_checked(&self.entities, self.character(session)?.location)?;
        let observer = self.observer(session)?;
        let mut items = self
            .item_locations
            .iter()
            .filter_map(|(stable_id, location)| {
                matches!(location, SharedItemLocation::Room(room_id) if room_id == &current_room.stable_id)
                    .then_some(stable_id)
            })
            .map(|stable_id| {
                self.item_visible_to(session, stable_id, &current_room.stable_id)
                    .map(|visible| {
                        visible.then(|| VisibleSceneEntityId {
                            stable_id: stable_id.clone(),
                            opaque_id: opaque_id(observer.salt, "item", stable_id),
                        })
                    })
            })
            .collect::<Result<Vec<_>, WorldError>>()?
            .into_iter()
            .flatten()
            .collect::<Vec<_>>();
        let mut npcs = self
            .npcs
            .values()
            .filter(|npc| npc.alive() && npc.spawn_room_id == current_room.stable_id)
            .map(|npc| {
                self.npc_visible_to(session, &npc.stable_id, &current_room.stable_id)
                    .map(|visible| {
                        visible.then(|| VisibleSceneEntityId {
                            stable_id: npc.stable_id.clone(),
                            opaque_id: opaque_id(observer.salt, "npc", &npc.stable_id),
                        })
                    })
            })
            .collect::<Result<Vec<_>, WorldError>>()?
            .into_iter()
            .flatten()
            .collect::<Vec<_>>();
        items.sort_by(|left, right| left.stable_id.cmp(&right.stable_id));
        npcs.sort_by(|left, right| left.stable_id.cmp(&right.stable_id));
        Ok(VisibleSceneEntityIds { items, npcs })
    }

    pub fn inventory_as_plain_text(&self, session: SessionId) -> Result<String, WorldError> {
        let character = self.character(session)?;
        let mut item_names = self
            .item_locations
            .iter()
            .filter_map(|(item_id, location)| {
                matches!(location, SharedItemLocation::Carried(owner) if *owner == character.id)
                    .then_some(item_id)
            })
            .map(|item_id| {
                self.stable_items
                    .get(item_id)
                    .map(|item| item.name.as_str())
                    .ok_or(WorldError::InvalidEntity)
            })
            .collect::<Result<Vec<_>, WorldError>>()?;
        item_names.sort_unstable();
        let gameplay = self
            .character_gameplay
            .get(&character.id)
            .ok_or(WorldError::InvalidEntity)?;
        if item_names.is_empty() && gameplay.equipment.is_empty() {
            return Ok("You are carrying nothing.\r\n".into());
        }
        let mut output = String::new();
        if !item_names.is_empty() {
            output.push_str("You are carrying:\r\n");
            for name in item_names {
                output.push_str("- ");
                output.push_str(name);
                output.push_str("\r\n");
            }
        }
        if !gameplay.equipment.is_empty() {
            output.push_str("Equipped:\r\n");
            for (slot, item_id) in &gameplay.equipment {
                let item = self
                    .stable_items
                    .get(item_id)
                    .ok_or(WorldError::InvalidEntity)?;
                output.push_str(&format!("- {}: {}\r\n", slot.as_str(), item.name));
            }
        }
        Ok(output)
    }

    fn apply_party(
        &mut self,
        session: SessionId,
        action: PartyAction,
    ) -> Result<ActionOutcome, WorldError> {
        let actor = self.character(session)?;
        let actor_id = actor.id;
        let actor_name = actor.name.clone();
        match action {
            PartyAction::Create => {
                if self.party_for_member(actor_id).is_some() {
                    return Err(WorldError::PartyUnavailable);
                }
                let party_id = self.adventure.next_party_id.max(1);
                self.adventure.next_party_id = party_id
                    .checked_add(1)
                    .ok_or(WorldError::PartyUnavailable)?;
                self.adventure.parties.insert(
                    party_id,
                    PartySnapshot {
                        id: party_id,
                        leader: actor_id,
                        members: vec![actor_id],
                        invitations: BTreeMap::new(),
                    },
                );
                let audience = self.party_member_audience(party_id, &[]);
                self.finish_adventure_action(
                    session,
                    "You create an adventuring party.\r\n".into(),
                    AdventureEvent::PartyChanged {
                        character_id: actor_id,
                        party_id,
                        audience,
                        summary: format!("{actor_name} creates an adventuring party."),
                    },
                )
            }
            PartyAction::Invite(target) => {
                let party_id = self
                    .party_for_member(actor_id)
                    .ok_or(WorldError::PartyUnavailable)?;
                let leader = self
                    .adventure
                    .parties
                    .get(&party_id)
                    .ok_or(WorldError::PartyUnavailable)?
                    .leader;
                if leader != actor_id {
                    return Err(WorldError::PartyForbidden);
                }
                let target_id = self.resolve_connected_character(&target)?;
                if target_id == actor_id || self.party_for_member(target_id).is_some() {
                    return Err(WorldError::PartyUnavailable);
                }
                let party = self
                    .adventure
                    .parties
                    .get_mut(&party_id)
                    .ok_or(WorldError::PartyUnavailable)?;
                if party.members.len() >= 6 || party.invitations.len() >= 12 {
                    return Err(WorldError::PartyUnavailable);
                }
                party
                    .invitations
                    .insert(target_id, self.logical_tick.saturating_add(20));
                let mut audience = vec![actor_id, target_id];
                audience.sort_unstable();
                audience.dedup();
                self.finish_adventure_action(
                    session,
                    format!("You invite {target} to your party.\r\n"),
                    AdventureEvent::PartyChanged {
                        character_id: actor_id,
                        party_id,
                        audience,
                        summary: format!("{actor_name} sends a party invitation."),
                    },
                )
            }
            PartyAction::Accept(leader) => {
                if self.party_for_member(actor_id).is_some() {
                    return Err(WorldError::PartyUnavailable);
                }
                let leader_id = self.resolve_connected_character(&leader)?;
                let party_id = self
                    .adventure
                    .parties
                    .iter()
                    .find_map(|(id, party)| (party.leader == leader_id).then_some(*id))
                    .ok_or(WorldError::PartyUnavailable)?;
                let party = self
                    .adventure
                    .parties
                    .get_mut(&party_id)
                    .ok_or(WorldError::PartyUnavailable)?;
                let expiry = *party
                    .invitations
                    .get(&actor_id)
                    .ok_or(WorldError::PartyForbidden)?;
                if expiry < self.logical_tick || party.members.len() >= 6 {
                    return Err(WorldError::PartyUnavailable);
                }
                party.invitations.remove(&actor_id);
                party.members.push(actor_id);
                party.members.sort_unstable();
                let audience = self.party_member_audience(party_id, &[]);
                self.finish_adventure_action(
                    session,
                    format!("You join {leader}'s party.\r\n"),
                    AdventureEvent::PartyChanged {
                        character_id: actor_id,
                        party_id,
                        audience,
                        summary: format!("{actor_name} joins an adventuring party."),
                    },
                )
            }
            PartyAction::Revoke(target) => {
                let party_id = self
                    .party_for_member(actor_id)
                    .ok_or(WorldError::PartyUnavailable)?;
                if self.adventure.parties[&party_id].leader != actor_id {
                    return Err(WorldError::PartyForbidden);
                }
                let target_id = self.resolve_connected_character(&target)?;
                let removed = self
                    .adventure
                    .parties
                    .get_mut(&party_id)
                    .ok_or(WorldError::PartyUnavailable)?
                    .invitations
                    .remove(&target_id)
                    .is_some();
                if !removed {
                    return Err(WorldError::PartyUnavailable);
                }
                let mut audience = vec![actor_id, target_id];
                audience.sort_unstable();
                audience.dedup();
                self.finish_adventure_action(
                    session,
                    format!("You revoke {target}'s party invitation.\r\n"),
                    AdventureEvent::PartyChanged {
                        character_id: actor_id,
                        party_id,
                        audience,
                        summary: format!("{actor_name} revokes a party invitation."),
                    },
                )
            }
            PartyAction::Kick(target) => {
                let party_id = self
                    .party_for_member(actor_id)
                    .ok_or(WorldError::PartyUnavailable)?;
                if self.adventure.parties[&party_id].leader != actor_id {
                    return Err(WorldError::PartyForbidden);
                }
                let target_id = self.resolve_connected_character(&target)?;
                if target_id == actor_id
                    || !self.adventure.parties[&party_id]
                        .members
                        .contains(&target_id)
                {
                    return Err(WorldError::PartyUnavailable);
                }
                let party = self
                    .adventure
                    .parties
                    .get_mut(&party_id)
                    .ok_or(WorldError::PartyUnavailable)?;
                party.members.retain(|member| *member != target_id);
                party.invitations.remove(&target_id);
                let audience = self.party_member_audience(party_id, &[target_id]);
                self.finish_adventure_action(
                    session,
                    format!("You remove {target} from your party.\r\n"),
                    AdventureEvent::PartyChanged {
                        character_id: actor_id,
                        party_id,
                        audience,
                        summary: format!("{actor_name} removes a companion from the party."),
                    },
                )
            }
            PartyAction::Transfer(target) => {
                let party_id = self
                    .party_for_member(actor_id)
                    .ok_or(WorldError::PartyUnavailable)?;
                if self.adventure.parties[&party_id].leader != actor_id {
                    return Err(WorldError::PartyForbidden);
                }
                let target_id = self.resolve_connected_character(&target)?;
                if target_id == actor_id
                    || !self.adventure.parties[&party_id]
                        .members
                        .contains(&target_id)
                {
                    return Err(WorldError::PartyUnavailable);
                }
                self.adventure
                    .parties
                    .get_mut(&party_id)
                    .ok_or(WorldError::PartyUnavailable)?
                    .leader = target_id;
                let audience = self.party_member_audience(party_id, &[]);
                self.finish_adventure_action(
                    session,
                    format!("You transfer party leadership to {target}.\r\n"),
                    AdventureEvent::PartyChanged {
                        character_id: actor_id,
                        party_id,
                        audience,
                        summary: format!("{actor_name} transfers party leadership."),
                    },
                )
            }
            PartyAction::Disband => {
                let party_id = self
                    .party_for_member(actor_id)
                    .ok_or(WorldError::PartyUnavailable)?;
                if self.adventure.parties[&party_id].leader != actor_id {
                    return Err(WorldError::PartyForbidden);
                }
                let audience = self.party_event_audience(party_id, &[]);
                self.adventure.parties.remove(&party_id);
                self.finish_adventure_action(
                    session,
                    "You disband your party.\r\n".into(),
                    AdventureEvent::PartyChanged {
                        character_id: actor_id,
                        party_id,
                        audience,
                        summary: format!("{actor_name} disbands an adventuring party."),
                    },
                )
            }
            PartyAction::Leave => {
                let party_id = self
                    .party_for_member(actor_id)
                    .ok_or(WorldError::PartyUnavailable)?;
                let audience = self.party_event_audience(party_id, &[actor_id]);
                let mut remove_party = false;
                {
                    let party = self
                        .adventure
                        .parties
                        .get_mut(&party_id)
                        .ok_or(WorldError::PartyUnavailable)?;
                    party.members.retain(|member| *member != actor_id);
                    party.invitations.remove(&actor_id);
                    if party.members.is_empty() {
                        remove_party = true;
                    } else if party.leader == actor_id {
                        party.leader = party.members[0];
                    }
                }
                if remove_party {
                    self.adventure.parties.remove(&party_id);
                }
                self.finish_adventure_action(
                    session,
                    "You leave your party.\r\n".into(),
                    AdventureEvent::PartyChanged {
                        character_id: actor_id,
                        party_id,
                        audience,
                        summary: format!("{actor_name} leaves an adventuring party."),
                    },
                )
            }
            PartyAction::Status => {
                let output = if let Some(party_id) = self.party_for_member(actor_id) {
                    let party = &self.adventure.parties[&party_id];
                    let names = party
                        .members
                        .iter()
                        .map(|member| {
                            self.connected_character_name(*member)
                                .unwrap_or_else(|| "an absent companion".into())
                        })
                        .collect::<Vec<_>>()
                        .join(", ");
                    format!("Party members: {names}.\r\n")
                } else {
                    "You are not in a party.\r\n".into()
                };
                Ok(ActionOutcome {
                    scene: self.scene_for(session)?,
                    output: Some(output),
                    changed_character: None,
                    domain_events: Vec::new(),
                })
            }
        }
    }

    fn apply_quest(
        &mut self,
        session: SessionId,
        action: QuestAction,
    ) -> Result<ActionOutcome, WorldError> {
        const QUEST_ID: &str = "first-hunt";
        let character = self.character(session)?;
        let character_id = character.id;
        let character_name = character.name.clone();
        let room_id = room_checked(&self.entities, character.location)?
            .stable_id
            .clone();
        match action {
            QuestAction::List => Ok(ActionOutcome {
                scene: self.scene_for(session)?,
                output: Some("Available quest: first-hunt — defeat one hostile creature; reward 25 coin and 50 experience.\r\n".into()),
                changed_character: None,
                domain_events: Vec::new(),
            }),
            QuestAction::Status => {
                let state = self
                    .character_gameplay
                    .get(&character_id)
                    .ok_or(WorldError::InvalidEntity)?;
                let output = match state.quests.get(QUEST_ID) {
                    None => "The first-hunt quest has not been accepted.\r\n".into(),
                    Some(progress) => format!(
                        "First Hunt: {:?} ({}/{}).\r\n",
                        progress.status, progress.current, progress.target
                    ),
                };
                Ok(ActionOutcome {
                    scene: self.scene_for(session)?,
                    output: Some(output),
                    changed_character: None,
                    domain_events: Vec::new(),
                })
            }
            QuestAction::Accept(query) => {
                if !quest_matches(&query) {
                    return Err(WorldError::QuestUnavailable);
                }
                let state = self
                    .character_gameplay
                    .get_mut(&character_id)
                    .ok_or(WorldError::InvalidEntity)?;
                if state.quests.contains_key(QUEST_ID) || state.quests.len() >= 32 {
                    return Err(WorldError::QuestUnavailable);
                }
                state.quests.insert(
                    QUEST_ID.into(),
                    QuestProgress {
                        status: QuestStatus::Active,
                        current: 0,
                        target: 1,
                    },
                );
                self.finish_adventure_action(
                    session,
                    "You accept First Hunt.\r\n".into(),
                    AdventureEvent::QuestChanged {
                        character_id,
                        quest_id: QUEST_ID.into(),
                        summary: "First Hunt accepted.".into(),
                    },
                )
            }
            QuestAction::Complete(query) => {
                if !quest_matches(&query) {
                    return Err(WorldError::QuestUnavailable);
                }
                let rewarded_experience = {
                    let state = self
                        .character_gameplay
                        .get_mut(&character_id)
                        .ok_or(WorldError::InvalidEntity)?;
                    let progress = state
                        .quests
                        .get_mut(QUEST_ID)
                        .ok_or(WorldError::QuestUnavailable)?;
                    if progress.status != QuestStatus::ReadyToClaim {
                        return Err(WorldError::QuestUnavailable);
                    }
                    state.coins = state
                        .coins
                        .checked_add(25)
                        .ok_or(WorldError::QuestUnavailable)?;
                    state.experience = state
                        .experience
                        .checked_add(50)
                        .ok_or(WorldError::QuestUnavailable)?;
                    progress.status = QuestStatus::Completed;
                    state.experience
                };
                let mut progression_events = vec![DomainEvent::Gameplay {
                    event: GameplayEvent::ExperienceGained {
                        character_id,
                        character_name: character_name.clone(),
                        amount: 50,
                        total: rewarded_experience,
                        room_id: room_id.clone(),
                    },
                }];
                self.advance_character_progression(
                    character_id,
                    &character_name,
                    &room_id,
                    &mut progression_events,
                )?;
                let mut outcome = self.finish_adventure_action(
                    session,
                    "First Hunt complete: you receive 25 coin and 50 experience.\r\n".into(),
                    AdventureEvent::QuestChanged {
                        character_id,
                        quest_id: QUEST_ID.into(),
                        summary: "First Hunt completed and its reward claimed.".into(),
                    },
                )?;
                outcome.domain_events.extend(progression_events);
                Ok(outcome)
            }
        }
    }

    fn apply_economy(
        &mut self,
        session: SessionId,
        action: EconomyAction,
    ) -> Result<ActionOutcome, WorldError> {
        let character = self.character(session)?;
        let character_id = character.id;
        let at_shop = room_checked(&self.entities, character.location)?.stable_id
            == self.start_room_stable_id;
        match action {
            EconomyAction::Balance => {
                let coins = self
                    .character_gameplay
                    .get(&character_id)
                    .ok_or(WorldError::InvalidEntity)?
                    .coins;
                Ok(ActionOutcome {
                    scene: self.scene_for(session)?,
                    output: Some(format!("You have {coins} coin.\r\n")),
                    changed_character: None,
                    domain_events: Vec::new(),
                })
            }
            EconomyAction::Shop => {
                if !at_shop {
                    return Err(WorldError::ShopUnavailable);
                }
                let output = commodity_specs()
                    .iter()
                    .map(|(id, label, buy, sell)| {
                        let stock = self.adventure.shop_stock.get(*id).copied().unwrap_or(0);
                        format!("- {label}: buy {buy}, sell {sell}, stock {stock}\r\n")
                    })
                    .collect::<String>();
                Ok(ActionOutcome {
                    scene: self.scene_for(session)?,
                    output: Some(format!("Lantern Market:\r\n{output}")),
                    changed_character: None,
                    domain_events: Vec::new(),
                })
            }
            EconomyAction::Buy(query) => {
                if !at_shop {
                    return Err(WorldError::ShopUnavailable);
                }
                let (commodity_id, label, price, _) = resolve_commodity(&query)?;
                let stock = self
                    .adventure
                    .shop_stock
                    .get_mut(commodity_id)
                    .ok_or(WorldError::CommodityNotFound)?;
                if *stock == 0 {
                    return Err(WorldError::ShopUnavailable);
                }
                let state = self
                    .character_gameplay
                    .get_mut(&character_id)
                    .ok_or(WorldError::InvalidEntity)?;
                if state.coins < price {
                    return Err(WorldError::InsufficientFunds);
                }
                let owned = state.commodities.entry(commodity_id.into()).or_default();
                if *owned >= 99 {
                    return Err(WorldError::ShopUnavailable);
                }
                state.coins -= price;
                *owned += 1;
                *stock -= 1;
                self.finish_adventure_action(
                    session,
                    format!("You buy {label} for {price} coin.\r\n"),
                    AdventureEvent::EconomyChanged {
                        character_id,
                        commodity_id: Some(commodity_id.into()),
                        summary: format!("A market purchase of {label} is completed."),
                    },
                )
            }
            EconomyAction::Sell(query) => {
                if !at_shop {
                    return Err(WorldError::ShopUnavailable);
                }
                let (commodity_id, label, _, price) = resolve_commodity(&query)?;
                let state = self
                    .character_gameplay
                    .get_mut(&character_id)
                    .ok_or(WorldError::InvalidEntity)?;
                let owned = state
                    .commodities
                    .get_mut(commodity_id)
                    .ok_or(WorldError::CommodityNotOwned)?;
                if *owned == 0 {
                    return Err(WorldError::CommodityNotOwned);
                }
                state.coins = state
                    .coins
                    .checked_add(price)
                    .ok_or(WorldError::ShopUnavailable)?;
                *owned -= 1;
                let stock = self
                    .adventure
                    .shop_stock
                    .entry(commodity_id.into())
                    .or_default();
                *stock = stock.saturating_add(1).min(100);
                self.finish_adventure_action(
                    session,
                    format!("You sell {label} for {price} coin.\r\n"),
                    AdventureEvent::EconomyChanged {
                        character_id,
                        commodity_id: Some(commodity_id.into()),
                        summary: format!("A market sale of {label} is completed."),
                    },
                )
            }
        }
    }

    fn finish_adventure_action(
        &mut self,
        session: SessionId,
        output: String,
        event: AdventureEvent,
    ) -> Result<ActionOutcome, WorldError> {
        let event = DomainEvent::Adventure { event };
        self.revision = self.revision.wrapping_add(1).max(1);
        self.bump_view_revisions(&event)?;
        Ok(ActionOutcome {
            scene: self.scene_for(session)?,
            output: Some(output),
            changed_character: None,
            domain_events: vec![event],
        })
    }

    fn party_for_member(&self, character_id: CharacterId) -> Option<u64> {
        self.adventure
            .parties
            .iter()
            .find_map(|(id, party)| party.members.contains(&character_id).then_some(*id))
    }

    fn party_event_audience(&self, party_id: u64, additional: &[CharacterId]) -> Vec<CharacterId> {
        let mut audience = self
            .adventure
            .parties
            .get(&party_id)
            .map(|party| {
                party
                    .members
                    .iter()
                    .chain(party.invitations.keys())
                    .copied()
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        audience.extend_from_slice(additional);
        audience.sort_unstable();
        audience.dedup();
        audience
    }

    fn party_member_audience(&self, party_id: u64, additional: &[CharacterId]) -> Vec<CharacterId> {
        let mut audience = self
            .adventure
            .parties
            .get(&party_id)
            .map(|party| party.members.clone())
            .unwrap_or_default();
        audience.extend_from_slice(additional);
        audience.sort_unstable();
        audience.dedup();
        audience
    }

    fn resolve_connected_character(&self, query: &str) -> Result<CharacterId, WorldError> {
        let query = query.trim();
        let matches = self
            .connected_sessions()
            .into_iter()
            .filter_map(|session| self.character(session).ok())
            .filter(|character| character.name.eq_ignore_ascii_case(query))
            .map(|character| character.id)
            .collect::<Vec<_>>();
        match matches.as_slice() {
            [id] => Ok(*id),
            _ => Err(WorldError::PartyTargetNotFound),
        }
    }

    fn connected_character_name(&self, character_id: CharacterId) -> Option<String> {
        self.session_for_character(character_id)
            .and_then(|session| self.character(session).ok())
            .map(|character| character.name.clone())
    }

    fn start_combat(
        &mut self,
        session: SessionId,
        query: &str,
    ) -> Result<ActionOutcome, WorldError> {
        let character = self.character(session)?;
        let character_id = character.id;
        let character_name = character.name.clone();
        let gameplay = self
            .character_gameplay
            .get(&character_id)
            .ok_or(WorldError::InvalidEntity)?;
        if gameplay.vitals.health <= 0
            || gameplay.recovery_at_tick.is_some()
            || gameplay
                .statuses
                .iter()
                .any(|status| status.kind == ActiveStatusKind::Stun)
        {
            return Err(WorldError::CombatUnavailable);
        }
        let room_id = room_checked(&self.entities, character.location)?
            .stable_id
            .clone();
        let npc_id = self.resolve_visible_npc(session, query)?;
        let npc = self.npcs.get_mut(&npc_id).ok_or(WorldError::NpcNotFound)?;
        if !npc.alive()
            || npc.spawn_room_id != room_id
            || npc
                .engaged_character
                .is_some_and(|engaged| engaged != character_id)
        {
            return Err(WorldError::CombatUnavailable);
        }
        npc.engaged_character = Some(character_id);
        let npc_name = npc.name.clone();
        let event = DomainEvent::Gameplay {
            event: GameplayEvent::CombatStarted {
                character_id,
                character_name,
                npc_id,
                npc_name: npc_name.clone(),
                room_id,
            },
        };
        self.finish_gameplay_action(
            session,
            format!("You engage the {npc_name}.\r\n"),
            vec![event],
        )
    }

    fn use_ability(
        &mut self,
        session: SessionId,
        ability_query: &str,
        target_query: Option<&str>,
    ) -> Result<ActionOutcome, WorldError> {
        let checkpoint = self.checkpoint();
        match self.use_ability_inner(session, ability_query, target_query) {
            Ok(outcome) => Ok(outcome),
            Err(error) => {
                self.restore_checkpoint(checkpoint);
                Err(error)
            }
        }
    }

    fn use_ability_inner(
        &mut self,
        session: SessionId,
        ability_query: &str,
        target_query: Option<&str>,
    ) -> Result<ActionOutcome, WorldError> {
        if ability_query.is_empty()
            || ability_query.len() > 128
            || ability_query.chars().any(char::is_control)
        {
            return Err(WorldError::AbilityNotKnown);
        }
        let character = self.character(session)?;
        let character_id = character.id;
        let character_name = character.name.clone();
        let room_id = room_checked(&self.entities, character.location)?
            .stable_id
            .clone();
        let state = self
            .character_gameplay
            .get(&character_id)
            .cloned()
            .ok_or(WorldError::InvalidEntity)?;
        if state.vitals.health <= 0
            || state.recovery_at_tick.is_some()
            || state.ability_global_ready_tick > self.logical_tick
            || state
                .statuses
                .iter()
                .any(|status| status.kind == ActiveStatusKind::Stun)
        {
            return Err(WorldError::AbilityUnavailable);
        }
        let known = state
            .ability_ids
            .iter()
            .filter_map(|id| self.abilities.get(id))
            .filter(|ability| {
                ability.id.eq_ignore_ascii_case(ability_query)
                    || ability.name.eq_ignore_ascii_case(ability_query)
            })
            .cloned()
            .collect::<Vec<_>>();
        let [ability] = known.as_slice() else {
            return Err(WorldError::AbilityNotKnown);
        };
        let ability = ability.clone();
        if state
            .ability_cooldowns
            .get(&ability.id)
            .is_some_and(|ready| *ready > self.logical_tick)
        {
            return Err(WorldError::AbilityUnavailable);
        }
        if ability.resource_costs.iter().any(|(id, cost)| {
            state
                .resources
                .get(id)
                .is_none_or(|pool| pool.current < *cost)
        }) {
            return Err(WorldError::InsufficientResource);
        }

        let npc_target = match ability.targeting {
            AbilityTargeting::SelfOnly => {
                if target_query.is_some() {
                    return Err(WorldError::InvalidAbilityTarget);
                }
                None
            }
            AbilityTargeting::SingleEnemy => {
                let target = target_query.ok_or(WorldError::InvalidAbilityTarget)?;
                let npc_id = self.resolve_visible_npc(session, target)?;
                let npc = self.npcs.get(&npc_id).ok_or(WorldError::NpcNotFound)?;
                if !npc.alive()
                    || npc.spawn_room_id != room_id
                    || npc
                        .engaged_character
                        .is_some_and(|engaged| engaged != character_id)
                {
                    return Err(WorldError::InvalidAbilityTarget);
                }
                Some(npc_id)
            }
            AbilityTargeting::SingleAlly => {
                if target_query.is_some_and(|target| {
                    !target.eq_ignore_ascii_case("self")
                        && !target.eq_ignore_ascii_case(&character_name)
                }) {
                    return Err(WorldError::InvalidAbilityTarget);
                }
                None
            }
        };

        let caster_statuses = ability
            .status_effects
            .iter()
            .filter(|status| status_target(status) == AbilityEffectTarget::Caster)
            .count()
            + if npc_target.is_none() {
                ability
                    .status_effects
                    .iter()
                    .filter(|status| status_target(status) == AbilityEffectTarget::Target)
                    .count()
            } else {
                0
            };
        if state.statuses.len() + caster_statuses > MAX_ACTIVE_STATUSES {
            return Err(WorldError::AbilityUnavailable);
        }
        if let Some(npc_id) = &npc_target {
            let target_statuses = ability
                .status_effects
                .iter()
                .filter(|status| status_target(status) == AbilityEffectTarget::Target)
                .count();
            if self.npcs[npc_id].statuses.len() + target_statuses > MAX_ACTIVE_STATUSES {
                return Err(WorldError::AbilityUnavailable);
            }
        }

        let npc_name = npc_target
            .as_ref()
            .and_then(|id| self.npcs.get(id).map(|npc| npc.name.clone()));
        let mut events = vec![DomainEvent::Gameplay {
            event: GameplayEvent::AbilityUsed {
                character_id,
                character_name: character_name.clone(),
                ability_id: ability.id.clone(),
                ability_name: ability.name.clone(),
                npc_id: npc_target.clone(),
                npc_name,
                room_id: room_id.clone(),
            },
        }];

        {
            let state = self
                .character_gameplay
                .get_mut(&character_id)
                .ok_or(WorldError::InvalidEntity)?;
            for (resource_id, cost) in &ability.resource_costs {
                let pool = state
                    .resources
                    .get_mut(resource_id)
                    .ok_or(WorldError::InsufficientResource)?;
                pool.current -= *cost;
            }
            if ability.cooldown_ticks > 0 {
                state.ability_cooldowns.insert(
                    ability.id.clone(),
                    self.logical_tick
                        .saturating_add(u64::from(ability.cooldown_ticks)),
                );
            }
            state.ability_global_ready_tick = self.logical_tick.saturating_add(1);
            if npc_target.is_some() {
                state.last_offensive_ability_tick = Some(self.logical_tick);
            }
        }

        if let Some(npc_id) = &npc_target {
            self.npcs
                .get_mut(npc_id)
                .ok_or(WorldError::NpcNotFound)?
                .engaged_character = Some(character_id);
        }

        if let Some(damage) = &ability.damage {
            let amount = ability_scaled_amount(
                damage.base_amount,
                damage.scaling_attribute.as_deref(),
                damage.scaling_percent,
                &state,
            );
            match (damage.target, npc_target.as_deref()) {
                (AbilityEffectTarget::Target, Some(npc_id)) => {
                    let resistance = self.npcs[npc_id]
                        .resistances
                        .get(&damage.damage_type)
                        .copied()
                        .unwrap_or(0);
                    let amount = resisted_amount(amount, resistance);
                    let npc = self.npcs.get_mut(npc_id).ok_or(WorldError::NpcNotFound)?;
                    npc.health = npc.health.saturating_sub(amount).max(0);
                    let remaining_health = npc.health;
                    let npc_name = npc.name.clone();
                    events.push(DomainEvent::Gameplay {
                        event: GameplayEvent::NpcDamaged {
                            character_id,
                            character_name: character_name.clone(),
                            npc_id: npc_id.to_owned(),
                            npc_name: npc_name.clone(),
                            amount,
                            remaining_health,
                            room_id: room_id.clone(),
                        },
                    });
                    if remaining_health == 0 {
                        self.resolve_npc_defeat(
                            self.logical_tick,
                            npc_id,
                            character_id,
                            &character_name,
                            &room_id,
                            &mut events,
                        )?;
                    }
                }
                (AbilityEffectTarget::Caster, _) | (AbilityEffectTarget::Target, None) => {
                    let resistance = state
                        .resistances
                        .get(&damage.damage_type)
                        .copied()
                        .unwrap_or(0);
                    let amount = resisted_amount(amount, resistance);
                    let state = self
                        .character_gameplay
                        .get_mut(&character_id)
                        .ok_or(WorldError::InvalidEntity)?;
                    state.vitals.health = state.vitals.health.saturating_sub(amount).max(0);
                }
            }
        }

        if let Some(heal) = &ability.heal {
            // Ally targeting is intentionally self-only in the first version;
            // PvP/group target authority is a later protocol milestone.
            if heal.target == AbilityEffectTarget::Target && npc_target.is_some() {
                return Err(WorldError::InvalidAbilityTarget);
            }
            let amount = ability_scaled_amount(
                heal.base_amount,
                heal.scaling_attribute.as_deref(),
                heal.scaling_percent,
                &state,
            );
            let state = self
                .character_gameplay
                .get_mut(&character_id)
                .ok_or(WorldError::InvalidEntity)?;
            let previous = state.vitals.health;
            state.vitals.health = state
                .vitals
                .health
                .saturating_add(amount.max(0))
                .min(state.vitals.max_health);
            let healed = state.vitals.health.saturating_sub(previous);
            if healed > 0 {
                events.push(DomainEvent::Gameplay {
                    event: GameplayEvent::CharacterHealed {
                        character_id,
                        character_name: character_name.clone(),
                        amount: healed,
                        health: state.vitals.health,
                        room_id: room_id.clone(),
                    },
                });
            }
        }

        for status in &ability.status_effects {
            let active = active_status(&ability.id, Some(character_id), None, status);
            if status_target(status) == AbilityEffectTarget::Target
                && let Some(npc_id) = &npc_target
            {
                let npc = self.npcs.get_mut(npc_id).ok_or(WorldError::NpcNotFound)?;
                if npc.alive() {
                    npc.statuses.push(active);
                }
            } else {
                self.character_gameplay
                    .get_mut(&character_id)
                    .ok_or(WorldError::InvalidEntity)?
                    .statuses
                    .push(active);
            }
        }

        let collapsed = {
            let state = self
                .character_gameplay
                .get_mut(&character_id)
                .ok_or(WorldError::InvalidEntity)?;
            if state.vitals.health == 0 && state.recovery_at_tick.is_none() {
                state.recovery_at_tick = Some(self.logical_tick.saturating_add(3));
                state.statuses.clear();
                true
            } else {
                false
            }
        };
        if collapsed {
            for npc in self.npcs.values_mut() {
                if npc.engaged_character == Some(character_id) {
                    npc.engaged_character = None;
                }
            }
            events.push(DomainEvent::Gameplay {
                event: GameplayEvent::CharacterCollapsed {
                    character_id,
                    character_name: character_name.clone(),
                    recover_at_tick: self.logical_tick.saturating_add(3),
                    room_id: room_id.clone(),
                },
            });
        }

        self.finish_gameplay_action(session, format!("You use {}.\r\n", ability.name), events)
    }

    fn use_item(&mut self, session: SessionId, query: &str) -> Result<ActionOutcome, WorldError> {
        let item_id = self.resolve_accessible_item(session, query, ItemScope::Carried)?;
        let definition = self
            .item_gameplay
            .get(&item_id)
            .cloned()
            .ok_or(WorldError::ItemNotUsable)?;
        if definition.heal_amount == 0 && definition.effect.is_none() {
            return Err(WorldError::ItemNotUsable);
        }
        let character = self.character(session)?;
        let character_id = character.id;
        let character_name = character.name.clone();
        let room_id = room_checked(&self.entities, character.location)?
            .stable_id
            .clone();
        let item_name = self
            .stable_items
            .get(&item_id)
            .ok_or(WorldError::InvalidEntity)?
            .name
            .clone();
        let state = self
            .character_gameplay
            .get_mut(&character_id)
            .ok_or(WorldError::InvalidEntity)?;
        if definition.effect.is_some() && state.effects.len() >= MAX_ACTIVE_EFFECTS {
            return Err(WorldError::GameplayUnavailable);
        }
        let mut events = vec![DomainEvent::Gameplay {
            event: GameplayEvent::ItemUsed {
                character_id,
                character_name: character_name.clone(),
                item_id: item_id.clone(),
                item_name: item_name.clone(),
                room_id: room_id.clone(),
            },
        }];
        let previous_health = state.vitals.health;
        state.vitals.health = state
            .vitals
            .health
            .saturating_add(definition.heal_amount.max(0))
            .min(state.vitals.max_health);
        let healed = state.vitals.health.saturating_sub(previous_health);
        if healed > 0 {
            events.push(DomainEvent::Gameplay {
                event: GameplayEvent::CharacterHealed {
                    character_id,
                    character_name: character_name.clone(),
                    amount: healed,
                    health: state.vitals.health,
                    room_id: room_id.clone(),
                },
            });
        }
        if let Some(effect) = definition.effect {
            state.effects.push(effect.clone());
            events.push(DomainEvent::Gameplay {
                event: GameplayEvent::EffectApplied {
                    character_id,
                    character_name,
                    effect,
                    room_id,
                },
            });
        }
        if definition.consumable {
            self.item_locations
                .insert(item_id, SharedItemLocation::Consumed);
        }
        self.finish_gameplay_action(session, format!("You use the {item_name}.\r\n"), events)
    }

    fn equip_item(&mut self, session: SessionId, query: &str) -> Result<ActionOutcome, WorldError> {
        let item_id = self.resolve_accessible_item(session, query, ItemScope::Carried)?;
        let definition = self
            .item_gameplay
            .get(&item_id)
            .ok_or(WorldError::ItemNotEquippable)?;
        let slot = definition
            .equipment_slot
            .ok_or(WorldError::ItemNotEquippable)?;
        let character = self.character(session)?;
        let character_id = character.id;
        let character_name = character.name.clone();
        let room_id = room_checked(&self.entities, character.location)?
            .stable_id
            .clone();
        let item_name = self
            .stable_items
            .get(&item_id)
            .ok_or(WorldError::InvalidEntity)?
            .name
            .clone();
        let state = self
            .character_gameplay
            .get_mut(&character_id)
            .ok_or(WorldError::InvalidEntity)?;
        let mut events = Vec::new();
        if let Some(previous_item_id) = state.equipment.insert(slot, item_id.clone()) {
            self.item_locations.insert(
                previous_item_id.clone(),
                SharedItemLocation::Carried(character_id),
            );
            let previous_name = self
                .stable_items
                .get(&previous_item_id)
                .ok_or(WorldError::InvalidEntity)?
                .name
                .clone();
            events.push(DomainEvent::Gameplay {
                event: GameplayEvent::ItemUnequipped {
                    character_id,
                    character_name: character_name.clone(),
                    item_id: previous_item_id,
                    item_name: previous_name,
                    slot,
                    room_id: room_id.clone(),
                },
            });
        }
        self.item_locations.insert(
            item_id.clone(),
            SharedItemLocation::Equipped(character_id, slot),
        );
        events.push(DomainEvent::Gameplay {
            event: GameplayEvent::ItemEquipped {
                character_id,
                character_name,
                item_id,
                item_name: item_name.clone(),
                slot,
                room_id,
            },
        });
        self.finish_gameplay_action(session, format!("You equip the {item_name}.\r\n"), events)
    }

    fn unequip_item(
        &mut self,
        session: SessionId,
        query: &str,
    ) -> Result<ActionOutcome, WorldError> {
        let slot = EquipmentSlot::parse(&query.trim().to_ascii_lowercase())
            .ok_or(WorldError::EquipmentSlotEmpty)?;
        let character = self.character(session)?;
        let character_id = character.id;
        let character_name = character.name.clone();
        let room_id = room_checked(&self.entities, character.location)?
            .stable_id
            .clone();
        let state = self
            .character_gameplay
            .get_mut(&character_id)
            .ok_or(WorldError::InvalidEntity)?;
        let item_id = state
            .equipment
            .remove(&slot)
            .ok_or(WorldError::EquipmentSlotEmpty)?;
        self.item_locations
            .insert(item_id.clone(), SharedItemLocation::Carried(character_id));
        let item_name = self
            .stable_items
            .get(&item_id)
            .ok_or(WorldError::InvalidEntity)?
            .name
            .clone();
        let event = DomainEvent::Gameplay {
            event: GameplayEvent::ItemUnequipped {
                character_id,
                character_name,
                item_id,
                item_name: item_name.clone(),
                slot,
                room_id,
            },
        };
        self.finish_gameplay_action(
            session,
            format!("You unequip the {item_name}.\r\n"),
            vec![event],
        )
    }

    fn finish_gameplay_action(
        &mut self,
        session: SessionId,
        output: String,
        events: Vec<DomainEvent>,
    ) -> Result<ActionOutcome, WorldError> {
        self.revision = self.revision.wrapping_add(1).max(1);
        if let Some(event) = events.first() {
            self.bump_view_revisions(event)?;
        }
        Ok(ActionOutcome {
            scene: self.scene_for(session)?,
            output: Some(output),
            changed_character: None,
            domain_events: events,
        })
    }

    fn promote_character(
        &mut self,
        session: SessionId,
        class_id: &str,
    ) -> Result<ActionOutcome, WorldError> {
        let checkpoint = self.checkpoint();
        match self.promote_character_inner(session, class_id) {
            Ok(outcome) => Ok(outcome),
            Err(error) => {
                self.restore_checkpoint(checkpoint);
                Err(error)
            }
        }
    }

    fn promote_character_inner(
        &mut self,
        session: SessionId,
        class_id: &str,
    ) -> Result<ActionOutcome, WorldError> {
        let character = self.character(session)?;
        let character_id = character.id;
        let character_name = character.name.clone();
        let room_id = room_checked(&self.entities, character.location)?
            .stable_id
            .clone();
        let previous = self
            .character_gameplay
            .get(&character_id)
            .cloned()
            .ok_or(WorldError::InvalidEntity)?;
        let from_class_id = previous.active_class_id.clone();
        let next = self.progression.promote(
            &previous,
            class_id,
            &self.world_progression_flags,
            &self.abilities,
        )?;
        let class_name = self.progression.class_view(class_id).label;
        self.character_gameplay.insert(character_id, next);
        let event = DomainEvent::Gameplay {
            event: GameplayEvent::CharacterPromoted {
                character_id,
                character_name,
                from_class_id,
                to_class_id: class_id.to_owned(),
                room_id,
            },
        };
        self.finish_gameplay_action(
            session,
            format!("You advance to the {class_name} class.\r\n"),
            vec![event],
        )
    }

    fn acquire_talent(
        &mut self,
        session: SessionId,
        graph_id: &str,
        node_id: &str,
    ) -> Result<ActionOutcome, WorldError> {
        let checkpoint = self.checkpoint();
        match self.acquire_talent_inner(session, graph_id, node_id) {
            Ok(outcome) => Ok(outcome),
            Err(error) => {
                self.restore_checkpoint(checkpoint);
                Err(error)
            }
        }
    }

    fn acquire_talent_inner(
        &mut self,
        session: SessionId,
        graph_id: &str,
        node_id: &str,
    ) -> Result<ActionOutcome, WorldError> {
        let character = self.character(session)?;
        let character_id = character.id;
        let character_name = character.name.clone();
        let room_id = room_checked(&self.entities, character.location)?
            .stable_id
            .clone();
        let previous = self
            .character_gameplay
            .get(&character_id)
            .cloned()
            .ok_or(WorldError::InvalidEntity)?;
        let next = self.progression.acquire_talent(
            &previous,
            graph_id,
            node_id,
            &self.world_progression_flags,
            &self.abilities,
        )?;
        let rank = next
            .talent_node_ranks
            .get(graph_id)
            .and_then(|nodes| nodes.get(node_id))
            .copied()
            .ok_or(WorldError::InvalidProgressionTransition)?;
        let node_name = self
            .progression
            .talent_views(&next, &self.world_progression_flags)
            .into_iter()
            .find(|graph| graph.id == graph_id)
            .and_then(|graph| graph.nodes.into_iter().find(|node| node.id == node_id))
            .map_or_else(|| humanize_id(node_id), |node| node.label);
        self.character_gameplay.insert(character_id, next);
        let event = DomainEvent::Gameplay {
            event: GameplayEvent::TalentRankAcquired {
                character_id,
                character_name,
                graph_id: graph_id.to_owned(),
                node_id: node_id.to_owned(),
                rank,
                room_id,
            },
        };
        self.finish_gameplay_action(
            session,
            format!("You acquire rank {rank} of {node_name}.\r\n"),
            vec![event],
        )
    }

    pub fn receive_progression_signal(
        &mut self,
        character_id: CharacterId,
        signal_id: &str,
    ) -> Result<bool, WorldError> {
        if !valid_definition_id(signal_id) {
            return Err(WorldError::InvalidProgressionTransition);
        }
        let state = self
            .character_gameplay
            .get_mut(&character_id)
            .ok_or(WorldError::InvalidEntity)?;
        if state.received_signals.contains(signal_id) {
            return Ok(false);
        }
        if state.received_signals.len() >= MAX_RECEIVED_SIGNALS {
            return Err(WorldError::ProgressionCapExceeded);
        }
        state.received_signals.insert(signal_id.to_owned());
        Ok(true)
    }

    pub fn set_world_progression_flag(
        &mut self,
        flag_id: &str,
        enabled: bool,
    ) -> Result<bool, WorldError> {
        if !valid_definition_id(flag_id) {
            return Err(WorldError::InvalidProgressionTransition);
        }
        let changed = if enabled {
            if self.world_progression_flags.len() >= 64
                && !self.world_progression_flags.contains(flag_id)
            {
                return Err(WorldError::ProgressionCapExceeded);
            }
            self.world_progression_flags.insert(flag_id.to_owned())
        } else {
            self.world_progression_flags.remove(flag_id)
        };
        Ok(changed)
    }

    fn move_item(
        &mut self,
        session: SessionId,
        query: &str,
        take: bool,
    ) -> Result<ActionOutcome, WorldError> {
        let scope = if take {
            ItemScope::CurrentRoom
        } else {
            ItemScope::Carried
        };
        let item_id = self.resolve_accessible_item(session, query, scope)?;
        let item_name = self
            .stable_items
            .get(&item_id)
            .ok_or(WorldError::InvalidEntity)?
            .name
            .clone();
        let room_id = room_checked(&self.entities, self.character(session)?.location)?
            .stable_id
            .clone();
        let character = self.character(session)?;
        let actor_id = character.id;
        let actor_name = character.name.clone();
        let destination = if take {
            SharedItemLocation::Carried(actor_id)
        } else {
            SharedItemLocation::Room(room_id.clone())
        };
        self.item_locations.insert(item_id.clone(), destination);
        self.revision = self.revision.wrapping_add(1).max(1);
        let event = if take {
            DomainEvent::ItemTaken {
                actor_id,
                actor_name,
                item_id,
                item_name: item_name.clone(),
                room_id,
            }
        } else {
            DomainEvent::ItemDropped {
                actor_id,
                actor_name,
                item_id,
                item_name: item_name.clone(),
                room_id,
            }
        };
        self.bump_view_revisions(&event)?;
        Ok(ActionOutcome {
            scene: self.scene_for(session)?,
            output: Some(if take {
                format!("You take the {item_name}.\r\n")
            } else {
                format!("You drop the {item_name}.\r\n")
            }),
            changed_character: None,
            domain_events: vec![event],
        })
    }

    fn resolve_accessible_item(
        &self,
        session: SessionId,
        query: &str,
        scope: ItemScope,
    ) -> Result<String, WorldError> {
        let character = self.character(session)?;
        let room_id = room_checked(&self.entities, character.location)?
            .stable_id
            .as_str();
        let candidates = self
            .item_locations
            .iter()
            .map(|(item_id, location)| {
                let accessible = match scope {
                    ItemScope::Carried => {
                        matches!(location, SharedItemLocation::Carried(owner) if *owner == character.id)
                    }
                    ItemScope::CurrentRoom => {
                        matches!(location, SharedItemLocation::Room(item_room) if item_room == room_id)
                    }
                    ItemScope::Accessible => {
                        matches!(location, SharedItemLocation::Carried(owner) if *owner == character.id)
                            || matches!(location, SharedItemLocation::Room(item_room) if item_room == room_id)
                    }
                };
                let visible = match location {
                    SharedItemLocation::Room(item_room) => self
                        .item_visible_to(session, item_id, item_room)?,
                    SharedItemLocation::Carried(owner) => *owner == character.id,
                    SharedItemLocation::Staged
                    | SharedItemLocation::Equipped(_, _)
                    | SharedItemLocation::HeldByNpc(_)
                    | SharedItemLocation::Consumed => false,
                };
                Ok((accessible && visible).then_some(item_id.as_str()))
            })
            .collect::<Result<Vec<_>, WorldError>>()?;
        resolve_item_query(
            &self.stable_items,
            candidates.into_iter().flatten(),
            query,
            self.observer(session)?.salt,
        )
    }

    fn resolve_visible_npc(&self, session: SessionId, query: &str) -> Result<String, WorldError> {
        let character = self.character(session)?;
        let room_id = room_checked(&self.entities, character.location)?
            .stable_id
            .as_str();
        let observer = self.observer(session)?;
        let query = query.trim().to_ascii_lowercase();
        let visible = self
            .npcs
            .values()
            .map(|npc| {
                let present = npc.alive() && npc.spawn_room_id == room_id;
                self.npc_visible_to(session, &npc.stable_id, room_id)
                    .map(|visible| (present && visible).then_some(npc))
            })
            .collect::<Result<Vec<_>, WorldError>>()?
            .into_iter()
            .flatten()
            .collect::<Vec<_>>();
        if let Some(npc) = visible
            .iter()
            .find(|npc| opaque_id(observer.salt, "npc", &npc.stable_id) == query)
        {
            return Ok(npc.stable_id.clone());
        }
        let exact = visible
            .iter()
            .filter(|npc| npc.name.eq_ignore_ascii_case(&query))
            .collect::<Vec<_>>();
        match exact.as_slice() {
            [npc] => return Ok(npc.stable_id.clone()),
            [_, ..] => return Err(WorldError::AmbiguousNpc),
            [] => {}
        }
        let prefix = visible
            .iter()
            .filter(|npc| {
                let name = npc.name.to_ascii_lowercase();
                name.starts_with(&query)
                    || name.split_whitespace().any(|word| word.starts_with(&query))
            })
            .collect::<Vec<_>>();
        match prefix.as_slice() {
            [npc] => Ok(npc.stable_id.clone()),
            [] => Err(WorldError::NpcNotFound),
            [_, ..] => Err(WorldError::AmbiguousNpc),
        }
    }

    fn item_visible_to(
        &self,
        session: SessionId,
        item_id: &str,
        room_id: &str,
    ) -> Result<bool, WorldError> {
        self.can_detect(
            session,
            room_id,
            self.hidden_items.get(item_id).copied().unwrap_or(0),
        )
    }

    fn npc_visible_to(
        &self,
        session: SessionId,
        npc_id: &str,
        room_id: &str,
    ) -> Result<bool, WorldError> {
        self.can_detect(
            session,
            room_id,
            self.hidden_npcs.get(npc_id).copied().unwrap_or(0),
        )
    }

    fn exit_visible_to(
        &self,
        session: SessionId,
        room_id: &str,
        direction: Direction,
    ) -> Result<bool, WorldError> {
        self.can_detect(
            session,
            room_id,
            self.secret_exits
                .get(&(room_id.to_owned(), direction))
                .copied()
                .unwrap_or(0),
        )
    }

    fn can_detect(
        &self,
        session: SessionId,
        room_id: &str,
        difficulty: u16,
    ) -> Result<bool, WorldError> {
        let observer = self.observer(session)?;
        if observer.role == AccountRole::Admin {
            return Ok(true);
        }
        let character = self.character(session)?;
        let gameplay = self
            .character_gameplay
            .get(&character.id)
            .ok_or(WorldError::InvalidEntity)?;
        let senses = perception_senses(gameplay);
        let tremorsense = senses.contains(&Sense::Tremorsense);
        let light = self
            .room_light
            .get(room_id)
            .copied()
            .unwrap_or(LightLevel::Bright);
        let usable_sense = tremorsense
            || match light {
                LightLevel::Bright | LightLevel::Dim => senses.contains(&Sense::Vision),
                LightLevel::Dark => senses.contains(&Sense::Darkvision),
            };
        if !usable_sense {
            return Ok(false);
        }
        if difficulty == 0 {
            return Ok(true);
        }
        let mut score = gameplay.skills.get("perception").copied().unwrap_or(0);
        if light == LightLevel::Dim && !tremorsense {
            score = score.saturating_sub(10);
        }
        Ok(score >= difficulty)
    }

    pub fn can_receive_visual_scene(&self, session: SessionId) -> Result<bool, WorldError> {
        if self.observer(session)?.role == AccountRole::Admin {
            return Ok(true);
        }
        let character = self.character(session)?;
        let room_id = &room_checked(&self.entities, character.location)?.stable_id;
        let gameplay = self
            .character_gameplay
            .get(&character.id)
            .ok_or(WorldError::InvalidEntity)?;
        let senses = perception_senses(gameplay);
        Ok(
            match self
                .room_light
                .get(room_id)
                .copied()
                .unwrap_or(LightLevel::Bright)
            {
                LightLevel::Bright | LightLevel::Dim => senses.contains(&Sense::Vision),
                LightLevel::Dark => senses.contains(&Sense::Darkvision),
            },
        )
    }

    fn can_receive_audio_scene(&self, session: SessionId) -> Result<bool, WorldError> {
        if self.observer(session)?.role == AccountRole::Admin {
            return Ok(true);
        }
        let character = self.character(session)?;
        let gameplay = self
            .character_gameplay
            .get(&character.id)
            .ok_or(WorldError::InvalidEntity)?;
        Ok(perception_senses(gameplay).contains(&Sense::Hearing))
    }

    pub fn connected_sessions(&self) -> Vec<SessionId> {
        let mut sessions = self.sessions.keys().copied().collect::<Vec<_>>();
        sessions.sort_by_key(|session| session.0);
        sessions
    }

    /// Server-internal presence data used to route social messages. Transports
    /// must project this into public names/text and never serialize the durable
    /// account or character identifiers directly.
    pub fn connected_characters(&self) -> Result<Vec<ConnectedCharacter>, WorldError> {
        self.connected_sessions()
            .into_iter()
            .map(|session_id| {
                let character = self.character(session_id)?;
                let room_id = room_checked(&self.entities, character.location)?
                    .stable_id
                    .clone();
                Ok(ConnectedCharacter {
                    session_id,
                    character_id: character.id,
                    account_id: character.account_id,
                    name: character.name.clone(),
                    room_id,
                })
            })
            .collect()
    }

    /// Server-internal connected members of the caller's durable party. The
    /// result includes the caller and never exposes party IDs to transports.
    pub fn connected_party_members(
        &self,
        session: SessionId,
    ) -> Result<Vec<ConnectedCharacter>, WorldError> {
        let character = self.character(session)?;
        let party_id = self
            .party_for_member(character.id)
            .ok_or(WorldError::PartyUnavailable)?;
        let party = self
            .adventure
            .parties
            .get(&party_id)
            .ok_or(WorldError::PartyUnavailable)?;
        Ok(self
            .connected_characters()?
            .into_iter()
            .filter(|candidate| party.members.contains(&candidate.character_id))
            .collect())
    }

    pub fn current_scene(&self, session: SessionId) -> Result<SceneView, WorldError> {
        self.scene_for(session)
    }

    /// Produce the complete browser/Telnet-neutral player projection. Durable
    /// account, character, room, item and party IDs never cross this boundary.
    pub fn player_state_view(&self, session: SessionId) -> Result<PlayerStateView, WorldError> {
        let character = self.character(session)?;
        let observer = self.observer(session)?;
        let current_room = room_checked(&self.entities, character.location)?;
        let gameplay = self
            .character_gameplay
            .get(&character.id)
            .ok_or(WorldError::InvalidEntity)?;

        let mut inventory = self
            .item_locations
            .iter()
            .filter_map(|(item_id, location)| {
                matches!(location, SharedItemLocation::Carried(owner) if *owner == character.id)
                    .then_some(item_id)
            })
            .map(|item_id| {
                let item = self
                    .stable_items
                    .get(item_id)
                    .ok_or(WorldError::InvalidEntity)?;
                let id = opaque_id(observer.salt, "item", item_id);
                let mut actions = vec![
                    SceneAction {
                        label: "Examine".into(),
                        command: format!("examine {id}"),
                    },
                    SceneAction {
                        label: "Drop".into(),
                        command: format!("drop {id}"),
                    },
                ];
                if let Some(definition) = self.item_gameplay.get(item_id) {
                    if definition.heal_amount != 0 || definition.effect.is_some() {
                        actions.push(SceneAction {
                            label: "Use".into(),
                            command: format!("use {id}"),
                        });
                    }
                    if definition.equipment_slot.is_some() {
                        actions.push(SceneAction {
                            label: "Equip".into(),
                            command: format!("equip {id}"),
                        });
                    }
                }
                Ok(PlayerItemView {
                    id,
                    label: item.name.clone(),
                    description: item.description.clone(),
                    actions,
                })
            })
            .collect::<Result<Vec<_>, WorldError>>()?;
        inventory.sort_by(|left, right| left.label.cmp(&right.label));

        let equipment = gameplay
            .equipment
            .iter()
            .map(|(slot, item_id)| {
                let item = self
                    .stable_items
                    .get(item_id)
                    .ok_or(WorldError::InvalidEntity)?;
                Ok(EquippedItemView {
                    id: opaque_id(observer.salt, "item", item_id),
                    slot: slot.as_str().into(),
                    label: item.name.clone(),
                    actions: vec![SceneAction {
                        label: "Unequip".into(),
                        command: format!("unequip {}", slot.as_str()),
                    }],
                })
            })
            .collect::<Result<Vec<_>, WorldError>>()?;

        let mut rooms = observer
            .discovered_rooms
            .iter()
            .map(|room_id| {
                let entity = self
                    .stable_rooms
                    .get(room_id)
                    .copied()
                    .ok_or(WorldError::InvalidEntity)?;
                let room = room_checked(&self.entities, entity)?;
                Ok(MapRoomView {
                    id: opaque_id(observer.salt, "room", room_id),
                    title: room.title.clone(),
                    current: room.stable_id == current_room.stable_id,
                })
            })
            .collect::<Result<Vec<_>, WorldError>>()?;
        rooms.sort_by(|left, right| left.title.cmp(&right.title));
        let mut paths = Vec::new();
        for room_id in &observer.discovered_rooms {
            let entity = self
                .stable_rooms
                .get(room_id)
                .copied()
                .ok_or(WorldError::InvalidEntity)?;
            let room = room_checked(&self.entities, entity)?;
            for (direction, destination) in &room.exits {
                if !self.exit_visible_to(session, room_id, *direction)? {
                    continue;
                }
                let destination = room_checked(&self.entities, *destination)?;
                paths.push(MapPathView {
                    from: opaque_id(observer.salt, "room", room_id),
                    direction: direction.as_str().into(),
                    to: observer
                        .discovered_rooms
                        .contains(&destination.stable_id)
                        .then(|| opaque_id(observer.salt, "room", &destination.stable_id)),
                });
            }
        }
        paths.sort_by(|left, right| {
            (&left.from, &left.direction, &left.to).cmp(&(&right.from, &right.direction, &right.to))
        });

        let (quest_status, quest_current, quest_target, quest_actions) =
            match gameplay.quests.get("first-hunt") {
                None => (
                    "available",
                    0,
                    1,
                    vec![SceneAction {
                        label: "Accept".into(),
                        command: "quest accept first-hunt".into(),
                    }],
                ),
                Some(progress) => {
                    let (status, actions) = match progress.status {
                        QuestStatus::Active => ("active", Vec::new()),
                        QuestStatus::ReadyToClaim => (
                            "readyToClaim",
                            vec![SceneAction {
                                label: "Claim reward".into(),
                                command: "quest complete first-hunt".into(),
                            }],
                        ),
                        QuestStatus::Completed => ("completed", Vec::new()),
                    };
                    (status, progress.current, progress.target, actions)
                }
            };
        let quests = vec![QuestView {
            id: "first-hunt".into(),
            label: "First Hunt".into(),
            summary: "Defeat one hostile creature; reward 25 coin and 50 experience.".into(),
            status: quest_status.into(),
            current: quest_current,
            target: quest_target,
            actions: quest_actions,
        }];

        let party = self.party_for_member(character.id).map(|party_id| {
            let party = &self.adventure.parties[&party_id];
            let is_leader = party.leader == character.id;
            let members = party
                .members
                .iter()
                .map(|member| PartyMemberView {
                    id: opaque_id(observer.salt, "character", &member.0.to_string()),
                    name: self
                        .connected_character_name(*member)
                        .unwrap_or_else(|| "Absent companion".into()),
                    leader: *member == party.leader,
                    online: self.session_for_character(*member).is_some(),
                })
                .collect();
            let mut actions = vec![SceneAction {
                label: "Leave party".into(),
                command: "party leave".into(),
            }];
            if is_leader {
                actions.push(SceneAction {
                    label: "Disband party".into(),
                    command: "party disband".into(),
                });
            }
            PartyView {
                leader: is_leader,
                members,
                actions,
            }
        });
        let mut party_invites = self
            .adventure
            .parties
            .values()
            .filter_map(|party| {
                let expiry = party.invitations.get(&character.id)?;
                (*expiry >= self.logical_tick).then_some((party.leader, *expiry))
            })
            .filter_map(|(leader, expiry)| {
                self.connected_character_name(leader)
                    .map(|name| (name, expiry))
            })
            .map(|(leader_name, expiry)| PartyInviteView {
                actions: vec![SceneAction {
                    label: "Accept invitation".into(),
                    command: format!("party accept {leader_name}"),
                }],
                leader_name,
                expires_in_ticks: expiry.saturating_sub(self.logical_tick),
            })
            .collect::<Vec<_>>();
        party_invites.sort_by(|left, right| left.leader_name.cmp(&right.leader_name));

        let commodities = commodity_specs()
            .iter()
            .filter_map(|(id, label, _, _)| {
                let quantity = gameplay.commodities.get(*id).copied().unwrap_or(0);
                (quantity > 0).then(|| CommodityView {
                    id: (*id).into(),
                    label: (*label).into(),
                    quantity,
                })
            })
            .collect();
        let shop_available = current_room.stable_id == self.start_room_stable_id;
        let shop = if shop_available {
            commodity_specs()
                .iter()
                .map(|(id, label, buy_price, sell_price)| {
                    let stock = self.adventure.shop_stock.get(*id).copied().unwrap_or(0);
                    let owned = gameplay.commodities.get(*id).copied().unwrap_or(0);
                    let mut actions = Vec::new();
                    if stock > 0 && gameplay.coins >= *buy_price && owned < 99 {
                        actions.push(SceneAction {
                            label: "Buy".into(),
                            command: format!("buy {id}"),
                        });
                    }
                    if owned > 0 {
                        actions.push(SceneAction {
                            label: "Sell".into(),
                            command: format!("sell {id}"),
                        });
                    }
                    ShopItemView {
                        id: (*id).into(),
                        label: (*label).into(),
                        buy_price: *buy_price,
                        sell_price: *sell_price,
                        stock,
                        owned,
                        actions,
                    }
                })
                .collect()
        } else {
            Vec::new()
        };

        let mut online = self
            .connected_characters()?
            .into_iter()
            .map(|connected| OnlineCharacterView {
                id: opaque_id(
                    observer.salt,
                    "character",
                    &connected.character_id.0.to_string(),
                ),
                name: connected.name,
                self_character: connected.character_id == character.id,
            })
            .collect::<Vec<_>>();
        online.sort_by(|left, right| {
            left.name
                .to_ascii_lowercase()
                .cmp(&right.name.to_ascii_lowercase())
        });

        Ok(PlayerStateView {
            revision: observer.revision,
            character: PlayerCharacterView {
                name: character.name.clone(),
                lineage: self.progression.lineage_view(&gameplay.lineage_id),
                background: gameplay
                    .background_id
                    .as_ref()
                    .map(|id| self.progression.background_view(id)),
                base_class: self.progression.class_view(&gameplay.base_class_id),
                active_class: self.progression.class_view(&gameplay.active_class_id),
                class_history: gameplay
                    .class_history
                    .iter()
                    .map(|id| self.progression.class_view(id))
                    .collect(),
                level: gameplay.level,
                experience: gameplay.experience,
                next_level_experience: self.progression.next_level_experience(gameplay.level),
                health: gameplay.vitals.health,
                max_health: gameplay.vitals.max_health,
                strength: gameplay.attributes.strength,
                defence: gameplay.attributes.defence,
                attributes: std::iter::once(AttributeView {
                    id: "strength".into(),
                    label: "Strength".into(),
                    value: gameplay.attributes.strength,
                })
                .chain(std::iter::once(AttributeView {
                    id: "defence".into(),
                    label: "Defence".into(),
                    value: gameplay.attributes.defence,
                }))
                .chain(
                    gameplay
                        .attributes
                        .additional
                        .iter()
                        .map(|(id, value)| AttributeView {
                            id: id.clone(),
                            label: humanize_id(id),
                            value: *value,
                        }),
                )
                .collect(),
                skills: gameplay
                    .skills
                    .iter()
                    .map(|(id, rank)| SkillView {
                        id: id.clone(),
                        label: skill_label(id),
                        rank: *rank,
                    })
                    .collect(),
                resources: gameplay
                    .resources
                    .iter()
                    .map(|(id, pool)| ResourceView {
                        id: id.clone(),
                        label: humanize_id(id),
                        current: pool.current,
                        max: pool.max,
                    })
                    .collect(),
                abilities: gameplay
                    .ability_ids
                    .iter()
                    .filter_map(|id| self.abilities.get(id))
                    .map(|ability| {
                        let cooldown_remaining = gameplay
                            .ability_cooldowns
                            .get(&ability.id)
                            .copied()
                            .unwrap_or(0)
                            .saturating_sub(self.logical_tick)
                            .max(
                                gameplay
                                    .ability_global_ready_tick
                                    .saturating_sub(self.logical_tick),
                            );
                        let has_resources = ability.resource_costs.iter().all(|(id, cost)| {
                            gameplay
                                .resources
                                .get(id)
                                .is_some_and(|pool| pool.current >= *cost)
                        });
                        AbilityView {
                            id: ability.id.clone(),
                            name: ability.name.clone(),
                            description: ability.description.clone(),
                            kind: ability.category.as_str().into(),
                            resource_costs: ability
                                .resource_costs
                                .iter()
                                .map(|(id, amount)| AbilityResourceCostView {
                                    resource_id: id.clone(),
                                    label: humanize_id(id),
                                    amount: *amount,
                                })
                                .collect(),
                            cooldown_remaining,
                            usable: cooldown_remaining == 0
                                && has_resources
                                && gameplay.recovery_at_tick.is_none()
                                && !gameplay
                                    .statuses
                                    .iter()
                                    .any(|status| status.kind == ActiveStatusKind::Stun),
                            targeting: ability.targeting.as_str().into(),
                        }
                    })
                    .collect(),
                resistances: gameplay
                    .resistances
                    .iter()
                    .map(|(damage_type, percent)| ResistanceView {
                        damage_type: *damage_type,
                        percent: *percent,
                    })
                    .collect(),
                statuses: gameplay
                    .statuses
                    .iter()
                    .map(|status| StatusView {
                        id: status.source_ability_id.clone(),
                        label: self
                            .abilities
                            .get(&status.source_ability_id)
                            .map(|ability| ability.name.clone())
                            .unwrap_or_else(|| humanize_id(&status.source_ability_id)),
                        kind: status.kind,
                        magnitude: status.magnitude,
                        remaining_ticks: status.remaining_ticks,
                    })
                    .collect(),
                progression_points: gameplay.progression_points.clone(),
                talents: self
                    .progression
                    .talent_views(gameplay, &self.world_progression_flags),
                promotions: self
                    .progression
                    .promotions(gameplay, &self.world_progression_flags),
                effects: gameplay.effects.clone(),
                recovery_in_ticks: gameplay
                    .recovery_at_tick
                    .map(|tick| tick.saturating_sub(self.logical_tick)),
            },
            inventory,
            equipment,
            map: DiscoveredMapView {
                current_room_id: opaque_id(observer.salt, "room", &current_room.stable_id),
                rooms,
                paths,
            },
            quests,
            party,
            party_invites,
            economy: EconomyView {
                coins: gameplay.coins,
                commodities,
                shop_available,
                shop,
            },
            social: SocialView { online },
        })
    }

    /// Projects an authoritative event according to one observer's current
    /// perception. Domain IDs are replaced with observer-specific opaque IDs.
    pub fn project_event(
        &self,
        session: SessionId,
        event: &DomainEvent,
    ) -> Result<Option<ViewEvent>, WorldError> {
        // The clock changes observer-visible state (weather countdowns, effects,
        // recovery timers), but is not itself player-facing narration.
        if matches!(
            event,
            DomainEvent::Gameplay {
                event: GameplayEvent::TickAdvanced { .. }
            }
        ) {
            return Ok(None);
        }
        if !self.perceives_event(session, event)? {
            return Ok(None);
        }
        let observer = self.observer(session)?;
        let viewer = self.character(session)?;
        let (kind, actor_id, target_id, text, item_id) = match event {
            DomainEvent::CharacterMoved {
                actor_id,
                actor_name,
                direction,
                from_room_id,
                ..
            } => {
                let text = if viewer.id == *actor_id {
                    format!("You go {}.", direction.as_str())
                } else {
                    let viewer_room = room_checked(&self.entities, viewer.location)?
                        .stable_id
                        .as_str();
                    if viewer_room == from_room_id {
                        format!("{actor_name} leaves {}.", direction.as_str())
                    } else {
                        format!("{actor_name} arrives.")
                    }
                };
                (
                    ViewEventKind::CharacterMoved,
                    opaque_id(observer.salt, "character", &actor_id.0.to_string()),
                    None,
                    text,
                    None,
                )
            }
            DomainEvent::ItemTaken {
                actor_id,
                actor_name,
                item_id,
                item_name,
                ..
            } => {
                let text = if viewer.id == *actor_id {
                    format!("You take the {item_name}.")
                } else {
                    format!("{actor_name} takes the {item_name}.")
                };
                (
                    ViewEventKind::ItemTaken,
                    opaque_id(observer.salt, "character", &actor_id.0.to_string()),
                    None,
                    text,
                    Some(opaque_id(observer.salt, "item", item_id)),
                )
            }
            DomainEvent::Gameplay { event } => {
                let (actor_id, target_id, text, item_id) =
                    project_gameplay_event(observer.salt, viewer.id, event);
                (ViewEventKind::Gameplay, actor_id, target_id, text, item_id)
            }
            DomainEvent::Adventure { event } => match event {
                AdventureEvent::PartyChanged {
                    character_id,
                    party_id,
                    summary,
                    ..
                } => (
                    ViewEventKind::Adventure,
                    opaque_id(observer.salt, "character", &character_id.0.to_string()),
                    Some(opaque_id(observer.salt, "party", &party_id.to_string())),
                    summary.clone(),
                    None,
                ),
                AdventureEvent::QuestChanged {
                    character_id,
                    summary,
                    ..
                } => (
                    ViewEventKind::Adventure,
                    opaque_id(observer.salt, "character", &character_id.0.to_string()),
                    None,
                    summary.clone(),
                    None,
                ),
                AdventureEvent::EconomyChanged {
                    character_id,
                    commodity_id,
                    summary,
                } => (
                    ViewEventKind::Adventure,
                    opaque_id(observer.salt, "character", &character_id.0.to_string()),
                    None,
                    summary.clone(),
                    commodity_id
                        .as_ref()
                        .map(|id| opaque_id(observer.salt, "commodity", id)),
                ),
                AdventureEvent::WeatherChanged { weather, .. } => (
                    ViewEventKind::Adventure,
                    opaque_id(observer.salt, "world", "weather"),
                    None,
                    format!("The weather turns {}.", weather_label(*weather)),
                    None,
                ),
            },
            DomainEvent::ItemDropped {
                actor_id,
                actor_name,
                item_id,
                item_name,
                ..
            } => {
                let text = if viewer.id == *actor_id {
                    format!("You drop the {item_name}.")
                } else {
                    format!("{actor_name} drops the {item_name}.")
                };
                (
                    ViewEventKind::ItemDropped,
                    opaque_id(observer.salt, "character", &actor_id.0.to_string()),
                    None,
                    text,
                    Some(opaque_id(observer.salt, "item", item_id)),
                )
            }
        };
        Ok(Some(ViewEvent {
            revision: observer.revision,
            kind,
            text,
            actor_id,
            target_id,
            item_id,
        }))
    }

    /// Build a perception-safe narration event addressed to exactly one
    /// authorized session. The caller must keep the resulting event on that
    /// session's targeted delivery; it is not a domain event and must never be
    /// broadcast or persisted.
    pub fn project_private_narration(
        &self,
        session: SessionId,
        text: &str,
    ) -> Result<ViewEvent, WorldError> {
        let text = text.trim();
        if text.is_empty()
            || text.len() > 1_024
            || text
                .chars()
                .any(|character| character.is_control() && character != '\n')
        {
            return Err(WorldError::InvalidScriptEffect);
        }
        let observer = self.observer(session)?;
        let viewer = self.character(session)?;
        Ok(ViewEvent {
            revision: observer.revision,
            kind: ViewEventKind::Gameplay,
            text: text.to_owned(),
            actor_id: opaque_id(observer.salt, "character", &viewer.id.0.to_string()),
            target_id: None,
            item_id: None,
        })
    }

    fn bump_view_revisions(&mut self, event: &DomainEvent) -> Result<(), WorldError> {
        let affected = self
            .connected_sessions()
            .into_iter()
            .map(|session| Ok((session, self.perceives_event(session, event)?)))
            .collect::<Result<Vec<_>, WorldError>>()?;
        let mut previous = Vec::new();
        for (session, perceived) in affected {
            if perceived {
                let view = self
                    .observer_views
                    .get_mut(&session)
                    .ok_or(WorldError::InvalidEntity)?;
                previous.push((session, view.revision));
                view.revision = view.revision.wrapping_add(1).max(1);
            }
        }
        self.last_view_bump = Some((self.revision, previous));
        Ok(())
    }

    fn bump_view_revisions_for_events(&mut self, events: &[DomainEvent]) -> Result<(), WorldError> {
        let affected = self
            .connected_sessions()
            .into_iter()
            .map(|session| {
                for event in events {
                    if self.perceives_event(session, event)? {
                        return Ok((session, true));
                    }
                }
                Ok((session, false))
            })
            .collect::<Result<Vec<_>, WorldError>>()?;
        let mut previous = Vec::new();
        for (session, perceived) in affected {
            if perceived {
                let view = self
                    .observer_views
                    .get_mut(&session)
                    .ok_or(WorldError::InvalidEntity)?;
                previous.push((session, view.revision));
                view.revision = view.revision.wrapping_add(1).max(1);
            }
        }
        self.last_view_bump = Some((self.revision, previous));
        Ok(())
    }

    fn restore_last_view_bump(&mut self) {
        let matches_current_revision = self
            .last_view_bump
            .as_ref()
            .is_some_and(|(resulting_revision, _)| *resulting_revision == self.revision);
        if !matches_current_revision {
            return;
        }
        if let Some((_, previous_views)) = self.last_view_bump.take() {
            for (affected_session, previous_revision) in previous_views {
                if let Some(view) = self.observer_views.get_mut(&affected_session) {
                    view.revision = previous_revision;
                }
            }
        }
    }

    fn validate_persisted_item_event(
        &self,
        actor_id: CharacterId,
        actor_name: &str,
        item_id: &str,
        item_name: &str,
        room_id: &str,
    ) -> Result<(), WorldError> {
        let valid_actor = actor_id.0 > 0 && !actor_name.trim().is_empty();
        let valid_item = self
            .stable_items
            .get(item_id)
            .is_some_and(|item| item.name == item_name && !item_name.trim().is_empty());
        let valid_room = self.stable_rooms.contains_key(room_id);
        if valid_actor && valid_item && valid_room {
            Ok(())
        } else {
            Err(WorldError::InvalidDomainEvent)
        }
    }

    fn perceives_event(&self, session: SessionId, event: &DomainEvent) -> Result<bool, WorldError> {
        let viewer = self.character(session)?;
        let viewer_room = room_checked(&self.entities, viewer.location)?
            .stable_id
            .as_str();
        let perceived = match event {
            DomainEvent::CharacterMoved {
                actor_id,
                from_room_id,
                to_room_id,
                direction,
                ..
            } => {
                viewer.id == *actor_id
                    || viewer_room == to_room_id
                    || (viewer_room == from_room_id
                        && self.exit_visible_to(session, from_room_id, *direction)?)
            }
            DomainEvent::ItemTaken {
                actor_id,
                item_id,
                room_id,
                ..
            }
            | DomainEvent::ItemDropped {
                actor_id,
                item_id,
                room_id,
                ..
            } => {
                viewer.id == *actor_id
                    || (viewer_room == room_id
                        && self.item_visible_to(session, item_id, room_id)?)
            }
            DomainEvent::Gameplay { event } => {
                let base = matches!(event, GameplayEvent::TickAdvanced { .. })
                    || (gameplay_event_character(event).is_some_and(|id| id == viewer.id)
                        || gameplay_event_room(event).is_some_and(|room| room == viewer_room));
                let npc_visible = match (gameplay_event_npc(event), gameplay_event_room(event)) {
                    (Some(npc_id), Some(room_id))
                        if gameplay_event_character(event) != Some(viewer.id) =>
                    {
                        self.npc_visible_to(session, npc_id, room_id)?
                    }
                    _ => true,
                };
                let item_visible = match (gameplay_event_item(event), gameplay_event_room(event)) {
                    (Some(item_id), Some(room_id))
                        if gameplay_event_character(event) != Some(viewer.id) =>
                    {
                        self.item_visible_to(session, item_id, room_id)?
                    }
                    _ => true,
                };
                base && npc_visible && item_visible
            }
            DomainEvent::Adventure { event } => match event {
                AdventureEvent::PartyChanged {
                    character_id,
                    audience,
                    ..
                } => *character_id == viewer.id || audience.contains(&viewer.id),
                AdventureEvent::QuestChanged { character_id, .. }
                | AdventureEvent::EconomyChanged { character_id, .. } => *character_id == viewer.id,
                AdventureEvent::WeatherChanged { .. } => true,
            },
        };
        Ok(perceived)
    }

    fn observer(&self, session: SessionId) -> Result<&ObserverView, WorldError> {
        self.observer_views
            .get(&session)
            .ok_or(WorldError::UnknownSession)
    }

    fn session_for_character(&self, character_id: CharacterId) -> Option<SessionId> {
        self.sessions.iter().find_map(|(session, entity_id)| {
            matches!(
                self.entities.get(*entity_id),
                Some(Entity::Character(character)) if character.id == character_id
            )
            .then_some(*session)
        })
    }

    fn character(&self, session: SessionId) -> Result<&Character, WorldError> {
        let character_id = *self
            .sessions
            .get(&session)
            .ok_or(WorldError::UnknownSession)?;
        match self.entities.get(character_id) {
            Some(Entity::Character(character)) => Ok(character),
            _ => Err(WorldError::InvalidEntity),
        }
    }
}

#[derive(Clone, Copy)]
enum ItemScope {
    Carried,
    CurrentRoom,
    Accessible,
}

fn quest_matches(query: &str) -> bool {
    matches!(
        query.trim().to_ascii_lowercase().as_str(),
        "first-hunt" | "first hunt" | "hunt"
    )
}

fn commodity_specs() -> [(&'static str, &'static str, u64, u64); 2] {
    [
        ("tonic", "healing tonic", 12, 6),
        ("trail-ration", "trail ration", 8, 4),
    ]
}

fn resolve_commodity(query: &str) -> Result<(&'static str, &'static str, u64, u64), WorldError> {
    let query = query.trim().to_ascii_lowercase();
    commodity_specs()
        .into_iter()
        .find(|(id, label, _, _)| {
            *id == query || label.eq_ignore_ascii_case(&query) || label.starts_with(&query)
        })
        .ok_or(WorldError::CommodityNotFound)
}

fn weather_label(weather: WeatherKind) -> &'static str {
    match weather {
        WeatherKind::Clear => "clear",
        WeatherKind::Rain => "rainy",
        WeatherKind::Wind => "windy",
        WeatherKind::Fog => "foggy",
    }
}

fn skill_label(id: &str) -> String {
    id.split(['-', '_'])
        .filter(|part| !part.is_empty())
        .map(|part| {
            let mut characters = part.chars();
            characters.next().map_or_else(String::new, |first| {
                first.to_uppercase().collect::<String>() + characters.as_str()
            })
        })
        .collect::<Vec<_>>()
        .join(" ")
}

fn validate_ability_definition(ability: &AbilityDefinition) -> Result<(), WorldError> {
    let scaling_valid = |attribute: &Option<String>, percent: u16| {
        percent <= 1_000
            && attribute.is_some() == (percent > 0)
            && attribute.as_deref().is_none_or(valid_stat_id)
    };
    let status_valid = |status: &AbilityStatusEffectDefinition| match status {
        AbilityStatusEffectDefinition::AttributeModifier {
            attribute,
            magnitude,
            duration_ticks,
            ..
        } => {
            valid_stat_id(attribute)
                && attribute != "max_health"
                && *magnitude != 0
                && magnitude.unsigned_abs() <= MAX_STATUS_MAGNITUDE as u32
                && (1..=MAX_STATUS_TICKS).contains(duration_ticks)
        }
        AbilityStatusEffectDefinition::Regeneration {
            magnitude,
            duration_ticks,
            ..
        }
        | AbilityStatusEffectDefinition::DamageOverTime {
            magnitude,
            duration_ticks,
            ..
        } => {
            (1..=MAX_STATUS_MAGNITUDE as u32).contains(magnitude)
                && (1..=MAX_STATUS_TICKS).contains(duration_ticks)
        }
        AbilityStatusEffectDefinition::Stun { duration_ticks, .. } => {
            (1..=MAX_STATUS_TICKS).contains(duration_ticks)
        }
    };
    let effect_target_valid = |target: AbilityEffectTarget| {
        !(ability.targeting == AbilityTargeting::SelfOnly && target == AbilityEffectTarget::Target)
    };
    let target_semantics_valid = ability.damage.as_ref().is_none_or(|damage| {
        effect_target_valid(damage.target)
            && !(damage.target == AbilityEffectTarget::Target
                && ability.targeting == AbilityTargeting::SingleAlly)
    }) && ability.heal.as_ref().is_none_or(|heal| {
        effect_target_valid(heal.target)
            && !(heal.target == AbilityEffectTarget::Target
                && ability.targeting == AbilityTargeting::SingleEnemy)
    }) && ability
        .status_effects
        .iter()
        .all(|status| effect_target_valid(status_target(status)));
    let valid = valid_definition_id(&ability.id)
        && safe_definition_text(&ability.name, 128)
        && safe_definition_text(&ability.description, 2_048)
        && ability.resource_costs.len() <= MAX_ABILITY_RESOURCE_COSTS
        && ability
            .resource_costs
            .iter()
            .all(|(id, amount)| valid_stat_id(id) && *amount > 0 && *amount <= MAX_RESOURCE_VALUE)
        && ability.cooldown_ticks <= MAX_COOLDOWN_TICKS
        && ability.status_effects.len() <= MAX_ABILITY_EFFECTS
        && target_semantics_valid
        && (ability.damage.is_some()
            || ability.heal.is_some()
            || !ability.status_effects.is_empty())
        && ability.damage.as_ref().is_none_or(|damage| {
            (1..=1_000_000).contains(&damage.base_amount)
                && scaling_valid(&damage.scaling_attribute, damage.scaling_percent)
        })
        && ability.heal.as_ref().is_none_or(|heal| {
            (1..=1_000_000).contains(&heal.base_amount)
                && scaling_valid(&heal.scaling_attribute, heal.scaling_percent)
        })
        && ability.status_effects.iter().all(status_valid);
    valid
        .then_some(())
        .ok_or(WorldError::InvalidAbilityDefinition)
}

fn safe_definition_text(value: &str, maximum_bytes: usize) -> bool {
    let trimmed = value.trim();
    !trimmed.is_empty()
        && trimmed.len() <= maximum_bytes
        && !trimmed
            .chars()
            .any(|character| character.is_control() || character == '\u{7f}')
}

fn validate_active_status(
    status: &ActiveStatusEffect,
    abilities: &BTreeMap<String, AbilityDefinition>,
) -> bool {
    validate_active_status_shape(status)
        && abilities
            .get(&status.source_ability_id)
            .is_some_and(|ability| {
                ability
                    .status_effects
                    .iter()
                    .any(|effect| active_status_matches_definition(status, effect))
            })
}

fn validate_active_status_shape(status: &ActiveStatusEffect) -> bool {
    let attributed = status.source_character_id.is_some() ^ status.source_npc_id.is_some();
    let valid_source = valid_definition_id(&status.source_ability_id)
        && status.source_character_id.is_none_or(|id| id.0 > 0)
        && status
            .source_npc_id
            .as_deref()
            .is_none_or(valid_definition_id);
    let valid_shape = match status.kind {
        ActiveStatusKind::AttributeModifier => {
            status.magnitude != 0
                && status.magnitude.unsigned_abs() <= MAX_STATUS_MAGNITUDE as u32
                && status.attribute.as_deref().is_some_and(valid_stat_id)
                && status.damage_type.is_none()
        }
        ActiveStatusKind::Regeneration => {
            (1..=MAX_STATUS_MAGNITUDE).contains(&status.magnitude)
                && status.attribute.is_none()
                && status.damage_type.is_none()
        }
        ActiveStatusKind::DamageOverTime => {
            (1..=MAX_STATUS_MAGNITUDE).contains(&status.magnitude)
                && status.attribute.is_none()
                && status.damage_type.is_some()
        }
        ActiveStatusKind::Stun => {
            status.magnitude == 0 && status.attribute.is_none() && status.damage_type.is_none()
        }
    };
    attributed
        && valid_source
        && valid_shape
        && (1..=MAX_STATUS_TICKS).contains(&status.remaining_ticks)
}

fn active_status_matches_definition(
    status: &ActiveStatusEffect,
    definition: &AbilityStatusEffectDefinition,
) -> bool {
    match definition {
        AbilityStatusEffectDefinition::AttributeModifier {
            attribute,
            magnitude,
            duration_ticks,
            ..
        } => {
            status.kind == ActiveStatusKind::AttributeModifier
                && status.magnitude == *magnitude
                && status.attribute.as_deref() == Some(attribute.as_str())
                && status.damage_type.is_none()
                && status.remaining_ticks <= *duration_ticks
        }
        AbilityStatusEffectDefinition::Regeneration {
            magnitude,
            duration_ticks,
            ..
        } => {
            status.kind == ActiveStatusKind::Regeneration
                && status.magnitude == i32::try_from(*magnitude).unwrap_or(i32::MAX)
                && status.attribute.is_none()
                && status.damage_type.is_none()
                && status.remaining_ticks <= *duration_ticks
        }
        AbilityStatusEffectDefinition::DamageOverTime {
            damage_type,
            magnitude,
            duration_ticks,
            ..
        } => {
            status.kind == ActiveStatusKind::DamageOverTime
                && status.magnitude == i32::try_from(*magnitude).unwrap_or(i32::MAX)
                && status.attribute.is_none()
                && status.damage_type == Some(*damage_type)
                && status.remaining_ticks <= *duration_ticks
        }
        AbilityStatusEffectDefinition::Stun { duration_ticks, .. } => {
            status.kind == ActiveStatusKind::Stun
                && status.magnitude == 0
                && status.attribute.is_none()
                && status.damage_type.is_none()
                && status.remaining_ticks <= *duration_ticks
        }
    }
}

fn status_target(status: &AbilityStatusEffectDefinition) -> AbilityEffectTarget {
    match status {
        AbilityStatusEffectDefinition::AttributeModifier { target, .. }
        | AbilityStatusEffectDefinition::Regeneration { target, .. }
        | AbilityStatusEffectDefinition::DamageOverTime { target, .. }
        | AbilityStatusEffectDefinition::Stun { target, .. } => *target,
    }
}

fn npc_effect_targets_enemy(ability: &AbilityDefinition, target: AbilityEffectTarget) -> bool {
    ability.targeting == AbilityTargeting::SingleEnemy && target == AbilityEffectTarget::Target
}

fn npc_enemy_status_count(ability: &AbilityDefinition) -> usize {
    ability
        .status_effects
        .iter()
        .filter(|status| npc_effect_targets_enemy(ability, status_target(status)))
        .count()
}

fn npc_self_status_count(ability: &AbilityDefinition) -> usize {
    ability.status_effects.len() - npc_enemy_status_count(ability)
}

fn npc_ability_score(ability: &AbilityDefinition, npc: &Npc) -> i64 {
    let mut score = 0_i64;
    if let Some(damage) = &ability.damage
        && npc_effect_targets_enemy(ability, damage.target)
    {
        score =
            score
                .saturating_add(1_000_000)
                .saturating_add(i64::from(ability_scaled_amount_npc(
                    damage.base_amount,
                    damage.scaling_attribute.as_deref(),
                    damage.scaling_percent,
                    npc,
                )));
    }
    if let Some(heal) = &ability.heal
        && !npc_effect_targets_enemy(ability, heal.target)
        && npc.health < npc.max_health
    {
        let amount = ability_scaled_amount_npc(
            heal.base_amount,
            heal.scaling_attribute.as_deref(),
            heal.scaling_percent,
            npc,
        );
        score = score.saturating_add(500_000).saturating_add(i64::from(
            amount.min(npc.max_health.saturating_sub(npc.health)),
        ));
    }
    for status in &ability.status_effects {
        if npc_effect_targets_enemy(ability, status_target(status)) {
            let value = match status {
                AbilityStatusEffectDefinition::AttributeModifier {
                    magnitude,
                    duration_ticks,
                    ..
                } => i64::from(magnitude.unsigned_abs()) * i64::from(*duration_ticks),
                AbilityStatusEffectDefinition::Regeneration {
                    magnitude,
                    duration_ticks,
                    ..
                }
                | AbilityStatusEffectDefinition::DamageOverTime {
                    magnitude,
                    duration_ticks,
                    ..
                } => i64::from(*magnitude) * i64::from(*duration_ticks),
                AbilityStatusEffectDefinition::Stun { duration_ticks, .. } => {
                    100_000 * i64::from(*duration_ticks)
                }
            };
            score = score.saturating_add(250_000).saturating_add(value);
        } else if !npc
            .statuses
            .iter()
            .any(|active| active.source_ability_id == ability.id)
        {
            score = score.saturating_add(100_000);
        }
    }
    score
}

fn active_status(
    source_ability_id: &str,
    source_character_id: Option<CharacterId>,
    source_npc_id: Option<&str>,
    status: &AbilityStatusEffectDefinition,
) -> ActiveStatusEffect {
    let (kind, magnitude, remaining_ticks, attribute, damage_type) = match status {
        AbilityStatusEffectDefinition::AttributeModifier {
            attribute,
            magnitude,
            duration_ticks,
            ..
        } => (
            ActiveStatusKind::AttributeModifier,
            *magnitude,
            *duration_ticks,
            Some(attribute.clone()),
            None,
        ),
        AbilityStatusEffectDefinition::Regeneration {
            magnitude,
            duration_ticks,
            ..
        } => (
            ActiveStatusKind::Regeneration,
            i32::try_from(*magnitude).unwrap_or(i32::MAX),
            *duration_ticks,
            None,
            None,
        ),
        AbilityStatusEffectDefinition::DamageOverTime {
            damage_type,
            magnitude,
            duration_ticks,
            ..
        } => (
            ActiveStatusKind::DamageOverTime,
            i32::try_from(*magnitude).unwrap_or(i32::MAX),
            *duration_ticks,
            None,
            Some(*damage_type),
        ),
        AbilityStatusEffectDefinition::Stun { duration_ticks, .. } => {
            (ActiveStatusKind::Stun, 0, *duration_ticks, None, None)
        }
    };
    ActiveStatusEffect {
        source_ability_id: source_ability_id.to_owned(),
        source_character_id,
        source_npc_id: source_npc_id.map(str::to_owned),
        kind,
        magnitude,
        remaining_ticks,
        attribute,
        damage_type,
    }
}

fn attribute_value(attributes: &Attributes, id: &str) -> i32 {
    match id {
        "strength" => attributes.strength,
        "defence" => attributes.defence,
        _ => attributes.additional.get(id).copied().unwrap_or(0),
    }
}

pub(crate) fn permanent_gameplay_attribute(gameplay: &CharacterGameplaySnapshot, id: &str) -> i32 {
    match id {
        "max_health" => gameplay.vitals.max_health,
        _ => attribute_value(&gameplay.attributes, id),
    }
}

fn ability_scaled_amount(
    base: u32,
    scaling_attribute: Option<&str>,
    scaling_percent: u16,
    gameplay: &CharacterGameplaySnapshot,
) -> i32 {
    let scaling = scaling_attribute
        .map(|id| {
            let value = if id == "max_health" {
                gameplay.vitals.max_health
            } else {
                effective_gameplay_attribute(gameplay, id)
            };
            i64::from(value.max(0))
        })
        .unwrap_or(0)
        .saturating_mul(i64::from(scaling_percent))
        / 100;
    i64::from(base)
        .saturating_add(scaling)
        .clamp(0, i64::from(i32::MAX)) as i32
}

fn ability_scaled_amount_npc(
    base: u32,
    scaling_attribute: Option<&str>,
    scaling_percent: u16,
    npc: &Npc,
) -> i32 {
    let scaling = scaling_attribute
        .map(|id| {
            let value = if id == "max_health" {
                npc.max_health
            } else {
                effective_npc_attribute(npc, id)
            };
            i64::from(value.max(0))
        })
        .unwrap_or(0)
        .saturating_mul(i64::from(scaling_percent))
        / 100;
    i64::from(base)
        .saturating_add(scaling)
        .clamp(0, i64::from(i32::MAX)) as i32
}

fn effective_gameplay_attribute(gameplay: &CharacterGameplaySnapshot, id: &str) -> i32 {
    gameplay
        .statuses
        .iter()
        .filter(|status| {
            status.kind == ActiveStatusKind::AttributeModifier
                && status.attribute.as_deref() == Some(id)
        })
        .fold(
            attribute_value(&gameplay.attributes, id),
            |value, status| value.saturating_add(status.magnitude),
        )
}

fn effective_npc_attribute(npc: &Npc, id: &str) -> i32 {
    let base = match id {
        "strength" => npc.strength,
        "defence" => npc.defence,
        _ => 0,
    };
    npc.statuses
        .iter()
        .filter(|status| {
            status.kind == ActiveStatusKind::AttributeModifier
                && status.attribute.as_deref() == Some(id)
        })
        .fold(base, |value, status| value.saturating_add(status.magnitude))
}

fn resisted_amount(amount: i32, resistance: u16) -> i32 {
    if amount <= 0 {
        return 0;
    }
    let retained = 100_i64.saturating_sub(i64::from(resistance));
    (i64::from(amount).saturating_mul(retained) / 100).clamp(1, i64::from(i32::MAX)) as i32
}

fn resolve_item_query<'a>(
    items: &HashMap<String, Item>,
    candidates: impl Iterator<Item = &'a str>,
    query: &str,
    observer_salt: u64,
) -> Result<String, WorldError> {
    let query = query.trim().to_ascii_lowercase();
    let candidate_items = candidates
        .map(|item_id| items.get(item_id).ok_or(WorldError::InvalidEntity))
        .collect::<Result<Vec<_>, _>>()?;

    if let Some(item) = candidate_items
        .iter()
        .find(|item| opaque_id(observer_salt, "item", &item.stable_id) == query)
    {
        return Ok(item.stable_id.clone());
    }

    let exact = candidate_items
        .iter()
        .copied()
        .filter(|item| item_matches(item, &query, false))
        .collect::<Vec<_>>();
    if exact.len() == 1 {
        return Ok(exact[0].stable_id.clone());
    }
    if exact.len() > 1 {
        return Err(ambiguous_item_error(query, &exact));
    }

    let prefix = candidate_items
        .iter()
        .copied()
        .filter(|item| item_matches(item, &query, true))
        .collect::<Vec<_>>();
    match prefix.as_slice() {
        [item] => Ok(item.stable_id.clone()),
        [] => Err(WorldError::ItemNotFound(query)),
        matches => Err(ambiguous_item_error(query, matches)),
    }
}

fn item_matches(item: &Item, query: &str, prefix: bool) -> bool {
    let matches = |term: &str| {
        if prefix {
            term.to_ascii_lowercase().starts_with(query)
        } else {
            term.eq_ignore_ascii_case(query)
        }
    };
    matches(&item.name) || item.aliases.iter().any(|alias| matches(alias))
}

fn ambiguous_item_error(query: String, items: &[&Item]) -> WorldError {
    let mut names = items
        .iter()
        .map(|item| item.name.clone())
        .collect::<Vec<_>>();
    names.sort_unstable();
    names.dedup();
    WorldError::AmbiguousItem {
        query,
        matches: names.join(", "),
    }
}

fn gameplay_event_room(event: &GameplayEvent) -> Option<&str> {
    match event {
        GameplayEvent::TickAdvanced { .. } => None,
        GameplayEvent::NpcSpawned { room_id, .. }
        | GameplayEvent::CombatStarted { room_id, .. }
        | GameplayEvent::AbilityUsed { room_id, .. }
        | GameplayEvent::NpcAbilityUsed { room_id, .. }
        | GameplayEvent::NpcDamaged { room_id, .. }
        | GameplayEvent::CharacterDamaged { room_id, .. }
        | GameplayEvent::NpcDefeated { room_id, .. }
        | GameplayEvent::LootDropped { room_id, .. }
        | GameplayEvent::ExperienceGained { room_id, .. }
        | GameplayEvent::CharacterLeveled { room_id, .. }
        | GameplayEvent::CharacterPromoted { room_id, .. }
        | GameplayEvent::TalentRankAcquired { room_id, .. }
        | GameplayEvent::CharacterDefeated { room_id, .. }
        | GameplayEvent::CharacterCollapsed { room_id, .. }
        | GameplayEvent::CharacterRecovered { room_id, .. }
        | GameplayEvent::NpcRespawned { room_id, .. }
        | GameplayEvent::ItemEquipped { room_id, .. }
        | GameplayEvent::ItemUnequipped { room_id, .. }
        | GameplayEvent::ItemUsed { room_id, .. }
        | GameplayEvent::CharacterHealed { room_id, .. }
        | GameplayEvent::EffectApplied { room_id, .. }
        | GameplayEvent::EffectExpired { room_id, .. } => Some(room_id),
    }
}

fn gameplay_event_character(event: &GameplayEvent) -> Option<CharacterId> {
    match event {
        GameplayEvent::CombatStarted { character_id, .. }
        | GameplayEvent::AbilityUsed { character_id, .. }
        | GameplayEvent::NpcAbilityUsed { character_id, .. }
        | GameplayEvent::NpcDamaged { character_id, .. }
        | GameplayEvent::CharacterDamaged { character_id, .. }
        | GameplayEvent::NpcDefeated { character_id, .. }
        | GameplayEvent::ExperienceGained { character_id, .. }
        | GameplayEvent::CharacterLeveled { character_id, .. }
        | GameplayEvent::CharacterPromoted { character_id, .. }
        | GameplayEvent::TalentRankAcquired { character_id, .. }
        | GameplayEvent::CharacterDefeated { character_id, .. }
        | GameplayEvent::CharacterCollapsed { character_id, .. }
        | GameplayEvent::CharacterRecovered { character_id, .. }
        | GameplayEvent::ItemEquipped { character_id, .. }
        | GameplayEvent::ItemUnequipped { character_id, .. }
        | GameplayEvent::ItemUsed { character_id, .. }
        | GameplayEvent::CharacterHealed { character_id, .. }
        | GameplayEvent::EffectApplied { character_id, .. }
        | GameplayEvent::EffectExpired { character_id, .. } => Some(*character_id),
        GameplayEvent::TickAdvanced { .. }
        | GameplayEvent::NpcSpawned { .. }
        | GameplayEvent::LootDropped { .. }
        | GameplayEvent::NpcRespawned { .. } => None,
    }
}

fn gameplay_event_npc(event: &GameplayEvent) -> Option<&str> {
    match event {
        GameplayEvent::AbilityUsed { npc_id, .. } => npc_id.as_deref(),
        GameplayEvent::NpcSpawned { npc_id, .. }
        | GameplayEvent::NpcAbilityUsed { npc_id, .. }
        | GameplayEvent::CombatStarted { npc_id, .. }
        | GameplayEvent::NpcDamaged { npc_id, .. }
        | GameplayEvent::CharacterDamaged { npc_id, .. }
        | GameplayEvent::NpcDefeated { npc_id, .. }
        | GameplayEvent::LootDropped { npc_id, .. }
        | GameplayEvent::CharacterDefeated { npc_id, .. }
        | GameplayEvent::NpcRespawned { npc_id, .. } => Some(npc_id),
        GameplayEvent::TickAdvanced { .. }
        | GameplayEvent::ExperienceGained { .. }
        | GameplayEvent::CharacterLeveled { .. }
        | GameplayEvent::CharacterPromoted { .. }
        | GameplayEvent::TalentRankAcquired { .. }
        | GameplayEvent::CharacterCollapsed { .. }
        | GameplayEvent::CharacterRecovered { .. }
        | GameplayEvent::ItemEquipped { .. }
        | GameplayEvent::ItemUnequipped { .. }
        | GameplayEvent::ItemUsed { .. }
        | GameplayEvent::CharacterHealed { .. }
        | GameplayEvent::EffectApplied { .. }
        | GameplayEvent::EffectExpired { .. } => None,
    }
}

fn gameplay_event_item(event: &GameplayEvent) -> Option<&str> {
    match event {
        GameplayEvent::LootDropped { item_id, .. }
        | GameplayEvent::ItemEquipped { item_id, .. }
        | GameplayEvent::ItemUnequipped { item_id, .. }
        | GameplayEvent::ItemUsed { item_id, .. } => Some(item_id),
        GameplayEvent::TickAdvanced { .. }
        | GameplayEvent::NpcSpawned { .. }
        | GameplayEvent::CombatStarted { .. }
        | GameplayEvent::AbilityUsed { .. }
        | GameplayEvent::NpcAbilityUsed { .. }
        | GameplayEvent::NpcDamaged { .. }
        | GameplayEvent::CharacterDamaged { .. }
        | GameplayEvent::NpcDefeated { .. }
        | GameplayEvent::ExperienceGained { .. }
        | GameplayEvent::CharacterLeveled { .. }
        | GameplayEvent::CharacterPromoted { .. }
        | GameplayEvent::TalentRankAcquired { .. }
        | GameplayEvent::CharacterDefeated { .. }
        | GameplayEvent::CharacterCollapsed { .. }
        | GameplayEvent::CharacterRecovered { .. }
        | GameplayEvent::NpcRespawned { .. }
        | GameplayEvent::CharacterHealed { .. }
        | GameplayEvent::EffectApplied { .. }
        | GameplayEvent::EffectExpired { .. } => None,
    }
}

fn project_gameplay_event(
    salt: u64,
    viewer_id: CharacterId,
    event: &GameplayEvent,
) -> (String, Option<String>, String, Option<String>) {
    let character_id = |id: CharacterId| opaque_id(salt, "character", &id.0.to_string());
    let npc_id = |id: &str| opaque_id(salt, "npc", id);
    let item_id = |id: &str| opaque_id(salt, "item", id);
    match event {
        GameplayEvent::TickAdvanced { .. } => (
            opaque_id(salt, "world", "clock"),
            None,
            "Time advances.".into(),
            None,
        ),
        GameplayEvent::NpcSpawned {
            npc_id: npc,
            npc_name,
            ..
        }
        | GameplayEvent::NpcRespawned {
            npc_id: npc,
            npc_name,
            ..
        } => (npc_id(npc), None, format!("{npc_name} appears."), None),
        GameplayEvent::CombatStarted {
            character_id: character,
            character_name,
            npc_id: npc,
            npc_name,
            ..
        } => (
            character_id(*character),
            Some(npc_id(npc)),
            if viewer_id == *character {
                format!("You engage {npc_name}.")
            } else {
                format!("{character_name} engages {npc_name}.")
            },
            None,
        ),
        GameplayEvent::AbilityUsed {
            character_id: character,
            character_name,
            ability_name,
            npc_id: npc,
            npc_name,
            ..
        } => (
            character_id(*character),
            npc.as_deref().map(npc_id),
            match (viewer_id == *character, npc_name) {
                (true, Some(target)) => format!("You use {ability_name} on {target}."),
                (true, None) => format!("You use {ability_name}."),
                (false, Some(target)) => {
                    format!("{character_name} uses {ability_name} on {target}.")
                }
                (false, None) => format!("{character_name} uses {ability_name}."),
            },
            None,
        ),
        GameplayEvent::NpcAbilityUsed {
            npc_id: npc,
            npc_name,
            character_id: character,
            character_name,
            ability_name,
            ..
        } => (
            npc_id(npc),
            Some(character_id(*character)),
            if viewer_id == *character {
                format!("{npc_name} uses {ability_name} on you.")
            } else {
                format!("{npc_name} uses {ability_name} on {character_name}.")
            },
            None,
        ),
        GameplayEvent::NpcDamaged {
            character_id: character,
            character_name,
            npc_id: npc,
            npc_name,
            amount,
            remaining_health,
            ..
        } => (
            character_id(*character),
            Some(npc_id(npc)),
            if viewer_id == *character {
                format!("You hit {npc_name} for {amount} ({remaining_health} health).")
            } else {
                format!("{character_name} hits {npc_name} for {amount}.")
            },
            None,
        ),
        GameplayEvent::CharacterDamaged {
            character_id: character,
            character_name,
            npc_id: npc,
            npc_name,
            amount,
            remaining_health,
            ..
        } => (
            npc_id(npc),
            Some(character_id(*character)),
            if viewer_id == *character {
                format!("{npc_name} hits you for {amount} ({remaining_health} health).")
            } else {
                format!("{npc_name} hits {character_name} for {amount}.")
            },
            None,
        ),
        GameplayEvent::NpcDefeated {
            character_id: character,
            character_name,
            npc_id: npc,
            npc_name,
            ..
        } => (
            character_id(*character),
            Some(npc_id(npc)),
            if viewer_id == *character {
                format!("You defeat {npc_name}.")
            } else {
                format!("{character_name} defeats {npc_name}.")
            },
            None,
        ),
        GameplayEvent::LootDropped {
            npc_id: npc,
            npc_name,
            item_id: item,
            item_name,
            ..
        } => (
            npc_id(npc),
            None,
            format!("{npc_name} drops {item_name}."),
            Some(item_id(item)),
        ),
        GameplayEvent::ExperienceGained {
            character_id: character,
            character_name,
            amount,
            ..
        } => (
            character_id(*character),
            None,
            if viewer_id == *character {
                format!("You gain {amount} experience.")
            } else {
                format!("{character_name} grows more experienced.")
            },
            None,
        ),
        GameplayEvent::CharacterLeveled {
            character_id: character,
            character_name,
            level,
            ..
        } => (
            character_id(*character),
            None,
            if viewer_id == *character {
                format!("You reach level {level}.")
            } else {
                format!("{character_name} grows in power.")
            },
            None,
        ),
        GameplayEvent::CharacterPromoted {
            character_id: character,
            character_name,
            to_class_id,
            ..
        } => (
            character_id(*character),
            None,
            if viewer_id == *character {
                format!("You advance to {}.", humanize_id(to_class_id))
            } else {
                format!("{character_name} advances to a new class.")
            },
            None,
        ),
        GameplayEvent::TalentRankAcquired {
            character_id: character,
            character_name,
            node_id,
            rank,
            ..
        } => (
            character_id(*character),
            None,
            if viewer_id == *character {
                format!("You acquire rank {rank} of {}.", humanize_id(node_id))
            } else {
                format!("{character_name} develops a new talent.")
            },
            None,
        ),
        GameplayEvent::CharacterDefeated {
            character_id: character,
            character_name,
            npc_id: npc,
            npc_name,
            ..
        } => (
            npc_id(npc),
            Some(character_id(*character)),
            if viewer_id == *character {
                format!("{npc_name} defeats you.")
            } else {
                format!("{npc_name} defeats {character_name}.")
            },
            None,
        ),
        GameplayEvent::CharacterCollapsed {
            character_id: character,
            character_name,
            ..
        } => (
            character_id(*character),
            None,
            if viewer_id == *character {
                "You collapse and lose consciousness.".into()
            } else {
                format!("{character_name} collapses.")
            },
            None,
        ),
        GameplayEvent::CharacterRecovered {
            character_id: character,
            character_name,
            ..
        } => (
            character_id(*character),
            None,
            if viewer_id == *character {
                "You recover at the sanctuary.".into()
            } else {
                format!("{character_name} recovers.")
            },
            None,
        ),
        GameplayEvent::ItemEquipped {
            character_id: character,
            character_name,
            item_id: item,
            item_name,
            ..
        }
        | GameplayEvent::ItemUnequipped {
            character_id: character,
            character_name,
            item_id: item,
            item_name,
            ..
        }
        | GameplayEvent::ItemUsed {
            character_id: character,
            character_name,
            item_id: item,
            item_name,
            ..
        } => (
            character_id(*character),
            None,
            if viewer_id == *character {
                format!("You handle {item_name}.")
            } else {
                format!("{character_name} handles {item_name}.")
            },
            Some(item_id(item)),
        ),
        GameplayEvent::CharacterHealed {
            character_id: character,
            character_name,
            amount,
            ..
        } => (
            character_id(*character),
            None,
            if viewer_id == *character {
                format!("You recover {amount} health.")
            } else {
                format!("{character_name} looks healthier.")
            },
            None,
        ),
        GameplayEvent::EffectApplied {
            character_id: character,
            character_name,
            effect,
            ..
        } => (
            character_id(*character),
            None,
            if viewer_id == *character {
                format!("You gain the {:?} effect.", effect.kind)
            } else {
                format!("{character_name} is surrounded by a brief aura.")
            },
            None,
        ),
        GameplayEvent::EffectExpired {
            character_id: character,
            character_name,
            kind,
            ..
        } => (
            character_id(*character),
            None,
            if viewer_id == *character {
                format!("Your {kind:?} effect expires.")
            } else {
                format!("The aura around {character_name} fades.")
            },
            None,
        ),
    }
}

fn opaque_id(observer_salt: u64, kind: &str, domain_id: &str) -> String {
    let mut hasher = DefaultHasher::new();
    ("tiamud-view-v1", observer_salt, kind, domain_id).hash(&mut hasher);
    format!("o{:016x}", hasher.finish())
}

fn room(entities: &SlotMap<EntityId, Entity>, id: EntityId) -> &Room {
    match &entities[id] {
        Entity::Room(room) => room,
        Entity::Character(_) => unreachable!("room reference points to a character"),
    }
}

fn room_mut(entities: &mut SlotMap<EntityId, Entity>, id: EntityId) -> &mut Room {
    match &mut entities[id] {
        Entity::Room(room) => room,
        Entity::Character(_) => unreachable!("room reference points to a character"),
    }
}

fn room_checked(entities: &SlotMap<EntityId, Entity>, id: EntityId) -> Result<&Room, WorldError> {
    match entities.get(id) {
        Some(Entity::Room(room)) => Ok(room),
        Some(Entity::Character(_)) | None => Err(WorldError::InvalidEntity),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn connected_world() -> (World, SessionId) {
        let mut world = World::demo();
        let session = SessionId(7);
        world
            .connect(
                session,
                CharacterSnapshot {
                    id: CharacterId(101),
                    account_id: AccountId(10),
                    name: "Tester".into(),
                    room_id: world.start_room_id().to_owned(),
                    item_locations: BTreeMap::new(),
                },
            )
            .unwrap();
        (world, session)
    }

    fn phase_two_world(seed: u64, npc_strength: i32, npc_health: i32) -> (World, SessionId) {
        let mut world = World::from_definition_with_staged_items(
            WorldDefinition {
                start_room_id: "game:start".into(),
                items: vec![
                    ItemDefinition {
                        stable_id: "game:sword".into(),
                        name: "iron sword".into(),
                        description: "A balanced beginner's blade.".into(),
                        aliases: vec!["sword".into(), "blade".into()],
                    },
                    ItemDefinition {
                        stable_id: "game:potion".into(),
                        name: "red potion".into(),
                        description: "A restorative draught.".into(),
                        aliases: vec!["potion".into()],
                    },
                    ItemDefinition {
                        stable_id: "game:fang".into(),
                        name: "silver fang".into(),
                        description: "Proof of victory over the guardian.".into(),
                        aliases: vec!["fang".into()],
                    },
                ],
                rooms: vec![
                    RoomDefinition {
                        stable_id: "game:start".into(),
                        title: "Sanctuary".into(),
                        prose: vec!["A quiet place to prepare.".into()],
                        image: None,
                        audio: None,
                        exits: vec![ExitDefinition {
                            direction: Direction::North,
                            target_room_id: "game:arena".into(),
                        }],
                        item_ids: vec!["game:sword".into(), "game:potion".into()],
                    },
                    RoomDefinition {
                        stable_id: "game:arena".into(),
                        title: "Old Arena".into(),
                        prose: vec!["Claw marks cross the sand.".into()],
                        image: None,
                        audio: None,
                        exits: vec![ExitDefinition {
                            direction: Direction::South,
                            target_room_id: "game:start".into(),
                        }],
                        item_ids: Vec::new(),
                    },
                ],
            },
            vec!["game:fang".into()],
        )
        .unwrap();
        world.set_rng_seed(seed);
        world
            .configure_item_gameplay(ItemGameplayDefinition {
                item_id: "game:sword".into(),
                equipment_slot: Some(EquipmentSlot::Weapon),
                attack_bonus: 4,
                heal_amount: 0,
                consumable: false,
                effect: None,
            })
            .unwrap();
        world
            .configure_item_gameplay(ItemGameplayDefinition {
                item_id: "game:potion".into(),
                equipment_slot: None,
                attack_bonus: 0,
                heal_amount: 5,
                consumable: true,
                effect: Some(ActiveEffect {
                    kind: EffectKind::Regeneration,
                    magnitude: 2,
                    remaining_ticks: 2,
                }),
            })
            .unwrap();
        world
            .spawn_npc(NpcDefinition {
                stable_id: "game:guardian".into(),
                name: "fang guardian".into(),
                description: "A scarred beast guards the arena.".into(),
                room_id: "game:arena".into(),
                level: 2,
                max_health: npc_health,
                strength: npc_strength,
                defence: 1,
                experience_reward: 120,
                loot_item_ids: vec!["game:fang".into()],
                respawn_ticks: 3,
            })
            .unwrap();
        let session = SessionId(900);
        world
            .connect(
                session,
                CharacterSnapshot {
                    id: CharacterId(900),
                    account_id: AccountId(90),
                    name: "Adventurer".into(),
                    room_id: "game:start".into(),
                    item_locations: BTreeMap::new(),
                },
            )
            .unwrap();
        (world, session)
    }

    #[test]
    fn runtime_gameplay_definitions_enforce_numeric_caps() {
        let (mut world, _) = phase_two_world(1, 2, 10);
        let max_stat_value = i32::try_from(MAX_STAT_VALUE).expect("MAX_STAT_VALUE fits in i32");
        let base_item = ItemGameplayDefinition {
            item_id: "game:sword".into(),
            equipment_slot: Some(EquipmentSlot::Weapon),
            attack_bonus: 1,
            heal_amount: 0,
            consumable: false,
            effect: None,
        };
        for invalid_item in [
            ItemGameplayDefinition {
                attack_bonus: max_stat_value + 1,
                ..base_item.clone()
            },
            ItemGameplayDefinition {
                heal_amount: max_stat_value + 1,
                ..base_item.clone()
            },
            ItemGameplayDefinition {
                effect: Some(ActiveEffect {
                    kind: EffectKind::Strength,
                    magnitude: MAX_STATUS_MAGNITUDE + 1,
                    remaining_ticks: 1,
                }),
                ..base_item
            },
        ] {
            assert!(matches!(
                world.configure_item_gameplay(invalid_item),
                Err(WorldError::InvalidGameplayDefinition)
            ));
        }

        let base_npc = NpcDefinition {
            stable_id: "game:bounded-sentinel".into(),
            name: "bounded sentinel".into(),
            description: "A sentinel used to verify runtime bounds.".into(),
            room_id: "game:start".into(),
            level: 1,
            max_health: 10,
            strength: 2,
            defence: 1,
            experience_reward: 1,
            loot_item_ids: Vec::new(),
            respawn_ticks: 1,
        };
        let mut invalid_npcs = Vec::new();
        let mut invalid = base_npc.clone();
        invalid.level = MAX_PROGRESSION_LEVEL + 1;
        invalid_npcs.push(invalid);
        let mut invalid = base_npc.clone();
        invalid.max_health = max_stat_value + 1;
        invalid_npcs.push(invalid);
        let mut invalid = base_npc.clone();
        invalid.strength = max_stat_value + 1;
        invalid_npcs.push(invalid);
        let mut invalid = base_npc;
        invalid.defence = max_stat_value + 1;
        invalid_npcs.push(invalid);
        for invalid_npc in invalid_npcs {
            assert!(matches!(
                world.spawn_npc(invalid_npc),
                Err(WorldError::InvalidGameplayDefinition)
            ));
        }
    }

    #[test]
    fn restored_item_effects_enforce_numeric_caps() {
        let (mut world, session) = phase_two_world(1, 2, 10);
        let mut snapshot = world.character_gameplay_snapshot(session).unwrap();
        snapshot.effects.push(ActiveEffect {
            kind: EffectKind::Strength,
            magnitude: MAX_STATUS_MAGNITUDE + 1,
            remaining_ticks: 1,
        });

        assert!(matches!(
            world.restore_character_gameplay(CharacterId(900), snapshot),
            Err(WorldError::InvalidGameplayDefinition)
        ));
    }

    #[test]
    fn combat_saturates_equipment_and_strength_effect_bonuses() {
        let (mut world, session) = phase_two_world(7, 2, 10);
        world
            .apply(session, PlayerAction::Move(Direction::North))
            .unwrap();

        for (item_id, slot) in [
            ("game:sword", EquipmentSlot::Weapon),
            ("game:potion", EquipmentSlot::Body),
            ("game:fang", EquipmentSlot::Trinket),
        ] {
            world.item_gameplay.insert(
                item_id.into(),
                ItemGameplayDefinition {
                    item_id: item_id.into(),
                    equipment_slot: Some(slot),
                    attack_bonus: i32::MAX,
                    heal_amount: 0,
                    consumable: false,
                    effect: None,
                },
            );
            world
                .character_gameplay
                .get_mut(&CharacterId(900))
                .unwrap()
                .equipment
                .insert(slot, item_id.into());
        }
        world
            .character_gameplay
            .get_mut(&CharacterId(900))
            .unwrap()
            .effects = vec![
            ActiveEffect {
                kind: EffectKind::Strength,
                magnitude: i32::MAX,
                remaining_ticks: 2,
            },
            ActiveEffect {
                kind: EffectKind::Strength,
                magnitude: i32::MAX,
                remaining_ticks: 2,
            },
        ];
        world.npcs.get_mut("game:guardian").unwrap().defence = 0;

        world
            .apply(session, PlayerAction::Attack("guardian".into()))
            .unwrap();
        let events = world.advance_tick().unwrap();
        let damage = events.iter().find_map(|event| match event {
            DomainEvent::Gameplay {
                event:
                    GameplayEvent::NpcDamaged {
                        amount,
                        remaining_health,
                        ..
                    },
            } => Some((*amount, *remaining_health)),
            _ => None,
        });

        assert_eq!(damage, Some((i32::MAX, 0)));
        assert_eq!(world.npcs["game:guardian"].health, 0);
    }

    #[test]
    fn parses_legacy_friendly_movement_commands() {
        assert_eq!(parse_action("l"), Ok(PlayerAction::Look));
        assert_eq!(
            parse_action("go north"),
            Ok(PlayerAction::Move(Direction::North))
        );
    }

    #[test]
    fn parses_inventory_and_item_commands_with_safe_missing_targets() {
        assert_eq!(parse_action("inventory"), Ok(PlayerAction::Inventory));
        assert_eq!(
            parse_action("get Blue Lantern"),
            Ok(PlayerAction::Take("blue lantern".into()))
        );
        assert_eq!(
            parse_action("drop lamp"),
            Ok(PlayerAction::Drop("lamp".into()))
        );
        assert_eq!(
            parse_action("x lantern"),
            Ok(PlayerAction::Examine("lantern".into()))
        );
        assert_eq!(
            parse_action("take   "),
            Err(ParseActionError::MissingTarget { command: "take" })
        );
        assert_eq!(
            parse_action("examine"),
            Err(ParseActionError::MissingTarget { command: "examine" })
        );
    }

    #[test]
    fn parses_social_and_moderation_commands_without_lowercasing_messages() {
        assert_eq!(
            parse_action("say Hold the North Gate!"),
            Ok(PlayerAction::Social(SocialAction::Say(
                "Hold the North Gate!".into()
            )))
        );
        assert_eq!(
            parse_action("tell Aria Meet Me There"),
            Ok(PlayerAction::Social(SocialAction::Tell {
                target: "Aria".into(),
                message: "Meet Me There".into(),
            }))
        );
        assert_eq!(
            parse_action("party say Hold the North Gate!"),
            Ok(PlayerAction::Social(SocialAction::PartySay(
                "Hold the North Gate!".into()
            )))
        );
        assert_eq!(
            parse_action("reply I Am On My Way"),
            Ok(PlayerAction::Social(SocialAction::Reply(
                "I Am On My Way".into()
            )))
        );
        assert_eq!(
            parse_action("channel list"),
            Ok(PlayerAction::Social(SocialAction::ChannelList))
        );
        assert_eq!(
            parse_action("channel join newbie"),
            Ok(PlayerAction::Social(SocialAction::ChannelJoin(
                SocialChannelName::Newbie
            )))
        );
        assert_eq!(
            parse_action("channel leave general"),
            Ok(PlayerAction::Social(SocialAction::ChannelLeave(
                SocialChannelName::General
            )))
        );
        assert_eq!(
            parse_action("channel say general Keep This Case"),
            Ok(PlayerAction::Social(SocialAction::ChannelSay {
                channel: SocialChannelName::General,
                message: "Keep This Case".into(),
            }))
        );
        assert_eq!(
            parse_action("party revoke Aria"),
            Ok(PlayerAction::Party(PartyAction::Revoke("Aria".into())))
        );
        assert_eq!(
            parse_action("party kick Aria"),
            Ok(PlayerAction::Party(PartyAction::Kick("Aria".into())))
        );
        assert_eq!(
            parse_action("party transfer Aria"),
            Ok(PlayerAction::Party(PartyAction::Transfer("Aria".into())))
        );
        assert_eq!(
            parse_action("party disband"),
            Ok(PlayerAction::Party(PartyAction::Disband))
        );
        for command in ["party say", "reply", "channel join", "channel say general"] {
            assert!(matches!(
                parse_action(command),
                Err(ParseActionError::MissingTarget { .. })
            ));
        }
        assert_eq!(
            parse_action("mute Griefer 2h repeated flooding"),
            Ok(PlayerAction::Social(SocialAction::Mute {
                target: "Griefer".into(),
                duration_seconds: Some(7_200),
                reason: "repeated flooding".into(),
            }))
        );
        assert_eq!(
            parse_action("ban Griefer permanent severe abuse"),
            Ok(PlayerAction::Social(SocialAction::Ban {
                target: "Griefer".into(),
                duration_seconds: None,
                reason: "severe abuse".into(),
            }))
        );
        assert_eq!(
            parse_action("role Aria moderator"),
            Ok(PlayerAction::Social(SocialAction::SetRole {
                target: "Aria".into(),
                role: AccountRole::Moderator,
            }))
        );
    }

    #[test]
    fn movement_changes_the_stable_room_reference() {
        let (mut world, session) = connected_world();
        let outcome = world
            .apply(session, PlayerAction::Move(Direction::North))
            .unwrap();
        assert_ne!(outcome.scene.view_id, "demo:north-gate");
        assert_eq!(outcome.scene.title, "The North Gate");
        assert_eq!(
            outcome.changed_character.unwrap().room_id,
            "demo:north-gate"
        );
    }

    #[test]
    fn failed_movement_does_not_change_state() {
        let (mut world, session) = connected_world();
        let before = world.character_snapshot(session).unwrap();
        assert!(matches!(
            world.apply(session, PlayerAction::Move(Direction::West)),
            Err(WorldError::NoExit)
        ));
        assert_eq!(world.character_snapshot(session).unwrap(), before);
    }

    #[test]
    fn unknown_persisted_rooms_fall_back_safely() {
        let mut world = World::demo();
        let session = SessionId(11);
        let scene = world
            .connect(
                session,
                CharacterSnapshot {
                    id: CharacterId(102),
                    account_id: AccountId(10),
                    name: "Old Save".into(),
                    room_id: "missing:room".into(),
                    item_locations: BTreeMap::new(),
                },
            )
            .unwrap();
        assert_ne!(scene.view_id, "demo:lantern-square");
        assert_eq!(scene.title, "Lantern Square");
    }

    #[test]
    fn corrupt_exit_references_return_an_error_instead_of_panicking() {
        let (mut world, session) = connected_world();
        let character_id = world.sessions[&session];
        room_mut(&mut world.entities, world.start_room)
            .exits
            .insert(Direction::West, character_id);

        assert!(matches!(
            world.scene_for(session),
            Err(WorldError::InvalidEntity)
        ));
        assert!(matches!(
            world.apply(session, PlayerAction::Move(Direction::West)),
            Err(WorldError::InvalidEntity)
        ));
        assert_eq!(
            world.character_snapshot(session).unwrap().room_id,
            "demo:lantern-square"
        );
    }

    #[test]
    fn durable_character_identity_survives_world_changes() {
        let (mut world, session) = connected_world();
        world
            .apply(session, PlayerAction::Move(Direction::North))
            .unwrap();

        let snapshot = world.character_snapshot(session).unwrap();
        assert_eq!(snapshot.id, CharacterId(101));
        assert_eq!(snapshot.account_id, AccountId(10));
        assert_eq!(snapshot.name, "Tester");
    }

    #[test]
    fn a_character_can_reconnect_after_disconnect() {
        let (mut world, first_session) = connected_world();
        let snapshot = world.character_snapshot(first_session).unwrap();
        world.disconnect(first_session);

        let scene = world.connect(SessionId(9), snapshot).unwrap();
        assert_ne!(scene.view_id, "demo:lantern-square");
        assert_eq!(scene.title, "Lantern Square");
    }

    #[test]
    fn a_character_cannot_be_connected_by_two_sessions_at_once() {
        let (mut world, first_session) = connected_world();
        let snapshot = world.character_snapshot(first_session).unwrap();

        assert!(matches!(
            world.connect(SessionId(9), snapshot),
            Err(WorldError::CharacterAlreadyConnected)
        ));
        assert_eq!(
            world.character_snapshot(first_session).unwrap().id,
            CharacterId(101)
        );
    }

    #[test]
    fn rollback_cannot_replace_a_sessions_durable_identity() {
        let (mut world, session) = connected_world();
        let mut foreign = world.character_snapshot(session).unwrap();
        foreign.id = CharacterId(999);
        let revision = world.revision();

        assert!(matches!(
            world.restore_character(session, &foreign, revision),
            Err(WorldError::InvalidEntity)
        ));
        assert_eq!(
            world.character_snapshot(session).unwrap().id,
            CharacterId(101)
        );
    }

    #[test]
    fn builds_a_world_from_a_transport_independent_definition() {
        let image = SceneImage {
            alt: String::new(),
            localized_alt: BTreeMap::new(),
            manifest: Some("/assets/aaaaaaaa/manifest.json".into()),
            avif: None,
            webp: None,
            jpeg: None,
            width: None,
            height: None,
        };
        let mut world = World::from_definition(WorldDefinition {
            start_room_id: "test:start".into(),
            items: Vec::new(),
            rooms: vec![
                RoomDefinition {
                    stable_id: "test:start".into(),
                    title: "Start".into(),
                    prose: vec!["A complete textual scene.".into()],
                    image: Some(image.clone()),
                    audio: None,
                    exits: vec![ExitDefinition {
                        direction: Direction::North,
                        target_room_id: "test:north".into(),
                    }],
                    item_ids: Vec::new(),
                },
                RoomDefinition {
                    stable_id: "test:north".into(),
                    title: "North".into(),
                    prose: vec!["The northern room.".into()],
                    image: None,
                    audio: None,
                    exits: Vec::new(),
                    item_ids: Vec::new(),
                },
            ],
        })
        .unwrap();
        let session = SessionId(42);
        let scene = world
            .connect(
                session,
                CharacterSnapshot {
                    id: CharacterId(7),
                    account_id: AccountId(3),
                    name: "Builder".into(),
                    room_id: "test:start".into(),
                    item_locations: BTreeMap::new(),
                },
            )
            .unwrap();

        assert_eq!(scene.image, Some(image));
        assert_eq!(scene.exits[0].command, "north");
        assert_eq!(
            world
                .apply(session, PlayerAction::Move(Direction::North))
                .unwrap()
                .scene
                .title,
            "North"
        );
    }

    #[test]
    fn rejects_invalid_world_definition_references() {
        let missing_default = World::from_definition(WorldDefinition {
            start_room_id: "missing".into(),
            items: Vec::new(),
            rooms: vec![RoomDefinition {
                stable_id: "only".into(),
                title: "Only".into(),
                prose: Vec::new(),
                image: None,
                audio: None,
                exits: Vec::new(),
                item_ids: Vec::new(),
            }],
        });
        assert!(matches!(
            missing_default,
            Err(WorldBuildError::UnknownStartRoom(room)) if room == "missing"
        ));

        let missing_exit = World::from_definition(WorldDefinition {
            start_room_id: "only".into(),
            items: Vec::new(),
            rooms: vec![RoomDefinition {
                stable_id: "only".into(),
                title: "Only".into(),
                prose: Vec::new(),
                image: None,
                audio: None,
                exits: vec![ExitDefinition {
                    direction: Direction::Down,
                    target_room_id: "void".into(),
                }],
                item_ids: Vec::new(),
            }],
        });
        assert!(matches!(
            missing_exit,
            Err(WorldBuildError::UnknownExitTarget { target_room_id, .. }) if target_room_id == "void"
        ));

        let duplicate_direction = World::from_definition(WorldDefinition {
            start_room_id: "only".into(),
            items: Vec::new(),
            rooms: vec![RoomDefinition {
                stable_id: "only".into(),
                title: "Only".into(),
                prose: Vec::new(),
                image: None,
                audio: None,
                exits: vec![
                    ExitDefinition {
                        direction: Direction::North,
                        target_room_id: "only".into(),
                    },
                    ExitDefinition {
                        direction: Direction::North,
                        target_room_id: "only".into(),
                    },
                ],
                item_ids: Vec::new(),
            }],
        });
        assert!(matches!(
            duplicate_direction,
            Err(WorldBuildError::DuplicateExitDirection {
                direction: Direction::North,
                ..
            })
        ));
    }

    #[test]
    fn legacy_character_item_state_is_ignored_and_scene_ids_are_opaque() {
        let (world, session) = connected_world();

        let snapshot = world.character_snapshot(session).unwrap();
        assert!(snapshot.item_locations.is_empty());
        let scene = world.scene_for(session).unwrap();
        assert_eq!(scene.items.len(), 1);
        assert_eq!(scene.items[0].label, "blue-glass lantern");
        assert_ne!(scene.items[0].id, "demo:blue-lantern");
        assert!(!scene.items[0].actions[0].command.contains("demo:"));
        assert!(
            scene
                .as_plain_text()
                .contains("You see here: blue-glass lantern.")
        );
    }

    #[test]
    fn take_inventory_and_drop_form_a_shared_world_loop() {
        let (mut world, session) = connected_world();

        let taken = world
            .apply(session, PlayerAction::Take("lantern".into()))
            .unwrap();
        assert_eq!(
            taken.output.as_deref(),
            Some("You take the blue-glass lantern.\r\n")
        );
        assert!(taken.scene.items.is_empty());
        assert!(taken.changed_character.is_none());
        assert!(matches!(
            taken.domain_events.as_slice(),
            [DomainEvent::ItemTaken { .. }]
        ));
        assert_eq!(
            world
                .apply(session, PlayerAction::Inventory)
                .unwrap()
                .output
                .as_deref(),
            Some("You are carrying:\r\n- blue-glass lantern\r\n")
        );

        world
            .apply(session, PlayerAction::Move(Direction::North))
            .unwrap();
        let dropped = world
            .apply(session, PlayerAction::Drop("lamp".into()))
            .unwrap();
        assert_ne!(dropped.scene.items[0].id, "demo:blue-lantern");
        assert!(dropped.changed_character.is_none());
        let saved = world.character_snapshot(session).unwrap();

        world.disconnect(session);
        let reconnected = world.connect(SessionId(77), saved).unwrap();
        assert_eq!(reconnected.title, "The North Gate");
        assert_ne!(reconnected.items[0].id, "demo:blue-lantern");
    }

    #[test]
    fn examine_is_read_only_and_works_for_visible_or_carried_items() {
        let (mut world, session) = connected_world();
        let before = world.character_snapshot(session).unwrap();
        let examined = world
            .apply(session, PlayerAction::Examine("blue".into()))
            .unwrap();
        assert!(examined.output.unwrap().contains("Cold blue light"));
        assert!(examined.changed_character.is_none());
        assert_eq!(world.character_snapshot(session).unwrap(), before);

        world
            .apply(session, PlayerAction::Take("lantern".into()))
            .unwrap();
        assert!(
            world
                .apply(session, PlayerAction::Examine("lamp".into()))
                .unwrap()
                .output
                .unwrap()
                .contains("rain-speckled glass")
        );
    }

    #[test]
    fn absent_items_fail_without_mutation() {
        let (mut world, session) = connected_world();
        let before = world.character_snapshot(session).unwrap();
        let revision = world.revision();

        assert!(matches!(
            world.apply(session, PlayerAction::Take("sword".into())),
            Err(WorldError::ItemNotFound(query)) if query == "sword"
        ));
        assert!(matches!(
            world.apply(session, PlayerAction::Drop("lantern".into())),
            Err(WorldError::ItemNotFound(query)) if query == "lantern"
        ));
        assert_eq!(world.character_snapshot(session).unwrap(), before);
        assert_eq!(world.revision(), revision);
    }

    #[test]
    fn ambiguous_item_names_fail_without_mutation() {
        let mut world = World::from_definition(WorldDefinition {
            start_room_id: "test:start".into(),
            items: vec![
                ItemDefinition {
                    stable_id: "test:blue-lantern".into(),
                    name: "blue lantern".into(),
                    description: "Blue.".into(),
                    aliases: vec!["lantern".into()],
                },
                ItemDefinition {
                    stable_id: "test:brass-lantern".into(),
                    name: "brass lantern".into(),
                    description: "Brass.".into(),
                    aliases: vec!["lantern".into()],
                },
            ],
            rooms: vec![RoomDefinition {
                stable_id: "test:start".into(),
                title: "Start".into(),
                prose: Vec::new(),
                image: None,
                audio: None,
                exits: Vec::new(),
                item_ids: vec!["test:blue-lantern".into(), "test:brass-lantern".into()],
            }],
        })
        .unwrap();
        let session = SessionId(8);
        world
            .connect(
                session,
                CharacterSnapshot {
                    id: CharacterId(8),
                    account_id: AccountId(8),
                    name: "Chooser".into(),
                    room_id: "test:start".into(),
                    item_locations: BTreeMap::new(),
                },
            )
            .unwrap();
        let before = world.character_snapshot(session).unwrap();

        assert!(matches!(
            world.apply(session, PlayerAction::Take("lantern".into())),
            Err(WorldError::AmbiguousItem { .. })
        ));
        assert_eq!(world.character_snapshot(session).unwrap(), before);
    }

    #[test]
    fn taking_an_item_is_observed_globally() {
        let (mut world, first) = connected_world();
        let second = SessionId(17);
        world
            .connect(
                second,
                CharacterSnapshot {
                    id: CharacterId(202),
                    account_id: AccountId(20),
                    name: "Second".into(),
                    room_id: world.start_room_id().to_owned(),
                    item_locations: BTreeMap::new(),
                },
            )
            .unwrap();

        world
            .apply(first, PlayerAction::Take("lantern".into()))
            .unwrap();
        assert!(world.scene_for(first).unwrap().items.is_empty());
        assert!(world.scene_for(second).unwrap().items.is_empty());
        assert_eq!(
            world.inventory_as_plain_text(second).unwrap(),
            "You are carrying nothing.\r\n"
        );
    }

    #[test]
    fn one_domain_event_projects_differently_by_perception() {
        let (mut world, actor) = connected_world();
        let peer = SessionId(17);
        world
            .connect(
                peer,
                CharacterSnapshot {
                    id: CharacterId(202),
                    account_id: AccountId(20),
                    name: "Witness".into(),
                    room_id: world.start_room_id().to_owned(),
                    item_locations: BTreeMap::new(),
                },
            )
            .unwrap();
        let distant = SessionId(18);
        world
            .connect(
                distant,
                CharacterSnapshot {
                    id: CharacterId(303),
                    account_id: AccountId(30),
                    name: "Archivist".into(),
                    room_id: "demo:archive".into(),
                    item_locations: BTreeMap::new(),
                },
            )
            .unwrap();

        let actor_scene = world.scene_for(actor).unwrap();
        let peer_scene = world.scene_for(peer).unwrap();
        assert_ne!(actor_scene.view_id, peer_scene.view_id);
        assert_ne!(actor_scene.items[0].id, peer_scene.items[0].id);
        assert!(!actor_scene.items[0].id.contains("demo:"));
        assert!(!actor_scene.items[0].actions[1].command.contains("demo:"));
        let distant_revision = world.scene_for(distant).unwrap().revision;

        let action = parse_action(&actor_scene.items[0].actions[1].command).unwrap();
        let outcome = world.apply(actor, action).unwrap();
        let event = outcome.domain_events.first().unwrap();
        let actor_view = world.project_event(actor, event).unwrap().unwrap();
        let peer_view = world.project_event(peer, event).unwrap().unwrap();

        assert_eq!(actor_view.text, "You take the blue-glass lantern.");
        assert_eq!(peer_view.text, "Tester takes the blue-glass lantern.");
        assert_ne!(actor_view.actor_id, peer_view.actor_id);
        assert_ne!(actor_view.item_id, peer_view.item_id);
        assert!(!actor_view.actor_id.contains("101"));
        assert!(!actor_view.item_id.as_deref().unwrap().contains("demo:"));
        assert!(world.project_event(distant, event).unwrap().is_none());
        assert_eq!(world.scene_for(actor).unwrap().revision, 2);
        assert_eq!(world.scene_for(peer).unwrap().revision, 2);
        assert_eq!(world.scene_for(distant).unwrap().revision, distant_revision);
    }

    #[test]
    fn a_dropped_item_becomes_visible_to_other_characters() {
        let (mut world, actor) = connected_world();
        let peer = SessionId(17);
        world
            .connect(
                peer,
                CharacterSnapshot {
                    id: CharacterId(202),
                    account_id: AccountId(20),
                    name: "Witness".into(),
                    room_id: world.start_room_id().to_owned(),
                    item_locations: BTreeMap::new(),
                },
            )
            .unwrap();
        world
            .apply(actor, PlayerAction::Take("lantern".into()))
            .unwrap();
        world
            .apply(actor, PlayerAction::Move(Direction::North))
            .unwrap();
        world
            .apply(actor, PlayerAction::Drop("lantern".into()))
            .unwrap();

        assert_eq!(world.scene_for(actor).unwrap().items.len(), 1);
        assert!(world.scene_for(peer).unwrap().items.is_empty());
        world
            .apply(peer, PlayerAction::Move(Direction::North))
            .unwrap();
        assert_eq!(world.scene_for(peer).unwrap().items.len(), 1);
        assert_eq!(
            world.scene_for(peer).unwrap().items[0].label,
            "blue-glass lantern"
        );
    }

    #[test]
    fn rollback_restores_movement_and_observer_revision() {
        let (mut world, session) = connected_world();
        let before = world.character_snapshot(session).unwrap();
        let view_revision = world.scene_for(session).unwrap().revision;
        let revision = world.revision();
        world
            .apply(session, PlayerAction::Move(Direction::North))
            .unwrap();

        world.restore_character(session, &before, revision).unwrap();

        assert_eq!(world.character_snapshot(session).unwrap(), before);
        assert_eq!(world.revision(), revision);
        assert_eq!(world.scene_for(session).unwrap().revision, view_revision);
        assert_eq!(world.scene_for(session).unwrap().items.len(), 1);
        assert_eq!(
            world.inventory_as_plain_text(session).unwrap(),
            "You are carrying nothing.\r\n"
        );
    }

    #[test]
    fn replay_reconstructs_take_and_drop_with_the_owner_disconnected() {
        let owner = CharacterId(404);
        let taken = DomainEvent::ItemTaken {
            actor_id: owner,
            actor_name: "Offline Hero".into(),
            item_id: "demo:blue-lantern".into(),
            item_name: "blue-glass lantern".into(),
            room_id: "demo:lantern-square".into(),
        };
        let dropped = DomainEvent::ItemDropped {
            actor_id: owner,
            actor_name: "Offline Hero".into(),
            item_id: "demo:blue-lantern".into(),
            item_name: "blue-glass lantern".into(),
            room_id: "demo:north-gate".into(),
        };

        let mut carried_world = World::demo();
        carried_world.replay_persisted_event(&taken).unwrap();
        let owner_session = SessionId(404);
        carried_world
            .connect(
                owner_session,
                CharacterSnapshot {
                    id: owner,
                    account_id: AccountId(40),
                    name: "Offline Hero".into(),
                    room_id: "demo:archive".into(),
                    item_locations: BTreeMap::new(),
                },
            )
            .unwrap();
        assert_eq!(
            carried_world
                .inventory_as_plain_text(owner_session)
                .unwrap(),
            "You are carrying:\r\n- blue-glass lantern\r\n"
        );

        let mut dropped_world = World::demo();
        dropped_world.replay_persisted_event(&taken).unwrap();
        dropped_world.replay_persisted_event(&dropped).unwrap();
        let witness = SessionId(405);
        let scene = dropped_world
            .connect(
                witness,
                CharacterSnapshot {
                    id: CharacterId(405),
                    account_id: AccountId(40),
                    name: "Witness".into(),
                    room_id: "demo:north-gate".into(),
                    item_locations: BTreeMap::new(),
                },
            )
            .unwrap();
        assert_eq!(scene.items.len(), 1);
        assert_eq!(scene.items[0].label, "blue-glass lantern");
        assert_eq!(dropped_world.revision(), 3);
    }

    #[test]
    fn replay_rejects_invalid_metadata_and_transitions_atomically() {
        let mut world = World::demo();
        let invalid_name = DomainEvent::ItemTaken {
            actor_id: CharacterId(404),
            actor_name: "Offline Hero".into(),
            item_id: "demo:blue-lantern".into(),
            item_name: "forged name".into(),
            room_id: "demo:lantern-square".into(),
        };
        let invalid_transition = DomainEvent::ItemDropped {
            actor_id: CharacterId(404),
            actor_name: "Offline Hero".into(),
            item_id: "demo:blue-lantern".into(),
            item_name: "blue-glass lantern".into(),
            room_id: "demo:lantern-square".into(),
        };
        let unknown_item = DomainEvent::ItemTaken {
            actor_id: CharacterId(404),
            actor_name: "Offline Hero".into(),
            item_id: "demo:forged-item".into(),
            item_name: "blue-glass lantern".into(),
            room_id: "demo:lantern-square".into(),
        };
        let unknown_room = DomainEvent::ItemTaken {
            actor_id: CharacterId(404),
            actor_name: "Offline Hero".into(),
            item_id: "demo:blue-lantern".into(),
            item_name: "blue-glass lantern".into(),
            room_id: "demo:forged-room".into(),
        };

        for invalid in [&invalid_name, &unknown_item, &unknown_room] {
            assert!(matches!(
                world.replay_persisted_event(invalid),
                Err(WorldError::InvalidDomainEvent)
            ));
        }
        assert!(matches!(
            world.replay_persisted_event(&invalid_transition),
            Err(WorldError::InvalidDomainTransition)
        ));
        assert_eq!(world.revision(), 1);
        let session = SessionId(406);
        assert_eq!(
            world
                .connect(
                    session,
                    CharacterSnapshot {
                        id: CharacterId(406),
                        account_id: AccountId(40),
                        name: "Witness".into(),
                        room_id: "demo:lantern-square".into(),
                        item_locations: BTreeMap::new(),
                    },
                )
                .unwrap()
                .items
                .len(),
            1
        );
    }

    #[test]
    fn rollback_domain_events_restores_shared_state_and_view_revisions() {
        let (mut world, actor) = connected_world();
        let peer = SessionId(407);
        world
            .connect(
                peer,
                CharacterSnapshot {
                    id: CharacterId(407),
                    account_id: AccountId(40),
                    name: "Witness".into(),
                    room_id: "demo:lantern-square".into(),
                    item_locations: BTreeMap::new(),
                },
            )
            .unwrap();
        let domain_revision = world.revision();
        let actor_revision = world.scene_for(actor).unwrap().revision;
        let peer_revision = world.scene_for(peer).unwrap().revision;
        let outcome = world
            .apply(actor, PlayerAction::Take("lantern".into()))
            .unwrap();
        assert!(world.scene_for(peer).unwrap().items.is_empty());

        world
            .rollback_domain_events(&outcome.domain_events)
            .unwrap();

        assert_eq!(world.revision(), domain_revision);
        assert_eq!(world.scene_for(actor).unwrap().revision, actor_revision);
        assert_eq!(world.scene_for(peer).unwrap().revision, peer_revision);
        assert_eq!(world.scene_for(actor).unwrap().items.len(), 1);
        assert_eq!(world.scene_for(peer).unwrap().items.len(), 1);
        assert_eq!(
            world.inventory_as_plain_text(actor).unwrap(),
            "You are carrying nothing.\r\n"
        );
    }

    #[test]
    fn invalid_rollback_batch_does_not_partially_mutate_state() {
        let (mut world, actor) = connected_world();
        let outcome = world
            .apply(actor, PlayerAction::Take("lantern".into()))
            .unwrap();
        let revision = world.revision();
        let view_revision = world.scene_for(actor).unwrap().revision;
        let mut forged = outcome.domain_events.clone();
        if let DomainEvent::ItemTaken { item_name, .. } = &mut forged[0] {
            *item_name = "forged name".into();
        }

        assert!(matches!(
            world.rollback_domain_events(&forged),
            Err(WorldError::InvalidDomainEvent)
        ));
        assert_eq!(world.revision(), revision);
        assert_eq!(world.scene_for(actor).unwrap().revision, view_revision);
        assert_eq!(
            world.inventory_as_plain_text(actor).unwrap(),
            "You are carrying:\r\n- blue-glass lantern\r\n"
        );
    }

    #[test]
    fn parses_phase_two_gameplay_commands() {
        assert_eq!(
            parse_action("attack guardian"),
            Ok(PlayerAction::Attack("guardian".into()))
        );
        assert_eq!(
            parse_action("use potion"),
            Ok(PlayerAction::Use("potion".into()))
        );
        assert_eq!(
            parse_action("wear sword"),
            Ok(PlayerAction::Equip("sword".into()))
        );
        assert_eq!(
            parse_action("remove weapon"),
            Ok(PlayerAction::Unequip("weapon".into()))
        );
        assert_eq!(
            parse_action("attack"),
            Err(ParseActionError::MissingTarget { command: "attack" })
        );
    }

    #[test]
    fn deterministic_explore_combat_loot_and_progression_cycle() {
        let (mut world, session) = phase_two_world(42, 2, 14);
        assert_eq!(world.scene_for(session).unwrap().title, "Sanctuary");
        world
            .apply(session, PlayerAction::Take("sword".into()))
            .unwrap();
        let equipped = world
            .apply(session, PlayerAction::Equip("sword".into()))
            .unwrap();
        assert!(matches!(
            &equipped.domain_events[0],
            DomainEvent::Gameplay {
                event: GameplayEvent::ItemEquipped { .. }
            }
        ));
        world
            .apply(session, PlayerAction::Move(Direction::North))
            .unwrap();
        let arena = world.scene_for(session).unwrap();
        assert_eq!(arena.title, "Old Arena");
        assert_eq!(arena.npcs.len(), 1);
        assert!(!arena.npcs[0].id.contains("game:"));
        assert!(!arena.npcs[0].actions[0].command.contains("game:"));
        let attack = parse_action(&arena.npcs[0].actions[0].command).unwrap();
        world.apply(session, attack).unwrap();

        let mut combat_events = Vec::new();
        for _ in 0..5 {
            let events = world.advance_tick().unwrap();
            combat_events.extend(events);
            if world.scene_for(session).unwrap().npcs.is_empty() {
                break;
            }
        }
        assert!(combat_events.iter().any(|event| matches!(
            event,
            DomainEvent::Gameplay {
                event: GameplayEvent::NpcDefeated { .. }
            }
        )));
        assert!(combat_events.iter().any(|event| matches!(
            event,
            DomainEvent::Gameplay {
                event: GameplayEvent::LootDropped { .. }
            }
        )));
        let gameplay = world.character_gameplay_snapshot(session).unwrap();
        assert_eq!(gameplay.level, 2);
        assert_eq!(gameplay.experience, 120);
        assert_eq!(gameplay.skills.get("combat"), Some(&11));
        assert!(
            world
                .player_state_view(session)
                .unwrap()
                .character
                .skills
                .iter()
                .any(|skill| skill.id == "combat" && skill.label == "Combat" && skill.rank == 11)
        );
        let loot_scene = world.scene_for(session).unwrap();
        assert_eq!(loot_scene.items.len(), 1);
        assert_eq!(loot_scene.items[0].label, "silver fang");
        world
            .apply(session, PlayerAction::Take("fang".into()))
            .unwrap();
        assert!(
            world
                .inventory_as_plain_text(session)
                .unwrap()
                .contains("silver fang")
        );

        for _ in 0..3 {
            world.advance_tick().unwrap();
        }
        assert_eq!(world.scene_for(session).unwrap().npcs.len(), 1);
    }

    #[test]
    fn seeded_rng_produces_identical_combat_events() {
        let (mut first, first_session) = phase_two_world(77, 3, 30);
        let (mut second, second_session) = phase_two_world(77, 3, 30);
        for (world, session) in [(&mut first, first_session), (&mut second, second_session)] {
            world
                .apply(session, PlayerAction::Move(Direction::North))
                .unwrap();
            world
                .apply(session, PlayerAction::Attack("guardian".into()))
                .unwrap();
        }

        assert_eq!(
            first.advance_tick().unwrap(),
            second.advance_tick().unwrap()
        );
        assert_eq!(
            first.advance_tick().unwrap(),
            second.advance_tick().unwrap()
        );
        assert_eq!(first.logical_tick(), second.logical_tick());
    }

    #[test]
    fn use_effect_equip_and_unequip_update_gameplay_state() {
        let (mut world, session) = phase_two_world(9, 2, 20);
        let mut wounded = world.character_gameplay_snapshot(session).unwrap();
        wounded.vitals.health = 8;
        world
            .restore_character_gameplay(CharacterId(900), wounded)
            .unwrap();
        world
            .apply(session, PlayerAction::Take("potion".into()))
            .unwrap();
        let used = world
            .apply(session, PlayerAction::Use("potion".into()))
            .unwrap();
        assert!(used.domain_events.iter().any(|event| matches!(
            event,
            DomainEvent::Gameplay {
                event: GameplayEvent::EffectApplied { .. }
            }
        )));
        assert_eq!(
            world
                .character_gameplay_snapshot(session)
                .unwrap()
                .vitals
                .health,
            13
        );
        world.advance_tick().unwrap();
        let second_tick = world.advance_tick().unwrap();
        assert_eq!(
            world
                .character_gameplay_snapshot(session)
                .unwrap()
                .vitals
                .health,
            17
        );
        assert!(second_tick.iter().any(|event| matches!(
            event,
            DomainEvent::Gameplay {
                event: GameplayEvent::EffectExpired { .. }
            }
        )));

        world
            .apply(session, PlayerAction::Take("sword".into()))
            .unwrap();
        world
            .apply(session, PlayerAction::Equip("sword".into()))
            .unwrap();
        assert_eq!(
            world
                .character_gameplay_snapshot(session)
                .unwrap()
                .equipment[&EquipmentSlot::Weapon],
            "game:sword"
        );
        world
            .apply(session, PlayerAction::Unequip("weapon".into()))
            .unwrap();
        assert!(
            world
                .character_gameplay_snapshot(session)
                .unwrap()
                .equipment
                .is_empty()
        );
    }

    #[test]
    fn defeat_recovers_at_start_after_deterministic_ticks() {
        let (mut world, session) = phase_two_world(5, 50, 100);
        world
            .apply(session, PlayerAction::Move(Direction::North))
            .unwrap();
        world
            .apply(session, PlayerAction::Attack("guardian".into()))
            .unwrap();
        let defeat_tick = world.advance_tick().unwrap();
        assert!(defeat_tick.iter().any(|event| matches!(
            event,
            DomainEvent::Gameplay {
                event: GameplayEvent::CharacterDefeated { .. }
            }
        )));
        assert_eq!(
            world
                .character_gameplay_snapshot(session)
                .unwrap()
                .vitals
                .health,
            0
        );
        assert!(matches!(
            world.apply(session, PlayerAction::Move(Direction::South)),
            Err(WorldError::GameplayUnavailable)
        ));
        for _ in 0..3 {
            world.advance_tick().unwrap();
        }
        let recovered = world.character_gameplay_snapshot(session).unwrap();
        assert_eq!(recovered.vitals.health, recovered.vitals.max_health);
        assert!(recovered.recovery_at_tick.is_none());
        assert_eq!(world.scene_for(session).unwrap().title, "Sanctuary");
    }

    #[test]
    fn portable_snapshots_resume_combat_loot_xp_equipment_effects_and_rng() {
        let (mut original, session) = phase_two_world(41, 2, 3);
        original
            .apply(session, PlayerAction::Take("potion".into()))
            .unwrap();
        original
            .apply(session, PlayerAction::Use("potion".into()))
            .unwrap();
        original
            .apply(session, PlayerAction::Take("sword".into()))
            .unwrap();
        original
            .apply(session, PlayerAction::Equip("sword".into()))
            .unwrap();
        original
            .apply(session, PlayerAction::Move(Direction::North))
            .unwrap();
        original
            .apply(session, PlayerAction::Attack("guardian".into()))
            .unwrap();
        original.advance_tick().unwrap();

        let runtime_json = serde_json::to_string(&original.runtime_snapshot()).unwrap();
        let durable_json =
            serde_json::to_string(&original.durable_character_snapshot(session).unwrap()).unwrap();
        let runtime: WorldRuntimeSnapshot = serde_json::from_str(&runtime_json).unwrap();
        let durable: DurableCharacterSnapshot = serde_json::from_str(&durable_json).unwrap();

        let (mut resumed, resumed_session) = phase_two_world(999, 2, 3);
        let revision = runtime.revision;
        resumed.restore_runtime_snapshot(runtime).unwrap();
        resumed
            .restore_character(resumed_session, &durable.character, revision)
            .unwrap();
        resumed
            .restore_character_gameplay(CharacterId(900), durable.gameplay)
            .unwrap();

        assert_eq!(resumed.runtime_snapshot(), original.runtime_snapshot());
        assert_eq!(
            resumed
                .character_gameplay_snapshot(resumed_session)
                .unwrap(),
            original.character_gameplay_snapshot(session).unwrap()
        );
        assert_eq!(
            resumed.inventory_as_plain_text(resumed_session).unwrap(),
            original.inventory_as_plain_text(session).unwrap()
        );
        assert_eq!(
            resumed.advance_tick().unwrap(),
            original.advance_tick().unwrap()
        );
    }

    #[test]
    fn portable_snapshots_resume_defeat_recovery_and_npc_respawn_ticks() {
        let (mut original, session) = phase_two_world(5, 50, 100);
        original
            .apply(session, PlayerAction::Move(Direction::North))
            .unwrap();
        original
            .apply(session, PlayerAction::Attack("guardian".into()))
            .unwrap();
        original.advance_tick().unwrap();
        assert!(
            original
                .character_gameplay_snapshot(session)
                .unwrap()
                .recovery_at_tick
                .is_some()
        );

        let runtime = original.runtime_snapshot();
        let durable = original.durable_character_snapshot(session).unwrap();
        let (mut resumed, resumed_session) = phase_two_world(999, 50, 100);
        let revision = runtime.revision;
        resumed.restore_runtime_snapshot(runtime).unwrap();
        resumed
            .restore_character(resumed_session, &durable.character, revision)
            .unwrap();
        resumed
            .restore_character_gameplay(CharacterId(900), durable.gameplay)
            .unwrap();

        for _ in 0..4 {
            assert_eq!(
                resumed.advance_tick().unwrap(),
                original.advance_tick().unwrap()
            );
        }
        assert_eq!(
            resumed
                .character_gameplay_snapshot(resumed_session)
                .unwrap(),
            original.character_gameplay_snapshot(session).unwrap()
        );
        assert_eq!(
            resumed.scene_for(resumed_session).unwrap().title,
            "Sanctuary"
        );
    }

    #[test]
    fn checkpoint_restores_all_gameplay_and_observer_state() {
        let (mut world, session) = phase_two_world(17, 2, 20);
        world
            .apply(session, PlayerAction::Take("potion".into()))
            .unwrap();
        let before_runtime = world.runtime_snapshot();
        let before_gameplay = world.character_gameplay_snapshot(session).unwrap();
        let before_scene = world.scene_for(session).unwrap();
        let checkpoint = world.checkpoint();

        world
            .apply(session, PlayerAction::Use("potion".into()))
            .unwrap();
        world.advance_tick().unwrap();
        world.restore_checkpoint(checkpoint);

        assert_eq!(world.runtime_snapshot(), before_runtime);
        assert_eq!(
            world.character_gameplay_snapshot(session).unwrap(),
            before_gameplay
        );
        let restored_scene = world.scene_for(session).unwrap();
        assert_eq!(restored_scene.view_id, before_scene.view_id);
        assert_eq!(restored_scene.revision, before_scene.revision);
        assert_eq!(restored_scene.title, before_scene.title);
        assert_eq!(restored_scene.items, before_scene.items);
        assert_eq!(restored_scene.npcs, before_scene.npcs);
    }

    #[test]
    fn combat_view_events_are_opaque_and_perception_scoped() {
        let (mut world, session) = phase_two_world(13, 2, 20);
        world
            .apply(session, PlayerAction::Move(Direction::North))
            .unwrap();
        world
            .apply(session, PlayerAction::Attack("guardian".into()))
            .unwrap();
        let events = world.advance_tick().unwrap();
        let damage = events
            .iter()
            .find(|event| {
                matches!(
                    event,
                    DomainEvent::Gameplay {
                        event: GameplayEvent::NpcDamaged { .. }
                    }
                )
            })
            .unwrap();
        let view = world.project_event(session, damage).unwrap().unwrap();
        assert_ne!(view.actor_id, "900");
        assert!(!view.target_id.as_deref().unwrap().contains("game:"));
        assert!(!view.text.contains("game:"));
    }

    #[test]
    fn private_narration_uses_only_the_authorized_observers_opaque_identity() {
        let (world, session) = phase_two_world(13, 2, 20);
        let view = world
            .project_private_narration(session, "Only you hear the archive whisper.")
            .unwrap();

        assert_eq!(view.kind, ViewEventKind::Gameplay);
        assert_eq!(view.text, "Only you hear the archive whisper.");
        assert!(!view.actor_id.contains("900"));
        assert!(view.target_id.is_none());
        assert!(view.item_id.is_none());
        assert!(matches!(
            world.project_private_narration(session, "\u{1b}[31munsafe"),
            Err(WorldError::InvalidScriptEffect)
        ));
    }

    #[test]
    fn internal_clock_ticks_refresh_but_do_not_narrate_observer_views() {
        let (mut world, session) = phase_two_world(13, 2, 20);
        let before = world.scene_for(session).unwrap().revision;

        let events = world.advance_tick().unwrap();
        let tick = events
            .iter()
            .find(|event| {
                matches!(
                    event,
                    DomainEvent::Gameplay {
                        event: GameplayEvent::TickAdvanced { .. }
                    }
                )
            })
            .unwrap();

        assert_eq!(world.project_event(session, tick).unwrap(), None);
        assert!(world.scene_for(session).unwrap().revision > before);
    }

    #[test]
    fn perception_authorizes_scenes_actions_maps_and_events_before_projection() {
        let (mut world, actor) = phase_two_world(13, 2, 20);
        world.configure_hidden_item("game:sword", 20).unwrap();
        world
            .configure_secret_exit("game:start", Direction::North, 20)
            .unwrap();
        world.configure_hidden_npc("game:guardian", 20).unwrap();
        world
            .configure_room_light("game:arena", LightLevel::Dark)
            .unwrap();
        let arena_entity = world.stable_rooms["game:arena"];
        let arena = room_mut(&mut world.entities, arena_entity);
        arena.image = Some(SceneImage {
            alt: "A moonless arena.".into(),
            localized_alt: BTreeMap::from([("en".into(), "A moonless arena.".into())]),
            manifest: None,
            avif: Some("/assets/arena.avif".into()),
            webp: None,
            jpeg: None,
            width: Some(640),
            height: Some(360),
        });
        arena.audio = Some(SceneAudio {
            channel: "ambience".into(),
            url: format!("/assets/{}/arena.opus", "a".repeat(64)),
            looped: true,
        });

        let start_observer = SessionId(901);
        world
            .connect(
                start_observer,
                CharacterSnapshot {
                    id: CharacterId(901),
                    account_id: AccountId(91),
                    name: "Scout".into(),
                    room_id: "game:start".into(),
                    item_locations: BTreeMap::new(),
                },
            )
            .unwrap();
        let low_scene = world.scene_for(start_observer).unwrap();
        assert!(low_scene.exits.is_empty());
        assert_eq!(
            low_scene
                .items
                .iter()
                .map(|item| item.label.as_str())
                .collect::<Vec<_>>(),
            vec!["red potion"]
        );
        assert!(matches!(
            world.apply(start_observer, PlayerAction::Move(Direction::North)),
            Err(WorldError::NoExit)
        ));
        assert!(matches!(
            world.apply(start_observer, PlayerAction::Take("sword".into())),
            Err(WorldError::ItemNotFound(_))
        ));
        assert!(
            world
                .player_state_view(start_observer)
                .unwrap()
                .map
                .paths
                .is_empty()
        );

        world.set_observer_role(actor, AccountRole::Admin).unwrap();
        let admin_scene = world.scene_for(actor).unwrap();
        assert_eq!(admin_scene.exits.len(), 1);
        assert!(
            admin_scene
                .items
                .iter()
                .any(|item| item.label == "iron sword")
        );
        assert!(!admin_scene.exits[0].id.contains("game:"));

        let item_event = world
            .apply(actor, PlayerAction::Take("sword".into()))
            .unwrap()
            .domain_events
            .remove(0);
        assert_eq!(
            world.project_event(start_observer, &item_event).unwrap(),
            None
        );
        let move_event = world
            .apply(actor, PlayerAction::Move(Direction::North))
            .unwrap()
            .domain_events
            .remove(0);
        assert_eq!(
            world.project_event(start_observer, &move_event).unwrap(),
            None
        );

        let arena_observer = SessionId(902);
        world
            .connect(
                arena_observer,
                CharacterSnapshot {
                    id: CharacterId(902),
                    account_id: AccountId(92),
                    name: "Listener".into(),
                    room_id: "game:arena".into(),
                    item_locations: BTreeMap::new(),
                },
            )
            .unwrap();
        let dark_scene = world.scene_for(arena_observer).unwrap();
        assert!(dark_scene.npcs.is_empty());
        assert!(!dark_scene.prose.join(" ").contains("Claw marks"));
        assert!(dark_scene.image.is_none());
        assert!(dark_scene.audio.is_some());

        let combat_event = world
            .apply(actor, PlayerAction::Attack("guardian".into()))
            .unwrap()
            .domain_events
            .remove(0);
        assert_eq!(
            world.project_event(arena_observer, &combat_event).unwrap(),
            None
        );

        let mut perceptive = world.character_gameplay_snapshot(arena_observer).unwrap();
        perceptive.skills.insert("perception".into(), 100);
        perceptive.script_flags.insert("sense.darkvision".into());
        world
            .restore_character_gameplay(CharacterId(902), perceptive.clone())
            .unwrap();
        let darkvision_scene = world.scene_for(arena_observer).unwrap();
        assert_eq!(darkvision_scene.npcs.len(), 1);
        assert!(darkvision_scene.prose.join(" ").contains("Claw marks"));
        assert!(darkvision_scene.image.is_some());

        perceptive.script_flags.insert("condition.blind".into());
        world
            .restore_character_gameplay(CharacterId(902), perceptive.clone())
            .unwrap();
        let blind_scene = world.scene_for(arena_observer).unwrap();
        assert!(blind_scene.npcs.is_empty());
        assert!(blind_scene.image.is_none());
        assert!(blind_scene.audio.is_some());
        perceptive.script_flags.insert("sense.tremorsense".into());
        world
            .restore_character_gameplay(CharacterId(902), perceptive.clone())
            .unwrap();
        assert_eq!(world.scene_for(arena_observer).unwrap().npcs.len(), 1);
        perceptive.script_flags.insert("condition.deaf".into());
        world
            .restore_character_gameplay(CharacterId(902), perceptive)
            .unwrap();
        assert!(world.scene_for(arena_observer).unwrap().audio.is_none());
    }

    #[test]
    fn rejects_invalid_item_definitions_and_room_references() {
        let room = |item_ids| RoomDefinition {
            stable_id: "test:start".into(),
            title: "Start".into(),
            prose: Vec::new(),
            image: None,
            audio: None,
            exits: Vec::new(),
            item_ids,
        };
        let item = ItemDefinition {
            stable_id: "test:item".into(),
            name: "item".into(),
            description: "An item.".into(),
            aliases: Vec::new(),
        };

        assert!(matches!(
            World::from_definition(WorldDefinition {
                start_room_id: "test:start".into(),
                items: Vec::new(),
                rooms: vec![room(vec!["missing".into()])],
            }),
            Err(WorldBuildError::UnknownRoomItem { .. })
        ));
        assert!(matches!(
            World::from_definition(WorldDefinition {
                start_room_id: "test:start".into(),
                items: vec![item.clone()],
                rooms: vec![room(Vec::new())],
            }),
            Err(WorldBuildError::UnplacedItem(item_id)) if item_id == "test:item"
        ));
        assert!(matches!(
            World::from_definition(WorldDefinition {
                start_room_id: "test:start".into(),
                items: vec![item],
                rooms: vec![room(vec!["test:item".into(), "test:item".into()])],
            }),
            Err(WorldBuildError::DuplicateRoomItem { .. })
        ));

        let item = ItemDefinition {
            stable_id: "test:item".into(),
            name: "item".into(),
            description: "An item.".into(),
            aliases: Vec::new(),
        };
        assert!(matches!(
            World::from_definition_with_staged_items(
                WorldDefinition {
                    start_room_id: "test:start".into(),
                    items: vec![item],
                    rooms: vec![room(vec!["test:item".into()])],
                },
                vec!["test:item".into()],
            ),
            Err(WorldBuildError::ItemPlacedAndStaged { item_id, .. })
                if item_id == "test:item"
        ));
    }

    #[test]
    fn staged_npc_loot_is_neither_visible_nor_interactable_before_spawn() {
        let mut world = World::from_definition_with_staged_items(
            WorldDefinition {
                start_room_id: "test:start".into(),
                items: vec![ItemDefinition {
                    stable_id: "test:loot".into(),
                    name: "silver fang".into(),
                    description: "A trophy carried by a guardian.".into(),
                    aliases: vec!["fang".into()],
                }],
                rooms: vec![RoomDefinition {
                    stable_id: "test:start".into(),
                    title: "Start".into(),
                    prose: Vec::new(),
                    image: None,
                    audio: None,
                    exits: Vec::new(),
                    item_ids: Vec::new(),
                }],
            },
            vec!["test:loot".into()],
        )
        .unwrap();
        let session = SessionId(91);
        world
            .connect(
                session,
                CharacterSnapshot {
                    id: CharacterId(92),
                    account_id: AccountId(93),
                    name: "Observer".into(),
                    room_id: "test:start".into(),
                    item_locations: BTreeMap::new(),
                },
            )
            .unwrap();

        assert!(world.scene_for(session).unwrap().items.is_empty());
        assert!(matches!(
            world.apply(session, PlayerAction::Take("fang".into())),
            Err(WorldError::ItemNotFound(query)) if query == "fang"
        ));
        world
            .spawn_npc(NpcDefinition {
                stable_id: "test:guardian".into(),
                name: "guardian".into(),
                description: "A silent guardian.".into(),
                room_id: "test:start".into(),
                level: 1,
                max_health: 1,
                strength: 1,
                defence: 0,
                experience_reward: 1,
                loot_item_ids: vec!["test:loot".into()],
                respawn_ticks: 1,
            })
            .unwrap();
        assert_eq!(
            world.runtime_snapshot().item_locations["test:loot"],
            RuntimeItemLocation::HeldByNpc {
                npc_id: "test:guardian".into(),
            }
        );
    }

    #[test]
    fn parses_adventure_commands_without_losing_targets() {
        assert_eq!(
            parse_action("party invite Aria").unwrap(),
            PlayerAction::Party(PartyAction::Invite("Aria".into()))
        );
        assert_eq!(
            parse_action("quest accept first hunt").unwrap(),
            PlayerAction::Quest(QuestAction::Accept("first hunt".into()))
        );
        assert_eq!(
            parse_action("buy healing tonic").unwrap(),
            PlayerAction::Economy(EconomyAction::Buy("healing tonic".into()))
        );
        assert_eq!(parse_action("weather").unwrap(), PlayerAction::Weather);
    }

    #[test]
    fn party_loop_enforces_leader_authority_and_projects_opaque_ids() {
        let (mut world, leader_session) = connected_world();
        let member_session = SessionId(8);
        world
            .connect(
                member_session,
                CharacterSnapshot {
                    id: CharacterId(102),
                    account_id: AccountId(11),
                    name: "Aria".into(),
                    room_id: world.start_room_id().into(),
                    item_locations: BTreeMap::new(),
                },
            )
            .unwrap();
        let outsider_session = SessionId(9);
        world
            .connect(
                outsider_session,
                CharacterSnapshot {
                    id: CharacterId(103),
                    account_id: AccountId(12),
                    name: "Bram".into(),
                    room_id: world.start_room_id().into(),
                    item_locations: BTreeMap::new(),
                },
            )
            .unwrap();

        let created = world
            .apply(leader_session, PlayerAction::Party(PartyAction::Create))
            .unwrap();
        let view = world
            .project_event(leader_session, &created.domain_events[0])
            .unwrap()
            .unwrap();
        assert_eq!(view.kind, ViewEventKind::Adventure);
        assert!(!view.actor_id.contains("101"));
        assert_ne!(view.target_id.unwrap(), "1");

        world
            .apply(
                leader_session,
                PlayerAction::Party(PartyAction::Invite("Bram".into())),
            )
            .unwrap();
        let invited_member = world
            .apply(
                leader_session,
                PlayerAction::Party(PartyAction::Invite("Aria".into())),
            )
            .unwrap();
        assert!(
            world
                .project_event(outsider_session, &invited_member.domain_events[0])
                .unwrap()
                .is_none()
        );
        let revoked = world
            .apply(
                leader_session,
                PlayerAction::Party(PartyAction::Revoke("Bram".into())),
            )
            .unwrap();
        assert!(
            world
                .project_event(outsider_session, &revoked.domain_events[0])
                .unwrap()
                .is_some()
        );
        let accepted = world
            .apply(
                member_session,
                PlayerAction::Party(PartyAction::Accept("Tester".into())),
            )
            .unwrap();
        assert!(
            world
                .project_event(leader_session, &accepted.domain_events[0])
                .unwrap()
                .is_some()
        );
        assert!(
            world
                .project_event(outsider_session, &accepted.domain_events[0])
                .unwrap()
                .is_none()
        );
        assert!(matches!(
            world.apply(
                member_session,
                PlayerAction::Party(PartyAction::Invite("Bram".into()))
            ),
            Err(WorldError::PartyForbidden)
        ));
        assert!(
            world
                .apply(member_session, PlayerAction::Party(PartyAction::Status))
                .unwrap()
                .output
                .unwrap()
                .contains("Tester, Aria")
        );
        let json = serde_json::to_string(&world.runtime_snapshot()).unwrap();
        let restored: WorldRuntimeSnapshot = serde_json::from_str(&json).unwrap();
        assert_eq!(restored.adventure.parties.len(), 1);

        assert!(matches!(
            world.apply(
                member_session,
                PlayerAction::Party(PartyAction::Kick("Tester".into()))
            ),
            Err(WorldError::PartyForbidden)
        ));
        world
            .apply(
                leader_session,
                PlayerAction::Party(PartyAction::Transfer("Aria".into())),
            )
            .unwrap();
        assert!(
            world
                .player_state_view(member_session)
                .unwrap()
                .party
                .unwrap()
                .leader
        );
        assert!(matches!(
            world.apply(
                leader_session,
                PlayerAction::Party(PartyAction::Kick("Aria".into()))
            ),
            Err(WorldError::PartyForbidden)
        ));
        let kicked = world
            .apply(
                member_session,
                PlayerAction::Party(PartyAction::Kick("Tester".into())),
            )
            .unwrap();
        assert!(
            world
                .project_event(leader_session, &kicked.domain_events[0])
                .unwrap()
                .is_some()
        );
        assert!(
            world
                .player_state_view(leader_session)
                .unwrap()
                .party
                .is_none()
        );
        world
            .apply(member_session, PlayerAction::Party(PartyAction::Disband))
            .unwrap();
        assert!(
            world
                .player_state_view(member_session)
                .unwrap()
                .party
                .is_none()
        );
    }

    #[test]
    fn economy_buy_sell_and_checkpoint_are_bounded_and_reversible() {
        let (mut world, session) = connected_world();
        let checkpoint = world.checkpoint();
        world
            .apply(
                session,
                PlayerAction::Economy(EconomyAction::Buy("tonic".into())),
            )
            .unwrap();
        let after_buy = world.character_gameplay_snapshot(session).unwrap();
        assert_eq!(after_buy.coins, 28);
        assert_eq!(after_buy.commodities["tonic"], 1);
        assert_eq!(world.runtime_snapshot().adventure.shop_stock["tonic"], 19);
        world
            .apply(
                session,
                PlayerAction::Economy(EconomyAction::Sell("tonic".into())),
            )
            .unwrap();
        assert_eq!(
            world.character_gameplay_snapshot(session).unwrap().coins,
            34
        );
        world.restore_checkpoint(checkpoint);
        assert_eq!(
            world.character_gameplay_snapshot(session).unwrap().coins,
            40
        );
        assert_eq!(world.runtime_snapshot().adventure.shop_stock["tonic"], 20);
    }

    #[test]
    fn quest_progresses_from_combat_and_reward_can_only_be_claimed_once() {
        let (mut world, session) = phase_two_world(5, 1, 1);
        world
            .apply(
                session,
                PlayerAction::Quest(QuestAction::Accept("first-hunt".into())),
            )
            .unwrap();
        world
            .apply(session, PlayerAction::Move(Direction::North))
            .unwrap();
        world
            .apply(session, PlayerAction::Attack("guardian".into()))
            .unwrap();
        world.advance_tick().unwrap();
        let ready = world.character_gameplay_snapshot(session).unwrap();
        assert_eq!(ready.quests["first-hunt"].status, QuestStatus::ReadyToClaim);
        let coins = ready.coins;
        let experience = ready.experience;
        world
            .apply(
                session,
                PlayerAction::Quest(QuestAction::Complete("first-hunt".into())),
            )
            .unwrap();
        let completed = world.character_gameplay_snapshot(session).unwrap();
        assert_eq!(completed.coins, coins + 25);
        assert_eq!(completed.experience, experience + 50);
        assert!(matches!(
            world.apply(
                session,
                PlayerAction::Quest(QuestAction::Complete("first-hunt".into()))
            ),
            Err(WorldError::QuestUnavailable)
        ));
    }

    #[test]
    fn weather_timer_is_deterministic_and_survives_runtime_restore() {
        let (mut first, _) = connected_world();
        let (mut second, _) = connected_world();
        first.set_rng_seed(777);
        second.set_rng_seed(777);
        for _ in 0..5 {
            assert_eq!(
                first.advance_tick().unwrap(),
                second.advance_tick().unwrap()
            );
        }
        assert_ne!(
            first.runtime_snapshot().adventure.weather,
            WeatherKind::Rain
        );

        let snapshot = first.runtime_snapshot();
        let (mut restored, _) = connected_world();
        restored.restore_runtime_snapshot(snapshot).unwrap();
        assert_eq!(
            restored.runtime_snapshot().adventure,
            first.runtime_snapshot().adventure
        );
    }

    #[test]
    fn player_state_is_complete_bounded_and_never_serialises_durable_entity_ids() {
        let (mut world, session) = connected_world();
        let initial = world.player_state_view(session).unwrap();
        assert_eq!(initial.character.name, "Tester");
        assert_eq!(initial.character.health, 20);
        assert_eq!(initial.map.rooms.len(), 1);
        assert!(initial.map.rooms[0].current);
        assert_eq!(initial.quests[0].status, "available");
        assert_eq!(initial.economy.coins, 40);
        assert_eq!(initial.social.online.len(), 1);

        world
            .apply(session, PlayerAction::Take("lantern".into()))
            .unwrap();
        world
            .apply(session, PlayerAction::Move(Direction::North))
            .unwrap();
        let discovered = world.player_state_view(session).unwrap();
        assert_eq!(discovered.inventory.len(), 1);
        assert_eq!(discovered.map.rooms.len(), 2);
        assert_eq!(
            discovered
                .map
                .rooms
                .iter()
                .filter(|room| room.current)
                .count(),
            1
        );
        assert!(
            discovered
                .inventory
                .iter()
                .all(|item| item.id.starts_with('o') && item.id.len() == 17)
        );
        assert!(
            discovered
                .equipment
                .iter()
                .all(|item| item.id.starts_with('o') && item.id.len() == 17)
        );

        let json = serde_json::to_string(&discovered).unwrap();
        assert!(!json.contains("demo:lantern-square"));
        assert!(!json.contains("demo:north-gate"));
        assert!(!json.contains("demo:blue-lantern"));
        assert!(!json.contains("accountId"));
        assert!(!json.contains("characterId"));
        assert!(!json.contains("partyId"));
        assert!(!json.contains("\"id\":101"));
    }
}
