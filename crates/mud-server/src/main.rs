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
use operations::{
    ActiveSessionLimiter, AreaGenerationMetricPhase, AreaGenerationMetricResult, AuthRateLimiter,
    RuntimeMetrics,
};
use persistence::{
    CharacterStore, DurableAreaGeneration, MeasuredCharacterStore, SqliteCharacterStore, StoreError,
};
use persistence_postgres::PostgresCharacterStore;
use std::{env, error::Error, net::SocketAddr, path::Path, sync::Arc, time::Instant};
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
use world_actor::{AreaGenerationActivationError, WorldHandle, WorldSchedulerConfig};
use world_loader::{
    LoadedWorldDefinition, build_loaded_world, default_loaded_world, load_loaded_world,
    parse_pinned_world_content,
};
use world_pack::{
    GenerationReplacementKind, LocalAreaGeneration, LocalAreaRegistryError,
    StagedLocalAreaGenerationReplacement, local_area_catalog_path, stage_generation_replacement,
    stage_generation_rollback, validate_generation_replacement,
};

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
    let legacy_area_generation_ack = optional_env("TIAMUD_ACK_LEGACY_AREA_GENERATION")?;
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
    let durable_generation_before_recovery = store.load_area_generation().await?;
    let recovery = store.load_startup_world_recovery().await?;
    let has_durable_character_authority = store.has_durable_character_authority().await?;
    let durable_area_generation = store.load_area_generation().await?;
    if durable_generation_before_recovery != durable_area_generation {
        return Err(
            "the durable area generation changed while startup recovery was read; refusing a mixed authority"
                .into(),
        );
    }
    let tick_millis = env_u32("TIAMUD_TICK_MILLIS", 1_000)?;
    let scheduler = WorldSchedulerConfig::every(Duration::from_millis(u64::from(tick_millis)));
    let (world, world_pack_source, catalog_runtime) = match prepared_world {
        PreparedWorldSource::Monolithic { loaded, source } => {
            if legacy_area_generation_ack.is_some() {
                return Err(
                    "TIAMUD_ACK_LEGACY_AREA_GENERATION is valid only for catalog mode".into(),
                );
            }
            if durable_area_generation.is_some() {
                return Err(
                    "the database is bound to an area catalog generation but monolithic world mode was selected"
                        .into(),
                );
            }
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
            (handle, source, None)
        }
        PreparedWorldSource::AreaCatalog {
            mut generation,
            registry,
            ..
        } => {
            if let Some(durable) = durable_area_generation.as_ref() {
                if durable.world_id != generation.catalog().world_id {
                    return Err(format!(
                        "the database is bound to world {:?}, not the configured catalog world {:?}",
                        durable.world_id,
                        generation.catalog().world_id
                    )
                    .into());
                }
                if durable.active_catalog_sha256 != generation.generation_id() {
                    let catalog_path = local_area_catalog_path(
                        Path::new(&registry),
                        &durable.world_id,
                        &durable.active_catalog_sha256,
                    );
                    *generation = LocalAreaGeneration::open_exact(&registry, catalog_path)?;
                }
            }
            // A durable pointer must never name a partially available release.
            // This revalidation leaves runtime residency lazy for an empty
            // world, but proves every immutable blob before readiness.
            generation.verify_complete_release()?;
            let world_id = generation.catalog().world_id.clone();
            let generation_id = generation.generation_id().to_owned();
            if !valid_legacy_area_generation_ack(
                durable_area_generation.is_some(),
                recovery.requires_full_generation(),
                has_durable_character_authority,
                legacy_area_generation_ack.as_deref(),
                &generation_id,
            ) {
                return Err(
                    "TIAMUD_ACK_LEGACY_AREA_GENERATION is accepted only for a non-empty unbound 6B database and must equal its exact configured catalog SHA-256; remove it after binding, and do not use it for monolithic conversion"
                        .into(),
                );
            }
            let pinned = parse_pinned_world_content(generation.world_artifact())?;
            let source = format!("area catalog {generation_id}");
            let coordinator = AreaLoadCoordinator::new(*generation, pinned, Some(metrics.clone()));
            let bootstrap = coordinator
                .bootstrap_world(recovery.requires_full_generation())
                .await?;
            let confirm_initial_dynamic_state =
                !recovery.requires_full_generation() && bootstrap.created_dynamic_state;
            if durable_area_generation.is_none() {
                let proposed = DurableAreaGeneration {
                    world_id: world_id.clone(),
                    active_catalog_sha256: generation_id.clone(),
                    previous_catalog_sha256: None,
                    activation_sequence: 1,
                };
                let actual = store
                    .bind_area_generation_if_absent(proposed.clone())
                    .await?;
                if actual != proposed {
                    return Err(
                        "another process bound a different area generation during startup; refusing to start the actor"
                            .into(),
                    );
                }
            }
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
            (handle, source, Some(CatalogRuntime { registry, world_id }))
        }
    };
    let auth_attempts_per_minute = env_u32("TIAMUD_AUTH_ATTEMPTS_PER_MINUTE", 10)?;
    let auth_burst = env_u32("TIAMUD_AUTH_BURST", 5)?;
    let command_rate_per_second = env_u32("TIAMUD_COMMANDS_PER_SECOND", 12)?;
    let command_burst = env_u32("TIAMUD_COMMAND_BURST", 20)?;
    let max_sessions_per_account = env_u32("TIAMUD_MAX_SESSIONS_PER_ACCOUNT", 4)?;
    let max_sessions_per_ip = env_u32("TIAMUD_MAX_SESSIONS_PER_IP", 16)?;
    let (shutdown_sender, shutdown_receiver) = watch::channel(false);
    let area_reconciler_task = catalog_runtime.map(|catalog| {
        tokio::spawn(run_area_generation_reconciler(
            catalog,
            world.clone(),
            store.clone(),
            metrics.clone(),
            shutdown_receiver.clone(),
        ))
    });
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
    if let Some(mut area_reconciler_task) = area_reconciler_task {
        match timeout(Duration::from_secs(10), &mut area_reconciler_task).await {
            Ok(Ok(())) => info!("area generation reconciler stopped"),
            Ok(Err(error)) => error!(%error, "area generation reconciler task failed"),
            Err(_) => {
                error!("area generation reconciler shutdown exceeded ten seconds");
                area_reconciler_task.abort();
                let _ = area_reconciler_task.await;
            }
        }
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

#[derive(Clone, Debug, Eq, PartialEq)]
struct CatalogRuntime {
    registry: String,
    world_id: String,
}

enum PreparedWorldSource {
    Monolithic {
        loaded: Box<LoadedWorldDefinition>,
        source: String,
    },
    AreaCatalog {
        generation: Box<LocalAreaGeneration>,
        registry: String,
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

fn valid_legacy_area_generation_ack(
    durable_bound: bool,
    recovery_requires_full_generation: bool,
    has_durable_character_authority: bool,
    acknowledged_generation: Option<&str>,
    configured_generation: &str,
) -> bool {
    match (
        durable_bound,
        recovery_requires_full_generation || has_durable_character_authority,
    ) {
        (false, true) => acknowledged_generation == Some(configured_generation),
        _ => acknowledged_generation.is_none(),
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
            Ok(PreparedWorldSource::AreaCatalog {
                generation: Box::new(generation),
                registry,
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

#[derive(Debug, Error)]
enum AreaGenerationReconcileError {
    #[error("the world is not quiescent")]
    Busy,
    #[error("the published area generation could not be verified: {0}")]
    Verification(#[source] LocalAreaRegistryError),
    #[error("the published area generation is incompatible: {0}")]
    Incompatible(#[source] LocalAreaRegistryError),
    #[error("area generation persistence failed: {0}")]
    Persistence(#[source] StoreError),
    #[error("the durable and in-memory area generations conflict")]
    Conflict,
    #[error("area generation activation failed: {0}")]
    Activation(#[source] AreaGenerationActivationError),
    #[error("area generation reconciliation failed internally: {0}")]
    Internal(String),
}

impl AreaGenerationReconcileError {
    fn metric_result(&self) -> AreaGenerationMetricResult {
        match self {
            Self::Busy | Self::Activation(AreaGenerationActivationError::Busy) => {
                AreaGenerationMetricResult::Busy
            }
            Self::Verification(_)
            | Self::Activation(AreaGenerationActivationError::InvalidCandidate) => {
                AreaGenerationMetricResult::VerificationFailed
            }
            Self::Incompatible(_) | Self::Activation(AreaGenerationActivationError::Runtime(_)) => {
                AreaGenerationMetricResult::Incompatible
            }
            Self::Persistence(StoreError::AreaGenerationConflict)
            | Self::Conflict
            | Self::Activation(AreaGenerationActivationError::WrongGeneration)
            | Self::Activation(AreaGenerationActivationError::Persistence(
                StoreError::AreaGenerationConflict,
            )) => AreaGenerationMetricResult::Conflict,
            Self::Persistence(_)
            | Self::Activation(AreaGenerationActivationError::Persistence(_)) => {
                AreaGenerationMetricResult::PersistenceFailed
            }
            Self::Activation(AreaGenerationActivationError::ActorStopped) | Self::Internal(_) => {
                AreaGenerationMetricResult::InternalFailed
            }
            Self::Activation(AreaGenerationActivationError::DurableAuthorityAmbiguous) => {
                AreaGenerationMetricResult::InternalFailed
            }
        }
    }
}

async fn run_area_generation_reconciler(
    catalog: CatalogRuntime,
    world: WorldHandle,
    store: Arc<dyn CharacterStore>,
    metrics: Arc<RuntimeMetrics>,
    mut shutdown: watch::Receiver<bool>,
) {
    #[cfg(unix)]
    {
        let mut hangup = match signal::unix::signal(signal::unix::SignalKind::hangup()) {
            Ok(signal) => signal,
            Err(error) => {
                error!(%error, "could not install SIGHUP area-generation reconciler");
                return;
            }
        };
        loop {
            tokio::select! {
                changed = shutdown.changed() => {
                    if changed.is_err() || *shutdown.borrow() {
                        return;
                    }
                }
                received = hangup.recv() => {
                    if received.is_none() {
                        return;
                    }
                    let started = Instant::now();
                    match reconcile_area_generation(&catalog, &world, store.as_ref(), &metrics).await {
                        Ok(kind) => {
                            let result = match kind {
                                GenerationReplacementKind::Activate => {
                                    info!("area catalog generation activated after SIGHUP");
                                    AreaGenerationMetricResult::Activated
                                }
                                GenerationReplacementKind::Rollback => {
                                    info!("area catalog generation rolled back after SIGHUP");
                                    AreaGenerationMetricResult::RolledBack
                                }
                                GenerationReplacementKind::Noop => {
                                    info!("SIGHUP area catalog generation is already active");
                                    AreaGenerationMetricResult::Noop
                                }
                            };
                            metrics.area_generation_finished(started.elapsed(), result);
                        }
                        Err(error) => {
                            let result = error.metric_result();
                            let authority_is_ambiguous = matches!(
                                &error,
                                AreaGenerationReconcileError::Activation(
                                    AreaGenerationActivationError::DurableAuthorityAmbiguous
                                )
                            );
                            if authority_is_ambiguous {
                                error!(%error, "area catalog generation reconciliation failed closed");
                            } else {
                                warn!(%error, "area catalog generation reconciliation was rejected");
                            }
                            metrics.area_generation_finished(started.elapsed(), result);
                            if authority_is_ambiguous {
                                metrics.set_area_generation_phase(
                                    AreaGenerationMetricPhase::Committing,
                                );
                                return;
                            }
                        }
                    }
                }
            }
        }
    }

    #[cfg(not(unix))]
    {
        let _ = (catalog, world, store, metrics);
        let _ = shutdown.changed().await;
    }
}

async fn reconcile_area_generation(
    catalog: &CatalogRuntime,
    world: &WorldHandle,
    store: &dyn CharacterStore,
    metrics: &Arc<RuntimeMetrics>,
) -> Result<GenerationReplacementKind, AreaGenerationReconcileError> {
    let quiescence = world
        .inspect_area_quiescence()
        .await
        .map_err(AreaGenerationReconcileError::Activation)?;
    if quiescence.connected_sessions != 0 {
        return Err(AreaGenerationReconcileError::Busy);
    }

    let durable = store
        .load_area_generation()
        .await
        .map_err(AreaGenerationReconcileError::Persistence)?
        .ok_or_else(|| {
            AreaGenerationReconcileError::Internal(
                "catalog runtime has no durable generation binding".into(),
            )
        })?;
    if durable.world_id != catalog.world_id {
        return Err(AreaGenerationReconcileError::Conflict);
    }
    let status = world
        .area_generation_status()
        .ok_or(AreaGenerationReconcileError::Conflict)?;
    if status.active_generation.as_str() != durable.active_catalog_sha256 {
        return Err(AreaGenerationReconcileError::Conflict);
    }
    tracing::debug!(
        phase = ?status.phase,
        active_leases = status.active_leases,
        has_target = status.target_generation.is_some(),
        "starting local area generation reconciliation"
    );
    metrics.set_area_generation_phase(AreaGenerationMetricPhase::Staging);

    let catalog_for_worker = catalog.clone();
    let durable_for_worker = durable.clone();
    let staged = tokio::task::spawn_blocking(move || {
        stage_active_area_generation(&catalog_for_worker, &durable_for_worker)
    })
    .await
    .map_err(|error| AreaGenerationReconcileError::Internal(error.to_string()))??;
    let kind = staged.kind();
    if kind == GenerationReplacementKind::Noop {
        return Ok(kind);
    }

    let pinned = parse_pinned_world_content(staged.candidate().world_artifact())
        .map_err(|error| AreaGenerationReconcileError::Internal(error.to_string()))?;
    let candidate = AreaLoadCoordinator::new(
        staged.candidate().clone(),
        pinned,
        Some(Arc::clone(metrics)),
    );
    candidate.set_metrics_suppressed(true);
    let prepared = candidate.stage_full_generation_replacement(&staged).await;
    candidate.set_metrics_suppressed(false);
    let prepared =
        prepared.map_err(|error| AreaGenerationReconcileError::Internal(error.to_string()))?;
    let next_sequence = durable.activation_sequence.checked_add(1).ok_or_else(|| {
        AreaGenerationReconcileError::Internal("activation sequence overflow".into())
    })?;
    let next = DurableAreaGeneration {
        world_id: durable.world_id.clone(),
        active_catalog_sha256: staged.candidate_generation_id().to_owned(),
        previous_catalog_sha256: Some(durable.active_catalog_sha256.clone()),
        activation_sequence: next_sequence,
    };
    metrics.set_area_generation_phase(AreaGenerationMetricPhase::Draining);
    world
        .activate_prepared_generation(prepared, durable, next)
        .await
        .map_err(AreaGenerationReconcileError::Activation)?;
    Ok(kind)
}

fn stage_active_area_generation(
    catalog: &CatalogRuntime,
    durable: &DurableAreaGeneration,
) -> Result<StagedLocalAreaGenerationReplacement, AreaGenerationReconcileError> {
    let current_path = local_area_catalog_path(
        Path::new(&catalog.registry),
        &catalog.world_id,
        &durable.active_catalog_sha256,
    );
    let current = LocalAreaGeneration::open_exact(&catalog.registry, current_path)
        .map_err(AreaGenerationReconcileError::Verification)?;
    let candidate = LocalAreaGeneration::open_active(&catalog.registry, &catalog.world_id)
        .map_err(AreaGenerationReconcileError::Verification)?;
    let kind = validate_generation_replacement(&current, &candidate)
        .map_err(AreaGenerationReconcileError::Incompatible)?;
    match kind {
        GenerationReplacementKind::Activate | GenerationReplacementKind::Noop => {
            stage_generation_replacement(&current, candidate)
                .map_err(AreaGenerationReconcileError::Verification)
        }
        GenerationReplacementKind::Rollback => stage_generation_rollback(&current, candidate)
            .map_err(AreaGenerationReconcileError::Verification),
    }
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
    fn legacy_area_generation_ack_is_exact_and_single_use() {
        let generation = "a".repeat(64);
        assert!(valid_legacy_area_generation_ack(
            false,
            true,
            false,
            Some(&generation),
            &generation,
        ));
        assert!(!valid_legacy_area_generation_ack(
            false,
            true,
            false,
            None,
            &generation,
        ));
        assert!(!valid_legacy_area_generation_ack(
            false,
            true,
            false,
            Some(&"b".repeat(64)),
            &generation,
        ));
        assert!(valid_legacy_area_generation_ack(
            false,
            false,
            false,
            None,
            &generation,
        ));
        assert!(!valid_legacy_area_generation_ack(
            false,
            false,
            false,
            Some(&generation),
            &generation,
        ));
        assert!(!valid_legacy_area_generation_ack(
            true,
            true,
            true,
            Some(&generation),
            &generation,
        ));
        assert!(!valid_legacy_area_generation_ack(
            false,
            false,
            true,
            None,
            &generation,
        ));
        assert!(valid_legacy_area_generation_ack(
            false,
            false,
            true,
            Some(&generation),
            &generation,
        ));
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
