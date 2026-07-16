use crate::{
    auth::AuthenticatedSession,
    delivery::{DeliveryInbox, DeliveryRouter},
    persistence::{CharacterStore, DurableCharacterGameplay, StaffAccess, StoreError},
    runtime_content::{RuntimeContent, ScenePresentation},
    social::{SocialChannel, SocialMessage, SocialRuntime, apply_social},
    staff::{apply_staff, derived_role},
};
use std::{
    collections::{HashMap, HashSet},
    sync::Arc,
    time::{Duration, Instant},
};
use thiserror::Error;
use tiamud_core::{
    AccountId, AccountRole, AccountSnapshot, AdventureEvent, BackgroundSelection,
    CharacterCreationCatalogView, CharacterGameplaySnapshot, CharacterId, CharacterSnapshot,
    DomainEvent, GameplayEvent, PlayerAction, PlayerStateView, SceneView, SessionId, ViewEvent,
    World, WorldError as CoreWorldError,
};
use tiamud_scripting::ScriptHook;
use tokio::sync::{mpsc, oneshot};
use tokio::time::MissedTickBehavior;
use tracing::error;

const WORLD_COMMAND_QUEUE_CAPACITY: usize = 256;
const PERSIST_TICK_SCRIPT_WALL_BUDGET: Duration = Duration::from_millis(100);

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
    #[error("world actor startup failed during {stage}: {reason}")]
    Startup { stage: &'static str, reason: String },
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
    Leave {
        session: SessionId,
    },
    Shutdown {
        reply: oneshot::Sender<()>,
    },
}

#[derive(Clone)]
pub struct WorldHandle {
    tx: mpsc::Sender<Command>,
    store: Arc<dyn CharacterStore>,
    start_room_id: Arc<str>,
    creation_catalog: Arc<CharacterCreationCatalogView>,
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

