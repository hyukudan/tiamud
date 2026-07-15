use crate::world_actor::{GameError, WorldHandle};
use axum::{
    Json,
    body::Body,
    extract::{
        State, WebSocketUpgrade,
        ws::{Message, WebSocket},
    },
    http::{HeaderMap, StatusCode, Uri, header},
    response::{IntoResponse, Response},
};
use mud_core::{SceneView, SessionId, WorldError, parse_action};
use rust_embed::RustEmbed;
use serde_json::{Value, json};
use std::sync::{
    Arc,
    atomic::{AtomicU64, Ordering},
};
use tokio::time::{Duration, timeout};
use tracing::warn;

const MAX_WS_MESSAGE_BYTES: usize = 16 * 1024;
const MAX_COMMAND_BYTES: usize = 1024;
const MAX_REQUEST_ID_BYTES: usize = 128;
const MAX_PROTOCOL_ERRORS: u8 = 3;

#[derive(Clone)]
pub struct AppState {
    pub world: WorldHandle,
    pub guest_counter: Arc<AtomicU64>,
}

#[derive(RustEmbed)]
#[cfg(web_dist)]
#[folder = "../../apps/web/dist"]
struct WebAssets;

struct SessionLease {
    world: WorldHandle,
    session: SessionId,
}

impl Drop for SessionLease {
    fn drop(&mut self) {
        self.world.leave_nowait(self.session);
    }
}

#[derive(RustEmbed)]
#[cfg(not(web_dist))]
#[folder = "web-fallback"]
struct WebAssets;

pub async fn live() -> Json<Value> {
    Json(json!({ "status": "ok" }))
}

pub async fn ready() -> Json<Value> {
    Json(json!({ "status": "ready", "storage": "sqlite" }))
}

pub async fn status() -> Json<Value> {
    Json(json!({
        "name": "Project Envy",
        "version": env!("CARGO_PKG_VERSION"),
        "protocol": "mud.v1",
        "images": ["avif", "webp", "jpeg"]
    }))
}

pub async fn ws_handler(
    State(state): State<AppState>,
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
    websocket
        .protocols(["mud.v1"])
        .max_message_size(MAX_WS_MESSAGE_BYTES)
        .max_frame_size(MAX_WS_MESSAGE_BYTES)
        .on_upgrade(move |socket| websocket_session(socket, state))
        .into_response()
}

async fn websocket_session(mut socket: WebSocket, state: AppState) {
    let hello = timeout(Duration::from_secs(10), socket.recv()).await;
    let valid_hello = match hello {
        Ok(Some(Ok(Message::Text(text)))) => valid_client_hello(&text),
        _ => false,
    };
    if !valid_hello {
        let _ = send_json(
            &mut socket,
            json!({"v": 1, "type": "server.notice", "data": {"level": "error", "message": "Expected client.hello"}}),
        )
        .await;
        let _ = socket.send(Message::Close(None)).await;
        return;
    }

    let guest = state.guest_counter.fetch_add(1, Ordering::Relaxed);
    let entered = match state.world.enter(format!("WebGuest-{guest}")).await {
        Ok(entered) => entered,
        Err(error) => {
            warn!(%error, "could not enter the world");
            let _ = send_json(
                &mut socket,
                json!({"v": 1, "type": "server.notice", "data": {"level": "error", "message": "The world is temporarily unavailable."}}),
            )
            .await;
            return;
        }
    };
    // Cancellation, a send error, and a normal close all clean the world
    // session through this guard.
    let _session_lease = SessionLease {
        world: state.world.clone(),
        session: entered.session_id,
    };

    let mut sequence = 1_u64;
    if send_json(
        &mut socket,
        json!({
            "v": 1,
            "type": "server.welcome",
            "seq": sequence,
            "data": {
                "serverName": "Project Envy",
                "motd": "A browser-first, text-complete MUD prototype.",
                "sessionId": entered.session_id.0.to_string()
            }
        }),
    )
    .await
    .is_err()
    {
        return;
    }
    sequence += 1;
    if send_scene(&mut socket, sequence, &entered.scene)
        .await
        .is_err()
    {
        return;
    }

    let mut protocol_errors = 0_u8;
    while let Some(message) = socket.recv().await {
        let Ok(Message::Text(text)) = message else {
            if matches!(message, Ok(Message::Close(_)) | Err(_)) {
                break;
            }
            continue;
        };
        let Ok(envelope) = serde_json::from_str::<Value>(&text) else {
            sequence += 1;
            let _ = send_notice(&mut socket, sequence, "Malformed JSON command.").await;
            protocol_errors += 1;
            if protocol_errors >= MAX_PROTOCOL_ERRORS {
                break;
            }
            continue;
        };
        if envelope.get("v").and_then(Value::as_u64) != Some(1) {
            sequence += 1;
            let _ = send_notice(&mut socket, sequence, "Unsupported protocol version.").await;
            protocol_errors += 1;
            if protocol_errors >= MAX_PROTOCOL_ERRORS {
                break;
            }
            continue;
        }
        if envelope.get("type").and_then(Value::as_str) != Some("command.submit") {
            continue;
        }
        let data = envelope.get("data").and_then(Value::as_object);
        let request_id = data
            .and_then(|data| data.get("requestId"))
            .and_then(Value::as_str)
            .unwrap_or_default();
        let command = data
            .and_then(|data| data.get("command"))
            .and_then(Value::as_str)
            .unwrap_or_default();

        if request_id.len() > MAX_REQUEST_ID_BYTES || command.len() > MAX_COMMAND_BYTES {
            sequence += 1;
            let _ = send_notice(&mut socket, sequence, "Command envelope exceeds its limit.").await;
            protocol_errors += 1;
            if protocol_errors >= MAX_PROTOCOL_ERRORS {
                break;
            }
            continue;
        }
        protocol_errors = 0;

        match parse_action(command) {
            Ok(action) => match state.world.apply(entered.session_id, action).await {
                Ok(outcome) => {
                    sequence += 1;
                    if send_action_result(&mut socket, sequence, request_id, true, None)
                        .await
                        .is_err()
                    {
                        break;
                    }
                    sequence += 1;
                    if send_scene(&mut socket, sequence, &outcome.scene)
                        .await
                        .is_err()
                    {
                        break;
                    }
                }
                Err(error) => {
                    warn!(%error, "game action failed");
                    sequence += 1;
                    if send_action_result(
                        &mut socket,
                        sequence,
                        request_id,
                        false,
                        Some(public_game_error(&error)),
                    )
                    .await
                    .is_err()
                    {
                        break;
                    }
                }
            },
            Err(error) => {
                sequence += 1;
                if send_action_result(
                    &mut socket,
                    sequence,
                    request_id,
                    false,
                    Some(error.to_string()),
                )
                .await
                .is_err()
                {
                    break;
                }
            }
        }
    }
}

