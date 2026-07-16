//! Portable, versioned world data produced by importers and authoring tools.

use std::collections::{BTreeMap, HashMap, HashSet, VecDeque};

use serde::{Deserialize, Serialize};
use thiserror::Error;

mod authoring;
mod composition;
mod progression;

pub use authoring::{
    AudioChannel, AudioCue, Condition, ConditionContext, ConditionField, ConditionValidationError,
    ConditionValidationErrors, ConditionValue, ContentTest, ContentTestExpectation,
    ContentTestFailure, ContentTestReport, DocumentToolError, EntityDiff, EntityMediaDefinition,
    EntityMediaKind, EntityMediaLayer, EntitySpritePoses, EntitySpriteSource, LightLevel,
    LocalizedText, MAX_ENTITY_MEDIA_DEFINITIONS, MAX_PROJECTED_ENTITY_MEDIA,
    MAX_PROJECTED_ENTITY_POSES, NormalizedAnchor, NormalizedFootprint, PreviewFormat,
    ResolvedScene, RoomMediaDefinition, RoomPerceptionDefinition, RoomSceneVariants,
    SceneResolutionError, SceneVariant, ScriptCapability, ScriptDefinition, ScriptHook,
    ScriptLimitsDefinition, TimePeriod, Weather, WorldPackDiff, WorldPackDocument,
};
pub use composition::{
    AREA_PACK_SCHEMA_VERSION, AreaComposition, AreaCrossReference, AreaCrossReferenceKind,
    AreaOwnership, AreaPack, ComposedWorld, CompositionError, CompositionReport, MAX_AREA_COUNT,
    MAX_AREA_FILE_BYTES, MAX_COMPOSED_WORLD_BYTES, MAX_PROJECT_MANIFEST_BYTES,
    MAX_TOTAL_AREA_BYTES, ProjectArea, WORLD_PROJECT_SCHEMA_VERSION, WorldProjectManifest,
    compose_world_project,
};
pub use progression::{
    AdvancedClassDefinition, BackgroundDefinition, BaseClassDefinition,
    CharacterDefaultsDefinition, CharacterProgressionDefinition, GrantDefinition,
    LevelGrantDefinition, LevelThresholdDefinition, LineageDefinition, MAX_ADVANCED_CLASSES,
    MAX_BACKGROUNDS, MAX_BASE_CLASSES, MAX_GRANTS_PER_ENTRY, MAX_LINEAGES, MAX_NODES_PER_GRAPH,
    MAX_PROGRESSION_POINT_POOLS, MAX_PROGRESSION_POINTS, MAX_RANKS_PER_NODE, MAX_REQUIREMENT_DEPTH,
    MAX_REQUIREMENT_NODES, MAX_TALENT_GRAPHS, RequirementDefinition, RequirementDisclosure,
    RequirementPredicate, TalentGraphDefinition, TalentNodeDefinition, TalentRankDefinition,
};

pub const CURRENT_SCHEMA_VERSION: u32 = 3;
pub const MAX_ARCHETYPES: usize = 32;
pub const MAX_ABILITIES: usize = 512;
pub const MAX_NPC_ABILITY_SETS: usize = 512;
pub const MAX_PROGRESSION_LEVEL: u32 = 100;
pub const MAX_STATS_PER_ARCHETYPE: usize = 64;
pub const MAX_RESOURCES_PER_ARCHETYPE: usize = 16;
pub const MAX_ABILITIES_PER_ACTOR: usize = 64;
pub const MAX_ABILITY_RESOURCE_COSTS: usize = 4;
pub const MAX_ABILITY_STATUS_EFFECTS: usize = 8;
pub const MAX_ABILITY_COOLDOWN_TICKS: u32 = 10_000;
pub const MAX_ABILITY_EFFECT_DURATION_TICKS: u32 = 10_000;
pub const MAX_ABILITY_EFFECT_MAGNITUDE: i32 = 100_000;
pub const MAX_ABILITY_SCALING_PERCENT: u16 = 1_000;
pub const MAX_GAMEPLAY_VALUE: u32 = 1_000_000;
pub const MAX_SKILL_VALUE: u16 = 1_000;
pub const MAX_RESISTANCE_PERCENT: u16 = 90;

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
    /// Canonical WorldPack v3 character creation and advancement rules.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub character_progression: Option<CharacterProgressionDefinition>,
    /// Archetype selected when character creation does not name one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default_archetype_id: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub archetypes: Vec<ArchetypeDefinition>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub abilities: Vec<AbilityDefinition>,
    /// Ability loadouts are separate from [`NpcPrototype`] so existing Rust
    /// struct literals and WorldPack v2 JSON remain source-compatible.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub npc_ability_sets: Vec<NpcAbilitySet>,
}

impl GameplayDefinition {
    pub fn is_empty(&self) -> bool {
        self.rng_seed.is_none()
            && self.item_rules.is_empty()
            && self.npc_prototypes.is_empty()
            && self.npc_spawns.is_empty()
            && self.character_progression.is_none()
            && self.default_archetype_id.is_none()
            && self.archetypes.is_empty()
            && self.abilities.is_empty()
            && self.npc_ability_sets.is_empty()
    }
}

/// Data-driven class/archetype shown during character creation.
///
/// Attribute, skill and resource keys are bounded identifiers instead of Rust
/// enums so world authors can extend a stat sheet without a schema migration.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ArchetypeDefinition {
    pub id: String,
    pub name: String,
    pub description: String,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub initial_attributes: BTreeMap<String, u32>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub initial_skills: BTreeMap<String, u16>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub initial_resources: BTreeMap<String, u32>,
    /// Amount restored per world tick for each initial resource pool.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub resource_regeneration: BTreeMap<String, u32>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub initial_resistances: BTreeMap<DamageType, u16>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub initial_ability_ids: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub level_progression: Vec<LevelProgressionDefinition>,
}

