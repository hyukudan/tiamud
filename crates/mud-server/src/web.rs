use crate::{
    boards::BoardDescriptor,
    delivery::SessionDelivery,
    operations::{
        ActiveSessionLimit, ActiveSessionLimiter, AuthRateLimiter, CommandRateLimiter,
        OutputTransport, RuntimeMetrics,
    },
    persistence::{
        BoardEntry, BoardPage, CharacterStore, MAX_REPORT_REASON_BYTES, MAX_SOCIAL_REASON_BYTES,
        MailMessage, MailPage, StoreError,
    },
    social::{MAX_CHAT_MESSAGE_BYTES, MAX_SOCIAL_TARGET_BYTES},
    story_sessions::{STORY_INPUT_HELP, parse_story_command},
    world_actor::{
        CharacterBackgroundSelection, CombatCueKind, GameError, ProjectedViewEvent, WorldHandle,
    },
};
use axum::{
    Json,
    body::Body,
    extract::{
        ConnectInfo, State, WebSocketUpgrade,
        ws::{CloseFrame, Message, WebSocket},
    },
    http::{HeaderMap, StatusCode, Uri, header},
    response::{IntoResponse, Response},
};
use rust_embed::RustEmbed;
use serde::{
    Deserialize, Deserializer, Serialize,
    de::{Error as _, MapAccess, SeqAccess, Visitor},
};
use serde_json::{Map, Value, json};
use std::{
    collections::{BTreeMap, HashMap, HashSet, VecDeque},
    net::SocketAddr,
    sync::{Arc, Mutex},
    time::Instant,
};
use tiamud_core::{
    AccountRole, BoardAction, BoardKey, CharacterId, CharacterSnapshot, ContainerContentsView,
    EconomyAction, ItemSelection, LootAction, MAX_BOARD_BODY_BYTES, MAX_BOARD_PAGE,
    MAX_BOARD_REPORT_REASON_BYTES, MAX_MAIL_BODY_BYTES, MAX_MAIL_PAGE,
    MAX_MAIL_REPORT_REASON_BYTES, MailAction, PartyAction, PlayerAction, PlayerStateView,
    QuestAction, RewardOddsView, SceneView, SessionId, SocialAction, SocialChannelName,
    StaffAction, StaffCapability, StaffProfileKey, StaffReportResolution, TradeAction,
    TradeContentsView, WorldError, parse_action, valid_board_public_id, valid_board_subject,
    valid_board_visible_text, valid_mail_public_id, valid_mail_subject, valid_mail_target,
    valid_mail_visible_text,
};
use tiamud_protocol::{self as protocol, ServerMessageType};
use tokio::{
    sync::{Notify, mpsc, watch},
    task::JoinHandle,
    time::{Duration, MissedTickBehavior, interval, timeout},
};
use tracing::warn;

const MAX_WS_MESSAGE_BYTES: usize = 16 * 1024;
// Leave room for the envelope, sync metadata and a small optional annotation.
// A deterministic semantic window also keeps tick diffs bounded when hundreds
// of ground-loot reservations exist in one room.
const MAX_WS_SCENE_SEMANTIC_BYTES: usize = 14 * 1024;
const TRADE_LABEL_FALLBACK_BYTES: [usize; 4] = [128, 64, 32, 16];
const MAX_COMMAND_BYTES: usize = 1024;
const MAX_REQUEST_ID_BYTES: usize = 128;
const MAX_PROTOCOL_ERRORS: u8 = 3;
const MAX_ACTION_TARGET_BYTES: usize = 256;
const MAX_STAFF_TARGET_BYTES: usize = 64;
const MAX_STAFF_REASON_BYTES: usize = 1_024;
const MAX_STAFF_LORE_TITLE_BYTES: usize = 128;
const MAX_ABILITY_ID_BYTES: usize = 128;
const MAX_PROGRESSION_ID_BYTES: usize = 128;
const MAX_JSON_DEPTH: usize = 16;
const GLOBAL_REQUEST_CACHE_CAPACITY: usize = 4_096;
const WS_INBOUND_CAPACITY: usize = 32;
const WS_OUTBOUND_CAPACITY: usize = 32;
const MAX_WS_SEMANTIC_DELTA_FRAMES: usize = 8;
const MAX_PROJECTED_EVENTS_PER_BATCH: usize = 16;
const WS_WRITE_TIMEOUT: Duration = Duration::from_secs(5);
const HEARTBEAT_INTERVAL: Duration = Duration::from_secs(15);
const HEARTBEAT_TIMEOUT: Duration = Duration::from_secs(45);

#[derive(Clone)]
pub struct AppState {
    pub world: WorldHandle,
    pub store: Arc<dyn CharacterStore>,
    pub metrics: Arc<RuntimeMetrics>,
    pub auth_rate_limiter: Arc<AuthRateLimiter>,
    pub active_session_limiter: Arc<ActiveSessionLimiter>,
    pub command_rate_per_second: u32,
    pub command_burst: u32,
    pub shutdown: watch::Receiver<bool>,
    pub(crate) action_replay: Arc<Mutex<RequestCache>>,
    /// `None` when story sessions are disabled (no `TIAMUD_STORY_DIR`).
    pub(crate) stories: Option<crate::story_sessions::StoryManager>,
}

pub(crate) fn action_replay_cache() -> Arc<Mutex<RequestCache>> {
    Arc::new(Mutex::new(RequestCache::new(GLOBAL_REQUEST_CACHE_CAPACITY)))
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CredentialsRequest {
    name: String,
    password: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct CreateCharacterRequest {
    name: String,
    #[serde(default)]
    lineage_id: Option<String>,
    #[serde(
        default,
        rename = "backgroundId",
        deserialize_with = "deserialize_background_selection"
    )]
    background: CharacterBackgroundSelection,
    #[serde(default)]
    base_class_id: Option<String>,
}

