use crate::{
    auth::AuthenticatedSession,
    delivery::{DeliveryInbox, DeliveryRouter},
    persistence::{CharacterStore, StoreError},
    runtime_content::RuntimeContent,
    social::{SocialRuntime, apply_social},
};
use std::{
    collections::HashMap,
    sync::Arc,
    time::{Duration, Instant},
};
use thiserror::Error;
use tiamud_core::{
    AccountId, AccountRole, AccountSnapshot, AdventureEvent, CharacterGameplaySnapshot,
    CharacterId, CharacterSnapshot, DomainEvent, GameplayEvent, PlayerAction, PlayerStateView,
    SceneView, SessionId, ViewEvent, World, WorldError as CoreWorldError,
};
use tiamud_scripting::ScriptHook;
use tokio::sync::{mpsc, oneshot};
use tokio::time::MissedTickBehavior;

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
}

#[derive(Debug)]
pub struct EnteredWorld {
    pub session_id: SessionId,
    pub account_id: AccountId,
    pub scene: SceneView,
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
    pub player: PlayerStateView,
    pub output: Option<String>,
    pub events: Vec<ViewEvent>,
}

/// A fresh perception-safe projection caused by another actor or by a world
/// tick. The router addresses each projection directly to one session; no
/// stable room, entity, account or character IDs cross the transport edge.
#[derive(Debug)]
pub struct WorldDelivery {
    pub scene: SceneView,
    pub player: PlayerStateView,
    pub events: Vec<ViewEvent>,
}

