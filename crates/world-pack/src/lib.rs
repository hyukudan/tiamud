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

pub const CURRENT_SCHEMA_VERSION: u32 = 5;
pub const MAX_ITEM_PROTOTYPES: usize = 1_024;
pub const MAX_INITIAL_ITEM_INSTANCES: usize = 8_192;
pub const MAX_ITEM_ALIASES: usize = 32;
pub const MAX_ITEM_NAME_BYTES: usize = 128;
pub const MAX_ITEM_DESCRIPTION_BYTES: usize = 2_048;
pub const MAX_ITEM_ALIAS_BYTES: usize = 64;
pub const MAX_ITEM_WEIGHT_GRAMS: u32 = 1_000_000;
pub const MAX_ITEM_SLOT_COST: u16 = 100;
pub const MAX_ITEM_LEVEL: u16 = 1_000;
pub const MAX_ITEM_STACK_UNITS: u16 = 1_000;
pub const MAX_ITEM_CONTAINER_SLOTS: u16 = 1_000;
pub const MAX_ITEM_CONTAINER_WEIGHT_GRAMS: u32 = 1_000_000;
pub const MAX_ITEM_CONTAINER_DEPTH: usize = 4;
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
    pub item_prototypes: Vec<ItemPrototype>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub initial_item_instances: Vec<InitialItemInstance>,
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
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub legacy: Option<LegacyRoomData>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ItemPrototype {
    pub id: String,
    pub name: String,
    pub description: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub aliases: Vec<String>,
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
    pub weight_grams: u32,
    pub slot_cost: u16,
    pub rarity: ItemRarity,
    pub item_level: u16,
    pub stack_policy: ItemStackPolicy,
    pub max_stack_units: u16,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub container: Option<ItemContainerDefinition>,
    pub transfer_policy: ItemTransferPolicy,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ItemContainerDefinition {
    pub max_slots: u16,
    pub max_weight_grams: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct InitialItemInstance {
    pub id: String,
    pub prototype_id: String,
    pub location: InitialItemLocation,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(
    tag = "kind",
    rename_all = "snake_case",
    rename_all_fields = "camelCase"
)]
pub enum InitialItemLocation {
    Room { room_id: String },
    NpcSpawn { npc_spawn_id: String },
    Container { container_instance_id: String },
    Staged,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ItemRarity {
    Common,
    Uncommon,
    Rare,
    Epic,
    Legendary,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ItemStackPolicy {
    Unique,
    Fungible,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ItemTransferPolicy {
    Transferable,
    CharacterBound,
    NonTransferable,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GameplayDefinition {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rng_seed: Option<u64>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub npc_prototypes: Vec<NpcPrototype>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub npc_spawns: Vec<NpcSpawn>,
    /// Canonical character creation and advancement model introduced by WorldPack v3.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub character_progression: Option<CharacterProgressionDefinition>,
    /// Archetype selected when character creation does not name one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default_archetype_id: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub archetypes: Vec<ArchetypeDefinition>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub abilities: Vec<AbilityDefinition>,
    /// Ability loadouts remain separate from [`NpcPrototype`] and reusable.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub npc_ability_sets: Vec<NpcAbilitySet>,
}

impl GameplayDefinition {
    pub fn is_empty(&self) -> bool {
        self.rng_seed.is_none()
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
        }

        if !self.start_room_id.trim().is_empty() && !room_ids.contains(self.start_room_id.as_str())
        {
            errors.push(ValidationError::MissingStartRoom(
                self.start_room_id.clone(),
            ));
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

        if self.item_prototypes.len() > MAX_ITEM_PROTOTYPES {
            errors.push(ValidationError::TooManyItemPrototypes {
                actual: self.item_prototypes.len(),
                maximum: MAX_ITEM_PROTOTYPES,
            });
        }
        let max_gameplay_value =
            i32::try_from(MAX_GAMEPLAY_VALUE).expect("MAX_GAMEPLAY_VALUE fits in i32");
        let mut item_prototype_ids = HashSet::new();
        for prototype in &self.item_prototypes {
            if prototype.id.trim().is_empty() {
                errors.push(ValidationError::EmptyItemId);
            } else if !valid_gameplay_identifier(&prototype.id) {
                errors.push(ValidationError::InvalidGameplay(format!(
                    "item prototype id {:?} must contain 1..=128 lowercase namespaced identifier bytes",
                    prototype.id
                )));
            } else if !item_prototype_ids.insert(prototype.id.as_str()) {
                errors.push(ValidationError::DuplicateItemId(prototype.id.clone()));
            }
            if prototype.name.trim().is_empty() {
                errors.push(ValidationError::EmptyItemName(prototype.id.clone()));
            } else if !safe_gameplay_label(&prototype.name, MAX_ITEM_NAME_BYTES) {
                errors.push(ValidationError::InvalidGameplay(format!(
                    "item prototype {:?} name must contain 1..={MAX_ITEM_NAME_BYTES} safe bytes without control characters",
                    prototype.id
                )));
            }
            if prototype.description.trim().is_empty() {
                errors.push(ValidationError::EmptyItemDescription(prototype.id.clone()));
            } else if !safe_gameplay_text(&prototype.description, MAX_ITEM_DESCRIPTION_BYTES) {
                errors.push(ValidationError::InvalidGameplay(format!(
                    "item prototype {:?} description must contain 1..={MAX_ITEM_DESCRIPTION_BYTES} safe bytes",
                    prototype.id
                )));
            }
            if prototype.aliases.len() > MAX_ITEM_ALIASES {
                errors.push(ValidationError::InvalidGameplay(format!(
                    "item prototype {:?} has {} aliases; the maximum is {MAX_ITEM_ALIASES}",
                    prototype.id,
                    prototype.aliases.len()
                )));
            }
            let mut aliases = HashSet::new();
            if prototype.aliases.iter().any(|alias| {
                !safe_gameplay_label(alias, MAX_ITEM_ALIAS_BYTES)
                    || !aliases.insert(alias.to_ascii_lowercase())
            }) {
                errors.push(ValidationError::InvalidGameplay(format!(
                    "item prototype {:?} has an unsafe, oversized, empty, or duplicate alias; aliases are limited to {MAX_ITEM_ALIAS_BYTES} bytes without control characters",
                    prototype.id
                )));
            }
            if prototype.weight_grams > MAX_ITEM_WEIGHT_GRAMS
                || !(1..=MAX_ITEM_SLOT_COST).contains(&prototype.slot_cost)
                || !(1..=MAX_ITEM_LEVEL).contains(&prototype.item_level)
                || !(0..=max_gameplay_value).contains(&prototype.attack_bonus)
                || !(0..=max_gameplay_value).contains(&prototype.heal_amount)
                || prototype.effect.is_some_and(|effect| {
                    !(1..=MAX_ABILITY_EFFECT_MAGNITUDE).contains(&effect.magnitude)
                        || !(1..=MAX_ABILITY_EFFECT_DURATION_TICKS)
                            .contains(&effect.remaining_ticks)
                })
            {
                errors.push(ValidationError::InvalidGameplay(format!(
                    "item prototype {:?} exceeds an item property bound",
                    prototype.id
                )));
            }
            if !prototype.consumable && (prototype.heal_amount != 0 || prototype.effect.is_some()) {
                errors.push(ValidationError::InvalidGameplay(format!(
                    "non-consumable item prototype {:?} declares a consumable effect",
                    prototype.id
                )));
            }
            if prototype.consumable && prototype.equipment_slot.is_some() {
                errors.push(ValidationError::InvalidGameplay(format!(
                    "consumable item prototype {:?} cannot declare an equipment slot",
                    prototype.id
                )));
            }
            if prototype.transfer_policy != ItemTransferPolicy::Transferable {
                errors.push(ValidationError::InvalidGameplay(format!(
                    "item prototype {:?} uses a transfer policy reserved for phase 3C",
                    prototype.id
                )));
            }
            match prototype.stack_policy {
                ItemStackPolicy::Unique if prototype.max_stack_units != 1 => {
                    errors.push(ValidationError::InvalidGameplay(format!(
                        "unique item prototype {:?} must declare maxStackUnits 1",
                        prototype.id
                    )));
                }
                ItemStackPolicy::Fungible
                    if !(2..=MAX_ITEM_STACK_UNITS).contains(&prototype.max_stack_units) =>
                {
                    errors.push(ValidationError::InvalidGameplay(format!(
                        "fungible item prototype {:?} must declare maxStackUnits between 2 and {MAX_ITEM_STACK_UNITS}",
                        prototype.id
                    )));
                }
                _ => {}
            }
            if let Some(container) = prototype.container {
                if prototype.stack_policy != ItemStackPolicy::Unique
                    || prototype.max_stack_units != 1
                    || prototype.consumable
                {
                    errors.push(ValidationError::InvalidGameplay(format!(
                        "container item prototype {:?} must be unique, have maxStackUnits 1, and not be consumable",
                        prototype.id
                    )));
                }
                if !(1..=MAX_ITEM_CONTAINER_SLOTS).contains(&container.max_slots)
                    || container.max_weight_grams > MAX_ITEM_CONTAINER_WEIGHT_GRAMS
                {
                    errors.push(ValidationError::InvalidGameplay(format!(
                        "container item prototype {:?} exceeds a container capacity bound",
                        prototype.id
                    )));
                }
            }
        }

        if self.initial_item_instances.len() > MAX_INITIAL_ITEM_INSTANCES {
            errors.push(ValidationError::TooManyInitialItemInstances {
                actual: self.initial_item_instances.len(),
                maximum: MAX_INITIAL_ITEM_INSTANCES,
            });
        }
        let mut item_instance_ids = HashSet::new();
        for instance in &self.initial_item_instances {
            if instance.id.trim().is_empty() {
                errors.push(ValidationError::InvalidGameplay(
                    "initial item instance id must not be empty".to_owned(),
                ));
            } else if !item_instance_ids.insert(instance.id.as_str()) {
                errors.push(ValidationError::InvalidGameplay(format!(
                    "initial item instance id {:?} appears more than once",
                    instance.id
                )));
            }
            if !item_prototype_ids.contains(instance.prototype_id.as_str()) {
                errors.push(ValidationError::InvalidGameplay(format!(
                    "initial item instance {:?} references missing prototype {:?}",
                    instance.id, instance.prototype_id
                )));
            }
            if let InitialItemLocation::Room { room_id } = &instance.location
                && !room_ids.contains(room_id.as_str())
            {
                errors.push(ValidationError::InvalidGameplay(format!(
                    "initial item instance {:?} references missing room {:?}",
                    instance.id, room_id
                )));
            }
        }
        validate_initial_item_containers(
            &self.item_prototypes,
            &self.initial_item_instances,
            &mut errors,
        );

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
                Some(_) => {
                    used_prototypes.insert(spawn.prototype_id.as_str());
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

        for instance in &self.initial_item_instances {
            match &instance.location {
                InitialItemLocation::NpcSpawn { npc_spawn_id }
                    if !spawn_ids.contains(npc_spawn_id.as_str()) =>
                {
                    errors.push(ValidationError::InvalidGameplay(format!(
                        "initial item instance {:?} references missing NPC spawn {:?}",
                        instance.id, npc_spawn_id
                    )));
                }
                InitialItemLocation::Room { .. }
                | InitialItemLocation::NpcSpawn { .. }
                | InitialItemLocation::Container { .. }
                | InitialItemLocation::Staged => {}
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

fn validate_initial_item_containers(
    prototypes: &[ItemPrototype],
    instances: &[InitialItemInstance],
    errors: &mut Vec<ValidationError>,
) {
    let prototypes_by_id = prototypes
        .iter()
        .map(|prototype| (prototype.id.as_str(), prototype))
        .collect::<HashMap<_, _>>();
    let instances_by_id = instances
        .iter()
        .map(|instance| (instance.id.as_str(), instance))
        .collect::<HashMap<_, _>>();
    let mut children_by_container: HashMap<&str, Vec<&InitialItemInstance>> = HashMap::new();

    for instance in instances {
        let InitialItemLocation::Container {
            container_instance_id,
        } = &instance.location
        else {
            continue;
        };
        if container_instance_id == &instance.id {
            errors.push(ValidationError::InvalidGameplay(format!(
                "initial item instance {:?} cannot contain itself",
                instance.id
            )));
            continue;
        }
        let Some(container_instance) = instances_by_id.get(container_instance_id.as_str()) else {
            errors.push(ValidationError::InvalidGameplay(format!(
                "initial item instance {:?} references missing container instance {:?}",
                instance.id, container_instance_id
            )));
            continue;
        };
        let is_container = prototypes_by_id
            .get(container_instance.prototype_id.as_str())
            .is_some_and(|prototype| prototype.container.is_some());
        if !is_container {
            errors.push(ValidationError::InvalidGameplay(format!(
                "initial item instance {:?} references non-container instance {:?}",
                instance.id, container_instance_id
            )));
            continue;
        }
        children_by_container
            .entry(container_instance_id)
            .or_default()
            .push(instance);
    }

    for instance in instances {
        let mut seen = HashSet::from([instance.id.as_str()]);
        let mut current = instance;
        let mut depth = 0usize;
        while let InitialItemLocation::Container {
            container_instance_id,
        } = &current.location
        {
            depth += 1;
            if depth > MAX_ITEM_CONTAINER_DEPTH {
                errors.push(ValidationError::InvalidGameplay(format!(
                    "initial item instance {:?} exceeds maximum container depth {MAX_ITEM_CONTAINER_DEPTH}",
                    instance.id
                )));
                break;
            }
            if !seen.insert(container_instance_id.as_str()) {
                errors.push(ValidationError::InvalidGameplay(format!(
                    "initial item instance {:?} participates in a container cycle",
                    instance.id
                )));
                break;
            }
            let Some(next) = instances_by_id.get(container_instance_id.as_str()) else {
                break;
            };
            current = next;
        }
    }

    for container_instance in instances {
        let Some(prototype) = prototypes_by_id.get(container_instance.prototype_id.as_str()) else {
            continue;
        };
        let Some(container) = prototype.container else {
            continue;
        };
        let children = children_by_container
            .get(container_instance.id.as_str())
            .map(Vec::as_slice)
            .unwrap_or_default();
        let mut units_by_prototype: HashMap<&str, u32> = HashMap::new();
        for child in children {
            *units_by_prototype
                .entry(child.prototype_id.as_str())
                .or_default() += 1;
        }
        let slots = units_by_prototype
            .into_iter()
            .try_fold(0u32, |used, (prototype_id, units)| {
                let child_prototype = prototypes_by_id.get(prototype_id)?;
                let max_stack_units = u32::from(child_prototype.max_stack_units);
                if max_stack_units == 0 {
                    return None;
                }
                let stacks = units.div_ceil(max_stack_units);
                used.checked_add(stacks.checked_mul(u32::from(child_prototype.slot_cost))?)
            });
        if slots.is_none_or(|slots| slots > u32::from(container.max_slots)) {
            errors.push(ValidationError::InvalidGameplay(format!(
                "container instance {:?} exceeds its authored slot capacity",
                container_instance.id
            )));
        }

        let weight = children.iter().try_fold(0u64, |used, child| {
            used.checked_add(authored_item_total_weight(
                &child.id,
                &prototypes_by_id,
                &instances_by_id,
                &children_by_container,
            )?)
        });
        if weight.is_none_or(|weight| weight > u64::from(container.max_weight_grams)) {
            errors.push(ValidationError::InvalidGameplay(format!(
                "container instance {:?} exceeds its authored weight capacity",
                container_instance.id
            )));
        }
    }
}

fn authored_item_total_weight<'a>(
    instance_id: &'a str,
    prototypes_by_id: &HashMap<&'a str, &'a ItemPrototype>,
    instances_by_id: &HashMap<&'a str, &'a InitialItemInstance>,
    children_by_container: &HashMap<&'a str, Vec<&'a InitialItemInstance>>,
) -> Option<u64> {
    let mut pending = vec![(instance_id, 0usize)];
    let mut visited = HashSet::new();
    let mut weight = 0u64;
    while let Some((current_id, depth)) = pending.pop() {
        if depth > MAX_ITEM_CONTAINER_DEPTH || !visited.insert(current_id) {
            return None;
        }
        let instance = instances_by_id.get(current_id)?;
        let prototype = prototypes_by_id.get(instance.prototype_id.as_str())?;
        weight = weight.checked_add(u64::from(prototype.weight_grams))?;
        if let Some(children) = children_by_container.get(current_id) {
            pending.extend(
                children
                    .iter()
                    .map(|child| (child.id.as_str(), depth.saturating_add(1))),
            );
        }
    }
    Some(weight)
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

fn safe_gameplay_label(value: &str, maximum_bytes: usize) -> bool {
    let trimmed = value.trim();
    !trimmed.is_empty() && trimmed.len() <= maximum_bytes && !trimmed.chars().any(char::is_control)
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
    #[error("WorldPack declares {actual} item prototypes; the maximum is {maximum}")]
    TooManyItemPrototypes { actual: usize, maximum: usize },
    #[error("WorldPack declares {actual} initial item instances; the maximum is {maximum}")]
    TooManyInitialItemInstances { actual: usize, maximum: usize },
    #[error("item {0:?} must have a name")]
    EmptyItemName(String),
    #[error("item {0:?} must have a description")]
    EmptyItemDescription(String),
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
            legacy: None,
        }
    }

    fn item_prototype(id: &str) -> ItemPrototype {
        ItemPrototype {
            id: id.to_owned(),
            name: "test item".into(),
            description: "An item used by tests.".into(),
            aliases: Vec::new(),
            equipment_slot: None,
            attack_bonus: 0,
            heal_amount: 0,
            consumable: false,
            effect: None,
            weight_grams: 100,
            slot_cost: 1,
            rarity: ItemRarity::Common,
            item_level: 1,
            stack_policy: ItemStackPolicy::Unique,
            max_stack_units: 1,
            container: None,
            transfer_policy: ItemTransferPolicy::Transferable,
        }
    }

    fn pack() -> WorldPack {
        WorldPack {
            schema_version: CURRENT_SCHEMA_VERSION,
            id: "test".to_owned(),
            name: "Test".to_owned(),
            start_room_id: "test:1".to_owned(),
            source: None,
            item_prototypes: Vec::new(),
            initial_item_instances: Vec::new(),
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
        let mut invalid_item = item_prototype("test:invalid");
        invalid_item.attack_bonus = -1;
        pack.item_prototypes.push(invalid_item);
        pack.initial_item_instances.push(InitialItemInstance {
            id: "test:missing-instance".into(),
            prototype_id: "missing:item".into(),
            location: InitialItemLocation::Room {
                room_id: "missing:room".into(),
            },
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
                >= 4
        );
    }

    #[test]
    fn item_gameplay_values_cannot_exceed_authored_caps() {
        let max_gameplay_value =
            i32::try_from(MAX_GAMEPLAY_VALUE).expect("MAX_GAMEPLAY_VALUE fits in i32");
        let mut base = item_prototype("test:blade");
        base.equipment_slot = Some(EquipmentSlot::Weapon);
        base.attack_bonus = 1;
        let mut invalid_prototypes = Vec::new();
        let mut invalid = base.clone();
        invalid.attack_bonus = max_gameplay_value + 1;
        invalid_prototypes.push(invalid);
        let mut invalid = base.clone();
        invalid.heal_amount = max_gameplay_value + 1;
        invalid.consumable = true;
        invalid.equipment_slot = None;
        invalid_prototypes.push(invalid);
        let mut invalid = base.clone();
        invalid.effect = Some(EffectRule {
            kind: EffectKind::Strength,
            magnitude: MAX_ABILITY_EFFECT_MAGNITUDE + 1,
            remaining_ticks: 1,
        });
        invalid.consumable = true;
        invalid.equipment_slot = None;
        invalid_prototypes.push(invalid);
        let mut invalid = base;
        invalid.effect = Some(EffectRule {
            kind: EffectKind::Strength,
            magnitude: 1,
            remaining_ticks: MAX_ABILITY_EFFECT_DURATION_TICKS + 1,
        });
        invalid.consumable = true;
        invalid.equipment_slot = None;
        invalid_prototypes.push(invalid);

        for prototype in invalid_prototypes {
            let mut candidate = pack();
            candidate.item_prototypes.push(prototype);
            candidate.initial_item_instances.push(InitialItemInstance {
                id: "test:blade-1".into(),
                prototype_id: "test:blade".into(),
                location: InitialItemLocation::Room {
                    room_id: "test:1".into(),
                },
            });

            assert!(candidate.validate().unwrap_err().0.iter().any(|error| {
                matches!(error, ValidationError::InvalidGameplay(message)
                    if message.contains("item property bound"))
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
        let item = item_prototype("test:lantern");
        let mut valid = pack();
        valid.item_prototypes.push(item.clone());
        valid.initial_item_instances.push(InitialItemInstance {
            id: "test:lantern-1".into(),
            prototype_id: item.id.clone(),
            location: InitialItemLocation::Room {
                room_id: "test:1".into(),
            },
        });
        valid.validate().unwrap();

        let mut duplicate_id = valid.clone();
        duplicate_id.item_prototypes.push(item.clone());
        assert!(
            duplicate_id
                .validate()
                .unwrap_err()
                .0
                .contains(&ValidationError::DuplicateItemId(item.id.clone()))
        );

        let mut missing = pack();
        missing.initial_item_instances.push(InitialItemInstance {
            id: "test:missing-1".into(),
            prototype_id: "test:missing".into(),
            location: InitialItemLocation::Room {
                room_id: "missing:room".into(),
            },
        });
        let messages = missing
            .validate()
            .unwrap_err()
            .0
            .into_iter()
            .map(|error| error.to_string())
            .collect::<Vec<_>>()
            .join("\n");
        assert!(messages.contains("missing prototype"));
        assert!(messages.contains("missing room"));

        let mut duplicate_instance = valid;
        duplicate_instance
            .initial_item_instances
            .push(duplicate_instance.initial_item_instances[0].clone());
        assert!(
            duplicate_instance
                .validate()
                .unwrap_err()
                .to_string()
                .contains("validation failed")
        );
    }

    #[test]
    fn an_item_has_one_typed_initial_location() {
        let mut value = pack();
        value.item_prototypes.push(item_prototype("test:lantern"));
        value.initial_item_instances.push(InitialItemInstance {
            id: "test:lantern-1".into(),
            prototype_id: "test:lantern".into(),
            location: InitialItemLocation::Staged,
        });
        value.validate().unwrap();

        let serialized = serde_json::to_value(&value.initial_item_instances[0]).unwrap();
        assert_eq!(serialized["location"]["kind"], "staged");
        assert!(serialized["location"].get("roomId").is_none());
        assert!(serialized["location"].get("npcSpawnId").is_none());
    }

    #[test]
    fn an_item_can_start_on_an_exact_npc_spawn() {
        let npc_prototype = NpcPrototype {
            id: "test:wolf".into(),
            name: "wolf".into(),
            description: "A watchful wolf.".into(),
            level: 1,
            max_health: 10,
            strength: 2,
            defence: 0,
            experience_reward: 10,
            respawn_ticks: 2,
        };
        let spawn = NpcSpawn {
            id: "test:wolf-1".into(),
            prototype_id: npc_prototype.id.clone(),
            room_id: "test:1".into(),
        };

        let mut loot_only = pack();
        loot_only.item_prototypes.push(item_prototype("test:fang"));
        loot_only.initial_item_instances.push(InitialItemInstance {
            id: "test:fang-1".into(),
            prototype_id: "test:fang".into(),
            location: InitialItemLocation::NpcSpawn {
                npc_spawn_id: spawn.id.clone(),
            },
        });
        loot_only.gameplay.npc_prototypes.push(npc_prototype);
        loot_only.gameplay.npc_spawns.push(spawn.clone());
        loot_only.validate().unwrap();

        let mut missing_spawn = loot_only;
        missing_spawn.initial_item_instances[0].location = InitialItemLocation::NpcSpawn {
            npc_spawn_id: "missing:spawn".into(),
        };
        assert!(
            missing_spawn
                .validate()
                .unwrap_err()
                .0
                .iter()
                .any(|error| error.to_string().contains("missing NPC spawn"))
        );
    }

    #[test]
    fn item_capacity_and_property_bounds_are_explicit() {
        let mut zero_weight = pack();
        let mut prototype = item_prototype("test:weightless");
        prototype.weight_grams = 0;
        zero_weight.item_prototypes.push(prototype);
        zero_weight.validate().unwrap();

        for mutate in [
            |prototype: &mut ItemPrototype| prototype.weight_grams = MAX_ITEM_WEIGHT_GRAMS + 1,
            |prototype: &mut ItemPrototype| prototype.slot_cost = 0,
            |prototype: &mut ItemPrototype| prototype.slot_cost = MAX_ITEM_SLOT_COST + 1,
            |prototype: &mut ItemPrototype| prototype.item_level = 0,
            |prototype: &mut ItemPrototype| prototype.item_level = MAX_ITEM_LEVEL + 1,
        ] {
            let mut candidate = pack();
            let mut prototype = item_prototype("test:bounded");
            mutate(&mut prototype);
            candidate.item_prototypes.push(prototype);
            assert!(candidate.validate().is_err());
        }

        let mut too_many_aliases = pack();
        let mut prototype = item_prototype("test:aliases");
        prototype.aliases = (0..=MAX_ITEM_ALIASES)
            .map(|index| format!("alias-{index}"))
            .collect();
        too_many_aliases.item_prototypes.push(prototype);
        assert!(too_many_aliases.validate().is_err());

        let mut reserved_transfer = pack();
        let mut prototype = item_prototype("test:bound");
        prototype.transfer_policy = ItemTransferPolicy::CharacterBound;
        reserved_transfer.item_prototypes.push(prototype);
        assert!(
            reserved_transfer
                .validate()
                .unwrap_err()
                .0
                .iter()
                .any(|error| error.to_string().contains("reserved for phase 3C"))
        );

        let mut too_many_prototypes = pack();
        too_many_prototypes.item_prototypes = (0..=MAX_ITEM_PROTOTYPES)
            .map(|index| item_prototype(&format!("test:prototype-{index}")))
            .collect();
        assert!(
            too_many_prototypes
                .validate()
                .unwrap_err()
                .0
                .iter()
                .any(|error| { matches!(error, ValidationError::TooManyItemPrototypes { .. }) })
        );

        let mut too_many_instances = pack();
        too_many_instances
            .item_prototypes
            .push(item_prototype("test:shared"));
        too_many_instances.initial_item_instances = (0..=MAX_INITIAL_ITEM_INSTANCES)
            .map(|index| InitialItemInstance {
                id: format!("test:instance-{index}"),
                prototype_id: "test:shared".into(),
                location: InitialItemLocation::Staged,
            })
            .collect();
        assert!(
            too_many_instances
                .validate()
                .unwrap_err()
                .0
                .iter()
                .any(|error| {
                    matches!(error, ValidationError::TooManyInitialItemInstances { .. })
                })
        );
    }

    #[test]
    fn item_display_text_is_byte_bounded_and_control_safe() {
        let mut boundary = pack();
        let mut prototype = item_prototype("test:text-boundary");
        prototype.name = "n".repeat(MAX_ITEM_NAME_BYTES);
        prototype.description = format!("{}\n{}", "d".repeat(MAX_ITEM_DESCRIPTION_BYTES - 2), "d");
        prototype.aliases = vec!["a".repeat(MAX_ITEM_ALIAS_BYTES)];
        boundary.item_prototypes.push(prototype);
        boundary.validate().unwrap();

        for (field, mutate) in [
            (
                "name",
                (|prototype: &mut ItemPrototype| {
                    prototype.name = "n".repeat(MAX_ITEM_NAME_BYTES + 1);
                }) as fn(&mut ItemPrototype),
            ),
            ("name", |prototype: &mut ItemPrototype| {
                prototype.name = "unsafe\u{1b}name".into()
            }),
            ("description", |prototype: &mut ItemPrototype| {
                prototype.description = "d".repeat(MAX_ITEM_DESCRIPTION_BYTES + 1);
            }),
            ("description", |prototype: &mut ItemPrototype| {
                prototype.description = "unsafe\u{1b}description".into();
            }),
            ("alias", |prototype: &mut ItemPrototype| {
                prototype.aliases = vec!["a".repeat(MAX_ITEM_ALIAS_BYTES + 1)];
            }),
            ("alias", |prototype: &mut ItemPrototype| {
                prototype.aliases = vec!["unsafe\u{1b}alias".into()];
            }),
        ] {
            let mut candidate = pack();
            let mut prototype = item_prototype("test:unsafe-text");
            mutate(&mut prototype);
            candidate.item_prototypes.push(prototype);
            let message = candidate
                .validate()
                .unwrap_err()
                .0
                .into_iter()
                .map(|error| error.to_string())
                .collect::<Vec<_>>()
                .join("\n");
            assert!(message.contains(field), "unexpected validation: {message}");
        }
    }

    #[test]
    fn item_prototype_ids_use_the_runtime_identifier_contract() {
        for invalid_id in [
            "Test:uppercase".to_owned(),
            "test:trailing-".to_owned(),
            format!("test:{}", "a".repeat(124)),
        ] {
            let mut candidate = pack();
            candidate.item_prototypes.push(item_prototype(&invalid_id));
            let errors = candidate.validate().unwrap_err().0;
            assert!(errors.iter().any(|error| {
                error
                    .to_string()
                    .contains("lowercase namespaced identifier")
            }));
        }
    }

    #[test]
    fn stack_and_container_prototype_bounds_are_closed() {
        let mut valid = pack();
        let mut fungible = item_prototype("test:tonic");
        fungible.stack_policy = ItemStackPolicy::Fungible;
        fungible.max_stack_units = 2;
        valid.item_prototypes.push(fungible);
        let mut container = item_prototype("test:satchel");
        container.container = Some(ItemContainerDefinition {
            max_slots: 1,
            max_weight_grams: 0,
        });
        valid.item_prototypes.push(container);
        valid.validate().unwrap();

        for mutate in [
            |prototype: &mut ItemPrototype| prototype.max_stack_units = 0,
            |prototype: &mut ItemPrototype| prototype.max_stack_units = 2,
        ] {
            let mut candidate = pack();
            let mut prototype = item_prototype("test:unique");
            mutate(&mut prototype);
            candidate.item_prototypes.push(prototype);
            assert!(candidate.validate().is_err());
        }

        for max_stack_units in [1, MAX_ITEM_STACK_UNITS + 1] {
            let mut candidate = pack();
            let mut prototype = item_prototype("test:fungible");
            prototype.stack_policy = ItemStackPolicy::Fungible;
            prototype.max_stack_units = max_stack_units;
            candidate.item_prototypes.push(prototype);
            assert!(candidate.validate().is_err());
        }

        let mut invalid_container = pack();
        let mut prototype = item_prototype("test:container");
        prototype.consumable = true;
        prototype.container = Some(ItemContainerDefinition {
            max_slots: 0,
            max_weight_grams: MAX_ITEM_CONTAINER_WEIGHT_GRAMS + 1,
        });
        invalid_container.item_prototypes.push(prototype);
        let messages = invalid_container
            .validate()
            .unwrap_err()
            .0
            .into_iter()
            .map(|error| error.to_string())
            .collect::<Vec<_>>()
            .join("\n");
        assert!(messages.contains("must be unique"));
        assert!(messages.contains("container capacity bound"));
    }

    #[test]
    fn authored_container_graph_rejects_bad_targets_cycles_and_depth_five() {
        let container_prototype = || {
            let mut prototype = item_prototype("test:container");
            prototype.container = Some(ItemContainerDefinition {
                max_slots: 100,
                max_weight_grams: MAX_ITEM_CONTAINER_WEIGHT_GRAMS,
            });
            prototype
        };

        let mut missing = pack();
        missing.item_prototypes.push(item_prototype("test:item"));
        missing.initial_item_instances.push(InitialItemInstance {
            id: "test:item-1".into(),
            prototype_id: "test:item".into(),
            location: InitialItemLocation::Container {
                container_instance_id: "test:missing".into(),
            },
        });
        assert!(
            missing
                .validate()
                .unwrap_err()
                .0
                .iter()
                .any(|error| error.to_string().contains("missing container instance"))
        );

        let mut non_container = pack();
        non_container
            .item_prototypes
            .push(item_prototype("test:item"));
        non_container.initial_item_instances.extend([
            InitialItemInstance {
                id: "test:item-1".into(),
                prototype_id: "test:item".into(),
                location: InitialItemLocation::Container {
                    container_instance_id: "test:item-2".into(),
                },
            },
            InitialItemInstance {
                id: "test:item-2".into(),
                prototype_id: "test:item".into(),
                location: InitialItemLocation::Staged,
            },
        ]);
        assert!(
            non_container
                .validate()
                .unwrap_err()
                .0
                .iter()
                .any(|error| error.to_string().contains("non-container"))
        );

        let mut self_contained = pack();
        self_contained.item_prototypes.push(container_prototype());
        self_contained
            .initial_item_instances
            .push(InitialItemInstance {
                id: "test:self".into(),
                prototype_id: "test:container".into(),
                location: InitialItemLocation::Container {
                    container_instance_id: "test:self".into(),
                },
            });
        assert!(
            self_contained
                .validate()
                .unwrap_err()
                .0
                .iter()
                .any(|error| error.to_string().contains("cannot contain itself"))
        );

        let mut cycle = pack();
        cycle.item_prototypes.push(container_prototype());
        cycle.initial_item_instances.extend([
            InitialItemInstance {
                id: "test:a".into(),
                prototype_id: "test:container".into(),
                location: InitialItemLocation::Container {
                    container_instance_id: "test:b".into(),
                },
            },
            InitialItemInstance {
                id: "test:b".into(),
                prototype_id: "test:container".into(),
                location: InitialItemLocation::Container {
                    container_instance_id: "test:a".into(),
                },
            },
        ]);
        assert!(
            cycle
                .validate()
                .unwrap_err()
                .0
                .iter()
                .any(|error| error.to_string().contains("container cycle"))
        );

        let mut depth = pack();
        depth.item_prototypes.push(container_prototype());
        depth.item_prototypes.push(item_prototype("test:leaf"));
        depth.initial_item_instances.push(InitialItemInstance {
            id: "test:leaf-1".into(),
            prototype_id: "test:leaf".into(),
            location: InitialItemLocation::Container {
                container_instance_id: "test:c1".into(),
            },
        });
        for index in 1..=4 {
            depth.initial_item_instances.push(InitialItemInstance {
                id: format!("test:c{index}"),
                prototype_id: "test:container".into(),
                location: if index == 4 {
                    InitialItemLocation::Room {
                        room_id: "test:1".into(),
                    }
                } else {
                    InitialItemLocation::Container {
                        container_instance_id: format!("test:c{}", index + 1),
                    }
                },
            });
        }
        depth.validate().unwrap();
        depth
            .initial_item_instances
            .iter_mut()
            .find(|instance| instance.id == "test:c4")
            .unwrap()
            .location = InitialItemLocation::Container {
            container_instance_id: "test:c5".into(),
        };
        depth.initial_item_instances.push(InitialItemInstance {
            id: "test:c5".into(),
            prototype_id: "test:container".into(),
            location: InitialItemLocation::Room {
                room_id: "test:1".into(),
            },
        });
        assert!(
            depth
                .validate()
                .unwrap_err()
                .0
                .iter()
                .any(|error| error.to_string().contains("maximum container depth 4"))
        );
    }

    #[test]
    fn authored_container_capacity_uses_derived_stacks_and_recursive_weight() {
        let mut value = pack();
        let mut tonic = item_prototype("test:tonic");
        tonic.stack_policy = ItemStackPolicy::Fungible;
        tonic.max_stack_units = 2;
        tonic.slot_cost = 2;
        tonic.weight_grams = 100;
        let mut outer = item_prototype("test:outer");
        outer.weight_grams = 50;
        outer.container = Some(ItemContainerDefinition {
            max_slots: 5,
            max_weight_grams: 350,
        });
        let mut inner = item_prototype("test:inner");
        inner.weight_grams = 50;
        inner.container = Some(ItemContainerDefinition {
            max_slots: 4,
            max_weight_grams: 300,
        });
        value.item_prototypes.extend([tonic, outer, inner]);
        value.initial_item_instances.extend([
            InitialItemInstance {
                id: "test:outer-1".into(),
                prototype_id: "test:outer".into(),
                location: InitialItemLocation::Room {
                    room_id: "test:1".into(),
                },
            },
            InitialItemInstance {
                id: "test:inner-1".into(),
                prototype_id: "test:inner".into(),
                location: InitialItemLocation::Container {
                    container_instance_id: "test:outer-1".into(),
                },
            },
        ]);
        for index in 1..=3 {
            value.initial_item_instances.push(InitialItemInstance {
                id: format!("test:tonic-{index}"),
                prototype_id: "test:tonic".into(),
                location: InitialItemLocation::Container {
                    container_instance_id: "test:inner-1".into(),
                },
            });
        }
        value.validate().unwrap();

        value
            .item_prototypes
            .iter_mut()
            .find(|prototype| prototype.id == "test:inner")
            .unwrap()
            .container
            .as_mut()
            .unwrap()
            .max_slots = 3;
        assert!(
            value
                .validate()
                .unwrap_err()
                .0
                .iter()
                .any(|error| error.to_string().contains("slot capacity"))
        );

        value
            .item_prototypes
            .iter_mut()
            .find(|prototype| prototype.id == "test:inner")
            .unwrap()
            .container
            .as_mut()
            .unwrap()
            .max_slots = 4;
        value
            .item_prototypes
            .iter_mut()
            .find(|prototype| prototype.id == "test:outer")
            .unwrap()
            .container
            .as_mut()
            .unwrap()
            .max_weight_grams = 349;
        assert!(
            value
                .validate()
                .unwrap_err()
                .0
                .iter()
                .any(|error| error.to_string().contains("weight capacity"))
        );

        value
            .item_prototypes
            .iter_mut()
            .find(|prototype| prototype.id == "test:tonic")
            .unwrap()
            .max_stack_units = 0;
        assert!(value.validate().is_err());
    }
}
