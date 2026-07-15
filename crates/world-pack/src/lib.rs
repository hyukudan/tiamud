//! Portable, versioned world data produced by importers and authoring tools.

use std::collections::{HashMap, HashSet, VecDeque};

use serde::{Deserialize, Serialize};
use thiserror::Error;

mod authoring;
mod composition;

pub use authoring::{
    AudioChannel, AudioCue, Condition, ConditionContext, ConditionField, ConditionValidationError,
    ConditionValidationErrors, ConditionValue, ContentTest, ContentTestExpectation,
    ContentTestFailure, ContentTestReport, DocumentToolError, EntityDiff, LightLevel,
    LocalizedText, PreviewFormat, ResolvedScene, RoomMediaDefinition, RoomPerceptionDefinition,
    RoomSceneVariants, SceneResolutionError, SceneVariant, ScriptCapability, ScriptDefinition,
    ScriptHook, ScriptLimitsDefinition, TimePeriod, Weather, WorldPackDiff, WorldPackDocument,
};
pub use composition::{
    AREA_PACK_SCHEMA_VERSION, AreaComposition, AreaCrossReference, AreaCrossReferenceKind,
    AreaOwnership, AreaPack, ComposedWorld, CompositionError, CompositionReport, MAX_AREA_COUNT,
    MAX_AREA_FILE_BYTES, MAX_COMPOSED_WORLD_BYTES, MAX_PROJECT_MANIFEST_BYTES,
    MAX_TOTAL_AREA_BYTES, ProjectArea, WORLD_PROJECT_SCHEMA_VERSION, WorldProjectManifest,
    compose_world_project,
};