fn deserialize_background_selection<'de, D>(
    deserializer: D,
) -> Result<CharacterBackgroundSelection, D::Error>
where
    D: Deserializer<'de>,
{
    Ok(match Option::<String>::deserialize(deserializer)? {
        Some(id) => CharacterBackgroundSelection::Id(id),
        None => CharacterBackgroundSelection::None,
    })
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct CharactersResponse {
    characters: Vec<CharacterSnapshot>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct CharacterResponse {
    character: CharacterSnapshot,
}

#[derive(Debug, Eq, PartialEq)]
struct ClientHelloCredentials {
    token: String,
    character_id: i64,
    audio: bool,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
struct ActionPerformData {
    request_id: String,
    action: String,
    #[serde(default)]
    emote: Option<String>,
    #[serde(default)]
    mail_id: Option<String>,
    #[serde(default)]
    post_id: Option<String>,
    #[serde(default)]
    board: Option<BoardKey>,
    #[serde(default)]
    subject: Option<String>,
    #[serde(default)]
    body: Option<String>,
    #[serde(default)]
    ability_id: Option<String>,
    #[serde(default)]
    class_id: Option<String>,
    #[serde(default)]
    graph_id: Option<String>,
    #[serde(default)]
    node_id: Option<String>,
    #[serde(default)]
    target: Option<String>,
    #[serde(default)]
    recipient: Option<String>,
    #[serde(default)]
    container: Option<String>,
    #[serde(default)]
    quantity: Option<u16>,
    #[serde(default)]
    page: Option<u16>,
    #[serde(default)]
    trade_id: Option<String>,
    #[serde(default)]
    coins: Option<u64>,
    #[serde(default)]
    state_revision: Option<u64>,
    #[serde(default)]
    message: Option<String>,
    #[serde(default)]
    channel: Option<SocialChannelName>,
    #[serde(default)]
    duration_seconds: Option<u64>,
    #[serde(default)]
    reason: Option<String>,
    #[serde(default)]
    role: Option<AccountRole>,
    #[serde(default)]
    report_id: Option<i64>,
    #[serde(default)]
    resolution: Option<StaffReportResolution>,
    #[serde(default, deserialize_with = "deserialize_nullable_field")]
    profile: NullableField<StaffProfileKey>,
    #[serde(default, deserialize_with = "deserialize_nullable_field")]
    lore_title: NullableField<String>,
    #[serde(default)]
    capability: Option<StaffCapability>,
    #[serde(default)]
    enabled: Option<bool>,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
enum NullableField<T> {
    #[default]
    Missing,
    Null,
    Value(T),
}

fn deserialize_nullable_field<'de, D, T>(deserializer: D) -> Result<NullableField<T>, D::Error>
where
    D: Deserializer<'de>,
    T: Deserialize<'de>,
{
    Ok(match Option::<T>::deserialize(deserializer)? {
        Some(value) => NullableField::Value(value),
        None => NullableField::Null,
    })
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd)]
enum OutboundPriority {
    Control = 0,
    Critical = 1,
    Narrative = 2,
    State = 3,
    BestEffort = 4,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum CoalesceKey {
    Scene,
    Player,
    ScenePatch,
    Vitals,
    Inventory,
    Map,
    SyncScene,
    SyncPlayer,
    Heartbeat,
}

#[derive(Debug)]
struct OutboundEnvelope {
    priority: OutboundPriority,
    coalesce: Option<CoalesceKey>,
    sequence: Option<u64>,
    value: Value,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum EnqueueResult {
    Queued,
    Coalesced,
    Dropped,
    Disconnect,
}

#[derive(Debug)]
struct OutboundQueue {
    capacity: usize,
    frames: VecDeque<OutboundEnvelope>,
    disconnect_reason: Option<String>,
}

impl OutboundQueue {
    fn new(capacity: usize) -> Self {
        assert!(capacity > 0, "outbound queue capacity must be positive");
        Self {
            capacity,
            frames: VecDeque::with_capacity(capacity),
            disconnect_reason: None,
        }
    }

    fn enqueue(&mut self, frame: OutboundEnvelope) -> EnqueueResult {
        if self.disconnect_reason.is_some() {
            return EnqueueResult::Disconnect;
        }

        let mut coalesced = false;
        if let Some(key) = frame.coalesce
            && let Some(index) = self
                .frames
                .iter()
                .position(|queued| queued.coalesce == Some(key))
        {
            self.frames.remove(index);
            coalesced = true;
        }

        // A newly accepted critical result may overtake queued state. Remove
        // that stale state rather than ever deliver server sequences backwards.
        if frame.sequence.is_some() && frame.priority <= OutboundPriority::Critical {
            self.frames.retain(|queued| {
                queued.sequence.is_none() || queued.priority <= OutboundPriority::Critical
            });
        }

        if self.frames.len() == self.capacity {
            let eviction = self
                .frames
                .iter()
                .enumerate()
                .filter(|(_, queued)| queued.priority > frame.priority)
                .max_by_key(|(_, queued)| queued.priority)
                .map(|(index, _)| index);
            if let Some(index) = eviction {
                self.frames.remove(index);
            } else if frame.priority <= OutboundPriority::Critical {
                self.disconnect_reason =
                    Some("The connection could not keep up with critical game output.".to_owned());
                return EnqueueResult::Disconnect;
            } else {
                return EnqueueResult::Dropped;
            }
        }

        self.frames.push_back(frame);
        if coalesced {
            EnqueueResult::Coalesced
        } else {
            EnqueueResult::Queued
        }
    }

    fn pop(&mut self) -> Option<OutboundEnvelope> {
        let index = self
            .frames
            .iter()
            .enumerate()
            // Sequence numbers are the public ordering contract. Priority is
            // applied at enqueue time by evicting/coalescing stale lower-value
            // frames; reordering two retained sequenced frames here would make
            // a newer number overtake an older one and force an impossible
            // backwards stream after sync recovery.
            .min_by_key(|(_, frame)| {
                frame
                    .sequence
                    .map(|sequence| (0_u8, sequence))
                    .unwrap_or((1_u8, frame.priority as u64))
            })
            .map(|(index, _)| index)?;
        self.frames.remove(index)
    }

    fn disconnect(&mut self, reason: impl Into<String>) {
        self.disconnect_reason.get_or_insert_with(|| reason.into());
    }

    fn take_disconnect_reason(&mut self) -> Option<String> {
        self.disconnect_reason.take()
    }
}

#[derive(Debug)]
struct OutboundHandle {
    queue: Mutex<OutboundQueue>,
    changed: Notify,
    metrics: Arc<RuntimeMetrics>,
}

impl OutboundHandle {
    fn new(capacity: usize, metrics: Arc<RuntimeMetrics>) -> Self {
        assert!(capacity > 0, "request cache capacity must be positive");
        Self {
            queue: Mutex::new(OutboundQueue::new(capacity)),
            changed: Notify::new(),
            metrics,
        }
    }

    fn enqueue(&self, frame: OutboundEnvelope) -> EnqueueResult {
        let encoded_bytes =
            serde_json::to_vec(&frame.value).map_or(usize::MAX, |encoded| encoded.len());
        if encoded_bytes > MAX_WS_MESSAGE_BYTES {
            self.queue
                .lock()
                .unwrap_or_else(|error| error.into_inner())
                .disconnect("Server output exceeded the WebSocket message byte budget.");
            self.metrics.websocket_outbound_budget_exceeded();
            self.changed.notify_one();
            warn!(
                encoded_bytes,
                maximum_bytes = MAX_WS_MESSAGE_BYTES,
                "rejected oversized outbound WebSocket frame"
            );
            return EnqueueResult::Disconnect;
        }
        let (result, before, after) = {
            let mut queue = self.queue.lock().unwrap_or_else(|error| error.into_inner());
            let before = queue.frames.len();
            let result = queue.enqueue(frame);
            (result, before, queue.frames.len())
        };
        for _ in before..after {
            self.metrics
                .output_frame_enqueued(OutputTransport::Websocket);
        }
        for _ in after..before {
            self.metrics
                .output_frame_removed(OutputTransport::Websocket);
        }
        if result == EnqueueResult::Coalesced {
            self.metrics.output_coalesced(OutputTransport::Websocket);
        } else if result == EnqueueResult::Dropped {
            self.metrics.output_dropped(OutputTransport::Websocket);
        } else if result == EnqueueResult::Disconnect {
            self.metrics
                .slow_client_disconnected(OutputTransport::Websocket);
        }
        self.changed.notify_one();
        result
    }

    fn disconnect(&self, reason: impl Into<String>) {
        self.queue
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .disconnect(reason);
        self.changed.notify_one();
    }

    fn pop(&self) -> Option<OutboundEnvelope> {
        let frame = self
            .queue
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .pop();
        if frame.is_some() {
            self.metrics
                .output_frame_removed(OutputTransport::Websocket);
        }
        frame
    }

    fn take_disconnect_reason(&self) -> Option<String> {
        self.queue
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .take_disconnect_reason()
    }
}

impl Drop for OutboundHandle {
    fn drop(&mut self) {
        let remaining = self
            .queue
            .get_mut()
            .unwrap_or_else(|error| error.into_inner())
            .frames
            .len();
        for _ in 0..remaining {
            self.metrics
                .output_frame_removed(OutputTransport::Websocket);
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct CachedActionResult {
    signature: String,
    ok: bool,
    message: Option<String>,
    container_contents: Option<ContainerContentsView>,
    trade_contents: Option<TradeContentsView>,
    reward_odds: Option<RewardOddsView>,
    mail_page: Option<MailPage>,
    mail_message: Option<MailMessage>,
    board_catalog: Option<Vec<BoardDescriptor>>,
    board_page: Option<BoardPage>,
    board_entry: Option<BoardEntry>,
    projection_session: SessionId,
}

impl CachedActionResult {
    fn container_contents_for(&self, session: SessionId) -> Option<&ContainerContentsView> {
        (self.projection_session == session)
            .then_some(self.container_contents.as_ref())
            .flatten()
    }

    fn message_for(&self, session: SessionId) -> Option<String> {
        if self.projection_session != session {
            Some(
                "This request was already handled on a previous connection; the current state follows."
                    .into(),
            )
        } else {
            self.message.clone()
        }
    }

    fn trade_contents_for(&self, session: SessionId) -> Option<&TradeContentsView> {
        (self.projection_session == session)
            .then_some(self.trade_contents.as_ref())
            .flatten()
    }

    fn reward_odds_for(&self, session: SessionId) -> Option<&RewardOddsView> {
        (self.projection_session == session)
            .then_some(self.reward_odds.as_ref())
            .flatten()
    }

    fn mail_page_for(&self, session: SessionId) -> Option<&MailPage> {
        (self.projection_session == session)
            .then_some(self.mail_page.as_ref())
            .flatten()
    }

    fn mail_message_for(&self, session: SessionId) -> Option<&MailMessage> {
        (self.projection_session == session)
            .then_some(self.mail_message.as_ref())
            .flatten()
    }

    fn board_catalog_for(&self, session: SessionId) -> Option<&[BoardDescriptor]> {
        (self.projection_session == session)
            .then_some(self.board_catalog.as_deref())
            .flatten()
    }

    fn board_page_for(&self, session: SessionId) -> Option<&BoardPage> {
        (self.projection_session == session)
            .then_some(self.board_page.as_ref())
            .flatten()
    }

    fn board_entry_for(&self, session: SessionId) -> Option<&BoardEntry> {
        (self.projection_session == session)
            .then_some(self.board_entry.as_ref())
            .flatten()
    }
}

#[derive(Debug)]
pub(crate) struct RequestCache {
    capacity: usize,
    order: VecDeque<String>,
    entries: HashMap<String, CachedActionResult>,
}

impl RequestCache {
    fn new(capacity: usize) -> Self {
        Self {
            capacity,
            order: VecDeque::with_capacity(capacity),
            entries: HashMap::with_capacity(capacity),
        }
    }

    fn get(&self, request_id: &str) -> Option<&CachedActionResult> {
        self.entries.get(request_id)
    }

    fn insert(&mut self, request_id: String, result: CachedActionResult) {
        if let Some(cached) = self.entries.get_mut(&request_id) {
            *cached = result;
            return;
        }
        while self.entries.len() >= self.capacity {
            if let Some(oldest) = self.order.pop_front() {
                self.entries.remove(&oldest);
            } else {
                break;
            }
        }
        self.order.push_back(request_id.clone());
        self.entries.insert(request_id, result);
    }
}

fn action_replay_key(character_id: CharacterId, request_id: &str) -> String {
    format!("{}:{request_id}", character_id.0)
}

#[derive(Debug)]
struct RequestedAction {
    request_id: String,
    signature: String,
    action: Result<PlayerAction, String>,
    trade_selector: Option<String>,
    /// Raw `command.submit` text, checked for story commands before the world.
    text: Option<String>,
}

#[derive(Debug)]
enum ClientMessage {
    Action(RequestedAction),
    Sync {
        after_seq: u64,
    },
    Heartbeat {
        nonce: u64,
    },
    StoryInput {
        request: crate::story_sessions::StoryRequest,
        view_rev: u32,
    },
    StoryUndo,
    StoryLeave,
    StorySave {
        slot: u8,
    },
    /// `story.restore {slot}`; `story.continue` is slot 0.
    StoryRestore {
        slot: u8,
    },
    StorySlots,
    StoryDeleteSlot {
        slot: u8,
    },
    /// `story.vote {choiceId?, viewRev}` (ADR 0033).
    StoryVote {
        leaf: Option<String>,
        view_rev: u32,
    },
    StoryShare,
    StoryJoin {
        code: String,
        spectate: bool,
    },
    StoryInvite {
        name: String,
    },
    StoryKick {
        name: String,
    },
    StoryMode {
        mode: protocol::StoryDecisionMode,
        vote_seconds: Option<u16>,
    },
    /// `story.command {text, viewRev}`: free text for a parser story.
    StoryCommand {
        text: String,
        view_rev: u32,
    },
}

#[derive(Debug, Eq, PartialEq)]
enum ClientMessageError {
    MalformedJson,
    UnsupportedVersion,
    UnknownType,
    InvalidEnvelope,
}

impl ClientMessageError {
    const fn public_message(&self) -> &'static str {
        match self {
            Self::MalformedJson => "Malformed JSON command.",
            Self::UnsupportedVersion => "Unsupported protocol version.",
            Self::UnknownType => "Unknown protocol message type.",
            Self::InvalidEnvelope => "Invalid protocol envelope.",
        }
    }
}

#[derive(RustEmbed)]
#[cfg(web_dist)]
#[folder = "../../apps/web/dist"]
struct WebAssets;

struct SessionLease {
    world: WorldHandle,
    session: SessionId,
    metrics: Arc<RuntimeMetrics>,
}

impl Drop for SessionLease {
    fn drop(&mut self) {
        self.metrics.session_closed();
        self.world.leave_nowait(self.session);
    }
}

struct WebSocketLease(Arc<RuntimeMetrics>);

impl Drop for WebSocketLease {
    fn drop(&mut self) {
        self.0.websocket_closed();
    }
}

#[derive(RustEmbed)]
#[cfg(not(web_dist))]
#[folder = "web-fallback"]
struct WebAssets;

pub async fn live() -> Json<Value> {
    Json(json!({ "status": "ok" }))
}

pub async fn ready(State(state): State<AppState>) -> Response {
    let (world, storage) = tokio::join!(
        timeout(Duration::from_secs(2), state.world.health_check()),
        timeout(Duration::from_secs(2), state.store.health_check()),
    );
    let world_ready = matches!(world, Ok(Ok(())));
    let storage_ready = matches!(storage, Ok(Ok(())));
    let status = if world_ready && storage_ready {
        StatusCode::OK
    } else {
        StatusCode::SERVICE_UNAVAILABLE
    };
    (
        status,
        Json(readiness_body(
            status,
            world_ready,
            storage_ready,
            state.store.backend_name(),
        )),
    )
        .into_response()
}

fn readiness_body(
    status: StatusCode,
    world_ready: bool,
    storage_ready: bool,
    backend: &str,
) -> Value {
    json!({
        "status": if status == StatusCode::OK { "ready" } else { "not_ready" },
        "world": world_ready,
        "storage": { "backend": backend, "ready": storage_ready }
    })
}

pub async fn metrics(State(state): State<AppState>) -> Response {
    state
        .metrics
        .set_database_queue_depth(state.store.queue_depth());
    let mut body = state.metrics.prometheus(state.world.queue_depth());
    if let Some(stories) = &state.stories {
        body.push_str(&stories.prometheus());
    }
    (
        StatusCode::OK,
        [(
            header::CONTENT_TYPE,
            "text/plain; version=0.0.4; charset=utf-8",
        )],
        Body::from(body),
    )
        .into_response()
}

pub async fn status() -> Json<Value> {
    Json(json!({
        "name": "Tiamud",
        "version": env!("CARGO_PKG_VERSION"),
        "protocol": "tiamud.v1",
        "images": ["avif", "webp", "jpeg"]
    }))
}

pub async fn register(
    ConnectInfo(remote): ConnectInfo<SocketAddr>,
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(request): Json<CredentialsRequest>,
) -> Response {
    if !allow_auth_attempt(&state, request_address(&headers, remote)) {
        return rate_limit_error();
    }
    match state
        .world
        .register_account(request.name, request.password)
        .await
    {
        Ok(session) => (StatusCode::CREATED, Json(session)).into_response(),
        Err(error) => api_error(error),
    }
}

pub async fn login(
    ConnectInfo(remote): ConnectInfo<SocketAddr>,
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(request): Json<CredentialsRequest>,
) -> Response {
    if !allow_auth_attempt(&state, request_address(&headers, remote)) {
        return rate_limit_error();
    }
    match state
        .world
        .login_account(request.name, request.password)
        .await
    {
        Ok(session) => Json(session).into_response(),
        Err(error) => api_error(error),
    }
}

pub async fn logout(State(state): State<AppState>, headers: HeaderMap) -> Response {
    let token = match bearer_token(&headers) {
        Ok(token) => token,
        Err(status) => return bearer_error(status),
    };
    match state.world.revoke_session(token).await {
        Ok(()) => StatusCode::NO_CONTENT.into_response(),
        Err(error) => api_error(error),
    }
}

pub async fn auth_session(State(state): State<AppState>, headers: HeaderMap) -> Response {
    let token = match bearer_token(&headers) {
        Ok(token) => token,
        Err(status) => return bearer_error(status),
    };
    match state.world.resolve_session(token).await {
        Ok(account) => Json(json!({"account": account})).into_response(),
        Err(error) => api_error(error),
    }
}

pub async fn characters(State(state): State<AppState>, headers: HeaderMap) -> Response {
    let token = match bearer_token(&headers) {
        Ok(token) => token,
        Err(status) => return bearer_error(status),
    };
    match state.world.authenticated_roster(token).await {
        Ok(characters) => Json(CharactersResponse { characters }).into_response(),
        Err(error) => api_error(error),
    }
}

pub async fn character_options(State(state): State<AppState>) -> Json<Value> {
    Json(json!(state.world.creation_catalog()))
}

pub async fn create_character(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(request): Json<CreateCharacterRequest>,
) -> Response {
    let token = match bearer_token(&headers) {
        Ok(token) => token,
        Err(status) => return bearer_error(status),
    };
    let catalog = state.world.creation_catalog();
    let known = |id: &str, options: &[tiamud_core::ProgressionOptionView]| {
        valid_definition_id(id, MAX_PROGRESSION_ID_BYTES)
            && options.iter().any(|option| option.id == id)
    };
    if request
        .lineage_id
        .as_deref()
        .is_some_and(|id| !known(id, &catalog.lineages))
        || matches!(
            &request.background,
            CharacterBackgroundSelection::Id(id) if !known(id, &catalog.backgrounds)
        )
        || request
            .base_class_id
            .as_deref()
            .is_some_and(|id| !known(id, &catalog.base_classes))
    {
        return (
            StatusCode::BAD_REQUEST,
            Json(json!({"error": "Unknown character creation option."})),
        )
            .into_response();
    }
    match state
        .world
        .create_authenticated_character_with_progression(
            token,
            request.name,
            request.lineage_id,
            request.background,
            request.base_class_id,
        )
        .await
    {
        Ok(character) => {
            (StatusCode::CREATED, Json(CharacterResponse { character })).into_response()
        }
        Err(error) => api_error(error),
    }
}

pub async fn ws_handler(
    State(state): State<AppState>,
    ConnectInfo(remote): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
    websocket: WebSocketUpgrade,
) -> Response {
    if !websocket_request_allowed(&headers) {
        return (
            StatusCode::FORBIDDEN,
            "WebSocket origin or protocol rejected",
        )
            .into_response();
    }
    let peer_address = request_address(&headers, remote);
    websocket
        .protocols(["tiamud.v1"])
        .max_message_size(MAX_WS_MESSAGE_BYTES)
        .max_frame_size(MAX_WS_MESSAGE_BYTES)
        .on_upgrade(move |socket| websocket_session(socket, state, peer_address))
        .into_response()
}

async fn websocket_session(mut socket: WebSocket, state: AppState, peer_address: std::net::IpAddr) {
    state.metrics.websocket_opened();
    let _websocket_lease = WebSocketLease(state.metrics.clone());
    let hello = timeout(Duration::from_secs(10), socket.recv()).await;
    let credentials = match hello {
        Ok(Some(Ok(Message::Text(text)))) => parse_client_hello(&text),
        _ => None,
    };
    let Some(credentials) = credentials else {
        reject_before_welcome(
            &mut socket,
            "access.protocol_rejected",
            "Expected an authenticated client.hello",
        )
        .await;
        return;
    };
    let character_id = CharacterId(credentials.character_id);
    let audio_enabled = credentials.audio;
    let mut shutdown = state.shutdown.clone();
    let entered = match state
        .world
        .enter_authenticated_character(credentials.token, character_id)
        .await
    {
        Ok(entered) => entered,
        Err(error) => {
            warn!(%error, "authenticated client could not enter the world");
            let code = access_rejection_code(&error);
            reject_before_welcome(&mut socket, code, &public_game_error(&error)).await;
            return;
        }
    };
    let _active_session_permit = match state
        .active_session_limiter
        .acquire(entered.account_id, peer_address)
    {
        Ok(permit) => permit,
        Err(limit) => {
            state.world.leave(entered.session_id).await;
            let message = match limit {
                ActiveSessionLimit::Account => {
                    "This account has reached its concurrent session limit."
                }
                ActiveSessionLimit::Address => {
                    "This network address has reached its concurrent session limit."
                }
            };
            reject_before_welcome(&mut socket, "access.session_limit", message).await;
            return;
        }
    };
    // Cancellation, a send error, and a normal close all clean the world
    // session through this guard.
    let _session_lease = SessionLease {
        world: state.world.clone(),
        session: entered.session_id,
        metrics: state.metrics.clone(),
    };
    state.metrics.session_opened();
    let mut deliveries = entered.deliveries;

    let outbound = Arc::new(OutboundHandle::new(
        WS_OUTBOUND_CAPACITY,
        state.metrics.clone(),
    ));
    let (inbound_tx, mut inbound_rx) = mpsc::channel(WS_INBOUND_CAPACITY);
    let driver = tokio::spawn(websocket_driver(socket, outbound.clone(), inbound_tx));

    let mut sequence = 1_u64;
    let mut last_scene = bounded_web_scene(entered.scene);
    let mut last_presentation = bounded_web_presentation(entered.presentation, &last_scene);
    let mut last_player = entered.player;
    let mut ignore_world_through_revision = None;
    if !queue_sequenced(
        &outbound,
        &mut sequence,
        OutboundPriority::Control,
        None,
        ServerMessageType::ServerWelcome,
        protocol::ServerWelcome {
            server_name: "Tiamud".to_owned(),
            motd: "Welcome to Tiamud, a text-complete world of adventures.".to_owned(),
            session_id: entered.session_id.0.to_string(),
        },
    ) || !queue_scene(
        &outbound,
        &mut sequence,
        ServerMessageType::SceneSnapshot,
        &last_scene,
        &last_presentation,
    ) || !queue_player(&outbound, &mut sequence, &last_player)
        || !queue_media_transition(
            &outbound,
            &mut sequence,
            audio_enabled,
            None,
            last_scene.audio.as_ref(),
        )
    {
        finish_websocket_driver(&outbound, driver).await;
        return;
    }
    // A reconnect resumes the account's live story instance, if any.
    let mut story =
        story::WebStorySession::new(state.stories.clone(), entered.account_id, audio_enabled)
            .with_name(&last_player.character.name);
    if !story.resume(&outbound, &mut sequence).await {
        finish_websocket_driver(&outbound, driver).await;
        return;
    }

    let mut protocol_errors = 0_u8;
    let mut rate_violations = 0_u8;
    let mut command_limiter = CommandRateLimiter::per_second(
        state.command_rate_per_second,
        state.command_burst,
        Instant::now(),
    );
    let mut heartbeat = interval(HEARTBEAT_INTERVAL);
    heartbeat.set_missed_tick_behavior(MissedTickBehavior::Skip);
    heartbeat.tick().await;
    let mut heartbeat_nonce = 0_u64;
    let mut pending_heartbeat = None;
    let mut last_heartbeat_ack = Instant::now();

    'session: loop {
        tokio::select! {
            changed = shutdown.changed() => {
                if changed.is_err() || *shutdown.borrow() {
                    let _ = queue_sequenced(
                        &outbound,
                        &mut sequence,
                        OutboundPriority::Critical,
                        None,
                        ServerMessageType::ServerNotice,
                        protocol::ServerNotice::new(
                            protocol::NoticeLevel::Info,
                            "Server shutdown is starting; new traffic is stopping and accepted actions will be drained.",
                        ),
                    );
                    outbound.disconnect("Server shutdown.");
                    break;
                }
            }
            delivery = deliveries.recv() => {
                match delivery {
                    Some(SessionDelivery::World(delivery)) => {
                        if ignore_world_through_revision
                            .is_some_and(|revision| delivery.scene.revision <= revision)
                        {
                            continue;
                        }
                        ignore_world_through_revision = None;
                        let next_scene = bounded_web_scene(delivery.scene);
                        let next_presentation =
                            bounded_web_presentation(delivery.presentation, &next_scene);
                        if !queue_view_events(
                            &outbound,
                            &mut sequence,
                            delivery.events,
                            &last_presentation,
                            &next_presentation,
                        ) {
                            break 'session;
                        }
                        if next_scene.revision <= last_scene.revision {
                            continue;
                        }
                        let previous_scene = last_scene.clone();
                        let previous_player = last_player.clone();
                        last_scene = next_scene;
                        last_presentation = next_presentation;
                        last_player = delivery.player;
                        if !queue_semantic_updates(
                            &outbound,
                            &mut sequence,
                            &previous_scene,
                            &last_scene,
                            &previous_player,
                            &last_player,
                        ) || !queue_media_transition(
                            &outbound,
                            &mut sequence,
                            audio_enabled,
                            previous_scene.audio.as_ref(),
                            last_scene.audio.as_ref(),
                        ) || !queue_scene(
                            &outbound,
                            &mut sequence,
                            ServerMessageType::SceneSnapshot,
                            &last_scene,
                            &last_presentation,
                        ) || !queue_player(&outbound, &mut sequence, &last_player) {
                            break;
                        }
                    }
                    Some(SessionDelivery::WorldLagged) => {
                        match state.world.current_state(entered.session_id).await {
                            Ok((scene, presentation, player)) => {
                                let previous_audio = last_scene.audio.clone();
                                ignore_world_through_revision = Some(scene.revision);
                                last_scene = bounded_web_scene(scene);
                                last_presentation =
                                    bounded_web_presentation(presentation, &last_scene);
                                last_player = player;
                                if !queue_notice(
                                    &outbound,
                                    &mut sequence,
                                    "World updates were coalesced; an authoritative snapshot follows.",
                                ) || !queue_media_transition(
                                    &outbound,
                                    &mut sequence,
                                    audio_enabled,
                                    previous_audio.as_ref(),
                                    last_scene.audio.as_ref(),
                                ) || !queue_scene(
                                    &outbound,
                                    &mut sequence,
                                    ServerMessageType::SceneSnapshot,
                                    &last_scene,
                                    &last_presentation,
                                ) || !queue_player(&outbound, &mut sequence, &last_player)
                                {
                                    break;
                                }
                            }
                            Err(_) => break,
                        }
                    }
                    Some(SessionDelivery::Social(delivery)) => {
                        if !queue_sequenced(
                            &outbound,
                            &mut sequence,
                            OutboundPriority::Narrative,
                            None,
                            ServerMessageType::ChatMessage,
                            chat_message(delivery),
                        ) {
                            break 'session;
                        }
                    }
                    Some(SessionDelivery::SocialLagged) => {
                        if !queue_notice(
                            &outbound,
                            &mut sequence,
                            "Some chat messages were skipped because this client fell behind.",
                        ) {
                            break 'session;
                        }
                    }
                    None => break,
                }
            }
            event = story.next_event() => {
                let was_in_story = story.active();
                if !story.handle_event(&outbound, &mut sequence, event)
                    || (was_in_story
                        && !story.active()
                        && !queue_media_transition(
                            &outbound,
                            &mut sequence,
                            audio_enabled,
                            None,
                            last_scene.audio.as_ref(),
                        ))
                {
                    break;
                }
            }
            inbound = inbound_rx.recv() => {
                let Some(message) = inbound else { break };
                let Message::Text(text) = message else {
                    protocol_errors += 1;
                    if !queue_notice(&outbound, &mut sequence, "Only UTF-8 JSON text messages are accepted.")
                        || protocol_errors >= MAX_PROTOCOL_ERRORS
                    {
                        break;
                    }
                    continue;
                };
                let client_message = match parse_client_message(&text) {
                    Ok(message) => message,
                    Err(error) => {
                        protocol_errors += 1;
                        if !queue_notice(&outbound, &mut sequence, error.public_message())
                            || protocol_errors >= MAX_PROTOCOL_ERRORS
                        {
                            break;
                        }
                        continue;
                    }
                };
                protocol_errors = 0;

                match client_message {
                    ClientMessage::Heartbeat { nonce } => {
                        if pending_heartbeat == Some(nonce) {
                            pending_heartbeat = None;
                            last_heartbeat_ack = Instant::now();
                        }
                    }
                    ClientMessage::Sync { after_seq } => {
                        if !queue_sync_recovery(
                            &outbound,
                            &mut sequence,
                            after_seq,
                            &last_scene,
                            &last_presentation,
                            &last_player,
                        ) || !story.resend(&outbound, &mut sequence).await
                        {
                            break;
                        }
                    }
                    message @ (ClientMessage::StoryInput { .. }
                    | ClientMessage::StoryUndo
                    | ClientMessage::StoryLeave
                    | ClientMessage::StorySave { .. }
                    | ClientMessage::StoryRestore { .. }
                    | ClientMessage::StorySlots
                    | ClientMessage::StoryDeleteSlot { .. }
                    | ClientMessage::StoryVote { .. }
                    | ClientMessage::StoryShare
                    | ClientMessage::StoryJoin { .. }
                    | ClientMessage::StoryInvite { .. }
                    | ClientMessage::StoryKick { .. }
                    | ClientMessage::StoryMode { .. }
                    | ClientMessage::StoryCommand { .. }) => {
                        if !command_limiter.check(Instant::now()) {
                            if !queue_notice(&outbound, &mut sequence, "Command rate limit exceeded.") {
                                break;
                            }
                            rate_violations += 1;
                            if rate_violations >= MAX_PROTOCOL_ERRORS {
                                break;
                            }
                            continue;
                        }
                        rate_violations = 0;
                        let was_in_story = story.active();
                        let delivered = match message {
                            ClientMessage::StoryInput { request, view_rev } => {
                                story.input(&outbound, &mut sequence, request, view_rev).await
                            }
                            ClientMessage::StoryUndo => story.undo(&outbound, &mut sequence).await,
                            ClientMessage::StorySave { slot } => {
                                story.save(&outbound, &mut sequence, slot).await
                            }
                            ClientMessage::StoryRestore { slot } => {
                                story.restore(&outbound, &mut sequence, slot).await
                            }
                            ClientMessage::StorySlots => story.slots(&outbound, &mut sequence).await,
                            ClientMessage::StoryDeleteSlot { slot } => {
                                story.delete_slot(&outbound, &mut sequence, slot).await
                            }
                            ClientMessage::StoryVote { leaf, view_rev } => {
                                story.vote(&outbound, &mut sequence, leaf, view_rev).await
                            }
                            ClientMessage::StoryShare => story.share(&outbound, &mut sequence).await,
                            ClientMessage::StoryJoin { code, spectate } => {
                                story.join(&outbound, &mut sequence, &code, spectate).await
                            }
                            ClientMessage::StoryInvite { name } => {
                                story.invite(&outbound, &mut sequence, &name).await
                            }
                            ClientMessage::StoryKick { name } => {
                                story.kick(&outbound, &mut sequence, &name).await
                            }
                            ClientMessage::StoryMode { mode, vote_seconds } => {
                                story.mode(&outbound, &mut sequence, mode, vote_seconds).await
                            }
                            ClientMessage::StoryCommand { text, view_rev } => {
                                story.command(&outbound, &mut sequence, &text, view_rev).await
                            }
                            _ => story.leave(&outbound, &mut sequence).await,
                        };
                        if !delivered
                            || (was_in_story
                                && !story.active()
                                && !queue_media_transition(
                                    &outbound,
                                    &mut sequence,
                                    audio_enabled,
                                    None,
                                    last_scene.audio.as_ref(),
                                ))
                        {
                            break;
                        }
                    }
                    ClientMessage::Action(requested)
                        if story.active()
                            || requested
                                .text
                                .as_deref()
                                .is_some_and(|text| parse_story_command(text).is_some()) =>
                    {
                        if !command_limiter.check(Instant::now()) {
                            if !queue_notice(&outbound, &mut sequence, "Command rate limit exceeded.") {
                                break;
                            }
                            rate_violations += 1;
                            if rate_violations >= MAX_PROTOCOL_ERRORS {
                                break;
                            }
                            continue;
                        }
                        rate_violations = 0;
                        let was_in_story = story.active();
                        let delivered = match requested.text.as_deref() {
                            Some(text) => story
                                .text_command(&outbound, &mut sequence, &requested.request_id, text)
                                .await
                                .unwrap_or(true),
                            // Structured world actions wait until the story ends.
                            None => queue_action_result(
                                &outbound,
                                &mut sequence,
                                &requested.request_id,
                                false,
                                Some(STORY_INPUT_HELP.to_owned()),
                            ),
                        };
                        if !delivered
                            || (was_in_story
                                && !story.active()
                                && !queue_media_transition(
                                    &outbound,
                                    &mut sequence,
                                    audio_enabled,
                                    None,
                                    last_scene.audio.as_ref(),
                                ))
                        {
                            break;
                        }
                    }
                    ClientMessage::Action(requested) => {
                        let replay_key = action_replay_key(character_id, &requested.request_id);
                        let cached = state
                            .action_replay
                            .lock()
                            .unwrap_or_else(|error| error.into_inner())
                            .get(&replay_key)
                            .cloned();
                        if let Some(cached) = cached {
                            if cached.signature != requested.signature {
                                protocol_errors += 1;
                                if !queue_notice(
                                    &outbound,
                                    &mut sequence,
                                    "A requestId cannot be reused for a different action.",
                                ) || protocol_errors >= MAX_PROTOCOL_ERRORS
                                {
                                    break;
                                }
                                continue;
                            }
                            if !queue_action_result(
                                &outbound,
                                &mut sequence,
                                &requested.request_id,
                                cached.ok,
                                cached.message_for(entered.session_id),
                            ) || cached.container_contents_for(entered.session_id).is_some_and(|contents| {
                                !queue_container_contents(&outbound, &mut sequence, contents)
                            }) || cached.trade_contents_for(entered.session_id).is_some_and(|contents| {
                                !queue_trade_contents(&outbound, &mut sequence, contents)
                            }) || cached.reward_odds_for(entered.session_id).is_some_and(|odds| {
                                !queue_reward_odds(&outbound, &mut sequence, odds)
                            }) || cached.mail_page_for(entered.session_id).is_some_and(|page| {
                                !queue_mail_page(&outbound, &mut sequence, page)
                            }) || cached.mail_message_for(entered.session_id).is_some_and(|message| {
                                !queue_mail_message(&outbound, &mut sequence, message)
                            }) || cached.board_catalog_for(entered.session_id).is_some_and(|catalog| {
                                !queue_board_catalog(&outbound, &mut sequence, catalog)
                            }) || cached.board_page_for(entered.session_id).is_some_and(|page| {
                                !queue_board_page(&outbound, &mut sequence, page)
                            }) || cached.board_entry_for(entered.session_id).is_some_and(|entry| {
                                !queue_board_entry(&outbound, &mut sequence, entry)
                            }) || !queue_scene(
                                &outbound,
                                &mut sequence,
                                ServerMessageType::SceneSnapshot,
                                &last_scene,
                                &last_presentation,
                            ) || !queue_player(
                                &outbound,
                                &mut sequence,
                                &last_player,
                            ) {
                                break;
                            }
                            continue;
                        }

                        if !command_limiter.check(Instant::now()) {
                            if !queue_notice(&outbound, &mut sequence, "Command rate limit exceeded.") {
                                break;
                            }
                            rate_violations += 1;
                            if rate_violations >= MAX_PROTOCOL_ERRORS {
                                break;
                            }
                            continue;
                        }
                        rate_violations = 0;

                        let action_started = Instant::now();
                        let previous_scene = last_scene.clone();
                        let previous_presentation = last_presentation.clone();
                        let previous_player = last_player.clone();
                        let trade_selector_matches = requested.trade_selector.as_ref().is_none_or(
                            |selector| last_player.trade.as_ref().is_some_and(|trade| &trade.id == selector),
                        );
                        let (
                            ok,
                            message,
                            events,
                            container_contents,
                            trade_contents,
                            reward_odds,
                            mail_page,
                            mail_message,
                            board_catalog,
                            board_page,
                            board_entry,
                        ) = match requested.action {
                            Ok(_) if !trade_selector_matches => {
                                state.metrics.action_finished(action_started.elapsed(), false);
                                (
                                    false,
                                    Some("That trade is not available.".to_owned()),
                                    Vec::new(),
                                    None,
                                    None,
                                    None,
                                    None,
                                    None,
                                    None,
                                    None,
                                    None,
                                )
                            }
                            Ok(action) => match state.world.apply(entered.session_id, action).await {
                                Ok(outcome) => {
                                    state.metrics.action_finished(action_started.elapsed(), true);
                                    last_scene = bounded_web_scene(outcome.scene);
                                    last_presentation = bounded_web_presentation(
                                        outcome.presentation,
                                        &last_scene,
                                    );
                                    last_player = outcome.player;
                                    (
                                        true,
                                        outcome.output,
                                        outcome.events,
                                        outcome.container_contents,
                                        outcome.trade_contents,
                                        outcome.reward_odds,
                                        outcome.mail_page,
                                        outcome.mail_message,
                                        outcome.board_catalog,
                                        outcome.board_page,
                                        outcome.board_entry,
                                    )
                                }
                                Err(error) => {
                                    state.metrics.action_finished(action_started.elapsed(), false);
                                    warn!(%error, "game action failed");
                                    (
                                        false,
                                        Some(public_game_error(&error)),
                                        Vec::new(),
                                        None,
                                        None,
                                        None,
                                        None,
                                        None,
                                        None,
                                        None,
                                        None,
                                    )
                                }
                            },
                            Err(error) => {
                                state.metrics.action_finished(action_started.elapsed(), false);
                                (
                                    false,
                                    Some(error),
                                    Vec::new(),
                                    None,
                                    None,
                                    None,
                                    None,
                                    None,
                                    None,
                                    None,
                                    None,
                                )
                            }
                        };
                        state
                            .action_replay
                            .lock()
                            .unwrap_or_else(|error| error.into_inner())
                            .insert(
                                replay_key,
                                CachedActionResult {
                                    signature: requested.signature,
                                    ok,
                                    message: message.clone(),
                                    container_contents: container_contents.clone(),
                                    trade_contents: trade_contents.clone(),
                                    reward_odds: reward_odds.clone(),
                                    mail_page: mail_page.clone(),
                                    mail_message: mail_message.clone(),
                                    board_catalog: board_catalog.clone(),
                                    board_page: board_page.clone(),
                                    board_entry: board_entry.clone(),
                                    projection_session: entered.session_id,
                                },
                            );
                        if !queue_action_result(
                            &outbound,
                            &mut sequence,
                            &requested.request_id,
                            ok,
                            message,
                        ) {
                            break;
                        }
                        if let Some(contents) = &container_contents
                            && !queue_container_contents(&outbound, &mut sequence, contents)
                        {
                            break;
                        }
                        if let Some(contents) = &trade_contents
                            && !queue_trade_contents(&outbound, &mut sequence, contents)
                        {
                            break;
                        }
                        if let Some(odds) = &reward_odds
                            && !queue_reward_odds(&outbound, &mut sequence, odds)
                        {
                            break;
                        }
                        if let Some(page) = &mail_page
                            && !queue_mail_page(&outbound, &mut sequence, page)
                        {
                            break;
                        }
                        if let Some(message) = &mail_message
                            && !queue_mail_message(&outbound, &mut sequence, message)
                        {
                            break;
                        }
                        if let Some(catalog) = &board_catalog
                            && !queue_board_catalog(&outbound, &mut sequence, catalog)
                        {
                            break;
                        }
                        if let Some(page) = &board_page
                            && !queue_board_page(&outbound, &mut sequence, page)
                        {
                            break;
                        }
                        if let Some(entry) = &board_entry
                            && !queue_board_entry(&outbound, &mut sequence, entry)
                        {
                            break;
                        }
                        if !queue_view_events(
                            &outbound,
                            &mut sequence,
                            events,
                            &previous_presentation,
                            &last_presentation,
                        ) {
                            break 'session;
                        }
                        if ok && !queue_semantic_updates(
                            &outbound,
                            &mut sequence,
                            &previous_scene,
                            &last_scene,
                            &previous_player,
                            &last_player,
                        ) {
                            break;
                        }
                        if ok && !queue_media_transition(
                            &outbound,
                            &mut sequence,
                            audio_enabled,
                            previous_scene.audio.as_ref(),
                            last_scene.audio.as_ref(),
                        ) {
                            break;
                        }
                        if ok && (!queue_scene(
                            &outbound,
                            &mut sequence,
                            ServerMessageType::SceneSnapshot,
                            &last_scene,
                            &last_presentation,
                        ) || !queue_player(&outbound, &mut sequence, &last_player)) {
                            break;
                        }
                    }
                }
            }
            _ = heartbeat.tick() => {
                if Instant::now().saturating_duration_since(last_heartbeat_ack) >= HEARTBEAT_TIMEOUT {
                    outbound.disconnect("Heartbeat timeout: the client stopped responding.");
                    break;
                }
                heartbeat_nonce = heartbeat_nonce.wrapping_add(1).max(1);
                pending_heartbeat = Some(heartbeat_nonce);
                let Some(value) = heartbeat_frame(heartbeat_nonce) else {
                    outbound.disconnect("Server output could not be encoded.");
                    break;
                };
                let result = outbound.enqueue(OutboundEnvelope {
                    priority: OutboundPriority::BestEffort,
                    coalesce: Some(CoalesceKey::Heartbeat),
                    sequence: None,
                    value,
                });
                if result == EnqueueResult::Disconnect {
                    break;
                }
            }
        }
    }

    finish_websocket_driver(&outbound, driver).await;
}

fn heartbeat_frame(nonce: u64) -> Option<Value> {
    protocol_value(protocol::UnsequencedEnvelope::new(
        ServerMessageType::ServerHeartbeat,
        protocol::ServerHeartbeat {
            nonce,
            timeout_ms: u64::try_from(HEARTBEAT_TIMEOUT.as_millis()).unwrap_or(u64::MAX),
        },
    ))
}

/// Converts the transport-neutral social delivery into its v1 wire payload.
fn chat_message(delivery: crate::social::SocialMessage) -> protocol::ChatMessage {
    use crate::social::SocialChannel;
    let channel = match delivery.channel {
        SocialChannel::Announcement => protocol::ChatChannel::Announcement,
        SocialChannel::Say => protocol::ChatChannel::Say,
        SocialChannel::Yell => protocol::ChatChannel::Yell,
        SocialChannel::Shout => protocol::ChatChannel::Shout,
        SocialChannel::Tell => protocol::ChatChannel::Tell,
        SocialChannel::Telepathy => protocol::ChatChannel::Telepathy,
        SocialChannel::Emote => protocol::ChatChannel::Emote,
        SocialChannel::Party => protocol::ChatChannel::Party,
        SocialChannel::General => protocol::ChatChannel::General,
        SocialChannel::Newbie => protocol::ChatChannel::Newbie,
        SocialChannel::Staff => protocol::ChatChannel::Staff,
    };
    protocol::ChatMessage {
        channel,
        sender: delivery.sender,
        text: delivery.text,
    }
}

const fn protocol_board_key(board: BoardKey) -> protocol::BoardKey {
    match board {
        BoardKey::General => protocol::BoardKey::General,
        BoardKey::Newbie => protocol::BoardKey::Newbie,
    }
}

async fn send_json(socket: &mut WebSocket, value: Value) -> Result<(), axum::Error> {
    socket.send(Message::Text(value.to_string().into())).await
}

async fn websocket_driver(
    mut socket: WebSocket,
    outbound: Arc<OutboundHandle>,
    inbound: mpsc::Sender<Message>,
) {
    loop {
        if let Some(frame) = outbound.pop() {
            let message = Message::Text(frame.value.to_string().into());
            if !matches!(
                timeout(WS_WRITE_TIMEOUT, socket.send(message)).await,
                Ok(Ok(()))
            ) {
                let close = Message::Close(Some(CloseFrame {
                    code: 1013,
                    reason: "Client too slow for critical output.".into(),
                }));
                let _ = timeout(WS_WRITE_TIMEOUT, socket.send(close)).await;
                break;
            }
            continue;
        }
        if let Some(reason) = outbound.take_disconnect_reason() {
            let close = Message::Close(Some(CloseFrame {
                code: 1013,
                reason: reason.into(),
            }));
            let _ = timeout(WS_WRITE_TIMEOUT, socket.send(close)).await;
            break;
        }

        tokio::select! {
            message = socket.recv() => {
                match message {
                    Some(Ok(Message::Ping(payload))) => {
                        if !matches!(
                            timeout(WS_WRITE_TIMEOUT, socket.send(Message::Pong(payload))).await,
                            Ok(Ok(()))
                        ) {
                            break;
                        }
                    }
                    Some(Ok(Message::Pong(_))) => {}
                    Some(Ok(Message::Close(frame))) => {
                        // Complete the WebSocket closing handshake instead of
                        // dropping the transport while the peer is waiting for
                        // our close response. Some strict clients surface an
                        // abrupt drop as a transport error during reconnect.
                        let _ = timeout(
                            WS_WRITE_TIMEOUT,
                            socket.send(Message::Close(frame)),
                        )
                        .await;
                        break;
                    }
                    Some(Err(_)) | None => break,
                    Some(Ok(message)) => {
                        if inbound.try_send(message).is_err() {
                            let close = Message::Close(Some(CloseFrame {
                                code: 1013,
                                reason: "Inbound client queue is saturated.".into(),
                            }));
                            let _ = timeout(WS_WRITE_TIMEOUT, socket.send(close)).await;
                            break;
                        }
                    }
                }
            }
            _ = outbound.changed.notified() => {}
        }
    }
}

async fn finish_websocket_driver(outbound: &OutboundHandle, driver: JoinHandle<()>) {
    outbound.disconnect("Session closed.");
    let _ = timeout(WS_WRITE_TIMEOUT, driver).await;
}

/// Encodes a typed `tiamud.v1` server message as the canonical JSON value.
///
/// Conversion through `serde_json::Value` keeps the exact key order and bytes
/// of the former ad hoc construction (`serde_json::Map` sorts keys).
fn protocol_value(message: impl Serialize) -> Option<Value> {
    serde_json::to_value(message)
        .inspect_err(|error| warn!(%error, "could not encode a typed protocol message"))
        .ok()
}

fn sequenced_frame(
    message_type: ServerMessageType,
    seq: u64,
    data: impl Serialize,
) -> Option<Value> {
    protocol_value(protocol::ServerEnvelope::new(message_type, seq, data))
}

/// Byte length of one compact sequenced frame, or `usize::MAX` if it cannot
/// be encoded. Used by the 16 KiB budget fallbacks before queueing.
fn sequenced_frame_bytes(message_type: ServerMessageType, seq: u64, data: impl Serialize) -> usize {
    serde_json::to_vec(&protocol::ServerEnvelope::new(message_type, seq, data))
        .map_or(usize::MAX, |bytes| bytes.len())
}

fn queue_sequenced(
    outbound: &OutboundHandle,
    sequence: &mut u64,
    priority: OutboundPriority,
    coalesce: Option<CoalesceKey>,
    message_type: ServerMessageType,
    data: impl Serialize,
) -> bool {
    let current = *sequence;
    *sequence = sequence.wrapping_add(1).max(1);
    let Some(value) = sequenced_frame(message_type, current, data) else {
        // Fail closed: never emit a partial or substitute frame.
        outbound.disconnect("Server output could not be encoded.");
        return false;
    };
    outbound.enqueue(OutboundEnvelope {
        priority,
        coalesce,
        sequence: Some(current),
        value,
    }) != EnqueueResult::Disconnect
}

/// A sequenced error notice sent directly on the socket before `server.welcome`.
fn pre_welcome_rejection(code: &str, message: &str) -> Option<Value> {
    sequenced_frame(
        ServerMessageType::ServerNotice,
        1,
        protocol::ServerNotice::with_code(protocol::NoticeLevel::Error, code, message),
    )
}

/// Sends a pre-welcome rejection and closes the socket.
async fn reject_before_welcome(socket: &mut WebSocket, code: &str, message: &str) {
    if let Some(frame) = pre_welcome_rejection(code, message) {
        let _ = send_json(socket, frame).await;
    }
    let _ = socket.send(Message::Close(None)).await;
}

fn queue_scene(
    outbound: &OutboundHandle,
    sequence: &mut u64,
    message_type: ServerMessageType,
    scene: &SceneView,
    presentation: &crate::runtime_content::ScenePresentation,
) -> bool {
    let mut presentation = presentation.clone();
    loop {
        let data = scene_with_presentation(scene, &presentation);
        if sequenced_frame_bytes(message_type, *sequence, &data) <= MAX_WS_MESSAGE_BYTES
            || presentation.entities.is_empty()
        {
            return queue_sequenced(
                outbound,
                sequence,
                OutboundPriority::Critical,
                Some(CoalesceKey::Scene),
                message_type,
                data,
            );
        }
        // Definitions are priority-descending; shed the least important
        // optional annotation before risking the semantic scene frame.
        presentation.entities.pop();
    }
}

fn bounded_web_scene(mut scene: SceneView) -> SceneView {
    let fits = |scene: &SceneView| {
        serde_json::to_vec(scene).is_ok_and(|bytes| bytes.len() <= MAX_WS_SCENE_SEMANTIC_BYTES)
    };
    if fits(&scene) {
        return scene;
    }

    // When a room is crowded, keep the actionable window deterministic and
    // useful: own reservations first, then public items, then reservations of
    // others. Removing/taking an item naturally reveals the next stack on the
    // following authoritative projection.
    scene.items.sort_by(|left, right| {
        let priority =
            |item: &tiamud_core::SceneItem| match item.loot.as_ref().map(|loot| loot.access) {
                Some(tiamud_core::LootAccessView::Yours) => 0_u8,
                None => 1,
                Some(tiamud_core::LootAccessView::Reserved) => 2,
            };
        priority(left)
            .cmp(&priority(right))
            .then_with(|| left.label.cmp(&right.label))
            .then_with(|| left.id.cmp(&right.id))
    });
    let original_prose = scene.prose.clone();
    let original_items = scene.items.len();
    let original_npcs = scene.npcs.len();
    let mut omitted_items = 0_usize;
    let mut omitted_npcs = 0_usize;
    while !fits(&scene) && !scene.items.is_empty() {
        scene.items.pop();
        omitted_items = original_items.saturating_sub(scene.items.len());
        set_scene_projection_notice(&mut scene, omitted_items, omitted_npcs, false);
    }
    while !fits(&scene) && !scene.npcs.is_empty() {
        scene.npcs.pop();
        omitted_npcs = original_npcs.saturating_sub(scene.npcs.len());
        set_scene_projection_notice(&mut scene, omitted_items, omitted_npcs, false);
    }
    if !fits(&scene) {
        let prose = original_prose.join("\n\n");
        for maximum in [4_096, 2_048, 1_024, 512, 256, 0] {
            scene.prose = if maximum == 0 {
                Vec::new()
            } else {
                vec![bounded_utf8_prefix(&prose, maximum)]
            };
            set_scene_projection_notice(&mut scene, omitted_items, omitted_npcs, true);
            if fits(&scene) {
                break;
            }
        }
    }
    scene
}

fn set_scene_projection_notice(
    scene: &mut SceneView,
    omitted_items: usize,
    omitted_npcs: usize,
    prose_shortened: bool,
) {
    scene
        .prose
        .retain(|paragraph| !paragraph.starts_with("Bounded scene notice:"));
    let mut details = Vec::new();
    if omitted_items > 0 {
        details.push(format!(
            "{omitted_items} additional item stack(s); interact with visible items or use loot status to reveal the next entries"
        ));
    }
    if omitted_npcs > 0 {
        details.push(format!("{omitted_npcs} additional creature(s)"));
    }
    if prose_shortened {
        details.push("the room description was shortened for this transport".into());
    }
    scene
        .prose
        .push(format!("Bounded scene notice: {}.", details.join("; ")));
}

fn bounded_utf8_prefix(value: &str, maximum_bytes: usize) -> String {
    let mut end = 0;
    for (index, character) in value.char_indices() {
        let next = index + character.len_utf8();
        if next > maximum_bytes {
            break;
        }
        end = next;
    }
    value[..end].to_owned()
}

fn bounded_web_presentation(
    mut presentation: crate::runtime_content::ScenePresentation,
    scene: &SceneView,
) -> crate::runtime_content::ScenePresentation {
    let visible = scene
        .items
        .iter()
        .map(|item| item.id.as_str())
        .chain(scene.npcs.iter().map(|npc| npc.id.as_str()))
        .collect::<HashSet<_>>();
    presentation.retain_entities(|entity_id| visible.contains(entity_id));
    presentation
}

fn queue_player(outbound: &OutboundHandle, sequence: &mut u64, player: &PlayerStateView) -> bool {
    let Some(data) = bounded_player_projection(player, |data| {
        sequenced_frame_bytes(ServerMessageType::PlayerSnapshot, *sequence, data)
            <= MAX_WS_MESSAGE_BYTES
    }) else {
        outbound.disconnect("Server output could not be encoded.");
        return false;
    };
    queue_sequenced(
        outbound,
        sequence,
        OutboundPriority::State,
        Some(CoalesceKey::Player),
        ServerMessageType::PlayerSnapshot,
        data,
    )
}

/// Keeps the semantic and actionable trade projection intact while shedding
/// only bounded display labels if a maximal 16-line offer would otherwise
/// push a player frame over the protocol budget. If even the shortest safe
/// labels do not fit, the unchanged outbound budget guard disconnects rather
/// than emitting a partial or oversized state.
fn bounded_player_projection(
    player: &PlayerStateView,
    fits: impl Fn(&Value) -> bool,
) -> Option<Value> {
    let mut projection = protocol_value(player)?;
    if fits(&projection) {
        return Some(projection);
    }
    for maximum in TRADE_LABEL_FALLBACK_BYTES {
        truncate_trade_offer_labels(&mut projection, maximum);
        if fits(&projection) {
            return Some(projection);
        }
    }
    Some(projection)
}

fn truncate_trade_offer_labels(player: &mut Value, maximum_bytes: usize) {
    let Some(trade) = player.get_mut("trade").and_then(Value::as_object_mut) else {
        return;
    };
    for offer_name in ["ownOffer", "counterpartyOffer"] {
        let Some(lines) = trade
            .get_mut(offer_name)
            .and_then(Value::as_object_mut)
            .and_then(|offer| offer.get_mut("lines"))
            .and_then(Value::as_array_mut)
        else {
            continue;
        };
        for line in lines {
            let Some(label) = line
                .as_object()
                .and_then(|line| line.get("label"))
                .and_then(Value::as_str)
            else {
                continue;
            };
            if label.len() <= maximum_bytes {
                continue;
            }
            let mut end = 0;
            for (index, character) in label.char_indices() {
                let next = index + character.len_utf8();
                if next > maximum_bytes {
                    break;
                }
                end = next;
            }
            let shortened = label[..end].to_owned();
            if let Some(object) = line.as_object_mut() {
                object.insert("label".into(), Value::String(shortened));
            }
        }
    }
}

/// Emits gap recovery as two independently budgeted, correlated frames.
///
/// A single combined scene/player snapshot can exceed the WebSocket protocol's
/// 16 KiB frame budget even when both projections are independently valid. The
/// stable sync ID lets clients discard incomplete/coalesced older pairs without
/// treating either recovery frame's sequence jump as another gap.
fn queue_sync_recovery(
    outbound: &OutboundHandle,
    sequence: &mut u64,
    after_seq: u64,
    scene: &SceneView,
    presentation: &crate::runtime_content::ScenePresentation,
    player: &PlayerStateView,
) -> bool {
    let sync_id = format!("sync-{}", *sequence);
    let mut presentation = presentation.clone();
    let scene_queued = loop {
        let data = protocol::SyncScene {
            sync_id: sync_id.clone(),
            after_seq,
            scene: scene_with_presentation(scene, &presentation),
        };
        if sequenced_frame_bytes(ServerMessageType::SyncScene, *sequence, &data)
            <= MAX_WS_MESSAGE_BYTES
            || presentation.entities.is_empty()
        {
            break queue_sequenced(
                outbound,
                sequence,
                OutboundPriority::Critical,
                Some(CoalesceKey::SyncScene),
                ServerMessageType::SyncScene,
                data,
            );
        }
        presentation.entities.pop();
    };
    let Some(player) = bounded_player_projection(player, |player| {
        sequenced_frame_bytes(
            ServerMessageType::SyncPlayer,
            *sequence,
            protocol::SyncPlayer::new(sync_id.as_str(), after_seq, player),
        ) <= MAX_WS_MESSAGE_BYTES
    }) else {
        outbound.disconnect("Server output could not be encoded.");
        return false;
    };
    scene_queued
        && queue_sequenced(
            outbound,
            sequence,
            OutboundPriority::Critical,
            Some(CoalesceKey::SyncPlayer),
            ServerMessageType::SyncPlayer,
            protocol::SyncPlayer::new(sync_id, after_seq, player),
        )
}

/// `scene.snapshot`/`sync.scene` scene payload: the projection with its
/// optional presentation annotation merged into the same object.
fn scene_with_presentation<'a>(
    scene: &'a SceneView,
    presentation: &'a crate::runtime_content::ScenePresentation,
) -> protocol::SceneFrame<&'a SceneView, &'a crate::runtime_content::ScenePresentation> {
    protocol::SceneFrame {
        scene,
        presentation: (!presentation.is_empty()).then_some(presentation),
    }
}

fn queue_media_transition(
    outbound: &OutboundHandle,
    sequence: &mut u64,
    enabled: bool,
    previous: Option<&tiamud_core::SceneAudio>,
    next: Option<&tiamud_core::SceneAudio>,
) -> bool {
    if !enabled || previous == next {
        return true;
    }
    if let Some(previous) = previous
        && let Some(channel) = safe_audio_channel(&previous.channel)
        && !queue_sequenced(
            outbound,
            sequence,
            OutboundPriority::Critical,
            None,
            ServerMessageType::MediaStop,
            protocol::MediaStop { channel },
        )
    {
        return false;
    }
    next.is_none_or(|next| {
        let Some(channel) = safe_scene_audio(next) else {
            return true;
        };
        queue_sequenced(
            outbound,
            sequence,
            OutboundPriority::State,
            None,
            ServerMessageType::MediaPlay,
            protocol::MediaPlay {
                channel,
                url: next.url.clone(),
                looped: next.looped,
            },
        )
    })
}

/// The closed media channel of a safe, content-addressed scene audio track.
fn safe_scene_audio(audio: &tiamud_core::SceneAudio) -> Option<protocol::MediaChannel> {
    safe_audio_channel(&audio.channel).filter(|_| safe_content_addressed_audio_url(&audio.url))
}

fn safe_audio_channel(channel: &str) -> Option<protocol::MediaChannel> {
    protocol::MediaChannel::from_name(channel)
}

pub(crate) fn safe_content_addressed_audio_url(value: &str) -> bool {
    if value.len() > 161 || value.contains(['?', '#', '\\']) {
        return false;
    }
    let Some(path) = value.strip_prefix("/assets/") else {
        return false;
    };
    let Some((digest, file)) = path.split_once('/') else {
        return false;
    };
    digest.len() == 64
        && digest
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
        && !file.contains('/')
        && [".opus", ".ogg", ".mp3"].iter().any(|extension| {
            file.strip_prefix("audio-")
                .and_then(|name| name.strip_suffix(extension))
                .is_some_and(|name| !name.is_empty())
        })
        && file.bytes().all(|byte| {
            byte.is_ascii_lowercase() || byte.is_ascii_digit() || matches!(byte, b'-' | b'.')
        })
}

fn queue_semantic_updates(
    outbound: &OutboundHandle,
    sequence: &mut u64,
    previous_scene: &SceneView,
    scene: &SceneView,
    previous_player: &PlayerStateView,
    player: &PlayerStateView,
) -> bool {
    let (Some(previous_scene_json), Some(scene_json)) =
        (protocol_value(previous_scene), protocol_value(scene))
    else {
        return false;
    };
    let Some(previous_scene_object) = previous_scene_json.as_object() else {
        return false;
    };
    let Some(scene_object) = scene_json.as_object() else {
        return false;
    };
    let previous_entities = scene_entities(previous_scene_object);
    let entities = scene_entities(scene_object);
    let entity_delta_count = previous_entities
        .keys()
        .filter(|key| !entities.contains_key(*key))
        .count()
        + entities
            .iter()
            .filter(|(key, entity)| {
                previous_entities
                    .get(*key)
                    .is_none_or(|previous| previous != *entity)
            })
            .count();
    let emit_scene_deltas = entity_delta_count <= MAX_WS_SEMANTIC_DELTA_FRAMES;

    // Change detection compares the encoded projections, exactly as the wire
    // observes them; the patch then carries the typed changed fields.
    let changed = |field: &str| previous_scene_object.get(field) != scene_object.get(field);
    let mut patch = protocol::ScenePatch::new(scene.view_id.as_str(), scene.revision);
    if changed("title") {
        patch.title = Some(scene.title.clone());
    }
    if changed("prose") {
        patch.prose = Some(scene.prose.clone());
    }
    if changed("image") {
        patch.image = Some(scene.image.as_ref());
    }
    if changed("exits") {
        patch.exits = Some(scene.exits.iter().collect::<Vec<_>>());
    }
    if changed("weather") {
        patch.weather = Some(&scene.weather);
    }
    if emit_scene_deltas
        && patch.has_changes()
        && !queue_sequenced(
            outbound,
            sequence,
            OutboundPriority::State,
            Some(CoalesceKey::ScenePatch),
            ServerMessageType::ScenePatch,
            patch,
        )
    {
        return false;
    }

    for ((kind, id), _) in previous_entities
        .iter()
        .filter(|(key, _)| emit_scene_deltas && !entities.contains_key(*key))
    {
        if !queue_sequenced(
            outbound,
            sequence,
            OutboundPriority::State,
            None,
            ServerMessageType::EntityRemove,
            protocol::EntityRemove {
                view_id: scene.view_id.clone(),
                revision: scene.revision,
                kind: *kind,
                id: id.clone(),
            },
        ) {
            return false;
        }
    }
    for ((kind, id), entity) in entities.iter().filter(|(key, entity)| {
        emit_scene_deltas
            && previous_entities
                .get(*key)
                .is_none_or(|previous| previous != *entity)
    }) {
        if !queue_sequenced(
            outbound,
            sequence,
            OutboundPriority::State,
            None,
            ServerMessageType::EntityUpsert,
            protocol::EntityUpsert {
                view_id: scene.view_id.clone(),
                revision: scene.revision,
                kind: *kind,
                id: id.clone(),
                entity,
            },
        ) {
            return false;
        }
    }

    if previous_player.character != player.character
        && !queue_sequenced(
            outbound,
            sequence,
            OutboundPriority::State,
            Some(CoalesceKey::Vitals),
            ServerMessageType::VitalsUpdate,
            protocol::VitalsUpdate {
                revision: player.revision,
                character: &player.character,
            },
        )
    {
        return false;
    }
    if (previous_player.inventory != player.inventory
        || previous_player.equipment != player.equipment
        || previous_player.inventory_capacity != player.inventory_capacity)
        && !queue_sequenced(
            outbound,
            sequence,
            OutboundPriority::State,
            Some(CoalesceKey::Inventory),
            ServerMessageType::InventoryPatch,
            protocol::InventoryPatch {
                revision: player.revision,
                inventory: player.inventory.iter().collect(),
                equipment: player.equipment.iter().collect(),
                inventory_capacity: &player.inventory_capacity,
            },
        )
    {
        return false;
    }
    if previous_player.map != player.map
        && !queue_sequenced(
            outbound,
            sequence,
            OutboundPriority::State,
            Some(CoalesceKey::Map),
            ServerMessageType::MapSnapshot,
            protocol::MapSnapshot {
                revision: player.revision,
                map: &player.map,
            },
        )
    {
        return false;
    }
    let other_player_state_changed = previous_player.quests != player.quests
        || previous_player.party != player.party
        || previous_player.party_invites != player.party_invites
        || previous_player.economy != player.economy
        || previous_player.social != player.social
        || previous_player.trade != player.trade;
    !other_player_state_changed || queue_player(outbound, sequence, player)
}

fn scene_entities(scene: &Map<String, Value>) -> BTreeMap<(protocol::EntityKind, String), Value> {
    let mut entities = BTreeMap::new();
    for (kind, field) in [
        (protocol::EntityKind::Item, "items"),
        (protocol::EntityKind::Npc, "npcs"),
    ] {
        let Some(values) = scene.get(field).and_then(Value::as_array) else {
            continue;
        };
        for value in values {
            if let Some(id) = value.get("id").and_then(Value::as_str) {
                entities.insert((kind, id.to_owned()), value.clone());
            }
        }
    }
    entities
}

fn queue_action_result(
    outbound: &OutboundHandle,
    sequence: &mut u64,
    request_id: &str,
    ok: bool,
    message: Option<String>,
) -> bool {
    queue_sequenced(
        outbound,
        sequence,
        OutboundPriority::Critical,
        None,
        ServerMessageType::ActionResult,
        protocol::ActionResult {
            request_id: request_id.to_owned(),
            ok,
            message,
        },
    )
}

fn queue_container_contents(
    outbound: &OutboundHandle,
    sequence: &mut u64,
    contents: &ContainerContentsView,
) -> bool {
    // A container page is an actionable summary. Descriptions remain available
    // through `examine`, avoiding twenty unbounded prose fields in one 16 KiB
    // WebSocket frame while preserving every selector and capacity fact.
    let items = contents
        .items
        .iter()
        .map(item_summary_without_description)
        .collect::<Option<Vec<_>>>();
    let Some(items) = items else {
        outbound.disconnect("Server output could not be encoded.");
        return false;
    };
    queue_sequenced(
        outbound,
        sequence,
        OutboundPriority::Critical,
        None,
        ServerMessageType::ContainerContents,
        protocol::ContainerContents {
            container: contents.selector.clone(),
            items,
            page: contents.page,
            total_pages: contents.total_pages,
            capacity: &contents.capacity,
        },
    )
}

fn queue_trade_contents(
    outbound: &OutboundHandle,
    sequence: &mut u64,
    contents: &TradeContentsView,
) -> bool {
    // Offered-container inspection is private to the two participants. Keep
    // the independently paged frame below the ordinary 16 KiB budget by
    // omitting prose exactly like `container.contents`; selectors, quantities,
    // reservation state, policy and capacity remain complete and actionable.
    let items = contents
        .items
        .iter()
        .map(item_summary_without_description)
        .collect::<Option<Vec<_>>>();
    let Some(items) = items else {
        outbound.disconnect("Server output could not be encoded.");
        return false;
    };
    queue_sequenced(
        outbound,
        sequence,
        OutboundPriority::Critical,
        None,
        ServerMessageType::TradeContents,
        protocol::TradeContents {
            trade_id: contents.trade_id.clone(),
            container: contents.container.clone(),
            items,
            page: contents.page,
            total_pages: contents.total_pages,
            capacity: &contents.capacity,
        },
    )
}

/// A paged item summary: the core item projection without its prose.
fn item_summary_without_description(item: &tiamud_core::PlayerItemView) -> Option<Value> {
    let mut summary = protocol_value(item)?;
    if let Some(object) = summary.as_object_mut() {
        object.remove("description");
    }
    Some(summary)
}

fn queue_reward_odds(outbound: &OutboundHandle, sequence: &mut u64, odds: &RewardOddsView) -> bool {
    // Core pages this private projection before it reaches the transport. Keep
    // exact integer probability fractions and every actionable selector in a
    // dedicated critical frame; the ordinary 16 KiB gate remains authoritative.
    queue_sequenced(
        outbound,
        sequence,
        OutboundPriority::Critical,
        None,
        ServerMessageType::RewardOdds,
        odds,
    )
}

fn mail_summary(summary: &crate::persistence::MailMessageSummary) -> protocol::MailMessageSummary {
    protocol::MailMessageSummary {
        id: summary.public_id.clone(),
        sender: summary.sender_name.clone(),
        subject: summary.subject.clone(),
        created_at: summary.created_at,
        expires_at: summary.expires_at,
        read: summary.read,
    }
}

fn queue_mail_page(outbound: &OutboundHandle, sequence: &mut u64, page: &MailPage) -> bool {
    let messages = page.messages.iter().map(mail_summary).collect::<Vec<_>>();
    queue_sequenced(
        outbound,
        sequence,
        OutboundPriority::Critical,
        None,
        ServerMessageType::MailList,
        protocol::MailList {
            page: page.page,
            total_pages: page.total_pages,
            total_messages: page.total_messages,
            unread_count: page.unread_count,
            capacity: page.capacity,
            messages,
        },
    )
}

fn queue_mail_message(
    outbound: &OutboundHandle,
    sequence: &mut u64,
    message: &MailMessage,
) -> bool {
    let summary = &message.summary;
    queue_sequenced(
        outbound,
        sequence,
        OutboundPriority::Critical,
        None,
        ServerMessageType::MailMessage,
        protocol::MailMessage {
            id: summary.public_id.clone(),
            sender: summary.sender_name.clone(),
            subject: summary.subject.clone(),
            body: message.body.clone(),
            created_at: summary.created_at,
            expires_at: summary.expires_at,
            read: summary.read,
        },
    )
}

fn board_summary(summary: &crate::persistence::BoardPostSummary) -> protocol::BoardPostSummary {
    protocol::BoardPostSummary {
        post_id: summary.public_id.clone(),
        board: protocol_board_key(summary.board),
        author: summary.author_name.clone(),
        subject: summary.subject.clone(),
        created_at: summary.created_at,
        expires_at: summary.expires_at,
        can_delete: summary.can_delete,
        can_report: summary.can_report,
    }
}

fn queue_board_catalog(
    outbound: &OutboundHandle,
    sequence: &mut u64,
    catalog: &[BoardDescriptor],
) -> bool {
    let boards = catalog
        .iter()
        .map(|descriptor| {
            let (label, description) = match descriptor.board {
                BoardKey::General => ("General", "Conversación comunitaria deliberada."),
                BoardKey::Newbie => ("Personas nuevas", "Preguntas y orientación para empezar."),
            };
            protocol::BoardDescriptor {
                key: protocol_board_key(descriptor.board),
                label: label.to_owned(),
                description: description.to_owned(),
                can_post: descriptor.can_post,
            }
        })
        .collect::<Vec<_>>();
    queue_sequenced(
        outbound,
        sequence,
        OutboundPriority::Critical,
        None,
        ServerMessageType::BoardCatalog,
        protocol::BoardCatalog {
            boards,
            retention_days: 90,
        },
    )
}

fn queue_board_page(outbound: &OutboundHandle, sequence: &mut u64, page: &BoardPage) -> bool {
    let posts = page.posts.iter().map(board_summary).collect::<Vec<_>>();
    queue_sequenced(
        outbound,
        sequence,
        OutboundPriority::Critical,
        None,
        ServerMessageType::BoardList,
        protocol::BoardList {
            board: protocol_board_key(page.board),
            page: page.page,
            total_pages: page.total_pages,
            total_posts: page.total_posts,
            capacity: page.capacity,
            posts,
        },
    )
}

fn queue_board_entry(outbound: &OutboundHandle, sequence: &mut u64, entry: &BoardEntry) -> bool {
    let summary = &entry.summary;
    queue_sequenced(
        outbound,
        sequence,
        OutboundPriority::Critical,
        None,
        ServerMessageType::BoardEntry,
        protocol::BoardEntry {
            post_id: summary.public_id.clone(),
            board: protocol_board_key(summary.board),
            author: summary.author_name.clone(),
            subject: summary.subject.clone(),
            body: entry.body.clone(),
            created_at: summary.created_at,
            expires_at: summary.expires_at,
            can_delete: summary.can_delete,
            can_report: summary.can_report,
        },
    )
}

fn queue_notice(outbound: &OutboundHandle, sequence: &mut u64, message: &str) -> bool {
    queue_sequenced(
        outbound,
        sequence,
        OutboundPriority::Critical,
        None,
        ServerMessageType::ServerNotice,
        protocol::ServerNotice::new(protocol::NoticeLevel::Error, message),
    )
}

fn queue_view_event(
    outbound: &OutboundHandle,
    sequence: &mut u64,
    event: ProjectedViewEvent,
    previous_presentation: &crate::runtime_content::ScenePresentation,
    current_presentation: &crate::runtime_content::ScenePresentation,
) -> bool {
    let cue = event
        .combat_cue
        .filter(|cue| {
            previous_presentation.contains_entity(&cue.entity_id)
                || current_presentation.contains_entity(&cue.entity_id)
        })
        .map(|cue| match cue.kind {
            CombatCueKind::Attack => protocol::CombatCue::attack(cue.entity_id),
            CombatCueKind::Hit => protocol::CombatCue::hit(cue.entity_id),
            CombatCueKind::Defeat => protocol::CombatCue::defeat(cue.entity_id),
        });
    queue_sequenced(
        outbound,
        sequence,
        OutboundPriority::Narrative,
        None,
        ServerMessageType::MessageAppend,
        protocol::MessageAppend {
            event: &event.event,
            cue,
        },
    )
}

fn queue_view_events(
    outbound: &OutboundHandle,
    sequence: &mut u64,
    events: Vec<ProjectedViewEvent>,
    previous_presentation: &crate::runtime_content::ScenePresentation,
    current_presentation: &crate::runtime_content::ScenePresentation,
) -> bool {
    if events.len() > MAX_PROJECTED_EVENTS_PER_BATCH {
        return queue_sequenced(
            outbound,
            sequence,
            OutboundPriority::Narrative,
            None,
            ServerMessageType::ServerNotice,
            protocol::ServerNotice::with_code(
                protocol::NoticeLevel::Info,
                "world.events_coalesced",
                format!(
                    "{} world changes were coalesced; the authoritative scene follows.",
                    events.len()
                ),
            ),
        );
    }
    events.into_iter().all(|event| {
        queue_view_event(
            outbound,
            sequence,
            event,
            previous_presentation,
            current_presentation,
        )
    })
}

pub async fn static_asset(uri: Uri) -> Response {
    let requested = uri.path().trim_start_matches('/');
    let requested = if requested.is_empty() {
        "index.html"
    } else {
        requested
    };
    if requested
        .split('/')
        .any(|component| matches!(component, ".." | ".") || component.contains('\\'))
    {
        return (StatusCode::BAD_REQUEST, "Invalid asset path").into_response();
    }
    let (asset_path, asset) = match WebAssets::get(requested) {
        Some(asset) => (requested, Some(asset)),
        None if !requested.contains('.') => ("index.html", WebAssets::get("index.html")),
        None => (requested, None),
    };

    match asset {
        Some(asset) => {
            let mime = mime_guess::from_path(asset_path).first_or_octet_stream();
            let cache = if immutable_asset_path(asset_path) {
                "public, max-age=31536000, immutable"
            } else {
                "no-cache"
            };
            (
                StatusCode::OK,
                [
                    (header::CONTENT_TYPE, mime.as_ref()),
                    (header::CACHE_CONTROL, cache),
                    (header::X_CONTENT_TYPE_OPTIONS, "nosniff"),
                    (header::REFERRER_POLICY, "no-referrer"),
                    (
                        header::CONTENT_SECURITY_POLICY,
                        "default-src 'self'; script-src 'self'; style-src 'self'; img-src 'self'; connect-src 'self' ws: wss:; object-src 'none'; base-uri 'none'; frame-ancestors 'none'",
                    ),
                ],
                Body::from(asset.data.into_owned()),
            )
                .into_response()
        }
        None => (StatusCode::NOT_FOUND, "Not found").into_response(),
    }
}

fn immutable_asset_path(path: &str) -> bool {
    let mut components = path.split('/');
    if components.next() == Some("assets")
        && components.next().is_some_and(|component| {
            component.len() == 64 && component.bytes().all(|b| b.is_ascii_hexdigit())
        })
    {
        return true;
    }

    let Some(file_name) = path.rsplit('/').next() else {
        return false;
    };
    let stem = file_name
        .rsplit_once('.')
        .map_or(file_name, |(stem, _)| stem);
    stem.split_once('-').is_some_and(|(_, hash)| {
        hash.len() >= 8
            && hash
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'))
    })
}

fn parse_client_hello(text: &str) -> Option<ClientHelloCredentials> {
    if text.len() > MAX_WS_MESSAGE_BYTES || !json_depth_within(text, MAX_JSON_DEPTH) {
        return None;
    }
    let message =
        serde_json::from_value::<protocol::ClientHello>(parse_unique_json(text).ok()?).ok()?;
    (message.v == protocol::PROTOCOL_VERSION
        && message.message_type == "client.hello"
        && message.data.client == protocol::ClientKind::Web
        && (message.data.capabilities.images.is_empty()
            || message.data.capabilities.images == protocol::SUPPORTED_IMAGE_FORMATS)
        && message.data.auth.character_id > 0
        && message.data.auth.token.len() == 64
        && message
            .data
            .auth
            .token
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit()))
    .then_some(ClientHelloCredentials {
        token: message.data.auth.token,
        character_id: message.data.auth.character_id,
        audio: message.data.capabilities.audio,
    })
}

fn parse_client_message(text: &str) -> Result<ClientMessage, ClientMessageError> {
    if text.len() > MAX_WS_MESSAGE_BYTES || !json_depth_within(text, MAX_JSON_DEPTH) {
        return Err(ClientMessageError::InvalidEnvelope);
    }
    let envelope = serde_json::from_value::<protocol::ClientEnvelope>(
        parse_unique_json(text).map_err(|_| ClientMessageError::MalformedJson)?,
    )
    .map_err(|_| ClientMessageError::MalformedJson)?;
    if envelope.v != protocol::PROTOCOL_VERSION {
        return Err(ClientMessageError::UnsupportedVersion);
    }
    if envelope
        .id
        .as_deref()
        .is_some_and(|id| !valid_request_id(id))
    {
        return Err(ClientMessageError::InvalidEnvelope);
    }

    match envelope.message_type.as_str() {
        "command.submit" => {
            let data = serde_json::from_value::<protocol::CommandSubmitData>(envelope.data)
                .map_err(|_| ClientMessageError::InvalidEnvelope)?;
            if !valid_request_id(&data.request_id)
                || data.command.len() > MAX_COMMAND_BYTES
                || data.command.chars().any(char::is_control)
            {
                return Err(ClientMessageError::InvalidEnvelope);
            }
            Ok(ClientMessage::Action(RequestedAction {
                request_id: data.request_id,
                signature: format!("command:{}", data.command),
                action: parse_action(&data.command).map_err(|error| error.to_string()),
                trade_selector: None,
                text: Some(data.command),
            }))
        }
        "action.perform" => {
            if trade_action_contains_null(&envelope.data)
                || !special_social_action_has_exact_fields(&envelope.data)
            {
                return Err(ClientMessageError::InvalidEnvelope);
            }
            let data = serde_json::from_value::<ActionPerformData>(envelope.data)
                .map_err(|_| ClientMessageError::InvalidEnvelope)?;
            if !valid_request_id(&data.request_id)
                || data.action.is_empty()
                || data.action.len() > 64
                || data.action.chars().any(char::is_control)
                || data
                    .emote
                    .as_deref()
                    .is_some_and(|emote| !valid_social_emote_command(emote))
                || data
                    .mail_id
                    .as_deref()
                    .is_some_and(|mail_id| !valid_mail_public_id(mail_id))
                || data
                    .post_id
                    .as_deref()
                    .is_some_and(|post_id| !valid_board_public_id(post_id))
                || data.subject.as_deref().is_some_and(|subject| {
                    if data.action == "board.post" {
                        !valid_board_subject(subject)
                    } else {
                        !valid_mail_subject(subject)
                    }
                })
                || data.body.as_deref().is_some_and(|body| {
                    if data.action == "board.post" {
                        !valid_board_visible_text(body, MAX_BOARD_BODY_BYTES)
                    } else {
                        !valid_mail_visible_text(body, MAX_MAIL_BODY_BYTES)
                    }
                })
                || data.ability_id.as_deref().is_some_and(|ability_id| {
                    !valid_definition_id(ability_id, MAX_ABILITY_ID_BYTES)
                })
                || data.class_id.as_deref().is_some_and(|class_id| {
                    !valid_definition_id(class_id, MAX_PROGRESSION_ID_BYTES)
                })
                || data.graph_id.as_deref().is_some_and(|graph_id| {
                    !valid_definition_id(graph_id, MAX_PROGRESSION_ID_BYTES)
                })
                || data
                    .node_id
                    .as_deref()
                    .is_some_and(|node_id| !valid_definition_id(node_id, MAX_PROGRESSION_ID_BYTES))
                || data.target.as_deref().is_some_and(|target| {
                    target.is_empty()
                        || target.len() > MAX_ACTION_TARGET_BYTES
                        || target.chars().any(char::is_control)
                })
                || (data.action == "social.emote"
                    && data
                        .target
                        .as_deref()
                        .is_some_and(|target| target.len() > MAX_SOCIAL_TARGET_BYTES))
                || data.recipient.as_deref().is_some_and(|recipient| {
                    recipient.is_empty()
                        || recipient.len() > MAX_SOCIAL_TARGET_BYTES
                        || recipient.chars().any(char::is_control)
                })
                || data.container.as_deref().is_some_and(|container| {
                    container.is_empty()
                        || container.len() > MAX_ACTION_TARGET_BYTES
                        || container.chars().any(char::is_control)
                })
                || data
                    .quantity
                    .is_some_and(|quantity| quantity == 0 || quantity > 256)
                || data.page == Some(0)
                || (data.action == "mail.list"
                    && data.page.is_some_and(|page| page > MAX_MAIL_PAGE))
                || (data.action == "board.list"
                    && data.page.is_some_and(|page| page > MAX_BOARD_PAGE))
                || data
                    .trade_id
                    .as_deref()
                    .is_some_and(|trade_id| !valid_opaque_selector(trade_id))
                || data.coins.is_some_and(|coins| coins > 1_000_000_000)
                || data.state_revision == Some(0)
                || (data.action == "trade.request"
                    && data
                        .recipient
                        .as_deref()
                        .is_none_or(|recipient| !valid_opaque_selector(recipient)))
                || (matches!(
                    data.action.as_str(),
                    "trade.item.set" | "trade.item.remove" | "trade.contents"
                ) && data
                    .target
                    .as_deref()
                    .is_none_or(|target| !valid_opaque_selector(target)))
                || data.message.as_deref().is_some_and(|message| {
                    message.is_empty()
                        || message.len() > MAX_CHAT_MESSAGE_BYTES
                        || message.chars().any(char::is_control)
                })
                || data.reason.as_deref().is_some_and(|reason| {
                    reason.is_empty()
                        || reason.len() > MAX_REPORT_REASON_BYTES
                        || reason.chars().any(char::is_control)
                })
                || (data.action == "mail.report"
                    && data.reason.as_deref().is_some_and(|reason| {
                        !valid_mail_visible_text(reason, MAX_MAIL_REPORT_REASON_BYTES)
                    }))
                || (data.action == "board.report"
                    && data.reason.as_deref().is_some_and(|reason| {
                        !valid_board_visible_text(reason, MAX_BOARD_REPORT_REASON_BYTES)
                    }))
                || (data.action == "mail.send"
                    && data
                        .target
                        .as_deref()
                        .is_some_and(|target| !valid_mail_target(target)))
                || matches!(
                    &data.lore_title,
                    NullableField::Value(title)
                        if title.is_empty()
                            || title.len() > MAX_STAFF_LORE_TITLE_BYTES
                            || title.chars().any(char::is_control)
                )
            {
                return Err(ClientMessageError::InvalidEnvelope);
            }
            let signature = format!(
                "action:{:?}",
                (
                    (
                        &data.action,
                        &data.emote,
                        &data.ability_id,
                        &data.class_id,
                        &data.graph_id,
                        &data.node_id,
                        &data.target,
                        &data.recipient,
                        &data.container,
                        data.quantity,
                        data.page,
                    ),
                    (
                        &data.trade_id,
                        data.coins,
                        data.state_revision,
                        &data.message,
                        data.channel,
                        data.duration_seconds,
                        &data.reason,
                        data.role,
                        data.report_id,
                        data.resolution,
                    ),
                    (
                        &data.profile,
                        &data.lore_title,
                        data.capability,
                        data.enabled,
                        &data.mail_id,
                        &data.post_id,
                        data.board,
                        &data.subject,
                        &data.body,
                    ),
                )
            );
            let action = structured_action(&data);
            let trade_selector = data.trade_id.clone();
            Ok(ClientMessage::Action(RequestedAction {
                request_id: data.request_id,
                signature,
                action,
                trade_selector,
                text: None,
            }))
        }
        "sync.request" => {
            let data = serde_json::from_value::<protocol::SyncRequestData>(envelope.data)
                .map_err(|_| ClientMessageError::InvalidEnvelope)?;
            Ok(ClientMessage::Sync {
                after_seq: data.after_seq,
            })
        }
        "client.heartbeat" => {
            let data = serde_json::from_value::<protocol::ClientHeartbeatData>(envelope.data)
                .map_err(|_| ClientMessageError::InvalidEnvelope)?;
            if data.nonce == 0 {
                return Err(ClientMessageError::InvalidEnvelope);
            }
            Ok(ClientMessage::Heartbeat { nonce: data.nonce })
        }
        "story.choose" | "story.dialogue" => {
            let data = serde_json::from_value::<protocol::StoryChooseData>(envelope.data)
                .map_err(|_| ClientMessageError::InvalidEnvelope)?;
            // Shape only: whether the id was ever published is decided by the
            // instance, which rejects forged and stale ids without changes.
            if !valid_story_id_shape(&data.id) || data.view_rev == 0 {
                return Err(ClientMessageError::InvalidEnvelope);
            }
            let request = if envelope.message_type == "story.choose" {
                crate::story_sessions::StoryRequest::Choose { id: data.id }
            } else {
                crate::story_sessions::StoryRequest::Dialogue { id: data.id }
            };
            Ok(ClientMessage::StoryInput {
                request,
                view_rev: data.view_rev,
            })
        }
        "story.menu" => {
            let data = serde_json::from_value::<protocol::StoryMenuData>(envelope.data)
                .map_err(|_| ClientMessageError::InvalidEnvelope)?;
            if !valid_story_id_shape(&data.verb)
                || data
                    .target
                    .as_deref()
                    .is_some_and(|target| !valid_story_id_shape(target))
                || data.view_rev == 0
            {
                return Err(ClientMessageError::InvalidEnvelope);
            }
            Ok(ClientMessage::StoryInput {
                request: crate::story_sessions::StoryRequest::Menu {
                    verb: data.verb,
                    target: data.target,
                },
                view_rev: data.view_rev,
            })
        }
        "story.undo" | "story.leave" => {
            // Exactly `{}`: serde would also accept an empty array.
            if !envelope.data.as_object().is_some_and(Map::is_empty) {
                return Err(ClientMessageError::InvalidEnvelope);
            }
            Ok(if envelope.message_type == "story.undo" {
                ClientMessage::StoryUndo
            } else {
                ClientMessage::StoryLeave
            })
        }
        "story.save" | "story.restore" | "story.deleteSlot" => {
            let data = serde_json::from_value::<protocol::StorySlotData>(envelope.data)
                .map_err(|_| ClientMessageError::InvalidEnvelope)?;
            // Shape only: slot 0 (the autosave) is refused for `story.save` by
            // the session with a typed `invalid_slot` error.
            if data.slot > protocol::MAX_STORY_SLOT {
                return Err(ClientMessageError::InvalidEnvelope);
            }
            Ok(match envelope.message_type.as_str() {
                "story.save" => ClientMessage::StorySave { slot: data.slot },
                "story.restore" => ClientMessage::StoryRestore { slot: data.slot },
                _ => ClientMessage::StoryDeleteSlot { slot: data.slot },
            })
        }
        "story.slots" | "story.continue" | "story.share" => {
            if !envelope.data.as_object().is_some_and(Map::is_empty) {
                return Err(ClientMessageError::InvalidEnvelope);
            }
            Ok(match envelope.message_type.as_str() {
                "story.slots" => ClientMessage::StorySlots,
                "story.share" => ClientMessage::StoryShare,
                _ => ClientMessage::StoryRestore { slot: 0 },
            })
        }
        "story.command" => {
            let data = serde_json::from_value::<protocol::StoryCommandData>(envelope.data)
                .map_err(|_| ClientMessageError::InvalidEnvelope)?;
            // Shape only; the instance answers stale views and the parser
            // answers empty or unknown text.
            if data.text.len() > protocol::MAX_STORY_COMMAND_BYTES || data.view_rev == 0 {
                return Err(ClientMessageError::InvalidEnvelope);
            }
            Ok(ClientMessage::StoryCommand {
                text: data.text,
                view_rev: data.view_rev,
            })
        }
        "story.vote" => {
            let data = serde_json::from_value::<protocol::StoryVoteData>(envelope.data)
                .map_err(|_| ClientMessageError::InvalidEnvelope)?;
            if data
                .choice_id
                .as_deref()
                .is_some_and(|id| !valid_story_id_shape(id))
                || data.view_rev == 0
            {
                return Err(ClientMessageError::InvalidEnvelope);
            }
            Ok(ClientMessage::StoryVote {
                leaf: data.choice_id,
                view_rev: data.view_rev,
            })
        }
        "story.join" => {
            let data = serde_json::from_value::<protocol::StoryJoinData>(envelope.data)
                .map_err(|_| ClientMessageError::InvalidEnvelope)?;
            // Shape only; the code itself is checked (and rate limited) later.
            if data.code.is_empty()
                || data.code.len() > protocol::STORY_JOIN_CODE_BYTES
                || !data.code.bytes().all(|byte| byte.is_ascii_alphanumeric())
            {
                return Err(ClientMessageError::InvalidEnvelope);
            }
            Ok(ClientMessage::StoryJoin {
                code: data.code,
                spectate: data.spectate,
            })
        }
        "story.invite" | "story.kick" => {
            let data = serde_json::from_value::<protocol::StoryNameData>(envelope.data)
                .map_err(|_| ClientMessageError::InvalidEnvelope)?;
            let name = data.name.trim();
            if name.is_empty()
                || name.len() > crate::story_sessions::shared::MAX_MEMBER_NAME_BYTES
                || name.chars().any(char::is_control)
            {
                return Err(ClientMessageError::InvalidEnvelope);
            }
            let name = name.to_owned();
            Ok(if envelope.message_type == "story.invite" {
                ClientMessage::StoryInvite { name }
            } else {
                ClientMessage::StoryKick { name }
            })
        }
        "story.mode" => {
            let data = serde_json::from_value::<protocol::StoryModeData>(envelope.data)
                .map_err(|_| ClientMessageError::InvalidEnvelope)?;
            Ok(ClientMessage::StoryMode {
                mode: data.mode,
                vote_seconds: data.vote_seconds,
            })
        }
        _ => Err(ClientMessageError::UnknownType),
    }
}

/// Bounded alphanumeric shape of a story id; publication is checked later.
fn valid_story_id_shape(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= protocol::STORY_CHOICE_ID_BYTES
        && id.bytes().all(|byte| byte.is_ascii_alphanumeric())
}

struct UniqueJsonValue(Value);

impl<'de> Deserialize<'de> for UniqueJsonValue {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        deserializer.deserialize_any(UniqueJsonVisitor)
    }
}

struct UniqueJsonVisitor;

impl<'de> Visitor<'de> for UniqueJsonVisitor {
    type Value = UniqueJsonValue;

    fn expecting(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("JSON without duplicate object keys")
    }

    fn visit_bool<E>(self, value: bool) -> Result<Self::Value, E> {
        Ok(UniqueJsonValue(Value::Bool(value)))
    }

    fn visit_i64<E>(self, value: i64) -> Result<Self::Value, E> {
        Ok(UniqueJsonValue(Value::Number(value.into())))
    }

    fn visit_u64<E>(self, value: u64) -> Result<Self::Value, E> {
        Ok(UniqueJsonValue(Value::Number(value.into())))
    }

    fn visit_f64<E>(self, value: f64) -> Result<Self::Value, E>
    where
        E: serde::de::Error,
    {
        serde_json::Number::from_f64(value)
            .map(Value::Number)
            .map(UniqueJsonValue)
            .ok_or_else(|| E::custom("non-finite JSON number"))
    }

    fn visit_str<E>(self, value: &str) -> Result<Self::Value, E> {
        Ok(UniqueJsonValue(Value::String(value.to_owned())))
    }

    fn visit_string<E>(self, value: String) -> Result<Self::Value, E> {
        Ok(UniqueJsonValue(Value::String(value)))
    }

    fn visit_none<E>(self) -> Result<Self::Value, E> {
        Ok(UniqueJsonValue(Value::Null))
    }

    fn visit_unit<E>(self) -> Result<Self::Value, E> {
        Ok(UniqueJsonValue(Value::Null))
    }

    fn visit_seq<A>(self, mut sequence: A) -> Result<Self::Value, A::Error>
    where
        A: SeqAccess<'de>,
    {
        let mut values = Vec::new();
        while let Some(value) = sequence.next_element::<UniqueJsonValue>()? {
            values.push(value.0);
        }
        Ok(UniqueJsonValue(Value::Array(values)))
    }

    fn visit_map<A>(self, mut object: A) -> Result<Self::Value, A::Error>
    where
        A: MapAccess<'de>,
    {
        let mut values = Map::new();
        while let Some(key) = object.next_key::<String>()? {
            if values.contains_key(&key) {
                return Err(A::Error::custom(format!("duplicate JSON key: {key}")));
            }
            values.insert(key, object.next_value::<UniqueJsonValue>()?.0);
        }
        Ok(UniqueJsonValue(Value::Object(values)))
    }
}

fn parse_unique_json(text: &str) -> Result<Value, serde_json::Error> {
    let mut deserializer = serde_json::Deserializer::from_str(text);
    let value = UniqueJsonValue::deserialize(&mut deserializer)?.0;
    deserializer.end()?;
    Ok(value)
}

fn json_depth_within(text: &str, maximum: usize) -> bool {
    let mut depth = 0_usize;
    let mut in_string = false;
    let mut escaped = false;
    for byte in text.bytes() {
        if in_string {
            if escaped {
                escaped = false;
            } else if byte == b'\\' {
                escaped = true;
            } else if byte == b'"' {
                in_string = false;
            }
            continue;
        }
        match byte {
            b'"' => in_string = true,
            b'{' | b'[' => {
                depth += 1;
                if depth > maximum {
                    return false;
                }
            }
            b'}' | b']' => {
                let Some(next) = depth.checked_sub(1) else {
                    return false;
                };
                depth = next;
            }
            _ => {}
        }
    }
    depth == 0 && !in_string && !escaped
}

fn valid_request_id(value: &str) -> bool {
    !value.is_empty() && value.len() <= MAX_REQUEST_ID_BYTES && !value.chars().any(char::is_control)
}

fn valid_opaque_selector(value: &str) -> bool {
    value.len() == 17
        && value.as_bytes().first() == Some(&b'o')
        && value.as_bytes()[1..]
            .iter()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(byte))
}

fn trade_action_contains_null(data: &Value) -> bool {
    let Some(fields) = data.as_object() else {
        return false;
    };
    fields
        .get("action")
        .and_then(Value::as_str)
        .is_some_and(|action| action.starts_with("trade."))
        && fields.values().any(Value::is_null)
}

fn special_social_action_has_exact_fields(data: &Value) -> bool {
    let Some(fields) = data.as_object() else {
        return true;
    };
    match fields.get("action").and_then(Value::as_str) {
        Some("social.yell" | "social.shout") => {
            const EXACT_FIELDS: [&str; 3] = ["requestId", "action", "message"];
            fields.len() == EXACT_FIELDS.len()
                && fields
                    .keys()
                    .all(|field| EXACT_FIELDS.contains(&field.as_str()))
                && fields.get("message").is_some_and(Value::is_string)
        }
        Some("social.telepathy") => {
            const EXACT_FIELDS: [&str; 5] =
                ["requestId", "action", "abilityId", "target", "message"];
            fields.len() == EXACT_FIELDS.len()
                && fields
                    .keys()
                    .all(|field| EXACT_FIELDS.contains(&field.as_str()))
        }
        Some("social.emote.list") => {
            const EXACT_FIELDS: [&str; 2] = ["requestId", "action"];
            fields.len() == EXACT_FIELDS.len()
                && fields
                    .keys()
                    .all(|field| EXACT_FIELDS.contains(&field.as_str()))
        }
        Some("social.emote") => {
            const EXACT_FIELDS: [&str; 4] = ["requestId", "action", "emote", "target"];
            (fields.len() == 3 || fields.len() == 4)
                && fields
                    .keys()
                    .all(|field| EXACT_FIELDS.contains(&field.as_str()))
                && fields.get("emote").is_some_and(Value::is_string)
                && fields.get("target").is_none_or(Value::is_string)
        }
        Some("mail.list") => {
            const EXACT_FIELDS: [&str; 3] = ["requestId", "action", "page"];
            (fields.len() == 2 || fields.len() == 3)
                && fields
                    .keys()
                    .all(|field| EXACT_FIELDS.contains(&field.as_str()))
                && fields.get("page").is_none_or(Value::is_number)
        }
        Some("mail.read" | "mail.delete") => {
            const EXACT_FIELDS: [&str; 3] = ["requestId", "action", "mailId"];
            fields.len() == EXACT_FIELDS.len()
                && fields
                    .keys()
                    .all(|field| EXACT_FIELDS.contains(&field.as_str()))
                && fields.get("mailId").is_some_and(Value::is_string)
        }
        Some("mail.send") => {
            const EXACT_FIELDS: [&str; 5] = ["requestId", "action", "target", "subject", "body"];
            fields.len() == EXACT_FIELDS.len()
                && fields
                    .keys()
                    .all(|field| EXACT_FIELDS.contains(&field.as_str()))
                && ["target", "subject", "body"]
                    .into_iter()
                    .all(|field| fields.get(field).is_some_and(Value::is_string))
        }
        Some("mail.report") => {
            const EXACT_FIELDS: [&str; 4] = ["requestId", "action", "mailId", "reason"];
            fields.len() == EXACT_FIELDS.len()
                && fields
                    .keys()
                    .all(|field| EXACT_FIELDS.contains(&field.as_str()))
                && fields.get("mailId").is_some_and(Value::is_string)
                && fields.get("reason").is_some_and(Value::is_string)
        }
        Some("board.catalog") => {
            const EXACT_FIELDS: [&str; 2] = ["requestId", "action"];
            fields.len() == EXACT_FIELDS.len()
                && fields
                    .keys()
                    .all(|field| EXACT_FIELDS.contains(&field.as_str()))
        }
        Some("board.list") => {
            const EXACT_FIELDS: [&str; 4] = ["requestId", "action", "board", "page"];
            fields.len() == EXACT_FIELDS.len()
                && fields
                    .keys()
                    .all(|field| EXACT_FIELDS.contains(&field.as_str()))
                && fields.get("board").is_some_and(Value::is_string)
                && fields.get("page").is_some_and(Value::is_number)
        }
        Some("board.read" | "board.delete") => {
            const EXACT_FIELDS: [&str; 3] = ["requestId", "action", "postId"];
            fields.len() == EXACT_FIELDS.len()
                && fields
                    .keys()
                    .all(|field| EXACT_FIELDS.contains(&field.as_str()))
                && fields.get("postId").is_some_and(Value::is_string)
        }
        Some("board.post") => {
            const EXACT_FIELDS: [&str; 5] = ["requestId", "action", "board", "subject", "body"];
            fields.len() == EXACT_FIELDS.len()
                && fields
                    .keys()
                    .all(|field| EXACT_FIELDS.contains(&field.as_str()))
                && ["board", "subject", "body"]
                    .into_iter()
                    .all(|field| fields.get(field).is_some_and(Value::is_string))
        }
        Some("board.report") => {
            const EXACT_FIELDS: [&str; 4] = ["requestId", "action", "postId", "reason"];
            fields.len() == EXACT_FIELDS.len()
                && fields
                    .keys()
                    .all(|field| EXACT_FIELDS.contains(&field.as_str()))
                && fields.get("postId").is_some_and(Value::is_string)
                && fields.get("reason").is_some_and(Value::is_string)
        }
        _ => {
            !fields.contains_key("emote")
                && !fields.contains_key("mailId")
                && !fields.contains_key("postId")
                && !fields.contains_key("board")
                && !fields.contains_key("subject")
                && !fields.contains_key("body")
        }
    }
}

fn valid_definition_id(value: &str, maximum: usize) -> bool {
    let bytes = value.as_bytes();
    !bytes.is_empty()
        && bytes.len() <= maximum
        && bytes[0].is_ascii_alphanumeric()
        && bytes[bytes.len() - 1].is_ascii_alphanumeric()
        && value.bytes().all(|byte| {
            byte.is_ascii_lowercase()
                || byte.is_ascii_digit()
                || matches!(byte, b'-' | b'_' | b'.' | b':')
        })
}

fn valid_social_emote_command(value: &str) -> bool {
    let bytes = value.as_bytes();
    !bytes.is_empty()
        && bytes.len() <= 32
        && bytes[0].is_ascii_lowercase()
        && bytes
            .iter()
            .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || *byte == b'-')
}

fn structured_action(data: &ActionPerformData) -> Result<PlayerAction, String> {
    let action = data.action.as_str();
    if action.starts_with("board.") {
        return structured_board_action(data);
    }
    if action.starts_with("mail.") {
        return structured_mail_action(data);
    }
    if data.mail_id.is_some()
        || data.post_id.is_some()
        || data.board.is_some()
        || data.subject.is_some()
        || data.body.is_some()
    {
        return Err(format!("Action {action:?} has unexpected fields."));
    }
    if !matches!(action, "social.emote" | "social.emote.list") && data.emote.is_some() {
        return Err(format!("Action {action:?} has unexpected fields."));
    }
    let trade_action = action.starts_with("trade.");
    if action != "item.give" && action != "trade.request" && data.recipient.is_some() {
        return Err(format!("Action {action:?} has unexpected fields."));
    }
    if !trade_action
        && (data.trade_id.is_some() || data.coins.is_some() || data.state_revision.is_some())
    {
        return Err(format!("Action {action:?} has unexpected fields."));
    }
    let has_item_fields =
        data.container.is_some() || data.quantity.is_some() || data.page.is_some();
    if !matches!(
        action,
        "item.take"
            | "item.drop"
            | "item.put"
            | "item.get"
            | "item.contents"
            | "item.give"
            | "reward.odds"
            | "trade.item.set"
            | "trade.contents"
    ) && has_item_fields
    {
        return Err(format!("Action {action:?} has unexpected fields."));
    }
    let has_staff_fields = data.report_id.is_some()
        || data.resolution.is_some()
        || !matches!(data.profile, NullableField::Missing)
        || !matches!(data.lore_title, NullableField::Missing)
        || data.capability.is_some()
        || data.enabled.is_some();
    if !action.starts_with("staff.") && has_staff_fields {
        return Err(format!("Action {action:?} has unexpected fields."));
    }
    let target = data.target.as_deref();
    let has_progression_fields =
        || data.class_id.is_some() || data.graph_id.is_some() || data.node_id.is_some();
    let no_target = || {
        target
            .is_none()
            .then_some(())
            .ok_or_else(|| format!("Action {action:?} does not accept a target."))
    };
    let required_target = || {
        target
            .map(str::to_owned)
            .ok_or_else(|| format!("Action {action:?} requires a target."))
    };
    let no_social_extras = || {
        (data.ability_id.is_none()
            && data.class_id.is_none()
            && data.graph_id.is_none()
            && data.node_id.is_none()
            && data.container.is_none()
            && data.recipient.is_none()
            && data.quantity.is_none()
            && data.page.is_none()
            && data.message.is_none()
            && data.channel.is_none()
            && data.duration_seconds.is_none()
            && data.reason.is_none()
            && data.role.is_none()
            && data.report_id.is_none()
            && data.resolution.is_none()
            && matches!(data.profile, NullableField::Missing)
            && matches!(data.lore_title, NullableField::Missing)
            && data.capability.is_none()
            && data.enabled.is_none())
        .then_some(())
        .ok_or_else(|| format!("Action {action:?} has unexpected fields."))
    };
    let exact_no_fields = || {
        no_target()?;
        no_social_extras()
    };
    let social_target = || {
        let target = required_target()?;
        if target.len() > MAX_SOCIAL_TARGET_BYTES {
            return Err("Social target is too long.".to_owned());
        }
        Ok(target)
    };
    let gameplay_target = || {
        no_social_extras()?;
        required_target()
    };
    let no_item_extras = || {
        (data.ability_id.is_none()
            && data.class_id.is_none()
            && data.graph_id.is_none()
            && data.node_id.is_none()
            && data.recipient.is_none()
            && data.message.is_none()
            && data.channel.is_none()
            && data.duration_seconds.is_none()
            && data.reason.is_none()
            && data.role.is_none()
            && data.report_id.is_none()
            && data.resolution.is_none()
            && matches!(data.profile, NullableField::Missing)
            && matches!(data.lore_title, NullableField::Missing)
            && data.capability.is_none()
            && data.enabled.is_none())
        .then_some(())
        .ok_or_else(|| format!("Action {action:?} has unexpected fields."))
    };
    let no_give_extras = || {
        (data.ability_id.is_none()
            && data.class_id.is_none()
            && data.graph_id.is_none()
            && data.node_id.is_none()
            && data.message.is_none()
            && data.channel.is_none()
            && data.duration_seconds.is_none()
            && data.reason.is_none()
            && data.role.is_none()
            && data.report_id.is_none()
            && data.resolution.is_none()
            && matches!(data.profile, NullableField::Missing)
            && matches!(data.lore_title, NullableField::Missing)
            && data.capability.is_none()
            && data.enabled.is_none())
        .then_some(())
        .ok_or_else(|| format!("Action {action:?} has unexpected fields."))
    };
    let item_selection = || {
        no_item_extras()?;
        Ok(ItemSelection {
            query: required_target()?,
            quantity: data.quantity.unwrap_or(1),
        })
    };
    let item_container = || {
        data.container
            .clone()
            .ok_or_else(|| format!("Action {action:?} requires container."))
    };
    let no_non_staff_fields = || {
        (data.ability_id.is_none()
            && data.class_id.is_none()
            && data.graph_id.is_none()
            && data.node_id.is_none()
            && data.channel.is_none()
            && data.duration_seconds.is_none()
            && data.role.is_none())
        .then_some(())
        .ok_or_else(|| format!("Action {action:?} has unexpected fields."))
    };
    let no_staff_extension_fields = || {
        (data.report_id.is_none()
            && data.resolution.is_none()
            && matches!(data.profile, NullableField::Missing)
            && matches!(data.lore_title, NullableField::Missing)
            && data.capability.is_none()
            && data.enabled.is_none())
        .then_some(())
        .ok_or_else(|| format!("Action {action:?} has unexpected fields."))
    };
    let staff_target = || {
        let target = required_target()?;
        if target.len() > MAX_STAFF_TARGET_BYTES {
            return Err("Staff target is too long.".to_owned());
        }
        Ok(target)
    };
    let staff_reason = || {
        let reason = data
            .reason
            .clone()
            .ok_or_else(|| format!("Action {action:?} requires reason."))?;
        if reason.len() > MAX_STAFF_REASON_BYTES {
            return Err("Staff reason is too long.".to_owned());
        }
        Ok(reason)
    };
    let trade_fields_error = || format!("Action {action:?} has unexpected or missing fields.");
    let trade_fields_exact = |target: bool,
                              recipient: bool,
                              quantity: bool,
                              page: bool,
                              coins: bool,
                              state_revision: bool,
                              trade_id: bool| {
        let exact = data.ability_id.is_none()
            && data.class_id.is_none()
            && data.graph_id.is_none()
            && data.node_id.is_none()
            && data.container.is_none()
            && (target == data.target.is_some())
            && (recipient == data.recipient.is_some())
            && (quantity == data.quantity.is_some())
            && (page == data.page.is_some())
            && (coins == data.coins.is_some())
            && (state_revision == data.state_revision.is_some())
            && (trade_id == data.trade_id.is_some())
            && data.message.is_none()
            && data.channel.is_none()
            && data.duration_seconds.is_none()
            && data.reason.is_none()
            && data.role.is_none()
            && data.report_id.is_none()
            && data.resolution.is_none()
            && matches!(data.profile, NullableField::Missing)
            && matches!(data.lore_title, NullableField::Missing)
            && data.capability.is_none()
            && data.enabled.is_none();
        exact.then_some(()).ok_or_else(trade_fields_error)
    };
    match action {
        "look" => exact_no_fields().map(|()| PlayerAction::Look),
        "inventory" => exact_no_fields().map(|()| PlayerAction::Inventory),
        "loot.status" => exact_no_fields().map(|()| PlayerAction::Loot(LootAction::Status)),
        "loot.pass" => gameplay_target()
            .map(LootAction::Pass)
            .map(PlayerAction::Loot),
        "reward.open" => gameplay_target().map(PlayerAction::OpenReward),
        "reward.odds" => {
            no_item_extras()?;
            if data.container.is_some() || data.quantity.is_some() {
                return Err(format!("Action {action:?} has unexpected fields."));
            }
            Ok(PlayerAction::RewardOdds {
                target: required_target()?,
                page: data.page.unwrap_or(1),
            })
        }
        "move" => {
            let target = gameplay_target()?;
            match parse_action(&target) {
                Ok(action @ PlayerAction::Move(_)) => Ok(action),
                _ => Err(format!("Unknown movement direction: {target}")),
            }
        }
        "item.take" => {
            if data.container.is_some() || data.page.is_some() {
                return Err(format!("Action {action:?} has unexpected fields."));
            }
            item_selection().map(PlayerAction::Take)
        }
        "item.drop" => {
            if data.container.is_some() || data.page.is_some() {
                return Err(format!("Action {action:?} has unexpected fields."));
            }
            item_selection().map(PlayerAction::Drop)
        }
        "item.give" => {
            if data.container.is_some() || data.page.is_some() {
                return Err(format!("Action {action:?} has unexpected fields."));
            }
            no_give_extras()?;
            let recipient = data
                .recipient
                .clone()
                .ok_or_else(|| format!("Action {action:?} requires recipient."))?;
            if recipient.len() > MAX_SOCIAL_TARGET_BYTES {
                return Err("Give recipient is too long.".to_owned());
            }
            let selection = ItemSelection {
                query: required_target()?,
                quantity: data.quantity.unwrap_or(1),
            };
            Ok(PlayerAction::Give {
                selection,
                recipient,
            })
        }
        "item.put" => {
            if data.page.is_some() {
                return Err(format!("Action {action:?} has unexpected fields."));
            }
            Ok(PlayerAction::Put {
                selection: item_selection()?,
                container: item_container()?,
            })
        }
        "item.get" => {
            if data.page.is_some() {
                return Err(format!("Action {action:?} has unexpected fields."));
            }
            Ok(PlayerAction::Get {
                selection: item_selection()?,
                container: item_container()?,
            })
        }
        "item.contents" => {
            no_item_extras()?;
            no_target()?;
            if data.quantity.is_some() {
                return Err(format!("Action {action:?} has unexpected fields."));
            }
            Ok(PlayerAction::Contents {
                container: item_container()?,
                page: data.page.unwrap_or(1),
            })
        }
        "item.examine" => gameplay_target().map(PlayerAction::Examine),
        "combat.attack" => gameplay_target().map(PlayerAction::Attack),
        "combat.ability" => {
            if data.class_id.is_some()
                || data.graph_id.is_some()
                || data.node_id.is_some()
                || data.message.is_some()
                || data.channel.is_some()
                || data.duration_seconds.is_some()
                || data.reason.is_some()
                || data.role.is_some()
            {
                return Err("Combat ability has unexpected fields.".to_owned());
            }
            let ability_id = data
                .ability_id
                .clone()
                .ok_or_else(|| "Combat ability requires abilityId.".to_owned())?;
            Ok(PlayerAction::Ability {
                ability_id,
                target: data.target.clone(),
            })
        }
        "progression.promote" => {
            if data.target.is_some()
                || data.ability_id.is_some()
                || data.graph_id.is_some()
                || data.node_id.is_some()
                || data.message.is_some()
                || data.channel.is_some()
                || data.duration_seconds.is_some()
                || data.reason.is_some()
                || data.role.is_some()
            {
                return Err("Class promotion has unexpected fields.".to_owned());
            }
            data.class_id
                .clone()
                .map(|class_id| PlayerAction::Promote { class_id })
                .ok_or_else(|| "Class promotion requires classId.".to_owned())
        }
        "progression.talent" => {
            if data.target.is_some()
                || data.ability_id.is_some()
                || data.class_id.is_some()
                || data.message.is_some()
                || data.channel.is_some()
                || data.duration_seconds.is_some()
                || data.reason.is_some()
                || data.role.is_some()
            {
                return Err("Talent acquisition has unexpected fields.".to_owned());
            }
            let graph_id = data
                .graph_id
                .clone()
                .ok_or_else(|| "Talent acquisition requires graphId.".to_owned())?;
            let node_id = data
                .node_id
                .clone()
                .ok_or_else(|| "Talent acquisition requires nodeId.".to_owned())?;
            Ok(PlayerAction::AcquireTalent { graph_id, node_id })
        }
        "item.use" => gameplay_target().map(PlayerAction::Use),
        "item.equip" => gameplay_target().map(PlayerAction::Equip),
        "item.unequip" => gameplay_target().map(PlayerAction::Unequip),
        "trade.request" => {
            trade_fields_exact(false, true, false, false, false, false, false)?;
            let Some(recipient) = data.recipient.clone() else {
                return Err(trade_fields_error());
            };
            Ok(PlayerAction::Trade(TradeAction::Request(recipient)))
        }
        "trade.accept" => {
            trade_fields_exact(false, false, false, false, false, false, true)?;
            Ok(PlayerAction::Trade(TradeAction::Accept))
        }
        "trade.decline" => {
            trade_fields_exact(false, false, false, false, false, false, true)?;
            Ok(PlayerAction::Trade(TradeAction::Decline))
        }
        "trade.item.set" => {
            trade_fields_exact(true, false, true, false, false, false, true)?;
            let (Some(query), Some(quantity)) = (data.target.clone(), data.quantity) else {
                return Err(trade_fields_error());
            };
            if quantity > 128 {
                return Err("Trade item quantity must be between 1 and 128.".to_owned());
            }
            Ok(PlayerAction::Trade(TradeAction::OfferItem(ItemSelection {
                query,
                quantity,
            })))
        }
        "trade.item.remove" => {
            trade_fields_exact(true, false, false, false, false, false, true)?;
            let Some(target) = data.target.clone() else {
                return Err(trade_fields_error());
            };
            Ok(PlayerAction::Trade(TradeAction::RemoveItem(target)))
        }
        "trade.coins.set" => {
            trade_fields_exact(false, false, false, false, true, false, true)?;
            let Some(coins) = data.coins else {
                return Err(trade_fields_error());
            };
            Ok(PlayerAction::Trade(TradeAction::OfferCoins(coins)))
        }
        "trade.contents" => {
            trade_fields_exact(true, false, false, true, false, false, true)?;
            let (Some(container), Some(page)) = (data.target.clone(), data.page) else {
                return Err(trade_fields_error());
            };
            Ok(PlayerAction::Trade(TradeAction::Contents {
                container,
                page,
            }))
        }
        "trade.confirm" => {
            trade_fields_exact(false, false, false, false, false, true, true)?;
            let Some(state_revision) = data.state_revision else {
                return Err(trade_fields_error());
            };
            Ok(PlayerAction::Trade(TradeAction::Confirm { state_revision }))
        }
        "trade.status" => {
            trade_fields_exact(false, false, false, false, false, false, true)?;
            Ok(PlayerAction::Trade(TradeAction::Status))
        }
        "trade.cancel" => {
            trade_fields_exact(false, false, false, false, false, false, true)?;
            Ok(PlayerAction::Trade(TradeAction::Cancel))
        }
        "party.create" => exact_no_fields().map(|()| PlayerAction::Party(PartyAction::Create)),
        "party.disband" => exact_no_fields().map(|()| PlayerAction::Party(PartyAction::Disband)),
        "party.leave" => exact_no_fields().map(|()| PlayerAction::Party(PartyAction::Leave)),
        "party.status" => exact_no_fields().map(|()| PlayerAction::Party(PartyAction::Status)),
        "party.invite" => {
            gameplay_target().map(|target| PlayerAction::Party(PartyAction::Invite(target)))
        }
        "party.accept" => {
            gameplay_target().map(|target| PlayerAction::Party(PartyAction::Accept(target)))
        }
        "party.revoke" => {
            gameplay_target().map(|target| PlayerAction::Party(PartyAction::Revoke(target)))
        }
        "party.kick" => {
            gameplay_target().map(|target| PlayerAction::Party(PartyAction::Kick(target)))
        }
        "party.transfer" => {
            gameplay_target().map(|target| PlayerAction::Party(PartyAction::Transfer(target)))
        }
        "party.say" => {
            no_target()?;
            if has_progression_fields()
                || data.ability_id.is_some()
                || data.channel.is_some()
                || data.duration_seconds.is_some()
                || data.reason.is_some()
                || data.role.is_some()
            {
                return Err("Party say has unexpected fields.".to_owned());
            }
            data.message
                .clone()
                .map(SocialAction::PartySay)
                .map(PlayerAction::Social)
                .ok_or_else(|| "Party say requires message.".to_owned())
        }
        "quest.list" => exact_no_fields().map(|()| PlayerAction::Quest(QuestAction::List)),
        "quest.status" => exact_no_fields().map(|()| PlayerAction::Quest(QuestAction::Status)),
        "quest.accept" => {
            gameplay_target().map(|target| PlayerAction::Quest(QuestAction::Accept(target)))
        }
        "quest.complete" => {
            gameplay_target().map(|target| PlayerAction::Quest(QuestAction::Complete(target)))
        }
        "economy.balance" => {
            exact_no_fields().map(|()| PlayerAction::Economy(EconomyAction::Balance))
        }
        "economy.shop" => exact_no_fields().map(|()| PlayerAction::Economy(EconomyAction::Shop)),
        "economy.buy" => {
            gameplay_target().map(|target| PlayerAction::Economy(EconomyAction::Buy(target)))
        }
        "economy.sell" => {
            gameplay_target().map(|target| PlayerAction::Economy(EconomyAction::Sell(target)))
        }
        "world.weather" => exact_no_fields().map(|()| PlayerAction::Weather),
        "world.time" => exact_no_fields().map(|()| PlayerAction::Time),
        "social.say" => {
            no_target()?;
            if has_progression_fields()
                || data.ability_id.is_some()
                || data.channel.is_some()
                || data.duration_seconds.is_some()
                || data.reason.is_some()
                || data.role.is_some()
            {
                return Err("Social say has unexpected fields.".to_owned());
            }
            data.message
                .clone()
                .map(SocialAction::Say)
                .map(PlayerAction::Social)
                .ok_or_else(|| "Social say requires message.".to_owned())
        }
        "social.yell" | "social.shout" => {
            no_target()?;
            if has_progression_fields()
                || data.ability_id.is_some()
                || data.channel.is_some()
                || data.duration_seconds.is_some()
                || data.reason.is_some()
                || data.role.is_some()
            {
                return Err(format!("Action {action:?} has unexpected fields."));
            }
            let message = data
                .message
                .clone()
                .ok_or_else(|| format!("Action {action:?} requires message."))?;
            Ok(PlayerAction::Social(if action == "social.yell" {
                SocialAction::Yell(message)
            } else {
                SocialAction::Shout(message)
            }))
        }
        "social.tell" => {
            let target = social_target()?;
            if has_progression_fields()
                || data.ability_id.is_some()
                || data.channel.is_some()
                || data.duration_seconds.is_some()
                || data.reason.is_some()
                || data.role.is_some()
            {
                return Err("Social tell has unexpected fields.".to_owned());
            }
            data.message
                .clone()
                .map(|message| SocialAction::Tell { target, message })
                .map(PlayerAction::Social)
                .ok_or_else(|| "Social tell requires message.".to_owned())
        }
        "social.telepathy" => {
            let target = social_target()?;
            if data.class_id.is_some()
                || data.graph_id.is_some()
                || data.node_id.is_some()
                || data.container.is_some()
                || data.recipient.is_some()
                || data.quantity.is_some()
                || data.page.is_some()
                || data.trade_id.is_some()
                || data.coins.is_some()
                || data.state_revision.is_some()
                || data.channel.is_some()
                || data.duration_seconds.is_some()
                || data.reason.is_some()
                || data.role.is_some()
                || data.report_id.is_some()
                || data.resolution.is_some()
                || !matches!(data.profile, NullableField::Missing)
                || !matches!(data.lore_title, NullableField::Missing)
                || data.capability.is_some()
                || data.enabled.is_some()
            {
                return Err("Social telepathy has unexpected fields.".to_owned());
            }
            let ability_id = data
                .ability_id
                .clone()
                .ok_or_else(|| "Social telepathy requires abilityId.".to_owned())?;
            let message = data
                .message
                .clone()
                .ok_or_else(|| "Social telepathy requires message.".to_owned())?;
            Ok(PlayerAction::Telepathy {
                ability_id,
                target,
                message,
            })
        }
        "social.emote.list" => Ok(PlayerAction::Social(SocialAction::EmoteList)),
        "social.emote" => Ok(PlayerAction::Social(SocialAction::Emote {
            command: data
                .emote
                .clone()
                .ok_or_else(|| "Social emote requires emote.".to_owned())?,
            target: data.target.clone(),
        })),
        "social.reply" => {
            no_target()?;
            if has_progression_fields()
                || data.ability_id.is_some()
                || data.channel.is_some()
                || data.duration_seconds.is_some()
                || data.reason.is_some()
                || data.role.is_some()
            {
                return Err("Social reply has unexpected fields.".to_owned());
            }
            data.message
                .clone()
                .map(SocialAction::Reply)
                .map(PlayerAction::Social)
                .ok_or_else(|| "Social reply requires message.".to_owned())
        }
        "social.channel.list" => {
            exact_no_fields().map(|()| PlayerAction::Social(SocialAction::ChannelList))
        }
        "social.channel.join" | "social.channel.leave" => {
            no_target()?;
            if has_progression_fields()
                || data.ability_id.is_some()
                || data.message.is_some()
                || data.duration_seconds.is_some()
                || data.reason.is_some()
                || data.role.is_some()
            {
                return Err("Social channel membership has unexpected fields.".to_owned());
            }
            let channel = data
                .channel
                .ok_or_else(|| "Social channel membership requires channel.".to_owned())?;
            Ok(PlayerAction::Social(if action == "social.channel.join" {
                SocialAction::ChannelJoin(channel)
            } else {
                SocialAction::ChannelLeave(channel)
            }))
        }
        "social.channel.say" => {
            no_target()?;
            if has_progression_fields()
                || data.ability_id.is_some()
                || data.duration_seconds.is_some()
                || data.reason.is_some()
                || data.role.is_some()
            {
                return Err("Social channel message has unexpected fields.".to_owned());
            }
            let channel = data
                .channel
                .ok_or_else(|| "Social channel message requires channel.".to_owned())?;
            let message = data
                .message
                .clone()
                .ok_or_else(|| "Social channel message requires message.".to_owned())?;
            Ok(PlayerAction::Social(SocialAction::ChannelSay {
                channel,
                message,
            }))
        }
        "social.who" => exact_no_fields().map(|()| PlayerAction::Social(SocialAction::Who)),
        "social.help" => {
            if has_progression_fields()
                || data.ability_id.is_some()
                || data.message.is_some()
                || data.channel.is_some()
                || data.duration_seconds.is_some()
                || data.reason.is_some()
                || data.role.is_some()
            {
                return Err("Social help has unexpected fields.".to_owned());
            }
            Ok(PlayerAction::Social(SocialAction::Help(
                data.target.clone(),
            )))
        }
        "social.ignore" | "social.unignore" | "social.block" | "social.unblock" => {
            let target = social_target()?;
            no_social_extras()?;
            let action = match action {
                "social.ignore" => SocialAction::Ignore(target),
                "social.unignore" => SocialAction::Unignore(target),
                "social.block" => SocialAction::Block(target),
                _ => SocialAction::Unblock(target),
            };
            Ok(PlayerAction::Social(action))
        }
        "social.report" => {
            let target = social_target()?;
            if has_progression_fields()
                || data.ability_id.is_some()
                || data.message.is_some()
                || data.channel.is_some()
                || data.duration_seconds.is_some()
                || data.role.is_some()
            {
                return Err("Social report has unexpected fields.".to_owned());
            }
            let reason = data
                .reason
                .clone()
                .ok_or_else(|| "Social report requires reason.".to_owned())?;
            Ok(PlayerAction::Social(SocialAction::Report {
                target,
                reason,
            }))
        }
        "staff.status" => exact_no_fields().map(|()| PlayerAction::Staff(StaffAction::Status)),
        "staff.reports" => {
            no_target()?;
            no_non_staff_fields()?;
            no_staff_extension_fields()?;
            if data.message.is_some() {
                return Err("Staff reports has unexpected fields.".to_owned());
            }
            Ok(PlayerAction::Staff(StaffAction::Reports {
                reason: staff_reason()?,
            }))
        }
        "staff.report.resolve" => {
            no_target()?;
            no_non_staff_fields()?;
            if data.message.is_some()
                || !matches!(data.profile, NullableField::Missing)
                || !matches!(data.lore_title, NullableField::Missing)
                || data.capability.is_some()
                || data.enabled.is_some()
            {
                return Err("Staff report resolution has unexpected fields.".to_owned());
            }
            let report_id = data
                .report_id
                .filter(|report_id| *report_id > 0)
                .ok_or_else(|| {
                    "Staff report resolution requires a positive reportId.".to_owned()
                })?;
            let resolution = data
                .resolution
                .ok_or_else(|| "Staff report resolution requires resolution.".to_owned())?;
            Ok(PlayerAction::Staff(StaffAction::ResolveReport {
                report_id,
                resolution,
                reason: staff_reason()?,
            }))
        }
        "staff.inspect" | "staff.where" | "staff.goto" | "staff.transfer" | "staff.restore" => {
            no_non_staff_fields()?;
            no_staff_extension_fields()?;
            if data.message.is_some() {
                return Err(format!("Action {action:?} has unexpected fields."));
            }
            let target = staff_target()?;
            let reason = staff_reason()?;
            let action = match action {
                "staff.inspect" => StaffAction::Inspect { target, reason },
                "staff.where" => StaffAction::Where { target, reason },
                "staff.goto" => StaffAction::Goto { target, reason },
                "staff.transfer" => StaffAction::Transfer { target, reason },
                _ => StaffAction::Restore { target, reason },
            };
            Ok(PlayerAction::Staff(action))
        }
        "staff.announce" => {
            no_target()?;
            no_non_staff_fields()?;
            no_staff_extension_fields()?;
            let message = data
                .message
                .clone()
                .ok_or_else(|| "Staff announcement requires message.".to_owned())?;
            Ok(PlayerAction::Staff(StaffAction::Announce {
                message,
                reason: staff_reason()?,
            }))
        }
        "staff.audit" => {
            no_target()?;
            no_non_staff_fields()?;
            no_staff_extension_fields()?;
            if data.message.is_some() {
                return Err("Staff audit has unexpected fields.".to_owned());
            }
            Ok(PlayerAction::Staff(StaffAction::Audit {
                reason: staff_reason()?,
            }))
        }
        "staff.profile.set" => {
            no_non_staff_fields()?;
            if data.message.is_some()
                || data.report_id.is_some()
                || data.resolution.is_some()
                || data.capability.is_some()
                || data.enabled.is_some()
            {
                return Err("Staff profile assignment has unexpected fields.".to_owned());
            }
            let profile = match &data.profile {
                NullableField::Missing => {
                    return Err("Staff profile assignment requires profile.".to_owned());
                }
                NullableField::Null => None,
                NullableField::Value(profile) => Some(*profile),
            };
            let lore_title = match &data.lore_title {
                NullableField::Missing => {
                    return Err("Staff profile assignment requires loreTitle.".to_owned());
                }
                NullableField::Null => None,
                NullableField::Value(title) => Some(title.clone()),
            };
            Ok(PlayerAction::Staff(StaffAction::SetProfile {
                target: staff_target()?,
                profile,
                lore_title,
                reason: staff_reason()?,
            }))
        }
        "staff.capability.set" => {
            no_non_staff_fields()?;
            if data.message.is_some()
                || data.report_id.is_some()
                || data.resolution.is_some()
                || !matches!(data.profile, NullableField::Missing)
                || !matches!(data.lore_title, NullableField::Missing)
            {
                return Err("Staff capability assignment has unexpected fields.".to_owned());
            }
            let capability = data
                .capability
                .ok_or_else(|| "Staff capability assignment requires capability.".to_owned())?;
            let enabled = data
                .enabled
                .ok_or_else(|| "Staff capability assignment requires enabled.".to_owned())?;
            Ok(PlayerAction::Staff(StaffAction::SetCapability {
                target: staff_target()?,
                capability,
                enabled,
                reason: staff_reason()?,
            }))
        }
        "moderation.mute" | "moderation.ban" => {
            let target = social_target()?;
            if has_progression_fields()
                || data.ability_id.is_some()
                || data.message.is_some()
                || data.channel.is_some()
                || data.role.is_some()
            {
                return Err("Moderation action has unexpected fields.".to_owned());
            }
            let reason = data
                .reason
                .clone()
                .ok_or_else(|| "Moderation action requires reason.".to_owned())?;
            if reason.len() > MAX_SOCIAL_REASON_BYTES {
                return Err("Moderation reason is too long.".to_owned());
            }
            let action = if action == "moderation.ban" {
                SocialAction::Ban {
                    target,
                    duration_seconds: data.duration_seconds,
                    reason,
                }
            } else {
                SocialAction::Mute {
                    target,
                    duration_seconds: data.duration_seconds,
                    reason,
                }
            };
            Ok(PlayerAction::Social(action))
        }
        "moderation.unmute" | "moderation.unban" => {
            let target = social_target()?;
            if has_progression_fields()
                || data.ability_id.is_some()
                || data.message.is_some()
                || data.channel.is_some()
                || data.duration_seconds.is_some()
                || data.role.is_some()
                || has_staff_fields
            {
                return Err("Moderation reversal has unexpected fields.".to_owned());
            }
            let reason = data
                .reason
                .clone()
                .ok_or_else(|| "Moderation reversal requires reason.".to_owned())?;
            if reason.len() > MAX_SOCIAL_REASON_BYTES {
                return Err("Moderation reason is too long.".to_owned());
            }
            Ok(PlayerAction::Social(if action == "moderation.unban" {
                SocialAction::Unban { target, reason }
            } else {
                SocialAction::Unmute { target, reason }
            }))
        }
        "moderation.role" => {
            let target = social_target()?;
            if has_progression_fields()
                || data.ability_id.is_some()
                || data.message.is_some()
                || data.channel.is_some()
                || data.duration_seconds.is_some()
                || has_staff_fields
            {
                return Err("Role action has unexpected fields.".to_owned());
            }
            let role = data
                .role
                .ok_or_else(|| "Role action requires role.".to_owned())?;
            let reason = data
                .reason
                .clone()
                .ok_or_else(|| "Role action requires reason.".to_owned())?;
            if reason.len() > MAX_SOCIAL_REASON_BYTES {
                return Err("Role reason is too long.".to_owned());
            }
            Ok(PlayerAction::Social(SocialAction::SetRole {
                target,
                role,
                reason,
            }))
        }
        _ => Err(format!("Unknown structured action: {action}")),
    }
}

fn structured_board_action(data: &ActionPerformData) -> Result<PlayerAction, String> {
    let action = data.action.as_str();
    let common_extras = data.emote.is_some()
        || data.mail_id.is_some()
        || data.ability_id.is_some()
        || data.class_id.is_some()
        || data.graph_id.is_some()
        || data.node_id.is_some()
        || data.target.is_some()
        || data.recipient.is_some()
        || data.container.is_some()
        || data.quantity.is_some()
        || data.trade_id.is_some()
        || data.coins.is_some()
        || data.state_revision.is_some()
        || data.message.is_some()
        || data.channel.is_some()
        || data.duration_seconds.is_some()
        || data.role.is_some()
        || data.report_id.is_some()
        || data.resolution.is_some()
        || !matches!(data.profile, NullableField::Missing)
        || !matches!(data.lore_title, NullableField::Missing)
        || data.capability.is_some()
        || data.enabled.is_some();
    if common_extras {
        return Err(format!("Action {action:?} has unexpected fields."));
    }
    match action {
        "board.catalog" => {
            if data.board.is_some()
                || data.post_id.is_some()
                || data.subject.is_some()
                || data.body.is_some()
                || data.page.is_some()
                || data.reason.is_some()
            {
                return Err("Board catalog has unexpected fields.".to_owned());
            }
            Ok(PlayerAction::Board(BoardAction::Catalog))
        }
        "board.list" => {
            if data.post_id.is_some()
                || data.subject.is_some()
                || data.body.is_some()
                || data.reason.is_some()
            {
                return Err("Board list has unexpected fields.".to_owned());
            }
            let board = data
                .board
                .ok_or_else(|| "Board list requires board.".to_owned())?;
            let page = data
                .page
                .ok_or_else(|| "Board list requires page.".to_owned())?;
            if !(1..=MAX_BOARD_PAGE).contains(&page) {
                return Err("Board page is not available.".to_owned());
            }
            Ok(PlayerAction::Board(BoardAction::List { board, page }))
        }
        "board.read" | "board.delete" => {
            if data.board.is_some()
                || data.subject.is_some()
                || data.body.is_some()
                || data.page.is_some()
                || data.reason.is_some()
            {
                return Err(format!("Action {action:?} has unexpected fields."));
            }
            let post_id = data
                .post_id
                .as_deref()
                .filter(|post_id| valid_board_public_id(post_id))
                .ok_or_else(|| "Board action requires a valid postId.".to_owned())?
                .to_owned();
            Ok(PlayerAction::Board(if action == "board.read" {
                BoardAction::Read { post_id }
            } else {
                BoardAction::Delete { post_id }
            }))
        }
        "board.post" => {
            if data.post_id.is_some() || data.page.is_some() || data.reason.is_some() {
                return Err("Board post has unexpected fields.".to_owned());
            }
            let board = data
                .board
                .ok_or_else(|| "Board post requires board.".to_owned())?;
            let subject = data
                .subject
                .as_deref()
                .filter(|value| valid_board_subject(value))
                .ok_or_else(|| "Board post requires subject.".to_owned())?;
            let body = data
                .body
                .as_deref()
                .filter(|value| valid_board_visible_text(value, MAX_BOARD_BODY_BYTES))
                .ok_or_else(|| "Board post requires body.".to_owned())?;
            Ok(PlayerAction::Board(BoardAction::Post {
                board,
                subject: subject.to_owned(),
                body: body.to_owned(),
            }))
        }
        "board.report" => {
            if data.board.is_some()
                || data.subject.is_some()
                || data.body.is_some()
                || data.page.is_some()
            {
                return Err("Board report has unexpected fields.".to_owned());
            }
            let post_id = data
                .post_id
                .as_deref()
                .filter(|post_id| valid_board_public_id(post_id))
                .ok_or_else(|| "Board report requires a valid postId.".to_owned())?;
            let reason = data
                .reason
                .as_deref()
                .filter(|reason| valid_board_visible_text(reason, MAX_BOARD_REPORT_REASON_BYTES))
                .ok_or_else(|| "Board report requires reason.".to_owned())?;
            Ok(PlayerAction::Board(BoardAction::Report {
                post_id: post_id.to_owned(),
                reason: reason.to_owned(),
            }))
        }
        _ => Err(format!("Unknown structured action: {action}")),
    }
}

fn structured_mail_action(data: &ActionPerformData) -> Result<PlayerAction, String> {
    let action = data.action.as_str();
    let common_extras = data.emote.is_some()
        || data.post_id.is_some()
        || data.board.is_some()
        || data.ability_id.is_some()
        || data.class_id.is_some()
        || data.graph_id.is_some()
        || data.node_id.is_some()
        || data.recipient.is_some()
        || data.container.is_some()
        || data.quantity.is_some()
        || data.trade_id.is_some()
        || data.coins.is_some()
        || data.state_revision.is_some()
        || data.message.is_some()
        || data.channel.is_some()
        || data.duration_seconds.is_some()
        || data.role.is_some()
        || data.report_id.is_some()
        || data.resolution.is_some()
        || !matches!(data.profile, NullableField::Missing)
        || !matches!(data.lore_title, NullableField::Missing)
        || data.capability.is_some()
        || data.enabled.is_some();
    if common_extras {
        return Err(format!("Action {action:?} has unexpected fields."));
    }
    match action {
        "mail.list" => {
            if data.target.is_some()
                || data.mail_id.is_some()
                || data.subject.is_some()
                || data.body.is_some()
                || data.reason.is_some()
            {
                return Err("Mail list has unexpected fields.".to_owned());
            }
            let page = data.page.unwrap_or(1);
            if !(1..=MAX_MAIL_PAGE).contains(&page) {
                return Err("Mail page is not available.".to_owned());
            }
            Ok(PlayerAction::Mail(MailAction::List { page }))
        }
        "mail.read" | "mail.delete" => {
            if data.target.is_some()
                || data.subject.is_some()
                || data.body.is_some()
                || data.page.is_some()
                || data.reason.is_some()
            {
                return Err(format!("Action {action:?} has unexpected fields."));
            }
            let mail_id = data
                .mail_id
                .as_deref()
                .filter(|mail_id| valid_mail_public_id(mail_id))
                .ok_or_else(|| "Mail action requires a valid mailId.".to_owned())?
                .to_owned();
            Ok(PlayerAction::Mail(if action == "mail.read" {
                MailAction::Read { mail_id }
            } else {
                MailAction::Delete { mail_id }
            }))
        }
        "mail.send" => {
            if data.mail_id.is_some() || data.page.is_some() || data.reason.is_some() {
                return Err("Mail send has unexpected fields.".to_owned());
            }
            let target = data
                .target
                .as_deref()
                .filter(|value| valid_mail_target(value))
                .ok_or_else(|| "Mail send requires target.".to_owned())?;
            let subject = data
                .subject
                .as_deref()
                .filter(|value| valid_mail_subject(value))
                .ok_or_else(|| "Mail send requires subject.".to_owned())?;
            let body = data
                .body
                .as_deref()
                .filter(|value| valid_mail_visible_text(value, MAX_MAIL_BODY_BYTES))
                .ok_or_else(|| "Mail send requires body.".to_owned())?;
            Ok(PlayerAction::Mail(MailAction::Send {
                target: target.to_owned(),
                subject: subject.to_owned(),
                body: body.to_owned(),
            }))
        }
        "mail.report" => {
            if data.target.is_some()
                || data.subject.is_some()
                || data.body.is_some()
                || data.page.is_some()
            {
                return Err("Mail report has unexpected fields.".to_owned());
            }
            let mail_id = data
                .mail_id
                .as_deref()
                .filter(|mail_id| valid_mail_public_id(mail_id))
                .ok_or_else(|| "Mail report requires a valid mailId.".to_owned())?;
            let reason = data
                .reason
                .as_deref()
                .filter(|reason| valid_mail_visible_text(reason, MAX_MAIL_REPORT_REASON_BYTES))
                .ok_or_else(|| "Mail report requires reason.".to_owned())?;
            Ok(PlayerAction::Mail(MailAction::Report {
                mail_id: mail_id.to_owned(),
                reason: reason.to_owned(),
            }))
        }
        _ => Err(format!("Unknown structured action: {action}")),
    }
}

fn public_game_error(error: &GameError) -> String {
    match error {
        GameError::AreaUnavailable | GameError::World(WorldError::AreaLoadRequired(_)) => {
            "The destination is not available right now.".to_owned()
        }
        GameError::AreaOperationBusy => {
            "That area is busy and cannot be retired or rehydrated right now.".to_owned()
        }
        GameError::AreaOperationInvalid => {
            "That area retirement or rehydration request is not valid.".to_owned()
        }
        GameError::MailRecipientUnavailable => "That mail recipient is not available.".to_owned(),
        GameError::GiveUnavailable => "That item transfer is not available.".to_owned(),
        GameError::World(WorldError::LootUnavailable) => "That loot is not available.".to_owned(),
        GameError::TradeUnavailable
        | GameError::World(
            WorldError::TradeUnavailable
            | WorldError::TradeTargetUnavailable
            | WorldError::ItemReservedForTrade
            | WorldError::StaleTradeRevision,
        ) => "That trade is not available.".to_owned(),
        GameError::World(WorldError::NoExit) => error.to_string(),
        GameError::World(WorldError::ItemNotFound(_))
        | GameError::World(WorldError::AmbiguousItem { .. })
        | GameError::World(WorldError::CharacterAlreadyConnected)
        | GameError::World(WorldError::GameplayUnavailable)
        | GameError::World(WorldError::AbilityNotKnown)
        | GameError::World(WorldError::AbilityUnavailable)
        | GameError::World(WorldError::InsufficientResource)
        | GameError::World(WorldError::InvalidAbilityTarget)
        | GameError::World(WorldError::EmoteUnavailable)
        | GameError::World(WorldError::EmoteTargetUnavailable)
        | GameError::World(WorldError::ClassNotFound)
        | GameError::World(WorldError::PromotionUnavailable)
        | GameError::World(WorldError::TalentUnavailable)
        | GameError::World(WorldError::InsufficientProgressionPoints)
        | GameError::World(WorldError::InventoryCapacityExceeded)
        | GameError::World(WorldError::ItemTransferForbidden) => error.to_string(),
        GameError::World(WorldError::GiveTargetUnavailable) => {
            "That item transfer is not available.".to_owned()
        }
        GameError::World(
            WorldError::PartyUnavailable
            | WorldError::PartyForbidden
            | WorldError::PartyTargetNotFound
            | WorldError::QuestUnavailable
            | WorldError::ShopUnavailable
            | WorldError::CommodityNotFound
            | WorldError::InsufficientFunds
            | WorldError::CommodityNotOwned,
        ) => error.to_string(),
        GameError::World(WorldError::UnknownSession) => "This session has expired.".to_owned(),
        GameError::Persistence(StoreError::SocialTargetNotFound) => {
            "No connected character matches that name.".to_owned()
        }
        GameError::Persistence(StoreError::AmbiguousSocialTarget) => {
            "That character name is ambiguous.".to_owned()
        }
        GameError::Persistence(StoreError::SocialSelfTarget) => {
            "You cannot target yourself with that action.".to_owned()
        }
        GameError::Persistence(StoreError::SocialForbidden) => {
            "Your account is not authorized for that social action.".to_owned()
        }
        GameError::Persistence(StoreError::StaffForbidden | StoreError::StaffHierarchy) => {
            "Your account is not authorized for that staff action.".to_owned()
        }
        GameError::Persistence(StoreError::StaffReportUnavailable(_)) => {
            "That report is not available for resolution.".to_owned()
        }
        GameError::Persistence(StoreError::ChatMuted) => {
            "You cannot send messages while your account is muted.".to_owned()
        }
        GameError::Persistence(StoreError::ChatRateLimited) => {
            "Chat rate limit exceeded; wait before sending another message.".to_owned()
        }
        GameError::Persistence(StoreError::InvalidSocialInput) => {
            "The social action is invalid or rate limited.".to_owned()
        }
        GameError::Persistence(StoreError::InvalidMailInput) => {
            "The mail action is invalid or exceeds its limits.".to_owned()
        }
        GameError::Persistence(StoreError::MailRateLimited) => {
            "Mail send quota exceeded; wait before sending more mail.".to_owned()
        }
        GameError::Persistence(StoreError::MailNotFound) => {
            "That mail is not available.".to_owned()
        }
        GameError::Persistence(StoreError::MailAlreadyReported) => {
            "That mail has already been reported.".to_owned()
        }
        GameError::Persistence(StoreError::MailReportLimit | StoreError::MailEvidenceLimit) => {
            "Mail reporting is temporarily unavailable.".to_owned()
        }
        GameError::Persistence(StoreError::InvalidBoardInput) => {
            "The board action is invalid or exceeds its limits.".to_owned()
        }
        GameError::Persistence(StoreError::BoardRateLimited) => {
            "Board post quota exceeded; wait before publishing more posts.".to_owned()
        }
        GameError::Persistence(StoreError::BoardCapacity) => {
            "That board is currently at capacity.".to_owned()
        }
        GameError::Persistence(StoreError::BoardPostUnavailable) => {
            "That board post is not available.".to_owned()
        }
        GameError::Persistence(StoreError::BoardAlreadyReported) => {
            "That board post has already been reported.".to_owned()
        }
        GameError::Persistence(StoreError::BoardReportLimit | StoreError::BoardEvidenceLimit) => {
            "Board reporting is temporarily unavailable.".to_owned()
        }
        GameError::Persistence(StoreError::InvalidStaffInput) => {
            "The staff action is invalid or exceeds its limits.".to_owned()
        }
        GameError::Persistence(StoreError::AccountBanned) => {
            "This account is suspended.".to_owned()
        }
        GameError::Persistence(StoreError::SessionExpired) => {
            "This session has expired. Sign in again.".to_owned()
        }
        GameError::Persistence(StoreError::InvalidSession | StoreError::SessionRevoked) => {
            "This session is no longer valid. Sign in again.".to_owned()
        }
        GameError::Persistence(StoreError::CharacterNotFound { .. }) => {
            "The selected character is no longer available.".to_owned()
        }
        GameError::CharacterContentIncompatible => {
            "This character needs a content migration; contact the server operator.".to_owned()
        }
        _ => "The world is temporarily unavailable.".to_owned(),
    }
}

fn access_rejection_code(error: &GameError) -> &'static str {
    match error {
        GameError::Persistence(StoreError::SessionExpired) => "access.session_expired",
        GameError::Persistence(StoreError::InvalidSession | StoreError::SessionRevoked) => {
            "access.session_invalid"
        }
        GameError::Persistence(StoreError::CharacterNotFound { .. }) => "access.character_invalid",
        GameError::World(WorldError::CharacterAlreadyConnected) => "access.character_connected",
        GameError::CharacterContentIncompatible => "access.character_incompatible",
        _ => "access.temporarily_unavailable",
    }
}