    pub async fn spawn_with_scheduler_and_content(
        world: World,
        store: Arc<dyn CharacterStore>,
        scheduler: WorldSchedulerConfig,
        runtime_content: RuntimeContent,
    ) -> Result<Self, GameError> {
        let (tx, mut rx) = mpsc::channel(WORLD_COMMAND_QUEUE_CAPACITY);
        let (startup_reply, startup_receive) = oneshot::channel();
        let mut social = SocialRuntime::new();
        let operational_metrics = runtime_content.operational_metrics();
        let mut actor_delivery = DeliveryRouter::new(operational_metrics.clone());
        let start_room_id: Arc<str> = Arc::from(world.start_room_id());
        let creation_catalog = Arc::new(world.progression_catalog().clone());
        let actor_store = Arc::clone(&store);
        tokio::spawn(async move {
            let mut world = world;
            let mut next_session = 1_u64;
            let mut session_roles = HashMap::new();
            let mut session_staff_accesses = HashMap::new();

            let startup = match actor_store.load_runtime_snapshot().await {
                Ok(Some(snapshot)) => {
                    world
                        .restore_runtime_snapshot(snapshot)
                        .map_err(|error| GameError::Startup {
                            stage: "restore_runtime_snapshot",
                            reason: error.to_string(),
                        })
                }
                Ok(None) => match actor_store.load_domain_events().await {
                    Ok(events) => {
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
                    Err(error) => Err(GameError::Startup {
                        stage: "load_domain_events",
                        reason: error.to_string(),
                    }),
                },
                Err(error) => Err(GameError::Startup {
                    stage: "load_runtime_snapshot",
                    reason: error.to_string(),
                }),
            };
            if let Err(error) = startup {
                return report_startup_failure(startup_reply, error);
            }
            let runtime_owner_ids = world
                .runtime_item_owner_ids()
                .into_iter()
                .collect::<Vec<_>>();
            let durable_gameplay = match actor_store
                .load_character_gameplay_batch(runtime_owner_ids)
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
            if let Err(error) = validate_startup_character_aggregates(&world, durable_gameplay) {
                return report_startup_failure(
                    startup_reply,
                    GameError::Startup {
                        stage: "validate_character_aggregates",
                        reason: error.to_string(),
                    },
                );
            }
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
                    }
                } else {
                    rx.recv().await
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
                    Command::Leave { session } => {
                        let role = session_roles
                            .remove(&session)
                            .unwrap_or(AccountRole::Player);
                        session_staff_accesses.remove(&session);
                        let checkpoint = world.checkpoint();
                        let hook = runtime_content.run_hook(
                            &mut world,
                            session,
                            role,
                            ScriptHook::OnLeave,
                        );
                        if (hook.changed || !hook.domain_events.is_empty())
                            && actor_store
                                .commit_action(
                                    durable_connected_characters(&world).unwrap_or_default(),
                                    world.runtime_snapshot(),
                                    hook.domain_events.clone(),
                                )
                                .await
                                .is_err()
                        {
                            world.restore_checkpoint(checkpoint);
                        } else if hook.changed || !hook.domain_events.is_empty() {
                            broadcast_world_deliveries(
                                &world,
                                &runtime_content,
                                &session_roles,
                                Some(session),
                                &hook.domain_events,
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
        // All authentication, password work and read-only storage waits happen
        // before the state-owning actor sees the entry command. The actor only
        // receives an authorized, fully loaded snapshot and serializes the
        // world mutation that grants the live character lease.
        let snapshot = self.store.load_character(account_id, character_id).await?;
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

fn validate_startup_character_aggregates(
    world: &World,
    aggregates: Vec<DurableCharacterGameplay>,
) -> Result<(), CoreWorldError> {
    let expected_owners = world.runtime_item_owner_ids();
    let mut durable_characters = std::collections::BTreeSet::new();
    if aggregates.iter().any(|aggregate| {
        aggregate.character_id.0 <= 0 || !durable_characters.insert(aggregate.character_id)
    }) || durable_characters != expected_owners
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

    // A removed or renamed room in an old save is repaired immediately.
    let resolved = world.character_snapshot(session)?;
    if resolved != snapshot
        && let Err(error) = store.save(resolved.clone()).await
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
    if (hook.changed || !hook.domain_events.is_empty())
        && let Err(error) = store
            .commit_action(
                durable_connected_characters(world)?,
                world.runtime_snapshot(),
                hook.domain_events.clone(),
            )
            .await
    {
        world.restore_checkpoint(checkpoint);
        world.disconnect(entered.session_id);
        return Err(error.into());
    }
    if hook.changed || !hook.domain_events.is_empty() {
        broadcast_world_deliveries(
            world,
            runtime_content,
            session_roles,
            Some(entered.session_id),
            &hook.domain_events,
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
                events,
            });
        }
        action => action,
    };
    if let PlayerAction::Social(action) = action {
        let talks = matches!(
            &action,
            tiamud_core::SocialAction::Say(_)
                | tiamud_core::SocialAction::PartySay(_)
                | tiamud_core::SocialAction::Tell { .. }
                | tiamud_core::SocialAction::Reply(_)
                | tiamud_core::SocialAction::ChannelSay { .. }
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
        let hook = if talks {
            runtime_content.run_hook(world, session, role, ScriptHook::OnTalk)
        } else {
            crate::runtime_content::ScriptBatchOutcome::default()
        };
        if (hook.changed || !hook.domain_events.is_empty())
            && let Err(error) = store
                .commit_action(
                    durable_connected_characters(world)?,
                    world.runtime_snapshot(),
                    hook.domain_events.clone(),
                )
                .await
        {
            world.restore_checkpoint(checkpoint);
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
        let mut scene = world.current_scene(session)?;
        let current_role = session_roles
            .get(&session)
            .copied()
            .unwrap_or(AccountRole::Player);
        runtime_content.resolve_scene(world, session, current_role, &mut scene);
        let events = project_view_events(world, session, &hook.domain_events)?;
        if hook.changed || !hook.domain_events.is_empty() {
            broadcast_world_deliveries(
                world,
                runtime_content,
                session_roles,
                Some(session),
                &hook.domain_events,
                delivery,
            );
        }
        return Ok(AppliedAction {
            scene,
            presentation: runtime_content.resolve_entity_presentation(world, session),
            player: world.player_state_view(session)?,
            output: combine_output(Some(applied.outcome.output), hook.narration),
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
    if (outcome.changed_character.is_some() || !committed_events.is_empty() || script_changed)
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
    if !committed_events.is_empty() || script_changed {
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
        events,
    })
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
    let result = world.advance_tick().and_then(|mut events| {
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
        Ok((characters, world.runtime_snapshot(), events))
    });
    if scripting_budget_exhausted {
        world.restore_checkpoint(checkpoint.clone());
        tracing::warn!(
            maximum_milliseconds = script_wall_budget.as_millis(),
            "persist_tick scripting budget exhausted; retrying the base tick without scripts"
        );
        targeted_narration.clear();
        let base_tick = world.advance_tick().and_then(|events| {
            let characters = durable_connected_characters(world)?;
            Ok((characters, world.runtime_snapshot(), events))
        });
        let Ok((characters, runtime, events)) = base_tick else {
            world.restore_checkpoint(checkpoint);
            return false;
        };
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
    let Ok((characters, runtime, events)) = result else {
        world.restore_checkpoint(checkpoint);
        return false;
    };
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
    use crate::runtime_content::RuntimeScript;
    use async_trait::async_trait;
    use std::{
        collections::{BTreeSet, HashMap, HashSet},
        sync::{
            Mutex,
            atomic::{AtomicBool, Ordering},
        },
    };
    use tiamud_core::{
        AccountRole, ActiveEffect, CharacterGameplaySnapshot, Direction, EconomyAction, EffectKind,
        EquipmentSlot, InitialItemInstanceDefinition, InitialItemLocation, ItemInstanceId,
        ItemPrototypeDefinition, ItemRarity, ItemStackPolicy, ItemTransferPolicy, NpcDefinition,
        PartyAction, QuestAction, RoomDefinition, SocialAction, SocialChannelName, WorldDefinition,
        WorldRuntimeSnapshot,
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
        audit: Vec<crate::persistence::AuditEntry>,
        staff_profiles: HashMap<AccountId, (tiamud_core::StaffProfileKey, Option<String>)>,
        staff_capabilities: HashMap<AccountId, BTreeSet<tiamud_core::StaffCapability>>,
        next_staff_report: i64,
        staff_reports: Vec<crate::persistence::StaffReport>,
        resolved_staff_reports: HashSet<i64>,
        staff_audit: Vec<crate::persistence::StaffAuditEntry>,
    }

    #[derive(Default)]
    struct MemoryStore {
        state: Mutex<MemoryState>,
        fail_saves: AtomicBool,
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
                    transfer_policy: ItemTransferPolicy::Transferable,
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
                    transfer_policy: ItemTransferPolicy::Transferable,
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
                    transfer_policy: ItemTransferPolicy::Transferable,
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

        async fn load_character_gameplay_batch(
            &self,
            character_ids: Vec<CharacterId>,
        ) -> Result<Vec<DurableCharacterGameplay>, StoreError> {
            let state = self.state.lock().unwrap();
            let character_ids = crate::persistence::normalise_startup_owner_ids(character_ids)?;
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
                    crate::persistence::decode_startup_owner_gameplay(
                        character_id,
                        &json,
                        &mut total_json_bytes,
                    )
                })
                .collect()
        }

        async fn load_runtime_snapshot(&self) -> Result<Option<WorldRuntimeSnapshot>, StoreError> {
            Ok(self.state.lock().unwrap().runtime.clone())
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
            _resolution: tiamud_core::StaffReportResolution,
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
            let target_account = state
                .staff_reports
                .iter()
                .find(|report| report.id == report_id)
                .map(|report| report.target_account_id)
                .ok_or(StoreError::StaffReportUnavailable(report_id))?;
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
    async fn invalid_runtime_snapshot_fails_the_startup_barrier() {
        let store = Arc::new(MemoryStore::default());
        let mut snapshot = World::demo().runtime_snapshot();
        snapshot.snapshot_version += 1;
        store.state.lock().unwrap().runtime = Some(snapshot);

        let result = WorldHandle::spawn(World::demo(), store).await;

        assert!(matches!(
            result,
            Err(GameError::Startup {
                stage: "restore_runtime_snapshot",
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
            vec![DurableCharacterGameplay {
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
                PlayerAction::Take("blue-glass lantern".into()),
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
                PlayerAction::Take("blue-glass lantern".into()),
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
            .apply(entered.session_id, PlayerAction::Take("potion".into()))
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
                PlayerAction::Take("blue-glass lantern".into()),
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
                 - blue-glass lantern [common; level 1; 1000 g; ref o"
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
            PlayerAction::Take("potion".into()),
            PlayerAction::Use("potion".into()),
            PlayerAction::Take("sword".into()),
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
        assert!(inventory.output.unwrap().contains("weapon: bronze sword"));
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

    fn drain_deliveries(inbox: &mut DeliveryInbox) {
        while inbox.try_recv().is_ok() {}
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
                capabilities: BTreeSet::from([ScriptCapability::SetFlag]),
                limits: ScriptLimits::default(),
                source: r#"
                    function on_talk(ctx)
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
        store.fail_saves.store(true, Ordering::SeqCst);

        assert!(matches!(
            world
                .apply(
                    sender.session_id,
                    PlayerAction::Social(SocialAction::Say("not committed".into())),
                )
                .await,
            Err(GameError::Persistence(_))
        ));
        assert!(
            tokio::time::timeout(Duration::from_millis(20), recipient.deliveries.recv())
                .await
                .is_err()
        );

        store.fail_saves.store(false, Ordering::SeqCst);
        world
            .apply(
                sender.session_id,
                PlayerAction::Social(SocialAction::Say("committed".into())),
            )
            .await
            .unwrap();
        assert_eq!(
            next_social(&mut recipient.deliveries).await.text,
            "committed"
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
