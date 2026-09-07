use crate::{
    area_runtime::{
        AreaGenerationCutoverGuard, AreaGenerationRouteError, AreaGenerationRouter,
        AreaLoadCoordinator, AreaPreparationError, PreparedAreaBatch, PreparedAreaRetirementBatch,
        PreparedGenerationCutover,
    },
    auth::AuthenticatedSession,
    boards::{BoardDescriptor, apply_board},
    delivery::{DeliveryInbox, DeliveryRouter},
    operations::{AreaGenerationMetricPhase, AreaLoadMetricResult},
    persistence::{
        BoardEntry, BoardPage, CharacterStore, DurableAreaCheckpoint, DurableAreaGeneration,
        DurableCharacterGameplay, MailMessage, MailPage, StaffAccess, StartupGameplayLoadLimits,
        StartupWorldRecovery, StoreError, validate_area_checkpoint,
    },
    runtime_content::{RuntimeContent, ScenePresentation},
    social::{
        MAX_CHAT_MESSAGE_BYTES, MAX_SOCIAL_TARGET_BYTES, SocialChannel, SocialMessage,
        SocialRuntime, apply_mail, apply_social, ensure_chat_allowed, normalise_text,
        resolve_connected,
    },
    staff::{apply_staff, derived_role},
};
use sha2::{Digest, Sha256};
use std::{
    collections::{BTreeMap, BTreeSet, HashMap, HashSet},
    sync::Arc,
    time::{Duration, Instant},
};
use thiserror::Error;
use tiamud_core::{
    AccountId, AccountRole, AccountSnapshot, AdventureEvent,
    AreaRetirementError as CoreAreaRetirementError, BackgroundSelection,
    CharacterCreationCatalogView, CharacterGameplaySnapshot, CharacterId, CharacterSnapshot,
    ContainerContentsView, DomainEvent, GameplayEvent, PlayerAction, PlayerStateView,
    RewardOddsView, SceneView, SessionId, TradeAction, TradeCancellationReason, TradeContentsView,
    ViewEvent, World, WorldCheckpoint, WorldError as CoreWorldError, WorldRuntimeSnapshot,
};
use tiamud_scripting::ScriptHook;
use tokio::sync::{Mutex as AsyncMutex, mpsc, oneshot};
use tokio::time::MissedTickBehavior;
use tracing::{error, warn};
#[cfg(unix)]
use world_pack::{LocalAreaPublicationReadGuard, LocalAreaRegistryError};

const WORLD_COMMAND_QUEUE_CAPACITY: usize = 256;
const PERSIST_TICK_SCRIPT_WALL_BUDGET: Duration = Duration::from_millis(100);
const SOCIAL_MAINTENANCE_INTERVAL: Duration = Duration::from_secs(60 * 60);

