//! Typed wire contract for the experimental browser protocol `tiamud.v1`.
//!
//! This crate is the single source of truth for the v1 envelope, the message
//! type vocabulary, every client→server payload except `action.perform`, and
//! the server→client payloads whose shape is owned by the transport rather
//! than by a `tiamud-core` projection. Payloads that embed core projections
//! (`SceneView`, `PlayerStateView`, item/map views, narrative events) are
//! generic over that projection so the server instantiates them with the
//! authoritative core types while the browser instantiates them with its
//! strictly validated equivalents.
//!
//! The TypeScript declarations in `apps/web/src/lib/generated/protocol.ts` are
//! generated from these types by a test in this crate (see `typescript`). The
//! test fails if the committed file is stale, so `cargo test --workspace`
//! enforces that the browser and server share one declaration.
//!
//! Wire compatibility rules: field names, optionality (omitted versus `null`)
//! and tagging must match `docs/protocol/tiamud-v1.md` exactly. The server
//! converts every typed message to a `serde_json::Value` before queueing it,
//! so key order is the canonical sorted order of `serde_json::Map` and the
//! encoded bytes are identical to the former ad hoc `json!` construction.
//! Golden tests in this crate and in `tiamud-server` pin that equivalence.

#![forbid(unsafe_code)]

use serde::{Deserialize, Serialize, Serializer};
#[cfg(test)]
use ts_rs::TS;

#[cfg(test)]
mod typescript;

/// The only accepted value of the envelope `v` field.
pub const PROTOCOL_VERSION: u8 = 1;
/// The WebSocket subprotocol name offered by the official client.
pub const WEBSOCKET_SUBPROTOCOL: &str = "tiamud.v1";

/// Serializes as the literal protocol version `1`.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct ProtocolVersion;

impl Serialize for ProtocolVersion {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_u8(PROTOCOL_VERSION)
    }
}

/// Every server→client message type of `tiamud.v1`.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq, Serialize)]
#[cfg_attr(test, derive(TS))]
pub enum ServerMessageType {
    #[serde(rename = "server.welcome")]
    ServerWelcome,
    #[serde(rename = "server.notice")]
    ServerNotice,
    #[serde(rename = "server.heartbeat")]
    ServerHeartbeat,
    #[serde(rename = "action.result")]
    ActionResult,
    #[serde(rename = "scene.snapshot")]
    SceneSnapshot,
    #[serde(rename = "player.snapshot")]
    PlayerSnapshot,
    #[serde(rename = "scene.patch")]
    ScenePatch,
    #[serde(rename = "entity.upsert")]
    EntityUpsert,
    #[serde(rename = "entity.remove")]
    EntityRemove,
    #[serde(rename = "vitals.update")]
    VitalsUpdate,
    #[serde(rename = "inventory.patch")]
    InventoryPatch,
    #[serde(rename = "container.contents")]
    ContainerContents,
    #[serde(rename = "trade.contents")]
    TradeContents,
    #[serde(rename = "reward.odds")]
    RewardOdds,
    #[serde(rename = "mail.list")]
    MailList,
    #[serde(rename = "mail.message")]
    MailMessage,
    #[serde(rename = "board.catalog")]
    BoardCatalog,
    #[serde(rename = "board.list")]
    BoardList,
    #[serde(rename = "board.entry")]
    BoardEntry,
    #[serde(rename = "map.snapshot")]
    MapSnapshot,
    #[serde(rename = "message.append")]
    MessageAppend,
    #[serde(rename = "chat.message")]
    ChatMessage,
    #[serde(rename = "sync.scene")]
    SyncScene,
    #[serde(rename = "sync.player")]
    SyncPlayer,
    #[serde(rename = "media.play")]
    MediaPlay,
    #[serde(rename = "media.stop")]
    MediaStop,
    #[serde(rename = "story.entered")]
    StoryEntered,
    #[serde(rename = "story.view")]
    StoryView,
    #[serde(rename = "story.left")]
    StoryLeft,
    #[serde(rename = "story.error")]
    StoryError,
}

