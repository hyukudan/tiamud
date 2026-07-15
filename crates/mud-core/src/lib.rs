//! Transport-independent world model for Project Envy.

use serde::{Deserialize, Serialize};
use slotmap::{SlotMap, new_key_type};
use std::collections::{BTreeMap, HashMap};
use thiserror::Error;

new_key_type! {
    /// Internal, generational entity identifier. Never exposed as a persistent ID.
    struct EntityId;
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct SessionId(pub u64);

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub enum Direction {
    North,
    East,
    South,
    West,
    Up,
    Down,
}

impl Direction {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::North => "north",
            Self::East => "east",
            Self::South => "south",
            Self::West => "west",
            Self::Up => "up",
            Self::Down => "down",
        }
    }

    fn parse(value: &str) -> Option<Self> {
        match value {
            "north" | "n" => Some(Self::North),
            "east" | "e" => Some(Self::East),
            "south" | "s" => Some(Self::South),
            "west" | "w" => Some(Self::West),
            "up" | "u" => Some(Self::Up),
            "down" | "d" => Some(Self::Down),
            _ => None,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum PlayerAction {
    Look,
    Move(Direction),
}

#[derive(Debug, Error, Eq, PartialEq)]
pub enum ParseActionError {
    #[error("Please enter a command.")]
    Empty,
    #[error("Unknown command: {0}")]
    Unknown(String),
}

pub fn parse_action(input: &str) -> Result<PlayerAction, ParseActionError> {
    let normalised = input.trim().to_ascii_lowercase();
    if normalised.is_empty() {
        return Err(ParseActionError::Empty);
    }
    if matches!(normalised.as_str(), "look" | "l") {
        return Ok(PlayerAction::Look);
    }
    if let Some(direction) = Direction::parse(&normalised) {
        return Ok(PlayerAction::Move(direction));
    }
    if let Some(rest) = normalised.strip_prefix("go ")
        && let Some(direction) = Direction::parse(rest.trim())
    {
        return Ok(PlayerAction::Move(direction));
    }
    Err(ParseActionError::Unknown(input.trim().to_owned()))
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct CharacterSnapshot {
    pub name: String,
    pub room_id: String,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SceneImage {
    pub alt: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub avif: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub webp: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub jpeg: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub width: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub height: Option<u32>,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SceneExit {
    pub id: String,
    pub label: String,
    pub command: String,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SceneView {
    pub view_id: String,
    pub revision: u64,
    pub title: String,
    pub prose: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub image: Option<SceneImage>,
    pub exits: Vec<SceneExit>,
}

impl SceneView {
    pub fn as_plain_text(&self) -> String {
        let mut output = format!("\r\n{}\r\n", self.title);
        for paragraph in &self.prose {
            output.push_str(paragraph);
            output.push_str("\r\n");
        }
        if self.exits.is_empty() {
            output.push_str("Exits: none\r\n");
        } else {
            let exits = self
                .exits
                .iter()
                .map(|exit| exit.label.as_str())
                .collect::<Vec<_>>()
                .join(", ");
            output.push_str(&format!("Exits: {exits}\r\n"));
        }
        output
    }
}

#[derive(Clone, Debug)]
struct Room {
    stable_id: String,
    title: String,
    prose: Vec<String>,
    image: Option<SceneImage>,
    exits: BTreeMap<Direction, EntityId>,
}

#[derive(Clone, Debug)]
struct Character {
    name: String,
    location: EntityId,
}

#[derive(Clone, Debug)]
enum Entity {
    Room(Room),
    Character(Character),
}

#[derive(Debug, Error)]
pub enum WorldError {
    #[error("No character is attached to this session.")]
    UnknownSession,
    #[error("There is no exit in that direction.")]
    NoExit,
    #[error("The world contains an invalid entity reference.")]
    InvalidEntity,
}

#[derive(Clone, Debug)]
pub struct ActionOutcome {
    pub scene: SceneView,
    /// Present when the action changed persistent character state.
    pub changed_character: Option<CharacterSnapshot>,
}

/// A single-owner world graph. Callers are expected to place it behind an actor.
pub struct World {
    entities: SlotMap<EntityId, Entity>,
    stable_rooms: HashMap<String, EntityId>,
    sessions: HashMap<SessionId, EntityId>,
    default_room: EntityId,
    default_room_stable_id: String,
    revision: u64,
}

impl World {
    pub fn demo() -> Self {
        let mut entities = SlotMap::with_key();
        let square = entities.insert(Entity::Room(Room {
            stable_id: "demo:lantern-square".into(),
            title: "Lantern Square".into(),
            prose: vec![
                "Rain has polished the old stones to a dark shine. Beneath the clock tower, a few late travellers gather around a blue-glass lantern.".into(),
                "The illustration, when available, is atmospheric only; the text remains the complete source of playable information.".into(),
            ],
            image: None,
            exits: BTreeMap::new(),
        }));
        let gate = entities.insert(Entity::Room(Room {
            stable_id: "demo:north-gate".into(),
            title: "The North Gate".into(),
            prose: vec![
                "An iron-bound gate stands open between squat stone towers. The road continues into fields silvered by rain.".into(),
            ],
            image: None,
            exits: BTreeMap::new(),
        }));
        let archive = entities.insert(Entity::Room(Room {
            stable_id: "demo:archive".into(),
            title: "The Cartographer's Archive".into(),
            prose: vec![
                "Shelves of annotated maps surround a broad oak table. A dry stair climbs back toward the square.".into(),
            ],
            image: None,
            exits: BTreeMap::new(),
        }));

        room_mut(&mut entities, square)
            .exits
            .insert(Direction::North, gate);
        room_mut(&mut entities, square)
            .exits
            .insert(Direction::Down, archive);
        room_mut(&mut entities, gate)
            .exits
            .insert(Direction::South, square);
        room_mut(&mut entities, archive)
            .exits
            .insert(Direction::Up, square);

        let stable_rooms = [square, gate, archive]
            .into_iter()
            .map(|id| (room(&entities, id).stable_id.clone(), id))
            .collect();

        let default_room_stable_id = room(&entities, square).stable_id.clone();

        Self {
            entities,
            stable_rooms,
            sessions: HashMap::new(),
            default_room: square,
            default_room_stable_id,
            revision: 1,
        }
    }

    pub fn default_room_id(&self) -> &str {
        &self.default_room_stable_id
    }

    pub fn connect(
        &mut self,
        session: SessionId,
        snapshot: CharacterSnapshot,
    ) -> Result<SceneView, WorldError> {
        let location = self
            .stable_rooms
            .get(&snapshot.room_id)
            .copied()
            .unwrap_or(self.default_room);
        room_checked(&self.entities, location)?;
        let character = self.entities.insert(Entity::Character(Character {
            name: snapshot.name,
            location,
        }));
        if let Some(previous) = self.sessions.insert(session, character) {
            self.entities.remove(previous);
        }
        self.scene_for(session)
    }

    pub fn disconnect(&mut self, session: SessionId) {
        if let Some(character) = self.sessions.remove(&session) {
            self.entities.remove(character);
        }
    }

    pub fn character_snapshot(&self, session: SessionId) -> Result<CharacterSnapshot, WorldError> {
        let character = self.character(session)?;
        let current_room = room_checked(&self.entities, character.location)?;
        Ok(CharacterSnapshot {
            name: character.name.clone(),
            room_id: current_room.stable_id.clone(),
        })
    }

    pub fn restore_character(
        &mut self,
        session: SessionId,
        snapshot: &CharacterSnapshot,
    ) -> Result<(), WorldError> {
        let location = self
            .stable_rooms
            .get(&snapshot.room_id)
            .copied()
            .ok_or(WorldError::InvalidEntity)?;
        room_checked(&self.entities, location)?;
        let character_id = *self
            .sessions
            .get(&session)
            .ok_or(WorldError::UnknownSession)?;
        match self.entities.get_mut(character_id) {
            Some(Entity::Character(character)) => {
                character.name.clone_from(&snapshot.name);
                character.location = location;
                Ok(())
            }
            _ => Err(WorldError::InvalidEntity),
        }
    }

    pub fn apply(
        &mut self,
        session: SessionId,
        action: PlayerAction,
    ) -> Result<ActionOutcome, WorldError> {
        match action {
            PlayerAction::Look => Ok(ActionOutcome {
                scene: self.scene_for(session)?,
                changed_character: None,
            }),
            PlayerAction::Move(direction) => {
                let location = self.character(session)?.location;
                let destination = room_checked(&self.entities, location)?
                    .exits
                    .get(&direction)
                    .copied()
                    .ok_or(WorldError::NoExit)?;
                // Validate the destination before mutating character state. This
                // turns a corrupt graph edge into a recoverable world error.
                room_checked(&self.entities, destination)?;
                let character_id = *self
                    .sessions
                    .get(&session)
                    .ok_or(WorldError::UnknownSession)?;
                match self.entities.get_mut(character_id) {
                    Some(Entity::Character(character)) => character.location = destination,
                    _ => return Err(WorldError::InvalidEntity),
                }
                self.revision = self.revision.wrapping_add(1).max(1);
                Ok(ActionOutcome {
                    scene: self.scene_for(session)?,
                    changed_character: Some(self.character_snapshot(session)?),
                })
            }
        }
    }

    pub fn scene_for(&self, session: SessionId) -> Result<SceneView, WorldError> {
        let current_room = room_checked(&self.entities, self.character(session)?.location)?;
        let exits = current_room
            .exits
            .iter()
            .map(|(direction, destination)| {
                let destination = room_checked(&self.entities, *destination)?;
                Ok(SceneExit {
                    id: destination.stable_id.clone(),
                    label: direction.as_str().to_owned(),
                    command: direction.as_str().to_owned(),
                })
            })
            .collect::<Result<Vec<_>, WorldError>>()?;
        Ok(SceneView {
            view_id: current_room.stable_id.clone(),
            revision: self.revision,
            title: current_room.title.clone(),
            prose: current_room.prose.clone(),
            image: current_room.image.clone(),
            exits,
        })
    }

    fn character(&self, session: SessionId) -> Result<&Character, WorldError> {
        let character_id = *self
            .sessions
            .get(&session)
            .ok_or(WorldError::UnknownSession)?;
        match self.entities.get(character_id) {
            Some(Entity::Character(character)) => Ok(character),
            _ => Err(WorldError::InvalidEntity),
        }
    }
}

fn room(entities: &SlotMap<EntityId, Entity>, id: EntityId) -> &Room {
    match &entities[id] {
        Entity::Room(room) => room,
        Entity::Character(_) => unreachable!("room reference points to a character"),
    }
}

fn room_mut(entities: &mut SlotMap<EntityId, Entity>, id: EntityId) -> &mut Room {
    match &mut entities[id] {
        Entity::Room(room) => room,
        Entity::Character(_) => unreachable!("room reference points to a character"),
    }
}

fn room_checked(entities: &SlotMap<EntityId, Entity>, id: EntityId) -> Result<&Room, WorldError> {
    match entities.get(id) {
        Some(Entity::Room(room)) => Ok(room),
        Some(Entity::Character(_)) | None => Err(WorldError::InvalidEntity),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn connected_world() -> (World, SessionId) {
        let mut world = World::demo();
        let session = SessionId(7);
        world
            .connect(
                session,
                CharacterSnapshot {
                    name: "Tester".into(),
                    room_id: world.default_room_id().to_owned(),
                },
            )
            .unwrap();
        (world, session)
    }

    #[test]
    fn parses_legacy_friendly_movement_commands() {
        assert_eq!(parse_action("l"), Ok(PlayerAction::Look));
        assert_eq!(
            parse_action("go north"),
            Ok(PlayerAction::Move(Direction::North))
        );
    }

    #[test]
    fn movement_changes_the_stable_room_reference() {
        let (mut world, session) = connected_world();
        let outcome = world
            .apply(session, PlayerAction::Move(Direction::North))
            .unwrap();
        assert_eq!(outcome.scene.view_id, "demo:north-gate");
        assert_eq!(
            outcome.changed_character.unwrap().room_id,
            "demo:north-gate"
        );
    }

    #[test]
    fn failed_movement_does_not_change_state() {
        let (mut world, session) = connected_world();
        let before = world.character_snapshot(session).unwrap();
        assert!(matches!(
            world.apply(session, PlayerAction::Move(Direction::West)),
            Err(WorldError::NoExit)
        ));
        assert_eq!(world.character_snapshot(session).unwrap(), before);
    }

    #[test]
    fn unknown_persisted_rooms_fall_back_safely() {
        let mut world = World::demo();
        let session = SessionId(11);
        let scene = world
            .connect(
                session,
                CharacterSnapshot {
                    name: "Old Save".into(),
                    room_id: "missing:room".into(),
                },
            )
            .unwrap();
        assert_eq!(scene.view_id, "demo:lantern-square");
    }

    #[test]
    fn corrupt_exit_references_return_an_error_instead_of_panicking() {
        let (mut world, session) = connected_world();
        let character_id = world.sessions[&session];
        room_mut(&mut world.entities, world.default_room)
            .exits
            .insert(Direction::West, character_id);

        assert!(matches!(
            world.scene_for(session),
            Err(WorldError::InvalidEntity)
        ));
        assert!(matches!(
            world.apply(session, PlayerAction::Move(Direction::West)),
            Err(WorldError::InvalidEntity)
        ));
        assert_eq!(
            world.character_snapshot(session).unwrap().room_id,
            "demo:lantern-square"
        );
    }
}
