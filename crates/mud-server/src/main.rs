mod area_runtime;
mod auth;
mod boards;
mod delivery;
mod operations;
mod persistence;
mod persistence_postgres;
mod runtime_content;
mod social;
mod staff;
mod telnet;
mod telnet_transport;
mod web;
mod world_actor;
mod world_loader;

use area_runtime::AreaLoadCoordinator;
use axum::{
    Router,
    extract::DefaultBodyLimit,
    routing::{get, post},
};
use operations::{ActiveSessionLimiter, AuthRateLimiter, RuntimeMetrics};
use persistence::{CharacterStore, MeasuredCharacterStore, SqliteCharacterStore};
use persistence_postgres::PostgresCharacterStore;
use std::{env, error::Error, net::SocketAddr, path::Path, sync::Arc};
use thiserror::Error;
use tokio::{
    net::TcpListener,
    signal,
    sync::watch,
    time::{Duration, timeout},
};
use tower_http::trace::TraceLayer;
use tracing::{error, info, warn};
use tracing_subscriber::EnvFilter;
use web::AppState;
use world_actor::{WorldHandle, WorldSchedulerConfig};
use world_loader::{
    LoadedWorldDefinition, PinnedWorldContent, build_loaded_world, default_loaded_world,
    load_loaded_world, parse_pinned_world_content,
};
use world_pack::LocalAreaGeneration;