impl ServerMessageType {
    /// Wire name of this message type.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::ServerWelcome => "server.welcome",
            Self::ServerNotice => "server.notice",
            Self::ServerHeartbeat => "server.heartbeat",
            Self::ActionResult => "action.result",
            Self::SceneSnapshot => "scene.snapshot",
            Self::PlayerSnapshot => "player.snapshot",
            Self::ScenePatch => "scene.patch",
            Self::EntityUpsert => "entity.upsert",
            Self::EntityRemove => "entity.remove",
            Self::VitalsUpdate => "vitals.update",
            Self::InventoryPatch => "inventory.patch",
            Self::ContainerContents => "container.contents",
            Self::TradeContents => "trade.contents",
            Self::RewardOdds => "reward.odds",
            Self::MailList => "mail.list",
            Self::MailMessage => "mail.message",
            Self::BoardCatalog => "board.catalog",
            Self::BoardList => "board.list",
            Self::BoardEntry => "board.entry",
            Self::MapSnapshot => "map.snapshot",
            Self::MessageAppend => "message.append",
            Self::ChatMessage => "chat.message",
            Self::SyncScene => "sync.scene",
            Self::SyncPlayer => "sync.player",
            Self::MediaPlay => "media.play",
            Self::MediaStop => "media.stop",
            Self::StoryEntered => "story.entered",
            Self::StoryView => "story.view",
            Self::StoryLeft => "story.left",
            Self::StoryError => "story.error",
        }
    }
}

/// Every client→server message type of `tiamud.v1`.
#[derive(Clone, Copy, Debug, Deserialize, Eq, Hash, PartialEq, Serialize)]
#[cfg_attr(test, derive(TS))]
pub enum ClientMessageType {
    #[serde(rename = "client.hello")]
    ClientHello,
    #[serde(rename = "command.submit")]
    CommandSubmit,
    #[serde(rename = "action.perform")]
    ActionPerform,
    #[serde(rename = "sync.request")]
    SyncRequest,
    #[serde(rename = "client.heartbeat")]
    ClientHeartbeat,
    #[serde(rename = "story.choose")]
    StoryChoose,
    #[serde(rename = "story.menu")]
    StoryMenu,
    #[serde(rename = "story.dialogue")]
    StoryDialogue,
    #[serde(rename = "story.undo")]
    StoryUndo,
    #[serde(rename = "story.leave")]
    StoryLeave,
}

/// A sequenced server→client envelope.
#[derive(Clone, Debug, PartialEq, Serialize)]
#[cfg_attr(test, derive(TS))]
pub struct ServerEnvelope<T> {
    #[cfg_attr(test, ts(type = "ProtocolVersion"))]
    pub v: ProtocolVersion,
    #[serde(rename = "type")]
    pub message_type: ServerMessageType,
    pub seq: u64,
    pub data: T,
}

impl<T> ServerEnvelope<T> {
    pub const fn new(message_type: ServerMessageType, seq: u64, data: T) -> Self {
        Self {
            v: ProtocolVersion,
            message_type,
            seq,
            data,
        }
    }
}

/// An unsequenced server→client envelope. Only `server.heartbeat` uses it, so
/// best-effort liveness traffic never displaces an accepted result.
#[derive(Clone, Debug, PartialEq, Serialize)]
#[cfg_attr(test, derive(TS))]
pub struct UnsequencedEnvelope<T> {
    #[cfg_attr(test, ts(type = "ProtocolVersion"))]
    pub v: ProtocolVersion,
    #[serde(rename = "type")]
    pub message_type: ServerMessageType,
    pub data: T,
}

impl<T> UnsequencedEnvelope<T> {
    pub const fn new(message_type: ServerMessageType, data: T) -> Self {
        Self {
            v: ProtocolVersion,
            message_type,
            data,
        }
    }
}

// ---------------------------------------------------------------------------
// Client → server
// ---------------------------------------------------------------------------

/// Generic inbound envelope. `data` is decoded per `type` by the server after
/// recursive duplicate-key and depth checks.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ClientEnvelope {
    pub v: u8,
    #[serde(rename = "type")]
    pub message_type: String,
    #[serde(default)]
    pub id: Option<String>,
    pub data: serde_json::Value,
}

/// The first, authenticated client frame.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ClientHello {
    pub v: u8,
    #[serde(rename = "type")]
    pub message_type: String,
    pub data: ClientHelloData,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[cfg_attr(test, derive(TS))]
