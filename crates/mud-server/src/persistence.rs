use async_trait::async_trait;
use mud_core::CharacterSnapshot;
use rusqlite::{Connection, OptionalExtension, params};
use std::{io, path::Path, thread};
use thiserror::Error;
use tokio::sync::{mpsc, oneshot};

#[derive(Debug, Error)]
pub enum StoreError {
    #[error("SQLite error: {0}")]
    Sqlite(#[from] rusqlite::Error),
    #[error("the persistence worker stopped unexpectedly")]
    WorkerStopped,
    #[error("could not start the persistence worker: {0}")]
    ThreadSpawn(#[source] io::Error),
}

#[async_trait]
pub trait CharacterStore: Send + Sync {
    async fn load_or_create(
        &self,
        name: &str,
        default_room: &str,
    ) -> Result<CharacterSnapshot, StoreError>;

    async fn save(&self, snapshot: CharacterSnapshot) -> Result<(), StoreError>;
}

enum Request {
    LoadOrCreate {
        name: String,
        default_room: String,
        reply: oneshot::Sender<Result<CharacterSnapshot, StoreError>>,
    },
    Save {
        snapshot: CharacterSnapshot,
        reply: oneshot::Sender<Result<(), StoreError>>,
    },
}

/// SQLite adapter with one dedicated owner thread.
///
/// `rusqlite::Connection` never crosses thread boundaries and gameplay tasks do
/// not block a Tokio executor while SQLite is writing.
#[derive(Clone)]
pub struct SqliteCharacterStore {
    tx: mpsc::Sender<Request>,
}

impl SqliteCharacterStore {
    pub async fn open(path: impl AsRef<Path>) -> Result<Self, StoreError> {
        let path = path.as_ref().to_owned();
        // Backpressure here prevents an accidental caller from building an
        // unbounded heap queue while the database is busy.
        let (tx, mut rx) = mpsc::channel(256);
        let (ready_tx, ready_rx) = oneshot::channel();

        thread::Builder::new()
            .name("sqlite-writer".into())
            .spawn(move || {
                let connection = match Connection::open(path).and_then(|connection| {
                    initialise(&connection)?;
                    Ok(connection)
                }) {
                    Ok(connection) => {
                        let _ = ready_tx.send(Ok(()));
                        connection
                    }
                    Err(error) => {
                        let _ = ready_tx.send(Err(StoreError::Sqlite(error)));
                        return;
                    }
                };

                while let Some(request) = rx.blocking_recv() {
                    match request {
                        Request::LoadOrCreate {
                            name,
                            default_room,
                            reply,
                        } => {
                            let _ = reply.send(load_or_create(&connection, &name, &default_room));
                        }
                        Request::Save { snapshot, reply } => {
                            let _ = reply.send(save(&connection, &snapshot));
                        }
                    }
                }
            })
            .map_err(StoreError::ThreadSpawn)?;

        ready_rx.await.map_err(|_| StoreError::WorkerStopped)??;
        Ok(Self { tx })
    }
}

#[async_trait]
impl CharacterStore for SqliteCharacterStore {
    async fn load_or_create(
        &self,
        name: &str,
        default_room: &str,
    ) -> Result<CharacterSnapshot, StoreError> {
        let (reply, receive) = oneshot::channel();
        self.tx
            .send(Request::LoadOrCreate {
                name: name.to_owned(),
                default_room: default_room.to_owned(),
                reply,
            })
            .await
            .map_err(|_| StoreError::WorkerStopped)?;
        receive.await.map_err(|_| StoreError::WorkerStopped)?
    }

    async fn save(&self, snapshot: CharacterSnapshot) -> Result<(), StoreError> {
        let (reply, receive) = oneshot::channel();
        self.tx
            .send(Request::Save { snapshot, reply })
            .await
            .map_err(|_| StoreError::WorkerStopped)?;
        receive.await.map_err(|_| StoreError::WorkerStopped)?
    }
}

fn initialise(connection: &Connection) -> rusqlite::Result<()> {
    connection.execute_batch(
        "PRAGMA journal_mode = WAL;
         PRAGMA synchronous = FULL;
         PRAGMA foreign_keys = ON;
         PRAGMA busy_timeout = 5000;
         CREATE TABLE IF NOT EXISTS characters (
             name TEXT PRIMARY KEY COLLATE NOCASE,
             room_id TEXT NOT NULL,
             updated_at INTEGER NOT NULL DEFAULT (unixepoch())
         );",
    )
}

fn load_or_create(
    connection: &Connection,
    name: &str,
    default_room: &str,
) -> Result<CharacterSnapshot, StoreError> {
    let stored = connection
        .query_row(
            "SELECT name, room_id FROM characters WHERE name = ?1",
            [name],
            |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?)),
        )
        .optional()?;

    let (name, room_id) = match stored {
        Some(stored) => stored,
        None => {
            connection.execute(
                "INSERT INTO characters (name, room_id) VALUES (?1, ?2)",
                params![name, default_room],
            )?;
            (name.to_owned(), default_room.to_owned())
        }
    };

    Ok(CharacterSnapshot { name, room_id })
}

fn save(connection: &Connection, snapshot: &CharacterSnapshot) -> Result<(), StoreError> {
    connection.execute(
        "INSERT INTO characters (name, room_id, updated_at)
         VALUES (?1, ?2, unixepoch())
         ON CONFLICT(name) DO UPDATE SET
             room_id = excluded.room_id,
             updated_at = excluded.updated_at",
        params![snapshot.name, snapshot.room_id],
    )?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn round_trips_a_character_snapshot() {
        let store = SqliteCharacterStore::open(":memory:").await.unwrap();
        let created = store
            .load_or_create("Aria", "demo:lantern-square")
            .await
            .unwrap();
        assert_eq!(created.room_id, "demo:lantern-square");

        store
            .save(CharacterSnapshot {
                name: "Aria".into(),
                room_id: "demo:north-gate".into(),
            })
            .await
            .unwrap();

        let loaded = store.load_or_create("aria", "ignored").await.unwrap();
        assert_eq!(loaded.name, "Aria");
        assert_eq!(loaded.room_id, "demo:north-gate");
    }
}
