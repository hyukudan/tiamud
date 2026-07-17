//! Transport-independent world model for Tiamud.

mod ability;
mod progression;

pub use ability::*;
pub use progression::*;

use rand_core::{OsRng, RngCore};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use slotmap::{SlotMap, new_key_type};
use std::borrow::Borrow;
use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::fmt::Write as _;
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
    Take(ItemSelection),
    Drop(ItemSelection),
    Contents {
        container: String,
        page: u16,
    },
    Put {
        selection: ItemSelection,
        container: String,
    },
    Get {
        selection: ItemSelection,
        container: String,
    },
    Give {
        selection: ItemSelection,
        recipient: String,
    },
    Loot(LootAction),
    Trade(TradeAction),
    OpenReward(String),
    RewardOdds {
        target: String,
        page: u16,
    },
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
    Staff(StaffAction),
}

pub const MAX_ITEM_SELECTION_QUANTITY: u16 = 256;

pub const MAX_LOOT_UNITS_PER_RESOLUTION: usize = 64;
pub const MAX_SEALED_REWARD_UNITS: usize = 16;
pub const MAX_AFFIXES_PER_ITEM: usize = 1;
pub const MAX_AFFIX_DEFINITIONS: usize = 512;
pub const MAX_AFFIX_MODIFIER: i32 = 1_000;
pub const MAX_AFFIX_POOLS: usize = 256;
pub const MAX_AFFIX_POOL_ENTRIES: usize = 64;
pub const MAX_TOTAL_AFFIX_POOL_ENTRIES: usize = 4_096;
pub const MAX_REWARD_ODDS_ROWS: usize = 1_024;
pub const REWARD_ODDS_PAGE_SIZE: usize = 20;
pub const MAX_GROUND_LOOT_RESERVATIONS: usize = 2_048;
pub const MAX_LOOT_ENCOUNTERS: usize = 2_048;
pub const LOOT_EXCLUSIVE_TICKS: u64 = 20;
pub const LOOT_DESPAWN_TICKS: u64 = 120;

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum LootAction {
    Status,
    Pass(String),
}

pub const MAX_TRADE_ITEM_QUANTITY: u16 = 128;
pub const MAX_TRADE_COINS: u64 = 1_000_000_000;
pub const MAX_LIVE_TRADES: usize = 64;
pub const MAX_TRADE_LINES_PER_PARTICIPANT: usize = 8;
pub const MAX_TRADE_ROOTS: usize = 128;
pub const MAX_RESERVED_TRADE_ROOTS: usize = 2_048;
pub const TRADE_REQUEST_TICKS: u64 = 20;
pub const TRADE_NEGOTIATION_TICKS: u64 = 120;
pub const TRADE_REQUEST_RATE_TICKS: u64 = 5;
pub const MAX_TRADE_STATUS_INVENTORY_GROUPS: usize = 20;
pub const MAX_TRADE_STATUS_TEXT_BYTES: usize = 16 * 1024;

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum TradeAction {
    Request(String),
    Accept,
    Decline,
    OfferItem(ItemSelection),
    RemoveItem(String),
    OfferCoins(u64),
    Status,
    Contents { container: String, page: u16 },
    Confirm { state_revision: u64 },
    Cancel,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ItemSelection {
    pub query: String,
    pub quantity: u16,
}

impl ItemSelection {
    pub fn one(query: impl Into<String>) -> Self {
        Self {
            query: query.into(),
            quantity: 1,
        }
    }
}

impl From<String> for ItemSelection {
    fn from(query: String) -> Self {
        Self::one(query)
    }
}

impl From<&str> for ItemSelection {
    fn from(query: &str) -> Self {
        Self::one(query)
    }
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

/// Closed, durable authorization vocabulary for in-game staff work.
///
/// These capabilities are assigned to accounts by administrators. They are
/// deliberately unrelated to character level, class, talents or WorldPack
/// content.
#[derive(Clone, Copy, Debug, Deserialize, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize)]
pub enum StaffCapability {
    #[serde(rename = "reports.view")]
    ReportsView,
    #[serde(rename = "reports.resolve")]
    ReportsResolve,
    #[serde(rename = "moderation.mute")]
    ModerationMute,
    #[serde(rename = "moderation.ban")]
    ModerationBan,
    #[serde(rename = "staff.assign")]
    StaffAssign,
    #[serde(rename = "staff.inspect")]
    StaffInspect,
    #[serde(rename = "staff.where")]
    StaffWhere,
    #[serde(rename = "staff.goto")]
    StaffGoto,
    #[serde(rename = "staff.transfer")]
    StaffTransfer,
    #[serde(rename = "staff.restore")]
    StaffRestore,
    #[serde(rename = "staff.announce")]
    StaffAnnounce,
    #[serde(rename = "staff.audit")]
    StaffAudit,
}

impl StaffCapability {
    pub const ALL: [Self; 12] = [
        Self::ReportsView,
        Self::ReportsResolve,
        Self::ModerationMute,
        Self::ModerationBan,
        Self::StaffAssign,
        Self::StaffInspect,
        Self::StaffWhere,
        Self::StaffGoto,
        Self::StaffTransfer,
        Self::StaffRestore,
        Self::StaffAnnounce,
        Self::StaffAudit,
    ];

    pub const fn as_str(self) -> &'static str {
        match self {
            Self::ReportsView => "reports.view",
            Self::ReportsResolve => "reports.resolve",
            Self::ModerationMute => "moderation.mute",
            Self::ModerationBan => "moderation.ban",
            Self::StaffAssign => "staff.assign",
            Self::StaffInspect => "staff.inspect",
            Self::StaffWhere => "staff.where",
            Self::StaffGoto => "staff.goto",
            Self::StaffTransfer => "staff.transfer",
            Self::StaffRestore => "staff.restore",
            Self::StaffAnnounce => "staff.announce",
            Self::StaffAudit => "staff.audit",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        Self::ALL
            .into_iter()
            .find(|capability| capability.as_str().eq_ignore_ascii_case(value.trim()))
    }
}

/// Operational profile used to seed a reviewed capability bundle.
/// `lore_title` is stored separately and never authorizes an action.
#[derive(Clone, Copy, Debug, Deserialize, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum StaffProfileKey {
    Helper,
    Builder,
    Moderator,
    Administrator,
    Implementor,
}

impl StaffProfileKey {
    pub const ALL: [Self; 5] = [
        Self::Helper,
        Self::Builder,
        Self::Moderator,
        Self::Administrator,
        Self::Implementor,
    ];

    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Helper => "helper",
            Self::Builder => "builder",
            Self::Moderator => "moderator",
            Self::Administrator => "administrator",
            Self::Implementor => "implementor",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        Self::ALL
            .into_iter()
            .find(|profile| profile.as_str().eq_ignore_ascii_case(value.trim()))
    }

    pub fn default_capabilities(self) -> &'static [StaffCapability] {
        const HELPER: &[StaffCapability] = &[
            StaffCapability::ReportsView,
            StaffCapability::ReportsResolve,
        ];
        const BUILDER: &[StaffCapability] = &[];
        const MODERATOR: &[StaffCapability] = &[
            StaffCapability::ReportsView,
            StaffCapability::ReportsResolve,
            StaffCapability::ModerationMute,
            StaffCapability::ModerationBan,
        ];
        match self {
            Self::Helper => HELPER,
            Self::Builder => BUILDER,
            Self::Moderator => MODERATOR,
            Self::Administrator | Self::Implementor => &StaffCapability::ALL,
        }
    }

    /// Explicit staff hierarchy for negative authorization. Declaration order
    /// is presentation only and must never become policy accidentally.
    pub const fn authority_rank(self) -> u8 {
        match self {
            Self::Helper => 10,
            Self::Builder => 20,
            Self::Moderator => 30,
            Self::Administrator => 40,
            Self::Implementor => 50,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum StaffAction {
    Status,
    Reports {
        reason: String,
    },
    ResolveReport {
        report_id: i64,
        resolution: StaffReportResolution,
        reason: String,
    },
    Inspect {
        target: String,
        reason: String,
    },
    Where {
        target: String,
        reason: String,
    },
    Goto {
        target: String,
        reason: String,
    },
    Transfer {
        target: String,
        reason: String,
    },
    Restore {
        target: String,
        reason: String,
    },
    Announce {
        message: String,
        reason: String,
    },
    Audit {
        reason: String,
    },
    SetProfile {
        target: String,
        profile: Option<StaffProfileKey>,
        lore_title: Option<String>,
        reason: String,
    },
    SetCapability {
        target: String,
        capability: StaffCapability,
        enabled: bool,
        reason: String,
    },
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum StaffReportResolution {
    Dismissed,
    ActionTaken,
    NoAction,
}

impl StaffReportResolution {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Dismissed => "dismissed",
            Self::ActionTaken => "action_taken",
            Self::NoAction => "no_action",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        match value.trim().to_ascii_lowercase().as_str() {
            "dismissed" => Some(Self::Dismissed),
            "action_taken" | "action-taken" => Some(Self::ActionTaken),
            "no_action" | "no-action" => Some(Self::NoAction),
            _ => None,
        }
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
    Unmute {
        target: String,
        reason: String,
    },
    Report {
        target: String,
        reason: String,
    },
    Ban {
        target: String,
        duration_seconds: Option<u64>,
        reason: String,
    },
    Unban {
        target: String,
        reason: String,
    },
    SetRole {
        target: String,
        role: AccountRole,
        reason: String,
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
    if matches!(normalised.as_str(), "loot" | "loot status") {
        return Ok(PlayerAction::Loot(LootAction::Status));
    }
    if let Some(target) = command_tail_phrase(input, "loot pass") {
        return Ok(PlayerAction::Loot(LootAction::Pass(target)));
    }
    if normalised == "loot pass" {
        return Err(ParseActionError::MissingTarget { command: "loot" });
    }
    match normalised.as_str() {
        "trade accept" => return Ok(PlayerAction::Trade(TradeAction::Accept)),
        "trade decline" => return Ok(PlayerAction::Trade(TradeAction::Decline)),
        "trade status" | "trade" => return Ok(PlayerAction::Trade(TradeAction::Status)),
        "trade cancel" => return Ok(PlayerAction::Trade(TradeAction::Cancel)),
        _ => {}
    }
    if let Some(target) = command_tail_phrase(input, "trade request") {
        return Ok(PlayerAction::Trade(TradeAction::Request(target)));
    }
    if normalised == "trade request" {
        return Err(ParseActionError::MissingTarget { command: "trade" });
    }
    if let Some(value) = command_tail_phrase(input, "trade offer item") {
        let selection = parse_item_selection(&value, input)?;
        if selection.quantity > MAX_TRADE_ITEM_QUANTITY {
            return Err(ParseActionError::Unknown(input.to_owned()));
        }
        return Ok(PlayerAction::Trade(TradeAction::OfferItem(selection)));
    }
    if normalised == "trade offer item" {
        return Err(ParseActionError::MissingTarget { command: "trade" });
    }
    if let Some(target) = command_tail_phrase(input, "trade remove item") {
        return Ok(PlayerAction::Trade(TradeAction::RemoveItem(target)));
    }
    if normalised == "trade remove item" {
        return Err(ParseActionError::MissingTarget { command: "trade" });
    }
    if let Some(value) = command_tail_phrase(input, "trade offer coins") {
        let coins = value
            .parse::<u64>()
            .ok()
            .filter(|coins| *coins <= MAX_TRADE_COINS)
            .ok_or_else(|| ParseActionError::Unknown(input.to_owned()))?;
        return Ok(PlayerAction::Trade(TradeAction::OfferCoins(coins)));
    }
    if normalised == "trade offer coins" {
        return Err(ParseActionError::MissingTarget { command: "trade" });
    }
    if let Some(value) = command_tail_phrase(input, "trade confirm") {
        let state_revision = value
            .parse::<u64>()
            .ok()
            .filter(|revision| *revision > 0)
            .ok_or_else(|| ParseActionError::Unknown(input.to_owned()))?;
        return Ok(PlayerAction::Trade(TradeAction::Confirm { state_revision }));
    }
    if normalised == "trade confirm" {
        return Err(ParseActionError::MissingTarget { command: "trade" });
    }
    if let Some(value) = command_tail_phrase(input, "trade contents") {
        let mut parts = value.rsplitn(2, char::is_whitespace);
        let last = parts.next().unwrap_or_default().trim();
        let previous = parts
            .next()
            .map(str::trim)
            .filter(|value| !value.is_empty());
        let (container, page) = match (previous, last.parse::<u16>().ok()) {
            (Some(container), Some(page)) if page > 0 => (container.to_owned(), page),
            _ => (value, 1),
        };
        return Ok(PlayerAction::Trade(TradeAction::Contents {
            container,
            page,
        }));
    }
    if normalised == "trade contents" {
        return Err(ParseActionError::MissingTarget { command: "trade" });
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
    ] {
        if normalised == command {
            return Err(ParseActionError::MissingTarget { command });
        }
        if let Some(target) = command_tail(input, command) {
            return Ok(PlayerAction::Social(constructor(target)));
        }
    }
    for (command, is_unban) in [("unmute", false), ("unban", true)] {
        if normalised == command {
            return Err(ParseActionError::MissingTarget { command });
        }
        if let Some(rest) = command_tail(input, command) {
            let Some((target, reason)) = rest.split_once(char::is_whitespace) else {
                return Err(ParseActionError::MissingTarget { command });
            };
            let reason = reason.trim();
            if target.is_empty() || reason.is_empty() {
                return Err(ParseActionError::MissingTarget { command });
            }
            return Ok(PlayerAction::Social(if is_unban {
                SocialAction::Unban {
                    target: target.to_owned(),
                    reason: reason.to_owned(),
                }
            } else {
                SocialAction::Unmute {
                    target: target.to_owned(),
                    reason: reason.to_owned(),
                }
            }));
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
        let mut parts = rest.splitn(3, char::is_whitespace);
        let target = parts.next().unwrap_or_default().trim();
        let role = parts.next().unwrap_or_default().trim();
        let reason = parts.next().unwrap_or_default().trim();
        if target.is_empty() || role.is_empty() || reason.is_empty() {
            return Err(ParseActionError::MissingTarget { command: "role" });
        }
        let role = match role.to_ascii_lowercase().as_str() {
            "player" => AccountRole::Player,
            "moderator" | "mod" => AccountRole::Moderator,
            "admin" => AccountRole::Admin,
            _ => return Err(ParseActionError::Unknown(input.to_owned())),
        };
        return Ok(PlayerAction::Social(SocialAction::SetRole {
            target: target.to_owned(),
            role,
            reason: reason.to_owned(),
        }));
    }
    if normalised == "role" {
        return Err(ParseActionError::MissingTarget { command: "role" });
    }
    if matches!(normalised.as_str(), "staff" | "staff status") {
        return Ok(PlayerAction::Staff(StaffAction::Status));
    }
    if let Some(reason) = command_tail_phrase(input, "staff reports") {
        return Ok(PlayerAction::Staff(StaffAction::Reports { reason }));
    }
    if normalised == "staff reports" {
        return Err(ParseActionError::MissingTarget { command: "staff" });
    }
    if let Some(rest) = command_tail_phrase(input, "staff resolve") {
        let mut parts = rest.splitn(3, char::is_whitespace);
        let report_id = parts.next().unwrap_or_default().trim();
        let resolution = parts.next().unwrap_or_default().trim();
        let reason = parts.next().unwrap_or_default().trim();
        if report_id.is_empty() || resolution.is_empty() || reason.is_empty() {
            return Err(ParseActionError::MissingTarget { command: "staff" });
        }
        let report_id = report_id
            .parse::<i64>()
            .ok()
            .filter(|report_id| *report_id > 0)
            .ok_or_else(|| ParseActionError::Unknown(input.to_owned()))?;
        let resolution = StaffReportResolution::parse(resolution)
            .ok_or_else(|| ParseActionError::Unknown(input.to_owned()))?;
        return Ok(PlayerAction::Staff(StaffAction::ResolveReport {
            report_id,
            resolution,
            reason: reason.to_owned(),
        }));
    }
    if normalised == "staff resolve" {
        return Err(ParseActionError::MissingTarget { command: "staff" });
    }
    type StaffTargetConstructor = fn(String, String) -> StaffAction;
    let staff_target_commands: [(&str, StaffTargetConstructor); 5] = [
        ("staff inspect", |target, reason| StaffAction::Inspect {
            target,
            reason,
        }),
        ("staff where", |target, reason| StaffAction::Where {
            target,
            reason,
        }),
        ("staff goto", |target, reason| StaffAction::Goto {
            target,
            reason,
        }),
        ("staff transfer", |target, reason| StaffAction::Transfer {
            target,
            reason,
        }),
        ("staff restore", |target, reason| StaffAction::Restore {
            target,
            reason,
        }),
    ];
    for (command, constructor) in staff_target_commands {
        if let Some(rest) = command_tail_phrase(input, command) {
            let (target, reason) = split_staff_reason(&rest)
                .ok_or(ParseActionError::MissingTarget { command: "staff" })?;
            return Ok(PlayerAction::Staff(constructor(target, reason)));
        }
        if normalised == command {
            return Err(ParseActionError::MissingTarget { command: "staff" });
        }
    }
    if let Some(rest) = command_tail_phrase(input, "staff announce") {
        let (message, reason) = split_staff_reason(&rest)
            .ok_or(ParseActionError::MissingTarget { command: "staff" })?;
        return Ok(PlayerAction::Staff(StaffAction::Announce {
            message,
            reason,
        }));
    }
    if normalised == "staff announce" {
        return Err(ParseActionError::MissingTarget { command: "staff" });
    }
    if let Some(reason) = command_tail_phrase(input, "staff audit") {
        return Ok(PlayerAction::Staff(StaffAction::Audit { reason }));
    }
    if normalised == "staff audit" {
        return Err(ParseActionError::MissingTarget { command: "staff" });
    }
    if let Some(rest) = command_tail_phrase(input, "staff profile") {
        let (body, reason) = split_staff_reason(&rest)
            .ok_or(ParseActionError::MissingTarget { command: "staff" })?;
        let mut parts = body.splitn(3, char::is_whitespace);
        let target = parts.next().unwrap_or_default().trim();
        let profile = parts.next().unwrap_or_default().trim();
        let lore_title = parts.next().unwrap_or_default().trim();
        if target.is_empty() || profile.is_empty() || lore_title.is_empty() {
            return Err(ParseActionError::MissingTarget { command: "staff" });
        }
        let profile = if profile.eq_ignore_ascii_case("none") {
            None
        } else {
            Some(
                StaffProfileKey::parse(profile)
                    .ok_or_else(|| ParseActionError::Unknown(input.to_owned()))?,
            )
        };
        let lore_title = (!lore_title.eq_ignore_ascii_case("none")).then(|| lore_title.to_owned());
        return Ok(PlayerAction::Staff(StaffAction::SetProfile {
            target: target.to_owned(),
            profile,
            lore_title,
            reason,
        }));
    }
    if normalised == "staff profile" {
        return Err(ParseActionError::MissingTarget { command: "staff" });
    }
    if let Some(rest) = command_tail_phrase(input, "staff capability") {
        let (body, reason) = split_staff_reason(&rest)
            .ok_or(ParseActionError::MissingTarget { command: "staff" })?;
        let mut parts = body.split_whitespace();
        let operation = parts.next().unwrap_or_default();
        let target = parts.next().unwrap_or_default();
        let capability = parts.next().unwrap_or_default();
        if target.is_empty() || capability.is_empty() || parts.next().is_some() {
            return Err(ParseActionError::MissingTarget { command: "staff" });
        }
        let enabled = match operation.to_ascii_lowercase().as_str() {
            "grant" => true,
            "revoke" => false,
            _ => return Err(ParseActionError::Unknown(input.to_owned())),
        };
        let capability = StaffCapability::parse(capability)
            .ok_or_else(|| ParseActionError::Unknown(input.to_owned()))?;
        return Ok(PlayerAction::Staff(StaffAction::SetCapability {
            target: target.to_owned(),
            capability,
            enabled,
            reason,
        }));
    }
    if normalised == "staff capability" || normalised.starts_with("staff ") {
        return Err(ParseActionError::MissingTarget { command: "staff" });
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
    if let Some(rest) = command_tail(input, "contents") {
        let rest = rest.to_ascii_lowercase();
        let (container, page) = if let Some((container, page)) = rest.rsplit_once(" page ") {
            let page = page
                .trim()
                .parse::<u16>()
                .ok()
                .filter(|page| *page > 0)
                .ok_or_else(|| ParseActionError::Unknown(input.to_owned()))?;
            (container.trim(), page)
        } else if let Some((container, page)) = rest.rsplit_once(char::is_whitespace)
            && let Ok(page) = page.trim().parse::<u16>()
        {
            if page == 0 {
                return Err(ParseActionError::Unknown(input.to_owned()));
            }
            (container.trim(), page)
        } else {
            (rest.trim(), 1)
        };
        if container.is_empty() {
            return Err(ParseActionError::MissingTarget {
                command: "contents",
            });
        }
        return Ok(PlayerAction::Contents {
            container: container.to_owned(),
            page,
        });
    }
    if normalised == "contents" {
        return Err(ParseActionError::MissingTarget {
            command: "contents",
        });
    }
    if let Some(rest) = command_tail(input, "put") {
        let rest = rest.to_ascii_lowercase();
        let (selection, container) = rest
            .rsplit_once(" in ")
            .ok_or(ParseActionError::MissingTarget { command: "put" })?;
        let selection = parse_item_selection(selection, input)?;
        let container = container.trim();
        if container.is_empty() {
            return Err(ParseActionError::MissingTarget { command: "put" });
        }
        return Ok(PlayerAction::Put {
            selection,
            container: container.to_owned(),
        });
    }
    if normalised == "put" {
        return Err(ParseActionError::MissingTarget { command: "put" });
    }
    if let Some(rest) = command_tail(input, "give") {
        let separator = rest
            .to_ascii_lowercase()
            .rfind(" to ")
            .ok_or(ParseActionError::MissingTarget { command: "give" })?;
        let (selection, recipient) = rest.split_at(separator);
        let recipient = recipient
            .get(" to ".len()..)
            .ok_or(ParseActionError::MissingTarget { command: "give" })?;
        let selection = parse_item_selection(selection, input)?;
        let recipient = recipient.trim();
        if recipient.is_empty() {
            return Err(ParseActionError::MissingTarget { command: "give" });
        }
        return Ok(PlayerAction::Give {
            selection,
            recipient: recipient.to_owned(),
        });
    }
    if normalised == "give" {
        return Err(ParseActionError::MissingTarget { command: "give" });
    }
    // Container retrieval must be parsed before the historical `get` alias
    // for room pickup.
    if let Some(rest) = command_tail(input, "get").map(|rest| rest.to_ascii_lowercase())
        && let Some((selection, container)) = rest.rsplit_once(" from ")
    {
        let selection = parse_item_selection(selection, input)?;
        let container = container.trim();
        if container.is_empty() {
            return Err(ParseActionError::MissingTarget { command: "get" });
        }
        return Ok(PlayerAction::Get {
            selection,
            container: container.to_owned(),
        });
    }
    for (command, aliases, take) in [
        ("take", &["take", "get"][..], true),
        ("drop", &["drop"][..], false),
    ] {
        for alias in aliases {
            if normalised == *alias {
                return Err(ParseActionError::MissingTarget { command });
            }
            if let Some(target) = command_tail(input, alias) {
                let selection = parse_item_selection(&target.to_ascii_lowercase(), input)?;
                return Ok(if take {
                    PlayerAction::Take(selection)
                } else {
                    PlayerAction::Drop(selection)
                });
            }
        }
    }
    if normalised == "reward odds" {
        return Err(ParseActionError::MissingTarget {
            command: "reward odds",
        });
    }
    if let Some(rest) = normalised.strip_prefix("reward odds ") {
        let (target, page) = if let Some((target, page)) = rest.rsplit_once(char::is_whitespace)
            && let Ok(page) = page.parse::<u16>()
        {
            if page == 0 {
                return Err(ParseActionError::Unknown(input.to_owned()));
            }
            (target.trim(), page)
        } else {
            (rest.trim(), 1)
        };
        if target.is_empty() {
            return Err(ParseActionError::MissingTarget {
                command: "reward odds",
            });
        }
        return Ok(PlayerAction::RewardOdds {
            target: target.to_owned(),
            page,
        });
    }
    for (command, aliases, action) in [
        (
            "examine",
            &["examine", "x"][..],
            PlayerAction::Examine as fn(String) -> PlayerAction,
        ),
        ("attack", &["attack", "kill"][..], PlayerAction::Attack),
        ("use", &["use"][..], PlayerAction::Use),
        ("open", &["open"][..], PlayerAction::OpenReward),
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

fn parse_item_selection(value: &str, original: &str) -> Result<ItemSelection, ParseActionError> {
    let value = value.trim();
    if value.is_empty() {
        return Err(ParseActionError::MissingTarget { command: "item" });
    }
    let Some((first, rest)) = value.split_once(char::is_whitespace) else {
        return Ok(ItemSelection::one(value));
    };
    let Some(quantity) = first.parse::<u16>().ok() else {
        return Ok(ItemSelection::one(value));
    };
    let query = rest.trim();
    if quantity == 0 || quantity > MAX_ITEM_SELECTION_QUANTITY || query.is_empty() {
        return Err(ParseActionError::Unknown(original.trim().to_owned()));
    }
    Ok(ItemSelection {
        query: query.to_owned(),
        quantity,
    })
}

fn split_staff_reason(value: &str) -> Option<(String, String)> {
    let (body, reason) = value.rsplit_once('|')?;
    let body = body.trim();
    let reason = reason.trim();
    (!body.is_empty() && !reason.is_empty()).then(|| (body.to_owned(), reason.to_owned()))
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

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum LootAccessView {
    Yours,
    Reserved,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SceneLootAccessView {
    pub access: LootAccessView,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub expires_in_ticks: Option<u64>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SceneItem {
    pub id: String,
    pub label: String,
    pub description: String,
    pub rarity: String,
    pub item_level: u16,
    pub weight_grams: u32,
    pub quantity: u16,
    pub stack_slots: u32,
    pub total_weight_grams: u64,
    pub transfer_policy: ItemTransferPolicy,
    pub bound_to_you: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub affix: Option<ItemAffixView>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub container_capacity: Option<InventoryCapacityView>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub loot: Option<SceneLootAccessView>,
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
    pub inventory_capacity: InventoryCapacityView,
    pub map: DiscoveredMapView,
    pub quests: Vec<QuestView>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub party: Option<PartyView>,
    pub party_invites: Vec<PartyInviteView>,
    pub economy: EconomyView,
    pub social: SocialView,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub trade: Option<TradeView>,
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
    pub rarity: String,
    pub item_level: u16,
    pub weight_grams: u32,
    pub quantity: u16,
    pub available_quantity: u16,
    pub reserved_for_trade: u16,
    pub stack_slots: u32,
    pub total_weight_grams: u64,
    pub transfer_policy: ItemTransferPolicy,
    pub bound_to_you: bool,
    pub can_give: bool,
    pub can_trade: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub affix: Option<ItemAffixView>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub container_capacity: Option<InventoryCapacityView>,
    pub actions: Vec<SceneAction>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct EquippedItemView {
    pub id: String,
    pub slot: String,
    pub label: String,
    pub rarity: String,
    pub item_level: u16,
    pub weight_grams: u32,
    pub quantity: u16,
    pub stack_slots: u32,
    pub total_weight_grams: u64,
    pub transfer_policy: ItemTransferPolicy,
    pub bound_to_you: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub affix: Option<ItemAffixView>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub container_capacity: Option<InventoryCapacityView>,
    pub actions: Vec<SceneAction>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct InventoryCapacityView {
    pub used_slots: u16,
    pub max_slots: u16,
    pub used_weight_grams: u32,
    pub max_weight_grams: u32,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ItemAffixView {
    pub label: String,
    pub position: AffixPosition,
    pub attack_bonus_delta: i32,
    pub heal_amount_delta: i32,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RewardOddsView {
    pub selector: String,
    pub page: u16,
    pub total_pages: u16,
    pub guaranteed: Vec<RewardOddsGuaranteedRow>,
    pub weighted: Vec<RewardOddsWeightedRow>,
    pub affixes: Vec<RewardOddsAffixRow>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RewardOddsGuaranteedRow {
    pub label: String,
    pub quantity: u16,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub affix_pool_label: Option<String>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RewardOddsWeightedRow {
    pub pool_label: String,
    pub rolls: u8,
    pub outcome_label: String,
    pub min_quantity: u16,
    pub max_quantity: u16,
    pub probability_numerator: u64,
    pub probability_denominator: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub affix_pool_label: Option<String>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RewardOddsAffixRow {
    pub parent_outcome_label: String,
    pub affix_label: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub position: Option<AffixPosition>,
    pub attack_bonus_delta: i32,
    pub heal_amount_delta: i32,
    pub conditional_probability_numerator: u64,
    pub conditional_probability_denominator: u64,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ContainerContentsView {
    pub selector: String,
    pub items: Vec<PlayerItemView>,
    pub page: u16,
    pub total_pages: u16,
    pub capacity: InventoryCapacityView,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TradeOfferLineView {
    pub target: String,
    pub label: String,
    pub quantity: u16,
    pub container: bool,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TradeOfferView {
    pub coins: u64,
    pub lines: Vec<TradeOfferLineView>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TradeView {
    pub id: String,
    pub phase: TradePhase,
    pub counterparty: String,
    pub expires_in_ticks: u64,
    pub state_revision: u64,
    pub own_confirmed: bool,
    pub counterparty_confirmed: bool,
    pub own_offer: TradeOfferView,
    pub counterparty_offer: TradeOfferView,
    pub can_accept: bool,
    pub can_confirm: bool,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TradeContentsView {
    pub trade_id: String,
    pub container: String,
    pub items: Vec<PlayerItemView>,
    pub page: u16,
    pub total_pages: u16,
    pub capacity: InventoryCapacityView,
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
    pub reserved_coins: u64,
    pub spendable_coins: u64,
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
    pub nearby: bool,
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

/// Durable server-side identity of one concrete item.
///
/// The wrapped value is intentionally never projected directly to a client.
/// Transport views derive a session-salted opaque identifier from it.
#[derive(Clone, Debug, Deserialize, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(transparent)]
pub struct ItemInstanceId(pub String);

impl ItemInstanceId {
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl From<String> for ItemInstanceId {
    fn from(value: String) -> Self {
        Self(value)
    }
}

impl From<&str> for ItemInstanceId {
    fn from(value: &str) -> Self {
        Self(value.to_owned())
    }
}

impl Borrow<str> for ItemInstanceId {
    fn borrow(&self) -> &str {
        self.as_str()
    }
}

pub fn valid_item_instance_id(value: &str) -> bool {
    let bytes = value.as_bytes();
    if bytes.is_empty() || bytes.len() > MAX_ITEM_INSTANCE_ID_BYTES {
        return false;
    }
    if value.starts_with("runtime:") {
        return runtime_item_sequence(&ItemInstanceId(value.to_owned())).is_some();
    }
    bytes[0].is_ascii_alphanumeric()
        && bytes[bytes.len() - 1].is_ascii_alphanumeric()
        && bytes.iter().all(|byte| {
            byte.is_ascii_lowercase()
                || byte.is_ascii_digit()
                || matches!(byte, b':' | b'-' | b'_' | b'.')
        })
}

fn runtime_item_sequence(item_id: &ItemInstanceId) -> Option<u64> {
    let value = item_id.as_str().strip_prefix("runtime:")?;
    if value.is_empty()
        || !value.bytes().all(|byte| byte.is_ascii_digit())
        || (value.len() > 1 && value.starts_with('0'))
    {
        return None;
    }
    value.parse::<u64>().ok().filter(|sequence| *sequence > 0)
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ItemRarity {
    Common,
    Uncommon,
    Rare,
    Epic,
    Legendary,
}

impl ItemRarity {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Common => "common",
            Self::Uncommon => "uncommon",
            Self::Rare => "rare",
            Self::Epic => "epic",
            Self::Legendary => "legendary",
        }
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, Hash, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ItemStackPolicy {
    Unique,
    Fungible,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ItemContainerDefinition {
    pub max_slots: u16,
    pub max_weight_grams: u32,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, Hash, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ItemTransferPolicy {
    Transferable,
    CharacterBound,
    NonTransferable,
}

impl ItemTransferPolicy {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Transferable => "transferable",
            Self::CharacterBound => "character_bound",
            Self::NonTransferable => "non_transferable",
        }
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(
    tag = "kind",
    rename_all = "camelCase",
    rename_all_fields = "camelCase",
    deny_unknown_fields
)]
pub enum ItemBinding {
    Unbound,
    Character { character_id: CharacterId },
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct InventoryCapacity {
    pub max_slots: u16,
    pub max_weight_grams: u32,
}

impl Default for InventoryCapacity {
    fn default() -> Self {
        Self {
            max_slots: 24,
            max_weight_grams: 50_000,
        }
    }
}

/// Transport- and authoring-format-independent reusable item content.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ItemPrototypeDefinition {
    pub prototype_id: String,
    pub name: String,
    pub description: String,
    pub aliases: Vec<String>,
    pub equipment_slot: Option<EquipmentSlot>,
    pub attack_bonus: i32,
    pub heal_amount: i32,
    pub consumable: bool,
    pub effect: Option<ActiveEffect>,
    pub weight_grams: u32,
    pub slot_cost: u16,
    pub rarity: ItemRarity,
    pub item_level: u16,
    pub stack_policy: ItemStackPolicy,
    pub max_stack_units: u16,
    pub container: Option<ItemContainerDefinition>,
    pub transfer_policy: ItemTransferPolicy,
    pub sealed_loot_table_id: Option<String>,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum AffixPosition {
    Prefix,
    Suffix,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AffixDefinition {
    pub id: String,
    pub label: String,
    pub position: AffixPosition,
    pub attack_bonus_delta: i32,
    pub heal_amount_delta: i32,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum AffixPoolEntryKind {
    Affix,
    Nothing,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AffixPoolEntry {
    pub kind: AffixPoolEntryKind,
    pub id: String,
    pub weight: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub affix_id: Option<String>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AffixPoolDefinition {
    pub id: String,
    pub entries: Vec<AffixPoolEntry>,
}

/// Immutable, self-contained result of one reviewed affix roll.
#[derive(Clone, Debug, Deserialize, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AppliedItemAffix {
    pub affix_id: String,
    pub definition_digest_sha256: String,
    pub pool_id: String,
    pub pool_digest_sha256: String,
    pub record_digest_sha256: String,
    pub label: String,
    pub position: AffixPosition,
    pub attack_bonus_delta: i32,
    pub heal_amount_delta: i32,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum InitialItemLocation {
    Room(String),
    NpcSpawn(String),
    Container(ItemInstanceId),
    Staged,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct InitialItemInstanceDefinition {
    pub instance_id: ItemInstanceId,
    pub prototype_id: String,
    pub location: InitialItemLocation,
}

/// Transport- and authoring-format-independent input used to construct a world.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct WorldDefinition {
    pub start_room_id: String,
    pub item_prototypes: Vec<ItemPrototypeDefinition>,
    pub initial_item_instances: Vec<InitialItemInstanceDefinition>,
    pub rooms: Vec<RoomDefinition>,
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
    pub equipment: BTreeMap<EquipmentSlot, ItemInstanceId>,
    #[serde(default)]
    pub inventory_capacity: InventoryCapacity,
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
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PartySnapshot {
    pub id: u64,
    pub leader: CharacterId,
    pub members: Vec<CharacterId>,
    pub invitations: BTreeMap<CharacterId, u64>,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, Hash, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum LootRightsPolicy {
    CreditedCharacter,
    PartyRoundRobin,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, Hash, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum LootEntryKind {
    Item,
    Nothing,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, Hash, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum LootSourceKind {
    NpcSpawn,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct LootGuaranteedDefinition {
    pub id: String,
    pub item_prototype_id: String,
    pub quantity: u16,
    pub affix_pool_id: Option<String>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct LootEntryDefinition {
    pub kind: LootEntryKind,
    pub id: String,
    pub weight: u32,
    pub item_prototype_id: Option<String>,
    pub min_quantity: Option<u16>,
    pub max_quantity: Option<u16>,
    pub affix_pool_id: Option<String>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct LootPoolDefinition {
    pub id: String,
    pub rolls: u8,
    pub entries: Vec<LootEntryDefinition>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct LootTableDefinition {
    pub id: String,
    pub digest_sha256: String,
    pub guaranteed: Vec<LootGuaranteedDefinition>,
    pub pools: Vec<LootPoolDefinition>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct LootSourceDefinition {
    pub kind: LootSourceKind,
    pub id: String,
    pub npc_spawn_id: String,
    pub loot_table_id: String,
    pub rights: LootRightsPolicy,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(transparent)]
pub struct LootDropId(pub u64);

#[derive(Clone, Copy, Debug, Deserialize, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(transparent)]
pub struct RewardOpenId(pub u64);

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(
    tag = "kind",
    rename_all = "snake_case",
    rename_all_fields = "camelCase",
    deny_unknown_fields
)]
pub enum ItemProvenance {
    AuthoredInitial,
    LegacySnapshot {
        source_version: u32,
    },
    RuntimeCreated {
        created_tick: u64,
    },
    NpcLoot {
        drop_id: LootDropId,
        npc_spawn_id: String,
        loot_table_id: String,
        table_digest_sha256: String,
        entry_id: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        pool_id: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        sealed_loot_table_id: Option<String>,
        roll_index: u8,
        quantity_index: u16,
        generated_tick: u64,
    },
    SealedReward {
        open_id: RewardOpenId,
        source_item_id: ItemInstanceId,
        source_npc_loot_digest_sha256: String,
        sealed_prototype_id: String,
        loot_table_id: String,
        table_digest_sha256: String,
        entry_id: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        pool_id: Option<String>,
        roll_index: u8,
        quantity_index: u16,
        generated_tick: u64,
    },
}

/// Exact provenance vocabulary accepted from snapshot v6.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(
    tag = "kind",
    rename_all = "snake_case",
    rename_all_fields = "camelCase",
    deny_unknown_fields
)]
pub enum ItemProvenanceV6 {
    AuthoredInitial,
    LegacySnapshot {
        source_version: u32,
    },
    RuntimeCreated {
        created_tick: u64,
    },
    NpcLoot {
        drop_id: LootDropId,
        npc_spawn_id: String,
        loot_table_id: String,
        table_digest_sha256: String,
        entry_id: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        pool_id: Option<String>,
        roll_index: u8,
        quantity_index: u16,
        generated_tick: u64,
    },
}

impl From<ItemProvenanceV6> for ItemProvenance {
    fn from(value: ItemProvenanceV6) -> Self {
        match value {
            ItemProvenanceV6::AuthoredInitial => Self::AuthoredInitial,
            ItemProvenanceV6::LegacySnapshot { source_version } => {
                Self::LegacySnapshot { source_version }
            }
            ItemProvenanceV6::RuntimeCreated { created_tick } => {
                Self::RuntimeCreated { created_tick }
            }
            ItemProvenanceV6::NpcLoot {
                drop_id,
                npc_spawn_id,
                loot_table_id,
                table_digest_sha256,
                entry_id,
                pool_id,
                roll_index,
                quantity_index,
                generated_tick,
            } => Self::NpcLoot {
                drop_id,
                npc_spawn_id,
                loot_table_id,
                table_digest_sha256,
                entry_id,
                pool_id,
                sealed_loot_table_id: None,
                roll_index,
                quantity_index,
                generated_tick,
            },
        }
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct LootEncounter {
    pub npc_spawn_id: String,
    pub initiator: CharacterId,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub party_id: Option<u64>,
    pub cohort: Vec<CharacterId>,
    pub room_id: String,
    pub started_tick: u64,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct GroundLootReservation {
    pub drop_id: LootDropId,
    pub root_item_id: ItemInstanceId,
    pub room_id: String,
    pub cohort: Vec<CharacterId>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub beneficiary: Option<CharacterId>,
    #[serde(default)]
    pub passed: BTreeSet<CharacterId>,
    pub created_tick: u64,
    pub claim_until_tick: u64,
    pub despawn_at_tick: u64,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
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

pub const WORLD_RUNTIME_SNAPSHOT_VERSION: u32 = 7;
pub const MAX_LEGACY_WORLD_RUNTIME_SNAPSHOT_BYTES: usize = 1_048_576;
pub const MAX_WORLD_RUNTIME_SNAPSHOT_BYTES: usize = 2_097_152;
const MAX_ACTIVE_EFFECTS: usize = 32;
pub const MAX_ITEM_INSTANCES: usize = 65_536;
pub const MAX_ITEM_INSTANCE_ID_BYTES: usize = 128;

#[derive(Clone, Copy, Debug, Deserialize, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(transparent)]
pub struct TradeId(pub u64);

#[derive(Clone, Copy, Debug, Deserialize, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum TradePhase {
    Requested,
    Negotiating,
}

#[derive(Clone, Debug, Deserialize, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum TradeOfferKey {
    Fungible { prototype_id: String },
    Unique { item_id: ItemInstanceId },
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct TradeOfferLine {
    pub key: TradeOfferKey,
    pub item_ids: Vec<ItemInstanceId>,
}

#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct TradeOffer {
    pub lines: Vec<TradeOfferLine>,
    pub coins: u64,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct TradeConfirmation {
    pub state_revision: u64,
    pub basis_sha256: [u8; 32],
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct TradeSnapshot {
    pub id: TradeId,
    pub requester: CharacterId,
    pub recipient: CharacterId,
    pub room_id: String,
    pub phase: TradePhase,
    pub created_tick: u64,
    pub opened_tick: Option<u64>,
    pub expires_at_tick: u64,
    pub state_revision: u64,
    pub offers: BTreeMap<CharacterId, TradeOffer>,
    pub confirmations: BTreeMap<CharacterId, TradeConfirmation>,
}

impl TradeSnapshot {
    pub fn participants(&self) -> [CharacterId; 2] {
        [self.requester, self.recipient]
    }

    pub fn includes(&self, character_id: CharacterId) -> bool {
        self.requester == character_id || self.recipient == character_id
    }

    pub fn counterpart(&self, character_id: CharacterId) -> Option<CharacterId> {
        if self.requester == character_id {
            Some(self.recipient)
        } else if self.recipient == character_id {
            Some(self.requester)
        } else {
            None
        }
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum TradeCancellationReason {
    Declined,
    CancelledByParticipant,
    Expired,
    ParticipantsSeparated,
    CombatStarted,
    ParticipantDefeated,
    ParticipantDisconnected,
    SocialPolicyChanged,
    InvalidReservation,
    CounterExhausted,
    Restart,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "camelCase", deny_unknown_fields)]
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
    Contained {
        container_id: ItemInstanceId,
    },
    Consumed,
}

/// Exact pre-container location vocabulary retained only for snapshot v1/v2
/// conversion. Keeping it separate prevents a v2 payload from smuggling a v3
/// `contained` edge.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "camelCase", deny_unknown_fields)]
pub enum RuntimeItemLocationV2 {
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

impl From<RuntimeItemLocationV2> for RuntimeItemLocation {
    fn from(location: RuntimeItemLocationV2) -> Self {
        match location {
            RuntimeItemLocationV2::Staged => Self::Staged,
            RuntimeItemLocationV2::Carried { character_id } => Self::Carried { character_id },
            RuntimeItemLocationV2::Room { room_id } => Self::Room { room_id },
            RuntimeItemLocationV2::Equipped { character_id, slot } => {
                Self::Equipped { character_id, slot }
            }
            RuntimeItemLocationV2::HeldByNpc { npc_id } => Self::HeldByNpc { npc_id },
            RuntimeItemLocationV2::Consumed => Self::Consumed,
        }
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RuntimeItemInstanceSnapshot {
    pub prototype_id: String,
    pub location: RuntimeItemLocation,
    pub binding: ItemBinding,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub item_affixes: Vec<AppliedItemAffix>,
}

/// Exact item shape written by snapshot v6. It cannot opt into v7 affixes.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RuntimeItemInstanceSnapshotV6 {
    pub prototype_id: String,
    pub location: RuntimeItemLocation,
    pub binding: ItemBinding,
}

/// Exact item shape written by inventory phase 3B. It intentionally lacks
/// `binding`, so a v3 payload cannot opt into v4 fields before conversion.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RuntimeItemInstanceSnapshotV3 {
    pub prototype_id: String,
    pub location: RuntimeItemLocation,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
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
    pub loot_item_ids: Vec<ItemInstanceId>,
    pub respawn_ticks: u64,
    pub respawn_at_tick: Option<u64>,
    pub engaged_character: Option<CharacterId>,
    #[serde(default, deserialize_with = "deserialize_runtime_resource_pools")]
    pub resources: BTreeMap<String, ResourcePool>,
    #[serde(default)]
    pub resource_regeneration: BTreeMap<String, u32>,
    #[serde(default)]
    pub ability_ids: Vec<String>,
    #[serde(default)]
    pub ability_cooldowns: BTreeMap<String, u64>,
    #[serde(default)]
    pub resistances: BTreeMap<DamageType, u16>,
    #[serde(default, deserialize_with = "deserialize_runtime_statuses")]
    pub statuses: Vec<ActiveStatusEffect>,
}

/// Snapshot-only exact shape. `ResourcePool` is also used by other durable
/// documents, so strictness is applied at this boundary without changing
/// their compatibility contract.
#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct RuntimeResourcePoolSnapshot {
    current: u32,
    max: u32,
}

impl From<RuntimeResourcePoolSnapshot> for ResourcePool {
    fn from(pool: RuntimeResourcePoolSnapshot) -> Self {
        Self {
            current: pool.current,
            max: pool.max,
        }
    }
}

fn deserialize_runtime_resource_pools<'de, D>(
    deserializer: D,
) -> Result<BTreeMap<String, ResourcePool>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    BTreeMap::<String, RuntimeResourcePoolSnapshot>::deserialize(deserializer).map(|pools| {
        pools
            .into_iter()
            .map(|(id, pool)| (id, pool.into()))
            .collect()
    })
}

/// Snapshot-only exact shape for status records. Optional fields retain the
/// same defaults as `ActiveStatusEffect`, while unknown fields fail closed.
#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct RuntimeActiveStatusSnapshot {
    source_ability_id: String,
    #[serde(default)]
    source_character_id: Option<CharacterId>,
    #[serde(default)]
    source_npc_id: Option<String>,
    kind: ActiveStatusKind,
    magnitude: i32,
    remaining_ticks: u32,
    #[serde(default)]
    attribute: Option<String>,
    #[serde(default)]
    damage_type: Option<DamageType>,
}

impl From<RuntimeActiveStatusSnapshot> for ActiveStatusEffect {
    fn from(status: RuntimeActiveStatusSnapshot) -> Self {
        Self {
            source_ability_id: status.source_ability_id,
            source_character_id: status.source_character_id,
            source_npc_id: status.source_npc_id,
            kind: status.kind,
            magnitude: status.magnitude,
            remaining_ticks: status.remaining_ticks,
            attribute: status.attribute,
            damage_type: status.damage_type,
        }
    }
}

fn deserialize_runtime_statuses<'de, D>(
    deserializer: D,
) -> Result<Vec<ActiveStatusEffect>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    Vec::<RuntimeActiveStatusSnapshot>::deserialize(deserializer)
        .map(|statuses| statuses.into_iter().map(Into::into).collect())
}

/// Portable shared-world state. It deliberately excludes sessions and opaque
/// observer IDs, which are transport lifetime state and are rebuilt on start.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct WorldRuntimeSnapshot {
    pub snapshot_version: u32,
    pub revision: u64,
    pub logical_tick: u64,
    pub rng_state: u64,
    pub next_item_instance_sequence: u64,
    pub item_instances: BTreeMap<ItemInstanceId, RuntimeItemInstanceSnapshot>,
    pub npcs: BTreeMap<String, NpcRuntimeSnapshot>,
    #[serde(default)]
    pub adventure: AdventureRuntimeSnapshot,
    #[serde(default)]
    pub progression_flags: BTreeSet<String>,
    pub next_trade_id: TradeId,
    pub trades: BTreeMap<TradeId, TradeSnapshot>,
    pub next_loot_drop_id: LootDropId,
    pub item_provenance: BTreeMap<ItemInstanceId, ItemProvenance>,
    pub loot_encounters: BTreeMap<String, LootEncounter>,
    pub ground_loot: BTreeMap<ItemInstanceId, GroundLootReservation>,
    pub party_loot_cursors: BTreeMap<u64, u64>,
    pub next_reward_open_id: RewardOpenId,
}

/// Exact snapshot shape produced by repeatable-loot phase 4A. It cannot opt
/// into affixes, sealed provenance or the reward-open counter introduced by
/// v7.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct WorldRuntimeSnapshotV6 {
    pub snapshot_version: u32,
    pub revision: u64,
    pub logical_tick: u64,
    pub rng_state: u64,
    pub next_item_instance_sequence: u64,
    pub item_instances: BTreeMap<ItemInstanceId, RuntimeItemInstanceSnapshotV6>,
    pub npcs: BTreeMap<String, NpcRuntimeSnapshot>,
    #[serde(default)]
    pub adventure: AdventureRuntimeSnapshot,
    #[serde(default)]
    pub progression_flags: BTreeSet<String>,
    pub next_trade_id: TradeId,
    pub trades: BTreeMap<TradeId, TradeSnapshot>,
    pub next_loot_drop_id: LootDropId,
    pub item_provenance: BTreeMap<ItemInstanceId, ItemProvenanceV6>,
    pub loot_encounters: BTreeMap<String, LootEncounter>,
    pub ground_loot: BTreeMap<ItemInstanceId, GroundLootReservation>,
    pub party_loot_cursors: BTreeMap<u64, u64>,
}

/// Exact snapshot shape produced by inventory phase 3D. It intentionally has
/// no phase-4 loot fields; v5 inputs receive conservative legacy provenance.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct WorldRuntimeSnapshotV5 {
    pub snapshot_version: u32,
    pub revision: u64,
    pub logical_tick: u64,
    pub rng_state: u64,
    pub next_item_instance_sequence: u64,
    pub item_instances: BTreeMap<ItemInstanceId, RuntimeItemInstanceSnapshotV6>,
    pub npcs: BTreeMap<String, NpcRuntimeSnapshot>,
    #[serde(default)]
    pub adventure: AdventureRuntimeSnapshot,
    #[serde(default)]
    pub progression_flags: BTreeSet<String>,
    pub next_trade_id: TradeId,
    pub trades: BTreeMap<TradeId, TradeSnapshot>,
}

/// Exact snapshot shape produced by inventory phase 3C. It intentionally has
/// no trade fields; v4 payloads may only obtain an empty trade state through
/// the explicit v4 -> v5 conversion.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct WorldRuntimeSnapshotV4 {
    pub snapshot_version: u32,
    pub revision: u64,
    pub logical_tick: u64,
    pub rng_state: u64,
    pub next_item_instance_sequence: u64,
    pub item_instances: BTreeMap<ItemInstanceId, RuntimeItemInstanceSnapshotV6>,
    pub npcs: BTreeMap<String, NpcRuntimeSnapshot>,
    #[serde(default)]
    pub adventure: AdventureRuntimeSnapshot,
    #[serde(default)]
    pub progression_flags: BTreeSet<String>,
}

/// Exact snapshot shape produced by inventory phase 3B.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct WorldRuntimeSnapshotV3 {
    pub snapshot_version: u32,
    pub revision: u64,
    pub logical_tick: u64,
    pub rng_state: u64,
    pub next_item_instance_sequence: u64,
    pub item_instances: BTreeMap<ItemInstanceId, RuntimeItemInstanceSnapshotV3>,
    pub npcs: BTreeMap<String, NpcRuntimeSnapshot>,
    #[serde(default)]
    pub adventure: AdventureRuntimeSnapshot,
    #[serde(default)]
    pub progression_flags: BTreeSet<String>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RuntimeItemInstanceSnapshotV2 {
    pub prototype_id: String,
    pub location: RuntimeItemLocationV2,
}

/// Exact snapshot shape produced by inventory phase 3A.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct WorldRuntimeSnapshotV2 {
    pub snapshot_version: u32,
    pub revision: u64,
    pub logical_tick: u64,
    pub rng_state: u64,
    pub next_item_instance_sequence: u64,
    pub item_instances: BTreeMap<ItemInstanceId, RuntimeItemInstanceSnapshotV2>,
    pub npcs: BTreeMap<String, NpcRuntimeSnapshot>,
    #[serde(default)]
    pub adventure: AdventureRuntimeSnapshot,
    #[serde(default)]
    pub progression_flags: BTreeSet<String>,
}

/// Exact portable shape written before the prototype/instance split.
/// Callers must inspect `snapshot_version` and opt into this conversion rather
/// than allowing missing v2 fields to deserialize as an empty inventory.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct WorldRuntimeSnapshotV1 {
    pub snapshot_version: u32,
    pub revision: u64,
    pub logical_tick: u64,
    pub rng_state: u64,
    pub item_locations: BTreeMap<String, RuntimeItemLocationV2>,
    pub npcs: BTreeMap<String, NpcRuntimeSnapshot>,
    #[serde(default)]
    pub adventure: AdventureRuntimeSnapshot,
    #[serde(default)]
    pub progression_flags: BTreeSet<String>,
}

impl WorldRuntimeSnapshot {
    pub fn upgrade_v1(snapshot: WorldRuntimeSnapshotV1) -> Result<Self, WorldError> {
        let mut upgraded = Self::upgrade_v2(WorldRuntimeSnapshotV2::upgrade_v1(snapshot)?)?;
        for provenance in upgraded.item_provenance.values_mut() {
            *provenance = ItemProvenance::LegacySnapshot { source_version: 1 };
        }
        Ok(upgraded)
    }

    pub fn upgrade_v2(snapshot: WorldRuntimeSnapshotV2) -> Result<Self, WorldError> {
        if snapshot.snapshot_version != 2 {
            return Err(WorldError::InvalidRuntimeSnapshot);
        }
        let item_instances = snapshot
            .item_instances
            .into_iter()
            .map(|(item_id, instance)| {
                (
                    item_id,
                    RuntimeItemInstanceSnapshot {
                        prototype_id: instance.prototype_id,
                        location: instance.location.into(),
                        binding: ItemBinding::Unbound,
                        item_affixes: Vec::new(),
                    },
                )
            })
            .collect::<BTreeMap<_, _>>();
        let item_provenance = legacy_item_provenance(&item_instances, 2);
        Ok(Self {
            snapshot_version: WORLD_RUNTIME_SNAPSHOT_VERSION,
            revision: snapshot.revision,
            logical_tick: snapshot.logical_tick,
            rng_state: snapshot.rng_state,
            next_item_instance_sequence: snapshot.next_item_instance_sequence,
            item_instances,
            npcs: snapshot.npcs,
            adventure: snapshot.adventure,
            progression_flags: snapshot.progression_flags,
            next_trade_id: TradeId(1),
            trades: BTreeMap::new(),
            next_loot_drop_id: LootDropId(1),
            item_provenance,
            loot_encounters: BTreeMap::new(),
            ground_loot: BTreeMap::new(),
            party_loot_cursors: BTreeMap::new(),
            next_reward_open_id: RewardOpenId(1),
        })
    }

    pub fn upgrade_v3(snapshot: WorldRuntimeSnapshotV3) -> Result<Self, WorldError> {
        if snapshot.snapshot_version != 3 {
            return Err(WorldError::InvalidRuntimeSnapshot);
        }
        let item_instances = snapshot
            .item_instances
            .into_iter()
            .map(|(item_id, instance)| {
                (
                    item_id,
                    RuntimeItemInstanceSnapshot {
                        prototype_id: instance.prototype_id,
                        location: instance.location,
                        binding: ItemBinding::Unbound,
                        item_affixes: Vec::new(),
                    },
                )
            })
            .collect::<BTreeMap<_, _>>();
        let item_provenance = legacy_item_provenance(&item_instances, 3);
        Ok(Self {
            snapshot_version: WORLD_RUNTIME_SNAPSHOT_VERSION,
            revision: snapshot.revision,
            logical_tick: snapshot.logical_tick,
            rng_state: snapshot.rng_state,
            next_item_instance_sequence: snapshot.next_item_instance_sequence,
            item_instances,
            npcs: snapshot.npcs,
            adventure: snapshot.adventure,
            progression_flags: snapshot.progression_flags,
            next_trade_id: TradeId(1),
            trades: BTreeMap::new(),
            next_loot_drop_id: LootDropId(1),
            item_provenance,
            loot_encounters: BTreeMap::new(),
            ground_loot: BTreeMap::new(),
            party_loot_cursors: BTreeMap::new(),
            next_reward_open_id: RewardOpenId(1),
        })
    }

    pub fn upgrade_v4(snapshot: WorldRuntimeSnapshotV4) -> Result<Self, WorldError> {
        if snapshot.snapshot_version != 4 {
            return Err(WorldError::InvalidRuntimeSnapshot);
        }
        let item_provenance = legacy_item_provenance(&snapshot.item_instances, 4);
        let item_instances = upgrade_v6_item_instances(snapshot.item_instances);
        Ok(Self {
            snapshot_version: WORLD_RUNTIME_SNAPSHOT_VERSION,
            revision: snapshot.revision,
            logical_tick: snapshot.logical_tick,
            rng_state: snapshot.rng_state,
            next_item_instance_sequence: snapshot.next_item_instance_sequence,
            item_instances,
            npcs: snapshot.npcs,
            adventure: snapshot.adventure,
            progression_flags: snapshot.progression_flags,
            next_trade_id: TradeId(1),
            trades: BTreeMap::new(),
            next_loot_drop_id: LootDropId(1),
            item_provenance,
            loot_encounters: BTreeMap::new(),
            ground_loot: BTreeMap::new(),
            party_loot_cursors: BTreeMap::new(),
            next_reward_open_id: RewardOpenId(1),
        })
    }

    pub fn upgrade_v5(snapshot: WorldRuntimeSnapshotV5) -> Result<Self, WorldError> {
        if snapshot.snapshot_version != 5 {
            return Err(WorldError::InvalidRuntimeSnapshot);
        }
        let item_provenance = legacy_item_provenance(&snapshot.item_instances, 5);
        let item_instances = upgrade_v6_item_instances(snapshot.item_instances);
        Ok(Self {
            snapshot_version: WORLD_RUNTIME_SNAPSHOT_VERSION,
            revision: snapshot.revision,
            logical_tick: snapshot.logical_tick,
            rng_state: snapshot.rng_state,
            next_item_instance_sequence: snapshot.next_item_instance_sequence,
            item_instances,
            npcs: snapshot.npcs,
            adventure: snapshot.adventure,
            progression_flags: snapshot.progression_flags,
            next_trade_id: snapshot.next_trade_id,
            trades: snapshot.trades,
            next_loot_drop_id: LootDropId(1),
            item_provenance,
            loot_encounters: BTreeMap::new(),
            ground_loot: BTreeMap::new(),
            party_loot_cursors: BTreeMap::new(),
            next_reward_open_id: RewardOpenId(1),
        })
    }

    pub fn upgrade_v6(snapshot: WorldRuntimeSnapshotV6) -> Result<Self, WorldError> {
        if snapshot.snapshot_version != 6 {
            return Err(WorldError::InvalidRuntimeSnapshot);
        }
        Ok(Self {
            snapshot_version: WORLD_RUNTIME_SNAPSHOT_VERSION,
            revision: snapshot.revision,
            logical_tick: snapshot.logical_tick,
            rng_state: snapshot.rng_state,
            next_item_instance_sequence: snapshot.next_item_instance_sequence,
            item_instances: upgrade_v6_item_instances(snapshot.item_instances),
            npcs: snapshot.npcs,
            adventure: snapshot.adventure,
            progression_flags: snapshot.progression_flags,
            next_trade_id: snapshot.next_trade_id,
            trades: snapshot.trades,
            next_loot_drop_id: snapshot.next_loot_drop_id,
            item_provenance: snapshot
                .item_provenance
                .into_iter()
                .map(|(id, provenance)| (id, provenance.into()))
                .collect(),
            loot_encounters: snapshot.loot_encounters,
            ground_loot: snapshot.ground_loot,
            party_loot_cursors: snapshot.party_loot_cursors,
            next_reward_open_id: RewardOpenId(1),
        })
    }
}

fn upgrade_v6_item_instances(
    instances: BTreeMap<ItemInstanceId, RuntimeItemInstanceSnapshotV6>,
) -> BTreeMap<ItemInstanceId, RuntimeItemInstanceSnapshot> {
    instances
        .into_iter()
        .map(|(id, instance)| {
            (
                id,
                RuntimeItemInstanceSnapshot {
                    prototype_id: instance.prototype_id,
                    location: instance.location,
                    binding: instance.binding,
                    item_affixes: Vec::new(),
                },
            )
        })
        .collect()
}

fn legacy_item_provenance<T>(
    items: &BTreeMap<ItemInstanceId, T>,
    source_version: u32,
) -> BTreeMap<ItemInstanceId, ItemProvenance> {
    items
        .keys()
        .cloned()
        .map(|item_id| (item_id, ItemProvenance::LegacySnapshot { source_version }))
        .collect()
}

impl WorldRuntimeSnapshotV2 {
    pub fn upgrade_v1(snapshot: WorldRuntimeSnapshotV1) -> Result<Self, WorldError> {
        if snapshot.snapshot_version != 1 {
            return Err(WorldError::InvalidRuntimeSnapshot);
        }
        let item_instances = snapshot
            .item_locations
            .into_iter()
            .map(|(legacy_id, location)| {
                (
                    ItemInstanceId(legacy_id.clone()),
                    RuntimeItemInstanceSnapshotV2 {
                        prototype_id: legacy_id,
                        location,
                    },
                )
            })
            .collect();
        Ok(Self {
            snapshot_version: 2,
            revision: snapshot.revision,
            logical_tick: snapshot.logical_tick,
            rng_state: snapshot.rng_state,
            next_item_instance_sequence: 1,
            item_instances,
            npcs: snapshot.npcs,
            adventure: snapshot.adventure,
            progression_flags: snapshot.progression_flags,
        })
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum CompatibleWorldRuntimeSnapshot {
    V7(WorldRuntimeSnapshot),
    V6(WorldRuntimeSnapshotV6),
    V5(WorldRuntimeSnapshotV5),
    V4(WorldRuntimeSnapshotV4),
    V3(WorldRuntimeSnapshotV3),
    V2(WorldRuntimeSnapshotV2),
    V1(WorldRuntimeSnapshotV1),
}

impl<'de> Deserialize<'de> for CompatibleWorldRuntimeSnapshot {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let value = serde_json::Value::deserialize(deserializer)?;
        let version = value
            .get("snapshotVersion")
            .and_then(serde_json::Value::as_u64)
            .ok_or_else(|| serde::de::Error::custom("missing snapshotVersion"))?;
        match version {
            1 => serde_json::from_value(value)
                .map(Self::V1)
                .map_err(serde::de::Error::custom),
            2 => serde_json::from_value(value)
                .map(Self::V2)
                .map_err(serde::de::Error::custom),
            3 => serde_json::from_value(value)
                .map(Self::V3)
                .map_err(serde::de::Error::custom),
            4 => serde_json::from_value(value)
                .map(Self::V4)
                .map_err(serde::de::Error::custom),
            5 => serde_json::from_value(value)
                .map(Self::V5)
                .map_err(serde::de::Error::custom),
            6 => serde_json::from_value(value)
                .map(Self::V6)
                .map_err(serde::de::Error::custom),
            7 => serde_json::from_value(value)
                .map(Self::V7)
                .map_err(serde::de::Error::custom),
            _ => Err(serde::de::Error::custom("unsupported snapshotVersion")),
        }
    }
}

impl CompatibleWorldRuntimeSnapshot {
    pub fn into_current(self) -> Result<WorldRuntimeSnapshot, WorldError> {
        match self {
            Self::V7(snapshot) => Ok(snapshot),
            Self::V6(snapshot) => WorldRuntimeSnapshot::upgrade_v6(snapshot),
            Self::V5(snapshot) => WorldRuntimeSnapshot::upgrade_v5(snapshot),
            Self::V4(snapshot) => WorldRuntimeSnapshot::upgrade_v4(snapshot),
            Self::V3(snapshot) => WorldRuntimeSnapshot::upgrade_v3(snapshot),
            Self::V2(snapshot) => WorldRuntimeSnapshot::upgrade_v2(snapshot),
            Self::V1(snapshot) => WorldRuntimeSnapshot::upgrade_v1(snapshot),
        }
    }
}

/// Decode one durable runtime payload only after comparing the storage column
/// with the payload's original version. Normalization never erases evidence of
/// a mismatched column/version pair.
pub fn decode_world_runtime_snapshot(
    durable_version: u32,
    json: &str,
) -> Result<WorldRuntimeSnapshot, WorldError> {
    let byte_limit = if durable_version <= 5 {
        MAX_LEGACY_WORLD_RUNTIME_SNAPSHOT_BYTES
    } else {
        MAX_WORLD_RUNTIME_SNAPSHOT_BYTES
    };
    if json.len() > byte_limit {
        return Err(WorldError::InvalidRuntimeSnapshot);
    }
    let compatible: CompatibleWorldRuntimeSnapshot =
        serde_json::from_str(json).map_err(|_| WorldError::InvalidRuntimeSnapshot)?;
    let payload_version = match &compatible {
        CompatibleWorldRuntimeSnapshot::V7(snapshot) => snapshot.snapshot_version,
        CompatibleWorldRuntimeSnapshot::V6(snapshot) => snapshot.snapshot_version,
        CompatibleWorldRuntimeSnapshot::V5(snapshot) => snapshot.snapshot_version,
        CompatibleWorldRuntimeSnapshot::V4(snapshot) => snapshot.snapshot_version,
        CompatibleWorldRuntimeSnapshot::V3(snapshot) => snapshot.snapshot_version,
        CompatibleWorldRuntimeSnapshot::V2(snapshot) => snapshot.snapshot_version,
        CompatibleWorldRuntimeSnapshot::V1(snapshot) => snapshot.snapshot_version,
    };
    if payload_version != durable_version {
        return Err(WorldError::InvalidRuntimeSnapshot);
    }
    compatible.into_current()
}

/// Validates every portable invariant that can be established without a
/// particular WorldPack. Offline movers and recovery tools use this same
/// authority after version-directed decoding; `World::restore_runtime_snapshot`
/// adds only the authored room/prototype/policy/capacity checks.
pub fn validate_world_runtime_snapshot_semantics(
    snapshot: &WorldRuntimeSnapshot,
) -> Result<(), WorldError> {
    if serde_json::to_vec(snapshot)
        .map_err(|_| WorldError::InvalidRuntimeSnapshot)?
        .len()
        > MAX_WORLD_RUNTIME_SNAPSHOT_BYTES
    {
        return Err(WorldError::InvalidRuntimeSnapshot);
    }
    if snapshot.snapshot_version != WORLD_RUNTIME_SNAPSHOT_VERSION
        || snapshot.revision == 0
        || snapshot.rng_state == 0
        || snapshot.next_item_instance_sequence == 0
        || snapshot.item_instances.len() > MAX_ITEM_INSTANCES
        || snapshot.progression_flags.len() > 64
        || !snapshot
            .progression_flags
            .iter()
            .all(|flag| valid_definition_id(flag))
    {
        return Err(WorldError::InvalidRuntimeSnapshot);
    }
    let runtime_sequence_valid = snapshot
        .item_instances
        .keys()
        .filter_map(runtime_item_sequence)
        .max()
        .is_none_or(|sequence| sequence < snapshot.next_item_instance_sequence);
    if !runtime_sequence_valid {
        return Err(WorldError::InvalidRuntimeSnapshot);
    }
    if snapshot.next_loot_drop_id.0 == 0
        || snapshot.next_reward_open_id.0 == 0
        || snapshot.item_provenance.len() != snapshot.item_instances.len()
        || snapshot
            .item_provenance
            .keys()
            .ne(snapshot.item_instances.keys())
        || snapshot.loot_encounters.len() > MAX_LOOT_ENCOUNTERS
        || snapshot.ground_loot.len() > MAX_GROUND_LOOT_RESERVATIONS
    {
        return Err(WorldError::InvalidRuntimeSnapshot);
    }

    let mut children = BTreeMap::<ItemInstanceId, Vec<ItemInstanceId>>::new();
    for (item_id, instance) in &snapshot.item_instances {
        let location_valid = match &instance.location {
            RuntimeItemLocation::Staged | RuntimeItemLocation::Consumed => true,
            RuntimeItemLocation::Carried { character_id }
            | RuntimeItemLocation::Equipped { character_id, .. } => character_id.0 > 0,
            RuntimeItemLocation::Room { room_id } => valid_definition_id(room_id),
            RuntimeItemLocation::HeldByNpc { npc_id } => valid_definition_id(npc_id),
            RuntimeItemLocation::Contained { container_id } => {
                if container_id == item_id
                    || !valid_item_instance_id(container_id.as_str())
                    || !snapshot.item_instances.contains_key(container_id)
                {
                    false
                } else {
                    children
                        .entry(container_id.clone())
                        .or_default()
                        .push(item_id.clone());
                    true
                }
            }
        };
        if !valid_item_instance_id(item_id.as_str())
            || !valid_definition_id(&instance.prototype_id)
            || !location_valid
            || matches!(instance.binding, ItemBinding::Character { character_id } if character_id.0 <= 0)
            || instance.item_affixes.len() > MAX_AFFIXES_PER_ITEM
            || instance.item_affixes.iter().any(|affix| {
                !valid_definition_id(&affix.affix_id)
                    || !valid_digest_sha256(&affix.definition_digest_sha256)
                    || !valid_definition_id(&affix.pool_id)
                    || !valid_digest_sha256(&affix.pool_digest_sha256)
                    || !valid_digest_sha256(&affix.record_digest_sha256)
                    || !safe_definition_text(&affix.label, 64)
                    || !(0..=MAX_AFFIX_MODIFIER).contains(&affix.attack_bonus_delta)
                    || !(0..=MAX_AFFIX_MODIFIER).contains(&affix.heal_amount_delta)
                    || (affix.attack_bonus_delta == 0 && affix.heal_amount_delta == 0)
            })
            || !instance.item_affixes.windows(2).all(|pair| {
                (pair[0].position, pair[0].affix_id.as_str())
                    < (pair[1].position, pair[1].affix_id.as_str())
            })
        {
            return Err(WorldError::InvalidRuntimeSnapshot);
        }
        for affix in &instance.item_affixes {
            let definition = AffixDefinition {
                id: affix.affix_id.clone(),
                label: affix.label.clone(),
                position: affix.position,
                attack_bonus_delta: affix.attack_bonus_delta,
                heal_amount_delta: affix.heal_amount_delta,
            };
            if canonical_sha256(&definition).map_err(|_| WorldError::InvalidRuntimeSnapshot)?
                != affix.definition_digest_sha256
                || applied_affix_record_sha256(affix)
                    .map_err(|_| WorldError::InvalidRuntimeSnapshot)?
                    != affix.record_digest_sha256
            {
                return Err(WorldError::InvalidRuntimeSnapshot);
            }
        }
    }
    for item_id in snapshot.item_instances.keys() {
        let mut current = item_id;
        let mut visited = BTreeSet::new();
        let mut depth = 0_usize;
        while let Some(RuntimeItemInstanceSnapshot {
            location: RuntimeItemLocation::Contained { container_id },
            ..
        }) = snapshot.item_instances.get(current)
        {
            if !visited.insert(current.clone()) {
                return Err(WorldError::InvalidRuntimeSnapshot);
            }
            depth = depth
                .checked_add(1)
                .ok_or(WorldError::InvalidRuntimeSnapshot)?;
            if depth > MAX_CONTAINER_DEPTH {
                return Err(WorldError::InvalidRuntimeSnapshot);
            }
            current = container_id;
        }
    }

    let mut greatest_drop_id = 0_u64;
    let mut greatest_open_id = 0_u64;
    let mut open_sources =
        BTreeMap::<RewardOpenId, (ItemInstanceId, String, String, String, String, u64)>::new();
    let mut source_opens = BTreeMap::<ItemInstanceId, RewardOpenId>::new();
    let mut reward_units = BTreeSet::new();
    let mut open_unit_counts = BTreeMap::<RewardOpenId, usize>::new();
    let mut quantity_indexes =
        BTreeMap::<(RewardOpenId, String, Option<String>, u8), BTreeSet<u16>>::new();
    for (item_id, provenance) in &snapshot.item_provenance {
        match provenance {
            ItemProvenance::AuthoredInitial => {}
            ItemProvenance::LegacySnapshot { source_version } => {
                if !(1..=5).contains(source_version) {
                    return Err(WorldError::InvalidRuntimeSnapshot);
                }
            }
            ItemProvenance::RuntimeCreated { created_tick } => {
                if *created_tick > snapshot.logical_tick {
                    return Err(WorldError::InvalidRuntimeSnapshot);
                }
            }
            ItemProvenance::NpcLoot {
                drop_id,
                npc_spawn_id,
                loot_table_id,
                table_digest_sha256,
                entry_id,
                pool_id,
                sealed_loot_table_id,
                roll_index,
                quantity_index,
                generated_tick,
            } => {
                greatest_drop_id = greatest_drop_id.max(drop_id.0);
                if drop_id.0 == 0
                    || !valid_definition_id(npc_spawn_id)
                    || !valid_definition_id(loot_table_id)
                    || !valid_definition_id(entry_id)
                    || pool_id.as_ref().is_some_and(|id| !valid_definition_id(id))
                    || sealed_loot_table_id
                        .as_ref()
                        .is_some_and(|id| !valid_definition_id(id))
                    || !valid_digest_sha256(table_digest_sha256)
                    || *roll_index >= 8
                    || *quantity_index >= 32
                    || *generated_tick > snapshot.logical_tick
                {
                    return Err(WorldError::InvalidRuntimeSnapshot);
                }
            }
            ItemProvenance::SealedReward {
                open_id,
                source_item_id,
                source_npc_loot_digest_sha256,
                sealed_prototype_id,
                loot_table_id,
                table_digest_sha256,
                entry_id,
                pool_id,
                roll_index,
                quantity_index,
                generated_tick,
            } => {
                greatest_open_id = greatest_open_id.max(open_id.0);
                if open_id.0 == 0
                    || source_item_id == item_id
                    || !valid_item_instance_id(source_item_id.as_str())
                    || !valid_digest_sha256(source_npc_loot_digest_sha256)
                    || !valid_definition_id(sealed_prototype_id)
                    || !valid_definition_id(loot_table_id)
                    || !valid_digest_sha256(table_digest_sha256)
                    || !valid_definition_id(entry_id)
                    || pool_id.as_ref().is_some_and(|id| !valid_definition_id(id))
                    || *roll_index >= 8
                    || usize::from(*quantity_index) >= MAX_SEALED_REWARD_UNITS
                    || *generated_tick > snapshot.logical_tick
                {
                    return Err(WorldError::InvalidRuntimeSnapshot);
                }
                let source_key = (
                    source_item_id.clone(),
                    source_npc_loot_digest_sha256.clone(),
                    sealed_prototype_id.clone(),
                    loot_table_id.clone(),
                    table_digest_sha256.clone(),
                    *generated_tick,
                );
                if open_sources
                    .insert(*open_id, source_key.clone())
                    .is_some_and(|previous| previous != source_key)
                    || source_opens
                        .insert(source_item_id.clone(), *open_id)
                        .is_some_and(|previous| previous != *open_id)
                    || !reward_units.insert((
                        *open_id,
                        entry_id.clone(),
                        pool_id.clone(),
                        *roll_index,
                        *quantity_index,
                    ))
                {
                    return Err(WorldError::InvalidRuntimeSnapshot);
                }
                *open_unit_counts.entry(*open_id).or_default() += 1;
                quantity_indexes
                    .entry((*open_id, entry_id.clone(), pool_id.clone(), *roll_index))
                    .or_default()
                    .insert(*quantity_index);
            }
        }
    }
    if open_unit_counts
        .values()
        .any(|count| !(1..=MAX_SEALED_REWARD_UNITS).contains(count))
        || quantity_indexes
            .values()
            .any(|indexes| indexes.iter().copied().ne((0_u16..).take(indexes.len())))
    {
        return Err(WorldError::InvalidRuntimeSnapshot);
    }
    for (_, (source_item_id, expected_digest, sealed_prototype_id, _, _, _)) in open_sources {
        let Some(source_instance) = snapshot.item_instances.get(&source_item_id) else {
            continue;
        };
        let source_provenance = snapshot
            .item_provenance
            .get(&source_item_id)
            .ok_or(WorldError::InvalidRuntimeSnapshot)?;
        if !matches!(source_instance.location, RuntimeItemLocation::Consumed)
            || source_instance.prototype_id != sealed_prototype_id
            || !matches!(source_provenance, ItemProvenance::NpcLoot { .. })
            || canonical_sha256(source_provenance)
                .map_err(|_| WorldError::InvalidRuntimeSnapshot)?
                != expected_digest
        {
            return Err(WorldError::InvalidRuntimeSnapshot);
        }
    }
    for (npc_id, encounter) in &snapshot.loot_encounters {
        let npc = snapshot
            .npcs
            .get(npc_id)
            .ok_or(WorldError::InvalidRuntimeSnapshot)?;
        if npc_id != &encounter.npc_spawn_id
            || npc.engaged_character != Some(encounter.initiator)
            || encounter.initiator.0 <= 0
            || encounter.started_tick > snapshot.logical_tick
            || !valid_definition_id(&encounter.room_id)
            || encounter.room_id != npc.spawn_room_id
            || encounter.party_id == Some(0)
            || encounter.cohort.is_empty()
            || encounter.cohort.len() > 6
            || !encounter.cohort.contains(&encounter.initiator)
            || encounter.cohort.iter().any(|id| id.0 <= 0)
            || !encounter.cohort.windows(2).all(|pair| pair[0] < pair[1])
        {
            return Err(WorldError::InvalidRuntimeSnapshot);
        }
    }
    for (root_id, reservation) in &snapshot.ground_loot {
        greatest_drop_id = greatest_drop_id.max(reservation.drop_id.0);
        let instance = snapshot
            .item_instances
            .get(root_id)
            .ok_or(WorldError::InvalidRuntimeSnapshot)?;
        if root_id != &reservation.root_item_id
            || reservation.drop_id.0 == 0
            || instance.location
                != (RuntimeItemLocation::Room {
                    room_id: reservation.room_id.clone(),
                })
            || !valid_definition_id(&reservation.room_id)
            || reservation.cohort.is_empty()
            || reservation.cohort.len() > 6
            || reservation.cohort.iter().any(|id| id.0 <= 0)
            || !reservation.cohort.windows(2).all(|pair| pair[0] < pair[1])
            || reservation.beneficiary.is_some_and(|id| {
                !reservation.cohort.contains(&id) || reservation.passed.contains(&id)
            })
            || (snapshot.logical_tick >= reservation.claim_until_tick
                && reservation.beneficiary.is_some())
            || !reservation
                .passed
                .iter()
                .all(|id| reservation.cohort.contains(id))
            || reservation.claim_until_tick
                != reservation
                    .created_tick
                    .checked_add(LOOT_EXCLUSIVE_TICKS)
                    .ok_or(WorldError::InvalidRuntimeSnapshot)?
            || reservation.despawn_at_tick
                != reservation
                    .created_tick
                    .checked_add(LOOT_DESPAWN_TICKS)
                    .ok_or(WorldError::InvalidRuntimeSnapshot)?
            || snapshot.logical_tick >= reservation.despawn_at_tick
        {
            return Err(WorldError::InvalidRuntimeSnapshot);
        }
    }
    if greatest_drop_id >= snapshot.next_loot_drop_id.0
        || greatest_open_id >= snapshot.next_reward_open_id.0
        || snapshot
            .party_loot_cursors
            .iter()
            .any(|(party_id, cursor)| {
                *party_id == 0 || *cursor == 0 || !snapshot.adventure.parties.contains_key(party_id)
            })
    {
        return Err(WorldError::InvalidRuntimeSnapshot);
    }

    let mut party_members = BTreeSet::new();
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
    let npc_shape_valid = snapshot.npcs.iter().all(|(npc_id, npc)| {
        npc_id == &npc.stable_id
            && valid_definition_id(npc_id)
            && npc
                .engaged_character
                .is_none_or(|character| character.0 > 0)
            && npc
                .loot_item_ids
                .iter()
                .all(|item_id| valid_item_instance_id(item_id.as_str()))
    });
    if !adventure_valid || !npc_shape_valid {
        return Err(WorldError::InvalidRuntimeSnapshot);
    }

    validate_portable_trade_state(snapshot, &children)
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
            inventory_capacity: InventoryCapacity::default(),
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
    /// Durable instance ID used only to derive the observer-salted identity.
    pub stable_id: String,
    /// Authored prototype join key for reviewed presentation metadata.
    pub prototype_id: String,
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
    pub loot_item_ids: Vec<ItemInstanceId>,
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
    /// Transitional compatibility field. Canonical v4 placement lives in
    /// `WorldDefinition::initial_item_instances`.
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
    #[error("A world declares more than {MAX_ITEM_INSTANCES} item instances.")]
    TooManyItemInstances,
    #[error("Item prototype {0:?} is invalid.")]
    InvalidItemPrototype(String),
    #[error("Item instance points to missing prototype {0:?}.")]
    UnknownItemPrototype(String),
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
struct ItemPrototype {
    name: String,
    description: String,
    aliases: Vec<String>,
    equipment_slot: Option<EquipmentSlot>,
    attack_bonus: i32,
    heal_amount: i32,
    consumable: bool,
    effect: Option<ActiveEffect>,
    weight_grams: u32,
    slot_cost: u16,
    rarity: ItemRarity,
    item_level: u16,
    stack_policy: ItemStackPolicy,
    max_stack_units: u16,
    container: Option<ItemContainerDefinition>,
    transfer_policy: ItemTransferPolicy,
    sealed_loot_table_id: Option<String>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct ItemInstance {
    prototype_id: String,
    location: SharedItemLocation,
    binding: ItemBinding,
    item_affixes: Vec<AppliedItemAffix>,
}

#[derive(Clone, Debug)]
struct AffixPool {
    id: String,
    digest_sha256: String,
    entries: Vec<AffixPoolEntry>,
    total_weight: u64,
}

#[derive(Clone, Debug)]
struct LootTable {
    id: String,
    digest_sha256: String,
    guaranteed: Vec<LootGuaranteedDefinition>,
    pools: Vec<LootPoolDefinition>,
    worst_case_units: usize,
}

#[derive(Clone, Debug)]
struct LootSource {
    loot_table_id: String,
    rights: LootRightsPolicy,
}

#[derive(Clone, Debug)]
struct RolledLootUnit {
    prototype_id: String,
    entry_id: String,
    pool_id: Option<String>,
    roll_index: u8,
    quantity_index: u16,
    affix_pool_id: Option<String>,
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
    Contained(ItemInstanceId),
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
    loot_item_ids: Vec<ItemInstanceId>,
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

    fn uniform_below(&mut self, upper_exclusive: u64) -> Result<u64, WorldError> {
        if upper_exclusive == 0 {
            return Err(WorldError::InvalidGameplayDefinition);
        }
        let threshold = upper_exclusive.wrapping_neg() % upper_exclusive;
        loop {
            let value = self.next_u64();
            if value >= threshold {
                return Ok(value % upper_exclusive);
            }
        }
    }
}

#[derive(Clone, Debug)]
struct ObserverView {
    salt: [u8; 32],
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
    #[error("That item would exceed inventory slot or weight capacity.")]
    InventoryCapacityExceeded,
    #[error("That item cannot change ownership.")]
    ItemTransferForbidden,
    #[error("That recipient is not available for an item transfer.")]
    GiveTargetUnavailable,
    #[error("That trade action is not available.")]
    TradeUnavailable,
    #[error("That trade participant is not available.")]
    TradeTargetUnavailable,
    #[error("That item is reserved for a trade.")]
    ItemReservedForTrade,
    #[error("That loot action is not available.")]
    LootUnavailable,
    #[error("That sealed reward is not available.")]
    RewardUnavailable,
    #[error("That trade confirmation refers to an obsolete revision.")]
    StaleTradeRevision,
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
    CharacterRelocated {
        actor_id: CharacterId,
        actor_name: String,
        from_room_id: String,
        to_room_id: String,
    },
    ItemTaken {
        actor_id: CharacterId,
        actor_name: String,
        item_id: ItemInstanceId,
        item_name: String,
        room_id: String,
    },
    ItemDropped {
        actor_id: CharacterId,
        actor_name: String,
        item_id: ItemInstanceId,
        item_name: String,
        room_id: String,
    },
    ItemsMoved {
        actor_id: CharacterId,
        actor_name: String,
        item_ids: Vec<ItemInstanceId>,
        item_name: String,
        container_id: Option<ItemInstanceId>,
        operation: ContainerMoveOperation,
        room_id: String,
    },
    ItemsGiven {
        giver_id: CharacterId,
        giver_name: String,
        recipient_id: CharacterId,
        recipient_name: String,
        item_ids: Vec<ItemInstanceId>,
        item_name: String,
        room_id: String,
    },
    TradeRequested {
        trade: TradeSnapshot,
    },
    TradeOpened {
        previous: TradeSnapshot,
        current: TradeSnapshot,
    },
    TradeOfferSet {
        previous: TradeSnapshot,
        current: TradeSnapshot,
    },
    TradeConfirmed {
        participant: CharacterId,
        confirmation: TradeConfirmation,
    },
    TradeInvalidated {
        previous: TradeSnapshot,
        current: TradeSnapshot,
    },
    TradeCancelled {
        previous_trade: TradeSnapshot,
        reason: TradeCancellationReason,
    },
    TradeCompleted {
        previous_trade: TradeSnapshot,
    },
    Gameplay {
        event: GameplayEvent,
    },
    Adventure {
        event: AdventureEvent,
    },
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum ContainerMoveOperation {
    Take,
    Drop,
    Put,
    Get,
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
        item_id: ItemInstanceId,
        item_name: String,
        room_id: String,
    },
    LootGenerated {
        drop_id: LootDropId,
        npc_id: String,
        npc_name: String,
        item_ids: Vec<ItemInstanceId>,
        room_id: String,
    },
    LootRightsChanged {
        item_id: ItemInstanceId,
        room_id: String,
        summary: String,
    },
    LootGenerationSuppressed {
        drop_id: LootDropId,
        npc_id: String,
        npc_name: String,
        reason: String,
        room_id: String,
    },
    LootDespawned {
        item_ids: Vec<ItemInstanceId>,
        room_id: String,
    },
    LootEncounterCancelled {
        npc_id: String,
        npc_name: String,
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
        item_id: ItemInstanceId,
        item_name: String,
        slot: EquipmentSlot,
        room_id: String,
    },
    ItemUnequipped {
        character_id: CharacterId,
        character_name: String,
        item_id: ItemInstanceId,
        item_name: String,
        slot: EquipmentSlot,
        room_id: String,
    },
    ItemUsed {
        character_id: CharacterId,
        character_name: String,
        item_id: ItemInstanceId,
        item_name: String,
        room_id: String,
    },
    RewardOpened {
        open_id: RewardOpenId,
        character_id: CharacterId,
        character_name: String,
        source_item_id: ItemInstanceId,
        source_item_name: String,
        item_ids: Vec<ItemInstanceId>,
        item_names: Vec<String>,
        room_id: String,
    },
    CharacterHealed {
        character_id: CharacterId,
        character_name: String,
        amount: i32,
        health: i32,
        room_id: String,
    },
    CharacterRestored {
        character_id: CharacterId,
        character_name: String,
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
    ItemGiven,
    TradeRequested,
    TradeChanged,
    TradeCompleted,
    TradeCancelled,
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
    /// Present only for an explicit, paginated container inspection.
    pub container_contents: Option<ContainerContentsView>,
    /// Present only for an explicit inspection of an offered container.
    pub trade_contents: Option<TradeContentsView>,
    /// Present only for an explicit, bounded sealed-reward odds request.
    pub reward_odds: Option<RewardOddsView>,
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
    item_prototypes: HashMap<String, ItemPrototype>,
    /// Immutable authored identity mapping used to merge an older runtime
    /// snapshot with newly published initial instances without allowing a
    /// snapshot to retarget an existing authored ID to another prototype.
    initial_item_prototypes: BTreeMap<ItemInstanceId, String>,
    item_instances: BTreeMap<ItemInstanceId, ItemInstance>,
    next_item_instance_sequence: u64,
    item_provenance: BTreeMap<ItemInstanceId, ItemProvenance>,
    affixes: BTreeMap<String, AffixDefinition>,
    affix_pools: BTreeMap<String, AffixPool>,
    loot_tables: BTreeMap<String, LootTable>,
    loot_sources: BTreeMap<String, LootSource>,
    next_loot_drop_id: LootDropId,
    next_reward_open_id: RewardOpenId,
    loot_encounters: BTreeMap<String, LootEncounter>,
    ground_loot: BTreeMap<ItemInstanceId, GroundLootReservation>,
    party_loot_cursors: BTreeMap<u64, u64>,
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
    last_view_bump: Option<(u64, Vec<(SessionId, u64)>)>,
    start_room: EntityId,
    start_room_stable_id: String,
    revision: u64,
    logical_tick: u64,
    rng: DeterministicRng,
    adventure: AdventureRuntimeSnapshot,
    next_trade_id: TradeId,
    trades: BTreeMap<TradeId, TradeSnapshot>,
    /// Ephemeral abuse control. This deliberately does not enter the portable
    /// runtime snapshot and may reset on process restart.
    last_trade_request_ticks: HashMap<CharacterId, u64>,
    tick_resolution_pending: bool,
}

/// Exact in-memory checkpoint used around one durable commit. Unlike the
/// portable snapshots this also contains observer revision bookkeeping, so a
/// failed database transaction is invisible to connected clients.
#[derive(Clone)]
pub struct WorldCheckpoint {
    entities: SlotMap<EntityId, Entity>,
    item_instances: BTreeMap<ItemInstanceId, ItemInstance>,
    next_item_instance_sequence: u64,
    item_provenance: BTreeMap<ItemInstanceId, ItemProvenance>,
    next_reward_open_id: RewardOpenId,
    next_loot_drop_id: LootDropId,
    loot_encounters: BTreeMap<String, LootEncounter>,
    ground_loot: BTreeMap<ItemInstanceId, GroundLootReservation>,
    party_loot_cursors: BTreeMap<u64, u64>,
    npcs: BTreeMap<String, Npc>,
    character_gameplay: HashMap<CharacterId, CharacterGameplaySnapshot>,
    observer_views: HashMap<SessionId, ObserverView>,
    last_view_bump: Option<(u64, Vec<(SessionId, u64)>)>,
    revision: u64,
    logical_tick: u64,
    rng: DeterministicRng,
    adventure: AdventureRuntimeSnapshot,
    world_progression_flags: BTreeSet<String>,
    next_trade_id: TradeId,
    trades: BTreeMap<TradeId, TradeSnapshot>,
    last_trade_request_ticks: HashMap<CharacterId, u64>,
    tick_resolution_pending: bool,
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
        let mut item_prototypes = HashMap::with_capacity(definition.item_prototypes.len());
        for item in definition.item_prototypes {
            if item.prototype_id.trim().is_empty() {
                return Err(WorldBuildError::EmptyItemId);
            }
            if item.name.trim().is_empty() {
                return Err(WorldBuildError::EmptyItemName(item.prototype_id));
            }
            let stack_valid = match item.stack_policy {
                ItemStackPolicy::Unique => item.max_stack_units == 1,
                ItemStackPolicy::Fungible => (2..=1_000).contains(&item.max_stack_units),
            };
            let container_valid = item.container.as_ref().is_none_or(|container| {
                item.stack_policy == ItemStackPolicy::Unique
                    && item.max_stack_units == 1
                    && !item.consumable
                    && (1..=1_000).contains(&container.max_slots)
                    && container.max_weight_grams <= 1_000_000
            });
            let presentation_valid = valid_definition_id(&item.prototype_id)
                && safe_definition_text(&item.name, 128)
                && safe_definition_multiline_text(&item.description, 2_048)
                && item.aliases.len() <= 32
                && item
                    .aliases
                    .iter()
                    .all(|alias| safe_definition_text(alias, 64));
            let sealed_valid = item.sealed_loot_table_id.as_ref().is_none_or(|table_id| {
                valid_definition_id(table_id)
                    && item.stack_policy == ItemStackPolicy::Unique
                    && item.max_stack_units == 1
                    && item.consumable
                    && item.container.is_none()
                    && item.equipment_slot.is_none()
                    && item.attack_bonus == 0
                    && item.heal_amount == 0
                    && item.effect.is_none()
                    && item.transfer_policy == ItemTransferPolicy::CharacterBound
            });
            if !presentation_valid
                || item.weight_grams > 1_000_000
                || item.slot_cost == 0
                || item.slot_cost > 1_000
                || item.item_level == 0
                || item.item_level > 1_000
                || !stack_valid
                || !container_valid
                || !sealed_valid
            {
                return Err(WorldBuildError::InvalidItemPrototype(item.prototype_id));
            }
            let prototype_id = item.prototype_id;
            if item_prototypes.contains_key(&prototype_id) {
                return Err(WorldBuildError::DuplicateItemId(prototype_id));
            }
            item_prototypes.insert(
                prototype_id.clone(),
                ItemPrototype {
                    name: item.name,
                    description: item.description,
                    aliases: item.aliases,
                    equipment_slot: item.equipment_slot,
                    attack_bonus: item.attack_bonus,
                    heal_amount: item.heal_amount,
                    consumable: item.consumable,
                    effect: item.effect,
                    weight_grams: item.weight_grams,
                    slot_cost: item.slot_cost,
                    rarity: item.rarity,
                    item_level: item.item_level,
                    stack_policy: item.stack_policy,
                    max_stack_units: item.max_stack_units,
                    container: item.container,
                    transfer_policy: item.transfer_policy,
                    sealed_loot_table_id: item.sealed_loot_table_id,
                },
            );
        }
        let mut unresolved_exits = Vec::with_capacity(definition.rooms.len());

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

        let mut item_instances = BTreeMap::new();
        for instance in definition.initial_item_instances {
            let instance_id = instance.instance_id;
            if !valid_item_instance_id(instance_id.as_str()) {
                return Err(WorldBuildError::EmptyItemId);
            }
            if !item_prototypes.contains_key(&instance.prototype_id) {
                return Err(WorldBuildError::UnknownItemPrototype(instance.prototype_id));
            }
            if item_prototypes
                .get(&instance.prototype_id)
                .is_some_and(|prototype| prototype.sealed_loot_table_id.is_some())
            {
                return Err(WorldBuildError::InvalidItemPrototype(instance.prototype_id));
            }
            if item_instances.contains_key(&instance_id) {
                return Err(WorldBuildError::DuplicateItemId(instance_id.0));
            }
            let location = match instance.location {
                InitialItemLocation::Room(room_id) => {
                    if !stable_rooms.contains_key(&room_id) {
                        return Err(WorldBuildError::UnknownRoomItem {
                            room_id,
                            item_id: instance_id.0,
                        });
                    }
                    SharedItemLocation::Room(room_id)
                }
                InitialItemLocation::NpcSpawn(npc_id) => SharedItemLocation::HeldByNpc(npc_id),
                InitialItemLocation::Container(container_id) => {
                    SharedItemLocation::Contained(container_id)
                }
                InitialItemLocation::Staged => SharedItemLocation::Staged,
            };
            item_instances.insert(
                instance_id.clone(),
                ItemInstance {
                    prototype_id: instance.prototype_id,
                    location,
                    binding: ItemBinding::Unbound,
                    item_affixes: Vec::new(),
                },
            );
        }
        if item_instances.len() > MAX_ITEM_INSTANCES {
            return Err(WorldBuildError::TooManyItemInstances);
        }
        if validate_item_graph(&item_instances, &item_prototypes).is_err() {
            return Err(WorldBuildError::InvalidItemPrototype(
                "container graph".into(),
            ));
        }
        let mut staged = std::collections::HashSet::new();
        for item_id in staged_item_ids {
            let item_id = ItemInstanceId(item_id);
            if !staged.insert(item_id.clone()) {
                return Err(WorldBuildError::DuplicateStagedItem(item_id.0));
            }
            if !item_instances.contains_key(&item_id) {
                return Err(WorldBuildError::UnknownStagedItem(item_id.0));
            }
            if let Some(ItemInstance {
                location: SharedItemLocation::Room(room_id),
                ..
            }) = item_instances.get(&item_id)
            {
                return Err(WorldBuildError::ItemPlacedAndStaged {
                    item_id: item_id.0,
                    room_id: room_id.clone(),
                });
            }
            item_instances
                .get_mut(&item_id)
                .expect("validated staged instance exists")
                .location = SharedItemLocation::Staged;
        }

        let progression =
            ProgressionCatalog::compile(builtin_progression_definition(), &BTreeMap::new())
                .expect("the built-in progression is valid");
        let progression_catalog = progression.creation_view();
        let initial_item_prototypes = item_instances
            .iter()
            .map(|(id, instance)| (id.clone(), instance.prototype_id.clone()))
            .collect();
        let item_provenance = item_instances
            .keys()
            .cloned()
            .map(|id| (id, ItemProvenance::AuthoredInitial))
            .collect();
        let next_item_instance_sequence = item_instances
            .keys()
            .filter_map(runtime_item_sequence)
            .max()
            .unwrap_or(0)
            .checked_add(1)
            .ok_or_else(|| WorldBuildError::InvalidItemPrototype("runtime:*".into()))?;

        Ok(Self {
            entities,
            stable_rooms,
            item_prototypes,
            initial_item_prototypes,
            item_instances,
            next_item_instance_sequence,
            item_provenance,
            affixes: BTreeMap::new(),
            affix_pools: BTreeMap::new(),
            loot_tables: BTreeMap::new(),
            loot_sources: BTreeMap::new(),
            next_loot_drop_id: LootDropId(1),
            next_reward_open_id: RewardOpenId(1),
            loot_encounters: BTreeMap::new(),
            ground_loot: BTreeMap::new(),
            party_loot_cursors: BTreeMap::new(),
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
            last_view_bump: None,
            start_room,
            start_room_stable_id: definition.start_room_id,
            revision: 1,
            logical_tick: 0,
            rng: DeterministicRng::new(0x5449_414d_5544),
            adventure: AdventureRuntimeSnapshot::default(),
            next_trade_id: TradeId(1),
            trades: BTreeMap::new(),
            last_trade_request_ticks: HashMap::new(),
            tick_resolution_pending: false,
        })
    }

    pub fn demo() -> Self {
        Self::from_definition(WorldDefinition {
            start_room_id: "demo:lantern-square".into(),
            item_prototypes: vec![ItemPrototypeDefinition {
                prototype_id: "demo:blue-lantern".into(),
                name: "blue-glass lantern".into(),
                description: "Cold blue light pools inside the rain-speckled glass.".into(),
                aliases: vec!["lantern".into(), "lamp".into()],
                equipment_slot: None,
                attack_bonus: 0,
                heal_amount: 0,
                consumable: false,
                effect: None,
                weight_grams: 1_000,
                slot_cost: 1,
                rarity: ItemRarity::Common,
                item_level: 1,
                stack_policy: ItemStackPolicy::Unique,
                max_stack_units: 1,
                container: None,
                transfer_policy: ItemTransferPolicy::Transferable,
                sealed_loot_table_id: None,
            }],
            initial_item_instances: vec![InitialItemInstanceDefinition {
                instance_id: ItemInstanceId("demo:blue-lantern".into()),
                prototype_id: "demo:blue-lantern".into(),
                location: InitialItemLocation::Room("demo:lantern-square".into()),
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
                    item_ids: Vec::new(),
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
                },
                RoomDefinition {
                    stable_id: "demo:north-gate".into(),
                    title: "The North Gate".into(),
                    prose: vec![
                        "An iron-bound gate stands open between squat stone towers. The road continues into fields silvered by rain.".into(),
                    ],
                    image: None,
                    audio: None,
                    item_ids: Vec::new(),
                    exits: vec![ExitDefinition {
                        direction: Direction::South,
                        target_room_id: "demo:lantern-square".into(),
                    }],
                },
                RoomDefinition {
                    stable_id: "demo:archive".into(),
                    title: "The Cartographer's Archive".into(),
                    prose: vec![
                        "Shelves of annotated maps surround a broad oak table. A dry stair climbs back toward the square.".into(),
                    ],
                    image: None,
                    audio: None,
                    item_ids: Vec::new(),
                    exits: vec![ExitDefinition {
                        direction: Direction::Up,
                        target_room_id: "demo:lantern-square".into(),
                    }],
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

    pub fn configure_affixes(
        &mut self,
        mut definitions: Vec<AffixDefinition>,
        mut pools: Vec<AffixPoolDefinition>,
    ) -> Result<(), WorldError> {
        if definitions.len() > MAX_AFFIX_DEFINITIONS || pools.len() > MAX_AFFIX_POOLS {
            return Err(WorldError::InvalidGameplayDefinition);
        }
        definitions.sort_by(|left, right| left.id.cmp(&right.id));
        pools.sort_by(|left, right| left.id.cmp(&right.id));
        let mut compiled_definitions = BTreeMap::new();
        for definition in definitions {
            let valid_delta = (0..=MAX_AFFIX_MODIFIER).contains(&definition.attack_bonus_delta)
                && (0..=MAX_AFFIX_MODIFIER).contains(&definition.heal_amount_delta)
                && (definition.attack_bonus_delta > 0 || definition.heal_amount_delta > 0);
            if !valid_definition_id(&definition.id)
                || !safe_definition_text(&definition.label, 64)
                || !valid_delta
                || compiled_definitions
                    .insert(definition.id.clone(), definition)
                    .is_some()
            {
                return Err(WorldError::InvalidGameplayDefinition);
            }
        }
        let mut compiled_pools = BTreeMap::new();
        let mut total_entries = 0_usize;
        for mut pool in pools {
            pool.entries.sort_by(|left, right| left.id.cmp(&right.id));
            total_entries = total_entries
                .checked_add(pool.entries.len())
                .ok_or(WorldError::InvalidGameplayDefinition)?;
            if !valid_definition_id(&pool.id)
                || pool.entries.is_empty()
                || pool.entries.len() > MAX_AFFIX_POOL_ENTRIES
                || total_entries > MAX_TOTAL_AFFIX_POOL_ENTRIES
            {
                return Err(WorldError::InvalidGameplayDefinition);
            }
            let digest_sha256 = canonical_sha256(&pool)?;
            let mut ids = BTreeSet::new();
            let mut total_weight = 0_u64;
            for entry in &pool.entries {
                if !ids.insert(entry.id.clone())
                    || !valid_definition_id(&entry.id)
                    || !(1..=1_000_000).contains(&entry.weight)
                {
                    return Err(WorldError::InvalidGameplayDefinition);
                }
                total_weight = total_weight
                    .checked_add(u64::from(entry.weight))
                    .ok_or(WorldError::InvalidGameplayDefinition)?;
                match entry.kind {
                    AffixPoolEntryKind::Affix => {
                        if entry
                            .affix_id
                            .as_ref()
                            .is_none_or(|id| !compiled_definitions.contains_key(id))
                        {
                            return Err(WorldError::InvalidGameplayDefinition);
                        }
                    }
                    AffixPoolEntryKind::Nothing if entry.affix_id.is_none() => {}
                    AffixPoolEntryKind::Nothing => {
                        return Err(WorldError::InvalidGameplayDefinition);
                    }
                }
            }
            if compiled_pools
                .insert(
                    pool.id.clone(),
                    AffixPool {
                        id: pool.id,
                        digest_sha256,
                        entries: pool.entries,
                        total_weight,
                    },
                )
                .is_some()
            {
                return Err(WorldError::InvalidGameplayDefinition);
            }
        }
        self.affixes = compiled_definitions;
        self.affix_pools = compiled_pools;
        Ok(())
    }

    pub fn configure_loot(
        &mut self,
        mut tables: Vec<LootTableDefinition>,
        mut sources: Vec<LootSourceDefinition>,
    ) -> Result<(), WorldError> {
        if tables.len() > 256 || sources.len() > 1_024 {
            return Err(WorldError::InvalidGameplayDefinition);
        }
        tables.sort_by(|left, right| left.id.cmp(&right.id));
        sources.sort_by(|left, right| left.id.cmp(&right.id));
        let sealed_table_ids = self
            .item_prototypes
            .values()
            .filter_map(|prototype| prototype.sealed_loot_table_id.clone())
            .collect::<BTreeSet<_>>();
        let mut compiled = BTreeMap::new();
        let mut total_entries = 0_usize;
        for mut table in tables {
            if !valid_definition_id(&table.id)
                || table.digest_sha256.len() != 64
                || !table
                    .digest_sha256
                    .bytes()
                    .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
                || table.guaranteed.len() > 8
                || table.pools.len() > 8
            {
                return Err(WorldError::InvalidGameplayDefinition);
            }
            table
                .guaranteed
                .sort_by(|left, right| left.id.cmp(&right.id));
            table.pools.sort_by(|left, right| left.id.cmp(&right.id));
            let mut ids = BTreeSet::new();
            let mut worst_case_units = 0_usize;
            let mut minimum_units = 0_usize;
            let mut odds_rows = 0_usize;
            let sealed_table = sealed_table_ids.contains(&table.id);
            for guaranteed in &table.guaranteed {
                let prototype = self
                    .item_prototypes
                    .get(&guaranteed.item_prototype_id)
                    .ok_or(WorldError::InvalidGameplayDefinition)?;
                if !ids.insert(guaranteed.id.clone())
                    || !valid_definition_id(&guaranteed.id)
                    || !(1..=32).contains(&guaranteed.quantity)
                    || guaranteed.affix_pool_id.as_ref().is_some_and(|pool_id| {
                        guaranteed.quantity != 1
                            || !affix_pool_applies(
                                pool_id,
                                prototype,
                                &self.affix_pools,
                                &self.affixes,
                            )
                    })
                    || (sealed_table && prototype.sealed_loot_table_id.is_some())
                {
                    return Err(WorldError::InvalidGameplayDefinition);
                }
                worst_case_units = worst_case_units
                    .checked_add(usize::from(guaranteed.quantity))
                    .ok_or(WorldError::InvalidGameplayDefinition)?;
                minimum_units = minimum_units
                    .checked_add(usize::from(guaranteed.quantity))
                    .ok_or(WorldError::InvalidGameplayDefinition)?;
                if sealed_table {
                    odds_rows = odds_rows
                        .checked_add(1)
                        .and_then(|rows| {
                            guaranteed
                                .affix_pool_id
                                .as_ref()
                                .map_or(Some(rows), |pool_id| {
                                    rows.checked_add(self.affix_pools.get(pool_id)?.entries.len())
                                })
                        })
                        .ok_or(WorldError::InvalidGameplayDefinition)?;
                }
            }
            for pool in &mut table.pools {
                if !ids.insert(pool.id.clone())
                    || !valid_definition_id(&pool.id)
                    || !(1..=8).contains(&pool.rolls)
                    || pool.entries.is_empty()
                    || pool.entries.len() > 64
                {
                    return Err(WorldError::InvalidGameplayDefinition);
                }
                pool.entries.sort_by(|left, right| left.id.cmp(&right.id));
                total_entries = total_entries
                    .checked_add(pool.entries.len())
                    .ok_or(WorldError::InvalidGameplayDefinition)?;
                let mut total_weight = 0_u64;
                let mut maximum_quantity = 0_u16;
                let mut minimum_quantity = u16::MAX;
                let mut can_roll_nothing = false;
                for entry in &pool.entries {
                    if !ids.insert(entry.id.clone())
                        || !valid_definition_id(&entry.id)
                        || !(1..=1_000_000).contains(&entry.weight)
                    {
                        return Err(WorldError::InvalidGameplayDefinition);
                    }
                    total_weight = total_weight
                        .checked_add(u64::from(entry.weight))
                        .ok_or(WorldError::InvalidGameplayDefinition)?;
                    if sealed_table {
                        odds_rows = odds_rows
                            .checked_add(1)
                            .and_then(|rows| {
                                entry.affix_pool_id.as_ref().map_or(Some(rows), |pool_id| {
                                    rows.checked_add(self.affix_pools.get(pool_id)?.entries.len())
                                })
                            })
                            .ok_or(WorldError::InvalidGameplayDefinition)?;
                    }
                    match entry.kind {
                        LootEntryKind::Item => {
                            let prototype_id = entry
                                .item_prototype_id
                                .as_ref()
                                .ok_or(WorldError::InvalidGameplayDefinition)?;
                            let minimum = entry
                                .min_quantity
                                .ok_or(WorldError::InvalidGameplayDefinition)?;
                            let maximum = entry
                                .max_quantity
                                .ok_or(WorldError::InvalidGameplayDefinition)?;
                            let prototype = self
                                .item_prototypes
                                .get(prototype_id)
                                .ok_or(WorldError::InvalidGameplayDefinition)?;
                            if minimum == 0
                                || minimum > maximum
                                || maximum > 32
                                || entry.affix_pool_id.as_ref().is_some_and(|pool_id| {
                                    minimum != 1
                                        || maximum != 1
                                        || !affix_pool_applies(
                                            pool_id,
                                            prototype,
                                            &self.affix_pools,
                                            &self.affixes,
                                        )
                                })
                                || (sealed_table && prototype.sealed_loot_table_id.is_some())
                            {
                                return Err(WorldError::InvalidGameplayDefinition);
                            }
                            maximum_quantity = maximum_quantity.max(maximum);
                            minimum_quantity = minimum_quantity.min(minimum);
                        }
                        LootEntryKind::Nothing => {
                            if entry.item_prototype_id.is_some()
                                || entry.min_quantity.is_some()
                                || entry.max_quantity.is_some()
                                || entry.affix_pool_id.is_some()
                            {
                                return Err(WorldError::InvalidGameplayDefinition);
                            }
                            can_roll_nothing = true;
                        }
                    }
                }
                if total_weight == 0 {
                    return Err(WorldError::InvalidGameplayDefinition);
                }
                worst_case_units = worst_case_units
                    .checked_add(usize::from(pool.rolls) * usize::from(maximum_quantity))
                    .ok_or(WorldError::InvalidGameplayDefinition)?;
                if !can_roll_nothing {
                    minimum_units = minimum_units
                        .checked_add(usize::from(pool.rolls) * usize::from(minimum_quantity))
                        .ok_or(WorldError::InvalidGameplayDefinition)?;
                }
            }
            if total_entries > 4_096
                || worst_case_units > MAX_LOOT_UNITS_PER_RESOLUTION
                || (sealed_table
                    && (minimum_units == 0
                        || worst_case_units > MAX_SEALED_REWARD_UNITS
                        || odds_rows > MAX_REWARD_ODDS_ROWS))
            {
                return Err(WorldError::InvalidGameplayDefinition);
            }
            let id = table.id.clone();
            if compiled
                .insert(
                    id.clone(),
                    LootTable {
                        id,
                        digest_sha256: table.digest_sha256,
                        guaranteed: table.guaranteed,
                        pools: table.pools,
                        worst_case_units,
                    },
                )
                .is_some()
            {
                return Err(WorldError::InvalidGameplayDefinition);
            }
        }
        let mut compiled_sources = BTreeMap::new();
        let mut used_tables = BTreeSet::new();
        let mut source_ids = BTreeSet::new();
        for source in sources {
            if source.kind != LootSourceKind::NpcSpawn
                || !valid_definition_id(&source.id)
                || !source_ids.insert(source.id.clone())
                || !valid_definition_id(&source.npc_spawn_id)
                || !compiled.contains_key(&source.loot_table_id)
                || sealed_table_ids.contains(&source.loot_table_id)
                || !self.npcs.contains_key(&source.npc_spawn_id)
            {
                return Err(WorldError::InvalidGameplayDefinition);
            }
            let combined_worst_case = self
                .npcs
                .get(&source.npc_spawn_id)
                .ok_or(WorldError::InvalidGameplayDefinition)?
                .loot_item_ids
                .len()
                .checked_add(
                    compiled
                        .get(&source.loot_table_id)
                        .ok_or(WorldError::InvalidGameplayDefinition)?
                        .worst_case_units,
                )
                .ok_or(WorldError::InvalidGameplayDefinition)?;
            if combined_worst_case > MAX_LOOT_UNITS_PER_RESOLUTION {
                return Err(WorldError::InvalidGameplayDefinition);
            }
            used_tables.insert(source.loot_table_id.clone());
            let npc_spawn_id = source.npc_spawn_id;
            if compiled_sources
                .insert(
                    npc_spawn_id,
                    LootSource {
                        loot_table_id: source.loot_table_id,
                        rights: source.rights,
                    },
                )
                .is_some()
            {
                return Err(WorldError::InvalidGameplayDefinition);
            }
        }
        used_tables.extend(sealed_table_ids.iter().cloned());
        if sealed_table_ids.iter().any(|id| !compiled.contains_key(id))
            || compiled.keys().any(|id| !used_tables.contains(id))
        {
            return Err(WorldError::InvalidGameplayDefinition);
        }
        self.loot_tables = compiled;
        self.loot_sources = compiled_sources;
        Ok(())
    }

    /// Allocates one durable repeatable instance without publishing it into a
    /// room or inventory. The caller must include creation/assignment in the
    /// same checkpointed durable commit before making the instance visible.
    pub fn create_staged_item_instance(
        &mut self,
        prototype_id: &str,
    ) -> Result<ItemInstanceId, WorldError> {
        if !self.item_prototypes.contains_key(prototype_id)
            || self.item_instances.len() >= MAX_ITEM_INSTANCES
        {
            return Err(WorldError::InvalidGameplayDefinition);
        }
        loop {
            let sequence = self.next_item_instance_sequence;
            self.next_item_instance_sequence = sequence
                .checked_add(1)
                .ok_or(WorldError::InvalidGameplayDefinition)?;
            let instance_id = ItemInstanceId(format!("runtime:{sequence}"));
            if self.item_instances.contains_key(&instance_id) {
                continue;
            }
            self.item_instances.insert(
                instance_id.clone(),
                ItemInstance {
                    prototype_id: prototype_id.to_owned(),
                    location: SharedItemLocation::Staged,
                    binding: ItemBinding::Unbound,
                    item_affixes: Vec::new(),
                },
            );
            self.item_provenance.insert(
                instance_id.clone(),
                ItemProvenance::RuntimeCreated {
                    created_tick: self.logical_tick,
                },
            );
            return Ok(instance_id);
        }
    }

    fn item_prototype_for(
        &self,
        instance_id: &ItemInstanceId,
    ) -> Result<&ItemPrototype, WorldError> {
        let instance = self
            .item_instances
            .get(instance_id)
            .ok_or(WorldError::InvalidEntity)?;
        let prototype = self
            .item_prototypes
            .get(&instance.prototype_id)
            .ok_or(WorldError::InvalidEntity)?;
        debug_assert!(matches!(
            prototype.transfer_policy,
            ItemTransferPolicy::Transferable
                | ItemTransferPolicy::CharacterBound
                | ItemTransferPolicy::NonTransferable
        ));
        Ok(prototype)
    }

    fn item_gameplay_for(
        &self,
        instance_id: &ItemInstanceId,
    ) -> Result<&ItemPrototype, WorldError> {
        self.item_prototype_for(instance_id)
    }

    fn item_affix(&self, instance_id: &ItemInstanceId) -> Option<&AppliedItemAffix> {
        self.item_instances
            .get(instance_id)
            .and_then(|instance| instance.item_affixes.first())
    }

    fn item_affix_view(&self, instance_id: &ItemInstanceId) -> Option<ItemAffixView> {
        self.item_affix(instance_id).map(|affix| ItemAffixView {
            label: affix.label.clone(),
            position: affix.position,
            attack_bonus_delta: affix.attack_bonus_delta,
            heal_amount_delta: affix.heal_amount_delta,
        })
    }

    fn item_effective_name(&self, instance_id: &ItemInstanceId) -> Result<String, WorldError> {
        let prototype = self.item_prototype_for(instance_id)?;
        Ok(match self.item_affix(instance_id) {
            Some(affix) if affix.position == AffixPosition::Prefix => {
                format!("{} {}", affix.label, prototype.name)
            }
            Some(affix) => format!("{} {}", prototype.name, affix.label),
            None => prototype.name.clone(),
        })
    }

    fn item_effective_attack_bonus(&self, instance_id: &ItemInstanceId) -> Result<i32, WorldError> {
        let prototype = self.item_prototype_for(instance_id)?;
        prototype
            .attack_bonus
            .checked_add(
                self.item_affix(instance_id)
                    .map_or(0, |affix| affix.attack_bonus_delta),
            )
            .ok_or(WorldError::InvalidEntity)
    }

    fn item_effective_heal_amount(&self, instance_id: &ItemInstanceId) -> Result<i32, WorldError> {
        let prototype = self.item_prototype_for(instance_id)?;
        prototype
            .heal_amount
            .checked_add(
                self.item_affix(instance_id)
                    .map_or(0, |affix| affix.heal_amount_delta),
            )
            .ok_or(WorldError::InvalidEntity)
    }

    fn item_is_openable_reward(
        &self,
        instance_id: &ItemInstanceId,
        character_id: CharacterId,
    ) -> bool {
        let Some(instance) = self.item_instances.get(instance_id) else {
            return false;
        };
        let Some(prototype) = self.item_prototypes.get(&instance.prototype_id) else {
            return false;
        };
        matches!(instance.location, SharedItemLocation::Carried(owner) if owner == character_id)
            && instance.binding == (ItemBinding::Character { character_id })
            && instance.item_affixes.is_empty()
            && prototype.transfer_policy == ItemTransferPolicy::CharacterBound
            && prototype.stack_policy == ItemStackPolicy::Unique
            && prototype.max_stack_units == 1
            && prototype.consumable
            && prototype.container.is_none()
            && prototype.equipment_slot.is_none()
            && prototype.attack_bonus == 0
            && prototype.heal_amount == 0
            && prototype.effect.is_none()
            && prototype
                .sealed_loot_table_id
                .as_ref()
                .is_some_and(|table_id| {
                    self.loot_tables.contains_key(table_id)
                        && matches!(
                            self.item_provenance.get(instance_id),
                            Some(ItemProvenance::NpcLoot {
                                sealed_loot_table_id: Some(authored_table_id),
                                ..
                            }) if authored_table_id == table_id
                        )
                })
    }

    fn item_location(&self, instance_id: &ItemInstanceId) -> Option<&SharedItemLocation> {
        self.item_instances
            .get(instance_id)
            .map(|instance| &instance.location)
    }

    fn set_item_location(
        &mut self,
        instance_id: &ItemInstanceId,
        location: SharedItemLocation,
    ) -> Result<(), WorldError> {
        self.item_instances
            .get_mut(instance_id)
            .ok_or(WorldError::InvalidEntity)?
            .location = location;
        Ok(())
    }

    pub fn checkpoint(&self) -> WorldCheckpoint {
        WorldCheckpoint {
            entities: self.entities.clone(),
            item_instances: self.item_instances.clone(),
            next_item_instance_sequence: self.next_item_instance_sequence,
            item_provenance: self.item_provenance.clone(),
            next_reward_open_id: self.next_reward_open_id,
            next_loot_drop_id: self.next_loot_drop_id,
            loot_encounters: self.loot_encounters.clone(),
            ground_loot: self.ground_loot.clone(),
            party_loot_cursors: self.party_loot_cursors.clone(),
            npcs: self.npcs.clone(),
            character_gameplay: self.character_gameplay.clone(),
            observer_views: self.observer_views.clone(),
            last_view_bump: self.last_view_bump.clone(),
            revision: self.revision,
            logical_tick: self.logical_tick,
            rng: self.rng.clone(),
            adventure: self.adventure.clone(),
            world_progression_flags: self.world_progression_flags.clone(),
            next_trade_id: self.next_trade_id,
            trades: self.trades.clone(),
            last_trade_request_ticks: self.last_trade_request_ticks.clone(),
            tick_resolution_pending: self.tick_resolution_pending,
        }
    }

    pub fn restore_checkpoint(&mut self, checkpoint: WorldCheckpoint) {
        self.entities = checkpoint.entities;
        self.item_instances = checkpoint.item_instances;
        self.next_item_instance_sequence = checkpoint.next_item_instance_sequence;
        self.item_provenance = checkpoint.item_provenance;
        self.next_reward_open_id = checkpoint.next_reward_open_id;
        self.next_loot_drop_id = checkpoint.next_loot_drop_id;
        self.loot_encounters = checkpoint.loot_encounters;
        self.ground_loot = checkpoint.ground_loot;
        self.party_loot_cursors = checkpoint.party_loot_cursors;
        self.npcs = checkpoint.npcs;
        self.character_gameplay = checkpoint.character_gameplay;
        self.observer_views = checkpoint.observer_views;
        self.last_view_bump = checkpoint.last_view_bump;
        self.revision = checkpoint.revision;
        self.logical_tick = checkpoint.logical_tick;
        self.rng = checkpoint.rng;
        self.adventure = checkpoint.adventure;
        self.world_progression_flags = checkpoint.world_progression_flags;
        self.next_trade_id = checkpoint.next_trade_id;
        self.trades = checkpoint.trades;
        self.last_trade_request_ticks = checkpoint.last_trade_request_ticks;
        self.tick_resolution_pending = checkpoint.tick_resolution_pending;
    }

    pub fn runtime_snapshot(&self) -> WorldRuntimeSnapshot {
        WorldRuntimeSnapshot {
            snapshot_version: WORLD_RUNTIME_SNAPSHOT_VERSION,
            revision: self.revision,
            logical_tick: self.logical_tick,
            rng_state: self.rng.state,
            next_item_instance_sequence: self.next_item_instance_sequence,
            item_instances: self
                .item_instances
                .iter()
                .map(|(item_id, instance)| {
                    let location = match &instance.location {
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
                        SharedItemLocation::Contained(container_id) => {
                            RuntimeItemLocation::Contained {
                                container_id: container_id.clone(),
                            }
                        }
                        SharedItemLocation::Consumed => RuntimeItemLocation::Consumed,
                    };
                    (
                        item_id.clone(),
                        RuntimeItemInstanceSnapshot {
                            prototype_id: instance.prototype_id.clone(),
                            location,
                            binding: instance.binding,
                            item_affixes: instance.item_affixes.clone(),
                        },
                    )
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
            next_trade_id: self.next_trade_id,
            trades: self.trades.clone(),
            next_loot_drop_id: self.next_loot_drop_id,
            item_provenance: self.item_provenance.clone(),
            loot_encounters: self.loot_encounters.clone(),
            ground_loot: self.ground_loot.clone(),
            party_loot_cursors: self.party_loot_cursors.clone(),
            next_reward_open_id: self.next_reward_open_id,
        }
    }

    pub fn restore_runtime_snapshot(
        &mut self,
        mut snapshot: WorldRuntimeSnapshot,
    ) -> Result<(), WorldError> {
        validate_world_runtime_snapshot_semantics(&snapshot)?;
        let item_ids_match = snapshot.item_instances.iter().all(|(item_id, instance)| {
            self.item_prototypes.contains_key(&instance.prototype_id)
                && match self.initial_item_prototypes.get(item_id) {
                    Some(prototype_id) => prototype_id == &instance.prototype_id,
                    None => runtime_item_sequence(item_id).is_some(),
                }
        });
        let npc_ids_match = snapshot
            .npcs
            .iter()
            .all(|(npc_id, npc)| npc_id == &npc.stable_id && self.npcs.contains_key(npc_id));
        if !item_ids_match || !npc_ids_match {
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

        // An older snapshot cannot know about initial instances introduced by
        // a newer WorldPack. Preserve those authored instances from the fresh
        // world, then overlay every durable location from the snapshot. Do not
        // preserve runtime-created instances omitted by the snapshot.
        let mut item_instances = self
            .item_instances
            .iter()
            .filter(|(item_id, _)| self.initial_item_prototypes.contains_key(*item_id))
            .map(|(item_id, instance)| (item_id.clone(), instance.clone()))
            .collect::<BTreeMap<_, _>>();
        for (item_id, saved_instance) in &snapshot.item_instances {
            let location = match &saved_instance.location {
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
                        .item_prototypes
                        .get(&saved_instance.prototype_id)
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
                RuntimeItemLocation::Contained { container_id } => {
                    SharedItemLocation::Contained(container_id.clone())
                }
                RuntimeItemLocation::Consumed => SharedItemLocation::Consumed,
            };
            item_instances.insert(
                item_id.clone(),
                ItemInstance {
                    prototype_id: saved_instance.prototype_id.clone(),
                    location,
                    binding: saved_instance.binding,
                    item_affixes: saved_instance.item_affixes.clone(),
                },
            );
        }
        let mut item_provenance = item_instances
            .keys()
            .filter(|item_id| self.initial_item_prototypes.contains_key(*item_id))
            .cloned()
            .map(|item_id| (item_id, ItemProvenance::AuthoredInitial))
            .collect::<BTreeMap<_, _>>();
        for (item_id, provenance) in &snapshot.item_provenance {
            item_provenance.insert(item_id.clone(), provenance.clone());
        }
        if item_provenance.keys().ne(item_instances.keys()) {
            return Err(WorldError::InvalidRuntimeSnapshot);
        }

        if validate_item_graph(&item_instances, &self.item_prototypes).is_err()
            || validate_restored_item_affixes(&item_instances, &self.item_prototypes).is_err()
            || validate_restored_sealed_items(
                &item_instances,
                &item_provenance,
                &snapshot.ground_loot,
                &self.item_prototypes,
            )
            .is_err()
            || npcs.values().any(|npc| {
                npc.loot_item_ids
                    .iter()
                    .any(|item_id| !item_instances.contains_key(item_id))
            })
            || item_instances.iter().any(|(item_id, instance)| {
                matches!(
                    &instance.location,
                    SharedItemLocation::HeldByNpc(owner)
                        if npcs
                            .get(owner)
                            .is_none_or(|npc| !npc.loot_item_ids.contains(item_id))
                )
            })
        {
            return Err(WorldError::InvalidRuntimeSnapshot);
        }
        validate_trade_state(
            snapshot.next_trade_id,
            &snapshot.trades,
            &item_instances,
            &self.item_prototypes,
            &self.stable_rooms,
        )?;

        self.revision = snapshot.revision;
        self.logical_tick = snapshot.logical_tick;
        self.rng = DeterministicRng {
            state: snapshot.rng_state,
        };
        self.npcs = npcs;
        self.item_instances = item_instances;
        self.item_provenance = item_provenance;
        self.next_item_instance_sequence = snapshot
            .next_item_instance_sequence
            .max(self.next_item_instance_sequence);
        snapshot.adventure.next_weather_tick = snapshot
            .adventure
            .next_weather_tick
            .max(snapshot.logical_tick.saturating_add(1));
        self.adventure = snapshot.adventure;
        self.world_progression_flags = snapshot.progression_flags;
        self.next_trade_id = snapshot.next_trade_id;
        self.trades = snapshot.trades;
        self.next_loot_drop_id = snapshot.next_loot_drop_id;
        self.loot_encounters = snapshot.loot_encounters;
        self.ground_loot = snapshot.ground_loot;
        self.party_loot_cursors = snapshot.party_loot_cursors;
        self.next_reward_open_id = snapshot.next_reward_open_id;
        self.tick_resolution_pending = false;
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
        let item_id = ItemInstanceId(item_id.to_owned());
        let item = self
            .item_prototype_for(&item_id)
            .map_err(|_| WorldError::ItemNotFound(item_id.0.clone()))?;
        let item_name = item.name.clone();
        match self.item_location(&item_id) {
            Some(SharedItemLocation::Carried(owner)) if *owner == actor_id => return Ok(None),
            Some(SharedItemLocation::Room(owner_room)) if owner_room == &room_id => {}
            _ => return Err(WorldError::GameplayUnavailable),
        }
        let mut prospective = self.item_instances.clone();
        prospective
            .get_mut(&item_id)
            .ok_or(WorldError::InvalidEntity)?
            .location = SharedItemLocation::Carried(actor_id);
        self.apply_item_ownership_transitions(&mut prospective, std::slice::from_ref(&item_id))?;
        self.validate_prospective_item_locations(&prospective, [actor_id])?;
        self.item_instances = prospective;
        self.revision = self.revision.wrapping_add(1).max(1);
        let event = DomainEvent::ItemTaken {
            actor_id,
            actor_name,
            item_id,
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

    pub fn validate_character_gameplay_snapshot(
        &self,
        character_id: CharacterId,
        snapshot: &CharacterGameplaySnapshot,
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
            .validate_snapshot(snapshot, &self.abilities)
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
                self.item_instances.contains_key(item_id)
                    && self
                        .item_gameplay_for(item_id)
                        .ok()
                        .is_some_and(|item| item.equipment_slot == Some(*slot))
                    && matches!(
                        self.item_location(item_id),
                        Some(SharedItemLocation::Equipped(owner, equipped_slot))
                            if *owner == character_id && *equipped_slot == *slot
                    )
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
        let equipment_locations_exact =
            self.item_instances
                .iter()
                .all(|(item_id, instance)| match &instance.location {
                    SharedItemLocation::Equipped(owner, slot) if *owner == character_id => {
                        snapshot.equipment.get(slot) == Some(item_id)
                    }
                    _ => true,
                });
        let capacity_valid = snapshot.inventory_capacity.max_slots > 0
            && snapshot.inventory_capacity.max_slots <= 1_000
            && snapshot.inventory_capacity.max_weight_grams <= 1_000_000
            && self.inventory_usage_for_capacity(character_id, &snapshot.inventory_capacity)?;
        if !valid || !adventure_valid || !equipment_locations_exact || !capacity_valid {
            return Err(WorldError::InvalidGameplayDefinition);
        }
        Ok(())
    }

    pub fn restore_character_gameplay(
        &mut self,
        character_id: CharacterId,
        snapshot: CharacterGameplaySnapshot,
    ) -> Result<(), WorldError> {
        self.validate_character_gameplay_snapshot(character_id, &snapshot)?;
        self.character_gameplay.insert(character_id, snapshot);
        Ok(())
    }

    pub fn runtime_item_owner_ids(&self) -> BTreeSet<CharacterId> {
        self.item_instances
            .iter()
            .filter_map(|(item_id, instance)| {
                self.item_root_owner(item_id)
                    .ok()
                    .flatten()
                    .or(match instance.binding {
                        ItemBinding::Unbound => None,
                        ItemBinding::Character { character_id } => Some(character_id),
                    })
            })
            .collect()
    }

    fn item_root_owner(&self, item_id: &ItemInstanceId) -> Result<Option<CharacterId>, WorldError> {
        let mut current = item_id;
        for _ in 0..=MAX_CONTAINER_DEPTH {
            let instance = self
                .item_instances
                .get(current)
                .ok_or(WorldError::InvalidEntity)?;
            match &instance.location {
                SharedItemLocation::Carried(character_id)
                | SharedItemLocation::Equipped(character_id, _) => {
                    return Ok(Some(*character_id));
                }
                SharedItemLocation::Contained(parent) => current = parent,
                SharedItemLocation::Staged
                | SharedItemLocation::Room(_)
                | SharedItemLocation::HeldByNpc(_)
                | SharedItemLocation::Consumed => return Ok(None),
            }
        }
        Err(WorldError::InvalidEntity)
    }

    pub fn configure_item_gameplay(
        &mut self,
        definition: ItemGameplayDefinition,
    ) -> Result<(), WorldError> {
        let max_stat_value = i32::try_from(MAX_STAT_VALUE).expect("MAX_STAT_VALUE fits in i32");
        let valid = self.item_prototypes.contains_key(&definition.item_id)
            && (0..=max_stat_value).contains(&definition.attack_bonus)
            && (0..=max_stat_value).contains(&definition.heal_amount)
            && definition.effect.as_ref().is_none_or(|effect| {
                (1..=MAX_STATUS_MAGNITUDE).contains(&effect.magnitude)
                    && (1..=MAX_STATUS_TICKS).contains(&effect.remaining_ticks)
            });
        if !valid {
            return Err(WorldError::InvalidGameplayDefinition);
        }
        let prototype = self
            .item_prototypes
            .get_mut(&definition.item_id)
            .ok_or(WorldError::InvalidGameplayDefinition)?;
        prototype.equipment_slot = definition.equipment_slot;
        prototype.attack_bonus = definition.attack_bonus;
        prototype.heal_amount = definition.heal_amount;
        prototype.consumable = definition.consumable;
        prototype.effect = definition.effect;
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
        if !self.item_instances.contains_key(item_id) || !(1..=100).contains(&difficulty) {
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
            && definition.loot_item_ids.len() <= MAX_LOOT_UNITS_PER_RESOLUTION
            && definition.loot_item_ids.iter().all(|item_id| {
                self.item_instances.contains_key(item_id)
                    && (matches!(
                        self.item_location(item_id),
                        Some(SharedItemLocation::Staged)
                    ) || matches!(
                        self.item_location(item_id),
                        Some(SharedItemLocation::HeldByNpc(owner))
                            if owner == &definition.stable_id
                    ))
            });
        if !valid {
            return Err(WorldError::InvalidGameplayDefinition);
        }
        for item_id in &definition.loot_item_ids {
            self.set_item_location(
                item_id,
                SharedItemLocation::HeldByNpc(definition.stable_id.clone()),
            )?;
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
        let advanced = (|| {
            self.begin_tick()?;
            let mut events = self.cancel_expired_trades()?;
            events.extend(self.expire_ground_loot()?);
            events.extend(self.resolve_tick_after_expiry()?);
            Ok(events)
        })();
        match advanced {
            Ok(events) => Ok(events),
            Err(error) => {
                self.restore_checkpoint(checkpoint);
                Err(error)
            }
        }
    }

    /// Opens a logical tick without resolving effects, respawns or combat.
    /// The actor cancels expired trades and expires ground loot next, then
    /// calls `resolve_tick_after_expiry` exactly once.
    pub fn begin_tick(&mut self) -> Result<(), WorldError> {
        if self.tick_resolution_pending {
            return Err(WorldError::InvalidDomainTransition);
        }
        let next_tick = self
            .logical_tick
            .checked_add(1)
            .ok_or(WorldError::InvalidDomainTransition)?;
        self.logical_tick = next_tick;
        self.tick_resolution_pending = true;
        Ok(())
    }

    /// Resolves the tick opened by `begin_tick`. Expiration must already have
    /// been processed, so combat cannot observe an expired reservation.
    pub fn resolve_tick_after_expiry(&mut self) -> Result<Vec<DomainEvent>, WorldError> {
        if !self.tick_resolution_pending {
            return Err(WorldError::InvalidDomainTransition);
        }
        let checkpoint = self.checkpoint();
        match self.resolve_tick_after_expiry_inner() {
            Ok(events) => Ok(events),
            Err(error) => {
                self.restore_checkpoint(checkpoint);
                Err(error)
            }
        }
    }

    fn resolve_tick_after_expiry_inner(&mut self) -> Result<Vec<DomainEvent>, WorldError> {
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
        self.revision = self
            .revision
            .checked_add(1)
            .ok_or(WorldError::InvalidDomainTransition)?;
        self.bump_view_revisions_for_events(&events)?;
        self.tick_resolution_pending = false;
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
                let mut released_npcs = Vec::new();
                for npc in self.npcs.values_mut() {
                    if npc.engaged_character == Some(character_id) {
                        npc.engaged_character = None;
                        released_npcs.push(npc.stable_id.clone());
                    }
                }
                for npc_id in released_npcs {
                    self.loot_encounters.remove(&npc_id);
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
                self.loot_encounters.remove(&npc.stable_id);
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
                self.loot_encounters.remove(&npc_id);
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
                        self.item_location(&item_id),
                        Some(SharedItemLocation::HeldByNpc(owner)) if owner == &npc_id
                    ) {
                        self.set_item_location(
                            &item_id,
                            SharedItemLocation::Room(room_id.clone()),
                        )?;
                        let item_name = self.item_effective_name(&item_id)?;
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
            self.loot_encounters.remove(npc_id);
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
            self.loot_encounters.remove(npc_id);
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
                .filter_map(|item_id| self.item_effective_attack_bonus(item_id).ok())
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
            self.loot_encounters.remove(npc_id);
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

    fn prune_consumed_runtime_items(&mut self) -> Result<(), WorldError> {
        let reserved = self.reserved_item_ids()?;
        let parents = self
            .item_instances
            .values()
            .filter_map(|instance| match &instance.location {
                SharedItemLocation::Contained(parent) => Some(parent.clone()),
                _ => None,
            })
            .collect::<BTreeSet<_>>();
        let removable = self
            .item_instances
            .iter()
            .filter_map(|(item_id, instance)| {
                (runtime_item_sequence(item_id).is_some()
                    && matches!(instance.location, SharedItemLocation::Consumed)
                    && !self.initial_item_prototypes.contains_key(item_id)
                    && !self.ground_loot.contains_key(item_id)
                    && !reserved.contains(item_id)
                    && !parents.contains(item_id))
                .then_some(item_id.clone())
            })
            .collect::<Vec<_>>();
        for item_id in removable {
            self.item_instances.remove(&item_id);
            self.item_provenance.remove(&item_id);
        }
        Ok(())
    }

    fn roll_loot_table(
        &mut self,
        table_id: &str,
    ) -> Result<(LootTable, Vec<RolledLootUnit>), WorldError> {
        let table = self
            .loot_tables
            .get(table_id)
            .cloned()
            .ok_or(WorldError::InvalidGameplayDefinition)?;
        let mut units = Vec::with_capacity(table.worst_case_units);
        for guaranteed in &table.guaranteed {
            for quantity_index in 0..guaranteed.quantity {
                units.push(RolledLootUnit {
                    prototype_id: guaranteed.item_prototype_id.clone(),
                    entry_id: guaranteed.id.clone(),
                    pool_id: None,
                    roll_index: 0,
                    quantity_index,
                    affix_pool_id: guaranteed.affix_pool_id.clone(),
                });
            }
        }
        for pool in &table.pools {
            let total_weight = pool.entries.iter().try_fold(0_u64, |total, entry| {
                total
                    .checked_add(u64::from(entry.weight))
                    .ok_or(WorldError::InvalidGameplayDefinition)
            })?;
            for roll_index in 0..pool.rolls {
                let selected = self.rng.uniform_below(total_weight)?;
                let mut cumulative = 0_u64;
                let entry = pool
                    .entries
                    .iter()
                    .find(|entry| {
                        cumulative = cumulative.saturating_add(u64::from(entry.weight));
                        selected < cumulative
                    })
                    .ok_or(WorldError::InvalidGameplayDefinition)?;
                if entry.kind == LootEntryKind::Nothing {
                    continue;
                }
                let minimum = entry
                    .min_quantity
                    .ok_or(WorldError::InvalidGameplayDefinition)?;
                let maximum = entry
                    .max_quantity
                    .ok_or(WorldError::InvalidGameplayDefinition)?;
                let width = u64::from(maximum - minimum) + 1;
                let quantity = minimum
                    + u16::try_from(self.rng.uniform_below(width)?)
                        .map_err(|_| WorldError::InvalidGameplayDefinition)?;
                for quantity_index in 0..quantity {
                    units.push(RolledLootUnit {
                        prototype_id: entry
                            .item_prototype_id
                            .clone()
                            .ok_or(WorldError::InvalidGameplayDefinition)?,
                        entry_id: entry.id.clone(),
                        pool_id: Some(pool.id.clone()),
                        roll_index,
                        quantity_index,
                        affix_pool_id: entry.affix_pool_id.clone(),
                    });
                }
            }
        }
        if units.len() > MAX_LOOT_UNITS_PER_RESOLUTION {
            return Err(WorldError::InvalidGameplayDefinition);
        }
        Ok((table, units))
    }

    fn final_loot_cohort(&self, encounter: &LootEncounter, room_id: &str) -> Vec<CharacterId> {
        let mut cohort = vec![encounter.initiator];
        if let Some(party_id) = encounter.party_id
            && let Some(party) = self.adventure.parties.get(&party_id)
            && party.members.contains(&encounter.initiator)
        {
            for member in encounter
                .cohort
                .iter()
                .copied()
                .filter(|member| *member != encounter.initiator)
            {
                let eligible = party.members.contains(&member)
                    && self
                        .session_for_character(member)
                        .and_then(|session| self.character(session).ok())
                        .and_then(|character| {
                            room_checked(&self.entities, character.location)
                                .ok()
                                .map(|room| room.stable_id == room_id)
                        })
                        .unwrap_or(false)
                    && self.character_gameplay.get(&member).is_some_and(|state| {
                        state.vitals.health > 0 && state.recovery_at_tick.is_none()
                    });
                if eligible {
                    cohort.push(member);
                }
            }
        }
        cohort.sort_unstable();
        cohort.dedup();
        cohort
    }

    fn reserve_ground_loot(
        &mut self,
        drop_id: LootDropId,
        roots: &[ItemInstanceId],
        encounter: &LootEncounter,
        rights: LootRightsPolicy,
        room_id: &str,
        tick: u64,
    ) -> Result<(), WorldError> {
        let cohort = self.final_loot_cohort(encounter, room_id);
        let round_robin_party = encounter.party_id.filter(|party_id| {
            rights == LootRightsPolicy::PartyRoundRobin
                && cohort.len() > 1
                && self.adventure.parties.contains_key(party_id)
        });
        let mut cursor = round_robin_party
            .and_then(|party_id| self.party_loot_cursors.get(&party_id).copied())
            .unwrap_or(0);
        for root_item_id in roots {
            if self.ground_loot.len() >= MAX_GROUND_LOOT_RESERVATIONS {
                return Err(WorldError::InvalidDomainTransition);
            }
            let beneficiary = if round_robin_party.is_some() {
                let index = usize::try_from(cursor % u64::try_from(cohort.len()).unwrap_or(1))
                    .map_err(|_| WorldError::InvalidDomainTransition)?;
                cursor = cursor
                    .checked_add(1)
                    .ok_or(WorldError::InvalidDomainTransition)?;
                Some(cohort[index])
            } else {
                Some(encounter.initiator)
            };
            self.ground_loot.insert(
                root_item_id.clone(),
                GroundLootReservation {
                    drop_id,
                    root_item_id: root_item_id.clone(),
                    room_id: room_id.to_owned(),
                    cohort: cohort.clone(),
                    beneficiary,
                    passed: BTreeSet::new(),
                    created_tick: tick,
                    claim_until_tick: tick
                        .checked_add(LOOT_EXCLUSIVE_TICKS)
                        .ok_or(WorldError::InvalidDomainTransition)?,
                    despawn_at_tick: tick
                        .checked_add(LOOT_DESPAWN_TICKS)
                        .ok_or(WorldError::InvalidDomainTransition)?,
                },
            );
        }
        if let Some(party_id) = round_robin_party
            && cursor > 0
        {
            self.party_loot_cursors.insert(party_id, cursor);
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
        let encounter = self
            .loot_encounters
            .remove(npc_id)
            .unwrap_or_else(|| LootEncounter {
                npc_spawn_id: npc_id.to_owned(),
                initiator: character_id,
                party_id: None,
                cohort: vec![character_id],
                room_id: room_id.to_owned(),
                started_tick: tick,
            });
        let mut loot = loot;
        loot.sort();
        let fixed_candidates = loot
            .into_iter()
            .filter(|item_id| {
                self.item_location(item_id)
                    == Some(&SharedItemLocation::HeldByNpc(npc_id.to_owned()))
            })
            .collect::<Vec<_>>();
        let source = self.loot_sources.get(npc_id).cloned();
        if source.is_some() || !fixed_candidates.is_empty() {
            self.prune_consumed_runtime_items()?;
            let drop_id = self.next_loot_drop_id;
            self.next_loot_drop_id = LootDropId(
                drop_id
                    .0
                    .checked_add(1)
                    .ok_or(WorldError::InvalidDomainTransition)?,
            );
            // Roll before any capacity decision. A rejected drop therefore
            // advances the deterministic RNG and durable drop identity exactly
            // as an admitted drop would.
            let rolled = source
                .as_ref()
                .map(|source| self.roll_loot_table(&source.loot_table_id))
                .transpose()?;
            let rolled = rolled
                .map(|(table, units)| {
                    let mut prepared = Vec::with_capacity(units.len());
                    for unit in units {
                        let item_affixes = unit
                            .affix_pool_id
                            .as_deref()
                            .map(|pool_id| self.roll_item_affix(pool_id))
                            .transpose()?
                            .into_iter()
                            .flatten()
                            .collect::<Vec<_>>();
                        prepared.push((unit, item_affixes));
                    }
                    Ok::<_, WorldError>((table, prepared))
                })
                .transpose()?;
            let rights = source
                .as_ref()
                .map_or(LootRightsPolicy::CreditedCharacter, |source| source.rights);
            let mut fixed_suppressed = 0_usize;
            for item_id in fixed_candidates {
                let checkpoint = self.checkpoint();
                let admitted = if self.ground_loot.len() < MAX_GROUND_LOOT_RESERVATIONS {
                    self.set_item_location(&item_id, SharedItemLocation::Room(room_id.to_owned()))?;
                    self.reserve_ground_loot(
                        drop_id,
                        std::slice::from_ref(&item_id),
                        &encounter,
                        rights,
                        room_id,
                        tick,
                    )?;
                    let fits = serde_json::to_vec(&self.runtime_snapshot())
                        .map_err(|_| WorldError::InvalidDomainTransition)?
                        .len()
                        <= MAX_WORLD_RUNTIME_SNAPSHOT_BYTES;
                    if !fits {
                        self.restore_checkpoint(checkpoint);
                    }
                    fits
                } else {
                    false
                };
                if admitted {
                    let item_name = self.item_effective_name(&item_id)?;
                    events.push(DomainEvent::Gameplay {
                        event: GameplayEvent::LootDropped {
                            npc_id: npc_id.to_owned(),
                            npc_name: npc_name.clone(),
                            item_id,
                            item_name,
                            room_id: room_id.to_owned(),
                        },
                    });
                } else {
                    self.set_item_location(&item_id, SharedItemLocation::Staged)?;
                    fixed_suppressed = fixed_suppressed.saturating_add(1);
                }
            }
            if fixed_suppressed > 0 {
                events.push(DomainEvent::Gameplay {
                    event: GameplayEvent::LootGenerationSuppressed {
                        drop_id,
                        npc_id: npc_id.to_owned(),
                        npc_name: npc_name.clone(),
                        reason: format!(
                            "capacity: {fixed_suppressed} authored fixed root(s) tombstoned"
                        ),
                        room_id: room_id.to_owned(),
                    },
                });
            }
            if let Some((table, units)) = rolled {
                let capacity_suppressed = !units.is_empty()
                    && (self
                        .item_instances
                        .len()
                        .checked_add(table.worst_case_units)
                        .is_none_or(|total| total > MAX_ITEM_INSTANCES)
                        || self
                            .ground_loot
                            .len()
                            .checked_add(units.len())
                            .is_none_or(|total| total > MAX_GROUND_LOOT_RESERVATIONS));
                if capacity_suppressed {
                    events.push(DomainEvent::Gameplay {
                        event: GameplayEvent::LootGenerationSuppressed {
                            drop_id,
                            npc_id: npc_id.to_owned(),
                            npc_name: npc_name.clone(),
                            reason: "capacity".into(),
                            room_id: room_id.to_owned(),
                        },
                    });
                } else if !units.is_empty() {
                    // This checkpoint is intentionally after RNG/drop-id and
                    // after all admitted fixed roots. Restoring it suppresses
                    // the complete random batch without rewinding either.
                    let checkpoint = self.checkpoint();
                    let mut generated_events = Vec::new();
                    let mut generated_roots = Vec::with_capacity(units.len());
                    for (unit, item_affixes) in units {
                        let sealed_loot_table_id = self
                            .item_prototypes
                            .get(&unit.prototype_id)
                            .and_then(|prototype| prototype.sealed_loot_table_id.clone());
                        let item_id = self.create_staged_item_instance(&unit.prototype_id)?;
                        self.item_instances
                            .get_mut(&item_id)
                            .ok_or(WorldError::InvalidDomainTransition)?
                            .item_affixes = item_affixes;
                        self.set_item_location(
                            &item_id,
                            SharedItemLocation::Room(room_id.to_owned()),
                        )?;
                        self.item_provenance.insert(
                            item_id.clone(),
                            ItemProvenance::NpcLoot {
                                drop_id,
                                npc_spawn_id: npc_id.to_owned(),
                                loot_table_id: table.id.clone(),
                                table_digest_sha256: table.digest_sha256.clone(),
                                entry_id: unit.entry_id,
                                pool_id: unit.pool_id,
                                sealed_loot_table_id,
                                roll_index: unit.roll_index,
                                quantity_index: unit.quantity_index,
                                generated_tick: tick,
                            },
                        );
                        let item_name = self.item_effective_name(&item_id)?;
                        generated_roots.push(item_id.clone());
                        generated_events.push(DomainEvent::Gameplay {
                            event: GameplayEvent::LootDropped {
                                npc_id: npc_id.to_owned(),
                                npc_name: npc_name.clone(),
                                item_id,
                                item_name,
                                room_id: room_id.to_owned(),
                            },
                        });
                    }
                    self.reserve_ground_loot(
                        drop_id,
                        &generated_roots,
                        &encounter,
                        rights,
                        room_id,
                        tick,
                    )?;
                    let fits = serde_json::to_vec(&self.runtime_snapshot())
                        .map_err(|_| WorldError::InvalidDomainTransition)?
                        .len()
                        <= MAX_WORLD_RUNTIME_SNAPSHOT_BYTES;
                    if fits {
                        generated_events.push(DomainEvent::Gameplay {
                            event: GameplayEvent::LootGenerated {
                                drop_id,
                                npc_id: npc_id.to_owned(),
                                npc_name: npc_name.clone(),
                                item_ids: generated_roots,
                                room_id: room_id.to_owned(),
                            },
                        });
                        events.extend(generated_events);
                    } else {
                        self.restore_checkpoint(checkpoint);
                        events.push(DomainEvent::Gameplay {
                            event: GameplayEvent::LootGenerationSuppressed {
                                drop_id,
                                npc_id: npc_id.to_owned(),
                                npc_name: npc_name.clone(),
                                reason: "snapshot_bytes".into(),
                                room_id: room_id.to_owned(),
                            },
                        });
                    }
                }
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
            DomainEvent::Gameplay {
                event:
                    GameplayEvent::LootGenerated { .. }
                    | GameplayEvent::LootRightsChanged { .. }
                    | GameplayEvent::LootGenerationSuppressed { .. }
                    | GameplayEvent::LootDespawned { .. }
                    | GameplayEvent::LootEncounterCancelled { .. }
                    | GameplayEvent::RewardOpened { .. },
            } => Err(WorldError::InvalidDomainTransition),
            DomainEvent::CharacterMoved { .. }
            | DomainEvent::CharacterRelocated { .. }
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
                if self.item_location(item_id) != Some(&SharedItemLocation::Room(room_id.clone())) {
                    return Err(WorldError::InvalidDomainTransition);
                }
                let mut prospective = self.item_instances.clone();
                prospective
                    .get_mut(item_id)
                    .ok_or(WorldError::InvalidDomainTransition)?
                    .location = SharedItemLocation::Carried(*actor_id);
                self.apply_item_ownership_transitions(
                    &mut prospective,
                    std::slice::from_ref(item_id),
                )?;
                if self.character_gameplay.contains_key(actor_id) {
                    self.validate_prospective_item_locations(&prospective, [*actor_id])?;
                } else {
                    validate_item_graph(&prospective, &self.item_prototypes)?;
                }
                self.item_instances = prospective;
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
                if self.item_location(item_id) != Some(&SharedItemLocation::Carried(*actor_id)) {
                    return Err(WorldError::InvalidDomainTransition);
                }
                let mut prospective = self.item_instances.clone();
                prospective
                    .get_mut(item_id)
                    .ok_or(WorldError::InvalidDomainTransition)?
                    .location = SharedItemLocation::Room(room_id.clone());
                self.apply_item_ownership_transitions(
                    &mut prospective,
                    std::slice::from_ref(item_id),
                )?;
                if self.character_gameplay.contains_key(actor_id) {
                    self.validate_prospective_item_locations(&prospective, [*actor_id])?;
                } else {
                    validate_item_graph(&prospective, &self.item_prototypes)?;
                }
                self.item_instances = prospective;
                self.revision = self.revision.wrapping_add(1).max(1);
                Ok(())
            }
            DomainEvent::ItemsMoved {
                actor_id,
                item_ids,
                container_id,
                operation,
                room_id,
                ..
            } => {
                self.validate_items_moved_event(event)?;
                let mut prospective = self.item_instances.clone();
                for item_id in item_ids {
                    let expected = match operation {
                        ContainerMoveOperation::Take => SharedItemLocation::Room(room_id.clone()),
                        ContainerMoveOperation::Drop | ContainerMoveOperation::Put => {
                            SharedItemLocation::Carried(*actor_id)
                        }
                        ContainerMoveOperation::Get => SharedItemLocation::Contained(
                            container_id
                                .clone()
                                .ok_or(WorldError::InvalidDomainTransition)?,
                        ),
                    };
                    let destination = match operation {
                        ContainerMoveOperation::Take | ContainerMoveOperation::Get => {
                            SharedItemLocation::Carried(*actor_id)
                        }
                        ContainerMoveOperation::Drop => SharedItemLocation::Room(room_id.clone()),
                        ContainerMoveOperation::Put => SharedItemLocation::Contained(
                            container_id
                                .clone()
                                .ok_or(WorldError::InvalidDomainTransition)?,
                        ),
                    };
                    let instance = prospective
                        .get_mut(item_id)
                        .ok_or(WorldError::InvalidDomainTransition)?;
                    if instance.location != expected {
                        return Err(WorldError::InvalidDomainTransition);
                    }
                    instance.location = destination;
                }
                self.apply_item_ownership_transitions(&mut prospective, item_ids)?;
                // A cold event replay deliberately supports an offline owner:
                // the durable character aggregate is loaded and checked by
                // the startup ownership barrier. When the aggregate is
                // already present, retain the stronger immediate capacity
                // check as well.
                if self.character_gameplay.contains_key(actor_id) {
                    self.validate_prospective_item_locations(&prospective, [*actor_id])?;
                } else {
                    validate_item_graph(&prospective, &self.item_prototypes)?;
                }
                self.item_instances = prospective;
                self.revision = self.revision.wrapping_add(1).max(1);
                Ok(())
            }
            DomainEvent::ItemsGiven {
                giver_id,
                recipient_id,
                item_ids,
                room_id: _,
                ..
            } => {
                self.validate_items_given_event(event)?;
                let mut prospective = self.item_instances.clone();
                for item_id in item_ids {
                    let instance = prospective
                        .get_mut(item_id)
                        .ok_or(WorldError::InvalidDomainTransition)?;
                    if instance.location != SharedItemLocation::Carried(*giver_id) {
                        return Err(WorldError::InvalidDomainTransition);
                    }
                    instance.location = SharedItemLocation::Carried(*recipient_id);
                }
                self.apply_item_ownership_transitions(&mut prospective, item_ids)?;
                let affected = [*giver_id, *recipient_id]
                    .into_iter()
                    .filter(|id| self.character_gameplay.contains_key(id))
                    .collect::<Vec<_>>();
                if affected.is_empty() {
                    validate_item_graph(&prospective, &self.item_prototypes)?;
                } else {
                    self.validate_prospective_item_locations(&prospective, affected)?;
                }
                self.item_instances = prospective;
                self.revision = self.revision.wrapping_add(1).max(1);
                Ok(())
            }
            DomainEvent::TradeRequested { trade } => {
                if trade.id != self.next_trade_id
                    || self.trades.contains_key(&trade.id)
                    || self.trade_for_character(trade.requester).is_some()
                    || self.trade_for_character(trade.recipient).is_some()
                {
                    return Err(WorldError::InvalidDomainTransition);
                }
                let next = TradeId(
                    trade
                        .id
                        .0
                        .checked_add(1)
                        .ok_or(WorldError::InvalidDomainTransition)?,
                );
                let mut candidate_trades = self.trades.clone();
                candidate_trades.insert(trade.id, trade.clone());
                validate_trade_state(
                    next,
                    &candidate_trades,
                    &self.item_instances,
                    &self.item_prototypes,
                    &self.stable_rooms,
                )?;
                self.next_trade_id = next;
                self.trades.insert(trade.id, trade.clone());
                self.revision = self
                    .revision
                    .checked_add(1)
                    .ok_or(WorldError::InvalidDomainTransition)?;
                Ok(())
            }
            DomainEvent::TradeOpened { previous, current } => {
                if self.trades.get(&previous.id) != Some(previous)
                    || !valid_trade_open_transition(previous, current)
                {
                    return Err(WorldError::InvalidDomainTransition);
                }
                self.replay_trade_state_change(current)
            }
            DomainEvent::TradeOfferSet { previous, current } => {
                if self.trades.get(&previous.id) != Some(previous)
                    || !valid_trade_offer_transition(previous, current)
                {
                    return Err(WorldError::InvalidDomainTransition);
                }
                self.replay_trade_state_change(current)
            }
            DomainEvent::TradeInvalidated { previous, current } => {
                if self.trades.get(&previous.id) != Some(previous)
                    || !valid_trade_invalidation_transition(previous, current)
                {
                    return Err(WorldError::InvalidDomainTransition);
                }
                self.replay_trade_state_change(current)
            }
            DomainEvent::TradeConfirmed {
                participant,
                confirmation,
            } => {
                let trade = self
                    .trade_for_character(*participant)
                    .cloned()
                    .ok_or(WorldError::InvalidDomainTransition)?;
                if trade.phase != TradePhase::Negotiating
                    || !trade_has_assets(&trade)
                    || confirmation.state_revision != trade.state_revision
                    || trade.confirmations.contains_key(participant)
                    || (!trade.confirmations.is_empty())
                    || self
                        .trade_basis_sha256_if_participants_loaded(&trade)?
                        .is_some_and(|basis| basis != confirmation.basis_sha256)
                {
                    return Err(WorldError::InvalidDomainTransition);
                }
                let mut current = trade;
                current
                    .confirmations
                    .insert(*participant, confirmation.clone());
                self.replay_trade_state_change(&current)
            }
            DomainEvent::TradeCancelled { previous_trade, .. } => {
                if self.trades.get(&previous_trade.id) != Some(previous_trade) {
                    return Err(WorldError::InvalidDomainTransition);
                }
                self.trades.remove(&previous_trade.id);
                self.revision = self
                    .revision
                    .checked_add(1)
                    .ok_or(WorldError::InvalidDomainTransition)?;
                Ok(())
            }
            DomainEvent::TradeCompleted { previous_trade } => {
                let current = self
                    .trades
                    .get(&previous_trade.id)
                    .cloned()
                    .ok_or(WorldError::InvalidDomainTransition)?;
                let participants =
                    BTreeSet::from([previous_trade.requester, previous_trade.recipient]);
                let canonical_basis = self.trade_basis_sha256_if_participants_loaded(&current)?;
                let confirmations_valid = previous_trade.confirmations.len() == 2
                    && previous_trade
                        .confirmations
                        .keys()
                        .copied()
                        .collect::<BTreeSet<_>>()
                        == participants
                    && previous_trade.confirmations.values().all(|confirmation| {
                        confirmation.state_revision == previous_trade.state_revision
                            && canonical_basis
                                .is_none_or(|basis| confirmation.basis_sha256 == basis)
                    })
                    && previous_trade
                        .confirmations
                        .values()
                        .map(|confirmation| confirmation.basis_sha256)
                        .collect::<BTreeSet<_>>()
                        .len()
                        == 1;
                let mut expected_current = previous_trade.clone();
                expected_current.confirmations = current.confirmations.clone();
                if current.phase != TradePhase::Negotiating
                    || !trade_has_assets(previous_trade)
                    || current.confirmations.len() != 1
                    || current != expected_current
                    || !confirmations_valid
                    || current
                        .confirmations
                        .iter()
                        .any(|(participant, confirmation)| {
                            previous_trade.confirmations.get(participant) != Some(confirmation)
                        })
                {
                    return Err(WorldError::InvalidDomainTransition);
                }
                let mut completed_candidate = self.trades.clone();
                completed_candidate.insert(previous_trade.id, previous_trade.clone());
                validate_trade_state(
                    self.next_trade_id,
                    &completed_candidate,
                    &self.item_instances,
                    &self.item_prototypes,
                    &self.stable_rooms,
                )?;
                if let (Some(requester), Some(recipient)) = (
                    self.character_gameplay.get(&previous_trade.requester),
                    self.character_gameplay.get(&previous_trade.recipient),
                ) {
                    let requester_offer = previous_trade
                        .offers
                        .get(&previous_trade.requester)
                        .ok_or(WorldError::InvalidDomainTransition)?;
                    let recipient_offer = previous_trade
                        .offers
                        .get(&previous_trade.recipient)
                        .ok_or(WorldError::InvalidDomainTransition)?;
                    requester
                        .coins
                        .checked_sub(requester_offer.coins)
                        .and_then(|coins| coins.checked_add(recipient_offer.coins))
                        .ok_or(WorldError::InvalidDomainTransition)?;
                    recipient
                        .coins
                        .checked_sub(recipient_offer.coins)
                        .and_then(|coins| coins.checked_add(requester_offer.coins))
                        .ok_or(WorldError::InvalidDomainTransition)?;
                }
                let mut prospective = self.item_instances.clone();
                for (owner, destination) in [
                    (previous_trade.requester, previous_trade.recipient),
                    (previous_trade.recipient, previous_trade.requester),
                ] {
                    for root in previous_trade
                        .offers
                        .get(&owner)
                        .into_iter()
                        .flat_map(|offer| &offer.lines)
                        .flat_map(|line| &line.item_ids)
                    {
                        let instance = prospective
                            .get_mut(root)
                            .ok_or(WorldError::InvalidDomainTransition)?;
                        if instance.location != SharedItemLocation::Carried(owner) {
                            return Err(WorldError::InvalidDomainTransition);
                        }
                        instance.location = SharedItemLocation::Carried(destination);
                    }
                }
                let affected = [previous_trade.requester, previous_trade.recipient]
                    .into_iter()
                    .filter(|id| self.character_gameplay.contains_key(id))
                    .collect::<Vec<_>>();
                if affected.is_empty() {
                    validate_item_graph(&prospective, &self.item_prototypes)?;
                } else {
                    self.validate_prospective_item_locations(&prospective, affected)?;
                }
                self.item_instances = prospective;
                self.trades.remove(&previous_trade.id);
                self.revision = self
                    .revision
                    .checked_add(1)
                    .ok_or(WorldError::InvalidDomainTransition)?;
                Ok(())
            }
        }
    }

    fn replay_trade_state_change(&mut self, current: &TradeSnapshot) -> Result<(), WorldError> {
        let mut candidate_trades = self.trades.clone();
        candidate_trades.insert(current.id, current.clone());
        validate_trade_state(
            self.next_trade_id,
            &candidate_trades,
            &self.item_instances,
            &self.item_prototypes,
            &self.stable_rooms,
        )?;
        self.trades.insert(current.id, current.clone());
        self.revision = self
            .revision
            .checked_add(1)
            .ok_or(WorldError::InvalidDomainTransition)?;
        Ok(())
    }

    fn trade_basis_sha256_if_participants_loaded(
        &self,
        trade: &TradeSnapshot,
    ) -> Result<Option<[u8; 32]>, WorldError> {
        if trade
            .participants()
            .into_iter()
            .all(|participant| self.character_gameplay.contains_key(&participant))
        {
            self.trade_basis_sha256(trade).map(Some)
        } else {
            Ok(None)
        }
    }

    /// Reverses uncommitted shared-item events. Validation runs against a copy
    /// first, so a malformed batch cannot partially mutate the live world.
    pub fn rollback_domain_events(&mut self, events: &[DomainEvent]) -> Result<(), WorldError> {
        let mut restored = self.item_instances.clone();
        let mut item_event_count = 0_u64;
        let mut affected_characters = BTreeSet::new();
        for event in events.iter().rev() {
            match event {
                DomainEvent::CharacterMoved { .. }
                | DomainEvent::CharacterRelocated { .. }
                | DomainEvent::Gameplay { .. }
                | DomainEvent::Adventure { .. }
                | DomainEvent::TradeRequested { .. }
                | DomainEvent::TradeOpened { .. }
                | DomainEvent::TradeOfferSet { .. }
                | DomainEvent::TradeConfirmed { .. }
                | DomainEvent::TradeInvalidated { .. }
                | DomainEvent::TradeCancelled { .. }
                | DomainEvent::TradeCompleted { .. } => {}
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
                    if restored.get(item_id).map(|item| &item.location)
                        != Some(&SharedItemLocation::Carried(*actor_id))
                    {
                        return Err(WorldError::InvalidDomainTransition);
                    }
                    restored
                        .get_mut(item_id)
                        .expect("validated item exists")
                        .location = SharedItemLocation::Room(room_id.clone());
                    self.reconcile_rollback_bindings(&mut restored, std::slice::from_ref(item_id))?;
                    affected_characters.insert(*actor_id);
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
                    if restored.get(item_id).map(|item| &item.location)
                        != Some(&SharedItemLocation::Room(room_id.clone()))
                    {
                        return Err(WorldError::InvalidDomainTransition);
                    }
                    restored
                        .get_mut(item_id)
                        .expect("validated item exists")
                        .location = SharedItemLocation::Carried(*actor_id);
                    self.reconcile_rollback_bindings(&mut restored, std::slice::from_ref(item_id))?;
                    affected_characters.insert(*actor_id);
                    item_event_count += 1;
                }
                DomainEvent::ItemsMoved {
                    actor_id,
                    item_ids,
                    container_id,
                    operation,
                    room_id,
                    ..
                } => {
                    self.validate_items_moved_event(event)?;
                    for item_id in item_ids {
                        let expected = match operation {
                            ContainerMoveOperation::Take | ContainerMoveOperation::Get => {
                                SharedItemLocation::Carried(*actor_id)
                            }
                            ContainerMoveOperation::Drop => {
                                SharedItemLocation::Room(room_id.clone())
                            }
                            ContainerMoveOperation::Put => SharedItemLocation::Contained(
                                container_id
                                    .clone()
                                    .ok_or(WorldError::InvalidDomainTransition)?,
                            ),
                        };
                        let destination = match operation {
                            ContainerMoveOperation::Take => {
                                SharedItemLocation::Room(room_id.clone())
                            }
                            ContainerMoveOperation::Drop | ContainerMoveOperation::Put => {
                                SharedItemLocation::Carried(*actor_id)
                            }
                            ContainerMoveOperation::Get => SharedItemLocation::Contained(
                                container_id
                                    .clone()
                                    .ok_or(WorldError::InvalidDomainTransition)?,
                            ),
                        };
                        let instance = restored
                            .get_mut(item_id)
                            .ok_or(WorldError::InvalidDomainTransition)?;
                        if instance.location != expected {
                            return Err(WorldError::InvalidDomainTransition);
                        }
                        instance.location = destination;
                    }
                    self.reconcile_rollback_bindings(&mut restored, item_ids)?;
                    affected_characters.insert(*actor_id);
                    item_event_count += 1;
                }
                DomainEvent::ItemsGiven {
                    giver_id,
                    recipient_id,
                    item_ids,
                    ..
                } => {
                    self.validate_items_given_event(event)?;
                    for item_id in item_ids {
                        let instance = restored
                            .get_mut(item_id)
                            .ok_or(WorldError::InvalidDomainTransition)?;
                        if instance.location != SharedItemLocation::Carried(*recipient_id) {
                            return Err(WorldError::InvalidDomainTransition);
                        }
                        instance.location = SharedItemLocation::Carried(*giver_id);
                    }
                    self.reconcile_rollback_bindings(&mut restored, item_ids)?;
                    affected_characters.insert(*giver_id);
                    affected_characters.insert(*recipient_id);
                    item_event_count += 1;
                }
            }
        }
        if !affected_characters.is_empty() {
            let loaded = affected_characters
                .iter()
                .copied()
                .filter(|id| self.character_gameplay.contains_key(id))
                .collect::<Vec<_>>();
            if loaded.is_empty() {
                validate_item_graph(&restored, &self.item_prototypes)?;
            } else {
                self.validate_prospective_item_locations(&restored, loaded)?;
            }
        }
        self.item_instances = restored;
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
        let mut salt = [0_u8; 32];
        OsRng
            .try_fill_bytes(&mut salt)
            .map_err(|_| WorldError::InvalidEntity)?;
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
                self.last_trade_request_ticks.remove(&character_id);
                // Combat is connection-scoped in v1: an NPC immediately stops
                // attacking a departed character. Remove that character's
                // pending effects as well, so damage-over-time cannot award
                // progress only in memory after the durable character is no
                // longer part of the tick commit.
                let mut abandoned_encounters = Vec::new();
                for npc in self.npcs.values_mut() {
                    if npc.engaged_character == Some(character_id) {
                        npc.engaged_character = None;
                        abandoned_encounters.push(npc.stable_id.clone());
                    }
                    npc.statuses
                        .retain(|status| status.source_character_id != Some(character_id));
                }
                for npc_id in abandoned_encounters {
                    self.loot_encounters.remove(&npc_id);
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

    /// Builds a bounded, staff-only diagnostic view. Callers must authorize
    /// `staff.inspect` and audit the access before exposing this text.
    pub fn staff_inspection_as_plain_text(&self, session: SessionId) -> Result<String, WorldError> {
        let character = self.character(session)?;
        let room = room_checked(&self.entities, character.location)?;
        let gameplay = self
            .character_gameplay
            .get(&character.id)
            .ok_or(WorldError::InvalidEntity)?;
        let carried = self
            .item_instances
            .values()
            .filter(|instance| {
                matches!(&instance.location, SharedItemLocation::Carried(owner) if *owner == character.id)
            })
            .count();
        Ok(format!(
            concat!(
                "Character: {}\r\n",
                "Room: {}\r\n",
                "Progression: level {}, XP {}, class {}\r\n",
                "Vitals: {}/{} health; {} resources; {} statuses\r\n",
                "Inventory: {} carried; {} equipped\r\n"
            ),
            character.name,
            room.title,
            gameplay.level,
            gameplay.experience,
            gameplay.active_class_id,
            gameplay.vitals.health,
            gameplay.vitals.max_health,
            gameplay.resources.len(),
            gameplay.statuses.len(),
            carried,
            gameplay.equipment.len(),
        ))
    }

    /// Returns stable operational location data for an already authorized
    /// staff diagnostic. Stable room identity never enters ordinary views.
    pub fn staff_location_as_plain_text(&self, session: SessionId) -> Result<String, WorldError> {
        let character = self.character(session)?;
        let room = room_checked(&self.entities, character.location)?;
        Ok(format!("{} is in {}.\r\n", character.name, room.title))
    }

    /// Relocates the acting staff character to a connected target.
    pub fn staff_goto(
        &mut self,
        actor: SessionId,
        target: SessionId,
    ) -> Result<ActionOutcome, WorldError> {
        self.staff_relocate(actor, target, actor, "You travel to the target.\r\n")
    }

    /// Brings a connected target to the acting staff character.
    pub fn staff_transfer(
        &mut self,
        actor: SessionId,
        target: SessionId,
    ) -> Result<ActionOutcome, WorldError> {
        self.staff_relocate(
            target,
            actor,
            actor,
            "The target is transferred to you.\r\n",
        )
    }

    fn staff_relocate(
        &mut self,
        moving: SessionId,
        destination: SessionId,
        response_session: SessionId,
        output: &str,
    ) -> Result<ActionOutcome, WorldError> {
        let moving_entity = *self
            .sessions
            .get(&moving)
            .ok_or(WorldError::UnknownSession)?;
        let destination_entity = *self
            .sessions
            .get(&destination)
            .ok_or(WorldError::UnknownSession)?;
        let destination_location = self
            .entities
            .get(destination_entity)
            .and_then(|entity| match entity {
                Entity::Character(character) => Some(character.location),
                _ => None,
            })
            .ok_or(WorldError::InvalidEntity)?;
        let to_room_id = room_checked(&self.entities, destination_location)?
            .stable_id
            .clone();
        let (actor_id, actor_name, from_room_id) = self
            .entities
            .get(moving_entity)
            .and_then(|entity| match entity {
                Entity::Character(character) => {
                    Some((character.id, character.name.clone(), character.location))
                }
                _ => None,
            })
            .map(|(id, name, location)| {
                room_checked(&self.entities, location)
                    .map(|room| (id, name, room.stable_id.clone()))
            })
            .transpose()?
            .ok_or(WorldError::InvalidEntity)?;
        if from_room_id == to_room_id {
            return Err(WorldError::GameplayUnavailable);
        }
        match self.entities.get_mut(moving_entity) {
            Some(Entity::Character(character)) => character.location = destination_location,
            _ => return Err(WorldError::InvalidEntity),
        }
        self.revision = self.revision.wrapping_add(1).max(1);
        let event = DomainEvent::CharacterRelocated {
            actor_id,
            actor_name,
            from_room_id,
            to_room_id: to_room_id.clone(),
        };
        let mut events = vec![event];
        let mut released_npcs = Vec::new();
        for npc in self.npcs.values_mut() {
            if npc.engaged_character == Some(actor_id) {
                npc.engaged_character = None;
                released_npcs.push((
                    npc.stable_id.clone(),
                    npc.name.clone(),
                    npc.spawn_room_id.clone(),
                ));
            }
        }
        for (npc_id, npc_name, room_id) in released_npcs {
            self.loot_encounters.remove(&npc_id);
            events.push(DomainEvent::Gameplay {
                event: GameplayEvent::LootEncounterCancelled {
                    npc_id,
                    npc_name,
                    room_id,
                },
            });
        }
        self.bump_view_revisions_for_events(&events)?;
        self.observer_views
            .get_mut(&moving)
            .ok_or(WorldError::InvalidEntity)?
            .discovered_rooms
            .insert(to_room_id);
        Ok(ActionOutcome {
            scene: self.scene_for(response_session)?,
            container_contents: None,
            trade_contents: None,
            reward_odds: None,
            output: Some(output.to_owned()),
            changed_character: Some(self.character_snapshot(moving)?),
            domain_events: events,
        })
    }

    /// Fully restores a connected character's bounded gameplay resources.
    /// The caller is responsible for capability/hierarchy checks and durable
    /// audit in the same transaction as the resulting snapshots.
    pub fn staff_restore(
        &mut self,
        actor: SessionId,
        target: SessionId,
    ) -> Result<ActionOutcome, WorldError> {
        let target_character = self.character(target)?;
        let character_id = target_character.id;
        let character_name = target_character.name.clone();
        let room_id = room_checked(&self.entities, target_character.location)?
            .stable_id
            .clone();
        let gameplay = self
            .character_gameplay
            .get_mut(&character_id)
            .ok_or(WorldError::InvalidEntity)?;
        gameplay.vitals.health = gameplay.vitals.max_health;
        for resource in gameplay.resources.values_mut() {
            resource.current = resource.max;
        }
        gameplay.statuses.clear();
        gameplay.effects.clear();
        gameplay.ability_cooldowns.clear();
        gameplay.ability_global_ready_tick = self.logical_tick;
        gameplay.recovery_at_tick = None;
        let mut released_npcs = Vec::new();
        for npc in self.npcs.values_mut() {
            if npc.engaged_character == Some(character_id) {
                npc.engaged_character = None;
                released_npcs.push(npc.stable_id.clone());
            }
        }
        for npc_id in released_npcs {
            self.loot_encounters.remove(&npc_id);
        }
        self.revision = self.revision.wrapping_add(1).max(1);
        let event = DomainEvent::Gameplay {
            event: GameplayEvent::CharacterRestored {
                character_id,
                character_name,
                health: gameplay.vitals.health,
                room_id,
            },
        };
        self.bump_view_revisions(&event)?;
        Ok(ActionOutcome {
            scene: self.scene_for(actor)?,
            container_contents: None,
            trade_contents: None,
            reward_odds: None,
            output: Some("The target has been fully restored.\r\n".to_owned()),
            changed_character: Some(self.character_snapshot(target)?),
            domain_events: vec![event],
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

    fn apply_loot(
        &mut self,
        session: SessionId,
        action: LootAction,
    ) -> Result<ActionOutcome, WorldError> {
        let character = self.character(session)?;
        let actor_id = character.id;
        let room_id = room_checked(&self.entities, character.location)?
            .stable_id
            .clone();
        match action {
            LootAction::Status => {
                const MAX_STATUS_ROWS: usize = 20;
                let observer = self.observer(session)?;
                let mut claims = self
                    .ground_loot
                    .iter()
                    .filter(|(_, reservation)| {
                        reservation.room_id == room_id
                            && reservation.claim_until_tick > self.logical_tick
                            && reservation.beneficiary == Some(actor_id)
                    })
                    .collect::<Vec<_>>();
                claims.sort_by(|(left, _), (right, _)| left.cmp(right));
                let omitted = claims.len().saturating_sub(MAX_STATUS_ROWS);
                let mut output = String::from("Your reserved loot in this room:\r\n");
                if claims.is_empty() {
                    output.push_str("- none\r\n");
                } else {
                    for (item_id, reservation) in claims.into_iter().take(MAX_STATUS_ROWS) {
                        let item = self.item_prototype_for(item_id)?;
                        let selector = opaque_id(observer.salt, "item", item_id.as_str());
                        let remaining = reservation
                            .claim_until_tick
                            .saturating_sub(self.logical_tick);
                        output.push_str(&format!(
                            "- {} ({} ticks; ref {})\r\n",
                            item.name, remaining, selector
                        ));
                    }
                    if omitted > 0 {
                        output.push_str(&format!("- and {omitted} more\r\n"));
                    }
                }
                Ok(ActionOutcome {
                    scene: self.scene_for(session)?,
                    container_contents: None,
                    trade_contents: None,
                    reward_odds: None,
                    output: Some(output),
                    changed_character: None,
                    domain_events: Vec::new(),
                })
            }
            LootAction::Pass(query) => {
                let candidates = self
                    .ground_loot
                    .iter()
                    .filter_map(|(item_id, reservation)| {
                        (reservation.room_id == room_id
                            && reservation.claim_until_tick > self.logical_tick
                            && (reservation.beneficiary == Some(actor_id)
                                || reservation.passed.contains(&actor_id)))
                        .then_some(item_id)
                    });
                let item_id = resolve_item_query(
                    &self.item_instances,
                    &self.item_prototypes,
                    candidates,
                    &query,
                    self.observer(session)?.salt,
                )
                .map_err(|_| WorldError::LootUnavailable)?;
                let reservation = self
                    .ground_loot
                    .get(&item_id)
                    .cloned()
                    .ok_or(WorldError::LootUnavailable)?;
                let item_name = self.item_effective_name(&item_id)?;
                if reservation.passed.contains(&actor_id) {
                    return Ok(ActionOutcome {
                        scene: self.scene_for(session)?,
                        container_contents: None,
                        trade_contents: None,
                        reward_odds: None,
                        output: Some(format!("You have already passed on {item_name}.\r\n")),
                        changed_character: None,
                        domain_events: Vec::new(),
                    });
                }
                if reservation.beneficiary != Some(actor_id) {
                    return Err(WorldError::LootUnavailable);
                }
                let actor_index = reservation
                    .cohort
                    .iter()
                    .position(|member| *member == actor_id)
                    .ok_or(WorldError::LootUnavailable)?;
                let mut passed = reservation.passed.clone();
                passed.insert(actor_id);
                let beneficiary = (1..=reservation.cohort.len())
                    .map(|offset| {
                        reservation.cohort[(actor_index + offset) % reservation.cohort.len()]
                    })
                    .find(|candidate| !passed.contains(candidate));
                let claim = self
                    .ground_loot
                    .get_mut(&item_id)
                    .ok_or(WorldError::LootUnavailable)?;
                claim.passed = passed;
                claim.beneficiary = beneficiary;
                self.revision = self.revision.wrapping_add(1).max(1);
                let summary = beneficiary.map_or_else(
                    || format!("{item_name} is now public loot."),
                    |_| format!("The reservation for {item_name} passes to the next member."),
                );
                let event = DomainEvent::Gameplay {
                    event: GameplayEvent::LootRightsChanged {
                        item_id,
                        room_id,
                        summary: summary.clone(),
                    },
                };
                self.bump_view_revisions(&event)?;
                Ok(ActionOutcome {
                    scene: self.scene_for(session)?,
                    container_contents: None,
                    trade_contents: None,
                    reward_odds: None,
                    output: Some(format!("{summary}\r\n")),
                    changed_character: None,
                    domain_events: vec![event],
                })
            }
        }
    }

    fn open_reward(
        &mut self,
        session: SessionId,
        query: &str,
    ) -> Result<ActionOutcome, WorldError> {
        let checkpoint = self.checkpoint();
        match self.open_reward_inner(session, query) {
            Ok(outcome) => Ok(outcome),
            Err(error) => {
                self.restore_checkpoint(checkpoint);
                Err(error)
            }
        }
    }

    fn open_reward_inner(
        &mut self,
        session: SessionId,
        query: &str,
    ) -> Result<ActionOutcome, WorldError> {
        let source_item_id = self.resolve_accessible_item(session, query, ItemScope::Carried)?;
        self.ensure_items_unreserved(std::iter::once(&source_item_id))?;
        let character = self.character(session)?;
        let character_id = character.id;
        let character_name = character.name.clone();
        let room_id = room_checked(&self.entities, character.location)?
            .stable_id
            .clone();
        let source_instance = self
            .item_instances
            .get(&source_item_id)
            .cloned()
            .ok_or(WorldError::RewardUnavailable)?;
        if source_instance.binding != (ItemBinding::Character { character_id })
            || !source_instance.item_affixes.is_empty()
        {
            return Err(WorldError::RewardUnavailable);
        }
        let source_prototype = self
            .item_prototypes
            .get(&source_instance.prototype_id)
            .cloned()
            .ok_or(WorldError::RewardUnavailable)?;
        let table_id = source_prototype
            .sealed_loot_table_id
            .clone()
            .ok_or(WorldError::RewardUnavailable)?;
        if source_prototype.transfer_policy != ItemTransferPolicy::CharacterBound
            || source_prototype.stack_policy != ItemStackPolicy::Unique
            || source_prototype.max_stack_units != 1
            || !source_prototype.consumable
            || source_prototype.container.is_some()
            || source_prototype.equipment_slot.is_some()
            || source_prototype.attack_bonus != 0
            || source_prototype.heal_amount != 0
            || source_prototype.effect.is_some()
        {
            return Err(WorldError::RewardUnavailable);
        }
        let source_provenance = self
            .item_provenance
            .get(&source_item_id)
            .cloned()
            .ok_or(WorldError::RewardUnavailable)?;
        if !matches!(
            &source_provenance,
            ItemProvenance::NpcLoot {
                sealed_loot_table_id: Some(authored_table_id),
                ..
            } if authored_table_id == &table_id
        ) {
            return Err(WorldError::RewardUnavailable);
        }
        let source_npc_loot_digest_sha256 = canonical_sha256(&source_provenance)?;
        let source_item_name = self.item_effective_name(&source_item_id)?;
        let open_id = self.next_reward_open_id;
        self.next_reward_open_id = RewardOpenId(
            open_id
                .0
                .checked_add(1)
                .ok_or(WorldError::RewardUnavailable)?,
        );
        let (table, units) = self.roll_loot_table(&table_id)?;
        if units.is_empty() || units.len() > MAX_SEALED_REWARD_UNITS {
            return Err(WorldError::RewardUnavailable);
        }

        let mut generated = Vec::with_capacity(units.len());
        for unit in units {
            let item_id = self.create_staged_item_instance(&unit.prototype_id)?;
            let item_affixes = unit
                .affix_pool_id
                .as_deref()
                .map(|pool_id| self.roll_item_affix(pool_id))
                .transpose()?
                .into_iter()
                .flatten()
                .collect::<Vec<_>>();
            self.item_instances
                .get_mut(&item_id)
                .ok_or(WorldError::RewardUnavailable)?
                .item_affixes = item_affixes;
            generated.push((item_id, unit));
        }

        let mut prospective = self.item_instances.clone();
        prospective
            .get_mut(&source_item_id)
            .ok_or(WorldError::RewardUnavailable)?
            .location = SharedItemLocation::Consumed;
        for (item_id, _) in &generated {
            prospective
                .get_mut(item_id)
                .ok_or(WorldError::RewardUnavailable)?
                .location = SharedItemLocation::Carried(character_id);
        }
        let mut roots = Vec::with_capacity(generated.len() + 1);
        roots.push(source_item_id.clone());
        roots.extend(generated.iter().map(|(item_id, _)| item_id.clone()));
        self.apply_item_ownership_transitions(&mut prospective, &roots)?;
        self.validate_prospective_item_locations(&prospective, [character_id])?;
        self.item_instances = prospective;

        let mut item_ids = Vec::with_capacity(generated.len());
        let mut item_names = Vec::with_capacity(generated.len());
        for (item_id, unit) in generated {
            self.item_provenance.insert(
                item_id.clone(),
                ItemProvenance::SealedReward {
                    open_id,
                    source_item_id: source_item_id.clone(),
                    source_npc_loot_digest_sha256: source_npc_loot_digest_sha256.clone(),
                    sealed_prototype_id: source_instance.prototype_id.clone(),
                    loot_table_id: table.id.clone(),
                    table_digest_sha256: table.digest_sha256.clone(),
                    entry_id: unit.entry_id,
                    pool_id: unit.pool_id,
                    roll_index: unit.roll_index,
                    quantity_index: unit.quantity_index,
                    generated_tick: self.logical_tick,
                },
            );
            item_names.push(self.item_effective_name(&item_id)?);
            item_ids.push(item_id);
        }
        let event = DomainEvent::Gameplay {
            event: GameplayEvent::RewardOpened {
                open_id,
                character_id,
                character_name,
                source_item_id,
                source_item_name,
                item_ids,
                item_names: item_names.clone(),
                room_id,
            },
        };
        let outcome = self.finish_gameplay_action(
            session,
            format!(
                "Abres la recompensa sellada y obtienes: {}.\r\n",
                item_names.join(", ")
            ),
            vec![event],
        )?;
        if serde_json::to_vec(&self.runtime_snapshot())
            .map_err(|_| WorldError::RewardUnavailable)?
            .len()
            > MAX_WORLD_RUNTIME_SNAPSHOT_BYTES
        {
            return Err(WorldError::InventoryCapacityExceeded);
        }
        Ok(outcome)
    }

    fn roll_item_affix(&mut self, pool_id: &str) -> Result<Option<AppliedItemAffix>, WorldError> {
        let pool = self
            .affix_pools
            .get(pool_id)
            .cloned()
            .ok_or(WorldError::InvalidGameplayDefinition)?;
        let selected = self.rng.uniform_below(pool.total_weight)?;
        let mut cumulative = 0_u64;
        let entry = pool
            .entries
            .iter()
            .find(|entry| {
                cumulative = cumulative.saturating_add(u64::from(entry.weight));
                selected < cumulative
            })
            .ok_or(WorldError::InvalidGameplayDefinition)?;
        if entry.kind == AffixPoolEntryKind::Nothing {
            return Ok(None);
        }
        let definition = self
            .affixes
            .get(
                entry
                    .affix_id
                    .as_deref()
                    .ok_or(WorldError::InvalidGameplayDefinition)?,
            )
            .cloned()
            .ok_or(WorldError::InvalidGameplayDefinition)?;
        let mut applied = AppliedItemAffix {
            affix_id: definition.id.clone(),
            definition_digest_sha256: canonical_sha256(&definition)?,
            pool_id: pool.id,
            pool_digest_sha256: pool.digest_sha256,
            record_digest_sha256: String::new(),
            label: definition.label,
            position: definition.position,
            attack_bonus_delta: definition.attack_bonus_delta,
            heal_amount_delta: definition.heal_amount_delta,
        };
        applied.record_digest_sha256 = applied_affix_record_sha256(&applied)?;
        Ok(Some(applied))
    }

    fn reward_odds(
        &self,
        session: SessionId,
        target: &str,
        page: u16,
    ) -> Result<ActionOutcome, WorldError> {
        if page == 0 {
            return Err(WorldError::RewardUnavailable);
        }
        let source_item_id = self.resolve_accessible_item(session, target, ItemScope::Carried)?;
        let character_id = self.character(session)?.id;
        if !self.item_is_openable_reward(&source_item_id, character_id) {
            return Err(WorldError::RewardUnavailable);
        }
        let source = self
            .item_instances
            .get(&source_item_id)
            .ok_or(WorldError::RewardUnavailable)?;
        let table_id = self
            .item_prototypes
            .get(&source.prototype_id)
            .and_then(|prototype| prototype.sealed_loot_table_id.as_deref())
            .ok_or(WorldError::RewardUnavailable)?;
        let table = self
            .loot_tables
            .get(table_id)
            .ok_or(WorldError::RewardUnavailable)?;

        enum OddsRow {
            Guaranteed(RewardOddsGuaranteedRow),
            Weighted(RewardOddsWeightedRow),
            Affix(RewardOddsAffixRow),
        }
        let mut rows = Vec::new();
        let append_affixes =
            |outcome_label: &str, pool_id: Option<&str>| -> Result<Vec<OddsRow>, WorldError> {
                let Some(pool_id) = pool_id else {
                    return Ok(Vec::new());
                };
                let pool = self
                    .affix_pools
                    .get(pool_id)
                    .ok_or(WorldError::InvalidGameplayDefinition)?;
                let mut affix_rows = Vec::with_capacity(pool.entries.len());
                for entry in &pool.entries {
                    let (affix_label, position, attack_bonus_delta, heal_amount_delta) =
                        if entry.kind == AffixPoolEntryKind::Nothing {
                            ("sin afijo".to_owned(), None, 0, 0)
                        } else {
                            let definition = self
                                .affixes
                                .get(
                                    entry
                                        .affix_id
                                        .as_deref()
                                        .ok_or(WorldError::InvalidGameplayDefinition)?,
                                )
                                .ok_or(WorldError::InvalidGameplayDefinition)?;
                            (
                                definition.label.clone(),
                                Some(definition.position),
                                definition.attack_bonus_delta,
                                definition.heal_amount_delta,
                            )
                        };
                    affix_rows.push(OddsRow::Affix(RewardOddsAffixRow {
                        parent_outcome_label: outcome_label.to_owned(),
                        affix_label,
                        position,
                        attack_bonus_delta,
                        heal_amount_delta,
                        conditional_probability_numerator: u64::from(entry.weight),
                        conditional_probability_denominator: pool.total_weight,
                    }));
                }
                Ok(affix_rows)
            };
        for guaranteed in &table.guaranteed {
            let label = self
                .item_prototypes
                .get(&guaranteed.item_prototype_id)
                .ok_or(WorldError::InvalidGameplayDefinition)?
                .name
                .clone();
            rows.push(OddsRow::Guaranteed(RewardOddsGuaranteedRow {
                label: label.clone(),
                quantity: guaranteed.quantity,
                affix_pool_label: guaranteed
                    .affix_pool_id
                    .as_ref()
                    .map(|_| "afijo aleatorio".to_owned()),
            }));
            rows.extend(append_affixes(&label, guaranteed.affix_pool_id.as_deref())?);
        }
        for (pool_index, pool) in table.pools.iter().enumerate() {
            let denominator = pool.entries.iter().try_fold(0_u64, |total, entry| {
                total
                    .checked_add(u64::from(entry.weight))
                    .ok_or(WorldError::InvalidGameplayDefinition)
            })?;
            for entry in &pool.entries {
                let (label, minimum, maximum) = if entry.kind == LootEntryKind::Nothing {
                    ("sin recompensa".to_owned(), 0, 0)
                } else {
                    (
                        self.item_prototypes
                            .get(
                                entry
                                    .item_prototype_id
                                    .as_deref()
                                    .ok_or(WorldError::InvalidGameplayDefinition)?,
                            )
                            .ok_or(WorldError::InvalidGameplayDefinition)?
                            .name
                            .clone(),
                        entry
                            .min_quantity
                            .ok_or(WorldError::InvalidGameplayDefinition)?,
                        entry
                            .max_quantity
                            .ok_or(WorldError::InvalidGameplayDefinition)?,
                    )
                };
                rows.push(OddsRow::Weighted(RewardOddsWeightedRow {
                    pool_label: format!("Tirada {}", pool_index + 1),
                    rolls: pool.rolls,
                    outcome_label: label.clone(),
                    min_quantity: minimum,
                    max_quantity: maximum,
                    probability_numerator: u64::from(entry.weight),
                    probability_denominator: denominator,
                    affix_pool_label: entry
                        .affix_pool_id
                        .as_ref()
                        .map(|_| "afijo aleatorio".to_owned()),
                }));
                rows.extend(append_affixes(&label, entry.affix_pool_id.as_deref())?);
            }
        }
        if rows.len() > MAX_REWARD_ODDS_ROWS {
            return Err(WorldError::InvalidGameplayDefinition);
        }
        let total_pages = rows.len().max(1).div_ceil(REWARD_ODDS_PAGE_SIZE);
        let page_index = usize::from(page - 1);
        if page_index >= total_pages {
            return Err(WorldError::RewardUnavailable);
        }
        let mut guaranteed = Vec::new();
        let mut weighted = Vec::new();
        let mut affixes = Vec::new();
        for row in rows
            .into_iter()
            .skip(page_index * REWARD_ODDS_PAGE_SIZE)
            .take(REWARD_ODDS_PAGE_SIZE)
        {
            match row {
                OddsRow::Guaranteed(row) => guaranteed.push(row),
                OddsRow::Weighted(row) => weighted.push(row),
                OddsRow::Affix(row) => affixes.push(row),
            }
        }
        let total_pages = u16::try_from(total_pages).map_err(|_| WorldError::RewardUnavailable)?;
        let selector = opaque_id(
            self.observer(session)?.salt,
            "item",
            source_item_id.as_str(),
        );
        let mut output = format!("Probabilidades de recompensa, página {page}/{total_pages}:\r\n");
        for row in &guaranteed {
            let affix = if row.affix_pool_label.is_some() {
                "; afijo aleatorio"
            } else {
                ""
            };
            output.push_str(&format!(
                "- {} × {}: garantizado{affix}.\r\n",
                row.quantity, row.label,
            ));
        }
        for row in &weighted {
            let affix = if row.affix_pool_label.is_some() {
                "; afijo aleatorio"
            } else {
                ""
            };
            output.push_str(&format!(
                "- {}: {} tirada(s); {}; cantidad {}..={}; probabilidad {}/{} por tirada{affix}.\r\n",
                row.pool_label,
                row.rolls,
                row.outcome_label,
                row.min_quantity,
                row.max_quantity,
                row.probability_numerator,
                row.probability_denominator,
            ));
        }
        for row in &affixes {
            let position = match row.position {
                Some(AffixPosition::Prefix) => "prefix",
                Some(AffixPosition::Suffix) => "suffix",
                None => "none",
            };
            output.push_str(&format!(
                "- {} → {}: posición {position}; ataque {:+}; curación {:+}; probabilidad {}/{} condicional.\r\n",
                row.parent_outcome_label,
                row.affix_label,
                row.attack_bonus_delta,
                row.heal_amount_delta,
                row.conditional_probability_numerator,
                row.conditional_probability_denominator
            ));
        }
        Ok(ActionOutcome {
            scene: self.scene_for(session)?,
            container_contents: None,
            trade_contents: None,
            reward_odds: Some(RewardOddsView {
                selector,
                page,
                total_pages,
                guaranteed,
                weighted,
                affixes,
            }),
            output: Some(output),
            changed_character: None,
            domain_events: Vec::new(),
        })
    }

    fn apply_trade(
        &mut self,
        session: SessionId,
        action: TradeAction,
    ) -> Result<ActionOutcome, WorldError> {
        let checkpoint = self.checkpoint();
        match self.apply_trade_inner(session, action) {
            Ok(outcome) => Ok(outcome),
            Err(error) => {
                self.restore_checkpoint(checkpoint);
                Err(error)
            }
        }
    }

    fn apply_trade_inner(
        &mut self,
        session: SessionId,
        action: TradeAction,
    ) -> Result<ActionOutcome, WorldError> {
        let character = self.character(session)?;
        let actor_id = character.id;
        match action {
            TradeAction::Request(query) => {
                if self.trades.len() >= MAX_LIVE_TRADES
                    || self.trade_for_character(actor_id).is_some()
                    || self
                        .last_trade_request_ticks
                        .get(&actor_id)
                        .is_some_and(|last_tick| {
                            last_tick
                                .checked_add(TRADE_REQUEST_RATE_TICKS)
                                .is_none_or(|next_tick| self.logical_tick < next_tick)
                        })
                {
                    return Err(WorldError::TradeUnavailable);
                }
                let recipient = self.resolve_trade_recipient(session, &query)?;
                if self.trade_for_character(recipient.character_id).is_some()
                    || self.character_in_combat(actor_id)
                    || self.character_in_combat(recipient.character_id)
                    || self.character_defeated(actor_id)
                    || self.character_defeated(recipient.character_id)
                {
                    return Err(WorldError::TradeUnavailable);
                }
                let room_id = room_checked(&self.entities, character.location)?
                    .stable_id
                    .clone();
                let id = self.next_trade_id;
                self.next_trade_id = TradeId(
                    id.0.checked_add(1)
                        .ok_or(WorldError::InvalidDomainTransition)?,
                );
                let trade = TradeSnapshot {
                    id,
                    requester: actor_id,
                    recipient: recipient.character_id,
                    room_id,
                    phase: TradePhase::Requested,
                    created_tick: self.logical_tick,
                    opened_tick: None,
                    expires_at_tick: self
                        .logical_tick
                        .checked_add(TRADE_REQUEST_TICKS)
                        .ok_or(WorldError::InvalidDomainTransition)?,
                    state_revision: 1,
                    offers: BTreeMap::new(),
                    confirmations: BTreeMap::new(),
                };
                self.last_trade_request_ticks
                    .insert(actor_id, self.logical_tick);
                self.trades.insert(id, trade.clone());
                self.finish_trade_action(
                    session,
                    format!("You ask {} to trade.\r\n", recipient.name),
                    vec![DomainEvent::TradeRequested { trade }],
                )
            }
            TradeAction::Accept => {
                let previous = self
                    .trade_for_character(actor_id)
                    .filter(|trade| {
                        trade.phase == TradePhase::Requested && trade.recipient == actor_id
                    })
                    .cloned()
                    .ok_or(WorldError::TradeUnavailable)?;
                self.ensure_trade_available(&previous)?;
                let mut current = previous.clone();
                current.phase = TradePhase::Negotiating;
                current.opened_tick = Some(self.logical_tick);
                current.expires_at_tick = self
                    .logical_tick
                    .checked_add(TRADE_NEGOTIATION_TICKS)
                    .ok_or(WorldError::InvalidDomainTransition)?;
                current.state_revision = current
                    .state_revision
                    .checked_add(1)
                    .ok_or(WorldError::InvalidDomainTransition)?;
                current.offers = BTreeMap::from([
                    (current.requester, TradeOffer::default()),
                    (current.recipient, TradeOffer::default()),
                ]);
                self.trades.insert(current.id, current.clone());
                self.finish_trade_action(
                    session,
                    "You accept the trade request.\r\n".into(),
                    vec![DomainEvent::TradeOpened { previous, current }],
                )
            }
            TradeAction::Decline => {
                let trade = self
                    .trade_for_character(actor_id)
                    .filter(|trade| {
                        trade.phase == TradePhase::Requested && trade.recipient == actor_id
                    })
                    .cloned()
                    .ok_or(WorldError::TradeUnavailable)?;
                self.trades.remove(&trade.id);
                self.finish_trade_action(
                    session,
                    "You decline the trade request.\r\n".into(),
                    vec![DomainEvent::TradeCancelled {
                        previous_trade: trade,
                        reason: TradeCancellationReason::Declined,
                    }],
                )
            }
            TradeAction::Cancel => {
                let trade = self
                    .trade_for_character(actor_id)
                    .cloned()
                    .ok_or(WorldError::TradeUnavailable)?;
                self.trades.remove(&trade.id);
                self.finish_trade_action(
                    session,
                    "You cancel the trade.\r\n".into(),
                    vec![DomainEvent::TradeCancelled {
                        previous_trade: trade,
                        reason: TradeCancellationReason::CancelledByParticipant,
                    }],
                )
            }
            TradeAction::OfferCoins(coins) => {
                if coins > MAX_TRADE_COINS
                    || self
                        .character_gameplay
                        .get(&actor_id)
                        .is_none_or(|gameplay| gameplay.coins < coins)
                {
                    return Err(WorldError::InsufficientFunds);
                }
                let previous = self.negotiating_trade(actor_id)?.clone();
                self.ensure_trade_available(&previous)?;
                let mut current = previous.clone();
                let offer = current
                    .offers
                    .get_mut(&actor_id)
                    .ok_or(WorldError::TradeUnavailable)?;
                if offer.coins == coins {
                    return self.trade_status_outcome(session);
                }
                offer.coins = coins;
                self.publish_trade_offer_change(session, previous, current)
            }
            TradeAction::OfferItem(selection) => {
                if selection.quantity == 0 || selection.quantity > MAX_TRADE_ITEM_QUANTITY {
                    return Err(WorldError::TradeUnavailable);
                }
                let previous = self.negotiating_trade(actor_id)?.clone();
                self.ensure_trade_available(&previous)?;
                let mut all_carried = self.item_candidates(session, ItemScope::Carried, None)?;
                let own_reserved = previous
                    .offers
                    .get(&actor_id)
                    .into_iter()
                    .flat_map(|offer| &offer.lines)
                    .flat_map(|line| &line.item_ids)
                    .cloned()
                    .collect::<BTreeSet<_>>();
                let all_reserved = self.reserved_item_ids()?;
                all_carried.retain(|id| !all_reserved.contains(id) || own_reserved.contains(id));
                let representative = self
                    .resolve_item_selection(
                        session,
                        &ItemSelection::one(&selection.query),
                        all_carried.clone(),
                    )?
                    .into_iter()
                    .next()
                    .ok_or(WorldError::TradeUnavailable)?;
                let instance = self
                    .item_instances
                    .get(&representative)
                    .ok_or(WorldError::InvalidEntity)?;
                let prototype = self
                    .item_prototypes
                    .get(&instance.prototype_id)
                    .ok_or(WorldError::InvalidEntity)?;
                let key = match prototype.stack_policy {
                    ItemStackPolicy::Fungible => TradeOfferKey::Fungible {
                        prototype_id: instance.prototype_id.clone(),
                    },
                    ItemStackPolicy::Unique => TradeOfferKey::Unique {
                        item_id: representative.clone(),
                    },
                };
                if prototype.stack_policy == ItemStackPolicy::Unique && selection.quantity != 1 {
                    return Err(WorldError::TradeUnavailable);
                }
                let mut current = previous.clone();
                let offer = current
                    .offers
                    .get_mut(&actor_id)
                    .ok_or(WorldError::TradeUnavailable)?;
                let line_index = offer.lines.iter().position(|line| line.key == key);
                let mut ids = line_index
                    .map(|index| offer.lines[index].item_ids.clone())
                    .unwrap_or_default();
                let desired = usize::from(selection.quantity);
                if ids.len() < desired {
                    let mut candidates = all_carried
                        .into_iter()
                        .filter(|id| {
                            self.item_instances.get(id).is_some_and(|candidate| {
                                candidate.prototype_id == instance.prototype_id
                                    && !all_reserved.contains(id)
                            })
                        })
                        .collect::<Vec<_>>();
                    candidates.sort();
                    for candidate in candidates {
                        if ids.len() == desired {
                            break;
                        }
                        if !ids.contains(&candidate) {
                            ids.push(candidate);
                        }
                    }
                } else {
                    ids.truncate(desired);
                }
                ids.sort();
                if ids.len() != desired {
                    return Err(WorldError::TradeUnavailable);
                }
                let graph = build_item_graph_index(&self.item_instances, &self.item_prototypes)?;
                if ids.iter().any(|id| {
                    self.item_location(id) != Some(&SharedItemLocation::Carried(actor_id))
                        || !graph.subtree_is_transferable(id).unwrap_or(false)
                }) {
                    return Err(WorldError::ItemTransferForbidden);
                }
                let new_line = TradeOfferLine { key, item_ids: ids };
                if let Some(index) = line_index {
                    if offer.lines[index] == new_line {
                        return self.trade_status_outcome(session);
                    }
                    offer.lines[index] = new_line;
                } else {
                    if offer.lines.len() >= MAX_TRADE_LINES_PER_PARTICIPANT {
                        return Err(WorldError::TradeUnavailable);
                    }
                    offer.lines.push(new_line);
                    offer.lines.sort_by(|left, right| left.key.cmp(&right.key));
                }
                let total_roots = current
                    .offers
                    .values()
                    .flat_map(|offer| &offer.lines)
                    .map(|line| line.item_ids.len())
                    .sum::<usize>();
                if total_roots > MAX_TRADE_ROOTS {
                    return Err(WorldError::TradeUnavailable);
                }
                self.publish_trade_offer_change(session, previous, current)
            }
            TradeAction::RemoveItem(query) => {
                let previous = self.negotiating_trade(actor_id)?.clone();
                let observer = self.observer(session)?;
                let mut current = previous.clone();
                let offer = current
                    .offers
                    .get_mut(&actor_id)
                    .ok_or(WorldError::TradeUnavailable)?;
                let matches = offer
                    .lines
                    .iter()
                    .enumerate()
                    .filter(|(_, line)| {
                        let Some(first) = line.item_ids.first() else {
                            return false;
                        };
                        let label_matches = self.item_prototype_for(first).is_ok_and(|prototype| {
                            prototype.name.eq_ignore_ascii_case(query.trim())
                        });
                        label_matches
                            || opaque_id(observer.salt, "item", first.as_str()) == query.trim()
                    })
                    .map(|(index, _)| index)
                    .collect::<Vec<_>>();
                let [index] = matches.as_slice() else {
                    return Err(WorldError::TradeUnavailable);
                };
                offer.lines.remove(*index);
                self.publish_trade_offer_change(session, previous, current)
            }
            TradeAction::Confirm { state_revision } => {
                let previous = self.negotiating_trade(actor_id)?.clone();
                self.ensure_trade_available(&previous)?;
                if previous.state_revision != state_revision {
                    return Err(WorldError::StaleTradeRevision);
                }
                if !trade_has_assets(&previous) {
                    return Err(WorldError::TradeUnavailable);
                }
                let basis_sha256 = self.trade_basis_sha256(&previous)?;
                let confirmation = TradeConfirmation {
                    state_revision,
                    basis_sha256,
                };
                if previous.confirmations.get(&actor_id) == Some(&confirmation) {
                    return self.trade_status_outcome(session);
                }
                let mut current = previous.clone();
                current.confirmations.insert(actor_id, confirmation.clone());
                if current.confirmations.len() == 2 {
                    if current
                        .confirmations
                        .values()
                        .any(|candidate| candidate != &confirmation)
                    {
                        return Err(WorldError::StaleTradeRevision);
                    }
                    self.complete_trade(session, current)
                } else {
                    self.trades.insert(current.id, current);
                    self.finish_trade_action(
                        session,
                        "You confirm the current trade terms.\r\n".into(),
                        vec![DomainEvent::TradeConfirmed {
                            participant: actor_id,
                            confirmation,
                        }],
                    )
                }
            }
            TradeAction::Status => self.trade_status_outcome(session),
            TradeAction::Contents { container, page } => {
                self.inspect_trade_container(session, &container, page)
            }
        }
    }

    fn negotiating_trade(&self, character_id: CharacterId) -> Result<&TradeSnapshot, WorldError> {
        self.trade_for_character(character_id)
            .filter(|trade| trade.phase == TradePhase::Negotiating)
            .ok_or(WorldError::TradeUnavailable)
    }

    fn character_in_combat(&self, character_id: CharacterId) -> bool {
        self.npcs
            .values()
            .any(|npc| npc.engaged_character == Some(character_id))
    }

    fn character_defeated(&self, character_id: CharacterId) -> bool {
        self.character_gameplay
            .get(&character_id)
            .is_none_or(|gameplay| gameplay.recovery_at_tick.is_some())
    }

    fn ensure_trade_available(&self, trade: &TradeSnapshot) -> Result<(), WorldError> {
        if self.logical_tick >= trade.expires_at_tick
            || self.character_in_combat(trade.requester)
            || self.character_in_combat(trade.recipient)
            || self.character_defeated(trade.requester)
            || self.character_defeated(trade.recipient)
        {
            return Err(WorldError::TradeUnavailable);
        }
        let rooms = trade
            .participants()
            .into_iter()
            .map(|participant| {
                self.session_for_character(participant)
                    .ok_or(WorldError::TradeUnavailable)
                    .and_then(|session| self.character(session))
                    .and_then(|character| room_checked(&self.entities, character.location))
                    .map(|room| room.stable_id.as_str())
            })
            .collect::<Result<Vec<_>, WorldError>>()?;
        if rooms.iter().any(|room| *room != trade.room_id) {
            return Err(WorldError::TradeUnavailable);
        }
        Ok(())
    }

    fn publish_trade_offer_change(
        &mut self,
        session: SessionId,
        previous: TradeSnapshot,
        mut current: TradeSnapshot,
    ) -> Result<ActionOutcome, WorldError> {
        current.state_revision = current
            .state_revision
            .checked_add(1)
            .ok_or(WorldError::InvalidDomainTransition)?;
        current.confirmations.clear();
        let mut candidate_trades = self.trades.clone();
        candidate_trades.insert(current.id, current.clone());
        validate_trade_state(
            self.next_trade_id,
            &candidate_trades,
            &self.item_instances,
            &self.item_prototypes,
            &self.stable_rooms,
        )?;
        self.trades.insert(current.id, current.clone());
        self.finish_trade_action(
            session,
            "You update your trade offer.\r\n".into(),
            vec![DomainEvent::TradeOfferSet { previous, current }],
        )
    }

    fn complete_trade(
        &mut self,
        session: SessionId,
        trade: TradeSnapshot,
    ) -> Result<ActionOutcome, WorldError> {
        self.ensure_trade_available(&trade)?;
        let offer_a = trade
            .offers
            .get(&trade.requester)
            .ok_or(WorldError::TradeUnavailable)?;
        let offer_b = trade
            .offers
            .get(&trade.recipient)
            .ok_or(WorldError::TradeUnavailable)?;
        let roots_a = offer_a
            .lines
            .iter()
            .flat_map(|line| &line.item_ids)
            .cloned()
            .collect::<Vec<_>>();
        let roots_b = offer_b
            .lines
            .iter()
            .flat_map(|line| &line.item_ids)
            .cloned()
            .collect::<Vec<_>>();
        let graph = build_item_graph_index(&self.item_instances, &self.item_prototypes)?;
        if roots_a.iter().any(|root| {
            self.item_location(root) != Some(&SharedItemLocation::Carried(trade.requester))
                || !graph.subtree_is_transferable(root).unwrap_or(false)
        }) || roots_b.iter().any(|root| {
            self.item_location(root) != Some(&SharedItemLocation::Carried(trade.recipient))
                || !graph.subtree_is_transferable(root).unwrap_or(false)
        }) {
            return Err(WorldError::TradeUnavailable);
        }
        let mut prospective = self.item_instances.clone();
        for root in &roots_a {
            prospective
                .get_mut(root)
                .ok_or(WorldError::InvalidEntity)?
                .location = SharedItemLocation::Carried(trade.recipient);
        }
        for root in &roots_b {
            prospective
                .get_mut(root)
                .ok_or(WorldError::InvalidEntity)?
                .location = SharedItemLocation::Carried(trade.requester);
        }
        self.validate_prospective_item_locations(&prospective, [trade.requester, trade.recipient])?;
        let coins_a = self
            .character_gameplay
            .get(&trade.requester)
            .ok_or(WorldError::InvalidEntity)?
            .coins;
        let coins_b = self
            .character_gameplay
            .get(&trade.recipient)
            .ok_or(WorldError::InvalidEntity)?
            .coins;
        let final_a = coins_a
            .checked_sub(offer_a.coins)
            .and_then(|coins| coins.checked_add(offer_b.coins))
            .ok_or(WorldError::InsufficientFunds)?;
        let final_b = coins_b
            .checked_sub(offer_b.coins)
            .and_then(|coins| coins.checked_add(offer_a.coins))
            .ok_or(WorldError::InsufficientFunds)?;
        self.item_instances = prospective;
        self.character_gameplay
            .get_mut(&trade.requester)
            .ok_or(WorldError::InvalidEntity)?
            .coins = final_a;
        self.character_gameplay
            .get_mut(&trade.recipient)
            .ok_or(WorldError::InvalidEntity)?
            .coins = final_b;
        self.trades.remove(&trade.id);
        self.finish_trade_action(
            session,
            "The trade completes atomically.\r\n".into(),
            vec![DomainEvent::TradeCompleted {
                previous_trade: trade,
            }],
        )
    }

    fn finish_trade_action(
        &mut self,
        session: SessionId,
        output: String,
        events: Vec<DomainEvent>,
    ) -> Result<ActionOutcome, WorldError> {
        self.revision = self
            .revision
            .checked_add(1)
            .ok_or(WorldError::InvalidDomainTransition)?;
        self.bump_view_revisions_for_events(&events)?;
        Ok(ActionOutcome {
            scene: self.scene_for(session)?,
            container_contents: None,
            trade_contents: None,
            reward_odds: None,
            output: Some(output),
            changed_character: None,
            domain_events: events,
        })
    }

    fn trade_status_outcome(&self, session: SessionId) -> Result<ActionOutcome, WorldError> {
        let viewer = self.character(session)?;
        let durable_trade = self
            .trade_for_character(viewer.id)
            .ok_or(WorldError::TradeUnavailable)?;
        let counterparty_id = durable_trade
            .counterpart(viewer.id)
            .ok_or(WorldError::InvalidRuntimeSnapshot)?;
        let counterparty_name = self
            .connected_character_name(counterparty_id)
            .unwrap_or_else(|| "Disconnected character".into());
        let player = self.player_state_view(session)?;
        let trade = player.trade.as_ref().ok_or(WorldError::TradeUnavailable)?;
        let offer_text = |label: &str, offer: &TradeOfferView| {
            let mut output = format!("{label}: {} coin", offer.coins);
            if offer.lines.is_empty() {
                output.push_str("; no items");
            } else {
                for line in &offer.lines {
                    output.push_str(&format!(
                        "; {} × {} [ref {}{}]",
                        line.quantity,
                        line.label,
                        line.target,
                        if line.container { "; container" } else { "" }
                    ));
                }
            }
            output.push_str(".\r\n");
            output
        };
        let mut output = format!(
            "Trade {} with {}: {:?}; revision {}; expires in {} ticks.\r\nYour confirmation: {}; counterpart confirmation: {}.\r\n",
            trade.id,
            counterparty_name,
            trade.phase,
            trade.state_revision,
            trade.expires_in_ticks,
            if trade.own_confirmed { "yes" } else { "no" },
            if trade.counterparty_confirmed {
                "yes"
            } else {
                "no"
            },
        );
        output.push_str(&offer_text("Your offer", &trade.own_offer));
        output.push_str(&offer_text("Counterpart offer", &trade.counterparty_offer));
        output.push_str(&format!(
            "Coins: {} total; {} reserved; {} spendable.\r\n",
            player.economy.coins, player.economy.reserved_coins, player.economy.spendable_coins,
        ));
        output.push_str("Inventory available for trade:\r\n");
        if player.inventory.is_empty() {
            output.push_str("- none\r\n");
        } else {
            for item in player
                .inventory
                .iter()
                .take(MAX_TRADE_STATUS_INVENTORY_GROUPS)
            {
                output.push_str(&format!(
                    "- {}: {} total; {} available; {} reserved [ref {}].\r\n",
                    item.label,
                    item.quantity,
                    item.available_quantity,
                    item.reserved_for_trade,
                    item.id,
                ));
            }
            let omitted = player
                .inventory
                .len()
                .saturating_sub(MAX_TRADE_STATUS_INVENTORY_GROUPS);
            if omitted > 0 {
                output.push_str(&format!(
                    "- {omitted} more inventory groups omitted; use inventory to choose an opaque reference. Use trade contents <offered-container> <page> to inspect an offered container.\r\n"
                ));
            }
        }
        if output.len() > MAX_TRADE_STATUS_TEXT_BYTES {
            return Err(WorldError::InvalidRuntimeSnapshot);
        }
        Ok(ActionOutcome {
            scene: self.scene_for(session)?,
            container_contents: None,
            trade_contents: None,
            reward_odds: None,
            output: Some(output),
            changed_character: None,
            domain_events: Vec::new(),
        })
    }

    fn inspect_trade_container(
        &self,
        session: SessionId,
        container: &str,
        page: u16,
    ) -> Result<ActionOutcome, WorldError> {
        if page == 0 {
            return Err(WorldError::TradeUnavailable);
        }
        let viewer = self.character(session)?;
        let trade = self.negotiating_trade(viewer.id)?;
        let observer = self.observer(session)?;
        let query = container.trim();
        let matches = trade
            .offers
            .values()
            .flat_map(|offer| &offer.lines)
            .flat_map(|line| &line.item_ids)
            .filter(|item_id| {
                self.item_prototype_for(item_id).is_ok_and(|prototype| {
                    prototype.container.is_some()
                        && (prototype.name.eq_ignore_ascii_case(query)
                            || opaque_id(observer.salt, "item", item_id.as_str()) == query)
                })
            })
            .cloned()
            .collect::<Vec<_>>();
        let [container_id] = matches.as_slice() else {
            return Err(WorldError::TradeUnavailable);
        };
        let graph = build_item_graph_index(&self.item_instances, &self.item_prototypes)?;
        let direct_ids = self
            .item_instances
            .iter()
            .filter_map(|(item_id, instance)| {
                matches!(&instance.location, SharedItemLocation::Contained(parent) if parent == container_id)
                    .then_some(item_id)
            })
            .collect::<Vec<_>>();
        let groups = derive_stack_groups(
            &self.item_instances,
            &self.item_prototypes,
            direct_ids.into_iter(),
        )?;
        let total_pages = groups.len().max(1).div_ceil(CONTAINER_PAGE_SIZE);
        let page_index = usize::from(page - 1);
        if page_index >= total_pages {
            return Err(WorldError::TradeUnavailable);
        }
        let items = groups
            .into_iter()
            .skip(page_index * CONTAINER_PAGE_SIZE)
            .take(CONTAINER_PAGE_SIZE)
            .map(|group| {
                let first = group
                    .members
                    .first()
                    .ok_or(WorldError::InvalidRuntimeSnapshot)?;
                let prototype = self.item_prototype_for(first)?;
                let quantity = u16::try_from(group.members.len())
                    .map_err(|_| WorldError::InvalidRuntimeSnapshot)?;
                Ok(PlayerItemView {
                    id: opaque_id(observer.salt, "item", first.as_str()),
                    label: self.item_effective_name(first)?,
                    description: prototype.description.clone(),
                    rarity: prototype.rarity.as_str().into(),
                    item_level: prototype.item_level,
                    weight_grams: prototype.weight_grams,
                    quantity,
                    available_quantity: 0,
                    reserved_for_trade: quantity,
                    stack_slots: stack_slots(prototype, group.members.len())?,
                    total_weight_grams: group.members.iter().try_fold(
                        0_u64,
                        |total, item_id| {
                            total
                                .checked_add(graph.subtree_weight(item_id)?)
                                .ok_or(WorldError::InvalidEntity)
                        },
                    )?,
                    transfer_policy: prototype.transfer_policy,
                    bound_to_you: false,
                    can_give: false,
                    can_trade: false,
                    affix: self.item_affix_view(first),
                    container_capacity: prototype
                        .container
                        .as_ref()
                        .map(|_| {
                            container_usage(
                                first,
                                &self.item_instances,
                                &self.item_prototypes,
                                &graph,
                            )
                        })
                        .transpose()?,
                    actions: Vec::new(),
                })
            })
            .collect::<Result<Vec<_>, WorldError>>()?;
        let capacity = container_usage(
            container_id,
            &self.item_instances,
            &self.item_prototypes,
            &graph,
        )?;
        let total_pages =
            u16::try_from(total_pages).map_err(|_| WorldError::InvalidRuntimeSnapshot)?;
        let mut output = format!(
            "Offered container contents, page {page}/{total_pages}: {}/{} slots; {}/{} g.\r\n",
            capacity.used_slots,
            capacity.max_slots,
            capacity.used_weight_grams,
            capacity.max_weight_grams
        );
        if items.is_empty() {
            output.push_str("- empty\r\n");
        } else {
            for item in &items {
                output.push_str(&format!(
                    "- {} × {} [{}; level {}; {} g total; ref {}].\r\n",
                    item.quantity,
                    item.label,
                    item.rarity,
                    item.item_level,
                    item.total_weight_grams,
                    item.id,
                ));
            }
        }
        Ok(ActionOutcome {
            scene: self.scene_for(session)?,
            container_contents: None,
            trade_contents: Some(TradeContentsView {
                trade_id: opaque_id(observer.salt, "trade", &trade.id.0.to_string()),
                container: opaque_id(observer.salt, "item", container_id.as_str()),
                items,
                page,
                total_pages,
                capacity: capacity.clone(),
            }),
            reward_odds: None,
            output: Some(output),
            changed_character: None,
            domain_events: Vec::new(),
        })
    }

    fn trade_basis_sha256(&self, trade: &TradeSnapshot) -> Result<[u8; 32], WorldError> {
        let mut digest = Sha256::new();
        digest.update(b"tiamud.trade.basis.v2\0");
        canonical_u64(&mut digest, trade.id.0);
        canonical_i64(&mut digest, trade.requester.0);
        canonical_i64(&mut digest, trade.recipient.0);
        canonical_str(&mut digest, &trade.room_id)?;
        canonical_u64(&mut digest, trade.state_revision);
        let graph = build_item_graph_index(&self.item_instances, &self.item_prototypes)?;
        for participant in trade.participants() {
            canonical_i64(&mut digest, participant.0);
            let owned_items = self
                .item_instances
                .iter()
                .filter_map(|(item_id, instance)| {
                    item_effective_owner(&self.item_instances, item_id)
                        .map(|owner| (owner == Some(participant)).then_some((item_id, instance)))
                        .transpose()
                })
                .collect::<Result<Vec<_>, WorldError>>()?;
            canonical_u64(
                &mut digest,
                u64::try_from(owned_items.len()).map_err(|_| WorldError::InvalidRuntimeSnapshot)?,
            );
            for (item_id, instance) in owned_items {
                let prototype = self
                    .item_prototypes
                    .get(&instance.prototype_id)
                    .ok_or(WorldError::InvalidRuntimeSnapshot)?;
                canonical_str(&mut digest, item_id.as_str())?;
                canonical_str(&mut digest, &instance.prototype_id)?;
                canonical_item_location(&mut digest, &instance.location)?;
                canonical_item_binding(&mut digest, instance.binding);
                canonical_item_affixes(&mut digest, &instance.item_affixes)?;
                digest.update([match prototype.transfer_policy {
                    ItemTransferPolicy::Transferable => 0,
                    ItemTransferPolicy::CharacterBound => 1,
                    ItemTransferPolicy::NonTransferable => 2,
                }]);
                digest.update([match prototype.stack_policy {
                    ItemStackPolicy::Unique => 0,
                    ItemStackPolicy::Fungible => 1,
                }]);
                canonical_u64(&mut digest, u64::from(prototype.weight_grams));
                canonical_u64(&mut digest, u64::from(prototype.slot_cost));
                match &prototype.container {
                    Some(capacity) => {
                        digest.update([1]);
                        canonical_u64(&mut digest, u64::from(capacity.max_slots));
                        canonical_u64(&mut digest, u64::from(capacity.max_weight_grams));
                    }
                    None => digest.update([0]),
                }
            }
            let offer = trade
                .offers
                .get(&participant)
                .ok_or(WorldError::InvalidRuntimeSnapshot)?;
            canonical_u64(&mut digest, offer.coins);
            canonical_u64(
                &mut digest,
                u64::try_from(offer.lines.len()).map_err(|_| WorldError::InvalidRuntimeSnapshot)?,
            );
            for line in &offer.lines {
                canonical_trade_key(&mut digest, &line.key)?;
                canonical_u64(
                    &mut digest,
                    u64::try_from(line.item_ids.len())
                        .map_err(|_| WorldError::InvalidRuntimeSnapshot)?,
                );
                for root in &line.item_ids {
                    for item_id in graph.subtree_ids(root)? {
                        let instance = self
                            .item_instances
                            .get(&item_id)
                            .ok_or(WorldError::InvalidRuntimeSnapshot)?;
                        let prototype = self
                            .item_prototypes
                            .get(&instance.prototype_id)
                            .ok_or(WorldError::InvalidRuntimeSnapshot)?;
                        canonical_str(&mut digest, item_id.as_str())?;
                        canonical_str(&mut digest, &instance.prototype_id)?;
                        digest.update([prototype.transfer_policy as u8]);
                        canonical_item_binding(&mut digest, instance.binding);
                        canonical_item_affixes(&mut digest, &instance.item_affixes)?;
                        canonical_item_location(&mut digest, &instance.location)?;
                    }
                }
            }
            let gameplay = self
                .character_gameplay
                .get(&participant)
                .ok_or(WorldError::InvalidRuntimeSnapshot)?;
            canonical_u64(&mut digest, gameplay.coins);
            canonical_u64(
                &mut digest,
                u64::from(gameplay.inventory_capacity.max_slots),
            );
            canonical_u64(
                &mut digest,
                u64::from(gameplay.inventory_capacity.max_weight_grams),
            );
            canonical_u64(
                &mut digest,
                u64::try_from(gameplay.equipment.len())
                    .map_err(|_| WorldError::InvalidRuntimeSnapshot)?,
            );
            for (slot, item_id) in &gameplay.equipment {
                digest.update([*slot as u8]);
                canonical_str(&mut digest, item_id.as_str())?;
            }
        }
        Ok(digest.finalize().into())
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
                    | PlayerAction::Contents { .. }
                    | PlayerAction::Loot(_)
                    | PlayerAction::RewardOdds { .. }
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
                    | PlayerAction::Contents { .. }
                    | PlayerAction::Loot(_)
                    | PlayerAction::RewardOdds { .. }
                    | PlayerAction::Weather
                    | PlayerAction::Time
            )
        {
            return Err(WorldError::GameplayUnavailable);
        }
        match action {
            PlayerAction::Look => Ok(ActionOutcome {
                scene: self.scene_for(session)?,
                container_contents: None,
                trade_contents: None,
                reward_odds: None,
                output: None,
                changed_character: None,
                domain_events: Vec::new(),
            }),
            PlayerAction::Inventory => Ok(ActionOutcome {
                scene: self.scene_for(session)?,
                container_contents: None,
                trade_contents: None,
                reward_odds: None,
                output: Some(self.inventory_as_plain_text(session)?),
                changed_character: None,
                domain_events: Vec::new(),
            }),
            PlayerAction::Examine(query) => {
                let item_id =
                    self.resolve_accessible_item(session, &query, ItemScope::Accessible)?;
                let item = self.item_prototype_for(&item_id)?;
                let name = self.item_effective_name(&item_id)?;
                let affix = self.item_affix(&item_id);
                let affix_text = affix.map_or_else(String::new, |affix| {
                    format!(
                        "\r\nAfijo: {} (ataque {:+}, curación {:+}).",
                        affix.label, affix.attack_bonus_delta, affix.heal_amount_delta
                    )
                });
                Ok(ActionOutcome {
                    scene: self.scene_for(session)?,
                    container_contents: None,
                    trade_contents: None,
                    reward_odds: None,
                    output: Some(format!("{name}: {}{affix_text}\r\n", item.description)),
                    changed_character: None,
                    domain_events: Vec::new(),
                })
            }
            PlayerAction::Take(selection) => self.move_item(session, &selection, true),
            PlayerAction::Drop(selection) => self.move_item(session, &selection, false),
            PlayerAction::Contents { container, page } => {
                self.inspect_container(session, &container, page)
            }
            PlayerAction::Put {
                selection,
                container,
            } => self.move_item_to_container(session, &selection, &container, true),
            PlayerAction::Get {
                selection,
                container,
            } => self.move_item_to_container(session, &selection, &container, false),
            PlayerAction::Give {
                selection,
                recipient,
            } => self.give_item(session, &selection, &recipient),
            PlayerAction::Loot(action) => self.apply_loot(session, action),
            PlayerAction::Trade(action) => self.apply_trade(session, action),
            PlayerAction::OpenReward(query) => self.open_reward(session, &query),
            PlayerAction::RewardOdds { target, page } => self.reward_odds(session, &target, page),
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
                container_contents: None,
                trade_contents: None,
                reward_odds: None,
                output: Some(format!(
                    "The weather is {}.\r\n",
                    weather_label(self.adventure.weather)
                )),
                changed_character: None,
                domain_events: Vec::new(),
            }),
            PlayerAction::Time => Ok(ActionOutcome {
                scene: self.scene_for(session)?,
                container_contents: None,
                trade_contents: None,
                reward_odds: None,
                output: Some(format!(
                    "It is hour {} of day {} (world tick {}).\r\n",
                    self.logical_tick % 24,
                    self.logical_tick / 24 + 1,
                    self.logical_tick
                )),
                changed_character: None,
                domain_events: Vec::new(),
            }),
            PlayerAction::Social(_) | PlayerAction::Staff(_) => {
                Err(WorldError::GameplayUnavailable)
            }
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
                let mut events = vec![event];
                let mut released_npcs = Vec::new();
                for npc in self.npcs.values_mut() {
                    if npc.engaged_character == Some(actor_id) {
                        npc.engaged_character = None;
                        released_npcs.push((
                            npc.stable_id.clone(),
                            npc.name.clone(),
                            npc.spawn_room_id.clone(),
                        ));
                    }
                }
                for (npc_id, npc_name, room_id) in released_npcs {
                    self.loot_encounters.remove(&npc_id);
                    events.push(DomainEvent::Gameplay {
                        event: GameplayEvent::LootEncounterCancelled {
                            npc_id,
                            npc_name,
                            room_id,
                        },
                    });
                }
                self.bump_view_revisions_for_events(&events)?;
                self.observer_views
                    .get_mut(&session)
                    .ok_or(WorldError::InvalidEntity)?
                    .discovered_rooms
                    .insert(to_room_id);
                Ok(ActionOutcome {
                    scene: self.scene_for(session)?,
                    container_contents: None,
                    trade_contents: None,
                    reward_odds: None,
                    output: None,
                    changed_character: Some(self.character_snapshot(session)?),
                    domain_events: events,
                })
            }
        }
    }

    pub fn scene_for(&self, session: SessionId) -> Result<SceneView, WorldError> {
        let current_room = room_checked(&self.entities, self.character(session)?.location)?;
        let character_id = self.character(session)?.id;
        let observer = self.observer(session)?;
        let visual_scene = self.can_receive_visual_scene(session)?;
        let item_graph = build_item_graph_index(&self.item_instances, &self.item_prototypes)?;
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
        let visible_item_ids = self
            .item_instances
            .iter()
            .filter_map(|(item_id, instance)| match &instance.location {
                SharedItemLocation::Room(room_id) if room_id == &current_room.stable_id => {
                    Some(item_id)
                }
                SharedItemLocation::Staged
                | SharedItemLocation::Carried(_)
                | SharedItemLocation::Room(_)
                | SharedItemLocation::Equipped(_, _)
                | SharedItemLocation::HeldByNpc(_)
                | SharedItemLocation::Contained(_)
                | SharedItemLocation::Consumed => None,
            })
            .map(|item_id| {
                self.item_visible_to(session, item_id, &current_room.stable_id)
                    .map(|visible| visible.then_some(item_id))
            })
            .collect::<Result<Vec<_>, WorldError>>()?
            .into_iter()
            .flatten()
            .cloned()
            .collect::<Vec<_>>();
        let mut item_buckets = BTreeMap::<(u8, Option<u64>), Vec<ItemInstanceId>>::new();
        for item_id in visible_item_ids {
            let key = match self.scene_loot_access(character_id, &item_id)? {
                None => (0, None),
                Some(SceneLootAccessView {
                    access: LootAccessView::Yours,
                    expires_in_ticks,
                }) => (1, expires_in_ticks),
                Some(SceneLootAccessView {
                    access: LootAccessView::Reserved,
                    expires_in_ticks,
                }) => (2, expires_in_ticks),
            };
            item_buckets.entry(key).or_default().push(item_id);
        }
        let mut visible_groups = Vec::new();
        for ((access, expires_in_ticks), item_ids) in item_buckets {
            let loot = match access {
                0 => None,
                1 => Some(SceneLootAccessView {
                    access: LootAccessView::Yours,
                    expires_in_ticks,
                }),
                2 => Some(SceneLootAccessView {
                    access: LootAccessView::Reserved,
                    expires_in_ticks,
                }),
                _ => return Err(WorldError::InvalidEntity),
            };
            for group in
                derive_stack_groups(&self.item_instances, &self.item_prototypes, item_ids.iter())?
            {
                visible_groups.push((group, loot.clone()));
            }
        }
        let mut items = visible_groups
            .into_iter()
            .map(|(group, loot)| {
                let item = self
                    .item_prototypes
                    .get(&group.prototype_id)
                    .ok_or(WorldError::InvalidEntity)?;
                let selector = group.members.first().ok_or(WorldError::InvalidEntity)?;
                let view_item_id = opaque_id(observer.salt, "item", selector.as_str());
                let quantity =
                    u16::try_from(group.members.len()).map_err(|_| WorldError::InvalidEntity)?;
                let total_weight_grams =
                    group.members.iter().try_fold(0_u64, |total, item_id| {
                        total
                            .checked_add(item_graph.subtree_weight(item_id)?)
                            .ok_or(WorldError::InvalidEntity)
                    })?;
                let mut actions = vec![SceneAction {
                    label: "Examine".into(),
                    command: format!("examine {view_item_id}"),
                }];
                if !matches!(
                    loot.as_ref().map(|loot| loot.access),
                    Some(LootAccessView::Reserved)
                ) {
                    actions.push(SceneAction {
                        label: "Take".into(),
                        command: format!("take {view_item_id}"),
                    });
                }
                if matches!(
                    loot.as_ref().map(|loot| loot.access),
                    Some(LootAccessView::Yours)
                ) {
                    actions.push(SceneAction {
                        label: "Pass".into(),
                        command: format!("loot pass {view_item_id}"),
                    });
                }
                if item.container.is_some()
                    && !matches!(
                        loot.as_ref().map(|loot| loot.access),
                        Some(LootAccessView::Reserved)
                    )
                {
                    actions.push(SceneAction {
                        label: "Contents".into(),
                        command: format!("contents {view_item_id}"),
                    });
                }
                Ok(SceneItem {
                    id: view_item_id.clone(),
                    label: self.item_effective_name(selector)?,
                    description: item.description.clone(),
                    rarity: item.rarity.as_str().into(),
                    item_level: item.item_level,
                    weight_grams: item.weight_grams,
                    quantity,
                    stack_slots: stack_slots(item, group.members.len())?,
                    total_weight_grams,
                    transfer_policy: item.transfer_policy,
                    bound_to_you: false,
                    affix: self.item_affix_view(selector),
                    container_capacity: item
                        .container
                        .as_ref()
                        .map(|_| {
                            container_usage(
                                selector,
                                &self.item_instances,
                                &self.item_prototypes,
                                &item_graph,
                            )
                        })
                        .transpose()?,
                    loot,
                    actions,
                })
            })
            .collect::<Result<Vec<_>, WorldError>>()?;
        items.sort_by(|left, right| {
            left.label
                .cmp(&right.label)
                .then_with(|| left.id.cmp(&right.id))
        });
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
        let visible_ids = self
            .item_instances
            .iter()
            .filter_map(|(stable_id, instance)| {
                matches!(&instance.location, SharedItemLocation::Room(room_id) if room_id == &current_room.stable_id)
                    .then_some(stable_id)
            })
            .map(|item_id| {
                self.item_visible_to(session, item_id, &current_room.stable_id)
                    .map(|visible| visible.then_some(item_id.clone()))
            })
            .collect::<Result<Vec<_>, WorldError>>()?
            .into_iter()
            .flatten()
            .collect::<Vec<_>>();
        let mut items = derive_stack_groups(
            &self.item_instances,
            &self.item_prototypes,
            visible_ids.iter(),
        )?
        .into_iter()
        .map(|group| {
            let first = group
                .members
                .first()
                .expect("derived stack groups are non-empty");
            VisibleSceneEntityId {
                stable_id: first.0.clone(),
                prototype_id: group.prototype_id,
                opaque_id: opaque_id(observer.salt, "item", first.as_str()),
            }
        })
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
                            prototype_id: npc.stable_id.clone(),
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

    fn inventory_usage(
        &self,
        character_id: CharacterId,
    ) -> Result<InventoryCapacityView, WorldError> {
        let capacity = &self
            .character_gameplay
            .get(&character_id)
            .ok_or(WorldError::InvalidEntity)?
            .inventory_capacity;
        let (used_slots, used_weight_grams) = self.inventory_totals(character_id)?;
        Ok(InventoryCapacityView {
            used_slots: u16::try_from(used_slots).map_err(|_| WorldError::InvalidEntity)?,
            max_slots: capacity.max_slots,
            used_weight_grams: u32::try_from(used_weight_grams)
                .map_err(|_| WorldError::InvalidEntity)?,
            max_weight_grams: capacity.max_weight_grams,
        })
    }

    fn inventory_totals(&self, character_id: CharacterId) -> Result<(u32, u64), WorldError> {
        character_inventory_totals(character_id, &self.item_instances, &self.item_prototypes)
    }

    fn inventory_usage_for_capacity(
        &self,
        character_id: CharacterId,
        capacity: &InventoryCapacity,
    ) -> Result<bool, WorldError> {
        let (slots, weight) = self.inventory_totals(character_id)?;
        Ok(
            slots <= u32::from(capacity.max_slots)
                && weight <= u64::from(capacity.max_weight_grams),
        )
    }

    fn validate_prospective_item_locations(
        &self,
        instances: &BTreeMap<ItemInstanceId, ItemInstance>,
        character_ids: impl IntoIterator<Item = CharacterId>,
    ) -> Result<(), WorldError> {
        let graph = build_item_graph_index(instances, &self.item_prototypes)?;
        validate_container_capacities(instances, &self.item_prototypes, &graph)?;
        validate_item_bindings(instances, &self.item_prototypes)?;
        for character_id in character_ids {
            let capacity = &self
                .character_gameplay
                .get(&character_id)
                .ok_or(WorldError::InvalidEntity)?
                .inventory_capacity;
            let (slots, weight) = character_inventory_totals_with_graph(
                character_id,
                instances,
                &self.item_prototypes,
                &graph,
            )?;
            if slots > u32::from(capacity.max_slots)
                || weight > u64::from(capacity.max_weight_grams)
            {
                return Err(WorldError::InventoryCapacityExceeded);
            }
        }
        Ok(())
    }

    fn apply_item_ownership_transitions(
        &self,
        prospective: &mut BTreeMap<ItemInstanceId, ItemInstance>,
        roots: &[ItemInstanceId],
    ) -> Result<(), WorldError> {
        let current_graph = build_item_graph_index(&self.item_instances, &self.item_prototypes)?;
        let mut visited = BTreeSet::new();
        for root in roots {
            let old_owner = item_effective_owner(&self.item_instances, root)?;
            let new_owner = item_effective_owner(prospective, root)?;
            let consumed = matches!(
                prospective.get(root).map(|instance| &instance.location),
                Some(SharedItemLocation::Consumed)
            );
            let subtree = current_graph.subtree_ids(root)?;
            if consumed {
                if old_owner.is_none() || subtree.len() != 1 {
                    return Err(WorldError::ItemTransferForbidden);
                }
                visited.extend(subtree);
                continue;
            }
            match (old_owner, new_owner) {
                (None, Some(character_id)) => {
                    for item_id in subtree {
                        if !visited.insert(item_id.clone()) {
                            continue;
                        }
                        let instance = prospective
                            .get_mut(&item_id)
                            .ok_or(WorldError::InvalidEntity)?;
                        let prototype = self
                            .item_prototypes
                            .get(&instance.prototype_id)
                            .ok_or(WorldError::InvalidEntity)?;
                        if prototype.transfer_policy == ItemTransferPolicy::CharacterBound {
                            match instance.binding {
                                ItemBinding::Unbound => {
                                    instance.binding = ItemBinding::Character { character_id };
                                }
                                ItemBinding::Character {
                                    character_id: bound,
                                } if bound == character_id => {}
                                ItemBinding::Character { .. } => {
                                    return Err(WorldError::ItemTransferForbidden);
                                }
                            }
                        }
                    }
                }
                (Some(previous), Some(next)) if previous != next => {
                    if !current_graph.subtree_is_transferable(root)? {
                        return Err(WorldError::ItemTransferForbidden);
                    }
                    visited.extend(subtree);
                }
                (Some(_), None) => {
                    if !current_graph.subtree_is_transferable(root)? {
                        return Err(WorldError::ItemTransferForbidden);
                    }
                    visited.extend(subtree);
                }
                _ => {
                    visited.extend(subtree);
                }
            }
        }
        validate_item_bindings(prospective, &self.item_prototypes)
            .map_err(|_| WorldError::ItemTransferForbidden)
    }

    fn reconcile_rollback_bindings(
        &self,
        prospective: &mut BTreeMap<ItemInstanceId, ItemInstance>,
        roots: &[ItemInstanceId],
    ) -> Result<(), WorldError> {
        let graph = build_item_graph_index(prospective, &self.item_prototypes)?;
        let mut visited = BTreeSet::new();
        for root in roots {
            for item_id in graph.subtree_ids(root)? {
                if !visited.insert(item_id.clone()) {
                    continue;
                }
                let owner = item_effective_owner(prospective, &item_id)?;
                let instance = prospective
                    .get_mut(&item_id)
                    .ok_or(WorldError::InvalidEntity)?;
                let prototype = self
                    .item_prototypes
                    .get(&instance.prototype_id)
                    .ok_or(WorldError::InvalidEntity)?;
                if prototype.transfer_policy != ItemTransferPolicy::CharacterBound {
                    continue;
                }
                match owner {
                    Some(character_id) => {
                        if instance.binding != (ItemBinding::Character { character_id }) {
                            return Err(WorldError::InvalidDomainTransition);
                        }
                    }
                    None if !matches!(instance.location, SharedItemLocation::Consumed) => {
                        instance.binding = ItemBinding::Unbound;
                    }
                    None => {}
                }
            }
        }
        validate_item_bindings(prospective, &self.item_prototypes)
            .map_err(|_| WorldError::InvalidDomainTransition)
    }

    pub fn inventory_as_plain_text(&self, session: SessionId) -> Result<String, WorldError> {
        let character = self.character(session)?;
        let observer_salt = self.observer(session)?.salt;
        let item_graph = build_item_graph_index(&self.item_instances, &self.item_prototypes)?;
        let carried_ids = self
            .item_instances
            .iter()
            .filter_map(|(item_id, instance)| {
                matches!(&instance.location, SharedItemLocation::Carried(owner) if *owner == character.id)
                    .then_some(item_id)
            })
            .cloned()
            .collect::<Vec<_>>();
        let mut item_rows = derive_stack_groups(
            &self.item_instances,
            &self.item_prototypes,
            carried_ids.iter(),
        )?
        .into_iter()
        .map(|group| {
            let item = self
                .item_prototypes
                .get(&group.prototype_id)
                .ok_or(WorldError::InvalidEntity)?;
            let first = group.members.first().ok_or(WorldError::InvalidEntity)?;
            let total_weight = group.members.iter().try_fold(0_u64, |total, item_id| {
                total
                    .checked_add(item_graph.subtree_weight(item_id)?)
                    .ok_or(WorldError::InvalidEntity)
            })?;
            let container_capacity = item
                .container
                .as_ref()
                .map(|_| {
                    container_usage(
                        first,
                        &self.item_instances,
                        &self.item_prototypes,
                        &item_graph,
                    )
                })
                .transpose()?;
            let affix_details = self.item_affix(first).map_or_else(String::new, |affix| {
                format!(
                    "; affix {} (attack {:+}, healing {:+})",
                    affix.label, affix.attack_bonus_delta, affix.heal_amount_delta
                )
            });
            Ok((
                self.item_effective_name(first)?,
                item.rarity,
                item.item_level,
                item.weight_grams,
                group.members.len(),
                stack_slots(item, group.members.len())?,
                total_weight,
                opaque_id(observer_salt, "item", first.as_str()),
                container_capacity,
                item.transfer_policy,
                matches!(
                    group.binding,
                    ItemBinding::Character {
                        character_id: owner
                    } if owner == character.id
                ),
                affix_details,
            ))
        })
        .collect::<Result<Vec<_>, WorldError>>()?;
        item_rows.sort_unstable_by(|left, right| {
            left.0.cmp(&right.0).then_with(|| left.7.cmp(&right.7))
        });
        let gameplay = self
            .character_gameplay
            .get(&character.id)
            .ok_or(WorldError::InvalidEntity)?;
        let usage = self.inventory_usage(character.id)?;
        let mut output = String::new();
        output.push_str(&format!(
            "Capacity: {}/{} slots; {}/{} g.\r\n",
            usage.used_slots,
            gameplay.inventory_capacity.max_slots,
            usage.used_weight_grams,
            gameplay.inventory_capacity.max_weight_grams
        ));
        if item_rows.is_empty() && gameplay.equipment.is_empty() {
            output.push_str("You are carrying nothing.\r\n");
            return Ok(output);
        }
        if !item_rows.is_empty() {
            output.push_str("You are carrying:\r\n");
            for (
                name,
                rarity,
                level,
                unit_weight,
                quantity,
                slots,
                total_weight,
                reference,
                container_capacity,
                transfer_policy,
                bound_to_you,
                affix_details,
            ) in item_rows
            {
                let stack_slots = if slots == 1 {
                    "stack slot"
                } else {
                    "stack slots"
                };
                let container = container_capacity.map_or_else(String::new, |capacity| {
                    format!(
                        "; container {}/{} slots; {}/{} g",
                        capacity.used_slots,
                        capacity.max_slots,
                        capacity.used_weight_grams,
                        capacity.max_weight_grams,
                    )
                });
                output.push_str(&format!(
                    "- {quantity} × {name} [{}; level {level}; {unit_weight} g each; {slots} {stack_slots}; {total_weight} g total; policy {}{}{affix_details}{container}; ref {reference}]\r\n",
                    rarity.as_str(),
                    transfer_policy.as_str(),
                    if bound_to_you { "; bound to you" } else { "" },
                ));
            }
        }
        if !gameplay.equipment.is_empty() {
            output.push_str("Equipped:\r\n");
            for (slot, item_id) in &gameplay.equipment {
                let item = self.item_prototype_for(item_id)?;
                let total_weight = item_graph.subtree_weight(item_id)?;
                let slots = stack_slots(item, 1)?;
                let stack_slots = if slots == 1 {
                    "stack slot"
                } else {
                    "stack slots"
                };
                let container = item
                    .container
                    .as_ref()
                    .map(|_| {
                        container_usage(
                            item_id,
                            &self.item_instances,
                            &self.item_prototypes,
                            &item_graph,
                        )
                    })
                    .transpose()?
                    .map_or_else(String::new, |capacity| {
                        format!(
                            "; container {}/{} slots; {}/{} g",
                            capacity.used_slots,
                            capacity.max_slots,
                            capacity.used_weight_grams,
                            capacity.max_weight_grams,
                        )
                    });
                let name = self.item_effective_name(item_id)?;
                let affix_details = self.item_affix(item_id).map_or_else(String::new, |affix| {
                    format!(
                        "; affix {} (attack {:+}, healing {:+})",
                        affix.label, affix.attack_bonus_delta, affix.heal_amount_delta
                    )
                });
                output.push_str(&format!(
                    "- {}: 1 × {} [{}; level {}; {} g each; {slots} {stack_slots}; {total_weight} g total; policy {}{}{affix_details}{container}; ref {}]\r\n",
                    slot.as_str(),
                    name,
                    item.rarity.as_str(),
                    item.item_level,
                    item.weight_grams,
                    item.transfer_policy.as_str(),
                    if matches!(
                        self.item_instances
                            .get(item_id)
                            .ok_or(WorldError::InvalidEntity)?
                            .binding,
                        ItemBinding::Character { character_id: owner } if owner == character.id
                    ) {
                        "; bound to you"
                    } else {
                        ""
                    },
                    opaque_id(observer_salt, "item", item_id.as_str())
                ));
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
                    container_contents: None,
                    trade_contents: None,
                    reward_odds: None,
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
                container_contents: None,
                trade_contents: None,
                reward_odds: None,
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
                    container_contents: None,
                    trade_contents: None,
                    reward_odds: None,
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
                    container_contents: None,
                    trade_contents: None,
                    reward_odds: None,
                    output: Some(format!(
                        "You have {coins} coin; {} reserved and {} spendable.\r\n",
                        self.reserved_coins(character_id),
                        self.spendable_coins(character_id)?
                    )),
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
                    container_contents: None,
                    trade_contents: None,
                    reward_odds: None,
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
                if self.spendable_coins(character_id)? < price {
                    return Err(WorldError::InsufficientFunds);
                }
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
        self.party_loot_cursors
            .retain(|party_id, _| self.adventure.parties.contains_key(party_id));
        self.revision = self.revision.wrapping_add(1).max(1);
        self.bump_view_revisions(&event)?;
        Ok(ActionOutcome {
            scene: self.scene_for(session)?,
            container_contents: None,
            trade_contents: None,
            reward_odds: None,
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

    fn freeze_loot_encounter(
        &mut self,
        npc_id: &str,
        character_id: CharacterId,
        room_id: &str,
    ) -> Result<(), WorldError> {
        if self.loot_encounters.contains_key(npc_id) {
            return Ok(());
        }
        if self.loot_encounters.len() >= MAX_LOOT_ENCOUNTERS {
            return Err(WorldError::LootUnavailable);
        }
        let party_id = self.party_for_member(character_id);
        let mut peers = Vec::new();
        if let Some(party_id) = party_id {
            let party = self
                .adventure
                .parties
                .get(&party_id)
                .ok_or(WorldError::PartyUnavailable)?;
            for connected in self.connected_characters()? {
                if connected.character_id != character_id
                    && connected.room_id == room_id
                    && party.members.contains(&connected.character_id)
                    && self
                        .character_gameplay
                        .get(&connected.character_id)
                        .is_some_and(|state| {
                            state.vitals.health > 0 && state.recovery_at_tick.is_none()
                        })
                {
                    peers.push(connected.character_id);
                }
            }
        }
        peers.sort_unstable();
        peers.dedup();
        peers.truncate(5);
        let mut cohort = peers;
        cohort.push(character_id);
        cohort.sort_unstable();
        self.loot_encounters.insert(
            npc_id.to_owned(),
            LootEncounter {
                npc_spawn_id: npc_id.to_owned(),
                initiator: character_id,
                party_id,
                cohort,
                room_id: room_id.to_owned(),
                started_tick: self.logical_tick,
            },
        );
        Ok(())
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
        {
            let npc = self.npcs.get(&npc_id).ok_or(WorldError::NpcNotFound)?;
            if !npc.alive()
                || npc.spawn_room_id != room_id
                || npc
                    .engaged_character
                    .is_some_and(|engaged| engaged != character_id)
            {
                return Err(WorldError::CombatUnavailable);
            }
        }
        self.freeze_loot_encounter(&npc_id, character_id, &room_id)?;
        let npc = self.npcs.get_mut(&npc_id).ok_or(WorldError::NpcNotFound)?;
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
            self.freeze_loot_encounter(npc_id, character_id, &room_id)?;
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
            let mut released_npcs = Vec::new();
            for npc in self.npcs.values_mut() {
                if npc.engaged_character == Some(character_id) {
                    npc.engaged_character = None;
                    released_npcs.push(npc.stable_id.clone());
                }
            }
            for npc_id in released_npcs {
                self.loot_encounters.remove(&npc_id);
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
        self.ensure_items_unreserved(std::iter::once(&item_id))?;
        let definition = self
            .item_gameplay_for(&item_id)
            .cloned()
            .map_err(|_| WorldError::ItemNotUsable)?;
        let heal_amount = self.item_effective_heal_amount(&item_id)?;
        if heal_amount == 0 && definition.effect.is_none() {
            return Err(WorldError::ItemNotUsable);
        }
        let character = self.character(session)?;
        let character_id = character.id;
        let character_name = character.name.clone();
        let room_id = room_checked(&self.entities, character.location)?
            .stable_id
            .clone();
        let item_name = self.item_effective_name(&item_id)?;
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
            .saturating_add(heal_amount.max(0))
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
            let mut prospective = self.item_instances.clone();
            prospective
                .get_mut(&item_id)
                .ok_or(WorldError::InvalidEntity)?
                .location = SharedItemLocation::Consumed;
            self.apply_item_ownership_transitions(
                &mut prospective,
                std::slice::from_ref(&item_id),
            )?;
            validate_item_graph(&prospective, &self.item_prototypes)?;
            self.item_instances = prospective;
        }
        self.finish_gameplay_action(session, format!("You use the {item_name}.\r\n"), events)
    }

    fn equip_item(&mut self, session: SessionId, query: &str) -> Result<ActionOutcome, WorldError> {
        let item_id = self.resolve_accessible_item(session, query, ItemScope::Carried)?;
        self.ensure_items_unreserved(std::iter::once(&item_id))?;
        let definition = self.item_gameplay_for(&item_id)?;
        let slot = definition
            .equipment_slot
            .ok_or(WorldError::ItemNotEquippable)?;
        let character = self.character(session)?;
        let character_id = character.id;
        let character_name = character.name.clone();
        let room_id = room_checked(&self.entities, character.location)?
            .stable_id
            .clone();
        let item_name = self.item_effective_name(&item_id)?;
        let previous_item_id = self
            .character_gameplay
            .get(&character_id)
            .ok_or(WorldError::InvalidEntity)?
            .equipment
            .get(&slot)
            .cloned();
        let mut prospective = self.item_instances.clone();
        prospective
            .get_mut(&item_id)
            .ok_or(WorldError::InvalidEntity)?
            .location = SharedItemLocation::Equipped(character_id, slot);
        if let Some(previous_item_id) = &previous_item_id {
            prospective
                .get_mut(previous_item_id)
                .ok_or(WorldError::InvalidEntity)?
                .location = SharedItemLocation::Carried(character_id);
        }
        self.validate_prospective_item_locations(&prospective, [character_id])?;
        let mut events = Vec::new();
        if let Some(previous_item_id) = &previous_item_id {
            let previous_name = self.item_effective_name(previous_item_id)?;
            events.push(DomainEvent::Gameplay {
                event: GameplayEvent::ItemUnequipped {
                    character_id,
                    character_name: character_name.clone(),
                    item_id: previous_item_id.clone(),
                    item_name: previous_name,
                    slot,
                    room_id: room_id.clone(),
                },
            });
        }
        self.character_gameplay
            .get_mut(&character_id)
            .ok_or(WorldError::InvalidEntity)?
            .equipment
            .insert(slot, item_id.clone());
        self.item_instances = prospective;
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
        let item_id = self
            .character_gameplay
            .get(&character_id)
            .ok_or(WorldError::InvalidEntity)?
            .equipment
            .get(&slot)
            .cloned()
            .ok_or(WorldError::EquipmentSlotEmpty)?;
        let item_name = self.item_effective_name(&item_id)?;
        let mut prospective = self.item_instances.clone();
        prospective
            .get_mut(&item_id)
            .ok_or(WorldError::InvalidEntity)?
            .location = SharedItemLocation::Carried(character_id);
        self.validate_prospective_item_locations(&prospective, [character_id])?;
        self.character_gameplay
            .get_mut(&character_id)
            .ok_or(WorldError::InvalidEntity)?
            .equipment
            .remove(&slot);
        self.item_instances = prospective;
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
            container_contents: None,
            trade_contents: None,
            reward_odds: None,
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
        selection: &ItemSelection,
        take: bool,
    ) -> Result<ActionOutcome, WorldError> {
        let scope = if take {
            ItemScope::CurrentRoom
        } else {
            ItemScope::Carried
        };
        let all_candidates = self.item_candidates(session, scope, None)?;
        let mut candidates = all_candidates.clone();
        if take {
            let character_id = self.character(session)?.id;
            candidates.retain(|item_id| {
                self.can_take_ground_loot(character_id, item_id)
                    .unwrap_or(false)
            });
        }
        let item_ids = match self.resolve_item_selection(session, selection, candidates) {
            Ok(item_ids) => item_ids,
            Err(_)
                if take
                    && self
                        .resolve_item_selection(session, selection, all_candidates)
                        .is_ok() =>
            {
                return Err(WorldError::LootUnavailable);
            }
            Err(error) => return Err(error),
        };
        self.ensure_items_unreserved(&item_ids)?;
        let first_item_id = item_ids.first().ok_or(WorldError::InvalidEntity)?;
        let item = self.item_prototype_for(first_item_id)?;
        let item_name = item.name.clone();
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
        let mut prospective = self.item_instances.clone();
        for item_id in &item_ids {
            prospective
                .get_mut(item_id)
                .ok_or(WorldError::InvalidEntity)?
                .location = destination.clone();
        }
        self.apply_item_ownership_transitions(&mut prospective, &item_ids)?;
        self.validate_prospective_item_locations(&prospective, [actor_id])?;
        self.item_instances = prospective;
        if take {
            for item_id in &item_ids {
                self.ground_loot.remove(item_id);
            }
        }
        self.revision = self.revision.wrapping_add(1).max(1);
        let event = if let [item_id] = item_ids.as_slice() {
            if take {
                DomainEvent::ItemTaken {
                    actor_id,
                    actor_name: actor_name.clone(),
                    item_id: item_id.clone(),
                    item_name: item_name.clone(),
                    room_id: room_id.clone(),
                }
            } else {
                DomainEvent::ItemDropped {
                    actor_id,
                    actor_name: actor_name.clone(),
                    item_id: item_id.clone(),
                    item_name: item_name.clone(),
                    room_id: room_id.clone(),
                }
            }
        } else {
            DomainEvent::ItemsMoved {
                actor_id,
                actor_name,
                item_ids,
                item_name: item_name.clone(),
                container_id: None,
                operation: if take {
                    ContainerMoveOperation::Take
                } else {
                    ContainerMoveOperation::Drop
                },
                room_id,
            }
        };
        let events = vec![event];
        self.bump_view_revisions_for_events(&events)?;
        Ok(ActionOutcome {
            scene: self.scene_for(session)?,
            container_contents: None,
            trade_contents: None,
            reward_odds: None,
            output: Some(match (take, selection.quantity) {
                (true, 1) => format!("You take the {item_name}.\r\n"),
                (false, 1) => format!("You drop the {item_name}.\r\n"),
                (true, quantity) => format!("You take {quantity} × {item_name}.\r\n"),
                (false, quantity) => format!("You drop {quantity} × {item_name}.\r\n"),
            }),
            changed_character: None,
            domain_events: events,
        })
    }

    fn give_item(
        &mut self,
        session: SessionId,
        selection: &ItemSelection,
        recipient_query: &str,
    ) -> Result<ActionOutcome, WorldError> {
        let recipient = self.resolve_give_recipient(session, recipient_query)?;
        let giver = self.character(session)?;
        let giver_id = giver.id;
        let giver_name = giver.name.clone();
        let room_id = room_checked(&self.entities, giver.location)?
            .stable_id
            .clone();
        let candidates = self.item_candidates(session, ItemScope::Carried, None)?;
        let item_ids = self.resolve_item_selection(session, selection, candidates)?;
        self.ensure_items_unreserved(&item_ids)?;
        let first_item_id = item_ids.first().ok_or(WorldError::InvalidEntity)?;
        let item_name = self.item_effective_name(first_item_id)?;
        let mut prospective = self.item_instances.clone();
        for item_id in &item_ids {
            prospective
                .get_mut(item_id)
                .ok_or(WorldError::InvalidEntity)?
                .location = SharedItemLocation::Carried(recipient.character_id);
        }
        self.apply_item_ownership_transitions(&mut prospective, &item_ids)?;
        self.validate_prospective_item_locations(&prospective, [giver_id, recipient.character_id])?;
        self.item_instances = prospective;
        self.revision = self.revision.wrapping_add(1).max(1);
        let event = DomainEvent::ItemsGiven {
            giver_id,
            giver_name,
            recipient_id: recipient.character_id,
            recipient_name: recipient.name.clone(),
            item_ids,
            item_name: item_name.clone(),
            room_id,
        };
        self.bump_view_revisions(&event)?;
        Ok(ActionOutcome {
            scene: self.scene_for(session)?,
            container_contents: None,
            trade_contents: None,
            reward_odds: None,
            output: Some(if selection.quantity == 1 {
                format!("You give the {item_name} to {}.\r\n", recipient.name)
            } else {
                format!(
                    "You give {} × {item_name} to {}.\r\n",
                    selection.quantity, recipient.name
                )
            }),
            changed_character: None,
            domain_events: vec![event],
        })
    }

    fn item_candidates(
        &self,
        session: SessionId,
        scope: ItemScope,
        direct_container: Option<&ItemInstanceId>,
    ) -> Result<Vec<ItemInstanceId>, WorldError> {
        let character = self.character(session)?;
        let room_id = &room_checked(&self.entities, character.location)?.stable_id;
        self.item_instances
            .iter()
            .map(|(item_id, instance)| {
                let accessible = if let Some(container_id) = direct_container {
                    matches!(&instance.location, SharedItemLocation::Contained(parent) if parent == container_id)
                } else {
                    match scope {
                        ItemScope::Carried => {
                            matches!(&instance.location, SharedItemLocation::Carried(owner) if *owner == character.id)
                        }
                        ItemScope::CurrentRoom => {
                            matches!(&instance.location, SharedItemLocation::Room(item_room) if item_room == room_id)
                        }
                        ItemScope::Accessible => {
                            matches!(&instance.location, SharedItemLocation::Carried(owner) if *owner == character.id)
                                || matches!(&instance.location, SharedItemLocation::Room(item_room) if item_room == room_id)
                        }
                    }
                };
                let visible = match &instance.location {
                    SharedItemLocation::Room(item_room) => {
                        self.item_visible_to(session, item_id, item_room)?
                    }
                    SharedItemLocation::Carried(owner) => *owner == character.id,
                    SharedItemLocation::Contained(parent) => direct_container == Some(parent),
                    SharedItemLocation::Staged
                    | SharedItemLocation::Equipped(_, _)
                    | SharedItemLocation::HeldByNpc(_)
                    | SharedItemLocation::Consumed => false,
                };
                Ok((accessible && visible).then_some(item_id.clone()))
            })
            .collect::<Result<Vec<_>, WorldError>>()
            .map(|items| items.into_iter().flatten().collect())
    }

    fn resolve_item_selection(
        &self,
        session: SessionId,
        selection: &ItemSelection,
        mut candidates: Vec<ItemInstanceId>,
    ) -> Result<Vec<ItemInstanceId>, WorldError> {
        if selection.quantity == 0 || selection.quantity > MAX_ITEM_SELECTION_QUANTITY {
            return Err(WorldError::GameplayUnavailable);
        }
        candidates.sort();
        let observer_salt = self.observer(session)?.salt;
        let query = selection.query.trim().to_ascii_lowercase();
        if query.is_empty() {
            return Err(WorldError::ItemNotFound(query));
        }

        if let Some(exact_id) = candidates
            .iter()
            .find(|item_id| opaque_id(observer_salt, "item", item_id.as_str()) == query)
        {
            let instance = self
                .item_instances
                .get(exact_id)
                .ok_or(WorldError::InvalidEntity)?;
            let prototype = self
                .item_prototypes
                .get(&instance.prototype_id)
                .ok_or(WorldError::InvalidEntity)?;
            let eligible = if prototype.stack_policy == ItemStackPolicy::Fungible {
                derive_stack_groups(
                    &self.item_instances,
                    &self.item_prototypes,
                    candidates.iter().filter(|candidate| {
                        self.item_instances
                            .get(*candidate)
                            .is_some_and(|candidate_instance| {
                                candidate_instance.prototype_id == instance.prototype_id
                            })
                    }),
                )?
                .into_iter()
                .find(|group| group.members.contains(exact_id))
                .ok_or_else(|| WorldError::ItemNotFound(query.clone()))?
                .members
            } else {
                vec![exact_id.clone()]
            };
            return take_selected_quantity(eligible, selection.quantity, &query);
        }

        let mut representatives = Vec::new();
        let mut represented_fungible = BTreeSet::new();
        for item_id in &candidates {
            let instance = self
                .item_instances
                .get(item_id)
                .ok_or(WorldError::InvalidEntity)?;
            let prototype = self
                .item_prototypes
                .get(&instance.prototype_id)
                .ok_or(WorldError::InvalidEntity)?;
            if prototype.stack_policy == ItemStackPolicy::Unique
                || represented_fungible.insert(instance.prototype_id.clone())
            {
                representatives.push(item_id);
            }
        }
        let resolved = resolve_item_query(
            &self.item_instances,
            &self.item_prototypes,
            representatives.into_iter(),
            &query,
            observer_salt,
        )?;
        let instance = self
            .item_instances
            .get(&resolved)
            .ok_or(WorldError::InvalidEntity)?;
        let prototype = self
            .item_prototypes
            .get(&instance.prototype_id)
            .ok_or(WorldError::InvalidEntity)?;
        let eligible = if prototype.stack_policy == ItemStackPolicy::Fungible {
            candidates
                .into_iter()
                .filter(|candidate| {
                    self.item_instances
                        .get(candidate)
                        .is_some_and(|candidate_instance| {
                            candidate_instance.prototype_id == instance.prototype_id
                        })
                })
                .collect()
        } else {
            vec![resolved]
        };
        take_selected_quantity(eligible, selection.quantity, &query)
    }

    fn item_accessible_as_container(
        &self,
        session: SessionId,
        item_id: &ItemInstanceId,
    ) -> Result<bool, WorldError> {
        let character = self.character(session)?;
        let room_id = &room_checked(&self.entities, character.location)?.stable_id;
        let mut current = item_id;
        for _ in 0..=MAX_CONTAINER_DEPTH {
            let instance = self
                .item_instances
                .get(current)
                .ok_or(WorldError::InvalidEntity)?;
            match &instance.location {
                SharedItemLocation::Carried(owner) | SharedItemLocation::Equipped(owner, _) => {
                    return Ok(*owner == character.id);
                }
                SharedItemLocation::Room(root_room) => {
                    return Ok(root_room == room_id
                        && self.item_visible_to(session, current, root_room)?);
                }
                SharedItemLocation::Contained(parent) => current = parent,
                SharedItemLocation::Staged
                | SharedItemLocation::HeldByNpc(_)
                | SharedItemLocation::Consumed => return Ok(false),
            }
        }
        Err(WorldError::InvalidEntity)
    }

    fn ground_loot_root_for(
        &self,
        item_id: &ItemInstanceId,
    ) -> Result<Option<ItemInstanceId>, WorldError> {
        let mut current = item_id;
        for _ in 0..=MAX_CONTAINER_DEPTH {
            if self.ground_loot.contains_key(current) {
                return Ok(Some(current.clone()));
            }
            let instance = self
                .item_instances
                .get(current)
                .ok_or(WorldError::InvalidEntity)?;
            match &instance.location {
                SharedItemLocation::Contained(parent) => current = parent,
                _ => return Ok(None),
            }
        }
        Err(WorldError::InvalidEntity)
    }

    fn loot_reservation_for_item(
        &self,
        item_id: &ItemInstanceId,
    ) -> Result<Option<&GroundLootReservation>, WorldError> {
        self.ground_loot_root_for(item_id)
            .map(|root| root.and_then(|root| self.ground_loot.get(&root)))
    }

    fn can_take_ground_loot(
        &self,
        character_id: CharacterId,
        item_id: &ItemInstanceId,
    ) -> Result<bool, WorldError> {
        Ok(self
            .loot_reservation_for_item(item_id)?
            .is_none_or(|reservation| {
                self.logical_tick >= reservation.claim_until_tick
                    || reservation.beneficiary.is_none()
                    || reservation.beneficiary == Some(character_id)
            }))
    }

    fn scene_loot_access(
        &self,
        character_id: CharacterId,
        item_id: &ItemInstanceId,
    ) -> Result<Option<SceneLootAccessView>, WorldError> {
        let Some(reservation) = self.loot_reservation_for_item(item_id)? else {
            return Ok(None);
        };
        if self.logical_tick >= reservation.claim_until_tick || reservation.beneficiary.is_none() {
            return Ok(None);
        }
        Ok(Some(if reservation.beneficiary == Some(character_id) {
            SceneLootAccessView {
                access: LootAccessView::Yours,
                expires_in_ticks: Some(
                    reservation
                        .claim_until_tick
                        .saturating_sub(self.logical_tick),
                ),
            }
        } else {
            SceneLootAccessView {
                access: LootAccessView::Reserved,
                expires_in_ticks: None,
            }
        }))
    }

    fn resolve_accessible_container(
        &self,
        session: SessionId,
        query: &str,
    ) -> Result<ItemInstanceId, WorldError> {
        let character_id = self.character(session)?.id;
        let candidates = self
            .item_instances
            .iter()
            .map(|(item_id, instance)| {
                let is_container = self
                    .item_prototypes
                    .get(&instance.prototype_id)
                    .is_some_and(|prototype| prototype.container.is_some());
                let reserved_descendant =
                    matches!(&instance.location, SharedItemLocation::Contained(_))
                        && !self.can_take_ground_loot(character_id, item_id)?;
                self.item_accessible_as_container(session, item_id)
                    .map(|accessible| {
                        (is_container && accessible && !reserved_descendant).then_some(item_id)
                    })
            })
            .collect::<Result<Vec<_>, WorldError>>()?;
        resolve_item_query(
            &self.item_instances,
            &self.item_prototypes,
            candidates.into_iter().flatten(),
            query,
            self.observer(session)?.salt,
        )
    }

    fn inspect_container(
        &self,
        session: SessionId,
        query: &str,
        page: u16,
    ) -> Result<ActionOutcome, WorldError> {
        if page == 0 {
            return Err(WorldError::GameplayUnavailable);
        }
        let container_id = self.resolve_accessible_container(session, query)?;
        let character_id = self.character(session)?.id;
        if !self.can_take_ground_loot(character_id, &container_id)? {
            return Err(WorldError::LootUnavailable);
        }
        let observer = self.observer(session)?;
        let item_graph = build_item_graph_index(&self.item_instances, &self.item_prototypes)?;
        let reserved_item_ids = reserved_item_ids_with_graph(&self.trades, &item_graph)?;
        let container_reserved = reserved_item_ids.contains(&container_id);
        let selector = opaque_id(observer.salt, "item", container_id.as_str());
        let direct_ids =
            self.item_candidates(session, ItemScope::Accessible, Some(&container_id))?;
        let groups = derive_stack_groups(
            &self.item_instances,
            &self.item_prototypes,
            direct_ids.iter(),
        )?;
        let total_pages = groups.len().max(1).div_ceil(CONTAINER_PAGE_SIZE);
        let page_index = usize::from(page.saturating_sub(1));
        if page_index >= total_pages {
            return Err(WorldError::GameplayUnavailable);
        }
        let start = page_index
            .checked_mul(CONTAINER_PAGE_SIZE)
            .ok_or(WorldError::GameplayUnavailable)?;
        let items = groups
            .into_iter()
            .skip(start)
            .take(CONTAINER_PAGE_SIZE)
            .map(|group| {
                let prototype = self
                    .item_prototypes
                    .get(&group.prototype_id)
                    .ok_or(WorldError::InvalidEntity)?;
                let first = group.members.first().ok_or(WorldError::InvalidEntity)?;
                let item_selector = opaque_id(observer.salt, "item", first.as_str());
                let quantity =
                    u16::try_from(group.members.len()).map_err(|_| WorldError::InvalidEntity)?;
                let reserved_for_trade = container_reserved
                    || group
                        .members
                        .iter()
                        .any(|item_id| reserved_item_ids.contains(item_id));
                let total_weight_grams =
                    group.members.iter().try_fold(0_u64, |total, item_id| {
                        total
                            .checked_add(item_graph.subtree_weight(item_id)?)
                            .ok_or(WorldError::InvalidEntity)
                    })?;
                let mut actions = vec![SceneAction {
                    label: "Examine".into(),
                    command: format!("examine {item_selector}"),
                }];
                if !reserved_for_trade {
                    actions.push(SceneAction {
                        label: "Get".into(),
                        command: format!("get {item_selector} from {selector}"),
                    });
                }
                Ok(PlayerItemView {
                    id: item_selector.clone(),
                    label: self.item_effective_name(first)?,
                    description: prototype.description.clone(),
                    rarity: prototype.rarity.as_str().into(),
                    item_level: prototype.item_level,
                    weight_grams: prototype.weight_grams,
                    quantity,
                    available_quantity: if reserved_for_trade { 0 } else { quantity },
                    reserved_for_trade: if reserved_for_trade { quantity } else { 0 },
                    stack_slots: stack_slots(prototype, group.members.len())?,
                    total_weight_grams,
                    transfer_policy: prototype.transfer_policy,
                    bound_to_you: matches!(
                        group.binding,
                        ItemBinding::Character {
                            character_id: owner
                        } if owner == character_id
                    ),
                    can_give: false,
                    can_trade: false,
                    affix: self.item_affix_view(first),
                    container_capacity: prototype
                        .container
                        .as_ref()
                        .map(|_| {
                            container_usage(
                                first,
                                &self.item_instances,
                                &self.item_prototypes,
                                &item_graph,
                            )
                        })
                        .transpose()?,
                    actions,
                })
            })
            .collect::<Result<Vec<_>, WorldError>>()?;
        let capacity = container_usage(
            &container_id,
            &self.item_instances,
            &self.item_prototypes,
            &item_graph,
        )?;
        let total_pages = u16::try_from(total_pages).map_err(|_| WorldError::InvalidEntity)?;
        let mut output = format!(
            "Container capacity: {}/{} slots; {}/{} g. Page {page}/{total_pages}.\r\n",
            capacity.used_slots,
            capacity.max_slots,
            capacity.used_weight_grams,
            capacity.max_weight_grams,
        );
        if items.is_empty() {
            output.push_str("It is empty.\r\n");
        } else {
            for item in &items {
                let stack_slots = if item.stack_slots == 1 {
                    "stack slot"
                } else {
                    "stack slots"
                };
                let container =
                    item.container_capacity
                        .as_ref()
                        .map_or_else(String::new, |capacity| {
                            format!(
                                "; container {}/{} slots; {}/{} g",
                                capacity.used_slots,
                                capacity.max_slots,
                                capacity.used_weight_grams,
                                capacity.max_weight_grams,
                            )
                        });
                let reservation = if item.reserved_for_trade == 0 {
                    String::new()
                } else {
                    format!(
                        "; {} reserved for trade; {} available",
                        item.reserved_for_trade, item.available_quantity
                    )
                };
                output.push_str(&format!(
                    "- {} × {} [{}; level {}; {} g each; {} {stack_slots}; {} g total; policy {}{}{}{container}; ref {}]\r\n",
                    item.quantity,
                    item.label,
                    item.rarity,
                    item.item_level,
                    item.weight_grams,
                    item.stack_slots,
                    item.total_weight_grams,
                    item.transfer_policy.as_str(),
                    if item.bound_to_you { "; bound to you" } else { "" },
                    reservation,
                    item.id,
                ));
            }
        }
        Ok(ActionOutcome {
            scene: self.scene_for(session)?,
            container_contents: Some(ContainerContentsView {
                selector,
                items,
                page,
                total_pages,
                capacity,
            }),
            trade_contents: None,
            reward_odds: None,
            output: Some(output),
            changed_character: None,
            domain_events: Vec::new(),
        })
    }

    fn move_item_to_container(
        &mut self,
        session: SessionId,
        selection: &ItemSelection,
        container_query: &str,
        put: bool,
    ) -> Result<ActionOutcome, WorldError> {
        let container_id = self.resolve_accessible_container(session, container_query)?;
        self.ensure_items_unreserved(std::iter::once(&container_id))?;
        let actor_id = self.character(session)?.id;
        if !self.can_take_ground_loot(actor_id, &container_id)? {
            return Err(WorldError::LootUnavailable);
        }
        let candidates = if put {
            self.item_candidates(session, ItemScope::Carried, None)?
        } else {
            self.item_candidates(session, ItemScope::Accessible, Some(&container_id))?
        };
        let item_ids = self.resolve_item_selection(session, selection, candidates)?;
        self.ensure_items_unreserved(&item_ids)?;
        if !put
            && item_ids.iter().any(|item_id| {
                !self
                    .can_take_ground_loot(actor_id, item_id)
                    .unwrap_or(false)
            })
        {
            return Err(WorldError::LootUnavailable);
        }
        let first_id = item_ids.first().ok_or(WorldError::InvalidEntity)?;
        let item_name = self.item_effective_name(first_id)?;
        let character = self.character(session)?;
        let actor_id = character.id;
        let actor_name = character.name.clone();
        let room_id = room_checked(&self.entities, character.location)?
            .stable_id
            .clone();
        let destination = if put {
            SharedItemLocation::Contained(container_id.clone())
        } else {
            SharedItemLocation::Carried(actor_id)
        };
        let mut prospective = self.item_instances.clone();
        for item_id in &item_ids {
            prospective
                .get_mut(item_id)
                .ok_or(WorldError::InvalidEntity)?
                .location = destination.clone();
        }
        self.apply_item_ownership_transitions(&mut prospective, &item_ids)?;
        self.validate_prospective_item_locations(&prospective, [actor_id])?;
        self.item_instances = prospective;
        self.revision = self.revision.wrapping_add(1).max(1);
        let event = DomainEvent::ItemsMoved {
            actor_id,
            actor_name,
            item_ids,
            item_name: item_name.clone(),
            container_id: Some(container_id),
            operation: if put {
                ContainerMoveOperation::Put
            } else {
                ContainerMoveOperation::Get
            },
            room_id,
        };
        self.bump_view_revisions(&event)?;
        Ok(ActionOutcome {
            scene: self.scene_for(session)?,
            container_contents: None,
            trade_contents: None,
            reward_odds: None,
            output: Some(if put {
                format!(
                    "You put {} × {item_name} in the container.\r\n",
                    selection.quantity
                )
            } else {
                format!(
                    "You get {} × {item_name} from the container.\r\n",
                    selection.quantity
                )
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
    ) -> Result<ItemInstanceId, WorldError> {
        let character = self.character(session)?;
        let room_id = room_checked(&self.entities, character.location)?
            .stable_id
            .as_str();
        let candidates = self
            .item_instances
            .iter()
            .map(|(item_id, instance)| {
                let location = &instance.location;
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
                            || (matches!(location, SharedItemLocation::Contained(_))
                                && self.item_accessible_as_container(session, item_id)?
                                && self.can_take_ground_loot(character.id, item_id)?)
                    }
                };
                let visible = match location {
                    SharedItemLocation::Room(item_room) => self
                        .item_visible_to(session, item_id, item_room)?,
                    SharedItemLocation::Carried(owner) => *owner == character.id,
                    SharedItemLocation::Contained(_) => {
                        scope == ItemScope::Accessible
                            && self.item_accessible_as_container(session, item_id)?
                            && self.can_take_ground_loot(character.id, item_id)?
                    }
                    SharedItemLocation::Staged
                    | SharedItemLocation::Equipped(_, _)
                    | SharedItemLocation::HeldByNpc(_)
                    | SharedItemLocation::Consumed => false,
                };
                Ok((accessible && visible).then_some(item_id))
            })
            .collect::<Result<Vec<_>, WorldError>>()?;
        resolve_item_query(
            &self.item_instances,
            &self.item_prototypes,
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
        item_id: &ItemInstanceId,
        room_id: &str,
    ) -> Result<bool, WorldError> {
        if !self.item_instances.contains_key(item_id) {
            return Err(WorldError::InvalidEntity);
        }
        self.can_detect(
            session,
            room_id,
            self.hidden_items
                .get(item_id.as_str())
                .copied()
                .unwrap_or(0),
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

    /// Resolve a connected, co-located give recipient using only the acting
    /// session's opaque character selectors or one exact, unambiguous name.
    /// The returned durable IDs are server-internal and must not be serialized.
    pub fn resolve_give_recipient(
        &self,
        session: SessionId,
        query: &str,
    ) -> Result<ConnectedCharacter, WorldError> {
        let giver = self.character(session)?;
        let giver_room = room_checked(&self.entities, giver.location)?
            .stable_id
            .as_str();
        let salt = self.observer(session)?.salt;
        let query = query.trim();
        if query.is_empty() {
            return Err(WorldError::GiveTargetUnavailable);
        }
        let mut matches = self
            .connected_characters()?
            .into_iter()
            .filter(|candidate| {
                candidate.name.eq_ignore_ascii_case(query)
                    || opaque_id(salt, "character", &candidate.character_id.0.to_string()) == query
            })
            .collect::<Vec<_>>();
        if matches.len() != 1 {
            return Err(WorldError::GiveTargetUnavailable);
        }
        let recipient = matches.pop().expect("exactly one recipient");
        if recipient.character_id == giver.id || recipient.room_id != giver_room {
            return Err(WorldError::GiveTargetUnavailable);
        }
        Ok(recipient)
    }

    pub fn resolve_trade_recipient(
        &self,
        session: SessionId,
        query: &str,
    ) -> Result<ConnectedCharacter, WorldError> {
        self.resolve_give_recipient(session, query)
            .map_err(|_| WorldError::TradeTargetUnavailable)
    }

    pub fn trade_for_character(&self, character_id: CharacterId) -> Option<&TradeSnapshot> {
        self.trades
            .values()
            .find(|trade| trade.includes(character_id))
    }

    pub fn trade_participants(&self) -> BTreeSet<CharacterId> {
        self.trades
            .values()
            .flat_map(|trade| trade.participants())
            .collect()
    }

    pub fn reserved_item_ids(&self) -> Result<BTreeSet<ItemInstanceId>, WorldError> {
        let graph = build_item_graph_index(&self.item_instances, &self.item_prototypes)?;
        reserved_item_ids_with_graph(&self.trades, &graph)
    }

    pub fn item_reserved_for_trade(&self, item_id: &ItemInstanceId) -> Result<bool, WorldError> {
        Ok(self.reserved_item_ids()?.contains(item_id))
    }

    fn ensure_items_unreserved<'a>(
        &self,
        item_ids: impl IntoIterator<Item = &'a ItemInstanceId>,
    ) -> Result<(), WorldError> {
        let reserved = self.reserved_item_ids()?;
        if item_ids
            .into_iter()
            .any(|item_id| reserved.contains(item_id))
        {
            Err(WorldError::ItemReservedForTrade)
        } else {
            Ok(())
        }
    }

    pub fn reserved_coins(&self, character_id: CharacterId) -> u64 {
        self.trade_for_character(character_id)
            .and_then(|trade| trade.offers.get(&character_id))
            .map_or(0, |offer| offer.coins)
    }

    pub fn spendable_coins(&self, character_id: CharacterId) -> Result<u64, WorldError> {
        self.character_gameplay
            .get(&character_id)
            .ok_or(WorldError::InvalidEntity)?
            .coins
            .checked_sub(self.reserved_coins(character_id))
            .ok_or(WorldError::InvalidRuntimeSnapshot)
    }

    pub fn cancel_trade_for_character(
        &mut self,
        character_id: CharacterId,
        reason: TradeCancellationReason,
    ) -> Result<Vec<DomainEvent>, WorldError> {
        let Some(trade) = self.trade_for_character(character_id).cloned() else {
            return Ok(Vec::new());
        };
        self.trades.remove(&trade.id);
        self.revision = self
            .revision
            .checked_add(1)
            .ok_or(WorldError::InvalidDomainTransition)?;
        Ok(vec![DomainEvent::TradeCancelled {
            previous_trade: trade,
            reason,
        }])
    }

    /// Removes every trade whose logical deadline has been reached. The actor
    /// calls this before timer/combat/script hooks so an expired reservation
    /// can never survive into another tick's mutations. This deliberately does
    /// not run basis invalidation; the post-hook reconciliation owns that
    /// separate comparison.
    pub fn cancel_expired_trades(&mut self) -> Result<Vec<DomainEvent>, WorldError> {
        let expired = self
            .trades
            .iter()
            .filter(|(_, trade)| self.logical_tick >= trade.expires_at_tick)
            .map(|(trade_id, trade)| (*trade_id, trade.clone()))
            .collect::<Vec<_>>();
        if expired.is_empty() {
            return Ok(Vec::new());
        }
        let revision = self
            .revision
            .checked_add(1)
            .ok_or(WorldError::InvalidDomainTransition)?;
        for (trade_id, _) in &expired {
            self.trades.remove(trade_id);
        }
        self.revision = revision;
        Ok(expired
            .into_iter()
            .map(|(_, previous_trade)| DomainEvent::TradeCancelled {
                previous_trade,
                reason: TradeCancellationReason::Expired,
            })
            .collect())
    }

    pub fn loot_identity_ids(&self) -> BTreeSet<CharacterId> {
        self.loot_encounters
            .values()
            .flat_map(|encounter| encounter.cohort.iter().copied())
            .chain(
                self.ground_loot
                    .values()
                    .flat_map(|reservation| reservation.cohort.iter().copied()),
            )
            .collect()
    }

    pub fn cancel_all_loot_encounters_for_restart(
        &mut self,
    ) -> Result<Vec<DomainEvent>, WorldError> {
        let encounters = std::mem::take(&mut self.loot_encounters);
        let mut events = Vec::with_capacity(encounters.len());
        for (npc_id, _) in encounters {
            let npc = self
                .npcs
                .get_mut(&npc_id)
                .ok_or(WorldError::InvalidRuntimeSnapshot)?;
            npc.engaged_character = None;
            npc.statuses.clear();
            npc.ability_cooldowns.clear();
            events.push(DomainEvent::Gameplay {
                event: GameplayEvent::LootEncounterCancelled {
                    npc_id,
                    npc_name: npc.name.clone(),
                    room_id: npc.spawn_room_id.clone(),
                },
            });
        }
        if !events.is_empty() {
            self.revision = self
                .revision
                .checked_add(1)
                .ok_or(WorldError::InvalidDomainTransition)?;
        }
        Ok(events)
    }

    pub fn expire_ground_loot(&mut self) -> Result<Vec<DomainEvent>, WorldError> {
        let tick = self.logical_tick;
        let mut events = Vec::new();
        let roots = self.ground_loot.keys().cloned().collect::<Vec<_>>();
        for root in roots {
            let reservation = self
                .ground_loot
                .get(&root)
                .cloned()
                .ok_or(WorldError::InvalidRuntimeSnapshot)?;
            if tick >= reservation.despawn_at_tick {
                let graph = build_item_graph_index(&self.item_instances, &self.item_prototypes)?;
                let subtree = graph.subtree_ids(&root)?;
                let mut preserved = BTreeSet::new();
                for item_id in &subtree {
                    // Legacy v1-v5 snapshots deliberately label every
                    // restored unit as LegacySnapshot because those DTOs did
                    // not carry provenance. Identity still tells us whether a
                    // unit belongs to the current WorldPack. Preserve/stage
                    // those authored identities as tombstones too, otherwise
                    // omitting them from the next snapshot would make restore
                    // recreate fixed NPC loot at its authored location.
                    if self.initial_item_prototypes.contains_key(item_id)
                        || matches!(
                            self.item_provenance.get(item_id),
                            Some(ItemProvenance::AuthoredInitial)
                        )
                    {
                        preserved.extend(graph.subtree_ids(item_id)?);
                    }
                }
                let removed = subtree
                    .iter()
                    .filter(|item_id| !preserved.contains(*item_id))
                    .cloned()
                    .collect::<Vec<_>>();
                let reserved = self.reserved_item_ids()?;
                if removed.iter().any(|item_id| reserved.contains(item_id)) {
                    return Err(WorldError::InvalidRuntimeSnapshot);
                }
                let detached = preserved
                    .iter()
                    .filter(|item_id| {
                        *item_id == &root
                            || self.item_instances.get(*item_id).is_some_and(|instance| {
                                matches!(
                                    &instance.location,
                                    SharedItemLocation::Contained(parent) if !preserved.contains(parent)
                                )
                            })
                    })
                    .cloned()
                    .collect::<Vec<_>>();
                for item_id in detached {
                    self.set_item_location(&item_id, SharedItemLocation::Staged)?;
                }
                for item_id in &removed {
                    self.item_instances.remove(item_id);
                    self.item_provenance.remove(item_id);
                }
                self.ground_loot.remove(&root);
                events.push(DomainEvent::Gameplay {
                    event: GameplayEvent::LootDespawned {
                        item_ids: subtree,
                        room_id: reservation.room_id,
                    },
                });
            } else if tick >= reservation.claim_until_tick && reservation.beneficiary.is_some() {
                self.ground_loot
                    .get_mut(&root)
                    .ok_or(WorldError::InvalidRuntimeSnapshot)?
                    .beneficiary = None;
                events.push(DomainEvent::Gameplay {
                    event: GameplayEvent::LootRightsChanged {
                        item_id: root,
                        room_id: reservation.room_id,
                        summary: "The loot is now public.".into(),
                    },
                });
            }
        }
        self.party_loot_cursors
            .retain(|party_id, _| self.adventure.parties.contains_key(party_id));
        Ok(events)
    }

    pub fn cancel_trades(
        &mut self,
        trade_ids: &BTreeSet<TradeId>,
        reason: TradeCancellationReason,
    ) -> Result<Vec<DomainEvent>, WorldError> {
        let mut events = Vec::new();
        for trade_id in trade_ids {
            if let Some(previous_trade) = self.trades.remove(trade_id) {
                events.push(DomainEvent::TradeCancelled {
                    previous_trade,
                    reason,
                });
            }
        }
        if !events.is_empty() {
            self.revision = self
                .revision
                .checked_add(1)
                .ok_or(WorldError::InvalidDomainTransition)?;
        }
        Ok(events)
    }

    pub fn cancel_all_trades(
        &mut self,
        reason: TradeCancellationReason,
    ) -> Result<Vec<DomainEvent>, WorldError> {
        let trades = std::mem::take(&mut self.trades);
        if trades.is_empty() {
            return Ok(Vec::new());
        }
        self.revision = self
            .revision
            .checked_add(1)
            .ok_or(WorldError::InvalidDomainTransition)?;
        Ok(trades
            .into_values()
            .map(|previous_trade| DomainEvent::TradeCancelled {
                previous_trade,
                reason,
            })
            .collect())
    }

    pub fn validate_trade_participant_gameplay(
        &self,
        participants: &BTreeMap<CharacterId, CharacterGameplaySnapshot>,
    ) -> Result<(), WorldError> {
        for character_id in self.trade_participants() {
            let gameplay = participants
                .get(&character_id)
                .or_else(|| self.character_gameplay.get(&character_id))
                .ok_or(WorldError::InvalidRuntimeSnapshot)?;
            if gameplay.coins < self.reserved_coins(character_id) {
                return Err(WorldError::InvalidRuntimeSnapshot);
            }
            self.validate_character_gameplay_snapshot(character_id, gameplay)
                .map_err(|_| WorldError::InvalidRuntimeSnapshot)?;
        }
        Ok(())
    }

    pub fn reconcile_trades(
        &mut self,
        before: &WorldCheckpoint,
    ) -> Result<Vec<DomainEvent>, WorldError> {
        let mut events = Vec::new();
        let mut cancelled = BTreeSet::new();
        let mut invalidated = false;
        let graph = build_item_graph_index(&self.item_instances, &self.item_prototypes)?;
        let before_items_by_owner = items_by_effective_owner(&before.item_instances)?;
        let current_items_by_owner = items_by_effective_owner(&self.item_instances)?;
        let trade_ids = self.trades.keys().copied().collect::<Vec<_>>();
        for trade_id in trade_ids {
            let Some(trade) = self.trades.get(&trade_id).cloned() else {
                continue;
            };
            let requester_session = self.session_for_character(trade.requester);
            let recipient_session = self.session_for_character(trade.recipient);
            let disconnected = requester_session.is_none() || recipient_session.is_none();
            let room_for = |world: &Self, character_id| {
                world
                    .session_for_character(character_id)
                    .and_then(|session| world.character(session).ok())
                    .and_then(|character| room_checked(&world.entities, character.location).ok())
                    .map(|room| room.stable_id.clone())
            };
            let separated = !disconnected
                && (room_for(self, trade.requester) != Some(trade.room_id.clone())
                    || room_for(self, trade.recipient) != Some(trade.room_id.clone()));
            let defeated = trade.participants().into_iter().any(|participant| {
                self.character_gameplay
                    .get(&participant)
                    .is_some_and(|gameplay| gameplay.recovery_at_tick.is_some())
            });
            let combat = self.npcs.values().any(|npc| {
                npc.engaged_character
                    .is_some_and(|participant| trade.includes(participant))
            });
            let reservation_valid = validate_trade_state_with_graph(
                self.next_trade_id,
                &BTreeMap::from([(trade.id, trade.clone())]),
                &self.item_instances,
                &self.item_prototypes,
                &self.stable_rooms,
                &graph,
            )
            .is_ok()
                && trade.participants().into_iter().all(|participant| {
                    self.character_gameplay
                        .get(&participant)
                        .is_some_and(|gameplay| gameplay.coins >= self.reserved_coins(participant))
                });
            let reason = if self.logical_tick >= trade.expires_at_tick {
                Some(TradeCancellationReason::Expired)
            } else if disconnected {
                Some(TradeCancellationReason::ParticipantDisconnected)
            } else if separated {
                Some(TradeCancellationReason::ParticipantsSeparated)
            } else if defeated {
                Some(TradeCancellationReason::ParticipantDefeated)
            } else if combat {
                Some(TradeCancellationReason::CombatStarted)
            } else if !reservation_valid {
                Some(TradeCancellationReason::InvalidReservation)
            } else {
                None
            };
            if let Some(reason) = reason {
                self.trades.remove(&trade.id);
                events.push(DomainEvent::TradeCancelled {
                    previous_trade: trade,
                    reason,
                });
                cancelled.insert(trade_id);
                continue;
            }

            // A trade command publishes its own single state revision. For any
            // other mutation, compare the material authorities retained in the
            // checkpoint and invalidate confirmations exactly once.
            if before.trades.get(&trade_id) == Some(&trade) {
                let participant_gameplay_changed = trade.participants().into_iter().any(|id| {
                    before.character_gameplay.get(&id) != self.character_gameplay.get(&id)
                });
                let participant_location_changed = trade.participants().into_iter().any(|id| {
                    character_room_id(&before.entities, id) != character_room_id(&self.entities, id)
                });
                if participant_gameplay_changed
                    || participant_location_changed
                    || trade.participants().into_iter().any(|participant| {
                        before_items_by_owner.get(&participant)
                            != current_items_by_owner.get(&participant)
                    })
                {
                    let previous = trade.clone();
                    let mut current = trade;
                    current.state_revision = current
                        .state_revision
                        .checked_add(1)
                        .ok_or(WorldError::InvalidDomainTransition)?;
                    current.confirmations.clear();
                    self.trades.insert(trade_id, current.clone());
                    events.push(DomainEvent::TradeInvalidated { previous, current });
                    invalidated = true;
                }
            }
        }
        if !cancelled.is_empty() || invalidated {
            self.revision = self
                .revision
                .checked_add(1)
                .ok_or(WorldError::InvalidDomainTransition)?;
        }
        Ok(events)
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
        let item_graph = build_item_graph_index(&self.item_instances, &self.item_prototypes)?;

        let carried_item_ids = self
            .item_instances
            .iter()
            .filter_map(|(item_id, instance)| {
                matches!(&instance.location, SharedItemLocation::Carried(owner) if *owner == character.id)
                    .then_some(item_id)
            })
            .cloned()
            .collect::<Vec<_>>();
        let reserved_items = self.reserved_item_ids()?;
        let groups = derive_stack_groups(
            &self.item_instances,
            &self.item_prototypes,
            carried_item_ids.iter(),
        )?;
        let mut split_groups = Vec::new();
        for group in groups {
            let (reserved, available): (Vec<_>, Vec<_>) = group
                .members
                .into_iter()
                .partition(|item_id| reserved_items.contains(item_id));
            if !available.is_empty() {
                split_groups.push((
                    ItemStackGroup {
                        prototype_id: group.prototype_id.clone(),
                        binding: group.binding,
                        members: available,
                    },
                    false,
                ));
            }
            if !reserved.is_empty() {
                split_groups.push((
                    ItemStackGroup {
                        prototype_id: group.prototype_id,
                        binding: group.binding,
                        members: reserved,
                    },
                    true,
                ));
            }
        }
        let mut inventory = split_groups
            .into_iter()
            .map(|(group, reserved_group)| {
                let item = self
                    .item_prototypes
                    .get(&group.prototype_id)
                    .ok_or(WorldError::InvalidEntity)?;
                let selector = group.members.first().ok_or(WorldError::InvalidEntity)?;
                let id = opaque_id(observer.salt, "item", selector.as_str());
                let mut actions = vec![SceneAction {
                    label: "Examine".into(),
                    command: format!("examine {id}"),
                }];
                let transferable = group
                    .members
                    .iter()
                    .map(|item_id| item_graph.subtree_is_transferable(item_id))
                    .collect::<Result<Vec<_>, WorldError>>()?
                    .into_iter()
                    .all(|allowed| allowed);
                let (available_quantity, reserved_for_trade) = if reserved_group {
                    (0, group.members.len())
                } else {
                    (group.members.len(), 0)
                };
                if transferable && !reserved_group {
                    actions.push(SceneAction {
                        label: "Drop".into(),
                        command: format!("drop {id}"),
                    });
                }
                {
                    if !reserved_group
                        && (self.item_effective_heal_amount(selector)? != 0
                            || item.effect.is_some())
                    {
                        actions.push(SceneAction {
                            label: "Use".into(),
                            command: format!("use {id}"),
                        });
                    }
                    if !reserved_group && item.equipment_slot.is_some() {
                        actions.push(SceneAction {
                            label: "Equip".into(),
                            command: format!("equip {id}"),
                        });
                    }
                    if !reserved_group && self.item_is_openable_reward(selector, character.id) {
                        actions.push(SceneAction {
                            label: "Abrir".into(),
                            command: format!("open {id}"),
                        });
                        actions.push(SceneAction {
                            label: "Probabilidades".into(),
                            command: format!("reward odds {id} 1"),
                        });
                    }
                }
                Ok(PlayerItemView {
                    id,
                    label: self.item_effective_name(selector)?,
                    description: item.description.clone(),
                    rarity: item.rarity.as_str().into(),
                    item_level: item.item_level,
                    weight_grams: item.weight_grams,
                    quantity: u16::try_from(group.members.len())
                        .map_err(|_| WorldError::InvalidEntity)?,
                    available_quantity: u16::try_from(available_quantity)
                        .map_err(|_| WorldError::InvalidEntity)?,
                    reserved_for_trade: u16::try_from(reserved_for_trade)
                        .map_err(|_| WorldError::InvalidEntity)?,
                    stack_slots: stack_slots(item, group.members.len())?,
                    total_weight_grams: group.members.iter().try_fold(
                        0_u64,
                        |total, item_id| {
                            total
                                .checked_add(item_graph.subtree_weight(item_id)?)
                                .ok_or(WorldError::InvalidEntity)
                        },
                    )?,
                    transfer_policy: item.transfer_policy,
                    bound_to_you: matches!(
                        group.binding,
                        ItemBinding::Character {
                            character_id: owner
                        } if owner == character.id
                    ),
                    can_give: transferable && reserved_for_trade == 0,
                    can_trade: transferable && !reserved_group,
                    affix: self.item_affix_view(selector),
                    container_capacity: item
                        .container
                        .as_ref()
                        .map(|_| {
                            container_usage(
                                selector,
                                &self.item_instances,
                                &self.item_prototypes,
                                &item_graph,
                            )
                        })
                        .transpose()?,
                    actions,
                })
            })
            .collect::<Result<Vec<_>, WorldError>>()?;
        inventory.sort_by(|left, right| left.label.cmp(&right.label));

        let equipment = gameplay
            .equipment
            .iter()
            .map(|(slot, item_id)| {
                let item = self.item_prototype_for(item_id)?;
                Ok(EquippedItemView {
                    id: opaque_id(observer.salt, "item", item_id.as_str()),
                    slot: slot.as_str().into(),
                    label: self.item_effective_name(item_id)?,
                    rarity: item.rarity.as_str().into(),
                    item_level: item.item_level,
                    weight_grams: item.weight_grams,
                    quantity: 1,
                    stack_slots: stack_slots(item, 1)?,
                    total_weight_grams: item_graph.subtree_weight(item_id)?,
                    transfer_policy: item.transfer_policy,
                    bound_to_you: matches!(
                        self.item_instances
                            .get(item_id)
                            .ok_or(WorldError::InvalidEntity)?
                            .binding,
                        ItemBinding::Character {
                            character_id: owner
                        } if owner == character.id
                    ),
                    affix: self.item_affix_view(item_id),
                    container_capacity: item
                        .container
                        .as_ref()
                        .map(|_| {
                            container_usage(
                                item_id,
                                &self.item_instances,
                                &self.item_prototypes,
                                &item_graph,
                            )
                        })
                        .transpose()?,
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
        let spendable_coins = self.spendable_coins(character.id)?;
        let shop = if shop_available {
            commodity_specs()
                .iter()
                .map(|(id, label, buy_price, sell_price)| {
                    let stock = self.adventure.shop_stock.get(*id).copied().unwrap_or(0);
                    let owned = gameplay.commodities.get(*id).copied().unwrap_or(0);
                    let mut actions = Vec::new();
                    if stock > 0 && spendable_coins >= *buy_price && owned < 99 {
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
                nearby: connected.room_id == current_room.stable_id,
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
            inventory_capacity: self.inventory_usage(character.id)?,
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
                reserved_coins: self.reserved_coins(character.id),
                spendable_coins: self.spendable_coins(character.id)?,
                commodities,
                shop_available,
                shop,
            },
            social: SocialView { online },
            trade: self.trade_view_for(session)?,
        })
    }

    fn trade_view_for(&self, session: SessionId) -> Result<Option<TradeView>, WorldError> {
        let viewer = self.character(session)?;
        let Some(trade) = self.trade_for_character(viewer.id) else {
            return Ok(None);
        };
        let observer = self.observer(session)?;
        let counterpart = trade
            .counterpart(viewer.id)
            .ok_or(WorldError::InvalidRuntimeSnapshot)?;
        let counterparty = opaque_id(observer.salt, "character", &counterpart.0.to_string());
        let offer_view = |participant: CharacterId| -> Result<TradeOfferView, WorldError> {
            let offer = trade.offers.get(&participant).cloned().unwrap_or_default();
            let lines = offer
                .lines
                .iter()
                .map(|line| {
                    let first = line
                        .item_ids
                        .first()
                        .ok_or(WorldError::InvalidRuntimeSnapshot)?;
                    let prototype = self.item_prototype_for(first)?;
                    Ok(TradeOfferLineView {
                        target: opaque_id(observer.salt, "item", first.as_str()),
                        label: prototype.name.clone(),
                        quantity: u16::try_from(line.item_ids.len())
                            .map_err(|_| WorldError::InvalidRuntimeSnapshot)?,
                        container: prototype.container.is_some(),
                    })
                })
                .collect::<Result<Vec<_>, WorldError>>()?;
            Ok(TradeOfferView {
                coins: offer.coins,
                lines,
            })
        };
        let empty = trade
            .offers
            .values()
            .all(|offer| offer.coins == 0 && offer.lines.is_empty());
        Ok(Some(TradeView {
            id: opaque_id(observer.salt, "trade", &trade.id.0.to_string()),
            phase: trade.phase,
            counterparty,
            expires_in_ticks: trade.expires_at_tick.saturating_sub(self.logical_tick),
            state_revision: trade.state_revision,
            own_confirmed: trade.confirmations.contains_key(&viewer.id),
            counterparty_confirmed: trade.confirmations.contains_key(&counterpart),
            own_offer: offer_view(viewer.id)?,
            counterparty_offer: offer_view(counterpart)?,
            can_accept: trade.phase == TradePhase::Requested && trade.recipient == viewer.id,
            can_confirm: trade.phase == TradePhase::Negotiating && !empty,
        }))
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
            DomainEvent::CharacterRelocated {
                actor_id,
                actor_name,
                from_room_id,
                ..
            } => {
                let text = if viewer.id == *actor_id {
                    "You are relocated by staff authority.".to_owned()
                } else {
                    let viewer_room = room_checked(&self.entities, viewer.location)?
                        .stable_id
                        .as_str();
                    if viewer_room == from_room_id {
                        format!("{actor_name} vanishes.")
                    } else {
                        format!("{actor_name} appears.")
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
                    Some(opaque_id(observer.salt, "item", item_id.as_str())),
                )
            }
            DomainEvent::ItemsMoved {
                actor_id,
                actor_name,
                item_ids,
                item_name,
                container_id,
                operation,
                room_id,
            } => {
                let visible_item_ids = self
                    .perceived_moved_item_ids(session, *actor_id, item_ids, *operation, room_id)?;
                let actor_is_viewer = viewer.id == *actor_id;
                let quantity = if actor_is_viewer
                    || matches!(
                        operation,
                        ContainerMoveOperation::Take | ContainerMoveOperation::Drop
                    ) {
                    visible_item_ids.len()
                } else {
                    // Putting/getting is observable room narration, like
                    // equip/use, but another player never receives selectors
                    // for the actor's private inventory or container.
                    item_ids.len()
                };
                let text = match (viewer.id == *actor_id, operation) {
                    (true, ContainerMoveOperation::Take) => {
                        format!("You take {quantity} × {item_name}.")
                    }
                    (true, ContainerMoveOperation::Drop) => {
                        format!("You drop {quantity} × {item_name}.")
                    }
                    (true, ContainerMoveOperation::Put) => {
                        format!("You put {quantity} × {item_name} in the container.")
                    }
                    (true, ContainerMoveOperation::Get) => {
                        format!("You get {quantity} × {item_name} from the container.")
                    }
                    (false, ContainerMoveOperation::Take) => {
                        format!("{actor_name} takes {quantity} × {item_name}.")
                    }
                    (false, ContainerMoveOperation::Drop) => {
                        format!("{actor_name} drops {quantity} × {item_name}.")
                    }
                    (false, ContainerMoveOperation::Put) => {
                        format!("{actor_name} puts {quantity} × {item_name} in a container.")
                    }
                    (false, ContainerMoveOperation::Get) => {
                        format!("{actor_name} gets {quantity} × {item_name} from a container.")
                    }
                };
                (
                    match operation {
                        ContainerMoveOperation::Take => ViewEventKind::ItemTaken,
                        ContainerMoveOperation::Drop => ViewEventKind::ItemDropped,
                        ContainerMoveOperation::Put | ContainerMoveOperation::Get => {
                            ViewEventKind::Gameplay
                        }
                    },
                    opaque_id(observer.salt, "character", &actor_id.0.to_string()),
                    actor_is_viewer
                        .then(|| {
                            container_id.as_ref().map(|container_id| {
                                opaque_id(observer.salt, "item", container_id.as_str())
                            })
                        })
                        .flatten(),
                    text,
                    visible_item_ids
                        .first()
                        .map(|item_id| opaque_id(observer.salt, "item", item_id.as_str())),
                )
            }
            DomainEvent::ItemsGiven {
                giver_id,
                giver_name,
                recipient_id,
                recipient_name,
                item_ids,
                item_name,
                ..
            } => {
                let quantity = item_ids.len();
                let text = if viewer.id == *giver_id {
                    format!("You give {quantity} × {item_name} to {recipient_name}.")
                } else if viewer.id == *recipient_id {
                    format!("{giver_name} gives you {quantity} × {item_name}.")
                } else {
                    format!("{giver_name} gives {quantity} × {item_name} to {recipient_name}.")
                };
                (
                    ViewEventKind::ItemGiven,
                    opaque_id(observer.salt, "character", &giver_id.0.to_string()),
                    Some(opaque_id(
                        observer.salt,
                        "character",
                        &recipient_id.0.to_string(),
                    )),
                    text,
                    (viewer.id == *giver_id || viewer.id == *recipient_id).then(|| {
                        opaque_id(
                            observer.salt,
                            "item",
                            item_ids
                                .first()
                                .expect("validated ItemsGiven is non-empty")
                                .as_str(),
                        )
                    }),
                )
            }
            DomainEvent::TradeRequested { trade } => (
                ViewEventKind::TradeRequested,
                opaque_id(observer.salt, "character", &trade.requester.0.to_string()),
                Some(opaque_id(
                    observer.salt,
                    "character",
                    &trade.recipient.0.to_string(),
                )),
                if viewer.id == trade.recipient {
                    "You receive a trade request.".into()
                } else {
                    "Your trade request is pending.".into()
                },
                None,
            ),
            DomainEvent::TradeOpened { current, .. }
            | DomainEvent::TradeOfferSet { current, .. }
            | DomainEvent::TradeInvalidated { current, .. } => (
                ViewEventKind::TradeChanged,
                opaque_id(observer.salt, "character", &current.requester.0.to_string()),
                Some(opaque_id(
                    observer.salt,
                    "character",
                    &current.recipient.0.to_string(),
                )),
                "The private trade terms change.".into(),
                None,
            ),
            DomainEvent::TradeConfirmed { participant, .. } => (
                ViewEventKind::TradeChanged,
                opaque_id(observer.salt, "character", &participant.0.to_string()),
                None,
                if viewer.id == *participant {
                    "You confirm the current trade terms.".into()
                } else {
                    "The other participant confirms the current trade terms.".into()
                },
                None,
            ),
            DomainEvent::TradeCancelled { previous_trade, .. } => (
                ViewEventKind::TradeCancelled,
                opaque_id(
                    observer.salt,
                    "character",
                    &previous_trade.requester.0.to_string(),
                ),
                Some(opaque_id(
                    observer.salt,
                    "character",
                    &previous_trade.recipient.0.to_string(),
                )),
                "The trade is cancelled.".into(),
                None,
            ),
            DomainEvent::TradeCompleted { previous_trade } => (
                ViewEventKind::TradeCompleted,
                opaque_id(
                    observer.salt,
                    "character",
                    &previous_trade.requester.0.to_string(),
                ),
                Some(opaque_id(
                    observer.salt,
                    "character",
                    &previous_trade.recipient.0.to_string(),
                )),
                if previous_trade.includes(viewer.id) {
                    "Your trade completes atomically.".into()
                } else {
                    "Two characters complete a trade.".into()
                },
                None,
            ),
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
                    Some(opaque_id(observer.salt, "item", item_id.as_str())),
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
        item_id: &ItemInstanceId,
        item_name: &str,
        room_id: &str,
    ) -> Result<(), WorldError> {
        let valid_actor = actor_id.0 > 0 && !actor_name.trim().is_empty();
        let valid_item = self
            .item_prototype_for(item_id)
            .is_ok_and(|item| item.name == item_name && !item_name.trim().is_empty());
        let valid_room = self.stable_rooms.contains_key(room_id);
        if valid_actor && valid_item && valid_room {
            Ok(())
        } else {
            Err(WorldError::InvalidDomainEvent)
        }
    }

    fn validate_items_moved_event(&self, event: &DomainEvent) -> Result<(), WorldError> {
        let DomainEvent::ItemsMoved {
            actor_id,
            actor_name,
            item_ids,
            item_name,
            container_id,
            operation,
            room_id,
        } = event
        else {
            return Err(WorldError::InvalidDomainEvent);
        };
        let container_shape_valid = match operation {
            ContainerMoveOperation::Take | ContainerMoveOperation::Drop => container_id.is_none(),
            ContainerMoveOperation::Put | ContainerMoveOperation::Get => {
                container_id.as_ref().is_some_and(|container_id| {
                    self.item_instances
                        .get(container_id)
                        .and_then(|instance| self.item_prototypes.get(&instance.prototype_id))
                        .is_some_and(|prototype| prototype.container.is_some())
                        && self
                            .container_root_matches_event(container_id, *actor_id, room_id)
                            .unwrap_or(false)
                })
            }
        };
        let unique = item_ids.iter().collect::<BTreeSet<_>>().len() == item_ids.len();
        let first_prototype = item_ids.first().and_then(|item_id| {
            self.item_instances
                .get(item_id)
                .map(|instance| instance.prototype_id.as_str())
        });
        let items_valid = !item_ids.is_empty()
            && item_ids.len() <= usize::from(MAX_ITEM_SELECTION_QUANTITY)
            && unique
            && first_prototype.is_some()
            && item_ids.iter().all(|item_id| {
                self.item_instances
                    .get(item_id)
                    .is_some_and(|instance| Some(instance.prototype_id.as_str()) == first_prototype)
                    && self
                        .item_prototype_for(item_id)
                        .is_ok_and(|prototype| prototype.name == item_name.as_str())
            });
        if actor_id.0 <= 0
            || actor_name.trim().is_empty()
            || item_name.trim().is_empty()
            || !self.stable_rooms.contains_key(room_id)
            || !container_shape_valid
            || !items_valid
        {
            return Err(WorldError::InvalidDomainEvent);
        }
        Ok(())
    }

    fn validate_items_given_event(&self, event: &DomainEvent) -> Result<(), WorldError> {
        let DomainEvent::ItemsGiven {
            giver_id,
            giver_name,
            recipient_id,
            recipient_name,
            item_ids,
            item_name,
            room_id,
        } = event
        else {
            return Err(WorldError::InvalidDomainEvent);
        };
        let unique = item_ids.iter().collect::<BTreeSet<_>>().len() == item_ids.len();
        let first_prototype = item_ids.first().and_then(|item_id| {
            self.item_instances
                .get(item_id)
                .map(|instance| instance.prototype_id.as_str())
        });
        let graph = build_item_graph_index(&self.item_instances, &self.item_prototypes)
            .map_err(|_| WorldError::InvalidDomainEvent)?;
        let items_valid = !item_ids.is_empty()
            && item_ids.len() <= usize::from(MAX_ITEM_SELECTION_QUANTITY)
            && unique
            && first_prototype.is_some()
            && item_ids.iter().all(|item_id| {
                self.item_instances
                    .get(item_id)
                    .is_some_and(|instance| Some(instance.prototype_id.as_str()) == first_prototype)
                    && self
                        .item_prototype_for(item_id)
                        .is_ok_and(|prototype| prototype.name == item_name.as_str())
                    && graph.subtree_is_transferable(item_id).unwrap_or(false)
            });
        if giver_id.0 <= 0
            || recipient_id.0 <= 0
            || giver_id == recipient_id
            || giver_name.trim().is_empty()
            || recipient_name.trim().is_empty()
            || item_name.trim().is_empty()
            || !self.stable_rooms.contains_key(room_id)
            || !items_valid
        {
            return Err(WorldError::InvalidDomainEvent);
        }
        Ok(())
    }

    fn container_root_matches_event(
        &self,
        container_id: &ItemInstanceId,
        actor_id: CharacterId,
        room_id: &str,
    ) -> Result<bool, WorldError> {
        let mut current = container_id;
        for _ in 0..=MAX_CONTAINER_DEPTH {
            let instance = self
                .item_instances
                .get(current)
                .ok_or(WorldError::InvalidEntity)?;
            match &instance.location {
                SharedItemLocation::Carried(owner) | SharedItemLocation::Equipped(owner, _) => {
                    return Ok(*owner == actor_id);
                }
                SharedItemLocation::Room(root_room) => return Ok(root_room == room_id),
                SharedItemLocation::Contained(parent) => current = parent,
                SharedItemLocation::Staged
                | SharedItemLocation::HeldByNpc(_)
                | SharedItemLocation::Consumed => return Ok(false),
            }
        }
        Err(WorldError::InvalidEntity)
    }

    fn perceived_moved_item_ids(
        &self,
        session: SessionId,
        actor_id: CharacterId,
        item_ids: &[ItemInstanceId],
        operation: ContainerMoveOperation,
        room_id: &str,
    ) -> Result<Vec<ItemInstanceId>, WorldError> {
        let viewer = self.character(session)?;
        if viewer.id == actor_id {
            return Ok(item_ids.to_vec());
        }
        let viewer_room = room_checked(&self.entities, viewer.location)?;
        if viewer_room.stable_id != room_id {
            return Ok(Vec::new());
        }
        match operation {
            ContainerMoveOperation::Take | ContainerMoveOperation::Drop => item_ids
                .iter()
                .map(|item_id| {
                    self.item_visible_to(session, item_id, room_id)
                        .map(|visible| visible.then_some(item_id.clone()))
                })
                .collect::<Result<Vec<_>, WorldError>>()
                .map(|items| items.into_iter().flatten().collect()),
            ContainerMoveOperation::Put | ContainerMoveOperation::Get => Ok(Vec::new()),
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
            DomainEvent::CharacterRelocated {
                actor_id,
                from_room_id,
                to_room_id,
                ..
            } => viewer.id == *actor_id || viewer_room == from_room_id || viewer_room == to_room_id,
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
            DomainEvent::ItemsMoved {
                actor_id,
                item_ids,
                operation,
                room_id,
                ..
            } => match operation {
                ContainerMoveOperation::Take | ContainerMoveOperation::Drop => !self
                    .perceived_moved_item_ids(session, *actor_id, item_ids, *operation, room_id)?
                    .is_empty(),
                ContainerMoveOperation::Put | ContainerMoveOperation::Get => {
                    viewer.id == *actor_id || viewer_room == room_id
                }
            },
            DomainEvent::ItemsGiven {
                giver_id,
                recipient_id,
                room_id,
                ..
            } => viewer.id == *giver_id || viewer.id == *recipient_id || viewer_room == room_id,
            DomainEvent::TradeRequested { trade } => trade.includes(viewer.id),
            DomainEvent::TradeOpened { current, .. }
            | DomainEvent::TradeOfferSet { current, .. }
            | DomainEvent::TradeInvalidated { current, .. } => current.includes(viewer.id),
            DomainEvent::TradeConfirmed { participant, .. } => self
                .trade_for_character(*participant)
                .is_some_and(|trade| trade.includes(viewer.id)),
            DomainEvent::TradeCancelled { previous_trade, .. } => {
                previous_trade.includes(viewer.id)
            }
            DomainEvent::TradeCompleted { previous_trade } => {
                previous_trade.includes(viewer.id) || viewer_room == previous_trade.room_id
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

#[derive(Clone, Copy, Eq, PartialEq)]
enum ItemScope {
    Carried,
    CurrentRoom,
    Accessible,
}

const MAX_CONTAINER_DEPTH: usize = 4;
const CONTAINER_PAGE_SIZE: usize = 20;

#[derive(Clone, Debug)]
struct ItemStackGroup {
    prototype_id: String,
    binding: ItemBinding,
    members: Vec<ItemInstanceId>,
}

fn derive_stack_groups<'a>(
    instances: &BTreeMap<ItemInstanceId, ItemInstance>,
    prototypes: &HashMap<String, ItemPrototype>,
    item_ids: impl Iterator<Item = &'a ItemInstanceId>,
) -> Result<Vec<ItemStackGroup>, WorldError> {
    let mut by_prototype =
        BTreeMap::<(String, ItemBinding, Vec<AppliedItemAffix>), Vec<ItemInstanceId>>::new();
    for item_id in item_ids {
        let instance = instances.get(item_id).ok_or(WorldError::InvalidEntity)?;
        by_prototype
            .entry((
                instance.prototype_id.clone(),
                instance.binding,
                instance.item_affixes.clone(),
            ))
            .or_default()
            .push(item_id.clone());
    }
    let mut groups = Vec::new();
    for ((prototype_id, binding, _), mut members) in by_prototype {
        let prototype = prototypes
            .get(&prototype_id)
            .ok_or(WorldError::InvalidEntity)?;
        members.sort();
        for chunk in members.chunks(usize::from(prototype.max_stack_units)) {
            groups.push(ItemStackGroup {
                prototype_id: prototype_id.clone(),
                binding,
                members: chunk.to_vec(),
            });
        }
    }
    groups.sort_by(|left, right| left.members[0].cmp(&right.members[0]));
    Ok(groups)
}

fn take_selected_quantity(
    mut eligible: Vec<ItemInstanceId>,
    quantity: u16,
    query: &str,
) -> Result<Vec<ItemInstanceId>, WorldError> {
    eligible.sort();
    let quantity = usize::from(quantity);
    if eligible.len() < quantity {
        return Err(WorldError::ItemNotFound(query.to_owned()));
    }
    eligible.truncate(quantity);
    Ok(eligible)
}

fn stack_slots(prototype: &ItemPrototype, quantity: usize) -> Result<u32, WorldError> {
    let max = usize::from(prototype.max_stack_units);
    if max == 0 {
        return Err(WorldError::InvalidEntity);
    }
    let chunks = quantity
        .checked_add(max - 1)
        .ok_or(WorldError::InvalidEntity)?
        / max;
    u32::try_from(chunks)
        .ok()
        .and_then(|chunks| chunks.checked_mul(u32::from(prototype.slot_cost)))
        .ok_or(WorldError::InvalidEntity)
}

#[derive(Clone, Debug)]
struct ItemGraphIndex {
    children: BTreeMap<ItemInstanceId, Vec<ItemInstanceId>>,
    subtree_weights: BTreeMap<ItemInstanceId, u64>,
    subtree_transferable: BTreeMap<ItemInstanceId, bool>,
}

impl ItemGraphIndex {
    fn subtree_weight(&self, item_id: &ItemInstanceId) -> Result<u64, WorldError> {
        self.subtree_weights
            .get(item_id)
            .copied()
            .ok_or(WorldError::InvalidEntity)
    }

    fn subtree_is_transferable(&self, item_id: &ItemInstanceId) -> Result<bool, WorldError> {
        self.subtree_transferable
            .get(item_id)
            .copied()
            .ok_or(WorldError::InvalidEntity)
    }

    fn subtree_ids(&self, root: &ItemInstanceId) -> Result<Vec<ItemInstanceId>, WorldError> {
        if !self.subtree_weights.contains_key(root) {
            return Err(WorldError::InvalidEntity);
        }
        let mut result = Vec::new();
        let mut pending = vec![root.clone()];
        while let Some(item_id) = pending.pop() {
            if let Some(children) = self.children.get(&item_id) {
                pending.extend(children.iter().rev().cloned());
            }
            result.push(item_id);
        }
        Ok(result)
    }
}

fn build_item_graph_index(
    instances: &BTreeMap<ItemInstanceId, ItemInstance>,
    prototypes: &HashMap<String, ItemPrototype>,
) -> Result<ItemGraphIndex, WorldError> {
    let mut children = BTreeMap::<ItemInstanceId, Vec<ItemInstanceId>>::new();
    let mut subtree_weights = BTreeMap::<ItemInstanceId, u64>::new();
    let mut subtree_transferable = BTreeMap::<ItemInstanceId, bool>::new();
    for (item_id, instance) in instances {
        let prototype = prototypes
            .get(&instance.prototype_id)
            .ok_or(WorldError::InvalidEntity)?;
        subtree_weights.insert(item_id.clone(), u64::from(prototype.weight_grams));
        subtree_transferable.insert(
            item_id.clone(),
            prototype.transfer_policy == ItemTransferPolicy::Transferable
                && instance.binding == ItemBinding::Unbound,
        );
        if let SharedItemLocation::Contained(parent) = &instance.location {
            let parent_instance = instances.get(parent).ok_or(WorldError::InvalidEntity)?;
            if prototypes
                .get(&parent_instance.prototype_id)
                .and_then(|prototype| prototype.container.as_ref())
                .is_none()
            {
                return Err(WorldError::InvalidEntity);
            }
            children
                .entry(parent.clone())
                .or_default()
                .push(item_id.clone());
        }
    }

    // Parent chains are at most four edges, so this is O(N) with a small,
    // explicit constant and detects cycles before any recursive accounting.
    let mut depths = BTreeMap::<ItemInstanceId, usize>::new();
    for item_id in instances.keys() {
        let mut current = item_id;
        let mut visited = BTreeSet::new();
        let mut depth = 0_usize;
        while let Some(ItemInstance {
            location: SharedItemLocation::Contained(parent),
            ..
        }) = instances.get(current)
        {
            if !visited.insert(current.clone()) || parent == current {
                return Err(WorldError::InvalidEntity);
            }
            depth = depth.checked_add(1).ok_or(WorldError::InvalidEntity)?;
            if depth > MAX_CONTAINER_DEPTH {
                return Err(WorldError::InvalidEntity);
            }
            current = parent;
        }
        depths.insert(item_id.clone(), depth);
    }

    // Descendants contribute once to their parent after their own descendants
    // have already been folded in. This avoids rescanning all N instances for
    // every container or root inventory item.
    let mut deepest_first = depths.into_iter().collect::<Vec<_>>();
    deepest_first.sort_by(|(left_id, left_depth), (right_id, right_depth)| {
        right_depth
            .cmp(left_depth)
            .then_with(|| left_id.cmp(right_id))
    });
    for (item_id, depth) in deepest_first {
        if depth == 0 {
            continue;
        }
        let parent = match &instances
            .get(&item_id)
            .ok_or(WorldError::InvalidEntity)?
            .location
        {
            SharedItemLocation::Contained(parent) => parent,
            _ => return Err(WorldError::InvalidEntity),
        };
        let child_weight = subtree_weights
            .get(&item_id)
            .copied()
            .ok_or(WorldError::InvalidEntity)?;
        let parent_weight = subtree_weights
            .get_mut(parent)
            .ok_or(WorldError::InvalidEntity)?;
        *parent_weight = parent_weight
            .checked_add(child_weight)
            .ok_or(WorldError::InvalidEntity)?;
        let child_transferable = subtree_transferable
            .get(&item_id)
            .copied()
            .ok_or(WorldError::InvalidEntity)?;
        let parent_transferable = subtree_transferable
            .get_mut(parent)
            .ok_or(WorldError::InvalidEntity)?;
        *parent_transferable &= child_transferable;
    }

    Ok(ItemGraphIndex {
        children,
        subtree_weights,
        subtree_transferable,
    })
}

fn container_usage(
    container_id: &ItemInstanceId,
    instances: &BTreeMap<ItemInstanceId, ItemInstance>,
    prototypes: &HashMap<String, ItemPrototype>,
    graph: &ItemGraphIndex,
) -> Result<InventoryCapacityView, WorldError> {
    let container_instance = instances
        .get(container_id)
        .ok_or(WorldError::InvalidEntity)?;
    let container_prototype = prototypes
        .get(&container_instance.prototype_id)
        .ok_or(WorldError::InvalidEntity)?;
    let capacity = container_prototype
        .container
        .as_ref()
        .ok_or(WorldError::GameplayUnavailable)?;
    let direct = graph
        .children
        .get(container_id)
        .map(Vec::as_slice)
        .unwrap_or_default();
    let mut quantities = BTreeMap::<(String, ItemBinding), usize>::new();
    let mut used_weight = 0_u64;
    for item_id in direct {
        let instance = instances.get(item_id).ok_or(WorldError::InvalidEntity)?;
        *quantities
            .entry((instance.prototype_id.clone(), instance.binding))
            .or_default() += 1;
        used_weight = used_weight
            .checked_add(graph.subtree_weight(item_id)?)
            .ok_or(WorldError::InvalidEntity)?;
    }
    let mut used_slots = 0_u32;
    for ((prototype_id, _), quantity) in quantities {
        let prototype = prototypes
            .get(&prototype_id)
            .ok_or(WorldError::InvalidEntity)?;
        used_slots = used_slots
            .checked_add(stack_slots(prototype, quantity)?)
            .ok_or(WorldError::InvalidEntity)?;
    }
    Ok(InventoryCapacityView {
        used_slots: u16::try_from(used_slots).map_err(|_| WorldError::InvalidEntity)?,
        max_slots: capacity.max_slots,
        used_weight_grams: u32::try_from(used_weight).map_err(|_| WorldError::InvalidEntity)?,
        max_weight_grams: capacity.max_weight_grams,
    })
}

fn character_inventory_totals(
    character_id: CharacterId,
    instances: &BTreeMap<ItemInstanceId, ItemInstance>,
    prototypes: &HashMap<String, ItemPrototype>,
) -> Result<(u32, u64), WorldError> {
    let graph = build_item_graph_index(instances, prototypes)?;
    character_inventory_totals_with_graph(character_id, instances, prototypes, &graph)
}

fn character_inventory_totals_with_graph(
    character_id: CharacterId,
    instances: &BTreeMap<ItemInstanceId, ItemInstance>,
    prototypes: &HashMap<String, ItemPrototype>,
    graph: &ItemGraphIndex,
) -> Result<(u32, u64), WorldError> {
    let mut carried_quantities = BTreeMap::<(String, ItemBinding), usize>::new();
    let mut weight = 0_u64;
    for (item_id, instance) in instances {
        let carried = matches!(&instance.location, SharedItemLocation::Carried(owner) if *owner == character_id);
        let equipped = matches!(&instance.location, SharedItemLocation::Equipped(owner, _) if *owner == character_id);
        if !carried && !equipped {
            continue;
        }
        if carried {
            *carried_quantities
                .entry((instance.prototype_id.clone(), instance.binding))
                .or_default() += 1;
        }
        weight = weight
            .checked_add(graph.subtree_weight(item_id)?)
            .ok_or(WorldError::InvalidEntity)?;
    }
    let mut slots = 0_u32;
    for ((prototype_id, _), quantity) in carried_quantities {
        let prototype = prototypes
            .get(&prototype_id)
            .ok_or(WorldError::InvalidEntity)?;
        slots = slots
            .checked_add(stack_slots(prototype, quantity)?)
            .ok_or(WorldError::InvalidEntity)?;
    }
    Ok((slots, weight))
}

fn validate_item_graph(
    instances: &BTreeMap<ItemInstanceId, ItemInstance>,
    prototypes: &HashMap<String, ItemPrototype>,
) -> Result<(), WorldError> {
    let graph = build_item_graph_index(instances, prototypes)?;
    validate_container_capacities(instances, prototypes, &graph)?;
    validate_item_bindings(instances, prototypes)
}

fn item_effective_owner(
    instances: &BTreeMap<ItemInstanceId, ItemInstance>,
    item_id: &ItemInstanceId,
) -> Result<Option<CharacterId>, WorldError> {
    let mut current = item_id;
    for _ in 0..=MAX_CONTAINER_DEPTH {
        let instance = instances.get(current).ok_or(WorldError::InvalidEntity)?;
        match &instance.location {
            SharedItemLocation::Carried(character_id)
            | SharedItemLocation::Equipped(character_id, _) => return Ok(Some(*character_id)),
            SharedItemLocation::Contained(parent) => current = parent,
            SharedItemLocation::Staged
            | SharedItemLocation::Room(_)
            | SharedItemLocation::HeldByNpc(_)
            | SharedItemLocation::Consumed => return Ok(None),
        }
    }
    Err(WorldError::InvalidEntity)
}

fn items_by_effective_owner(
    instances: &BTreeMap<ItemInstanceId, ItemInstance>,
) -> Result<BTreeMap<CharacterId, BTreeMap<ItemInstanceId, ItemInstance>>, WorldError> {
    let mut by_owner = BTreeMap::<CharacterId, BTreeMap<ItemInstanceId, ItemInstance>>::new();
    for (item_id, instance) in instances {
        if let Some(owner) = item_effective_owner(instances, item_id)? {
            by_owner
                .entry(owner)
                .or_default()
                .insert(item_id.clone(), instance.clone());
        }
    }
    Ok(by_owner)
}

fn reserved_item_ids_with_graph(
    trades: &BTreeMap<TradeId, TradeSnapshot>,
    graph: &ItemGraphIndex,
) -> Result<BTreeSet<ItemInstanceId>, WorldError> {
    let mut reserved = BTreeSet::new();
    for root in trades
        .values()
        .flat_map(|trade| trade.offers.values())
        .flat_map(|offer| &offer.lines)
        .flat_map(|line| &line.item_ids)
    {
        for item_id in graph.subtree_ids(root)? {
            if !reserved.insert(item_id) {
                return Err(WorldError::InvalidRuntimeSnapshot);
            }
        }
    }
    Ok(reserved)
}

fn character_room_id(
    entities: &SlotMap<EntityId, Entity>,
    character_id: CharacterId,
) -> Option<&str> {
    entities.values().find_map(|entity| match entity {
        Entity::Character(character) if character.id == character_id => entities
            .get(character.location)
            .and_then(|room| match room {
                Entity::Room(room) => Some(room.stable_id.as_str()),
                Entity::Character(_) => None,
            }),
        _ => None,
    })
}

fn canonical_u64(digest: &mut Sha256, value: u64) {
    digest.update(value.to_be_bytes());
}

fn canonical_i64(digest: &mut Sha256, value: i64) {
    digest.update(value.to_be_bytes());
}

fn canonical_str(digest: &mut Sha256, value: &str) -> Result<(), WorldError> {
    let length = u32::try_from(value.len()).map_err(|_| WorldError::InvalidRuntimeSnapshot)?;
    digest.update(length.to_be_bytes());
    digest.update(value.as_bytes());
    Ok(())
}

fn canonical_trade_key(digest: &mut Sha256, key: &TradeOfferKey) -> Result<(), WorldError> {
    match key {
        TradeOfferKey::Fungible { prototype_id } => {
            digest.update([0]);
            canonical_str(digest, prototype_id)
        }
        TradeOfferKey::Unique { item_id } => {
            digest.update([1]);
            canonical_str(digest, item_id.as_str())
        }
    }
}

fn canonical_item_binding(digest: &mut Sha256, binding: ItemBinding) {
    match binding {
        ItemBinding::Unbound => digest.update([0]),
        ItemBinding::Character { character_id } => {
            digest.update([1]);
            canonical_i64(digest, character_id.0);
        }
    }
}

fn canonical_item_affixes(
    digest: &mut Sha256,
    affixes: &[AppliedItemAffix],
) -> Result<(), WorldError> {
    canonical_u64(
        digest,
        u64::try_from(affixes.len()).map_err(|_| WorldError::InvalidRuntimeSnapshot)?,
    );
    for affix in affixes {
        canonical_str(digest, &affix.affix_id)?;
        canonical_str(digest, &affix.definition_digest_sha256)?;
        canonical_str(digest, &affix.pool_id)?;
        canonical_str(digest, &affix.pool_digest_sha256)?;
        canonical_str(digest, &affix.record_digest_sha256)?;
        canonical_str(digest, &affix.label)?;
        digest.update([match affix.position {
            AffixPosition::Prefix => 0,
            AffixPosition::Suffix => 1,
        }]);
        digest.update(affix.attack_bonus_delta.to_be_bytes());
        digest.update(affix.heal_amount_delta.to_be_bytes());
    }
    Ok(())
}

fn canonical_item_location(
    digest: &mut Sha256,
    location: &SharedItemLocation,
) -> Result<(), WorldError> {
    match location {
        SharedItemLocation::Staged => digest.update([0]),
        SharedItemLocation::Carried(character_id) => {
            digest.update([1]);
            canonical_i64(digest, character_id.0);
        }
        SharedItemLocation::Room(room_id) => {
            digest.update([2]);
            canonical_str(digest, room_id)?;
        }
        SharedItemLocation::Equipped(character_id, slot) => {
            digest.update([3, *slot as u8]);
            canonical_i64(digest, character_id.0);
        }
        SharedItemLocation::HeldByNpc(npc_id) => {
            digest.update([4]);
            canonical_str(digest, npc_id)?;
        }
        SharedItemLocation::Contained(container_id) => {
            digest.update([5]);
            canonical_str(digest, container_id.as_str())?;
        }
        SharedItemLocation::Consumed => digest.update([6]),
    }
    Ok(())
}

fn validate_item_bindings(
    instances: &BTreeMap<ItemInstanceId, ItemInstance>,
    prototypes: &HashMap<String, ItemPrototype>,
) -> Result<(), WorldError> {
    for (item_id, instance) in instances {
        let prototype = prototypes
            .get(&instance.prototype_id)
            .ok_or(WorldError::InvalidEntity)?;
        let owner = item_effective_owner(instances, item_id)?;
        if owner.is_some_and(|character_id| character_id.0 <= 0) {
            return Err(WorldError::InvalidEntity);
        }
        let valid = match prototype.transfer_policy {
            ItemTransferPolicy::Transferable | ItemTransferPolicy::NonTransferable => {
                instance.binding == ItemBinding::Unbound
            }
            ItemTransferPolicy::CharacterBound => match (owner, instance.binding) {
                (Some(owner), ItemBinding::Character { character_id }) => {
                    owner.0 > 0 && character_id.0 > 0 && owner == character_id
                }
                (None, ItemBinding::Unbound) => {
                    !matches!(instance.location, SharedItemLocation::Consumed)
                }
                (None, ItemBinding::Character { character_id }) => {
                    character_id.0 > 0 && matches!(instance.location, SharedItemLocation::Consumed)
                }
                (Some(_), ItemBinding::Unbound) => false,
            },
        };
        if !valid {
            return Err(WorldError::InvalidEntity);
        }
    }
    Ok(())
}

fn validate_restored_item_affixes(
    instances: &BTreeMap<ItemInstanceId, ItemInstance>,
    prototypes: &HashMap<String, ItemPrototype>,
) -> Result<(), WorldError> {
    for instance in instances.values() {
        let prototype = prototypes
            .get(&instance.prototype_id)
            .ok_or(WorldError::InvalidRuntimeSnapshot)?;
        if !instance.item_affixes.is_empty()
            && (prototype.stack_policy != ItemStackPolicy::Unique
                || prototype.max_stack_units != 1
                || prototype.sealed_loot_table_id.is_some())
        {
            return Err(WorldError::InvalidRuntimeSnapshot);
        }
        for applied in &instance.item_affixes {
            if (applied.attack_bonus_delta != 0 && prototype.equipment_slot.is_none())
                || (applied.heal_amount_delta != 0 && !prototype.consumable)
                || prototype
                    .attack_bonus
                    .checked_add(applied.attack_bonus_delta)
                    .is_none_or(|value| {
                        !(0..=i32::try_from(MAX_STAT_VALUE).expect("stat cap fits i32"))
                            .contains(&value)
                    })
                || prototype
                    .heal_amount
                    .checked_add(applied.heal_amount_delta)
                    .is_none_or(|value| {
                        !(0..=i32::try_from(MAX_STAT_VALUE).expect("stat cap fits i32"))
                            .contains(&value)
                    })
            {
                return Err(WorldError::InvalidRuntimeSnapshot);
            }
        }
    }
    Ok(())
}

fn validate_restored_sealed_items(
    instances: &BTreeMap<ItemInstanceId, ItemInstance>,
    provenance: &BTreeMap<ItemInstanceId, ItemProvenance>,
    ground_loot: &BTreeMap<ItemInstanceId, GroundLootReservation>,
    prototypes: &HashMap<String, ItemPrototype>,
) -> Result<(), WorldError> {
    for (item_id, instance) in instances {
        let prototype = prototypes
            .get(&instance.prototype_id)
            .ok_or(WorldError::InvalidRuntimeSnapshot)?;
        if prototype.sealed_loot_table_id.is_none() {
            continue;
        }
        let ItemProvenance::NpcLoot {
            drop_id,
            sealed_loot_table_id,
            ..
        } = provenance
            .get(item_id)
            .ok_or(WorldError::InvalidRuntimeSnapshot)?
        else {
            return Err(WorldError::InvalidRuntimeSnapshot);
        };
        if sealed_loot_table_id.as_ref() != prototype.sealed_loot_table_id.as_ref()
            || !instance.item_affixes.is_empty()
        {
            return Err(WorldError::InvalidRuntimeSnapshot);
        }
        match &instance.location {
            SharedItemLocation::Room(room_id) => {
                let reservation = ground_loot
                    .get(item_id)
                    .ok_or(WorldError::InvalidRuntimeSnapshot)?;
                if instance.binding != ItemBinding::Unbound
                    || reservation.root_item_id != *item_id
                    || reservation.room_id != *room_id
                    || reservation.drop_id != *drop_id
                {
                    return Err(WorldError::InvalidRuntimeSnapshot);
                }
            }
            SharedItemLocation::Carried(_) | SharedItemLocation::Contained(_) => {
                let owner = item_effective_owner(instances, item_id)
                    .map_err(|_| WorldError::InvalidRuntimeSnapshot)?;
                if !matches!(
                    (owner, instance.binding),
                    (Some(owner), ItemBinding::Character { character_id }) if owner == character_id
                ) {
                    return Err(WorldError::InvalidRuntimeSnapshot);
                }
            }
            SharedItemLocation::Consumed => {
                if !matches!(
                    instance.binding,
                    ItemBinding::Character { character_id } if character_id.0 > 0
                ) {
                    return Err(WorldError::InvalidRuntimeSnapshot);
                }
            }
            SharedItemLocation::Staged
            | SharedItemLocation::Equipped(_, _)
            | SharedItemLocation::HeldByNpc(_) => {
                return Err(WorldError::InvalidRuntimeSnapshot);
            }
        }
    }
    Ok(())
}

fn same_trade_identity_and_lifetime(left: &TradeSnapshot, right: &TradeSnapshot) -> bool {
    left.id == right.id
        && left.requester == right.requester
        && left.recipient == right.recipient
        && left.room_id == right.room_id
        && left.created_tick == right.created_tick
        && left.opened_tick == right.opened_tick
        && left.expires_at_tick == right.expires_at_tick
}

fn next_trade_revision(previous: &TradeSnapshot) -> Option<u64> {
    previous.state_revision.checked_add(1)
}

fn valid_trade_open_transition(previous: &TradeSnapshot, current: &TradeSnapshot) -> bool {
    let expected_offers = BTreeMap::from([
        (previous.requester, TradeOffer::default()),
        (previous.recipient, TradeOffer::default()),
    ]);
    previous.phase == TradePhase::Requested
        && previous.opened_tick.is_none()
        && previous.offers.is_empty()
        && previous.confirmations.is_empty()
        && current.id == previous.id
        && current.requester == previous.requester
        && current.recipient == previous.recipient
        && current.room_id == previous.room_id
        && current.created_tick == previous.created_tick
        && current.phase == TradePhase::Negotiating
        && current
            .opened_tick
            .is_some_and(|opened_tick| opened_tick < previous.expires_at_tick)
        && current.state_revision == next_trade_revision(previous).unwrap_or(0)
        && current.offers == expected_offers
        && current.confirmations.is_empty()
}

fn trade_has_assets(trade: &TradeSnapshot) -> bool {
    trade
        .offers
        .values()
        .any(|offer| offer.coins > 0 || !offer.lines.is_empty())
}

fn one_offer_assignment_changed(previous: &TradeOffer, current: &TradeOffer) -> bool {
    if previous.coins != current.coins {
        return previous.lines == current.lines;
    }
    if previous.lines == current.lines {
        return false;
    }
    let previous_by_key = previous
        .lines
        .iter()
        .map(|line| (&line.key, line))
        .collect::<BTreeMap<_, _>>();
    let current_by_key = current
        .lines
        .iter()
        .map(|line| (&line.key, line))
        .collect::<BTreeMap<_, _>>();
    let changed_keys = previous_by_key
        .keys()
        .chain(current_by_key.keys())
        .collect::<BTreeSet<_>>()
        .into_iter()
        .filter(|key| previous_by_key.get(**key) != current_by_key.get(**key))
        .collect::<Vec<_>>();
    let [key] = changed_keys.as_slice() else {
        return false;
    };
    match (previous_by_key.get(**key), current_by_key.get(**key)) {
        (None, Some(_)) | (Some(_), None) => true,
        (Some(previous_line), Some(current_line)) => {
            previous_line.key == current_line.key
                && ((previous_line.item_ids.len() < current_line.item_ids.len()
                    && previous_line
                        .item_ids
                        .iter()
                        .all(|item_id| current_line.item_ids.contains(item_id)))
                    || (current_line.item_ids.len() < previous_line.item_ids.len()
                        && current_line.item_ids
                            == previous_line.item_ids[..current_line.item_ids.len()]))
        }
        (None, None) => false,
    }
}

fn valid_trade_offer_transition(previous: &TradeSnapshot, current: &TradeSnapshot) -> bool {
    if previous.phase != TradePhase::Negotiating
        || current.phase != TradePhase::Negotiating
        || !same_trade_identity_and_lifetime(previous, current)
        || current.state_revision != next_trade_revision(previous).unwrap_or(0)
        || !current.confirmations.is_empty()
    {
        return false;
    }
    let changed = previous
        .participants()
        .into_iter()
        .filter(|participant| previous.offers.get(participant) != current.offers.get(participant))
        .collect::<Vec<_>>();
    let [participant] = changed.as_slice() else {
        return false;
    };
    previous
        .offers
        .get(participant)
        .zip(current.offers.get(participant))
        .is_some_and(|(before, after)| one_offer_assignment_changed(before, after))
}

fn valid_trade_invalidation_transition(previous: &TradeSnapshot, current: &TradeSnapshot) -> bool {
    let Some(next_revision) = next_trade_revision(previous) else {
        return false;
    };
    let mut expected = previous.clone();
    expected.state_revision = next_revision;
    expected.confirmations.clear();
    current == &expected
}

fn portable_subtree_ids(
    root: &ItemInstanceId,
    children: &BTreeMap<ItemInstanceId, Vec<ItemInstanceId>>,
) -> Result<Vec<ItemInstanceId>, WorldError> {
    let mut result = Vec::new();
    let mut pending = vec![root.clone()];
    while let Some(item_id) = pending.pop() {
        if result.len() >= MAX_ITEM_INSTANCES {
            return Err(WorldError::InvalidRuntimeSnapshot);
        }
        if let Some(direct) = children.get(&item_id) {
            pending.extend(direct.iter().rev().cloned());
        }
        result.push(item_id);
    }
    Ok(result)
}

fn validate_portable_trade_state(
    snapshot: &WorldRuntimeSnapshot,
    children: &BTreeMap<ItemInstanceId, Vec<ItemInstanceId>>,
) -> Result<(), WorldError> {
    if snapshot.next_trade_id.0 == 0 || snapshot.trades.len() > MAX_LIVE_TRADES {
        return Err(WorldError::InvalidRuntimeSnapshot);
    }
    let mut participants = BTreeSet::new();
    let mut reserved_roots = BTreeSet::new();
    let mut reserved_subtrees = BTreeSet::new();
    for (trade_id, trade) in &snapshot.trades {
        if *trade_id != trade.id
            || trade_id.0 == 0
            || trade_id.0 >= snapshot.next_trade_id.0
            || trade.requester.0 <= 0
            || trade.recipient.0 <= 0
            || trade.requester == trade.recipient
            || !valid_definition_id(&trade.room_id)
            || trade.state_revision == 0
            || trade.created_tick >= trade.expires_at_tick
            || !participants.insert(trade.requester)
            || !participants.insert(trade.recipient)
        {
            return Err(WorldError::InvalidRuntimeSnapshot);
        }
        match trade.phase {
            TradePhase::Requested => {
                if trade.opened_tick.is_some()
                    || !trade.offers.is_empty()
                    || !trade.confirmations.is_empty()
                    || trade.expires_at_tick
                        != trade
                            .created_tick
                            .checked_add(TRADE_REQUEST_TICKS)
                            .ok_or(WorldError::InvalidRuntimeSnapshot)?
                {
                    return Err(WorldError::InvalidRuntimeSnapshot);
                }
            }
            TradePhase::Negotiating => {
                let opened = trade
                    .opened_tick
                    .ok_or(WorldError::InvalidRuntimeSnapshot)?;
                if opened < trade.created_tick
                    || trade.expires_at_tick
                        != opened
                            .checked_add(TRADE_NEGOTIATION_TICKS)
                            .ok_or(WorldError::InvalidRuntimeSnapshot)?
                    || trade.offers.len() != 2
                    || !trade.offers.contains_key(&trade.requester)
                    || !trade.offers.contains_key(&trade.recipient)
                {
                    return Err(WorldError::InvalidRuntimeSnapshot);
                }
            }
        }
        if trade.confirmations.keys().any(|participant| {
            !trade.includes(*participant)
                || trade
                    .confirmations
                    .get(participant)
                    .is_none_or(|confirmation| confirmation.state_revision != trade.state_revision)
        }) || (trade.confirmations.len() == 2
            && trade
                .confirmations
                .values()
                .map(|confirmation| confirmation.basis_sha256)
                .collect::<BTreeSet<_>>()
                .len()
                != 1)
        {
            return Err(WorldError::InvalidRuntimeSnapshot);
        }
        let mut trade_roots = 0_usize;
        for (owner, offer) in &trade.offers {
            if !trade.includes(*owner)
                || offer.coins > MAX_TRADE_COINS
                || offer.lines.len() > MAX_TRADE_LINES_PER_PARTICIPANT
                || !offer.lines.windows(2).all(|pair| pair[0].key < pair[1].key)
            {
                return Err(WorldError::InvalidRuntimeSnapshot);
            }
            for line in &offer.lines {
                if line.item_ids.is_empty()
                    || line.item_ids.len() > usize::from(MAX_TRADE_ITEM_QUANTITY)
                    || !line.item_ids.windows(2).all(|pair| pair[0] < pair[1])
                {
                    return Err(WorldError::InvalidRuntimeSnapshot);
                }
                trade_roots = trade_roots
                    .checked_add(line.item_ids.len())
                    .ok_or(WorldError::InvalidRuntimeSnapshot)?;
                for root in &line.item_ids {
                    if !valid_item_instance_id(root.as_str())
                        || !reserved_roots.insert(root.clone())
                    {
                        return Err(WorldError::InvalidRuntimeSnapshot);
                    }
                    let instance = snapshot
                        .item_instances
                        .get(root)
                        .ok_or(WorldError::InvalidRuntimeSnapshot)?;
                    if instance.location
                        != (RuntimeItemLocation::Carried {
                            character_id: *owner,
                        })
                    {
                        return Err(WorldError::InvalidRuntimeSnapshot);
                    }
                    let key_valid = match &line.key {
                        TradeOfferKey::Fungible { prototype_id } => {
                            valid_definition_id(prototype_id)
                                && &instance.prototype_id == prototype_id
                        }
                        TradeOfferKey::Unique { item_id } => {
                            item_id == root && line.item_ids.len() == 1
                        }
                    };
                    if !key_valid {
                        return Err(WorldError::InvalidRuntimeSnapshot);
                    }
                    for member in portable_subtree_ids(root, children)? {
                        if !reserved_subtrees.insert(member) {
                            return Err(WorldError::InvalidRuntimeSnapshot);
                        }
                    }
                }
            }
        }
        if trade_roots > MAX_TRADE_ROOTS {
            return Err(WorldError::InvalidRuntimeSnapshot);
        }
    }
    if reserved_roots.len() > MAX_RESERVED_TRADE_ROOTS {
        return Err(WorldError::InvalidRuntimeSnapshot);
    }
    let mut ground_subtrees = BTreeSet::new();
    for root in snapshot.ground_loot.keys() {
        for member in portable_subtree_ids(root, children)? {
            if !ground_subtrees.insert(member.clone()) || reserved_subtrees.contains(&member) {
                return Err(WorldError::InvalidRuntimeSnapshot);
            }
        }
    }
    Ok(())
}

fn validate_trade_state(
    next_trade_id: TradeId,
    trades: &BTreeMap<TradeId, TradeSnapshot>,
    instances: &BTreeMap<ItemInstanceId, ItemInstance>,
    prototypes: &HashMap<String, ItemPrototype>,
    room_ids: &HashMap<String, EntityId>,
) -> Result<(), WorldError> {
    let graph = build_item_graph_index(instances, prototypes)
        .map_err(|_| WorldError::InvalidRuntimeSnapshot)?;
    validate_trade_state_with_graph(
        next_trade_id,
        trades,
        instances,
        prototypes,
        room_ids,
        &graph,
    )
}

fn validate_trade_state_with_graph(
    next_trade_id: TradeId,
    trades: &BTreeMap<TradeId, TradeSnapshot>,
    instances: &BTreeMap<ItemInstanceId, ItemInstance>,
    prototypes: &HashMap<String, ItemPrototype>,
    room_ids: &HashMap<String, EntityId>,
    graph: &ItemGraphIndex,
) -> Result<(), WorldError> {
    if next_trade_id.0 == 0 || trades.len() > MAX_LIVE_TRADES {
        return Err(WorldError::InvalidRuntimeSnapshot);
    }
    let mut participants = BTreeSet::new();
    let mut reserved_roots = BTreeSet::new();
    let mut reserved_subtrees = BTreeSet::new();
    for (id, trade) in trades {
        if *id != trade.id
            || id.0 == 0
            || id.0 >= next_trade_id.0
            || trade.requester.0 <= 0
            || trade.recipient.0 <= 0
            || trade.requester == trade.recipient
            || !room_ids.contains_key(&trade.room_id)
            || trade.state_revision == 0
            || trade.created_tick >= trade.expires_at_tick
            || !participants.insert(trade.requester)
            || !participants.insert(trade.recipient)
        {
            return Err(WorldError::InvalidRuntimeSnapshot);
        }
        match trade.phase {
            TradePhase::Requested => {
                if trade.opened_tick.is_some()
                    || !trade.offers.is_empty()
                    || !trade.confirmations.is_empty()
                    || trade.expires_at_tick
                        != trade
                            .created_tick
                            .checked_add(TRADE_REQUEST_TICKS)
                            .ok_or(WorldError::InvalidRuntimeSnapshot)?
                {
                    return Err(WorldError::InvalidRuntimeSnapshot);
                }
            }
            TradePhase::Negotiating => {
                let opened = trade
                    .opened_tick
                    .ok_or(WorldError::InvalidRuntimeSnapshot)?;
                if opened < trade.created_tick
                    || trade.expires_at_tick
                        != opened
                            .checked_add(TRADE_NEGOTIATION_TICKS)
                            .ok_or(WorldError::InvalidRuntimeSnapshot)?
                    || trade.offers.len() != 2
                    || !trade.offers.contains_key(&trade.requester)
                    || !trade.offers.contains_key(&trade.recipient)
                {
                    return Err(WorldError::InvalidRuntimeSnapshot);
                }
            }
        }
        if trade.confirmations.keys().any(|participant| {
            !trade.includes(*participant)
                || trade
                    .confirmations
                    .get(participant)
                    .is_none_or(|confirmation| confirmation.state_revision != trade.state_revision)
        }) || (trade.confirmations.len() == 2
            && trade
                .confirmations
                .values()
                .map(|confirmation| confirmation.basis_sha256)
                .collect::<BTreeSet<_>>()
                .len()
                != 1)
        {
            return Err(WorldError::InvalidRuntimeSnapshot);
        }
        let mut trade_roots = 0usize;
        for (owner, offer) in &trade.offers {
            if !trade.includes(*owner)
                || offer.coins > MAX_TRADE_COINS
                || offer.lines.len() > MAX_TRADE_LINES_PER_PARTICIPANT
                || !offer.lines.windows(2).all(|pair| pair[0].key < pair[1].key)
            {
                return Err(WorldError::InvalidRuntimeSnapshot);
            }
            for line in &offer.lines {
                if line.item_ids.is_empty()
                    || line.item_ids.len() > usize::from(MAX_TRADE_ITEM_QUANTITY)
                    || !line.item_ids.windows(2).all(|pair| pair[0] < pair[1])
                {
                    return Err(WorldError::InvalidRuntimeSnapshot);
                }
                trade_roots = trade_roots
                    .checked_add(line.item_ids.len())
                    .ok_or(WorldError::InvalidRuntimeSnapshot)?;
                for root in &line.item_ids {
                    if !valid_item_instance_id(root.as_str())
                        || !reserved_roots.insert(root.clone())
                    {
                        return Err(WorldError::InvalidRuntimeSnapshot);
                    }
                    let instance = instances
                        .get(root)
                        .ok_or(WorldError::InvalidRuntimeSnapshot)?;
                    let prototype = prototypes
                        .get(&instance.prototype_id)
                        .ok_or(WorldError::InvalidRuntimeSnapshot)?;
                    if instance.location != SharedItemLocation::Carried(*owner)
                        || !graph
                            .subtree_is_transferable(root)
                            .map_err(|_| WorldError::InvalidRuntimeSnapshot)?
                    {
                        return Err(WorldError::InvalidRuntimeSnapshot);
                    }
                    let key_valid = match &line.key {
                        TradeOfferKey::Fungible { prototype_id } => {
                            prototype.stack_policy == ItemStackPolicy::Fungible
                                && &instance.prototype_id == prototype_id
                        }
                        TradeOfferKey::Unique { item_id } => {
                            prototype.stack_policy == ItemStackPolicy::Unique
                                && item_id == root
                                && line.item_ids.len() == 1
                        }
                    };
                    if !key_valid {
                        return Err(WorldError::InvalidRuntimeSnapshot);
                    }
                    for member in graph
                        .subtree_ids(root)
                        .map_err(|_| WorldError::InvalidRuntimeSnapshot)?
                    {
                        if !reserved_subtrees.insert(member) {
                            return Err(WorldError::InvalidRuntimeSnapshot);
                        }
                    }
                }
            }
        }
        if trade_roots > MAX_TRADE_ROOTS {
            return Err(WorldError::InvalidRuntimeSnapshot);
        }
    }
    if reserved_roots.len() > MAX_RESERVED_TRADE_ROOTS {
        return Err(WorldError::InvalidRuntimeSnapshot);
    }
    Ok(())
}

fn validate_container_capacities(
    instances: &BTreeMap<ItemInstanceId, ItemInstance>,
    prototypes: &HashMap<String, ItemPrototype>,
    graph: &ItemGraphIndex,
) -> Result<(), WorldError> {
    for (item_id, instance) in instances {
        let prototype = prototypes
            .get(&instance.prototype_id)
            .ok_or(WorldError::InvalidEntity)?;
        if let Some(capacity) = &prototype.container {
            let usage = container_usage(item_id, instances, prototypes, graph)?;
            if usage.used_slots > capacity.max_slots
                || usage.used_weight_grams > capacity.max_weight_grams
            {
                return Err(WorldError::InventoryCapacityExceeded);
            }
        }
    }
    Ok(())
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

fn valid_digest_sha256(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

fn canonical_sha256(value: &impl Serialize) -> Result<String, WorldError> {
    let bytes = serde_json::to_vec(value).map_err(|_| WorldError::InvalidGameplayDefinition)?;
    let digest = Sha256::digest(bytes);
    let mut encoded = String::with_capacity(64);
    for byte in digest {
        write!(&mut encoded, "{byte:02x}").map_err(|_| WorldError::InvalidGameplayDefinition)?;
    }
    Ok(encoded)
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct AppliedItemAffixRecord<'a> {
    affix_id: &'a str,
    definition_digest_sha256: &'a str,
    pool_id: &'a str,
    pool_digest_sha256: &'a str,
    label: &'a str,
    position: AffixPosition,
    attack_bonus_delta: i32,
    heal_amount_delta: i32,
}

fn applied_affix_record_sha256(affix: &AppliedItemAffix) -> Result<String, WorldError> {
    canonical_sha256(&AppliedItemAffixRecord {
        affix_id: &affix.affix_id,
        definition_digest_sha256: &affix.definition_digest_sha256,
        pool_id: &affix.pool_id,
        pool_digest_sha256: &affix.pool_digest_sha256,
        label: &affix.label,
        position: affix.position,
        attack_bonus_delta: affix.attack_bonus_delta,
        heal_amount_delta: affix.heal_amount_delta,
    })
}

fn affix_pool_applies(
    pool_id: &str,
    prototype: &ItemPrototype,
    pools: &BTreeMap<String, AffixPool>,
    affixes: &BTreeMap<String, AffixDefinition>,
) -> bool {
    prototype.stack_policy == ItemStackPolicy::Unique
        && prototype.max_stack_units == 1
        && prototype.sealed_loot_table_id.is_none()
        && pools.get(pool_id).is_some_and(|pool| {
            pool.entries.iter().all(|entry| match entry.kind {
                AffixPoolEntryKind::Nothing => true,
                AffixPoolEntryKind::Affix => entry
                    .affix_id
                    .as_ref()
                    .and_then(|id| affixes.get(id))
                    .is_some_and(|affix| {
                        (affix.attack_bonus_delta == 0 || prototype.equipment_slot.is_some())
                            && (affix.heal_amount_delta == 0 || prototype.consumable)
                            && prototype
                                .attack_bonus
                                .checked_add(affix.attack_bonus_delta)
                                .is_some_and(|value| {
                                    (0..=i32::try_from(MAX_STAT_VALUE).expect("stat cap fits i32"))
                                        .contains(&value)
                                })
                            && prototype
                                .heal_amount
                                .checked_add(affix.heal_amount_delta)
                                .is_some_and(|value| {
                                    (0..=i32::try_from(MAX_STAT_VALUE).expect("stat cap fits i32"))
                                        .contains(&value)
                                })
                    }),
            })
        })
}

fn safe_definition_multiline_text(value: &str, maximum_bytes: usize) -> bool {
    let trimmed = value.trim();
    !trimmed.is_empty()
        && trimmed.len() <= maximum_bytes
        && !trimmed.chars().any(|character| {
            (character.is_control() && !matches!(character, '\n' | '\r' | '\t'))
                || character == '\u{7f}'
        })
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
    instances: &BTreeMap<ItemInstanceId, ItemInstance>,
    prototypes: &HashMap<String, ItemPrototype>,
    candidates: impl Iterator<Item = &'a ItemInstanceId>,
    query: &str,
    observer_salt: [u8; 32],
) -> Result<ItemInstanceId, WorldError> {
    let query = query.trim().to_ascii_lowercase();
    let candidate_items = candidates
        .map(|item_id| {
            let instance = instances.get(item_id).ok_or(WorldError::InvalidEntity)?;
            let prototype = prototypes
                .get(&instance.prototype_id)
                .ok_or(WorldError::InvalidEntity)?;
            Ok((item_id, prototype))
        })
        .collect::<Result<Vec<_>, _>>()?;

    if let Some((item_id, _)) = candidate_items
        .iter()
        .find(|(item_id, _)| opaque_id(observer_salt, "item", item_id.as_str()) == query)
    {
        return Ok((*item_id).clone());
    }

    let exact = candidate_items
        .iter()
        .copied()
        .filter(|(_, item)| item_matches(item, &query, false))
        .collect::<Vec<_>>();
    if exact.len() == 1 {
        return Ok(exact[0].0.clone());
    }
    if exact.len() > 1 {
        return Err(ambiguous_item_error(query, &exact));
    }

    let prefix = candidate_items
        .iter()
        .copied()
        .filter(|(_, item)| item_matches(item, &query, true))
        .collect::<Vec<_>>();
    match prefix.as_slice() {
        [(item_id, _)] => Ok((*item_id).clone()),
        [] => Err(WorldError::ItemNotFound(query)),
        matches => Err(ambiguous_item_error(query, matches)),
    }
}

fn item_matches(item: &ItemPrototype, query: &str, prefix: bool) -> bool {
    let matches = |term: &str| {
        if prefix {
            term.to_ascii_lowercase().starts_with(query)
        } else {
            term.eq_ignore_ascii_case(query)
        }
    };
    matches(&item.name) || item.aliases.iter().any(|alias| matches(alias))
}

fn ambiguous_item_error(query: String, items: &[(&ItemInstanceId, &ItemPrototype)]) -> WorldError {
    let mut names = items
        .iter()
        .map(|(_, item)| item.name.clone())
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
        | GameplayEvent::LootGenerated { room_id, .. }
        | GameplayEvent::LootRightsChanged { room_id, .. }
        | GameplayEvent::LootGenerationSuppressed { room_id, .. }
        | GameplayEvent::LootDespawned { room_id, .. }
        | GameplayEvent::LootEncounterCancelled { room_id, .. }
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
        | GameplayEvent::RewardOpened { room_id, .. }
        | GameplayEvent::CharacterHealed { room_id, .. }
        | GameplayEvent::CharacterRestored { room_id, .. }
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
        | GameplayEvent::RewardOpened { character_id, .. }
        | GameplayEvent::CharacterHealed { character_id, .. }
        | GameplayEvent::CharacterRestored { character_id, .. }
        | GameplayEvent::EffectApplied { character_id, .. }
        | GameplayEvent::EffectExpired { character_id, .. } => Some(*character_id),
        GameplayEvent::TickAdvanced { .. }
        | GameplayEvent::NpcSpawned { .. }
        | GameplayEvent::LootDropped { .. }
        | GameplayEvent::LootGenerated { .. }
        | GameplayEvent::LootRightsChanged { .. }
        | GameplayEvent::LootGenerationSuppressed { .. }
        | GameplayEvent::LootDespawned { .. }
        | GameplayEvent::LootEncounterCancelled { .. }
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
        | GameplayEvent::LootGenerated { npc_id, .. }
        | GameplayEvent::LootGenerationSuppressed { npc_id, .. }
        | GameplayEvent::LootEncounterCancelled { npc_id, .. }
        | GameplayEvent::CharacterDefeated { npc_id, .. }
        | GameplayEvent::NpcRespawned { npc_id, .. } => Some(npc_id),
        GameplayEvent::TickAdvanced { .. }
        | GameplayEvent::ExperienceGained { .. }
        | GameplayEvent::CharacterLeveled { .. }
        | GameplayEvent::CharacterPromoted { .. }
        | GameplayEvent::TalentRankAcquired { .. }
        | GameplayEvent::LootRightsChanged { .. }
        | GameplayEvent::LootDespawned { .. }
        | GameplayEvent::CharacterCollapsed { .. }
        | GameplayEvent::CharacterRecovered { .. }
        | GameplayEvent::ItemEquipped { .. }
        | GameplayEvent::ItemUnequipped { .. }
        | GameplayEvent::ItemUsed { .. }
        | GameplayEvent::RewardOpened { .. }
        | GameplayEvent::CharacterHealed { .. }
        | GameplayEvent::CharacterRestored { .. }
        | GameplayEvent::EffectApplied { .. }
        | GameplayEvent::EffectExpired { .. } => None,
    }
}

fn gameplay_event_item(event: &GameplayEvent) -> Option<&ItemInstanceId> {
    match event {
        GameplayEvent::LootDropped { item_id, .. }
        | GameplayEvent::LootRightsChanged { item_id, .. }
        | GameplayEvent::ItemEquipped { item_id, .. }
        | GameplayEvent::ItemUnequipped { item_id, .. }
        | GameplayEvent::ItemUsed { item_id, .. } => Some(item_id),
        GameplayEvent::RewardOpened { source_item_id, .. } => Some(source_item_id),
        GameplayEvent::TickAdvanced { .. }
        | GameplayEvent::NpcSpawned { .. }
        | GameplayEvent::CombatStarted { .. }
        | GameplayEvent::AbilityUsed { .. }
        | GameplayEvent::NpcAbilityUsed { .. }
        | GameplayEvent::NpcDamaged { .. }
        | GameplayEvent::CharacterDamaged { .. }
        | GameplayEvent::NpcDefeated { .. }
        | GameplayEvent::LootGenerated { .. }
        | GameplayEvent::LootGenerationSuppressed { .. }
        | GameplayEvent::LootDespawned { .. }
        | GameplayEvent::LootEncounterCancelled { .. }
        | GameplayEvent::ExperienceGained { .. }
        | GameplayEvent::CharacterLeveled { .. }
        | GameplayEvent::CharacterPromoted { .. }
        | GameplayEvent::TalentRankAcquired { .. }
        | GameplayEvent::CharacterDefeated { .. }
        | GameplayEvent::CharacterCollapsed { .. }
        | GameplayEvent::CharacterRecovered { .. }
        | GameplayEvent::NpcRespawned { .. }
        | GameplayEvent::CharacterHealed { .. }
        | GameplayEvent::CharacterRestored { .. }
        | GameplayEvent::EffectApplied { .. }
        | GameplayEvent::EffectExpired { .. } => None,
    }
}

fn project_gameplay_event(
    salt: [u8; 32],
    viewer_id: CharacterId,
    event: &GameplayEvent,
) -> (String, Option<String>, String, Option<String>) {
    let character_id = |id: CharacterId| opaque_id(salt, "character", &id.0.to_string());
    let npc_id = |id: &str| opaque_id(salt, "npc", id);
    let item_id = |id: &ItemInstanceId| opaque_id(salt, "item", id.as_str());
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
        GameplayEvent::LootGenerated {
            npc_id: npc,
            npc_name,
            item_ids,
            ..
        } => (
            npc_id(npc),
            None,
            format!(
                "{npc_name} yields {} generated loot unit(s).",
                item_ids.len()
            ),
            None,
        ),
        GameplayEvent::LootRightsChanged {
            item_id: item,
            summary,
            ..
        } => (item_id(item), None, summary.clone(), None),
        GameplayEvent::LootGenerationSuppressed {
            npc_id: npc,
            npc_name,
            reason,
            ..
        } => (
            npc_id(npc),
            None,
            format!("Loot from {npc_name} is suppressed: {reason}."),
            None,
        ),
        GameplayEvent::LootDespawned { item_ids, room_id } => (
            opaque_id(salt, "room", room_id),
            None,
            format!("{} unclaimed loot unit(s) fade away.", item_ids.len()),
            None,
        ),
        GameplayEvent::LootEncounterCancelled {
            npc_id: npc,
            npc_name,
            ..
        } => (
            npc_id(npc),
            None,
            format!("The loot encounter with {npc_name} is cancelled."),
            None,
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
        GameplayEvent::RewardOpened {
            character_id: character,
            character_name,
            source_item_id,
            source_item_name,
            item_names,
            ..
        } => (
            character_id(*character),
            None,
            if viewer_id == *character {
                format!(
                    "Abres {source_item_name} y obtienes: {}.",
                    item_names.join(", ")
                )
            } else {
                format!("{character_name} abre {source_item_name}.")
            },
            Some(item_id(source_item_id)),
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
        GameplayEvent::CharacterRestored {
            character_id: character,
            character_name,
            ..
        } => (
            character_id(*character),
            None,
            if viewer_id == *character {
                "You are fully restored by staff authority.".into()
            } else {
                format!("{character_name} is fully restored.")
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

fn opaque_id(observer_salt: [u8; 32], kind: &str, domain_id: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(b"tiamud-view-v2");
    hasher.update(observer_salt);
    hasher.update((kind.len() as u64).to_be_bytes());
    hasher.update(kind.as_bytes());
    hasher.update((domain_id.len() as u64).to_be_bytes());
    hasher.update(domain_id.as_bytes());
    let digest = hasher.finalize();
    let token = u64::from_be_bytes(
        digest[..8]
            .try_into()
            .expect("a SHA-256 digest always contains eight bytes"),
    );
    format!("o{token:016x}")
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

    fn item_prototype(
        id: &str,
        name: &str,
        description: &str,
        aliases: &[&str],
    ) -> ItemPrototypeDefinition {
        ItemPrototypeDefinition {
            prototype_id: id.into(),
            name: name.into(),
            description: description.into(),
            aliases: aliases.iter().map(|alias| (*alias).into()).collect(),
            equipment_slot: None,
            attack_bonus: 0,
            heal_amount: 0,
            consumable: false,
            effect: None,
            weight_grams: 1_000,
            slot_cost: 1,
            rarity: ItemRarity::Common,
            item_level: 1,
            stack_policy: ItemStackPolicy::Unique,
            max_stack_units: 1,
            container: None,
            transfer_policy: ItemTransferPolicy::Transferable,
            sealed_loot_table_id: None,
        }
    }

    fn item_instance(
        id: &str,
        prototype_id: &str,
        location: InitialItemLocation,
    ) -> InitialItemInstanceDefinition {
        InitialItemInstanceDefinition {
            instance_id: id.into(),
            prototype_id: prototype_id.into(),
            location,
        }
    }

    fn sealed_reward_definition() -> WorldDefinition {
        let mut seal = item_prototype(
            "reward:sealed-cache",
            "sealed cache",
            "A reward earned in combat and sealed against rerolls.",
            &["cache", "seal"],
        );
        seal.consumable = true;
        seal.transfer_policy = ItemTransferPolicy::CharacterBound;
        seal.sealed_loot_table_id = Some("reward:cache-table".into());
        let mut sword = item_prototype(
            "reward:sword",
            "iron sword",
            "A deterministic reward blade.",
            &["sword", "blade"],
        );
        sword.equipment_slot = Some(EquipmentSlot::Weapon);
        sword.attack_bonus = 1;
        sword.slot_cost = 2;
        WorldDefinition {
            start_room_id: "test:start".into(),
            item_prototypes: vec![seal, sword],
            initial_item_instances: Vec::new(),
            rooms: vec![RoomDefinition {
                stable_id: "test:start".into(),
                title: "Reward Test".into(),
                prose: vec!["A bounded reward test room.".into()],
                image: None,
                audio: None,
                item_ids: Vec::new(),
                exits: Vec::new(),
            }],
        }
    }

    fn sealed_reward_world(seed: u64) -> (World, SessionId, ItemInstanceId) {
        let (mut world, session) = connect_definition(sealed_reward_definition());
        world.set_rng_seed(seed);
        world
            .configure_affixes(
                vec![AffixDefinition {
                    id: "reward:keen".into(),
                    label: "afilada".into(),
                    position: AffixPosition::Prefix,
                    attack_bonus_delta: 2,
                    heal_amount_delta: 0,
                }],
                vec![AffixPoolDefinition {
                    id: "reward:weapon-affix".into(),
                    entries: vec![AffixPoolEntry {
                        kind: AffixPoolEntryKind::Affix,
                        id: "reward:keen-outcome".into(),
                        weight: 3,
                        affix_id: Some("reward:keen".into()),
                    }],
                }],
            )
            .unwrap();
        world
            .configure_loot(
                vec![LootTableDefinition {
                    id: "reward:cache-table".into(),
                    digest_sha256: "d".repeat(64),
                    guaranteed: vec![LootGuaranteedDefinition {
                        id: "reward:sword-outcome".into(),
                        item_prototype_id: "reward:sword".into(),
                        quantity: 1,
                        affix_pool_id: Some("reward:weapon-affix".into()),
                    }],
                    pools: vec![LootPoolDefinition {
                        id: "reward:empty-draw".into(),
                        rolls: 2,
                        entries: vec![LootEntryDefinition {
                            kind: LootEntryKind::Nothing,
                            id: "reward:nothing".into(),
                            weight: 2,
                            item_prototype_id: None,
                            min_quantity: None,
                            max_quantity: None,
                            affix_pool_id: None,
                        }],
                    }],
                }],
                Vec::new(),
            )
            .unwrap();
        let source = world
            .create_staged_item_instance("reward:sealed-cache")
            .unwrap();
        let instance = world.item_instances.get_mut(&source).unwrap();
        instance.location = SharedItemLocation::Carried(CharacterId(701));
        instance.binding = ItemBinding::Character {
            character_id: CharacterId(701),
        };
        world.item_provenance.insert(
            source.clone(),
            ItemProvenance::NpcLoot {
                drop_id: LootDropId(1),
                npc_spawn_id: "reward:earned-source".into(),
                loot_table_id: "reward:earned-table".into(),
                table_digest_sha256: "e".repeat(64),
                entry_id: "reward:earned-cache".into(),
                pool_id: Some("reward:earned-pool".into()),
                sealed_loot_table_id: Some("reward:cache-table".into()),
                roll_index: 0,
                quantity_index: 0,
                generated_tick: 0,
            },
        );
        world.next_loot_drop_id = LootDropId(2);
        (world, session, source)
    }

    fn redact_and_validate_opaque_item_refs(mut text: &str) -> String {
        const PREFIX: &str = "; ref o";
        let mut redacted = String::with_capacity(text.len());
        while let Some(index) = text.find(PREFIX) {
            redacted.push_str(&text[..index]);
            let suffix = &text[index + PREFIX.len()..];
            assert!(suffix.len() >= 16, "truncated opaque reference in {text:?}");
            let token = &suffix[..16];
            assert!(
                token.bytes().all(|byte| byte.is_ascii_hexdigit()),
                "invalid opaque reference in {text:?}"
            );
            text = &suffix[16..];
        }
        redacted.push_str(text);
        redacted
    }

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

    fn runtime_snapshot_json_for_version(
        snapshot: &WorldRuntimeSnapshot,
        version: u32,
    ) -> serde_json::Value {
        let mut value = serde_json::to_value(snapshot).unwrap();
        value["snapshotVersion"] = serde_json::json!(version);
        if version <= 6 {
            value.as_object_mut().unwrap().remove("nextRewardOpenId");
            for instance in value["itemInstances"].as_object_mut().unwrap().values_mut() {
                instance.as_object_mut().unwrap().remove("itemAffixes");
            }
            for provenance in value["itemProvenance"]
                .as_object_mut()
                .unwrap()
                .values_mut()
            {
                provenance
                    .as_object_mut()
                    .unwrap()
                    .remove("sealedLootTableId");
            }
        }
        if version <= 4 {
            let root = value.as_object_mut().unwrap();
            root.remove("nextTradeId");
            root.remove("trades");
        }
        if version <= 5 {
            let root = value.as_object_mut().unwrap();
            root.remove("nextLootDropId");
            root.remove("itemProvenance");
            root.remove("lootEncounters");
            root.remove("groundLoot");
            root.remove("partyLootCursors");
        }
        match version {
            1 => {
                let instances = value
                    .as_object_mut()
                    .unwrap()
                    .remove("itemInstances")
                    .unwrap();
                let locations = instances
                    .as_object()
                    .unwrap()
                    .iter()
                    .map(|(item_id, instance)| (item_id.clone(), instance["location"].clone()))
                    .collect::<serde_json::Map<_, _>>();
                let root = value.as_object_mut().unwrap();
                root.remove("nextItemInstanceSequence");
                root.insert("itemLocations".into(), serde_json::Value::Object(locations));
            }
            2 | 3 => {
                for instance in value["itemInstances"].as_object_mut().unwrap().values_mut() {
                    instance.as_object_mut().unwrap().remove("binding");
                }
            }
            4..=7 => {}
            _ => panic!("test helper only supports runtime snapshot v1-v7"),
        }
        value
    }

    fn first_runtime_npc(value: &mut serde_json::Value) -> &mut serde_json::Value {
        value["npcs"]
            .as_object_mut()
            .unwrap()
            .values_mut()
            .next()
            .expect("runtime snapshot fixture contains an NPC")
    }

    fn connect_definition(definition: WorldDefinition) -> (World, SessionId) {
        let mut world = World::from_definition(definition).unwrap();
        let session = SessionId(71);
        world
            .connect(
                session,
                CharacterSnapshot {
                    id: CharacterId(701),
                    account_id: AccountId(70),
                    name: "Stack Tester".into(),
                    room_id: "test:start".into(),
                    item_locations: BTreeMap::new(),
                },
            )
            .unwrap();
        (world, session)
    }

    fn policy_definition() -> WorldDefinition {
        let mut bag = item_prototype(
            "policy:bag",
            "traveller bag",
            "A bag used to test ownership policy.",
            &["bag"],
        );
        bag.container = Some(ItemContainerDefinition {
            max_slots: 20,
            max_weight_grams: 20_000,
        });
        let mut chest = item_prototype(
            "policy:chest",
            "stone chest",
            "A room-accessible container.",
            &["chest"],
        );
        chest.container = Some(ItemContainerDefinition {
            max_slots: 20,
            max_weight_grams: 20_000,
        });
        let mut clasp = item_prototype(
            "policy:clasp",
            "wolf clasp",
            "A character-bound clasp.",
            &["clasp"],
        );
        clasp.transfer_policy = ItemTransferPolicy::CharacterBound;
        clasp.weight_grams = 10;
        let mut token = item_prototype(
            "policy:token",
            "road token",
            "A non-transferable token.",
            &["token"],
        );
        token.transfer_policy = ItemTransferPolicy::NonTransferable;
        token.weight_grams = 10;
        let mut scripted = item_prototype(
            "policy:scripted",
            "scripted sigil",
            "A staged character-bound grant.",
            &["sigil"],
        );
        scripted.transfer_policy = ItemTransferPolicy::CharacterBound;
        scripted.weight_grams = 10;
        WorldDefinition {
            start_room_id: "test:start".into(),
            item_prototypes: vec![bag, chest, clasp, token, scripted],
            initial_item_instances: vec![
                item_instance(
                    "policy:bag-instance",
                    "policy:bag",
                    InitialItemLocation::Room("test:start".into()),
                ),
                item_instance(
                    "policy:clasp-instance",
                    "policy:clasp",
                    InitialItemLocation::Container("policy:bag-instance".into()),
                ),
                item_instance(
                    "policy:token-instance",
                    "policy:token",
                    InitialItemLocation::Room("test:start".into()),
                ),
                item_instance(
                    "policy:chest-instance",
                    "policy:chest",
                    InitialItemLocation::Room("test:start".into()),
                ),
                item_instance(
                    "policy:scripted-instance",
                    "policy:scripted",
                    InitialItemLocation::Room("test:start".into()),
                ),
            ],
            rooms: vec![RoomDefinition {
                stable_id: "test:start".into(),
                title: "Start".into(),
                prose: vec!["A test room.".into()],
                image: None,
                audio: None,
                item_ids: Vec::new(),
                exits: Vec::new(),
            }],
        }
    }

    fn give_definition() -> WorldDefinition {
        let mut token = item_prototype(
            "give:token",
            "moon token",
            "A fungible transferable token.",
            &["token"],
        );
        token.stack_policy = ItemStackPolicy::Fungible;
        token.max_stack_units = 2;
        token.weight_grams = 100;
        WorldDefinition {
            start_room_id: "give:start".into(),
            item_prototypes: vec![token],
            initial_item_instances: (0..3)
                .map(|index| {
                    item_instance(
                        &format!("give:token-{index}"),
                        "give:token",
                        InitialItemLocation::Room("give:start".into()),
                    )
                })
                .collect(),
            rooms: vec![
                RoomDefinition {
                    stable_id: "give:start".into(),
                    title: "Start".into(),
                    prose: vec!["A shared test room.".into()],
                    image: None,
                    audio: None,
                    item_ids: Vec::new(),
                    exits: vec![ExitDefinition {
                        direction: Direction::North,
                        target_room_id: "give:remote".into(),
                    }],
                },
                RoomDefinition {
                    stable_id: "give:remote".into(),
                    title: "Remote".into(),
                    prose: vec!["A distant test room.".into()],
                    image: None,
                    audio: None,
                    item_ids: Vec::new(),
                    exits: vec![ExitDefinition {
                        direction: Direction::South,
                        target_room_id: "give:start".into(),
                    }],
                },
            ],
        }
    }

    fn phase_two_world(seed: u64, npc_strength: i32, npc_health: i32) -> (World, SessionId) {
        let mut world = World::from_definition_with_staged_items(
            WorldDefinition {
                start_room_id: "game:start".into(),
                item_prototypes: vec![
                    item_prototype(
                        "game:sword",
                        "iron sword",
                        "A balanced beginner's blade.",
                        &["sword", "blade"],
                    ),
                    item_prototype(
                        "game:potion",
                        "red potion",
                        "A restorative draught.",
                        &["potion"],
                    ),
                    item_prototype(
                        "game:fang",
                        "silver fang",
                        "Proof of victory over the guardian.",
                        &["fang"],
                    ),
                ],
                initial_item_instances: vec![
                    item_instance(
                        "game:sword",
                        "game:sword",
                        InitialItemLocation::Room("game:start".into()),
                    ),
                    item_instance(
                        "game:potion",
                        "game:potion",
                        InitialItemLocation::Room("game:start".into()),
                    ),
                    item_instance("game:fang", "game:fang", InitialItemLocation::Staged),
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
                loot_item_ids: vec![ItemInstanceId("game:fang".into())],
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
            let prototype = world.item_prototypes.get_mut(item_id).unwrap();
            prototype.equipment_slot = Some(slot);
            prototype.attack_bonus = i32::MAX;
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
            parse_action("open o0123456789abcdef"),
            Ok(PlayerAction::OpenReward("o0123456789abcdef".into()))
        );
        assert_eq!(
            parse_action("reward odds o0123456789abcdef"),
            Ok(PlayerAction::RewardOdds {
                target: "o0123456789abcdef".into(),
                page: 1,
            })
        );
        assert_eq!(
            parse_action("reward odds sealed cache 12"),
            Ok(PlayerAction::RewardOdds {
                target: "sealed cache".into(),
                page: 12,
            })
        );
        assert!(parse_action("reward odds sealed cache 0").is_err());
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
            parse_action("role Aria moderator operational promotion"),
            Ok(PlayerAction::Social(SocialAction::SetRole {
                target: "Aria".into(),
                role: AccountRole::Moderator,
                reason: "operational promotion".into(),
            }))
        );
        assert_eq!(
            parse_action("unmute Griefer appeal accepted"),
            Ok(PlayerAction::Social(SocialAction::Unmute {
                target: "Griefer".into(),
                reason: "appeal accepted".into(),
            }))
        );
    }

    #[test]
    fn parses_staff_commands_with_closed_values_and_mandatory_reasons() {
        assert_eq!(
            parse_action("staff inspect Aria | investigate stuck character"),
            Ok(PlayerAction::Staff(StaffAction::Inspect {
                target: "Aria".into(),
                reason: "investigate stuck character".into(),
            }))
        );
        assert_eq!(
            parse_action("staff announce Server restart in ten minutes | planned maintenance"),
            Ok(PlayerAction::Staff(StaffAction::Announce {
                message: "Server restart in ten minutes".into(),
                reason: "planned maintenance".into(),
            }))
        );
        assert_eq!(
            parse_action("staff resolve 7 action_taken verified harassment"),
            Ok(PlayerAction::Staff(StaffAction::ResolveReport {
                report_id: 7,
                resolution: StaffReportResolution::ActionTaken,
                reason: "verified harassment".into(),
            }))
        );
        assert_eq!(
            parse_action("staff profile Aria helper Angel of Welcome | onboarding assignment"),
            Ok(PlayerAction::Staff(StaffAction::SetProfile {
                target: "Aria".into(),
                profile: Some(StaffProfileKey::Helper),
                lore_title: Some("Angel of Welcome".into()),
                reason: "onboarding assignment".into(),
            }))
        );
        assert_eq!(
            parse_action("staff capability grant Aria reports.view | support rotation"),
            Ok(PlayerAction::Staff(StaffAction::SetCapability {
                target: "Aria".into(),
                capability: StaffCapability::ReportsView,
                enabled: true,
                reason: "support rotation".into(),
            }))
        );
        for invalid in [
            "staff inspect Aria",
            "staff announce notice without reason",
            "staff resolve 0 action_taken invalid id",
            "staff resolve 7 arbitrary invalid resolution",
            "staff capability grant Aria unknown | invalid capability",
        ] {
            assert!(parse_action(invalid).is_err(), "{invalid}");
        }
    }

    #[test]
    fn staff_profiles_use_explicit_ranks_and_closed_default_bundles() {
        assert!(
            StaffProfileKey::Helper.authority_rank() < StaffProfileKey::Moderator.authority_rank()
        );
        assert!(
            StaffProfileKey::Administrator.authority_rank()
                < StaffProfileKey::Implementor.authority_rank()
        );
        assert_eq!(StaffProfileKey::Builder.default_capabilities(), &[]);
        assert!(
            StaffProfileKey::Moderator
                .default_capabilities()
                .contains(&StaffCapability::ModerationBan)
        );
        assert_eq!(
            StaffProfileKey::Administrator.default_capabilities(),
            &StaffCapability::ALL
        );
    }

    #[test]
    fn staff_relocation_and_restore_are_checkpoint_safe_and_project_no_stable_ids() {
        let mut world = World::demo();
        let actor = SessionId(501);
        let target = SessionId(502);
        world
            .connect(
                actor,
                CharacterSnapshot {
                    id: CharacterId(501),
                    account_id: AccountId(501),
                    name: "Keeper".into(),
                    room_id: "demo:lantern-square".into(),
                    item_locations: BTreeMap::new(),
                },
            )
            .unwrap();
        world
            .connect(
                target,
                CharacterSnapshot {
                    id: CharacterId(502),
                    account_id: AccountId(502),
                    name: "Aria".into(),
                    room_id: "demo:north-gate".into(),
                    item_locations: BTreeMap::new(),
                },
            )
            .unwrap();

        let inspection = world.staff_inspection_as_plain_text(target).unwrap();
        assert!(inspection.contains("Character: Aria"));
        assert!(!inspection.contains("demo:north-gate"));
        assert!(!inspection.contains("account #"));

        let checkpoint = world.checkpoint();
        let relocated = world.staff_goto(actor, target).unwrap();
        assert!(matches!(
            relocated.domain_events.as_slice(),
            [DomainEvent::CharacterRelocated { actor_id, .. }] if *actor_id == CharacterId(501)
        ));
        assert_eq!(
            world.character_snapshot(actor).unwrap().room_id,
            "demo:north-gate"
        );
        world.restore_checkpoint(checkpoint);
        assert_eq!(
            world.character_snapshot(actor).unwrap().room_id,
            "demo:lantern-square"
        );

        let mut gameplay = world.character_gameplay_snapshot(target).unwrap();
        gameplay.vitals.health = 0;
        gameplay.recovery_at_tick = Some(world.logical_tick().saturating_add(3));
        world
            .restore_character_gameplay(CharacterId(502), gameplay)
            .unwrap();
        let restored = world.staff_restore(actor, target).unwrap();
        assert!(matches!(
            restored.domain_events.as_slice(),
            [DomainEvent::Gameplay {
                event: GameplayEvent::CharacterRestored { character_id, .. }
            }] if *character_id == CharacterId(502)
        ));
        let restored_gameplay = world.character_gameplay_snapshot(target).unwrap();
        assert_eq!(
            restored_gameplay.vitals.health,
            restored_gameplay.vitals.max_health
        );
        assert_eq!(restored_gameplay.recovery_at_tick, None);
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
        let first_scene = world.scene_for(first_session).unwrap();
        let snapshot = world.character_snapshot(first_session).unwrap();
        world.disconnect(first_session);

        let scene = world.connect(SessionId(9), snapshot.clone()).unwrap();
        assert_ne!(scene.view_id, "demo:lantern-square");
        assert_ne!(scene.view_id, first_scene.view_id);
        assert_ne!(scene.items[0].id, first_scene.items[0].id);
        assert_eq!(scene.title, "Lantern Square");

        let mut restarted = World::demo();
        let restarted_scene = restarted.connect(SessionId(1), snapshot).unwrap();
        assert_ne!(restarted_scene.view_id, first_scene.view_id);
        assert_ne!(restarted_scene.items[0].id, first_scene.items[0].id);
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
            item_prototypes: Vec::new(),
            initial_item_instances: Vec::new(),
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
            item_prototypes: Vec::new(),
            initial_item_instances: Vec::new(),
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
            item_prototypes: Vec::new(),
            initial_item_instances: Vec::new(),
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
            item_prototypes: Vec::new(),
            initial_item_instances: Vec::new(),
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
        let inventory = world
            .apply(session, PlayerAction::Inventory)
            .unwrap()
            .output
            .unwrap();
        assert_eq!(
            redact_and_validate_opaque_item_refs(&inventory),
            "Capacity: 1/24 slots; 1000/50000 g.\r\nYou are carrying:\r\n- 1 × blue-glass lantern [common; level 1; 1000 g each; 1 stack slot; 1000 g total; policy transferable]\r\n"
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
            item_prototypes: vec![
                item_prototype("test:blue", "blue lantern", "Blue.", &["lantern"]),
                item_prototype("test:brass", "brass lantern", "Brass.", &["lantern"]),
            ],
            initial_item_instances: vec![
                item_instance(
                    "test:blue-lantern",
                    "test:blue",
                    InitialItemLocation::Room("test:start".into()),
                ),
                item_instance(
                    "test:brass-lantern",
                    "test:brass",
                    InitialItemLocation::Room("test:start".into()),
                ),
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
            "Capacity: 0/24 slots; 0/50000 g.\r\nYou are carrying nothing.\r\n"
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
            "Capacity: 0/24 slots; 0/50000 g.\r\nYou are carrying nothing.\r\n"
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
        let inventory = carried_world
            .inventory_as_plain_text(owner_session)
            .unwrap();
        assert_eq!(
            redact_and_validate_opaque_item_refs(&inventory),
            "Capacity: 1/24 slots; 1000/50000 g.\r\nYou are carrying:\r\n- 1 × blue-glass lantern [common; level 1; 1000 g each; 1 stack slot; 1000 g total; policy transferable]\r\n"
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
            "Capacity: 0/24 slots; 0/50000 g.\r\nYou are carrying nothing.\r\n"
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
        let inventory = world.inventory_as_plain_text(actor).unwrap();
        assert_eq!(
            redact_and_validate_opaque_item_refs(&inventory),
            "Capacity: 1/24 slots; 1000/50000 g.\r\nYou are carrying:\r\n- 1 × blue-glass lantern [common; level 1; 1000 g each; 1 stack slot; 1000 g total; policy transferable]\r\n"
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
            ItemInstanceId("game:sword".into())
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
        let resumed_inventory = resumed.inventory_as_plain_text(resumed_session).unwrap();
        let original_inventory = original.inventory_as_plain_text(session).unwrap();
        assert_ne!(resumed_inventory, original_inventory);
        assert_eq!(
            redact_and_validate_opaque_item_refs(&resumed_inventory),
            redact_and_validate_opaque_item_refs(&original_inventory)
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
    fn hidden_item_policy_targets_one_instance_not_its_reusable_prototype() {
        let mut world = World::from_definition(WorldDefinition {
            start_room_id: "test:start".into(),
            item_prototypes: vec![item_prototype(
                "test:lantern",
                "blue lantern",
                "A cold blue lantern.",
                &["lantern"],
            )],
            initial_item_instances: vec![item_instance(
                "test:lantern:one",
                "test:lantern",
                InitialItemLocation::Room("test:start".into()),
            )],
            rooms: vec![RoomDefinition {
                stable_id: "test:start".into(),
                title: "Start".into(),
                prose: Vec::new(),
                image: None,
                audio: None,
                exits: Vec::new(),
                item_ids: Vec::new(),
            }],
        })
        .unwrap();
        assert!(matches!(
            world.configure_hidden_item("test:lantern", 100),
            Err(WorldError::InvalidGameplayDefinition)
        ));
        world
            .configure_hidden_item("test:lantern:one", 100)
            .unwrap();
        let session = SessionId(777);
        world
            .connect(
                session,
                CharacterSnapshot {
                    id: CharacterId(777),
                    account_id: AccountId(777),
                    name: "Observer".into(),
                    room_id: "test:start".into(),
                    item_locations: BTreeMap::new(),
                },
            )
            .unwrap();

        assert!(world.scene_for(session).unwrap().items.is_empty());
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
        let room = || RoomDefinition {
            stable_id: "test:start".into(),
            title: "Start".into(),
            prose: Vec::new(),
            image: None,
            audio: None,
            exits: Vec::new(),
            item_ids: Vec::new(),
        };
        let item = item_prototype("test:item", "item", "An item.", &[]);

        assert!(matches!(
            World::from_definition(WorldDefinition {
                start_room_id: "test:start".into(),
                item_prototypes: Vec::new(),
                initial_item_instances: vec![item_instance(
                    "test:instance",
                    "missing",
                    InitialItemLocation::Room("test:start".into()),
                )],
                rooms: vec![room()],
            }),
            Err(WorldBuildError::UnknownItemPrototype(id)) if id == "missing"
        ));
        assert!(
            World::from_definition(WorldDefinition {
                start_room_id: "test:start".into(),
                item_prototypes: vec![item.clone()],
                initial_item_instances: Vec::new(),
                rooms: vec![room()],
            })
            .is_ok()
        );
        assert!(matches!(
            World::from_definition(WorldDefinition {
                start_room_id: "test:start".into(),
                item_prototypes: vec![item.clone()],
                initial_item_instances: vec![
                    item_instance("duplicate", "test:item", InitialItemLocation::Staged),
                    item_instance("duplicate", "test:item", InitialItemLocation::Staged),
                ],
                rooms: vec![room()],
            }),
            Err(WorldBuildError::DuplicateItemId(id)) if id == "duplicate"
        ));

        assert!(matches!(
            World::from_definition_with_staged_items(
                WorldDefinition {
                    start_room_id: "test:start".into(),
                    item_prototypes: vec![item.clone()],
                    initial_item_instances: vec![item_instance(
                        "test:item-instance",
                        "test:item",
                        InitialItemLocation::Room("test:start".into()),
                    )],
                    rooms: vec![room()],
                },
                vec!["test:item-instance".into()],
            ),
            Err(WorldBuildError::ItemPlacedAndStaged { item_id, .. })
                if item_id == "test:item-instance"
        ));

        let mut weightless = item;
        weightless.weight_grams = 0;
        assert!(
            World::from_definition(WorldDefinition {
                start_room_id: "test:start".into(),
                item_prototypes: vec![weightless],
                initial_item_instances: Vec::new(),
                rooms: vec![room()],
            })
            .is_ok()
        );

        let mut reserved_transfer = item_prototype("test:bound", "bound", "Bound.", &[]);
        reserved_transfer.transfer_policy = ItemTransferPolicy::CharacterBound;
        assert!(
            World::from_definition(WorldDefinition {
                start_room_id: "test:start".into(),
                item_prototypes: vec![reserved_transfer],
                initial_item_instances: Vec::new(),
                rooms: vec![room()],
            })
            .is_ok()
        );
    }

    #[test]
    fn repeatable_instances_are_distinct_opaque_and_capacity_failures_are_atomic() {
        let mut prototype = item_prototype(
            "test:repeated",
            "etched token",
            "One concrete token from a reusable design.",
            &["token"],
        );
        prototype.weight_grams = 600;
        prototype.rarity = ItemRarity::Rare;
        prototype.item_level = 7;
        let mut world = World::from_definition(WorldDefinition {
            start_room_id: "test:start".into(),
            item_prototypes: vec![prototype],
            initial_item_instances: vec![
                item_instance(
                    "durable:first",
                    "test:repeated",
                    InitialItemLocation::Room("test:start".into()),
                ),
                item_instance(
                    "durable:second",
                    "test:repeated",
                    InitialItemLocation::Room("test:start".into()),
                ),
            ],
            rooms: vec![RoomDefinition {
                stable_id: "test:start".into(),
                title: "Start".into(),
                prose: Vec::new(),
                image: None,
                audio: None,
                exits: Vec::new(),
                item_ids: Vec::new(),
            }],
        })
        .unwrap();
        let session = SessionId(500);
        world
            .connect(
                session,
                CharacterSnapshot {
                    id: CharacterId(500),
                    account_id: AccountId(500),
                    name: "Carrier".into(),
                    room_id: "test:start".into(),
                    item_locations: BTreeMap::new(),
                },
            )
            .unwrap();
        let mut gameplay = world.character_gameplay_snapshot(session).unwrap();
        gameplay.inventory_capacity = InventoryCapacity {
            max_slots: 1,
            max_weight_grams: 1_000,
        };
        world
            .restore_character_gameplay(CharacterId(500), gameplay)
            .unwrap();

        let scene = world.scene_for(session).unwrap();
        assert_eq!(scene.items.len(), 2);
        assert_ne!(scene.items[0].id, scene.items[1].id);
        assert!(scene.items.iter().all(|item| item.rarity == "rare"));
        let scene_json = serde_json::to_string(&scene).unwrap();
        assert!(!scene_json.contains("durable:first"));
        assert!(!scene_json.contains("durable:second"));
        assert!(!scene_json.contains("test:repeated"));

        world
            .apply(
                session,
                PlayerAction::Take(scene.items[0].id.clone().into()),
            )
            .unwrap();
        let before = world.runtime_snapshot();
        let revision = world.revision();
        let remaining = world.scene_for(session).unwrap().items[0].id.clone();
        assert!(matches!(
            world.apply(session, PlayerAction::Take(remaining.into())),
            Err(WorldError::InventoryCapacityExceeded)
        ));
        assert_eq!(world.runtime_snapshot(), before);
        assert_eq!(world.revision(), revision);
        let text = world.inventory_as_plain_text(session).unwrap();
        assert!(text.contains("1/1 slots"));
        assert!(text.contains(
            "[rare; level 7; 600 g each; 1 stack slot; 600 g total; policy transferable; ref o"
        ));
        assert!(redact_and_validate_opaque_item_refs(&text).contains(
            "[rare; level 7; 600 g each; 1 stack slot; 600 g total; policy transferable]"
        ));
    }

    #[test]
    fn unequip_and_equipment_swap_preflight_slot_capacity() {
        let mut bulky = item_prototype("test:bulky", "bulky blade", "Bulky.", &["bulky"]);
        bulky.equipment_slot = Some(EquipmentSlot::Weapon);
        bulky.slot_cost = 2;
        let mut light = item_prototype("test:light", "light blade", "Light.", &["light"]);
        light.equipment_slot = Some(EquipmentSlot::Weapon);
        let mut world = World::from_definition(WorldDefinition {
            start_room_id: "test:start".into(),
            item_prototypes: vec![bulky, light],
            initial_item_instances: vec![
                item_instance(
                    "test:bulky-instance",
                    "test:bulky",
                    InitialItemLocation::Room("test:start".into()),
                ),
                item_instance(
                    "test:light-instance",
                    "test:light",
                    InitialItemLocation::Room("test:start".into()),
                ),
            ],
            rooms: vec![RoomDefinition {
                stable_id: "test:start".into(),
                title: "Start".into(),
                prose: Vec::new(),
                image: None,
                audio: None,
                exits: Vec::new(),
                item_ids: Vec::new(),
            }],
        })
        .unwrap();
        let session = SessionId(501);
        world
            .connect(
                session,
                CharacterSnapshot {
                    id: CharacterId(501),
                    account_id: AccountId(501),
                    name: "Carrier".into(),
                    room_id: "test:start".into(),
                    item_locations: BTreeMap::new(),
                },
            )
            .unwrap();
        world
            .apply(session, PlayerAction::Take("bulky".into()))
            .unwrap();
        world
            .apply(session, PlayerAction::Equip("bulky".into()))
            .unwrap();
        world
            .apply(session, PlayerAction::Take("light".into()))
            .unwrap();
        let mut gameplay = world.character_gameplay_snapshot(session).unwrap();
        gameplay.inventory_capacity.max_slots = 1;
        world
            .restore_character_gameplay(CharacterId(501), gameplay)
            .unwrap();
        let before = world.runtime_snapshot();
        let gameplay_before = world.character_gameplay_snapshot(session).unwrap();
        assert!(matches!(
            world.apply(session, PlayerAction::Unequip("weapon".into())),
            Err(WorldError::InventoryCapacityExceeded)
        ));
        assert!(matches!(
            world.apply(session, PlayerAction::Equip("light".into())),
            Err(WorldError::InventoryCapacityExceeded)
        ));
        assert_eq!(world.runtime_snapshot(), before);
        assert_eq!(
            world.character_gameplay_snapshot(session).unwrap(),
            gameplay_before
        );
    }

    #[test]
    fn snapshot_v1_upgrade_event_json_and_allocator_remain_compatible() {
        let world = World::demo();
        let current = world.runtime_snapshot();
        let legacy = WorldRuntimeSnapshotV1 {
            snapshot_version: 1,
            revision: current.revision,
            logical_tick: current.logical_tick,
            rng_state: current.rng_state,
            item_locations: current
                .item_instances
                .iter()
                .map(|(id, instance)| {
                    let location = match &instance.location {
                        RuntimeItemLocation::Staged => RuntimeItemLocationV2::Staged,
                        RuntimeItemLocation::Carried { character_id } => {
                            RuntimeItemLocationV2::Carried {
                                character_id: *character_id,
                            }
                        }
                        RuntimeItemLocation::Room { room_id } => RuntimeItemLocationV2::Room {
                            room_id: room_id.clone(),
                        },
                        RuntimeItemLocation::Equipped { character_id, slot } => {
                            RuntimeItemLocationV2::Equipped {
                                character_id: *character_id,
                                slot: *slot,
                            }
                        }
                        RuntimeItemLocation::HeldByNpc { npc_id } => {
                            RuntimeItemLocationV2::HeldByNpc {
                                npc_id: npc_id.clone(),
                            }
                        }
                        RuntimeItemLocation::Consumed => RuntimeItemLocationV2::Consumed,
                        RuntimeItemLocation::Contained { .. } => {
                            panic!("v1 fixture must not contain nested items")
                        }
                    };
                    (id.0.clone(), location)
                })
                .collect(),
            npcs: current.npcs,
            adventure: current.adventure,
            progression_flags: current.progression_flags,
        };
        let encoded = serde_json::to_string(&legacy).unwrap();
        let compatible: CompatibleWorldRuntimeSnapshot = serde_json::from_str(&encoded).unwrap();
        let upgraded = compatible.into_current().unwrap();
        assert_eq!(upgraded.snapshot_version, WORLD_RUNTIME_SNAPSHOT_VERSION);
        assert_eq!(upgraded.item_instances.len(), 1);
        assert_eq!(
            upgraded.item_instances[&ItemInstanceId("demo:blue-lantern".into())].prototype_id,
            "demo:blue-lantern"
        );

        let event = DomainEvent::ItemTaken {
            actor_id: CharacterId(1),
            actor_name: "Carrier".into(),
            item_id: ItemInstanceId("demo:blue-lantern".into()),
            item_name: "blue-glass lantern".into(),
            room_id: "demo:lantern-square".into(),
        };
        let event_json = serde_json::to_string(&event).unwrap();
        assert!(
            event_json.contains("\"item_id\":\"demo:blue-lantern\""),
            "{event_json}"
        );
        assert_eq!(
            serde_json::from_str::<DomainEvent>(&event_json).unwrap(),
            event
        );

        let mut collision_prototype = item_prototype(
            "runtime:1",
            "legacy token",
            "Legacy runtime-looking identity.",
            &["token"],
        );
        collision_prototype.weight_grams = 0;
        let mut collision_world = World::from_definition(WorldDefinition {
            start_room_id: "test:start".into(),
            item_prototypes: vec![collision_prototype],
            initial_item_instances: vec![item_instance(
                "runtime:1",
                "runtime:1",
                InitialItemLocation::Staged,
            )],
            rooms: vec![RoomDefinition {
                stable_id: "test:start".into(),
                title: "Start".into(),
                prose: Vec::new(),
                image: None,
                audio: None,
                exits: Vec::new(),
                item_ids: Vec::new(),
            }],
        })
        .unwrap();
        let before = collision_world.runtime_snapshot();
        let checkpoint = collision_world.checkpoint();
        assert_eq!(
            collision_world
                .create_staged_item_instance("runtime:1")
                .unwrap(),
            ItemInstanceId("runtime:2".into())
        );
        collision_world.restore_checkpoint(checkpoint);
        assert_eq!(collision_world.runtime_snapshot(), before);
    }

    #[test]
    fn runtime_restore_merges_new_authored_instances_and_rejects_identity_drift() {
        let prototypes = || {
            vec![
                item_prototype("test:token", "token", "A token.", &["token"]),
                item_prototype("test:other", "other", "Another token.", &["other"]),
            ]
        };
        let room = || RoomDefinition {
            stable_id: "test:start".into(),
            title: "Start".into(),
            prose: Vec::new(),
            image: None,
            audio: None,
            exits: Vec::new(),
            item_ids: Vec::new(),
        };
        let build = |include_new: bool| {
            let mut instances = vec![item_instance(
                "test:token:one",
                "test:token",
                InitialItemLocation::Room("test:start".into()),
            )];
            if include_new {
                instances.push(item_instance(
                    "test:token:two",
                    "test:token",
                    InitialItemLocation::Room("test:start".into()),
                ));
            }
            World::from_definition(WorldDefinition {
                start_room_id: "test:start".into(),
                item_prototypes: prototypes(),
                initial_item_instances: instances,
                rooms: vec![room()],
            })
            .unwrap()
        };

        let old_snapshot = build(false).runtime_snapshot();
        let mut upgraded_world = build(true);
        upgraded_world
            .restore_runtime_snapshot(old_snapshot.clone())
            .unwrap();
        assert!(
            upgraded_world
                .runtime_snapshot()
                .item_instances
                .contains_key(&ItemInstanceId("test:token:two".into()))
        );

        let mut retargeted = old_snapshot.clone();
        retargeted
            .item_instances
            .get_mut(&ItemInstanceId("test:token:one".into()))
            .unwrap()
            .prototype_id = "test:other".into();
        assert!(matches!(
            build(true).restore_runtime_snapshot(retargeted),
            Err(WorldError::InvalidRuntimeSnapshot)
        ));

        let mut foreign_id = old_snapshot.clone();
        foreign_id.item_instances.insert(
            ItemInstanceId("phantom:anything".into()),
            RuntimeItemInstanceSnapshot {
                prototype_id: "test:token".into(),
                location: RuntimeItemLocation::Staged,
                binding: ItemBinding::Unbound,
                item_affixes: Vec::new(),
            },
        );
        assert!(matches!(
            build(true).restore_runtime_snapshot(foreign_id),
            Err(WorldError::InvalidRuntimeSnapshot)
        ));

        let mut runtime_authored = World::from_definition(WorldDefinition {
            start_room_id: "test:start".into(),
            item_prototypes: prototypes(),
            initial_item_instances: vec![
                item_instance(
                    "test:token:one",
                    "test:token",
                    InitialItemLocation::Room("test:start".into()),
                ),
                item_instance("runtime:100", "test:token", InitialItemLocation::Staged),
            ],
            rooms: vec![room()],
        })
        .unwrap();
        runtime_authored
            .restore_runtime_snapshot(old_snapshot.clone())
            .unwrap();
        assert_eq!(
            runtime_authored
                .create_staged_item_instance("test:token")
                .unwrap(),
            ItemInstanceId("runtime:101".into())
        );

        let mut stale_sequence = old_snapshot;
        stale_sequence.item_instances.insert(
            ItemInstanceId("runtime:100".into()),
            RuntimeItemInstanceSnapshot {
                prototype_id: "test:token".into(),
                location: RuntimeItemLocation::Staged,
                binding: ItemBinding::Unbound,
                item_affixes: Vec::new(),
            },
        );
        stale_sequence.next_item_instance_sequence = 100;
        assert!(matches!(
            build(true).restore_runtime_snapshot(stale_sequence),
            Err(WorldError::InvalidRuntimeSnapshot)
        ));
    }

    #[test]
    fn staged_npc_loot_is_neither_visible_nor_interactable_before_spawn() {
        let mut world = World::from_definition_with_staged_items(
            WorldDefinition {
                start_room_id: "test:start".into(),
                item_prototypes: vec![item_prototype(
                    "test:loot",
                    "silver fang",
                    "A trophy carried by a guardian.",
                    &["fang"],
                )],
                initial_item_instances: vec![item_instance(
                    "test:loot",
                    "test:loot",
                    InitialItemLocation::Staged,
                )],
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
                loot_item_ids: vec![ItemInstanceId("test:loot".into())],
                respawn_ticks: 1,
            })
            .unwrap();
        assert_eq!(
            world.runtime_snapshot().item_instances[&ItemInstanceId("test:loot".into())].location,
            RuntimeItemLocation::HeldByNpc {
                npc_id: "test:guardian".into(),
            }
        );
        world
            .spawn_npc(NpcDefinition {
                stable_id: "test:bystander".into(),
                name: "bystander".into(),
                description: "Another NPC.".into(),
                room_id: "test:start".into(),
                level: 1,
                max_health: 1,
                strength: 1,
                defence: 0,
                experience_reward: 0,
                loot_item_ids: Vec::new(),
                respawn_ticks: 1,
            })
            .unwrap();
        let mut wrong_owner = world.runtime_snapshot();
        wrong_owner
            .item_instances
            .get_mut(&ItemInstanceId("test:loot".into()))
            .unwrap()
            .location = RuntimeItemLocation::HeldByNpc {
            npc_id: "test:bystander".into(),
        };
        assert!(matches!(
            world.restore_runtime_snapshot(wrong_owner),
            Err(WorldError::InvalidRuntimeSnapshot)
        ));
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

    #[test]
    fn parses_bounded_stack_and_container_commands() {
        assert_eq!(
            parse_action("take 3 tonic").unwrap(),
            PlayerAction::Take(ItemSelection {
                query: "tonic".into(),
                quantity: 3,
            })
        );
        assert_eq!(
            parse_action("get 2 tonic from SATCHEL").unwrap(),
            PlayerAction::Get {
                selection: ItemSelection {
                    query: "tonic".into(),
                    quantity: 2,
                },
                container: "satchel".into(),
            }
        );
        assert_eq!(
            parse_action("put tonic in satchel").unwrap(),
            PlayerAction::Put {
                selection: ItemSelection::one("tonic"),
                container: "satchel".into(),
            }
        );
        assert_eq!(
            parse_action("contents satchel 2").unwrap(),
            PlayerAction::Contents {
                container: "satchel".into(),
                page: 2,
            }
        );
        assert_eq!(
            parse_action("give 2 Moon Tonic to Aria").unwrap(),
            PlayerAction::Give {
                selection: ItemSelection {
                    query: "Moon Tonic".into(),
                    quantity: 2,
                },
                recipient: "Aria".into(),
            }
        );
        assert_eq!(
            parse_action("trade offer item 12 Moon Tonic").unwrap(),
            PlayerAction::Trade(TradeAction::OfferItem(ItemSelection {
                query: "Moon Tonic".into(),
                quantity: 12,
            }))
        );
        assert_eq!(
            parse_action("trade offer coins 1000000000").unwrap(),
            PlayerAction::Trade(TradeAction::OfferCoins(1_000_000_000))
        );
        assert_eq!(
            parse_action("trade confirm 42").unwrap(),
            PlayerAction::Trade(TradeAction::Confirm { state_revision: 42 })
        );
        assert_eq!(
            parse_action("trade contents offered bag 3").unwrap(),
            PlayerAction::Trade(TradeAction::Contents {
                container: "offered bag".into(),
                page: 3,
            })
        );
        assert!(parse_action("take 257 tonic").is_err());
        assert!(parse_action("take 0 tonic").is_err());
        assert!(parse_action("give tonic").is_err());
        assert!(parse_action("give 257 tonic to Aria").is_err());
        assert!(parse_action("trade offer item 129 tonic").is_err());
        assert!(parse_action("trade offer coins 1000000001").is_err());
        assert!(parse_action("trade confirm 0").is_err());
    }

    #[test]
    fn fungible_stacks_and_containers_are_derived_atomic_and_opaque() {
        let mut tonic = item_prototype("test:tonic", "healing tonic", "Restorative.", &["tonic"]);
        tonic.stack_policy = ItemStackPolicy::Fungible;
        tonic.max_stack_units = 2;
        tonic.weight_grams = 100;
        let mut satchel = item_prototype(
            "test:satchel",
            "archive satchel",
            "A bounded satchel.",
            &["satchel"],
        );
        satchel.weight_grams = 500;
        satchel.container = Some(ItemContainerDefinition {
            max_slots: 2,
            max_weight_grams: 500,
        });
        let instances = vec![
            item_instance(
                "test:satchel-instance",
                "test:satchel",
                InitialItemLocation::Room("test:start".into()),
            ),
            item_instance(
                "test:tonic-a",
                "test:tonic",
                InitialItemLocation::Room("test:start".into()),
            ),
            item_instance(
                "test:tonic-b",
                "test:tonic",
                InitialItemLocation::Room("test:start".into()),
            ),
            item_instance(
                "test:tonic-c",
                "test:tonic",
                InitialItemLocation::Room("test:start".into()),
            ),
        ];
        let (mut world, session) = connect_definition(WorldDefinition {
            start_room_id: "test:start".into(),
            item_prototypes: vec![tonic, satchel],
            initial_item_instances: instances,
            rooms: vec![RoomDefinition {
                stable_id: "test:start".into(),
                title: "Start".into(),
                prose: vec!["A test room.".into()],
                image: None,
                audio: None,
                item_ids: Vec::new(),
                exits: Vec::new(),
            }],
        });

        let scene = world.scene_for(session).unwrap();
        let tonic_groups = scene
            .items
            .iter()
            .filter(|item| item.label == "healing tonic")
            .collect::<Vec<_>>();
        assert_eq!(tonic_groups.len(), 2);
        assert_eq!(
            tonic_groups.iter().map(|item| item.quantity).sum::<u16>(),
            3
        );
        assert!(tonic_groups.iter().all(|item| item.id.starts_with('o')));

        let before_take = world.runtime_snapshot();
        let before_take_revision = world.revision();
        let take = world
            .apply(
                session,
                PlayerAction::Take(ItemSelection {
                    query: "tonic".into(),
                    quantity: 3,
                }),
            )
            .unwrap();
        assert_eq!(world.revision(), before_take_revision + 1);
        assert!(matches!(
            take.domain_events.as_slice(),
            [DomainEvent::ItemsMoved {
                item_ids,
                operation: ContainerMoveOperation::Take,
                container_id: None,
                ..
            }] if item_ids.len() == 3
        ));
        let projected = world
            .project_event(session, &take.domain_events[0])
            .unwrap()
            .unwrap();
        assert_eq!(projected.kind, ViewEventKind::ItemTaken);
        assert!(projected.target_id.is_none());
        let after_take = world.runtime_snapshot();
        world.rollback_domain_events(&take.domain_events).unwrap();
        assert_eq!(world.runtime_snapshot(), before_take);
        let mut forged = take.domain_events[0].clone();
        if let DomainEvent::ItemsMoved { item_ids, .. } = &mut forged {
            item_ids[1] = item_ids[0].clone();
        }
        assert!(matches!(
            world.replay_persisted_event(&forged),
            Err(WorldError::InvalidDomainEvent)
        ));
        assert_eq!(world.runtime_snapshot(), before_take);
        world
            .replay_persisted_event(&take.domain_events[0])
            .unwrap();
        assert_eq!(world.runtime_snapshot(), after_take);
        world
            .apply(session, PlayerAction::Take(ItemSelection::one("satchel")))
            .unwrap();
        let player = world.player_state_view(session).unwrap();
        assert_eq!(player.inventory_capacity.used_slots, 3);
        assert_eq!(player.inventory_capacity.used_weight_grams, 800);

        world
            .apply(
                session,
                PlayerAction::Put {
                    selection: ItemSelection {
                        query: "tonic".into(),
                        quantity: 2,
                    },
                    container: "satchel".into(),
                },
            )
            .unwrap();
        let contents = world
            .apply(
                session,
                PlayerAction::Contents {
                    container: "satchel".into(),
                    page: 1,
                },
            )
            .unwrap()
            .container_contents
            .unwrap();
        assert!(contents.selector.starts_with('o'));
        assert_eq!(contents.items.len(), 1);
        assert_eq!(contents.items[0].quantity, 2);
        assert_eq!(contents.capacity.used_slots, 1);
        assert_eq!(contents.capacity.used_weight_grams, 200);

        let before_get = world.runtime_snapshot();
        let get = world
            .apply(
                session,
                PlayerAction::Get {
                    selection: ItemSelection {
                        query: "tonic".into(),
                        quantity: 2,
                    },
                    container: "satchel".into(),
                },
            )
            .unwrap();
        assert!(
            matches!(get.domain_events.as_slice(), [DomainEvent::ItemsMoved { item_ids, .. }] if item_ids.len() == 2)
        );
        world.rollback_domain_events(&get.domain_events).unwrap();
        assert_eq!(world.runtime_snapshot(), before_get);
    }

    #[test]
    fn container_text_and_actions_keep_capacity_stack_and_weight_facts() {
        let mut satchel = item_prototype(
            "test:satchel",
            "archive satchel",
            "A bounded satchel.",
            &["satchel"],
        );
        satchel.weight_grams = 500;
        satchel.equipment_slot = Some(EquipmentSlot::Trinket);
        satchel.container = Some(ItemContainerDefinition {
            max_slots: 4,
            max_weight_grams: 1_000,
        });
        let mut box_item = item_prototype("test:box", "map box", "A smaller container.", &["box"]);
        box_item.weight_grams = 100;
        box_item.container = Some(ItemContainerDefinition {
            max_slots: 2,
            max_weight_grams: 500,
        });
        let mut tonic = item_prototype("test:tonic", "healing tonic", "Restorative.", &["tonic"]);
        tonic.weight_grams = 100;
        let (mut world, session) = connect_definition(WorldDefinition {
            start_room_id: "test:start".into(),
            item_prototypes: vec![satchel, box_item, tonic],
            initial_item_instances: vec![
                item_instance(
                    "test:satchel-instance",
                    "test:satchel",
                    InitialItemLocation::Room("test:start".into()),
                ),
                item_instance(
                    "test:box-instance",
                    "test:box",
                    InitialItemLocation::Container(ItemInstanceId("test:satchel-instance".into())),
                ),
                item_instance(
                    "test:tonic-instance",
                    "test:tonic",
                    InitialItemLocation::Container(ItemInstanceId("test:box-instance".into())),
                ),
            ],
            rooms: vec![RoomDefinition {
                stable_id: "test:start".into(),
                title: "Start".into(),
                prose: vec!["A test room.".into()],
                image: None,
                audio: None,
                item_ids: Vec::new(),
                exits: Vec::new(),
            }],
        });

        let scene_satchel = world
            .scene_for(session)
            .unwrap()
            .items
            .into_iter()
            .find(|item| item.label == "archive satchel")
            .unwrap();
        assert_eq!(scene_satchel.quantity, 1);
        assert_eq!(scene_satchel.stack_slots, 1);
        assert_eq!(scene_satchel.total_weight_grams, 700);
        assert!(
            scene_satchel
                .actions
                .iter()
                .any(|action| action.label == "Contents"
                    && action.command == format!("contents {}", scene_satchel.id))
        );

        world
            .apply(session, PlayerAction::Take(ItemSelection::one("satchel")))
            .unwrap();
        let carried = world.inventory_as_plain_text(session).unwrap();
        assert!(carried.contains("1 × archive satchel"));
        assert!(carried.contains("1 stack slot"));
        assert!(carried.contains("700 g total"));
        assert!(carried.contains("container 1/4 slots; 200/1000 g"));

        world
            .apply(session, PlayerAction::Equip("satchel".into()))
            .unwrap();
        let equipped = world.inventory_as_plain_text(session).unwrap();
        assert!(equipped.contains("trinket: 1 × archive satchel"));
        assert!(equipped.contains("1 stack slot"));
        assert!(equipped.contains("container 1/4 slots; 200/1000 g"));

        let contents = world
            .apply(
                session,
                PlayerAction::Contents {
                    container: "satchel".into(),
                    page: 1,
                },
            )
            .unwrap();
        let text = contents.output.unwrap();
        assert!(text.contains("1 × map box"));
        assert!(text.contains("1 stack slot"));
        assert!(text.contains("200 g total"));
        assert!(text.contains("container 1/2 slots; 100/500 g"));
    }

    #[test]
    fn container_cycles_and_capacity_fail_without_partial_mutation() {
        let mut bag = item_prototype("test:bag", "canvas bag", "A bag.", &["bag"]);
        bag.container = Some(ItemContainerDefinition {
            max_slots: 2,
            max_weight_grams: 10_000,
        });
        let (mut world, session) = connect_definition(WorldDefinition {
            start_room_id: "test:start".into(),
            item_prototypes: vec![bag],
            initial_item_instances: vec![
                item_instance(
                    "test:bag-a",
                    "test:bag",
                    InitialItemLocation::Room("test:start".into()),
                ),
                item_instance(
                    "test:bag-b",
                    "test:bag",
                    InitialItemLocation::Room("test:start".into()),
                ),
            ],
            rooms: vec![RoomDefinition {
                stable_id: "test:start".into(),
                title: "Start".into(),
                prose: vec!["A test room.".into()],
                image: None,
                audio: None,
                item_ids: Vec::new(),
                exits: Vec::new(),
            }],
        });
        let ids = world.scene_for(session).unwrap().items;
        world
            .apply(session, PlayerAction::Take(ids[0].id.clone().into()))
            .unwrap();
        world
            .apply(session, PlayerAction::Take(ids[1].id.clone().into()))
            .unwrap();
        let carried = world.player_state_view(session).unwrap().inventory;
        world
            .apply(
                session,
                PlayerAction::Put {
                    selection: ItemSelection::one(carried[0].id.clone()),
                    container: carried[1].id.clone(),
                },
            )
            .unwrap();
        let before = world.runtime_snapshot();
        let nested = world
            .apply(
                session,
                PlayerAction::Contents {
                    container: carried[1].id.clone(),
                    page: 1,
                },
            )
            .unwrap()
            .container_contents
            .unwrap()
            .items[0]
            .id
            .clone();
        assert!(
            world
                .apply(
                    session,
                    PlayerAction::Put {
                        selection: ItemSelection::one(carried[1].id.clone()),
                        container: nested,
                    },
                )
                .is_err()
        );
        assert_eq!(world.runtime_snapshot(), before);
    }

    #[test]
    fn fungible_equipment_uses_prospective_stack_capacity() {
        let mut blade = item_prototype("test:blade", "thin blade", "A blade.", &["blade"]);
        blade.stack_policy = ItemStackPolicy::Fungible;
        blade.max_stack_units = 10;
        blade.equipment_slot = Some(EquipmentSlot::Weapon);
        let (mut world, session) = connect_definition(WorldDefinition {
            start_room_id: "test:start".into(),
            item_prototypes: vec![blade],
            initial_item_instances: vec![
                item_instance(
                    "test:blade-a",
                    "test:blade",
                    InitialItemLocation::Room("test:start".into()),
                ),
                item_instance(
                    "test:blade-b",
                    "test:blade",
                    InitialItemLocation::Room("test:start".into()),
                ),
            ],
            rooms: vec![RoomDefinition {
                stable_id: "test:start".into(),
                title: "Start".into(),
                prose: vec!["A test room.".into()],
                image: None,
                audio: None,
                item_ids: Vec::new(),
                exits: Vec::new(),
            }],
        });
        world
            .character_gameplay
            .get_mut(&CharacterId(701))
            .unwrap()
            .inventory_capacity
            .max_slots = 1;
        world
            .apply(
                session,
                PlayerAction::Take(ItemSelection {
                    query: "blade".into(),
                    quantity: 2,
                }),
            )
            .unwrap();
        let selector = world.player_state_view(session).unwrap().inventory[0]
            .id
            .clone();
        world.apply(session, PlayerAction::Equip(selector)).unwrap();
        world
            .apply(session, PlayerAction::Unequip("weapon".into()))
            .unwrap();
        let player = world.player_state_view(session).unwrap();
        assert_eq!(player.inventory.len(), 1);
        assert_eq!(player.inventory[0].quantity, 2);
        assert_eq!(player.inventory_capacity.used_slots, 1);
    }

    #[test]
    fn snapshot_decoder_is_version_directed_and_rejects_v2_v3_smuggling() {
        let (world, _) = connected_world();
        let json = serde_json::to_string(&world.runtime_snapshot()).unwrap();
        assert_eq!(
            decode_world_runtime_snapshot(WORLD_RUNTIME_SNAPSHOT_VERSION, &json)
                .unwrap()
                .snapshot_version,
            WORLD_RUNTIME_SNAPSHOT_VERSION
        );
        assert!(decode_world_runtime_snapshot(4, &json).is_err());
        let legacy_v3 = runtime_snapshot_json_for_version(&world.runtime_snapshot(), 3);
        let legacy_v3_json = serde_json::to_string(&legacy_v3).unwrap();
        assert_eq!(
            decode_world_runtime_snapshot(3, &legacy_v3_json)
                .unwrap()
                .snapshot_version,
            WORLD_RUNTIME_SNAPSHOT_VERSION
        );
        let mut v3_binding_smuggling = legacy_v3;
        v3_binding_smuggling["itemInstances"]
            .as_object_mut()
            .unwrap()
            .values_mut()
            .next()
            .unwrap()["binding"] = serde_json::json!({"kind": "unbound"});
        assert!(
            decode_world_runtime_snapshot(
                3,
                &serde_json::to_string(&v3_binding_smuggling).unwrap()
            )
            .is_err()
        );
        let mut smuggled: serde_json::Value = serde_json::from_str(&json).unwrap();
        smuggled["snapshotVersion"] = serde_json::json!(2);
        let instance = smuggled["itemInstances"]
            .as_object_mut()
            .unwrap()
            .values_mut()
            .next()
            .unwrap();
        instance["location"] = serde_json::json!({
            "kind": "contained",
            "containerId": "demo:blue-lantern"
        });
        let smuggled = serde_json::to_string(&smuggled).unwrap();
        assert!(decode_world_runtime_snapshot(2, &smuggled).is_err());

        let mut unknown_root: serde_json::Value = serde_json::from_str(&json).unwrap();
        unknown_root["quantity"] = serde_json::json!(1);
        assert!(
            decode_world_runtime_snapshot(6, &serde_json::to_string(&unknown_root).unwrap())
                .is_err()
        );

        let mut unknown_instance: serde_json::Value = serde_json::from_str(&json).unwrap();
        let instance = unknown_instance["itemInstances"]
            .as_object_mut()
            .unwrap()
            .values_mut()
            .next()
            .unwrap();
        instance["quantity"] = serde_json::json!(1);
        assert!(
            decode_world_runtime_snapshot(6, &serde_json::to_string(&unknown_instance).unwrap())
                .is_err()
        );

        let mut unknown_location: serde_json::Value = serde_json::from_str(&json).unwrap();
        let location = &mut unknown_location["itemInstances"]
            .as_object_mut()
            .unwrap()
            .values_mut()
            .next()
            .unwrap()["location"];
        location["containerId"] = serde_json::json!("demo:blue-lantern");
        assert!(
            decode_world_runtime_snapshot(6, &serde_json::to_string(&unknown_location).unwrap())
                .is_err()
        );

        let legacy_v4 = runtime_snapshot_json_for_version(&world.runtime_snapshot(), 4);
        let upgraded =
            decode_world_runtime_snapshot(4, &serde_json::to_string(&legacy_v4).unwrap()).unwrap();
        assert_eq!(upgraded.snapshot_version, WORLD_RUNTIME_SNAPSHOT_VERSION);
        assert_eq!(upgraded.next_trade_id, TradeId(1));
        assert!(upgraded.trades.is_empty());
        let mut smuggled_v4 = legacy_v4;
        smuggled_v4["trades"] = serde_json::json!({});
        assert!(
            decode_world_runtime_snapshot(4, &serde_json::to_string(&smuggled_v4).unwrap())
                .is_err()
        );

        let mut missing_trade_fields: serde_json::Value = serde_json::from_str(&json).unwrap();
        missing_trade_fields
            .as_object_mut()
            .unwrap()
            .remove("trades");
        assert!(
            decode_world_runtime_snapshot(
                6,
                &serde_json::to_string(&missing_trade_fields).unwrap()
            )
            .is_err()
        );
        let mut nested_trade = runtime_snapshot_json_for_version(&world.runtime_snapshot(), 6);
        nested_trade["nextTradeId"] = serde_json::json!(2);
        nested_trade["trades"]["1"] = serde_json::to_value(TradeSnapshot {
            id: TradeId(1),
            requester: CharacterId(101),
            recipient: CharacterId(102),
            room_id: world.start_room_id().into(),
            phase: TradePhase::Requested,
            created_tick: 0,
            opened_tick: None,
            expires_at_tick: TRADE_REQUEST_TICKS,
            state_revision: 1,
            offers: BTreeMap::new(),
            confirmations: BTreeMap::new(),
        })
        .unwrap();
        assert!(
            decode_world_runtime_snapshot(6, &serde_json::to_string(&nested_trade).unwrap())
                .is_ok()
        );
        nested_trade["trades"]["1"]["futureTradeField"] = serde_json::json!(true);
        assert!(
            decode_world_runtime_snapshot(6, &serde_json::to_string(&nested_trade).unwrap())
                .is_err()
        );

        assert!(valid_item_instance_id("runtime:1"));
        assert!(valid_item_instance_id("runtime:18446744073709551615"));
        assert!(valid_item_instance_id(&"i".repeat(128)));
        for invalid_id in [
            "runtime:",
            "runtime:0",
            "runtime:01",
            "runtime:-1",
            "runtime:+1",
            "runtime:18446744073709551616",
            &"i".repeat(129),
        ] {
            assert!(!valid_item_instance_id(invalid_id), "{invalid_id}");
        }
    }

    #[test]
    fn runtime_snapshot_nested_records_reject_unknown_fields_across_versions() {
        let mut world = World::demo();
        let room_id = world.start_room_id().to_owned();
        world
            .spawn_npc(NpcDefinition {
                stable_id: "test:snapshot-npc".into(),
                name: "snapshot sentinel".into(),
                description: "An NPC retained only to exercise the durable snapshot shape.".into(),
                room_id,
                level: 1,
                max_health: 10,
                strength: 1,
                defence: 0,
                experience_reward: 1,
                loot_item_ids: Vec::new(),
                respawn_ticks: 2,
            })
            .unwrap();
        let snapshot = world.runtime_snapshot();
        for version in 1..=WORLD_RUNTIME_SNAPSHOT_VERSION {
            let clean = runtime_snapshot_json_for_version(&snapshot, version);
            let clean_json = serde_json::to_string(&clean).unwrap();
            assert!(
                decode_world_runtime_snapshot(version, &clean_json).is_ok(),
                "valid v{version} fixture must decode"
            );

            let mut npc_smuggling = clean.clone();
            first_runtime_npc(&mut npc_smuggling)["futureNpcField"] = serde_json::json!(true);
            assert!(
                decode_world_runtime_snapshot(
                    version,
                    &serde_json::to_string(&npc_smuggling).unwrap()
                )
                .is_err(),
                "v{version} must reject an unknown NPC field"
            );

            let mut adventure_smuggling = clean.clone();
            adventure_smuggling["adventure"]["futureAdventureField"] = serde_json::json!(true);
            assert!(
                decode_world_runtime_snapshot(
                    version,
                    &serde_json::to_string(&adventure_smuggling).unwrap()
                )
                .is_err(),
                "v{version} must reject an unknown adventure field"
            );

            let mut party_smuggling = clean.clone();
            party_smuggling["adventure"]["parties"]["77"] = serde_json::json!({
                "id": 77,
                "leader": 701,
                "members": [701],
                "invitations": {},
                "futurePartyField": true
            });
            assert!(
                decode_world_runtime_snapshot(
                    version,
                    &serde_json::to_string(&party_smuggling).unwrap()
                )
                .is_err(),
                "v{version} must reject an unknown party field"
            );

            let mut resource_smuggling = clean.clone();
            first_runtime_npc(&mut resource_smuggling)["resources"]["test:focus"] = serde_json::json!({
                "current": 1,
                "max": 2,
                "futureResourceField": true
            });
            assert!(
                decode_world_runtime_snapshot(
                    version,
                    &serde_json::to_string(&resource_smuggling).unwrap()
                )
                .is_err(),
                "v{version} must reject an unknown resource-pool field"
            );

            let mut status_smuggling = clean.clone();
            first_runtime_npc(&mut status_smuggling)["statuses"] = serde_json::json!([{
                "sourceAbilityId": "test:stun",
                "kind": "stun",
                "magnitude": 1,
                "remainingTicks": 1,
                "futureStatusField": true
            }]);
            assert!(
                decode_world_runtime_snapshot(
                    version,
                    &serde_json::to_string(&status_smuggling).unwrap()
                )
                .is_err(),
                "v{version} must reject an unknown active-status field"
            );

            let mut legacy_defaults = clean;
            legacy_defaults.as_object_mut().unwrap().remove("adventure");
            let npc = first_runtime_npc(&mut legacy_defaults)
                .as_object_mut()
                .unwrap();
            for optional in [
                "resources",
                "resourceRegeneration",
                "abilityIds",
                "abilityCooldowns",
                "resistances",
                "statuses",
            ] {
                npc.remove(optional);
            }
            assert!(
                decode_world_runtime_snapshot(
                    version,
                    &serde_json::to_string(&legacy_defaults).unwrap()
                )
                .is_ok(),
                "v{version} must retain defaults for absent additive fields"
            );
        }
    }

    #[test]
    fn batch_item_projection_filters_hidden_units_and_private_container_selectors() {
        let mut tonic = item_prototype("test:tonic", "healing tonic", "Restorative.", &["tonic"]);
        tonic.stack_policy = ItemStackPolicy::Fungible;
        tonic.max_stack_units = 20;
        let mut satchel = item_prototype(
            "test:satchel",
            "archive satchel",
            "A bounded satchel.",
            &["satchel"],
        );
        satchel.container = Some(ItemContainerDefinition {
            max_slots: 20,
            max_weight_grams: 20_000,
        });
        let (mut world, actor) = connect_definition(WorldDefinition {
            start_room_id: "test:start".into(),
            item_prototypes: vec![tonic, satchel],
            initial_item_instances: vec![
                item_instance(
                    "test:tonic-visible",
                    "test:tonic",
                    InitialItemLocation::Room("test:start".into()),
                ),
                item_instance(
                    "test:tonic-hidden",
                    "test:tonic",
                    InitialItemLocation::Room("test:start".into()),
                ),
                item_instance(
                    "test:satchel-instance",
                    "test:satchel",
                    InitialItemLocation::Room("test:start".into()),
                ),
            ],
            rooms: vec![RoomDefinition {
                stable_id: "test:start".into(),
                title: "Start".into(),
                prose: vec!["A test room.".into()],
                image: None,
                audio: None,
                item_ids: Vec::new(),
                exits: Vec::new(),
            }],
        });
        world
            .configure_hidden_item("test:tonic-hidden", 20)
            .unwrap();
        world.set_observer_role(actor, AccountRole::Admin).unwrap();
        let peer = SessionId(72);
        world
            .connect(
                peer,
                CharacterSnapshot {
                    id: CharacterId(702),
                    account_id: AccountId(72),
                    name: "Witness".into(),
                    room_id: "test:start".into(),
                    item_locations: BTreeMap::new(),
                },
            )
            .unwrap();
        let peer_visible_tonic = world
            .scene_for(peer)
            .unwrap()
            .items
            .into_iter()
            .find(|item| item.label == "healing tonic")
            .unwrap()
            .id;

        let take = world
            .apply(
                actor,
                PlayerAction::Take(ItemSelection {
                    query: "tonic".into(),
                    quantity: 2,
                }),
            )
            .unwrap();
        let actor_event = world
            .project_event(actor, &take.domain_events[0])
            .unwrap()
            .unwrap();
        let peer_event = world
            .project_event(peer, &take.domain_events[0])
            .unwrap()
            .unwrap();
        assert!(actor_event.text.contains("2 × healing tonic"));
        assert!(peer_event.text.contains("1 × healing tonic"));
        assert_eq!(
            peer_event.item_id.as_deref(),
            Some(peer_visible_tonic.as_str())
        );

        world
            .apply(actor, PlayerAction::Take(ItemSelection::one("satchel")))
            .unwrap();
        let put = world
            .apply(
                actor,
                PlayerAction::Put {
                    selection: ItemSelection::one("tonic"),
                    container: "satchel".into(),
                },
            )
            .unwrap();
        let peer_put = world
            .project_event(peer, &put.domain_events[0])
            .unwrap()
            .unwrap();
        assert!(peer_put.text.contains("puts 1 × healing tonic"));
        assert!(peer_put.target_id.is_none());
        assert!(peer_put.item_id.is_none());
    }

    #[test]
    fn batch_replay_supports_offline_owners_but_rejects_foreign_container_roots() {
        let mut tonic = item_prototype("test:tonic", "healing tonic", "Restorative.", &["tonic"]);
        tonic.stack_policy = ItemStackPolicy::Fungible;
        tonic.max_stack_units = 20;
        let mut satchel = item_prototype(
            "test:satchel",
            "archive satchel",
            "A bounded satchel.",
            &["satchel"],
        );
        satchel.container = Some(ItemContainerDefinition {
            max_slots: 20,
            max_weight_grams: 20_000,
        });
        let definition = WorldDefinition {
            start_room_id: "test:start".into(),
            item_prototypes: vec![tonic, satchel],
            initial_item_instances: vec![
                item_instance(
                    "test:tonic-a",
                    "test:tonic",
                    InitialItemLocation::Room("test:start".into()),
                ),
                item_instance(
                    "test:tonic-b",
                    "test:tonic",
                    InitialItemLocation::Room("test:start".into()),
                ),
                item_instance(
                    "test:satchel-instance",
                    "test:satchel",
                    InitialItemLocation::Room("test:start".into()),
                ),
            ],
            rooms: vec![RoomDefinition {
                stable_id: "test:start".into(),
                title: "Start".into(),
                prose: vec!["A test room.".into()],
                image: None,
                audio: None,
                item_ids: Vec::new(),
                exits: Vec::new(),
            }],
        };
        let offline_owner = CharacterId(900);
        let offline_take = DomainEvent::ItemsMoved {
            actor_id: offline_owner,
            actor_name: "Offline owner".into(),
            item_ids: vec!["test:tonic-a".into(), "test:tonic-b".into()],
            item_name: "healing tonic".into(),
            container_id: None,
            operation: ContainerMoveOperation::Take,
            room_id: "test:start".into(),
        };
        let mut offline_world = World::from_definition(definition.clone()).unwrap();
        offline_world.replay_persisted_event(&offline_take).unwrap();
        assert_eq!(
            offline_world.runtime_item_owner_ids(),
            BTreeSet::from([offline_owner])
        );

        let (mut world, actor) = connect_definition(definition);
        let victim = SessionId(73);
        world
            .connect(
                victim,
                CharacterSnapshot {
                    id: CharacterId(703),
                    account_id: AccountId(73),
                    name: "Other owner".into(),
                    room_id: "test:start".into(),
                    item_locations: BTreeMap::new(),
                },
            )
            .unwrap();
        world
            .apply(actor, PlayerAction::Take(ItemSelection::one("tonic")))
            .unwrap();
        world
            .apply(victim, PlayerAction::Take(ItemSelection::one("satchel")))
            .unwrap();
        let forged = DomainEvent::ItemsMoved {
            actor_id: CharacterId(701),
            actor_name: "Stack Tester".into(),
            item_ids: vec!["test:tonic-a".into()],
            item_name: "healing tonic".into(),
            container_id: Some("test:satchel-instance".into()),
            operation: ContainerMoveOperation::Put,
            room_id: "test:start".into(),
        };
        let before = world.runtime_snapshot();
        assert!(matches!(
            world.replay_persisted_event(&forged),
            Err(WorldError::InvalidDomainEvent)
        ));
        assert_eq!(world.runtime_snapshot(), before);
    }

    #[test]
    fn rollback_revalidates_inventory_capacity_before_publishing_locations() {
        let tonic = item_prototype("test:tonic", "healing tonic", "Restorative.", &["tonic"]);
        let mut satchel = item_prototype(
            "test:satchel",
            "archive satchel",
            "A bounded satchel.",
            &["satchel"],
        );
        satchel.container = Some(ItemContainerDefinition {
            max_slots: 20,
            max_weight_grams: 20_000,
        });
        let (mut world, actor) = connect_definition(WorldDefinition {
            start_room_id: "test:start".into(),
            item_prototypes: vec![tonic, satchel],
            initial_item_instances: vec![
                item_instance(
                    "test:tonic-a",
                    "test:tonic",
                    InitialItemLocation::Room("test:start".into()),
                ),
                item_instance(
                    "test:satchel-instance",
                    "test:satchel",
                    InitialItemLocation::Room("test:start".into()),
                ),
            ],
            rooms: vec![RoomDefinition {
                stable_id: "test:start".into(),
                title: "Start".into(),
                prose: vec!["A test room.".into()],
                image: None,
                audio: None,
                item_ids: Vec::new(),
                exits: Vec::new(),
            }],
        });
        world
            .apply(actor, PlayerAction::Take(ItemSelection::one("tonic")))
            .unwrap();
        world
            .apply(actor, PlayerAction::Take(ItemSelection::one("satchel")))
            .unwrap();
        let put = world
            .apply(
                actor,
                PlayerAction::Put {
                    selection: ItemSelection::one("tonic"),
                    container: "satchel".into(),
                },
            )
            .unwrap();
        world
            .character_gameplay
            .get_mut(&CharacterId(701))
            .unwrap()
            .inventory_capacity
            .max_slots = 1;
        let before = world.runtime_snapshot();
        assert!(matches!(
            world.rollback_domain_events(&put.domain_events),
            Err(WorldError::InventoryCapacityExceeded)
        ));
        assert_eq!(world.runtime_snapshot(), before);
    }

    #[test]
    fn acquisition_binds_recursively_and_restricted_subtrees_cannot_escape_owner() {
        let (mut world, actor) = connect_definition(policy_definition());
        let actor_id = CharacterId(701);
        world
            .connect(
                SessionId(72),
                CharacterSnapshot {
                    id: CharacterId(702),
                    account_id: AccountId(72),
                    name: "Aria".into(),
                    room_id: "test:start".into(),
                    item_locations: BTreeMap::new(),
                },
            )
            .unwrap();
        let clasp_id = ItemInstanceId("policy:clasp-instance".into());
        let token_id = ItemInstanceId("policy:token-instance".into());
        assert_eq!(
            world.runtime_snapshot().item_instances[&clasp_id].binding,
            ItemBinding::Unbound
        );

        let original_max_weight = world
            .character_gameplay
            .get(&actor_id)
            .unwrap()
            .inventory_capacity
            .max_weight_grams;
        world
            .character_gameplay
            .get_mut(&actor_id)
            .unwrap()
            .inventory_capacity
            .max_weight_grams = 1_000;
        let before_failed_script_grant = world.runtime_snapshot();
        assert!(matches!(
            world.give_authorized_script_item(actor, "policy:bag-instance"),
            Err(WorldError::InventoryCapacityExceeded)
        ));
        assert_eq!(world.runtime_snapshot(), before_failed_script_grant);
        assert_eq!(
            world.runtime_snapshot().item_instances[&clasp_id].binding,
            ItemBinding::Unbound
        );
        world
            .character_gameplay
            .get_mut(&actor_id)
            .unwrap()
            .inventory_capacity
            .max_weight_grams = original_max_weight;

        let take_bag = world
            .apply(actor, PlayerAction::Take(ItemSelection::one("bag")))
            .unwrap();
        assert_eq!(
            world.runtime_snapshot().item_instances[&clasp_id].binding,
            ItemBinding::Character {
                character_id: actor_id
            }
        );
        assert_eq!(world.runtime_item_owner_ids(), BTreeSet::from([actor_id]));
        let bag = world
            .player_state_view(actor)
            .unwrap()
            .inventory
            .into_iter()
            .find(|item| item.label == "traveller bag")
            .unwrap();
        assert_eq!(bag.transfer_policy, ItemTransferPolicy::Transferable);
        assert!(!bag.bound_to_you);
        assert!(!bag.can_give);
        assert!(!bag.actions.iter().any(|action| action.label == "Drop"));

        let before_forbidden = world.runtime_snapshot();
        let forged_restricted_give = DomainEvent::ItemsGiven {
            giver_id: actor_id,
            giver_name: "Stack Tester".into(),
            recipient_id: CharacterId(702),
            recipient_name: "Aria".into(),
            item_ids: vec!["policy:bag-instance".into()],
            item_name: "traveller bag".into(),
            room_id: "test:start".into(),
        };
        assert!(matches!(
            world.replay_persisted_event(&forged_restricted_give),
            Err(WorldError::InvalidDomainEvent)
        ));
        assert_eq!(world.runtime_snapshot(), before_forbidden);
        assert!(matches!(
            world.apply(actor, PlayerAction::Drop(ItemSelection::one("bag"))),
            Err(WorldError::ItemTransferForbidden)
        ));
        assert!(matches!(
            world.apply(
                actor,
                PlayerAction::Give {
                    selection: ItemSelection::one("bag"),
                    recipient: "Aria".into(),
                }
            ),
            Err(WorldError::ItemTransferForbidden)
        ));
        assert_eq!(world.runtime_snapshot(), before_forbidden);

        world
            .apply(actor, PlayerAction::Take(ItemSelection::one("token")))
            .unwrap();
        assert_eq!(
            world.runtime_snapshot().item_instances[&token_id].binding,
            ItemBinding::Unbound
        );
        let restricted = world.runtime_snapshot();
        assert!(matches!(
            world.apply(actor, PlayerAction::Drop(ItemSelection::one("token"))),
            Err(WorldError::ItemTransferForbidden)
        ));
        assert!(matches!(
            world.apply(
                actor,
                PlayerAction::Give {
                    selection: ItemSelection::one("token"),
                    recipient: "Aria".into(),
                }
            ),
            Err(WorldError::ItemTransferForbidden)
        ));
        assert!(matches!(
            world.apply(
                actor,
                PlayerAction::Put {
                    selection: ItemSelection::one("token"),
                    container: "chest".into(),
                }
            ),
            Err(WorldError::ItemTransferForbidden)
        ));
        assert_eq!(world.runtime_snapshot(), restricted);

        world
            .configure_item_gameplay(ItemGameplayDefinition {
                item_id: "policy:token".into(),
                equipment_slot: None,
                attack_bonus: 0,
                heal_amount: 1,
                consumable: true,
                effect: None,
            })
            .unwrap();
        world
            .apply(actor, PlayerAction::Use("token".into()))
            .unwrap();
        let consumed_token_snapshot = world.runtime_snapshot();
        let consumed_token = &consumed_token_snapshot.item_instances[&token_id];
        assert_eq!(consumed_token.location, RuntimeItemLocation::Consumed);
        assert_eq!(consumed_token.binding, ItemBinding::Unbound);
        validate_item_graph(&world.item_instances, &world.item_prototypes).unwrap();
        let mut restored_consumed = World::from_definition(policy_definition()).unwrap();
        restored_consumed
            .restore_runtime_snapshot(consumed_token_snapshot)
            .unwrap();

        world
            .apply(actor, PlayerAction::Take(ItemSelection::one("chest")))
            .unwrap();
        world
            .apply(
                actor,
                PlayerAction::Put {
                    selection: ItemSelection::one("bag"),
                    container: "chest".into(),
                },
            )
            .unwrap();
        world
            .apply(
                actor,
                PlayerAction::Get {
                    selection: ItemSelection::one("bag"),
                    container: "chest".into(),
                },
            )
            .unwrap();
        assert_eq!(
            world.runtime_snapshot().item_instances[&clasp_id].binding,
            ItemBinding::Character {
                character_id: actor_id
            }
        );

        world
            .configure_item_gameplay(ItemGameplayDefinition {
                item_id: "policy:scripted".into(),
                equipment_slot: None,
                attack_bonus: 0,
                heal_amount: 1,
                consumable: true,
                effect: None,
            })
            .unwrap();
        let scripted_event = world
            .give_authorized_script_item(actor, "policy:scripted-instance")
            .unwrap()
            .unwrap();
        assert!(matches!(scripted_event, DomainEvent::ItemTaken { .. }));
        world
            .apply(actor, PlayerAction::Use("sigil".into()))
            .unwrap();
        let consumed_snapshot = world.runtime_snapshot();
        let scripted =
            &consumed_snapshot.item_instances[&ItemInstanceId("policy:scripted-instance".into())];
        assert_eq!(scripted.location, RuntimeItemLocation::Consumed);
        assert_eq!(
            scripted.binding,
            ItemBinding::Character {
                character_id: actor_id
            }
        );
        assert!(world.runtime_item_owner_ids().contains(&actor_id));

        let mut replay = World::from_definition(policy_definition()).unwrap();
        replay
            .replay_persisted_event(&take_bag.domain_events[0])
            .unwrap();
        assert_eq!(
            replay.runtime_snapshot().item_instances[&clasp_id].binding,
            ItemBinding::Character {
                character_id: actor_id
            }
        );
        replay
            .rollback_domain_events(&take_bag.domain_events)
            .unwrap();
        assert_eq!(
            replay.runtime_snapshot().item_instances[&clasp_id].binding,
            ItemBinding::Unbound
        );
    }

    #[test]
    fn give_is_atomic_capacity_bounded_opaque_and_replayable() {
        let definition = give_definition();
        let mut world = World::from_definition(definition.clone()).unwrap();
        let connect = |world: &mut World,
                       session: SessionId,
                       character_id: CharacterId,
                       name: &str,
                       room_id: &str| {
            world
                .connect(
                    session,
                    CharacterSnapshot {
                        id: character_id,
                        account_id: AccountId(character_id.0),
                        name: name.into(),
                        room_id: room_id.into(),
                        item_locations: BTreeMap::new(),
                    },
                )
                .unwrap();
        };
        let giver_session = SessionId(71);
        let recipient_session = SessionId(72);
        let observer_session = SessionId(73);
        let remote_session = SessionId(74);
        connect(
            &mut world,
            giver_session,
            CharacterId(701),
            "Stack Tester",
            "give:start",
        );
        connect(
            &mut world,
            recipient_session,
            CharacterId(702),
            "Aria",
            "give:start",
        );
        connect(
            &mut world,
            observer_session,
            CharacterId(703),
            "Bram",
            "give:start",
        );
        connect(
            &mut world,
            remote_session,
            CharacterId(704),
            "Distant",
            "give:remote",
        );

        let social = world.player_state_view(giver_session).unwrap().social;
        let aria = social
            .online
            .iter()
            .find(|entry| entry.name == "Aria")
            .unwrap();
        let aria_selector = aria.id.clone();
        assert!(aria.nearby);
        assert!(
            !social
                .online
                .iter()
                .find(|entry| entry.name == "Distant")
                .unwrap()
                .nearby
        );
        assert_eq!(
            world
                .resolve_give_recipient(giver_session, &aria_selector)
                .unwrap()
                .character_id,
            CharacterId(702)
        );
        assert_eq!(
            world
                .resolve_give_recipient(giver_session, "aRiA")
                .unwrap()
                .character_id,
            CharacterId(702)
        );
        connect(
            &mut world,
            SessionId(75),
            CharacterId(705),
            "ARIA",
            "give:start",
        );
        assert!(matches!(
            world.resolve_give_recipient(giver_session, "aria"),
            Err(WorldError::GiveTargetUnavailable)
        ));
        for invalid in ["Stack Tester", "Distant", "Offline"] {
            assert!(matches!(
                world.resolve_give_recipient(giver_session, invalid),
                Err(WorldError::GiveTargetUnavailable)
            ));
        }

        let take = world
            .apply(
                giver_session,
                PlayerAction::Take(ItemSelection {
                    query: "token".into(),
                    quantity: 3,
                }),
            )
            .unwrap();
        assert!(world.player_state_view(giver_session).unwrap().inventory[0].can_give);
        world
            .character_gameplay
            .get_mut(&CharacterId(702))
            .unwrap()
            .inventory_capacity
            .max_slots = 1;
        let before_capacity_failure = world.runtime_snapshot();
        assert!(matches!(
            world.apply(
                giver_session,
                PlayerAction::Give {
                    selection: ItemSelection {
                        query: "token".into(),
                        quantity: 3,
                    },
                    recipient: aria_selector.clone(),
                }
            ),
            Err(WorldError::InventoryCapacityExceeded)
        ));
        assert_eq!(world.runtime_snapshot(), before_capacity_failure);

        let given = world
            .apply(
                giver_session,
                PlayerAction::Give {
                    selection: ItemSelection {
                        query: "token".into(),
                        quantity: 2,
                    },
                    recipient: aria_selector,
                },
            )
            .unwrap();
        let event = given.domain_events.first().unwrap();
        assert!(matches!(
            event,
            DomainEvent::ItemsGiven {
                giver_id: CharacterId(701),
                recipient_id: CharacterId(702),
                item_ids,
                ..
            } if item_ids.len() == 2
        ));
        let giver_view = world.project_event(giver_session, event).unwrap().unwrap();
        let recipient_view = world
            .project_event(recipient_session, event)
            .unwrap()
            .unwrap();
        let observer_view = world
            .project_event(observer_session, event)
            .unwrap()
            .unwrap();
        assert_eq!(giver_view.kind, ViewEventKind::ItemGiven);
        assert!(giver_view.text.contains("to Aria"));
        assert!(recipient_view.text.contains("gives you"));
        assert!(observer_view.text.contains("to Aria"));
        assert!(giver_view.item_id.is_some());
        assert!(recipient_view.item_id.is_some());
        assert_ne!(giver_view.item_id, recipient_view.item_id);
        assert!(observer_view.item_id.is_none());
        assert!(
            world
                .project_event(remote_session, event)
                .unwrap()
                .is_none()
        );
        assert_eq!(
            world.player_state_view(giver_session).unwrap().inventory[0].quantity,
            1
        );
        assert_eq!(
            world
                .player_state_view(recipient_session)
                .unwrap()
                .inventory[0]
                .quantity,
            2
        );

        let mut replay = World::from_definition(definition.clone()).unwrap();
        replay
            .replay_persisted_event(&take.domain_events[0])
            .unwrap();
        replay.replay_persisted_event(event).unwrap();
        assert_eq!(
            replay
                .runtime_snapshot()
                .item_instances
                .values()
                .filter(|instance| {
                    instance.location
                        == (RuntimeItemLocation::Carried {
                            character_id: CharacterId(702),
                        })
                })
                .count(),
            2
        );
        replay
            .rollback_domain_events(std::slice::from_ref(event))
            .unwrap();
        replay.rollback_domain_events(&take.domain_events).unwrap();
        let initial = World::from_definition(definition)
            .unwrap()
            .runtime_snapshot();
        assert_eq!(replay.runtime_snapshot(), initial);

        world.disconnect(recipient_session);
        assert_eq!(
            world
                .runtime_snapshot()
                .item_instances
                .values()
                .filter(|instance| {
                    instance.location
                        == (RuntimeItemLocation::Carried {
                            character_id: CharacterId(702),
                        })
                })
                .count(),
            2
        );
    }

    #[test]
    fn confirmed_trade_reserves_invalidates_and_settles_both_sides_atomically() {
        let mut definition = give_definition();
        let mut bag = item_prototype(
            "give:bag",
            "trade satchel",
            "A container offered with visible contents.",
            &["satchel"],
        );
        bag.container = Some(ItemContainerDefinition {
            max_slots: 4,
            max_weight_grams: 2_000,
        });
        let pebble = item_prototype(
            "give:pebble",
            "moon pebble",
            "A small transferable stone.",
            &["pebble"],
        );
        definition.item_prototypes.extend([bag, pebble]);
        definition.initial_item_instances.extend([
            item_instance(
                "give:bag-instance",
                "give:bag",
                InitialItemLocation::Room("give:start".into()),
            ),
            item_instance(
                "give:pebble-instance",
                "give:pebble",
                InitialItemLocation::Container("give:bag-instance".into()),
            ),
        ]);
        let mut world = World::from_definition(definition.clone()).unwrap();
        let connect = |world: &mut World, session, id, name: &str| {
            world
                .connect(
                    session,
                    CharacterSnapshot {
                        id,
                        account_id: AccountId(id.0),
                        name: name.into(),
                        room_id: "give:start".into(),
                        item_locations: BTreeMap::new(),
                    },
                )
                .unwrap();
        };
        let a = SessionId(801);
        let b = SessionId(802);
        connect(&mut world, a, CharacterId(801), "Aster");
        connect(&mut world, b, CharacterId(802), "Bryn");
        let take = world
            .apply(
                a,
                PlayerAction::Take(ItemSelection {
                    query: "token".into(),
                    quantity: 2,
                }),
            )
            .unwrap();
        let take_bag = world
            .apply(a, PlayerAction::Take(ItemSelection::one("satchel")))
            .unwrap();

        let requested = world
            .apply(a, PlayerAction::Trade(TradeAction::Request("Bryn".into())))
            .unwrap();
        assert!(matches!(
            requested.domain_events.as_slice(),
            [DomainEvent::TradeRequested { trade }] if trade.state_revision == 1
        ));
        assert_eq!(
            world.runtime_snapshot().snapshot_version,
            WORLD_RUNTIME_SNAPSHOT_VERSION
        );
        assert_eq!(world.runtime_snapshot().next_trade_id, TradeId(2));
        let mut replay_events = requested.domain_events.clone();
        let opened = world
            .apply(b, PlayerAction::Trade(TradeAction::Accept))
            .unwrap();
        replay_events.extend(opened.domain_events);
        let item_offer = world
            .apply(
                a,
                PlayerAction::Trade(TradeAction::OfferItem(ItemSelection::one("token"))),
            )
            .unwrap();
        replay_events.extend(item_offer.domain_events);
        let coin_offer = world
            .apply(b, PlayerAction::Trade(TradeAction::OfferCoins(5)))
            .unwrap();
        replay_events.extend(coin_offer.domain_events);
        let bag_offer = world
            .apply(
                a,
                PlayerAction::Trade(TradeAction::OfferItem(ItemSelection::one("satchel"))),
            )
            .unwrap();
        replay_events.extend(bag_offer.domain_events);

        let a_view = world.player_state_view(a).unwrap();
        assert_eq!(
            a_view
                .inventory
                .iter()
                .map(|item| item.quantity)
                .sum::<u16>(),
            3
        );
        let available = a_view
            .inventory
            .iter()
            .find(|item| item.available_quantity == 1)
            .unwrap();
        let reserved = a_view
            .inventory
            .iter()
            .find(|item| item.reserved_for_trade == 1)
            .unwrap();
        assert!(available.can_trade);
        assert!(!reserved.can_trade);
        assert_ne!(available.id, reserved.id);
        let trade_view = a_view.trade.as_ref().unwrap();
        assert_eq!(trade_view.state_revision, 5);
        assert!(trade_view.counterparty.starts_with('o'));
        assert!(!trade_view.counterparty.contains("Bryn"));
        assert_ne!(
            trade_view.counterparty,
            world
                .player_state_view(b)
                .unwrap()
                .trade
                .unwrap()
                .counterparty
        );
        assert_eq!(world.reserved_coins(CharacterId(802)), 5);
        assert_eq!(world.spendable_coins(CharacterId(802)).unwrap(), 35);
        let status = world
            .apply(a, PlayerAction::Trade(TradeAction::Status))
            .unwrap()
            .output
            .unwrap();
        assert!(status.contains("Trade o"));
        assert!(status.contains("with Bryn"));
        assert!(status.contains("Your offer"));
        assert!(status.contains("Counterpart offer: 5 coin"));
        assert!(status.contains("reserved"));
        assert!(status.contains("spendable"));
        assert!(status.contains("ref o"));
        assert!(!status.contains("give:token"));
        assert!(!status.contains("give:bag"));
        let bag_target = trade_view
            .own_offer
            .lines
            .iter()
            .find(|line| line.container)
            .unwrap()
            .target
            .clone();
        let contents = world
            .apply(
                a,
                PlayerAction::Trade(TradeAction::Contents {
                    container: bag_target,
                    page: 1,
                }),
            )
            .unwrap()
            .output
            .unwrap();
        assert!(contents.contains("Offered container contents, page 1/1"));
        assert!(contents.contains("moon pebble"));
        assert!(contents.contains("ref o"));
        assert!(!contents.contains("give:pebble-instance"));
        assert!(matches!(
            world.apply(
                a,
                PlayerAction::Drop(ItemSelection::one(reserved.id.clone()))
            ),
            Err(WorldError::ItemReservedForTrade)
        ));
        assert!(matches!(
            world.apply(
                a,
                PlayerAction::Trade(TradeAction::Confirm { state_revision: 3 })
            ),
            Err(WorldError::StaleTradeRevision)
        ));

        let first_confirmation = world
            .apply(
                a,
                PlayerAction::Trade(TradeAction::Confirm { state_revision: 5 }),
            )
            .unwrap();
        replay_events.extend(first_confirmation.domain_events);
        let checkpoint = world.checkpoint();
        let completed = world
            .apply(
                b,
                PlayerAction::Trade(TradeAction::Confirm { state_revision: 5 }),
            )
            .unwrap();
        assert!(matches!(
            completed.domain_events.as_slice(),
            [DomainEvent::TradeCompleted { .. }]
        ));
        replay_events.extend(completed.domain_events.clone());
        assert!(world.trades.is_empty());
        assert_eq!(
            world
                .character_gameplay
                .get(&CharacterId(801))
                .unwrap()
                .coins,
            45
        );
        assert_eq!(
            world
                .character_gameplay
                .get(&CharacterId(802))
                .unwrap()
                .coins,
            35
        );
        assert_eq!(
            world
                .runtime_snapshot()
                .item_instances
                .values()
                .filter(|instance| instance.location
                    == RuntimeItemLocation::Carried {
                        character_id: CharacterId(802)
                    })
                .count(),
            2
        );

        let mut replay = World::from_definition(definition).unwrap();
        replay
            .replay_persisted_event(&take.domain_events[0])
            .unwrap();
        replay
            .replay_persisted_event(&take_bag.domain_events[0])
            .unwrap();
        for event in &replay_events {
            replay.replay_persisted_event(event).unwrap();
        }
        assert!(replay.runtime_snapshot().trades.is_empty());
        assert_eq!(
            replay
                .runtime_snapshot()
                .item_instances
                .values()
                .filter(|instance| instance.location
                    == RuntimeItemLocation::Carried {
                        character_id: CharacterId(802)
                    })
                .count(),
            2
        );

        world.restore_checkpoint(checkpoint);
        assert_eq!(world.trades.len(), 1);
        assert_eq!(
            world
                .character_gameplay
                .get(&CharacterId(801))
                .unwrap()
                .coins,
            40
        );
        assert_eq!(
            world
                .character_gameplay
                .get(&CharacterId(802))
                .unwrap()
                .coins,
            40
        );
    }

    #[test]
    fn trade_candidate_enforces_the_global_reservation_cap_and_portable_semantics() {
        let mut prototype = item_prototype(
            "cap:item",
            "capacity token",
            "A token used to exercise the global reservation boundary.",
            &["token"],
        );
        prototype.stack_policy = ItemStackPolicy::Fungible;
        prototype.max_stack_units = 256;
        let item_ids = (0..=MAX_RESERVED_TRADE_ROOTS)
            .map(|index| ItemInstanceId(format!("cap:item-{index:04}")))
            .collect::<Vec<_>>();
        let mut world = World::from_definition(WorldDefinition {
            start_room_id: "cap:start".into(),
            item_prototypes: vec![prototype],
            initial_item_instances: item_ids
                .iter()
                .map(|item_id| {
                    item_instance(
                        item_id.as_str(),
                        "cap:item",
                        InitialItemLocation::Room("cap:start".into()),
                    )
                })
                .collect(),
            rooms: vec![RoomDefinition {
                stable_id: "cap:start".into(),
                title: "Capacity".into(),
                prose: vec!["A bounded reservation fixture.".into()],
                image: None,
                audio: None,
                exits: Vec::new(),
                item_ids: Vec::new(),
            }],
        })
        .unwrap();
        let make_trade = |id: u64, roots: Vec<ItemInstanceId>| {
            let requester = CharacterId(10_000 + i64::try_from(id).unwrap() * 2);
            let recipient = CharacterId(requester.0 + 1);
            let lines = (!roots.is_empty())
                .then(|| TradeOfferLine {
                    key: TradeOfferKey::Fungible {
                        prototype_id: "cap:item".into(),
                    },
                    item_ids: roots,
                })
                .into_iter()
                .collect();
            TradeSnapshot {
                id: TradeId(id),
                requester,
                recipient,
                room_id: "cap:start".into(),
                phase: TradePhase::Negotiating,
                created_tick: 0,
                opened_tick: Some(0),
                expires_at_tick: TRADE_NEGOTIATION_TICKS,
                state_revision: 2,
                offers: BTreeMap::from([
                    (requester, TradeOffer { lines, coins: 0 }),
                    (recipient, TradeOffer::default()),
                ]),
                confirmations: BTreeMap::new(),
            }
        };
        for trade_index in 0..16_usize {
            let start = trade_index * usize::from(MAX_TRADE_ITEM_QUANTITY);
            let end = start + usize::from(MAX_TRADE_ITEM_QUANTITY);
            let roots = item_ids[start..end].to_vec();
            let trade = make_trade(u64::try_from(trade_index + 1).unwrap(), roots.clone());
            for root in roots {
                world.item_instances.get_mut(&root).unwrap().location =
                    SharedItemLocation::Carried(trade.requester);
            }
            world.trades.insert(trade.id, trade);
        }
        let previous = make_trade(17, Vec::new());
        world.trades.insert(previous.id, previous.clone());
        world.next_trade_id = TradeId(18);
        validate_trade_state(
            world.next_trade_id,
            &world.trades,
            &world.item_instances,
            &world.item_prototypes,
            &world.stable_rooms,
        )
        .unwrap();
        validate_world_runtime_snapshot_semantics(&world.runtime_snapshot()).unwrap();

        let overflow_root = item_ids[MAX_RESERVED_TRADE_ROOTS].clone();
        world
            .item_instances
            .get_mut(&overflow_root)
            .unwrap()
            .location = SharedItemLocation::Carried(previous.requester);
        let current = make_trade(17, vec![overflow_root]);
        let revision = world.revision;
        assert!(matches!(
            world.publish_trade_offer_change(SessionId(99_999), previous.clone(), current.clone()),
            Err(WorldError::InvalidRuntimeSnapshot)
        ));
        assert_eq!(world.trades.get(&TradeId(17)), Some(&previous));
        assert_eq!(world.revision, revision);

        let mut overflow_snapshot = world.runtime_snapshot();
        overflow_snapshot.trades.insert(TradeId(17), current);
        assert!(matches!(
            validate_world_runtime_snapshot_semantics(&overflow_snapshot),
            Err(WorldError::InvalidRuntimeSnapshot)
        ));
    }

    #[test]
    fn trade_replay_rejects_id_reuse_and_forged_transitions_or_confirmations() {
        let requested = TradeSnapshot {
            id: TradeId(1),
            requester: CharacterId(901),
            recipient: CharacterId(902),
            room_id: "give:start".into(),
            phase: TradePhase::Requested,
            created_tick: 0,
            opened_tick: None,
            expires_at_tick: TRADE_REQUEST_TICKS,
            state_revision: 1,
            offers: BTreeMap::new(),
            confirmations: BTreeMap::new(),
        };
        let mut anti_reuse = World::from_definition(give_definition()).unwrap();
        anti_reuse
            .replay_persisted_event(&DomainEvent::TradeRequested {
                trade: requested.clone(),
            })
            .unwrap();
        anti_reuse
            .replay_persisted_event(&DomainEvent::TradeCancelled {
                previous_trade: requested.clone(),
                reason: TradeCancellationReason::CancelledByParticipant,
            })
            .unwrap();
        assert!(matches!(
            anti_reuse.replay_persisted_event(&DomainEvent::TradeRequested {
                trade: requested.clone(),
            }),
            Err(WorldError::InvalidDomainTransition)
        ));

        let mut replay = World::from_definition(give_definition()).unwrap();
        for (session, character_id, name) in [
            (SessionId(901), CharacterId(901), "Aster"),
            (SessionId(902), CharacterId(902), "Bryn"),
        ] {
            replay
                .connect(
                    session,
                    CharacterSnapshot {
                        id: character_id,
                        account_id: AccountId(character_id.0),
                        name: name.into(),
                        room_id: "give:start".into(),
                        item_locations: BTreeMap::new(),
                    },
                )
                .unwrap();
        }
        replay
            .replay_persisted_event(&DomainEvent::TradeRequested {
                trade: requested.clone(),
            })
            .unwrap();
        let mut opened = requested.clone();
        opened.phase = TradePhase::Negotiating;
        opened.opened_tick = Some(0);
        opened.expires_at_tick = TRADE_NEGOTIATION_TICKS;
        opened.state_revision = 2;
        opened.offers = BTreeMap::from([
            (opened.requester, TradeOffer::default()),
            (opened.recipient, TradeOffer::default()),
        ]);
        let mut opened_after_expiry = opened.clone();
        opened_after_expiry.opened_tick = Some(TRADE_REQUEST_TICKS);
        opened_after_expiry.expires_at_tick = TRADE_REQUEST_TICKS + TRADE_NEGOTIATION_TICKS;
        assert!(matches!(
            replay.replay_persisted_event(&DomainEvent::TradeOpened {
                previous: requested.clone(),
                current: opened_after_expiry,
            }),
            Err(WorldError::InvalidDomainTransition)
        ));
        let mut forged_open = opened.clone();
        forged_open.room_id = "give:remote".into();
        assert!(matches!(
            replay.replay_persisted_event(&DomainEvent::TradeOpened {
                previous: requested.clone(),
                current: forged_open,
            }),
            Err(WorldError::InvalidDomainTransition)
        ));
        replay
            .replay_persisted_event(&DomainEvent::TradeOpened {
                previous: requested,
                current: opened.clone(),
            })
            .unwrap();

        let mut offered = opened.clone();
        offered.state_revision = 3;
        offered.offers.get_mut(&offered.requester).unwrap().coins = 1;
        let mut forged_offer = offered.clone();
        forged_offer
            .offers
            .get_mut(&forged_offer.recipient)
            .unwrap()
            .coins = 1;
        assert!(matches!(
            replay.replay_persisted_event(&DomainEvent::TradeOfferSet {
                previous: opened.clone(),
                current: forged_offer,
            }),
            Err(WorldError::InvalidDomainTransition)
        ));
        replay
            .replay_persisted_event(&DomainEvent::TradeOfferSet {
                previous: opened,
                current: offered.clone(),
            })
            .unwrap();

        let mut forged_invalidation = offered.clone();
        forged_invalidation.state_revision = 4;
        forged_invalidation
            .offers
            .get_mut(&forged_invalidation.requester)
            .unwrap()
            .coins = 2;
        assert!(matches!(
            replay.replay_persisted_event(&DomainEvent::TradeInvalidated {
                previous: offered.clone(),
                current: forged_invalidation,
            }),
            Err(WorldError::InvalidDomainTransition)
        ));

        let basis = replay.trade_basis_sha256(&offered).unwrap();
        let confirmation = TradeConfirmation {
            state_revision: offered.state_revision,
            basis_sha256: basis,
        };
        let mut forged_confirmation = confirmation.clone();
        forged_confirmation.basis_sha256 = [0x5a; 32];
        assert!(matches!(
            replay.replay_persisted_event(&DomainEvent::TradeConfirmed {
                participant: offered.requester,
                confirmation: forged_confirmation,
            }),
            Err(WorldError::InvalidDomainTransition)
        ));
        replay
            .replay_persisted_event(&DomainEvent::TradeConfirmed {
                participant: offered.requester,
                confirmation: confirmation.clone(),
            })
            .unwrap();

        let mut completed = offered;
        completed
            .confirmations
            .insert(completed.requester, confirmation.clone());
        completed
            .confirmations
            .insert(completed.recipient, confirmation.clone());
        let mut outsider = completed.clone();
        outsider.confirmations.remove(&outsider.recipient);
        outsider
            .confirmations
            .insert(CharacterId(999), confirmation.clone());
        assert!(matches!(
            replay.replay_persisted_event(&DomainEvent::TradeCompleted {
                previous_trade: outsider,
            }),
            Err(WorldError::InvalidDomainTransition)
        ));
        let mut bad_digest = completed.clone();
        bad_digest
            .confirmations
            .get_mut(&bad_digest.recipient)
            .unwrap()
            .basis_sha256 = [0xa5; 32];
        assert!(matches!(
            replay.replay_persisted_event(&DomainEvent::TradeCompleted {
                previous_trade: bad_digest,
            }),
            Err(WorldError::InvalidDomainTransition)
        ));
        replay
            .replay_persisted_event(&DomainEvent::TradeCompleted {
                previous_trade: completed,
            })
            .unwrap();
        assert!(replay.trades.is_empty());
    }

    #[test]
    fn cold_trade_replay_rejects_assetless_arbitrary_confirmations() {
        let requested = TradeSnapshot {
            id: TradeId(1),
            requester: CharacterId(961),
            recipient: CharacterId(962),
            room_id: "give:start".into(),
            phase: TradePhase::Requested,
            created_tick: 0,
            opened_tick: None,
            expires_at_tick: TRADE_REQUEST_TICKS,
            state_revision: 1,
            offers: BTreeMap::new(),
            confirmations: BTreeMap::new(),
        };
        let mut opened = requested.clone();
        opened.phase = TradePhase::Negotiating;
        opened.opened_tick = Some(0);
        opened.expires_at_tick = TRADE_NEGOTIATION_TICKS;
        opened.state_revision = 2;
        opened.offers = BTreeMap::from([
            (opened.requester, TradeOffer::default()),
            (opened.recipient, TradeOffer::default()),
        ]);
        let mut replay = World::from_definition(give_definition()).unwrap();
        replay
            .replay_persisted_event(&DomainEvent::TradeRequested {
                trade: requested.clone(),
            })
            .unwrap();
        replay
            .replay_persisted_event(&DomainEvent::TradeOpened {
                previous: requested,
                current: opened.clone(),
            })
            .unwrap();
        assert!(replay.character_gameplay.is_empty());
        let arbitrary = TradeConfirmation {
            state_revision: opened.state_revision,
            basis_sha256: [0x42; 32],
        };
        assert!(matches!(
            replay.replay_persisted_event(&DomainEvent::TradeConfirmed {
                participant: opened.requester,
                confirmation: arbitrary.clone(),
            }),
            Err(WorldError::InvalidDomainTransition)
        ));

        let mut one_confirmation = opened.clone();
        one_confirmation
            .confirmations
            .insert(opened.requester, arbitrary.clone());
        replay.trades.insert(opened.id, one_confirmation);
        let mut completed = opened;
        completed
            .confirmations
            .insert(completed.requester, arbitrary.clone());
        completed
            .confirmations
            .insert(completed.recipient, arbitrary);
        assert!(matches!(
            replay.replay_persisted_event(&DomainEvent::TradeCompleted {
                previous_trade: completed,
            }),
            Err(WorldError::InvalidDomainTransition)
        ));
        assert_eq!(replay.trades.len(), 1);
    }

    #[test]
    fn trade_rate_expiry_and_tick_counters_fail_closed() {
        let mut world = World::from_definition(give_definition()).unwrap();
        for (session, character_id, name) in [
            (SessionId(911), CharacterId(911), "Aster"),
            (SessionId(912), CharacterId(912), "Bryn"),
        ] {
            world
                .connect(
                    session,
                    CharacterSnapshot {
                        id: character_id,
                        account_id: AccountId(character_id.0),
                        name: name.into(),
                        room_id: "give:start".into(),
                        item_locations: BTreeMap::new(),
                    },
                )
                .unwrap();
        }
        world
            .apply(
                SessionId(911),
                PlayerAction::Trade(TradeAction::Request("Bryn".into())),
            )
            .unwrap();
        world
            .apply(SessionId(911), PlayerAction::Trade(TradeAction::Cancel))
            .unwrap();
        let rate_checkpoint = world.checkpoint();
        assert!(matches!(
            world.apply(
                SessionId(911),
                PlayerAction::Trade(TradeAction::Request("Bryn".into()))
            ),
            Err(WorldError::TradeUnavailable)
        ));
        for _ in 0..TRADE_REQUEST_RATE_TICKS {
            world.advance_tick().unwrap();
        }
        world.restore_checkpoint(rate_checkpoint);
        assert!(matches!(
            world.apply(
                SessionId(911),
                PlayerAction::Trade(TradeAction::Request("Bryn".into()))
            ),
            Err(WorldError::TradeUnavailable)
        ));
        for _ in 0..TRADE_REQUEST_RATE_TICKS {
            world.advance_tick().unwrap();
        }
        world
            .apply(
                SessionId(911),
                PlayerAction::Trade(TradeAction::Request("Bryn".into())),
            )
            .unwrap();

        world.logical_tick = TRADE_REQUEST_RATE_TICKS + TRADE_REQUEST_TICKS;
        world.revision = u64::MAX;
        let live_trade = world.trades.values().next().unwrap().clone();
        assert!(matches!(
            world.cancel_expired_trades(),
            Err(WorldError::InvalidDomainTransition)
        ));
        assert_eq!(world.trades.get(&live_trade.id), Some(&live_trade));
        world.revision = 1;
        let events = world.cancel_expired_trades().unwrap();
        assert!(matches!(
            events.as_slice(),
            [DomainEvent::TradeCancelled {
                reason: TradeCancellationReason::Expired,
                ..
            }]
        ));
        assert!(world.trades.is_empty());

        let mut overflow_tick = World::from_definition(give_definition()).unwrap();
        overflow_tick.logical_tick = u64::MAX;
        let before = overflow_tick.runtime_snapshot();
        assert!(matches!(
            overflow_tick.advance_tick(),
            Err(WorldError::InvalidDomainTransition)
        ));
        assert_eq!(overflow_tick.runtime_snapshot(), before);
    }

    #[test]
    fn trade_status_is_text_bounded_and_keeps_opaque_actionable_inventory() {
        let prototypes = (0..1_000)
            .map(|index| {
                let mut prototype = item_prototype(
                    &format!("status:item-{index:04}"),
                    &format!("status item {index:04}"),
                    "A unique item used to bound trade status output.",
                    &[],
                );
                prototype.weight_grams = 1;
                prototype
            })
            .collect::<Vec<_>>();
        let instances = (0..1_000)
            .map(|index| {
                item_instance(
                    &format!("status:instance-{index:04}"),
                    &format!("status:item-{index:04}"),
                    InitialItemLocation::Room("status:start".into()),
                )
            })
            .collect::<Vec<_>>();
        let mut world = World::from_definition(WorldDefinition {
            start_room_id: "status:start".into(),
            item_prototypes: prototypes,
            initial_item_instances: instances,
            rooms: vec![RoomDefinition {
                stable_id: "status:start".into(),
                title: "Status".into(),
                prose: vec!["A bounded text fixture.".into()],
                image: None,
                audio: None,
                exits: Vec::new(),
                item_ids: Vec::new(),
            }],
        })
        .unwrap();
        for (session, character_id, name) in [
            (SessionId(921), CharacterId(921), "Aster"),
            (SessionId(922), CharacterId(922), "Bryn"),
        ] {
            world
                .connect(
                    session,
                    CharacterSnapshot {
                        id: character_id,
                        account_id: AccountId(character_id.0),
                        name: name.into(),
                        room_id: "status:start".into(),
                        item_locations: BTreeMap::new(),
                    },
                )
                .unwrap();
        }
        for instance in world.item_instances.values_mut() {
            instance.location = SharedItemLocation::Carried(CharacterId(921));
        }
        world
            .character_gameplay
            .get_mut(&CharacterId(921))
            .unwrap()
            .inventory_capacity = InventoryCapacity {
            max_slots: 1_000,
            max_weight_grams: 1_000_000,
        };
        world
            .apply(
                SessionId(921),
                PlayerAction::Trade(TradeAction::Request("Bryn".into())),
            )
            .unwrap();
        let snapshot = world.player_state_view(SessionId(921)).unwrap();
        let counterparty = &snapshot.trade.as_ref().unwrap().counterparty;
        assert!(counterparty.starts_with('o'));
        assert!(!counterparty.contains("Bryn"));
        let status = world
            .apply(SessionId(921), PlayerAction::Trade(TradeAction::Status))
            .unwrap()
            .output
            .unwrap();
        assert!(status.len() <= MAX_TRADE_STATUS_TEXT_BYTES);
        assert!(status.contains("with Bryn"));
        assert!(status.contains("980 more inventory groups omitted"));
        assert!(status.contains("trade contents <offered-container> <page>"));
        assert!(status.contains("ref o"));
        assert!(!status.contains("status:instance-"));
    }

    #[test]
    fn trade_settlement_uses_net_capacity_and_reserved_subtrees_block_every_item_bypass() {
        let swap_definition = || WorldDefinition {
            start_room_id: "swap:start".into(),
            item_prototypes: vec![
                item_prototype(
                    "swap:amber",
                    "amber token",
                    "A unique amber token.",
                    &["amber"],
                ),
                item_prototype(
                    "swap:azure",
                    "azure token",
                    "A unique azure token.",
                    &["azure"],
                ),
            ],
            initial_item_instances: vec![
                item_instance(
                    "swap:amber-instance",
                    "swap:amber",
                    InitialItemLocation::Room("swap:start".into()),
                ),
                item_instance(
                    "swap:azure-instance",
                    "swap:azure",
                    InitialItemLocation::Room("swap:start".into()),
                ),
            ],
            rooms: vec![RoomDefinition {
                stable_id: "swap:start".into(),
                title: "Swap".into(),
                prose: vec!["A net-capacity fixture.".into()],
                image: None,
                audio: None,
                exits: Vec::new(),
                item_ids: Vec::new(),
            }],
        };
        let connect_swap = |world: &mut World| {
            for (session, character_id, name) in [
                (SessionId(931), CharacterId(931), "Aster"),
                (SessionId(932), CharacterId(932), "Bryn"),
            ] {
                world
                    .connect(
                        session,
                        CharacterSnapshot {
                            id: character_id,
                            account_id: AccountId(character_id.0),
                            name: name.into(),
                            room_id: "swap:start".into(),
                            item_locations: BTreeMap::new(),
                        },
                    )
                    .unwrap();
                world
                    .character_gameplay
                    .get_mut(&character_id)
                    .unwrap()
                    .inventory_capacity = InventoryCapacity {
                    max_slots: 1,
                    max_weight_grams: 2_000,
                };
            }
            world
                .apply(
                    SessionId(931),
                    PlayerAction::Take(ItemSelection::one("amber")),
                )
                .unwrap();
            world
                .apply(
                    SessionId(932),
                    PlayerAction::Take(ItemSelection::one("azure")),
                )
                .unwrap();
        };
        let mut swap = World::from_definition(swap_definition()).unwrap();
        connect_swap(&mut swap);
        swap.apply(
            SessionId(931),
            PlayerAction::Trade(TradeAction::Request("Bryn".into())),
        )
        .unwrap();
        swap.apply(SessionId(932), PlayerAction::Trade(TradeAction::Accept))
            .unwrap();
        swap.apply(
            SessionId(931),
            PlayerAction::Trade(TradeAction::OfferItem(ItemSelection::one("amber"))),
        )
        .unwrap();
        swap.apply(
            SessionId(932),
            PlayerAction::Trade(TradeAction::OfferItem(ItemSelection::one("azure"))),
        )
        .unwrap();
        swap.apply(
            SessionId(931),
            PlayerAction::Trade(TradeAction::Confirm { state_revision: 4 }),
        )
        .unwrap();
        swap.apply(
            SessionId(932),
            PlayerAction::Trade(TradeAction::Confirm { state_revision: 4 }),
        )
        .unwrap();
        assert_eq!(
            swap.item_location(&ItemInstanceId("swap:amber-instance".into())),
            Some(&SharedItemLocation::Carried(CharacterId(932)))
        );
        assert_eq!(
            swap.item_location(&ItemInstanceId("swap:azure-instance".into())),
            Some(&SharedItemLocation::Carried(CharacterId(931)))
        );

        let mut over_capacity = World::from_definition(swap_definition()).unwrap();
        connect_swap(&mut over_capacity);
        over_capacity
            .apply(
                SessionId(931),
                PlayerAction::Trade(TradeAction::Request("Bryn".into())),
            )
            .unwrap();
        over_capacity
            .apply(SessionId(932), PlayerAction::Trade(TradeAction::Accept))
            .unwrap();
        over_capacity
            .apply(
                SessionId(931),
                PlayerAction::Trade(TradeAction::OfferItem(ItemSelection::one("amber"))),
            )
            .unwrap();
        over_capacity
            .apply(
                SessionId(931),
                PlayerAction::Trade(TradeAction::Confirm { state_revision: 3 }),
            )
            .unwrap();
        assert!(matches!(
            over_capacity.apply(
                SessionId(932),
                PlayerAction::Trade(TradeAction::Confirm { state_revision: 3 })
            ),
            Err(WorldError::InventoryCapacityExceeded)
        ));
        assert_eq!(over_capacity.trades.len(), 1);

        let mut bag = item_prototype(
            "reserved:bag",
            "trade satchel",
            "A reserved container.",
            &["satchel"],
        );
        bag.container = Some(ItemContainerDefinition {
            max_slots: 4,
            max_weight_grams: 4_000,
        });
        let mut herb = item_prototype(
            "reserved:herb",
            "healing herb",
            "A herb inside the reserved container.",
            &["herb"],
        );
        herb.heal_amount = 1;
        herb.consumable = true;
        let spare = item_prototype(
            "reserved:spare",
            "spare stone",
            "A free item that must not enter a reserved container.",
            &["spare"],
        );
        let mut reserved = World::from_definition(WorldDefinition {
            start_room_id: "reserved:start".into(),
            item_prototypes: vec![bag, herb, spare],
            initial_item_instances: vec![
                item_instance(
                    "reserved:bag-instance",
                    "reserved:bag",
                    InitialItemLocation::Room("reserved:start".into()),
                ),
                item_instance(
                    "reserved:herb-instance",
                    "reserved:herb",
                    InitialItemLocation::Container("reserved:bag-instance".into()),
                ),
                item_instance(
                    "reserved:spare-instance",
                    "reserved:spare",
                    InitialItemLocation::Room("reserved:start".into()),
                ),
            ],
            rooms: vec![RoomDefinition {
                stable_id: "reserved:start".into(),
                title: "Reserved".into(),
                prose: vec!["A reservation bypass fixture.".into()],
                image: None,
                audio: None,
                exits: Vec::new(),
                item_ids: Vec::new(),
            }],
        })
        .unwrap();
        for (session, character_id, name) in [
            (SessionId(941), CharacterId(941), "Aster"),
            (SessionId(942), CharacterId(942), "Bryn"),
        ] {
            reserved
                .connect(
                    session,
                    CharacterSnapshot {
                        id: character_id,
                        account_id: AccountId(character_id.0),
                        name: name.into(),
                        room_id: "reserved:start".into(),
                        item_locations: BTreeMap::new(),
                    },
                )
                .unwrap();
        }
        reserved
            .apply(
                SessionId(941),
                PlayerAction::Take(ItemSelection::one("satchel")),
            )
            .unwrap();
        reserved
            .apply(
                SessionId(941),
                PlayerAction::Take(ItemSelection::one("spare")),
            )
            .unwrap();
        reserved
            .apply(
                SessionId(941),
                PlayerAction::Trade(TradeAction::Request("Bryn".into())),
            )
            .unwrap();
        reserved
            .apply(SessionId(942), PlayerAction::Trade(TradeAction::Accept))
            .unwrap();
        reserved
            .apply(
                SessionId(941),
                PlayerAction::Trade(TradeAction::OfferItem(ItemSelection::one("satchel"))),
            )
            .unwrap();
        let bag_id = ItemInstanceId("reserved:bag-instance".into());
        let herb_id = ItemInstanceId("reserved:herb-instance".into());
        assert!(reserved.item_reserved_for_trade(&bag_id).unwrap());
        assert!(reserved.item_reserved_for_trade(&herb_id).unwrap());
        let contents = reserved
            .apply(
                SessionId(941),
                PlayerAction::Contents {
                    container: "satchel".into(),
                    page: 1,
                },
            )
            .unwrap();
        let contents_view = contents.container_contents.as_ref().unwrap();
        let herb_view = contents_view
            .items
            .iter()
            .find(|item| item.label == "healing herb")
            .unwrap();
        assert_eq!(herb_view.quantity, 1);
        assert_eq!(herb_view.available_quantity, 0);
        assert_eq!(herb_view.reserved_for_trade, 1);
        assert!(!herb_view.can_trade);
        assert!(
            herb_view
                .actions
                .iter()
                .all(|action| action.label != "Get" && !action.command.starts_with("get "))
        );
        assert!(
            contents
                .output
                .as_deref()
                .unwrap()
                .contains("1 reserved for trade; 0 available")
        );
        let blocked = [
            PlayerAction::Drop(ItemSelection::one("satchel")),
            PlayerAction::Give {
                selection: ItemSelection::one("satchel"),
                recipient: "Bryn".into(),
            },
            PlayerAction::Put {
                selection: ItemSelection::one("spare"),
                container: "satchel".into(),
            },
            PlayerAction::Get {
                selection: ItemSelection::one("herb"),
                container: "satchel".into(),
            },
            PlayerAction::Use("satchel".into()),
            PlayerAction::Equip("satchel".into()),
        ];
        for action in blocked {
            assert!(matches!(
                reserved.apply(SessionId(941), action),
                Err(WorldError::ItemReservedForTrade)
            ));
        }
    }

    #[test]
    fn trade_revision_id_and_coin_overflows_are_atomic_and_shop_uses_spendable_coins() {
        let connect_pair = |world: &mut World| {
            for (session, character_id, name) in [
                (SessionId(951), CharacterId(951), "Aster"),
                (SessionId(952), CharacterId(952), "Bryn"),
            ] {
                world
                    .connect(
                        session,
                        CharacterSnapshot {
                            id: character_id,
                            account_id: AccountId(character_id.0),
                            name: name.into(),
                            room_id: "give:start".into(),
                            item_locations: BTreeMap::new(),
                        },
                    )
                    .unwrap();
            }
        };
        let mut revision_overflow = World::from_definition(give_definition()).unwrap();
        connect_pair(&mut revision_overflow);
        revision_overflow
            .apply(
                SessionId(951),
                PlayerAction::Trade(TradeAction::Request("Bryn".into())),
            )
            .unwrap();
        revision_overflow
            .apply(SessionId(952), PlayerAction::Trade(TradeAction::Accept))
            .unwrap();
        revision_overflow
            .trades
            .get_mut(&TradeId(1))
            .unwrap()
            .state_revision = u64::MAX;
        let before = revision_overflow.runtime_snapshot();
        assert!(matches!(
            revision_overflow.apply(
                SessionId(951),
                PlayerAction::Trade(TradeAction::OfferCoins(1))
            ),
            Err(WorldError::InvalidDomainTransition)
        ));
        assert_eq!(revision_overflow.runtime_snapshot(), before);

        let mut id_overflow = World::from_definition(give_definition()).unwrap();
        connect_pair(&mut id_overflow);
        id_overflow.next_trade_id = TradeId(u64::MAX);
        assert!(matches!(
            id_overflow.apply(
                SessionId(951),
                PlayerAction::Trade(TradeAction::Request("Bryn".into()))
            ),
            Err(WorldError::InvalidDomainTransition)
        ));
        assert!(id_overflow.trades.is_empty());
        assert_eq!(id_overflow.next_trade_id, TradeId(u64::MAX));

        let mut coin_overflow = World::from_definition(give_definition()).unwrap();
        connect_pair(&mut coin_overflow);
        coin_overflow
            .character_gameplay
            .get_mut(&CharacterId(951))
            .unwrap()
            .coins = u64::MAX;
        coin_overflow
            .apply(
                SessionId(951),
                PlayerAction::Trade(TradeAction::Request("Bryn".into())),
            )
            .unwrap();
        coin_overflow
            .apply(SessionId(952), PlayerAction::Trade(TradeAction::Accept))
            .unwrap();
        coin_overflow
            .apply(
                SessionId(952),
                PlayerAction::Trade(TradeAction::OfferCoins(1)),
            )
            .unwrap();
        coin_overflow
            .apply(
                SessionId(951),
                PlayerAction::Trade(TradeAction::Confirm { state_revision: 3 }),
            )
            .unwrap();
        assert!(matches!(
            coin_overflow.apply(
                SessionId(952),
                PlayerAction::Trade(TradeAction::Confirm { state_revision: 3 })
            ),
            Err(WorldError::InsufficientFunds)
        ));
        assert_eq!(coin_overflow.trades.len(), 1);

        let mut shop = World::from_definition(give_definition()).unwrap();
        connect_pair(&mut shop);
        shop.apply(
            SessionId(951),
            PlayerAction::Trade(TradeAction::Request("Bryn".into())),
        )
        .unwrap();
        shop.apply(SessionId(952), PlayerAction::Trade(TradeAction::Accept))
            .unwrap();
        shop.apply(
            SessionId(951),
            PlayerAction::Trade(TradeAction::OfferCoins(35)),
        )
        .unwrap();
        let tonic = shop
            .player_state_view(SessionId(951))
            .unwrap()
            .economy
            .shop
            .into_iter()
            .find(|item| item.id == "tonic")
            .unwrap();
        assert!(
            !tonic
                .actions
                .iter()
                .any(|action| action.command == "buy tonic")
        );
        assert!(matches!(
            shop.apply(
                SessionId(951),
                PlayerAction::Economy(EconomyAction::Buy("tonic".into()))
            ),
            Err(WorldError::InsufficientFunds)
        ));
    }

    #[test]
    fn snapshot_v4_requires_valid_binding_and_rejects_policy_drift() {
        let (mut world, actor) = connect_definition(policy_definition());
        world
            .apply(actor, PlayerAction::Take(ItemSelection::one("bag")))
            .unwrap();
        let snapshot = world.runtime_snapshot();
        let encoded = serde_json::to_string(&snapshot).unwrap();
        assert!(encoded.contains("\"snapshotVersion\":7"));
        assert!(encoded.contains("\"binding\":{\"kind\":\"character\",\"characterId\":701}"));
        let mut restored = World::from_definition(policy_definition()).unwrap();
        restored.restore_runtime_snapshot(snapshot.clone()).unwrap();
        assert!(
            restored
                .runtime_item_owner_ids()
                .contains(&CharacterId(701))
        );

        let clasp_id = ItemInstanceId("policy:clasp-instance".into());
        let mut zero = snapshot.clone();
        zero.item_instances.get_mut(&clasp_id).unwrap().binding = ItemBinding::Character {
            character_id: CharacterId(0),
        };
        assert!(matches!(
            World::from_definition(policy_definition())
                .unwrap()
                .restore_runtime_snapshot(zero),
            Err(WorldError::InvalidRuntimeSnapshot)
        ));
        let mut zero_location = snapshot.clone();
        zero_location
            .item_instances
            .get_mut(&ItemInstanceId("policy:bag-instance".into()))
            .unwrap()
            .location = RuntimeItemLocation::Carried {
            character_id: CharacterId(0),
        };
        assert!(matches!(
            World::from_definition(policy_definition())
                .unwrap()
                .restore_runtime_snapshot(zero_location),
            Err(WorldError::InvalidRuntimeSnapshot)
        ));
        let mut unbound = snapshot.clone();
        unbound.item_instances.get_mut(&clasp_id).unwrap().binding = ItemBinding::Unbound;
        assert!(matches!(
            World::from_definition(policy_definition())
                .unwrap()
                .restore_runtime_snapshot(unbound),
            Err(WorldError::InvalidRuntimeSnapshot)
        ));
        let mut escaped = snapshot.clone();
        escaped.item_instances.get_mut(&clasp_id).unwrap().location = RuntimeItemLocation::Room {
            room_id: "test:start".into(),
        };
        assert!(matches!(
            World::from_definition(policy_definition())
                .unwrap()
                .restore_runtime_snapshot(escaped),
            Err(WorldError::InvalidRuntimeSnapshot)
        ));
        let mut forged_transferable = snapshot.clone();
        forged_transferable
            .item_instances
            .get_mut(&ItemInstanceId("policy:bag-instance".into()))
            .unwrap()
            .binding = ItemBinding::Character {
            character_id: CharacterId(701),
        };
        assert!(matches!(
            World::from_definition(policy_definition())
                .unwrap()
                .restore_runtime_snapshot(forged_transferable),
            Err(WorldError::InvalidRuntimeSnapshot)
        ));
        let mut drifted_definition = policy_definition();
        drifted_definition
            .item_prototypes
            .iter_mut()
            .find(|prototype| prototype.prototype_id == "policy:clasp")
            .unwrap()
            .transfer_policy = ItemTransferPolicy::Transferable;
        assert!(matches!(
            World::from_definition(drifted_definition)
                .unwrap()
                .restore_runtime_snapshot(snapshot),
            Err(WorldError::InvalidRuntimeSnapshot)
        ));

        let mut missing_binding: serde_json::Value = serde_json::from_str(&encoded).unwrap();
        missing_binding["itemInstances"]
            .as_object_mut()
            .unwrap()
            .values_mut()
            .next()
            .unwrap()
            .as_object_mut()
            .unwrap()
            .remove("binding");
        assert!(
            decode_world_runtime_snapshot(4, &serde_json::to_string(&missing_binding).unwrap())
                .is_err()
        );
    }

    #[test]
    fn item_graph_index_handles_thousands_of_empty_containers_in_one_pass() {
        let mut container = item_prototype(
            "test:container",
            "empty container",
            "A bounded container.",
            &["container"],
        );
        container.container = Some(ItemContainerDefinition {
            max_slots: 1,
            max_weight_grams: 1,
        });
        let instances = (0..4_096)
            .map(|index| {
                item_instance(
                    &format!("test:container-{index}"),
                    "test:container",
                    InitialItemLocation::Staged,
                )
            })
            .collect();
        let world = World::from_definition(WorldDefinition {
            start_room_id: "test:start".into(),
            item_prototypes: vec![container],
            initial_item_instances: instances,
            rooms: vec![RoomDefinition {
                stable_id: "test:start".into(),
                title: "Start".into(),
                prose: vec!["A test room.".into()],
                image: None,
                audio: None,
                item_ids: Vec::new(),
                exits: Vec::new(),
            }],
        })
        .unwrap();
        let graph = build_item_graph_index(&world.item_instances, &world.item_prototypes).unwrap();
        assert_eq!(graph.subtree_weights.len(), 4_096);
        assert!(graph.children.is_empty());
    }

    #[test]
    fn item_presentation_text_is_bounded_before_it_reaches_transports() {
        let definition_for = |item: ItemPrototypeDefinition| WorldDefinition {
            start_room_id: "test:start".into(),
            item_prototypes: vec![item],
            initial_item_instances: Vec::new(),
            rooms: vec![RoomDefinition {
                stable_id: "test:start".into(),
                title: "Start".into(),
                prose: vec!["A test room.".into()],
                image: None,
                audio: None,
                item_ids: Vec::new(),
                exits: Vec::new(),
            }],
        };
        let mut oversized = item_prototype("test:item", "safe", "Safe.", &["item"]);
        oversized.name = "x".repeat(129);
        assert!(matches!(
            World::from_definition(definition_for(oversized)),
            Err(WorldBuildError::InvalidItemPrototype(_))
        ));

        let mut control = item_prototype("test:item", "safe", "Safe.", &["item"]);
        control.description = "unsafe\u{1b}line".into();
        assert!(matches!(
            World::from_definition(definition_for(control)),
            Err(WorldBuildError::InvalidItemPrototype(_))
        ));

        let mut aliases = item_prototype("test:item", "safe", "Safe.", &["item"]);
        aliases.aliases = (0..33).map(|index| format!("alias-{index}")).collect();
        assert!(matches!(
            World::from_definition(definition_for(aliases)),
            Err(WorldBuildError::InvalidItemPrototype(_))
        ));

        let mut multiline = item_prototype("test:item", "safe", "Safe.", &["item"]);
        multiline.description = "First line.\n\tSecond line.".into();
        assert!(World::from_definition(definition_for(multiline)).is_ok());
    }

    #[test]
    fn offensive_ability_freezes_a_bounded_cohort_without_dropping_the_initiator() {
        let (mut world, initiator_session) = phase_two_world(9, 1, 100);
        world
            .spawn_npc(NpcDefinition {
                stable_id: "game:training-target".into(),
                name: "training target".into(),
                description: "A target sturdy enough to retain an encounter.".into(),
                room_id: "game:start".into(),
                level: 1,
                max_health: 100,
                strength: 1,
                defence: 0,
                experience_reward: 0,
                loot_item_ids: Vec::new(),
                respawn_ticks: 1,
            })
            .unwrap();
        for id in 1..=6 {
            world
                .connect(
                    SessionId(1_000 + id as u64),
                    CharacterSnapshot {
                        id: CharacterId(id),
                        account_id: AccountId(id),
                        name: format!("Peer {id}"),
                        room_id: "game:start".into(),
                        item_locations: BTreeMap::new(),
                    },
                )
                .unwrap();
        }
        world.adventure.next_party_id = 78;
        world.adventure.parties.insert(
            77,
            PartySnapshot {
                id: 77,
                leader: CharacterId(900),
                members: vec![
                    CharacterId(1),
                    CharacterId(2),
                    CharacterId(3),
                    CharacterId(4),
                    CharacterId(5),
                    CharacterId(6),
                    CharacterId(900),
                ],
                invitations: BTreeMap::new(),
            },
        );
        world.abilities.insert(
            "game:jab".into(),
            AbilityDefinition {
                id: "game:jab".into(),
                name: "Jab".into(),
                description: "A controlled strike.".into(),
                category: AbilityCategory::Martial,
                resource_costs: BTreeMap::new(),
                cooldown_ticks: 0,
                targeting: AbilityTargeting::SingleEnemy,
                damage: Some(AbilityDamageDefinition {
                    target: AbilityEffectTarget::Target,
                    damage_type: DamageType::Physical,
                    base_amount: 1,
                    scaling_attribute: None,
                    scaling_percent: 0,
                }),
                heal: None,
                status_effects: Vec::new(),
            },
        );
        world
            .character_gameplay
            .get_mut(&CharacterId(900))
            .unwrap()
            .ability_ids
            .push("game:jab".into());

        world
            .apply(
                initiator_session,
                PlayerAction::Ability {
                    ability_id: "game:jab".into(),
                    target: Some("training target".into()),
                },
            )
            .unwrap();
        let encounter = &world.loot_encounters["game:training-target"];
        assert_eq!(encounter.initiator, CharacterId(900));
        assert_eq!(encounter.cohort.len(), 6);
        assert!(encounter.cohort.contains(&CharacterId(900)));
        assert_eq!(
            encounter.cohort,
            vec![
                CharacterId(1),
                CharacterId(2),
                CharacterId(3),
                CharacterId(4),
                CharacterId(5),
                CharacterId(900),
            ]
        );
    }

    #[test]
    fn loot_definition_caps_include_fixed_roots_and_source_ids() {
        let (mut world, _) = phase_two_world(1, 1, 10);
        let table = LootTableDefinition {
            id: "game:oversized".into(),
            digest_sha256: "a".repeat(64),
            guaranteed: vec![
                LootGuaranteedDefinition {
                    id: "game:first".into(),
                    item_prototype_id: "game:potion".into(),
                    quantity: 32,
                    affix_pool_id: None,
                },
                LootGuaranteedDefinition {
                    id: "game:second".into(),
                    item_prototype_id: "game:potion".into(),
                    quantity: 32,
                    affix_pool_id: None,
                },
            ],
            pools: Vec::new(),
        };
        assert!(matches!(
            world.configure_loot(
                vec![table],
                vec![LootSourceDefinition {
                    kind: LootSourceKind::NpcSpawn,
                    id: "game:guardian-source".into(),
                    npc_spawn_id: "game:guardian".into(),
                    loot_table_id: "game:oversized".into(),
                    rights: LootRightsPolicy::CreditedCharacter,
                }],
            ),
            Err(WorldError::InvalidGameplayDefinition)
        ));

        let mut roots = Vec::new();
        for _ in 0..=MAX_LOOT_UNITS_PER_RESOLUTION {
            roots.push(world.create_staged_item_instance("game:sword").unwrap());
        }
        assert!(matches!(
            world.spawn_npc(NpcDefinition {
                stable_id: "game:too-much-fixed-loot".into(),
                name: "overburdened target".into(),
                description: "An invalid fixed-loot source.".into(),
                room_id: "game:start".into(),
                level: 1,
                max_health: 1,
                strength: 1,
                defence: 0,
                experience_reward: 0,
                loot_item_ids: roots,
                respawn_ticks: 1,
            }),
            Err(WorldError::InvalidGameplayDefinition)
        ));
    }

    #[test]
    fn fixed_loot_wins_the_last_claim_and_random_capacity_never_blocks_defeat() {
        let (mut world, _) = phase_two_world(3, 1, 1);
        world
            .configure_loot(
                vec![LootTableDefinition {
                    id: "game:guardian-table".into(),
                    digest_sha256: "b".repeat(64),
                    guaranteed: vec![LootGuaranteedDefinition {
                        id: "game:potion-drop".into(),
                        item_prototype_id: "game:potion".into(),
                        quantity: 1,
                        affix_pool_id: None,
                    }],
                    pools: Vec::new(),
                }],
                vec![LootSourceDefinition {
                    kind: LootSourceKind::NpcSpawn,
                    id: "game:guardian-source".into(),
                    npc_spawn_id: "game:guardian".into(),
                    loot_table_id: "game:guardian-table".into(),
                    rights: LootRightsPolicy::CreditedCharacter,
                }],
            )
            .unwrap();
        world.next_loot_drop_id = LootDropId(2);
        for _ in 0..MAX_GROUND_LOOT_RESERVATIONS - 1 {
            let item_id = world.create_staged_item_instance("game:sword").unwrap();
            world
                .set_item_location(&item_id, SharedItemLocation::Room("game:arena".into()))
                .unwrap();
            world.ground_loot.insert(
                item_id.clone(),
                GroundLootReservation {
                    drop_id: LootDropId(1),
                    root_item_id: item_id,
                    room_id: "game:arena".into(),
                    cohort: vec![CharacterId(900)],
                    beneficiary: Some(CharacterId(900)),
                    passed: BTreeSet::new(),
                    created_tick: 0,
                    claim_until_tick: LOOT_EXCLUSIVE_TICKS,
                    despawn_at_tick: LOOT_DESPAWN_TICKS,
                },
            );
        }
        assert!(
            serde_json::to_vec(&world.runtime_snapshot()).unwrap().len()
                <= MAX_WORLD_RUNTIME_SNAPSHOT_BYTES
        );
        let mut events = Vec::new();
        world
            .resolve_npc_defeat(
                0,
                "game:guardian",
                CharacterId(900),
                "Adventurer",
                "game:arena",
                &mut events,
            )
            .unwrap();
        let fang = ItemInstanceId("game:fang".into());
        assert_eq!(
            world.item_location(&fang),
            Some(&SharedItemLocation::Room("game:arena".into()))
        );
        assert!(world.ground_loot.contains_key(&fang));
        assert_eq!(world.ground_loot.len(), MAX_GROUND_LOOT_RESERVATIONS);
        assert_eq!(world.next_loot_drop_id, LootDropId(3));
        assert!(events.iter().any(|event| matches!(
            event,
            DomainEvent::Gameplay {
                event: GameplayEvent::NpcDefeated { .. }
            }
        )));
        assert!(events.iter().any(|event| matches!(
            event,
            DomainEvent::Gameplay {
                event: GameplayEvent::LootGenerationSuppressed { reason, .. }
            } if reason == "capacity"
        )));
        assert_eq!(world.character_gameplay[&CharacterId(900)].experience, 120);
    }

    #[test]
    fn random_loot_is_suppressed_atomically_at_the_exact_snapshot_byte_budget() {
        let (mut world, _) = phase_two_world(11, 1, 10);
        world
            .spawn_npc(NpcDefinition {
                stable_id: "game:byte-target".into(),
                name: "byte target".into(),
                description: "A target for durable admission testing.".into(),
                room_id: "game:start".into(),
                level: 1,
                max_health: 1,
                strength: 1,
                defence: 0,
                experience_reward: 7,
                loot_item_ids: Vec::new(),
                respawn_ticks: 2,
            })
            .unwrap();
        world
            .configure_loot(
                vec![LootTableDefinition {
                    id: "game:byte-table".into(),
                    digest_sha256: "c".repeat(64),
                    guaranteed: Vec::new(),
                    pools: vec![LootPoolDefinition {
                        id: "game:byte-pool".into(),
                        rolls: 1,
                        entries: vec![LootEntryDefinition {
                            kind: LootEntryKind::Item,
                            id: "game:byte-entry".into(),
                            weight: 1,
                            item_prototype_id: Some("game:potion".into()),
                            min_quantity: Some(32),
                            max_quantity: Some(32),
                            affix_pool_id: None,
                        }],
                    }],
                }],
                vec![LootSourceDefinition {
                    kind: LootSourceKind::NpcSpawn,
                    id: "game:byte-source".into(),
                    npc_spawn_id: "game:byte-target".into(),
                    loot_table_id: "game:byte-table".into(),
                    rights: LootRightsPolicy::CreditedCharacter,
                }],
            )
            .unwrap();
        let mut encoded_len = serde_json::to_vec(&world.runtime_snapshot()).unwrap().len();
        while encoded_len < MAX_WORLD_RUNTIME_SNAPSHOT_BYTES - 40_000 {
            for _ in 0..100 {
                world.create_staged_item_instance("game:sword").unwrap();
            }
            encoded_len = serde_json::to_vec(&world.runtime_snapshot()).unwrap().len();
        }
        while encoded_len < MAX_WORLD_RUNTIME_SNAPSHOT_BYTES - 8_000 {
            world.create_staged_item_instance("game:sword").unwrap();
            encoded_len = serde_json::to_vec(&world.runtime_snapshot()).unwrap().len();
        }
        assert!(encoded_len <= MAX_WORLD_RUNTIME_SNAPSHOT_BYTES);
        let before_rng = world.rng.state;
        let mut events = Vec::new();
        world
            .resolve_npc_defeat(
                0,
                "game:byte-target",
                CharacterId(900),
                "Adventurer",
                "game:start",
                &mut events,
            )
            .unwrap();

        assert_eq!(world.next_loot_drop_id, LootDropId(2));
        assert_ne!(world.rng.state, before_rng);
        assert!(!world.item_provenance.values().any(|provenance| matches!(
            provenance,
            ItemProvenance::NpcLoot {
                npc_spawn_id, ..
            } if npc_spawn_id == "game:byte-target"
        )));
        assert!(events.iter().any(|event| matches!(
            event,
            DomainEvent::Gameplay {
                event: GameplayEvent::LootGenerationSuppressed { reason, .. }
            } if reason == "snapshot_bytes"
        )));
        assert!(
            serde_json::to_vec(&world.runtime_snapshot()).unwrap().len()
                <= MAX_WORLD_RUNTIME_SNAPSHOT_BYTES
        );
        assert_eq!(world.character_gameplay[&CharacterId(900)].experience, 7);
    }

    #[test]
    fn near_limit_v5_snapshot_upgrades_losslessly_below_the_v6_byte_budget() {
        let (mut world, _) = phase_two_world(13, 1, 10);
        let mut legacy = runtime_snapshot_json_for_version(&world.runtime_snapshot(), 5);
        let mut encoded_len = serde_json::to_vec(&legacy).unwrap().len();
        while encoded_len < MAX_LEGACY_WORLD_RUNTIME_SNAPSHOT_BYTES - 40_000 {
            for _ in 0..100 {
                world.create_staged_item_instance("game:sword").unwrap();
            }
            legacy = runtime_snapshot_json_for_version(&world.runtime_snapshot(), 5);
            encoded_len = serde_json::to_vec(&legacy).unwrap().len();
        }
        while encoded_len < MAX_LEGACY_WORLD_RUNTIME_SNAPSHOT_BYTES - 8_000 {
            world.create_staged_item_instance("game:sword").unwrap();
            legacy = runtime_snapshot_json_for_version(&world.runtime_snapshot(), 5);
            encoded_len = serde_json::to_vec(&legacy).unwrap().len();
        }
        assert!(encoded_len <= MAX_LEGACY_WORLD_RUNTIME_SNAPSHOT_BYTES);
        let legacy_json = serde_json::to_string(&legacy).unwrap();
        let upgraded = decode_world_runtime_snapshot(5, &legacy_json).unwrap();
        assert_eq!(upgraded.snapshot_version, WORLD_RUNTIME_SNAPSHOT_VERSION);
        assert_eq!(
            upgraded.item_provenance.len(),
            upgraded.item_instances.len()
        );
        assert!(upgraded.item_provenance.values().all(|provenance| matches!(
            provenance,
            ItemProvenance::LegacySnapshot { source_version: 5 }
        )));
        assert!(serde_json::to_vec(&upgraded).unwrap().len() <= MAX_WORLD_RUNTIME_SNAPSHOT_BYTES);
    }

    #[test]
    fn loot_views_and_pass_keep_reserved_ground_items_enforced_server_side() {
        let (mut world, actor) = connected_world();
        let other = SessionId(8);
        world
            .connect(
                other,
                CharacterSnapshot {
                    id: CharacterId(102),
                    account_id: AccountId(11),
                    name: "Other".into(),
                    room_id: world.start_room_id().to_owned(),
                    item_locations: BTreeMap::new(),
                },
            )
            .unwrap();
        let item_id = ItemInstanceId("demo:blue-lantern".into());
        world.next_loot_drop_id = LootDropId(2);
        world.ground_loot.insert(
            item_id.clone(),
            GroundLootReservation {
                drop_id: LootDropId(1),
                root_item_id: item_id,
                room_id: world.start_room_id().to_owned(),
                cohort: vec![CharacterId(101), CharacterId(102)],
                beneficiary: Some(CharacterId(101)),
                passed: BTreeSet::new(),
                created_tick: 0,
                claim_until_tick: LOOT_EXCLUSIVE_TICKS,
                despawn_at_tick: LOOT_DESPAWN_TICKS,
            },
        );
        let own = world.scene_for(actor).unwrap().items.remove(0);
        assert_eq!(
            own.loot.as_ref().map(|loot| loot.access),
            Some(LootAccessView::Yours)
        );
        assert!(own.actions.iter().any(|action| action.label == "Pass"));
        let reserved = world.scene_for(other).unwrap().items.remove(0);
        assert_eq!(
            reserved.loot.as_ref().map(|loot| loot.access),
            Some(LootAccessView::Reserved)
        );
        assert!(!reserved.actions.iter().any(|action| action.label == "Take"));
        assert!(matches!(
            world.apply(other, PlayerAction::Take(ItemSelection::one("lantern"))),
            Err(WorldError::LootUnavailable)
        ));

        world
            .apply(actor, PlayerAction::Loot(LootAction::Pass(own.id)))
            .unwrap();
        assert_eq!(
            world.ground_loot[&ItemInstanceId("demo:blue-lantern".into())].beneficiary,
            Some(CharacterId(102))
        );
        world
            .apply(other, PlayerAction::Take(ItemSelection::one("lantern")))
            .unwrap();
        assert!(world.ground_loot.is_empty());
    }

    #[test]
    fn reserved_ground_container_contents_are_private_to_the_beneficiary() {
        let (mut world, beneficiary) = connect_definition(policy_definition());
        world
            .item_prototypes
            .get_mut("policy:clasp")
            .unwrap()
            .container = Some(ItemContainerDefinition {
            max_slots: 2,
            max_weight_grams: 100,
        });
        let other = SessionId(72);
        world
            .connect(
                other,
                CharacterSnapshot {
                    id: CharacterId(702),
                    account_id: AccountId(71),
                    name: "Other".into(),
                    room_id: "test:start".into(),
                    item_locations: BTreeMap::new(),
                },
            )
            .unwrap();
        let bag_id = ItemInstanceId("policy:bag-instance".into());
        world.next_loot_drop_id = LootDropId(2);
        world.ground_loot.insert(
            bag_id.clone(),
            GroundLootReservation {
                drop_id: LootDropId(1),
                root_item_id: bag_id,
                room_id: "test:start".into(),
                cohort: vec![CharacterId(701), CharacterId(702)],
                beneficiary: Some(CharacterId(701)),
                passed: BTreeSet::new(),
                created_tick: 0,
                claim_until_tick: LOOT_EXCLUSIVE_TICKS,
                despawn_at_tick: LOOT_DESPAWN_TICKS,
            },
        );

        let own = world
            .apply(
                beneficiary,
                PlayerAction::Contents {
                    container: "bag".into(),
                    page: 1,
                },
            )
            .unwrap();
        assert_eq!(
            own.container_contents.as_ref().unwrap().items[0].label,
            "wolf clasp"
        );
        assert!(
            world
                .apply(beneficiary, PlayerAction::Examine("clasp".into()))
                .unwrap()
                .output
                .unwrap()
                .contains("character-bound clasp")
        );
        assert!(
            world
                .apply(
                    beneficiary,
                    PlayerAction::Contents {
                        container: "clasp".into(),
                        page: 1,
                    },
                )
                .is_ok()
        );
        assert!(matches!(
            world.apply(
                other,
                PlayerAction::Contents {
                    container: "bag".into(),
                    page: 1,
                },
            ),
            Err(WorldError::LootUnavailable)
        ));
        assert!(matches!(
            world.apply(
                other,
                PlayerAction::Contents {
                    container: "clasp".into(),
                    page: 1,
                },
            ),
            Err(WorldError::ItemNotFound(_))
        ));
        assert!(matches!(
            world.apply(
                other,
                PlayerAction::Contents {
                    container: "not-there".into(),
                    page: 1,
                },
            ),
            Err(WorldError::ItemNotFound(_))
        ));
        assert!(matches!(
            world.apply(
                other,
                PlayerAction::Get {
                    selection: ItemSelection::one("clasp"),
                    container: "bag".into(),
                },
            ),
            Err(WorldError::LootUnavailable)
        ));
        assert!(matches!(
            world.apply(
                other,
                PlayerAction::Get {
                    selection: ItemSelection::one("not-there"),
                    container: "bag".into(),
                },
            ),
            Err(WorldError::LootUnavailable)
        ));
        assert!(matches!(
            world.apply(
                other,
                PlayerAction::Put {
                    selection: ItemSelection::one("not-there"),
                    container: "bag".into(),
                },
            ),
            Err(WorldError::LootUnavailable)
        ));
        assert!(matches!(
            world.apply(other, PlayerAction::Examine("clasp".into())),
            Err(WorldError::ItemNotFound(_))
        ));
    }

    #[test]
    fn despawning_runtime_root_preserves_authored_descendants_as_tombstones() {
        let (mut world, _) = connect_definition(policy_definition());
        let root = world.create_staged_item_instance("policy:bag").unwrap();
        world
            .set_item_location(&root, SharedItemLocation::Room("test:start".into()))
            .unwrap();
        let authored = ItemInstanceId("policy:clasp-instance".into());
        world.item_instances.get_mut(&authored).unwrap().location =
            SharedItemLocation::Contained(root.clone());
        world.next_loot_drop_id = LootDropId(2);
        world.ground_loot.insert(
            root.clone(),
            GroundLootReservation {
                drop_id: LootDropId(1),
                root_item_id: root.clone(),
                room_id: "test:start".into(),
                cohort: vec![CharacterId(701)],
                beneficiary: Some(CharacterId(701)),
                passed: BTreeSet::new(),
                created_tick: 0,
                claim_until_tick: LOOT_EXCLUSIVE_TICKS,
                despawn_at_tick: LOOT_DESPAWN_TICKS,
            },
        );
        world.logical_tick = LOOT_DESPAWN_TICKS;

        world.expire_ground_loot().unwrap();
        assert!(!world.item_instances.contains_key(&root));
        assert_eq!(
            world.item_location(&authored),
            Some(&SharedItemLocation::Staged)
        );
        assert_eq!(
            world.item_provenance.get(&authored),
            Some(&ItemProvenance::AuthoredInitial)
        );
    }

    #[test]
    fn legacy_fixed_loot_despawn_stays_staged_after_the_next_restart() {
        let (source, _) = phase_two_world(17, 1, 10);
        let legacy = runtime_snapshot_json_for_version(&source.runtime_snapshot(), 5);
        let upgraded = decode_world_runtime_snapshot(5, &legacy.to_string()).unwrap();
        let fang = ItemInstanceId("game:fang".into());
        assert_eq!(
            upgraded.item_provenance.get(&fang),
            Some(&ItemProvenance::LegacySnapshot { source_version: 5 })
        );

        let (mut restored, _) = phase_two_world(17, 1, 10);
        restored.restore_runtime_snapshot(upgraded).unwrap();
        restored
            .set_item_location(&fang, SharedItemLocation::Room("game:start".into()))
            .unwrap();
        restored.next_loot_drop_id = LootDropId(2);
        restored.ground_loot.insert(
            fang.clone(),
            GroundLootReservation {
                drop_id: LootDropId(1),
                root_item_id: fang.clone(),
                room_id: "game:start".into(),
                cohort: vec![CharacterId(900)],
                beneficiary: Some(CharacterId(900)),
                passed: BTreeSet::new(),
                created_tick: 0,
                claim_until_tick: LOOT_EXCLUSIVE_TICKS,
                despawn_at_tick: LOOT_DESPAWN_TICKS,
            },
        );
        restored.logical_tick = LOOT_DESPAWN_TICKS;
        restored.expire_ground_loot().unwrap();
        assert_eq!(
            restored.item_location(&fang),
            Some(&SharedItemLocation::Staged)
        );
        assert!(restored.ground_loot.is_empty());

        let post_despawn = restored.runtime_snapshot();
        let (mut restarted, _) = phase_two_world(17, 1, 10);
        restarted.restore_runtime_snapshot(post_despawn).unwrap();
        assert_eq!(
            restarted.item_location(&fang),
            Some(&SharedItemLocation::Staged),
            "restart must not resurrect fixed loot at its authored NPC"
        );
        assert_eq!(
            restarted.item_provenance.get(&fang),
            Some(&ItemProvenance::LegacySnapshot { source_version: 5 })
        );
    }

    #[test]
    fn restart_cancellation_clears_lethal_npc_status_before_the_next_tick() {
        let (mut source, session) = phase_two_world(5, 1, 10);
        source
            .apply(session, PlayerAction::Move(Direction::North))
            .unwrap();
        source
            .apply(session, PlayerAction::Attack("guardian".into()))
            .unwrap();
        source.npcs.get_mut("game:guardian").unwrap().statuses = vec![ActiveStatusEffect {
            source_ability_id: "game:old-dot".into(),
            source_character_id: Some(CharacterId(900)),
            source_npc_id: None,
            kind: ActiveStatusKind::DamageOverTime,
            magnitude: 100,
            remaining_ticks: 1,
            attribute: None,
            damage_type: Some(DamageType::Physical),
        }];
        let snapshot = source.runtime_snapshot();
        let (mut restored, _) = phase_two_world(5, 1, 10);
        restored.restore_runtime_snapshot(snapshot).unwrap();

        let cancelled = restored.cancel_all_loot_encounters_for_restart().unwrap();
        assert_eq!(cancelled.len(), 1);
        assert!(restored.loot_encounters.is_empty());
        assert!(restored.npcs["game:guardian"].statuses.is_empty());
        let events = restored.advance_tick().unwrap();
        assert!(restored.npcs["game:guardian"].alive());
        assert!(!events.iter().any(|event| matches!(
            event,
            DomainEvent::Gameplay {
                event: GameplayEvent::NpcDefeated { .. }
            }
        )));
    }

    #[test]
    fn sealed_reward_open_and_odds_are_atomic_opaque_and_exact() {
        let (mut world, session, source) = sealed_reward_world(41);
        let before = world.runtime_snapshot();
        let inventory = world.player_state_view(session).unwrap().inventory;
        let seal = inventory
            .iter()
            .find(|item| item.label == "sealed cache")
            .unwrap();
        assert!(seal.actions.iter().any(|action| {
            action.label == "Abrir" && action.command == format!("open {}", seal.id)
        }));
        assert!(seal.actions.iter().any(|action| {
            action.label == "Probabilidades"
                && action.command == format!("reward odds {} 1", seal.id)
        }));

        let odds_outcome = world
            .apply(
                session,
                PlayerAction::RewardOdds {
                    target: seal.id.clone(),
                    page: 1,
                },
            )
            .unwrap();
        let odds_text = odds_outcome.output.unwrap();
        assert!(odds_text.contains("garantizado; afijo aleatorio"));
        assert!(odds_text.contains(
            "Tirada 1: 2 tirada(s); sin recompensa; cantidad 0..=0; probabilidad 2/2 por tirada"
        ));
        assert!(
            odds_text
                .contains("posición prefix; ataque +2; curación +0; probabilidad 3/3 condicional")
        );
        let odds = odds_outcome.reward_odds.unwrap();
        assert_eq!(odds.guaranteed.len(), 1);
        assert_eq!(odds.weighted.len(), 1);
        assert_eq!(odds.affixes.len(), 1);
        assert_eq!(odds.affixes[0].conditional_probability_numerator, 3);
        assert_eq!(odds.affixes[0].conditional_probability_denominator, 3);
        let encoded_odds = serde_json::to_string(&odds).unwrap();
        assert!(!encoded_odds.contains("reward:weapon-affix"));
        assert!(!encoded_odds.contains("reward:keen"));

        let opened = world
            .apply(session, PlayerAction::OpenReward(seal.id.clone()))
            .unwrap();
        assert!(matches!(
            opened.domain_events.as_slice(),
            [DomainEvent::Gameplay {
                event: GameplayEvent::RewardOpened {
                    open_id: RewardOpenId(1),
                    ..
                }
            }]
        ));
        assert_eq!(
            world.item_instances[&source].location,
            SharedItemLocation::Consumed
        );
        let reward_id = world
            .item_provenance
            .iter()
            .find_map(|(item_id, provenance)| {
                matches!(provenance, ItemProvenance::SealedReward { .. }).then_some(item_id.clone())
            })
            .unwrap();
        assert_eq!(
            world.item_effective_name(&reward_id).unwrap(),
            "afilada iron sword"
        );
        assert_eq!(world.item_effective_attack_bonus(&reward_id).unwrap(), 3);
        assert_eq!(world.item_instances[&reward_id].item_affixes.len(), 1);
        let after = world.runtime_snapshot();
        assert!(matches!(
            world.apply(session, PlayerAction::OpenReward(seal.id.clone())),
            Err(WorldError::ItemNotFound(_))
        ));
        assert_eq!(world.runtime_snapshot(), after);

        world.restore_checkpoint({
            let mut replay = sealed_reward_world(41).0;
            let checkpoint = replay.checkpoint();
            let replay_session = SessionId(71);
            replay
                .apply(
                    replay_session,
                    PlayerAction::OpenReward("sealed cache".into()),
                )
                .unwrap();
            checkpoint
        });
        assert_eq!(world.runtime_snapshot(), before);
    }

    #[test]
    fn sealed_reward_capacity_failure_cannot_reroll_or_consume() {
        let (mut world, session, source) = sealed_reward_world(73);
        world
            .character_gameplay
            .get_mut(&CharacterId(701))
            .unwrap()
            .inventory_capacity
            .max_slots = 1;
        let before = world.runtime_snapshot();
        assert!(matches!(
            world.apply(session, PlayerAction::OpenReward("sealed cache".into())),
            Err(WorldError::InventoryCapacityExceeded)
        ));
        assert_eq!(world.runtime_snapshot(), before);
        assert_eq!(
            world.item_instances[&source].location,
            SharedItemLocation::Carried(CharacterId(701))
        );
        assert_eq!(world.next_reward_open_id, RewardOpenId(1));
    }

    #[test]
    fn sealed_reward_survives_source_pruning_and_affix_authoring_changes() {
        let (mut world, session, source) = sealed_reward_world(91);
        world
            .apply(session, PlayerAction::OpenReward("sealed cache".into()))
            .unwrap();
        let reward_id = world
            .item_provenance
            .iter()
            .find_map(|(item_id, provenance)| {
                matches!(provenance, ItemProvenance::SealedReward { .. }).then_some(item_id.clone())
            })
            .unwrap();
        let source_digest = match &world.item_provenance[&reward_id] {
            ItemProvenance::SealedReward {
                source_npc_loot_digest_sha256,
                ..
            } => source_npc_loot_digest_sha256.clone(),
            _ => unreachable!(),
        };
        world.prune_consumed_runtime_items().unwrap();
        assert!(!world.item_instances.contains_key(&source));
        let snapshot = world.runtime_snapshot();
        assert!(valid_digest_sha256(&source_digest));

        let mut restored = World::from_definition(sealed_reward_definition()).unwrap();
        restored.restore_runtime_snapshot(snapshot).unwrap();
        assert_eq!(
            restored.item_effective_name(&reward_id).unwrap(),
            "afilada iron sword"
        );
        assert!(matches!(
            &restored.item_provenance[&reward_id],
            ItemProvenance::SealedReward {
                source_npc_loot_digest_sha256,
                ..
            } if source_npc_loot_digest_sha256 == &source_digest
        ));
    }

    #[test]
    fn snapshot_v6_rejects_affix_and_sealed_provenance_smuggling() {
        let (world, _, _) = sealed_reward_world(11);
        let mut legacy = runtime_snapshot_json_for_version(&world.runtime_snapshot(), 6);
        legacy["itemProvenance"]
            .as_object_mut()
            .unwrap()
            .values_mut()
            .next()
            .unwrap()["sealedLootTableId"] = serde_json::json!("reward:cache-table");
        assert!(decode_world_runtime_snapshot(6, &legacy.to_string()).is_err());
        legacy["itemProvenance"]
            .as_object_mut()
            .unwrap()
            .values_mut()
            .next()
            .unwrap()
            .as_object_mut()
            .unwrap()
            .remove("sealedLootTableId");
        let instance = legacy["itemInstances"]
            .as_object_mut()
            .unwrap()
            .values_mut()
            .next()
            .unwrap();
        instance["itemAffixes"] = serde_json::json!([]);
        assert!(decode_world_runtime_snapshot(6, &legacy.to_string()).is_err());

        let mut forged = world.runtime_snapshot();
        let source = forged.item_instances.keys().next().unwrap().clone();
        forged.item_instances.get_mut(&source).unwrap().item_affixes = vec![AppliedItemAffix {
            affix_id: "reward:forged".into(),
            definition_digest_sha256: "0".repeat(64),
            pool_id: "reward:forged-pool".into(),
            pool_digest_sha256: "1".repeat(64),
            record_digest_sha256: "2".repeat(64),
            label: "forged".into(),
            position: AffixPosition::Prefix,
            attack_bonus_delta: 1,
            heal_amount_delta: 0,
        }];
        assert!(matches!(
            validate_world_runtime_snapshot_semantics(&forged),
            Err(WorldError::InvalidRuntimeSnapshot)
        ));
    }

    #[test]
    fn sealed_restore_rejects_forged_provenance_location_binding_and_reservation() {
        fn assert_restore_rejected(snapshot: WorldRuntimeSnapshot) {
            validate_world_runtime_snapshot_semantics(&snapshot).unwrap();
            let mut restored = World::from_definition(sealed_reward_definition()).unwrap();
            assert!(matches!(
                restored.restore_runtime_snapshot(snapshot),
                Err(WorldError::InvalidRuntimeSnapshot)
            ));
        }

        let (world, _, source) = sealed_reward_world(13);
        let base = world.runtime_snapshot();
        let mut correct = World::from_definition(sealed_reward_definition()).unwrap();
        correct.restore_runtime_snapshot(base.clone()).unwrap();
        for marker in [None, Some("reward:other-table".to_owned())] {
            let mut forged = base.clone();
            let ItemProvenance::NpcLoot {
                sealed_loot_table_id,
                ..
            } = forged.item_provenance.get_mut(&source).unwrap()
            else {
                unreachable!();
            };
            *sealed_loot_table_id = marker;
            assert_restore_rejected(forged);
        }
        for forged_provenance in [
            ItemProvenance::AuthoredInitial,
            ItemProvenance::LegacySnapshot { source_version: 5 },
            ItemProvenance::RuntimeCreated { created_tick: 0 },
            ItemProvenance::SealedReward {
                open_id: RewardOpenId(1),
                source_item_id: ItemInstanceId("runtime:999".into()),
                source_npc_loot_digest_sha256: "a".repeat(64),
                sealed_prototype_id: "reward:former-seal".into(),
                loot_table_id: "reward:forged-table".into(),
                table_digest_sha256: "b".repeat(64),
                entry_id: "reward:forged-entry".into(),
                pool_id: None,
                roll_index: 0,
                quantity_index: 0,
                generated_tick: 0,
            },
        ] {
            let mut forged = base.clone();
            if matches!(forged_provenance, ItemProvenance::SealedReward { .. }) {
                forged.next_reward_open_id = RewardOpenId(2);
            }
            forged
                .item_provenance
                .insert(source.clone(), forged_provenance);
            assert_restore_rejected(forged);
        }

        let invalid_states = [
            (RuntimeItemLocation::Staged, ItemBinding::Unbound),
            (
                RuntimeItemLocation::Room {
                    room_id: "test:start".into(),
                },
                ItemBinding::Unbound,
            ),
            (
                RuntimeItemLocation::Carried {
                    character_id: CharacterId(701),
                },
                ItemBinding::Unbound,
            ),
            (
                RuntimeItemLocation::Carried {
                    character_id: CharacterId(701),
                },
                ItemBinding::Character {
                    character_id: CharacterId(702),
                },
            ),
            (
                RuntimeItemLocation::Equipped {
                    character_id: CharacterId(701),
                    slot: EquipmentSlot::Weapon,
                },
                ItemBinding::Character {
                    character_id: CharacterId(701),
                },
            ),
            (
                RuntimeItemLocation::HeldByNpc {
                    npc_id: "reward:holder".into(),
                },
                ItemBinding::Unbound,
            ),
            (RuntimeItemLocation::Consumed, ItemBinding::Unbound),
        ];
        for (location, binding) in invalid_states {
            let mut forged = base.clone();
            let instance = forged.item_instances.get_mut(&source).unwrap();
            instance.location = location;
            instance.binding = binding;
            assert_restore_rejected(forged);
        }

        let mut wrong_reservation = base;
        let instance = wrong_reservation.item_instances.get_mut(&source).unwrap();
        instance.location = RuntimeItemLocation::Room {
            room_id: "test:start".into(),
        };
        instance.binding = ItemBinding::Unbound;
        wrong_reservation.next_loot_drop_id = LootDropId(3);
        wrong_reservation.ground_loot.insert(
            source.clone(),
            GroundLootReservation {
                drop_id: LootDropId(2),
                root_item_id: source,
                room_id: "test:start".into(),
                cohort: vec![CharacterId(701)],
                beneficiary: Some(CharacterId(701)),
                passed: BTreeSet::new(),
                created_tick: 0,
                claim_until_tick: LOOT_EXCLUSIVE_TICKS,
                despawn_at_tick: LOOT_DESPAWN_TICKS,
            },
        );
        assert_restore_rejected(wrong_reservation);
    }

    #[test]
    fn sealed_snapshot_groups_are_bounded_consistent_and_affix_records_self_verify() {
        let (mut world, session, _) = sealed_reward_world(29);
        world
            .apply(session, PlayerAction::OpenReward("sealed cache".into()))
            .unwrap();
        let base = world.runtime_snapshot();
        let reward_id = base
            .item_provenance
            .iter()
            .find_map(|(item_id, provenance)| {
                matches!(provenance, ItemProvenance::SealedReward { .. }).then_some(item_id.clone())
            })
            .unwrap();

        let mut sixteen = base.clone();
        for quantity_index in 1..MAX_SEALED_REWARD_UNITS {
            let sequence = sixteen.next_item_instance_sequence;
            sixteen.next_item_instance_sequence += 1;
            let item_id = ItemInstanceId(format!("runtime:{sequence}"));
            sixteen
                .item_instances
                .insert(item_id.clone(), sixteen.item_instances[&reward_id].clone());
            let mut provenance = sixteen.item_provenance[&reward_id].clone();
            match &mut provenance {
                ItemProvenance::SealedReward {
                    quantity_index: index,
                    ..
                } => *index = u16::try_from(quantity_index).unwrap(),
                _ => unreachable!(),
            }
            sixteen.item_provenance.insert(item_id, provenance);
        }
        validate_world_runtime_snapshot_semantics(&sixteen).unwrap();

        let mut seventeen = sixteen.clone();
        let sequence = seventeen.next_item_instance_sequence;
        seventeen.next_item_instance_sequence += 1;
        let extra_id = ItemInstanceId(format!("runtime:{sequence}"));
        seventeen.item_instances.insert(
            extra_id.clone(),
            seventeen.item_instances[&reward_id].clone(),
        );
        let mut extra_provenance = seventeen.item_provenance[&reward_id].clone();
        match &mut extra_provenance {
            ItemProvenance::SealedReward {
                entry_id,
                quantity_index,
                ..
            } => {
                *entry_id = "reward:seventeenth".into();
                *quantity_index = 0;
            }
            _ => unreachable!(),
        }
        seventeen.item_provenance.insert(extra_id, extra_provenance);
        assert!(validate_world_runtime_snapshot_semantics(&seventeen).is_err());

        let mut mixed = base.clone();
        let sequence = mixed.next_item_instance_sequence;
        mixed.next_item_instance_sequence += 1;
        let mixed_id = ItemInstanceId(format!("runtime:{sequence}"));
        mixed
            .item_instances
            .insert(mixed_id.clone(), mixed.item_instances[&reward_id].clone());
        let mut mixed_provenance = mixed.item_provenance[&reward_id].clone();
        match &mut mixed_provenance {
            ItemProvenance::SealedReward {
                loot_table_id,
                entry_id,
                ..
            } => {
                *loot_table_id = "reward:other-table".into();
                *entry_id = "reward:other-entry".into();
            }
            _ => unreachable!(),
        }
        mixed.item_provenance.insert(mixed_id, mixed_provenance);
        assert!(validate_world_runtime_snapshot_semantics(&mixed).is_err());

        let mut corrupted_pool = base.clone();
        corrupted_pool
            .item_instances
            .get_mut(&reward_id)
            .unwrap()
            .item_affixes[0]
            .pool_digest_sha256 = "0".repeat(64);
        assert!(validate_world_runtime_snapshot_semantics(&corrupted_pool).is_err());
        let mut corrupted_record = base;
        corrupted_record
            .item_instances
            .get_mut(&reward_id)
            .unwrap()
            .item_affixes[0]
            .record_digest_sha256 = "0".repeat(64);
        assert!(validate_world_runtime_snapshot_semantics(&corrupted_record).is_err());
    }

    #[test]
    fn affix_modifier_caps_match_authoring_at_zero_one_thousand_and_overflow() {
        let mut world = World::from_definition(sealed_reward_definition()).unwrap();
        for invalid in [
            AffixDefinition {
                id: "reward:negative".into(),
                label: "negative".into(),
                position: AffixPosition::Prefix,
                attack_bonus_delta: -1,
                heal_amount_delta: 0,
            },
            AffixDefinition {
                id: "reward:empty".into(),
                label: "empty".into(),
                position: AffixPosition::Prefix,
                attack_bonus_delta: 0,
                heal_amount_delta: 0,
            },
            AffixDefinition {
                id: "reward:too-large".into(),
                label: "too large".into(),
                position: AffixPosition::Prefix,
                attack_bonus_delta: MAX_AFFIX_MODIFIER + 1,
                heal_amount_delta: 0,
            },
        ] {
            assert!(matches!(
                world.configure_affixes(vec![invalid], Vec::new()),
                Err(WorldError::InvalidGameplayDefinition)
            ));
        }
        world
            .configure_affixes(
                vec![AffixDefinition {
                    id: "reward:cap".into(),
                    label: "maximum".into(),
                    position: AffixPosition::Prefix,
                    attack_bonus_delta: MAX_AFFIX_MODIFIER,
                    heal_amount_delta: 0,
                }],
                Vec::new(),
            )
            .unwrap();
    }

    #[test]
    fn suppressed_and_admitted_npc_affixes_consume_identical_rng() {
        let (mut admitted, _) = phase_two_world(123, 1, 10);
        let (mut suppressed, _) = phase_two_world(123, 1, 10);
        for world in [&mut admitted, &mut suppressed] {
            world
                .configure_affixes(
                    vec![AffixDefinition {
                        id: "game:keen".into(),
                        label: "keen".into(),
                        position: AffixPosition::Prefix,
                        attack_bonus_delta: 1,
                        heal_amount_delta: 0,
                    }],
                    vec![AffixPoolDefinition {
                        id: "game:weapon-affix".into(),
                        entries: vec![AffixPoolEntry {
                            kind: AffixPoolEntryKind::Affix,
                            id: "game:keen-outcome".into(),
                            weight: 1,
                            affix_id: Some("game:keen".into()),
                        }],
                    }],
                )
                .unwrap();
            world
                .configure_loot(
                    vec![LootTableDefinition {
                        id: "game:guardian-table".into(),
                        digest_sha256: "f".repeat(64),
                        guaranteed: vec![LootGuaranteedDefinition {
                            id: "game:sword-outcome".into(),
                            item_prototype_id: "game:sword".into(),
                            quantity: 1,
                            affix_pool_id: Some("game:weapon-affix".into()),
                        }],
                        pools: Vec::new(),
                    }],
                    vec![LootSourceDefinition {
                        kind: LootSourceKind::NpcSpawn,
                        id: "game:guardian-source".into(),
                        npc_spawn_id: "game:guardian".into(),
                        loot_table_id: "game:guardian-table".into(),
                        rights: LootRightsPolicy::CreditedCharacter,
                    }],
                )
                .unwrap();
        }
        for index in 0..MAX_GROUND_LOOT_RESERVATIONS {
            let item_id = ItemInstanceId(format!("test:saturated-{index}"));
            suppressed.ground_loot.insert(
                item_id.clone(),
                GroundLootReservation {
                    drop_id: LootDropId(99),
                    root_item_id: item_id,
                    room_id: "game:arena".into(),
                    cohort: vec![CharacterId(900)],
                    beneficiary: Some(CharacterId(900)),
                    passed: BTreeSet::new(),
                    created_tick: 0,
                    claim_until_tick: LOOT_EXCLUSIVE_TICKS,
                    despawn_at_tick: LOOT_DESPAWN_TICKS,
                },
            );
        }
        let mut admitted_events = Vec::new();
        admitted
            .resolve_npc_defeat(
                0,
                "game:guardian",
                CharacterId(900),
                "Adventurer",
                "game:arena",
                &mut admitted_events,
            )
            .unwrap();
        let mut suppressed_events = Vec::new();
        suppressed
            .resolve_npc_defeat(
                0,
                "game:guardian",
                CharacterId(900),
                "Adventurer",
                "game:arena",
                &mut suppressed_events,
            )
            .unwrap();
        assert_eq!(admitted.rng.state, suppressed.rng.state);
        assert_eq!(admitted.next_loot_drop_id, suppressed.next_loot_drop_id);
        assert_eq!(admitted.rng.next_u64(), suppressed.rng.next_u64());
        let generated = admitted
            .item_instances
            .values()
            .find(|instance| {
                instance.prototype_id == "game:sword" && !instance.item_affixes.is_empty()
            })
            .unwrap();
        assert_eq!(generated.item_affixes[0].label, "keen");
        assert!(suppressed_events.iter().any(|event| matches!(
            event,
            DomainEvent::Gameplay {
                event: GameplayEvent::LootGenerationSuppressed { .. }
            }
        )));
    }

    #[test]
    fn affixed_items_project_effective_use_and_equip_actions() {
        let (mut reward_world, reward_session, _) = sealed_reward_world(17);
        reward_world
            .apply(
                reward_session,
                PlayerAction::OpenReward("sealed cache".into()),
            )
            .unwrap();
        let reward = reward_world
            .player_state_view(reward_session)
            .unwrap()
            .inventory
            .into_iter()
            .find(|item| item.affix.is_some())
            .unwrap();
        assert_eq!(reward.label, "afilada iron sword");
        assert!(reward.actions.iter().any(|action| action.label == "Equip"));
        let equipped = reward_world
            .apply(reward_session, PlayerAction::Equip(reward.id))
            .unwrap();
        assert!(equipped.output.unwrap().contains("afilada iron sword"));
        assert!(matches!(
            equipped.domain_events.last(),
            Some(DomainEvent::Gameplay {
                event: GameplayEvent::ItemEquipped { item_name, .. }
            }) if item_name == "afilada iron sword"
        ));

        let (mut consumable_world, consumable_session) = phase_two_world(19, 1, 10);
        consumable_world
            .apply(
                consumable_session,
                PlayerAction::Take(ItemSelection::one("potion")),
            )
            .unwrap();
        let potion_id = ItemInstanceId("game:potion".into());
        let definition = AffixDefinition {
            id: "game:restorative".into(),
            label: "restorative".into(),
            position: AffixPosition::Suffix,
            attack_bonus_delta: 0,
            heal_amount_delta: 2,
        };
        let pool = AffixPoolDefinition {
            id: "game:tonic-affix".into(),
            entries: vec![AffixPoolEntry {
                kind: AffixPoolEntryKind::Affix,
                id: "game:restorative-outcome".into(),
                weight: 1,
                affix_id: Some(definition.id.clone()),
            }],
        };
        consumable_world
            .item_prototypes
            .get_mut("game:potion")
            .unwrap()
            .heal_amount = 0;
        consumable_world
            .item_prototypes
            .get_mut("game:potion")
            .unwrap()
            .effect = None;
        let mut applied = AppliedItemAffix {
            affix_id: definition.id.clone(),
            definition_digest_sha256: canonical_sha256(&definition).unwrap(),
            pool_id: pool.id.clone(),
            pool_digest_sha256: canonical_sha256(&pool).unwrap(),
            record_digest_sha256: String::new(),
            label: definition.label,
            position: definition.position,
            attack_bonus_delta: 0,
            heal_amount_delta: 2,
        };
        applied.record_digest_sha256 = applied_affix_record_sha256(&applied).unwrap();
        consumable_world
            .item_instances
            .get_mut(&potion_id)
            .unwrap()
            .item_affixes = vec![applied];
        let potion = consumable_world
            .player_state_view(consumable_session)
            .unwrap()
            .inventory
            .into_iter()
            .find(|item| item.affix.is_some())
            .unwrap();
        assert_eq!(potion.label, "red potion restorative");
        assert!(potion.actions.iter().any(|action| action.label == "Use"));
        let used = consumable_world
            .apply(consumable_session, PlayerAction::Use(potion.id))
            .unwrap();
        assert!(used.output.unwrap().contains("red potion restorative"));
    }

    #[test]
    fn cold_replay_fails_closed_for_every_snapshot_v7_loot_ledger_event() {
        let (mut world, _) = connected_world();
        let item_id = ItemInstanceId("demo:blue-lantern".into());
        let events = vec![
            GameplayEvent::LootGenerated {
                drop_id: LootDropId(1),
                npc_id: "demo:npc".into(),
                npc_name: "npc".into(),
                item_ids: vec![item_id.clone()],
                room_id: "demo:lantern-square".into(),
            },
            GameplayEvent::LootRightsChanged {
                item_id: item_id.clone(),
                room_id: "demo:lantern-square".into(),
                summary: "changed".into(),
            },
            GameplayEvent::LootGenerationSuppressed {
                drop_id: LootDropId(1),
                npc_id: "demo:npc".into(),
                npc_name: "npc".into(),
                reason: "capacity".into(),
                room_id: "demo:lantern-square".into(),
            },
            GameplayEvent::LootDespawned {
                item_ids: vec![item_id],
                room_id: "demo:lantern-square".into(),
            },
            GameplayEvent::LootEncounterCancelled {
                npc_id: "demo:npc".into(),
                npc_name: "npc".into(),
                room_id: "demo:lantern-square".into(),
            },
            GameplayEvent::RewardOpened {
                open_id: RewardOpenId(1),
                character_id: CharacterId(101),
                character_name: "Tester".into(),
                source_item_id: ItemInstanceId("runtime:1".into()),
                source_item_name: "sealed cache".into(),
                item_ids: vec![ItemInstanceId("runtime:2".into())],
                item_names: vec!["forged reward".into()],
                room_id: "demo:lantern-square".into(),
            },
        ];
        for event in events {
            assert!(matches!(
                world.replay_persisted_event(&DomainEvent::Gameplay { event }),
                Err(WorldError::InvalidDomainTransition)
            ));
        }
    }
}
