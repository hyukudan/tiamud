mod persistence;
mod telnet;
mod web;
mod world_actor;

use axum::{Router, routing::get};
use persistence::{CharacterStore, SqliteCharacterStore};
use std::sync::atomic::AtomicU64;
use std::{
    env,
    error::Error,
    net::SocketAddr,
    path::Path,
    sync::Arc,
    time::{SystemTime, UNIX_EPOCH},
};
use tokio::{net::TcpListener, signal};
use tower_http::trace::TraceLayer;
use tracing::{error, info};
use tracing_subscriber::EnvFilter;
use web::AppState;
use world_actor::WorldHandle;

#[tokio::main]
async fn main() -> Result<(), Box<dyn Error + Send + Sync>> {
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "mud_server=info,tower_http=info".into()),
        )
        .try_init()?;

    let http_address: SocketAddr = env_value("MUD_HTTP_ADDR", "127.0.0.1:8080").parse()?;
    let telnet_address: SocketAddr = env_value("MUD_TELNET_ADDR", "127.0.0.1:4000").parse()?;
    let database_path = env_value("MUD_DB_PATH", "data/mud.db");
    create_database_parent(&database_path)?;

    let store: Arc<dyn CharacterStore> =
        Arc::new(SqliteCharacterStore::open(&database_path).await?);
    let world = WorldHandle::spawn(store);
    // Guest identities must not accidentally resume an unrelated guest's
    // persisted character after a process restart. Authenticated identities
    // will replace this process-local naming scheme later.
    let guest_counter = Arc::new(AtomicU64::new(guest_counter_seed()));
    let state = AppState {
        world: world.clone(),
        guest_counter: guest_counter.clone(),
    };

    let app = Router::new()
        .route("/health/live", get(web::live))
        .route("/health/ready", get(web::ready))
        .route("/api/status", get(web::status))
        .route("/ws", get(web::ws_handler))
        .fallback(web::static_asset)
        .with_state(state)
        .layer(TraceLayer::new_for_http());

    let telnet_listener = TcpListener::bind(telnet_address).await?;
    info!(%telnet_address, "telnet listener ready");
    let telnet_task = tokio::spawn(async move {
        if let Err(error) = telnet::run(telnet_listener, world, guest_counter).await {
            error!(%error, "telnet listener stopped");
        }
    });

    let http_listener = TcpListener::bind(http_address).await?;
    info!(%http_address, database = %database_path, "browser server ready");
    axum::serve(http_listener, app)
        .with_graceful_shutdown(shutdown_signal())
        .await?;
    telnet_task.abort();
    Ok(())
}

fn env_value(name: &str, default: &str) -> String {
    env::var(name).unwrap_or_else(|_| default.to_owned())
}

fn guest_counter_seed() -> u64 {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    let folded = (nanos as u64) ^ ((nanos >> 64) as u64) ^ u64::from(std::process::id());
    folded.max(1)
}

fn create_database_parent(database_path: &str) -> std::io::Result<()> {
    if database_path == ":memory:" {
        return Ok(());
    }
    if let Some(parent) = Path::new(database_path)
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
    {
        std::fs::create_dir_all(parent)?;
    }
    Ok(())
}

async fn shutdown_signal() {
    let ctrl_c = async {
        signal::ctrl_c()
            .await
            .expect("failed to install Ctrl+C handler");
    };

    #[cfg(unix)]
    let terminate = async {
        signal::unix::signal(signal::unix::SignalKind::terminate())
            .expect("failed to install SIGTERM handler")
            .recv()
            .await;
    };

    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();

    tokio::select! {
        () = ctrl_c => {},
        () = terminate => {},
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bare_database_filename_does_not_try_to_create_an_empty_directory() {
        create_database_parent("mud.db").unwrap();
        create_database_parent(":memory:").unwrap();
    }
}
