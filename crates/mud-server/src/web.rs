use crate::{
    delivery::SessionDelivery,
    operations::{
        ActiveSessionLimit, ActiveSessionLimiter, AuthRateLimiter, CommandRateLimiter,
        OutputTransport, RuntimeMetrics,
    },
    persistence::{CharacterStore, MAX_REPORT_REASON_BYTES, MAX_SOCIAL_REASON_BYTES, StoreError},
    social::{MAX_CHAT_MESSAGE_BYTES, MAX_SOCIAL_TARGET_BYTES},
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
use serde::{Deserialize, Deserializer, Serialize};
use serde_json::{Map, Value, json};
use std::{
    collections::{BTreeMap, HashMap, VecDeque},
    net::SocketAddr,
    sync::{Arc, Mutex},
    time::Instant,
};
use tiamud_core::{
    AccountRole, CharacterId, CharacterSnapshot, EconomyAction, PartyAction, PlayerAction,
    PlayerStateView, QuestAction, SceneView, SessionId, SocialAction, SocialChannelName,
    StaffAction, StaffCapability, StaffProfileKey, StaffReportResolution, WorldError, parse_action,
};
use tokio::{
    sync::{Notify, mpsc, watch},
    task::JoinHandle,
    time::{Duration, MissedTickBehavior, interval, timeout},
};
use tracing::warn;

const MAX_WS_MESSAGE_BYTES: usize = 16 * 1024;
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

#[derive(Debug, Deserialize, Eq, PartialEq)]
#[serde(deny_unknown_fields)]
#[serde(rename_all = "camelCase")]
struct ClientAuth {
    token: String,
    character_id: i64,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct ClientCapabilities {
    images: Vec<String>,
    #[serde(rename = "reducedMotion")]
    _reduced_motion: bool,
    #[serde(default)]
    audio: bool,
}

#[derive(Debug, Eq, PartialEq)]
struct ClientHelloCredentials {
    token: String,
    character_id: i64,
    audio: bool,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ClientHelloData {
    client: String,
    capabilities: ClientCapabilities,
    auth: ClientAuth,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ClientHello {
    v: u8,
    #[serde(rename = "type")]
    message_type: String,
    data: ClientHelloData,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ClientEnvelope {
    v: u8,
    #[serde(rename = "type")]
    message_type: String,
    #[serde(default)]
    id: Option<String>,
    data: Value,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
struct CommandSubmitData {
    request_id: String,
    command: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
struct ActionPerformData {
    request_id: String,
    action: String,
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

#[derive(Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
struct SyncRequestData {
    after_seq: u64,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct HeartbeatData {
    nonce: u64,
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
}

#[derive(Debug)]
enum ClientMessage {
    Action(RequestedAction),
    Sync { after_seq: u64 },
    Heartbeat { nonce: u64 },
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
    Response::builder()
        .status(StatusCode::OK)
        .header(
            header::CONTENT_TYPE,
            "text/plain; version=0.0.4; charset=utf-8",
        )
        .body(Body::from(
            state.metrics.prometheus(state.world.queue_depth()),
        ))
        .expect("valid metrics response")
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
        let _ = send_json(
            &mut socket,
            json!({"v": 1, "type": "server.notice", "seq": 1, "data": {"level": "error", "code": "access.protocol_rejected", "message": "Expected an authenticated client.hello"}}),
        )
        .await;
        let _ = socket.send(Message::Close(None)).await;
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
            let _ = send_json(
                &mut socket,
                json!({"v": 1, "type": "server.notice", "seq": 1, "data": {"level": "error", "code": code, "message": public_game_error(&error)}}),
            )
            .await;
            let _ = socket.send(Message::Close(None)).await;
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
            let _ = send_json(
                &mut socket,
                json!({"v": 1, "type": "server.notice", "seq": 1, "data": {"level": "error", "code": "access.session_limit", "message": message}}),
            )
            .await;
            let _ = socket.send(Message::Close(None)).await;
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
    let mut last_scene = entered.scene;
    let mut last_presentation = entered.presentation;
    let mut last_player = entered.player;
    let mut ignore_world_through_revision = None;
    if !queue_sequenced(
        &outbound,
        &mut sequence,
        OutboundPriority::Control,
        None,
        "server.welcome",
        json!({
            "serverName": "Tiamud",
            "motd": "Welcome to Tiamud, a text-complete world of adventures.",
            "sessionId": entered.session_id.0.to_string()
        }),
    ) || !queue_scene(
        &outbound,
        &mut sequence,
        "scene.snapshot",
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
                        "server.notice",
                        json!({
                            "level": "info",
                            "message": "Server shutdown is starting; new traffic is stopping and accepted actions will be drained."
                        }),
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
                        for event in delivery.events {
                            if !queue_view_event(
                                &outbound,
                                &mut sequence,
                                event,
                                &last_presentation,
                                &delivery.presentation,
                            ) {
                                break 'session;
                            }
                        }
                        if delivery.scene.revision <= last_scene.revision {
                            continue;
                        }
                        let previous_scene = last_scene.clone();
                        let previous_player = last_player.clone();
                        last_scene = delivery.scene;
                        last_presentation = delivery.presentation;
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
                            "scene.snapshot",
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
                                last_scene = scene;
                                last_presentation = presentation;
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
                                    "scene.snapshot",
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
                            "chat.message",
                            json!(delivery),
                        ) {
                            break;
                        }
                    }
                    Some(SessionDelivery::SocialLagged) => {
                        if !queue_notice(
                            &outbound,
                            &mut sequence,
                            "Some chat messages were skipped because this client fell behind.",
                        ) {
                            break;
                        }
                    }
                    None => break,
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
                        ) {
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
                                cached.message,
                            ) || !queue_scene(
                                &outbound,
                                &mut sequence,
                                "scene.snapshot",
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
                        let (ok, message, events) = match requested.action {
                            Ok(action) => match state.world.apply(entered.session_id, action).await {
                                Ok(outcome) => {
                                    state.metrics.action_finished(action_started.elapsed(), true);
                                    last_scene = outcome.scene;
                                    last_presentation = outcome.presentation;
                                    last_player = outcome.player;
                                    (true, outcome.output, outcome.events)
                                }
                                Err(error) => {
                                    state.metrics.action_finished(action_started.elapsed(), false);
                                    warn!(%error, "game action failed");
                                    (false, Some(public_game_error(&error)), Vec::new())
                                }
                            },
                            Err(error) => {
                                state.metrics.action_finished(action_started.elapsed(), false);
                                (false, Some(error), Vec::new())
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
                        for event in events {
                            if !queue_view_event(
                                &outbound,
                                &mut sequence,
                                event,
                                &previous_presentation,
                                &last_presentation,
                            ) {
                                break 'session;
                            }
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
                            "scene.snapshot",
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
                let result = outbound.enqueue(OutboundEnvelope {
                    priority: OutboundPriority::BestEffort,
                    coalesce: Some(CoalesceKey::Heartbeat),
                    sequence: None,
                    value: json!({
                        "v": 1,
                        "type": "server.heartbeat",
                        "data": {
                            "nonce": heartbeat_nonce,
                            "timeoutMs": HEARTBEAT_TIMEOUT.as_millis()
                        }
                    }),
                });
                if result == EnqueueResult::Disconnect {
                    break;
                }
            }
        }
    }

    finish_websocket_driver(&outbound, driver).await;
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

fn queue_sequenced(
    outbound: &OutboundHandle,
    sequence: &mut u64,
    priority: OutboundPriority,
    coalesce: Option<CoalesceKey>,
    message_type: &'static str,
    data: Value,
) -> bool {
    let current = *sequence;
    *sequence = sequence.wrapping_add(1).max(1);
    outbound.enqueue(OutboundEnvelope {
        priority,
        coalesce,
        sequence: Some(current),
        value: json!({"v": 1, "type": message_type, "seq": current, "data": data}),
    }) != EnqueueResult::Disconnect
}

fn queue_scene(
    outbound: &OutboundHandle,
    sequence: &mut u64,
    message_type: &'static str,
    scene: &SceneView,
    presentation: &crate::runtime_content::ScenePresentation,
) -> bool {
    let mut presentation = presentation.clone();
    loop {
        let mut data = json!(scene);
        if !presentation.is_empty() {
            data["presentation"] = json!(presentation);
        }
        let candidate = json!({
            "v": 1,
            "type": message_type,
            "seq": *sequence,
            "data": data,
        });
        if serde_json::to_vec(&candidate).is_ok_and(|bytes| bytes.len() <= MAX_WS_MESSAGE_BYTES)
            || presentation.entities.is_empty()
        {
            return queue_sequenced(
                outbound,
                sequence,
                OutboundPriority::State,
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

fn queue_player(outbound: &OutboundHandle, sequence: &mut u64, player: &PlayerStateView) -> bool {
    queue_sequenced(
        outbound,
        sequence,
        OutboundPriority::State,
        Some(CoalesceKey::Player),
        "player.snapshot",
        json!(player),
    )
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
        let data = json!({
            "syncId": sync_id,
            "afterSeq": after_seq,
            "scene": scene_with_presentation(scene, &presentation),
        });
        let candidate = json!({"v": 1, "type": "sync.scene", "seq": *sequence, "data": data});
        if serde_json::to_vec(&candidate).is_ok_and(|bytes| bytes.len() <= MAX_WS_MESSAGE_BYTES)
            || presentation.entities.is_empty()
        {
            break queue_sequenced(
                outbound,
                sequence,
                OutboundPriority::Critical,
                Some(CoalesceKey::SyncScene),
                "sync.scene",
                data,
            );
        }
        presentation.entities.pop();
    };
    scene_queued
        && queue_sequenced(
            outbound,
            sequence,
            OutboundPriority::Critical,
            Some(CoalesceKey::SyncPlayer),
            "sync.player",
            json!({
                "syncId": sync_id,
                "afterSeq": after_seq,
                "player": player,
                "complete": true,
            }),
        )
}

fn scene_with_presentation(
    scene: &SceneView,
    presentation: &crate::runtime_content::ScenePresentation,
) -> Value {
    let mut value = json!(scene);
    if !presentation.is_empty() {
        value["presentation"] = json!(presentation);
    }
    value
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
        && safe_audio_channel(&previous.channel)
        && !queue_sequenced(
            outbound,
            sequence,
            OutboundPriority::Critical,
            None,
            "media.stop",
            json!({"channel": previous.channel}),
        )
    {
        return false;
    }
    next.is_none_or(|next| {
        !safe_scene_audio(next)
            || queue_sequenced(
                outbound,
                sequence,
                OutboundPriority::State,
                None,
                "media.play",
                json!({"channel": next.channel, "url": next.url, "loop": next.looped}),
            )
    })
}

fn safe_scene_audio(audio: &tiamud_core::SceneAudio) -> bool {
    safe_audio_channel(&audio.channel) && safe_content_addressed_audio_url(&audio.url)
}

fn safe_audio_channel(channel: &str) -> bool {
    matches!(channel, "ambience" | "effects" | "music")
}

fn safe_content_addressed_audio_url(value: &str) -> bool {
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
    let previous_scene_json = json!(previous_scene);
    let scene_json = json!(scene);
    let Some(previous_scene_object) = previous_scene_json.as_object() else {
        return false;
    };
    let Some(scene_object) = scene_json.as_object() else {
        return false;
    };

    let mut patch = Map::from_iter([
        ("viewId".to_owned(), json!(scene.view_id)),
        ("revision".to_owned(), json!(scene.revision)),
    ]);
    for field in ["title", "prose", "image", "exits", "weather"] {
        if previous_scene_object.get(field) != scene_object.get(field) {
            patch.insert(
                field.to_owned(),
                scene_object.get(field).cloned().unwrap_or(Value::Null),
            );
        }
    }
    if patch.len() > 2
        && !queue_sequenced(
            outbound,
            sequence,
            OutboundPriority::State,
            Some(CoalesceKey::ScenePatch),
            "scene.patch",
            Value::Object(patch),
        )
    {
        return false;
    }

    let previous_entities = scene_entities(previous_scene_object);
    let entities = scene_entities(scene_object);
    for ((kind, id), _) in previous_entities
        .iter()
        .filter(|(key, _)| !entities.contains_key(*key))
    {
        if !queue_sequenced(
            outbound,
            sequence,
            OutboundPriority::State,
            None,
            "entity.remove",
            json!({
                "viewId": scene.view_id,
                "revision": scene.revision,
                "kind": kind,
                "id": id,
            }),
        ) {
            return false;
        }
    }
    for ((kind, id), entity) in entities.iter().filter(|(key, entity)| {
        previous_entities
            .get(*key)
            .is_none_or(|previous| previous != *entity)
    }) {
        if !queue_sequenced(
            outbound,
            sequence,
            OutboundPriority::State,
            None,
            "entity.upsert",
            json!({
                "viewId": scene.view_id,
                "revision": scene.revision,
                "kind": kind,
                "id": id,
                "entity": entity,
            }),
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
            "vitals.update",
            json!({"revision": player.revision, "character": player.character}),
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
            "inventory.patch",
            json!({
                "revision": player.revision,
                "inventory": player.inventory,
                "equipment": player.equipment,
                "inventoryCapacity": player.inventory_capacity,
            }),
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
            "map.snapshot",
            json!({"revision": player.revision, "map": player.map}),
        )
    {
        return false;
    }
    let other_player_state_changed = previous_player.quests != player.quests
        || previous_player.party != player.party
        || previous_player.party_invites != player.party_invites
        || previous_player.economy != player.economy
        || previous_player.social != player.social;
    !other_player_state_changed || queue_player(outbound, sequence, player)
}

fn scene_entities(scene: &Map<String, Value>) -> BTreeMap<(String, String), Value> {
    let mut entities = BTreeMap::new();
    for (kind, field) in [("item", "items"), ("npc", "npcs")] {
        let Some(values) = scene.get(field).and_then(Value::as_array) else {
            continue;
        };
        for value in values {
            if let Some(id) = value.get("id").and_then(Value::as_str) {
                entities.insert((kind.to_owned(), id.to_owned()), value.clone());
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
        "action.result",
        json!({"requestId": request_id, "ok": ok, "message": message}),
    )
}

fn queue_notice(outbound: &OutboundHandle, sequence: &mut u64, message: &str) -> bool {
    queue_sequenced(
        outbound,
        sequence,
        OutboundPriority::Critical,
        None,
        "server.notice",
        json!({"level": "error", "message": message}),
    )
}

fn queue_view_event(
    outbound: &OutboundHandle,
    sequence: &mut u64,
    event: ProjectedViewEvent,
    previous_presentation: &crate::runtime_content::ScenePresentation,
    current_presentation: &crate::runtime_content::ScenePresentation,
) -> bool {
    let mut data = json!(event.event);
    if let Some(cue) = event.combat_cue.filter(|cue| {
        previous_presentation.contains_entity(&cue.entity_id)
            || current_presentation.contains_entity(&cue.entity_id)
    }) {
        data["cue"] = match cue.kind {
            CombatCueKind::Attack => json!({
                "kind": "attack",
                "entityId": cue.entity_id,
                "pose": "attack",
                "intensity": 2,
                "durationMs": 420,
            }),
            CombatCueKind::Hit => json!({
                "kind": "hit",
                "entityId": cue.entity_id,
                "pose": "hit",
                "intensity": 2,
                "durationMs": 320,
            }),
            CombatCueKind::Defeat => json!({
                "kind": "defeat",
                "entityId": cue.entity_id,
                "pose": "defeated",
                "intensity": 3,
                "durationMs": 700,
            }),
        };
    }
    queue_sequenced(
        outbound,
        sequence,
        OutboundPriority::Narrative,
        None,
        "message.append",
        data,
    )
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
            Response::builder()
                .status(StatusCode::OK)
                .header(header::CONTENT_TYPE, mime.as_ref())
                .header(header::CACHE_CONTROL, cache)
                .header(header::X_CONTENT_TYPE_OPTIONS, "nosniff")
                .header(header::REFERRER_POLICY, "no-referrer")
                .header(
                    header::CONTENT_SECURITY_POLICY,
                    "default-src 'self'; script-src 'self'; style-src 'self'; img-src 'self'; connect-src 'self' ws: wss:; object-src 'none'; base-uri 'none'; frame-ancestors 'none'",
                )
                .body(Body::from(asset.data.into_owned()))
                .expect("valid static response")
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
    let message = serde_json::from_str::<ClientHello>(text).ok()?;
    (message.v == 1
        && message.message_type == "client.hello"
        && message.data.client == "web"
        && (message.data.capabilities.images.is_empty()
            || message.data.capabilities.images
                == ["avif".to_owned(), "webp".to_owned(), "jpeg".to_owned()])
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
    let envelope = serde_json::from_str::<ClientEnvelope>(text)
        .map_err(|_| ClientMessageError::MalformedJson)?;
    if envelope.v != 1 {
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
            let data = serde_json::from_value::<CommandSubmitData>(envelope.data)
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
            }))
        }
        "action.perform" => {
            let data = serde_json::from_value::<ActionPerformData>(envelope.data)
                .map_err(|_| ClientMessageError::InvalidEnvelope)?;
            if !valid_request_id(&data.request_id)
                || data.action.is_empty()
                || data.action.len() > 64
                || data.action.chars().any(char::is_control)
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
                "action:{}\0{}\0{}\0{}\0{}\0{}\0{}\0{:?}\0{:?}\0{}\0{:?}\0{:?}\0{:?}\0{:?}\0{:?}\0{:?}\0{:?}",
                data.action,
                data.ability_id.as_deref().unwrap_or_default(),
                data.class_id.as_deref().unwrap_or_default(),
                data.graph_id.as_deref().unwrap_or_default(),
                data.node_id.as_deref().unwrap_or_default(),
                data.target.as_deref().unwrap_or_default(),
                data.message.as_deref().unwrap_or_default(),
                data.channel,
                data.duration_seconds,
                data.reason.as_deref().unwrap_or_default(),
                data.role,
                data.report_id,
                data.resolution,
                data.profile,
                data.lore_title,
                data.capability,
                data.enabled,
            );
            let action = structured_action(&data);
            Ok(ClientMessage::Action(RequestedAction {
                request_id: data.request_id,
                signature,
                action,
            }))
        }
        "sync.request" => {
            let data = serde_json::from_value::<SyncRequestData>(envelope.data)
                .map_err(|_| ClientMessageError::InvalidEnvelope)?;
            Ok(ClientMessage::Sync {
                after_seq: data.after_seq,
            })
        }
        "client.heartbeat" => {
            let data = serde_json::from_value::<HeartbeatData>(envelope.data)
                .map_err(|_| ClientMessageError::InvalidEnvelope)?;
            if data.nonce == 0 {
                return Err(ClientMessageError::InvalidEnvelope);
            }
            Ok(ClientMessage::Heartbeat { nonce: data.nonce })
        }
        _ => Err(ClientMessageError::UnknownType),
    }
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

fn structured_action(data: &ActionPerformData) -> Result<PlayerAction, String> {
    let action = data.action.as_str();
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
    match action {
        "look" => exact_no_fields().map(|()| PlayerAction::Look),
        "inventory" => exact_no_fields().map(|()| PlayerAction::Inventory),
        "move" => {
            let target = gameplay_target()?;
            match parse_action(&target) {
                Ok(action @ PlayerAction::Move(_)) => Ok(action),
                _ => Err(format!("Unknown movement direction: {target}")),
            }
        }
        "item.take" => gameplay_target().map(PlayerAction::Take),
        "item.drop" => gameplay_target().map(PlayerAction::Drop),
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

fn public_game_error(error: &GameError) -> String {
    match error {
        GameError::World(WorldError::NoExit) => error.to_string(),
        GameError::World(WorldError::ItemNotFound(_))
        | GameError::World(WorldError::AmbiguousItem { .. })
        | GameError::World(WorldError::CharacterAlreadyConnected)
        | GameError::World(WorldError::GameplayUnavailable)
        | GameError::World(WorldError::AbilityNotKnown)
        | GameError::World(WorldError::AbilityUnavailable)
        | GameError::World(WorldError::InsufficientResource)
        | GameError::World(WorldError::InvalidAbilityTarget)
        | GameError::World(WorldError::ClassNotFound)
        | GameError::World(WorldError::PromotionUnavailable)
        | GameError::World(WorldError::TalentUnavailable)
        | GameError::World(WorldError::InsufficientProgressionPoints) => error.to_string(),
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
            "You cannot send chat while your account is muted.".to_owned()
        }
        GameError::Persistence(StoreError::ChatRateLimited) => {
            "Chat rate limit exceeded; wait before sending another message.".to_owned()
        }
        GameError::Persistence(StoreError::InvalidSocialInput) => {
            "The social action is invalid or rate limited.".to_owned()
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
    Response::builder()
        .status(StatusCode::TOO_MANY_REQUESTS)
        .header(header::RETRY_AFTER, "60")
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(
            json!({"error": "Too many authentication attempts. Try again later."}).to_string(),
        ))
        .expect("valid rate limit response")
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

#[cfg(test)]
mod tests {
    use super::*;

    fn requested_action(message: &str) -> RequestedAction {
        let ClientMessage::Action(action) = parse_client_message(message).unwrap() else {
            panic!("message must parse as an action");
        };
        action
    }

    fn test_entity_presentation() -> crate::runtime_content::ScenePresentation {
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
            Ok(PlayerAction::Take("blue lantern".to_owned()))
        );

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
    fn structured_party_reply_and_channel_actions_map_exactly() {
        let cases = [
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
        };
        shared.lock().unwrap().insert(first.clone(), cached.clone());

        let cache = shared.lock().unwrap();
        assert_eq!(cache.get(&first), Some(&cached));
        assert!(cache.get(&second).is_none());
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
            .apply(session, PlayerAction::Take("lantern".into()))
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
            "scene.snapshot",
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
            "scene.snapshot",
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
                actions: Vec::new(),
            })
            .collect();

        let metrics = Arc::new(RuntimeMetrics::default());
        let outbound = OutboundHandle::new(4, metrics.clone());
        let mut sequence = 1;
        assert!(!queue_scene(
            &outbound,
            &mut sequence,
            "scene.snapshot",
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
            .apply(session, PlayerAction::Take("lantern".into()))
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