fn bearer_token(headers: &HeaderMap) -> Result<String, StatusCode> {
    let token = headers
        .get(header::AUTHORIZATION)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.strip_prefix("Bearer "))
        .filter(|token| token.len() == 64 && token.bytes().all(|byte| byte.is_ascii_hexdigit()));
    token.map(str::to_owned).ok_or(StatusCode::UNAUTHORIZED)
}

fn bearer_error(status: StatusCode) -> Response {
    (
        status,
        Json(json!({"error": "A valid bearer token is required."})),
    )
        .into_response()
}

fn allow_auth_attempt(state: &AppState, address: std::net::IpAddr) -> bool {
    let allowed = state.auth_rate_limiter.check(address, Instant::now());
    if !allowed {
        state.metrics.auth_rate_limited();
    }
    allowed
}

fn request_address(headers: &HeaderMap, remote: SocketAddr) -> std::net::IpAddr {
    if remote.ip().is_loopback()
        && let Some(forwarded) = headers
            .get("x-forwarded-for")
            .and_then(|value| value.to_str().ok())
            .and_then(|value| value.split(',').next())
            .and_then(|value| value.trim().parse().ok())
    {
        return forwarded;
    }
    remote.ip()
}

fn rate_limit_error() -> Response {
    (
        StatusCode::TOO_MANY_REQUESTS,
        [
            (header::RETRY_AFTER, "60"),
            (header::CONTENT_TYPE, "application/json"),
        ],
        Body::from(
            json!({"error": "Too many authentication attempts. Try again later."}).to_string(),
        ),
    )
        .into_response()
}