#[tokio::main]
async fn main() -> Result<(), Box<dyn Error + Send + Sync>> {
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "tiamud_server=info,tower_http=info".into()),
        )
        .try_init()?;

    let http_address: SocketAddr = env_value("TIAMUD_HTTP_ADDR", "127.0.0.1:8080").parse()?;
    let telnet_address: SocketAddr = env_value("TIAMUD_TELNET_ADDR", "127.0.0.1:4000").parse()?;
    let telnet_transport = telnet_transport::TelnetTransport::from_environment()?;
    if !telnet_transport.is_tls() {
        warn!(
            "UNENCRYPTED TELNET IS ENABLED: credentials and game traffic are exposed; use only on a trusted private network"
        );
    }
    let database = database_config(
        optional_env("TIAMUD_DATABASE_URL")?,
        optional_env("TIAMUD_DB_PATH")?,
    )?;
    let world_source = world_source_config(
        optional_env("TIAMUD_WORLD_PACK")?,
        optional_env("TIAMUD_AREA_REGISTRY")?,
        optional_env("TIAMUD_AREA_CATALOG")?,
    )?;
    // Catalog mode must fail before opening a backend because opening either
    // backend may create a database or run migrations. Pin and validate the
    // exact generation, including its verified monolithic fallback, first.
    let prepared_world = prepare_world_before_database(&database, world_source)?;
    let metrics = Arc::new(RuntimeMetrics::default());
    // Exactly one branch is opened. Runtime startup never copies between or
    // writes to both persistence backends.
    let store: Arc<dyn CharacterStore> = match &database {
        DatabaseConfig::Sqlite(path) => Arc::new(SqliteCharacterStore::open(path).await?),
        DatabaseConfig::Postgres(url) => Arc::new(PostgresCharacterStore::open(url).await?),
    };
    let store: Arc<dyn CharacterStore> =
        Arc::new(MeasuredCharacterStore::new(store, metrics.clone()));
    let store_shutdown = store.clone();
    let recovery = store.load_startup_world_recovery().await?;
    let tick_millis = env_u32("TIAMUD_TICK_MILLIS", 1_000)?;
    let scheduler = WorldSchedulerConfig::every(Duration::from_millis(u64::from(tick_millis)));
    let (world, world_pack_source) = match prepared_world {
        PreparedWorldSource::Monolithic { loaded, source } => {
            let loaded = *loaded;
            let world = build_loaded_world(&loaded)?;
            let runtime_content = loaded
                .runtime_content
                .with_operational_metrics(metrics.clone());
            let handle = WorldHandle::spawn_with_scheduler_content_areas_and_recovery(
                world,
                store.clone(),
                scheduler,
                runtime_content,
                None,
                recovery,
                false,
            )
            .await?;
            (handle, source)
        }
        PreparedWorldSource::AreaCatalog {
            generation,
            pinned,
            source,
        } => {
            let coordinator = AreaLoadCoordinator::new(*generation, *pinned, Some(metrics.clone()));
            let bootstrap = coordinator
                .bootstrap_world(recovery.requires_full_generation())
                .await?;
            let confirm_initial_dynamic_state =
                !recovery.requires_full_generation() && bootstrap.created_dynamic_state;
            let handle = WorldHandle::spawn_with_scheduler_content_areas_and_recovery(
                bootstrap.world,
                store.clone(),
                scheduler,
                bootstrap.runtime_content,
                Some(coordinator),
                recovery,
                confirm_initial_dynamic_state,
            )
            .await?;
            (handle, source)
        }
    };
    let auth_attempts_per_minute = env_u32("TIAMUD_AUTH_ATTEMPTS_PER_MINUTE", 10)?;
    let auth_burst = env_u32("TIAMUD_AUTH_BURST", 5)?;
    let command_rate_per_second = env_u32("TIAMUD_COMMANDS_PER_SECOND", 12)?;
    let command_burst = env_u32("TIAMUD_COMMAND_BURST", 20)?;
    let max_sessions_per_account = env_u32("TIAMUD_MAX_SESSIONS_PER_ACCOUNT", 4)?;
    let max_sessions_per_ip = env_u32("TIAMUD_MAX_SESSIONS_PER_IP", 16)?;
    let (shutdown_sender, shutdown_receiver) = watch::channel(false);
    let auth_rate_limiter = Arc::new(AuthRateLimiter::per_minute(
        auth_attempts_per_minute,
        auth_burst,
    ));
    let active_session_limiter = Arc::new(ActiveSessionLimiter::new(
        max_sessions_per_account,
        max_sessions_per_ip,
    ));
    let state = AppState {
        world: world.clone(),
        store,
        metrics: metrics.clone(),
        auth_rate_limiter: auth_rate_limiter.clone(),
        active_session_limiter: active_session_limiter.clone(),
        command_rate_per_second,
        command_burst,
        shutdown: shutdown_receiver.clone(),
        action_replay: web::action_replay_cache(),
    };

    let app = Router::new()
        .route("/health/live", get(web::live))
        .route("/health/ready", get(web::ready))
        .route("/metrics", get(web::metrics))
        .route("/api/status", get(web::status))
        .route("/api/auth/register", post(web::register))
        .route("/api/auth/login", post(web::login))
        .route("/api/auth/logout", post(web::logout))
        .route("/api/auth/session", get(web::auth_session))
        .route(
            "/api/characters",
            get(web::characters).post(web::create_character),
        )
        .route("/api/character-options", get(web::character_options))
        .route("/ws", get(web::ws_handler))
        .fallback(web::static_asset)
        .with_state(state)
        .layer(DefaultBodyLimit::max(16 * 1024))
        .layer(TraceLayer::new_for_http());

    let telnet_listener = TcpListener::bind(telnet_address).await?;
    let world_shutdown = world.clone();
    info!(
        %telnet_address,
        transport = telnet_transport.mode_name(),
        max_pending_handshakes = telnet_transport.max_pending_handshakes(),
        "telnet listener ready"
    );
    let telnet_runtime = telnet::TelnetRuntime::new(
        world,
        command_rate_per_second,
        command_burst,
        auth_rate_limiter,
        active_session_limiter,
        metrics,
    );
    let telnet_task = tokio::spawn(async move {
        if let Err(error) = telnet::run(
            telnet_listener,
            telnet_runtime,
            telnet_transport,
            shutdown_receiver,
        )
        .await
        {
            error!(%error, "telnet listener stopped");
        }
    });

    let http_listener = TcpListener::bind(http_address).await?;
    info!(%http_address, database_backend = database.backend_name(), world_pack = %world_pack_source, "browser server ready");
    let shutdown_for_http = shutdown_sender.clone();
    axum::serve(
        http_listener,
        app.into_make_service_with_connect_info::<SocketAddr>(),
    )
    .with_graceful_shutdown(async move {
        shutdown_signal().await;
        let _ = shutdown_for_http.send(true);
    })
    .await?;
    if timeout(Duration::from_secs(5), telnet_task).await.is_err() {
        error!("telnet listener shutdown exceeded its five-second timeout");
    }
    match timeout(Duration::from_secs(10), world_shutdown.shutdown()).await {
        Ok(Ok(())) => info!("world actor drained and stopped"),
        Ok(Err(error)) => error!(%error, "world actor stopped before shutdown acknowledgement"),
        Err(_) => error!("world actor shutdown exceeded its ten-second timeout"),
    }
    match timeout(Duration::from_secs(5), store_shutdown.shutdown()).await {
        Ok(Ok(())) => info!("persistence worker drained and closed"),
        Ok(Err(error)) => {
            error!(%error, "persistence worker stopped before shutdown acknowledgement")
        }
        Err(_) => error!("persistence worker shutdown exceeded its five-second timeout"),
    }
    Ok(())
}

