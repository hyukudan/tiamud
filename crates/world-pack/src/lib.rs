//! Portable, versioned world data produced by importers and authoring tools.

use std::collections::{HashMap, HashSet};

use serde::{Deserialize, Serialize};
use thiserror::Error;

pub const CURRENT_SCHEMA_VERSION: u32 = 1;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WorldPack {
    pub schema_version: u32,
    pub id: String,
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source: Option<SourceMetadata>,
    #[serde(default)]
    pub rooms: Vec<Room>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SourceMetadata {
    pub format: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub dialect: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub file: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Room {
    pub id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub legacy_vnum: Option<i64>,
    pub title: String,
    pub description: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub exits: Vec<Exit>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub legacy: Option<LegacyRoomData>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LegacyRoomData {
    pub flags: i64,
    pub sector: i64,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub prefix_values: Vec<i64>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Exit {
    pub direction: Direction,
    pub target: ExitTarget,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub description: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub keywords: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub legacy_values: Vec<i64>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Direction {
    North,
    East,
    South,
    West,
    Up,
    Down,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ExitTarget {
    Room { room_id: String },
    LegacyVnum { vnum: i64 },
}

impl WorldPack {
    pub fn validate(&self) -> Result<(), ValidationErrors> {
        let mut errors = Vec::new();

        if self.schema_version != CURRENT_SCHEMA_VERSION {
            errors.push(ValidationError::UnsupportedSchemaVersion {
                found: self.schema_version,
                supported: CURRENT_SCHEMA_VERSION,
            });
        }
        if self.id.trim().is_empty() {
            errors.push(ValidationError::EmptyPackId);
        }
        if self.name.trim().is_empty() {
            errors.push(ValidationError::EmptyPackName);
        }

        let mut room_ids = HashSet::new();
        let mut legacy_vnums = HashMap::new();
        for room in &self.rooms {
            if room.id.trim().is_empty() {
                errors.push(ValidationError::EmptyRoomId);
            } else if !room_ids.insert(room.id.as_str()) {
                errors.push(ValidationError::DuplicateRoomId(room.id.clone()));
            }

            if let Some(vnum) = room.legacy_vnum
                && let Some(first_room) = legacy_vnums.insert(vnum, room.id.as_str())
            {
                errors.push(ValidationError::DuplicateLegacyVnum {
                    vnum,
                    first_room: first_room.to_owned(),
                    second_room: room.id.clone(),
                });
            }

            if room.title.trim().is_empty() {
                errors.push(ValidationError::EmptyRoomTitle(room.id.clone()));
            }

            let mut directions = HashSet::new();
            for exit in &room.exits {
                if !directions.insert(exit.direction) {
                    errors.push(ValidationError::DuplicateExitDirection {
                        room_id: room.id.clone(),
                        direction: exit.direction,
                    });
                }
            }
        }

        for room in &self.rooms {
            for exit in &room.exits {
                if let ExitTarget::Room { room_id } = &exit.target
                    && !room_ids.contains(room_id.as_str())
                {
                    errors.push(ValidationError::MissingRoomTarget {
                        room_id: room.id.clone(),
                        target: room_id.clone(),
                    });
                }
            }
        }

        if errors.is_empty() {
            Ok(())
        } else {
            Err(ValidationErrors(errors))
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum ValidationError {
    #[error("schema version {found} is unsupported; this build supports {supported}")]
    UnsupportedSchemaVersion { found: u32, supported: u32 },
    #[error("pack id must not be empty")]
    EmptyPackId,
    #[error("pack name must not be empty")]
    EmptyPackName,
    #[error("room id must not be empty")]
    EmptyRoomId,
    #[error("room id {0:?} appears more than once")]
    DuplicateRoomId(String),
    #[error("legacy VNUM {vnum} is shared by rooms {first_room:?} and {second_room:?}")]
    DuplicateLegacyVnum {
        vnum: i64,
        first_room: String,
        second_room: String,
    },
    #[error("room {0:?} must have a title")]
    EmptyRoomTitle(String),
    #[error("room {room_id:?} defines direction {direction:?} more than once")]
    DuplicateExitDirection {
        room_id: String,
        direction: Direction,
    },
    #[error("room {room_id:?} points to missing room {target:?}")]
    MissingRoomTarget { room_id: String, target: String },
}

#[derive(Debug, Clone, PartialEq, Eq, Error)]
#[error("world pack validation failed with {} error(s)", .0.len())]
pub struct ValidationErrors(pub Vec<ValidationError>);

#[cfg(test)]
mod tests {
    use super::*;

    fn room(id: &str, vnum: i64) -> Room {
        Room {
            id: id.to_owned(),
            legacy_vnum: Some(vnum),
            title: "A room".to_owned(),
            description: "A useful description.".to_owned(),
            exits: Vec::new(),
            legacy: None,
        }
    }

    fn pack() -> WorldPack {
        WorldPack {
            schema_version: CURRENT_SCHEMA_VERSION,
            id: "test".to_owned(),
            name: "Test".to_owned(),
            source: None,
            rooms: vec![room("test:1", 1)],
        }
    }

    #[test]
    fn valid_pack_round_trips_through_json() {
        let pack = pack();
        pack.validate().unwrap();

        let json = serde_json::to_string(&pack).unwrap();
        let decoded: WorldPack = serde_json::from_str(&json).unwrap();

        assert_eq!(decoded, pack);
    }

    #[test]
    fn duplicate_room_ids_and_vnums_are_rejected() {
        let mut pack = pack();
        pack.rooms.push(room("test:1", 1));

        let errors = pack.validate().unwrap_err().0;

        assert!(errors.contains(&ValidationError::DuplicateRoomId("test:1".to_owned())));
        assert!(
            errors
                .iter()
                .any(|error| matches!(error, ValidationError::DuplicateLegacyVnum { vnum: 1, .. }))
        );
    }

    #[test]
    fn local_room_targets_must_exist_but_legacy_vnums_may_be_external() {
        let mut pack = pack();
        pack.rooms[0].exits = vec![
            Exit {
                direction: Direction::North,
                target: ExitTarget::Room {
                    room_id: "missing:2".to_owned(),
                },
                description: String::new(),
                keywords: String::new(),
                legacy_values: Vec::new(),
            },
            Exit {
                direction: Direction::South,
                target: ExitTarget::LegacyVnum { vnum: 9000 },
                description: String::new(),
                keywords: String::new(),
                legacy_values: Vec::new(),
            },
        ];

        let errors = pack.validate().unwrap_err().0;
        assert_eq!(
            errors,
            vec![ValidationError::MissingRoomTarget {
                room_id: "test:1".to_owned(),
                target: "missing:2".to_owned(),
            }]
        );
    }
}