fn api_error(error: GameError) -> Response {
    let (status, message) = match &error {
        GameError::Persistence(StoreError::InvalidName { .. })
        | GameError::Persistence(StoreError::InvalidPassword) => {
            (StatusCode::BAD_REQUEST, error.to_string())
        }
        GameError::Persistence(StoreError::AccountAlreadyExists) => {
            (StatusCode::CONFLICT, error.to_string())
        }
        GameError::Persistence(StoreError::InvalidCredentials)
        | GameError::Persistence(StoreError::InvalidSession)
        | GameError::Persistence(StoreError::SessionExpired)
        | GameError::Persistence(StoreError::SessionRevoked) => {
            (StatusCode::UNAUTHORIZED, error.to_string())
        }
        GameError::Persistence(StoreError::CharacterNotFound { .. })
        | GameError::Persistence(StoreError::AccountNotFound(_)) => (
            StatusCode::NOT_FOUND,
            "The requested identity was not found.".to_owned(),
        ),
        GameError::World(WorldError::CharacterAlreadyConnected) => {
            (StatusCode::CONFLICT, error.to_string())
        }
        _ => (
            StatusCode::SERVICE_UNAVAILABLE,
            "The service is temporarily unavailable.".to_owned(),
        ),
    };
    if status.is_server_error() {
        warn!(%error, "API request failed");
    }
    (status, Json(json!({"error": message}))).into_response()
}