enum Command {
    Ping {
        reply: oneshot::Sender<()>,
    },
    CurrentState {
        session: SessionId,
        reply: oneshot::Sender<Result<(SceneView, PlayerStateView), GameError>>,
    },
    EnterPreparedCharacter {
        snapshot: CharacterSnapshot,
        gameplay: Box<CharacterGameplaySnapshot>,
        role: AccountRole,
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
    pub fn spawn(world: World, store: Arc<dyn CharacterStore>) -> Self {
        Self::spawn_with_scheduler(world, store, WorldSchedulerConfig::default())
    }

    #[cfg(test)]
    pub fn spawn_with_scheduler(
        world: World,
        store: Arc<dyn CharacterStore>,
        scheduler: WorldSchedulerConfig,
    ) -> Self {
        Self::spawn_with_scheduler_and_content(world, store, scheduler, RuntimeContent::default())
    }

    pub fn spawn_with_scheduler_and_content(
        world: World,
        store: Arc<dyn CharacterStore>,
        scheduler: WorldSchedulerConfig,
        runtime_content: RuntimeContent,
    ) -> Self {
        let (tx, mut rx) = mpsc::channel(WORLD_COMMAND_QUEUE_CAPACITY);
        let mut social = SocialRuntime::new();
        let operational_metrics = runtime_content.operational_metrics();
        let mut actor_delivery = DeliveryRouter::new(operational_metrics.clone());
        let start_room_id: Arc<str> = Arc::from(world.start_room_id());
        let actor_store = Arc::clone(&store);
        tokio::spawn(async move {
            let mut world = world;
            let mut next_session = 1_u64;
            let mut session_roles = HashMap::new();

            match actor_store.load_runtime_snapshot().await {
                Ok(Some(snapshot)) => {
                    if world.restore_runtime_snapshot(snapshot).is_err() {
                        return;
                    }
                }
                Ok(None) => match actor_store.load_domain_events().await {
                    Ok(events) => {
                        for event in events {
                            if world.replay_persisted_event(&event).is_err() {
                                return;
                            }
                        }
                    }
                    Err(_) => return,
                },
                Err(_) => return,
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
                    Command::EnterPreparedCharacter {
                        snapshot,
                        gameplay,
                        role,
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
                            &mut world,
                            &runtime_content,
                            &mut session_roles,
                            actor_store.as_ref(),
                            &mut actor_delivery,
                            role,
                            result,
                        )
                        .await
                        .map(|entered| EnteredWorld {
                            session_id: entered.session_id,
                            account_id: entered.account_id,
                            scene: entered.scene,
                            player: entered.player,
                            deliveries: actor_delivery.register(session),
                        });
                        if let Err(Ok(entered)) = reply.send(result) {
                            session_roles.remove(&entered.session_id);
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
                                session_roles: &session_roles,
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
        Self {
            tx,
            store,
            start_room_id,
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

    pub async fn create_authenticated_character(
        &self,
        token: String,
        name: String,
    ) -> Result<CharacterSnapshot, GameError> {
        create_authenticated_character(
            self.store.as_ref(),
            &token,
            &name,
            self.start_room_id.as_ref(),
        )
        .await
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
        let role = self.store.social_profile(character_id).await?.role;
        let (reply, receive) = oneshot::channel();
        self.tx
            .send(Command::EnterPreparedCharacter {
                snapshot,
                gameplay: Box::new(gameplay),
                role,
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
        self.store
            .load_or_create_character(account_id, &name, self.start_room_id.as_ref())
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
    ) -> Result<(SceneView, PlayerStateView), GameError> {
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

async fn create_authenticated_character(
    store: &dyn CharacterStore,
    token: &str,
    name: &str,
    start_room: &str,
) -> Result<CharacterSnapshot, GameError> {
    let account = store.resolve_session(token).await?;
    store
        .load_or_create_character(account.id, name, start_room)
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
    world.restore_character_gameplay(snapshot.id, gameplay)?;
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

async fn finish_enter(
    world: &mut World,
    runtime_content: &RuntimeContent,
    session_roles: &mut HashMap<SessionId, AccountRole>,
    store: &dyn CharacterStore,
    delivery: &mut DeliveryRouter,
    role: AccountRole,
    result: Result<PreparedWorldEntry, GameError>,
) -> Result<PreparedWorldEntry, GameError> {
    let mut entered = result?;
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
    Ok(entered)
}

struct ApplyResources<'a> {
    world: &'a mut World,
    runtime_content: &'a RuntimeContent,
    social: &'a mut SocialRuntime,
    store: &'a dyn CharacterStore,
    session_roles: &'a HashMap<SessionId, AccountRole>,
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
        delivery,
    } = resources;
    let checkpoint = world.checkpoint();
    if let PlayerAction::Social(action) = action {
        let outcome = apply_social(social, delivery, world, store, session, action).await?;
        let hook = runtime_content.run_hook(world, session, role, ScriptHook::OnTalk);
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
        let mut scene = world.current_scene(session)?;
        runtime_content.resolve_scene(world, session, role, &mut scene);
        let events = hook
            .domain_events
            .iter()
            .map(|event| world.project_event(session, event))
            .collect::<Result<Vec<_>, _>>()?
            .into_iter()
            .flatten()
            .collect();
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
            player: world.player_state_view(session)?,
            output: combine_output(Some(outcome.output), hook.narration),
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
    let events = committed_events
        .iter()
        .map(|event| world.project_event(session, event))
        .collect::<Result<Vec<_>, _>>()?
        .into_iter()
        .flatten()
        .collect();
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
        player: world.player_state_view(session)?,
        output: combine_output(outcome.output, narration),
        events,
    })
}

fn current_state(
    world: &World,
    runtime_content: &RuntimeContent,
    session_roles: &HashMap<SessionId, AccountRole>,
    session: SessionId,
) -> Result<(SceneView, PlayerStateView), GameError> {
    let role = session_roles
        .get(&session)
        .copied()
        .unwrap_or(AccountRole::Player);
    let mut scene = world.current_scene(session)?;
    runtime_content.resolve_scene(world, session, role, &mut scene);
    Ok((scene, world.player_state_view(session)?))
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
        let projected = domain_events
            .iter()
            .map(|event| world.project_event(observer, event))
            .collect::<Result<Vec<_>, _>>()
            .map(|events| events.into_iter().flatten().collect::<Vec<_>>());
        if !refreshes_state && matches!(&projected, Ok(events) if events.is_empty()) {
            continue;
        }
        let state = current_state(world, runtime_content, session_roles, observer);
        match (projected, state) {
            (Ok(events), Ok((scene, player))) => {
                delivery.send_world(
                    observer,
                    WorldDelivery {
                        scene,
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
        .map(|line| world.project_private_narration(session, line))
        .collect::<Result<Vec<_>, _>>();
    let state = current_state(world, runtime_content, session_roles, session);
    match (events, state) {
        (Ok(events), Ok((scene, player))) => {
            delivery.send_world(
                session,
                WorldDelivery {
                    scene,
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
        EquipmentSlot, ItemDefinition, ItemGameplayDefinition, NpcDefinition, PartyAction,
        QuestAction, RoomDefinition, SocialAction, WorldDefinition, WorldRuntimeSnapshot,
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
        audit: Vec<crate::persistence::AuditEntry>,
    }

    #[derive(Default)]
    struct MemoryStore {
        state: Mutex<MemoryState>,
        fail_saves: AtomicBool,
    }

    fn durable_gameplay_world() -> World {
        let mut world = World::from_definition_with_staged_items(
            WorldDefinition {
                start_room_id: "test:arena".into(),
                items: vec![
                    ItemDefinition {
                        stable_id: "test:sword".into(),
                        name: "bronze sword".into(),
                        description: "A balanced old blade.".into(),
                        aliases: vec!["sword".into()],
                    },
                    ItemDefinition {
                        stable_id: "test:potion".into(),
                        name: "ember potion".into(),
                        description: "It glows warmly.".into(),
                        aliases: vec!["potion".into()],
                    },
                    ItemDefinition {
                        stable_id: "test:loot".into(),
                        name: "silver fang".into(),
                        description: "A trophy from the guardian.".into(),
                        aliases: vec!["fang".into()],
                    },
                ],
                rooms: vec![RoomDefinition {
                    stable_id: "test:arena".into(),
                    title: "Arena".into(),
                    prose: vec!["Ancient banners stir overhead.".into()],
                    image: None,
                    audio: None,
                    exits: Vec::new(),
                    item_ids: vec!["test:sword".into(), "test:potion".into()],
                }],
            },
            vec!["test:loot".into()],
        )
        .unwrap();
        world
            .configure_item_gameplay(ItemGameplayDefinition {
                item_id: "test:sword".into(),
                equipment_slot: Some(EquipmentSlot::Weapon),
                attack_bonus: 8,
                heal_amount: 0,
                consumable: false,
                effect: None,
            })
            .unwrap();
        world
            .configure_item_gameplay(ItemGameplayDefinition {
                item_id: "test:potion".into(),
                equipment_slot: None,
                attack_bonus: 0,
                heal_amount: 1,
                consumable: true,
                effect: Some(ActiveEffect {
                    kind: EffectKind::Strength,
                    magnitude: 2,
                    remaining_ticks: 4,
                }),
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
                loot_item_ids: vec!["test:loot".into()],
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
            let role_change = matches!(&action, crate::persistence::ModerationAction::SetRole(_));
            if (role_change && actor.role != tiamud_core::AccountRole::Admin)
                || (!role_change && !actor.role.can_moderate(target.role))
            {
                return Err(StoreError::SocialForbidden);
            }
            let action_name = match action {
                crate::persistence::ModerationAction::Mute { .. } => {
                    state.muted.insert(target.account_id);
                    "mute"
                }
                crate::persistence::ModerationAction::Unmute => {
                    state.muted.remove(&target.account_id);
                    "unmute"
                }
                crate::persistence::ModerationAction::Ban { .. } => {
                    state.banned.insert(target.account_id);
                    "ban"
                }
                crate::persistence::ModerationAction::Unban => {
                    state.banned.remove(&target.account_id);
                    "unban"
                }
                crate::persistence::ModerationAction::SetRole(role) => {
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
    }

    #[tokio::test]
    async fn persistence_failure_rolls_back_a_critical_move() {
        let store = Arc::new(MemoryStore::default());
        let world = WorldHandle::spawn(World::demo(), store.clone());
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
        let world = WorldHandle::spawn(World::demo(), store.clone());
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
        let world = WorldHandle::spawn(World::demo(), store);
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
            delivery.events[0].kind,
            tiamud_core::ViewEventKind::ItemTaken
        );
        assert!(!delivery.events[0].actor_id.contains("Actor"));
        assert!(!delivery.events[0].actor_id.contains("demo:"));
    }

    #[tokio::test]
    async fn persistence_failure_rolls_back_consumption_effects_and_equipment() {
        let store = Arc::new(MemoryStore::default());
        let world = WorldHandle::spawn(durable_gameplay_world(), store.clone());
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
        assert!(inventory.output.unwrap().contains("ember potion"));
        let state = store.state.lock().unwrap();
        let character_id = *state.characters.keys().next().unwrap();
        assert!(state.gameplay[&character_id].effects.is_empty());
        assert!(matches!(
            state.runtime.as_ref().unwrap().item_locations["test:potion"],
            tiamud_core::RuntimeItemLocation::Carried { .. }
        ));
    }

    #[tokio::test]
    async fn persistence_failure_rolls_back_party_quest_and_market_state_atomically() {
        let store = Arc::new(MemoryStore::default());
        let world = WorldHandle::spawn(World::demo(), store.clone());
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
        let first_world = WorldHandle::spawn(World::demo(), store.clone());
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
        let restarted = WorldHandle::spawn(rebuilt, store);
        let witness = restarted.enter("Witness".into()).await.unwrap();
        assert!(witness.scene.items.is_empty());
        restarted.leave(witness.session_id).await;

        let collector = restarted.enter("Collector".into()).await.unwrap();
        let inventory = restarted
            .apply(collector.session_id, PlayerAction::Inventory)
            .await
            .unwrap();
        assert_eq!(
            inventory.output.as_deref(),
            Some("You are carrying:\r\n- blue-glass lantern\r\n")
        );
    }

    #[tokio::test]
    async fn simultaneous_guest_sessions_have_independent_world_positions() {
        let store = Arc::new(MemoryStore::default());
        let world = WorldHandle::spawn(World::demo(), store);
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
        let world = WorldHandle::spawn(World::demo(), store);
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
        let world = WorldHandle::spawn(World::demo(), store);
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
        let world = WorldHandle::spawn(World::demo(), store);
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
    async fn legacy_entry_keeps_the_same_character_id_across_reconnects() {
        let store = Arc::new(MemoryStore::default());
        let world = WorldHandle::spawn(World::demo(), store);
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
        assert_eq!(narration[0].1.text, "Target alone hears the bell.");
        assert_ne!(narration[0].1.actor_id, "71");
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
        );
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
                runtime.item_locations["test:potion"],
                tiamud_core::RuntimeItemLocation::Consumed
            ));
            (character_id, gameplay, defeated_at)
        };
        assert_eq!(gameplay.level, 2);
        assert_eq!(gameplay.experience, 120);
        assert_eq!(gameplay.equipment[&EquipmentSlot::Weapon], "test:sword");
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
        );
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
        assert_eq!(entered.scene.items[0].label, "silver fang");
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
        let first = WorldHandle::spawn(World::demo(), store.clone());
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

        let restarted = WorldHandle::spawn(World::demo(), store);
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
        let world = WorldHandle::spawn(World::demo(), store);
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
    }

    #[tokio::test]
    async fn social_roles_mute_and_chat_rate_limit_are_enforced() {
        let store = Arc::new(MemoryStore::default());
        let world = WorldHandle::spawn(World::demo(), store.clone());
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
                PlayerAction::Social(SocialAction::SetRole {
                    target: "Bryn".into(),
                    role: AccountRole::Moderator,
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
        let world = WorldHandle::spawn(World::demo(), store.clone());
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