fn env_value(name: &str, default: &str) -> String {
    env::var(name).unwrap_or_else(|_| default.to_owned())
}

#[derive(Error, Debug, Eq, PartialEq)]
enum DatabaseConfigError {
    #[error("TIAMUD_DATABASE_URL and TIAMUD_DB_PATH are mutually exclusive")]
    ConflictingVariables,
    #[error("TIAMUD_DATABASE_URL must not be empty or contain surrounding whitespace")]
    InvalidUrl,
    #[error(
        "unsupported TIAMUD_DATABASE_URL scheme; expected postgresql://, postgres:// or sqlite://"
    )]
    UnsupportedScheme,
    #[error("sqlite:// URL must contain a database path")]
    MissingSqlitePath,
    #[error("TIAMUD_DB_PATH must not be empty")]
    MissingSqlitePathVariable,
}

enum DatabaseConfig {
    Sqlite(String),
    Postgres(String),
}

impl std::fmt::Debug for DatabaseConfig {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Sqlite(path) => formatter.debug_tuple("Sqlite").field(path).finish(),
            Self::Postgres(_) => formatter.write_str("Postgres([REDACTED])"),
        }
    }
}

impl DatabaseConfig {
    fn backend_name(&self) -> &'static str {
        match self {
            Self::Sqlite(_) => "sqlite",
            Self::Postgres(_) => "postgresql",
        }
    }
}

fn optional_env(name: &str) -> Result<Option<String>, env::VarError> {
    match env::var(name) {
        Ok(value) => Ok(Some(value)),
        Err(env::VarError::NotPresent) => Ok(None),
        Err(error) => Err(error),
    }
}

#[derive(Debug, Eq, PartialEq)]
enum WorldSourceConfig {
    Monolithic(Option<String>),
    AreaCatalog { registry: String, catalog: String },
}

enum PreparedWorldSource {
    Monolithic {
        loaded: Box<LoadedWorldDefinition>,
        source: String,
    },
    AreaCatalog {
        generation: Box<LocalAreaGeneration>,
        pinned: Box<PinnedWorldContent>,
        source: String,
    },
}

#[derive(Debug, Error, Eq, PartialEq)]
enum WorldSourceConfigError {
    #[error(
        "TIAMUD_WORLD_PACK is mutually exclusive with TIAMUD_AREA_REGISTRY/TIAMUD_AREA_CATALOG"
    )]
    ConflictingModes,
    #[error("TIAMUD_AREA_REGISTRY and TIAMUD_AREA_CATALOG must be configured together")]
    IncompleteCatalogMode,
    #[error("world source paths must not be empty or contain surrounding whitespace")]
    InvalidPath,
}