fn websocket_request_allowed(headers: &HeaderMap) -> bool {
    let supports_protocol = headers
        .get_all(header::SEC_WEBSOCKET_PROTOCOL)
        .iter()
        .filter_map(|value| value.to_str().ok())
        .flat_map(|value| value.split(','))
        .any(|protocol| protocol.trim() == "tiamud.v1");
    if !supports_protocol {
        return false;
    }

    let Some(origin) = headers.get(header::ORIGIN) else {
        // Non-browser clients may omit Origin; the bearer credential remains
        // mandatory inside client.hello.
        return true;
    };
    let (Ok(origin), Some(host)) = (
        origin.to_str().unwrap_or_default().parse::<Uri>(),
        headers
            .get(header::HOST)
            .and_then(|host| host.to_str().ok()),
    ) else {
        return false;
    };
    matches!(origin.scheme_str(), Some("http" | "https"))
        && origin
            .authority()
            .is_some_and(|authority| authority.as_str().eq_ignore_ascii_case(host))
}

mod story;

#[cfg(test)]
#[path = "web_protocol_golden_tests.rs"]
mod protocol_golden_tests;

#[cfg(test)]
mod tests {
    use super::*;

    fn requested_action(message: &str) -> RequestedAction {
        let ClientMessage::Action(action) = parse_client_message(message).unwrap() else {
            panic!("message must parse as an action");
        };
        action
    }

    pub(super) fn test_entity_presentation() -> crate::runtime_content::ScenePresentation {
        let session = SessionId(8_800);
        let mut world = tiamud_core::World::demo();
        world
            .connect(
                session,
                tiamud_core::CharacterSnapshot {
                    id: CharacterId(8_801),
                    account_id: tiamud_core::AccountId(8_802),
                    name: "Presentation Test".into(),
                    room_id: "demo:lantern-square".into(),
                    item_locations: BTreeMap::new(),
                },
            )
            .unwrap();
        let hash = "a".repeat(64);
        let media = world_pack::EntityMediaDefinition {
            kind: world_pack::EntityMediaKind::Item,
            subject_id: "demo:blue-lantern".into(),
            alt: world_pack::LocalizedText(BTreeMap::from([(
                "en".into(),
                "A blue lantern".into(),
            )])),
            decorative: false,
            footprint: world_pack::NormalizedFootprint {
                width_milli: 180,
                height_milli: 420,
            },
            preferred_anchor: None,
            priority: 10,
            layer: world_pack::EntityMediaLayer::Foreground,
            poses: world_pack::EntitySpritePoses {
                idle: world_pack::EntitySpriteSource {
                    webp: Some(format!("assets/{hash}/sprite-idle-640.webp")),
                    png: Some(format!("assets/{hash}/sprite-idle-640.png")),
                },
                attack: None,
                hit: None,
                defeated: None,
            },
        };
        crate::runtime_content::RuntimeContent::default()
            .with_entity_media(vec![media])
            .resolve_entity_presentation(&world, session)
    }

    fn test_player_state() -> PlayerStateView {
        let session = SessionId(8_810);
        let mut world = tiamud_core::World::demo();
        world
            .connect(
                session,
                tiamud_core::CharacterSnapshot {
                    id: CharacterId(8_811),
                    account_id: tiamud_core::AccountId(8_812),
                    name: "Player State Test".into(),
                    room_id: "demo:lantern-square".into(),
                    item_locations: BTreeMap::new(),
                },
            )
            .unwrap();
        world.player_state_view(session).unwrap()
    }

    fn test_scene() -> SceneView {
        let session = SessionId(8_820);
        let mut world = tiamud_core::World::demo();
        world
            .connect(
                session,
                tiamud_core::CharacterSnapshot {
                    id: CharacterId(8_821),
                    account_id: tiamud_core::AccountId(8_822),
                    name: "Scene Test".into(),
                    room_id: "demo:lantern-square".into(),
                    item_locations: BTreeMap::new(),
                },
            )
            .unwrap();
        world.current_scene(session).unwrap()
    }