#[serde(rename_all = "lowercase")]
pub enum ClientKind {
    Web,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[cfg_attr(test, derive(TS))]
#[serde(rename_all = "lowercase")]
pub enum ImageFormat {
    Avif,
    Webp,
    Jpeg,
}

/// The only non-empty image preference list the server accepts, in order.
pub const SUPPORTED_IMAGE_FORMATS: [ImageFormat; 3] =
    [ImageFormat::Avif, ImageFormat::Webp, ImageFormat::Jpeg];

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[cfg_attr(test, derive(TS))]
#[serde(deny_unknown_fields)]
pub struct ClientHelloData {
    pub client: ClientKind,
    pub capabilities: ClientCapabilities,
    pub auth: ClientAuth,
}

/// Capability advertisement. Unlike every other client object this one
/// tolerates additive unknown keys, which an older server ignores.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[cfg_attr(test, derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct ClientCapabilities {
    pub images: Vec<ImageFormat>,
    pub reduced_motion: bool,
    #[serde(default)]
    pub audio: bool,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[cfg_attr(test, derive(TS))]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct ClientAuth {
    pub token: String,
    pub character_id: i64,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[cfg_attr(test, derive(TS))]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct CommandSubmitData {
    pub request_id: String,
    pub command: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[cfg_attr(test, derive(TS))]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct SyncRequestData {
    pub after_seq: u64,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[cfg_attr(test, derive(TS))]
#[serde(deny_unknown_fields)]
pub struct ClientHeartbeatData {
    pub nonce: u64,
}

// ---------------------------------------------------------------------------
// Server → client: control
// ---------------------------------------------------------------------------

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[cfg_attr(test, derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct ServerWelcome {
    pub server_name: String,
    pub motd: String,
    pub session_id: String,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[cfg_attr(test, derive(TS))]
#[serde(rename_all = "lowercase")]
pub enum NoticeLevel {
    Info,
    Warning,
    Error,
}

/// Operational/protocol notice. `code` is omitted, never `null`, when absent.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[cfg_attr(test, derive(TS))]
pub struct ServerNotice {
    pub level: NoticeLevel,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[cfg_attr(test, ts(optional))]
    pub code: Option<String>,
    pub message: String,
}

impl ServerNotice {
    pub fn new(level: NoticeLevel, message: impl Into<String>) -> Self {
        Self {
            level,
            code: None,
            message: message.into(),
        }
    }

    pub fn with_code(
        level: NoticeLevel,
        code: impl Into<String>,
        message: impl Into<String>,
    ) -> Self {
        Self {
            level,
            code: Some(code.into()),
            message: message.into(),
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[cfg_attr(test, derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct ServerHeartbeat {
    pub nonce: u64,
    pub timeout_ms: u64,
}

/// Correlated result. `message` is always present and is `null` when absent.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[cfg_attr(test, derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct ActionResult {
    pub request_id: String,
    pub ok: bool,
    pub message: Option<String>,
}

// ---------------------------------------------------------------------------
// Server → client: scene and player state
// ---------------------------------------------------------------------------

/// `scene.snapshot` payload: the authorized scene projection with an optional
/// presentation annotation merged into the same object.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct SceneFrame<Scene, Presentation> {
    #[serde(flatten)]
    pub scene: Scene,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub presentation: Option<Presentation>,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[cfg_attr(test, derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct SyncScene<Scene> {
    pub sync_id: String,
    pub after_seq: u64,
    pub scene: Scene,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[cfg_attr(test, derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct SyncPlayer<Player> {
    pub sync_id: String,
    pub after_seq: u64,
    pub player: Player,
    // Always `true`: the player part closes the recovery pair.
    #[cfg_attr(test, ts(type = "true"))]
    pub complete: bool,
}

impl<Player> SyncPlayer<Player> {
    pub fn new(sync_id: impl Into<String>, after_seq: u64, player: Player) -> Self {
        Self {
            sync_id: sync_id.into(),
            after_seq,
            player,
            complete: true,
        }
    }
}

/// Changed scene scalars. Unchanged fields are omitted; an image that
/// disappeared is sent as `null`.
#[derive(Clone, Debug, PartialEq, Serialize)]
#[cfg_attr(test, derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct ScenePatch<Image, Exit, Weather> {
    pub view_id: String,
    pub revision: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[cfg_attr(test, ts(optional))]
    pub title: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[cfg_attr(test, ts(optional))]
    pub prose: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[cfg_attr(test, ts(optional))]
    pub image: Option<Option<Image>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[cfg_attr(test, ts(optional))]
    pub exits: Option<Vec<Exit>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[cfg_attr(test, ts(optional))]
    pub weather: Option<Weather>,
}

impl<Image, Exit, Weather> ScenePatch<Image, Exit, Weather> {
    pub fn new(view_id: impl Into<String>, revision: u64) -> Self {
        Self {
            view_id: view_id.into(),
            revision,
            title: None,
            prose: None,
            image: None,
            exits: None,
            weather: None,
        }
    }

    /// Whether at least one scene field changed.
    pub const fn has_changes(&self) -> bool {
        self.title.is_some()
            || self.prose.is_some()
            || self.image.is_some()
            || self.exits.is_some()
            || self.weather.is_some()
    }
}

#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize)]
#[cfg_attr(test, derive(TS))]
#[serde(rename_all = "lowercase")]
pub enum EntityKind {
    Item,
    Npc,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[cfg_attr(test, derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct EntityRemove {
    pub view_id: String,
    pub revision: u64,
    pub kind: EntityKind,
    pub id: String,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[cfg_attr(test, derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct EntityUpsert<Entity> {
    pub view_id: String,
    pub revision: u64,
    pub kind: EntityKind,
    pub id: String,
    pub entity: Entity,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[cfg_attr(test, derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct VitalsUpdate<Character> {
    pub revision: u64,
    pub character: Character,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[cfg_attr(test, derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct InventoryPatch<Item, Equipped, Capacity> {
    pub revision: u64,
    pub inventory: Vec<Item>,
    pub equipment: Vec<Equipped>,
    pub inventory_capacity: Capacity,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[cfg_attr(test, derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct MapSnapshot<DiscoveredMap> {
    pub revision: u64,
    pub map: DiscoveredMap,
}

/// Authorized page of up to 20 direct child groups of a carried container.
#[derive(Clone, Debug, PartialEq, Serialize)]
#[cfg_attr(test, derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct ContainerContents<Item, Capacity> {
    pub container: String,
    pub items: Vec<Item>,
    pub page: u16,
    pub total_pages: u16,
    pub capacity: Capacity,
}

/// Private page of up to 20 direct child groups of an offered container.
#[derive(Clone, Debug, PartialEq, Serialize)]
#[cfg_attr(test, derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct TradeContents<Item, Capacity> {
    pub trade_id: String,
    pub container: String,
    pub items: Vec<Item>,
    pub page: u16,
    pub total_pages: u16,
    pub capacity: Capacity,
}

// ---------------------------------------------------------------------------
// Server → client: narrative, conversation and media
// ---------------------------------------------------------------------------

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[cfg_attr(test, derive(TS))]
#[serde(rename_all = "lowercase")]
pub enum CombatCueKind {
    Attack,
    Hit,
    Defeat,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[cfg_attr(test, derive(TS))]
#[serde(rename_all = "lowercase")]
pub enum CombatCuePose {
    Attack,
    Hit,
    Defeated,
}

/// Authoritative presentation cue attached to a `message.append`.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[cfg_attr(test, derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct CombatCue {
    pub kind: CombatCueKind,
    pub entity_id: String,
    pub pose: CombatCuePose,
    pub intensity: u8,
    pub duration_ms: u32,
}

impl CombatCue {
    pub fn attack(entity_id: impl Into<String>) -> Self {
        Self {
            kind: CombatCueKind::Attack,
            entity_id: entity_id.into(),
            pose: CombatCuePose::Attack,
            intensity: 2,
            duration_ms: 420,
        }
    }

    pub fn hit(entity_id: impl Into<String>) -> Self {
        Self {
            kind: CombatCueKind::Hit,
            entity_id: entity_id.into(),
            pose: CombatCuePose::Hit,
            intensity: 2,
            duration_ms: 320,
        }
    }

    pub fn defeat(entity_id: impl Into<String>) -> Self {
        Self {
            kind: CombatCueKind::Defeat,
            entity_id: entity_id.into(),
            pose: CombatCuePose::Defeated,
            intensity: 3,
            duration_ms: 700,
        }
    }
}

/// `message.append` payload: the perception-filtered narrative event with an
/// optional cue merged into the same object.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct MessageAppend<Event> {
    #[serde(flatten)]
    pub event: Event,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cue: Option<CombatCue>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[cfg_attr(test, derive(TS))]
#[serde(rename_all = "lowercase")]
pub enum ChatChannel {
    Announcement,
    Say,
    Yell,
    Shout,
    Tell,
    Telepathy,
    Emote,
    Party,
    General,
    Newbie,
    Staff,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[cfg_attr(test, derive(TS))]
pub struct ChatMessage {
    pub channel: ChatChannel,
    pub sender: String,
    pub text: String,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[cfg_attr(test, derive(TS))]
#[serde(rename_all = "lowercase")]
pub enum MediaChannel {
    Ambience,
    Effects,
    Music,
}

impl MediaChannel {
    /// Parses one of the closed audio channel names.
    pub fn from_name(name: &str) -> Option<Self> {
        match name {
            "ambience" => Some(Self::Ambience),
            "effects" => Some(Self::Effects),
            "music" => Some(Self::Music),
            _ => None,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[cfg_attr(test, derive(TS))]
pub struct MediaPlay {
    pub channel: MediaChannel,
    pub url: String,
    #[serde(rename = "loop")]
    pub looped: bool,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[cfg_attr(test, derive(TS))]
pub struct MediaStop {
    pub channel: MediaChannel,
}

// ---------------------------------------------------------------------------
// Server → client: offline mail and community boards
// ---------------------------------------------------------------------------

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[cfg_attr(test, derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct MailMessageSummary {
    pub id: String,
    pub sender: String,
    pub subject: String,
    pub created_at: i64,
    pub expires_at: i64,
    pub read: bool,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[cfg_attr(test, derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct MailList {
    pub page: u32,
    pub total_pages: u32,
    pub total_messages: u32,
    pub unread_count: u32,
    pub capacity: u32,
    pub messages: Vec<MailMessageSummary>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[cfg_attr(test, derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct MailMessage {
    pub id: String,
    pub sender: String,
    pub subject: String,
    pub body: String,
    pub created_at: i64,
    pub expires_at: i64,
    pub read: bool,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[cfg_attr(test, derive(TS))]
#[serde(rename_all = "snake_case")]
pub enum BoardKey {
    General,
    Newbie,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[cfg_attr(test, derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct BoardDescriptor {
    pub key: BoardKey,
    pub label: String,
    pub description: String,
    pub can_post: bool,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[cfg_attr(test, derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct BoardCatalog {
    pub boards: Vec<BoardDescriptor>,
    pub retention_days: u32,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[cfg_attr(test, derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct BoardPostSummary {
    pub post_id: String,
    pub board: BoardKey,
    pub author: String,
    pub subject: String,
    pub created_at: i64,
    pub expires_at: i64,
    pub can_delete: bool,
    pub can_report: bool,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[cfg_attr(test, derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct BoardList {
    pub board: BoardKey,
    pub page: u32,
    pub total_pages: u32,
    pub total_posts: u32,
    pub capacity: u32,
    pub posts: Vec<BoardPostSummary>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[cfg_attr(test, derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct BoardEntry {
    pub post_id: String,
    pub board: BoardKey,
    pub author: String,
    pub subject: String,
    pub body: String,
    pub created_at: i64,
    pub expires_at: i64,
    pub can_delete: bool,
    pub can_report: bool,
}

// ---------------------------------------------------------------------------
// Story sessions (ADR 0029, phase S2 of ADR 0025)
// ---------------------------------------------------------------------------

/// Bytes of an opaque per-view story id (`c` + 16 lowercase hex digits).
pub const STORY_CHOICE_ID_BYTES: usize = 17;

/// `story.choose` and `story.dialogue`: takes a gamebook choice or answers the
/// active dialogue. `id` is an opaque per-view id and `viewRev` the revision
/// that published it; a stale revision or an id that was never published is
/// rejected without changing state.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[cfg_attr(test, derive(TS))]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct StoryChooseData {
    pub id: String,
    pub view_rev: u32,
}

/// `story.menu`: applies a menu verb, with one of that verb's targets when the
/// verb needs one. Both are opaque ids of the same view.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[cfg_attr(test, derive(TS))]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct StoryMenuData {
    pub verb: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(test, ts(optional))]
    pub target: Option<String>,
    pub view_rev: u32,
}

/// `story.undo` and `story.leave` carry exactly `{}`.
#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[cfg_attr(test, derive(TS))]
#[serde(deny_unknown_fields)]
pub struct StoryEmptyData {}

/// `story.entered`: the session now routes input to a story instance.
/// `storyId` is the public catalog id the player typed, never a node id.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[cfg_attr(test, derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct StoryEntered {
    pub story_id: String,
    pub title: String,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[cfg_attr(test, derive(TS))]
#[serde(rename_all = "lowercase")]
pub enum StoryBlockKind {
    /// What the last transition did (dice, items, score, visible stats).
    Event,
    /// A menu response or dialogue line, optionally spoken by a character.
    Line,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[cfg_attr(test, derive(TS))]
#[serde(rename_all = "lowercase")]
pub enum StoryEndingKind {
    Victory,
    Defeat,
    Neutral,
}

/// Ending kind only; the authored ending id stays on the server.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[cfg_attr(test, derive(TS))]
pub struct StoryEnding {
    pub kind: StoryEndingKind,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[cfg_attr(test, derive(TS))]
#[serde(rename_all = "lowercase")]
pub enum StoryCueAction {
    Play,
    Stop,
}

/// One self-contained piece of a story view, in display order. Ids are
/// opaque per view; authored ids, use counters and RNG state never appear.
/// Presentation-only items (`cue`, `portrait`) may be ignored without losing
/// any playable fact.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[cfg_attr(test, derive(TS))]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum StoryItem {
    /// Title, node prose, score and ending of the current scene.
    Scene {
        title: String,
        text: String,
        score: i64,
        ending: Option<StoryEnding>,
    },
    /// Media cue. `url` is present only for `play` and only when the
    /// authored asset id resolved through the reviewed story media map to a
    /// content-addressed `/assets/<sha256>/audio-*.{opus,ogg,mp3}` path.
    Cue {
        action: StoryCueAction,
        channel: MediaChannel,
        #[serde(skip_serializing_if = "Option::is_none")]
        #[cfg_attr(test, ts(optional))]
        url: Option<String>,
        #[serde(rename = "loop")]
        looped: bool,
    },
    /// A character present in the scene (portrait images are not delivered
    /// in S2; the name is the text equivalent).
    Portrait { name: String },
    Block {
        kind: StoryBlockKind,
        #[serde(skip_serializing_if = "Option::is_none")]
        #[cfg_attr(test, ts(optional))]
        speaker: Option<String>,
        text: String,
    },
    /// A visible stat with its value rendered as text.
    Stat { label: String, value: String },
    /// An inventory entry.
    Item { name: String },
    /// A gamebook choice (`story.choose`).
    Choice { id: String, label: String },
    /// A menu verb (`story.menu`). With `needsTarget`, one of the following
    /// `target` items naming this verb must accompany it.
    Verb {
        id: String,
        label: String,
        #[serde(rename = "needsTarget")]
        needs_target: bool,
    },
    /// A target of the verb whose id is `verb`.
    Target {
        verb: String,
        id: String,
        label: String,
    },
    /// An option of the active dialogue (`story.dialogue` or `story.choose`).
    #[serde(rename = "dialogueOption")]
    DialogueOption { id: String, label: String },
}

/// One part of a story view. A view whose encoding would exceed the frame
/// budget is split into `parts` frames sharing `viewRev`; the client applies
/// the view once every part `0..parts` has arrived, concatenating `items` in
/// part order.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[cfg_attr(test, derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct StoryView {
    pub view_rev: u32,
    pub part: u16,
    pub parts: u16,
    pub turn: u32,
    pub can_undo: bool,
    pub items: Vec<StoryItem>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[cfg_attr(test, derive(TS))]
#[serde(rename_all = "lowercase")]
pub enum StoryLeftReason {
    /// The player left.
    Left,
    /// The instance no longer exists: idle eviction or closed by another
    /// session of the account (S2 has no save slots).
    Evicted,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[cfg_attr(test, derive(TS))]
pub struct StoryLeft {
    pub reason: StoryLeftReason,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[cfg_attr(test, derive(TS))]
#[serde(rename_all = "snake_case")]
pub enum StoryErrorCode {
    Disabled,
    UnknownStory,
    AlreadyInStory,
    Capacity,
    NotInStory,
    StaleView,
    UnknownChoice,
    Ended,
    NothingToUndo,
    Busy,
    Internal,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[cfg_attr(test, derive(TS))]
pub struct StoryError {
    pub code: StoryErrorCode,
    pub message: String,
}

#[cfg(test)]
mod golden;