/// Complete, deterministic rule for entering one level above level 1.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LevelProgressionDefinition {
    pub level: u32,
    /// Cumulative experience required to enter this level.
    pub total_experience_required: u64,
    /// Base-stat increases only: `max_health` raises the ceiling without
    /// healing. The authored cumulative value must remain within engine caps.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub attribute_gains: BTreeMap<String, u32>,
    /// Authored cumulative ranks must remain within the skill cap; runtime
    /// saturation is only a defensive safeguard for restored state.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub skill_gains: BTreeMap<String, u16>,
    /// Capacity increases only: current pools are preserved, and a resource
    /// first introduced by a milestone starts empty.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub resource_gains: BTreeMap<String, u32>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub ability_unlock_ids: Vec<String>,
}

/// A single bounded action model shared by player martial techniques, spells
/// and NPC innate actions.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AbilityDefinition {
    pub id: String,
    pub name: String,
    pub description: String,
    pub category: AbilityCategory,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub resource_costs: BTreeMap<String, u32>,
    #[serde(default)]
    pub cooldown_ticks: u32,
    pub targeting: AbilityTargeting,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub damage: Option<AbilityDamage>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub heal: Option<AbilityHeal>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub status_effects: Vec<AbilityStatusEffect>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AbilityCategory {
    Martial,
    Spell,
    Utility,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AbilityTargeting {
    SelfOnly,
    SingleEnemy,
    SingleAlly,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AbilityEffectTarget {
    Caster,
    Target,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DamageType {
    Physical,
    Arcane,
    Fire,
    Frost,
    Lightning,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AbilityDamage {
    pub target: AbilityEffectTarget,
    pub damage_type: DamageType,
    pub base_amount: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub scaling_attribute: Option<String>,
    #[serde(default)]
    pub scaling_percent: u16,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AbilityHeal {
    pub target: AbilityEffectTarget,
    pub base_amount: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub scaling_attribute: Option<String>,
    #[serde(default)]
    pub scaling_percent: u16,
}

/// Ongoing effects have explicit, closed semantics and hard duration/magnitude
/// limits. The tagged representation prevents impossible field combinations.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(
    tag = "kind",
    rename_all = "snake_case",
    rename_all_fields = "camelCase"
)]
pub enum AbilityStatusEffect {
    AttributeModifier {
        target: AbilityEffectTarget,
        attribute: String,
        magnitude: i32,
        duration_ticks: u32,
    },
    Regeneration {
        target: AbilityEffectTarget,
        magnitude: u32,
        duration_ticks: u32,
    },
    DamageOverTime {
        target: AbilityEffectTarget,
        damage_type: DamageType,
        magnitude: u32,
        duration_ticks: u32,
    },
    Stun {
        target: AbilityEffectTarget,
        duration_ticks: u32,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NpcAbilitySet {
    pub npc_prototype_id: String,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub resource_pools: BTreeMap<String, u32>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub resource_regeneration: BTreeMap<String, u32>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub resistances: BTreeMap<DamageType, u16>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub ability_ids: Vec<String>,
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
    pub flags: LegacyBitvector,
    pub sector: i64,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub prefix_values: Vec<i64>,
}

/// A historical flag field whose source dialect may encode either a numeric
/// mask or ROM-style alphabetic letters. Alphabetic values are kept verbatim:
/// mapping letters to bits without the originating flag table would invent
/// semantics that the importer cannot prove.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum LegacyBitvector {
    Numeric(i64),
    Alphabetic(String),
}

impl LegacyBitvector {
    pub const fn is_zero(&self) -> bool {
        matches!(self, Self::Numeric(0))
    }

    pub fn raw(&self) -> String {
        match self {
            Self::Numeric(value) => value.to_string(),
            Self::Alphabetic(value) => value.clone(),
        }
    }

    fn is_valid(&self) -> bool {
        match self {
            Self::Numeric(_) => true,
            Self::Alphabetic(value) => {
                !value.is_empty() && value.bytes().all(|byte| byte.is_ascii_alphabetic())
            }
        }
    }
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
            if let Some(legacy) = &room.legacy
                && !legacy.flags.is_valid()
            {
                errors.push(ValidationError::InvalidLegacyRoomBitvector {
                    room_id: room.id.clone(),
                    value: legacy.flags.raw(),
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
        let declared_items = self
            .items
            .iter()
            .map(|item| item.id.as_str())
            .collect::<HashSet<_>>();
        let mut ruled_items = HashSet::new();
        let max_gameplay_value =
            i32::try_from(MAX_GAMEPLAY_VALUE).expect("MAX_GAMEPLAY_VALUE fits in i32");
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
            if !(0..=max_gameplay_value).contains(&rule.attack_bonus)
                || !(0..=max_gameplay_value).contains(&rule.heal_amount)
                || rule.effect.is_some_and(|effect| {
                    !(1..=MAX_ABILITY_EFFECT_MAGNITUDE).contains(&effect.magnitude)
                        || !(1..=MAX_ABILITY_EFFECT_DURATION_TICKS)
                            .contains(&effect.remaining_ticks)
                })
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
                || !(1..=MAX_PROGRESSION_LEVEL).contains(&prototype.level)
                || !(1..=max_gameplay_value).contains(&prototype.max_health)
                || !(1..=max_gameplay_value).contains(&prototype.strength)
                || !(0..=max_gameplay_value).contains(&prototype.defence)
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

        validate_character_and_abilities(&self.gameplay, &prototypes, &mut errors);
        if let Some(progression) = &self.gameplay.character_progression {
            progression::validate_character_progression(
                progression,
                &self.gameplay.abilities,
                &mut errors,
            );
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

        for item_id in item_ids {
            match (
                placed_items.get(item_id).copied(),
                allocated_loot.contains(item_id),
            ) {
                (Some(room_id), true) => {
                    errors.push(ValidationError::ItemPlacedAndAllocatedAsLoot {
                        item_id: item_id.to_owned(),
                        room_id: room_id.to_owned(),
                    });
                }
                (None, false) => errors.push(ValidationError::UnplacedItem(item_id.to_owned())),
                _ => {}
            }
        }

        if errors.is_empty() {
            Ok(())
        } else {
            Err(ValidationErrors(errors))
        }
    }

    /// Conservative topology findings which do not make a pack invalid.
    ///
    /// Directed reachability cannot account for teleportation, scripted travel,
    /// admin-only transitions, or content loaded by a larger composition. The
    /// result is therefore review guidance, not a publication gate.
    pub fn diagnostics(&self) -> Vec<WorldPackDiagnostic> {
        let rooms_by_id = self
            .rooms
            .iter()
            .map(|room| (room.id.as_str(), room))
            .collect::<HashMap<_, _>>();
        if !rooms_by_id.contains_key(self.start_room_id.as_str()) {
            return Vec::new();
        }
        let legacy_vnums = self
            .rooms
            .iter()
            .filter_map(|room| room.legacy_vnum.map(|vnum| (vnum, room.id.as_str())))
            .collect::<HashMap<_, _>>();
        let mut reachable = HashSet::from([self.start_room_id.as_str()]);
        let mut pending = VecDeque::from([self.start_room_id.as_str()]);
        while let Some(room_id) = pending.pop_front() {
            let Some(room) = rooms_by_id.get(room_id) else {
                continue;
            };
            for exit in &room.exits {
                let target = match &exit.target {
                    ExitTarget::Room { room_id } => Some(room_id.as_str()),
                    ExitTarget::LegacyVnum { vnum } => legacy_vnums.get(vnum).copied(),
                };
                if let Some(target) = target
                    && rooms_by_id.contains_key(target)
                    && reachable.insert(target)
                {
                    pending.push_back(target);
                }
            }
        }
        self.rooms
            .iter()
            .filter(|room| !reachable.contains(room.id.as_str()))
            .map(|room| WorldPackDiagnostic::DirectedUnreachableRoom {
                room_id: room.id.clone(),
                start_room_id: self.start_room_id.clone(),
            })
            .collect()
    }
}

fn validate_character_and_abilities(
    gameplay: &GameplayDefinition,
    prototypes: &HashMap<&str, &NpcPrototype>,
    errors: &mut Vec<ValidationError>,
) {
    if gameplay.abilities.len() > MAX_ABILITIES {
        invalid_gameplay(
            errors,
            format!(
                "world declares {} abilities; the maximum is {MAX_ABILITIES}",
                gameplay.abilities.len()
            ),
        );
    }
    let mut ability_by_id = HashMap::new();
    let mut ability_names = HashSet::new();
    for ability in &gameplay.abilities {
        if !valid_gameplay_identifier(&ability.id) {
            invalid_gameplay(
                errors,
                format!(
                    "ability id {:?} must contain 1..=128 safe bytes",
                    ability.id
                ),
            );
        }
        if !safe_gameplay_text(&ability.name, 128)
            || !safe_gameplay_text(&ability.description, 2_048)
        {
            invalid_gameplay(
                errors,
                format!(
                    "ability {:?} has an invalid name or description",
                    ability.id
                ),
            );
        }
        if ability_by_id.insert(ability.id.as_str(), ability).is_some() {
            invalid_gameplay(
                errors,
                format!("ability {:?} appears more than once", ability.id),
            );
        }
        if !ability_names.insert(ability.name.to_lowercase()) {
            invalid_gameplay(
                errors,
                format!(
                    "ability name {:?} is ambiguous; names must be unique ignoring case",
                    ability.name
                ),
            );
        }
        validate_ability(ability, errors);
    }

    if gameplay.archetypes.len() > MAX_ARCHETYPES {
        invalid_gameplay(
            errors,
            format!(
                "world declares {} archetypes; the maximum is {MAX_ARCHETYPES}",
                gameplay.archetypes.len()
            ),
        );
    }
    if gameplay.archetypes.is_empty() {
        if gameplay.default_archetype_id.is_some() {
            invalid_gameplay(
                errors,
                "defaultArchetypeId requires at least one archetype".to_owned(),
            );
        }
    } else if gameplay.default_archetype_id.is_none() {
        invalid_gameplay(
            errors,
            "worlds with archetypes must declare defaultArchetypeId".to_owned(),
        );
    }

    let mut archetype_ids = HashSet::new();
    for archetype in &gameplay.archetypes {
        if !valid_gameplay_identifier(&archetype.id) {
            invalid_gameplay(
                errors,
                format!(
                    "archetype id {:?} must contain 1..=128 safe bytes",
                    archetype.id
                ),
            );
        }
        if !archetype_ids.insert(archetype.id.as_str()) {
            invalid_gameplay(
                errors,
                format!("archetype {:?} appears more than once", archetype.id),
            );
        }
        if !safe_gameplay_text(&archetype.name, 128)
            || !safe_gameplay_text(&archetype.description, 2_048)
        {
            invalid_gameplay(
                errors,
                format!(
                    "archetype {:?} has an invalid name or description",
                    archetype.id
                ),
            );
        }
        validate_archetype(archetype, &ability_by_id, errors);
    }
    if let Some(default_id) = &gameplay.default_archetype_id
        && !archetype_ids.contains(default_id.as_str())
    {
        invalid_gameplay(
            errors,
            format!("default archetype {default_id:?} does not exist"),
        );
    }

    if gameplay.npc_ability_sets.len() > MAX_NPC_ABILITY_SETS {
        invalid_gameplay(
            errors,
            format!(
                "world declares {} NPC ability sets; the maximum is {MAX_NPC_ABILITY_SETS}",
                gameplay.npc_ability_sets.len()
            ),
        );
    }
    let mut assigned_npcs = HashSet::new();
    for set in &gameplay.npc_ability_sets {
        if !assigned_npcs.insert(set.npc_prototype_id.as_str()) {
            invalid_gameplay(
                errors,
                format!(
                    "NPC prototype {:?} has more than one ability set",
                    set.npc_prototype_id
                ),
            );
        }
        if !prototypes.contains_key(set.npc_prototype_id.as_str()) {
            invalid_gameplay(
                errors,
                format!(
                    "NPC ability set references missing prototype {:?}",
                    set.npc_prototype_id
                ),
            );
        }
        validate_u32_map(
            &set.resource_pools,
            MAX_RESOURCES_PER_ARCHETYPE,
            false,
            &format!("NPC {:?} resource pools", set.npc_prototype_id),
            errors,
        );
        validate_u32_map(
            &set.resource_regeneration,
            MAX_RESOURCES_PER_ARCHETYPE,
            true,
            &format!("NPC {:?} resource regeneration", set.npc_prototype_id),
            errors,
        );
        for resource_id in set.resource_regeneration.keys() {
            if !set.resource_pools.contains_key(resource_id) {
                invalid_gameplay(
                    errors,
                    format!(
                        "NPC {:?} regenerates missing resource {:?}",
                        set.npc_prototype_id, resource_id
                    ),
                );
            }
        }
        validate_resistances(
            &set.resistances,
            &format!("NPC {:?} resistances", set.npc_prototype_id),
            errors,
        );
        validate_actor_abilities(
            &set.ability_ids,
            &set.resource_pools,
            &ability_by_id,
            &format!("NPC {:?}", set.npc_prototype_id),
            errors,
        );
    }
}

fn validate_archetype(
    archetype: &ArchetypeDefinition,
    ability_by_id: &HashMap<&str, &AbilityDefinition>,
    errors: &mut Vec<ValidationError>,
) {
    let mut projected_attribute_ids = HashSet::from(["strength", "defence"]);
    projected_attribute_ids.extend(
        archetype
            .initial_attributes
            .keys()
            .map(String::as_str)
            .filter(|id| *id != "max_health"),
    );
    let mut skill_ids = archetype
        .initial_skills
        .keys()
        .map(String::as_str)
        .collect::<HashSet<_>>();
    let mut projected_attribute_values = archetype.initial_attributes.clone();
    let mut projected_skill_values = archetype.initial_skills.clone();
    validate_u32_map(
        &archetype.initial_attributes,
        MAX_STATS_PER_ARCHETYPE,
        true,
        &format!("archetype {:?} initial attributes", archetype.id),
        errors,
    );
    for required in ["max_health", "strength", "defence"] {
        if !archetype.initial_attributes.contains_key(required) {
            invalid_gameplay(
                errors,
                format!(
                    "archetype {:?} is missing required initial attribute {required:?}",
                    archetype.id
                ),
            );
        }
    }
    if archetype.initial_attributes.get("max_health").copied() == Some(0) {
        invalid_gameplay(
            errors,
            format!("archetype {:?} max_health must be positive", archetype.id),
        );
    }
    validate_u16_map(
        &archetype.initial_skills,
        MAX_STATS_PER_ARCHETYPE,
        true,
        &format!("archetype {:?} initial skills", archetype.id),
        errors,
    );
    validate_u32_map(
        &archetype.initial_resources,
        MAX_RESOURCES_PER_ARCHETYPE,
        false,
        &format!("archetype {:?} initial resources", archetype.id),
        errors,
    );
    validate_u32_map(
        &archetype.resource_regeneration,
        MAX_RESOURCES_PER_ARCHETYPE,
        true,
        &format!("archetype {:?} resource regeneration", archetype.id),
        errors,
    );
    for resource_id in archetype.resource_regeneration.keys() {
        if !archetype.initial_resources.contains_key(resource_id) {
            invalid_gameplay(
                errors,
                format!(
                    "archetype {:?} regenerates missing initial resource {:?}",
                    archetype.id, resource_id
                ),
            );
        }
    }
    validate_resistances(
        &archetype.initial_resistances,
        &format!("archetype {:?} initial resistances", archetype.id),
        errors,
    );

    let context = format!("archetype {:?} initial abilities", archetype.id);
    validate_actor_abilities(
        &archetype.initial_ability_ids,
        &archetype.initial_resources,
        ability_by_id,
        &context,
        errors,
    );
    let mut unlocked = archetype
        .initial_ability_ids
        .iter()
        .map(String::as_str)
        .collect::<HashSet<_>>();
    let mut available_resources = archetype.initial_resources.clone();
    let mut levels = BTreeMap::new();
    for progression in &archetype.level_progression {
        if levels.insert(progression.level, progression).is_some() {
            invalid_gameplay(
                errors,
                format!(
                    "archetype {:?} defines level {} more than once",
                    archetype.id, progression.level
                ),
            );
        }
    }
    if levels.len() > (MAX_PROGRESSION_LEVEL - 1) as usize {
        invalid_gameplay(
            errors,
            format!(
                "archetype {:?} declares too many progression levels",
                archetype.id
            ),
        );
    }
    let mut expected_level = 2;
    let mut previous_experience = 0;
    for (level, progression) in levels {
        if level != expected_level || level > MAX_PROGRESSION_LEVEL {
            invalid_gameplay(
                errors,
                format!(
                    "archetype {:?} progression must be contiguous from level 2 through {MAX_PROGRESSION_LEVEL}",
                    archetype.id
                ),
            );
            expected_level = level.saturating_add(1);
        } else {
            expected_level += 1;
        }
        if progression.total_experience_required <= previous_experience {
            invalid_gameplay(
                errors,
                format!(
                    "archetype {:?} level {level} cumulative experience must increase",
                    archetype.id
                ),
            );
        }
        previous_experience = progression.total_experience_required;
        validate_u32_map(
            &progression.attribute_gains,
            MAX_STATS_PER_ARCHETYPE,
            false,
            &format!("archetype {:?} level {level} attribute gains", archetype.id),
            errors,
        );
        for (attribute_id, gain) in &progression.attribute_gains {
            let value = projected_attribute_values
                .entry(attribute_id.clone())
                .or_default();
            let Some(total) = value.checked_add(*gain) else {
                invalid_gameplay(
                    errors,
                    format!(
                        "archetype {:?} attribute {:?} overflows during progression",
                        archetype.id, attribute_id
                    ),
                );
                continue;
            };
            if total > MAX_GAMEPLAY_VALUE {
                invalid_gameplay(
                    errors,
                    format!(
                        "archetype {:?} attribute {:?} exceeds {MAX_GAMEPLAY_VALUE} during progression",
                        archetype.id, attribute_id
                    ),
                );
            }
            *value = total.min(MAX_GAMEPLAY_VALUE);
        }
        projected_attribute_ids.extend(
            progression
                .attribute_gains
                .keys()
                .map(String::as_str)
                .filter(|id| *id != "max_health"),
        );
        if projected_attribute_ids.len() > MAX_STATS_PER_ARCHETYPE {
            invalid_gameplay(
                errors,
                format!(
                    "archetype {:?} can project more than {MAX_STATS_PER_ARCHETYPE} attributes",
                    archetype.id
                ),
            );
        }
        validate_u16_map(
            &progression.skill_gains,
            MAX_STATS_PER_ARCHETYPE,
            false,
            &format!("archetype {:?} level {level} skill gains", archetype.id),
            errors,
        );
        for (skill_id, gain) in &progression.skill_gains {
            let value = projected_skill_values.entry(skill_id.clone()).or_default();
            let Some(total) = value.checked_add(*gain) else {
                invalid_gameplay(
                    errors,
                    format!(
                        "archetype {:?} skill {:?} overflows during progression",
                        archetype.id, skill_id
                    ),
                );
                continue;
            };
            if total > MAX_SKILL_VALUE {
                invalid_gameplay(
                    errors,
                    format!(
                        "archetype {:?} skill {:?} exceeds {MAX_SKILL_VALUE} during progression",
                        archetype.id, skill_id
                    ),
                );
            }
            *value = total.min(MAX_SKILL_VALUE);
        }
        skill_ids.extend(progression.skill_gains.keys().map(String::as_str));
        if skill_ids.len() > MAX_STATS_PER_ARCHETYPE {
            invalid_gameplay(
                errors,
                format!(
                    "archetype {:?} can learn more than {MAX_STATS_PER_ARCHETYPE} skills",
                    archetype.id
                ),
            );
        }
        validate_u32_map(
            &progression.resource_gains,
            MAX_RESOURCES_PER_ARCHETYPE,
            false,
            &format!("archetype {:?} level {level} resource gains", archetype.id),
            errors,
        );
        for (resource_id, gain) in &progression.resource_gains {
            let pool = available_resources.entry(resource_id.clone()).or_default();
            let Some(total) = pool.checked_add(*gain) else {
                invalid_gameplay(
                    errors,
                    format!(
                        "archetype {:?} resource {:?} overflows during progression",
                        archetype.id, resource_id
                    ),
                );
                continue;
            };
            if total > MAX_GAMEPLAY_VALUE {
                invalid_gameplay(
                    errors,
                    format!(
                        "archetype {:?} resource {:?} exceeds {MAX_GAMEPLAY_VALUE} during progression",
                        archetype.id, resource_id
                    ),
                );
            }
            *pool = total.min(MAX_GAMEPLAY_VALUE);
        }
        if available_resources.len() > MAX_RESOURCES_PER_ARCHETYPE {
            invalid_gameplay(
                errors,
                format!(
                    "archetype {:?} can own more than {MAX_RESOURCES_PER_ARCHETYPE} resources",
                    archetype.id
                ),
            );
        }
        if progression.ability_unlock_ids.len() > MAX_ABILITIES_PER_ACTOR {
            invalid_gameplay(
                errors,
                format!(
                    "archetype {:?} level {level} unlocks too many abilities",
                    archetype.id
                ),
            );
        }
        let mut level_unlocks = HashSet::new();
        for ability_id in &progression.ability_unlock_ids {
            if !level_unlocks.insert(ability_id.as_str()) || !unlocked.insert(ability_id.as_str()) {
                invalid_gameplay(
                    errors,
                    format!(
                        "archetype {:?} unlocks ability {:?} more than once",
                        archetype.id, ability_id
                    ),
                );
                continue;
            }
            validate_ability_reference(
                ability_id,
                &available_resources,
                ability_by_id,
                &format!("archetype {:?} level {level}", archetype.id),
                errors,
            );
        }
        if unlocked.len() > MAX_ABILITIES_PER_ACTOR {
            invalid_gameplay(
                errors,
                format!(
                    "archetype {:?} can unlock more than {MAX_ABILITIES_PER_ACTOR} abilities",
                    archetype.id
                ),
            );
        }
    }
}

fn validate_ability(ability: &AbilityDefinition, errors: &mut Vec<ValidationError>) {
    validate_u32_map(
        &ability.resource_costs,
        MAX_ABILITY_RESOURCE_COSTS,
        false,
        &format!("ability {:?} resource costs", ability.id),
        errors,
    );
    if ability.cooldown_ticks > MAX_ABILITY_COOLDOWN_TICKS {
        invalid_gameplay(
            errors,
            format!(
                "ability {:?} cooldown exceeds {MAX_ABILITY_COOLDOWN_TICKS} ticks",
                ability.id
            ),
        );
    }
    if ability.damage.is_none() && ability.heal.is_none() && ability.status_effects.is_empty() {
        invalid_gameplay(errors, format!("ability {:?} has no effect", ability.id));
    }
    if ability.status_effects.len() > MAX_ABILITY_STATUS_EFFECTS {
        invalid_gameplay(
            errors,
            format!(
                "ability {:?} has {} status effects; the maximum is {MAX_ABILITY_STATUS_EFFECTS}",
                ability.id,
                ability.status_effects.len()
            ),
        );
    }
    if let Some(damage) = &ability.damage {
        validate_amount_and_scaling(
            damage.base_amount,
            damage.scaling_attribute.as_deref(),
            damage.scaling_percent,
            &format!("ability {:?} damage", ability.id),
            errors,
        );
        validate_effect_target(ability, damage.target, "damage", errors);
        if damage.target == AbilityEffectTarget::Target
            && ability.targeting == AbilityTargeting::SingleAlly
        {
            invalid_gameplay(
                errors,
                format!("ability {:?} cannot damage its selected ally", ability.id),
            );
        }
    }
    if let Some(heal) = &ability.heal {
        validate_amount_and_scaling(
            heal.base_amount,
            heal.scaling_attribute.as_deref(),
            heal.scaling_percent,
            &format!("ability {:?} heal", ability.id),
            errors,
        );
        validate_effect_target(ability, heal.target, "heal", errors);
        if heal.target == AbilityEffectTarget::Target
            && ability.targeting == AbilityTargeting::SingleEnemy
        {
            invalid_gameplay(
                errors,
                format!("ability {:?} cannot heal its selected enemy", ability.id),
            );
        }
    }
    for status in &ability.status_effects {
        let (target, duration_ticks, magnitude) = match status {
            AbilityStatusEffect::AttributeModifier {
                target,
                attribute,
                magnitude,
                duration_ticks,
            } => {
                if !valid_stat_identifier(attribute) || attribute == "max_health" || *magnitude == 0
                {
                    invalid_gameplay(
                        errors,
                        format!("ability {:?} has an invalid attribute modifier", ability.id),
                    );
                }
                (*target, *duration_ticks, magnitude.unsigned_abs())
            }
            AbilityStatusEffect::Regeneration {
                target,
                magnitude,
                duration_ticks,
            }
            | AbilityStatusEffect::DamageOverTime {
                target,
                magnitude,
                duration_ticks,
                ..
            } => (*target, *duration_ticks, *magnitude),
            AbilityStatusEffect::Stun {
                target,
                duration_ticks,
            } => (*target, *duration_ticks, 1),
        };
        validate_effect_target(ability, target, "status effect", errors);
        if duration_ticks == 0 || duration_ticks > MAX_ABILITY_EFFECT_DURATION_TICKS {
            invalid_gameplay(
                errors,
                format!(
                    "ability {:?} status duration must be 1..={MAX_ABILITY_EFFECT_DURATION_TICKS}",
                    ability.id
                ),
            );
        }
        if magnitude == 0 || magnitude > MAX_ABILITY_EFFECT_MAGNITUDE as u32 {
            invalid_gameplay(
                errors,
                format!(
                    "ability {:?} status magnitude must be 1..={MAX_ABILITY_EFFECT_MAGNITUDE}",
                    ability.id
                ),
            );
        }
    }
}

fn validate_effect_target(
    ability: &AbilityDefinition,
    target: AbilityEffectTarget,
    effect: &str,
    errors: &mut Vec<ValidationError>,
) {
    if ability.targeting == AbilityTargeting::SelfOnly && target == AbilityEffectTarget::Target {
        invalid_gameplay(
            errors,
            format!(
                "self-only ability {:?} has target-directed {effect}",
                ability.id
            ),
        );
    }
}

fn validate_amount_and_scaling(
    amount: u32,
    scaling_attribute: Option<&str>,
    scaling_percent: u16,
    context: &str,
    errors: &mut Vec<ValidationError>,
) {
    if amount == 0 || amount > MAX_GAMEPLAY_VALUE {
        invalid_gameplay(
            errors,
            format!("{context} base amount must be 1..={MAX_GAMEPLAY_VALUE}"),
        );
    }
    if scaling_percent > MAX_ABILITY_SCALING_PERCENT
        || scaling_attribute.is_some() != (scaling_percent > 0)
        || scaling_attribute.is_some_and(|attribute| !valid_stat_identifier(attribute))
    {
        invalid_gameplay(
            errors,
            format!(
                "{context} scaling requires a safe attribute and 1..={MAX_ABILITY_SCALING_PERCENT} percent"
            ),
        );
    }
}

fn validate_actor_abilities(
    ability_ids: &[String],
    resource_pools: &BTreeMap<String, u32>,
    ability_by_id: &HashMap<&str, &AbilityDefinition>,
    context: &str,
    errors: &mut Vec<ValidationError>,
) {
    if ability_ids.len() > MAX_ABILITIES_PER_ACTOR {
        invalid_gameplay(
            errors,
            format!("{context} exceeds {MAX_ABILITIES_PER_ACTOR} abilities"),
        );
    }
    let mut ids = HashSet::new();
    for ability_id in ability_ids {
        if !ids.insert(ability_id.as_str()) {
            invalid_gameplay(errors, format!("{context} repeats ability {ability_id:?}"));
            continue;
        }
        validate_ability_reference(ability_id, resource_pools, ability_by_id, context, errors);
    }
}

fn validate_ability_reference(
    ability_id: &str,
    resource_pools: &BTreeMap<String, u32>,
    ability_by_id: &HashMap<&str, &AbilityDefinition>,
    context: &str,
    errors: &mut Vec<ValidationError>,
) {
    let Some(ability) = ability_by_id.get(ability_id) else {
        invalid_gameplay(
            errors,
            format!("{context} references missing ability {ability_id:?}"),
        );
        return;
    };
    for resource_id in ability.resource_costs.keys() {
        if !resource_pools.contains_key(resource_id) {
            invalid_gameplay(
                errors,
                format!(
                    "{context} cannot use ability {ability_id:?}: resource {resource_id:?} is unavailable"
                ),
            );
        }
    }
}

fn validate_u32_map(
    values: &BTreeMap<String, u32>,
    maximum_entries: usize,
    allow_zero: bool,
    context: &str,
    errors: &mut Vec<ValidationError>,
) {
    if values.len() > maximum_entries {
        invalid_gameplay(
            errors,
            format!("{context} has too many entries; the maximum is {maximum_entries}"),
        );
    }
    for (id, value) in values {
        if !valid_stat_identifier(id) || *value > MAX_GAMEPLAY_VALUE || (!allow_zero && *value == 0)
        {
            invalid_gameplay(
                errors,
                format!("{context} entry {id:?} must use a safe id and a value within bounds"),
            );
        }
    }
}

fn validate_u16_map(
    values: &BTreeMap<String, u16>,
    maximum_entries: usize,
    allow_zero: bool,
    context: &str,
    errors: &mut Vec<ValidationError>,
) {
    if values.len() > maximum_entries {
        invalid_gameplay(
            errors,
            format!("{context} has too many entries; the maximum is {maximum_entries}"),
        );
    }
    for (id, value) in values {
        if !valid_stat_identifier(id) || *value > MAX_SKILL_VALUE || (!allow_zero && *value == 0) {
            invalid_gameplay(
                errors,
                format!("{context} entry {id:?} must use a safe id and a value within bounds"),
            );
        }
    }
}

fn validate_resistances(
    resistances: &BTreeMap<DamageType, u16>,
    context: &str,
    errors: &mut Vec<ValidationError>,
) {
    for (damage_type, percent) in resistances {
        if *percent > MAX_RESISTANCE_PERCENT {
            invalid_gameplay(
                errors,
                format!(
                    "{context} {damage_type:?} value {percent} exceeds {MAX_RESISTANCE_PERCENT} percent"
                ),
            );
        }
    }
}

fn valid_gameplay_identifier(value: &str) -> bool {
    let bytes = value.as_bytes();
    !bytes.is_empty()
        && bytes.len() <= 128
        && bytes[0].is_ascii_alphanumeric()
        && bytes[bytes.len() - 1].is_ascii_alphanumeric()
        && bytes.iter().all(|byte| {
            byte.is_ascii_lowercase()
                || byte.is_ascii_digit()
                || matches!(byte, b':' | b'-' | b'_' | b'.')
        })
}

fn valid_stat_identifier(value: &str) -> bool {
    value.len() <= 64 && valid_gameplay_identifier(value)
}

fn safe_gameplay_text(value: &str, maximum_bytes: usize) -> bool {
    let trimmed = value.trim();
    !trimmed.is_empty()
        && trimmed.len() <= maximum_bytes
        && !trimmed
            .chars()
            .any(|character| character.is_control() && !matches!(character, '\n' | '\r' | '\t'))
}

fn invalid_gameplay(errors: &mut Vec<ValidationError>, message: String) {
    errors.push(ValidationError::InvalidGameplay(message));
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "code", rename_all = "snake_case")]
pub enum WorldPackDiagnostic {
    DirectedUnreachableRoom {
        room_id: String,
        start_room_id: String,
    },
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
    #[error("room {room_id:?} has invalid legacy alphabetic bitvector {value:?}")]
    InvalidLegacyRoomBitvector { room_id: String, value: String },
    #[error("room {room_id:?} defines direction {direction:?} more than once")]
    DuplicateExitDirection {
        room_id: String,
        direction: Direction,
    },
    #[error("room {room_id:?} points to missing room {target:?}")]
    MissingRoomTarget { room_id: String, target: String },
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
    #[error("item {item_id:?} is both placed in room {room_id:?} and allocated as NPC loot")]
    ItemPlacedAndAllocatedAsLoot { item_id: String, room_id: String },
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
    #[error("WorldPack declares more than the bounded number of entity media definitions")]
    TooManyEntityMedia,
    #[error("entity media subject {kind}:{subject_id} appears more than once")]
    DuplicateEntityMedia { kind: String, subject_id: String },
    #[error("entity media subject {kind}:{subject_id} is invalid: {reason}")]
    InvalidEntityMedia {
        kind: String,
        subject_id: String,
        reason: String,
    },
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
    #[error(
        "room {room_id:?} declares {actual} script modules for hook {hook:?}; the maximum is {maximum}"
    )]
    TooManyScriptsForRoomHook {
        room_id: String,
        hook: ScriptHook,
        actual: usize,
        maximum: usize,
    },
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
    #[error("script {script_id:?} is not a valid sandboxed Lua module: {reason}")]
    InvalidScriptModule { script_id: String, reason: String },
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
    fn item_gameplay_values_cannot_exceed_authored_caps() {
        let max_gameplay_value =
            i32::try_from(MAX_GAMEPLAY_VALUE).expect("MAX_GAMEPLAY_VALUE fits in i32");
        let base_rule = ItemRule {
            item_id: "test:blade".into(),
            equipment_slot: Some(EquipmentSlot::Weapon),
            attack_bonus: 1,
            heal_amount: 0,
            consumable: false,
            effect: None,
        };
        let invalid_rules = [
            ItemRule {
                attack_bonus: max_gameplay_value + 1,
                ..base_rule.clone()
            },
            ItemRule {
                heal_amount: max_gameplay_value + 1,
                ..base_rule.clone()
            },
            ItemRule {
                effect: Some(EffectRule {
                    kind: EffectKind::Strength,
                    magnitude: MAX_ABILITY_EFFECT_MAGNITUDE + 1,
                    remaining_ticks: 1,
                }),
                ..base_rule.clone()
            },
            ItemRule {
                effect: Some(EffectRule {
                    kind: EffectKind::Strength,
                    magnitude: 1,
                    remaining_ticks: MAX_ABILITY_EFFECT_DURATION_TICKS + 1,
                }),
                ..base_rule
            },
        ];

        for rule in invalid_rules {
            let mut candidate = pack();
            candidate.items.push(Item {
                id: "test:blade".into(),
                name: "test blade".into(),
                description: "A blade used to verify authoring bounds.".into(),
                aliases: Vec::new(),
            });
            candidate.rooms[0].items.push("test:blade".into());
            candidate.gameplay.item_rules.push(rule);

            assert!(candidate.validate().unwrap_err().0.iter().any(|error| {
                matches!(error, ValidationError::InvalidGameplay(message)
                    if message.contains("invalid bonuses or effect bounds"))
            }));
        }
    }

    #[test]
    fn npc_gameplay_values_cannot_exceed_authored_caps() {
        let max_gameplay_value =
            i32::try_from(MAX_GAMEPLAY_VALUE).expect("MAX_GAMEPLAY_VALUE fits in i32");
        let base_prototype = NpcPrototype {
            id: "test:sentinel".into(),
            name: "sentinel".into(),
            description: "A sentinel used to verify authoring bounds.".into(),
            level: 1,
            max_health: 10,
            strength: 2,
            defence: 1,
            experience_reward: 1,
            respawn_ticks: 1,
            loot_item_ids: Vec::new(),
        };
        let mut invalid_prototypes = Vec::new();
        let mut invalid = base_prototype.clone();
        invalid.level = MAX_PROGRESSION_LEVEL + 1;
        invalid_prototypes.push(invalid);
        let mut invalid = base_prototype.clone();
        invalid.max_health = max_gameplay_value + 1;
        invalid_prototypes.push(invalid);
        let mut invalid = base_prototype.clone();
        invalid.strength = max_gameplay_value + 1;
        invalid_prototypes.push(invalid);
        let mut invalid = base_prototype;
        invalid.defence = max_gameplay_value + 1;
        invalid_prototypes.push(invalid);

        for prototype in invalid_prototypes {
            let mut candidate = pack();
            candidate.gameplay.npc_spawns.push(NpcSpawn {
                id: "test:sentinel-1".into(),
                prototype_id: prototype.id.clone(),
                room_id: "test:1".into(),
            });
            candidate.gameplay.npc_prototypes.push(prototype);

            assert!(candidate.validate().unwrap_err().0.iter().any(|error| {
                matches!(error, ValidationError::InvalidGameplay(message)
                    if message.contains("invalid required fields"))
            }));
        }
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
    fn legacy_room_bitvectors_preserve_rom_letters_without_guessing_a_mask() {
        let numeric: LegacyBitvector = serde_json::from_str("321").unwrap();
        let alphabetic: LegacyBitvector = serde_json::from_str(r#""ABcz""#).unwrap();
        assert_eq!(numeric, LegacyBitvector::Numeric(321));
        assert_eq!(alphabetic, LegacyBitvector::Alphabetic("ABcz".into()));

        let mut value = pack();
        value.rooms[0].legacy = Some(LegacyRoomData {
            flags: alphabetic,
            sector: 0,
            prefix_values: Vec::new(),
        });
        value.validate().unwrap();

        value.rooms[0].legacy.as_mut().unwrap().flags = LegacyBitvector::Alphabetic("A|B".into());
        assert!(value.validate().unwrap_err().0.contains(
            &ValidationError::InvalidLegacyRoomBitvector {
                room_id: "test:1".into(),
                value: "A|B".into(),
            }
        ));
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
    fn directed_unreachable_rooms_are_diagnostics_not_validation_errors() {
        let mut disconnected = pack();
        disconnected.rooms.push(room("test:isolated", 2));
        disconnected.validate().unwrap();
        assert_eq!(
            disconnected.diagnostics(),
            vec![WorldPackDiagnostic::DirectedUnreachableRoom {
                room_id: "test:isolated".into(),
                start_room_id: "test:1".into(),
            }]
        );
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
            supported: CURRENT_SCHEMA_VERSION,
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

    #[test]
    fn an_item_must_start_in_exactly_one_room_or_one_loot_allocation() {
        let item = Item {
            id: "test:fang".into(),
            name: "fang".into(),
            description: "A sharp fang.".into(),
            aliases: Vec::new(),
        };
        let prototype = NpcPrototype {
            id: "test:wolf".into(),
            name: "wolf".into(),
            description: "A watchful wolf.".into(),
            level: 1,
            max_health: 10,
            strength: 2,
            defence: 0,
            experience_reward: 10,
            respawn_ticks: 2,
            loot_item_ids: vec![item.id.clone()],
        };
        let spawn = NpcSpawn {
            id: "test:wolf-1".into(),
            prototype_id: prototype.id.clone(),
            room_id: "test:1".into(),
        };

        let mut loot_only = pack();
        loot_only.items.push(item.clone());
        loot_only.gameplay.npc_prototypes.push(prototype.clone());
        loot_only.gameplay.npc_spawns.push(spawn.clone());
        loot_only.validate().unwrap();

        let mut ambiguous = loot_only;
        ambiguous.rooms[0].items.push(item.id.clone());
        assert!(ambiguous.validate().unwrap_err().0.contains(
            &ValidationError::ItemPlacedAndAllocatedAsLoot {
                item_id: item.id.clone(),
                room_id: "test:1".into(),
            }
        ));

        let mut nowhere = pack();
        nowhere.items.push(item);
        nowhere.gameplay.npc_prototypes.push(prototype);
        nowhere.gameplay.npc_spawns.push(NpcSpawn {
            prototype_id: "missing:prototype".into(),
            ..spawn
        });
        assert!(
            nowhere
                .validate()
                .unwrap_err()
                .0
                .contains(&ValidationError::UnplacedItem("test:fang".into()))
        );
    }
}