    #[test]
    fn character_creation_uses_clean_v3_fields_and_preserves_background_intent() {
        let defaults: CreateCharacterRequest = serde_json::from_str(r#"{"name":"Aria"}"#).unwrap();
        assert_eq!(defaults.name, "Aria");
        assert_eq!(defaults.lineage_id, None);
        assert_eq!(defaults.background, CharacterBackgroundSelection::Default);
        assert_eq!(defaults.base_class_id, None);

        let none: CreateCharacterRequest =
            serde_json::from_str(r#"{"name":"Aria","backgroundId":null}"#).unwrap();
        assert_eq!(none.background, CharacterBackgroundSelection::None);

        let selected: CreateCharacterRequest = serde_json::from_str(
            r#"{"name":"Aria","lineageId":"stonekin","backgroundId":"guard","baseClassId":"vanguard"}"#,
        )
        .unwrap();
        assert_eq!(selected.lineage_id.as_deref(), Some("stonekin"));
        assert_eq!(
            selected.background,
            CharacterBackgroundSelection::Id("guard".into())
        );
        assert_eq!(selected.base_class_id.as_deref(), Some("vanguard"));
        assert!(
            serde_json::from_str::<CreateCharacterRequest>(
                r#"{"name":"Aria","base_class_id":"vanguard"}"#
            )
            .is_err()
        );
        assert!(
            serde_json::from_str::<CreateCharacterRequest>(
                r#"{"name":"Aria","archetypeId":"vanguard"}"#
            )
            .is_err()
        );
    }

    #[test]
    fn readiness_reports_the_selected_storage_backend() {
        assert_eq!(
            readiness_body(StatusCode::OK, true, true, "postgresql"),
            json!({
                "status": "ready",
                "world": true,
                "storage": { "backend": "postgresql", "ready": true }
            })
        );
    }

    #[test]
    fn hello_requires_authentication_and_the_current_version() {
        let token = "ab".repeat(32);
        let hello = format!(
            r#"{{"v":1,"type":"client.hello","data":{{"client":"web","capabilities":{{"images":["avif","webp","jpeg"],"reducedMotion":false,"audio":true}},"auth":{{"token":"{token}","characterId":7}}}}}}"#
        );
        assert_eq!(
            parse_client_hello(&hello),
            Some(ClientHelloCredentials {
                token,
                character_id: 7,
                audio: true,
            })
        );
        assert!(parse_client_hello(r#"{"v":1,"type":"client.hello","data":{}}"#).is_none());
        assert!(
            parse_client_hello(
                r#"{"v":2,"type":"client.hello","data":{"auth":{"token":"bad","characterId":7}}}"#
            )
            .is_none()
        );
        let no_images = format!(
            r#"{{"v":1,"type":"client.hello","data":{{"client":"web","capabilities":{{"images":[],"reducedMotion":true,"audio":false}},"auth":{{"token":"{}","characterId":7}}}}}}"#,
            "ab".repeat(32)
        );
        assert!(parse_client_hello(&no_images).is_some());

        let additive_capability = format!(
            r#"{{"v":1,"type":"client.hello","data":{{"client":"web","capabilities":{{"images":[],"reducedMotion":true,"audio":false,"sceneSprites":true}} ,"auth":{{"token":"{}","characterId":7}}}}}}"#,
            "ab".repeat(32)
        );
        assert!(parse_client_hello(&additive_capability).is_some());

        for malformed_capabilities in [
            r#"{"images":"avif","reducedMotion":false,"audio":false}"#,
            r#"{"images":[],"reducedMotion":"no","audio":false}"#,
        ] {
            let hello = format!(
                r#"{{"v":1,"type":"client.hello","data":{{"client":"web","capabilities":{malformed_capabilities},"auth":{{"token":"{}","characterId":7}}}}}}"#,
                "ab".repeat(32)
            );
            assert!(parse_client_hello(&hello).is_none());
        }
        let malformed_auth = format!(
            r#"{{"v":1,"type":"client.hello","data":{{"client":"web","capabilities":{{"images":[],"reducedMotion":false,"audio":false}},"auth":{{"token":"{}","characterId":"7"}}}}}}"#,
            "ab".repeat(32)
        );
        assert!(parse_client_hello(&malformed_auth).is_none());
        let duplicate_auth = format!(
            r#"{{"v":1,"type":"client.hello","data":{{"client":"web","capabilities":{{"images":[],"reducedMotion":false,"audio":false}},"auth":{{"token":"{}","characterId":7,"characterId":8}}}}}}"#,
            "a".repeat(64),
        );
        assert!(parse_client_hello(&duplicate_auth).is_none());
    }

    #[test]
    fn browser_websockets_must_be_same_origin_and_use_the_protocol() {
        let mut headers = HeaderMap::new();
        headers.insert(header::HOST, "tiamud.example:8443".parse().unwrap());
        headers.insert(
            header::SEC_WEBSOCKET_PROTOCOL,
            "chat, tiamud.v1".parse().unwrap(),
        );
        headers.insert(
            header::ORIGIN,
            "https://tiamud.example:8443".parse().unwrap(),
        );
        assert!(websocket_request_allowed(&headers));

        headers.insert(header::ORIGIN, "ftp://tiamud.example:8443".parse().unwrap());
        assert!(!websocket_request_allowed(&headers));
        headers.insert(header::ORIGIN, "https://evil.example".parse().unwrap());
        assert!(!websocket_request_allowed(&headers));
        headers.remove(header::ORIGIN);
        headers.remove(header::SEC_WEBSOCKET_PROTOCOL);
        assert!(!websocket_request_allowed(&headers));
    }

    #[test]
    fn only_content_addressed_static_assets_are_immutable() {
        assert!(immutable_asset_path("assets/index-CtsH2LhL.js"));
        assert!(immutable_asset_path("assets/index-DCFEe-LW.js"));
        assert!(immutable_asset_path(
            "assets/0000000000000000000000000000000000000000000000000000000000000000/manifest.json"
        ));
        assert!(!immutable_asset_path("index.html"));
        assert!(!immutable_asset_path("assets/favicon.svg"));
    }

    #[test]
    fn internal_world_errors_are_not_exposed_to_clients() {
        assert_eq!(
            public_game_error(&GameError::AreaUnavailable),
            "The destination is not available right now."
        );
        assert_eq!(
            public_game_error(&GameError::World(WorldError::InvalidEntity)),
            "The world is temporarily unavailable."
        );
        assert_eq!(
            public_game_error(&GameError::World(WorldError::NoExit)),
            "There is no exit in that direction."
        );
        assert_eq!(
            public_game_error(&GameError::World(WorldError::InsufficientResource)),
            "There are not enough resources to use that ability."
        );
        assert_eq!(
            public_game_error(&GameError::World(WorldError::InventoryCapacityExceeded)),
            "That item would exceed inventory slot or weight capacity."
        );
        assert_eq!(
            public_game_error(&GameError::World(WorldError::ItemTransferForbidden)),
            "That item cannot change ownership."
        );
        assert_eq!(
            public_game_error(&GameError::GiveUnavailable),
            "That item transfer is not available."
        );
        assert_eq!(
            public_game_error(&GameError::World(WorldError::StaleTradeRevision)),
            "That trade is not available."
        );
        assert_eq!(
            public_game_error(&GameError::World(WorldError::LootUnavailable)),
            "That loot is not available."
        );
        assert_eq!(
            public_game_error(&GameError::World(WorldError::EmoteTargetUnavailable)),
            "That social-emote target is not available."
        );
    }

    #[test]
    fn websocket_entry_rejections_distinguish_credentials_from_capacity() {
        assert_eq!(
            access_rejection_code(&GameError::Persistence(StoreError::SessionExpired)),
            "access.session_expired"
        );
        assert_eq!(
            access_rejection_code(&GameError::Persistence(StoreError::InvalidSession)),
            "access.session_invalid"
        );
        assert_eq!(
            access_rejection_code(&GameError::World(WorldError::CharacterAlreadyConnected)),
            "access.character_connected"
        );
        assert_eq!(
            access_rejection_code(&GameError::CharacterContentIncompatible),
            "access.character_incompatible"
        );
        assert_eq!(
            public_game_error(&GameError::CharacterContentIncompatible),
            "This character needs a content migration; contact the server operator."
        );
        assert_eq!(
            access_rejection_code(&GameError::ActorStopped),
            "access.temporarily_unavailable"
        );
        assert_eq!(
            public_game_error(&GameError::Persistence(StoreError::SessionExpired)),
            "This session has expired. Sign in again."
        );
    }

    #[test]
    fn forwarded_address_is_only_trusted_from_the_loopback_proxy() {
        let mut headers = HeaderMap::new();
        headers.insert("x-forwarded-for", "203.0.113.9, 127.0.0.1".parse().unwrap());
        assert_eq!(
            request_address(&headers, "127.0.0.1:1234".parse().unwrap()),
            "203.0.113.9".parse::<std::net::IpAddr>().unwrap()
        );
        assert_eq!(
            request_address(&headers, "198.51.100.4:1234".parse().unwrap()),
            "198.51.100.4".parse::<std::net::IpAddr>().unwrap()
        );
    }

    #[test]
    fn auth_rate_limit_response_tells_clients_when_to_retry() {
        let response = rate_limit_error();
        assert_eq!(response.status(), StatusCode::TOO_MANY_REQUESTS);
        assert_eq!(response.headers()[header::RETRY_AFTER], "60");
    }

    #[test]
    fn client_envelopes_are_strict_and_keep_command_compatibility() {
        let ClientMessage::Action(command) = parse_client_message(
            r#"{"v":1,"type":"command.submit","data":{"requestId":"request-1","command":"north"}}"#,
        )
        .unwrap() else {
            panic!("command.submit must parse as an action");
        };
        assert_eq!(command.request_id, "request-1");
        assert!(matches!(command.action, Ok(PlayerAction::Move(_))));

        let ClientMessage::Action(action) = parse_client_message(
            r#"{"v":1,"type":"action.perform","data":{"requestId":"request-2","action":"item.take","target":"blue lantern"}}"#,
        )
        .unwrap()
        else {
            panic!("action.perform must parse as an action");
        };
        assert_eq!(
            action.action,
            Ok(PlayerAction::Take(ItemSelection::one("blue lantern")))
        );

        let ClientMessage::Action(put) = parse_client_message(
            r#"{"v":1,"type":"action.perform","data":{"requestId":"request-put","action":"item.put","target":"o1111111111111111","container":"o2222222222222222","quantity":12}}"#,
        )
        .unwrap()
        else {
            panic!("structured item.put must parse as an action");
        };
        assert_eq!(
            put.action,
            Ok(PlayerAction::Put {
                selection: ItemSelection {
                    query: "o1111111111111111".into(),
                    quantity: 12,
                },
                container: "o2222222222222222".into(),
            })
        );
        let ClientMessage::Action(contents) = parse_client_message(
            r#"{"v":1,"type":"action.perform","data":{"requestId":"request-contents","action":"item.contents","container":"o2222222222222222","page":2}}"#,
        )
        .unwrap()
        else {
            panic!("structured item.contents must parse as an action");
        };
        assert_eq!(
            contents.action,
            Ok(PlayerAction::Contents {
                container: "o2222222222222222".into(),
                page: 2,
            })
        );
        let ClientMessage::Action(give) = parse_client_message(
            r#"{"v":1,"type":"action.perform","data":{"requestId":"request-give","action":"item.give","target":"o1111111111111111","recipient":"o3333333333333333","quantity":2}}"#,
        )
        .unwrap()
        else {
            panic!("structured item.give must parse as an action");
        };
        assert_eq!(
            give.action,
            Ok(PlayerAction::Give {
                selection: ItemSelection {
                    query: "o1111111111111111".into(),
                    quantity: 2,
                },
                recipient: "o3333333333333333".into(),
            })
        );
        let loot_status = requested_action(
            r#"{"v":1,"type":"action.perform","data":{"requestId":"loot-status","action":"loot.status"}}"#,
        );
        assert_eq!(
            loot_status.action,
            Ok(PlayerAction::Loot(LootAction::Status))
        );
        let loot_pass = requested_action(
            r#"{"v":1,"type":"action.perform","data":{"requestId":"loot-pass","action":"loot.pass","target":"o5555555555555555"}}"#,
        );
        assert_eq!(
            loot_pass.action,
            Ok(PlayerAction::Loot(LootAction::Pass(
                "o5555555555555555".into()
            )))
        );
        assert_ne!(loot_status.signature, loot_pass.signature);
        let reward_open = requested_action(
            r#"{"v":1,"type":"action.perform","data":{"requestId":"reward-open","action":"reward.open","target":"o6666666666666666"}}"#,
        );
        assert_eq!(
            reward_open.action,
            Ok(PlayerAction::OpenReward("o6666666666666666".into()))
        );
        let reward_odds = requested_action(
            r#"{"v":1,"type":"action.perform","data":{"requestId":"reward-odds","action":"reward.odds","target":"o6666666666666666","page":2}}"#,
        );
        assert_eq!(
            reward_odds.action,
            Ok(PlayerAction::RewardOdds {
                target: "o6666666666666666".into(),
                page: 2,
            })
        );
        assert_ne!(reward_open.signature, reward_odds.signature);
        assert_ne!(action.signature, put.signature);
        let signatures = [
            r#"{"v":1,"type":"action.perform","data":{"requestId":"same","action":"item.put","target":"o1111111111111111","container":"o2222222222222222","quantity":12}}"#,
            r#"{"v":1,"type":"action.perform","data":{"requestId":"same","action":"item.put","target":"o1111111111111111","container":"o2222222222222222","quantity":13}}"#,
            r#"{"v":1,"type":"action.perform","data":{"requestId":"same","action":"item.put","target":"o1111111111111111","container":"o3333333333333333","quantity":12}}"#,
            r#"{"v":1,"type":"action.perform","data":{"requestId":"same","action":"item.contents","container":"o2222222222222222","page":1}}"#,
            r#"{"v":1,"type":"action.perform","data":{"requestId":"same","action":"item.contents","container":"o2222222222222222","page":2}}"#,
            r#"{"v":1,"type":"action.perform","data":{"requestId":"same","action":"item.give","target":"o1111111111111111","recipient":"o3333333333333333","quantity":1}}"#,
            r#"{"v":1,"type":"action.perform","data":{"requestId":"same","action":"item.give","target":"o1111111111111111","recipient":"o4444444444444444","quantity":1}}"#,
            r#"{"v":1,"type":"action.perform","data":{"requestId":"same","action":"item.give","target":"o1111111111111111","recipient":"o3333333333333333","quantity":2}}"#,
        ]
        .map(|raw| match parse_client_message(raw).unwrap() {
            ClientMessage::Action(action) => action.signature,
            _ => panic!("structured inventory request must parse as an action"),
        });
        assert_eq!(
            signatures
                .iter()
                .collect::<std::collections::BTreeSet<_>>()
                .len(),
            signatures.len()
        );

        for invalid in [
            r#"{"v":1,"type":"action.perform","data":{"requestId":"bad-item-1","action":"item.take","target":"tonic","quantity":0}}"#,
            r#"{"v":1,"type":"action.perform","data":{"requestId":"bad-item-2","action":"item.drop","target":"tonic","quantity":257}}"#,
            r#"{"v":1,"type":"action.perform","data":{"requestId":"bad-item-3","action":"item.contents","container":"bag","page":0}}"#,
            r#"{"v":1,"type":"action.perform","data":{"requestId":"bad-give-1","action":"item.give","target":"tonic","recipient":"Bram","quantity":0}}"#,
            r#"{"v":1,"type":"action.perform","data":{"requestId":"bad-give-2","action":"item.give","target":"tonic","recipient":"Bram","quantity":257}}"#,
            r#"{"v":1,"type":"action.perform","data":{"requestId":"bad-reward-page","action":"reward.odds","target":"reward","page":0}}"#,
        ] {
            assert_eq!(
                parse_client_message(invalid).unwrap_err(),
                ClientMessageError::InvalidEnvelope
            );
        }
        for null_field in [
            r#"{"v":1,"type":"action.perform","data":{"requestId":"null","action":"trade.request","recipient":null}}"#,
            r#"{"v":1,"type":"action.perform","data":{"requestId":"null","action":"trade.accept","tradeId":null}}"#,
            r#"{"v":1,"type":"action.perform","data":{"requestId":"null","action":"trade.accept","tradeId":"o2222222222222222","page":null}}"#,
            r#"{"v":1,"type":"action.perform","data":{"requestId":"null","action":"trade.item.set","tradeId":"o2222222222222222","target":null,"quantity":1}}"#,
            r#"{"v":1,"type":"action.perform","data":{"requestId":"null","action":"trade.item.set","tradeId":"o2222222222222222","target":"o3333333333333333","quantity":null}}"#,
            r#"{"v":1,"type":"action.perform","data":{"requestId":"null","action":"trade.item.remove","tradeId":"o2222222222222222","target":null}}"#,
            r#"{"v":1,"type":"action.perform","data":{"requestId":"null","action":"trade.coins.set","tradeId":"o2222222222222222","coins":null}}"#,
            r#"{"v":1,"type":"action.perform","data":{"requestId":"null","action":"trade.contents","tradeId":"o2222222222222222","target":"o3333333333333333","page":null}}"#,
            r#"{"v":1,"type":"action.perform","data":{"requestId":"null","action":"trade.confirm","tradeId":"o2222222222222222","stateRevision":null}}"#,
            r#"{"v":1,"type":"action.perform","data":{"requestId":"null","action":"trade.status","tradeId":"o2222222222222222","message":null}}"#,
        ] {
            assert_eq!(
                parse_client_message(null_field).unwrap_err(),
                ClientMessageError::InvalidEnvelope
            );
        }
        for invalid_fields in [
            r#"{"v":1,"type":"action.perform","data":{"requestId":"give-missing-recipient","action":"item.give","target":"tonic"}}"#,
            r#"{"v":1,"type":"action.perform","data":{"requestId":"give-extra","action":"item.give","target":"tonic","recipient":"Bram","message":"smuggled"}}"#,
            r#"{"v":1,"type":"action.perform","data":{"requestId":"drop-recipient","action":"item.drop","target":"tonic","recipient":"Bram"}}"#,
            r#"{"v":1,"type":"action.perform","data":{"requestId":"loot-status-target","action":"loot.status","target":"tonic"}}"#,
            r#"{"v":1,"type":"action.perform","data":{"requestId":"loot-pass-missing","action":"loot.pass"}}"#,
            r#"{"v":1,"type":"action.perform","data":{"requestId":"loot-pass-extra","action":"loot.pass","target":"tonic","message":"smuggled"}}"#,
            r#"{"v":1,"type":"action.perform","data":{"requestId":"reward-open-page","action":"reward.open","target":"parcel","page":1}}"#,
            r#"{"v":1,"type":"action.perform","data":{"requestId":"reward-odds-missing","action":"reward.odds","page":1}}"#,
            r#"{"v":1,"type":"action.perform","data":{"requestId":"reward-odds-extra","action":"reward.odds","target":"parcel","quantity":1}}"#,
        ] {
            let ClientMessage::Action(action) = parse_client_message(invalid_fields).unwrap()
            else {
                panic!("well-formed action must return a structured action error");
            };
            assert!(action.action.is_err());
        }
        let ClientMessage::Action(extra_item_field) = parse_client_message(
            r#"{"v":1,"type":"action.perform","data":{"requestId":"bad-item-extra","action":"look","quantity":1}}"#,
        )
        .unwrap()
        else {
            panic!("well-formed invalid structured action must produce a result");
        };
        assert!(extra_item_field.action.is_err());

        let ClientMessage::Action(ability) = parse_client_message(
            r#"{"v":1,"type":"action.perform","data":{"requestId":"request-ability","action":"combat.ability","abilityId":"ember-bolt","target":"guardian"}}"#,
        )
        .unwrap()
        else {
            panic!("structured combat ability must parse");
        };
        assert_eq!(
            ability.action,
            Ok(PlayerAction::Ability {
                ability_id: "ember-bolt".into(),
                target: Some("guardian".into()),
            })
        );

        let ClientMessage::Action(promotion) = parse_client_message(
            r#"{"v":1,"type":"action.perform","data":{"requestId":"request-promote","action":"progression.promote","classId":"tiamud:bastion"}}"#,
        )
        .unwrap()
        else {
            panic!("structured class promotion must parse");
        };
        assert_eq!(
            promotion.action,
            Ok(PlayerAction::Promote {
                class_id: "tiamud:bastion".into(),
            })
        );

        let ClientMessage::Action(talent) = parse_client_message(
            r#"{"v":1,"type":"action.perform","data":{"requestId":"request-talent","action":"progression.talent","graphId":"tiamud:martial-talents","nodeId":"tiamud:bulwark"}}"#,
        )
        .unwrap()
        else {
            panic!("structured talent acquisition must parse");
        };
        assert_eq!(
            talent.action,
            Ok(PlayerAction::AcquireTalent {
                graph_id: "tiamud:martial-talents".into(),
                node_id: "tiamud:bulwark".into(),
            })
        );

        let ClientMessage::Action(incomplete_talent) = parse_client_message(
            r#"{"v":1,"type":"action.perform","data":{"requestId":"request-talent-missing","action":"progression.talent","graphId":"tiamud:martial-talents"}}"#,
        )
        .unwrap()
        else {
            panic!("invalid structured talent must remain an action result");
        };
        assert!(incomplete_talent.action.is_err());

        let ClientMessage::Action(missing_ability) = parse_client_message(
            r#"{"v":1,"type":"action.perform","data":{"requestId":"request-ability-missing","action":"combat.ability","target":"guardian"}}"#,
        )
        .unwrap()
        else {
            panic!("invalid structured ability must remain an action result");
        };
        assert!(missing_ability.action.is_err());

        let ClientMessage::Action(unexpected_ability) = parse_client_message(
            r#"{"v":1,"type":"action.perform","data":{"requestId":"request-ability-extra","action":"look","abilityId":"ember-bolt"}}"#,
        )
        .unwrap()
        else {
            panic!("unexpected abilityId must remain an action result");
        };
        assert!(unexpected_ability.action.is_err());

        let oversized_ability = format!(
            r#"{{"v":1,"type":"action.perform","data":{{"requestId":"request-ability-long","action":"combat.ability","abilityId":"{}"}}}}"#,
            "a".repeat(MAX_ABILITY_ID_BYTES + 1)
        );
        assert_eq!(
            parse_client_message(&oversized_ability).unwrap_err(),
            ClientMessageError::InvalidEnvelope
        );
        assert_eq!(
            parse_client_message(
                r#"{"v":1,"type":"action.perform","data":{"requestId":"request-ability-invalid","action":"combat.ability","abilityId":"Ember Bolt"}}"#
            )
            .unwrap_err(),
            ClientMessageError::InvalidEnvelope
        );

        let ClientMessage::Action(social) = parse_client_message(
            r#"{"v":1,"type":"action.perform","data":{"requestId":"request-3","action":"social.tell","target":"Aria","message":"Meet at the gate."}}"#,
        )
        .unwrap()
        else {
            panic!("structured social action must parse");
        };
        assert_eq!(
            social.action,
            Ok(PlayerAction::Social(SocialAction::Tell {
                target: "Aria".into(),
                message: "Meet at the gate.".into(),
            }))
        );

        let ClientMessage::Action(party) = parse_client_message(
            r#"{"v":1,"type":"action.perform","data":{"requestId":"request-party","action":"party.invite","target":"Aria"}}"#,
        )
        .unwrap()
        else {
            panic!("structured party action must parse");
        };
        assert_eq!(
            party.action,
            Ok(PlayerAction::Party(PartyAction::Invite("Aria".into())))
        );

        let ClientMessage::Action(quest) = parse_client_message(
            r#"{"v":1,"type":"action.perform","data":{"requestId":"request-quest","action":"quest.accept","target":"first-hunt"}}"#,
        )
        .unwrap()
        else {
            panic!("structured quest action must parse");
        };
        assert_eq!(
            quest.action,
            Ok(PlayerAction::Quest(QuestAction::Accept(
                "first-hunt".into()
            )))
        );

        let ClientMessage::Action(economy) = parse_client_message(
            r#"{"v":1,"type":"action.perform","data":{"requestId":"request-buy","action":"economy.buy","target":"tonic"}}"#,
        )
        .unwrap()
        else {
            panic!("structured economy action must parse");
        };
        assert_eq!(
            economy.action,
            Ok(PlayerAction::Economy(EconomyAction::Buy("tonic".into())))
        );

        let ClientMessage::Action(moderation) = parse_client_message(
            r#"{"v":1,"type":"action.perform","data":{"requestId":"request-4","action":"moderation.ban","target":"Griefer","durationSeconds":3600,"reason":"repeated abuse"}}"#,
        )
        .unwrap()
        else {
            panic!("structured moderation action must parse");
        };
        assert_eq!(
            moderation.action,
            Ok(PlayerAction::Social(SocialAction::Ban {
                target: "Griefer".into(),
                duration_seconds: Some(3_600),
                reason: "repeated abuse".into(),
            }))
        );

        let ClientMessage::Action(invalid) = parse_client_message(
            r#"{"v":1,"type":"action.perform","data":{"requestId":"request-5","action":"look","message":"secret extra"}}"#,
        )
        .unwrap()
        else {
            panic!("invalid structured fields still produce an action result");
        };
        assert!(invalid.action.is_err());

        assert_eq!(
            parse_client_message(
                r#"{"v":1,"type":"sync.request","data":{"afterSeq":9,"unexpected":true}}"#
            )
            .unwrap_err(),
            ClientMessageError::InvalidEnvelope
        );
        assert_eq!(
            parse_client_message(
                r#"{"v":1,"type":"sync.request","unexpected":true,"data":{"afterSeq":9}}"#
            )
            .unwrap_err(),
            ClientMessageError::MalformedJson
        );
        assert_eq!(
            parse_client_message(r#"{"v":2,"type":"sync.request","data":{"afterSeq":9}}"#)
                .unwrap_err(),
            ClientMessageError::UnsupportedVersion
        );
        assert_eq!(
            parse_client_message(r#"{"v":1,"type":"unknown","data":{}}"#).unwrap_err(),
            ClientMessageError::UnknownType
        );
    }

    #[test]
    fn structured_spatial_party_reply_and_channel_actions_map_exactly() {
        let cases = [
            (
                r#"{"v":1,"type":"action.perform","data":{"requestId":"yell","action":"social.yell","message":"The gate is open."}}"#,
                PlayerAction::Social(SocialAction::Yell("The gate is open.".into())),
            ),
            (
                r#"{"v":1,"type":"action.perform","data":{"requestId":"shout","action":"social.shout","message":"Return to the square!"}}"#,
                PlayerAction::Social(SocialAction::Shout("Return to the square!".into())),
            ),
            (
                r#"{"v":1,"type":"action.perform","data":{"requestId":"party-disband","action":"party.disband"}}"#,
                PlayerAction::Party(PartyAction::Disband),
            ),
            (
                r#"{"v":1,"type":"action.perform","data":{"requestId":"party-revoke","action":"party.revoke","target":"Aria"}}"#,
                PlayerAction::Party(PartyAction::Revoke("Aria".into())),
            ),
            (
                r#"{"v":1,"type":"action.perform","data":{"requestId":"party-kick","action":"party.kick","target":"Aria"}}"#,
                PlayerAction::Party(PartyAction::Kick("Aria".into())),
            ),
            (
                r#"{"v":1,"type":"action.perform","data":{"requestId":"party-transfer","action":"party.transfer","target":"Aria"}}"#,
                PlayerAction::Party(PartyAction::Transfer("Aria".into())),
            ),
            (
                r#"{"v":1,"type":"action.perform","data":{"requestId":"party-say","action":"party.say","message":"Hold the line."}}"#,
                PlayerAction::Social(SocialAction::PartySay("Hold the line.".into())),
            ),
            (
                r#"{"v":1,"type":"action.perform","data":{"requestId":"reply","action":"social.reply","message":"On my way."}}"#,
                PlayerAction::Social(SocialAction::Reply("On my way.".into())),
            ),
            (
                r#"{"v":1,"type":"action.perform","data":{"requestId":"channel-list","action":"social.channel.list"}}"#,
                PlayerAction::Social(SocialAction::ChannelList),
            ),
            (
                r#"{"v":1,"type":"action.perform","data":{"requestId":"channel-join","action":"social.channel.join","channel":"general"}}"#,
                PlayerAction::Social(SocialAction::ChannelJoin(SocialChannelName::General)),
            ),
            (
                r#"{"v":1,"type":"action.perform","data":{"requestId":"channel-leave","action":"social.channel.leave","channel":"newbie"}}"#,
                PlayerAction::Social(SocialAction::ChannelLeave(SocialChannelName::Newbie)),
            ),
            (
                r#"{"v":1,"type":"action.perform","data":{"requestId":"channel-say","action":"social.channel.say","channel":"staff","message":"Status clear."}}"#,
                PlayerAction::Social(SocialAction::ChannelSay {
                    channel: SocialChannelName::Staff,
                    message: "Status clear.".into(),
                }),
            ),
        ];

        for (message, expected) in cases {
            assert_eq!(requested_action(message).action, Ok(expected));
        }
    }

    #[test]
    fn structured_staff_actions_map_exactly() {
        let cases = [
            (
                r#"{"v":1,"type":"action.perform","data":{"requestId":"staff-status","action":"staff.status"}}"#,
                PlayerAction::Staff(StaffAction::Status),
            ),
            (
                r#"{"v":1,"type":"action.perform","data":{"requestId":"staff-reports","action":"staff.reports","reason":"weekly review"}}"#,
                PlayerAction::Staff(StaffAction::Reports {
                    reason: "weekly review".into(),
                }),
            ),
            (
                r#"{"v":1,"type":"action.perform","data":{"requestId":"staff-resolve","action":"staff.report.resolve","reportId":7,"resolution":"action_taken","reason":"evidence verified"}}"#,
                PlayerAction::Staff(StaffAction::ResolveReport {
                    report_id: 7,
                    resolution: StaffReportResolution::ActionTaken,
                    reason: "evidence verified".into(),
                }),
            ),
            (
                r#"{"v":1,"type":"action.perform","data":{"requestId":"staff-inspect","action":"staff.inspect","target":"Aria","reason":"support request"}}"#,
                PlayerAction::Staff(StaffAction::Inspect {
                    target: "Aria".into(),
                    reason: "support request".into(),
                }),
            ),
            (
                r#"{"v":1,"type":"action.perform","data":{"requestId":"staff-where","action":"staff.where","target":"Aria","reason":"locate incident"}}"#,
                PlayerAction::Staff(StaffAction::Where {
                    target: "Aria".into(),
                    reason: "locate incident".into(),
                }),
            ),
            (
                r#"{"v":1,"type":"action.perform","data":{"requestId":"staff-goto","action":"staff.goto","target":"Aria","reason":"respond to incident"}}"#,
                PlayerAction::Staff(StaffAction::Goto {
                    target: "Aria".into(),
                    reason: "respond to incident".into(),
                }),
            ),
            (
                r#"{"v":1,"type":"action.perform","data":{"requestId":"staff-transfer","action":"staff.transfer","target":"Aria","reason":"unstick character"}}"#,
                PlayerAction::Staff(StaffAction::Transfer {
                    target: "Aria".into(),
                    reason: "unstick character".into(),
                }),
            ),
            (
                r#"{"v":1,"type":"action.perform","data":{"requestId":"staff-restore","action":"staff.restore","target":"Aria","reason":"recover after incident"}}"#,
                PlayerAction::Staff(StaffAction::Restore {
                    target: "Aria".into(),
                    reason: "recover after incident".into(),
                }),
            ),
            (
                r#"{"v":1,"type":"action.perform","data":{"requestId":"staff-announce","action":"staff.announce","message":"The shard will restart soon.","reason":"planned maintenance"}}"#,
                PlayerAction::Staff(StaffAction::Announce {
                    message: "The shard will restart soon.".into(),
                    reason: "planned maintenance".into(),
                }),
            ),
            (
                r#"{"v":1,"type":"action.perform","data":{"requestId":"staff-audit","action":"staff.audit","reason":"weekly review"}}"#,
                PlayerAction::Staff(StaffAction::Audit {
                    reason: "weekly review".into(),
                }),
            ),
            (
                r#"{"v":1,"type":"action.perform","data":{"requestId":"staff-profile","action":"staff.profile.set","target":"Aria","profile":"moderator","loreTitle":"Warden","reason":"approved promotion"}}"#,
                PlayerAction::Staff(StaffAction::SetProfile {
                    target: "Aria".into(),
                    profile: Some(StaffProfileKey::Moderator),
                    lore_title: Some("Warden".into()),
                    reason: "approved promotion".into(),
                }),
            ),
            (
                r#"{"v":1,"type":"action.perform","data":{"requestId":"staff-profile-clear","action":"staff.profile.set","target":"Aria","profile":null,"loreTitle":null,"reason":"assignment ended"}}"#,
                PlayerAction::Staff(StaffAction::SetProfile {
                    target: "Aria".into(),
                    profile: None,
                    lore_title: None,
                    reason: "assignment ended".into(),
                }),
            ),
            (
                r#"{"v":1,"type":"action.perform","data":{"requestId":"staff-capability","action":"staff.capability.set","target":"Aria","capability":"reports.resolve","enabled":true,"reason":"temporary duty"}}"#,
                PlayerAction::Staff(StaffAction::SetCapability {
                    target: "Aria".into(),
                    capability: StaffCapability::ReportsResolve,
                    enabled: true,
                    reason: "temporary duty".into(),
                }),
            ),
        ];

        for (message, expected) in cases {
            assert_eq!(requested_action(message).action, Ok(expected), "{message}");
        }
    }

    #[test]
    fn structured_staff_actions_are_strict_and_bounded() {
        for message in [
            r#"{"v":1,"type":"action.perform","data":{"requestId":"staff-extra","action":"staff.status","enabled":true}}"#,
            r#"{"v":1,"type":"action.perform","data":{"requestId":"staff-zero-report","action":"staff.report.resolve","reportId":0,"resolution":"dismissed","reason":"invalid id"}}"#,
            r#"{"v":1,"type":"action.perform","data":{"requestId":"staff-missing-profile","action":"staff.profile.set","target":"Aria","loreTitle":null,"reason":"incomplete"}}"#,
            r#"{"v":1,"type":"action.perform","data":{"requestId":"staff-missing-lore","action":"staff.profile.set","target":"Aria","profile":null,"reason":"incomplete"}}"#,
            r#"{"v":1,"type":"action.perform","data":{"requestId":"staff-missing-enabled","action":"staff.capability.set","target":"Aria","capability":"reports.view","reason":"incomplete"}}"#,
        ] {
            assert!(requested_action(message).action.is_err(), "{message}");
        }

        assert_eq!(
            parse_client_message(
                r#"{"v":1,"type":"action.perform","data":{"requestId":"staff-bad-resolution","action":"staff.report.resolve","reportId":1,"resolution":"closed","reason":"invalid state"}}"#,
            )
            .unwrap_err(),
            ClientMessageError::InvalidEnvelope
        );

        let oversized_target = format!(
            r#"{{"v":1,"type":"action.perform","data":{{"requestId":"staff-long-target","action":"staff.inspect","target":"{}","reason":"review"}}}}"#,
            "a".repeat(MAX_STAFF_TARGET_BYTES + 1)
        );
        assert!(requested_action(&oversized_target).action.is_err());

        let oversized_reason = format!(
            r#"{{"v":1,"type":"action.perform","data":{{"requestId":"staff-long-reason","action":"staff.audit","reason":"{}"}}}}"#,
            "a".repeat(MAX_STAFF_REASON_BYTES + 1)
        );
        assert!(requested_action(&oversized_reason).action.is_err());

        let oversized_lore = format!(
            r#"{{"v":1,"type":"action.perform","data":{{"requestId":"staff-long-lore","action":"staff.profile.set","target":"Aria","profile":"helper","loreTitle":"{}","reason":"review"}}}}"#,
            "a".repeat(MAX_STAFF_LORE_TITLE_BYTES + 1)
        );
        assert_eq!(
            parse_client_message(&oversized_lore).unwrap_err(),
            ClientMessageError::InvalidEnvelope
        );

        let oversized_message = format!(
            r#"{{"v":1,"type":"action.perform","data":{{"requestId":"staff-long-message","action":"staff.announce","message":"{}","reason":"review"}}}}"#,
            "a".repeat(MAX_CHAT_MESSAGE_BYTES + 1)
        );
        assert_eq!(
            parse_client_message(&oversized_message).unwrap_err(),
            ClientMessageError::InvalidEnvelope
        );
    }

    #[test]
    fn structured_moderation_reversals_and_roles_require_reasons() {
        let cases = [
            (
                r#"{"v":1,"type":"action.perform","data":{"requestId":"unmute","action":"moderation.unmute","target":"Aria","reason":"appeal accepted"}}"#,
                PlayerAction::Social(SocialAction::Unmute {
                    target: "Aria".into(),
                    reason: "appeal accepted".into(),
                }),
            ),
            (
                r#"{"v":1,"type":"action.perform","data":{"requestId":"unban","action":"moderation.unban","target":"Aria","reason":"appeal accepted"}}"#,
                PlayerAction::Social(SocialAction::Unban {
                    target: "Aria".into(),
                    reason: "appeal accepted".into(),
                }),
            ),
            (
                r#"{"v":1,"type":"action.perform","data":{"requestId":"role","action":"moderation.role","target":"Aria","role":"moderator","reason":"approved assignment"}}"#,
                PlayerAction::Social(SocialAction::SetRole {
                    target: "Aria".into(),
                    role: AccountRole::Moderator,
                    reason: "approved assignment".into(),
                }),
            ),
        ];
        for (message, expected) in cases {
            assert_eq!(requested_action(message).action, Ok(expected), "{message}");
        }

        for message in [
            r#"{"v":1,"type":"action.perform","data":{"requestId":"unmute-no-reason","action":"moderation.unmute","target":"Aria"}}"#,
            r#"{"v":1,"type":"action.perform","data":{"requestId":"unban-no-reason","action":"moderation.unban","target":"Aria"}}"#,
            r#"{"v":1,"type":"action.perform","data":{"requestId":"role-no-reason","action":"moderation.role","target":"Aria","role":"moderator"}}"#,
        ] {
            assert!(requested_action(message).action.is_err(), "{message}");
        }
    }

    #[test]
    fn structured_social_actions_reject_unexpected_fields_and_sign_channels() {
        for message in [
            r#"{"v":1,"type":"action.perform","data":{"requestId":"extra-party-say","action":"party.say","target":"Aria","message":"Hello"}}"#,
            r#"{"v":1,"type":"action.perform","data":{"requestId":"extra-reply","action":"social.reply","target":"Aria","message":"Hello"}}"#,
            r#"{"v":1,"type":"action.perform","data":{"requestId":"extra-list","action":"social.channel.list","channel":"general"}}"#,
            r#"{"v":1,"type":"action.perform","data":{"requestId":"extra-join","action":"social.channel.join","channel":"general","message":"Hello"}}"#,
            r#"{"v":1,"type":"action.perform","data":{"requestId":"extra-channel-say","action":"social.channel.say","channel":"general","target":"Aria","message":"Hello"}}"#,
            r#"{"v":1,"type":"action.perform","data":{"requestId":"extra-tell","action":"social.tell","target":"Aria","channel":"general","message":"Hello"}}"#,
        ] {
            assert!(requested_action(message).action.is_err(), "{message}");
        }

        for (message, expected) in [
            (
                r#"{"v":1,"type":"action.perform","data":{"requestId":"missing-yell","action":"social.yell"}}"#,
                ClientMessageError::InvalidEnvelope,
            ),
            (
                r#"{"v":1,"type":"action.perform","data":{"requestId":"null-yell","action":"social.yell","message":null}}"#,
                ClientMessageError::InvalidEnvelope,
            ),
            (
                r#"{"v":1,"type":"action.perform","data":{"requestId":"extra-shout","action":"social.shout","message":"Hello","target":"Aria"}}"#,
                ClientMessageError::InvalidEnvelope,
            ),
            (
                r#"{"v":1,"type":"action.perform","data":{"requestId":"duplicate-shout","action":"social.shout","message":"Hello","message":"Again"}}"#,
                ClientMessageError::MalformedJson,
            ),
        ] {
            assert_eq!(
                parse_client_message(message).unwrap_err(),
                expected,
                "{message}",
            );
        }

        assert_eq!(
            parse_client_message(
                r#"{"v":1,"type":"action.perform","data":{"requestId":"unknown-field","action":"party.disband","unexpected":true}}"#,
            )
            .unwrap_err(),
            ClientMessageError::InvalidEnvelope
        );
        assert_eq!(
            parse_client_message(
                r#"{"v":1,"type":"action.perform","data":{"requestId":"unknown-channel","action":"social.channel.join","channel":"trade"}}"#,
            )
            .unwrap_err(),
            ClientMessageError::InvalidEnvelope
        );

        let general = requested_action(
            r#"{"v":1,"type":"action.perform","data":{"requestId":"same-request","action":"social.channel.say","channel":"general","message":"Hello"}}"#,
        );
        let newbie = requested_action(
            r#"{"v":1,"type":"action.perform","data":{"requestId":"same-request","action":"social.channel.say","channel":"newbie","message":"Hello"}}"#,
        );
        assert_eq!(general.request_id, newbie.request_id);
        assert_ne!(general.signature, newbie.signature);
    }

    #[test]
    fn structured_mail_actions_accept_only_the_exact_private_wire_shapes() {
        let mail_id = "m0123456789abcdef0123456789abcdef";
        let cases = [
            (
                r#"{"v":1,"type":"action.perform","data":{"requestId":"mail-list","action":"mail.list"}}"#,
                PlayerAction::Mail(MailAction::List { page: 1 }),
            ),
            (
                r#"{"v":1,"type":"action.perform","data":{"requestId":"mail-list-7","action":"mail.list","page":7}}"#,
                PlayerAction::Mail(MailAction::List { page: 7 }),
            ),
            (
                r#"{"v":1,"type":"action.perform","data":{"requestId":"mail-read","action":"mail.read","mailId":"m0123456789abcdef0123456789abcdef"}}"#,
                PlayerAction::Mail(MailAction::Read {
                    mail_id: mail_id.into(),
                }),
            ),
            (
                r#"{"v":1,"type":"action.perform","data":{"requestId":"mail-delete","action":"mail.delete","mailId":"m0123456789abcdef0123456789abcdef"}}"#,
                PlayerAction::Mail(MailAction::Delete {
                    mail_id: mail_id.into(),
                }),
            ),
            (
                r#"{"v":1,"type":"action.perform","data":{"requestId":"mail-send","action":"mail.send","target":"Aria","subject":"Plan <&>","body":"Meet | beyond the gate."}}"#,
                PlayerAction::Mail(MailAction::Send {
                    target: "Aria".into(),
                    subject: "Plan <&>".into(),
                    body: "Meet | beyond the gate.".into(),
                }),
            ),
            (
                r#"{"v":1,"type":"action.perform","data":{"requestId":"mail-report","action":"mail.report","mailId":"m0123456789abcdef0123456789abcdef","reason":"Unwanted contact"}}"#,
                PlayerAction::Mail(MailAction::Report {
                    mail_id: mail_id.into(),
                    reason: "Unwanted contact".into(),
                }),
            ),
        ];

        for (message, expected) in cases {
            assert_eq!(requested_action(message).action, Ok(expected), "{message}");
        }

        for message in [
            r#"{"v":1,"type":"action.perform","data":{"requestId":"mail-null-page","action":"mail.list","page":null}}"#,
            r#"{"v":1,"type":"action.perform","data":{"requestId":"mail-page-zero","action":"mail.list","page":0}}"#,
            r#"{"v":1,"type":"action.perform","data":{"requestId":"mail-page-eight","action":"mail.list","page":8}}"#,
            r#"{"v":1,"type":"action.perform","data":{"requestId":"mail-read-null","action":"mail.read","mailId":null}}"#,
            r#"{"v":1,"type":"action.perform","data":{"requestId":"mail-read-extra","action":"mail.read","mailId":"m0123456789abcdef0123456789abcdef","body":null}}"#,
            r#"{"v":1,"type":"action.perform","data":{"requestId":"mail-send-missing","action":"mail.send","target":"Aria","subject":"Plan"}}"#,
            r#"{"v":1,"type":"action.perform","data":{"requestId":"mail-send-extra","action":"mail.send","target":"Aria","subject":"Plan","body":"Meet","message":null}}"#,
            r#"{"v":1,"type":"action.perform","data":{"requestId":"mail-report-extra","action":"mail.report","mailId":"m0123456789abcdef0123456789abcdef","reason":"Abuse","target":null}}"#,
            r#"{"v":1,"type":"action.perform","data":{"requestId":"mail-bad-id","action":"mail.read","mailId":"m0123456789ABCDEF0123456789abcdef"}}"#,
            r#"{"v":1,"type":"action.perform","data":{"requestId":"mail-bidi","action":"mail.send","target":"Aria","subject":"safe\u202etxt","body":"Meet"}}"#,
            r#"{"v":1,"type":"action.perform","data":{"requestId":"mail-smuggle","action":"look","mailId":null}}"#,
        ] {
            assert_eq!(
                parse_client_message(message).unwrap_err(),
                ClientMessageError::InvalidEnvelope,
                "{message}"
            );
        }

        let oversized_body = serde_json::json!({
            "v": 1,
            "type": "action.perform",
            "data": {
                "requestId": "mail-oversized-utf8",
                "action": "mail.send",
                "target": "Aria",
                "subject": "Plan",
                "body": "é".repeat(MAX_MAIL_BODY_BYTES / 2 + 1),
            }
        });
        assert_eq!(
            parse_client_message(&oversized_body.to_string()).unwrap_err(),
            ClientMessageError::InvalidEnvelope
        );
    }

    #[test]
    fn structured_board_actions_accept_only_the_exact_bounded_wire_shapes() {
        let post_id = "b0123456789abcdef0123456789abcdef";
        let cases = [
            (
                r#"{"v":1,"type":"action.perform","data":{"requestId":"board-catalog","action":"board.catalog"}}"#,
                PlayerAction::Board(BoardAction::Catalog),
            ),
            (
                r#"{"v":1,"type":"action.perform","data":{"requestId":"board-list","action":"board.list","board":"general","page":1}}"#,
                PlayerAction::Board(BoardAction::List {
                    board: BoardKey::General,
                    page: 1,
                }),
            ),
            (
                r#"{"v":1,"type":"action.perform","data":{"requestId":"board-list-100","action":"board.list","board":"newbie","page":100}}"#,
                PlayerAction::Board(BoardAction::List {
                    board: BoardKey::Newbie,
                    page: 100,
                }),
            ),
            (
                r#"{"v":1,"type":"action.perform","data":{"requestId":"board-read","action":"board.read","postId":"b0123456789abcdef0123456789abcdef"}}"#,
                PlayerAction::Board(BoardAction::Read {
                    post_id: post_id.into(),
                }),
            ),
            (
                r#"{"v":1,"type":"action.perform","data":{"requestId":"board-delete","action":"board.delete","postId":"b0123456789abcdef0123456789abcdef"}}"#,
                PlayerAction::Board(BoardAction::Delete {
                    post_id: post_id.into(),
                }),
            ),
            (
                r#"{"v":1,"type":"action.perform","data":{"requestId":"board-post","action":"board.post","board":"general","subject":"Plan <&>","body":"Meet | beyond the gate."}}"#,
                PlayerAction::Board(BoardAction::Post {
                    board: BoardKey::General,
                    subject: "Plan <&>".into(),
                    body: "Meet | beyond the gate.".into(),
                }),
            ),
            (
                r#"{"v":1,"type":"action.perform","data":{"requestId":"board-report","action":"board.report","postId":"b0123456789abcdef0123456789abcdef","reason":"Abuse | repeated"}}"#,
                PlayerAction::Board(BoardAction::Report {
                    post_id: post_id.into(),
                    reason: "Abuse | repeated".into(),
                }),
            ),
        ];

        for (message, expected) in cases {
            assert_eq!(requested_action(message).action, Ok(expected), "{message}");
        }

        for message in [
            r#"{"v":1,"type":"action.perform","data":{"requestId":"board-catalog-extra","action":"board.catalog","board":null}}"#,
            r#"{"v":1,"type":"action.perform","data":{"requestId":"board-list-missing-page","action":"board.list","board":"general"}}"#,
            r#"{"v":1,"type":"action.perform","data":{"requestId":"board-list-null","action":"board.list","board":"general","page":null}}"#,
            r#"{"v":1,"type":"action.perform","data":{"requestId":"board-list-zero","action":"board.list","board":"general","page":0}}"#,
            r#"{"v":1,"type":"action.perform","data":{"requestId":"board-list-101","action":"board.list","board":"general","page":101}}"#,
            r#"{"v":1,"type":"action.perform","data":{"requestId":"board-list-unknown","action":"board.list","board":"staff"}}"#,
            r#"{"v":1,"type":"action.perform","data":{"requestId":"board-read-null","action":"board.read","postId":null}}"#,
            r#"{"v":1,"type":"action.perform","data":{"requestId":"board-read-uppercase","action":"board.read","postId":"b0123456789ABCDEF0123456789abcdef"}}"#,
            r#"{"v":1,"type":"action.perform","data":{"requestId":"board-post-pipe","action":"board.post","board":"general","subject":"bad | subject","body":"Body"}}"#,
            r#"{"v":1,"type":"action.perform","data":{"requestId":"board-post-control","action":"board.post","board":"general","subject":"Subject","body":"line one\nline two"}}"#,
            r#"{"v":1,"type":"action.perform","data":{"requestId":"board-report-bidi","action":"board.report","postId":"b0123456789abcdef0123456789abcdef","reason":"safe\u202eunsafe"}}"#,
            r#"{"v":1,"type":"action.perform","data":{"requestId":"board-report-extra","action":"board.report","postId":"b0123456789abcdef0123456789abcdef","reason":"Abuse","target":null}}"#,
            r#"{"v":1,"type":"action.perform","data":{"requestId":"board-smuggle","action":"look","postId":null}}"#,
        ] {
            assert_eq!(
                parse_client_message(message).unwrap_err(),
                ClientMessageError::InvalidEnvelope,
                "{message}"
            );
        }

        for message in [
            r#"{"v":1,"type":"action.perform","data":{"requestId":"board-duplicate","action":"board.list","board":"general","board":"newbie","page":1}}"#,
            r#"{"v":1,"type":"action.perform","data":{"requestId":"board-duplicate-page","action":"board.list","board":"general","page":1,"page":2}}"#,
            r#"{"v":1,"type":"action.perform","data":{"requestId":"first","requestId":"second","action":"board.catalog"}}"#,
        ] {
            assert_eq!(
                parse_client_message(message).unwrap_err(),
                ClientMessageError::MalformedJson,
                "{message}"
            );
        }

        let oversized_body = serde_json::json!({
            "v": 1,
            "type": "action.perform",
            "data": {
                "requestId": "board-oversized-utf8",
                "action": "board.post",
                "board": "general",
                "subject": "Plan",
                "body": "é".repeat(MAX_BOARD_BODY_BYTES / 2 + 1),
            }
        });
        assert_eq!(
            parse_client_message(&oversized_body.to_string()).unwrap_err(),
            ClientMessageError::InvalidEnvelope
        );

        let general = requested_action(
            r#"{"v":1,"type":"action.perform","data":{"requestId":"same-request","action":"board.list","board":"general","page":1}}"#,
        );
        let newbie = requested_action(
            r#"{"v":1,"type":"action.perform","data":{"requestId":"same-request","action":"board.list","board":"newbie","page":1}}"#,
        );
        assert_ne!(general.signature, newbie.signature);
    }

    #[test]
    fn structured_telepathy_accepts_only_the_exact_wire_fields() {
        let exact = r#"{"v":1,"type":"action.perform","data":{"requestId":"telepathy-1","action":"social.telepathy","abilityId":"tiamud:mind-whisper","target":"Aria","message":"Meet beyond the gate."}}"#;
        assert_eq!(
            requested_action(exact).action,
            Ok(PlayerAction::Telepathy {
                ability_id: "tiamud:mind-whisper".into(),
                target: "Aria".into(),
                message: "Meet beyond the gate.".into(),
            })
        );

        for message in [
            r#"{"v":1,"type":"action.perform","data":{"requestId":"telepathy-no-ability","action":"social.telepathy","target":"Aria","message":"Hello"}}"#,
            r#"{"v":1,"type":"action.perform","data":{"requestId":"telepathy-no-target","action":"social.telepathy","abilityId":"tiamud:mind-whisper","message":"Hello"}}"#,
            r#"{"v":1,"type":"action.perform","data":{"requestId":"telepathy-no-message","action":"social.telepathy","abilityId":"tiamud:mind-whisper","target":"Aria"}}"#,
        ] {
            assert_eq!(
                parse_client_message(message).unwrap_err(),
                ClientMessageError::InvalidEnvelope,
                "{message}"
            );
        }

        for extra in [
            "classId",
            "graphId",
            "nodeId",
            "recipient",
            "container",
            "quantity",
            "page",
            "tradeId",
            "coins",
            "stateRevision",
            "channel",
            "durationSeconds",
            "reason",
            "role",
            "reportId",
            "resolution",
            "profile",
            "loreTitle",
            "capability",
            "enabled",
            "unknownFutureField",
        ] {
            let mut message = serde_json::json!({
                "v": 1,
                "type": "action.perform",
                "data": {
                    "requestId": "telepathy-extra-null",
                    "action": "social.telepathy",
                    "abilityId": "tiamud:mind-whisper",
                    "target": "Aria",
                    "message": "Hello"
                }
            });
            message["data"]
                .as_object_mut()
                .expect("test action data is an object")
                .insert(extra.to_owned(), Value::Null);
            assert_eq!(
                parse_client_message(&message.to_string()).unwrap_err(),
                ClientMessageError::InvalidEnvelope,
                "extra field {extra} must fail even when null"
            );
        }

        assert_eq!(
            parse_client_message(
                r#"{"v":1,"type":"action.perform","data":{"requestId":"telepathy-extra","action":"social.telepathy","abilityId":"tiamud:mind-whisper","target":"Aria","message":"Hello","quantity":1}}"#,
            )
            .unwrap_err(),
            ClientMessageError::InvalidEnvelope
        );
    }

    #[test]
    fn structured_emotes_accept_only_the_exact_optional_target_wire_shapes() {
        let list = r#"{"v":1,"type":"action.perform","data":{"requestId":"emotes","action":"social.emote.list"}}"#;
        assert_eq!(
            requested_action(list).action,
            Ok(PlayerAction::Social(SocialAction::EmoteList))
        );

        let untargeted = r#"{"v":1,"type":"action.perform","data":{"requestId":"wave","action":"social.emote","emote":"wave"}}"#;
        assert_eq!(
            requested_action(untargeted).action,
            Ok(PlayerAction::Social(SocialAction::Emote {
                command: "wave".into(),
                target: None,
            }))
        );
        let targeted = r#"{"v":1,"type":"action.perform","data":{"requestId":"salute","action":"social.emote","emote":"salute","target":"Aria"}}"#;
        assert_eq!(
            requested_action(targeted).action,
            Ok(PlayerAction::Social(SocialAction::Emote {
                command: "salute".into(),
                target: Some("Aria".into()),
            }))
        );
        assert_ne!(
            requested_action(untargeted).signature,
            requested_action(targeted).signature
        );
        let authored_name_characters = r#"{"v":1,"type":"action.perform","data":{"requestId":"ampersand-name","action":"social.emote","emote":"salute","target":"A&B"}}"#;
        assert_eq!(
            requested_action(authored_name_characters).action,
            Ok(PlayerAction::Social(SocialAction::Emote {
                command: "salute".into(),
                target: Some("A&B".into()),
            }))
        );

        for message in [
            r#"{"v":1,"type":"action.perform","data":{"requestId":"missing","action":"social.emote"}}"#,
            r#"{"v":1,"type":"action.perform","data":{"requestId":"null-emote","action":"social.emote","emote":null}}"#,
            r#"{"v":1,"type":"action.perform","data":{"requestId":"null-target","action":"social.emote","emote":"wave","target":null}}"#,
            r#"{"v":1,"type":"action.perform","data":{"requestId":"extra","action":"social.emote","emote":"wave","message":null}}"#,
            r#"{"v":1,"type":"action.perform","data":{"requestId":"list-extra","action":"social.emote.list","target":null}}"#,
            r#"{"v":1,"type":"action.perform","data":{"requestId":"foreign-extra","action":"look","emote":null}}"#,
            r#"{"v":1,"type":"action.perform","data":{"requestId":"uppercase","action":"social.emote","emote":"Smile"}}"#,
            r#"{"v":1,"type":"action.perform","data":{"requestId":"non-ascii","action":"social.emote","emote":"sonreír"}}"#,
            r#"{"v":1,"type":"action.perform","data":{"requestId":"too-long","action":"social.emote","emote":"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"}}"#,
            r#"{"v":1,"type":"action.perform","data":{"requestId":"target-too-long","action":"social.emote","emote":"smile","target":"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"}}"#,
        ] {
            assert_eq!(
                parse_client_message(message).unwrap_err(),
                ClientMessageError::InvalidEnvelope,
                "{message}"
            );
        }
    }

    #[test]
    fn sync_and_heartbeat_messages_validate_their_values() {
        assert!(matches!(
            parse_client_message(r#"{"v":1,"type":"sync.request","data":{"afterSeq":15}}"#),
            Ok(ClientMessage::Sync { after_seq: 15 })
        ));
        assert!(matches!(
            parse_client_message(r#"{"v":1,"type":"client.heartbeat","data":{"nonce":7}}"#),
            Ok(ClientMessage::Heartbeat { nonce: 7 })
        ));
        assert_eq!(
            parse_client_message(r#"{"v":1,"type":"client.heartbeat","data":{"nonce":0}}"#)
                .unwrap_err(),
            ClientMessageError::InvalidEnvelope
        );
    }

    #[test]
    fn trade_actions_have_closed_forms_complete_signatures_and_reject_present_nulls() {
        let cases = [
            (
                r#"{"v":1,"type":"action.perform","data":{"requestId":"trade-request","action":"trade.request","recipient":"o1111111111111111"}}"#,
                PlayerAction::Trade(TradeAction::Request("o1111111111111111".into())),
                None,
            ),
            (
                r#"{"v":1,"type":"action.perform","data":{"requestId":"trade-accept","action":"trade.accept","tradeId":"o2222222222222222"}}"#,
                PlayerAction::Trade(TradeAction::Accept),
                Some("o2222222222222222"),
            ),
            (
                r#"{"v":1,"type":"action.perform","data":{"requestId":"trade-item","action":"trade.item.set","tradeId":"o2222222222222222","target":"o3333333333333333","quantity":128}}"#,
                PlayerAction::Trade(TradeAction::OfferItem(ItemSelection {
                    query: "o3333333333333333".into(),
                    quantity: 128,
                })),
                Some("o2222222222222222"),
            ),
            (
                r#"{"v":1,"type":"action.perform","data":{"requestId":"trade-coins","action":"trade.coins.set","tradeId":"o2222222222222222","coins":1000000000}}"#,
                PlayerAction::Trade(TradeAction::OfferCoins(1_000_000_000)),
                Some("o2222222222222222"),
            ),
            (
                r#"{"v":1,"type":"action.perform","data":{"requestId":"trade-contents","action":"trade.contents","tradeId":"o2222222222222222","target":"o3333333333333333","page":2}}"#,
                PlayerAction::Trade(TradeAction::Contents {
                    container: "o3333333333333333".into(),
                    page: 2,
                }),
                Some("o2222222222222222"),
            ),
            (
                r#"{"v":1,"type":"action.perform","data":{"requestId":"trade-confirm","action":"trade.confirm","tradeId":"o2222222222222222","stateRevision":9}}"#,
                PlayerAction::Trade(TradeAction::Confirm { state_revision: 9 }),
                Some("o2222222222222222"),
            ),
        ];
        for (raw, expected, selector) in cases {
            let ClientMessage::Action(parsed) = parse_client_message(raw).unwrap() else {
                panic!("trade action must parse");
            };
            assert_eq!(parsed.action, Ok(expected));
            assert_eq!(parsed.trade_selector.as_deref(), selector);
        }

        let signatures = [
            r#"{"v":1,"type":"action.perform","data":{"requestId":"same","action":"trade.item.set","tradeId":"o2222222222222222","target":"o3333333333333333","quantity":1}}"#,
            r#"{"v":1,"type":"action.perform","data":{"requestId":"same","action":"trade.item.set","tradeId":"o2222222222222222","target":"o3333333333333333","quantity":2}}"#,
            r#"{"v":1,"type":"action.perform","data":{"requestId":"same","action":"trade.item.set","tradeId":"o2222222222222222","target":"o4444444444444444","quantity":1}}"#,
            r#"{"v":1,"type":"action.perform","data":{"requestId":"same","action":"trade.confirm","tradeId":"o2222222222222222","stateRevision":1}}"#,
            r#"{"v":1,"type":"action.perform","data":{"requestId":"same","action":"trade.confirm","tradeId":"o2222222222222222","stateRevision":2}}"#,
        ]
        .map(|raw| match parse_client_message(raw).unwrap() {
            ClientMessage::Action(action) => action.signature,
            _ => panic!("trade action must parse"),
        });
        assert_eq!(
            signatures
                .iter()
                .collect::<std::collections::BTreeSet<_>>()
                .len(),
            signatures.len()
        );

        for invalid in [
            r#"{"v":1,"type":"action.perform","data":{"requestId":"bad","action":"trade.accept","tradeId":"trade-1"}}"#,
            r#"{"v":1,"type":"action.perform","data":{"requestId":"bad","action":"trade.coins.set","tradeId":"o2222222222222222","coins":1000000001}}"#,
            r#"{"v":1,"type":"action.perform","data":{"requestId":"bad","action":"trade.confirm","tradeId":"o2222222222222222","stateRevision":0}}"#,
            r#"{"v":1,"type":"action.perform","data":{"requestId":"bad","action":"trade.request","recipient":"Aria"}}"#,
            r#"{"v":1,"type":"action.perform","data":{"requestId":"bad","action":"trade.item.set","tradeId":"o2222222222222222","target":"blue lantern","quantity":1}}"#,
            r#"{"v":1,"type":"action.perform","data":{"requestId":"bad","action":"trade.item.remove","tradeId":"o2222222222222222","target":"blue lantern"}}"#,
        ] {
            assert_eq!(
                parse_client_message(invalid).unwrap_err(),
                ClientMessageError::InvalidEnvelope
            );
        }
        for invalid_fields in [
            r#"{"v":1,"type":"action.perform","data":{"requestId":"bad","action":"trade.accept","tradeId":"o2222222222222222","page":1}}"#,
            r#"{"v":1,"type":"action.perform","data":{"requestId":"bad","action":"trade.item.set","tradeId":"o2222222222222222","target":"o3333333333333333","quantity":129}}"#,
            r#"{"v":1,"type":"action.perform","data":{"requestId":"bad","action":"trade.request","recipient":"o1111111111111111","tradeId":"o2222222222222222"}}"#,
        ] {
            let ClientMessage::Action(action) = parse_client_message(invalid_fields).unwrap()
            else {
                panic!("well-formed closed trade error must return an action result");
            };
            assert!(action.action.is_err());
        }
    }

    #[test]
    fn trade_json_rejects_every_present_null_before_option_deserialization() {
        for field in [
            "abilityId",
            "classId",
            "graphId",
            "nodeId",
            "target",
            "recipient",
            "container",
            "quantity",
            "page",
            "tradeId",
            "coins",
            "stateRevision",
            "message",
            "channel",
            "durationSeconds",
            "reason",
            "role",
            "reportId",
            "resolution",
            "profile",
            "loreTitle",
            "capability",
            "enabled",
        ] {
            let mut data = json!({
                "requestId": format!("null-{field}"),
                "action": "trade.status",
                "tradeId": "o2222222222222222",
            });
            data.as_object_mut()
                .unwrap()
                .insert(field.into(), Value::Null);
            let raw = json!({"v": 1, "type": "action.perform", "data": data}).to_string();
            assert_eq!(
                parse_client_message(&raw).unwrap_err(),
                ClientMessageError::InvalidEnvelope,
                "field {field:?} must not collapse from present null to absent",
            );
        }
    }

    #[test]
    fn json_depth_limit_ignores_brackets_in_strings_and_rejects_nesting_bombs() {
        assert!(json_depth_within(
            r#"{"text":"escaped \" [ { still text","value":[1]}"#,
            4
        ));
        let deeply_nested = format!(
            "{{\"v\":1,\"type\":\"sync.request\",\"data\":{{\"afterSeq\":0,\"x\":{} }}}}",
            "[".repeat(MAX_JSON_DEPTH) + &"]".repeat(MAX_JSON_DEPTH)
        );
        assert_eq!(
            parse_client_message(&deeply_nested).unwrap_err(),
            ClientMessageError::InvalidEnvelope
        );
    }

    #[test]
    fn request_cache_is_bounded_and_replays_the_stored_signature() {
        let mut cache = RequestCache::new(2);
        let result = |signature: &str| CachedActionResult {
            signature: signature.to_owned(),
            ok: true,
            message: Some("Done.".to_owned()),
            container_contents: None,
            trade_contents: None,
            reward_odds: None,
            mail_page: None,
            mail_message: None,
            board_catalog: None,
            board_page: None,
            board_entry: None,
            projection_session: SessionId(1),
        };
        cache.insert("request-1".to_owned(), result("look"));
        cache.insert("request-2".to_owned(), result("north"));
        cache.insert("request-3".to_owned(), result("inventory"));

        assert!(cache.get("request-1").is_none());
        assert_eq!(cache.get("request-2"), Some(&result("north")));
        assert_eq!(cache.get("request-3"), Some(&result("inventory")));
    }

    #[test]
    fn action_replay_keys_survive_connections_without_crossing_characters() {
        let shared = action_replay_cache();
        let first = action_replay_key(CharacterId(7), "same-request");
        let second = action_replay_key(CharacterId(8), "same-request");
        let cached = CachedActionResult {
            signature: "north".into(),
            ok: true,
            message: None,
            container_contents: None,
            trade_contents: None,
            reward_odds: None,
            mail_page: None,
            mail_message: None,
            board_catalog: None,
            board_page: None,
            board_entry: None,
            projection_session: SessionId(11),
        };
        shared.lock().unwrap().insert(first.clone(), cached.clone());

        let cache = shared.lock().unwrap();
        assert_eq!(cache.get(&first), Some(&cached));
        assert!(cache.get(&second).is_none());
    }

    #[test]
    fn cached_container_projection_never_crosses_session_salts() {
        let contents = ContainerContentsView {
            selector: "o9999999999999999".into(),
            items: Vec::new(),
            page: 1,
            total_pages: 1,
            capacity: tiamud_core::InventoryCapacityView {
                used_slots: 0,
                max_slots: 8,
                used_weight_grams: 0,
                max_weight_grams: 5_000,
            },
        };
        let cached = CachedActionResult {
            signature: "contents".into(),
            ok: true,
            message: Some("It is empty.".into()),
            container_contents: Some(contents.clone()),
            trade_contents: None,
            reward_odds: None,
            mail_page: None,
            mail_message: None,
            board_catalog: None,
            board_page: None,
            board_entry: None,
            projection_session: SessionId(11),
        };

        assert_eq!(
            cached.container_contents_for(SessionId(11)),
            Some(&contents)
        );
        assert_eq!(
            cached.message_for(SessionId(11)).as_deref(),
            Some("It is empty.")
        );
        assert!(cached.container_contents_for(SessionId(12)).is_none());
        let foreign_message = cached.message_for(SessionId(12)).unwrap();
        assert!(foreign_message.contains("previous connection"));
        assert!(!foreign_message.contains("o9999999999999999"));
    }

    #[test]
    fn cached_trade_projection_never_crosses_session_salts() {
        let contents = TradeContentsView {
            trade_id: "o1111111111111111".into(),
            container: "o2222222222222222".into(),
            items: Vec::new(),
            page: 1,
            total_pages: 1,
            capacity: tiamud_core::InventoryCapacityView {
                used_slots: 0,
                max_slots: 8,
                used_weight_grams: 0,
                max_weight_grams: 5_000,
            },
        };
        let cached = CachedActionResult {
            signature: "trade.contents".into(),
            ok: true,
            message: Some("That offered container is empty.".into()),
            container_contents: None,
            trade_contents: Some(contents.clone()),
            reward_odds: None,
            mail_page: None,
            mail_message: None,
            board_catalog: None,
            board_page: None,
            board_entry: None,
            projection_session: SessionId(11),
        };

        assert_eq!(cached.trade_contents_for(SessionId(11)), Some(&contents));
        assert!(cached.trade_contents_for(SessionId(12)).is_none());
        let foreign_message = cached.message_for(SessionId(12)).unwrap();
        assert!(foreign_message.contains("previous connection"));
        assert!(!foreign_message.contains("o1111111111111111"));
        assert!(!foreign_message.contains("o2222222222222222"));
    }

    #[test]
    fn reward_odds_are_exact_bounded_and_never_cross_session_salts() {
        let odds = RewardOddsView {
            selector: "o6666666666666666".into(),
            page: 1,
            total_pages: 1,
            guaranteed: vec![tiamud_core::RewardOddsGuaranteedRow {
                label: "three sparks".into(),
                quantity: 3,
                affix_pool_label: None,
            }],
            weighted: vec![tiamud_core::RewardOddsWeightedRow {
                pool_label: "storm draw".into(),
                rolls: 1,
                outcome_label: "nothing".into(),
                min_quantity: 0,
                max_quantity: 0,
                probability_numerator: 1,
                probability_denominator: 4,
                affix_pool_label: None,
            }],
            affixes: vec![tiamud_core::RewardOddsAffixRow {
                parent_outcome_label: "iron blade".into(),
                affix_label: "charged".into(),
                position: Some(tiamud_core::AffixPosition::Prefix),
                attack_bonus_delta: 2,
                heal_amount_delta: 0,
                conditional_probability_numerator: 1,
                conditional_probability_denominator: 5,
            }],
        };
        let cached = CachedActionResult {
            signature: "reward.odds".into(),
            ok: true,
            message: Some("Exact odds follow.".into()),
            container_contents: None,
            trade_contents: None,
            reward_odds: Some(odds.clone()),
            mail_page: None,
            mail_message: None,
            board_catalog: None,
            board_page: None,
            board_entry: None,
            projection_session: SessionId(31),
        };

        assert_eq!(cached.reward_odds_for(SessionId(31)), Some(&odds));
        assert!(cached.reward_odds_for(SessionId(32)).is_none());

        let outbound = OutboundHandle::new(2, Arc::new(RuntimeMetrics::default()));
        let mut sequence = 1;
        assert!(queue_reward_odds(&outbound, &mut sequence, &odds));
        let frame = outbound.pop().unwrap();
        assert_eq!(frame.value["type"], "reward.odds");
        assert_eq!(frame.value["data"]["selector"], "o6666666666666666");
        assert_eq!(
            frame.value["data"]["weighted"][0]["probabilityNumerator"],
            1
        );
        assert_eq!(
            frame.value["data"]["weighted"][0]["probabilityDenominator"],
            4
        );
        assert_eq!(
            frame.value["data"]["affixes"][0]["conditionalProbabilityDenominator"],
            5
        );
        let encoded = serde_json::to_vec(&frame.value).unwrap();
        assert!(encoded.len() <= MAX_WS_MESSAGE_BYTES);
        let encoded = String::from_utf8(encoded).unwrap();
        assert!(!encoded.contains("lootTableId"));
        assert!(!encoded.contains("affixPoolId"));
        assert!(!encoded.contains("digest"));
    }

    #[test]
    fn mail_frames_are_exact_bounded_and_never_cross_sessions() {
        let summary = crate::persistence::MailMessageSummary {
            public_id: "m0123456789abcdef0123456789abcdef".into(),
            sender_name: "A&B <sender>".into(),
            subject: "Plan <&>".into(),
            created_at: 10,
            expires_at: 20,
            read: false,
        };
        let page = MailPage {
            page: 1,
            total_pages: 1,
            total_messages: 1,
            unread_count: 1,
            capacity: 64,
            messages: vec![summary.clone()],
        };
        let message = MailMessage {
            summary: summary.clone(),
            body: "Meet | beyond the gate. <&>".into(),
        };
        let cached = CachedActionResult {
            signature: "mail.read".into(),
            ok: true,
            message: Some("Mail follows.".into()),
            container_contents: None,
            trade_contents: None,
            reward_odds: None,
            mail_page: Some(page.clone()),
            mail_message: Some(message.clone()),
            board_catalog: None,
            board_page: None,
            board_entry: None,
            projection_session: SessionId(41),
        };
        assert_eq!(cached.mail_page_for(SessionId(41)), Some(&page));
        assert_eq!(cached.mail_message_for(SessionId(41)), Some(&message));
        assert!(cached.mail_page_for(SessionId(42)).is_none());
        assert!(cached.mail_message_for(SessionId(42)).is_none());

        let outbound = OutboundHandle::new(3, Arc::new(RuntimeMetrics::default()));
        let mut sequence = 1;
        assert!(queue_mail_page(&outbound, &mut sequence, &page));
        assert!(queue_mail_message(&outbound, &mut sequence, &message));
        let list = outbound.pop().unwrap();
        let detail = outbound.pop().unwrap();
        assert_eq!(list.value["type"], "mail.list");
        assert_eq!(list.value["data"]["messages"][0]["id"], summary.public_id);
        assert_eq!(list.value["data"]["messages"][0]["subject"], "Plan <&>");
        assert!(list.value["data"]["messages"][0].get("body").is_none());
        assert_eq!(detail.value["type"], "mail.message");
        assert_eq!(detail.value["data"]["body"], message.body);
        for frame in [list, detail] {
            assert!(serde_json::to_vec(&frame.value).unwrap().len() <= MAX_WS_MESSAGE_BYTES);
            let encoded = frame.value.to_string();
            assert!(!encoded.contains("recipientCharacterId"));
            assert!(!encoded.contains("senderCharacterId"));
            assert!(!encoded.contains("accountId"));
        }
    }

    #[test]
    fn board_frames_are_exact_bounded_minimal_and_never_cross_sessions() {
        let catalog = vec![
            BoardDescriptor {
                board: BoardKey::General,
                can_post: true,
            },
            BoardDescriptor {
                board: BoardKey::Newbie,
                can_post: false,
            },
        ];
        let summary = crate::persistence::BoardPostSummary {
            public_id: "b0123456789abcdef0123456789abcdef".into(),
            board: BoardKey::General,
            author_name: "A&B <author>".into(),
            subject: "Plan <&>".into(),
            created_at: 10,
            expires_at: 7_776_010,
            can_delete: false,
            can_report: true,
        };
        let page = BoardPage {
            board: BoardKey::General,
            page: 1,
            total_pages: 1,
            total_posts: 1,
            capacity: 1_000,
            posts: vec![summary.clone()],
        };
        let entry = BoardEntry {
            summary: summary.clone(),
            body: "Meet | beyond the gate. <&>".into(),
        };
        let cached = CachedActionResult {
            signature: "board.read".into(),
            ok: true,
            message: Some("Board post follows.".into()),
            container_contents: None,
            trade_contents: None,
            reward_odds: None,
            mail_page: None,
            mail_message: None,
            board_catalog: Some(catalog.clone()),
            board_page: Some(page.clone()),
            board_entry: Some(entry.clone()),
            projection_session: SessionId(51),
        };
        assert_eq!(
            cached.board_catalog_for(SessionId(51)),
            Some(catalog.as_slice())
        );
        assert_eq!(cached.board_page_for(SessionId(51)), Some(&page));
        assert_eq!(cached.board_entry_for(SessionId(51)), Some(&entry));
        assert!(cached.board_catalog_for(SessionId(52)).is_none());
        assert!(cached.board_page_for(SessionId(52)).is_none());
        assert!(cached.board_entry_for(SessionId(52)).is_none());

        let outbound = OutboundHandle::new(4, Arc::new(RuntimeMetrics::default()));
        let mut sequence = 1;
        assert!(queue_board_catalog(&outbound, &mut sequence, &catalog));
        assert!(queue_board_page(&outbound, &mut sequence, &page));
        assert!(queue_board_entry(&outbound, &mut sequence, &entry));
        let catalog_frame = outbound.pop().unwrap();
        let list_frame = outbound.pop().unwrap();
        let entry_frame = outbound.pop().unwrap();
        assert_eq!(catalog_frame.value["type"], "board.catalog");
        assert_eq!(catalog_frame.value["data"]["retentionDays"], 90);
        assert_eq!(catalog_frame.value["data"]["boards"][0]["key"], "general");
        assert_eq!(catalog_frame.value["data"]["boards"][0]["canPost"], true);
        assert_eq!(catalog_frame.value["data"]["boards"][1]["key"], "newbie");
        assert_eq!(catalog_frame.value["data"]["boards"][1]["canPost"], false);
        assert_eq!(list_frame.value["type"], "board.list");
        assert_eq!(list_frame.value["data"]["capacity"], 1_000);
        assert_eq!(
            list_frame.value["data"]["posts"][0]["postId"],
            summary.public_id
        );
        assert_eq!(
            list_frame.value["data"]["posts"][0]["author"],
            "A&B <author>"
        );
        assert!(list_frame.value["data"]["posts"][0].get("body").is_none());
        assert_eq!(entry_frame.value["type"], "board.entry");
        assert_eq!(entry_frame.value["data"]["body"], entry.body);
        for frame in [catalog_frame, list_frame, entry_frame] {
            assert!(serde_json::to_vec(&frame.value).unwrap().len() <= MAX_WS_MESSAGE_BYTES);
            let encoded = frame.value.to_string();
            assert!(!encoded.contains("authorCharacterId"));
            assert!(!encoded.contains("reporterCharacterId"));
            assert!(!encoded.contains("accountId"));
            assert!(!encoded.contains("bodySnapshot"));
        }
    }

    #[test]
    fn cached_inventory_text_never_crosses_session_salts() {
        let cached = CachedActionResult {
            signature: "inventory".into(),
            ok: true,
            message: Some("You are carrying a lantern [ref o1111111111111111].\r\n".into()),
            container_contents: None,
            trade_contents: None,
            reward_odds: None,
            mail_page: None,
            mail_message: None,
            board_catalog: None,
            board_page: None,
            board_entry: None,
            projection_session: SessionId(21),
        };

        assert!(
            cached
                .message_for(SessionId(21))
                .unwrap()
                .contains("o1111111111111111")
        );
        let foreign_message = cached.message_for(SessionId(22)).unwrap();
        assert!(foreign_message.contains("previous connection"));
        assert!(!foreign_message.contains("o1111111111111111"));
        assert!(cached.container_contents_for(SessionId(22)).is_none());
    }

    fn queued_frame(
        priority: OutboundPriority,
        coalesce: Option<CoalesceKey>,
        sequence: u64,
    ) -> OutboundEnvelope {
        OutboundEnvelope {
            priority,
            coalesce,
            sequence: Some(sequence),
            value: json!({"seq": sequence}),
        }
    }

    #[test]
    fn outbound_queue_coalesces_state_and_prioritises_critical_output() {
        let mut queue = OutboundQueue::new(2);
        assert_eq!(
            queue.enqueue(queued_frame(
                OutboundPriority::State,
                Some(CoalesceKey::Scene),
                1
            )),
            EnqueueResult::Queued
        );
        assert_eq!(
            queue.enqueue(queued_frame(
                OutboundPriority::State,
                Some(CoalesceKey::Scene),
                2
            )),
            EnqueueResult::Coalesced
        );
        assert_eq!(queue.pop().and_then(|frame| frame.sequence), Some(2));

        queue.enqueue(queued_frame(OutboundPriority::State, None, 3));
        queue.enqueue(queued_frame(OutboundPriority::Narrative, None, 4));
        assert_eq!(
            queue.enqueue(queued_frame(OutboundPriority::Critical, None, 5)),
            EnqueueResult::Queued
        );
        assert_eq!(queue.pop().and_then(|frame| frame.sequence), Some(5));
        assert!(queue.pop().is_none());
    }

    #[test]
    fn outbound_queue_disconnects_instead_of_losing_critical_output() {
        let mut queue = OutboundQueue::new(1);
        queue.enqueue(queued_frame(OutboundPriority::Critical, None, 1));
        assert_eq!(
            queue.enqueue(queued_frame(OutboundPriority::Critical, None, 2)),
            EnqueueResult::Disconnect
        );
        assert!(queue.take_disconnect_reason().is_some());
    }

    #[test]
    fn websocket_queue_records_coalescing_and_slow_disconnect_metrics() {
        let metrics = Arc::new(RuntimeMetrics::default());
        let coalescing = OutboundHandle::new(2, metrics.clone());
        coalescing.enqueue(queued_frame(
            OutboundPriority::State,
            Some(CoalesceKey::Player),
            1,
        ));
        assert_eq!(
            coalescing.enqueue(queued_frame(
                OutboundPriority::State,
                Some(CoalesceKey::Player),
                2,
            )),
            EnqueueResult::Coalesced
        );

        let slow = OutboundHandle::new(1, metrics.clone());
        slow.enqueue(queued_frame(OutboundPriority::Critical, None, 3));
        assert_eq!(
            slow.enqueue(queued_frame(OutboundPriority::Critical, None, 4)),
            EnqueueResult::Disconnect
        );

        let output = metrics.prometheus(0);
        assert!(output.contains("tiamud_output_coalesced_total{transport=\"websocket\"} 1"));
        assert!(output.contains("tiamud_slow_client_disconnects_total{transport=\"websocket\"} 1"));
    }

    #[test]
    fn best_effort_heartbeat_never_evicts_critical_output() {
        let mut queue = OutboundQueue::new(1);
        queue.enqueue(queued_frame(OutboundPriority::Critical, None, 1));
        assert_eq!(
            queue.enqueue(OutboundEnvelope {
                priority: OutboundPriority::BestEffort,
                coalesce: Some(CoalesceKey::Heartbeat),
                sequence: None,
                value: json!({"type": "server.heartbeat"}),
            }),
            EnqueueResult::Dropped
        );
        assert_eq!(queue.pop().and_then(|frame| frame.sequence), Some(1));
    }

    #[test]
    fn only_perception_safe_view_events_are_serialised() {
        let outbound = OutboundHandle::new(2, Arc::new(RuntimeMetrics::default()));
        let mut sequence = 1;
        assert!(queue_view_event(
            &outbound,
            &mut sequence,
            ProjectedViewEvent {
                event: tiamud_core::ViewEvent {
                    revision: 8,
                    kind: tiamud_core::ViewEventKind::Gameplay,
                    text: "The brigand strikes you.".to_owned(),
                    actor_id: "opaque-actor".to_owned(),
                    target_id: Some("opaque-target".to_owned()),
                    item_id: None,
                },
                combat_cue: None,
            },
            &crate::runtime_content::ScenePresentation::default(),
            &crate::runtime_content::ScenePresentation::default(),
        ));

        let frame = outbound.pop().expect("projected event must be queued");
        assert_eq!(frame.value["type"], "message.append");
        assert_eq!(frame.value["data"]["actorId"], "opaque-actor");
        assert_eq!(frame.value["data"]["targetId"], "opaque-target");
        assert!(frame.value["data"].get("roomId").is_none());
        assert!(frame.value["data"].get("cue").is_none());

        assert!(queue_view_event(
            &outbound,
            &mut sequence,
            ProjectedViewEvent {
                event: tiamud_core::ViewEvent {
                    revision: 9,
                    kind: tiamud_core::ViewEventKind::ItemGiven,
                    text: "Aria gives a lantern to Bram.".to_owned(),
                    actor_id: "opaque-giver".to_owned(),
                    target_id: Some("opaque-recipient".to_owned()),
                    item_id: Some("opaque-item".to_owned()),
                },
                combat_cue: None,
            },
            &crate::runtime_content::ScenePresentation::default(),
            &crate::runtime_content::ScenePresentation::default(),
        ));
        let given = outbound.pop().expect("projected give must be queued");
        assert_eq!(given.value["data"]["kind"], "itemGiven");
        assert_eq!(given.value["data"]["targetId"], "opaque-recipient");
        assert_eq!(given.value["data"]["itemId"], "opaque-item");
        assert!(!given.value.to_string().contains("characterId"));
    }

    #[test]
    fn combat_cues_decorate_messages_only_for_presented_opaque_entities() {
        let presentation = test_entity_presentation();
        let presented_id = serde_json::to_value(&presentation).unwrap()["entities"][0]["id"]
            .as_str()
            .unwrap()
            .to_owned();
        let empty = crate::runtime_content::ScenePresentation::default();
        let outbound = OutboundHandle::new(4, Arc::new(RuntimeMetrics::default()));
        let mut sequence = 1;

        let projected = |kind, actor_id: String, target_id: Option<String>, entity_id: String| {
            ProjectedViewEvent {
                event: tiamud_core::ViewEvent {
                    revision: 8,
                    kind: tiamud_core::ViewEventKind::Gameplay,
                    text: "Authoritative combat narration.".into(),
                    actor_id,
                    target_id,
                    item_id: None,
                },
                combat_cue: Some(crate::world_actor::AuthoritativeCombatCue { kind, entity_id }),
            }
        };
        assert!(queue_view_event(
            &outbound,
            &mut sequence,
            projected(
                CombatCueKind::Attack,
                presented_id.clone(),
                Some("o0000000000000001".into()),
                presented_id.clone(),
            ),
            &presentation,
            &empty,
        ));
        assert!(queue_view_event(
            &outbound,
            &mut sequence,
            projected(
                CombatCueKind::Hit,
                "o0000000000000001".into(),
                Some(presented_id.clone()),
                presented_id.clone(),
            ),
            &empty,
            &presentation,
        ));
        assert!(queue_view_event(
            &outbound,
            &mut sequence,
            projected(
                CombatCueKind::Defeat,
                "o0000000000000001".into(),
                Some("o0000000000000002".into()),
                "o0000000000000002".into(),
            ),
            &empty,
            &empty,
        ));

        let attack = outbound.pop().unwrap().value;
        assert_eq!(attack["data"]["cue"]["kind"], "attack");
        assert_eq!(attack["data"]["cue"]["entityId"], presented_id);
        assert_eq!(attack["data"]["cue"]["pose"], "attack");
        let hit = outbound.pop().unwrap().value;
        assert_eq!(hit["data"]["cue"]["kind"], "hit");
        assert_eq!(hit["data"]["cue"]["entityId"], presented_id);
        assert_eq!(hit["data"]["cue"]["pose"], "hit");
        let suppressed = outbound.pop().unwrap().value;
        assert!(suppressed["data"].get("cue").is_none());
        assert_eq!(
            suppressed["data"]["text"],
            "Authoritative combat narration."
        );
        assert!(!attack.to_string().contains("demo:blue-lantern"));
    }

    #[test]
    fn websocket_media_requires_opt_in_and_rejects_mutable_or_remote_urls() {
        let digest = "a".repeat(64);
        let ambience = tiamud_core::SceneAudio {
            channel: "ambience".into(),
            url: format!("/assets/{digest}/audio-rain.opus"),
            looped: true,
        };
        let outbound = OutboundHandle::new(8, Arc::new(RuntimeMetrics::default()));
        let mut sequence = 1;

        assert!(queue_media_transition(
            &outbound,
            &mut sequence,
            false,
            None,
            Some(&ambience),
        ));
        assert!(outbound.pop().is_none());

        assert!(queue_media_transition(
            &outbound,
            &mut sequence,
            true,
            None,
            Some(&ambience),
        ));
        let play = outbound.pop().unwrap().value;
        assert_eq!(play["type"], "media.play");
        assert_eq!(play["data"]["url"], ambience.url);

        assert!(queue_media_transition(
            &outbound,
            &mut sequence,
            true,
            Some(&ambience),
            None,
        ));
        assert_eq!(outbound.pop().unwrap().value["type"], "media.stop");

        let unsafe_audio = tiamud_core::SceneAudio {
            channel: "ambience".into(),
            url: "https://tracker.invalid/audio-rain.mp3".into(),
            looped: false,
        };
        assert!(queue_media_transition(
            &outbound,
            &mut sequence,
            true,
            None,
            Some(&unsafe_audio),
        ));
        assert!(outbound.pop().is_none());
    }

    #[test]
    fn player_and_sync_snapshots_never_expose_durable_or_moderation_state() {
        let mut world = tiamud_core::World::demo();
        let session = SessionId(77);
        world
            .connect(
                session,
                CharacterSnapshot {
                    id: CharacterId(987_654),
                    account_id: tiamud_core::AccountId(876_543),
                    name: "Aria".into(),
                    room_id: world.start_room_id().to_owned(),
                    item_locations: std::collections::BTreeMap::new(),
                },
            )
            .unwrap();
        world
            .apply(session, PlayerAction::Take(ItemSelection::one("lantern")))
            .unwrap();
        let scene = world.current_scene(session).unwrap();
        let player = world.player_state_view(session).unwrap();
        let outbound = OutboundHandle::new(4, Arc::new(RuntimeMetrics::default()));
        let mut sequence = 1;
        assert!(queue_sync_recovery(
            &outbound,
            &mut sequence,
            0,
            &scene,
            &crate::runtime_content::ScenePresentation::default(),
            &player,
        ));
        assert!(queue_player(&outbound, &mut sequence, &player));

        let mut wire = String::new();
        while let Some(frame) = outbound.pop() {
            wire.push_str(&frame.value.to_string());
        }
        assert!(wire.contains("\"type\":\"player.snapshot\""));
        assert!(wire.contains("\"type\":\"sync.scene\""));
        assert!(wire.contains("\"type\":\"sync.player\""));
        assert!(wire.contains("\"syncId\":\"sync-1\""));
        for forbidden in [
            "demo:lantern-square",
            "demo:blue-lantern",
            "\"accountId\"",
            "\"characterId\"",
            "\"roomId\"",
            "\"itemId\"",
            "\"partyId\"",
            "\"role\"",
            "\"muted\"",
            "\"blocked\"",
            "\"reports\"",
            "\"moderation\"",
        ] {
            assert!(!wire.contains(forbidden), "wire leaked {forbidden}: {wire}");
        }
    }

    #[test]
    fn sync_recovery_is_two_bounded_correlated_and_coalescible_frames() {
        let mut world = tiamud_core::World::demo();
        let session = SessionId(78);
        world
            .connect(
                session,
                CharacterSnapshot {
                    id: CharacterId(1),
                    account_id: tiamud_core::AccountId(2),
                    name: "Sync Tester".into(),
                    room_id: world.start_room_id().to_owned(),
                    item_locations: BTreeMap::new(),
                },
            )
            .unwrap();
        let scene = world.current_scene(session).unwrap();
        let player = world.player_state_view(session).unwrap();
        let outbound = OutboundHandle::new(4, Arc::new(RuntimeMetrics::default()));
        let mut sequence = 10;

        assert!(queue_sync_recovery(
            &outbound,
            &mut sequence,
            4,
            &scene,
            &crate::runtime_content::ScenePresentation::default(),
            &player
        ));
        assert!(queue_sync_recovery(
            &outbound,
            &mut sequence,
            9,
            &scene,
            &crate::runtime_content::ScenePresentation::default(),
            &player
        ));

        let frames = std::iter::from_fn(|| outbound.pop()).collect::<Vec<_>>();
        assert_eq!(
            frames.len(),
            2,
            "the newer recovery pair coalesces the old pair"
        );
        assert_eq!(frames[0].value["type"], "sync.scene");
        assert_eq!(frames[1].value["type"], "sync.player");
        assert_eq!(frames[0].value["data"]["syncId"], "sync-12");
        assert_eq!(frames[1].value["data"]["syncId"], "sync-12");
        assert_eq!(frames[0].value["data"]["afterSeq"], 9);
        assert_eq!(frames[1].value["data"]["afterSeq"], 9);
        assert_eq!(frames[1].value["data"]["complete"], true);
        assert!(frames.iter().all(|frame| {
            serde_json::to_vec(&frame.value).unwrap().len() <= MAX_WS_MESSAGE_BYTES
        }));
    }

    #[test]
    fn split_sync_accepts_individually_bounded_states_that_do_not_fit_together() {
        let mut world = tiamud_core::World::demo();
        let session = SessionId(80);
        world
            .connect(
                session,
                CharacterSnapshot {
                    id: CharacterId(5),
                    account_id: tiamud_core::AccountId(6),
                    name: "Split Budget".into(),
                    room_id: world.start_room_id().to_owned(),
                    item_locations: BTreeMap::new(),
                },
            )
            .unwrap();
        let mut scene = world.current_scene(session).unwrap();
        scene.prose = vec!["s".repeat(5_000), "t".repeat(5_000)];
        let mut player = world.player_state_view(session).unwrap();
        player.inventory = (0..2)
            .map(|index| tiamud_core::PlayerItemView {
                id: format!("o{index:016x}"),
                label: format!("Budget item {index}"),
                description: "p".repeat(2_500),
                rarity: "common".into(),
                item_level: 1,
                weight_grams: 1,
                quantity: 1,
                available_quantity: 1,
                reserved_for_trade: 0,
                stack_slots: 1,
                total_weight_grams: 1,
                transfer_policy: tiamud_core::ItemTransferPolicy::Transferable,
                bound_to_you: false,
                affix: None,
                can_give: true,
                can_trade: true,
                container_capacity: None,
                actions: Vec::new(),
            })
            .collect();
        let old_combined = json!({
            "v": 1,
            "type": "sync.snapshot",
            "seq": 1,
            "data": {"afterSeq": 0, "scene": &scene, "player": &player},
        });
        assert!(serde_json::to_vec(&old_combined).unwrap().len() > MAX_WS_MESSAGE_BYTES);

        let outbound = OutboundHandle::new(4, Arc::new(RuntimeMetrics::default()));
        let mut sequence = 1;
        assert!(queue_sync_recovery(
            &outbound,
            &mut sequence,
            0,
            &scene,
            &crate::runtime_content::ScenePresentation::default(),
            &player
        ));
        let frames = std::iter::from_fn(|| outbound.pop()).collect::<Vec<_>>();
        assert_eq!(frames.len(), 2);
        assert!(frames.iter().all(|frame| {
            serde_json::to_vec(&frame.value).unwrap().len() <= MAX_WS_MESSAGE_BYTES
        }));
    }

    #[test]
    fn optional_entity_presentation_is_shed_before_semantic_scene_exceeds_budget() {
        let presentation = test_entity_presentation();
        assert_eq!(presentation.entities.len(), 1);
        let mut scene = test_scene();
        let mut chosen = None;
        for prose_bytes in (1_000..MAX_WS_MESSAGE_BYTES).rev() {
            scene.prose = vec!["x".repeat(prose_bytes)];
            let semantic = json!({"v":1,"type":"scene.snapshot","seq":1,"data":&scene});
            let decorated = json!({
                "v":1,
                "type":"scene.snapshot",
                "seq":1,
                "data":scene_with_presentation(&scene, &presentation),
            });
            if serde_json::to_vec(&semantic).unwrap().len() <= MAX_WS_MESSAGE_BYTES
                && serde_json::to_vec(&decorated).unwrap().len() > MAX_WS_MESSAGE_BYTES
            {
                chosen = Some(scene.clone());
                break;
            }
        }
        let scene = chosen.expect("a presentation-only overflow boundary must exist");

        let outbound = OutboundHandle::new(4, Arc::new(RuntimeMetrics::default()));
        let mut sequence = 1;
        assert!(queue_scene(
            &outbound,
            &mut sequence,
            ServerMessageType::SceneSnapshot,
            &scene,
            &presentation,
        ));
        let frame = outbound.pop().unwrap();
        assert_eq!(frame.value["data"]["title"], scene.title);
        assert!(frame.value["data"].get("presentation").is_none());
        assert!(serde_json::to_vec(&frame.value).unwrap().len() <= MAX_WS_MESSAGE_BYTES);

        let player = test_player_state();
        let mut sync_scene_model = test_scene();
        let mut sync_chosen = None;
        for prose_bytes in (1_000..MAX_WS_MESSAGE_BYTES).rev() {
            sync_scene_model.prose = vec!["x".repeat(prose_bytes)];
            let semantic = json!({
                "v":1,
                "type":"sync.scene",
                "seq":1,
                "data":{"syncId":"sync-1","afterSeq":0,"scene":&sync_scene_model},
            });
            let decorated = json!({
                "v":1,
                "type":"sync.scene",
                "seq":1,
                "data":{
                    "syncId":"sync-1",
                    "afterSeq":0,
                    "scene":scene_with_presentation(&sync_scene_model, &presentation),
                },
            });
            if serde_json::to_vec(&semantic).unwrap().len() <= MAX_WS_MESSAGE_BYTES
                && serde_json::to_vec(&decorated).unwrap().len() > MAX_WS_MESSAGE_BYTES
            {
                sync_chosen = Some(sync_scene_model.clone());
                break;
            }
        }
        let sync_scene_model =
            sync_chosen.expect("a sync presentation-only overflow boundary must exist");
        let outbound = OutboundHandle::new(4, Arc::new(RuntimeMetrics::default()));
        let mut sequence = 1;
        assert!(queue_sync_recovery(
            &outbound,
            &mut sequence,
            0,
            &sync_scene_model,
            &presentation,
            &player,
        ));
        let sync_scene = outbound.pop().unwrap();
        assert_eq!(sync_scene.value["type"], "sync.scene");
        assert!(
            sync_scene.value["data"]["scene"]
                .get("presentation")
                .is_none()
        );
        assert!(serde_json::to_vec(&sync_scene.value).unwrap().len() <= MAX_WS_MESSAGE_BYTES);
    }

    #[test]
    fn scene_snapshot_projects_entity_media_with_only_observer_opaque_ids() {
        let scene = test_scene();
        let presentation = test_entity_presentation();
        let opaque_id = serde_json::to_value(&presentation).unwrap()["entities"][0]["id"].clone();
        let outbound = OutboundHandle::new(4, Arc::new(RuntimeMetrics::default()));
        let mut sequence = 1;
        assert!(queue_scene(
            &outbound,
            &mut sequence,
            ServerMessageType::SceneSnapshot,
            &scene,
            &presentation,
        ));
        let frame = outbound.pop().unwrap();
        let wire = frame.value.to_string();
        assert_eq!(
            frame.value["data"]["presentation"]["entities"][0]["id"],
            opaque_id
        );
        assert!(wire.contains("sprite-idle-640.webp"));
        assert!(!wire.contains("demo:blue-lantern"));
        assert!(!scene.as_plain_text().contains("sprite-idle"));
    }

    #[test]
    fn oversized_scene_and_player_snapshots_fail_closed_before_queueing() {
        let mut world = tiamud_core::World::demo();
        let session = SessionId(79);
        world
            .connect(
                session,
                CharacterSnapshot {
                    id: CharacterId(3),
                    account_id: tiamud_core::AccountId(4),
                    name: "Budget Tester".into(),
                    room_id: world.start_room_id().to_owned(),
                    item_locations: BTreeMap::new(),
                },
            )
            .unwrap();
        let mut scene = world.current_scene(session).unwrap();
        scene.prose = (0..32).map(|_| "s".repeat(4_096)).collect();
        let mut player = world.player_state_view(session).unwrap();
        player.inventory = (0..256)
            .map(|index| tiamud_core::PlayerItemView {
                id: format!("o{index:016x}"),
                label: format!("Oversized budget item {index}"),
                description: "p".repeat(4_096),
                rarity: "common".into(),
                item_level: 1,
                weight_grams: 1,
                quantity: 1,
                available_quantity: 1,
                reserved_for_trade: 0,
                stack_slots: 1,
                total_weight_grams: 1,
                transfer_policy: tiamud_core::ItemTransferPolicy::Transferable,
                bound_to_you: false,
                affix: None,
                can_give: true,
                can_trade: true,
                container_capacity: None,
                actions: Vec::new(),
            })
            .collect();

        let metrics = Arc::new(RuntimeMetrics::default());
        let outbound = OutboundHandle::new(4, metrics.clone());
        let mut sequence = 1;
        assert!(!queue_scene(
            &outbound,
            &mut sequence,
            ServerMessageType::SceneSnapshot,
            &scene,
            &crate::runtime_content::ScenePresentation::default()
        ));
        assert!(outbound.pop().is_none());
        assert!(outbound.take_disconnect_reason().is_some());
        assert!(metrics.prometheus(0).contains(
            "tiamud_outbound_budget_exceeded_total{transport=\"websocket\",message=\"frame\"} 1"
        ));

        let metrics = Arc::new(RuntimeMetrics::default());
        let outbound = OutboundHandle::new(4, metrics.clone());
        let mut sequence = 1;
        assert!(!queue_player(&outbound, &mut sequence, &player));
        assert!(outbound.pop().is_none());
        assert!(outbound.take_disconnect_reason().is_some());
        assert!(metrics.prometheus(0).contains(
            "tiamud_outbound_budget_exceeded_total{transport=\"websocket\",message=\"frame\"} 1"
        ));
    }

    #[test]
    fn crowded_loot_uses_one_stable_bounded_window_for_diff_snapshot_and_sync() {
        let mut full = test_scene();
        full.revision = 10;
        full.items = (0..300)
            .map(|index| tiamud_core::SceneItem {
                id: format!("o{index:016x}"),
                label: format!("Reserved trophy {index:03}"),
                description: "d".repeat(2_048),
                rarity: "common".into(),
                item_level: 1,
                weight_grams: 1,
                quantity: 1,
                stack_slots: 1,
                total_weight_grams: 1,
                transfer_policy: tiamud_core::ItemTransferPolicy::Transferable,
                bound_to_you: false,
                affix: None,
                container_capacity: None,
                loot: Some(tiamud_core::SceneLootAccessView {
                    access: tiamud_core::LootAccessView::Yours,
                    expires_in_ticks: Some(20),
                }),
                actions: vec![tiamud_core::SceneAction {
                    label: "Pass".into(),
                    command: format!("loot pass o{index:016x}"),
                }],
            })
            .collect();

        let first = bounded_web_scene(full.clone());
        assert!(first.items.len() < full.items.len());
        assert!(
            first
                .prose
                .last()
                .is_some_and(|line| { line.starts_with("Bounded scene notice:") })
        );
        assert!(
            serde_json::to_vec(&first).unwrap().len() <= MAX_WS_SCENE_SEMANTIC_BYTES,
            "the retained semantic window must leave envelope headroom"
        );
        let mut prose_only = test_scene();
        prose_only.items.clear();
        prose_only.npcs.clear();
        prose_only.prose = vec!["multibyte ñ".repeat(20_000)];
        let bounded_prose = bounded_web_scene(prose_only);
        assert!(serde_json::to_vec(&bounded_prose).unwrap().len() <= MAX_WS_SCENE_SEMANTIC_BYTES);
        assert!(
            bounded_prose
                .prose
                .last()
                .is_some_and(|line| { line.contains("room description was shortened") })
        );

        full.revision = 11;
        for item in &mut full.items {
            item.loot
                .as_mut()
                .expect("crowded fixture uses reserved loot")
                .expires_in_ticks = Some(19);
        }
        let second = bounded_web_scene(full);
        assert_eq!(first.items.len(), second.items.len());

        let player = test_player_state();
        let outbound =
            OutboundHandle::new(WS_OUTBOUND_CAPACITY, Arc::new(RuntimeMetrics::default()));
        let mut sequence = 1;
        assert!(queue_semantic_updates(
            &outbound,
            &mut sequence,
            &first,
            &second,
            &player,
            &player,
        ));
        assert!(queue_scene(
            &outbound,
            &mut sequence,
            ServerMessageType::SceneSnapshot,
            &second,
            &crate::runtime_content::ScenePresentation::default(),
        ));
        assert!(outbound.take_disconnect_reason().is_none());
        let frames = std::iter::from_fn(|| outbound.pop()).collect::<Vec<_>>();
        assert!(frames.len() <= WS_OUTBOUND_CAPACITY);
        assert!(frames.iter().all(|frame| {
            serde_json::to_vec(&frame.value).unwrap().len() <= MAX_WS_MESSAGE_BYTES
        }));

        let mut replacement = second.clone();
        replacement.revision = 12;
        for (index, item) in replacement.items.iter_mut().enumerate() {
            item.id = format!("z{index:016x}");
        }
        assert!(
            first.items.len() + replacement.items.len() > MAX_WS_SEMANTIC_DELTA_FRAMES,
            "fixture must cross the delta-to-snapshot threshold"
        );
        let outbound =
            OutboundHandle::new(WS_OUTBOUND_CAPACITY, Arc::new(RuntimeMetrics::default()));
        let mut sequence = 50;
        assert!(queue_semantic_updates(
            &outbound,
            &mut sequence,
            &second,
            &replacement,
            &player,
            &player,
        ));
        assert!(queue_scene(
            &outbound,
            &mut sequence,
            ServerMessageType::SceneSnapshot,
            &replacement,
            &crate::runtime_content::ScenePresentation::default(),
        ));
        let replacement_frames = std::iter::from_fn(|| outbound.pop()).collect::<Vec<_>>();
        assert_eq!(replacement_frames.len(), 1);
        assert_eq!(replacement_frames[0].value["type"], "scene.snapshot");

        let outbound = OutboundHandle::new(4, Arc::new(RuntimeMetrics::default()));
        let mut sequence = 100;
        assert!(queue_sync_recovery(
            &outbound,
            &mut sequence,
            99,
            &second,
            &crate::runtime_content::ScenePresentation::default(),
            &player,
        ));
        let sync_scene = outbound.pop().expect("sync scene");
        assert_eq!(
            sync_scene.value["data"]["scene"]["items"]
                .as_array()
                .unwrap()
                .len(),
            second.items.len()
        );
        assert!(serde_json::to_vec(&sync_scene.value).unwrap().len() <= MAX_WS_MESSAGE_BYTES);
    }

    #[test]
    fn maximal_trade_player_snapshot_stays_bounded_without_losing_actionable_terms() {
        let offer = |start: u64| tiamud_core::TradeOfferView {
            coins: 1_000_000_000,
            lines: (start..start + 8)
                .map(|index| tiamud_core::TradeOfferLineView {
                    target: format!("o{index:016x}"),
                    label: "l".repeat(256),
                    quantity: 8,
                    container: false,
                })
                .collect(),
        };
        let mut player = test_player_state();
        player.economy.coins = 1_000_000_000;
        player.economy.reserved_coins = 1_000_000_000;
        player.economy.spendable_coins = 0;
        player.trade = Some(tiamud_core::TradeView {
            id: "o1000000000000000".into(),
            phase: tiamud_core::TradePhase::Negotiating,
            counterparty: "o2000000000000000".into(),
            expires_in_ticks: 120,
            state_revision: 17,
            own_confirmed: false,
            counterparty_confirmed: false,
            own_offer: offer(1),
            counterparty_offer: offer(9),
            can_accept: false,
            can_confirm: true,
        });
        player.inventory.push(tiamud_core::PlayerItemView {
            id: "o3000000000000000".into(),
            label: "padding-safe carried item".into(),
            description: String::new(),
            rarity: "common".into(),
            item_level: 1,
            weight_grams: 0,
            quantity: 1,
            available_quantity: 1,
            reserved_for_trade: 0,
            stack_slots: 1,
            total_weight_grams: 0,
            transfer_policy: tiamud_core::ItemTransferPolicy::Transferable,
            bound_to_you: false,
            affix: None,
            can_give: true,
            can_trade: true,
            container_capacity: None,
            actions: Vec::new(),
        });
        player.inventory_capacity.used_slots += 1;

        let frame_len = |player: &PlayerStateView| {
            serde_json::to_vec(&json!({
                "v": 1,
                "type": "player.snapshot",
                "seq": 1,
                "data": player,
            }))
            .unwrap()
            .len()
        };
        // Put an otherwise valid player projection just over the frame limit.
        // The maximal trade labels are the only optional presentation data the
        // bounded projection may reduce.
        let desired = MAX_WS_MESSAGE_BYTES + 256;
        let mut remaining = desired.saturating_sub(frame_len(&player));
        let mut pad = |description: &mut String| {
            let added = remaining.min(2_048_usize.saturating_sub(description.len()));
            description.push_str(&"p".repeat(added));
            remaining -= added;
        };
        pad(&mut player.character.lineage.description);
        pad(&mut player.character.base_class.description);
        pad(&mut player.character.active_class.description);
        for class in &mut player.character.class_history {
            pad(&mut class.description);
        }
        if let Some(background) = &mut player.character.background {
            pad(&mut background.description);
        }
        for graph in &mut player.character.talents {
            pad(&mut graph.description);
            for node in &mut graph.nodes {
                pad(&mut node.description);
            }
        }
        for promotion in &mut player.character.promotions {
            pad(&mut promotion.class.description);
        }
        for ability in &mut player.character.abilities {
            pad(&mut ability.description);
        }
        if remaining > 0 {
            let description = &mut player.inventory[0].description;
            let added = remaining.min(4_096_usize.saturating_sub(description.len()));
            description.push_str(&"p".repeat(added));
            remaining -= added;
        }
        assert_eq!(
            remaining, 0,
            "test player must provide enough bounded prose"
        );
        assert!(frame_len(&player) > MAX_WS_MESSAGE_BYTES);

        let outbound = OutboundHandle::new(4, Arc::new(RuntimeMetrics::default()));
        let mut sequence = 1;
        assert!(queue_player(&outbound, &mut sequence, &player));
        let frame = outbound.pop().unwrap();
        let encoded = serde_json::to_vec(&frame.value).unwrap();
        assert!(encoded.len() <= MAX_WS_MESSAGE_BYTES);
        assert!(outbound.take_disconnect_reason().is_none());
        let trade = &frame.value["data"]["trade"];
        let own = trade["ownOffer"]["lines"].as_array().unwrap();
        let counterparty = trade["counterpartyOffer"]["lines"].as_array().unwrap();
        assert_eq!(own.len() + counterparty.len(), 16);
        for line in own.iter().chain(counterparty) {
            assert!(line["target"].as_str().unwrap().starts_with('o'));
            assert_eq!(line["quantity"], 8);
            assert!(!line["label"].as_str().unwrap().is_empty());
            assert!(line["label"].as_str().unwrap().len() < 256);
        }
    }

    #[test]
    fn container_pages_shed_prose_and_stay_inside_the_websocket_frame_budget() {
        let contents = ContainerContentsView {
            selector: "o9999999999999999".into(),
            items: (0..20)
                .map(|index| tiamud_core::PlayerItemView {
                    id: format!("o{index:016x}"),
                    label: format!("{index:02}{}", "l".repeat(254)),
                    description: "p".repeat(4_096),
                    rarity: "common".into(),
                    item_level: 1,
                    weight_grams: 1,
                    quantity: 1,
                    available_quantity: 1,
                    reserved_for_trade: 0,
                    stack_slots: 1,
                    total_weight_grams: 1,
                    transfer_policy: tiamud_core::ItemTransferPolicy::Transferable,
                    bound_to_you: false,
                    affix: None,
                    can_give: false,
                    can_trade: true,
                    container_capacity: None,
                    actions: vec![tiamud_core::SceneAction {
                        label: "Get".into(),
                        command: format!("get o{index:016x} from o9999999999999999"),
                    }],
                })
                .collect(),
            page: 1,
            total_pages: 1,
            capacity: tiamud_core::InventoryCapacityView {
                used_slots: 20,
                max_slots: 20,
                used_weight_grams: 20,
                max_weight_grams: 20,
            },
        };
        let outbound = OutboundHandle::new(4, Arc::new(RuntimeMetrics::default()));
        let mut sequence = 1;

        assert!(queue_container_contents(
            &outbound,
            &mut sequence,
            &contents
        ));
        let frame = outbound.pop().unwrap();
        let encoded = serde_json::to_vec(&frame.value).unwrap();
        assert!(encoded.len() <= MAX_WS_MESSAGE_BYTES);
        assert!(
            !String::from_utf8(encoded)
                .unwrap()
                .contains(&"p".repeat(256))
        );

        let trade_contents = TradeContentsView {
            trade_id: "o1111111111111111".into(),
            container: contents.selector.clone(),
            items: contents.items.clone(),
            page: contents.page,
            total_pages: contents.total_pages,
            capacity: contents.capacity.clone(),
        };
        assert!(queue_trade_contents(
            &outbound,
            &mut sequence,
            &trade_contents
        ));
        let frame = outbound.pop().unwrap();
        assert_eq!(frame.value["type"], "trade.contents");
        assert_eq!(frame.value["data"]["tradeId"], "o1111111111111111");
        let encoded = serde_json::to_vec(&frame.value).unwrap();
        assert!(encoded.len() <= MAX_WS_MESSAGE_BYTES);
        assert!(
            !String::from_utf8(encoded)
                .unwrap()
                .contains(&"p".repeat(256))
        );
    }

    #[test]
    fn semantic_updates_are_typed_bounded_and_keep_domain_ids_private() {
        let mut world = tiamud_core::World::demo();
        let session = SessionId(81);
        world
            .connect(
                session,
                CharacterSnapshot {
                    id: CharacterId(1234),
                    account_id: tiamud_core::AccountId(5678),
                    name: "Patch Tester".into(),
                    room_id: world.start_room_id().to_owned(),
                    item_locations: BTreeMap::new(),
                },
            )
            .unwrap();
        let before_scene = world.current_scene(session).unwrap();
        let before_player = world.player_state_view(session).unwrap();
        world
            .apply(session, PlayerAction::Take(ItemSelection::one("lantern")))
            .unwrap();
        let scene = world.current_scene(session).unwrap();
        let player = world.player_state_view(session).unwrap();
        let outbound = OutboundHandle::new(16, Arc::new(RuntimeMetrics::default()));
        let mut sequence = 1;

        assert!(queue_semantic_updates(
            &outbound,
            &mut sequence,
            &before_scene,
            &scene,
            &before_player,
            &player,
        ));
        let wire = std::iter::from_fn(|| outbound.pop())
            .map(|frame| frame.value.to_string())
            .collect::<String>();
        assert!(wire.contains("\"type\":\"entity.remove\""));
        assert!(wire.contains("\"type\":\"inventory.patch\""));
        assert!(wire.contains("\"inventoryCapacity\""));
        assert!(wire.contains("\"rarity\""));
        assert!(wire.contains("\"itemLevel\""));
        assert!(!wire.contains("demo:lantern-square"));
        assert!(!wire.contains("demo:blue-lantern"));
        assert!(!wire.contains("prototypeId"));
        assert!(!wire.contains("characterId"));
    }
}
