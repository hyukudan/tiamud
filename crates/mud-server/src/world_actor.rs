use crate::persistence::{CharacterStore, StoreError};
use mud_core::{PlayerAction, SceneView, SessionId, World, WorldError as CoreWorldError};
use std::sync::Arc;
use thiserror::Error;
use tokio::sync::{mpsc, oneshot};

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
    pub scene: SceneView,
}

#[derive(Debug)]
pub struct AppliedAction {
    pub scene: SceneView,
}

enum Command {
    Enter {
        name: String,
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
}

#[derive(Clone)]
pub struct WorldHandle {
    tx: mpsc::Sender<Command>,
}

impl WorldHandle {
    pub fn spawn(store: Arc<dyn CharacterStore>) -> Self {
        let (tx, mut rx) = mpsc::channel(256);
        tokio::spawn(async move {
            let mut world = World::demo();
            let mut next_session = 1_u64;

            while let Some(command) = rx.recv().await {
                match command {
                    Command::Enter { name, reply } => {
                        let session = SessionId(next_session);
                        next_session = next_session.wrapping_add(1).max(1);
                        let result = enter(&mut world, store.as_ref(), session, name).await;
                        let _ = reply.send(result);
                    }
                    Command::Apply {
                        session,
                        action,
                        reply,
                    } => {
                        let result = apply(&mut world, store.as_ref(), session, action).await;
                        let _ = reply.send(result);
                    }
                    Command::Leave { session } => world.disconnect(session),
                }
            }
        });
        Self { tx }
    }

    pub async fn enter(&self, name: String) -> Result<EnteredWorld, GameError> {
        let (reply, receive) = oneshot::channel();
        self.tx
            .send(Command::Enter { name, reply })
            .await
            .map_err(|_| GameError::ActorStopped)?;
        receive.await.map_err(|_| GameError::ActorStopped)?
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
}

async fn enter(
    world: &mut World,
    store: &dyn CharacterStore,
    session: SessionId,
    name: String,
) -> Result<EnteredWorld, GameError> {
    let snapshot = store.load_or_create(&name, world.default_room_id()).await?;
    let scene = world.connect(session, snapshot.clone())?;

    // A removed or renamed room in an old save is repaired immediately.
    let resolved = world.character_snapshot(session)?;
    if resolved != snapshot
        && let Err(error) = store.save(resolved).await
    {
        world.disconnect(session);
        return Err(error.into());
    }

    Ok(EnteredWorld {
        session_id: session,
        scene,
    })
}

async fn apply(
    world: &mut World,
    store: &dyn CharacterStore,
    session: SessionId,
    action: PlayerAction,
) -> Result<AppliedAction, GameError> {
    let before = world.character_snapshot(session)?;
    let outcome = world.apply(session, action)?;
    if let Some(snapshot) = outcome.changed_character
        && let Err(error) = store.save(snapshot).await
    {
        // Critical world changes are only acknowledged after persistence. The
        // in-memory move is rolled back if the commit fails.
        world.restore_character(session, &before)?;
        return Err(error.into());
    }
    Ok(AppliedAction {
        scene: outcome.scene,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use async_trait::async_trait;
    use mud_core::{CharacterSnapshot, Direction};
    use std::{
        collections::HashMap,
        sync::{
            Mutex,
            atomic::{AtomicBool, Ordering},
        },
    };

    #[derive(Default)]
    struct MemoryStore {
        snapshots: Mutex<HashMap<String, CharacterSnapshot>>,
        fail_saves: AtomicBool,
    }

    #[async_trait]
    impl CharacterStore for MemoryStore {
        async fn load_or_create(
            &self,
            name: &str,
            default_room: &str,
        ) -> Result<CharacterSnapshot, StoreError> {
            let mut snapshots = self.snapshots.lock().unwrap();
            Ok(snapshots
                .entry(name.to_owned())
                .or_insert_with(|| CharacterSnapshot {
                    name: name.to_owned(),
                    room_id: default_room.to_owned(),
                })
                .clone())
        }

        async fn save(&self, snapshot: CharacterSnapshot) -> Result<(), StoreError> {
            if self.fail_saves.load(Ordering::SeqCst) {
                return Err(StoreError::WorkerStopped);
            }
            self.snapshots
                .lock()
                .unwrap()
                .insert(snapshot.name.clone(), snapshot);
            Ok(())
        }
    }

    #[tokio::test]
    async fn persistence_failure_rolls_back_a_critical_move() {
        let store = Arc::new(MemoryStore::default());
        let world = WorldHandle::spawn(store.clone());
        let entered = world.enter("Rollback Tester".into()).await.unwrap();
        store.fail_saves.store(true, Ordering::SeqCst);

        let failed = world
            .apply(entered.session_id, PlayerAction::Move(Direction::North))
            .await;
        assert!(matches!(failed, Err(GameError::Persistence(_))));

        let after = world
            .apply(entered.session_id, PlayerAction::Look)
            .await
            .unwrap();
        assert_eq!(after.scene.view_id, "demo:lantern-square");
    }

    #[tokio::test]
    async fn simultaneous_guest_sessions_have_independent_world_positions() {
        let store = Arc::new(MemoryStore::default());
        let world = WorldHandle::spawn(store);
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
        assert_eq!(second_view.scene.view_id, "demo:lantern-square");
    }
}