async fn send_scene(
    socket: &mut WebSocket,
    sequence: u64,
    scene: &SceneView,
) -> Result<(), axum::Error> {
    send_json(
        socket,
        json!({"v": 1, "type": "scene.snapshot", "seq": sequence, "data": scene}),
    )
    .await
}

async fn send_action_result(
    socket: &mut WebSocket,
    sequence: u64,
    request_id: &str,
    ok: bool,
    message: Option<String>,
) -> Result<(), axum::Error> {
    send_json(
        socket,
        json!({
            "v": 1,
            "type": "action.result",
            "seq": sequence,
            "data": {"requestId": request_id, "ok": ok, "message": message}
        }),
    )
    .await
}

async fn send_notice(
    socket: &mut WebSocket,
    sequence: u64,
    message: &str,
) -> Result<(), axum::Error> {
    send_json(
        socket,
        json!({"v": 1, "type": "server.notice", "seq": sequence, "data": {"level": "error", "message": message}}),
    )
    .await
}

async fn send_json(socket: &mut WebSocket, value: Value) -> Result<(), axum::Error> {
    socket.send(Message::Text(value.to_string().into())).await
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
            let cache = if asset_path == "index.html" {
                "no-cache"
            } else {
                "public, max-age=31536000, immutable"
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

fn valid_client_hello(text: &str) -> bool {
    if text.len() > MAX_WS_MESSAGE_BYTES {
        return false;
    }
    let Ok(message) = serde_json::from_str::<Value>(text) else {
        return false;
    };
    message.get("v").and_then(Value::as_u64) == Some(1)
        && message.get("type").and_then(Value::as_str) == Some("client.hello")
        && message.get("data").is_some_and(Value::is_object)
}

fn public_game_error(error: &GameError) -> String {
    match error {
        GameError::World(WorldError::NoExit) => error.to_string(),
        GameError::World(WorldError::UnknownSession) => "This session has expired.".to_owned(),
        GameError::World(WorldError::InvalidEntity)
        | GameError::Persistence(_)
        | GameError::ActorStopped => "The world is temporarily unavailable.".to_owned(),
    }
}

fn websocket_request_allowed(headers: &HeaderMap) -> bool {
    let supports_protocol = headers
        .get_all(header::SEC_WEBSOCKET_PROTOCOL)
        .iter()
        .filter_map(|value| value.to_str().ok())
        .flat_map(|value| value.split(','))
        .any(|protocol| protocol.trim() == "mud.v1");
    if !supports_protocol {
        return false;
    }

    let Some(origin) = headers.get(header::ORIGIN) else {
        // Non-browser reference clients may omit Origin. Authentication will
        // impose a stricter policy when it is introduced.
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
    origin
        .authority()
        .is_some_and(|authority| authority.as_str().eq_ignore_ascii_case(host))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hello_requires_the_current_version_and_data_object() {
        assert!(valid_client_hello(
            r#"{"v":1,"type":"client.hello","data":{}}"#
        ));
        assert!(!valid_client_hello(
            r#"{"v":2,"type":"client.hello","data":{}}"#
        ));
        assert!(!valid_client_hello(r#"{"v":1,"type":"client.hello"}"#));
    }

    #[test]
    fn browser_websockets_must_be_same_origin_and_use_the_protocol() {
        let mut headers = HeaderMap::new();
        headers.insert(header::HOST, "mud.example:8443".parse().unwrap());
        headers.insert(
            header::SEC_WEBSOCKET_PROTOCOL,
            "chat, mud.v1".parse().unwrap(),
        );
        headers.insert(header::ORIGIN, "https://mud.example:8443".parse().unwrap());
        assert!(websocket_request_allowed(&headers));

        headers.insert(header::ORIGIN, "https://evil.example".parse().unwrap());
        assert!(!websocket_request_allowed(&headers));
        headers.remove(header::ORIGIN);
        headers.remove(header::SEC_WEBSOCKET_PROTOCOL);
        assert!(!websocket_request_allowed(&headers));
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
    }
}