fn world_source_config(
    world_pack: Option<String>,
    area_registry: Option<String>,
    area_catalog: Option<String>,
) -> Result<WorldSourceConfig, WorldSourceConfigError> {
    let valid = |value: &str| !value.is_empty() && value.trim() == value;
    if world_pack.as_deref().is_some_and(|value| !valid(value))
        || area_registry.as_deref().is_some_and(|value| !valid(value))
        || area_catalog.as_deref().is_some_and(|value| !valid(value))
    {
        return Err(WorldSourceConfigError::InvalidPath);
    }
    match (world_pack, area_registry, area_catalog) {
        (Some(_), Some(_), _) | (Some(_), _, Some(_)) => {
            Err(WorldSourceConfigError::ConflictingModes)
        }
        (Some(path), None, None) => Ok(WorldSourceConfig::Monolithic(Some(path))),
        (None, Some(registry), Some(catalog)) => {
            Ok(WorldSourceConfig::AreaCatalog { registry, catalog })
        }
        (None, None, None) => Ok(WorldSourceConfig::Monolithic(None)),
        (None, Some(_), None) | (None, None, Some(_)) => {
            Err(WorldSourceConfigError::IncompleteCatalogMode)
        }
    }
}

fn prepare_world_source(
    source: WorldSourceConfig,
) -> Result<PreparedWorldSource, Box<dyn Error + Send + Sync>> {
    match source {
        WorldSourceConfig::Monolithic(path) => {
            let (loaded, source) = match path {
                Some(path) => (load_loaded_world(&path)?, path),
                None => (default_loaded_world()?, "embedded default".to_owned()),
            };
            Ok(PreparedWorldSource::Monolithic {
                loaded: Box::new(loaded),
                source,
            })
        }
        WorldSourceConfig::AreaCatalog { registry, catalog } => {
            let generation = LocalAreaGeneration::open_exact(&registry, &catalog)?;
            let pinned = parse_pinned_world_content(generation.world_artifact())?;
            let source = format!("area catalog {}", generation.generation_id());
            Ok(PreparedWorldSource::AreaCatalog {
                generation: Box::new(generation),
                pinned: Box::new(pinned),
                source,
            })
        }
    }
}

fn prepare_world_before_database(
    database: &DatabaseConfig,
    source: WorldSourceConfig,
) -> Result<PreparedWorldSource, Box<dyn Error + Send + Sync>> {
    let prepared = prepare_world_source(source)?;
    if let DatabaseConfig::Sqlite(path) = database {
        create_database_parent(path)?;
    }
    Ok(prepared)
}

fn database_config(
    database_url: Option<String>,
    sqlite_path: Option<String>,
) -> Result<DatabaseConfig, DatabaseConfigError> {
    match (database_url, sqlite_path) {
        (Some(_), Some(_)) => Err(DatabaseConfigError::ConflictingVariables),
        (None, Some(path)) => {
            if path.is_empty() {
                Err(DatabaseConfigError::MissingSqlitePathVariable)
            } else {
                Ok(DatabaseConfig::Sqlite(path))
            }
        }
        (None, None) => Ok(DatabaseConfig::Sqlite("data/tiamud.db".to_owned())),
        (Some(url), None) => {
            if url.is_empty() || url.trim() != url {
                return Err(DatabaseConfigError::InvalidUrl);
            }
            if url.starts_with("postgresql://") || url.starts_with("postgres://") {
                return Ok(DatabaseConfig::Postgres(url));
            }
            if let Some(path) = url.strip_prefix("sqlite://") {
                if path.is_empty() {
                    return Err(DatabaseConfigError::MissingSqlitePath);
                }
                return Ok(DatabaseConfig::Sqlite(path.to_owned()));
            }
            Err(DatabaseConfigError::UnsupportedScheme)
        }
    }
}

