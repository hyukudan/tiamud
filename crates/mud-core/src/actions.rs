//! Player action vocabulary: movement, items, loot, trade, party, quest,
//! economy, staff, social, mail and board actions with their bounds.

use super::*;

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

    pub(crate) fn parse(value: &str) -> Option<Self> {
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
    Telepathy {
        ability_id: String,
        target: String,
        message: String,
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
    Mail(MailAction),
    Board(BoardAction),
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

/// Maximum hard-dependency closure accepted by one additive area install.
pub const MAX_PREPARED_AREAS_PER_BATCH: usize = 32;

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum LootAction {
    /// Show the post-defeat reserved ground loot addressed to the actor.
    Status,
    /// Pass on a post-defeat ground reservation, advancing it to the next
    /// cohort member (phase 4A round-robin behaviour, unchanged).
    Pass(String),
    /// Show the in-combat group-loot decision state for the frozen encounter
    /// the actor belongs to: the active distribution mode, the declarable roots
    /// the NPC is carrying and the current declarations/assignments.
    Roll,
    /// Leader/initiator sets the group-loot distribution mode on the encounter
    /// they are engaged in. Switching modes clears the now-irrelevant advanced
    /// state so the frozen encounter always validates.
    Mode(loot_distribution::LootDistributionMode),
    /// A cohort member declares `need`/`greed`/`pass` on one carried root while
    /// the encounter is frozen and in `need_greed` mode.
    Declare {
        decision: loot_distribution::LootDecision,
        query: String,
    },
    /// The leader assigns one carried root to a cohort member while the
    /// encounter is frozen and in `master_loot` mode.
    Master { query: String, member: String },
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
    RetireArea {
        area: String,
        reason: String,
    },
    RehydrateArea {
        area: String,
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
    Yell(String),
    Shout(String),
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
    EmoteList,
    Emote {
        command: String,
        target: Option<String>,
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

pub const MAX_MAIL_TARGET_BYTES: usize = 64;
pub const MAX_MAIL_SUBJECT_BYTES: usize = 128;
pub const MAX_MAIL_BODY_BYTES: usize = 2_048;
pub const MAX_MAIL_REPORT_REASON_BYTES: usize = 2_048;
pub const MAX_MAIL_PAGE: u16 = 7;
pub const MAIL_PUBLIC_ID_BYTES: usize = 33;

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum MailAction {
    List {
        page: u16,
    },
    Read {
        mail_id: String,
    },
    Delete {
        mail_id: String,
    },
    Send {
        target: String,
        subject: String,
        body: String,
    },
    Report {
        mail_id: String,
        reason: String,
    },
}

pub const MAX_BOARD_SUBJECT_BYTES: usize = 128;
pub const MAX_BOARD_BODY_BYTES: usize = 4_096;
pub const MAX_BOARD_REPORT_REASON_BYTES: usize = 2_048;
pub const MAX_BOARD_PAGE: u16 = 100;
pub const BOARD_PUBLIC_ID_BYTES: usize = 33;

#[derive(Clone, Copy, Debug, Deserialize, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum BoardKey {
    General,
    Newbie,
}

impl BoardKey {
    pub const ALL: [Self; 2] = [Self::General, Self::Newbie];

    pub const fn as_str(self) -> &'static str {
        match self {
            Self::General => "general",
            Self::Newbie => "newbie",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        match value.trim().to_ascii_lowercase().as_str() {
            "general" => Some(Self::General),
            "newbie" => Some(Self::Newbie),
            _ => None,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum BoardAction {
    Catalog,
    List {
        board: BoardKey,
        page: u16,
    },
    Read {
        post_id: String,
    },
    Post {
        board: BoardKey,
        subject: String,
        body: String,
    },
    Delete {
        post_id: String,
    },
    Report {
        post_id: String,
        reason: String,
    },
}