pub const CURRENT_SCHEMA_VERSION: u32 = 2;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WorldPack {
    pub schema_version: u32,
    pub id: String,
    pub name: String,
    /// Stable room ID used for new characters and invalid legacy locations.
    #[serde(default)]
    pub start_room_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source: Option<SourceMetadata>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub items: Vec<Item>,
    #[serde(default)]
    pub rooms: Vec<Room>,
    #[serde(default, skip_serializing_if = "GameplayDefinition::is_empty")]
    pub gameplay: GameplayDefinition,
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
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub asset_manifest: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub exits: Vec<Exit>,
    /// Stable IDs of the item instances initially visible in this room.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub items: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub legacy: Option<LegacyRoomData>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Item {
    pub id: String,
    pub name: String,
    pub description: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub aliases: Vec<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GameplayDefinition {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rng_seed: Option<u64>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub item_rules: Vec<ItemRule>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub npc_prototypes: Vec<NpcPrototype>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub npc_spawns: Vec<NpcSpawn>,
}

impl GameplayDefinition {
    pub fn is_empty(&self) -> bool {
        self.rng_seed.is_none()
            && self.item_rules.is_empty()
            && self.npc_prototypes.is_empty()
            && self.npc_spawns.is_empty()
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ItemRule {
    pub item_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub equipment_slot: Option<EquipmentSlot>,
    #[serde(default)]
    pub attack_bonus: i32,
    #[serde(default)]
    pub heal_amount: i32,
    #[serde(default)]
    pub consumable: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub effect: Option<EffectRule>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EquipmentSlot {
    Weapon,
    Body,
    Trinket,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EffectRule {
    pub kind: EffectKind,
    pub magnitude: i32,
    pub remaining_ticks: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EffectKind {
    Regeneration,
    Strength,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NpcPrototype {
    pub id: String,
    pub name: String,
    pub description: String,
    pub level: u32,
    pub max_health: i32,
    pub strength: i32,
    pub defence: i32,
    pub experience_reward: u64,
    pub respawn_ticks: u64,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub loot_item_ids: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NpcSpawn {
    pub id: String,
    pub prototype_id: String,
    pub room_id: String,
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

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Ord, PartialOrd, Serialize, Deserialize)]
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
        if self.start_room_id.trim().is_empty() {
            errors.push(ValidationError::EmptyStartRoomId);
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
            if let Some(path) = &room.asset_manifest
                && !valid_relative_asset_path(path)
            {
                errors.push(ValidationError::InvalidAssetManifestPath {
                    room_id: room.id.clone(),
                    path: path.clone(),
                });
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

            let mut room_items = HashSet::new();
            for item_id in &room.items {
                if !room_items.insert(item_id.as_str()) {
                    errors.push(ValidationError::DuplicateRoomItem {
                        room_id: room.id.clone(),
                        item_id: item_id.clone(),
                    });
                }
            }
        }

        if room_ids.contains(self.start_room_id.as_str()) {
            let mut reachable = HashSet::from([self.start_room_id.as_str()]);
            let mut pending = VecDeque::from([self.start_room_id.as_str()]);
            while let Some(room_id) = pending.pop_front() {
                let Some(room) = self.rooms.iter().find(|room| room.id == room_id) else {
                    continue;
                };
                for exit in &room.exits {
                    let target = match &exit.target {
                        ExitTarget::Room { room_id } => Some(room_id.as_str()),
                        ExitTarget::LegacyVnum { vnum } => legacy_vnums.get(vnum).copied(),
                    };
                    if let Some(target) = target
                        && room_ids.contains(target)
                        && reachable.insert(target)
                    {
                        pending.push_back(target);
                    }
                }
            }
            for room in &self.rooms {
                if !reachable.contains(room.id.as_str()) {
                    errors.push(ValidationError::UnreachableRoom {
                        room_id: room.id.clone(),
                        start_room_id: self.start_room_id.clone(),
                    });
                }
            }
        }

        if !self.start_room_id.trim().is_empty() && !room_ids.contains(self.start_room_id.as_str())
        {
            errors.push(ValidationError::MissingStartRoom(
                self.start_room_id.clone(),
            ));
        }

        let mut item_ids = HashSet::new();
        for item in &self.items {
            if item.id.trim().is_empty() {
                errors.push(ValidationError::EmptyItemId);
            } else if !item_ids.insert(item.id.as_str()) {
                errors.push(ValidationError::DuplicateItemId(item.id.clone()));
            }
            if item.name.trim().is_empty() {
                errors.push(ValidationError::EmptyItemName(item.id.clone()));
            }
            if item.description.trim().is_empty() {
                errors.push(ValidationError::EmptyItemDescription(item.id.clone()));
            }
        }

        let mut placed_items = HashMap::new();
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
            let mut room_items = HashSet::new();
            for item_id in &room.items {
                if !room_items.insert(item_id.as_str()) {
                    continue;
                }
                if !item_ids.contains(item_id.as_str()) {
                    errors.push(ValidationError::MissingItemTarget {
                        room_id: room.id.clone(),
                        item_id: item_id.clone(),
                    });
                } else if let Some(first_room) =
                    placed_items.insert(item_id.as_str(), room.id.as_str())
                {
                    errors.push(ValidationError::ItemPlacedMoreThanOnce {
                        item_id: item_id.clone(),
                        first_room: first_room.to_owned(),
                        second_room: room.id.clone(),
                    });
                }
            }
        }
        for item_id in item_ids {
            if !placed_items.contains_key(item_id) {
                errors.push(ValidationError::UnplacedItem(item_id.to_owned()));
            }
        }

        let declared_items = self
            .items
            .iter()
            .map(|item| item.id.as_str())
            .collect::<HashSet<_>>();
        let mut ruled_items = HashSet::new();
        for rule in &self.gameplay.item_rules {
            if !ruled_items.insert(rule.item_id.as_str()) {
                errors.push(ValidationError::InvalidGameplay(format!(
                    "item {:?} has more than one gameplay rule",
                    rule.item_id
                )));
            }
            if !declared_items.contains(rule.item_id.as_str()) {
                errors.push(ValidationError::InvalidGameplay(format!(
                    "item gameplay rule references missing item {:?}",
                    rule.item_id
                )));
            }
            if rule.attack_bonus < 0
                || rule.heal_amount < 0
                || rule
                    .effect
                    .is_some_and(|effect| effect.magnitude <= 0 || effect.remaining_ticks == 0)
            {
                errors.push(ValidationError::InvalidGameplay(format!(
                    "item {:?} has invalid bonuses or effect bounds",
                    rule.item_id
                )));
            }
        }

        let mut prototypes = HashMap::new();
        for prototype in &self.gameplay.npc_prototypes {
            if prototype.id.trim().is_empty()
                || prototype.name.trim().is_empty()
                || prototype.description.trim().is_empty()
                || prototype.level == 0
                || prototype.max_health <= 0
                || prototype.strength <= 0
                || prototype.defence < 0
                || prototype.respawn_ticks == 0
            {
                errors.push(ValidationError::InvalidGameplay(format!(
                    "NPC prototype {:?} has invalid required fields",
                    prototype.id
                )));
            }
            if prototypes
                .insert(prototype.id.as_str(), prototype)
                .is_some()
            {
                errors.push(ValidationError::InvalidGameplay(format!(
                    "NPC prototype {:?} appears more than once",
                    prototype.id
                )));
            }
            let mut loot = HashSet::new();
            for item_id in &prototype.loot_item_ids {
                if !loot.insert(item_id.as_str()) {
                    errors.push(ValidationError::InvalidGameplay(format!(
                        "NPC prototype {:?} repeats loot item {:?}",
                        prototype.id, item_id
                    )));
                }
                if !declared_items.contains(item_id.as_str()) {
                    errors.push(ValidationError::InvalidGameplay(format!(
                        "NPC prototype {:?} references missing loot item {:?}",
                        prototype.id, item_id
                    )));
                }
            }
        }

        let mut spawn_ids = HashSet::new();
        let mut used_prototypes = HashSet::new();
        let mut allocated_loot = HashSet::new();
        for spawn in &self.gameplay.npc_spawns {
            if spawn.id.trim().is_empty() || !spawn_ids.insert(spawn.id.as_str()) {
                errors.push(ValidationError::InvalidGameplay(format!(
                    "NPC spawn id {:?} is empty or duplicated",
                    spawn.id
                )));
            }
            if !room_ids.contains(spawn.room_id.as_str()) {
                errors.push(ValidationError::InvalidGameplay(format!(
                    "NPC spawn {:?} references missing room {:?}",
                    spawn.id, spawn.room_id
                )));
            }
            match prototypes.get(spawn.prototype_id.as_str()) {
                Some(prototype) => {
                    used_prototypes.insert(spawn.prototype_id.as_str());
                    for item_id in &prototype.loot_item_ids {
                        if !allocated_loot.insert(item_id.as_str()) {
                            errors.push(ValidationError::InvalidGameplay(format!(
                                "loot item {:?} is allocated to more than one NPC spawn",
                                item_id
                            )));
                        }
                    }
                }
                None => errors.push(ValidationError::InvalidGameplay(format!(
                    "NPC spawn {:?} references missing prototype {:?}",
                    spawn.id, spawn.prototype_id
                ))),
            }
        }
        for prototype in self
            .gameplay
            .npc_prototypes
            .iter()
            .filter(|prototype| !used_prototypes.contains(prototype.id.as_str()))
        {
            errors.push(ValidationError::InvalidGameplay(format!(
                "NPC prototype {:?} has no spawn",
                prototype.id
            )));
        }

        if errors.is_empty() {
            Ok(())
        } else {
            Err(ValidationErrors(errors))
        }
    }
}

fn valid_relative_asset_path(path: &str) -> bool {
    !path.trim().is_empty()
        && !path.starts_with('/')
        && !has_windows_drive_prefix(path)
        && !path.contains('\\')
        && path
            .split('/')
            .all(|component| !matches!(component, "." | ".."))
}

pub(crate) fn valid_content_addressed_audio_path(path: &str) -> bool {
    if path.len() > 160 || path.contains(['?', '#', '\\']) {
        return false;
    }
    let mut components = path.split('/');
    let (Some("assets"), Some(digest), Some(file), None) = (
        components.next(),
        components.next(),
        components.next(),
        components.next(),
    ) else {
        return false;
    };
    digest.len() == 64
        && digest
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
        && valid_audio_file_name(file)
        && file.bytes().all(|byte| {
            byte.is_ascii_lowercase() || byte.is_ascii_digit() || matches!(byte, b'-' | b'.')
        })
}

fn valid_audio_file_name(file: &str) -> bool {
    [".opus", ".ogg", ".mp3"].iter().any(|extension| {
        file.strip_prefix("audio-")
            .and_then(|name| name.strip_suffix(extension))
            .is_some_and(|name| !name.is_empty())
    })
}

fn has_windows_drive_prefix(path: &str) -> bool {
    let bytes = path.as_bytes();
    matches!(bytes, [drive, b':', b'/', ..] if drive.is_ascii_alphabetic())
}

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum ValidationError {
    #[error("schema version {found} is unsupported; this build supports {supported}")]
    UnsupportedSchemaVersion { found: u32, supported: u32 },
    #[error("pack id must not be empty")]
    EmptyPackId,
    #[error("pack name must not be empty")]
    EmptyPackName,
    #[error("start room id must not be empty")]
    EmptyStartRoomId,
    #[error("start room {0:?} does not exist")]
    MissingStartRoom(String),
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
    #[error("room {room_id:?} has invalid relative asset manifest path {path:?}")]
    InvalidAssetManifestPath { room_id: String, path: String },
    #[error("room {room_id:?} defines direction {direction:?} more than once")]
    DuplicateExitDirection {
        room_id: String,
        direction: Direction,
    },
    #[error("room {room_id:?} points to missing room {target:?}")]
    MissingRoomTarget { room_id: String, target: String },
    #[error("room {room_id:?} is unreachable from start room {start_room_id:?}")]
    UnreachableRoom {
        room_id: String,
        start_room_id: String,
    },
    #[error("item id must not be empty")]
    EmptyItemId,
    #[error("item id {0:?} appears more than once")]
    DuplicateItemId(String),
    #[error("item {0:?} must have a name")]
    EmptyItemName(String),
    #[error("item {0:?} must have a description")]
    EmptyItemDescription(String),
    #[error("room {room_id:?} lists item {item_id:?} more than once")]
    DuplicateRoomItem { room_id: String, item_id: String },
    #[error("room {room_id:?} points to missing item {item_id:?}")]
    MissingItemTarget { room_id: String, item_id: String },
    #[error("item {item_id:?} is placed in both {first_room:?} and {second_room:?}")]
    ItemPlacedMoreThanOnce {
        item_id: String,
        first_room: String,
        second_room: String,
    },
    #[error("item {0:?} has no initial room")]
    UnplacedItem(String),
    #[error("invalid gameplay definition: {0}")]
    InvalidGameplay(String),
    #[error("scene variants reference missing room {room_id:?}")]
    MissingSceneVariantRoom { room_id: String },
    #[error("room {room_id:?} defines scene variants more than once")]
    DuplicateSceneVariantSet { room_id: String },
    #[error("room {room_id:?} has a scene variant with an empty id")]
    EmptySceneVariantId { room_id: String },
    #[error("room {room_id:?} defines scene variant {variant_id:?} more than once")]
    DuplicateSceneVariantId { room_id: String, variant_id: String },
    #[error(
        "room {room_id:?} scene variants {first_variant:?} and {second_variant:?} share priority {priority}"
    )]
    DuplicateSceneVariantPriority {
        room_id: String,
        priority: i32,
        first_variant: String,
        second_variant: String,
    },
    #[error("room {room_id:?} has variants but no unconditional default scene variant")]
    MissingDefaultSceneVariant { room_id: String },
    #[error(
        "room {room_id:?} has more than one unconditional default scene variant ({first_variant:?}, {second_variant:?})"
    )]
    DuplicateDefaultSceneVariant {
        room_id: String,
        first_variant: String,
        second_variant: String,
    },
    #[error("room {room_id:?} scene variant {variant_id:?} does not override any scene content")]
    EmptySceneVariant { room_id: String, variant_id: String },
    #[error("room {room_id:?} scene variant {variant_id:?} has an empty title override")]
    EmptySceneVariantTitle { room_id: String, variant_id: String },
    #[error("room {room_id:?} scene variant {variant_id:?} has an empty description override")]
    EmptySceneVariantDescription { room_id: String, variant_id: String },
    #[error(
        "room {room_id:?} scene variant {variant_id:?} has invalid relative asset manifest path {path:?}"
    )]
    InvalidSceneVariantAssetManifestPath {
        room_id: String,
        variant_id: String,
        path: String,
    },
    #[error("room media references missing room {room_id:?}")]
    MissingRoomMediaRoom { room_id: String },
    #[error("room {room_id:?} defines media more than once")]
    DuplicateRoomMedia { room_id: String },
    #[error("room {room_id:?} has an image manifest but no valid localized image alt text")]
    MissingRoomImageAlt { room_id: String },
    #[error("room {room_id:?} has localized image alt text without an image manifest")]
    RoomImageAltWithoutManifest { room_id: String },
    #[error("room {room_id:?} has invalid localized image alt text: {reason}")]
    InvalidRoomImageAlt { room_id: String, reason: String },
    #[error("room {room_id:?} has an invalid audio cue: {reason}")]
    InvalidRoomAudio { room_id: String, reason: String },
    #[error("perception policy references missing room {room_id:?}")]
    MissingRoomPerceptionRoom { room_id: String },
    #[error("room {room_id:?} defines perception policy more than once")]
    DuplicateRoomPerception { room_id: String },
    #[error("room {room_id:?} has invalid perception policy: {reason}")]
    InvalidPerception { room_id: String, reason: String },
    #[error(
        "room {room_id:?} scene variant {variant_id:?} has an image manifest but no valid localized image alt text"
    )]
    MissingSceneVariantImageAlt { room_id: String, variant_id: String },
    #[error(
        "room {room_id:?} scene variant {variant_id:?} has localized image alt text without an inherited or overridden image manifest"
    )]
    SceneVariantImageAltWithoutManifest { room_id: String, variant_id: String },
    #[error(
        "room {room_id:?} scene variant {variant_id:?} has invalid localized image alt text: {reason}"
    )]
    InvalidSceneVariantImageAlt {
        room_id: String,
        variant_id: String,
        reason: String,
    },
    #[error("room {room_id:?} scene variant {variant_id:?} has an invalid audio cue: {reason}")]
    InvalidSceneVariantAudio {
        room_id: String,
        variant_id: String,
        reason: String,
    },
    #[error("room {room_id:?} scene variant {variant_id:?} has an invalid condition: {reason}")]
    InvalidSceneVariantCondition {
        room_id: String,
        variant_id: String,
        reason: String,
    },
    #[error("a WorldPack may declare at most 256 scripts")]
    TooManyScripts,
    #[error("script id must not be empty")]
    EmptyScriptId,
    #[error("script id {0:?} appears more than once")]
    DuplicateScriptId(String),
    #[error("script {script_id:?} references missing room {room_id:?}")]
    MissingScriptRoom { script_id: String, room_id: String },
    #[error("script {0:?} must declare at least one hook")]
    EmptyScriptHooks(String),
    #[error("script {script_id:?} declares hook {hook:?} more than once")]
    DuplicateScriptHook { script_id: String, hook: ScriptHook },
    #[error("script {script_id:?} declares capability {capability:?} more than once")]
    DuplicateScriptCapability {
        script_id: String,
        capability: ScriptCapability,
    },
    #[error("script {0:?} source must not be empty")]
    EmptyScriptSource(String),
    #[error("script {script_id:?} source has {actual} bytes; its limit is {maximum}")]
    ScriptSourceTooLarge {
        script_id: String,
        actual: usize,
        maximum: usize,
    },
    #[error("script {0:?} has invalid or excessive runtime limits")]
    InvalidScriptLimits(String),
    #[error("content test name must not be empty")]
    EmptyContentTestName,
    #[error("content test {0:?} appears more than once")]
    DuplicateContentTestName(String),
    #[error("content test {test_name:?} references missing room {room_id:?}")]
    MissingContentTestRoom { test_name: String, room_id: String },
    #[error("content test {0:?} does not define any expectation")]
    EmptyContentTestExpectation(String),
    #[error("content test {test_name:?} has invalid context: {reason}")]
    InvalidContentTestContext { test_name: String, reason: String },
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
            asset_manifest: None,
            exits: Vec::new(),
            items: Vec::new(),
            legacy: None,
        }
    }

    fn pack() -> WorldPack {
        WorldPack {
            schema_version: CURRENT_SCHEMA_VERSION,
            id: "test".to_owned(),
            name: "Test".to_owned(),
            start_room_id: "test:1".to_owned(),
            source: None,
            items: Vec::new(),
            rooms: vec![room("test:1", 1)],
            gameplay: GameplayDefinition::default(),
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
    fn gameplay_references_and_resource_bounds_are_validated() {
        let mut pack = pack();
        pack.gameplay.item_rules.push(ItemRule {
            item_id: "missing:item".to_owned(),
            equipment_slot: None,
            attack_bonus: -1,
            heal_amount: 0,
            consumable: false,
            effect: None,
        });
        pack.gameplay.npc_prototypes.push(NpcPrototype {
            id: "demo:wolf".to_owned(),
            name: "wolf".to_owned(),
            description: "A watchful wolf.".to_owned(),
            level: 1,
            max_health: 10,
            strength: 2,
            defence: 0,
            experience_reward: 10,
            respawn_ticks: 2,
            loot_item_ids: vec!["missing:loot".to_owned()],
        });
        pack.gameplay.npc_spawns.push(NpcSpawn {
            id: "demo:wolf-1".to_owned(),
            prototype_id: "demo:wolf".to_owned(),
            room_id: "missing:room".to_owned(),
        });
        let errors = pack.validate().unwrap_err().0;
        assert!(
            errors
                .iter()
                .filter(|error| matches!(error, ValidationError::InvalidGameplay(_)))
                .count()
                >= 3
        );
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

    #[test]
    fn asset_manifest_must_be_a_safe_relative_path() {
        for invalid in [
            "",
            "   ",
            "/assets/manifest.json",
            "C:/assets/manifest.json",
            "./assets/manifest.json",
            "assets/./manifest.json",
            "assets/../manifest.json",
            "assets\\manifest.json",
        ] {
            let mut pack = pack();
            pack.rooms[0].asset_manifest = Some(invalid.to_owned());

            assert_eq!(
                pack.validate().unwrap_err().0,
                vec![ValidationError::InvalidAssetManifestPath {
                    room_id: "test:1".to_owned(),
                    path: invalid.to_owned(),
                }],
                "path {invalid:?} should have been rejected"
            );
        }
    }

    #[test]
    fn nested_relative_asset_manifest_is_valid() {
        let mut pack = pack();
        pack.rooms[0].asset_manifest = Some("assets/0123456789abcdef/manifest.json".to_owned());

        pack.validate().unwrap();
    }

    #[test]
    fn start_room_is_explicit_and_must_reference_a_room() {
        let mut missing = pack();
        missing.start_room_id = "test:missing".into();
        assert_eq!(
            missing.validate().unwrap_err().0,
            vec![ValidationError::MissingStartRoom("test:missing".into())]
        );

        let mut empty = pack();
        empty.start_room_id.clear();
        assert_eq!(
            empty.validate().unwrap_err().0,
            vec![ValidationError::EmptyStartRoomId]
        );
    }

    #[test]
    fn every_room_must_be_reachable_from_the_explicit_start() {
        let mut disconnected = pack();
        disconnected.rooms.push(room("test:isolated", 2));
        assert!(disconnected.validate().unwrap_err().0.contains(
            &ValidationError::UnreachableRoom {
                room_id: "test:isolated".into(),
                start_room_id: "test:1".into(),
            }
        ));
    }

    #[test]
    fn v1_json_can_be_read_for_a_clear_migration_error() {
        let legacy: WorldPack = serde_json::from_str(
            r#"{
                "schemaVersion": 1,
                "id": "legacy",
                "name": "Legacy",
                "rooms": [{
                    "id": "legacy:start",
                    "title": "Start",
                    "description": "Old data."
                }]
            }"#,
        )
        .unwrap();
        let errors = legacy.validate().unwrap_err().0;
        assert!(errors.contains(&ValidationError::UnsupportedSchemaVersion {
            found: 1,
            supported: 2,
        }));
        assert!(errors.contains(&ValidationError::EmptyStartRoomId));
    }

    #[test]
    fn item_ids_and_initial_room_references_are_validated() {
        let item = Item {
            id: "test:lantern".into(),
            name: "lantern".into(),
            description: "A lantern.".into(),
            aliases: vec!["lamp".into()],
        };
        let mut valid = pack();
        valid.items.push(item.clone());
        valid.rooms[0].items.push(item.id.clone());
        valid.validate().unwrap();

        let mut duplicate_id = valid.clone();
        duplicate_id.items.push(item.clone());
        assert!(
            duplicate_id
                .validate()
                .unwrap_err()
                .0
                .contains(&ValidationError::DuplicateItemId(item.id.clone()))
        );

        let mut missing = pack();
        missing.rooms[0].items.push("test:missing".into());
        assert_eq!(
            missing.validate().unwrap_err().0,
            vec![ValidationError::MissingItemTarget {
                room_id: "test:1".into(),
                item_id: "test:missing".into(),
            }]
        );

        let mut duplicate_room_reference = valid.clone();
        duplicate_room_reference.rooms[0]
            .items
            .push(item.id.clone());
        assert!(duplicate_room_reference.validate().unwrap_err().0.contains(
            &ValidationError::DuplicateRoomItem {
                room_id: "test:1".into(),
                item_id: item.id.clone(),
            }
        ));

        let mut unplaced = pack();
        unplaced.items.push(item);
        assert_eq!(
            unplaced.validate().unwrap_err().0,
            vec![ValidationError::UnplacedItem("test:lantern".into())]
        );
    }

    #[test]
    fn an_item_cannot_start_in_two_rooms() {
        let mut value = pack();
        value.items.push(Item {
            id: "test:lantern".into(),
            name: "lantern".into(),
            description: "A lantern.".into(),
            aliases: Vec::new(),
        });
        value.rooms[0].items.push("test:lantern".into());
        value.rooms[0].exits.push(Exit {
            direction: Direction::North,
            target: ExitTarget::Room {
                room_id: "test:2".into(),
            },
            description: String::new(),
            keywords: String::new(),
            legacy_values: Vec::new(),
        });
        let mut other = room("test:2", 2);
        other.items.push("test:lantern".into());
        value.rooms.push(other);

        assert_eq!(
            value.validate().unwrap_err().0,
            vec![ValidationError::ItemPlacedMoreThanOnce {
                item_id: "test:lantern".into(),
                first_room: "test:1".into(),
                second_room: "test:2".into(),
            }]
        );
    }
}