fn env_u32(name: &str, default: u32) -> Result<u32, Box<dyn Error + Send + Sync>> {
    match env::var(name) {
        Ok(value) => value
            .parse::<u32>()
            .map(|value| value.max(1))
            .map_err(|error| format!("invalid {name}={value:?}: {error}").into()),
        Err(env::VarError::NotPresent) => Ok(default),
        Err(error) => Err(error.into()),
    }
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
        create_database_parent("tiamud.db").unwrap();
        create_database_parent(":memory:").unwrap();
    }

    #[test]
    fn numeric_limits_are_positive_and_configurable() {
        let missing = "TIAMUD_TEST_MISSING_NUMERIC_LIMIT";
        // SAFETY: this unique variable is not accessed by any other test.
        unsafe { env::remove_var(missing) };
        assert_eq!(env_u32(missing, 7).unwrap(), 7);
    }

    #[test]
    fn database_configuration_defaults_to_sqlite() {
        let config = database_config(None, None).unwrap();
        assert_eq!(config.backend_name(), "sqlite");
        assert!(matches!(config, DatabaseConfig::Sqlite(path) if path == "data/tiamud.db"));
    }

    #[test]
    fn database_configuration_selects_one_explicit_backend() {
        let postgres =
            database_config(Some("postgresql://db.internal/tiamud".to_owned()), None).unwrap();
        assert_eq!(postgres.backend_name(), "postgresql");
        assert!(matches!(postgres, DatabaseConfig::Postgres(_)));

        let sqlite = database_config(Some("sqlite://var/tiamud.db".to_owned()), None).unwrap();
        assert!(matches!(sqlite, DatabaseConfig::Sqlite(path) if path == "var/tiamud.db"));
    }

    #[test]
    fn database_configuration_fails_closed_on_ambiguity_or_unknown_schemes() {
        assert_eq!(
            database_config(
                Some("postgresql://db/tiamud".to_owned()),
                Some("state.sqlite3".to_owned()),
            )
            .unwrap_err(),
            DatabaseConfigError::ConflictingVariables
        );
        assert_eq!(
            database_config(Some("mysql://db/tiamud".to_owned()), None).unwrap_err(),
            DatabaseConfigError::UnsupportedScheme
        );
        assert_eq!(
            database_config(Some(" postgresql://secret@db/tiamud".to_owned()), None).unwrap_err(),
            DatabaseConfigError::InvalidUrl
        );
    }

    #[test]
    fn world_source_selects_exactly_one_runtime_mode() {
        assert_eq!(
            world_source_config(None, None, None).unwrap(),
            WorldSourceConfig::Monolithic(None)
        );
        assert_eq!(
            world_source_config(Some("world.json".into()), None, None).unwrap(),
            WorldSourceConfig::Monolithic(Some("world.json".into()))
        );
        assert_eq!(
            world_source_config(
                None,
                Some("registry".into()),
                Some("registry/catalog.json".into()),
            )
            .unwrap(),
            WorldSourceConfig::AreaCatalog {
                registry: "registry".into(),
                catalog: "registry/catalog.json".into(),
            }
        );
    }

    #[test]
    fn world_source_rejects_mixed_incomplete_or_ambiguous_configuration() {
        assert_eq!(
            world_source_config(
                Some("world.json".into()),
                Some("registry".into()),
                Some("catalog.json".into()),
            ),
            Err(WorldSourceConfigError::ConflictingModes)
        );
        assert_eq!(
            world_source_config(None, Some("registry".into()), None),
            Err(WorldSourceConfigError::IncompleteCatalogMode)
        );
        assert_eq!(
            world_source_config(None, None, Some("catalog.json".into())),
            Err(WorldSourceConfigError::IncompleteCatalogMode)
        );
        assert_eq!(
            world_source_config(Some(" world.json".into()), None, None),
            Err(WorldSourceConfigError::InvalidPath)
        );
    }

    #[test]
    fn invalid_catalog_fails_before_creating_the_sqlite_parent() {
        let temporary = tempfile::tempdir().unwrap();
        let database_path = temporary.path().join("state/tiamud.db");
        let missing_registry = temporary.path().join("missing-registry");
        let missing_catalog = missing_registry.join("area-catalogs/missing.json");
        let database = DatabaseConfig::Sqlite(database_path.to_string_lossy().into_owned());
        let source = WorldSourceConfig::AreaCatalog {
            registry: missing_registry.to_string_lossy().into_owned(),
            catalog: missing_catalog.to_string_lossy().into_owned(),
        };

        assert!(prepare_world_before_database(&database, source).is_err());
        assert!(!database_path.parent().unwrap().exists());
    }
}