#[derive(Debug, Error)]
pub enum GameError {
    #[error(transparent)]
    World(#[from] CoreWorldError),
    #[error(transparent)]
    Persistence(#[from] StoreError),
    #[error("the world actor stopped unexpectedly")]
    ActorStopped,
    #[error("the character references content that requires an operator migration")]
    CharacterContentIncompatible,
    #[error("that item transfer is not available")]
    GiveUnavailable,
    #[error("the destination is not available right now")]
    AreaUnavailable,
    #[error("that trade is not available")]
    TradeUnavailable,
    #[error("that mail recipient is not available")]
    MailRecipientUnavailable,
    #[error("the action's script hook was rejected")]
    ScriptHookRejected,
    #[error("that area cannot be retired or rehydrated right now")]
    AreaOperationBusy,
    #[error("that area retirement or rehydration request is invalid")]
    AreaOperationInvalid,
    #[error("world actor startup failed during {stage}: {reason}")]
    Startup { stage: &'static str, reason: String },
}

#[derive(Debug, Error)]
pub(crate) enum AreaGenerationActivationError {
    #[error("another area generation transition or admission is in progress")]
    Busy,
    #[error("the area generation activation precondition is stale")]
    WrongGeneration,
    #[error("the prepared area generation is invalid")]
    InvalidCandidate,
    #[error("the candidate cannot restore the current runtime authority: {0}")]
    Runtime(#[source] CoreWorldError),
    #[error("the durable area generation commit failed: {0}")]
    Persistence(#[source] StoreError),
    #[error("the durable area generation authority is ambiguous; the world actor stopped")]
    DurableAuthorityAmbiguous,
    #[error("the world actor stopped unexpectedly")]
    ActorStopped,
}

impl From<AreaGenerationRouteError> for AreaGenerationActivationError {
    fn from(error: AreaGenerationRouteError) -> Self {
        match error {
            AreaGenerationRouteError::Busy => Self::Busy,
            AreaGenerationRouteError::WrongGeneration
            | AreaGenerationRouteError::SameGeneration => Self::WrongGeneration,
        }
    }
}

#[derive(Debug, Error)]
pub(crate) enum AreaRetirementOperationError {
    #[error("the area retirement or rehydration precondition is busy")]
    Busy,
    #[error("the prepared area retirement or checkpoint is invalid or stale")]
    Invalid,
    #[error("the durable area transition failed: {0}")]
    Persistence(#[source] StoreError),
    #[error("durable area authority is ambiguous; the world actor stopped")]
    DurableAuthorityAmbiguous,
    #[allow(dead_code)]
    #[error("the world actor stopped unexpectedly")]
    ActorStopped,
}

impl From<AreaPreparationError> for AreaRetirementOperationError {
    fn from(error: AreaPreparationError) -> Self {
        match error {
            AreaPreparationError::QueueFull
            | AreaPreparationError::Timeout
            | AreaPreparationError::Cancelled => Self::Busy,
            AreaPreparationError::UnknownArea
            | AreaPreparationError::Retired
            | AreaPreparationError::Invalid(_) => Self::Invalid,
        }
    }
}

#[derive(Debug)]
pub struct EnteredWorld {
    pub session_id: SessionId,
    pub account_id: AccountId,
    pub scene: SceneView,
    pub(crate) presentation: ScenePresentation,
    pub player: PlayerStateView,
    pub deliveries: DeliveryInbox,
}

struct AreaOperatorAuthority {
    account_id: AccountId,
    actor_name: String,
    scene: SceneView,
    presentation: ScenePresentation,
    player: PlayerStateView,
}

#[derive(Debug)]
struct PreparedWorldEntry {
    session_id: SessionId,
    account_id: AccountId,
    scene: SceneView,
    player: PlayerStateView,
}

#[derive(Debug)]
pub struct AppliedAction {
    pub scene: SceneView,
    pub(crate) presentation: ScenePresentation,
    pub player: PlayerStateView,
    pub output: Option<String>,
    pub container_contents: Option<ContainerContentsView>,
    pub trade_contents: Option<TradeContentsView>,
    pub reward_odds: Option<RewardOddsView>,
    pub mail_page: Option<MailPage>,
    pub mail_message: Option<MailMessage>,
    pub board_catalog: Option<Vec<BoardDescriptor>>,
    pub board_page: Option<BoardPage>,
    pub board_entry: Option<BoardEntry>,
    pub events: Vec<ProjectedViewEvent>,
}

/// Server-only transport metadata paired with the exact perception-safe event
/// from which it was derived. The core event schema remains the authoritative
/// narrative contract and is serialized unchanged by non-web transports.
#[derive(Debug)]
pub struct ProjectedViewEvent {
    pub event: ViewEvent,
    pub(crate) combat_cue: Option<AuthoritativeCombatCue>,
}

impl ProjectedViewEvent {
    fn narrative(event: ViewEvent) -> Self {
        Self {
            event,
            combat_cue: None,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum CombatCueKind {
    Attack,
    Hit,
    Defeat,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct AuthoritativeCombatCue {
    pub(crate) kind: CombatCueKind,
    pub(crate) entity_id: String,
}

/// A fresh perception-safe projection caused by another actor or by a world
/// tick. The router addresses each projection directly to one session; no
/// stable room, entity, account or character IDs cross the transport edge.
#[derive(Debug)]
pub struct WorldDelivery {
    pub scene: SceneView,
    pub(crate) presentation: ScenePresentation,
    pub player: PlayerStateView,
    pub events: Vec<ProjectedViewEvent>,
}

/// Owned transport representation of the core's borrowed tri-state background
/// selection. `Default` and `None` are intentionally different choices.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub enum CharacterBackgroundSelection {
    #[default]
    Default,
    None,
    Id(String),
}

impl CharacterBackgroundSelection {
    fn as_core(&self) -> BackgroundSelection<'_> {
        match self {
            Self::Default => BackgroundSelection::Default,
            Self::None => BackgroundSelection::None,
            Self::Id(id) => BackgroundSelection::Id(id),
        }
    }
}

enum Command {
    Ping {
        reply: oneshot::Sender<()>,
    },
    CurrentState {
        session: SessionId,
        reply: oneshot::Sender<Result<(SceneView, ScenePresentation, PlayerStateView), GameError>>,
    },
    InitialGameplay {
        lineage_id: Option<String>,
        background: CharacterBackgroundSelection,
        base_class_id: Option<String>,
        reply: oneshot::Sender<Result<CharacterGameplaySnapshot, GameError>>,
    },
    EnterPreparedCharacter {
        snapshot: CharacterSnapshot,
        gameplay: Box<CharacterGameplaySnapshot>,
        staff_access: StaffAccess,
        reply: oneshot::Sender<Result<EnteredWorld, GameError>>,
    },
    Apply {
        session: SessionId,
        action: PlayerAction,
        reply: oneshot::Sender<Result<AppliedAction, GameError>>,
    },
    InstallPreparedAreas {
        batch: Arc<PreparedAreaBatch>,
        reply: oneshot::Sender<Result<Vec<String>, GameError>>,
    },
    #[allow(dead_code)]
    RetirePreparedAreas {
        prepared: Arc<PreparedAreaRetirementBatch>,
        expected_generation: DurableAreaGeneration,
        reply: oneshot::Sender<Result<Vec<String>, AreaRetirementOperationError>>,
    },
    RehydratePreparedAreas {
        prepared: Arc<PreparedAreaBatch>,
        checkpoint: DurableAreaCheckpoint,
        expected_generation: DurableAreaGeneration,
        reply: oneshot::Sender<Result<Vec<String>, AreaRetirementOperationError>>,
    },
    InspectAreaQuiescence {
        reply: oneshot::Sender<AreaGenerationQuiescence>,
    },
    ResolveAreaOperator {
        session: SessionId,
        capability: tiamud_core::StaffCapability,
        reply: oneshot::Sender<Result<AreaOperatorAuthority, GameError>>,
    },
    ActivatePreparedGeneration {
        prepared: Box<PreparedGenerationCutover>,
        transition: AreaGenerationCutoverGuard,
        #[cfg(unix)]
        publication_guard: Option<LocalAreaPublicationReadGuard>,
        expected_current: DurableAreaGeneration,
        next: DurableAreaGeneration,
        reply: oneshot::Sender<Result<(), AreaGenerationActivationError>>,
    },
    Leave {
        session: SessionId,
    },
    Shutdown {
        reply: oneshot::Sender<()>,
    },
}

struct GenerationActivationCommit {
    prepared: PreparedGenerationCutover,
    transition: AreaGenerationCutoverGuard,
    #[cfg(unix)]
    publication_guard: Option<LocalAreaPublicationReadGuard>,
    expected_current: DurableAreaGeneration,
    next: DurableAreaGeneration,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct AreaGenerationQuiescence {
    pub(crate) connected_sessions: usize,
}

#[derive(Clone)]
pub struct WorldHandle {
    tx: mpsc::Sender<Command>,
    store: Arc<dyn CharacterStore>,
    start_room_id: Arc<str>,
    creation_catalog: Arc<CharacterCreationCatalogView>,
    area_router: Option<AreaGenerationRouter>,
    area_rehydration_lock: Arc<AsyncMutex<()>>,
}

#[derive(Clone, Copy, Debug, Default)]
pub struct WorldSchedulerConfig {
    pub tick_interval: Option<Duration>,
}

impl WorldSchedulerConfig {
    pub fn every(tick_interval: Duration) -> Self {
        Self {
            tick_interval: Some(tick_interval),
        }
    }
}

impl WorldHandle {
    #[cfg(test)]
    pub async fn spawn(world: World, store: Arc<dyn CharacterStore>) -> Result<Self, GameError> {
        Self::spawn_with_scheduler(world, store, WorldSchedulerConfig::default()).await
    }

    #[cfg(test)]
    pub async fn spawn_with_scheduler(
        world: World,
        store: Arc<dyn CharacterStore>,
        scheduler: WorldSchedulerConfig,
    ) -> Result<Self, GameError> {
        Self::spawn_with_scheduler_and_content(world, store, scheduler, RuntimeContent::default())
            .await
    }

    #[allow(dead_code)]
    pub async fn spawn_with_scheduler_and_content(
        world: World,
        store: Arc<dyn CharacterStore>,
        scheduler: WorldSchedulerConfig,
        runtime_content: RuntimeContent,
    ) -> Result<Self, GameError> {
        Self::spawn_with_scheduler_content_and_areas(world, store, scheduler, runtime_content, None)
            .await
    }

    #[allow(dead_code)]
    pub(crate) async fn spawn_with_scheduler_content_and_areas(
        world: World,
        store: Arc<dyn CharacterStore>,
        scheduler: WorldSchedulerConfig,
        runtime_content: RuntimeContent,
        area_coordinator: Option<AreaLoadCoordinator>,
    ) -> Result<Self, GameError> {
        let recovery =
            store
                .load_startup_world_recovery()
                .await
                .map_err(|error| GameError::Startup {
                    stage: "load_startup_world_recovery",
                    reason: error.to_string(),
                })?;
        Self::spawn_with_scheduler_content_areas_and_recovery(
            world,
            store,
            scheduler,
            runtime_content,
            area_coordinator,
            recovery,
            false,
        )
        .await
    }

    pub(crate) async fn spawn_with_scheduler_content_areas_and_recovery(
        world: World,
        store: Arc<dyn CharacterStore>,
        scheduler: WorldSchedulerConfig,
        runtime_content: RuntimeContent,
        area_coordinator: Option<AreaLoadCoordinator>,
        recovery: StartupWorldRecovery,
        confirm_initial_dynamic_state: bool,
    ) -> Result<Self, GameError> {
        let (tx, mut rx) = mpsc::channel(WORLD_COMMAND_QUEUE_CAPACITY);
        let (startup_reply, startup_receive) = oneshot::channel();
        let mut social = SocialRuntime::new();
        let operational_metrics = runtime_content.operational_metrics();
        let mut actor_delivery = DeliveryRouter::new(operational_metrics.clone());
        let start_room_id: Arc<str> = Arc::from(world.start_room_id());
        let creation_catalog = Arc::new(world.progression_catalog().clone());
        let actor_store = Arc::clone(&store);
        let area_router = area_coordinator.map(AreaGenerationRouter::new);
        let actor_area_router = area_router.clone();
        tokio::spawn(async move {
            let mut world = world;
            let mut runtime_content = runtime_content;
            let mut next_session = 1_u64;
            let mut session_roles = HashMap::new();
            let mut session_staff_accesses = HashMap::new();

            let startup = match recovery {
                StartupWorldRecovery::Empty => Ok(()),
                StartupWorldRecovery::Snapshot { snapshot, .. } => world
                    .restore_runtime_snapshot(snapshot)
                    .map_err(|error| GameError::Startup {
                        stage: "restore_runtime_snapshot",
                        reason: error.to_string(),
                    }),
                StartupWorldRecovery::Ledger(events) => {
                    for (index, event) in events.iter().enumerate() {
                        if let Err(error) = world.replay_persisted_event(event) {
                            return report_startup_failure(
                                startup_reply,
                                GameError::Startup {
                                    stage: "replay_domain_events",
                                    reason: format!("event {index}: {error}"),
                                },
                            );
                        }
                    }
                    Ok(())
                }
            };
            if let Err(error) = startup {
                return report_startup_failure(startup_reply, error);
            }
            let loot_identity_ids = world.loot_identity_ids();
            if let Err(error) = actor_store
                .ensure_character_ids_exist_batch(loot_identity_ids.iter().copied().collect())
                .await
            {
                return report_startup_failure(
                    startup_reply,
                    GameError::Startup {
                        stage: "validate_loot_identities",
                        reason: error.to_string(),
                    },
                );
            }
            let runtime_owner_ids = world.runtime_item_owner_ids();
            let trade_participant_ids = world.trade_participants();
            let extra_trade_participant_ids = trade_participant_ids
                .difference(&runtime_owner_ids)
                .copied()
                .collect::<BTreeSet<_>>();
            let durable_owner_gameplay = match actor_store
                .load_character_gameplay_batch(
                    runtime_owner_ids.iter().copied().collect(),
                    StartupGameplayLoadLimits::ITEM_OWNERS,
                )
                .await
            {
                Ok(aggregates) => aggregates,
                Err(error) => {
                    return report_startup_failure(
                        startup_reply,
                        GameError::Startup {
                            stage: "load_character_aggregates",
                            reason: error.to_string(),
                        },
                    );
                }
            };
            let durable_trade_gameplay = match actor_store
                .load_character_gameplay_batch(
                    extra_trade_participant_ids.iter().copied().collect(),
                    StartupGameplayLoadLimits::TRADE_PARTICIPANTS,
                )
                .await
            {
                Ok(aggregates) => aggregates,
                Err(error) => {
                    return report_startup_failure(
                        startup_reply,
                        GameError::Startup {
                            stage: "load_trade_participant_aggregates",
                            reason: error.to_string(),
                        },
                    );
                }
            };
            if let Err(error) = validate_startup_character_aggregates(
                &world,
                &runtime_owner_ids,
                &durable_owner_gameplay,
            ) {
                return report_startup_failure(
                    startup_reply,
                    GameError::Startup {
                        stage: "validate_character_aggregates",
                        reason: error.to_string(),
                    },
                );
            }
            if let Err(error) = validate_startup_character_aggregates(
                &world,
                &extra_trade_participant_ids,
                &durable_trade_gameplay,
            ) {
                return report_startup_failure(
                    startup_reply,
                    GameError::Startup {
                        stage: "validate_trade_participant_aggregates",
                        reason: error.to_string(),
                    },
                );
            }
            let startup_gameplay = durable_owner_gameplay
                .iter()
                .chain(&durable_trade_gameplay)
                .map(|aggregate| (aggregate.character_id, aggregate.gameplay.clone()))
                .collect::<BTreeMap<_, _>>();
            let trade_gameplay = trade_participant_ids
                .iter()
                .filter_map(|character_id| {
                    startup_gameplay
                        .get(character_id)
                        .cloned()
                        .map(|gameplay| (*character_id, gameplay))
                })
                .collect::<BTreeMap<_, _>>();
            let trade_validation = if trade_gameplay.len() != trade_participant_ids.len() {
                Err("one or more trade participant aggregates are absent".to_owned())
            } else {
                world
                    .validate_trade_participant_gameplay(&trade_gameplay)
                    .map_err(|error| error.to_string())
            };
            if let Err(reason) = trade_validation {
                return report_startup_failure(
                    startup_reply,
                    GameError::Startup {
                        stage: "validate_restored_trades",
                        reason,
                    },
                );
            }
            if !trade_participant_ids.is_empty() {
                let checkpoint = world.checkpoint();
                let events = match world.cancel_all_trades(TradeCancellationReason::Restart) {
                    Ok(events) => events,
                    Err(error) => {
                        world.restore_checkpoint(checkpoint);
                        return report_startup_failure(
                            startup_reply,
                            GameError::Startup {
                                stage: "cancel_restored_trades",
                                reason: error.to_string(),
                            },
                        );
                    }
                };
                if let Err(error) = actor_store
                    .commit_action(Vec::new(), world.runtime_snapshot(), events)
                    .await
                {
                    world.restore_checkpoint(checkpoint);
                    return report_startup_failure(
                        startup_reply,
                        GameError::Startup {
                            stage: "commit_restored_trade_cleanup",
                            reason: error.to_string(),
                        },
                    );
                }
            }
            let checkpoint = world.checkpoint();
            let loot_cleanup_events = match world.cancel_all_loot_encounters_for_restart() {
                Ok(events) => events,
                Err(error) => {
                    world.restore_checkpoint(checkpoint);
                    return report_startup_failure(
                        startup_reply,
                        GameError::Startup {
                            stage: "cancel_restored_loot_encounters",
                            reason: error.to_string(),
                        },
                    );
                }
            };
            if !loot_cleanup_events.is_empty()
                && let Err(error) = actor_store
                    .commit_action(Vec::new(), world.runtime_snapshot(), loot_cleanup_events)
                    .await
            {
                world.restore_checkpoint(checkpoint);
                return report_startup_failure(
                    startup_reply,
                    GameError::Startup {
                        stage: "commit_restored_loot_cleanup",
                        reason: error.to_string(),
                    },
                );
            }
            drop(loot_identity_ids);
            drop(trade_gameplay);
            drop(startup_gameplay);
            drop(durable_trade_gameplay);
            drop(durable_owner_gameplay);
            drop(extra_trade_participant_ids);
            drop(trade_participant_ids);
            drop(runtime_owner_ids);
            if confirm_initial_dynamic_state
                && let Err(error) = actor_store
                    .commit_action(Vec::new(), world.runtime_snapshot(), Vec::new())
                    .await
            {
                return report_startup_failure(
                    startup_reply,
                    GameError::Startup {
                        stage: "commit_initial_area_state",
                        reason: error.to_string(),
                    },
                );
            }
            // These purges mutate the selected backend. Keep them behind all
            // recovery replay/restore and startup validation so corrupt
            // durable world authority cannot change unrelated stored data
            // before the process fails closed.
            maintain_expired_social_content(actor_store.as_ref()).await;
            if startup_reply.send(Ok(())).is_err() {
                error!(
                    stage = "startup_barrier",
                    "world actor startup caller disappeared"
                );
                return;
            }

            let mut interval = scheduler.tick_interval.map(tokio::time::interval);
            if let Some(interval) = interval.as_mut() {
                interval.set_missed_tick_behavior(MissedTickBehavior::Skip);
                interval.tick().await;
            }
            let mut social_maintenance = tokio::time::interval(SOCIAL_MAINTENANCE_INTERVAL);
            social_maintenance.set_missed_tick_behavior(MissedTickBehavior::Skip);
            // Tokio intervals fire immediately. Startup already attempted the
            // bounded maintenance pass, so consume that first instant and
            // schedule subsequent runs one hour apart.
            social_maintenance.tick().await;

            loop {
                let command = if let Some(interval) = interval.as_mut() {
                    tokio::select! {
                        command = rx.recv() => command,
                        _ = interval.tick() => {
                            let started = Instant::now();
                            let succeeded = persist_tick(
                                &mut world,
                                &runtime_content,
                                &session_roles,
                                actor_store.as_ref(),
                                &mut actor_delivery,
                            ).await;
                            if let Some(metrics) = &operational_metrics {
                                metrics.tick_finished(started.elapsed(), succeeded);
                            }
                            continue;
                        }
                        _ = social_maintenance.tick() => {
                            maintain_expired_social_content(actor_store.as_ref()).await;
                            continue;
                        }
                    }
                } else {
                    tokio::select! {
                        command = rx.recv() => command,
                        _ = social_maintenance.tick() => {
                            maintain_expired_social_content(actor_store.as_ref()).await;
                            continue;
                        }
                    }
                };
                let Some(command) = command else {
                    break;
                };
                match command {
                    Command::Ping { reply } => {
                        let _ = reply.send(());
                    }
                    Command::CurrentState { session, reply } => {
                        let result =
                            current_state(&world, &runtime_content, &session_roles, session);
                        let _ = reply.send(result);
                    }
                    Command::InitialGameplay {
                        lineage_id,
                        background,
                        base_class_id,
                        reply,
                    } => {
                        let result = world
                            .initial_gameplay(
                                lineage_id.as_deref(),
                                background.as_core(),
                                base_class_id.as_deref(),
                            )
                            .map_err(GameError::from);
                        let _ = reply.send(result);
                    }
                    Command::EnterPreparedCharacter {
                        snapshot,
                        gameplay,
                        staff_access,
                        reply,
                    } => {
                        let session = allocate_session(&mut next_session);
                        let result = enter_prepared_character(
                            &mut world,
                            actor_store.as_ref(),
                            session,
                            snapshot,
                            *gameplay,
                        )
                        .await;
                        let result = finish_enter(
                            FinishEnterResources {
                                world: &mut world,
                                runtime_content: &runtime_content,
                                session_roles: &mut session_roles,
                                session_staff_accesses: &mut session_staff_accesses,
                                store: actor_store.as_ref(),
                                delivery: &mut actor_delivery,
                            },
                            staff_access,
                            result,
                        )
                        .await
                        .map(|entered| EnteredWorld {
                            session_id: entered.session_id,
                            account_id: entered.account_id,
                            scene: entered.scene,
                            presentation: runtime_content
                                .resolve_entity_presentation(&world, entered.session_id),
                            player: entered.player,
                            deliveries: actor_delivery.register(session),
                        });
                        if let Err(Ok(entered)) = reply.send(result) {
                            let checkpoint = world.checkpoint();
                            let cancellation = world
                                .character_snapshot(entered.session_id)
                                .map_err(GameError::from)
                                .and_then(|character| {
                                    world
                                        .cancel_trade_for_character(
                                            character.id,
                                            TradeCancellationReason::ParticipantDisconnected,
                                        )
                                        .map_err(GameError::from)
                                });
                            let committed = if let Ok(mut cancellation_events) = cancellation {
                                match reconcile_world_trades(
                                    &mut world,
                                    actor_store.as_ref(),
                                    &checkpoint,
                                )
                                .await
                                {
                                    Ok(events) => {
                                        cancellation_events.extend(events);
                                        cancellation_events.is_empty()
                                            || actor_store
                                                .commit_action(
                                                    durable_connected_characters(&world)
                                                        .unwrap_or_default(),
                                                    world.runtime_snapshot(),
                                                    cancellation_events,
                                                )
                                                .await
                                                .is_ok()
                                    }
                                    Err(_) => false,
                                }
                            } else {
                                false
                            };
                            if !committed {
                                world.restore_checkpoint(checkpoint);
                            }
                            session_roles.remove(&entered.session_id);
                            session_staff_accesses.remove(&entered.session_id);
                            if let Ok(character) = world.character_snapshot(entered.session_id) {
                                social.disconnected(character.id);
                            }
                            world.disconnect(entered.session_id);
                            actor_delivery.unregister(entered.session_id);
                        }
                    }
                    Command::Apply {
                        session,
                        action,
                        reply,
                    } => {
                        let role = session_roles
                            .get(&session)
                            .copied()
                            .unwrap_or(AccountRole::Player);
                        let result = apply(
                            ApplyResources {
                                world: &mut world,
                                runtime_content: &runtime_content,
                                social: &mut social,
                                store: actor_store.as_ref(),
                                session_roles: &mut session_roles,
                                session_staff_accesses: &mut session_staff_accesses,
                                delivery: &mut actor_delivery,
                            },
                            session,
                            role,
                            action,
                        )
                        .await;
                        let _ = reply.send(result);
                    }
                    Command::InstallPreparedAreas { batch, reply } => {
                        let result = install_prepared_areas(
                            &mut world,
                            &mut runtime_content,
                            actor_store.as_ref(),
                            batch.as_ref(),
                        )
                        .await;
                        if let (Ok(area_ids), Some(router)) = (&result, &actor_area_router) {
                            router.mark_resident_if_active(&batch.core.generation_id, area_ids);
                        }
                        if let Some(metrics) = &operational_metrics {
                            metrics.set_resident_areas(world.active_area_count());
                            metrics.set_deferred_area_exits(world.deferred_area_exit_count());
                        }
                        let _ = reply.send(result);
                    }
                    Command::RetirePreparedAreas {
                        prepared,
                        expected_generation,
                        reply,
                    } => {
                        let result = retire_prepared_areas(
                            &mut world,
                            &mut runtime_content,
                            actor_store.as_ref(),
                            prepared.as_ref(),
                            &expected_generation,
                        )
                        .await;
                        if let (Ok(area_ids), Some(router)) = (&result, &actor_area_router)
                            && router
                                .mark_nonresident_if_active(&prepared.core.generation_id, area_ids)
                                .is_err()
                        {
                            error!("durable retirement could not publish coordinator residence");
                            let _ = reply
                                .send(Err(AreaRetirementOperationError::DurableAuthorityAmbiguous));
                            rx.close();
                            break;
                        }
                        let must_fail_stop = matches!(
                            &result,
                            Err(AreaRetirementOperationError::DurableAuthorityAmbiguous)
                        );
                        let _ = reply.send(result);
                        if must_fail_stop {
                            rx.close();
                            break;
                        }
                    }
                    Command::RehydratePreparedAreas {
                        prepared,
                        checkpoint,
                        expected_generation,
                        reply,
                    } => {
                        let result = rehydrate_prepared_areas(
                            &mut world,
                            &mut runtime_content,
                            actor_store.as_ref(),
                            prepared.as_ref(),
                            &checkpoint,
                            &expected_generation,
                        )
                        .await;
                        if let (Ok(area_ids), Some(router)) = (&result, &actor_area_router)
                            && !router
                                .mark_resident_if_active(&prepared.core.generation_id, area_ids)
                        {
                            error!("durable rehydration could not publish coordinator residence");
                            let _ = reply
                                .send(Err(AreaRetirementOperationError::DurableAuthorityAmbiguous));
                            rx.close();
                            break;
                        }
                        let must_fail_stop = matches!(
                            &result,
                            Err(AreaRetirementOperationError::DurableAuthorityAmbiguous)
                        );
                        let _ = reply.send(result);
                        if must_fail_stop {
                            rx.close();
                            break;
                        }
                    }
                    Command::InspectAreaQuiescence { reply } => {
                        let _ = reply.send(AreaGenerationQuiescence {
                            connected_sessions: world.connected_sessions().len(),
                        });
                    }
                    Command::ResolveAreaOperator {
                        session,
                        capability,
                        reply,
                    } => {
                        let result = resolve_area_operator(
                            &world,
                            &runtime_content,
                            &session_roles,
                            &session_staff_accesses,
                            session,
                            capability,
                        );
                        let _ = reply.send(result);
                    }
                    Command::ActivatePreparedGeneration {
                        prepared,
                        transition,
                        #[cfg(unix)]
                        publication_guard,
                        expected_current,
                        next,
                        reply,
                    } => {
                        let result = activate_prepared_generation(
                            &mut world,
                            &mut runtime_content,
                            actor_store.as_ref(),
                            GenerationActivationCommit {
                                prepared: *prepared,
                                transition,
                                #[cfg(unix)]
                                publication_guard,
                                expected_current,
                                next,
                            },
                        )
                        .await;
                        if result.is_ok()
                            && let Some(metrics) = &operational_metrics
                        {
                            metrics.set_resident_areas(world.active_area_count());
                            metrics.set_deferred_area_exits(world.deferred_area_exit_count());
                        }
                        let must_fail_stop = matches!(
                            &result,
                            Err(AreaGenerationActivationError::DurableAuthorityAmbiguous)
                        );
                        let _ = reply.send(result);
                        if must_fail_stop {
                            error!(
                                "world actor stopped after an ambiguous durable area-generation commit"
                            );
                            rx.close();
                            break;
                        }
                    }
                    Command::Leave { session } => {
                        let role = session_roles
                            .remove(&session)
                            .unwrap_or(AccountRole::Player);
                        session_staff_accesses.remove(&session);
                        let checkpoint = world.checkpoint();
                        let leaving_character = world
                            .character_snapshot(session)
                            .ok()
                            .map(|character| character.id);
                        let (mut committed_events, mutation_failed) =
                            if let Some(character_id) = leaving_character {
                                match world.cancel_trade_for_character(
                                    character_id,
                                    TradeCancellationReason::ParticipantDisconnected,
                                ) {
                                    Ok(events) => (events, false),
                                    Err(_) => (Vec::new(), true),
                                }
                            } else {
                                (Vec::new(), false)
                            };
                        let hook = runtime_content.run_hook(
                            &mut world,
                            session,
                            role,
                            ScriptHook::OnLeave,
                        );
                        committed_events.extend(hook.domain_events.clone());
                        let reconciled =
                            reconcile_world_trades(&mut world, actor_store.as_ref(), &checkpoint)
                                .await;
                        let persistence_failed = mutation_failed
                            || match reconciled {
                                Ok(events) => {
                                    committed_events.extend(events);
                                    (hook.changed || !committed_events.is_empty())
                                        && actor_store
                                            .commit_action(
                                                durable_connected_characters(&world)
                                                    .unwrap_or_default(),
                                                world.runtime_snapshot(),
                                                committed_events.clone(),
                                            )
                                            .await
                                            .is_err()
                                }
                                Err(_) => true,
                            };
                        if persistence_failed {
                            world.restore_checkpoint(checkpoint);
                        } else if hook.changed || !committed_events.is_empty() {
                            broadcast_world_deliveries(
                                &world,
                                &runtime_content,
                                &session_roles,
                                Some(session),
                                &committed_events,
                                &mut actor_delivery,
                            );
                        }
                        if let Ok(character) = world.character_snapshot(session) {
                            social.disconnected(character.id);
                        }
                        world.disconnect(session);
                        actor_delivery.unregister(session);
                    }
                    Command::Shutdown { reply } => {
                        rx.close();
                        let _ = reply.send(());
                        break;
                    }
                }
            }
        });
        let handle = Self {
            tx,
            store,
            start_room_id,
            creation_catalog,
            area_router,
            area_rehydration_lock: Arc::new(AsyncMutex::new(())),
        };
        match startup_receive.await {
            Ok(Ok(())) => Ok(handle),
            Ok(Err(error)) => Err(error),
            Err(_) => {
                error!(
                    stage = "startup_task",
                    "world actor stopped before reporting startup readiness"
                );
                Err(GameError::Startup {
                    stage: "startup_task",
                    reason: "actor stopped before reporting readiness".to_owned(),
                })
            }
        }
    }

    pub async fn health_check(&self) -> Result<(), GameError> {
        let (reply, receive) = oneshot::channel();
        self.tx
            .send(Command::Ping { reply })
            .await
            .map_err(|_| GameError::ActorStopped)?;
        receive.await.map_err(|_| GameError::ActorStopped)
    }

    pub fn queue_depth(&self) -> usize {
        WORLD_COMMAND_QUEUE_CAPACITY.saturating_sub(self.tx.capacity())
    }

    pub async fn register_account(
        &self,
        name: String,
        password: String,
    ) -> Result<AuthenticatedSession, GameError> {
        self.store
            .register_account(&name, &password)
            .await
            .map_err(GameError::from)
    }

    pub async fn login_account(
        &self,
        name: String,
        password: String,
    ) -> Result<AuthenticatedSession, GameError> {
        self.store
            .login_account(&name, &password)
            .await
            .map_err(GameError::from)
    }

    pub async fn resolve_session(&self, token: String) -> Result<AccountSnapshot, GameError> {
        self.store
            .resolve_session(&token)
            .await
            .map_err(GameError::from)
    }

    pub async fn revoke_session(&self, token: String) -> Result<(), GameError> {
        self.store
            .revoke_session(&token)
            .await
            .map_err(GameError::from)
    }

    pub async fn authenticated_roster(
        &self,
        token: String,
    ) -> Result<Vec<CharacterSnapshot>, GameError> {
        authenticated_roster(self.store.as_ref(), &token).await
    }

    pub fn creation_catalog(&self) -> &CharacterCreationCatalogView {
        &self.creation_catalog
    }

    #[cfg(test)]
    pub async fn create_authenticated_character(
        &self,
        token: String,
        name: String,
    ) -> Result<CharacterSnapshot, GameError> {
        self.create_authenticated_character_with_progression(
            token,
            name,
            None,
            CharacterBackgroundSelection::Default,
            None,
        )
        .await
    }

    pub async fn create_authenticated_character_with_progression(
        &self,
        token: String,
        name: String,
        lineage_id: Option<String>,
        background: CharacterBackgroundSelection,
        base_class_id: Option<String>,
    ) -> Result<CharacterSnapshot, GameError> {
        let account = self.store.resolve_session(&token).await?;
        let initial_gameplay = self
            .initial_gameplay(lineage_id, background, base_class_id)
            .await?;
        self.store
            .load_or_create_character_with_gameplay(
                account.id,
                &name,
                self.start_room_id.as_ref(),
                initial_gameplay,
            )
            .await
            .map_err(GameError::from)
    }

    async fn initial_gameplay(
        &self,
        lineage_id: Option<String>,
        background: CharacterBackgroundSelection,
        base_class_id: Option<String>,
    ) -> Result<CharacterGameplaySnapshot, GameError> {
        let (reply, receive) = oneshot::channel();
        self.tx
            .send(Command::InitialGameplay {
                lineage_id,
                background,
                base_class_id,
                reply,
            })
            .await
            .map_err(|_| GameError::ActorStopped)?;
        receive.await.map_err(|_| GameError::ActorStopped)?
    }

    pub async fn enter_authenticated_character(
        &self,
        token: String,
        character_id: CharacterId,
    ) -> Result<EnteredWorld, GameError> {
        let account = self.store.resolve_session(&token).await?;
        self.enter_loaded_character(account.id, character_id).await
    }

    async fn enter_loaded_character(
        &self,
        account_id: AccountId,
        character_id: CharacterId,
    ) -> Result<EnteredWorld, GameError> {
        // Pin one exact generation before the first storage read. The guard
        // remains alive through lazy preparation, actor installation and the
        // final entry response, closing the pre-actor race with a 6C cutover.
        let area_route = self
            .area_router
            .as_ref()
            .map(|router| router.try_route(None))
            .transpose()
            .map_err(|_| GameError::AreaUnavailable)?;
        // All authentication, password work and read-only storage waits happen
        // before the state-owning actor sees the entry command. The actor only
        // receives an authorized, fully loaded snapshot and serializes the
        // world mutation that grants the live character lease.
        let snapshot = self.store.load_character(account_id, character_id).await?;
        if let Some(coordinator) = area_route.as_ref().map(|route| route.coordinator()) {
            let owner = coordinator
                .generation()
                .room_owner(&snapshot.room_id)
                .ok_or(GameError::AreaUnavailable)?;
            if !coordinator.is_resident(owner) {
                if coordinator.is_retired(owner) {
                    self.rehydrate_retired_area(coordinator, owner).await?;
                } else {
                    let batch = coordinator
                        .prepare(owner.to_owned())
                        .await
                        .map_err(|error| {
                            tracing::warn!(%error, "character area preparation failed");
                            GameError::AreaUnavailable
                        })?;
                    self.install_prepared_area_batch(batch)
                        .await
                        .map_err(|error| {
                            tracing::warn!(%error, "character area installation failed");
                            GameError::AreaUnavailable
                        })?;
                }
            }
        }
        let gameplay = self.store.load_character_gameplay(character_id).await?;
        let staff_access = self.store.staff_access(character_id).await?;
        let (reply, receive) = oneshot::channel();
        self.tx
            .send(Command::EnterPreparedCharacter {
                snapshot,
                gameplay: Box::new(gameplay),
                staff_access,
                reply,
            })
            .await
            .map_err(|_| GameError::ActorStopped)?;
        receive.await.map_err(|_| GameError::ActorStopped)?
    }

    #[cfg(test)]
    pub async fn enter(&self, name: String) -> Result<EnteredWorld, GameError> {
        let account = self.open_account(name.clone()).await?;
        let roster = self.list_characters(account.id).await?;
        let character = match roster
            .into_iter()
            .find(|character| character.name.eq_ignore_ascii_case(name.trim()))
        {
            Some(character) => character,
            None => self.load_or_create_character(account.id, name).await?,
        };
        self.enter_character(account.id, character.id).await
    }

    #[cfg(test)]
    async fn open_account(&self, name: String) -> Result<AccountSnapshot, GameError> {
        self.store
            .load_or_create_account(&name)
            .await
            .map_err(GameError::from)
    }

    #[cfg(test)]
    async fn list_characters(
        &self,
        account_id: AccountId,
    ) -> Result<Vec<CharacterSnapshot>, GameError> {
        self.store
            .list_characters(account_id)
            .await
            .map_err(GameError::from)
    }

    #[cfg(test)]
    async fn load_or_create_character(
        &self,
        account_id: AccountId,
        name: String,
    ) -> Result<CharacterSnapshot, GameError> {
        let initial_gameplay = self
            .initial_gameplay(None, CharacterBackgroundSelection::Default, None)
            .await?;
        self.store
            .load_or_create_character_with_gameplay(
                account_id,
                &name,
                self.start_room_id.as_ref(),
                initial_gameplay,
            )
            .await
            .map_err(GameError::from)
    }

    #[cfg(test)]
    async fn enter_character(
        &self,
        account_id: AccountId,
        character_id: CharacterId,
    ) -> Result<EnteredWorld, GameError> {
        self.enter_loaded_character(account_id, character_id).await
    }

    pub async fn apply(
        &self,
        session: SessionId,
        action: PlayerAction,
    ) -> Result<AppliedAction, GameError> {
        // Area lifecycle operations dispatch actor commands from this transport
        // layer; routing them through the actor's `apply` would re-enter the
        // actor and deadlock. Intercept them before any route lease is taken.
        match &action {
            PlayerAction::Staff(tiamud_core::StaffAction::RetireArea { area, reason }) => {
                return self.staff_retire_area(session, area, reason).await;
            }
            PlayerAction::Staff(tiamud_core::StaffAction::RehydrateArea { area, reason }) => {
                return self.staff_rehydrate_area(session, area, reason).await;
            }
            _ => {}
        }
        // One route lease spans the initial actor call, an optional deferred
        // load and install, and the single retry. An admitted action therefore
        // cannot observe two catalog generations.
        let area_route = self
            .area_router
            .as_ref()
            .map(|router| router.try_route(None))
            .transpose()
            .map_err(|_| GameError::AreaUnavailable)?;
        let first = self.apply_once(session, action.clone()).await;
        let request = match first {
            Err(GameError::World(CoreWorldError::AreaLoadRequired(request))) => request,
            result => return result,
        };
        let Some(route) = area_route.as_ref() else {
            return Err(GameError::AreaUnavailable);
        };
        if route.generation() != &request.generation_id {
            return Err(GameError::AreaUnavailable);
        }
        if route.coordinator().is_retired(&request.target_area_id) {
            self.rehydrate_retired_area(route.coordinator(), &request.target_area_id)
                .await?;
        } else {
            let batch = route
                .coordinator()
                .prepare(request.target_area_id)
                .await
                .map_err(|error| {
                    tracing::warn!(%error, "area preparation failed");
                    GameError::AreaUnavailable
                })?;
            self.install_prepared_area_batch(batch)
                .await
                .map_err(|error| {
                    tracing::warn!(%error, "area installation failed");
                    GameError::AreaUnavailable
                })?;
        }
        match self.apply_once(session, action).await {
            Err(GameError::World(CoreWorldError::AreaLoadRequired(_))) => {
                Err(GameError::AreaUnavailable)
            }
            result => result,
        }
    }

    async fn apply_once(
        &self,
        session: SessionId,
        action: PlayerAction,
    ) -> Result<AppliedAction, GameError> {
        let (reply, receive) = oneshot::channel();
        self.tx
            .send(Command::Apply {
                session,
                action,
                reply,
            })
            .await
            .map_err(|_| GameError::ActorStopped)?;
        receive.await.map_err(|_| GameError::ActorStopped)?
    }

    pub(crate) async fn install_prepared_area_batch(
        &self,
        batch: Arc<PreparedAreaBatch>,
    ) -> Result<(), GameError> {
        let (reply, receive) = oneshot::channel();
        self.tx
            .send(Command::InstallPreparedAreas { batch, reply })
            .await
            .map_err(|_| GameError::ActorStopped)?;
        let _installed = receive.await.map_err(|_| GameError::ActorStopped)??;
        Ok(())
    }

    async fn resolve_area_operator(
        &self,
        session: SessionId,
        capability: tiamud_core::StaffCapability,
    ) -> Result<AreaOperatorAuthority, GameError> {
        let (reply, receive) = oneshot::channel();
        self.tx
            .send(Command::ResolveAreaOperator {
                session,
                capability,
                reply,
            })
            .await
            .map_err(|_| GameError::ActorStopped)?;
        receive.await.map_err(|_| GameError::ActorStopped)?
    }

    async fn staff_retire_area(
        &self,
        session: SessionId,
        area: &str,
        reason: &str,
    ) -> Result<AppliedAction, GameError> {
        let area = crate::staff::normalise_text(area, 128)?;
        let reason =
            crate::staff::normalise_text(reason, crate::persistence::MAX_SOCIAL_REASON_BYTES)?;
        let authority = self
            .resolve_area_operator(session, tiamud_core::StaffCapability::StaffAssign)
            .await?;
        self.retire_area(&area)
            .await
            .map_err(map_area_operation_error)?;
        self.store
            .append_staff_audit(crate::persistence::StaffAuditRecord {
                actor_account_id: authority.account_id,
                target_account_id: None,
                capability: tiamud_core::StaffCapability::StaffAssign,
                action: "staff_area_retire".to_owned(),
                reason: reason.clone(),
                details: serde_json::json!({
                    "actor_name": authority.actor_name.clone(),
                    "area": area,
                }),
            })
            .await?;
        Ok(area_operation_applied(
            authority,
            "Area retirement accepted.\r\n",
        ))
    }

    async fn staff_rehydrate_area(
        &self,
        session: SessionId,
        area: &str,
        reason: &str,
    ) -> Result<AppliedAction, GameError> {
        let area = crate::staff::normalise_text(area, 128)?;
        let reason =
            crate::staff::normalise_text(reason, crate::persistence::MAX_SOCIAL_REASON_BYTES)?;
        let authority = self
            .resolve_area_operator(session, tiamud_core::StaffCapability::StaffAssign)
            .await?;
        let router = self
            .area_router
            .as_ref()
            .ok_or(GameError::AreaOperationInvalid)?;
        let route = router
            .try_route(None)
            .map_err(|_| GameError::AreaOperationBusy)?;
        self.rehydrate_retired_area(route.coordinator(), &area)
            .await?;
        drop(route);
        self.store
            .append_staff_audit(crate::persistence::StaffAuditRecord {
                actor_account_id: authority.account_id,
                target_account_id: None,
                capability: tiamud_core::StaffCapability::StaffAssign,
                action: "staff_area_rehydrate".to_owned(),
                reason: reason.clone(),
                details: serde_json::json!({
                    "actor_name": authority.actor_name.clone(),
                    "area": area,
                }),
            })
            .await?;
        Ok(area_operation_applied(
            authority,
            "Area rehydration accepted.\r\n",
        ))
    }

    pub(crate) async fn retire_area(
        &self,
        area_id: &str,
    ) -> Result<(), AreaRetirementOperationError> {
        let router = self
            .area_router
            .as_ref()
            .ok_or(AreaRetirementOperationError::Invalid)?;
        let route = router
            .try_route(None)
            .map_err(|_| AreaRetirementOperationError::Busy)?;
        let expected_generation = self
            .store
            .load_area_generation()
            .await
            .map_err(AreaRetirementOperationError::Persistence)?
            .ok_or(AreaRetirementOperationError::Invalid)?;
        if expected_generation.active_catalog_sha256 != route.generation().as_str() {
            return Err(AreaRetirementOperationError::Invalid);
        }
        let prepared = route.coordinator().prepare_retirement(area_id).await?;
        let (reply, receive) = oneshot::channel();
        self.tx
            .send(Command::RetirePreparedAreas {
                prepared,
                expected_generation,
                reply,
            })
            .await
            .map_err(|_| AreaRetirementOperationError::ActorStopped)?;
        let _area_ids = receive
            .await
            .map_err(|_| AreaRetirementOperationError::ActorStopped)??;
        drop(route);
        Ok(())
    }

    async fn rehydrate_retired_area(
        &self,
        coordinator: &AreaLoadCoordinator,
        area_id: &str,
    ) -> Result<(), GameError> {
        // One bounded process-wide critical section is sufficient for the
        // single-writer baseline and makes concurrent moves into the same
        // retired unit observe the first durable result instead of racing its
        // consumed checkpoint. It can be narrowed to a per-unit map later.
        let _rehydration = self.area_rehydration_lock.lock().await;
        if coordinator.is_resident(area_id) {
            return Ok(());
        }
        if !coordinator.is_retired(area_id) {
            return Err(GameError::AreaUnavailable);
        }
        let durable_checkpoints = self.store.load_area_checkpoints().await?;
        let mut selected = None;
        for durable in durable_checkpoints {
            let checkpoint = validate_area_checkpoint(&durable)?;
            if checkpoint.areas.iter().any(|area| area.area_id == area_id)
                && selected.replace((durable, checkpoint)).is_some()
            {
                return Err(GameError::AreaUnavailable);
            }
        }
        let (durable, checkpoint) = selected.ok_or(GameError::AreaUnavailable)?;
        let expected_generation = self
            .store
            .load_area_generation()
            .await?
            .ok_or(GameError::AreaUnavailable)?;
        if expected_generation.active_catalog_sha256 != coordinator.generation().generation_id() {
            return Err(GameError::AreaUnavailable);
        }
        let prepared = coordinator
            .prepare_rehydration(&checkpoint)
            .await
            .map_err(|error| {
                warn!(%error, "retired area rehydration preparation failed");
                GameError::AreaUnavailable
            })?;
        let (reply, receive) = oneshot::channel();
        self.tx
            .send(Command::RehydratePreparedAreas {
                prepared,
                checkpoint: durable,
                expected_generation,
                reply,
            })
            .await
            .map_err(|_| GameError::ActorStopped)?;
        receive
            .await
            .map_err(|_| GameError::ActorStopped)?
            .map_err(|error| {
                warn!(%error, "retired area rehydration failed");
                GameError::AreaUnavailable
            })?;
        Ok(())
    }

    pub(crate) fn area_generation_status(
        &self,
    ) -> Option<crate::area_runtime::AreaGenerationStatus> {
        self.area_router.as_ref().map(AreaGenerationRouter::status)
    }

    pub(crate) async fn inspect_area_quiescence(
        &self,
    ) -> Result<AreaGenerationQuiescence, AreaGenerationActivationError> {
        let (reply, receive) = oneshot::channel();
        self.tx
            .send(Command::InspectAreaQuiescence { reply })
            .await
            .map_err(|_| AreaGenerationActivationError::ActorStopped)?;
        receive
            .await
            .map_err(|_| AreaGenerationActivationError::ActorStopped)
    }

    /// Activates one already prepared, registry-compatible full generation.
    ///
    /// This is an operator-internal boundary: it closes new admissions and
    /// lazy loads, requires all admitted work to have released its generation
    /// lease, and lets the actor prove that no sessions remain. The durable
    /// compare-and-swap is delegated to `CharacterStore`; only after it
    /// succeeds are the candidate world, runtime content and router published.
    pub(crate) async fn activate_prepared_generation(
        &self,
        prepared: PreparedGenerationCutover,
        expected_current: DurableAreaGeneration,
        next: DurableAreaGeneration,
    ) -> Result<(), AreaGenerationActivationError> {
        if expected_current.active_catalog_sha256 != prepared.expected_current_generation.as_str()
            || next.active_catalog_sha256 != prepared.candidate_generation.as_str()
            || expected_current.world_id != next.world_id
            || prepared
                .candidate_coordinator()
                .generation()
                .catalog()
                .world_id
                != next.world_id
        {
            return Err(AreaGenerationActivationError::InvalidCandidate);
        }
        let router = self
            .area_router
            .as_ref()
            .ok_or(AreaGenerationActivationError::InvalidCandidate)?;
        if router.has_retired_areas() {
            return Err(AreaGenerationActivationError::Busy);
        }
        #[cfg(unix)]
        let publication_guard =
            prepared
                .try_acquire_candidate_publication_lock()
                .map_err(|error| match error {
                    LocalAreaRegistryError::PublicationLockBusy => {
                        AreaGenerationActivationError::Busy
                    }
                    _ => AreaGenerationActivationError::InvalidCandidate,
                })?;
        // The shared publication guard is retained by the actor command until
        // the durable CAS has a definitive/reconciled outcome. This first read
        // rejects a candidate that changed while lock acquisition raced.
        prepared
            .revalidate_candidate_pointer()
            .map_err(|_| AreaGenerationActivationError::InvalidCandidate)?;
        let mut transition = router.begin_staging(&prepared)?;
        transition.begin_draining()?;
        // Re-read after admission closes and immediately before enqueueing.
        // Cooperating writers remain excluded by the shared publication lock.
        prepared
            .revalidate_candidate_pointer()
            .map_err(|_| AreaGenerationActivationError::InvalidCandidate)?;
        let (reply, receive) = oneshot::channel();
        match self.tx.try_send(Command::ActivatePreparedGeneration {
            prepared: Box::new(prepared),
            transition,
            #[cfg(unix)]
            publication_guard,
            expected_current,
            next,
            reply,
        }) {
            Ok(()) => {}
            Err(mpsc::error::TrySendError::Full(_)) => {
                return Err(AreaGenerationActivationError::Busy);
            }
            Err(mpsc::error::TrySendError::Closed(_)) => {
                return Err(AreaGenerationActivationError::ActorStopped);
            }
        }
        receive
            .await
            .map_err(|_| AreaGenerationActivationError::ActorStopped)?
    }

    pub async fn current_state(
        &self,
        session: SessionId,
    ) -> Result<(SceneView, ScenePresentation, PlayerStateView), GameError> {
        let (reply, receive) = oneshot::channel();
        self.tx
            .send(Command::CurrentState { session, reply })
            .await
            .map_err(|_| GameError::ActorStopped)?;
        receive.await.map_err(|_| GameError::ActorStopped)?
    }

    pub async fn leave(&self, session: SessionId) {
        let _ = self.tx.send(Command::Leave { session }).await;
    }

    /// Schedule cleanup from a connection guard whose `Drop` cannot await.
    pub fn leave_nowait(&self, session: SessionId) {
        let command = Command::Leave { session };
        match self.tx.try_send(command) {
            Ok(()) | Err(mpsc::error::TrySendError::Closed(_)) => {}
            Err(mpsc::error::TrySendError::Full(command)) => {
                let tx = self.tx.clone();
                if let Ok(runtime) = tokio::runtime::Handle::try_current() {
                    runtime.spawn(async move {
                        let _ = tx.send(command).await;
                    });
                }
            }
        }
    }

    /// Queue a shutdown barrier. All commands accepted before it are drained;
    /// subsequent sends observe the closed actor channel.
    pub async fn shutdown(&self) -> Result<(), GameError> {
        let (reply, receive) = oneshot::channel();
        self.tx
            .send(Command::Shutdown { reply })
            .await
            .map_err(|_| GameError::ActorStopped)?;
        receive.await.map_err(|_| GameError::ActorStopped)
    }
}

async fn maintain_expired_social_content(store: &dyn CharacterStore) {
    match store.purge_expired_mail().await {
        Ok(0) => {}
        Ok(purged) => tracing::debug!(purged, "purged expired offline mail"),
        Err(error) => {
            // Expired rows are filtered from every read and capacity check.
            // Maintenance failure is therefore operationally visible but does
            // not weaken world availability or expose expired content.
            tracing::warn!(%error, "could not purge expired offline mail");
        }
    }
    match store.purge_expired_boards().await {
        Ok(0) => {}
        Ok(purged) => tracing::debug!(purged, "purged expired community-board content"),
        Err(error) => {
            // Board reads and capacity checks filter expired rows even if a
            // bounded physical purge has to be retried later.
            tracing::warn!(%error, "could not purge expired community-board content");
        }
    }
}

fn validate_startup_character_aggregates(
    world: &World,
    expected_characters: &BTreeSet<CharacterId>,
    aggregates: &[DurableCharacterGameplay],
) -> Result<(), CoreWorldError> {
    let mut durable_characters = std::collections::BTreeSet::new();
    if aggregates.iter().any(|aggregate| {
        aggregate.character_id.0 <= 0 || !durable_characters.insert(aggregate.character_id)
    }) || &durable_characters != expected_characters
    {
        return Err(CoreWorldError::InvalidRuntimeSnapshot);
    }

    for aggregate in aggregates {
        world.validate_character_gameplay_snapshot(aggregate.character_id, &aggregate.gameplay)?;
    }
    Ok(())
}

fn report_startup_failure(reply: oneshot::Sender<Result<(), GameError>>, failure: GameError) {
    let stage = match &failure {
        GameError::Startup { stage, .. } => *stage,
        _ => "unknown",
    };
    error!(stage, error = %failure, "world actor startup failed");
    let _ = reply.send(Err(failure));
}

fn allocate_session(next_session: &mut u64) -> SessionId {
    let session = SessionId(*next_session);
    *next_session = next_session.wrapping_add(1).max(1);
    session
}

async fn activate_prepared_generation(
    world: &mut World,
    runtime_content: &mut RuntimeContent,
    store: &dyn CharacterStore,
    activation: GenerationActivationCommit,
) -> Result<(), AreaGenerationActivationError> {
    let GenerationActivationCommit {
        prepared,
        mut transition,
        #[cfg(unix)]
            publication_guard: _publication_guard,
        expected_current,
        next,
    } = activation;
    if !world.connected_sessions().is_empty()
        || world
            .runtime_snapshot()
            .area_residency
            .values()
            .any(|state| matches!(state, tiamud_core::AreaRuntimeResidence::Retired { .. }))
        || world.area_generation() != Some(&prepared.expected_current_generation)
        || prepared.world.area_generation() != Some(&prepared.candidate_generation)
        || expected_current.active_catalog_sha256 != prepared.expected_current_generation.as_str()
        || next.active_catalog_sha256 != prepared.candidate_generation.as_str()
    {
        return Err(if world.connected_sessions().is_empty() {
            AreaGenerationActivationError::WrongGeneration
        } else {
            AreaGenerationActivationError::Busy
        });
    }

    // The registry compatibility gate has already proved that this is a
    // no-migration replacement. Reusing the ordinary strict restore against a
    // complete private candidate preserves every dynamic authority while
    // allowing static room/presentation/script content to change.
    let mut snapshot = world.runtime_snapshot();
    // A 6B bootstrap may have durably confirmed only its initially resident
    // closure. The 6C candidate is fully staged, so make the transition to a
    // complete resident manifest explicit before strict restore and commit.
    snapshot.area_residency = prepared.world.runtime_snapshot().area_residency;
    let PreparedGenerationCutover {
        expected_current_generation: _,
        candidate_generation: _,
        world: mut candidate_world,
        runtime_content: candidate_runtime_content,
        candidate_coordinator: _,
        ..
    } = prepared;
    candidate_world
        .restore_runtime_snapshot(snapshot)
        .map_err(AreaGenerationActivationError::Runtime)?;

    transition.begin_committing()?;
    if let Some(metrics) = runtime_content.operational_metrics() {
        metrics.set_area_generation_phase(AreaGenerationMetricPhase::Committing);
    }
    let candidate_snapshot = candidate_world.runtime_snapshot();
    if let Err(commit_error) = store
        .commit_area_generation_activation(
            candidate_snapshot,
            expected_current.clone(),
            next.clone(),
        )
        .await
    {
        match store.load_area_generation().await {
            Ok(Some(actual)) if actual == next => {
                warn!(
                    error = %commit_error,
                    "durable area-generation commit succeeded but its acknowledgement was lost"
                );
            }
            Ok(Some(actual)) if actual == expected_current => {
                return Err(AreaGenerationActivationError::Persistence(commit_error));
            }
            Ok(_) => {
                error!(
                    error = %commit_error,
                    "durable area-generation authority differs from both commit endpoints"
                );
                transition.seal_fail_stop();
                return Err(AreaGenerationActivationError::DurableAuthorityAmbiguous);
            }
            Err(reconcile_error) => {
                error!(
                    error = %commit_error,
                    reconciliation_error = %reconcile_error,
                    "durable area-generation authority could not be reconciled after commit failure"
                );
                transition.seal_fail_stop();
                return Err(AreaGenerationActivationError::DurableAuthorityAmbiguous);
            }
        }
    }

    // Durable authority now names the candidate. These assignments and the
    // router publication deliberately contain no validation or I/O. A process
    // failure from this point is recovered from the durable candidate.
    *world = candidate_world;
    *runtime_content = candidate_runtime_content;
    transition.finish_commit();
    Ok(())
}

async fn authenticated_roster(
    store: &dyn CharacterStore,
    token: &str,
) -> Result<Vec<CharacterSnapshot>, GameError> {
    let account = store.resolve_session(token).await?;
    store
        .list_characters(account.id)
        .await
        .map_err(GameError::from)
}

async fn install_prepared_areas(
    world: &mut World,
    runtime_content: &mut RuntimeContent,
    store: &dyn CharacterStore,
    prepared: &PreparedAreaBatch,
) -> Result<Vec<String>, GameError> {
    let started = Instant::now();
    let metrics = runtime_content.operational_metrics();
    let world_checkpoint = world.lifecycle_checkpoint();
    let runtime_checkpoint = runtime_content.clone();
    if prepared.core.areas.is_empty() {
        if world.area_generation() != Some(&prepared.core.generation_id) {
            return Err(CoreWorldError::InvalidAreaLifecycle.into());
        }
        if let Some(metrics) = &metrics {
            metrics.area_load_finished(started.elapsed(), 0, AreaLoadMetricResult::AlreadyResident);
        }
        return Ok(Vec::new());
    }
    let confirmed_area_ids = prepared
        .core
        .areas
        .iter()
        .map(|area| area.area_id.clone())
        .collect::<Vec<_>>();
    let result = (|| {
        let outcome = world.install_prepared_areas(prepared.core.clone())?;
        for area_id in &outcome.installed_area_ids {
            let delta = prepared
                .runtime_by_area
                .get(area_id)
                .cloned()
                .ok_or(CoreWorldError::InvalidAreaLifecycle)?;
            runtime_content
                .install_delta(delta)
                .map_err(|_| CoreWorldError::InvalidAreaLifecycle)?;
        }
        Ok::<_, GameError>(outcome)
    })();
    let outcome = match result {
        Ok(outcome) => outcome,
        Err(error) => {
            world.restore_lifecycle_checkpoint(world_checkpoint);
            *runtime_content = runtime_checkpoint;
            if let Some(metrics) = &metrics {
                metrics.area_load_finished(
                    started.elapsed(),
                    0,
                    AreaLoadMetricResult::InstallationFailed,
                );
            }
            return Err(error);
        }
    };
    if outcome.created_dynamic_state
        && let Err(error) = store
            .commit_action(Vec::new(), world.runtime_snapshot(), Vec::new())
            .await
    {
        world.restore_lifecycle_checkpoint(world_checkpoint);
        *runtime_content = runtime_checkpoint;
        if let Some(metrics) = &metrics {
            metrics.area_load_finished(
                started.elapsed(),
                0,
                AreaLoadMetricResult::InstallationFailed,
            );
        }
        return Err(error.into());
    }
    if let Some(metrics) = &metrics {
        let result = if outcome.installed_areas == 0 {
            AreaLoadMetricResult::AlreadyResident
        } else {
            AreaLoadMetricResult::Installed
        };
        metrics.area_load_finished(started.elapsed(), 0, result);
    }
    Ok(confirmed_area_ids)
}

async fn retire_prepared_areas(
    world: &mut World,
    runtime_content: &mut RuntimeContent,
    store: &dyn CharacterStore,
    prepared: &PreparedAreaRetirementBatch,
    expected_generation: &DurableAreaGeneration,
) -> Result<Vec<String>, AreaRetirementOperationError> {
    if world.area_generation() != Some(&prepared.core.generation_id)
        || expected_generation.active_catalog_sha256 != prepared.core.generation_id.as_str()
    {
        return Err(AreaRetirementOperationError::Invalid);
    }
    let before_snapshot = world.runtime_snapshot();
    let world_checkpoint = world.lifecycle_checkpoint();
    let runtime_checkpoint = runtime_content.clone();
    let retirement = world
        .prepare_area_retirement(prepared.core.clone())
        .map_err(map_core_area_retirement_error)?;
    let checkpoint_json = serde_json::to_string(&retirement.checkpoint)
        .map_err(|_| AreaRetirementOperationError::Invalid)?;
    let checkpoint_sha256 = hex::encode(Sha256::digest(checkpoint_json.as_bytes()));
    let residency_epoch = retirement
        .checkpoint
        .areas
        .first()
        .map(|area| area.residency_epoch)
        .ok_or(AreaRetirementOperationError::Invalid)?;
    if retirement
        .checkpoint
        .areas
        .iter()
        .any(|area| area.residency_epoch != residency_epoch)
    {
        return Err(AreaRetirementOperationError::Invalid);
    }
    let durable = DurableAreaCheckpoint {
        generation_sha256: prepared.core.generation_id.as_str().to_owned(),
        unit_id: prepared.unit_id.clone(),
        residency_epoch,
        checkpoint_version: retirement.checkpoint.checkpoint_version,
        checkpoint_sha256: checkpoint_sha256.clone(),
        captured_logical_tick: retirement.checkpoint.captured_logical_tick,
        checkpoint_json,
    };
    let area_ids = retirement
        .checkpoint
        .areas
        .iter()
        .map(|area| area.area_id.clone())
        .collect::<Vec<_>>();
    let mutated = (|| {
        world
            .apply_prepared_area_retirement(retirement, checkpoint_sha256)
            .map_err(map_core_area_retirement_error)?;
        for area_id in &area_ids {
            let delta = prepared
                .rehydration
                .runtime_by_area
                .get(area_id)
                .ok_or(AreaRetirementOperationError::Invalid)?;
            runtime_content
                .uninstall_delta(delta)
                .map_err(|_| AreaRetirementOperationError::Invalid)?;
        }
        Ok::<(), AreaRetirementOperationError>(())
    })();
    if let Err(error) = mutated {
        world.restore_lifecycle_checkpoint(world_checkpoint);
        *runtime_content = runtime_checkpoint;
        return Err(error);
    }
    let candidate_snapshot = world.runtime_snapshot();
    if let Err(error) = store
        .commit_area_retirement(
            candidate_snapshot.clone(),
            expected_generation.clone(),
            durable.clone(),
        )
        .await
    {
        if area_commit_error_is_definitive(&error) {
            world.restore_lifecycle_checkpoint(world_checkpoint);
            *runtime_content = runtime_checkpoint;
            return Err(AreaRetirementOperationError::Persistence(error));
        }
        match reconcile_area_commit_after_error(
            store,
            &before_snapshot,
            &candidate_snapshot,
            &durable,
            true,
        )
        .await
        {
            AreaCommitReadback::Committed => {
                warn!(%error, "durable area retirement succeeded but its acknowledgement was lost");
            }
            AreaCommitReadback::NotCommitted => {
                world.restore_lifecycle_checkpoint(world_checkpoint);
                *runtime_content = runtime_checkpoint;
                return Err(AreaRetirementOperationError::Persistence(error));
            }
            AreaCommitReadback::Ambiguous => {
                return Err(AreaRetirementOperationError::DurableAuthorityAmbiguous);
            }
        }
    }
    Ok(area_ids)
}

async fn rehydrate_prepared_areas(
    world: &mut World,
    runtime_content: &mut RuntimeContent,
    store: &dyn CharacterStore,
    prepared: &PreparedAreaBatch,
    durable: &DurableAreaCheckpoint,
    expected_generation: &DurableAreaGeneration,
) -> Result<Vec<String>, AreaRetirementOperationError> {
    if world.area_generation() != Some(&prepared.core.generation_id)
        || expected_generation.active_catalog_sha256 != prepared.core.generation_id.as_str()
        || durable.generation_sha256 != prepared.core.generation_id.as_str()
    {
        return Err(AreaRetirementOperationError::Invalid);
    }
    let checkpoint =
        validate_area_checkpoint(durable).map_err(AreaRetirementOperationError::Persistence)?;
    let area_ids = checkpoint
        .areas
        .iter()
        .map(|area| area.area_id.clone())
        .collect::<Vec<_>>();
    if prepared
        .core
        .areas
        .iter()
        .map(|area| area.area_id.as_str())
        .ne(area_ids.iter().map(String::as_str))
    {
        return Err(AreaRetirementOperationError::Invalid);
    }
    let before_snapshot = world.runtime_snapshot();
    let world_checkpoint = world.lifecycle_checkpoint();
    let runtime_checkpoint = runtime_content.clone();
    let mutated = (|| {
        world
            .rehydrate_retired_areas(
                prepared.core.clone(),
                checkpoint,
                &durable.checkpoint_sha256,
            )
            .map_err(map_core_area_retirement_error)?;
        for area_id in &area_ids {
            let delta = prepared
                .runtime_by_area
                .get(area_id)
                .cloned()
                .ok_or(AreaRetirementOperationError::Invalid)?;
            runtime_content
                .install_delta(delta)
                .map_err(|_| AreaRetirementOperationError::Invalid)?;
        }
        Ok::<(), AreaRetirementOperationError>(())
    })();
    if let Err(error) = mutated {
        world.restore_lifecycle_checkpoint(world_checkpoint);
        *runtime_content = runtime_checkpoint;
        return Err(error);
    }
    let candidate_snapshot = world.runtime_snapshot();
    if let Err(error) = store
        .commit_area_rehydration(
            candidate_snapshot.clone(),
            expected_generation.clone(),
            durable.clone(),
        )
        .await
    {
        if area_commit_error_is_definitive(&error) {
            world.restore_lifecycle_checkpoint(world_checkpoint);
            *runtime_content = runtime_checkpoint;
            return Err(AreaRetirementOperationError::Persistence(error));
        }
        match reconcile_area_commit_after_error(
            store,
            &before_snapshot,
            &candidate_snapshot,
            durable,
            false,
        )
        .await
        {
            AreaCommitReadback::Committed => {
                warn!(%error, "durable area rehydration succeeded but its acknowledgement was lost");
            }
            AreaCommitReadback::NotCommitted => {
                world.restore_lifecycle_checkpoint(world_checkpoint);
                *runtime_content = runtime_checkpoint;
                return Err(AreaRetirementOperationError::Persistence(error));
            }
            AreaCommitReadback::Ambiguous => {
                return Err(AreaRetirementOperationError::DurableAuthorityAmbiguous);
            }
        }
    }
    Ok(area_ids)
}

fn map_core_area_retirement_error(error: CoreAreaRetirementError) -> AreaRetirementOperationError {
    match error {
        CoreAreaRetirementError::Busy(_) => AreaRetirementOperationError::Busy,
        CoreAreaRetirementError::Invalid | CoreAreaRetirementError::Stale => {
            AreaRetirementOperationError::Invalid
        }
    }
}

fn area_commit_error_is_definitive(error: &StoreError) -> bool {
    matches!(
        error,
        StoreError::InvalidAreaCheckpoint
            | StoreError::AreaCheckpointConflict
            | StoreError::InvalidAreaGeneration
            | StoreError::AreaGenerationConflict
            | StoreError::AreaCheckpointUnsupported
            | StoreError::AreaGenerationUnsupported
            | StoreError::SnapshotTooLarge { .. }
    )
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum AreaCommitReadback {
    Committed,
    NotCommitted,
    Ambiguous,
}

async fn reconcile_area_commit_after_error(
    store: &dyn CharacterStore,
    before_snapshot: &WorldRuntimeSnapshot,
    candidate_snapshot: &WorldRuntimeSnapshot,
    expected_checkpoint: &DurableAreaCheckpoint,
    candidate_has_checkpoint: bool,
) -> AreaCommitReadback {
    let recovery = match store.load_startup_world_recovery().await {
        Ok(StartupWorldRecovery::Snapshot { snapshot, .. }) => snapshot,
        Ok(StartupWorldRecovery::Empty | StartupWorldRecovery::Ledger(_)) | Err(_) => {
            return AreaCommitReadback::Ambiguous;
        }
    };
    let checkpoints = match store.load_area_checkpoints().await {
        Ok(checkpoints) => checkpoints,
        Err(_) => return AreaCommitReadback::Ambiguous,
    };
    let matching_slot = checkpoints
        .iter()
        .filter(|checkpoint| {
            checkpoint.generation_sha256 == expected_checkpoint.generation_sha256
                && checkpoint.unit_id == expected_checkpoint.unit_id
        })
        .collect::<Vec<_>>();
    let checkpoint_present = match matching_slot.as_slice() {
        [] => false,
        [checkpoint] if *checkpoint == expected_checkpoint => true,
        _ => return AreaCommitReadback::Ambiguous,
    };
    if recovery == *candidate_snapshot && checkpoint_present == candidate_has_checkpoint {
        AreaCommitReadback::Committed
    } else if recovery == *before_snapshot && checkpoint_present != candidate_has_checkpoint {
        AreaCommitReadback::NotCommitted
    } else {
        AreaCommitReadback::Ambiguous
    }
}

async fn enter_prepared_character(
    world: &mut World,
    store: &dyn CharacterStore,
    session: SessionId,
    snapshot: CharacterSnapshot,
    gameplay: CharacterGameplaySnapshot,
) -> Result<PreparedWorldEntry, GameError> {
    let checkpoint = world.checkpoint();
    if let Err(error) = world.restore_character_gameplay(snapshot.id, gameplay) {
        world.restore_checkpoint(checkpoint);
        if matches!(&error, CoreWorldError::InvalidGameplayDefinition) {
            tracing::warn!(
                character_id = snapshot.id.0,
                "character references incompatible reviewed content"
            );
            return Err(GameError::CharacterContentIncompatible);
        }
        return Err(error.into());
    }
    let scene = match world.connect(session, snapshot.clone()) {
        Ok(scene) => scene,
        Err(error) => {
            world.restore_checkpoint(checkpoint);
            return Err(error.into());
        }
    };

    let resolved = world.character_snapshot(session)?;

    let mut trade_events = match world.cancel_trade_for_character(
        resolved.id,
        TradeCancellationReason::ParticipantDisconnected,
    ) {
        Ok(events) => events,
        Err(error) => {
            world.restore_checkpoint(checkpoint);
            return Err(error.into());
        }
    };
    match reconcile_world_trades(world, store, &checkpoint).await {
        Ok(events) => trade_events.extend(events),
        Err(error) => {
            world.restore_checkpoint(checkpoint);
            return Err(error);
        }
    }
    if !trade_events.is_empty()
        && let Err(error) = store
            .commit_action(
                durable_connected_characters(world)?,
                world.runtime_snapshot(),
                trade_events,
            )
            .await
    {
        world.restore_checkpoint(checkpoint);
        return Err(error.into());
    }

    let player = world.player_state_view(session)?;
    Ok(PreparedWorldEntry {
        session_id: session,
        account_id: resolved.account_id,
        scene,
        player,
    })
}

struct FinishEnterResources<'a> {
    world: &'a mut World,
    runtime_content: &'a RuntimeContent,
    session_roles: &'a mut HashMap<SessionId, AccountRole>,
    session_staff_accesses: &'a mut HashMap<SessionId, StaffAccess>,
    store: &'a dyn CharacterStore,
    delivery: &'a mut DeliveryRouter,
}

async fn finish_enter(
    resources: FinishEnterResources<'_>,
    staff_access: StaffAccess,
    result: Result<PreparedWorldEntry, GameError>,
) -> Result<PreparedWorldEntry, GameError> {
    let FinishEnterResources {
        world,
        runtime_content,
        session_roles,
        session_staff_accesses,
        store,
        delivery,
    } = resources;
    let mut entered = result?;
    let role = derived_role(&staff_access);
    world.set_observer_role(entered.session_id, role)?;
    let checkpoint = world.checkpoint();
    let hook = runtime_content.run_hook(world, entered.session_id, role, ScriptHook::OnEnter);
    let mut committed_events = hook.domain_events.clone();
    match reconcile_world_trades(world, store, &checkpoint).await {
        Ok(events) => committed_events.extend(events),
        Err(error) => {
            world.restore_checkpoint(checkpoint);
            world.disconnect(entered.session_id);
            return Err(error);
        }
    }
    if (hook.changed || !committed_events.is_empty())
        && let Err(error) = store
            .commit_action(
                durable_connected_characters(world)?,
                world.runtime_snapshot(),
                committed_events.clone(),
            )
            .await
    {
        world.restore_checkpoint(checkpoint);
        world.disconnect(entered.session_id);
        return Err(error.into());
    }
    if hook.changed || !committed_events.is_empty() {
        broadcast_world_deliveries(
            world,
            runtime_content,
            session_roles,
            Some(entered.session_id),
            &committed_events,
            delivery,
        );
    }
    entered.scene = world.current_scene(entered.session_id)?;
    runtime_content.resolve_scene(world, entered.session_id, role, &mut entered.scene);
    entered.player = world.player_state_view(entered.session_id)?;
    session_roles.insert(entered.session_id, role);
    session_staff_accesses.insert(entered.session_id, staff_access);
    Ok(entered)
}

struct ApplyResources<'a> {
    world: &'a mut World,
    runtime_content: &'a RuntimeContent,
    social: &'a mut SocialRuntime,
    store: &'a dyn CharacterStore,
    session_roles: &'a mut HashMap<SessionId, AccountRole>,
    session_staff_accesses: &'a mut HashMap<SessionId, StaffAccess>,
    delivery: &'a mut DeliveryRouter,
}

async fn apply(
    resources: ApplyResources<'_>,
    session: SessionId,
    role: AccountRole,
    action: PlayerAction,
) -> Result<AppliedAction, GameError> {
    let ApplyResources {
        world,
        runtime_content,
        social,
        store,
        session_roles,
        session_staff_accesses,
        delivery,
    } = resources;
    if let PlayerAction::Give { recipient, .. } = &action {
        let giver = world.character_snapshot(session)?;
        let recipient = world.resolve_give_recipient(session, recipient)?;
        if !store
            .social_delivery_allowed(giver.id, recipient.character_id)
            .await?
        {
            // Recipient-side ignore and bilateral block deliberately collapse
            // to one inventory error. The action is rejected before any world
            // mutation, revision bump, script hook or durable commit.
            return Err(GameError::GiveUnavailable);
        }
    }
    if let PlayerAction::Trade(trade_action) = &action {
        ensure_trade_social_preflight(world, store, session, trade_action).await?;
    }
    if let PlayerAction::Telepathy {
        ability_id,
        target,
        message,
    } = &action
    {
        let ability_id = normalise_text(ability_id, 64)?;
        let target = normalise_text(target, MAX_SOCIAL_TARGET_BYTES)?;
        let message = normalise_text(message, MAX_CHAT_MESSAGE_BYTES)?;
        let actor = world.character_snapshot(session)?;
        let connected = world.connected_characters()?;
        let recipient = resolve_connected(&connected, &target)?.clone();
        if recipient.character_id == actor.id {
            return Err(StoreError::SocialSelfTarget.into());
        }
        world.telepathy_preflight(session, &ability_id, &target, &message)?;
        ensure_chat_allowed(social, store, actor.id).await?;

        // Receiver-side policy and gameplay receptivity are deliberately
        // collapsed into the same accepted sender result. Their values are
        // used only to decide post-commit delivery, never to decide whether
        // the earned resource and cooldown cost is consumed.
        let socially_allowed = store
            .social_delivery_allowed(actor.id, recipient.character_id)
            .await?;
        let receptive = world.telepathy_receptive(recipient.session_id, &ability_id)?;

        let checkpoint = world.checkpoint();
        let plan = match world.apply_telepathy(session, &ability_id, &target, &message) {
            Ok(plan) => plan,
            Err(error) => {
                world.restore_checkpoint(checkpoint);
                return Err(error.into());
            }
        };
        let hook = runtime_content.run_hook(world, session, role, ScriptHook::OnTalk);
        if hook.rejected {
            world.restore_checkpoint(checkpoint);
            return Err(GameError::ScriptHookRejected);
        }
        let mut committed_events = hook.domain_events.clone();
        match reconcile_world_trades(world, store, &checkpoint).await {
            Ok(events) => committed_events.extend(events),
            Err(error) => {
                world.restore_checkpoint(checkpoint);
                return Err(error);
            }
        }
        let characters = match durable_connected_characters(world) {
            Ok(characters) => characters,
            Err(error) => {
                world.restore_checkpoint(checkpoint);
                return Err(error.into());
            }
        };
        let events = match project_view_events(world, session, &committed_events) {
            Ok(events) => events,
            Err(error) => {
                world.restore_checkpoint(checkpoint);
                return Err(error.into());
            }
        };
        let mut scene = match world.current_scene(session) {
            Ok(scene) => scene,
            Err(error) => {
                world.restore_checkpoint(checkpoint);
                return Err(error.into());
            }
        };
        runtime_content.resolve_scene(world, session, role, &mut scene);
        let presentation = runtime_content.resolve_entity_presentation(world, session);
        let player = match world.player_state_view(session) {
            Ok(player) => player,
            Err(error) => {
                world.restore_checkpoint(checkpoint);
                return Err(error.into());
            }
        };
        if let Err(error) = store
            .commit_action(
                characters,
                world.runtime_snapshot(),
                committed_events.clone(),
            )
            .await
        {
            world.restore_checkpoint(checkpoint);
            return Err(error.into());
        }

        if socially_allowed && receptive {
            // This path intentionally does not update last_tell_senders: a
            // telepathic delivery can never become a plain `reply` target.
            delivery.send_social(
                recipient.session_id,
                SocialMessage {
                    channel: SocialChannel::Telepathy,
                    sender: actor.name,
                    text: message.clone(),
                },
            );
        }
        if hook.changed || !committed_events.is_empty() {
            broadcast_world_deliveries(
                world,
                runtime_content,
                session_roles,
                Some(session),
                &committed_events,
                delivery,
            );
        }
        return Ok(AppliedAction {
            scene,
            presentation,
            player,
            output: combine_output(
                Some(format!(
                    "You reach for {} with {}, \"{message}\".\r\n",
                    recipient.name, plan.ability_name
                )),
                hook.narration,
            ),
            container_contents: None,
            trade_contents: None,
            reward_odds: None,
            mail_page: None,
            mail_message: None,
            board_catalog: None,
            board_page: None,
            board_entry: None,
            events,
        });
    }
    if let PlayerAction::Mail(action) = &action {
        let applied = apply_mail(social, world, store, session, action.clone()).await?;
        let mut scene = world.current_scene(session)?;
        runtime_content.resolve_scene(world, session, role, &mut scene);
        return Ok(AppliedAction {
            scene,
            presentation: runtime_content.resolve_entity_presentation(world, session),
            player: world.player_state_view(session)?,
            output: Some(applied.output),
            container_contents: None,
            trade_contents: None,
            reward_odds: None,
            mail_page: applied.page,
            mail_message: applied.message,
            board_catalog: None,
            board_page: None,
            board_entry: None,
            events: Vec::new(),
        });
    }
    if let PlayerAction::Board(action) = &action {
        let applied = apply_board(social, world, store, session, action.clone()).await?;
        let mut scene = world.current_scene(session)?;
        runtime_content.resolve_scene(world, session, role, &mut scene);
        return Ok(AppliedAction {
            scene,
            presentation: runtime_content.resolve_entity_presentation(world, session),
            player: world.player_state_view(session)?,
            output: Some(applied.output),
            container_contents: None,
            trade_contents: None,
            reward_odds: None,
            mail_page: None,
            mail_message: None,
            board_catalog: applied.catalog,
            board_page: applied.page,
            board_entry: applied.entry,
            events: Vec::new(),
        });
    }
    if let PlayerAction::Move(direction) = &action
        && let Some(request) = world.deferred_area_load_request(session, *direction)?
    {
        return Err(CoreWorldError::AreaLoadRequired(request).into());
    }
    let checkpoint = world.checkpoint();
    let action = match action {
        PlayerAction::Staff(action) => {
            let applied = apply_staff(world, store, session, action).await?;
            if let Some(account_id) = applied.refresh_account {
                refresh_session_authority(
                    world,
                    store,
                    session_roles,
                    session_staff_accesses,
                    account_id,
                )
                .await?;
            }
            if let Some(message) = applied.announcement {
                for recipient in world.connected_sessions() {
                    delivery.send_social(
                        recipient,
                        SocialMessage {
                            channel: SocialChannel::Announcement,
                            sender: "Tiamud".to_owned(),
                            text: message.clone(),
                        },
                    );
                }
            }
            let events = project_view_events(world, session, &applied.domain_events)?;
            let actor_role = session_roles
                .get(&session)
                .copied()
                .unwrap_or(AccountRole::Player);
            let mut scene = world.current_scene(session)?;
            runtime_content.resolve_scene(world, session, actor_role, &mut scene);
            if !applied.domain_events.is_empty() {
                broadcast_world_deliveries(
                    world,
                    runtime_content,
                    session_roles,
                    Some(session),
                    &applied.domain_events,
                    delivery,
                );
            }
            return Ok(AppliedAction {
                scene,
                presentation: runtime_content.resolve_entity_presentation(world, session),
                player: world.player_state_view(session)?,
                output: Some(applied.output),
                container_contents: None,
                trade_contents: None,
                reward_odds: None,
                mail_page: None,
                mail_message: None,
                board_catalog: None,
                board_page: None,
                board_entry: None,
                events,
            });
        }
        action => action,
    };
    if let PlayerAction::Social(action) = action {
        let talks = matches!(
            &action,
            tiamud_core::SocialAction::Say(_)
                | tiamud_core::SocialAction::Yell(_)
                | tiamud_core::SocialAction::Shout(_)
                | tiamud_core::SocialAction::PartySay(_)
                | tiamud_core::SocialAction::Tell { .. }
                | tiamud_core::SocialAction::Reply(_)
                | tiamud_core::SocialAction::ChannelSay { .. }
                | tiamud_core::SocialAction::Emote { .. }
        );
        let actor_access = session_staff_accesses
            .get(&session)
            .cloned()
            .ok_or(CoreWorldError::UnknownSession)?;
        let applied = apply_social(
            social,
            world,
            store,
            session,
            &actor_access,
            session_staff_accesses,
            action,
        )
        .await?;
        let planned_disconnects = applied.disconnects.clone();
        let hook = if talks {
            runtime_content.run_hook(world, session, role, ScriptHook::OnTalk)
        } else {
            crate::runtime_content::ScriptBatchOutcome::default()
        };
        let mut committed_events = hook.domain_events.clone();
        let disconnected_trades = planned_disconnects
            .iter()
            .filter_map(|(_, character_id)| world.trade_for_character(*character_id))
            .map(|trade| trade.id)
            .collect::<BTreeSet<_>>();
        match world.cancel_trades(
            &disconnected_trades,
            TradeCancellationReason::ParticipantDisconnected,
        ) {
            Ok(events) => committed_events.extend(events),
            Err(error) => {
                world.restore_checkpoint(checkpoint);
                disconnect_planned_sessions(
                    world,
                    social,
                    session_roles,
                    session_staff_accesses,
                    delivery,
                    &planned_disconnects,
                );
                return Err(error.into());
            }
        }
        match reconcile_world_trades(world, store, &checkpoint).await {
            Ok(events) => committed_events.extend(events),
            Err(error) => {
                world.restore_checkpoint(checkpoint);
                disconnect_planned_sessions(
                    world,
                    social,
                    session_roles,
                    session_staff_accesses,
                    delivery,
                    &planned_disconnects,
                );
                return Err(error);
            }
        }
        if (hook.changed || !committed_events.is_empty())
            && let Err(error) = store
                .commit_action(
                    durable_connected_characters(world)?,
                    world.runtime_snapshot(),
                    committed_events.clone(),
                )
                .await
        {
            world.restore_checkpoint(checkpoint);
            disconnect_planned_sessions(
                world,
                social,
                session_roles,
                session_staff_accesses,
                delivery,
                &planned_disconnects,
            );
            return Err(error.into());
        }
        for planned in applied.deliveries {
            if delivery.send_social(planned.session_id, planned.message)
                && let Some(sender) = planned.reply_from
            {
                social.record_delivered_tell(planned.recipient_id, sender);
            }
        }
        if let Some(account_id) = applied.authority_update {
            refresh_session_authority(
                world,
                store,
                session_roles,
                session_staff_accesses,
                account_id,
            )
            .await?;
        }
        disconnect_planned_sessions(
            world,
            social,
            session_roles,
            session_staff_accesses,
            delivery,
            &planned_disconnects,
        );
        let mut scene = world.current_scene(session)?;
        let current_role = session_roles
            .get(&session)
            .copied()
            .unwrap_or(AccountRole::Player);
        runtime_content.resolve_scene(world, session, current_role, &mut scene);
        let events = project_view_events(world, session, &committed_events)?;
        if hook.changed || !committed_events.is_empty() {
            broadcast_world_deliveries(
                world,
                runtime_content,
                session_roles,
                Some(session),
                &committed_events,
                delivery,
            );
        }
        return Ok(AppliedAction {
            scene,
            presentation: runtime_content.resolve_entity_presentation(world, session),
            player: world.player_state_view(session)?,
            output: combine_output(Some(applied.outcome.output), hook.narration),
            container_contents: None,
            trade_contents: None,
            reward_odds: None,
            mail_page: None,
            mail_message: None,
            board_catalog: None,
            board_page: None,
            board_entry: None,
            events,
        });
    }

    let mut narration = Vec::new();
    let mut script_changed = false;
    let mut script_domain_events = Vec::new();
    if matches!(action, PlayerAction::Move(_)) {
        merge_hook(
            &mut narration,
            &mut script_changed,
            &mut script_domain_events,
            runtime_content.run_hook(world, session, role, ScriptHook::OnLeave),
        );
    }
    let action_kind = action.clone();
    let revision_before_action = world.revision();
    let outcome = match world.apply(session, action) {
        Ok(outcome) => outcome,
        Err(error) => {
            world.restore_checkpoint(checkpoint);
            return Err(error.into());
        }
    };
    let combat_started = outcome.domain_events.iter().any(|event| {
        matches!(
            event,
            DomainEvent::Gameplay {
                event: GameplayEvent::CombatStarted { .. }
            }
        )
    });
    let defeated = outcome.domain_events.iter().any(|event| {
        matches!(
            event,
            DomainEvent::Gameplay {
                event: GameplayEvent::NpcDefeated { .. } | GameplayEvent::CharacterDefeated { .. }
            }
        )
    });
    let quest_event = outcome.domain_events.iter().any(|event| {
        matches!(
            event,
            DomainEvent::Adventure {
                event: AdventureEvent::QuestChanged { .. }
            }
        )
    });
    let mut hooks = Vec::new();
    match action_kind {
        PlayerAction::Move(_) => hooks.push(ScriptHook::OnEnter),
        PlayerAction::Look => hooks.push(ScriptHook::OnLook),
        PlayerAction::Use(_) => hooks.push(ScriptHook::OnUse),
        _ => {}
    }
    if combat_started {
        hooks.push(ScriptHook::OnCombatStart);
    }
    if defeated {
        hooks.push(ScriptHook::OnDefeat);
    }
    if quest_event {
        hooks.push(ScriptHook::OnQuestEvent);
    }
    for hook in hooks {
        merge_hook(
            &mut narration,
            &mut script_changed,
            &mut script_domain_events,
            runtime_content.run_hook(world, session, role, hook),
        );
    }

    let mut committed_events = outcome.domain_events.clone();
    committed_events.extend(script_domain_events);
    match reconcile_world_trades(world, store, &checkpoint).await {
        Ok(events) => committed_events.extend(events),
        Err(error) => {
            world.restore_checkpoint(checkpoint);
            return Err(error);
        }
    }
    // Runtime-only decisions (for example group-loot declarations) can change
    // durable world state without a character row or a narrative event. Never
    // acknowledge such a mutation before its snapshot has committed.
    let runtime_changed = world.revision() != revision_before_action;
    if (runtime_changed
        || outcome.changed_character.is_some()
        || !committed_events.is_empty()
        || script_changed)
        && let Err(error) = store
            .commit_action(
                durable_connected_characters(world)?,
                world.runtime_snapshot(),
                committed_events.clone(),
            )
            .await
    {
        world.restore_checkpoint(checkpoint);
        return Err(error.into());
    }
    let events = project_view_events(world, session, &committed_events)?;
    let mut scene = world.current_scene(session)?;
    runtime_content.resolve_scene(world, session, role, &mut scene);
    if runtime_changed || !committed_events.is_empty() || script_changed {
        broadcast_world_deliveries(
            world,
            runtime_content,
            session_roles,
            Some(session),
            &committed_events,
            delivery,
        );
    }
    Ok(AppliedAction {
        scene,
        presentation: runtime_content.resolve_entity_presentation(world, session),
        player: world.player_state_view(session)?,
        output: combine_output(outcome.output, narration),
        container_contents: outcome.container_contents,
        trade_contents: outcome.trade_contents,
        reward_odds: outcome.reward_odds,
        mail_page: None,
        mail_message: None,
        board_catalog: None,
        board_page: None,
        board_entry: None,
        events,
    })
}

fn disconnect_planned_sessions(
    world: &mut World,
    social: &mut SocialRuntime,
    session_roles: &mut HashMap<SessionId, AccountRole>,
    session_staff_accesses: &mut HashMap<SessionId, StaffAccess>,
    delivery: &mut DeliveryRouter,
    disconnects: &[(SessionId, CharacterId)],
) {
    for (session, character) in disconnects {
        social.disconnected(*character);
        session_roles.remove(session);
        session_staff_accesses.remove(session);
        world.disconnect(*session);
        delivery.unregister(*session);
    }
}

async fn ensure_trade_social_preflight(
    world: &World,
    store: &dyn CharacterStore,
    session: SessionId,
    action: &TradeAction,
) -> Result<(), GameError> {
    let requires_policy = matches!(
        action,
        TradeAction::Request(_)
            | TradeAction::Accept
            | TradeAction::OfferItem(_)
            | TradeAction::RemoveItem(_)
            | TradeAction::OfferCoins(_)
            | TradeAction::Contents { .. }
            | TradeAction::Confirm { .. }
    );
    if !requires_policy {
        return Ok(());
    }
    let actor = world.character_snapshot(session)?.id;
    let counterpart = match action {
        TradeAction::Request(target) => {
            world
                .resolve_trade_recipient(session, target)
                .map_err(|_| GameError::TradeUnavailable)?
                .character_id
        }
        _ => {
            let Some(trade) = world.trade_for_character(actor) else {
                return Ok(());
            };
            trade
                .counterpart(actor)
                .ok_or(GameError::TradeUnavailable)?
        }
    };
    let forward = store.social_delivery_allowed(actor, counterpart).await?;
    let reverse = store.social_delivery_allowed(counterpart, actor).await?;
    if !forward || !reverse {
        return Err(GameError::TradeUnavailable);
    }
    Ok(())
}

pub(crate) async fn reconcile_world_trades(
    world: &mut World,
    store: &dyn CharacterStore,
    before: &WorldCheckpoint,
) -> Result<Vec<DomainEvent>, GameError> {
    let trades = world
        .trade_participants()
        .into_iter()
        .filter_map(|participant| world.trade_for_character(participant).cloned())
        .map(|trade| (trade.id, trade))
        .collect::<BTreeMap<_, _>>()
        .into_values();
    let mut socially_invalid = BTreeSet::new();
    for trade in trades {
        if world
            .trade_for_character(trade.requester)
            .is_none_or(|current| current.id != trade.id)
        {
            continue;
        }
        let forward = store
            .social_delivery_allowed(trade.requester, trade.recipient)
            .await?;
        let reverse = store
            .social_delivery_allowed(trade.recipient, trade.requester)
            .await?;
        if !forward || !reverse {
            socially_invalid.insert(trade.id);
        }
    }
    let mut events = world.cancel_trades(
        &socially_invalid,
        TradeCancellationReason::SocialPolicyChanged,
    )?;
    events.extend(world.reconcile_trades(before)?);
    Ok(events)
}

async fn refresh_session_authority(
    world: &mut World,
    store: &dyn CharacterStore,
    session_roles: &mut HashMap<SessionId, AccountRole>,
    session_staff_accesses: &mut HashMap<SessionId, StaffAccess>,
    account_id: AccountId,
) -> Result<(), GameError> {
    let connected = world
        .connected_characters()?
        .into_iter()
        .filter(|character| character.account_id == account_id)
        .collect::<Vec<_>>();
    let Some(first) = connected.first() else {
        return Ok(());
    };
    let access = match store.staff_access(first.character_id).await {
        Ok(access) => access,
        Err(error) => {
            tracing::warn!(
                %error,
                account_id = account_id.0,
                "could not refresh staff authority; live sessions were reduced to player access"
            );
            StaffAccess {
                account_id,
                role: AccountRole::Player,
                profile: None,
                lore_title: None,
                capabilities: Default::default(),
            }
        }
    };
    let role = derived_role(&access);
    for character in connected {
        world.set_observer_role(character.session_id, role)?;
        session_roles.insert(character.session_id, role);
        session_staff_accesses.insert(character.session_id, access.clone());
    }
    Ok(())
}

fn current_state(
    world: &World,
    runtime_content: &RuntimeContent,
    session_roles: &HashMap<SessionId, AccountRole>,
    session: SessionId,
) -> Result<(SceneView, ScenePresentation, PlayerStateView), GameError> {
    let role = session_roles
        .get(&session)
        .copied()
        .unwrap_or(AccountRole::Player);
    let mut scene = world.current_scene(session)?;
    runtime_content.resolve_scene(world, session, role, &mut scene);
    Ok((
        scene,
        runtime_content.resolve_entity_presentation(world, session),
        world.player_state_view(session)?,
    ))
}

fn resolve_area_operator(
    world: &World,
    runtime_content: &RuntimeContent,
    session_roles: &HashMap<SessionId, AccountRole>,
    session_staff_accesses: &HashMap<SessionId, StaffAccess>,
    session: SessionId,
    capability: tiamud_core::StaffCapability,
) -> Result<AreaOperatorAuthority, GameError> {
    let actor = world.character_snapshot(session)?;
    let access = session_staff_accesses
        .get(&session)
        .ok_or(StoreError::StaffForbidden)?;
    crate::staff::ensure_capability(access, capability)?;
    let role = session_roles
        .get(&session)
        .copied()
        .unwrap_or(AccountRole::Player);
    let mut scene = world.current_scene(session)?;
    runtime_content.resolve_scene(world, session, role, &mut scene);
    let presentation = runtime_content.resolve_entity_presentation(world, session);
    let player = world.player_state_view(session)?;
    Ok(AreaOperatorAuthority {
        account_id: access.account_id,
        actor_name: actor.name,
        scene,
        presentation,
        player,
    })
}

fn map_area_operation_error(error: AreaRetirementOperationError) -> GameError {
    match error {
        AreaRetirementOperationError::Busy => GameError::AreaOperationBusy,
        AreaRetirementOperationError::Invalid => GameError::AreaOperationInvalid,
        AreaRetirementOperationError::Persistence(error) => GameError::Persistence(error),
        AreaRetirementOperationError::DurableAuthorityAmbiguous
        | AreaRetirementOperationError::ActorStopped => GameError::ActorStopped,
    }
}

fn area_operation_applied(authority: AreaOperatorAuthority, output: &str) -> AppliedAction {
    AppliedAction {
        scene: authority.scene,
        presentation: authority.presentation,
        player: authority.player,
        output: Some(output.to_owned()),
        container_contents: None,
        trade_contents: None,
        reward_odds: None,
        mail_page: None,
        mail_message: None,
        board_catalog: None,
        board_page: None,
        board_entry: None,
        events: Vec::new(),
    }
}

fn project_view_events(
    world: &World,
    session: SessionId,
    domain_events: &[DomainEvent],
) -> Result<Vec<ProjectedViewEvent>, CoreWorldError> {
    let ability_attackers = domain_events
        .iter()
        .filter_map(|event| match event {
            DomainEvent::Gameplay {
                event: GameplayEvent::NpcAbilityUsed { npc_id, .. },
            } => Some(npc_id.as_str()),
            _ => None,
        })
        .collect::<HashSet<_>>();
    domain_events
        .iter()
        .map(|domain_event| {
            world.project_event(session, domain_event).map(|projected| {
                projected.map(|event| ProjectedViewEvent {
                    combat_cue: combat_cue_from_pair(domain_event, &event, &ability_attackers),
                    event,
                })
            })
        })
        .collect::<Result<Vec<_>, _>>()
        .map(|events| events.into_iter().flatten().collect())
}

/// Derives presentation intent exclusively from an authoritative domain fact
/// paired with its successful observer projection. Text is deliberately not
/// inspected and stable domain IDs are deliberately not copied.
fn combat_cue_from_pair(
    domain_event: &DomainEvent,
    projected: &ViewEvent,
    ability_attackers: &HashSet<&str>,
) -> Option<AuthoritativeCombatCue> {
    let (kind, entity_id) = match domain_event {
        DomainEvent::Gameplay {
            event: GameplayEvent::NpcAbilityUsed { .. },
        } => (CombatCueKind::Attack, projected.actor_id.as_str()),
        DomainEvent::Gameplay {
            event: GameplayEvent::NpcDamaged {
                remaining_health, ..
            },
        } if *remaining_health > 0 => (CombatCueKind::Hit, projected.target_id.as_deref()?),
        DomainEvent::Gameplay {
            event: GameplayEvent::NpcDefeated { .. },
        } => (CombatCueKind::Defeat, projected.target_id.as_deref()?),
        DomainEvent::Gameplay {
            event: GameplayEvent::CharacterDamaged { npc_id, .. },
        } if !ability_attackers.contains(npc_id.as_str()) => {
            (CombatCueKind::Attack, projected.actor_id.as_str())
        }
        _ => return None,
    };
    Some(AuthoritativeCombatCue {
        kind,
        entity_id: entity_id.to_owned(),
    })
}

fn broadcast_world_deliveries(
    world: &World,
    runtime_content: &RuntimeContent,
    session_roles: &HashMap<SessionId, AccountRole>,
    excluded_session: Option<SessionId>,
    domain_events: &[DomainEvent],
    delivery: &mut DeliveryRouter,
) {
    let refreshes_state = domain_events.iter().any(is_internal_tick_event);
    for observer in world
        .connected_sessions()
        .into_iter()
        .filter(|observer| Some(*observer) != excluded_session)
    {
        let projected = project_view_events(world, observer, domain_events);
        if !refreshes_state && matches!(&projected, Ok(events) if events.is_empty()) {
            continue;
        }
        let state = current_state(world, runtime_content, session_roles, observer);
        match (projected, state) {
            (Ok(events), Ok((scene, presentation, player))) => {
                delivery.send_world(
                    observer,
                    WorldDelivery {
                        scene,
                        presentation,
                        player,
                        events,
                    },
                );
            }
            (Err(error), _) | (_, Err(GameError::World(error))) => {
                tracing::warn!(%error, ?observer, "could not project world delivery");
            }
            (_, Err(error)) => {
                tracing::warn!(%error, ?observer, "could not build world delivery");
            }
        }
    }
}

fn send_targeted_narration_delivery(
    world: &World,
    runtime_content: &RuntimeContent,
    session_roles: &HashMap<SessionId, AccountRole>,
    session: SessionId,
    narration: Vec<String>,
    delivery: &mut DeliveryRouter,
) {
    let events = narration
        .iter()
        .map(|line| {
            world
                .project_private_narration(session, line)
                .map(ProjectedViewEvent::narrative)
        })
        .collect::<Result<Vec<_>, _>>();
    let state = current_state(world, runtime_content, session_roles, session);
    match (events, state) {
        (Ok(events), Ok((scene, presentation, player))) => {
            delivery.send_world(
                session,
                WorldDelivery {
                    scene,
                    presentation,
                    player,
                    events,
                },
            );
        }
        (Err(error), _) | (_, Err(GameError::World(error))) => {
            tracing::warn!(%error, ?session, "could not project targeted script narration");
        }
        (_, Err(error)) => {
            tracing::warn!(%error, ?session, "could not build targeted script narration");
        }
    }
}

fn merge_hook(
    narration: &mut Vec<String>,
    changed: &mut bool,
    domain_events: &mut Vec<DomainEvent>,
    outcome: crate::runtime_content::ScriptBatchOutcome,
) {
    narration.extend(outcome.narration);
    *changed |= outcome.changed;
    domain_events.extend(outcome.domain_events);
}

fn combine_output(output: Option<String>, narration: Vec<String>) -> Option<String> {
    if narration.is_empty() {
        return output;
    }
    let mut combined = output.unwrap_or_default();
    for line in narration {
        combined.push_str(line.trim());
        combined.push_str("\r\n");
    }
    Some(combined)
}

fn durable_connected_characters(
    world: &World,
) -> Result<Vec<tiamud_core::DurableCharacterSnapshot>, CoreWorldError> {
    world
        .connected_sessions()
        .into_iter()
        .map(|session| world.durable_character_snapshot(session))
        .collect()
}

fn advance_tick_with_expiry_barrier(world: &mut World) -> Result<Vec<DomainEvent>, CoreWorldError> {
    world.begin_tick()?;
    let mut events = world.cancel_expired_trades()?;
    events.extend(world.expire_ground_loot()?);
    events.extend(world.resolve_tick_after_expiry()?);
    Ok(events)
}

async fn persist_tick(
    world: &mut World,
    runtime_content: &RuntimeContent,
    session_roles: &HashMap<SessionId, AccountRole>,
    store: &dyn CharacterStore,
    delivery: &mut DeliveryRouter,
) -> bool {
    persist_tick_with_script_budget(
        world,
        runtime_content,
        session_roles,
        store,
        delivery,
        PERSIST_TICK_SCRIPT_WALL_BUDGET,
    )
    .await
}

async fn persist_tick_with_script_budget(
    world: &mut World,
    runtime_content: &RuntimeContent,
    session_roles: &HashMap<SessionId, AccountRole>,
    store: &dyn CharacterStore,
    delivery: &mut DeliveryRouter,
    script_wall_budget: Duration,
) -> bool {
    let checkpoint = world.checkpoint();
    let scripting_started = Instant::now();
    let mut targeted_narration = Vec::new();
    let mut scripting_budget_exhausted = false;
    let Ok(mut events) = advance_tick_with_expiry_barrier(world) else {
        world.restore_checkpoint(checkpoint);
        return false;
    };
    let result = (|| {
        for session in world.connected_sessions() {
            let role = session_roles
                .get(&session)
                .copied()
                .unwrap_or(AccountRole::Player);
            let mut narration = Vec::new();
            let timer = runtime_content.run_hook_with_budget(
                world,
                session,
                role,
                ScriptHook::OnTimer,
                script_wall_budget.saturating_sub(scripting_started.elapsed()),
            );
            if timer.budget_exhausted {
                scripting_budget_exhausted = true;
                break;
            }
            narration.extend(timer.narration);
            events.extend(timer.domain_events);
            let context = world.authorized_runtime_context(session)?;
            if event_defeats_character(&events, context.character_id) {
                let hook = runtime_content.run_hook_with_budget(
                    world,
                    session,
                    role,
                    ScriptHook::OnDefeat,
                    script_wall_budget.saturating_sub(scripting_started.elapsed()),
                );
                if hook.budget_exhausted {
                    scripting_budget_exhausted = true;
                    break;
                }
                narration.extend(hook.narration);
                events.extend(hook.domain_events);
            }
            if event_changes_character_quest(&events, context.character_id) {
                let hook = runtime_content.run_hook_with_budget(
                    world,
                    session,
                    role,
                    ScriptHook::OnQuestEvent,
                    script_wall_budget.saturating_sub(scripting_started.elapsed()),
                );
                if hook.budget_exhausted {
                    scripting_budget_exhausted = true;
                    break;
                }
                narration.extend(hook.narration);
                events.extend(hook.domain_events);
            }
            if !narration.is_empty() {
                targeted_narration.push((session, narration));
            }
        }
        let characters = durable_connected_characters(world)?;
        Ok::<_, CoreWorldError>((characters, events))
    })();
    if scripting_budget_exhausted {
        world.restore_checkpoint(checkpoint.clone());
        tracing::warn!(
            maximum_milliseconds = script_wall_budget.as_millis(),
            "persist_tick scripting budget exhausted; retrying the base tick without scripts"
        );
        targeted_narration.clear();
        let Ok(mut events) = advance_tick_with_expiry_barrier(world) else {
            world.restore_checkpoint(checkpoint);
            return false;
        };
        match reconcile_world_trades(world, store, &checkpoint).await {
            Ok(trade_events) => events.extend(trade_events),
            Err(_) => {
                world.restore_checkpoint(checkpoint);
                return false;
            }
        }
        let Ok(characters) = durable_connected_characters(world) else {
            world.restore_checkpoint(checkpoint);
            return false;
        };
        let runtime = world.runtime_snapshot();
        let persisted_events = events
            .iter()
            .filter(|event| !is_internal_tick_event(event))
            .cloned()
            .collect();
        if store
            .commit_action(characters, runtime, persisted_events)
            .await
            .is_err()
        {
            world.restore_checkpoint(checkpoint);
            return false;
        }
        broadcast_world_deliveries(
            world,
            runtime_content,
            session_roles,
            None,
            &events,
            delivery,
        );
        return true;
    }
    let Ok((characters, mut events)) = result else {
        world.restore_checkpoint(checkpoint);
        return false;
    };
    match reconcile_world_trades(world, store, &checkpoint).await {
        Ok(trade_events) => events.extend(trade_events),
        Err(_) => {
            world.restore_checkpoint(checkpoint);
            return false;
        }
    }
    let runtime = world.runtime_snapshot();
    let persisted_events = events
        .iter()
        .filter(|event| !is_internal_tick_event(event))
        .cloned()
        .collect();
    if store
        .commit_action(characters, runtime, persisted_events)
        .await
        .is_err()
    {
        world.restore_checkpoint(checkpoint);
        return false;
    }
    broadcast_world_deliveries(
        world,
        runtime_content,
        session_roles,
        None,
        &events,
        delivery,
    );
    for (session, narration) in targeted_narration {
        send_targeted_narration_delivery(
            world,
            runtime_content,
            session_roles,
            session,
            narration,
            delivery,
        );
    }
    true
}

fn is_internal_tick_event(event: &DomainEvent) -> bool {
    matches!(
        event,
        DomainEvent::Gameplay {
            event: GameplayEvent::TickAdvanced { .. }
        }
    )
}

fn event_defeats_character(events: &[DomainEvent], character_id: CharacterId) -> bool {
    events.iter().any(|event| {
        matches!(
            event,
            DomainEvent::Gameplay {
                event: GameplayEvent::NpcDefeated { character_id: actor, .. }
                    | GameplayEvent::CharacterDefeated { character_id: actor, .. }
            } if *actor == character_id
        )
    })
}

fn event_changes_character_quest(events: &[DomainEvent], character_id: CharacterId) -> bool {
    events.iter().any(|event| {
        matches!(
            event,
            DomainEvent::Adventure {
                event: AdventureEvent::QuestChanged { character_id: actor, .. }
            } if *actor == character_id
        )
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::auth::SessionToken;
    use crate::runtime_content::{RuntimeContentDelta, RuntimeScript};
    use async_trait::async_trait;
    use std::{
        collections::{BTreeMap, BTreeSet, HashMap, HashSet},
        sync::{
            Mutex,
            atomic::{AtomicBool, AtomicUsize, Ordering},
        },
    };
    use tiamud_core::{
        AccountRole, ActiveEffect, CharacterGameplaySnapshot, Direction, EconomyAction, EffectKind,
        EquipmentSlot, InitialItemInstanceDefinition, InitialItemLocation, ItemInstanceId,
        ItemPrototypeDefinition, ItemRarity, ItemStackPolicy, ItemTransferPolicy, NpcDefinition,
        PartyAction, QuestAction, RoomDefinition, SocialAction, SocialChannelName,
        SocialEmoteAudience, SocialEmoteDefinition, SocialEmoteTargetedTemplates,
        SocialEmoteTargeting, SocialEmoteTemplates, SocialEmoteUntargetedTemplates,
        WorldDefinition, WorldRuntimeSnapshot,
    };
    use tiamud_scripting::{ScriptCapability, ScriptLimits};

    #[derive(Default)]
    struct MemoryState {
        next_account: i64,
        next_character: i64,
        accounts: HashMap<AccountId, AccountSnapshot>,
        passwords: HashMap<AccountId, String>,
        sessions: HashMap<String, AccountId>,
        characters: HashMap<CharacterId, CharacterSnapshot>,
        gameplay: HashMap<CharacterId, CharacterGameplaySnapshot>,
        runtime: Option<WorldRuntimeSnapshot>,
        domain_events: Vec<tiamud_core::DomainEvent>,
        roles: HashMap<AccountId, tiamud_core::AccountRole>,
        muted: HashSet<AccountId>,
        banned: HashSet<AccountId>,
        ignores: HashSet<(CharacterId, CharacterId)>,
        blocks: HashSet<(CharacterId, CharacterId)>,
        channel_subscriptions: BTreeSet<(CharacterId, SocialChannelName)>,
        mail: Vec<MemoryMail>,
        reported_mail: HashSet<String>,
        boards: Vec<MemoryBoardPost>,
        reported_board_posts: HashSet<String>,
        audit: Vec<crate::persistence::AuditEntry>,
        staff_profiles: HashMap<AccountId, (tiamud_core::StaffProfileKey, Option<String>)>,
        staff_capabilities: HashMap<AccountId, BTreeSet<tiamud_core::StaffCapability>>,
        next_staff_report: i64,
        staff_reports: Vec<crate::persistence::StaffReport>,
        resolved_staff_reports: HashSet<i64>,
        staff_audit: Vec<crate::persistence::StaffAuditEntry>,
    }

    fn additive_area_install_fixture() -> (World, PreparedAreaBatch) {
        let generation =
            tiamud_core::AreaGenerationId::from_catalog_sha256("a".repeat(64)).unwrap();
        let initial = tiamud_core::PreparedCoreAreaBatch {
            generation_id: generation.clone(),
            areas: vec![tiamud_core::PreparedCoreArea {
                area_id: "test:start-area".into(),
                revision: 1,
                bundle_sha256: "b".repeat(64),
                rooms: vec![RoomDefinition {
                    stable_id: "test:start-room".into(),
                    title: "Start".into(),
                    prose: vec!["A stable start room.".into()],
                    image: None,
                    audio: None,
                    exits: Vec::new(),
                    item_ids: Vec::new(),
                }],
                room_presentations: Vec::new(),
                item_prototypes: Vec::new(),
                initial_item_instances: Vec::new(),
                npcs: Vec::new(),
                npc_ability_sets: Vec::new(),
                loot_tables: Vec::new(),
                loot_sources: Vec::new(),
                affixes: Vec::new(),
                affix_pools: Vec::new(),
                room_light: Vec::new(),
                hidden_items: Vec::new(),
                hidden_npcs: Vec::new(),
                secret_exits: Vec::new(),
            }],
            deferred_exits: Vec::new(),
        };
        let (world, _) = World::from_initial_prepared_areas("test:start-room", initial).unwrap();
        let area_id = "test:dynamic-area".to_owned();
        let core = tiamud_core::PreparedCoreAreaBatch {
            generation_id: generation,
            areas: vec![tiamud_core::PreparedCoreArea {
                area_id: area_id.clone(),
                revision: 1,
                bundle_sha256: "c".repeat(64),
                rooms: vec![RoomDefinition {
                    stable_id: "test:dynamic-room".into(),
                    title: "Dynamic".into(),
                    prose: vec!["A dynamically installed room.".into()],
                    image: None,
                    audio: None,
                    exits: Vec::new(),
                    item_ids: Vec::new(),
                }],
                room_presentations: Vec::new(),
                item_prototypes: Vec::new(),
                initial_item_instances: Vec::new(),
                npcs: vec![NpcDefinition {
                    stable_id: "test:dynamic-npc".into(),
                    name: "sentinel".into(),
                    description: "A persistent test sentinel.".into(),
                    room_id: "test:dynamic-room".into(),
                    level: 1,
                    max_health: 10,
                    strength: 1,
                    defence: 0,
                    experience_reward: 0,
                    loot_item_ids: Vec::new(),
                    respawn_ticks: 10,
                }],
                npc_ability_sets: Vec::new(),
                loot_tables: Vec::new(),
                loot_sources: Vec::new(),
                affixes: Vec::new(),
                affix_pools: Vec::new(),
                room_light: Vec::new(),
                hidden_items: Vec::new(),
                hidden_npcs: Vec::new(),
                secret_exits: Vec::new(),
            }],
            deferred_exits: Vec::new(),
        };
        let runtime = RuntimeContentDelta::new(
            Vec::new(),
            Vec::new(),
            vec![RuntimeScript {
                id: "test:dynamic-script".into(),
                room_id: "test:dynamic-room".into(),
                hooks: BTreeSet::new(),
                capabilities: BTreeSet::new(),
                limits: ScriptLimits::default(),
                source: String::new(),
            }],
        );
        (
            world,
            PreparedAreaBatch {
                core,
                runtime_by_area: BTreeMap::from([(area_id, runtime)]),
                bundle_bytes: 1,
                closure_areas: 1,
            },
        )
    }

    #[derive(Clone)]
    struct MemoryMail {
        sender: CharacterId,
        recipient: CharacterId,
        message: crate::persistence::MailMessage,
    }

    #[derive(Clone)]
    struct MemoryBoardPost {
        author: CharacterId,
        public_id: String,
        board: tiamud_core::BoardKey,
        author_name: String,
        subject: String,
        body: String,
        created_at: i64,
        expires_at: i64,
    }

    #[derive(Default)]
    struct MemoryStore {
        state: Mutex<MemoryState>,
        fail_saves: AtomicBool,
        fail_social_reads: AtomicBool,
        successful_commits: AtomicUsize,
        purge_mail_calls: AtomicUsize,
        purge_board_calls: AtomicUsize,
    }

    fn durable_gameplay_world() -> World {
        let mut world = World::from_definition(WorldDefinition {
            start_room_id: "test:arena".into(),
            item_prototypes: vec![
                ItemPrototypeDefinition {
                    prototype_id: "test:sword".into(),
                    name: "bronze sword".into(),
                    description: "A balanced old blade.".into(),
                    aliases: vec!["sword".into()],
                    equipment_slot: Some(EquipmentSlot::Weapon),
                    attack_bonus: 8,
                    heal_amount: 0,
                    consumable: false,
                    effect: None,
                    weight_grams: 2_000,
                    slot_cost: 1,
                    rarity: ItemRarity::Common,
                    item_level: 1,
                    stack_policy: ItemStackPolicy::Unique,
                    max_stack_units: 1,
                    container: None,
                    transfer_policy: ItemTransferPolicy::Transferable,
                    sealed_loot_table_id: None,
                },
                ItemPrototypeDefinition {
                    prototype_id: "test:potion".into(),
                    name: "ember potion".into(),
                    description: "It glows warmly.".into(),
                    aliases: vec!["potion".into()],
                    equipment_slot: None,
                    attack_bonus: 0,
                    heal_amount: 1,
                    consumable: true,
                    effect: Some(ActiveEffect {
                        kind: EffectKind::Strength,
                        magnitude: 2,
                        remaining_ticks: 4,
                    }),
                    weight_grams: 250,
                    slot_cost: 1,
                    rarity: ItemRarity::Uncommon,
                    item_level: 2,
                    stack_policy: ItemStackPolicy::Unique,
                    max_stack_units: 1,
                    container: None,
                    transfer_policy: ItemTransferPolicy::Transferable,
                    sealed_loot_table_id: None,
                },
                ItemPrototypeDefinition {
                    prototype_id: "test:loot".into(),
                    name: "silver fang".into(),
                    description: "A trophy from the guardian.".into(),
                    aliases: vec!["fang".into()],
                    equipment_slot: None,
                    attack_bonus: 0,
                    heal_amount: 0,
                    consumable: false,
                    effect: None,
                    weight_grams: 100,
                    slot_cost: 1,
                    rarity: ItemRarity::Rare,
                    item_level: 2,
                    stack_policy: ItemStackPolicy::Unique,
                    max_stack_units: 1,
                    container: None,
                    transfer_policy: ItemTransferPolicy::Transferable,
                    sealed_loot_table_id: None,
                },
            ],
            initial_item_instances: vec![
                InitialItemInstanceDefinition {
                    instance_id: ItemInstanceId("test:sword".into()),
                    prototype_id: "test:sword".into(),
                    location: InitialItemLocation::Room("test:arena".into()),
                },
                InitialItemInstanceDefinition {
                    instance_id: ItemInstanceId("test:potion".into()),
                    prototype_id: "test:potion".into(),
                    location: InitialItemLocation::Room("test:arena".into()),
                },
                InitialItemInstanceDefinition {
                    instance_id: ItemInstanceId("test:loot".into()),
                    prototype_id: "test:loot".into(),
                    location: InitialItemLocation::NpcSpawn("test:guardian".into()),
                },
            ],
            rooms: vec![RoomDefinition {
                stable_id: "test:arena".into(),
                title: "Arena".into(),
                prose: vec!["Ancient banners stir overhead.".into()],
                image: None,
                audio: None,
                exits: Vec::new(),
                item_ids: Vec::new(),
            }],
        })
        .unwrap();
        world
            .spawn_npc(NpcDefinition {
                stable_id: "test:guardian".into(),
                name: "stone guardian".into(),
                description: "A cracked sentinel.".into(),
                room_id: "test:arena".into(),
                level: 1,
                max_health: 1,
                strength: 1,
                defence: 0,
                experience_reward: 120,
                loot_item_ids: vec![ItemInstanceId("test:loot".into())],
                respawn_ticks: 3,
            })
            .unwrap();
        world
    }

    fn memory_social_profile(
        state: &MemoryState,
        character_id: CharacterId,
    ) -> Result<crate::persistence::SocialProfile, StoreError> {
        let character = state
            .characters
            .get(&character_id)
            .ok_or(StoreError::SocialTargetNotFound)?;
        Ok(crate::persistence::SocialProfile {
            character_id,
            account_id: character.account_id,
            character_name: character.name.clone(),
            role: state
                .roles
                .get(&character.account_id)
                .copied()
                .unwrap_or(tiamud_core::AccountRole::Player),
            muted: state.muted.contains(&character.account_id),
        })
    }

    fn memory_board_entry(
        state: &MemoryState,
        actor: CharacterId,
        post: &MemoryBoardPost,
    ) -> Result<crate::persistence::BoardEntry, StoreError> {
        let actor_profile = memory_social_profile(state, actor)?;
        let author_profile = memory_social_profile(state, post.author)?;
        Ok(crate::persistence::BoardEntry {
            summary: crate::persistence::BoardPostSummary {
                public_id: post.public_id.clone(),
                board: post.board,
                author_name: post.author_name.clone(),
                subject: post.subject.clone(),
                created_at: post.created_at,
                expires_at: post.expires_at,
                can_delete: actor == post.author,
                can_report: actor_profile.account_id != author_profile.account_id
                    && !state.reported_board_posts.contains(&post.public_id),
            },
            body: post.body.clone(),
        })
    }

    fn memory_staff_access_for_account(
        state: &MemoryState,
        account_id: AccountId,
    ) -> Result<crate::persistence::StaffAccess, StoreError> {
        if !state.accounts.contains_key(&account_id) {
            return Err(StoreError::AccountNotFound(account_id));
        }
        let (profile, lore_title) = state
            .staff_profiles
            .get(&account_id)
            .map(|(profile, title)| (Some(*profile), title.clone()))
            .unwrap_or((None, None));
        Ok(crate::persistence::StaffAccess {
            account_id,
            role: state
                .roles
                .get(&account_id)
                .copied()
                .unwrap_or(tiamud_core::AccountRole::Player),
            profile,
            lore_title,
            capabilities: state
                .staff_capabilities
                .get(&account_id)
                .cloned()
                .unwrap_or_default(),
        })
    }

    fn memory_staff_access(
        state: &MemoryState,
        character_id: CharacterId,
    ) -> Result<crate::persistence::StaffAccess, StoreError> {
        let character = state
            .characters
            .get(&character_id)
            .ok_or(StoreError::SocialTargetNotFound)?;
        memory_staff_access_for_account(state, character.account_id)
    }

    fn memory_authorize_staff(
        state: &MemoryState,
        audit: &crate::persistence::StaffAuditRecord,
    ) -> Result<crate::persistence::StaffAccess, StoreError> {
        let actor = memory_staff_access_for_account(state, audit.actor_account_id)?;
        if !actor.capabilities.contains(&audit.capability) {
            return Err(StoreError::StaffForbidden);
        }
        if let Some(target_account) = audit.target_account_id {
            let target = memory_staff_access_for_account(state, target_account)?;
            let actor_rank = actor
                .profile
                .map(tiamud_core::StaffProfileKey::authority_rank)
                .unwrap_or(0);
            let target_rank = target
                .profile
                .map(tiamud_core::StaffProfileKey::authority_rank)
                .unwrap_or(0);
            if actor.account_id == target.account_id || actor_rank <= target_rank {
                return Err(StoreError::StaffHierarchy);
            }
        }
        Ok(actor)
    }

    fn memory_append_staff_audit(
        state: &mut MemoryState,
        audit: crate::persistence::StaffAuditRecord,
    ) -> Result<(), StoreError> {
        memory_authorize_staff(state, &audit)?;
        let reason = audit.reason.trim().to_owned();
        if reason.is_empty()
            || reason.len() > crate::persistence::MAX_SOCIAL_REASON_BYTES
            || reason.chars().any(char::is_control)
        {
            return Err(StoreError::InvalidStaffInput);
        }
        state.staff_audit.push(crate::persistence::StaffAuditEntry {
            sequence: i64::try_from(state.staff_audit.len() + 1).unwrap(),
            actor_account_id: audit.actor_account_id,
            target_account_id: audit.target_account_id,
            capability: audit.capability,
            action: audit.action,
            reason,
            details_json: serde_json::to_string(&audit.details)?,
            created_at: 1,
        });
        Ok(())
    }

    fn memory_audit(
        state: &mut MemoryState,
        actor: AccountId,
        target: Option<AccountId>,
        action: &str,
    ) {
        state.audit.push(crate::persistence::AuditEntry {
            sequence: i64::try_from(state.audit.len() + 1).unwrap(),
            actor_account_id: actor,
            target_account_id: target,
            action: action.to_owned(),
            details_json: "{}".to_owned(),
            created_at: 1,
        });
    }

    #[async_trait]
    impl CharacterStore for MemoryStore {
        async fn register_account(
            &self,
            name: &str,
            password: &str,
        ) -> Result<AuthenticatedSession, StoreError> {
            if !crate::auth::validate_password(password) {
                return Err(StoreError::InvalidPassword);
            }
            let mut state = self.state.lock().unwrap();
            if state
                .accounts
                .values()
                .any(|account| account.name.eq_ignore_ascii_case(name))
            {
                return Err(StoreError::AccountAlreadyExists);
            }
            state.next_account += 1;
            let account = AccountSnapshot {
                id: AccountId(state.next_account),
                name: name.to_owned(),
            };
            let role = if state
                .roles
                .values()
                .all(|role| *role != tiamud_core::AccountRole::Admin)
            {
                tiamud_core::AccountRole::Admin
            } else {
                tiamud_core::AccountRole::Player
            };
            let token = SessionToken::generate();
            state.accounts.insert(account.id, account.clone());
            state.roles.insert(account.id, role);
            if role == tiamud_core::AccountRole::Admin {
                state.staff_profiles.insert(
                    account.id,
                    (tiamud_core::StaffProfileKey::Administrator, None),
                );
                state.staff_capabilities.insert(
                    account.id,
                    tiamud_core::StaffProfileKey::Administrator
                        .default_capabilities()
                        .iter()
                        .copied()
                        .collect(),
                );
            }
            state.passwords.insert(account.id, password.to_owned());
            state.sessions.insert(token.as_str().to_owned(), account.id);
            Ok(AuthenticatedSession {
                account,
                token,
                expires_at: i64::MAX,
            })
        }

        async fn login_account(
            &self,
            name: &str,
            password: &str,
        ) -> Result<AuthenticatedSession, StoreError> {
            let mut state = self.state.lock().unwrap();
            let account = state
                .accounts
                .values()
                .find(|account| account.name.eq_ignore_ascii_case(name))
                .cloned()
                .ok_or(StoreError::InvalidCredentials)?;
            if state.passwords.get(&account.id).map(String::as_str) != Some(password) {
                return Err(StoreError::InvalidCredentials);
            }
            if state.banned.contains(&account.id) {
                return Err(StoreError::AccountBanned);
            }
            let token = SessionToken::generate();
            state.sessions.insert(token.as_str().to_owned(), account.id);
            Ok(AuthenticatedSession {
                account,
                token,
                expires_at: i64::MAX,
            })
        }

        async fn resolve_session(&self, token: &str) -> Result<AccountSnapshot, StoreError> {
            let state = self.state.lock().unwrap();
            let account_id = state
                .sessions
                .get(token)
                .ok_or(StoreError::InvalidSession)?;
            let account = state
                .accounts
                .get(account_id)
                .cloned()
                .ok_or(StoreError::InvalidSession)?;
            if state.banned.contains(account_id) {
                Err(StoreError::AccountBanned)
            } else {
                Ok(account)
            }
        }

        async fn revoke_session(&self, token: &str) -> Result<(), StoreError> {
            if self.state.lock().unwrap().sessions.remove(token).is_some() {
                Ok(())
            } else {
                Err(StoreError::InvalidSession)
            }
        }

        #[cfg(test)]
        async fn load_or_create_account(&self, name: &str) -> Result<AccountSnapshot, StoreError> {
            let mut state = self.state.lock().unwrap();
            if let Some(account) = state
                .accounts
                .values()
                .find(|account| account.name.eq_ignore_ascii_case(name))
                .cloned()
            {
                return if state.passwords.contains_key(&account.id) {
                    Err(StoreError::AccountAlreadyExists)
                } else {
                    Ok(account)
                };
            }
            state.next_account += 1;
            let account = AccountSnapshot {
                id: AccountId(state.next_account),
                name: name.to_owned(),
            };
            state.accounts.insert(account.id, account.clone());
            state
                .roles
                .entry(account.id)
                .or_insert(tiamud_core::AccountRole::Player);
            Ok(account)
        }

        async fn list_characters(
            &self,
            account_id: AccountId,
        ) -> Result<Vec<CharacterSnapshot>, StoreError> {
            let state = self.state.lock().unwrap();
            if !state.accounts.contains_key(&account_id) {
                return Err(StoreError::AccountNotFound(account_id));
            }
            Ok(state
                .characters
                .values()
                .filter(|character| character.account_id == account_id)
                .cloned()
                .collect())
        }

        async fn load_or_create_character(
            &self,
            account_id: AccountId,
            name: &str,
            start_room: &str,
        ) -> Result<CharacterSnapshot, StoreError> {
            let mut state = self.state.lock().unwrap();
            if !state.accounts.contains_key(&account_id) {
                return Err(StoreError::AccountNotFound(account_id));
            }
            if let Some(character) = state.characters.values().find(|character| {
                character.account_id == account_id && character.name.eq_ignore_ascii_case(name)
            }) {
                return Ok(character.clone());
            }
            state.next_character += 1;
            let character = CharacterSnapshot {
                id: CharacterId(state.next_character),
                account_id,
                name: name.to_owned(),
                room_id: start_room.to_owned(),
                item_locations: Default::default(),
            };
            state.characters.insert(character.id, character.clone());
            state.gameplay.insert(character.id, Default::default());
            Ok(character)
        }

        async fn load_or_create_character_with_gameplay(
            &self,
            account_id: AccountId,
            name: &str,
            start_room: &str,
            initial_gameplay: CharacterGameplaySnapshot,
        ) -> Result<CharacterSnapshot, StoreError> {
            let mut state = self.state.lock().unwrap();
            if !state.accounts.contains_key(&account_id) {
                return Err(StoreError::AccountNotFound(account_id));
            }
            if let Some(character) = state.characters.values().find(|character| {
                character.account_id == account_id && character.name.eq_ignore_ascii_case(name)
            }) {
                return Ok(character.clone());
            }
            state.next_character += 1;
            let character = CharacterSnapshot {
                id: CharacterId(state.next_character),
                account_id,
                name: name.to_owned(),
                room_id: start_room.to_owned(),
                item_locations: Default::default(),
            };
            state.characters.insert(character.id, character.clone());
            state.gameplay.insert(character.id, initial_gameplay);
            Ok(character)
        }

        async fn load_character(
            &self,
            account_id: AccountId,
            character_id: CharacterId,
        ) -> Result<CharacterSnapshot, StoreError> {
            self.state
                .lock()
                .unwrap()
                .characters
                .get(&character_id)
                .filter(|character| character.account_id == account_id)
                .cloned()
                .ok_or(StoreError::CharacterNotFound {
                    account_id,
                    character_id,
                })
        }

        async fn save(&self, snapshot: CharacterSnapshot) -> Result<(), StoreError> {
            if self.fail_saves.load(Ordering::SeqCst) {
                return Err(StoreError::WorkerStopped);
            }
            let mut state = self.state.lock().unwrap();
            match state.characters.get(&snapshot.id) {
                Some(stored) if stored.account_id == snapshot.account_id => {
                    state.characters.insert(snapshot.id, snapshot);
                    Ok(())
                }
                _ => Err(StoreError::CharacterNotFound {
                    account_id: snapshot.account_id,
                    character_id: snapshot.id,
                }),
            }
        }

        async fn load_character_gameplay(
            &self,
            character_id: CharacterId,
        ) -> Result<CharacterGameplaySnapshot, StoreError> {
            self.state
                .lock()
                .unwrap()
                .gameplay
                .get(&character_id)
                .cloned()
                .ok_or(StoreError::CharacterGameplayNotFound(character_id))
        }

        async fn ensure_character_ids_exist_batch(
            &self,
            character_ids: Vec<CharacterId>,
        ) -> Result<(), StoreError> {
            let character_ids =
                crate::persistence::normalise_character_identity_ids(character_ids)?;
            let state = self.state.lock().unwrap();
            let found = character_ids
                .iter()
                .copied()
                .filter(|character_id| state.characters.contains_key(character_id))
                .collect::<Vec<_>>();
            crate::persistence::ensure_all_character_ids_exist(&character_ids, &found)
        }

        async fn load_character_gameplay_batch(
            &self,
            character_ids: Vec<CharacterId>,
            limits: crate::persistence::StartupGameplayLoadLimits,
        ) -> Result<Vec<DurableCharacterGameplay>, StoreError> {
            let state = self.state.lock().unwrap();
            let character_ids =
                crate::persistence::normalise_startup_character_ids(character_ids, limits)?;
            let mut total_json_bytes = 0_usize;
            character_ids
                .into_iter()
                .map(|character_id| {
                    if !state.characters.contains_key(&character_id) {
                        return Err(StoreError::CharacterGameplayNotFound(character_id));
                    }
                    let gameplay = state
                        .gameplay
                        .get(&character_id)
                        .ok_or(StoreError::CharacterGameplayNotFound(character_id))?;
                    let json = serde_json::to_string(gameplay)?;
                    crate::persistence::decode_startup_gameplay(
                        character_id,
                        &json,
                        &mut total_json_bytes,
                        limits,
                    )
                })
                .collect()
        }

        async fn load_runtime_snapshot(&self) -> Result<Option<WorldRuntimeSnapshot>, StoreError> {
            Ok(self.state.lock().unwrap().runtime.clone())
        }

        async fn load_startup_world_recovery(
            &self,
        ) -> Result<crate::persistence::StartupWorldRecovery, StoreError> {
            let state = self.state.lock().unwrap();
            Ok(match &state.runtime {
                Some(snapshot) => crate::persistence::StartupWorldRecovery::Snapshot {
                    durable_version: tiamud_core::WORLD_RUNTIME_SNAPSHOT_VERSION,
                    snapshot: snapshot.clone(),
                },
                None if state.domain_events.is_empty() => {
                    crate::persistence::StartupWorldRecovery::Empty
                }
                None => {
                    crate::persistence::StartupWorldRecovery::Ledger(state.domain_events.clone())
                }
            })
        }

        async fn has_durable_character_authority(&self) -> Result<bool, StoreError> {
            Ok(!self.state.lock().unwrap().characters.is_empty())
        }

        async fn load_domain_events(&self) -> Result<Vec<tiamud_core::DomainEvent>, StoreError> {
            Ok(self.state.lock().unwrap().domain_events.clone())
        }

        async fn social_profile(
            &self,
            character_id: CharacterId,
        ) -> Result<crate::persistence::SocialProfile, StoreError> {
            memory_social_profile(&self.state.lock().unwrap(), character_id)
        }

        async fn social_target(
            &self,
            name: &str,
        ) -> Result<crate::persistence::SocialProfile, StoreError> {
            let state = self.state.lock().unwrap();
            let matches = state
                .characters
                .values()
                .filter(|character| character.name.eq_ignore_ascii_case(name.trim()))
                .map(|character| character.id)
                .collect::<Vec<_>>();
            match matches.as_slice() {
                [character_id] => memory_social_profile(&state, *character_id),
                [] => Err(StoreError::SocialTargetNotFound),
                [_, ..] => Err(StoreError::AmbiguousSocialTarget),
            }
        }

        async fn social_delivery_allowed(
            &self,
            sender: CharacterId,
            recipient: CharacterId,
        ) -> Result<bool, StoreError> {
            if self.fail_social_reads.load(Ordering::SeqCst) {
                return Err(StoreError::WorkerStopped);
            }
            let state = self.state.lock().unwrap();
            memory_social_profile(&state, sender)?;
            memory_social_profile(&state, recipient)?;
            Ok(!state.ignores.contains(&(recipient, sender))
                && !state.blocks.contains(&(recipient, sender))
                && !state.blocks.contains(&(sender, recipient)))
        }

        async fn set_social_relation(
            &self,
            actor: CharacterId,
            target: CharacterId,
            relation: crate::persistence::SocialRelationKind,
            enabled: bool,
        ) -> Result<(), StoreError> {
            if actor == target {
                return Err(StoreError::SocialSelfTarget);
            }
            let mut state = self.state.lock().unwrap();
            let actor_profile = memory_social_profile(&state, actor)?;
            let target_profile = memory_social_profile(&state, target)?;
            if actor_profile.account_id == target_profile.account_id {
                return Err(StoreError::SocialSelfTarget);
            }
            let (relations, action) = match relation {
                crate::persistence::SocialRelationKind::Ignore => (
                    &mut state.ignores,
                    if enabled { "ignore" } else { "unignore" },
                ),
                crate::persistence::SocialRelationKind::Block => {
                    (&mut state.blocks, if enabled { "block" } else { "unblock" })
                }
            };
            if enabled {
                relations.insert((actor, target));
            } else {
                relations.remove(&(actor, target));
            }
            memory_audit(
                &mut state,
                actor_profile.account_id,
                Some(target_profile.account_id),
                action,
            );
            Ok(())
        }

        async fn social_channel_subscriptions(
            &self,
            character_id: CharacterId,
        ) -> Result<Vec<SocialChannelName>, StoreError> {
            let state = self.state.lock().unwrap();
            memory_social_profile(&state, character_id)?;
            Ok(state
                .channel_subscriptions
                .iter()
                .filter_map(|(candidate, channel)| (*candidate == character_id).then_some(*channel))
                .collect())
        }

        async fn social_channel_subscribers(
            &self,
            channel: SocialChannelName,
        ) -> Result<Vec<CharacterId>, StoreError> {
            Ok(self
                .state
                .lock()
                .unwrap()
                .channel_subscriptions
                .iter()
                .filter_map(|(character_id, candidate)| {
                    (*candidate == channel).then_some(*character_id)
                })
                .collect())
        }

        async fn set_social_channel_subscription(
            &self,
            character_id: CharacterId,
            channel: SocialChannelName,
            subscribed: bool,
        ) -> Result<(), StoreError> {
            let mut state = self.state.lock().unwrap();
            memory_social_profile(&state, character_id)?;
            if subscribed {
                state.channel_subscriptions.insert((character_id, channel));
            } else {
                state.channel_subscriptions.remove(&(character_id, channel));
            }
            Ok(())
        }

        async fn send_mail(
            &self,
            sender: CharacterId,
            target_name: &str,
            public_id: &str,
            subject: &str,
            body: &str,
        ) -> Result<crate::persistence::MailSendResult, StoreError> {
            let mut state = self.state.lock().unwrap();
            memory_social_profile(&state, sender)?;
            let matches = state
                .characters
                .values()
                .filter(|character| character.name.eq_ignore_ascii_case(target_name))
                .map(|character| character.id)
                .collect::<Vec<_>>();
            let [recipient] = matches.as_slice() else {
                return Ok(crate::persistence::MailSendResult::Unavailable);
            };
            if *recipient == sender {
                return Ok(crate::persistence::MailSendResult::Unavailable);
            }
            if state.ignores.contains(&(*recipient, sender))
                || state.blocks.contains(&(*recipient, sender))
                || state.blocks.contains(&(sender, *recipient))
            {
                return Ok(crate::persistence::MailSendResult::Suppressed);
            }
            if state
                .mail
                .iter()
                .filter(|mail| mail.recipient == *recipient)
                .count()
                >= 64
            {
                return Ok(crate::persistence::MailSendResult::Suppressed);
            }
            let sender_name = state.characters[&sender].name.clone();
            state.mail.push(MemoryMail {
                sender,
                recipient: *recipient,
                message: crate::persistence::MailMessage {
                    summary: crate::persistence::MailMessageSummary {
                        public_id: public_id.to_owned(),
                        sender_name,
                        subject: subject.to_owned(),
                        created_at: 1,
                        expires_at: 2_592_001,
                        read: false,
                    },
                    body: body.to_owned(),
                },
            });
            Ok(crate::persistence::MailSendResult::Stored)
        }

        async fn mailbox_page(
            &self,
            recipient: CharacterId,
            page: u32,
        ) -> Result<crate::persistence::MailPage, StoreError> {
            memory_social_profile(&self.state.lock().unwrap(), recipient)?;
            if !(1..=7).contains(&page) {
                return Err(StoreError::InvalidMailInput);
            }
            let state = self.state.lock().unwrap();
            let messages = state
                .mail
                .iter()
                .rev()
                .filter(|mail| mail.recipient == recipient)
                .collect::<Vec<_>>();
            let total_messages = u32::try_from(messages.len()).unwrap_or(u32::MAX);
            let unread_count = u32::try_from(
                messages
                    .iter()
                    .filter(|mail| !mail.message.summary.read)
                    .count(),
            )
            .unwrap_or(u32::MAX);
            let total_pages = total_messages.div_ceil(10).max(1);
            let offset = usize::try_from((page - 1) * 10).unwrap_or(usize::MAX);
            Ok(crate::persistence::MailPage {
                page,
                total_pages,
                total_messages,
                unread_count,
                capacity: 64,
                messages: messages
                    .into_iter()
                    .skip(offset)
                    .take(10)
                    .map(|mail| mail.message.summary.clone())
                    .collect(),
            })
        }

        async fn read_mail(
            &self,
            recipient: CharacterId,
            public_id: &str,
        ) -> Result<crate::persistence::MailMessage, StoreError> {
            let mut state = self.state.lock().unwrap();
            let mail = state
                .mail
                .iter_mut()
                .find(|mail| {
                    mail.recipient == recipient && mail.message.summary.public_id == public_id
                })
                .ok_or(StoreError::MailNotFound)?;
            mail.message.summary.read = true;
            Ok(mail.message.clone())
        }

        async fn delete_mail(
            &self,
            recipient: CharacterId,
            public_id: &str,
        ) -> Result<(), StoreError> {
            let mut state = self.state.lock().unwrap();
            let before = state.mail.len();
            state.mail.retain(|mail| {
                mail.recipient != recipient || mail.message.summary.public_id != public_id
            });
            (state.mail.len() < before)
                .then_some(())
                .ok_or(StoreError::MailNotFound)
        }

        async fn report_mail(
            &self,
            recipient: CharacterId,
            public_id: &str,
            reason: &str,
        ) -> Result<i64, StoreError> {
            let mut state = self.state.lock().unwrap();
            if state.reported_mail.contains(public_id) {
                return Err(StoreError::MailAlreadyReported);
            }
            let mail = state
                .mail
                .iter()
                .find(|mail| {
                    mail.recipient == recipient && mail.message.summary.public_id == public_id
                })
                .cloned()
                .ok_or(StoreError::MailNotFound)?;
            let reporter = memory_social_profile(&state, recipient)?;
            let target = memory_social_profile(&state, mail.sender)?;
            state.next_staff_report += 1;
            let report_id = state.next_staff_report;
            state.staff_reports.push(crate::persistence::StaffReport {
                id: report_id,
                reporter_character_id: recipient,
                reporter_name: reporter.character_name,
                target_character_id: mail.sender,
                target_account_id: target.account_id,
                target_name: target.character_name,
                reason: reason.to_owned(),
                created_at: 1,
                mail_evidence: Some(crate::persistence::MailReportEvidence {
                    sender_name: mail.message.summary.sender_name,
                    subject: mail.message.summary.subject,
                    body: mail.message.body,
                    sent_at: mail.message.summary.created_at,
                    created_at: 1,
                    expires_at: 2_592_001,
                }),
                board_evidence: None,
            });
            state.reported_mail.insert(public_id.to_owned());
            Ok(report_id)
        }

        async fn purge_expired_mail(&self) -> Result<u64, StoreError> {
            self.purge_mail_calls.fetch_add(1, Ordering::SeqCst);
            Ok(0)
        }

        async fn list_board_posts(
            &self,
            actor: CharacterId,
            board: tiamud_core::BoardKey,
            page: u32,
        ) -> Result<crate::persistence::BoardPage, StoreError> {
            if !(1..=u32::from(tiamud_core::MAX_BOARD_PAGE)).contains(&page) {
                return Err(StoreError::InvalidBoardInput);
            }
            let state = self.state.lock().unwrap();
            memory_social_profile(&state, actor)?;
            let posts = state
                .boards
                .iter()
                .rev()
                .filter(|post| post.board == board)
                .collect::<Vec<_>>();
            let total_posts = u32::try_from(posts.len()).unwrap_or(u32::MAX);
            let total_pages = total_posts.div_ceil(10).max(1);
            let offset = usize::try_from((page - 1).saturating_mul(10)).unwrap_or(usize::MAX);
            let posts = posts
                .into_iter()
                .skip(offset)
                .take(10)
                .map(|post| memory_board_entry(&state, actor, post).map(|entry| entry.summary))
                .collect::<Result<Vec<_>, _>>()?;
            Ok(crate::persistence::BoardPage {
                board,
                page,
                total_pages,
                total_posts,
                capacity: 1_000,
                posts,
            })
        }

        async fn read_board_post(
            &self,
            actor: CharacterId,
            public_id: &str,
        ) -> Result<crate::persistence::BoardEntry, StoreError> {
            let state = self.state.lock().unwrap();
            let post = state
                .boards
                .iter()
                .find(|post| post.public_id == public_id)
                .ok_or(StoreError::BoardPostUnavailable)?;
            memory_board_entry(&state, actor, post)
        }

        async fn publish_board_post(
            &self,
            actor: CharacterId,
            board: tiamud_core::BoardKey,
            public_id: &str,
            subject: &str,
            body: &str,
        ) -> Result<crate::persistence::BoardEntry, StoreError> {
            if !tiamud_core::valid_board_public_id(public_id)
                || !tiamud_core::valid_board_subject(subject)
                || !tiamud_core::valid_board_visible_text(body, tiamud_core::MAX_BOARD_BODY_BYTES)
            {
                return Err(StoreError::InvalidBoardInput);
            }
            let mut state = self.state.lock().unwrap();
            let profile = memory_social_profile(&state, actor)?;
            if state.boards.iter().any(|post| post.public_id == public_id) {
                return Err(StoreError::InvalidBoardInput);
            }
            state.boards.push(MemoryBoardPost {
                author: actor,
                public_id: public_id.to_owned(),
                board,
                author_name: profile.character_name,
                subject: subject.to_owned(),
                body: body.to_owned(),
                created_at: 1,
                expires_at: 7_776_001,
            });
            let post = state.boards.last().expect("the post was just inserted");
            memory_board_entry(&state, actor, post)
        }

        async fn delete_board_post(
            &self,
            actor: CharacterId,
            public_id: &str,
        ) -> Result<(), StoreError> {
            let mut state = self.state.lock().unwrap();
            memory_social_profile(&state, actor)?;
            let before = state.boards.len();
            state
                .boards
                .retain(|post| post.public_id != public_id || post.author != actor);
            (state.boards.len() < before)
                .then_some(())
                .ok_or(StoreError::BoardPostUnavailable)
        }

        async fn report_board_post(
            &self,
            actor: CharacterId,
            public_id: &str,
            reason: &str,
        ) -> Result<i64, StoreError> {
            let mut state = self.state.lock().unwrap();
            if state.reported_board_posts.contains(public_id) {
                return Err(StoreError::BoardAlreadyReported);
            }
            let post = state
                .boards
                .iter()
                .find(|post| post.public_id == public_id)
                .cloned()
                .ok_or(StoreError::BoardPostUnavailable)?;
            let reporter = memory_social_profile(&state, actor)?;
            let author = memory_social_profile(&state, post.author)?;
            if reporter.account_id == author.account_id {
                return Err(StoreError::BoardPostUnavailable);
            }
            state.next_staff_report += 1;
            let report_id = state.next_staff_report;
            state.staff_reports.push(crate::persistence::StaffReport {
                id: report_id,
                reporter_character_id: actor,
                reporter_name: reporter.character_name,
                target_character_id: post.author,
                target_account_id: author.account_id,
                target_name: author.character_name,
                reason: reason.to_owned(),
                created_at: 1,
                mail_evidence: None,
                board_evidence: Some(crate::persistence::BoardReportEvidence {
                    board: post.board,
                    post_public_id: post.public_id.clone(),
                    author_name: post.author_name,
                    subject: post.subject,
                    body: post.body,
                    posted_at: post.created_at,
                    created_at: 1,
                    expires_at: 7_776_001,
                }),
            });
            state.reported_board_posts.insert(post.public_id);
            Ok(report_id)
        }

        async fn purge_expired_boards(&self) -> Result<u64, StoreError> {
            self.purge_board_calls.fetch_add(1, Ordering::SeqCst);
            Ok(0)
        }

        async fn report_character(
            &self,
            actor: CharacterId,
            target: CharacterId,
            reason: &str,
        ) -> Result<(), StoreError> {
            if actor == target || reason.trim().is_empty() {
                return Err(StoreError::InvalidSocialInput);
            }
            let mut state = self.state.lock().unwrap();
            let actor = memory_social_profile(&state, actor)?;
            let target = memory_social_profile(&state, target)?;
            if actor.account_id == target.account_id {
                return Err(StoreError::SocialSelfTarget);
            }
            state.next_staff_report += 1;
            let report_id = state.next_staff_report;
            state.staff_reports.push(crate::persistence::StaffReport {
                id: report_id,
                reporter_character_id: actor.character_id,
                reporter_name: actor.character_name.clone(),
                target_character_id: target.character_id,
                target_account_id: target.account_id,
                target_name: target.character_name.clone(),
                reason: reason.trim().to_owned(),
                created_at: 1,
                mail_evidence: None,
                board_evidence: None,
            });
            memory_audit(
                &mut state,
                actor.account_id,
                Some(target.account_id),
                "report",
            );
            Ok(())
        }

        async fn moderate_character(
            &self,
            actor: CharacterId,
            target: CharacterId,
            action: crate::persistence::ModerationAction,
        ) -> Result<(), StoreError> {
            if actor == target {
                return Err(StoreError::SocialSelfTarget);
            }
            let mut state = self.state.lock().unwrap();
            let actor = memory_social_profile(&state, actor)?;
            let target = memory_social_profile(&state, target)?;
            if actor.account_id == target.account_id {
                return Err(StoreError::SocialSelfTarget);
            }
            let capability = match &action {
                crate::persistence::ModerationAction::Mute { .. }
                | crate::persistence::ModerationAction::Unmute { .. } => {
                    tiamud_core::StaffCapability::ModerationMute
                }
                crate::persistence::ModerationAction::Ban { .. }
                | crate::persistence::ModerationAction::Unban { .. } => {
                    tiamud_core::StaffCapability::ModerationBan
                }
                crate::persistence::ModerationAction::SetRole { .. } => {
                    tiamud_core::StaffCapability::StaffAssign
                }
            };
            let actor_access = memory_staff_access_for_account(&state, actor.account_id)?;
            let target_access = memory_staff_access_for_account(&state, target.account_id)?;
            if !actor_access.capabilities.contains(&capability)
                || actor_access
                    .profile
                    .map(tiamud_core::StaffProfileKey::authority_rank)
                    .unwrap_or(0)
                    <= target_access
                        .profile
                        .map(tiamud_core::StaffProfileKey::authority_rank)
                        .unwrap_or(0)
            {
                return Err(StoreError::SocialForbidden);
            }
            let action_name = match action {
                crate::persistence::ModerationAction::Mute { .. } => {
                    state.muted.insert(target.account_id);
                    "mute"
                }
                crate::persistence::ModerationAction::Unmute { .. } => {
                    state.muted.remove(&target.account_id);
                    "unmute"
                }
                crate::persistence::ModerationAction::Ban { .. } => {
                    state.banned.insert(target.account_id);
                    "ban"
                }
                crate::persistence::ModerationAction::Unban { .. } => {
                    state.banned.remove(&target.account_id);
                    "unban"
                }
                crate::persistence::ModerationAction::SetRole { role, .. } => {
                    state.roles.insert(target.account_id, role);
                    "set_role"
                }
            };
            memory_audit(
                &mut state,
                actor.account_id,
                Some(target.account_id),
                action_name,
            );
            Ok(())
        }

        async fn staff_access(
            &self,
            character_id: CharacterId,
        ) -> Result<crate::persistence::StaffAccess, StoreError> {
            memory_staff_access(&self.state.lock().unwrap(), character_id)
        }

        async fn staff_reports(
            &self,
            audit: crate::persistence::StaffAuditRecord,
        ) -> Result<Vec<crate::persistence::StaffReport>, StoreError> {
            let mut state = self.state.lock().unwrap();
            let actor = memory_authorize_staff(&state, &audit)?;
            let actor_rank = actor
                .profile
                .map(tiamud_core::StaffProfileKey::authority_rank)
                .ok_or(StoreError::StaffForbidden)?;
            memory_append_staff_audit(&mut state, audit)?;
            Ok(state
                .staff_reports
                .iter()
                .filter(|report| !state.resolved_staff_reports.contains(&report.id))
                .filter(|report| {
                    memory_staff_access_for_account(&state, report.target_account_id)
                        .ok()
                        .and_then(|access| {
                            access
                                .profile
                                .map(tiamud_core::StaffProfileKey::authority_rank)
                        })
                        .unwrap_or(0)
                        < actor_rank
                })
                .take(10)
                .cloned()
                .collect())
        }

        async fn resolve_staff_report(
            &self,
            report_id: i64,
            resolution: tiamud_core::StaffReportResolution,
            resolution_reason: &str,
            mut audit: crate::persistence::StaffAuditRecord,
        ) -> Result<(), StoreError> {
            let resolution_reason = resolution_reason.trim();
            if report_id <= 0
                || resolution_reason.is_empty()
                || resolution_reason.len() > crate::persistence::MAX_SOCIAL_REASON_BYTES
                || resolution_reason.chars().any(char::is_control)
            {
                return Err(StoreError::InvalidStaffInput);
            }
            let mut state = self.state.lock().unwrap();
            let report = state
                .staff_reports
                .iter()
                .find(|report| report.id == report_id)
                .cloned()
                .ok_or(StoreError::StaffReportUnavailable(report_id))?;
            let target_account = report.target_account_id;
            if state.resolved_staff_reports.contains(&report_id) {
                return Err(StoreError::StaffReportUnavailable(report_id));
            }
            if audit
                .target_account_id
                .is_some_and(|audited_target| audited_target != target_account)
            {
                return Err(StoreError::StaffForbidden);
            }
            audit.target_account_id = Some(target_account);
            memory_append_staff_audit(&mut state, audit)?;
            if resolution == tiamud_core::StaffReportResolution::ActionTaken
                && let Some(evidence) = report.board_evidence
            {
                state
                    .boards
                    .retain(|post| post.public_id != evidence.post_public_id);
            }
            state.resolved_staff_reports.insert(report_id);
            Ok(())
        }

        async fn set_staff_profile(
            &self,
            target_account: AccountId,
            profile: Option<tiamud_core::StaffProfileKey>,
            lore_title: Option<&str>,
            mut audit: crate::persistence::StaffAuditRecord,
        ) -> Result<(), StoreError> {
            let lore_title = lore_title
                .map(str::trim)
                .filter(|title| !title.is_empty())
                .map(str::to_owned);
            if lore_title
                .as_ref()
                .is_some_and(|title| title.len() > 128 || title.chars().any(char::is_control))
                || (profile.is_none() && lore_title.is_some())
            {
                return Err(StoreError::InvalidStaffInput);
            }
            audit.target_account_id = Some(target_account);
            let mut state = self.state.lock().unwrap();
            let actor = memory_authorize_staff(&state, &audit)?;
            if profile.is_some_and(|profile| {
                profile.authority_rank()
                    >= actor
                        .profile
                        .map(tiamud_core::StaffProfileKey::authority_rank)
                        .unwrap_or(0)
            }) {
                return Err(StoreError::StaffHierarchy);
            }
            if profile.is_some_and(|profile| {
                profile
                    .default_capabilities()
                    .iter()
                    .any(|capability| !actor.capabilities.contains(capability))
            }) {
                return Err(StoreError::StaffForbidden);
            }
            memory_append_staff_audit(&mut state, audit)?;
            if let Some(profile) = profile {
                state
                    .staff_profiles
                    .insert(target_account, (profile, lore_title));
                state.staff_capabilities.insert(
                    target_account,
                    profile.default_capabilities().iter().copied().collect(),
                );
                state.roles.insert(
                    target_account,
                    match profile {
                        tiamud_core::StaffProfileKey::Administrator
                        | tiamud_core::StaffProfileKey::Implementor => AccountRole::Admin,
                        tiamud_core::StaffProfileKey::Moderator => AccountRole::Moderator,
                        tiamud_core::StaffProfileKey::Helper
                        | tiamud_core::StaffProfileKey::Builder => AccountRole::Player,
                    },
                );
            } else {
                state.staff_profiles.remove(&target_account);
                state.staff_capabilities.remove(&target_account);
                state.roles.insert(target_account, AccountRole::Player);
            }
            Ok(())
        }

        async fn set_staff_capability(
            &self,
            target_account: AccountId,
            capability: tiamud_core::StaffCapability,
            enabled: bool,
            mut audit: crate::persistence::StaffAuditRecord,
        ) -> Result<(), StoreError> {
            audit.target_account_id = Some(target_account);
            let mut state = self.state.lock().unwrap();
            let actor = memory_authorize_staff(&state, &audit)?;
            let target = memory_staff_access_for_account(&state, target_account)?;
            if enabled && (target.profile.is_none() || !actor.capabilities.contains(&capability)) {
                return Err(StoreError::StaffForbidden);
            }
            memory_append_staff_audit(&mut state, audit)?;
            let capabilities = state.staff_capabilities.entry(target_account).or_default();
            if enabled {
                capabilities.insert(capability);
            } else {
                capabilities.remove(&capability);
            }
            Ok(())
        }

        async fn staff_audit_entries(
            &self,
            audit: crate::persistence::StaffAuditRecord,
        ) -> Result<Vec<crate::persistence::StaffAuditEntry>, StoreError> {
            let mut state = self.state.lock().unwrap();
            memory_append_staff_audit(&mut state, audit)?;
            Ok(state.staff_audit.iter().rev().take(10).cloned().collect())
        }

        async fn append_staff_audit(
            &self,
            audit: crate::persistence::StaffAuditRecord,
        ) -> Result<(), StoreError> {
            memory_append_staff_audit(&mut self.state.lock().unwrap(), audit)
        }

        async fn audit_log(&self) -> Result<Vec<crate::persistence::AuditEntry>, StoreError> {
            Ok(self.state.lock().unwrap().audit.clone())
        }

        async fn commit_action(
            &self,
            characters: Vec<tiamud_core::DurableCharacterSnapshot>,
            runtime: WorldRuntimeSnapshot,
            events: Vec<tiamud_core::DomainEvent>,
        ) -> Result<(), StoreError> {
            if self.fail_saves.load(Ordering::SeqCst) {
                return Err(StoreError::WorkerStopped);
            }
            let mut state = self.state.lock().unwrap();
            for snapshot in &characters {
                if !matches!(
                    state.characters.get(&snapshot.character.id),
                    Some(stored) if stored.account_id == snapshot.character.account_id
                ) {
                    return Err(StoreError::CharacterNotFound {
                        account_id: snapshot.character.account_id,
                        character_id: snapshot.character.id,
                    });
                }
            }
            for snapshot in characters {
                state
                    .gameplay
                    .insert(snapshot.character.id, snapshot.gameplay);
                state
                    .characters
                    .insert(snapshot.character.id, snapshot.character);
            }
            state.runtime = Some(runtime);
            state.domain_events.extend(events);
            self.successful_commits.fetch_add(1, Ordering::SeqCst);
            Ok(())
        }

        async fn commit_staff_action(
            &self,
            characters: Vec<tiamud_core::DurableCharacterSnapshot>,
            runtime: WorldRuntimeSnapshot,
            events: Vec<tiamud_core::DomainEvent>,
            audit: crate::persistence::StaffAuditRecord,
        ) -> Result<(), StoreError> {
            if self.fail_saves.load(Ordering::SeqCst) {
                return Err(StoreError::WorkerStopped);
            }
            let mut state = self.state.lock().unwrap();
            for snapshot in &characters {
                if !matches!(
                    state.characters.get(&snapshot.character.id),
                    Some(stored) if stored.account_id == snapshot.character.account_id
                ) {
                    return Err(StoreError::CharacterNotFound {
                        account_id: snapshot.character.account_id,
                        character_id: snapshot.character.id,
                    });
                }
            }
            memory_authorize_staff(&state, &audit)?;
            memory_append_staff_audit(&mut state, audit)?;
            for snapshot in characters {
                state
                    .gameplay
                    .insert(snapshot.character.id, snapshot.gameplay);
                state
                    .characters
                    .insert(snapshot.character.id, snapshot.character);
            }
            state.runtime = Some(runtime);
            state.domain_events.extend(events);
            Ok(())
        }
    }

    #[tokio::test]
    async fn failed_dynamic_area_commit_restores_core_and_runtime_content() {
        let store = MemoryStore::default();
        store.fail_saves.store(true, Ordering::SeqCst);
        let (mut world, prepared) = additive_area_install_fixture();
        let mut runtime_content = RuntimeContent::default();
        let before = world.active_area_count();

        let failed =
            install_prepared_areas(&mut world, &mut runtime_content, &store, &prepared).await;

        assert!(matches!(failed, Err(GameError::Persistence(_))));
        assert_eq!(world.active_area_count(), before);
        assert!(!world.area_is_active("test:dynamic-area"));

        store.fail_saves.store(false, Ordering::SeqCst);
        install_prepared_areas(&mut world, &mut runtime_content, &store, &prepared)
            .await
            .unwrap();
        assert!(world.area_is_active("test:dynamic-area"));
        assert_eq!(world.active_area_count(), before + 1);
    }

    #[tokio::test]
    async fn invalid_runtime_snapshot_fails_the_startup_barrier() {
        let store = Arc::new(MemoryStore::default());
        let mut snapshot = World::demo().runtime_snapshot();
        snapshot.snapshot_version += 1;
        store.state.lock().unwrap().runtime = Some(snapshot);

        let result = WorldHandle::spawn(World::demo(), store.clone()).await;

        assert!(matches!(
            result,
            Err(GameError::Startup {
                stage: "restore_runtime_snapshot",
                ..
            })
        ));
        assert_eq!(store.purge_mail_calls.load(Ordering::SeqCst), 0);
        assert_eq!(store.purge_board_calls.load(Ordering::SeqCst), 0);
    }

    #[tokio::test]
    async fn empty_area_bootstrap_confirms_initial_dynamic_state_exactly_once() {
        let store = Arc::new(MemoryStore::default());

        let handle = WorldHandle::spawn_with_scheduler_content_areas_and_recovery(
            World::demo(),
            store.clone(),
            WorldSchedulerConfig::default(),
            RuntimeContent::default(),
            None,
            StartupWorldRecovery::Empty,
            true,
        )
        .await
        .unwrap();

        assert_eq!(store.successful_commits.load(Ordering::SeqCst), 1);
        assert!(store.state.lock().unwrap().runtime.is_some());
        assert_eq!(store.purge_mail_calls.load(Ordering::SeqCst), 1);
        assert_eq!(store.purge_board_calls.load(Ordering::SeqCst), 1);
        handle.shutdown().await.unwrap();
    }

    #[tokio::test]
    async fn failed_initial_area_commit_never_reaches_readiness_or_purges() {
        let store = Arc::new(MemoryStore::default());
        store.fail_saves.store(true, Ordering::SeqCst);

        let result = WorldHandle::spawn_with_scheduler_content_areas_and_recovery(
            World::demo(),
            store.clone(),
            WorldSchedulerConfig::default(),
            RuntimeContent::default(),
            None,
            StartupWorldRecovery::Empty,
            true,
        )
        .await;

        assert!(matches!(
            result,
            Err(GameError::Startup {
                stage: "commit_initial_area_state",
                ..
            })
        ));
        assert_eq!(store.successful_commits.load(Ordering::SeqCst), 0);
        assert!(store.state.lock().unwrap().runtime.is_none());
        assert_eq!(store.purge_mail_calls.load(Ordering::SeqCst), 0);
        assert_eq!(store.purge_board_calls.load(Ordering::SeqCst), 0);
    }

    #[tokio::test]
    async fn startup_loads_trade_participants_and_cancels_reservations_before_readiness() {
        let store = Arc::new(MemoryStore::default());
        let first = WorldHandle::spawn(World::demo(), store.clone())
            .await
            .unwrap();
        let requester = first.enter("Restart Requester".into()).await.unwrap();
        let recipient = first.enter("Restart Recipient".into()).await.unwrap();
        first
            .apply(
                requester.session_id,
                PlayerAction::Trade(tiamud_core::TradeAction::Request(
                    "Restart Recipient".into(),
                )),
            )
            .await
            .unwrap();
        first
            .apply(
                recipient.session_id,
                PlayerAction::Trade(tiamud_core::TradeAction::Accept),
            )
            .await
            .unwrap();
        assert_eq!(
            store
                .state
                .lock()
                .unwrap()
                .runtime
                .as_ref()
                .unwrap()
                .trades
                .len(),
            1
        );
        first.shutdown().await.unwrap();
        drop(first);

        let restarted = WorldHandle::spawn(World::demo(), store.clone())
            .await
            .unwrap();
        {
            let state = store.state.lock().unwrap();
            assert!(state.runtime.as_ref().unwrap().trades.is_empty());
            assert!(state.domain_events.iter().any(|event| matches!(
                event,
                DomainEvent::TradeCancelled {
                    reason: TradeCancellationReason::Restart,
                    ..
                }
            )));
        }
        restarted.shutdown().await.unwrap();
    }

    #[tokio::test]
    async fn startup_validates_loot_identities_and_cancels_open_encounters_before_readiness() {
        let store = Arc::new(MemoryStore::default());
        let first = WorldHandle::spawn(durable_gameplay_world(), store.clone())
            .await
            .unwrap();
        let entered = first.enter("Restart Hunter".into()).await.unwrap();
        first
            .apply(entered.session_id, PlayerAction::Attack("guardian".into()))
            .await
            .unwrap();
        assert_eq!(
            store
                .state
                .lock()
                .unwrap()
                .runtime
                .as_ref()
                .unwrap()
                .loot_encounters
                .len(),
            1
        );
        first.shutdown().await.unwrap();

        let restarted = WorldHandle::spawn(durable_gameplay_world(), store.clone())
            .await
            .unwrap();
        {
            let state = store.state.lock().unwrap();
            assert!(state.runtime.as_ref().unwrap().loot_encounters.is_empty());
            assert!(state.domain_events.iter().any(|event| matches!(
                event,
                DomainEvent::Gameplay {
                    event: GameplayEvent::LootEncounterCancelled { .. }
                }
            )));
        }
        restarted.shutdown().await.unwrap();
    }

    #[tokio::test]
    async fn startup_rejects_a_missing_identity_referenced_only_by_loot() {
        let store = Arc::new(MemoryStore::default());
        let first = WorldHandle::spawn(durable_gameplay_world(), store.clone())
            .await
            .unwrap();
        let entered = first.enter("Missing Hunter".into()).await.unwrap();
        first
            .apply(entered.session_id, PlayerAction::Attack("guardian".into()))
            .await
            .unwrap();
        let character_id = store
            .state
            .lock()
            .unwrap()
            .characters
            .values()
            .find(|character| character.name == "Missing Hunter")
            .unwrap()
            .id;
        first.shutdown().await.unwrap();
        {
            let mut state = store.state.lock().unwrap();
            state.characters.remove(&character_id);
            state.gameplay.remove(&character_id);
        }

        let result = WorldHandle::spawn(durable_gameplay_world(), store).await;
        assert!(matches!(
            result,
            Err(GameError::Startup {
                stage: "validate_loot_identities",
                ..
            })
        ));
    }

    /// Freeze a two-member party encounter on the guardian in `test:arena`,
    /// driven entirely through the shared text-command surface that both Web
    /// (`web.rs` -> `parse_action`) and Telnet (`telnet.rs` -> `parse_action`)
    /// dispatch. Returns the world plus the leader/partner ids and sessions.
    fn group_loot_arena() -> (World, SessionId, CharacterId, SessionId, CharacterId) {
        let store = MemoryStore::default();
        group_loot_arena_with_store(&store)
    }

    fn group_loot_arena_with_store(
        store: &MemoryStore,
    ) -> (World, SessionId, CharacterId, SessionId, CharacterId) {
        let mut world = durable_gameplay_world();
        world
            .configure_loot(
                vec![tiamud_core::LootTableDefinition {
                    id: "test:group-table".into(),
                    digest_sha256: "a".repeat(64),
                    guaranteed: Vec::new(),
                    pools: Vec::new(),
                }],
                vec![tiamud_core::LootSourceDefinition {
                    kind: tiamud_core::LootSourceKind::NpcSpawn,
                    id: "test:group-source".into(),
                    npc_spawn_id: "test:guardian".into(),
                    loot_table_id: "test:group-table".into(),
                    rights: tiamud_core::LootRightsPolicy::PartyRoundRobin,
                }],
            )
            .unwrap();
        let leader = SessionId(71);
        let partner = SessionId(72);
        connect_tick_character(
            &mut world,
            store,
            leader,
            CharacterId(71),
            "test:arena",
            "Loot Leader",
        );
        connect_tick_character(
            &mut world,
            store,
            partner,
            CharacterId(72),
            "test:arena",
            "Loot Partner",
        );
        run_text(&mut world, leader, "party create");
        run_text(&mut world, leader, "party invite Loot Partner");
        run_text(&mut world, partner, "party accept Loot Leader");
        run_text(&mut world, leader, "attack guardian");
        (world, leader, CharacterId(71), partner, CharacterId(72))
    }

    /// Apply a raw text command through the exact `parse_action` path both
    /// transports use, asserting it parses and applies without error.
    fn run_text(world: &mut World, session: SessionId, command: &str) {
        let action = tiamud_core::parse_action(command)
            .unwrap_or_else(|error| panic!("parse `{command}`: {error}"));
        world
            .apply(session, action)
            .unwrap_or_else(|error| panic!("apply `{command}`: {error:?}"));
    }

    fn drive_arena_loot(world: &mut World) {
        let root = ItemInstanceId("test:loot".into());
        for _ in 0..16 {
            world.advance_tick().unwrap();
            if world.runtime_snapshot().ground_loot.contains_key(&root) {
                return;
            }
        }
        panic!("guardian loot was never resolved");
    }

    #[tokio::test]
    async fn group_loot_decisions_commit_before_ack_and_roll_back_on_failure() {
        let store = MemoryStore::default();
        let (mut world, leader, _, partner, _) = group_loot_arena_with_store(&store);
        let content = RuntimeContent::default();
        let mut social = SocialRuntime::new();
        let mut roles = HashMap::new();
        let mut staff = HashMap::new();
        let mut delivery = DeliveryRouter::new(None);
        let mut partner_inbox = delivery.register(partner);
        for (session, command) in [
            (leader, "loot mode need_greed"),
            (partner, "need fang"),
            (leader, "loot mode master_loot"),
            (leader, "loot master fang to Loot Partner"),
        ] {
            let before = world.runtime_snapshot();
            let revision_before = world.revision();
            let commits_before = store.successful_commits.load(Ordering::SeqCst);
            store.fail_saves.store(true, Ordering::SeqCst);
            let failed = apply(
                ApplyResources {
                    world: &mut world,
                    runtime_content: &content,
                    social: &mut social,
                    store: &store,
                    session_roles: &mut roles,
                    session_staff_accesses: &mut staff,
                    delivery: &mut delivery,
                },
                session,
                AccountRole::Player,
                tiamud_core::parse_action(command).unwrap(),
            )
            .await;
            assert!(
                matches!(failed, Err(GameError::Persistence(_))),
                "{command}"
            );
            assert_eq!(world.runtime_snapshot(), before, "{command}");
            assert_eq!(world.revision(), revision_before);
            assert!(partner_inbox.try_recv().is_err());
            store.fail_saves.store(false, Ordering::SeqCst);
            apply(
                ApplyResources {
                    world: &mut world,
                    runtime_content: &content,
                    social: &mut social,
                    store: &store,
                    session_roles: &mut roles,
                    session_staff_accesses: &mut staff,
                    delivery: &mut delivery,
                },
                session,
                AccountRole::Player,
                tiamud_core::parse_action(command).unwrap(),
            )
            .await
            .unwrap();
            assert_eq!(
                store.successful_commits.load(Ordering::SeqCst),
                commits_before + 1
            );
            assert_eq!(
                store.state.lock().unwrap().runtime.as_ref(),
                Some(&world.runtime_snapshot())
            );
            while partner_inbox.try_recv().is_ok() {}
        }
        let commits = store.successful_commits.load(Ordering::SeqCst);
        apply(
            ApplyResources {
                world: &mut world,
                runtime_content: &content,
                social: &mut social,
                store: &store,
                session_roles: &mut roles,
                session_staff_accesses: &mut staff,
                delivery: &mut delivery,
            },
            leader,
            AccountRole::Player,
            tiamud_core::parse_action("loot roll").unwrap(),
        )
        .await
        .unwrap();
        assert_eq!(
            store.successful_commits.load(Ordering::SeqCst),
            commits,
            "inspection stays read-only"
        );
    }

    #[tokio::test]
    async fn need_greed_group_loot_resolves_through_the_shared_command_surface() {
        let (mut world, leader, _leader_id, partner, partner_id) = group_loot_arena();
        // The leader opens need/greed; the partner needs, the leader only greeds.
        run_text(&mut world, leader, "loot mode need_greed");
        run_text(&mut world, partner, "need fang");
        run_text(&mut world, leader, "greed fang");
        // The cohort-private projection is byte-identical whichever transport
        // requested it, and never leaks the internal instance id.
        let roll = tiamud_core::parse_action("loot roll").unwrap();
        let projection = world
            .apply(leader, roll)
            .unwrap()
            .output
            .expect("roll projection");
        assert!(projection.contains("need/greed"));
        assert!(projection.contains("silver fang"));
        assert!(!projection.contains("test:loot"));
        drive_arena_loot(&mut world);
        let root = ItemInstanceId("test:loot".into());
        assert_eq!(
            world.runtime_snapshot().ground_loot[&root].beneficiary,
            Some(partner_id)
        );
    }

    #[tokio::test]
    async fn master_loot_group_loot_resolves_through_the_shared_command_surface() {
        let (mut world, leader, _leader_id, _partner, partner_id) = group_loot_arena();
        run_text(&mut world, leader, "loot mode master_loot");
        run_text(&mut world, leader, "loot master fang to Loot Partner");
        drive_arena_loot(&mut world);
        let root = ItemInstanceId("test:loot".into());
        assert_eq!(
            world.runtime_snapshot().ground_loot[&root].beneficiary,
            Some(partner_id)
        );
    }

    #[tokio::test]
    async fn group_loot_rejects_out_of_cohort_and_repeat_declarations() {
        let (mut world, leader, _leader_id, partner, _partner_id) = group_loot_arena();
        run_text(&mut world, leader, "loot mode need_greed");
        // A repeat declaration by the same member on the same root is refused.
        run_text(&mut world, partner, "need fang");
        let repeat = tiamud_core::parse_action("greed fang").unwrap();
        assert!(world.apply(partner, repeat).is_err());
        // A non-leader cannot switch the distribution mode.
        let hijack = tiamud_core::parse_action("loot mode master_loot").unwrap();
        assert!(world.apply(partner, hijack).is_err());
    }

    #[tokio::test]
    async fn failed_startup_trade_cleanup_never_opens_readiness() {
        let store = Arc::new(MemoryStore::default());
        let first = WorldHandle::spawn(World::demo(), store.clone())
            .await
            .unwrap();
        let requester = first.enter("Failed Restart A".into()).await.unwrap();
        let recipient = first.enter("Failed Restart B".into()).await.unwrap();
        first
            .apply(
                requester.session_id,
                PlayerAction::Trade(tiamud_core::TradeAction::Request("Failed Restart B".into())),
            )
            .await
            .unwrap();
        first
            .apply(
                recipient.session_id,
                PlayerAction::Trade(tiamud_core::TradeAction::Accept),
            )
            .await
            .unwrap();
        first.shutdown().await.unwrap();
        drop(first);

        store.fail_saves.store(true, Ordering::SeqCst);
        let restarted = WorldHandle::spawn(World::demo(), store.clone()).await;
        assert!(matches!(
            restarted,
            Err(GameError::Startup {
                stage: "commit_restored_trade_cleanup",
                ..
            })
        ));
        assert_eq!(
            store
                .state
                .lock()
                .unwrap()
                .runtime
                .as_ref()
                .unwrap()
                .trades
                .len(),
            1
        );
    }

    #[tokio::test]
    async fn startup_rejects_a_missing_offline_trade_participant() {
        let store = Arc::new(MemoryStore::default());
        let first = WorldHandle::spawn(World::demo(), store.clone())
            .await
            .unwrap();
        let requester = first.enter("Missing Trade A".into()).await.unwrap();
        let recipient = first.enter("Missing Trade B".into()).await.unwrap();
        first
            .apply(
                requester.session_id,
                PlayerAction::Trade(tiamud_core::TradeAction::Request("Missing Trade B".into())),
            )
            .await
            .unwrap();
        first
            .apply(
                recipient.session_id,
                PlayerAction::Trade(tiamud_core::TradeAction::Accept),
            )
            .await
            .unwrap();
        first.shutdown().await.unwrap();
        drop(first);
        let recipient_id = store
            .state
            .lock()
            .unwrap()
            .characters
            .values()
            .find(|character| character.name == "Missing Trade B")
            .unwrap()
            .id;
        store.state.lock().unwrap().gameplay.remove(&recipient_id);

        let restarted = WorldHandle::spawn(World::demo(), store).await;
        assert!(matches!(
            restarted,
            Err(GameError::Startup {
                stage: "load_trade_participant_aggregates",
                ..
            })
        ));
    }

    #[tokio::test]
    async fn trade_preflight_is_bilateral_and_generic() {
        let store = Arc::new(MemoryStore::default());
        let world = WorldHandle::spawn(World::demo(), store).await.unwrap();
        let requester = world.enter("Policy Requester".into()).await.unwrap();
        let recipient = world.enter("Policy Recipient".into()).await.unwrap();
        world
            .apply(
                recipient.session_id,
                PlayerAction::Social(SocialAction::Ignore("Policy Requester".into())),
            )
            .await
            .unwrap();

        assert!(matches!(
            world
                .apply(
                    requester.session_id,
                    PlayerAction::Trade(tiamud_core::TradeAction::Request(
                        "Policy Recipient".into(),
                    )),
                )
                .await,
            Err(GameError::TradeUnavailable)
        ));
        assert!(
            world
                .current_state(requester.session_id)
                .await
                .unwrap()
                .2
                .trade
                .is_none()
        );
    }

    #[tokio::test]
    async fn failed_trade_commit_restores_the_exact_checkpoint() {
        let store = Arc::new(MemoryStore::default());
        let world = WorldHandle::spawn(World::demo(), store.clone())
            .await
            .unwrap();
        let requester = world.enter("Rollback Requester".into()).await.unwrap();
        world.enter("Rollback Recipient".into()).await.unwrap();
        store.fail_saves.store(true, Ordering::SeqCst);

        assert!(matches!(
            world
                .apply(
                    requester.session_id,
                    PlayerAction::Trade(tiamud_core::TradeAction::Request(
                        "Rollback Recipient".into(),
                    )),
                )
                .await,
            Err(GameError::Persistence(_))
        ));
        assert!(
            world
                .current_state(requester.session_id)
                .await
                .unwrap()
                .2
                .trade
                .is_none()
        );
        assert!(
            !store
                .state
                .lock()
                .unwrap()
                .domain_events
                .iter()
                .any(|event| { matches!(event, DomainEvent::TradeRequested { .. }) })
        );
    }

    #[tokio::test]
    async fn actor_trade_reconfirms_after_mutation_and_settles_durably_for_both_sessions() {
        let store = Arc::new(MemoryStore::default());
        let world = WorldHandle::spawn(durable_gameplay_world(), store.clone())
            .await
            .unwrap();
        let requester = world.enter("Settlement A".into()).await.unwrap();
        let recipient = world.enter("Settlement B".into()).await.unwrap();
        world
            .apply(
                requester.session_id,
                PlayerAction::Take(tiamud_core::ItemSelection::one("sword")),
            )
            .await
            .unwrap();
        world
            .apply(
                requester.session_id,
                PlayerAction::Trade(tiamud_core::TradeAction::Request("Settlement B".into())),
            )
            .await
            .unwrap();
        world
            .apply(
                recipient.session_id,
                PlayerAction::Trade(tiamud_core::TradeAction::Accept),
            )
            .await
            .unwrap();
        world
            .apply(
                requester.session_id,
                PlayerAction::Trade(tiamud_core::TradeAction::OfferItem(
                    tiamud_core::ItemSelection::one("sword"),
                )),
            )
            .await
            .unwrap();
        let offered_revision = world
            .current_state(requester.session_id)
            .await
            .unwrap()
            .2
            .trade
            .unwrap()
            .state_revision;
        world
            .apply(
                requester.session_id,
                PlayerAction::Trade(tiamud_core::TradeAction::Confirm {
                    state_revision: offered_revision,
                }),
            )
            .await
            .unwrap();

        world
            .apply(
                recipient.session_id,
                PlayerAction::Take(tiamud_core::ItemSelection::one("potion")),
            )
            .await
            .unwrap();
        let invalidated = world
            .current_state(requester.session_id)
            .await
            .unwrap()
            .2
            .trade
            .unwrap();
        assert_eq!(invalidated.state_revision, offered_revision + 1);
        assert!(!invalidated.own_confirmed);
        assert!(!invalidated.counterparty_confirmed);

        for session in [requester.session_id, recipient.session_id] {
            world
                .apply(
                    session,
                    PlayerAction::Trade(tiamud_core::TradeAction::Confirm {
                        state_revision: invalidated.state_revision,
                    }),
                )
                .await
                .unwrap();
        }

        let requester_player = world.current_state(requester.session_id).await.unwrap().2;
        let recipient_player = world.current_state(recipient.session_id).await.unwrap().2;
        assert!(requester_player.trade.is_none());
        assert!(recipient_player.trade.is_none());
        assert!(
            requester_player.inventory.iter().all(
                |item| item.reserved_for_trade == 0 && item.available_quantity == item.quantity
            )
        );
        assert!(
            recipient_player.inventory.iter().all(
                |item| item.reserved_for_trade == 0 && item.available_quantity == item.quantity
            )
        );
        assert!(
            !requester_player
                .inventory
                .iter()
                .any(|item| item.label == "bronze sword")
        );
        assert!(
            recipient_player
                .inventory
                .iter()
                .any(|item| item.label == "bronze sword")
        );
        let state = store.state.lock().unwrap();
        assert!(state.runtime.as_ref().unwrap().trades.is_empty());
        assert!(
            state
                .domain_events
                .iter()
                .any(|event| matches!(event, DomainEvent::TradeCompleted { .. }))
        );
    }

    #[tokio::test]
    async fn failed_leave_cleanup_keeps_the_reservation_and_delivers_nothing() {
        let store = Arc::new(MemoryStore::default());
        let world = WorldHandle::spawn(World::demo(), store.clone())
            .await
            .unwrap();
        let mut requester = world.enter("Leave Failure A".into()).await.unwrap();
        let mut recipient = world.enter("Leave Failure B".into()).await.unwrap();
        world
            .apply(
                requester.session_id,
                PlayerAction::Trade(tiamud_core::TradeAction::Request("Leave Failure B".into())),
            )
            .await
            .unwrap();
        world
            .apply(
                recipient.session_id,
                PlayerAction::Trade(tiamud_core::TradeAction::Accept),
            )
            .await
            .unwrap();
        drain_deliveries(&mut requester.deliveries);
        drain_deliveries(&mut recipient.deliveries);
        store.fail_saves.store(true, Ordering::SeqCst);

        world.leave(requester.session_id).await;
        let recipient_state = world.current_state(recipient.session_id).await.unwrap();

        assert!(recipient_state.2.trade.is_some());
        assert!(recipient.deliveries.try_recv().is_err());
        assert_eq!(
            store
                .state
                .lock()
                .unwrap()
                .runtime
                .as_ref()
                .unwrap()
                .trades
                .len(),
            1
        );
    }

    #[tokio::test]
    async fn social_policy_change_cancels_an_existing_trade_in_the_same_world_commit() {
        let store = Arc::new(MemoryStore::default());
        let world = WorldHandle::spawn(World::demo(), store.clone())
            .await
            .unwrap();
        let requester = world.enter("Policy Change A".into()).await.unwrap();
        let recipient = world.enter("Policy Change B".into()).await.unwrap();
        world
            .apply(
                requester.session_id,
                PlayerAction::Trade(tiamud_core::TradeAction::Request("Policy Change B".into())),
            )
            .await
            .unwrap();
        world
            .apply(
                recipient.session_id,
                PlayerAction::Trade(tiamud_core::TradeAction::Accept),
            )
            .await
            .unwrap();

        world
            .apply(
                recipient.session_id,
                PlayerAction::Social(SocialAction::Ignore("Policy Change A".into())),
            )
            .await
            .unwrap();

        for session in [requester.session_id, recipient.session_id] {
            assert!(
                world
                    .current_state(session)
                    .await
                    .unwrap()
                    .2
                    .trade
                    .is_none()
            );
        }
        let state = store.state.lock().unwrap();
        assert!(state.runtime.as_ref().unwrap().trades.is_empty());
        assert!(state.domain_events.iter().any(|event| matches!(
            event,
            DomainEvent::TradeCancelled {
                reason: TradeCancellationReason::SocialPolicyChanged,
                ..
            }
        )));
    }

    #[tokio::test]
    async fn failed_social_cleanup_is_fail_closed_and_retried() {
        let store = Arc::new(MemoryStore::default());
        let world = WorldHandle::spawn(World::demo(), store.clone())
            .await
            .unwrap();
        let requester = world.enter("Policy Retry A".into()).await.unwrap();
        let recipient = world.enter("Policy Retry B".into()).await.unwrap();
        world
            .apply(
                requester.session_id,
                PlayerAction::Trade(tiamud_core::TradeAction::Request("Policy Retry B".into())),
            )
            .await
            .unwrap();
        world
            .apply(
                recipient.session_id,
                PlayerAction::Trade(tiamud_core::TradeAction::Accept),
            )
            .await
            .unwrap();
        store.fail_saves.store(true, Ordering::SeqCst);
        assert!(matches!(
            world
                .apply(
                    recipient.session_id,
                    PlayerAction::Social(SocialAction::Ignore("Policy Retry A".into())),
                )
                .await,
            Err(GameError::Persistence(_))
        ));
        assert!(
            world
                .current_state(requester.session_id)
                .await
                .unwrap()
                .2
                .trade
                .is_some()
        );
        assert!(matches!(
            world
                .apply(
                    requester.session_id,
                    PlayerAction::Trade(tiamud_core::TradeAction::OfferCoins(1)),
                )
                .await,
            Err(GameError::TradeUnavailable)
        ));

        store.fail_saves.store(false, Ordering::SeqCst);
        world
            .apply(requester.session_id, PlayerAction::Look)
            .await
            .unwrap();
        assert!(
            world
                .current_state(requester.session_id)
                .await
                .unwrap()
                .2
                .trade
                .is_none()
        );
    }

    #[tokio::test]
    async fn failed_social_policy_read_preserves_the_trade_and_retries_cleanup() {
        let store = Arc::new(MemoryStore::default());
        let world = WorldHandle::spawn(World::demo(), store.clone())
            .await
            .unwrap();
        let requester = world.enter("Policy Store A".into()).await.unwrap();
        let recipient = world.enter("Policy Store B".into()).await.unwrap();
        world
            .apply(
                requester.session_id,
                PlayerAction::Trade(tiamud_core::TradeAction::Request("Policy Store B".into())),
            )
            .await
            .unwrap();
        world
            .apply(
                recipient.session_id,
                PlayerAction::Trade(tiamud_core::TradeAction::Accept),
            )
            .await
            .unwrap();
        store.fail_social_reads.store(true, Ordering::SeqCst);

        assert!(matches!(
            world
                .apply(
                    recipient.session_id,
                    PlayerAction::Social(SocialAction::Ignore("Policy Store A".into())),
                )
                .await,
            Err(GameError::Persistence(StoreError::WorkerStopped))
        ));
        assert!(
            world
                .current_state(requester.session_id)
                .await
                .unwrap()
                .2
                .trade
                .is_some()
        );

        store.fail_social_reads.store(false, Ordering::SeqCst);
        world
            .apply(requester.session_id, PlayerAction::Look)
            .await
            .unwrap();
        assert!(
            world
                .current_state(requester.session_id)
                .await
                .unwrap()
                .2
                .trade
                .is_none()
        );
        assert!(matches!(
            store.state.lock().unwrap().domain_events.last(),
            Some(DomainEvent::TradeCancelled {
                reason: TradeCancellationReason::SocialPolicyChanged,
                ..
            })
        ));
    }

    fn seed_offline_character(
        store: &MemoryStore,
        character_id: CharacterId,
        gameplay: CharacterGameplaySnapshot,
    ) {
        let account_id = AccountId(character_id.0);
        let mut state = store.state.lock().unwrap();
        state.accounts.insert(
            account_id,
            AccountSnapshot {
                id: account_id,
                name: format!("offline-owner-{}", character_id.0),
            },
        );
        state.characters.insert(
            character_id,
            CharacterSnapshot {
                id: character_id,
                account_id,
                name: format!("Offline Owner {}", character_id.0),
                room_id: "test:arena".into(),
                item_locations: Default::default(),
            },
        );
        state.gameplay.insert(character_id, gameplay);
    }

    #[tokio::test]
    async fn startup_rejects_item_owned_by_missing_durable_character() {
        let store = Arc::new(MemoryStore::default());
        let mut runtime = durable_gameplay_world().runtime_snapshot();
        runtime
            .item_instances
            .get_mut(&ItemInstanceId("test:sword".into()))
            .unwrap()
            .location = tiamud_core::RuntimeItemLocation::Carried {
            character_id: CharacterId(404),
        };
        store.state.lock().unwrap().runtime = Some(runtime);

        let result = WorldHandle::spawn(durable_gameplay_world(), store).await;

        assert!(matches!(
            result,
            Err(GameError::Startup {
                stage: "load_character_aggregates",
                ..
            })
        ));
    }

    #[tokio::test]
    async fn startup_rejects_offline_equipment_mismatch() {
        let store = Arc::new(MemoryStore::default());
        let character_id = CharacterId(7);
        seed_offline_character(&store, character_id, CharacterGameplaySnapshot::default());
        let mut runtime = durable_gameplay_world().runtime_snapshot();
        runtime
            .item_instances
            .get_mut(&ItemInstanceId("test:sword".into()))
            .unwrap()
            .location = tiamud_core::RuntimeItemLocation::Equipped {
            character_id,
            slot: EquipmentSlot::Weapon,
        };
        store.state.lock().unwrap().runtime = Some(runtime);

        let result = WorldHandle::spawn(durable_gameplay_world(), store).await;

        assert!(matches!(
            result,
            Err(GameError::Startup {
                stage: "validate_character_aggregates",
                ..
            })
        ));
    }

    #[tokio::test]
    async fn startup_rejects_offline_inventory_over_capacity() {
        let store = Arc::new(MemoryStore::default());
        let character_id = CharacterId(8);
        let mut gameplay = CharacterGameplaySnapshot::default();
        gameplay.inventory_capacity.max_weight_grams = 1_000;
        seed_offline_character(&store, character_id, gameplay);
        let mut runtime = durable_gameplay_world().runtime_snapshot();
        runtime
            .item_instances
            .get_mut(&ItemInstanceId("test:sword".into()))
            .unwrap()
            .location = tiamud_core::RuntimeItemLocation::Carried { character_id };
        store.state.lock().unwrap().runtime = Some(runtime);

        let result = WorldHandle::spawn(durable_gameplay_world(), store).await;

        assert!(matches!(
            result,
            Err(GameError::Startup {
                stage: "validate_character_aggregates",
                ..
            })
        ));
    }

    #[tokio::test]
    async fn startup_counts_contained_descendants_against_an_offline_owner() {
        let store = Arc::new(MemoryStore::default());
        let character_id = CharacterId(81);
        let loaded = crate::world_loader::default_loaded_world().unwrap();
        let mut world = crate::world_loader::build_loaded_world(&loaded).unwrap();
        let mut gameplay = world
            .initial_gameplay(None, BackgroundSelection::Default, None)
            .unwrap();
        gameplay.inventory_capacity.max_weight_grams = 500;
        seed_offline_character(&store, character_id, gameplay);
        let mut runtime = world.runtime_snapshot();
        runtime
            .item_instances
            .get_mut(&ItemInstanceId("tiamud:archive-satchel".into()))
            .unwrap()
            .location = tiamud_core::RuntimeItemLocation::Carried { character_id };
        world.restore_runtime_snapshot(runtime.clone()).unwrap();
        assert_eq!(world.runtime_item_owner_ids(), [character_id].into());
        store.state.lock().unwrap().runtime = Some(runtime);

        let result = WorldHandle::spawn(
            crate::world_loader::build_loaded_world(&loaded).unwrap(),
            store,
        )
        .await;

        assert!(matches!(
            result,
            Err(GameError::Startup {
                stage: "validate_character_aggregates",
                ..
            })
        ));
    }

    #[tokio::test]
    async fn startup_accepts_valid_offline_equipment_owner() {
        let store = Arc::new(MemoryStore::default());
        let character_id = CharacterId(9);
        let mut gameplay = CharacterGameplaySnapshot::default();
        gameplay
            .equipment
            .insert(EquipmentSlot::Weapon, ItemInstanceId("test:sword".into()));
        seed_offline_character(&store, character_id, gameplay);
        // A durable account unrelated to the runtime owner graph is not read
        // or retained during startup, even if its gameplay row is unavailable.
        store.state.lock().unwrap().characters.insert(
            CharacterId(99),
            CharacterSnapshot {
                id: CharacterId(99),
                account_id: AccountId(99),
                name: "Unreferenced".into(),
                room_id: "test:arena".into(),
                item_locations: Default::default(),
            },
        );
        let mut runtime = durable_gameplay_world().runtime_snapshot();
        runtime
            .item_instances
            .get_mut(&ItemInstanceId("test:sword".into()))
            .unwrap()
            .location = tiamud_core::RuntimeItemLocation::Equipped {
            character_id,
            slot: EquipmentSlot::Weapon,
        };
        store.state.lock().unwrap().runtime = Some(runtime);

        let world = WorldHandle::spawn(durable_gameplay_world(), store)
            .await
            .unwrap();

        world.shutdown().await.unwrap();
    }

    #[test]
    fn startup_validation_does_not_install_offline_gameplay() {
        let character_id = CharacterId(10);
        let mut gameplay = CharacterGameplaySnapshot {
            coins: 77,
            ..CharacterGameplaySnapshot::default()
        };
        gameplay
            .equipment
            .insert(EquipmentSlot::Weapon, ItemInstanceId("test:sword".into()));
        let mut world = durable_gameplay_world();
        let mut runtime = world.runtime_snapshot();
        runtime
            .item_instances
            .get_mut(&ItemInstanceId("test:sword".into()))
            .unwrap()
            .location = tiamud_core::RuntimeItemLocation::Equipped {
            character_id,
            slot: EquipmentSlot::Weapon,
        };
        world.restore_runtime_snapshot(runtime).unwrap();

        validate_startup_character_aggregates(
            &world,
            &BTreeSet::from([character_id]),
            &[DurableCharacterGameplay {
                character_id,
                gameplay,
            }],
        )
        .unwrap();
        world
            .connect(
                SessionId(10),
                CharacterSnapshot {
                    id: character_id,
                    account_id: AccountId(10),
                    name: "Offline Owner 10".into(),
                    room_id: "test:arena".into(),
                    item_locations: Default::default(),
                },
            )
            .unwrap();

        let connected = world.character_gameplay_snapshot(SessionId(10)).unwrap();
        assert_eq!(connected, CharacterGameplaySnapshot::default());
    }

    #[tokio::test]
    async fn persistence_failure_rolls_back_a_critical_move() {
        let store = Arc::new(MemoryStore::default());
        let world = WorldHandle::spawn(World::demo(), store.clone())
            .await
            .unwrap();
        let entered = world.enter("Rollback Tester".into()).await.unwrap();
        let accepted_revision = entered.scene.revision;
        let accepted_view_id = entered.scene.view_id.clone();
        store.fail_saves.store(true, Ordering::SeqCst);

        let failed = world
            .apply(entered.session_id, PlayerAction::Move(Direction::North))
            .await;
        assert!(matches!(failed, Err(GameError::Persistence(_))));

        let after = world
            .apply(entered.session_id, PlayerAction::Look)
            .await
            .unwrap();
        assert_eq!(after.scene.view_id, accepted_view_id);
        assert_eq!(after.scene.title, "Lantern Square");
        assert_eq!(after.scene.revision, accepted_revision);
    }

    #[tokio::test]
    async fn persistence_failure_rolls_back_a_shared_item_transition() {
        let store = Arc::new(MemoryStore::default());
        let world = WorldHandle::spawn(World::demo(), store.clone())
            .await
            .unwrap();
        let entered = world.enter("Item Rollback Tester".into()).await.unwrap();
        assert_eq!(entered.scene.items.len(), 1);
        store.fail_saves.store(true, Ordering::SeqCst);

        let failed = world
            .apply(
                entered.session_id,
                PlayerAction::Take(tiamud_core::ItemSelection::one("blue-glass lantern")),
            )
            .await;
        assert!(matches!(failed, Err(GameError::Persistence(_))));

        store.fail_saves.store(false, Ordering::SeqCst);
        let after = world
            .apply(entered.session_id, PlayerAction::Look)
            .await
            .unwrap();
        assert_eq!(after.scene.items.len(), 1);
        assert_eq!(after.scene.items[0].label, "blue-glass lantern");
        assert!(store.load_domain_events().await.unwrap().is_empty());
    }

    #[tokio::test]
    async fn committed_domain_events_fan_out_as_observer_specific_snapshots() {
        let store = Arc::new(MemoryStore::default());
        let world = WorldHandle::spawn(World::demo(), store).await.unwrap();
        let actor = world.enter("Actor".into()).await.unwrap();
        let mut observer = world.enter("Observer".into()).await.unwrap();

        world
            .apply(
                actor.session_id,
                PlayerAction::Take(tiamud_core::ItemSelection::one("blue-glass lantern")),
            )
            .await
            .unwrap();
        let delivery = tokio::time::timeout(Duration::from_secs(1), observer.deliveries.recv())
            .await
            .unwrap()
            .unwrap();
        let crate::delivery::SessionDelivery::World(delivery) = delivery else {
            panic!("world delivery expected");
        };

        assert!(delivery.scene.items.is_empty());
        assert!(delivery.player.inventory.is_empty());
        assert_eq!(delivery.events.len(), 1);
        assert_eq!(
            delivery.events[0].event.kind,
            tiamud_core::ViewEventKind::ItemTaken
        );
        assert!(!delivery.events[0].event.actor_id.contains("Actor"));
        assert!(!delivery.events[0].event.actor_id.contains("demo:"));
    }

    #[tokio::test]
    async fn committed_give_refreshes_both_owners_and_only_room_observers() {
        let store = Arc::new(MemoryStore::default());
        let world = WorldHandle::spawn(World::demo(), store.clone())
            .await
            .unwrap();
        let mut remote = world.enter("Give Remote".into()).await.unwrap();
        world
            .apply(remote.session_id, PlayerAction::Move(Direction::North))
            .await
            .unwrap();
        let sender = world.enter("Give Sender".into()).await.unwrap();
        let mut recipient = world.enter("Give Recipient".into()).await.unwrap();
        let mut observer = world.enter("Give Observer".into()).await.unwrap();

        let taken = world
            .apply(
                sender.session_id,
                PlayerAction::Take(tiamud_core::ItemSelection::one("blue-glass lantern")),
            )
            .await
            .unwrap();
        // Drain the pickup broadcast so the next messages must be caused by
        // the committed give.
        recipient.deliveries.recv().await.unwrap();
        observer.deliveries.recv().await.unwrap();
        let item = taken.player.inventory[0].id.clone();
        let recipient_selector = taken
            .player
            .social
            .online
            .iter()
            .find(|character| character.name == "Give Recipient")
            .unwrap()
            .id
            .clone();

        let given = world
            .apply(
                sender.session_id,
                PlayerAction::Give {
                    selection: tiamud_core::ItemSelection::one(item),
                    recipient: recipient_selector,
                },
            )
            .await
            .unwrap();
        assert!(given.player.inventory.is_empty());
        assert_eq!(
            given.events[0].event.kind,
            tiamud_core::ViewEventKind::ItemGiven
        );

        let recipient_delivery = recipient.deliveries.recv().await.unwrap();
        let crate::delivery::SessionDelivery::World(recipient_delivery) = recipient_delivery else {
            panic!("recipient world delivery expected");
        };
        assert_eq!(recipient_delivery.player.inventory.len(), 1);
        assert_eq!(
            recipient_delivery.events[0].event.kind,
            tiamud_core::ViewEventKind::ItemGiven
        );
        assert!(recipient_delivery.events[0].event.item_id.is_some());
        assert!(
            !recipient_delivery.player.inventory[0]
                .id
                .contains("demo:blue-lantern")
        );

        let observer_delivery = observer.deliveries.recv().await.unwrap();
        let crate::delivery::SessionDelivery::World(observer_delivery) = observer_delivery else {
            panic!("observer world delivery expected");
        };
        assert!(observer_delivery.player.inventory.is_empty());
        assert_eq!(
            observer_delivery.events[0].event.kind,
            tiamud_core::ViewEventKind::ItemGiven
        );
        assert!(observer_delivery.events[0].event.item_id.is_none());
        assert!(
            tokio::time::timeout(Duration::from_millis(20), remote.deliveries.recv())
                .await
                .is_err()
        );
        assert!(matches!(
            store.load_domain_events().await.unwrap().last(),
            Some(DomainEvent::ItemsGiven { .. })
        ));
    }

    #[tokio::test]
    async fn give_policy_and_failed_commit_publish_nothing_and_restore_both_owners() {
        let store = Arc::new(MemoryStore::default());
        let world = WorldHandle::spawn(World::demo(), store.clone())
            .await
            .unwrap();
        let sender = world.enter("Guarded Sender".into()).await.unwrap();
        let mut recipient = world.enter("Guarded Recipient".into()).await.unwrap();
        let taken = world
            .apply(
                sender.session_id,
                PlayerAction::Take(tiamud_core::ItemSelection::one("blue-glass lantern")),
            )
            .await
            .unwrap();
        recipient.deliveries.recv().await.unwrap();
        let item = taken.player.inventory[0].id.clone();
        let recipient_selector = taken
            .player
            .social
            .online
            .iter()
            .find(|character| character.name == "Guarded Recipient")
            .unwrap()
            .id
            .clone();
        let action = || PlayerAction::Give {
            selection: tiamud_core::ItemSelection::one(item.clone()),
            recipient: recipient_selector.clone(),
        };

        world
            .apply(
                recipient.session_id,
                PlayerAction::Social(SocialAction::Ignore("Guarded Sender".into())),
            )
            .await
            .unwrap();
        let revision_before = world
            .current_state(sender.session_id)
            .await
            .unwrap()
            .0
            .revision;
        assert!(matches!(
            world.apply(sender.session_id, action()).await,
            Err(GameError::GiveUnavailable)
        ));
        assert_eq!(
            world
                .current_state(sender.session_id)
                .await
                .unwrap()
                .0
                .revision,
            revision_before
        );
        assert!(
            tokio::time::timeout(Duration::from_millis(20), recipient.deliveries.recv())
                .await
                .is_err()
        );

        world
            .apply(
                recipient.session_id,
                PlayerAction::Social(SocialAction::Unignore("Guarded Sender".into())),
            )
            .await
            .unwrap();
        world
            .apply(
                recipient.session_id,
                PlayerAction::Social(SocialAction::Block("Guarded Sender".into())),
            )
            .await
            .unwrap();
        assert!(matches!(
            world.apply(sender.session_id, action()).await,
            Err(GameError::GiveUnavailable)
        ));
        assert!(
            tokio::time::timeout(Duration::from_millis(20), recipient.deliveries.recv())
                .await
                .is_err()
        );
        world
            .apply(
                recipient.session_id,
                PlayerAction::Social(SocialAction::Unblock("Guarded Sender".into())),
            )
            .await
            .unwrap();
        store.fail_saves.store(true, Ordering::SeqCst);
        assert!(matches!(
            world.apply(sender.session_id, action()).await,
            Err(GameError::Persistence(_))
        ));
        assert!(
            tokio::time::timeout(Duration::from_millis(20), recipient.deliveries.recv())
                .await
                .is_err()
        );
        store.fail_saves.store(false, Ordering::SeqCst);
        let sender_state = world.current_state(sender.session_id).await.unwrap().2;
        let recipient_state = world.current_state(recipient.session_id).await.unwrap().2;
        assert_eq!(sender_state.inventory.len(), 1);
        assert!(recipient_state.inventory.is_empty());
        assert!(
            store
                .load_domain_events()
                .await
                .unwrap()
                .iter()
                .all(|event| !matches!(event, DomainEvent::ItemsGiven { .. }))
        );
    }

    #[test]
    fn authoritative_combat_cues_are_batch_aware_and_use_only_projected_ids() {
        let mut world = World::demo();
        let session = SessionId(7_001);
        let character_id = CharacterId(7_002);
        world
            .spawn_npc(NpcDefinition {
                stable_id: "test:cue-wolf".into(),
                name: "cue wolf".into(),
                description: "A wolf used to test projected combat cues.".into(),
                room_id: "demo:lantern-square".into(),
                level: 1,
                max_health: 8,
                strength: 3,
                defence: 0,
                experience_reward: 1,
                loot_item_ids: Vec::new(),
                respawn_ticks: 2,
            })
            .unwrap();
        world
            .connect(
                session,
                CharacterSnapshot {
                    id: character_id,
                    account_id: AccountId(7_003),
                    name: "Cue Observer".into(),
                    room_id: "demo:lantern-square".into(),
                    item_locations: Default::default(),
                },
            )
            .unwrap();
        let ability = DomainEvent::Gameplay {
            event: GameplayEvent::NpcAbilityUsed {
                npc_id: "test:cue-wolf".into(),
                npc_name: "cue wolf".into(),
                character_id,
                character_name: "Cue Observer".into(),
                ability_id: "test:claw".into(),
                ability_name: "Claw".into(),
                room_id: "demo:lantern-square".into(),
            },
        };
        let character_damaged = DomainEvent::Gameplay {
            event: GameplayEvent::CharacterDamaged {
                character_id,
                character_name: "Cue Observer".into(),
                npc_id: "test:cue-wolf".into(),
                npc_name: "cue wolf".into(),
                amount: 3,
                remaining_health: 12,
                room_id: "demo:lantern-square".into(),
            },
        };
        let npc_damaged = |remaining_health| DomainEvent::Gameplay {
            event: GameplayEvent::NpcDamaged {
                character_id,
                character_name: "Cue Observer".into(),
                npc_id: "test:cue-wolf".into(),
                npc_name: "cue wolf".into(),
                amount: 3,
                remaining_health,
                room_id: "demo:lantern-square".into(),
            },
        };
        let npc_defeated = DomainEvent::Gameplay {
            event: GameplayEvent::NpcDefeated {
                character_id,
                character_name: "Cue Observer".into(),
                npc_id: "test:cue-wolf".into(),
                npc_name: "cue wolf".into(),
                respawn_at_tick: 10,
                room_id: "demo:lantern-square".into(),
            },
        };
        let unrelated = DomainEvent::Gameplay {
            event: GameplayEvent::ExperienceGained {
                character_id,
                character_name: "Cue Observer".into(),
                amount: 1,
                total: 1,
                room_id: "demo:lantern-square".into(),
            },
        };

        let ability_batch = project_view_events(
            &world,
            session,
            &[ability.clone(), character_damaged.clone()],
        )
        .unwrap();
        assert_eq!(ability_batch.len(), 2);
        assert_eq!(
            ability_batch[0].combat_cue,
            Some(AuthoritativeCombatCue {
                kind: CombatCueKind::Attack,
                entity_id: ability_batch[0].event.actor_id.clone(),
            })
        );
        assert!(ability_batch[1].combat_cue.is_none());

        let fallback = project_view_events(&world, session, &[character_damaged]).unwrap();
        assert_eq!(
            fallback[0].combat_cue.as_ref().unwrap().kind,
            CombatCueKind::Attack
        );
        assert_eq!(
            fallback[0].combat_cue.as_ref().unwrap().entity_id,
            fallback[0].event.actor_id
        );

        let outcomes = project_view_events(
            &world,
            session,
            &[npc_damaged(4), npc_damaged(0), npc_defeated, unrelated],
        )
        .unwrap();
        assert_eq!(
            outcomes[0].combat_cue.as_ref().unwrap().kind,
            CombatCueKind::Hit
        );
        assert_eq!(
            outcomes[0].combat_cue.as_ref().unwrap().entity_id,
            outcomes[0].event.target_id.clone().unwrap()
        );
        assert!(outcomes[1].combat_cue.is_none());
        assert_eq!(
            outcomes[2].combat_cue.as_ref().unwrap().kind,
            CombatCueKind::Defeat
        );
        assert_eq!(
            outcomes[2].combat_cue.as_ref().unwrap().entity_id,
            outcomes[2].event.target_id.clone().unwrap()
        );
        assert!(outcomes[3].combat_cue.is_none());
        for projected in ability_batch
            .iter()
            .chain(fallback.iter())
            .chain(outcomes.iter())
        {
            if let Some(cue) = &projected.combat_cue {
                assert!(cue.entity_id.starts_with('o'));
                assert!(!cue.entity_id.contains("test:cue-wolf"));
            }
        }

        let distant_session = SessionId(7_004);
        world
            .connect(
                distant_session,
                CharacterSnapshot {
                    id: CharacterId(7_005),
                    account_id: AccountId(7_006),
                    name: "Distant Observer".into(),
                    room_id: "demo:north-gate".into(),
                    item_locations: Default::default(),
                },
            )
            .unwrap();
        assert!(
            project_view_events(&world, distant_session, &[ability])
                .unwrap()
                .is_empty()
        );
    }

    #[tokio::test]
    async fn persistence_failure_rolls_back_consumption_effects_and_equipment() {
        let store = Arc::new(MemoryStore::default());
        let world = WorldHandle::spawn(durable_gameplay_world(), store.clone())
            .await
            .unwrap();
        let entered = world
            .enter("Gameplay Rollback Tester".into())
            .await
            .unwrap();
        world
            .apply(
                entered.session_id,
                PlayerAction::Take(tiamud_core::ItemSelection::one("potion")),
            )
            .await
            .unwrap();
        store.fail_saves.store(true, Ordering::SeqCst);

        assert!(matches!(
            world
                .apply(entered.session_id, PlayerAction::Use("potion".into()))
                .await,
            Err(GameError::Persistence(_))
        ));
        store.fail_saves.store(false, Ordering::SeqCst);
        let inventory = world
            .apply(entered.session_id, PlayerAction::Inventory)
            .await
            .unwrap();
        assert!(
            inventory
                .output
                .as_deref()
                .unwrap()
                .contains("ember potion")
        );
        assert_eq!(inventory.player.inventory[0].rarity, "uncommon");
        assert_eq!(inventory.player.inventory[0].item_level, 2);
        assert_eq!(inventory.player.inventory_capacity.used_slots, 1);
        assert_eq!(inventory.player.inventory_capacity.used_weight_grams, 250);
        let state = store.state.lock().unwrap();
        let character_id = *state.characters.keys().next().unwrap();
        assert!(state.gameplay[&character_id].effects.is_empty());
        assert!(matches!(
            state
                .runtime
                .as_ref()
                .unwrap()
                .item_instances
                .get(&ItemInstanceId("test:potion".into()))
                .map(|instance| &instance.location),
            Some(tiamud_core::RuntimeItemLocation::Carried { .. })
        ));
    }

    #[tokio::test]
    async fn persistence_failure_rolls_back_party_quest_and_market_state_atomically() {
        let store = Arc::new(MemoryStore::default());
        let world = WorldHandle::spawn(World::demo(), store.clone())
            .await
            .unwrap();
        let entered = world
            .enter("Adventure Rollback Tester".into())
            .await
            .unwrap();
        store.fail_saves.store(true, Ordering::SeqCst);

        for action in [
            PlayerAction::Party(PartyAction::Create),
            PlayerAction::Quest(QuestAction::Accept("first-hunt".into())),
            PlayerAction::Economy(EconomyAction::Buy("tonic".into())),
        ] {
            assert!(matches!(
                world.apply(entered.session_id, action).await,
                Err(GameError::Persistence(_))
            ));
        }

        store.fail_saves.store(false, Ordering::SeqCst);
        let party = world
            .apply(entered.session_id, PlayerAction::Party(PartyAction::Status))
            .await
            .unwrap();
        let quest = world
            .apply(entered.session_id, PlayerAction::Quest(QuestAction::Status))
            .await
            .unwrap();
        let balance = world
            .apply(
                entered.session_id,
                PlayerAction::Economy(EconomyAction::Balance),
            )
            .await
            .unwrap();
        let shop = world
            .apply(
                entered.session_id,
                PlayerAction::Economy(EconomyAction::Shop),
            )
            .await
            .unwrap();
        assert!(party.output.unwrap().contains("not in a party"));
        assert!(quest.output.unwrap().contains("not been accepted"));
        assert!(balance.output.unwrap().contains("40 coin"));
        assert!(shop.output.unwrap().contains("stock 20"));
    }

    #[tokio::test]
    async fn shared_item_events_rebuild_the_world_after_restart() {
        let store = Arc::new(MemoryStore::default());
        let first_world = WorldHandle::spawn(World::demo(), store.clone())
            .await
            .unwrap();
        let collector = first_world.enter("Collector".into()).await.unwrap();
        first_world
            .apply(
                collector.session_id,
                PlayerAction::Take(tiamud_core::ItemSelection::one("blue-glass lantern")),
            )
            .await
            .unwrap();
        first_world.leave(collector.session_id).await;
        drop(first_world);

        let persisted = store.load_domain_events().await.unwrap();
        assert_eq!(persisted.len(), 1);
        let mut rebuilt = World::demo();
        for event in &persisted {
            rebuilt.replay_persisted_event(event).unwrap();
        }
        let restarted = WorldHandle::spawn(rebuilt, store).await.unwrap();
        let witness = restarted.enter("Witness".into()).await.unwrap();
        assert!(witness.scene.items.is_empty());
        restarted.leave(witness.session_id).await;

        let collector = restarted.enter("Collector".into()).await.unwrap();
        let inventory = restarted
            .apply(collector.session_id, PlayerAction::Inventory)
            .await
            .unwrap();
        let output = inventory.output.unwrap();
        assert!(output.starts_with(
            "Capacity: 1/24 slots; 1000/50000 g.\r\n\
                 You are carrying:\r\n\
                 - 1 × blue-glass lantern [common; level 1; 1000 g each; 1 stack slot; 1000 g total; policy transferable; ref o"
        ));
        assert!(output.ends_with("]\r\n"));
        assert!(!output.contains("demo:blue-lantern"));
    }

    #[tokio::test]
    async fn simultaneous_guest_sessions_have_independent_world_positions() {
        let store = Arc::new(MemoryStore::default());
        let world = WorldHandle::spawn(World::demo(), store).await.unwrap();
        let first = world.enter("Guest-1".into()).await.unwrap();
        let second = world.enter("Guest-2".into()).await.unwrap();
        assert_ne!(first.session_id, second.session_id);

        world
            .apply(first.session_id, PlayerAction::Move(Direction::North))
            .await
            .unwrap();
        let second_view = world
            .apply(second.session_id, PlayerAction::Look)
            .await
            .unwrap();
        assert_eq!(second_view.scene.title, "Lantern Square");
        assert_ne!(second_view.scene.view_id, first.scene.view_id);
    }

    #[tokio::test]
    async fn account_roster_and_character_selection_use_durable_ids() {
        let store = Arc::new(MemoryStore::default());
        let world = WorldHandle::spawn(World::demo(), store).await.unwrap();
        let account = world.open_account("Player".into()).await.unwrap();
        let character = world
            .load_or_create_character(account.id, "Explorer".into())
            .await
            .unwrap();

        let roster = world.list_characters(account.id).await.unwrap();
        assert_eq!(roster, vec![character.clone()]);
        let entered = world
            .enter_character(account.id, character.id)
            .await
            .unwrap();
        assert_eq!(entered.scene.title, "Lantern Square");
        assert_ne!(entered.scene.view_id, "demo:lantern-square");
    }

    #[tokio::test]
    async fn character_selection_cannot_cross_account_boundaries() {
        let store = Arc::new(MemoryStore::default());
        let world = WorldHandle::spawn(World::demo(), store).await.unwrap();
        let owner = world.open_account("Owner".into()).await.unwrap();
        let stranger = world.open_account("Stranger".into()).await.unwrap();
        let character = world
            .load_or_create_character(owner.id, "Private".into())
            .await
            .unwrap();

        assert!(matches!(
            world.enter_character(stranger.id, character.id).await,
            Err(GameError::Persistence(StoreError::CharacterNotFound { .. }))
        ));
    }

    #[tokio::test]
    async fn authenticated_roster_and_selection_derive_ownership_from_token() {
        let store = Arc::new(MemoryStore::default());
        let world = WorldHandle::spawn(World::demo(), store).await.unwrap();
        let owner = world
            .register_account("Owner".into(), "a sufficiently long password".into())
            .await
            .unwrap();
        let stranger = world
            .register_account(
                "Stranger".into(),
                "another sufficiently long password".into(),
            )
            .await
            .unwrap();
        assert!(matches!(
            world.enter("owner".into()).await,
            Err(GameError::Persistence(StoreError::AccountAlreadyExists))
        ));
        let character = world
            .create_authenticated_character(owner.token.as_str().to_owned(), "Private".into())
            .await
            .unwrap();

        assert_eq!(
            world
                .authenticated_roster(owner.token.as_str().to_owned())
                .await
                .unwrap(),
            vec![character.clone()]
        );
        assert!(matches!(
            world
                .enter_authenticated_character(stranger.token.as_str().to_owned(), character.id,)
                .await,
            Err(GameError::Persistence(StoreError::CharacterNotFound { .. }))
        ));
        world
            .enter_authenticated_character(owner.token.as_str().to_owned(), character.id)
            .await
            .unwrap();
    }

    #[tokio::test]
    async fn authenticated_creation_persists_the_selected_composed_sheet() {
        let store = Arc::new(MemoryStore::default());
        let loaded = crate::world_loader::default_loaded_world().unwrap();
        let world = WorldHandle::spawn(
            crate::world_loader::build_loaded_world(&loaded).unwrap(),
            store.clone(),
        )
        .await
        .unwrap();
        let owner = world
            .register_account("Owner".into(), "a sufficiently long password".into())
            .await
            .unwrap();
        let lineage_id = world.creation_catalog().lineages[0].id.clone();
        let background_id = world.creation_catalog().backgrounds[0].id.clone();
        let base_class_id = world.creation_catalog().base_classes[0].id.clone();

        let character = world
            .create_authenticated_character_with_progression(
                owner.token.as_str().to_owned(),
                "Aria".into(),
                Some(lineage_id.clone()),
                CharacterBackgroundSelection::Id(background_id.clone()),
                Some(base_class_id.clone()),
            )
            .await
            .unwrap();
        let gameplay = store.load_character_gameplay(character.id).await.unwrap();
        assert_eq!(gameplay.lineage_id, lineage_id);
        assert_eq!(
            gameplay.background_id.as_deref(),
            Some(background_id.as_str())
        );
        assert_eq!(gameplay.base_class_id, base_class_id);
        assert_eq!(gameplay.active_class_id, base_class_id);

        let default_background = world
            .create_authenticated_character_with_progression(
                owner.token.as_str().to_owned(),
                "Default Origin".into(),
                None,
                CharacterBackgroundSelection::Default,
                None,
            )
            .await
            .unwrap();
        assert_eq!(
            store
                .load_character_gameplay(default_background.id)
                .await
                .unwrap()
                .background_id
                .as_deref(),
            world.creation_catalog().default_background_id.as_deref()
        );

        let no_background = world
            .create_authenticated_character_with_progression(
                owner.token.as_str().to_owned(),
                "No Origin".into(),
                None,
                CharacterBackgroundSelection::None,
                None,
            )
            .await
            .unwrap();
        assert_eq!(
            store
                .load_character_gameplay(no_background.id)
                .await
                .unwrap()
                .background_id,
            None
        );
    }

    #[tokio::test]
    async fn talent_and_promotion_actions_persist_the_v3_progression_state() {
        let store = Arc::new(MemoryStore::default());
        let loaded = crate::world_loader::default_loaded_world().unwrap();
        let world = WorldHandle::spawn(
            crate::world_loader::build_loaded_world(&loaded).unwrap(),
            store.clone(),
        )
        .await
        .unwrap();
        let owner = world
            .register_account(
                "Progression Owner".into(),
                "a sufficiently long password".into(),
            )
            .await
            .unwrap();
        let character = world
            .create_authenticated_character_with_progression(
                owner.token.as_str().to_owned(),
                "Iria".into(),
                Some("tiamud:mortal".into()),
                CharacterBackgroundSelection::Id("tiamud:city-guard".into()),
                Some("tiamud:vanguard".into()),
            )
            .await
            .unwrap();
        {
            let mut state = store.state.lock().unwrap();
            let gameplay = state.gameplay.get_mut(&character.id).unwrap();
            gameplay.level = 5;
            gameplay.experience = 1_600;
            gameplay.progression_points.insert("talent".into(), 1);
        }

        let entered = world
            .enter_authenticated_character(owner.token.as_str().to_owned(), character.id)
            .await
            .unwrap();
        let talent = world
            .apply(
                entered.session_id,
                PlayerAction::AcquireTalent {
                    graph_id: "tiamud:martial-talents".into(),
                    node_id: "tiamud:guarded-stance".into(),
                },
            )
            .await
            .unwrap();
        assert_eq!(
            talent
                .player
                .character
                .talents
                .iter()
                .find(|graph| graph.id == "tiamud:martial-talents")
                .unwrap()
                .nodes
                .iter()
                .find(|node| node.id == "tiamud:guarded-stance")
                .map(|node| node.rank),
            Some(1)
        );

        let promoted = world
            .apply(
                entered.session_id,
                PlayerAction::Promote {
                    class_id: "tiamud:bastion".into(),
                },
            )
            .await
            .unwrap();
        assert_eq!(promoted.player.character.base_class.id, "tiamud:vanguard");
        assert_eq!(promoted.player.character.active_class.id, "tiamud:bastion");

        let persisted = store.load_character_gameplay(character.id).await.unwrap();
        assert_eq!(persisted.active_class_id, "tiamud:bastion");
        assert_eq!(
            persisted.talent_node_ranks["tiamud:martial-talents"]["tiamud:guarded-stance"],
            1
        );
        assert_eq!(persisted.progression_points["talent"], 0);
    }

    #[tokio::test]
    async fn legacy_entry_keeps_the_same_character_id_across_reconnects() {
        let store = Arc::new(MemoryStore::default());
        let world = WorldHandle::spawn(World::demo(), store).await.unwrap();
        let first = world.enter("Guest".into()).await.unwrap();
        let account = world.open_account("guest".into()).await.unwrap();
        let first_roster = world.list_characters(account.id).await.unwrap();
        world.leave(first.session_id).await;
        let second = world.enter("guest".into()).await.unwrap();
        world.leave(second.session_id).await;
        let second_roster = world.list_characters(account.id).await.unwrap();

        assert_eq!(second_roster, first_roster);
        assert_eq!(second_roster.len(), 1);
    }

    fn connect_tick_character(
        world: &mut World,
        store: &MemoryStore,
        session: SessionId,
        character_id: CharacterId,
        room_id: &str,
        name: &str,
    ) {
        let character = CharacterSnapshot {
            id: character_id,
            account_id: AccountId(character_id.0),
            name: name.into(),
            room_id: room_id.into(),
            item_locations: std::collections::BTreeMap::new(),
        };
        world.connect(session, character.clone()).unwrap();
        let mut state = store.state.lock().unwrap();
        state.characters.insert(character_id, character);
        state
            .gameplay
            .insert(character_id, CharacterGameplaySnapshot::default());
    }

    fn timer_narration_content(
        metrics: Option<Arc<crate::operations::RuntimeMetrics>>,
    ) -> RuntimeContent {
        let content = RuntimeContent::new(
            Vec::new(),
            vec![RuntimeScript {
                id: "script:private-timer".into(),
                room_id: "demo:lantern-square".into(),
                hooks: BTreeSet::from([ScriptHook::OnTimer]),
                capabilities: BTreeSet::from([
                    ScriptCapability::Narrate,
                    ScriptCapability::SetFlag,
                ]),
                limits: ScriptLimits::default(),
                source: r#"
                    function on_timer(ctx)
                        tiamud.emit('narrate', 'self', ctx.actor_name .. ' alone hears the bell.')
                        tiamud.emit('set_flag', 'self', 'timer-ran')
                    end
                "#
                .into(),
            }],
            7,
        );
        match metrics {
            Some(metrics) => content.with_operational_metrics(metrics),
            None => content,
        }
    }

    fn seed_trade_expiring_on_next_tick(
        world: &mut World,
        requester: SessionId,
        recipient_name: &str,
    ) {
        world
            .apply(
                requester,
                PlayerAction::Trade(tiamud_core::TradeAction::Request(recipient_name.into())),
            )
            .unwrap();
        for _ in 1..tiamud_core::TRADE_REQUEST_TICKS {
            world.advance_tick().unwrap();
        }
        let requester_id = world
            .authorized_runtime_context(requester)
            .unwrap()
            .character_id;
        let trade = world.trade_for_character(requester_id).unwrap();
        assert_eq!(
            world.runtime_snapshot().logical_tick + 1,
            trade.expires_at_tick
        );
    }

    #[tokio::test]
    async fn tick_cancels_expired_trades_before_timer_hooks_in_normal_and_fallback_paths() {
        for script_budget in [Duration::from_secs(1), Duration::ZERO] {
            let store = MemoryStore::default();
            let mut world = World::demo();
            let requester = SessionId(61);
            let recipient = SessionId(62);
            connect_tick_character(
                &mut world,
                &store,
                requester,
                CharacterId(61),
                "demo:lantern-square",
                "Expiry Requester",
            );
            connect_tick_character(
                &mut world,
                &store,
                recipient,
                CharacterId(62),
                "demo:lantern-square",
                "Expiry Recipient",
            );
            seed_trade_expiring_on_next_tick(&mut world, requester, "Expiry Recipient");
            let content = timer_narration_content(None);
            let roles = HashMap::from([
                (requester, AccountRole::Player),
                (recipient, AccountRole::Player),
            ]);
            let mut delivery = DeliveryRouter::new(None);

            assert!(
                persist_tick_with_script_budget(
                    &mut world,
                    &content,
                    &roles,
                    &store,
                    &mut delivery,
                    script_budget,
                )
                .await
            );

            assert!(world.runtime_snapshot().trades.is_empty());
            assert!(matches!(
                store.state.lock().unwrap().domain_events.first(),
                Some(DomainEvent::TradeCancelled {
                    reason: TradeCancellationReason::Expired,
                    ..
                })
            ));
            let timer_ran = world
                .character_gameplay_snapshot(requester)
                .unwrap()
                .script_flags
                .contains("timer-ran");
            assert_eq!(timer_ran, !script_budget.is_zero());
        }
    }

    #[tokio::test]
    async fn tick_expiry_precedes_combat_and_needs_no_social_policy_read() {
        let store = MemoryStore::default();
        let mut world = durable_gameplay_world();
        let requester = SessionId(67);
        let recipient = SessionId(68);
        let requester_id = CharacterId(67);
        let recipient_id = CharacterId(68);
        connect_tick_character(
            &mut world,
            &store,
            requester,
            requester_id,
            "test:arena",
            "Collision A",
        );
        connect_tick_character(
            &mut world,
            &store,
            recipient,
            recipient_id,
            "test:arena",
            "Collision B",
        );
        seed_trade_expiring_on_next_tick(&mut world, requester, "Collision B");
        let mut runtime = world.runtime_snapshot();
        runtime
            .npcs
            .get_mut("test:guardian")
            .unwrap()
            .engaged_character = Some(requester_id);
        world.restore_runtime_snapshot(runtime).unwrap();
        {
            let mut state = store.state.lock().unwrap();
            state.ignores.insert((recipient_id, requester_id));
        }
        store.fail_social_reads.store(true, Ordering::SeqCst);
        let roles = HashMap::from([
            (requester, AccountRole::Player),
            (recipient, AccountRole::Player),
        ]);
        let mut delivery = DeliveryRouter::new(None);

        assert!(
            persist_tick(
                &mut world,
                &RuntimeContent::default(),
                &roles,
                &store,
                &mut delivery,
            )
            .await
        );

        let state = store.state.lock().unwrap();
        let expired = state
            .domain_events
            .iter()
            .position(|event| {
                matches!(
                    event,
                    DomainEvent::TradeCancelled {
                        reason: TradeCancellationReason::Expired,
                        ..
                    }
                )
            })
            .unwrap();
        let combat = state
            .domain_events
            .iter()
            .position(|event| {
                matches!(
                    event,
                    DomainEvent::Gameplay {
                        event: GameplayEvent::CharacterDamaged { .. }
                            | GameplayEvent::NpcDefeated { .. }
                    }
                )
            })
            .unwrap();
        assert!(expired < combat);
        assert!(state.runtime.as_ref().unwrap().trades.is_empty());
    }

    #[tokio::test]
    async fn tick_reconciles_script_changes_once_after_timer_hooks() {
        let store = MemoryStore::default();
        let mut world = World::demo();
        let requester = SessionId(65);
        let recipient = SessionId(66);
        let requester_id = CharacterId(65);
        connect_tick_character(
            &mut world,
            &store,
            requester,
            requester_id,
            "demo:lantern-square",
            "Timer Basis A",
        );
        connect_tick_character(
            &mut world,
            &store,
            recipient,
            CharacterId(66),
            "demo:lantern-square",
            "Timer Basis B",
        );
        let requester_gameplay = CharacterGameplaySnapshot {
            coins: 10,
            ..CharacterGameplaySnapshot::default()
        };
        world
            .restore_character_gameplay(requester_id, requester_gameplay.clone())
            .unwrap();
        store
            .state
            .lock()
            .unwrap()
            .gameplay
            .insert(requester_id, requester_gameplay);
        world
            .apply(
                requester,
                PlayerAction::Trade(tiamud_core::TradeAction::Request("Timer Basis B".into())),
            )
            .unwrap();
        world
            .apply(
                recipient,
                PlayerAction::Trade(tiamud_core::TradeAction::Accept),
            )
            .unwrap();
        world
            .apply(
                requester,
                PlayerAction::Trade(tiamud_core::TradeAction::OfferCoins(1)),
            )
            .unwrap();
        let state_revision = world
            .trade_for_character(requester_id)
            .unwrap()
            .state_revision;
        world
            .apply(
                requester,
                PlayerAction::Trade(tiamud_core::TradeAction::Confirm { state_revision }),
            )
            .unwrap();
        assert_eq!(
            world
                .trade_for_character(requester_id)
                .unwrap()
                .confirmations
                .len(),
            1
        );
        let roles = HashMap::from([
            (requester, AccountRole::Player),
            (recipient, AccountRole::Player),
        ]);
        let mut delivery = DeliveryRouter::new(None);

        assert!(
            persist_tick(
                &mut world,
                &timer_narration_content(None),
                &roles,
                &store,
                &mut delivery,
            )
            .await
        );

        let trade = world.trade_for_character(requester_id).unwrap();
        assert_eq!(trade.state_revision, state_revision + 1);
        assert!(trade.confirmations.is_empty());
        assert_eq!(
            store
                .state
                .lock()
                .unwrap()
                .domain_events
                .iter()
                .filter(|event| matches!(event, DomainEvent::TradeInvalidated { .. }))
                .count(),
            1
        );
    }

    #[tokio::test]
    async fn failed_tick_fallback_cleanup_restores_trade_and_publishes_no_delivery() {
        let store = MemoryStore::default();
        let mut world = World::demo();
        let requester = SessionId(63);
        let recipient = SessionId(64);
        connect_tick_character(
            &mut world,
            &store,
            requester,
            CharacterId(63),
            "demo:lantern-square",
            "Failed Tick Requester",
        );
        connect_tick_character(
            &mut world,
            &store,
            recipient,
            CharacterId(64),
            "demo:lantern-square",
            "Failed Tick Recipient",
        );
        seed_trade_expiring_on_next_tick(&mut world, requester, "Failed Tick Recipient");
        let before = world.runtime_snapshot();
        let roles = HashMap::from([
            (requester, AccountRole::Player),
            (recipient, AccountRole::Player),
        ]);
        let mut delivery = DeliveryRouter::new(None);
        let mut requester_inbox = delivery.register(requester);
        let mut recipient_inbox = delivery.register(recipient);
        store.fail_saves.store(true, Ordering::SeqCst);

        assert!(
            !persist_tick_with_script_budget(
                &mut world,
                &timer_narration_content(None),
                &roles,
                &store,
                &mut delivery,
                Duration::ZERO,
            )
            .await
        );

        assert_eq!(world.runtime_snapshot(), before);
        assert!(requester_inbox.try_recv().is_err());
        assert!(recipient_inbox.try_recv().is_err());
        assert!(store.state.lock().unwrap().domain_events.is_empty());
    }

    #[tokio::test]
    async fn timer_script_narration_is_delivered_only_to_its_authorized_session() {
        let store = MemoryStore::default();
        let mut world = World::demo();
        let target = SessionId(71);
        let peer = SessionId(72);
        connect_tick_character(
            &mut world,
            &store,
            target,
            CharacterId(71),
            "demo:lantern-square",
            "Target",
        );
        connect_tick_character(
            &mut world,
            &store,
            peer,
            CharacterId(72),
            "demo:north-gate",
            "Peer",
        );
        let content = timer_narration_content(None);
        let roles = HashMap::from([(target, AccountRole::Player), (peer, AccountRole::Player)]);
        let mut delivery = DeliveryRouter::new(None);
        let mut target_inbox = delivery.register(target);
        let mut peer_inbox = delivery.register(peer);

        assert!(persist_tick(&mut world, &content, &roles, &store, &mut delivery).await);

        let mut deliveries = Vec::new();
        for (session, inbox) in [(target, &mut target_inbox), (peer, &mut peer_inbox)] {
            while let Ok(crate::delivery::SessionDelivery::World(delivery)) = inbox.try_recv() {
                deliveries.push((session, delivery));
            }
        }
        let narration = deliveries
            .iter()
            .flat_map(|(session, delivery)| {
                delivery.events.iter().map(move |event| (*session, event))
            })
            .collect::<Vec<_>>();
        assert_eq!(narration.len(), 1);
        assert_eq!(narration[0].0, target);
        assert_eq!(narration[0].1.event.text, "Target alone hears the bell.");
        assert_ne!(narration[0].1.event.actor_id, "71");
        assert!(
            deliveries
                .iter()
                .filter(|(session, _)| *session == peer)
                .all(|(_, delivery)| delivery.events.is_empty())
        );
    }

    #[tokio::test]
    async fn exhausted_global_script_budget_commits_only_the_base_tick() {
        let store = MemoryStore::default();
        let mut world = World::demo();
        let session = SessionId(81);
        connect_tick_character(
            &mut world,
            &store,
            session,
            CharacterId(81),
            "demo:lantern-square",
            "Budget",
        );
        let before = world.runtime_snapshot();
        let metrics = Arc::new(crate::operations::RuntimeMetrics::default());
        let content = timer_narration_content(Some(metrics.clone()));
        let roles = HashMap::from([(session, AccountRole::Player)]);
        let mut delivery = DeliveryRouter::new(None);
        let mut receiver = delivery.register(session);

        assert!(
            persist_tick_with_script_budget(
                &mut world,
                &content,
                &roles,
                &store,
                &mut delivery,
                Duration::ZERO,
            )
            .await
        );
        assert_eq!(
            world.runtime_snapshot().logical_tick,
            before.logical_tick + 1
        );
        assert!(
            !world
                .character_gameplay_snapshot(session)
                .unwrap()
                .script_flags
                .contains("timer-ran")
        );
        assert_eq!(
            store
                .state
                .lock()
                .unwrap()
                .runtime
                .as_ref()
                .map(|runtime| runtime.logical_tick),
            Some(before.logical_tick + 1)
        );
        let mut received = Vec::new();
        while let Ok(crate::delivery::SessionDelivery::World(delivery)) = receiver.try_recv() {
            received.push(delivery);
        }
        assert!(!received.is_empty());
        assert!(received.iter().all(|delivery| delivery.events.is_empty()));
        assert!(
            metrics
                .prometheus(0)
                .contains("tiamud_script_timeouts_total 1")
        );
    }

    #[tokio::test]
    async fn scheduler_and_restart_restore_combat_progression_items_and_respawn() {
        let store = Arc::new(MemoryStore::default());
        let first_world = WorldHandle::spawn_with_scheduler(
            durable_gameplay_world(),
            store.clone(),
            WorldSchedulerConfig::every(Duration::from_millis(20)),
        )
        .await
        .unwrap();
        let entered = first_world.enter("Durable Hero".into()).await.unwrap();
        for action in [
            PlayerAction::Quest(QuestAction::Accept("first-hunt".into())),
            PlayerAction::Economy(EconomyAction::Buy("tonic".into())),
            PlayerAction::Take(tiamud_core::ItemSelection::one("potion")),
            PlayerAction::Use("potion".into()),
            PlayerAction::Take(tiamud_core::ItemSelection::one("sword")),
            PlayerAction::Equip("sword".into()),
            PlayerAction::Attack("guardian".into()),
        ] {
            first_world.apply(entered.session_id, action).await.unwrap();
        }

        tokio::time::timeout(Duration::from_secs(1), async {
            loop {
                let ready = store
                    .state
                    .lock()
                    .unwrap()
                    .runtime
                    .as_ref()
                    .is_some_and(|runtime| runtime.logical_tick >= 1);
                if ready {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(2)).await;
            }
        })
        .await
        .unwrap();
        first_world.shutdown().await.unwrap();

        let (character_id, gameplay, defeated_at) = {
            let state = store.state.lock().unwrap();
            let character_id = *state.characters.keys().next().unwrap();
            let gameplay = state.gameplay[&character_id].clone();
            let runtime = state.runtime.as_ref().unwrap();
            let defeated_at = runtime.npcs["test:guardian"].respawn_at_tick.unwrap();
            assert!(matches!(
                runtime
                    .item_instances
                    .get(&ItemInstanceId("test:potion".into()))
                    .map(|instance| &instance.location),
                Some(tiamud_core::RuntimeItemLocation::Consumed)
            ));
            (character_id, gameplay, defeated_at)
        };
        assert_eq!(gameplay.level, 2);
        assert_eq!(gameplay.experience, 120);
        assert_eq!(
            gameplay.equipment[&EquipmentSlot::Weapon],
            ItemInstanceId("test:sword".into())
        );
        assert!(!gameplay.effects.is_empty());
        assert_eq!(gameplay.coins, 28);
        assert_eq!(gameplay.commodities["tonic"], 1);
        assert_eq!(
            gameplay.quests["first-hunt"].status,
            tiamud_core::QuestStatus::ReadyToClaim
        );
        assert_eq!(
            store
                .state
                .lock()
                .unwrap()
                .runtime
                .as_ref()
                .unwrap()
                .adventure
                .shop_stock["tonic"],
            19
        );

        let restarted = WorldHandle::spawn_with_scheduler(
            durable_gameplay_world(),
            store.clone(),
            WorldSchedulerConfig::every(Duration::from_millis(20)),
        )
        .await
        .unwrap();
        let account_id = store.state.lock().unwrap().characters[&character_id].account_id;
        let entered = restarted
            .enter_character(account_id, character_id)
            .await
            .unwrap();
        let inventory = restarted
            .apply(entered.session_id, PlayerAction::Inventory)
            .await
            .unwrap();
        assert!(inventory.output.unwrap().contains(
            "weapon: 1 × bronze sword [common; level 1; 2000 g each; 1 stack slot; 2000 g total; policy transferable; ref o"
        ));
        assert_eq!(entered.player.inventory_capacity.used_slots, 0);
        assert_eq!(entered.player.inventory_capacity.used_weight_grams, 2_000);
        assert_eq!(entered.player.equipment[0].rarity, "common");
        assert_eq!(entered.player.equipment[0].item_level, 1);
        assert_eq!(entered.scene.items[0].label, "silver fang");
        assert_eq!(entered.scene.items[0].rarity, "rare");
        assert_eq!(entered.scene.items[0].item_level, 2);
        assert!(entered.scene.npcs.is_empty());
        let claimed = restarted
            .apply(
                entered.session_id,
                PlayerAction::Quest(QuestAction::Complete("first-hunt".into())),
            )
            .await
            .unwrap();
        assert!(claimed.output.unwrap().contains("25 coin"));
        assert_eq!(
            store.state.lock().unwrap().gameplay[&character_id].coins,
            53
        );

        tokio::time::timeout(Duration::from_secs(1), async {
            loop {
                let tick = store
                    .state
                    .lock()
                    .unwrap()
                    .runtime
                    .as_ref()
                    .unwrap()
                    .logical_tick;
                if tick >= defeated_at {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(2)).await;
            }
        })
        .await
        .unwrap();
        let after_respawn = restarted
            .apply(entered.session_id, PlayerAction::Look)
            .await
            .unwrap();
        assert_eq!(after_respawn.scene.npcs[0].label, "stone guardian");
        restarted.shutdown().await.unwrap();
    }

    #[tokio::test]
    async fn party_membership_and_authority_survive_actor_restart() {
        let store = Arc::new(MemoryStore::default());
        let first = WorldHandle::spawn(World::demo(), store.clone())
            .await
            .unwrap();
        let leader = first.enter("Party Leader".into()).await.unwrap();
        let member = first.enter("Party Member".into()).await.unwrap();
        first
            .apply(leader.session_id, PlayerAction::Party(PartyAction::Create))
            .await
            .unwrap();
        first
            .apply(
                leader.session_id,
                PlayerAction::Party(PartyAction::Invite("Party Member".into())),
            )
            .await
            .unwrap();
        first
            .apply(
                member.session_id,
                PlayerAction::Party(PartyAction::Accept("Party Leader".into())),
            )
            .await
            .unwrap();
        let identities = {
            let state = store.state.lock().unwrap();
            let mut values = state
                .characters
                .values()
                .map(|character| (character.name.clone(), character.account_id, character.id))
                .collect::<Vec<_>>();
            values.sort_by(|left, right| left.0.cmp(&right.0));
            values
        };
        first.shutdown().await.unwrap();

        let restarted = WorldHandle::spawn(World::demo(), store).await.unwrap();
        let leader = identities
            .iter()
            .find(|identity| identity.0 == "Party Leader")
            .unwrap();
        let member = identities
            .iter()
            .find(|identity| identity.0 == "Party Member")
            .unwrap();
        let leader = restarted.enter_character(leader.1, leader.2).await.unwrap();
        let member = restarted.enter_character(member.1, member.2).await.unwrap();
        let status = restarted
            .apply(member.session_id, PlayerAction::Party(PartyAction::Status))
            .await
            .unwrap();
        assert!(
            status
                .output
                .unwrap()
                .contains("Party Leader, Party Member")
        );
        assert!(matches!(
            restarted
                .apply(
                    member.session_id,
                    PlayerAction::Party(PartyAction::Invite("nobody".into())),
                )
                .await,
            Err(GameError::World(CoreWorldError::PartyForbidden))
        ));
        restarted.leave(leader.session_id).await;
        restarted.shutdown().await.unwrap();
    }

    #[tokio::test]
    async fn mail_is_private_durable_offline_and_does_not_enter_social_or_world_streams() {
        let store = Arc::new(MemoryStore::default());
        let world = WorldHandle::spawn(World::demo(), store.clone())
            .await
            .unwrap();
        let sender = world.enter("Mail Sender".into()).await.unwrap();
        let mut recipient = world.enter("Mail Recipient".into()).await.unwrap();
        let (sender_id, recipient_id) = {
            let state = store.state.lock().unwrap();
            let id_for = |name: &str| {
                state
                    .characters
                    .values()
                    .find(|character| character.name == name)
                    .unwrap()
                    .id
            };
            (id_for("Mail Sender"), id_for("Mail Recipient"))
        };
        let events_before = store.state.lock().unwrap().domain_events.len();

        let accepted = world
            .apply(
                sender.session_id,
                PlayerAction::Mail(tiamud_core::MailAction::Send {
                    target: "Mail Recipient".into(),
                    subject: "Online subject <&>".into(),
                    body: "Private online body | with a pipe.".into(),
                }),
            )
            .await
            .unwrap();
        assert_eq!(accepted.output.as_deref(), Some("Mail accepted.\r\n"));
        assert!(accepted.events.is_empty());
        assert!(accepted.mail_page.is_none());
        assert!(accepted.mail_message.is_none());
        assert!(
            tokio::time::timeout(Duration::from_millis(20), recipient.deliveries.recv())
                .await
                .is_err()
        );

        let recipient_account = recipient.account_id;
        world.leave(recipient.session_id).await;
        let accepted_offline = world
            .apply(
                sender.session_id,
                PlayerAction::Mail(tiamud_core::MailAction::Send {
                    target: "Mail Recipient".into(),
                    subject: "Offline subject".into(),
                    body: "Private offline body.".into(),
                }),
            )
            .await
            .unwrap();
        assert_eq!(
            accepted_offline.output.as_deref(),
            Some("Mail accepted.\r\n")
        );
        assert!(accepted_offline.events.is_empty());
        assert_eq!(
            store.state.lock().unwrap().domain_events.len(),
            events_before
        );

        let recipient = world
            .enter_character(recipient_account, recipient_id)
            .await
            .unwrap();
        let listed = world
            .apply(
                recipient.session_id,
                PlayerAction::Mail(tiamud_core::MailAction::List { page: 1 }),
            )
            .await
            .unwrap();
        let page = listed.mail_page.expect("private mailbox page");
        assert_eq!(page.total_messages, 2);
        assert_eq!(page.unread_count, 2);
        assert_eq!(page.messages.len(), 2);
        assert!(page.messages.iter().all(|message| {
            message.public_id.len() == tiamud_core::MAIL_PUBLIC_ID_BYTES
                && tiamud_core::valid_mail_public_id(&message.public_id)
        }));
        let offline_id = page
            .messages
            .iter()
            .find(|message| message.subject == "Offline subject")
            .unwrap()
            .public_id
            .clone();

        let read = world
            .apply(
                recipient.session_id,
                PlayerAction::Mail(tiamud_core::MailAction::Read {
                    mail_id: offline_id.clone(),
                }),
            )
            .await
            .unwrap();
        assert_eq!(
            read.mail_message
                .as_ref()
                .map(|message| message.body.as_str()),
            Some("Private offline body.")
        );
        assert!(matches!(
            world
                .apply(
                    recipient.session_id,
                    PlayerAction::Social(SocialAction::Reply("Mail is not reply state".into())),
                )
                .await,
            Err(GameError::Persistence(StoreError::SocialTargetNotFound))
        ));

        let reported = world
            .apply(
                recipient.session_id,
                PlayerAction::Mail(tiamud_core::MailAction::Report {
                    mail_id: offline_id.clone(),
                    reason: "Unwanted contact".into(),
                }),
            )
            .await
            .unwrap();
        assert!(reported.output.unwrap().contains("report 1"));
        {
            let state = store.state.lock().unwrap();
            let evidence = state.staff_reports[0].mail_evidence.as_ref().unwrap();
            assert_eq!(evidence.sender_name, "Mail Sender");
            assert_eq!(evidence.subject, "Offline subject");
            assert_eq!(evidence.body, "Private offline body.");
        }

        world
            .apply(
                recipient.session_id,
                PlayerAction::Mail(tiamud_core::MailAction::Delete {
                    mail_id: offline_id,
                }),
            )
            .await
            .unwrap();
        assert_eq!(
            store
                .state
                .lock()
                .unwrap()
                .mail
                .iter()
                .filter(|mail| mail.sender == sender_id && mail.recipient == recipient_id)
                .count(),
            1
        );
    }

    #[tokio::test]
    async fn boards_are_bounded_pull_only_public_and_do_not_become_chat_history() {
        let store = Arc::new(MemoryStore::default());
        let world = WorldHandle::spawn(World::demo(), store.clone())
            .await
            .unwrap();
        let author = world.enter("Board Author".into()).await.unwrap();
        let mut reader = world.enter("Board Reader".into()).await.unwrap();
        let events_before = store.state.lock().unwrap().domain_events.len();

        let catalog = world
            .apply(
                author.session_id,
                PlayerAction::Board(tiamud_core::BoardAction::Catalog),
            )
            .await
            .unwrap();
        assert_eq!(catalog.board_catalog.as_ref().map(Vec::len), Some(2));
        assert!(
            catalog
                .board_catalog
                .as_ref()
                .unwrap()
                .iter()
                .all(|board| board.can_post)
        );

        let published = world
            .apply(
                author.session_id,
                PlayerAction::Board(tiamud_core::BoardAction::Post {
                    board: tiamud_core::BoardKey::General,
                    subject: "Gate <&>".into(),
                    body: "Meet | beyond the gate.".into(),
                }),
            )
            .await
            .unwrap();
        let post_id = published
            .board_entry
            .as_ref()
            .map(|entry| entry.summary.public_id.clone())
            .expect("published entry");
        assert!(tiamud_core::valid_board_public_id(&post_id));
        assert!(published.events.is_empty());
        assert_eq!(
            store.state.lock().unwrap().domain_events.len(),
            events_before
        );
        assert!(
            tokio::time::timeout(Duration::from_millis(20), reader.deliveries.recv())
                .await
                .is_err(),
            "board publication must not fan out as chat"
        );

        store.state.lock().unwrap().muted.insert(author.account_id);
        let muted_catalog = world
            .apply(
                author.session_id,
                PlayerAction::Board(tiamud_core::BoardAction::Catalog),
            )
            .await
            .unwrap();
        assert!(
            muted_catalog
                .board_catalog
                .as_ref()
                .unwrap()
                .iter()
                .all(|board| !board.can_post)
        );
        assert!(matches!(
            world
                .apply(
                    author.session_id,
                    PlayerAction::Board(tiamud_core::BoardAction::Post {
                        board: tiamud_core::BoardKey::Newbie,
                        subject: "Muted".into(),
                        body: "This cannot be published.".into(),
                    }),
                )
                .await,
            Err(GameError::Persistence(StoreError::ChatMuted))
        ));

        world
            .apply(
                reader.session_id,
                PlayerAction::Social(SocialAction::Block("Board Author".into())),
            )
            .await
            .unwrap();
        let listed = world
            .apply(
                reader.session_id,
                PlayerAction::Board(tiamud_core::BoardAction::List {
                    board: tiamud_core::BoardKey::General,
                    page: 1,
                }),
            )
            .await
            .unwrap();
        let page = listed.board_page.expect("board page");
        assert_eq!(page.total_posts, 1);
        assert_eq!(page.posts[0].public_id, post_id);
        assert!(page.posts[0].can_report);
        assert!(!page.posts[0].can_delete);

        let read = world
            .apply(
                reader.session_id,
                PlayerAction::Board(tiamud_core::BoardAction::Read {
                    post_id: post_id.clone(),
                }),
            )
            .await
            .unwrap();
        assert_eq!(
            read.board_entry.as_ref().map(|entry| entry.body.as_str()),
            Some("Meet | beyond the gate.")
        );
        assert!(matches!(
            world
                .apply(
                    reader.session_id,
                    PlayerAction::Social(SocialAction::Reply("Not board state".into())),
                )
                .await,
            Err(GameError::Persistence(StoreError::SocialTargetNotFound))
        ));

        let reported = world
            .apply(
                reader.session_id,
                PlayerAction::Board(tiamud_core::BoardAction::Report {
                    post_id: post_id.clone(),
                    reason: "Unsafe public content".into(),
                }),
            )
            .await
            .unwrap();
        assert!(reported.output.unwrap().contains("report 1"));
        world
            .apply(
                author.session_id,
                PlayerAction::Board(tiamud_core::BoardAction::Delete {
                    post_id: post_id.clone(),
                }),
            )
            .await
            .unwrap();
        let state = store.state.lock().unwrap();
        assert!(state.boards.is_empty());
        let evidence = state.staff_reports[0].board_evidence.as_ref().unwrap();
        assert_eq!(evidence.post_public_id, post_id);
        assert_eq!(evidence.body, "Meet | beyond the gate.");
    }

    #[tokio::test]
    async fn social_delivery_honours_ignore_and_two_way_block_without_policy_leaks() {
        let store = Arc::new(MemoryStore::default());
        let world = WorldHandle::spawn(World::demo(), store).await.unwrap();
        let admin = world
            .register_account("Admin".into(), "correct horse battery staple".into())
            .await
            .unwrap();
        let player = world
            .register_account("Player".into(), "another correct horse battery".into())
            .await
            .unwrap();
        let admin_character = world
            .create_authenticated_character(admin.token.as_str().into(), "Aster".into())
            .await
            .unwrap();
        let player_character = world
            .create_authenticated_character(player.token.as_str().into(), "Bryn".into())
            .await
            .unwrap();
        let admin_entered = world
            .enter_authenticated_character(admin.token.as_str().into(), admin_character.id)
            .await
            .unwrap();
        let mut player_entered = world
            .enter_authenticated_character(player.token.as_str().into(), player_character.id)
            .await
            .unwrap();
        world
            .apply(
                player_entered.session_id,
                PlayerAction::Social(SocialAction::Ignore("Aster".into())),
            )
            .await
            .unwrap();
        world
            .apply(
                admin_entered.session_id,
                PlayerAction::Social(SocialAction::Say("The gate is open.".into())),
            )
            .await
            .unwrap();
        assert!(
            tokio::time::timeout(Duration::from_millis(20), player_entered.deliveries.recv())
                .await
                .is_err()
        );

        world
            .apply(
                player_entered.session_id,
                PlayerAction::Social(SocialAction::Unignore("Aster".into())),
            )
            .await
            .unwrap();
        world
            .apply(
                player_entered.session_id,
                PlayerAction::Social(SocialAction::Block("Aster".into())),
            )
            .await
            .unwrap();
        let acknowledged = world
            .apply(
                admin_entered.session_id,
                PlayerAction::Social(SocialAction::Tell {
                    target: "Bryn".into(),
                    message: "Can you hear me?".into(),
                }),
            )
            .await
            .unwrap();
        assert_eq!(
            acknowledged.output.as_deref(),
            Some("You tell Bryn, \"Can you hear me?\"\r\n")
        );
        assert!(
            tokio::time::timeout(Duration::from_millis(20), player_entered.deliveries.recv())
                .await
                .is_err()
        );
        assert!(matches!(
            world
                .apply(
                    player_entered.session_id,
                    PlayerAction::Social(SocialAction::Reply("No reply target".into())),
                )
                .await,
            Err(GameError::Persistence(StoreError::SocialTargetNotFound))
        ));
    }

    async fn next_social(inbox: &mut DeliveryInbox) -> crate::social::SocialMessage {
        tokio::time::timeout(Duration::from_secs(1), async {
            loop {
                match inbox.recv().await {
                    Some(crate::delivery::SessionDelivery::Social(message)) => break message,
                    Some(_) => continue,
                    None => panic!("delivery inbox closed"),
                }
            }
        })
        .await
        .expect("social delivery timed out")
    }

    async fn has_no_social(inbox: &mut DeliveryInbox) -> bool {
        tokio::time::timeout(Duration::from_millis(20), async {
            loop {
                match inbox.recv().await {
                    Some(crate::delivery::SessionDelivery::Social(_)) => break false,
                    Some(_) => continue,
                    None => break true,
                }
            }
        })
        .await
        .unwrap_or(true)
    }

    fn telepathy_test_world() -> World {
        let loaded = crate::world_loader::default_loaded_world().unwrap();
        crate::world_loader::build_loaded_world(&loaded).unwrap()
    }

    fn emote_test_world() -> World {
        let mut world = World::demo();
        world
            .configure_social_emotes(vec![SocialEmoteDefinition {
                id: "test:smile".into(),
                command: "smile".into(),
                name: "Sonreír".into(),
                audience: SocialEmoteAudience::Room,
                targeting: SocialEmoteTargeting::OptionalCharacter,
                templates: SocialEmoteTemplates {
                    untargeted: Some(SocialEmoteUntargetedTemplates {
                        actor: "Sonríes.".into(),
                        others: "{actor} sonríe.".into(),
                    }),
                    targeted: Some(SocialEmoteTargetedTemplates {
                        actor: "Sonríes a {target}.".into(),
                        target: "{actor} te sonríe.".into(),
                        others: "{actor} sonríe a {target}.".into(),
                    }),
                },
            }])
            .unwrap();
        world
    }

    fn spatial_voice_test_world() -> World {
        let room = |index: u8, exits: Vec<(Direction, u8)>| RoomDefinition {
            stable_id: format!("voice:r{index}"),
            title: format!("Voice room {index}"),
            prose: vec![format!("Bounded voice room {index}.")],
            image: None,
            audio: None,
            item_ids: Vec::new(),
            exits: exits
                .into_iter()
                .map(|(direction, target)| tiamud_core::ExitDefinition {
                    direction,
                    target_room_id: format!("voice:r{target}"),
                })
                .collect(),
        };
        World::from_definition(WorldDefinition {
            start_room_id: "voice:r0".into(),
            item_prototypes: Vec::new(),
            initial_item_instances: Vec::new(),
            rooms: vec![
                room(0, vec![(Direction::East, 1)]),
                room(1, vec![(Direction::West, 0), (Direction::East, 2)]),
                room(2, vec![(Direction::West, 1), (Direction::East, 3)]),
                room(3, vec![(Direction::West, 2), (Direction::East, 4)]),
                room(4, vec![(Direction::West, 3)]),
            ],
        })
        .unwrap()
    }

    async fn create_telepathy_character(
        world: &WorldHandle,
        name: &str,
    ) -> (String, CharacterSnapshot) {
        let authenticated = world
            .register_account(
                format!("{name} account"),
                "correct horse battery staple".into(),
            )
            .await
            .unwrap();
        let character = world
            .create_authenticated_character(authenticated.token.as_str().into(), name.into())
            .await
            .unwrap();
        (authenticated.token.as_str().to_owned(), character)
    }

    async fn enter_telepathy_character(
        world: &WorldHandle,
        token: &str,
        character: CharacterId,
    ) -> EnteredWorld {
        world
            .enter_authenticated_character(token.to_owned(), character)
            .await
            .unwrap()
    }

    fn projected_resource(player: &PlayerStateView, id: &str) -> u32 {
        player
            .character
            .resources
            .iter()
            .find(|resource| resource.id == id)
            .unwrap()
            .current
    }

    fn projected_ability<'a>(
        player: &'a PlayerStateView,
        id: &str,
    ) -> &'a tiamud_core::AbilityView {
        player
            .character
            .abilities
            .iter()
            .find(|ability| ability.id == id)
            .unwrap()
    }

    fn drain_deliveries(inbox: &mut DeliveryInbox) {
        while inbox.try_recv().is_ok() {}
    }

    #[tokio::test]
    async fn spatial_voice_routes_by_radius_and_shares_policy_and_chat_bucket() {
        let store = Arc::new(MemoryStore::default());
        let world = WorldHandle::spawn(spatial_voice_test_world(), store)
            .await
            .unwrap();
        let sender = world.enter("Sender".into()).await.unwrap();
        let mut local = world.enter("Local".into()).await.unwrap();
        let mut near = world.enter("Near".into()).await.unwrap();
        let mut mid = world.enter("Mid".into()).await.unwrap();
        let mut far = world.enter("Far".into()).await.unwrap();
        let mut beyond = world.enter("Beyond".into()).await.unwrap();
        world
            .apply(near.session_id, PlayerAction::Move(Direction::East))
            .await
            .unwrap();
        for _ in 0..2 {
            world
                .apply(mid.session_id, PlayerAction::Move(Direction::East))
                .await
                .unwrap();
        }
        for _ in 0..3 {
            world
                .apply(far.session_id, PlayerAction::Move(Direction::East))
                .await
                .unwrap();
        }
        for _ in 0..4 {
            world
                .apply(beyond.session_id, PlayerAction::Move(Direction::East))
                .await
                .unwrap();
        }

        let yelled = world
            .apply(
                sender.session_id,
                PlayerAction::Social(SocialAction::Yell("Near warning".into())),
            )
            .await
            .unwrap();
        assert_eq!(
            yelled.output.as_deref(),
            Some("You yell, \"Near warning\"\r\n")
        );
        for inbox in [&mut local.deliveries, &mut near.deliveries] {
            let message = next_social(inbox).await;
            assert_eq!(message.channel, crate::social::SocialChannel::Yell);
            assert_eq!(message.sender, "Sender");
            assert_eq!(message.text, "Near warning");
        }
        for inbox in [
            &mut mid.deliveries,
            &mut far.deliveries,
            &mut beyond.deliveries,
        ] {
            assert!(has_no_social(inbox).await);
        }

        world
            .apply(
                sender.session_id,
                PlayerAction::Social(SocialAction::Shout("Three-room warning".into())),
            )
            .await
            .unwrap();
        for inbox in [
            &mut local.deliveries,
            &mut near.deliveries,
            &mut mid.deliveries,
            &mut far.deliveries,
        ] {
            let message = next_social(inbox).await;
            assert_eq!(message.channel, crate::social::SocialChannel::Shout);
            assert_eq!(message.text, "Three-room warning");
        }
        assert!(has_no_social(&mut beyond.deliveries).await);

        world
            .apply(
                local.session_id,
                PlayerAction::Social(SocialAction::Ignore("Sender".into())),
            )
            .await
            .unwrap();
        world
            .apply(
                sender.session_id,
                PlayerAction::Social(SocialAction::Yell("Policy warning".into())),
            )
            .await
            .unwrap();
        assert!(has_no_social(&mut local.deliveries).await);
        assert_eq!(
            next_social(&mut near.deliveries).await.text,
            "Policy warning"
        );

        world
            .apply(
                near.session_id,
                PlayerAction::Social(SocialAction::Block("Sender".into())),
            )
            .await
            .unwrap();
        world
            .apply(
                sender.session_id,
                PlayerAction::Social(SocialAction::Shout("Blocked warning".into())),
            )
            .await
            .unwrap();
        assert!(has_no_social(&mut local.deliveries).await);
        assert!(has_no_social(&mut near.deliveries).await);
        for inbox in [&mut mid.deliveries, &mut far.deliveries] {
            assert_eq!(next_social(inbox).await.text, "Blocked warning");
        }
        assert!(has_no_social(&mut beyond.deliveries).await);

        assert!(matches!(
            world
                .apply(
                    sender.session_id,
                    PlayerAction::Social(SocialAction::Yell(
                        "a".repeat(crate::social::MAX_CHAT_MESSAGE_BYTES + 1)
                    )),
                )
                .await,
            Err(GameError::Persistence(StoreError::InvalidSocialInput))
        ));
        assert!(matches!(
            world
                .apply(
                    sender.session_id,
                    PlayerAction::Social(SocialAction::Yell("One too many".into())),
                )
                .await,
            Err(GameError::Persistence(StoreError::ChatRateLimited))
        ));
    }

    #[tokio::test]
    async fn staff_announcement_requires_the_exact_capability_and_fans_out_once() {
        let store = Arc::new(MemoryStore::default());
        let world = WorldHandle::spawn(World::demo(), store.clone())
            .await
            .unwrap();
        let admin = world
            .register_account("Admin".into(), "correct horse battery staple".into())
            .await
            .unwrap();
        let player = world
            .register_account("Player".into(), "another correct horse battery".into())
            .await
            .unwrap();
        let admin_character = world
            .create_authenticated_character(admin.token.as_str().into(), "Aster".into())
            .await
            .unwrap();
        let player_character = world
            .create_authenticated_character(player.token.as_str().into(), "Bryn".into())
            .await
            .unwrap();
        let mut admin_entered = world
            .enter_authenticated_character(admin.token.as_str().into(), admin_character.id)
            .await
            .unwrap();
        let mut player_entered = world
            .enter_authenticated_character(player.token.as_str().into(), player_character.id)
            .await
            .unwrap();

        assert!(matches!(
            world
                .apply(
                    player_entered.session_id,
                    PlayerAction::Staff(tiamud_core::StaffAction::Announce {
                        message: "forged".into(),
                        reason: "should be denied".into(),
                    }),
                )
                .await,
            Err(GameError::Persistence(StoreError::StaffForbidden))
        ));

        world
            .apply(
                admin_entered.session_id,
                PlayerAction::Staff(tiamud_core::StaffAction::Announce {
                    message: "The gates close at dusk.".into(),
                    reason: "scheduled event".into(),
                }),
            )
            .await
            .unwrap();
        for inbox in [
            &mut admin_entered.deliveries,
            &mut player_entered.deliveries,
        ] {
            let announcement = next_social(inbox).await;
            assert_eq!(
                announcement.channel,
                crate::social::SocialChannel::Announcement
            );
            assert_eq!(announcement.text, "The gates close at dusk.");
            assert!(inbox.try_recv().is_err());
        }
        let state = store.state.lock().unwrap();
        assert_eq!(
            state
                .staff_audit
                .iter()
                .filter(|entry| entry.action == "staff_announce")
                .count(),
            1
        );
    }

    #[tokio::test]
    async fn staff_area_retire_requires_capability_and_writes_no_audit_when_denied() {
        let store = Arc::new(MemoryStore::default());
        let world = WorldHandle::spawn(World::demo(), store.clone())
            .await
            .unwrap();
        let admin = world
            .register_account("Admin".into(), "correct horse battery staple".into())
            .await
            .unwrap();
        let player = world
            .register_account("Player".into(), "another correct horse battery".into())
            .await
            .unwrap();
        let _admin_character = world
            .create_authenticated_character(admin.token.as_str().into(), "Aster".into())
            .await
            .unwrap();
        let player_character = world
            .create_authenticated_character(player.token.as_str().into(), "Bryn".into())
            .await
            .unwrap();
        let player_entered = world
            .enter_authenticated_character(player.token.as_str().into(), player_character.id)
            .await
            .unwrap();

        assert!(matches!(
            world
                .apply(
                    player_entered.session_id,
                    PlayerAction::Staff(tiamud_core::StaffAction::RetireArea {
                        area: "square".into(),
                        reason: "cleanup".into(),
                    }),
                )
                .await,
            Err(GameError::Persistence(StoreError::StaffForbidden))
        ));

        assert_eq!(
            store
                .state
                .lock()
                .unwrap()
                .staff_audit
                .iter()
                .filter(|entry| entry.action == "staff_area_retire")
                .count(),
            0
        );
    }

    #[tokio::test]
    async fn staff_area_retire_maps_missing_router_to_invalid_without_audit() {
        let store = Arc::new(MemoryStore::default());
        let world = WorldHandle::spawn(World::demo(), store.clone())
            .await
            .unwrap();
        // The first registered account is provisioned as `implementor`, which
        // holds `StaffAssign`.
        let admin = world
            .register_account("Admin".into(), "correct horse battery staple".into())
            .await
            .unwrap();
        let admin_character = world
            .create_authenticated_character(admin.token.as_str().into(), "Aster".into())
            .await
            .unwrap();
        let admin_entered = world
            .enter_authenticated_character(admin.token.as_str().into(), admin_character.id)
            .await
            .unwrap();

        assert!(matches!(
            world
                .apply(
                    admin_entered.session_id,
                    PlayerAction::Staff(tiamud_core::StaffAction::RetireArea {
                        area: "square".into(),
                        reason: "cleanup".into(),
                    }),
                )
                .await,
            Err(GameError::AreaOperationInvalid)
        ));

        assert_eq!(
            store
                .state
                .lock()
                .unwrap()
                .staff_audit
                .iter()
                .filter(|entry| entry.action == "staff_area_retire")
                .count(),
            0
        );
    }

    #[tokio::test]
    async fn failed_staff_relocation_rolls_back_world_and_audit_together() {
        let store = Arc::new(MemoryStore::default());
        let world = WorldHandle::spawn(World::demo(), store.clone())
            .await
            .unwrap();
        let admin = world
            .register_account("Admin".into(), "correct horse battery staple".into())
            .await
            .unwrap();
        let player = world
            .register_account("Player".into(), "another correct horse battery".into())
            .await
            .unwrap();
        let admin_character = world
            .create_authenticated_character(admin.token.as_str().into(), "Aster".into())
            .await
            .unwrap();
        let player_character = world
            .create_authenticated_character(player.token.as_str().into(), "Bryn".into())
            .await
            .unwrap();
        let mut admin_entered = world
            .enter_authenticated_character(admin.token.as_str().into(), admin_character.id)
            .await
            .unwrap();
        let mut player_entered = world
            .enter_authenticated_character(player.token.as_str().into(), player_character.id)
            .await
            .unwrap();
        world
            .apply(
                player_entered.session_id,
                PlayerAction::Move(Direction::North),
            )
            .await
            .unwrap();
        let before = world
            .current_state(admin_entered.session_id)
            .await
            .unwrap()
            .0;
        let audit_before = store.state.lock().unwrap().staff_audit.len();
        drain_deliveries(&mut admin_entered.deliveries);
        drain_deliveries(&mut player_entered.deliveries);
        store.fail_saves.store(true, Ordering::SeqCst);

        assert!(matches!(
            world
                .apply(
                    admin_entered.session_id,
                    PlayerAction::Staff(tiamud_core::StaffAction::Goto {
                        target: "Bryn".into(),
                        reason: "investigate location".into(),
                    }),
                )
                .await,
            Err(GameError::Persistence(StoreError::WorkerStopped))
        ));
        let after = world
            .current_state(admin_entered.session_id)
            .await
            .unwrap()
            .0;
        assert_eq!(after.title, before.title);
        assert_eq!(store.state.lock().unwrap().staff_audit.len(), audit_before);
        assert!(admin_entered.deliveries.try_recv().is_err());
        assert!(player_entered.deliveries.try_recv().is_err());
    }

    #[tokio::test]
    async fn staff_restore_is_dispatched_even_when_the_target_is_defeated() {
        let store = Arc::new(MemoryStore::default());
        let world = WorldHandle::spawn(World::demo(), store.clone())
            .await
            .unwrap();
        let admin = world
            .register_account("Admin".into(), "correct horse battery staple".into())
            .await
            .unwrap();
        let player = world
            .register_account("Player".into(), "another correct horse battery".into())
            .await
            .unwrap();
        let admin_character = world
            .create_authenticated_character(admin.token.as_str().into(), "Aster".into())
            .await
            .unwrap();
        let player_character = world
            .create_authenticated_character(player.token.as_str().into(), "Bryn".into())
            .await
            .unwrap();
        {
            let mut state = store.state.lock().unwrap();
            let gameplay = state.gameplay.get_mut(&player_character.id).unwrap();
            gameplay.vitals.health = 0;
            gameplay.recovery_at_tick = Some(50);
        }
        let admin_entered = world
            .enter_authenticated_character(admin.token.as_str().into(), admin_character.id)
            .await
            .unwrap();
        let player_entered = world
            .enter_authenticated_character(player.token.as_str().into(), player_character.id)
            .await
            .unwrap();

        world
            .apply(
                admin_entered.session_id,
                PlayerAction::Staff(tiamud_core::StaffAction::Restore {
                    target: "Bryn".into(),
                    reason: "recover from a stuck defeat".into(),
                }),
            )
            .await
            .unwrap();
        let player = world
            .current_state(player_entered.session_id)
            .await
            .unwrap()
            .2;
        assert_eq!(player.character.health, player.character.max_health);
        assert_eq!(player.character.recovery_in_ticks, None);
    }

    #[tokio::test]
    async fn memory_staff_profile_rejects_a_bundle_the_assigner_cannot_delegate() {
        let store = MemoryStore::default();
        let admin = store
            .register_account("Admin", "correct horse battery staple")
            .await
            .unwrap();
        let target = store
            .register_account("Player", "another correct horse battery")
            .await
            .unwrap();
        store
            .load_or_create_character(admin.account.id, "Aster", "demo:lantern-square")
            .await
            .unwrap();
        store
            .load_or_create_character(target.account.id, "Bryn", "demo:lantern-square")
            .await
            .unwrap();
        {
            let mut state = store.state.lock().unwrap();
            state
                .staff_capabilities
                .get_mut(&admin.account.id)
                .unwrap()
                .remove(&tiamud_core::StaffCapability::ReportsResolve);
        }
        let audit_before = store.state.lock().unwrap().staff_audit.len();

        let result = store
            .set_staff_profile(
                target.account.id,
                Some(tiamud_core::StaffProfileKey::Helper),
                None,
                crate::persistence::StaffAuditRecord {
                    actor_account_id: admin.account.id,
                    target_account_id: Some(target.account.id),
                    capability: tiamud_core::StaffCapability::StaffAssign,
                    action: "staff_profile_set".to_owned(),
                    reason: "delegate helper duties".to_owned(),
                    details: serde_json::json!({"target_name": "Bryn"}),
                },
            )
            .await;

        assert!(matches!(result, Err(StoreError::StaffForbidden)));
        let state = store.state.lock().unwrap();
        assert!(!state.staff_profiles.contains_key(&target.account.id));
        assert!(!state.staff_capabilities.contains_key(&target.account.id));
        assert_eq!(state.staff_audit.len(), audit_before);
    }

    #[tokio::test]
    async fn memory_staff_report_and_audit_lists_share_the_production_limit() {
        let store = MemoryStore::default();
        let admin = store
            .register_account("Admin", "correct horse battery staple")
            .await
            .unwrap();
        let player = store
            .register_account("Player", "another correct horse battery")
            .await
            .unwrap();
        let admin_character = store
            .load_or_create_character(admin.account.id, "Aster", "demo:lantern-square")
            .await
            .unwrap();
        let player_character = store
            .load_or_create_character(player.account.id, "Bryn", "demo:lantern-square")
            .await
            .unwrap();
        for index in 0..12 {
            store
                .report_character(
                    admin_character.id,
                    player_character.id,
                    &format!("report reason {index}"),
                )
                .await
                .unwrap();
        }

        let reports = store
            .staff_reports(crate::persistence::StaffAuditRecord {
                actor_account_id: admin.account.id,
                target_account_id: None,
                capability: tiamud_core::StaffCapability::ReportsView,
                action: "reports_view".to_owned(),
                reason: "review report queue".to_owned(),
                details: serde_json::json!({"actor_name": "Aster"}),
            })
            .await
            .unwrap();
        assert_eq!(reports.len(), 10);

        for index in 0..12 {
            store
                .append_staff_audit(crate::persistence::StaffAuditRecord {
                    actor_account_id: admin.account.id,
                    target_account_id: None,
                    capability: tiamud_core::StaffCapability::StaffAudit,
                    action: format!("audit_seed_{index}"),
                    reason: "retain bounded audit evidence".to_owned(),
                    details: serde_json::json!({"actor_name": "Aster"}),
                })
                .await
                .unwrap();
        }
        let audit = store
            .staff_audit_entries(crate::persistence::StaffAuditRecord {
                actor_account_id: admin.account.id,
                target_account_id: None,
                capability: tiamud_core::StaffCapability::StaffAudit,
                action: "staff_audit_view".to_owned(),
                reason: "review staff audit".to_owned(),
                details: serde_json::json!({"actor_name": "Aster"}),
            })
            .await
            .unwrap();
        assert_eq!(audit.len(), 10);
    }

    #[tokio::test]
    async fn memory_report_resolution_rejects_a_discordant_audit_target_without_side_effects() {
        let store = MemoryStore::default();
        let admin = store
            .register_account("Admin", "correct horse battery staple")
            .await
            .unwrap();
        let player = store
            .register_account("Player", "another correct horse battery")
            .await
            .unwrap();
        let admin_character = store
            .load_or_create_character(admin.account.id, "Aster", "demo:lantern-square")
            .await
            .unwrap();
        let player_character = store
            .load_or_create_character(player.account.id, "Bryn", "demo:lantern-square")
            .await
            .unwrap();
        store
            .report_character(
                admin_character.id,
                player_character.id,
                "a report requiring review",
            )
            .await
            .unwrap();
        let audit_before = store.state.lock().unwrap().staff_audit.len();

        let discordant = store
            .resolve_staff_report(
                1,
                tiamud_core::StaffReportResolution::NoAction,
                "reviewed without action",
                crate::persistence::StaffAuditRecord {
                    actor_account_id: admin.account.id,
                    target_account_id: Some(admin.account.id),
                    capability: tiamud_core::StaffCapability::ReportsResolve,
                    action: "report_resolve".to_owned(),
                    reason: "resolve report".to_owned(),
                    details: serde_json::json!({"actor_name": "Aster"}),
                },
            )
            .await;
        assert!(matches!(discordant, Err(StoreError::StaffForbidden)));
        {
            let state = store.state.lock().unwrap();
            assert!(!state.resolved_staff_reports.contains(&1));
            assert_eq!(state.staff_audit.len(), audit_before);
        }

        store
            .resolve_staff_report(
                1,
                tiamud_core::StaffReportResolution::NoAction,
                "reviewed without action",
                crate::persistence::StaffAuditRecord {
                    actor_account_id: admin.account.id,
                    target_account_id: None,
                    capability: tiamud_core::StaffCapability::ReportsResolve,
                    action: "report_resolve".to_owned(),
                    reason: "resolve report".to_owned(),
                    details: serde_json::json!({"actor_name": "Aster"}),
                },
            )
            .await
            .unwrap();
        let state = store.state.lock().unwrap();
        assert!(state.resolved_staff_reports.contains(&1));
        assert_eq!(state.staff_audit.len(), audit_before + 1);
        assert_eq!(
            state.staff_audit.last().unwrap().target_account_id,
            Some(player.account.id)
        );
    }

    #[tokio::test]
    async fn failed_on_talk_commit_does_not_publish_the_social_plan() {
        let store = Arc::new(MemoryStore::default());
        let content = RuntimeContent::new(
            Vec::new(),
            vec![RuntimeScript {
                id: "script:transactional-talk".into(),
                room_id: "demo:lantern-square".into(),
                hooks: BTreeSet::from([ScriptHook::OnTalk]),
                capabilities: BTreeSet::from([
                    ScriptCapability::Narrate,
                    ScriptCapability::SetFlag,
                ]),
                limits: ScriptLimits::default(),
                source: r#"
                    function on_talk(ctx)
                        tiamud.emit('narrate', 'self', 'The square answers once.')
                        tiamud.emit('set_flag', 'self', 'spoke')
                    end
                "#
                .into(),
            }],
            11,
        );
        let world = WorldHandle::spawn_with_scheduler_and_content(
            World::demo(),
            store.clone(),
            WorldSchedulerConfig::default(),
            content,
        )
        .await
        .unwrap();
        let sender = world.enter("Sender".into()).await.unwrap();
        let mut recipient = world.enter("Recipient".into()).await.unwrap();
        let mut remote = world.enter("Remote".into()).await.unwrap();
        world
            .apply(remote.session_id, PlayerAction::Move(Direction::North))
            .await
            .unwrap();
        store.fail_saves.store(true, Ordering::SeqCst);

        assert!(matches!(
            world
                .apply(
                    sender.session_id,
                    PlayerAction::Social(SocialAction::Yell("not committed".into())),
                )
                .await,
            Err(GameError::Persistence(_))
        ));
        assert!(has_no_social(&mut recipient.deliveries).await);
        assert!(has_no_social(&mut remote.deliveries).await);

        store.fail_saves.store(false, Ordering::SeqCst);
        let applied = world
            .apply(
                sender.session_id,
                PlayerAction::Social(SocialAction::Yell("committed".into())),
            )
            .await
            .unwrap();
        assert_eq!(
            applied
                .output
                .as_deref()
                .unwrap()
                .matches("The square answers once.")
                .count(),
            1
        );
        assert_eq!(
            next_social(&mut recipient.deliveries).await.text,
            "committed"
        );
        assert_eq!(next_social(&mut remote.deliveries).await.text, "committed");
    }

    #[tokio::test]
    async fn authored_emotes_list_without_spending_chat_and_route_distinct_local_perspectives() {
        let store = Arc::new(MemoryStore::default());
        let world = WorldHandle::spawn(emote_test_world(), store.clone())
            .await
            .unwrap();
        let sender = world.enter("Sender".into()).await.unwrap();
        let mut target = world.enter("Target".into()).await.unwrap();
        let mut observer = world.enter("Observer".into()).await.unwrap();
        let mut remote = world.enter("Remote".into()).await.unwrap();
        world
            .apply(remote.session_id, PlayerAction::Move(Direction::North))
            .await
            .unwrap();

        for _ in 0..8 {
            let listed = world
                .apply(
                    sender.session_id,
                    PlayerAction::Social(SocialAction::EmoteList),
                )
                .await
                .unwrap();
            assert_eq!(
                listed.output.as_deref(),
                Some("Authored emotes:\r\n- smile: Sonreír (optional character)\r\n")
            );
        }
        let events_before = store.state.lock().unwrap().domain_events.len();
        let applied = world
            .apply(
                sender.session_id,
                PlayerAction::Social(SocialAction::Emote {
                    command: "smile".into(),
                    target: Some("target".into()),
                }),
            )
            .await
            .unwrap();
        assert_eq!(applied.output.as_deref(), Some("Sonríes a Target.\r\n"));

        assert_eq!(
            next_social(&mut target.deliveries).await,
            SocialMessage {
                channel: SocialChannel::Emote,
                sender: "Sender".into(),
                text: "Sender te sonríe.".into(),
            }
        );
        assert_eq!(
            next_social(&mut observer.deliveries).await,
            SocialMessage {
                channel: SocialChannel::Emote,
                sender: "Sender".into(),
                text: "Sender sonríe a Target.".into(),
            }
        );
        assert!(has_no_social(&mut remote.deliveries).await);
        assert_eq!(
            store.state.lock().unwrap().domain_events.len(),
            events_before
        );
        assert!(matches!(
            world
                .apply(
                    target.session_id,
                    PlayerAction::Social(SocialAction::Reply("No plain reply target".into())),
                )
                .await,
            Err(GameError::Persistence(StoreError::SocialTargetNotFound))
        ));

        for unavailable_target in ["Sender", "Remote", "Nobody"] {
            assert!(
                world
                    .apply(
                        sender.session_id,
                        PlayerAction::Social(SocialAction::Emote {
                            command: "smile".into(),
                            target: Some(unavailable_target.into()),
                        }),
                    )
                    .await
                    .is_err(),
                "{unavailable_target}"
            );
        }
        assert!(has_no_social(&mut target.deliveries).await);
        assert!(has_no_social(&mut observer.deliveries).await);
    }

    #[tokio::test]
    async fn private_emote_suppression_does_not_change_actor_confirmation_or_other_observers() {
        let store = Arc::new(MemoryStore::default());
        let world = WorldHandle::spawn(emote_test_world(), store.clone())
            .await
            .unwrap();
        let sender = world.enter("Sender".into()).await.unwrap();
        let mut target = world.enter("Target".into()).await.unwrap();
        let mut observer = world.enter("Observer".into()).await.unwrap();
        let (sender_id, target_id) = {
            let state = store.state.lock().unwrap();
            let id = |name: &str| {
                state
                    .characters
                    .values()
                    .find(|character| character.name == name)
                    .unwrap()
                    .id
            };
            (id("Sender"), id("Target"))
        };
        store
            .state
            .lock()
            .unwrap()
            .ignores
            .insert((target_id, sender_id));

        let applied = world
            .apply(
                sender.session_id,
                PlayerAction::Social(SocialAction::Emote {
                    command: "smile".into(),
                    target: Some("Target".into()),
                }),
            )
            .await
            .unwrap();
        assert_eq!(applied.output.as_deref(), Some("Sonríes a Target.\r\n"));
        assert!(has_no_social(&mut target.deliveries).await);
        assert_eq!(
            next_social(&mut observer.deliveries).await.text,
            "Sender sonríe a Target."
        );

        store.state.lock().unwrap().muted.insert(sender.account_id);
        assert!(matches!(
            world
                .apply(
                    sender.session_id,
                    PlayerAction::Social(SocialAction::Emote {
                        command: "smile".into(),
                        target: None,
                    }),
                )
                .await,
            Err(GameError::Persistence(StoreError::ChatMuted))
        ));
        assert!(has_no_social(&mut target.deliveries).await);
        assert!(has_no_social(&mut observer.deliveries).await);
    }

    #[tokio::test]
    async fn emote_on_talk_is_inert_on_script_rejection_but_commit_failure_delivers_nothing() {
        for (source, commit_fails) in [
            (
                "function on_talk(ctx) tiamud.emit('set_flag', 'another', 'rejected') end",
                false,
            ),
            (
                "function on_talk(ctx) tiamud.emit('set_flag', 'self', 'emoted') end",
                true,
            ),
        ] {
            let store = Arc::new(MemoryStore::default());
            let content = RuntimeContent::new(
                Vec::new(),
                vec![RuntimeScript {
                    id: "script:emote-talk".into(),
                    room_id: "demo:lantern-square".into(),
                    hooks: BTreeSet::from([ScriptHook::OnTalk]),
                    capabilities: BTreeSet::from([ScriptCapability::SetFlag]),
                    limits: ScriptLimits::default(),
                    source: source.into(),
                }],
                23,
            );
            let world = WorldHandle::spawn_with_scheduler_and_content(
                emote_test_world(),
                store.clone(),
                WorldSchedulerConfig::default(),
                content,
            )
            .await
            .unwrap();
            let sender = world.enter("Sender".into()).await.unwrap();
            let mut observer = world.enter("Observer".into()).await.unwrap();
            store.fail_saves.store(commit_fails, Ordering::SeqCst);

            let result = world
                .apply(
                    sender.session_id,
                    PlayerAction::Social(SocialAction::Emote {
                        command: "smile".into(),
                        target: None,
                    }),
                )
                .await;
            if commit_fails {
                assert!(matches!(result, Err(GameError::Persistence(_))));
                assert!(has_no_social(&mut observer.deliveries).await);
            } else {
                assert_eq!(result.unwrap().output.as_deref(), Some("Sonríes.\r\n"));
                assert_eq!(
                    next_social(&mut observer.deliveries).await.text,
                    "Sender sonríe."
                );
            }
        }
    }

    #[tokio::test]
    async fn telepathy_consumes_authored_costs_commits_then_delivers_without_reply_or_event() {
        let store = Arc::new(MemoryStore::default());
        let world = WorldHandle::spawn(telepathy_test_world(), store.clone())
            .await
            .unwrap();
        let (sender_token, sender_character) = create_telepathy_character(&world, "Sender").await;
        let (recipient_token, recipient_character) =
            create_telepathy_character(&world, "Recipient").await;
        let sender = enter_telepathy_character(&world, &sender_token, sender_character.id).await;
        let mut recipient =
            enter_telepathy_character(&world, &recipient_token, recipient_character.id).await;
        let before = world.current_state(sender.session_id).await.unwrap().2;

        let applied = world
            .apply(
                sender.session_id,
                PlayerAction::Telepathy {
                    ability_id: "tiamud:mind-whisper".into(),
                    target: "Recipient".into(),
                    message: "Meet beyond the gate".into(),
                },
            )
            .await
            .unwrap();
        assert_eq!(
            applied.output.as_deref(),
            Some("You reach for Recipient with Mind Whisper, \"Meet beyond the gate\".\r\n")
        );
        assert_eq!(
            projected_resource(&applied.player, "stamina"),
            projected_resource(&before, "stamina") - 2
        );
        let ability = projected_ability(&applied.player, "tiamud:mind-whisper");
        assert_eq!(ability.cooldown_remaining, 2);
        assert!(!ability.usable);

        let delivered = next_social(&mut recipient.deliveries).await;
        assert_eq!(delivered.channel, SocialChannel::Telepathy);
        assert_eq!(delivered.sender, "Sender");
        assert_eq!(delivered.text, "Meet beyond the gate");
        assert!(matches!(
            world
                .apply(
                    recipient.session_id,
                    PlayerAction::Social(SocialAction::Reply("I hear you".into())),
                )
                .await,
            Err(GameError::Persistence(StoreError::SocialTargetNotFound))
        ));

        let state = store.state.lock().unwrap();
        assert!(state.domain_events.is_empty());
        let persisted = state.gameplay.get(&sender_character.id).unwrap();
        assert_eq!(
            persisted.resources["stamina"].current,
            projected_resource(&before, "stamina") - 2
        );
        assert_eq!(persisted.ability_cooldowns["tiamud:mind-whisper"], 2);
        assert_eq!(persisted.ability_global_ready_tick, 1);
    }

    #[derive(Clone, Copy, Debug)]
    enum PrivateTelepathyBarrier {
        Ignore,
        Block,
        Defeated,
        BlockedRoom,
    }

    #[tokio::test]
    async fn private_receiver_barriers_are_indistinguishable_and_still_consume() {
        for barrier in [
            PrivateTelepathyBarrier::Ignore,
            PrivateTelepathyBarrier::Block,
            PrivateTelepathyBarrier::Defeated,
            PrivateTelepathyBarrier::BlockedRoom,
        ] {
            let store = Arc::new(MemoryStore::default());
            let world = WorldHandle::spawn(telepathy_test_world(), store.clone())
                .await
                .unwrap();
            let (sender_token, sender_character) =
                create_telepathy_character(&world, "Sender").await;
            let (recipient_token, recipient_character) =
                create_telepathy_character(&world, "Recipient").await;
            {
                let mut state = store.state.lock().unwrap();
                match barrier {
                    PrivateTelepathyBarrier::Ignore => {
                        state
                            .ignores
                            .insert((recipient_character.id, sender_character.id));
                    }
                    PrivateTelepathyBarrier::Block => {
                        state
                            .blocks
                            .insert((recipient_character.id, sender_character.id));
                    }
                    PrivateTelepathyBarrier::Defeated => {
                        let target = state.gameplay.get_mut(&recipient_character.id).unwrap();
                        target.vitals.health = 0;
                        target.recovery_at_tick = Some(3);
                    }
                    PrivateTelepathyBarrier::BlockedRoom => {
                        state
                            .characters
                            .get_mut(&recipient_character.id)
                            .unwrap()
                            .room_id = "demo:archive".into();
                    }
                }
            }
            let sender =
                enter_telepathy_character(&world, &sender_token, sender_character.id).await;
            let mut recipient =
                enter_telepathy_character(&world, &recipient_token, recipient_character.id).await;
            let before = world.current_state(sender.session_id).await.unwrap().2;

            let applied = world
                .apply(
                    sender.session_id,
                    PlayerAction::Telepathy {
                        ability_id: "tiamud:mind-whisper".into(),
                        target: "Recipient".into(),
                        message: "Still there?".into(),
                    },
                )
                .await
                .unwrap();
            assert_eq!(
                applied.output.as_deref(),
                Some("You reach for Recipient with Mind Whisper, \"Still there?\".\r\n"),
                "{barrier:?}"
            );
            assert_eq!(
                projected_resource(&applied.player, "stamina"),
                projected_resource(&before, "stamina") - 2,
                "{barrier:?}"
            );
            assert_eq!(
                projected_ability(&applied.player, "tiamud:mind-whisper").cooldown_remaining,
                2,
                "{barrier:?}"
            );
            assert!(
                tokio::time::timeout(Duration::from_millis(20), recipient.deliveries.recv())
                    .await
                    .is_err(),
                "{barrier:?}"
            );
        }
    }

    #[tokio::test]
    async fn invalid_or_muted_telepathy_fails_before_consuming_resources() {
        let store = Arc::new(MemoryStore::default());
        let world = WorldHandle::spawn(telepathy_test_world(), store.clone())
            .await
            .unwrap();
        let (sender_token, sender_character) = create_telepathy_character(&world, "Sender").await;
        let (recipient_token, recipient_character) =
            create_telepathy_character(&world, "Recipient").await;
        let sender = enter_telepathy_character(&world, &sender_token, sender_character.id).await;
        let mut recipient =
            enter_telepathy_character(&world, &recipient_token, recipient_character.id).await;
        let before = world.current_state(sender.session_id).await.unwrap().2;

        for target in ["Sender", "Nobody"] {
            assert!(
                world
                    .apply(
                        sender.session_id,
                        PlayerAction::Telepathy {
                            ability_id: "tiamud:mind-whisper".into(),
                            target: target.into(),
                            message: "No cost".into(),
                        },
                    )
                    .await
                    .is_err()
            );
            assert_eq!(
                world.current_state(sender.session_id).await.unwrap().2,
                before
            );
        }

        store.state.lock().unwrap().muted.insert(sender.account_id);
        assert!(matches!(
            world
                .apply(
                    sender.session_id,
                    PlayerAction::Telepathy {
                        ability_id: "tiamud:mind-whisper".into(),
                        target: "Recipient".into(),
                        message: "Muted".into(),
                    },
                )
                .await,
            Err(GameError::Persistence(StoreError::ChatMuted))
        ));
        assert_eq!(
            world.current_state(sender.session_id).await.unwrap().2,
            before
        );
        assert!(
            tokio::time::timeout(Duration::from_millis(20), recipient.deliveries.recv())
                .await
                .is_err()
        );
    }

    #[tokio::test]
    async fn telepathy_store_failure_rolls_back_and_never_delivers() {
        let store = Arc::new(MemoryStore::default());
        let world = WorldHandle::spawn(telepathy_test_world(), store.clone())
            .await
            .unwrap();
        let (sender_token, sender_character) = create_telepathy_character(&world, "Sender").await;
        let (recipient_token, recipient_character) =
            create_telepathy_character(&world, "Recipient").await;
        let sender = enter_telepathy_character(&world, &sender_token, sender_character.id).await;
        let mut recipient =
            enter_telepathy_character(&world, &recipient_token, recipient_character.id).await;
        let before = world.current_state(sender.session_id).await.unwrap().2;
        store.fail_saves.store(true, Ordering::SeqCst);

        assert!(matches!(
            world
                .apply(
                    sender.session_id,
                    PlayerAction::Telepathy {
                        ability_id: "tiamud:mind-whisper".into(),
                        target: "Recipient".into(),
                        message: "Do not deliver".into(),
                    },
                )
                .await,
            Err(GameError::Persistence(_))
        ));
        assert_eq!(
            world.current_state(sender.session_id).await.unwrap().2,
            before
        );
        assert!(
            tokio::time::timeout(Duration::from_millis(20), recipient.deliveries.recv())
                .await
                .is_err()
        );

        store.fail_saves.store(false, Ordering::SeqCst);
        world
            .apply(
                sender.session_id,
                PlayerAction::Telepathy {
                    ability_id: "tiamud:mind-whisper".into(),
                    target: "Recipient".into(),
                    message: "Now deliver".into(),
                },
            )
            .await
            .unwrap();
        assert_eq!(
            next_social(&mut recipient.deliveries).await.text,
            "Now deliver"
        );
    }

    #[tokio::test]
    async fn closed_receiver_mailbox_does_not_reveal_itself_or_refund_telepathy() {
        let store = Arc::new(MemoryStore::default());
        let world = WorldHandle::spawn(telepathy_test_world(), store)
            .await
            .unwrap();
        let (sender_token, sender_character) = create_telepathy_character(&world, "Sender").await;
        let (recipient_token, recipient_character) =
            create_telepathy_character(&world, "Recipient").await;
        let sender = enter_telepathy_character(&world, &sender_token, sender_character.id).await;
        let recipient =
            enter_telepathy_character(&world, &recipient_token, recipient_character.id).await;
        drop(recipient.deliveries);
        let before = world.current_state(sender.session_id).await.unwrap().2;

        let applied = world
            .apply(
                sender.session_id,
                PlayerAction::Telepathy {
                    ability_id: "tiamud:mind-whisper".into(),
                    target: "Recipient".into(),
                    message: "Are you there?".into(),
                },
            )
            .await
            .unwrap();
        assert_eq!(
            applied.output.as_deref(),
            Some("You reach for Recipient with Mind Whisper, \"Are you there?\".\r\n")
        );
        assert_eq!(
            projected_resource(&applied.player, "stamina"),
            projected_resource(&before, "stamina") - 2
        );
        assert_eq!(
            projected_ability(&applied.player, "tiamud:mind-whisper").cooldown_remaining,
            2
        );
    }

    #[tokio::test]
    async fn rejected_on_talk_rolls_back_telepathy_and_never_delivers() {
        let store = Arc::new(MemoryStore::default());
        let content = RuntimeContent::new(
            Vec::new(),
            vec![RuntimeScript {
                id: "script:reject-telepathy".into(),
                room_id: "demo:lantern-square".into(),
                hooks: BTreeSet::from([ScriptHook::OnTalk]),
                capabilities: BTreeSet::from([ScriptCapability::SetFlag]),
                limits: ScriptLimits::default(),
                source: r#"
                    function on_talk(ctx)
                        tiamud.emit('set_flag', 'another', 'must-fail-closed')
                    end
                "#
                .into(),
            }],
            19,
        );
        let world = WorldHandle::spawn_with_scheduler_and_content(
            telepathy_test_world(),
            store,
            WorldSchedulerConfig::default(),
            content,
        )
        .await
        .unwrap();
        let (sender_token, sender_character) = create_telepathy_character(&world, "Sender").await;
        let (recipient_token, recipient_character) =
            create_telepathy_character(&world, "Recipient").await;
        let sender = enter_telepathy_character(&world, &sender_token, sender_character.id).await;
        let mut recipient =
            enter_telepathy_character(&world, &recipient_token, recipient_character.id).await;
        let before = world.current_state(sender.session_id).await.unwrap().2;

        assert!(matches!(
            world
                .apply(
                    sender.session_id,
                    PlayerAction::Telepathy {
                        ability_id: "tiamud:mind-whisper".into(),
                        target: "Recipient".into(),
                        message: "Rejected".into(),
                    },
                )
                .await,
            Err(GameError::ScriptHookRejected)
        ));
        assert_eq!(
            world.current_state(sender.session_id).await.unwrap().2,
            before
        );
        assert!(
            tokio::time::timeout(Duration::from_millis(20), recipient.deliveries.recv())
                .await
                .is_err()
        );
    }

    #[tokio::test]
    async fn party_chat_and_reply_route_only_to_the_intended_characters() {
        let store = Arc::new(MemoryStore::default());
        let world = WorldHandle::spawn(World::demo(), store).await.unwrap();
        let mut leader = world.enter("Aster".into()).await.unwrap();
        let mut member = world.enter("Bryn".into()).await.unwrap();
        let mut outsider = world.enter("Cato".into()).await.unwrap();

        world
            .apply(leader.session_id, PlayerAction::Party(PartyAction::Create))
            .await
            .unwrap();
        world
            .apply(
                leader.session_id,
                PlayerAction::Party(PartyAction::Invite("Bryn".into())),
            )
            .await
            .unwrap();
        world
            .apply(
                member.session_id,
                PlayerAction::Party(PartyAction::Accept("Aster".into())),
            )
            .await
            .unwrap();
        drain_deliveries(&mut leader.deliveries);
        drain_deliveries(&mut member.deliveries);
        drain_deliveries(&mut outsider.deliveries);

        world
            .apply(
                leader.session_id,
                PlayerAction::Social(SocialAction::PartySay("Rally at the gate".into())),
            )
            .await
            .unwrap();
        let party = next_social(&mut member.deliveries).await;
        assert_eq!(party.channel, crate::social::SocialChannel::Party);
        assert_eq!(party.sender, "Aster");
        assert_eq!(party.text, "Rally at the gate");
        assert!(
            tokio::time::timeout(Duration::from_millis(20), outsider.deliveries.recv())
                .await
                .is_err()
        );

        world
            .apply(
                leader.session_id,
                PlayerAction::Social(SocialAction::Tell {
                    target: "Bryn".into(),
                    message: "Ready?".into(),
                }),
            )
            .await
            .unwrap();
        assert_eq!(next_social(&mut member.deliveries).await.text, "Ready?");
        world
            .apply(
                member.session_id,
                PlayerAction::Social(SocialAction::Reply("Ready.".into())),
            )
            .await
            .unwrap();
        let reply = next_social(&mut leader.deliveries).await;
        assert_eq!(reply.channel, crate::social::SocialChannel::Tell);
        assert_eq!(reply.sender, "Bryn");
        assert_eq!(reply.text, "Ready.");
    }

    #[tokio::test]
    async fn durable_channels_enforce_membership_and_live_staff_roles() {
        let store = Arc::new(MemoryStore::default());
        let world = WorldHandle::spawn(World::demo(), store.clone())
            .await
            .unwrap();
        let admin = world
            .register_account("Admin".into(), "correct horse battery staple".into())
            .await
            .unwrap();
        let player = world
            .register_account("Player".into(), "another correct horse battery".into())
            .await
            .unwrap();
        let outsider_account = world
            .register_account("Outsider".into(), "a third correct horse battery".into())
            .await
            .unwrap();
        let admin_character = world
            .create_authenticated_character(admin.token.as_str().into(), "Aster".into())
            .await
            .unwrap();
        let player_character = world
            .create_authenticated_character(player.token.as_str().into(), "Bryn".into())
            .await
            .unwrap();
        let outsider_character = world
            .create_authenticated_character(outsider_account.token.as_str().into(), "Cato".into())
            .await
            .unwrap();
        let mut admin_entered = world
            .enter_authenticated_character(admin.token.as_str().into(), admin_character.id)
            .await
            .unwrap();
        let mut player_entered = world
            .enter_authenticated_character(player.token.as_str().into(), player_character.id)
            .await
            .unwrap();
        let mut outsider_entered = world
            .enter_authenticated_character(
                outsider_account.token.as_str().into(),
                outsider_character.id,
            )
            .await
            .unwrap();

        for session in [admin_entered.session_id, player_entered.session_id] {
            world
                .apply(
                    session,
                    PlayerAction::Social(SocialAction::ChannelJoin(SocialChannelName::General)),
                )
                .await
                .unwrap();
        }
        world
            .apply(
                admin_entered.session_id,
                PlayerAction::Social(SocialAction::ChannelSay {
                    channel: SocialChannelName::General,
                    message: "Welcome everyone".into(),
                }),
            )
            .await
            .unwrap();
        let general = next_social(&mut player_entered.deliveries).await;
        assert_eq!(general.channel, crate::social::SocialChannel::General);
        assert!(
            tokio::time::timeout(
                Duration::from_millis(20),
                outsider_entered.deliveries.recv()
            )
            .await
            .is_err()
        );
        assert!(matches!(
            world
                .apply(
                    player_entered.session_id,
                    PlayerAction::Social(SocialAction::ChannelJoin(SocialChannelName::Staff)),
                )
                .await,
            Err(GameError::Persistence(StoreError::SocialForbidden))
        ));

        world
            .apply(
                admin_entered.session_id,
                PlayerAction::Staff(tiamud_core::StaffAction::SetProfile {
                    target: "Bryn".into(),
                    profile: Some(tiamud_core::StaffProfileKey::Moderator),
                    lore_title: None,
                    reason: "staff channel test".into(),
                }),
            )
            .await
            .unwrap();
        for session in [admin_entered.session_id, player_entered.session_id] {
            world
                .apply(
                    session,
                    PlayerAction::Social(SocialAction::ChannelJoin(SocialChannelName::Staff)),
                )
                .await
                .unwrap();
        }
        world
            .apply(
                player_entered.session_id,
                PlayerAction::Social(SocialAction::ChannelSay {
                    channel: SocialChannelName::Staff,
                    message: "Live role update works".into(),
                }),
            )
            .await
            .unwrap();
        let staff = next_social(&mut admin_entered.deliveries).await;
        assert_eq!(staff.channel, crate::social::SocialChannel::Staff);

        world
            .apply(
                admin_entered.session_id,
                PlayerAction::Staff(tiamud_core::StaffAction::SetProfile {
                    target: "Bryn".into(),
                    profile: None,
                    lore_title: None,
                    reason: "staff channel test demotion".into(),
                }),
            )
            .await
            .unwrap();
        assert!(matches!(
            world
                .apply(
                    player_entered.session_id,
                    PlayerAction::Social(SocialAction::ChannelSay {
                        channel: SocialChannelName::Staff,
                        message: "Should be denied".into(),
                    }),
                )
                .await,
            Err(GameError::Persistence(StoreError::SocialForbidden))
        ));
        let list = world
            .apply(
                player_entered.session_id,
                PlayerAction::Social(SocialAction::ChannelList),
            )
            .await
            .unwrap();
        assert!(list.output.unwrap().contains("staff: unavailable"));
        assert_eq!(
            store
                .social_channel_subscriptions(player_character.id)
                .await
                .unwrap(),
            vec![SocialChannelName::General, SocialChannelName::Staff]
        );
    }

    #[tokio::test]
    async fn social_roles_mute_and_chat_rate_limit_are_enforced() {
        let store = Arc::new(MemoryStore::default());
        let world = WorldHandle::spawn(World::demo(), store.clone())
            .await
            .unwrap();
        let admin = world
            .register_account("Admin".into(), "correct horse battery staple".into())
            .await
            .unwrap();
        let player = world
            .register_account("Player".into(), "another correct horse battery".into())
            .await
            .unwrap();
        let admin_character = world
            .create_authenticated_character(admin.token.as_str().into(), "Aster".into())
            .await
            .unwrap();
        let player_character = world
            .create_authenticated_character(player.token.as_str().into(), "Bryn".into())
            .await
            .unwrap();
        let admin_entered = world
            .enter_authenticated_character(admin.token.as_str().into(), admin_character.id)
            .await
            .unwrap();
        let player_entered = world
            .enter_authenticated_character(player.token.as_str().into(), player_character.id)
            .await
            .unwrap();

        assert!(matches!(
            world
                .apply(
                    player_entered.session_id,
                    PlayerAction::Social(SocialAction::Mute {
                        target: "Aster".into(),
                        duration_seconds: None,
                        reason: "unauthorized".into(),
                    }),
                )
                .await,
            Err(GameError::Persistence(StoreError::SocialForbidden))
        ));
        world
            .apply(
                admin_entered.session_id,
                PlayerAction::Staff(tiamud_core::StaffAction::SetProfile {
                    target: "Bryn".into(),
                    profile: Some(tiamud_core::StaffProfileKey::Moderator),
                    lore_title: None,
                    reason: "moderation test".into(),
                }),
            )
            .await
            .unwrap();
        assert_eq!(
            store
                .social_profile(player_character.id)
                .await
                .unwrap()
                .role,
            AccountRole::Moderator
        );
        world
            .apply(
                admin_entered.session_id,
                PlayerAction::Social(SocialAction::Mute {
                    target: "Bryn".into(),
                    duration_seconds: None,
                    reason: "flooding".into(),
                }),
            )
            .await
            .unwrap();
        assert!(matches!(
            world
                .apply(
                    player_entered.session_id,
                    PlayerAction::Social(SocialAction::Say("Muted message".into())),
                )
                .await,
            Err(GameError::Persistence(StoreError::ChatMuted))
        ));

        for index in 0..crate::social::CHAT_BURST {
            world
                .apply(
                    admin_entered.session_id,
                    PlayerAction::Social(SocialAction::Say(format!("message {index}"))),
                )
                .await
                .unwrap();
        }
        assert!(matches!(
            world
                .apply(
                    admin_entered.session_id,
                    PlayerAction::Social(SocialAction::Say("one too many".into())),
                )
                .await,
            Err(GameError::Persistence(StoreError::ChatRateLimited))
        ));
    }

    #[tokio::test]
    async fn actor_command_queue_is_strictly_bounded_and_auth_bypasses_it() {
        let (tx, _rx) = mpsc::channel(WORLD_COMMAND_QUEUE_CAPACITY);
        let store: Arc<dyn CharacterStore> = Arc::new(MemoryStore::default());
        let handle = WorldHandle {
            tx,
            store,
            start_room_id: Arc::from("demo:lantern-square"),
            creation_catalog: Arc::new(CharacterCreationCatalogView {
                default_lineage_id: "mortal".into(),
                default_background_id: None,
                default_base_class_id: "wanderer".into(),
                lineages: Vec::new(),
                backgrounds: Vec::new(),
                base_classes: Vec::new(),
            }),
            area_router: None,
            area_rehydration_lock: Arc::new(AsyncMutex::new(())),
        };
        for _ in 0..WORLD_COMMAND_QUEUE_CAPACITY {
            let (reply, _receive) = oneshot::channel();
            handle.tx.try_send(Command::Ping { reply }).unwrap();
        }
        let (reply, _receive) = oneshot::channel();
        assert!(matches!(
            handle.tx.try_send(Command::Ping { reply }),
            Err(mpsc::error::TrySendError::Full(_))
        ));
        assert_eq!(handle.queue_depth(), WORLD_COMMAND_QUEUE_CAPACITY);
        handle
            .register_account(
                "Outside Actor".to_owned(),
                "a sufficiently long password".to_owned(),
            )
            .await
            .unwrap();
        assert_eq!(handle.queue_depth(), WORLD_COMMAND_QUEUE_CAPACITY);
    }

    #[tokio::test]
    async fn shutdown_barrier_drains_previously_accepted_actions() {
        let store = Arc::new(MemoryStore::default());
        let world = WorldHandle::spawn(World::demo(), store.clone())
            .await
            .unwrap();
        let entered = world.enter("Shutdown Hero".into()).await.unwrap();
        world
            .apply(entered.session_id, PlayerAction::Move(Direction::North))
            .await
            .unwrap();

        world.shutdown().await.unwrap();
        assert!(matches!(
            world.health_check().await,
            Err(GameError::ActorStopped)
        ));
        let character = store
            .state
            .lock()
            .unwrap()
            .characters
            .values()
            .next()
            .cloned()
            .unwrap();
        assert_eq!(character.room_id, "demo:north-gate");
    }
}
