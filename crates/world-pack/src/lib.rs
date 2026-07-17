//! Portable, versioned world data produced by importers and authoring tools.

use std::collections::{BTreeMap, HashMap, HashSet, VecDeque};

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use thiserror::Error;

mod area_catalog;
mod area_registry;
mod authoring;
mod composition;
mod progression;

pub use area_catalog::{
    AREA_BUNDLE_SCHEMA_VERSION, AREA_CATALOG_SCHEMA_VERSION, AREA_RELEASE_SCHEMA_VERSION,
    AreaBundle, AreaCatalog, AreaCatalogEntry, AreaCatalogError, AreaCompatibility, AreaPortal,
    AreaPortalDirection, AreaReleaseEntry, AreaReleaseManifest, CompiledArea, ContentDigest,
    ReleaseTrustMode, VerifiedAreaRelease, build_area_release, parse_area_bundle,
    parse_area_catalog, parse_area_release_manifest, verify_area_release,
};
pub use area_registry::{
    ActiveAreaCatalog, ActiveAreaCatalogIdentity, GenerationReplacementKind, LocalAreaGeneration,
    LocalAreaRegistryError, MAX_LOCAL_AREA_POINTER_BYTES, MAX_LOCAL_HARD_DEPENDENCY_CLOSURE,
    StagedLocalAreaGenerationReplacement, local_active_area_catalog_path, local_area_bundle_path,
    local_area_catalog_path, local_area_publication_lock_path, local_area_world_path,
    read_bounded_local_registry_file, read_bounded_regular_file, stage_generation_replacement,
    stage_generation_rollback, validate_generation_replacement, verify_existing_registry_directory,
};
#[cfg(unix)]
pub use area_registry::{
    LocalAreaPublicationReadGuard, try_acquire_local_area_publication_read_lock,
};
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
    AREA_PACK_SCHEMA_VERSION, AreaAssetManifestDigest, AreaComposition, AreaCrossReference,
    AreaCrossReferenceKind, AreaOwnership, AreaPack, ComposedWorld, CompositionError,
    CompositionReport, MAX_AREA_COUNT, MAX_AREA_FILE_BYTES, MAX_COMPOSED_WORLD_BYTES,
    MAX_PROJECT_MANIFEST_BYTES, MAX_TOTAL_AREA_BYTES, ProjectArea, WORLD_PROJECT_SCHEMA_VERSION,
    WorldProjectManifest, compose_world_project,
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

pub const CURRENT_SCHEMA_VERSION: u32 = 9;
pub const MAX_ROOM_TITLE_BYTES: usize = 128;
pub const MAX_ROOM_DESCRIPTION_BYTES: usize = 8_192;
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
pub const MAX_LOOT_TABLES: usize = 256;
pub const MAX_LOOT_SOURCES: usize = 1_024;
pub const MAX_LOOT_GUARANTEED_PER_TABLE: usize = 8;
pub const MAX_LOOT_POOLS_PER_TABLE: usize = 8;
pub const MAX_LOOT_ROLLS_PER_POOL: u8 = 8;
pub const MAX_LOOT_ENTRIES_PER_POOL: usize = 64;
pub const MAX_LOOT_ENTRIES: usize = 4_096;
pub const MAX_LOOT_WEIGHT: u32 = 1_000_000;
pub const MAX_LOOT_QUANTITY: u16 = 32;
pub const MAX_LOOT_UNITS_PER_RESOLUTION: u64 = 64;
pub const MAX_LOOT_ROOTS_PER_DEFEAT: u64 = 64;
pub const MAX_SEALED_REWARD_UNITS: u64 = 16;
pub const MAX_AFFIX_DEFINITIONS: usize = 512;
pub const MAX_AFFIX_POOLS: usize = 256;
pub const MAX_AFFIX_ENTRIES_PER_POOL: usize = 64;
pub const MAX_AFFIX_ENTRIES: usize = 4_096;
pub const MAX_AFFIX_LABEL_BYTES: usize = 64;
pub const MAX_AFFIX_MODIFIER: i32 = 1_000;
/// Maximum number of base outcomes plus conditional affix outcomes that the
/// runtime can expose through the paginated `reward odds` view for one sealed
/// reward table.
pub const MAX_REWARD_ODDS_ROWS: usize = 1_024;
pub const MAX_ARCHETYPES: usize = 32;
pub const MAX_ABILITIES: usize = 512;
pub const MAX_NPC_ABILITY_SETS: usize = 512;
pub const MAX_PROGRESSION_LEVEL: u32 = 100;
pub const MAX_STATS_PER_ARCHETYPE: usize = 64;
pub const MAX_RESOURCES_PER_ARCHETYPE: usize = 16;
pub const MAX_ABILITIES_PER_ACTOR: usize = 64;
pub const MAX_ABILITY_RESOURCE_COSTS: usize = 4;
pub const MAX_ABILITY_STATUS_EFFECTS: usize = 8;
pub const MAX_COMMUNICATION_ABILITIES: usize = 32;
pub const MAX_COMMUNICATION_BLOCKED_ROOMS: usize = 64;
pub const MAX_SOCIAL_EMOTES: usize = 32;
pub const MAX_SOCIAL_EMOTE_ID_BYTES: usize = 64;
pub const MAX_SOCIAL_EMOTE_COMMAND_BYTES: usize = 32;
pub const MAX_SOCIAL_EMOTE_NAME_BYTES: usize = 64;
pub const MAX_SOCIAL_EMOTE_RENDERED_BYTES: usize = 512;
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
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sealed_loot_table_id: Option<String>,
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
    /// Closed, author-controlled room emotes. Templates are validated before
    /// the runtime receives them and may only interpolate actor/target names.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub social_emotes: Vec<SocialEmoteDefinition>,
    /// Ability loadouts remain separate from [`NpcPrototype`] and reusable.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub npc_ability_sets: Vec<NpcAbilitySet>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub loot_tables: Vec<LootTableDefinition>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub loot_sources: Vec<LootSourceDefinition>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub affixes: Vec<AffixDefinition>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub affix_pools: Vec<AffixPoolDefinition>,
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
            && self.social_emotes.is_empty()
            && self.npc_ability_sets.is_empty()
            && self.loot_tables.is_empty()
            && self.loot_sources.is_empty()
            && self.affixes.is_empty()
            && self.affix_pools.is_empty()
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SocialEmoteDefinition {
    pub id: String,
    pub command: String,
    pub name: String,
    pub audience: SocialEmoteAudience,
    pub targeting: SocialEmoteTargeting,
    pub templates: SocialEmoteTemplates,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SocialEmoteAudience {
    Room,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SocialEmoteTargeting {
    None,
    OptionalCharacter,
    RequiredCharacter,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SocialEmoteTemplates {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub untargeted: Option<SocialEmoteUntargetedTemplates>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub targeted: Option<SocialEmoteTargetedTemplates>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SocialEmoteUntargetedTemplates {
    pub actor: String,
    pub others: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SocialEmoteTargetedTemplates {
    pub actor: String,
    pub target: String,
    pub others: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct LootTableDefinition {
    pub id: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub guaranteed: Vec<LootGuaranteedDefinition>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub pools: Vec<LootPoolDefinition>,
}

impl LootTableDefinition {
    /// SHA-256 of the stable authoring form used by durable loot provenance.
    pub fn canonical_sha256(&self) -> String {
        let mut canonical = self.clone();
        canonical
            .guaranteed
            .sort_by(|left, right| left.id.cmp(&right.id));
        canonical
            .pools
            .sort_by(|left, right| left.id.cmp(&right.id));
        for pool in &mut canonical.pools {
            pool.entries.sort_by(|left, right| left.id.cmp(&right.id));
        }
        let bytes = serde_json::to_vec(&canonical)
            .expect("loot table definitions contain only serializable values");
        hex::encode(Sha256::digest(bytes))
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct LootGuaranteedDefinition {
    pub id: String,
    pub item_prototype_id: String,
    pub quantity: u16,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub affix_pool_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct LootPoolDefinition {
    pub id: String,
    pub rolls: u8,
    pub entries: Vec<LootEntryDefinition>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct LootEntryDefinition {
    pub kind: LootEntryKind,
    pub id: String,
    pub weight: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub item_prototype_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub min_quantity: Option<u16>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_quantity: Option<u16>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub affix_pool_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AffixDefinition {
    pub id: String,
    pub label: String,
    pub position: AffixPosition,
    #[serde(default)]
    pub attack_bonus_delta: i32,
    #[serde(default)]
    pub heal_amount_delta: i32,
}

impl AffixDefinition {
    pub fn canonical_sha256(&self) -> String {
        let bytes =
            serde_json::to_vec(self).expect("affix definitions contain only serializable values");
        hex::encode(Sha256::digest(bytes))
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AffixPosition {
    Prefix,
    Suffix,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AffixPoolDefinition {
    pub id: String,
    pub entries: Vec<AffixPoolEntry>,
}

impl AffixPoolDefinition {
    pub fn canonical_sha256(&self) -> String {
        let mut canonical = self.clone();
        canonical
            .entries
            .sort_by(|left, right| left.id.cmp(&right.id));
        let bytes = serde_json::to_vec(&canonical)
            .expect("affix pool definitions contain only serializable values");
        hex::encode(Sha256::digest(bytes))
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AffixPoolEntry {
    pub kind: AffixPoolEntryKind,
    pub id: String,
    pub weight: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub affix_id: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AffixPoolEntryKind {
    Affix,
    Nothing,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LootEntryKind {
    Item,
    Nothing,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct LootSourceDefinition {
    pub kind: LootSourceKind,
    pub id: String,
    pub npc_spawn_id: String,
    pub loot_table_id: String,
    pub rights: LootRightsPolicy,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LootSourceKind {
    NpcSpawn,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LootRightsPolicy {
    CreditedCharacter,
    PartyRoundRobin,
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
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub communication: Option<AbilityCommunicationDefinition>,
}

/// A closed, author-controlled communication effect. Communication abilities
/// still use the ordinary ability ownership, resource and cooldown rules.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AbilityCommunicationDefinition {
    pub kind: AbilityCommunicationKind,
    pub scope: AbilityCommunicationScope,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub blocked_room_ids: Vec<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AbilityCommunicationKind {
    Telepathy,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AbilityCommunicationScope {
    DirectOnline,
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
            } else if !safe_gameplay_label(&room.title, MAX_ROOM_TITLE_BYTES) {
                errors.push(ValidationError::InvalidGameplay(format!(
                    "room {:?} title must contain 1..={MAX_ROOM_TITLE_BYTES} safe bytes without control characters",
                    room.id
                )));
            }
            if !safe_gameplay_text(&room.description, MAX_ROOM_DESCRIPTION_BYTES) {
                errors.push(ValidationError::InvalidGameplay(format!(
                    "room {:?} description must contain 1..={MAX_ROOM_DESCRIPTION_BYTES} safe bytes",
                    room.id
                )));
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
            if let Some(table_id) = &prototype.sealed_loot_table_id
                && (!valid_gameplay_identifier(table_id)
                    || prototype.stack_policy != ItemStackPolicy::Unique
                    || prototype.max_stack_units != 1
                    || !prototype.consumable
                    || prototype.transfer_policy != ItemTransferPolicy::CharacterBound
                    || prototype.container.is_some()
                    || prototype.equipment_slot.is_some()
                    || prototype.attack_bonus != 0
                    || prototype.heal_amount != 0
                    || prototype.effect.is_some())
            {
                errors.push(ValidationError::InvalidGameplay(format!(
                    "sealed reward prototype {:?} must reference a canonical table and be unique, consumable, character_bound, non-structural, and effect-free",
                    prototype.id
                )));
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
            if !valid_item_instance_identifier(&instance.id) {
                errors.push(ValidationError::InvalidGameplay(format!(
                    "initial item instance id {:?} must be a canonical runtime id or contain 1..=128 lowercase namespaced identifier bytes",
                    instance.id
                )));
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
            if self
                .item_prototypes
                .iter()
                .find(|prototype| prototype.id == instance.prototype_id)
                .is_some_and(|prototype| prototype.sealed_loot_table_id.is_some())
            {
                errors.push(ValidationError::InvalidGameplay(format!(
                    "sealed reward prototype {:?} cannot have an initial instance",
                    instance.prototype_id
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

        validate_social_emotes(&self.gameplay.social_emotes, &mut errors);
        validate_character_and_abilities(&self.gameplay, &prototypes, &room_ids, &mut errors);
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

        validate_affix_authoring(
            &self.gameplay.affixes,
            &self.gameplay.affix_pools,
            &mut errors,
        );
        validate_loot_authoring(
            &self.gameplay.loot_tables,
            &self.gameplay.loot_sources,
            &self.initial_item_instances,
            &self.item_prototypes,
            &self.gameplay.affixes,
            &self.gameplay.affix_pools,
            &spawn_ids,
            &mut errors,
        );

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

fn validate_affix_authoring(
    affixes: &[AffixDefinition],
    pools: &[AffixPoolDefinition],
    errors: &mut Vec<ValidationError>,
) {
    if affixes.len() > MAX_AFFIX_DEFINITIONS {
        invalid_gameplay(
            errors,
            format!(
                "WorldPack declares {} affixes; the maximum is {MAX_AFFIX_DEFINITIONS}",
                affixes.len()
            ),
        );
    }
    if pools.len() > MAX_AFFIX_POOLS {
        invalid_gameplay(
            errors,
            format!(
                "WorldPack declares {} affix pools; the maximum is {MAX_AFFIX_POOLS}",
                pools.len()
            ),
        );
    }

    let mut affix_ids = HashSet::new();
    for affix in affixes {
        if !valid_gameplay_identifier(&affix.id) {
            invalid_gameplay(
                errors,
                format!(
                    "affix id {:?} must contain 1..=128 canonical identifier bytes",
                    affix.id
                ),
            );
        } else if !affix_ids.insert(affix.id.as_str()) {
            invalid_gameplay(
                errors,
                format!("affix id {:?} appears more than once", affix.id),
            );
        }
        if !safe_gameplay_label(&affix.label, MAX_AFFIX_LABEL_BYTES)
            || !(0..=MAX_AFFIX_MODIFIER).contains(&affix.attack_bonus_delta)
            || !(0..=MAX_AFFIX_MODIFIER).contains(&affix.heal_amount_delta)
            || (affix.attack_bonus_delta == 0 && affix.heal_amount_delta == 0)
        {
            invalid_gameplay(
                errors,
                format!(
                    "affix {:?} must have a safe label and at least one bounded positive modifier",
                    affix.id
                ),
            );
        }
    }

    let mut pool_ids = HashSet::new();
    let mut used_affixes = HashSet::new();
    let mut total_entries = 0usize;
    for pool in pools {
        if !valid_gameplay_identifier(&pool.id) {
            invalid_gameplay(
                errors,
                format!(
                    "affix pool id {:?} must contain 1..=128 canonical identifier bytes",
                    pool.id
                ),
            );
        } else if !pool_ids.insert(pool.id.as_str()) {
            invalid_gameplay(
                errors,
                format!("affix pool id {:?} appears more than once", pool.id),
            );
        }
        if pool.entries.is_empty() || pool.entries.len() > MAX_AFFIX_ENTRIES_PER_POOL {
            invalid_gameplay(
                errors,
                format!(
                    "affix pool {:?} must contain 1..={MAX_AFFIX_ENTRIES_PER_POOL} entries",
                    pool.id
                ),
            );
        }
        total_entries = total_entries.saturating_add(pool.entries.len());
        let mut entry_ids = HashSet::new();
        let mut pool_affixes = HashSet::new();
        let mut total_weight = 0u64;
        for entry in &pool.entries {
            if !valid_gameplay_identifier(&entry.id) || !entry_ids.insert(entry.id.as_str()) {
                invalid_gameplay(
                    errors,
                    format!(
                        "affix pool {:?} entry id {:?} is invalid or duplicated",
                        pool.id, entry.id
                    ),
                );
            }
            if !(1..=MAX_LOOT_WEIGHT).contains(&entry.weight) {
                invalid_gameplay(
                    errors,
                    format!(
                        "affix pool {:?} entry {:?} weight must be 1..={MAX_LOOT_WEIGHT}",
                        pool.id, entry.id
                    ),
                );
            }
            total_weight = total_weight.saturating_add(u64::from(entry.weight));
            match entry.kind {
                AffixPoolEntryKind::Affix => match entry.affix_id.as_deref() {
                    Some(affix_id)
                        if affix_ids.contains(affix_id) && pool_affixes.insert(affix_id) =>
                    {
                        used_affixes.insert(affix_id);
                    }
                    _ => invalid_gameplay(
                        errors,
                        format!(
                            "affix pool {:?} entry {:?} must reference one unique existing affix",
                            pool.id, entry.id
                        ),
                    ),
                },
                AffixPoolEntryKind::Nothing if entry.affix_id.is_some() => invalid_gameplay(
                    errors,
                    format!(
                        "affix pool {:?} nothing entry {:?} cannot reference an affix",
                        pool.id, entry.id
                    ),
                ),
                AffixPoolEntryKind::Nothing => {}
            }
        }
        if total_weight == u64::MAX {
            invalid_gameplay(
                errors,
                format!("affix pool {:?} weight sum overflows", pool.id),
            );
        }
    }
    if total_entries > MAX_AFFIX_ENTRIES {
        invalid_gameplay(
            errors,
            format!(
                "WorldPack declares {total_entries} weighted affix entries; the maximum is {MAX_AFFIX_ENTRIES}"
            ),
        );
    }
    for affix_id in affix_ids {
        if !used_affixes.contains(affix_id) {
            invalid_gameplay(errors, format!("affix {affix_id:?} has no pool entry"));
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn validate_loot_authoring(
    tables: &[LootTableDefinition],
    sources: &[LootSourceDefinition],
    initial_item_instances: &[InitialItemInstance],
    item_prototypes: &[ItemPrototype],
    affixes: &[AffixDefinition],
    affix_pools: &[AffixPoolDefinition],
    npc_spawn_ids: &HashSet<&str>,
    errors: &mut Vec<ValidationError>,
) {
    let item_prototypes = item_prototypes
        .iter()
        .map(|prototype| (prototype.id.as_str(), prototype))
        .collect::<HashMap<_, _>>();
    let item_prototype_ids = item_prototypes.keys().copied().collect::<HashSet<_>>();
    let affixes = affixes
        .iter()
        .map(|affix| (affix.id.as_str(), affix))
        .collect::<HashMap<_, _>>();
    let affix_pools = affix_pools
        .iter()
        .map(|pool| (pool.id.as_str(), pool))
        .collect::<HashMap<_, _>>();
    let sealed_prototype_ids = item_prototypes
        .iter()
        .filter_map(|(id, prototype)| prototype.sealed_loot_table_id.as_ref().map(|_| *id))
        .collect::<HashSet<_>>();
    let sealed_table_ids = item_prototypes
        .values()
        .filter_map(|prototype| prototype.sealed_loot_table_id.as_deref())
        .collect::<HashSet<_>>();
    if tables.len() > MAX_LOOT_TABLES {
        invalid_gameplay(
            errors,
            format!(
                "WorldPack declares {} loot tables; the maximum is {MAX_LOOT_TABLES}",
                tables.len()
            ),
        );
    }
    if sources.len() > MAX_LOOT_SOURCES {
        invalid_gameplay(
            errors,
            format!(
                "WorldPack declares {} loot sources; the maximum is {MAX_LOOT_SOURCES}",
                sources.len()
            ),
        );
    }

    let mut table_ids = HashSet::new();
    let mut table_minimum_units = HashMap::new();
    let mut table_worst_case_units = HashMap::new();
    let mut table_reward_odds_rows = HashMap::new();
    let mut table_output_prototypes = HashMap::<&str, HashSet<&str>>::new();
    let mut used_affix_pools = HashSet::<String>::new();
    let mut total_pool_entries = 0usize;
    for table in tables {
        if !valid_gameplay_identifier(&table.id) {
            invalid_gameplay(
                errors,
                format!(
                    "loot table id {:?} must contain 1..=128 canonical identifier bytes",
                    table.id
                ),
            );
        } else if !table_ids.insert(table.id.as_str()) {
            invalid_gameplay(
                errors,
                format!("loot table id {:?} appears more than once", table.id),
            );
        }
        if table.guaranteed.is_empty() && table.pools.is_empty() {
            invalid_gameplay(
                errors,
                format!("loot table {:?} must not be empty", table.id),
            );
        }
        if table.guaranteed.len() > MAX_LOOT_GUARANTEED_PER_TABLE {
            invalid_gameplay(
                errors,
                format!(
                    "loot table {:?} has {} guaranteed entries; the maximum is {MAX_LOOT_GUARANTEED_PER_TABLE}",
                    table.id,
                    table.guaranteed.len()
                ),
            );
        }
        if table.pools.len() > MAX_LOOT_POOLS_PER_TABLE {
            invalid_gameplay(
                errors,
                format!(
                    "loot table {:?} has {} pools; the maximum is {MAX_LOOT_POOLS_PER_TABLE}",
                    table.id,
                    table.pools.len()
                ),
            );
        }

        let mut minimum_units = 0u64;
        let mut worst_case_units = 0u64;
        let mut reward_odds_rows = 0usize;
        let mut component_ids = HashSet::new();
        let mut output_prototypes = HashSet::new();
        for guaranteed in &table.guaranteed {
            reward_odds_rows = reward_odds_rows.saturating_add(1);
            if let Some(pool) = guaranteed
                .affix_pool_id
                .as_deref()
                .and_then(|pool_id| affix_pools.get(pool_id))
            {
                reward_odds_rows = reward_odds_rows.saturating_add(pool.entries.len());
            }
            validate_loot_local_id(
                &guaranteed.id,
                &mut component_ids,
                "guaranteed entry",
                &table.id,
                errors,
            );
            if !item_prototype_ids.contains(guaranteed.item_prototype_id.as_str()) {
                invalid_gameplay(
                    errors,
                    format!(
                        "loot table {:?} guaranteed entry {:?} references missing item prototype {:?}",
                        table.id, guaranteed.id, guaranteed.item_prototype_id
                    ),
                );
            } else {
                output_prototypes.insert(guaranteed.item_prototype_id.as_str());
            }
            if !(1..=MAX_LOOT_QUANTITY).contains(&guaranteed.quantity) {
                invalid_gameplay(
                    errors,
                    format!(
                        "loot table {:?} guaranteed entry {:?} quantity must be 1..={MAX_LOOT_QUANTITY}",
                        table.id, guaranteed.id
                    ),
                );
            }
            if guaranteed.affix_pool_id.is_some() && guaranteed.quantity != 1 {
                invalid_gameplay(
                    errors,
                    format!(
                        "loot table {:?} guaranteed entry {:?} may apply one affix only with quantity 1",
                        table.id, guaranteed.id
                    ),
                );
            }
            worst_case_units = worst_case_units.saturating_add(u64::from(guaranteed.quantity));
            minimum_units = minimum_units.saturating_add(u64::from(guaranteed.quantity));
            validate_loot_affix_reference(
                guaranteed.affix_pool_id.as_deref(),
                &guaranteed.item_prototype_id,
                &item_prototypes,
                &affixes,
                &affix_pools,
                &mut used_affix_pools,
                &table.id,
                &guaranteed.id,
                errors,
            );
        }

        for pool in &table.pools {
            validate_loot_local_id(&pool.id, &mut component_ids, "pool", &table.id, errors);
            if !(1..=MAX_LOOT_ROLLS_PER_POOL).contains(&pool.rolls) {
                invalid_gameplay(
                    errors,
                    format!(
                        "loot table {:?} pool {:?} rolls must be 1..={MAX_LOOT_ROLLS_PER_POOL}",
                        table.id, pool.id
                    ),
                );
            }
            if pool.entries.is_empty() || pool.entries.len() > MAX_LOOT_ENTRIES_PER_POOL {
                invalid_gameplay(
                    errors,
                    format!(
                        "loot table {:?} pool {:?} must contain 1..={MAX_LOOT_ENTRIES_PER_POOL} entries",
                        table.id, pool.id
                    ),
                );
            }
            total_pool_entries = total_pool_entries.saturating_add(pool.entries.len());

            let mut total_weight = 0u64;
            let mut minimum_quantity = u16::MAX;
            let mut maximum_quantity = 0u16;
            for entry in &pool.entries {
                reward_odds_rows = reward_odds_rows.saturating_add(1);
                if let Some(affix_pool) = entry
                    .affix_pool_id
                    .as_deref()
                    .and_then(|pool_id| affix_pools.get(pool_id))
                {
                    reward_odds_rows = reward_odds_rows.saturating_add(affix_pool.entries.len());
                }
                validate_loot_local_id(
                    &entry.id,
                    &mut component_ids,
                    "pool entry",
                    &table.id,
                    errors,
                );
                if !(1..=MAX_LOOT_WEIGHT).contains(&entry.weight) {
                    invalid_gameplay(
                        errors,
                        format!(
                            "loot table {:?} pool {:?} entry {:?} weight must be 1..={MAX_LOOT_WEIGHT}",
                            table.id, pool.id, entry.id
                        ),
                    );
                }
                total_weight = total_weight.saturating_add(u64::from(entry.weight));
                match entry.kind {
                    LootEntryKind::Item => {
                        let (Some(prototype_id), Some(minimum), Some(maximum)) = (
                            entry.item_prototype_id.as_deref(),
                            entry.min_quantity,
                            entry.max_quantity,
                        ) else {
                            invalid_gameplay(
                                errors,
                                format!(
                                    "loot table {:?} pool {:?} item entry {:?} requires itemPrototypeId, minQuantity and maxQuantity",
                                    table.id, pool.id, entry.id
                                ),
                            );
                            continue;
                        };
                        if !item_prototype_ids.contains(prototype_id) {
                            invalid_gameplay(
                                errors,
                                format!(
                                    "loot table {:?} pool {:?} entry {:?} references missing item prototype {:?}",
                                    table.id, pool.id, entry.id, prototype_id
                                ),
                            );
                        } else {
                            output_prototypes.insert(prototype_id);
                        }
                        if minimum == 0 || minimum > maximum || maximum > MAX_LOOT_QUANTITY {
                            invalid_gameplay(
                                errors,
                                format!(
                                    "loot table {:?} pool {:?} entry {:?} quantity must be an ordered range within 1..={MAX_LOOT_QUANTITY}",
                                    table.id, pool.id, entry.id
                                ),
                            );
                        }
                        maximum_quantity = maximum_quantity.max(maximum);
                        minimum_quantity = minimum_quantity.min(minimum);
                        if entry.affix_pool_id.is_some() && (minimum != 1 || maximum != 1) {
                            invalid_gameplay(
                                errors,
                                format!(
                                    "loot table {:?} pool entry {:?} may apply one affix only with quantity exactly 1",
                                    table.id, entry.id
                                ),
                            );
                        }
                        validate_loot_affix_reference(
                            entry.affix_pool_id.as_deref(),
                            prototype_id,
                            &item_prototypes,
                            &affixes,
                            &affix_pools,
                            &mut used_affix_pools,
                            &table.id,
                            &entry.id,
                            errors,
                        );
                    }
                    LootEntryKind::Nothing => {
                        minimum_quantity = 0;
                        if entry.item_prototype_id.is_some()
                            || entry.min_quantity.is_some()
                            || entry.max_quantity.is_some()
                            || entry.affix_pool_id.is_some()
                        {
                            invalid_gameplay(
                                errors,
                                format!(
                                    "loot table {:?} pool {:?} nothing entry {:?} cannot declare item or quantity fields",
                                    table.id, pool.id, entry.id
                                ),
                            );
                        }
                    }
                }
            }
            if total_weight == u64::MAX {
                invalid_gameplay(
                    errors,
                    format!(
                        "loot table {:?} pool {:?} weight sum overflows",
                        table.id, pool.id
                    ),
                );
            }
            worst_case_units = worst_case_units
                .saturating_add(u64::from(pool.rolls).saturating_mul(u64::from(maximum_quantity)));
            let pool_minimum = if minimum_quantity != u16::MAX {
                minimum_quantity
            } else {
                0
            };
            minimum_units = minimum_units
                .saturating_add(u64::from(pool.rolls).saturating_mul(u64::from(pool_minimum)));
        }
        if worst_case_units > MAX_LOOT_UNITS_PER_RESOLUTION {
            invalid_gameplay(
                errors,
                format!(
                    "loot table {:?} can generate {worst_case_units} units; the maximum is {MAX_LOOT_UNITS_PER_RESOLUTION}",
                    table.id
                ),
            );
        }
        table_minimum_units.insert(table.id.as_str(), minimum_units);
        table_worst_case_units.insert(table.id.as_str(), worst_case_units);
        table_reward_odds_rows.insert(table.id.as_str(), reward_odds_rows);
        table_output_prototypes.insert(table.id.as_str(), output_prototypes);
    }
    if total_pool_entries > MAX_LOOT_ENTRIES {
        invalid_gameplay(
            errors,
            format!(
                "WorldPack declares {total_pool_entries} weighted loot entries; the maximum is {MAX_LOOT_ENTRIES}"
            ),
        );
    }
    for pool_id in affix_pools.keys() {
        if !used_affix_pools.contains(*pool_id) {
            invalid_gameplay(errors, format!("affix pool {pool_id:?} has no loot entry"));
        }
    }

    let mut source_ids = HashSet::new();
    let mut source_spawns = HashSet::new();
    let mut used_tables = HashSet::new();
    let mut npc_sourced_tables = HashSet::new();
    let mut fixed_roots_by_spawn = HashMap::<&str, u64>::new();
    for instance in initial_item_instances {
        if let InitialItemLocation::NpcSpawn { npc_spawn_id } = &instance.location {
            let count = fixed_roots_by_spawn
                .entry(npc_spawn_id.as_str())
                .or_default();
            *count = count.saturating_add(1);
        }
    }
    for source in sources {
        if !valid_gameplay_identifier(&source.id) {
            invalid_gameplay(
                errors,
                format!(
                    "loot source id {:?} must contain 1..=128 canonical identifier bytes",
                    source.id
                ),
            );
        } else if !source_ids.insert(source.id.as_str()) {
            invalid_gameplay(
                errors,
                format!("loot source id {:?} appears more than once", source.id),
            );
        }
        if !source_spawns.insert(source.npc_spawn_id.as_str()) {
            invalid_gameplay(
                errors,
                format!(
                    "NPC spawn {:?} has more than one loot source",
                    source.npc_spawn_id
                ),
            );
        }
        if !npc_spawn_ids.contains(source.npc_spawn_id.as_str()) {
            invalid_gameplay(
                errors,
                format!(
                    "loot source {:?} references missing NPC spawn {:?}",
                    source.id, source.npc_spawn_id
                ),
            );
        }
        if !table_ids.contains(source.loot_table_id.as_str()) {
            invalid_gameplay(
                errors,
                format!(
                    "loot source {:?} references missing loot table {:?}",
                    source.id, source.loot_table_id
                ),
            );
        } else if sealed_table_ids.contains(source.loot_table_id.as_str()) {
            invalid_gameplay(
                errors,
                format!(
                    "loot source {:?} cannot directly use sealed reward table {:?}",
                    source.id, source.loot_table_id
                ),
            );
        } else {
            used_tables.insert(source.loot_table_id.as_str());
            npc_sourced_tables.insert(source.loot_table_id.as_str());
        }
        let fixed_roots = fixed_roots_by_spawn
            .get(source.npc_spawn_id.as_str())
            .copied()
            .unwrap_or_default();
        let repeatable_roots = table_worst_case_units
            .get(source.loot_table_id.as_str())
            .copied()
            .unwrap_or_default();
        let total_roots = fixed_roots.saturating_add(repeatable_roots);
        if total_roots > MAX_LOOT_ROOTS_PER_DEFEAT {
            invalid_gameplay(
                errors,
                format!(
                    "NPC spawn {:?} can drop {total_roots} roots ({fixed_roots} fixed plus {repeatable_roots} repeatable); the maximum is {MAX_LOOT_ROOTS_PER_DEFEAT}",
                    source.npc_spawn_id
                ),
            );
        }
    }
    for (npc_spawn_id, fixed_roots) in fixed_roots_by_spawn {
        if !source_spawns.contains(npc_spawn_id) && fixed_roots > MAX_LOOT_ROOTS_PER_DEFEAT {
            invalid_gameplay(
                errors,
                format!(
                    "NPC spawn {npc_spawn_id:?} can drop {fixed_roots} fixed roots; the maximum is {MAX_LOOT_ROOTS_PER_DEFEAT}"
                ),
            );
        }
    }
    for prototype in item_prototypes.values() {
        let Some(table_id) = prototype.sealed_loot_table_id.as_deref() else {
            continue;
        };
        if !table_ids.contains(table_id) {
            invalid_gameplay(
                errors,
                format!("sealed reward prototype references missing loot table {table_id:?}"),
            );
            continue;
        }
        used_tables.insert(table_id);
        let worst_case = table_worst_case_units
            .get(table_id)
            .copied()
            .unwrap_or_default();
        let minimum = table_minimum_units
            .get(table_id)
            .copied()
            .unwrap_or_default();
        if minimum == 0 || worst_case > MAX_SEALED_REWARD_UNITS {
            invalid_gameplay(
                errors,
                format!(
                    "sealed reward table {table_id:?} must always generate 1..={MAX_SEALED_REWARD_UNITS} units"
                ),
            );
        }
        let reward_odds_rows = table_reward_odds_rows
            .get(table_id)
            .copied()
            .unwrap_or_default();
        if reward_odds_rows > MAX_REWARD_ODDS_ROWS {
            invalid_gameplay(
                errors,
                format!(
                    "sealed reward table {table_id:?} exposes {reward_odds_rows} probability rows; the maximum is {MAX_REWARD_ODDS_ROWS}"
                ),
            );
        }
        if table_output_prototypes
            .get(table_id)
            .is_some_and(|outputs| outputs.iter().any(|id| sealed_prototype_ids.contains(id)))
        {
            invalid_gameplay(
                errors,
                format!("sealed reward table {table_id:?} cannot generate another sealed reward"),
            );
        }
    }
    for sealed_id in &sealed_prototype_ids {
        let earned = npc_sourced_tables.iter().any(|table_id| {
            table_output_prototypes
                .get(table_id)
                .is_some_and(|outputs| outputs.contains(sealed_id))
        });
        if !earned {
            invalid_gameplay(
                errors,
                format!(
                    "sealed reward prototype {sealed_id:?} must be generated by an NPC loot source"
                ),
            );
        }
    }
    for table_id in table_ids {
        if !used_tables.contains(table_id) {
            invalid_gameplay(errors, format!("loot table {table_id:?} has no source"));
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn validate_loot_affix_reference(
    pool_id: Option<&str>,
    prototype_id: &str,
    item_prototypes: &HashMap<&str, &ItemPrototype>,
    affixes: &HashMap<&str, &AffixDefinition>,
    affix_pools: &HashMap<&str, &AffixPoolDefinition>,
    used_affix_pools: &mut HashSet<String>,
    table_id: &str,
    entry_id: &str,
    errors: &mut Vec<ValidationError>,
) {
    let Some(pool_id) = pool_id else {
        return;
    };
    let Some(pool) = affix_pools.get(pool_id) else {
        invalid_gameplay(
            errors,
            format!(
                "loot table {table_id:?} entry {entry_id:?} references missing affix pool {pool_id:?}"
            ),
        );
        return;
    };
    used_affix_pools.insert(pool_id.to_owned());
    let Some(prototype) = item_prototypes.get(prototype_id) else {
        return;
    };
    if prototype.stack_policy != ItemStackPolicy::Unique
        || prototype.max_stack_units != 1
        || prototype.sealed_loot_table_id.is_some()
    {
        invalid_gameplay(
            errors,
            format!(
                "loot table {table_id:?} entry {entry_id:?} can apply an affix only to a non-sealed unique prototype"
            ),
        );
        return;
    }
    let max_gameplay_value =
        i32::try_from(MAX_GAMEPLAY_VALUE).expect("MAX_GAMEPLAY_VALUE fits in i32");
    for entry in &pool.entries {
        let Some(affix_id) = entry.affix_id.as_deref() else {
            continue;
        };
        let Some(affix) = affixes.get(affix_id) else {
            continue;
        };
        let effective_attack = prototype.attack_bonus.checked_add(affix.attack_bonus_delta);
        let effective_heal = prototype.heal_amount.checked_add(affix.heal_amount_delta);
        if (affix.attack_bonus_delta > 0 && prototype.equipment_slot.is_none())
            || (affix.heal_amount_delta > 0 && !prototype.consumable)
            || effective_attack.is_none_or(|value| !(0..=max_gameplay_value).contains(&value))
            || effective_heal.is_none_or(|value| !(0..=max_gameplay_value).contains(&value))
        {
            invalid_gameplay(
                errors,
                format!(
                    "affix {affix_id:?} is incompatible with loot prototype {prototype_id:?} in table {table_id:?}"
                ),
            );
        }
    }
}

fn validate_loot_local_id<'a>(
    id: &'a str,
    seen: &mut HashSet<&'a str>,
    kind: &str,
    table_id: &str,
    errors: &mut Vec<ValidationError>,
) {
    if !valid_gameplay_identifier(id) {
        invalid_gameplay(
            errors,
            format!(
                "loot table {table_id:?} {kind} id {id:?} must contain 1..=128 canonical identifier bytes"
            ),
        );
    } else if !seen.insert(id) {
        invalid_gameplay(
            errors,
            format!("loot table {table_id:?} repeats {kind} id {id:?}"),
        );
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

fn validate_social_emotes(emotes: &[SocialEmoteDefinition], errors: &mut Vec<ValidationError>) {
    if emotes.len() > MAX_SOCIAL_EMOTES {
        invalid_gameplay(
            errors,
            format!(
                "world declares {} social emotes; the maximum is {MAX_SOCIAL_EMOTES}",
                emotes.len()
            ),
        );
    }

    let mut ids = HashSet::new();
    let mut commands = HashSet::new();
    for emote in emotes {
        if emote.id.len() > MAX_SOCIAL_EMOTE_ID_BYTES || !valid_gameplay_identifier(&emote.id) {
            invalid_gameplay(
                errors,
                format!(
                    "social emote id {:?} must contain 1..={MAX_SOCIAL_EMOTE_ID_BYTES} lowercase namespaced identifier bytes",
                    emote.id
                ),
            );
        } else if !ids.insert(emote.id.as_str()) {
            invalid_gameplay(
                errors,
                format!("social emote id {:?} appears more than once", emote.id),
            );
        }
        if !valid_social_emote_command(&emote.command) {
            invalid_gameplay(
                errors,
                format!(
                    "social emote {:?} command must match [a-z][a-z0-9-]* in 1..={MAX_SOCIAL_EMOTE_COMMAND_BYTES} ASCII bytes",
                    emote.id
                ),
            );
        } else if !commands.insert(emote.command.as_str()) {
            invalid_gameplay(
                errors,
                format!(
                    "social emote command {:?} appears more than once",
                    emote.command
                ),
            );
        }
        if !safe_social_emote_plain_text(&emote.name, MAX_SOCIAL_EMOTE_NAME_BYTES) {
            invalid_gameplay(
                errors,
                format!(
                    "social emote {:?} name must contain 1..={MAX_SOCIAL_EMOTE_NAME_BYTES} safe bytes",
                    emote.id
                ),
            );
        }

        let required_shape = match emote.targeting {
            SocialEmoteTargeting::None => {
                emote.templates.untargeted.is_some() && emote.templates.targeted.is_none()
            }
            SocialEmoteTargeting::OptionalCharacter => {
                emote.templates.untargeted.is_some() && emote.templates.targeted.is_some()
            }
            SocialEmoteTargeting::RequiredCharacter => {
                emote.templates.untargeted.is_none() && emote.templates.targeted.is_some()
            }
        };
        if !required_shape {
            invalid_gameplay(
                errors,
                format!(
                    "social emote {:?} templates do not match its targeting mode",
                    emote.id
                ),
            );
        }

        if let Some(templates) = &emote.templates.untargeted {
            validate_social_emote_template(
                &emote.id,
                "untargeted.actor",
                &templates.actor,
                0,
                0,
                errors,
            );
            validate_social_emote_template(
                &emote.id,
                "untargeted.others",
                &templates.others,
                1,
                0,
                errors,
            );
        }
        if let Some(templates) = &emote.templates.targeted {
            validate_social_emote_template(
                &emote.id,
                "targeted.actor",
                &templates.actor,
                0,
                1,
                errors,
            );
            validate_social_emote_template(
                &emote.id,
                "targeted.target",
                &templates.target,
                1,
                0,
                errors,
            );
            validate_social_emote_template(
                &emote.id,
                "targeted.others",
                &templates.others,
                1,
                1,
                errors,
            );
        }
    }
}

fn valid_social_emote_command(value: &str) -> bool {
    let bytes = value.as_bytes();
    !bytes.is_empty()
        && bytes.len() <= MAX_SOCIAL_EMOTE_COMMAND_BYTES
        && bytes[0].is_ascii_lowercase()
        && bytes[1..]
            .iter()
            .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || *byte == b'-')
}

fn safe_social_emote_plain_text(value: &str, maximum_bytes: usize) -> bool {
    !value.is_empty()
        && value == value.trim()
        && value.len() <= maximum_bytes
        && !value.chars().any(char::is_control)
        && !value.contains(['<', '>', '&'])
}

fn validate_social_emote_template(
    emote_id: &str,
    field: &str,
    template: &str,
    expected_actor: usize,
    expected_target: usize,
    errors: &mut Vec<ValidationError>,
) {
    const ACTOR: &str = "{actor}";
    const TARGET: &str = "{target}";

    let actor_count = template.matches(ACTOR).count();
    let target_count = template.matches(TARGET).count();
    let without_known_placeholders = template.replace(ACTOR, "").replace(TARGET, "");
    let unsafe_text = template.trim().is_empty()
        || template != template.trim()
        || template.chars().any(char::is_control)
        || template.contains(['<', '>', '&'])
        || without_known_placeholders.contains(['{', '}']);
    let rendered_bytes = template
        .len()
        .checked_sub(actor_count.saturating_mul(ACTOR.len()))
        .and_then(|bytes| bytes.checked_sub(target_count.saturating_mul(TARGET.len())))
        .and_then(|bytes| bytes.checked_add(actor_count.saturating_mul(64)))
        .and_then(|bytes| bytes.checked_add(target_count.saturating_mul(64)));
    if unsafe_text
        || actor_count != expected_actor
        || target_count != expected_target
        || rendered_bytes.is_none_or(|bytes| bytes > MAX_SOCIAL_EMOTE_RENDERED_BYTES)
    {
        invalid_gameplay(
            errors,
            format!(
                "social emote {emote_id:?} template {field} has unsafe text, invalid placeholders, or exceeds {MAX_SOCIAL_EMOTE_RENDERED_BYTES} rendered bytes"
            ),
        );
    }
}

fn validate_character_and_abilities(
    gameplay: &GameplayDefinition,
    prototypes: &HashMap<&str, &NpcPrototype>,
    room_ids: &HashSet<&str>,
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
    let communication_ability_count = gameplay
        .abilities
        .iter()
        .filter(|ability| ability.communication.is_some())
        .count();
    if communication_ability_count > MAX_COMMUNICATION_ABILITIES {
        invalid_gameplay(
            errors,
            format!(
                "world declares {communication_ability_count} communication abilities; the maximum is {MAX_COMMUNICATION_ABILITIES}"
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
        validate_ability(ability, room_ids, errors);
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
        for ability_id in &set.ability_ids {
            if ability_by_id
                .get(ability_id.as_str())
                .is_some_and(|ability| ability.communication.is_some())
            {
                invalid_gameplay(
                    errors,
                    format!(
                        "NPC {:?} cannot use communication ability {ability_id:?}",
                        set.npc_prototype_id
                    ),
                );
            }
        }
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

fn validate_ability(
    ability: &AbilityDefinition,
    room_ids: &HashSet<&str>,
    errors: &mut Vec<ValidationError>,
) {
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
    if ability.damage.is_none()
        && ability.heal.is_none()
        && ability.status_effects.is_empty()
        && ability.communication.is_none()
    {
        invalid_gameplay(errors, format!("ability {:?} has no effect", ability.id));
    }
    if let Some(communication) = &ability.communication {
        if !matches!(
            ability.category,
            AbilityCategory::Spell | AbilityCategory::Utility
        ) || ability.targeting != AbilityTargeting::SingleAlly
            || ability.resource_costs.is_empty()
            || ability.cooldown_ticks == 0
            || ability.damage.is_some()
            || ability.heal.is_some()
            || !ability.status_effects.is_empty()
        {
            invalid_gameplay(
                errors,
                format!(
                    "communication ability {:?} must be spell or utility, target one ally, spend a resource, have a positive cooldown, and have no damage, heal, or status effects",
                    ability.id
                ),
            );
        }
        if communication.blocked_room_ids.len() > MAX_COMMUNICATION_BLOCKED_ROOMS {
            invalid_gameplay(
                errors,
                format!(
                    "communication ability {:?} blocks {} rooms; the maximum is {MAX_COMMUNICATION_BLOCKED_ROOMS}",
                    ability.id,
                    communication.blocked_room_ids.len()
                ),
            );
        }
        let mut blocked_room_ids = HashSet::new();
        for room_id in &communication.blocked_room_ids {
            if !blocked_room_ids.insert(room_id.as_str()) {
                invalid_gameplay(
                    errors,
                    format!(
                        "communication ability {:?} repeats blocked room {room_id:?}",
                        ability.id
                    ),
                );
            } else if !room_ids.contains(room_id.as_str()) {
                invalid_gameplay(
                    errors,
                    format!(
                        "communication ability {:?} references missing blocked room {room_id:?}",
                        ability.id
                    ),
                );
            }
        }
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

fn valid_item_instance_identifier(value: &str) -> bool {
    let bytes = value.as_bytes();
    if bytes.is_empty() || bytes.len() > 128 {
        return false;
    }
    let Some(sequence) = value.strip_prefix("runtime:") else {
        return valid_gameplay_identifier(value);
    };
    !sequence.is_empty()
        && sequence.bytes().all(|byte| byte.is_ascii_digit())
        && !(sequence.len() > 1 && sequence.starts_with('0'))
        && sequence
            .parse::<u64>()
            .ok()
            .is_some_and(|sequence| sequence > 0)
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
            sealed_loot_table_id: None,
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

    fn pack_with_loot() -> WorldPack {
        let mut value = pack();
        value.item_prototypes.push(item_prototype("test:token"));
        value.gameplay.npc_prototypes.push(NpcPrototype {
            id: "test:wolf".into(),
            name: "wolf".into(),
            description: "A test wolf.".into(),
            level: 1,
            max_health: 10,
            strength: 2,
            defence: 0,
            experience_reward: 10,
            respawn_ticks: 2,
        });
        value.gameplay.npc_spawns.push(NpcSpawn {
            id: "test:wolf-1".into(),
            prototype_id: "test:wolf".into(),
            room_id: "test:1".into(),
        });
        value.gameplay.loot_tables.push(LootTableDefinition {
            id: "test:wolf-drops".into(),
            guaranteed: vec![LootGuaranteedDefinition {
                id: "test:guaranteed".into(),
                item_prototype_id: "test:token".into(),
                quantity: 1,
                affix_pool_id: None,
            }],
            pools: vec![LootPoolDefinition {
                id: "test:pool".into(),
                rolls: 1,
                entries: vec![
                    LootEntryDefinition {
                        kind: LootEntryKind::Item,
                        id: "test:item".into(),
                        weight: 6_000,
                        item_prototype_id: Some("test:token".into()),
                        min_quantity: Some(1),
                        max_quantity: Some(2),
                        affix_pool_id: None,
                    },
                    LootEntryDefinition {
                        kind: LootEntryKind::Nothing,
                        id: "test:nothing".into(),
                        weight: 4_000,
                        item_prototype_id: None,
                        min_quantity: None,
                        max_quantity: None,
                        affix_pool_id: None,
                    },
                ],
            }],
        });
        value.gameplay.loot_sources.push(LootSourceDefinition {
            kind: LootSourceKind::NpcSpawn,
            id: "test:wolf-source".into(),
            npc_spawn_id: "test:wolf-1".into(),
            loot_table_id: "test:wolf-drops".into(),
            rights: LootRightsPolicy::PartyRoundRobin,
        });
        value
    }

    fn pack_with_sealed_affix_rewards() -> WorldPack {
        let mut value = pack_with_loot();
        let mut blade = item_prototype("test:storm-blade");
        blade.name = "storm blade".into();
        blade.equipment_slot = Some(EquipmentSlot::Weapon);
        blade.attack_bonus = 2;
        blade.rarity = ItemRarity::Rare;
        let mut seal = item_prototype("test:storm-seal");
        seal.name = "sealed storm writ".into();
        seal.consumable = true;
        seal.transfer_policy = ItemTransferPolicy::CharacterBound;
        seal.sealed_loot_table_id = Some("test:storm-rewards".into());
        value.item_prototypes.extend([blade, seal]);

        value.gameplay.affixes.push(AffixDefinition {
            id: "test:charged".into(),
            label: "charged".into(),
            position: AffixPosition::Prefix,
            attack_bonus_delta: 1,
            heal_amount_delta: 0,
        });
        value.gameplay.affix_pools.push(AffixPoolDefinition {
            id: "test:storm-affixes".into(),
            entries: vec![
                AffixPoolEntry {
                    kind: AffixPoolEntryKind::Affix,
                    id: "test:charged-roll".into(),
                    weight: 3,
                    affix_id: Some("test:charged".into()),
                },
                AffixPoolEntry {
                    kind: AffixPoolEntryKind::Nothing,
                    id: "test:plain-roll".into(),
                    weight: 1,
                    affix_id: None,
                },
            ],
        });
        value.gameplay.loot_tables[0]
            .guaranteed
            .push(LootGuaranteedDefinition {
                id: "test:earned-seal".into(),
                item_prototype_id: "test:storm-seal".into(),
                quantity: 1,
                affix_pool_id: None,
            });
        value.gameplay.loot_tables.push(LootTableDefinition {
            id: "test:storm-rewards".into(),
            guaranteed: vec![LootGuaranteedDefinition {
                id: "test:storm-blade-reward".into(),
                item_prototype_id: "test:storm-blade".into(),
                quantity: 1,
                affix_pool_id: Some("test:storm-affixes".into()),
            }],
            pools: Vec::new(),
        });
        value
    }

    fn add_fixed_loot_roots(pack: &mut WorldPack, count: usize) {
        pack.initial_item_instances
            .extend((0..count).map(|index| InitialItemInstance {
                id: format!("test:fixed-{index}"),
                prototype_id: "test:token".into(),
                location: InitialItemLocation::NpcSpawn {
                    npc_spawn_id: "test:wolf-1".into(),
                },
            }));
    }

    fn sealed_reward_odds_boundary_pack(extra_guaranteed: bool) -> WorldPack {
        let mut value = pack_with_sealed_affix_rewards();
        value.gameplay.affix_pools[0].entries.truncate(1);
        let reward_table = &mut value.gameplay.loot_tables[1];
        reward_table.guaranteed.clear();
        reward_table.pools = (0..MAX_LOOT_POOLS_PER_TABLE)
            .map(|pool_index| LootPoolDefinition {
                id: format!("test:odds-pool-{pool_index}"),
                rolls: 1,
                entries: (0..MAX_LOOT_ENTRIES_PER_POOL)
                    .map(|entry_index| LootEntryDefinition {
                        kind: LootEntryKind::Item,
                        id: format!("test:odds-entry-{pool_index}-{entry_index}"),
                        weight: 1,
                        item_prototype_id: Some("test:storm-blade".into()),
                        min_quantity: Some(1),
                        max_quantity: Some(1),
                        affix_pool_id: Some("test:storm-affixes".into()),
                    })
                    .collect(),
            })
            .collect();
        if extra_guaranteed {
            reward_table.guaranteed.push(LootGuaranteedDefinition {
                id: "test:odds-extra".into(),
                item_prototype_id: "test:storm-blade".into(),
                quantity: 1,
                affix_pool_id: None,
            });
        }
        value
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
    fn repeatable_loot_contract_is_valid_and_has_a_canonical_digest() {
        let pack = pack_with_loot();
        pack.validate().unwrap();
        let table = &pack.gameplay.loot_tables[0];
        let digest = table.canonical_sha256();
        assert_eq!(digest.len(), 64);

        let mut reordered = table.clone();
        reordered.pools[0].entries.reverse();
        assert_eq!(reordered.canonical_sha256(), digest);
    }

    #[test]
    fn sealed_rewards_and_single_affix_rolls_are_bounded_and_canonical() {
        let pack = pack_with_sealed_affix_rewards();
        pack.validate().unwrap();
        let seal = pack
            .item_prototypes
            .iter()
            .find(|prototype| prototype.id == "test:storm-seal")
            .unwrap();
        assert_eq!(
            seal.sealed_loot_table_id.as_deref(),
            Some("test:storm-rewards")
        );
        assert_eq!(pack.gameplay.affixes[0].canonical_sha256().len(), 64);
        let digest = pack.gameplay.affix_pools[0].canonical_sha256();
        let mut reordered = pack.gameplay.affix_pools[0].clone();
        reordered.entries.reverse();
        assert_eq!(reordered.canonical_sha256(), digest);

        let encoded = serde_json::to_vec(&pack).unwrap();
        assert_eq!(serde_json::from_slice::<WorldPack>(&encoded).unwrap(), pack);
    }

    #[test]
    fn sealed_reward_probability_rows_match_the_runtime_boundary() {
        let exact = sealed_reward_odds_boundary_pack(false);
        exact.validate().unwrap();

        let oversized = sealed_reward_odds_boundary_pack(true);
        let message = validation_message(&oversized);
        assert!(
            message.contains(
                "sealed reward table \"test:storm-rewards\" exposes 1025 probability rows; the maximum is 1024"
            ),
            "{message}"
        );
    }

    #[test]
    fn sealed_rewards_must_be_earned_non_structural_non_nested_and_at_most_sixteen_units() {
        let base = pack_with_sealed_affix_rewards();

        let mut initial = base.clone();
        initial.initial_item_instances.push(InitialItemInstance {
            id: "test:initial-seal".into(),
            prototype_id: "test:storm-seal".into(),
            location: InitialItemLocation::Staged,
        });
        let message = validation_message(&initial);
        assert!(
            message.contains("cannot have an initial instance"),
            "{message}"
        );

        let mut unearned = base.clone();
        unearned.gameplay.loot_tables[0]
            .guaranteed
            .retain(|entry| entry.item_prototype_id != "test:storm-seal");
        let message = validation_message(&unearned);
        assert!(
            message.contains("must be generated by an NPC loot source"),
            "{message}"
        );

        let mut nested = base.clone();
        nested.gameplay.loot_tables[1].guaranteed[0].item_prototype_id = "test:storm-seal".into();
        nested.gameplay.loot_tables[1].guaranteed[0].affix_pool_id = None;
        let message = validation_message(&nested);
        assert!(
            message.contains("cannot generate another sealed reward"),
            "{message}"
        );

        let mut oversized = base.clone();
        oversized.gameplay.loot_tables[1].guaranteed[0].quantity = 17;
        let message = validation_message(&oversized);
        assert!(
            message.contains("must always generate 1..=16 units"),
            "{message}"
        );

        let mut possible_nothing = base.clone();
        possible_nothing.gameplay.loot_tables[1].guaranteed.clear();
        possible_nothing.gameplay.loot_tables[1]
            .pools
            .push(LootPoolDefinition {
                id: "test:uncertain-reward".into(),
                rolls: 1,
                entries: vec![
                    LootEntryDefinition {
                        kind: LootEntryKind::Item,
                        id: "test:possible-blade".into(),
                        weight: 1,
                        item_prototype_id: Some("test:storm-blade".into()),
                        min_quantity: Some(1),
                        max_quantity: Some(1),
                        affix_pool_id: Some("test:storm-affixes".into()),
                    },
                    LootEntryDefinition {
                        kind: LootEntryKind::Nothing,
                        id: "test:possible-nothing".into(),
                        weight: 1,
                        item_prototype_id: None,
                        min_quantity: None,
                        max_quantity: None,
                        affix_pool_id: None,
                    },
                ],
            });
        let message = validation_message(&possible_nothing);
        assert!(
            message.contains("must always generate 1..=16 units"),
            "{message}"
        );

        let mut guaranteed_with_nothing = base.clone();
        guaranteed_with_nothing.gameplay.loot_tables[1]
            .pools
            .push(possible_nothing.gameplay.loot_tables[1].pools[0].clone());
        guaranteed_with_nothing.validate().unwrap();

        let mut directly_sourced = base.clone();
        directly_sourced.gameplay.npc_spawns.push(NpcSpawn {
            id: "test:wolf-2".into(),
            prototype_id: "test:wolf".into(),
            room_id: "test:1".into(),
        });
        directly_sourced
            .gameplay
            .loot_sources
            .push(LootSourceDefinition {
                kind: LootSourceKind::NpcSpawn,
                id: "test:sealed-direct-source".into(),
                npc_spawn_id: "test:wolf-2".into(),
                loot_table_id: "test:storm-rewards".into(),
                rights: LootRightsPolicy::CreditedCharacter,
            });
        let message = validation_message(&directly_sourced);
        assert!(
            message.contains(
                "loot source \"test:sealed-direct-source\" cannot directly use sealed reward table \"test:storm-rewards\""
            ),
            "{message}"
        );

        let mut structural = base;
        let seal = structural
            .item_prototypes
            .iter_mut()
            .find(|prototype| prototype.id == "test:storm-seal")
            .unwrap();
        seal.container = Some(ItemContainerDefinition {
            max_slots: 1,
            max_weight_grams: 1,
        });
        let message = validation_message(&structural);
        assert!(
            message.contains("non-structural, and effect-free"),
            "{message}"
        );
    }

    #[test]
    fn affixes_apply_to_exactly_one_compatible_unique_loot_unit() {
        let base = pack_with_sealed_affix_rewards();

        let mut stack = base.clone();
        let target = stack
            .item_prototypes
            .iter_mut()
            .find(|prototype| prototype.id == "test:storm-blade")
            .unwrap();
        target.stack_policy = ItemStackPolicy::Fungible;
        target.max_stack_units = 2;
        let message = validation_message(&stack);
        assert!(
            message.contains("only to a non-sealed unique prototype"),
            "{message}"
        );

        let mut incompatible = base.clone();
        incompatible
            .item_prototypes
            .iter_mut()
            .find(|prototype| prototype.id == "test:storm-blade")
            .unwrap()
            .equipment_slot = None;
        let message = validation_message(&incompatible);
        assert!(
            message.contains("is incompatible with loot prototype"),
            "{message}"
        );

        let mut nothing = base;
        let entry = &mut nothing.gameplay.affix_pools[0].entries[1];
        entry.affix_id = Some("test:charged".into());
        let message = validation_message(&nothing);
        assert!(
            message.contains("nothing entry") && message.contains("cannot reference"),
            "{message}"
        );

        let mut direct = pack_with_sealed_affix_rewards();
        direct
            .item_prototypes
            .iter_mut()
            .find(|prototype| prototype.id == "test:token")
            .unwrap()
            .equipment_slot = Some(EquipmentSlot::Trinket);
        direct.gameplay.loot_tables[0].pools[0].entries[0].affix_pool_id =
            Some("test:storm-affixes".into());
        direct.gameplay.loot_tables[0].pools[0].entries[0].max_quantity = Some(1);
        direct.validate().unwrap();

        let mut quantity = pack_with_sealed_affix_rewards();
        quantity.gameplay.loot_tables[1].guaranteed[0].quantity = 2;
        let message = validation_message(&quantity);
        assert!(message.contains("with quantity 1"), "{message}");
    }

    fn validation_message(pack: &WorldPack) -> String {
        pack.validate()
            .unwrap_err()
            .0
            .into_iter()
            .map(|error| error.to_string())
            .collect::<Vec<_>>()
            .join("\n")
    }

    #[test]
    fn repeatable_loot_rejects_invalid_entry_shapes_references_and_duplicate_sources() {
        let mut invalid = pack_with_loot();
        let nothing = &mut invalid.gameplay.loot_tables[0].pools[0].entries[1];
        nothing.id = "test:pool".into();
        nothing.item_prototype_id = Some("test:missing".into());
        nothing.min_quantity = Some(1);
        invalid.gameplay.loot_sources.push(LootSourceDefinition {
            kind: LootSourceKind::NpcSpawn,
            id: "test:duplicate-source".into(),
            npc_spawn_id: "test:wolf-1".into(),
            loot_table_id: "test:missing-table".into(),
            rights: LootRightsPolicy::CreditedCharacter,
        });

        let message = invalid
            .validate()
            .unwrap_err()
            .0
            .into_iter()
            .map(|error| error.to_string())
            .collect::<Vec<_>>()
            .join("\n");
        assert!(message.contains("nothing entry"), "{message}");
        assert!(message.contains("repeats pool entry id"), "{message}");
        assert!(message.contains("more than one loot source"), "{message}");
        assert!(message.contains("missing loot table"), "{message}");
    }

    #[test]
    fn repeatable_loot_worst_case_is_limited_to_sixty_four_units() {
        let mut boundary = pack_with_loot();
        boundary.gameplay.loot_tables[0].guaranteed[0].quantity = 32;
        boundary.gameplay.loot_tables[0].pools[0].rolls = 8;
        boundary.gameplay.loot_tables[0].pools[0].entries[0].max_quantity = Some(4);
        boundary.validate().unwrap();

        boundary.gameplay.loot_tables[0].pools[0].entries[0].max_quantity = Some(5);
        let message = boundary
            .validate()
            .unwrap_err()
            .0
            .into_iter()
            .map(|error| error.to_string())
            .collect::<Vec<_>>()
            .join("\n");
        assert!(message.contains("can generate 72 units"), "{message}");
    }

    #[test]
    fn fixed_loot_roots_per_defeat_are_limited_to_sixty_four() {
        let mut invalid = pack_with_loot();
        invalid.gameplay.loot_tables.clear();
        invalid.gameplay.loot_sources.clear();
        add_fixed_loot_roots(&mut invalid, 65);

        let message = invalid
            .validate()
            .unwrap_err()
            .0
            .into_iter()
            .map(|error| error.to_string())
            .collect::<Vec<_>>()
            .join("\n");
        assert!(message.contains("can drop 65 fixed roots"), "{message}");
        assert!(message.contains("maximum is 64"), "{message}");
    }

    #[test]
    fn fixed_and_repeatable_loot_share_the_per_defeat_root_limit() {
        let mut invalid = pack_with_loot();
        add_fixed_loot_roots(&mut invalid, 62);

        let message = invalid
            .validate()
            .unwrap_err()
            .0
            .into_iter()
            .map(|error| error.to_string())
            .collect::<Vec<_>>()
            .join("\n");
        assert!(
            message.contains("can drop 65 roots (62 fixed plus 3 repeatable)"),
            "{message}"
        );
        assert!(message.contains("maximum is 64"), "{message}");
    }

    #[test]
    fn repeatable_loot_collection_and_member_limits_fail_closed() {
        let base = pack_with_loot();
        let mut oversized = base.clone();
        oversized.gameplay.loot_tables =
            vec![base.gameplay.loot_tables[0].clone(); MAX_LOOT_TABLES + 1];
        oversized.gameplay.loot_sources =
            vec![base.gameplay.loot_sources[0].clone(); MAX_LOOT_SOURCES + 1];
        let message = oversized
            .validate()
            .unwrap_err()
            .0
            .into_iter()
            .map(|error| error.to_string())
            .collect::<Vec<_>>()
            .join("\n");
        assert!(message.contains("maximum is 256"), "{message}");
        assert!(message.contains("maximum is 1024"), "{message}");

        let mut invalid = pack_with_loot();
        let table = &mut invalid.gameplay.loot_tables[0];
        table.guaranteed = vec![table.guaranteed[0].clone(); MAX_LOOT_GUARANTEED_PER_TABLE + 1];
        table.pools = vec![table.pools[0].clone(); MAX_LOOT_POOLS_PER_TABLE + 1];
        table.pools[0].rolls = MAX_LOOT_ROLLS_PER_POOL + 1;
        table.pools[0].entries =
            vec![table.pools[0].entries[0].clone(); MAX_LOOT_ENTRIES_PER_POOL + 1];
        table.pools[0].entries[0].weight = MAX_LOOT_WEIGHT + 1;
        table.pools[0].entries[0].max_quantity = Some(MAX_LOOT_QUANTITY + 1);
        let message = invalid
            .validate()
            .unwrap_err()
            .0
            .into_iter()
            .map(|error| error.to_string())
            .collect::<Vec<_>>()
            .join("\n");
        for expected in [
            "maximum is 8",
            "rolls must be 1..=8",
            "1..=64 entries",
            "weight must be 1..=1000000",
            "within 1..=32",
        ] {
            assert!(
                message.contains(expected),
                "missing {expected:?}: {message}"
            );
        }
    }

    #[test]
    fn repeatable_loot_ids_accept_128_bytes_and_reject_129() {
        let boundary_id = "a".repeat(128);
        let mut boundary = pack_with_loot();
        boundary.gameplay.loot_sources[0].loot_table_id = boundary_id.clone();
        boundary.gameplay.loot_tables[0].id = boundary_id.clone();
        boundary.gameplay.loot_sources[0].id = "b".repeat(128);
        boundary.gameplay.loot_tables[0].guaranteed[0].id = "c".repeat(128);
        boundary.gameplay.loot_tables[0].pools[0].id = "d".repeat(128);
        boundary.gameplay.loot_tables[0].pools[0].entries[0].id = "e".repeat(128);
        boundary.gameplay.loot_tables[0].pools[0].entries[1].id = "f".repeat(128);
        boundary.validate().unwrap();

        boundary.gameplay.loot_tables[0].pools[0].entries[1].id = "f".repeat(129);
        let message = boundary
            .validate()
            .unwrap_err()
            .0
            .into_iter()
            .map(|error| error.to_string())
            .collect::<Vec<_>>()
            .join("\n");
        assert!(message.contains("1..=128 canonical"), "{message}");
    }

    #[test]
    fn repeatable_loot_limits_weighted_entries_globally() {
        let mut value = pack_with_loot();
        value.gameplay.npc_spawns.clear();
        value.gameplay.loot_tables.clear();
        value.gameplay.loot_sources.clear();
        for table_index in 0..9 {
            let spawn_id = format!("test:wolf-{table_index}");
            let table_id = format!("test:table-{table_index}");
            value.gameplay.npc_spawns.push(NpcSpawn {
                id: spawn_id.clone(),
                prototype_id: "test:wolf".into(),
                room_id: "test:1".into(),
            });
            value.gameplay.loot_tables.push(LootTableDefinition {
                id: table_id.clone(),
                guaranteed: Vec::new(),
                pools: (0..8)
                    .map(|pool_index| LootPoolDefinition {
                        id: format!("test:pool-{pool_index}"),
                        rolls: 1,
                        entries: (0..64)
                            .map(|entry_index| LootEntryDefinition {
                                kind: LootEntryKind::Nothing,
                                id: format!("test:entry-{pool_index}-{entry_index}"),
                                weight: 1,
                                item_prototype_id: None,
                                min_quantity: None,
                                max_quantity: None,
                                affix_pool_id: None,
                            })
                            .collect(),
                    })
                    .collect(),
            });
            value.gameplay.loot_sources.push(LootSourceDefinition {
                kind: LootSourceKind::NpcSpawn,
                id: format!("test:source-{table_index}"),
                npc_spawn_id: spawn_id,
                loot_table_id: table_id,
                rights: LootRightsPolicy::CreditedCharacter,
            });
        }
        let message = value
            .validate()
            .unwrap_err()
            .0
            .into_iter()
            .map(|error| error.to_string())
            .collect::<Vec<_>>()
            .join("\n");
        assert!(
            message.contains("declares 4608 weighted loot entries"),
            "{message}"
        );
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
    fn room_text_is_byte_bounded_multiline_and_control_safe() {
        let mut boundary = pack();
        boundary.rooms[0].title = "é".repeat(MAX_ROOM_TITLE_BYTES / 2);
        boundary.rooms[0].description =
            format!("{}\n\tok", "é".repeat((MAX_ROOM_DESCRIPTION_BYTES - 4) / 2));
        assert_eq!(boundary.rooms[0].title.len(), MAX_ROOM_TITLE_BYTES);
        assert_eq!(
            boundary.rooms[0].description.len(),
            MAX_ROOM_DESCRIPTION_BYTES
        );
        boundary.validate().unwrap();

        let invalid_cases = [
            (
                "oversized title",
                "é".repeat(MAX_ROOM_TITLE_BYTES / 2 + 1),
                "safe description".to_owned(),
                "title must contain 1..=128 safe bytes",
            ),
            (
                "oversized description",
                "safe title".to_owned(),
                "é".repeat(MAX_ROOM_DESCRIPTION_BYTES / 2 + 1),
                "description must contain 1..=8192 safe bytes",
            ),
            (
                "title control",
                "unsafe\u{1b}title".to_owned(),
                "safe description".to_owned(),
                "title must contain 1..=128 safe bytes",
            ),
            (
                "description control",
                "safe title".to_owned(),
                "safe line\nunsafe\u{0}line".to_owned(),
                "description must contain 1..=8192 safe bytes",
            ),
            (
                "empty multiline description",
                "safe title".to_owned(),
                " \n\t\r ".to_owned(),
                "description must contain 1..=8192 safe bytes",
            ),
        ];
        for (case, title, description, expected) in invalid_cases {
            let mut invalid = pack();
            invalid.rooms[0].title = title;
            invalid.rooms[0].description = description;
            let message = invalid
                .validate()
                .unwrap_err()
                .0
                .into_iter()
                .map(|error| error.to_string())
                .collect::<Vec<_>>()
                .join("\n");
            assert!(message.contains(expected), "{case}: {message}");
        }
    }

    #[test]
    fn legacy_room_text_bounds_survive_every_supported_normalization() {
        let current = serde_json::to_value(pack()).unwrap();
        for schema_version in [2, 3, 4, 5] {
            for (field, value, expected) in [
                (
                    "title",
                    "é".repeat(MAX_ROOM_TITLE_BYTES / 2 + 1),
                    "title must contain 1..=128 safe bytes",
                ),
                (
                    "description",
                    "d".repeat(MAX_ROOM_DESCRIPTION_BYTES + 1),
                    "description must contain 1..=8192 safe bytes",
                ),
            ] {
                let mut legacy = current.clone();
                legacy["schemaVersion"] = serde_json::json!(schema_version);
                legacy["rooms"][0][field] = serde_json::Value::String(value);
                let normalized = WorldPackDocument::from_json(
                    &serde_json::to_vec(&legacy).expect("legacy fixture serializes"),
                )
                .expect("supported legacy schema normalizes structurally");
                assert_eq!(normalized.world.schema_version, CURRENT_SCHEMA_VERSION);
                let message = normalized
                    .validate()
                    .unwrap_err()
                    .0
                    .into_iter()
                    .map(|error| error.to_string())
                    .collect::<Vec<_>>()
                    .join("\n");
                assert!(
                    message.contains(expected),
                    "WorldPack v{schema_version} field {field}: {message}"
                );
            }
        }
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

        for (suffix, transfer_policy) in [
            ("transferable", ItemTransferPolicy::Transferable),
            ("bound", ItemTransferPolicy::CharacterBound),
            ("non-transferable", ItemTransferPolicy::NonTransferable),
        ] {
            let mut candidate = pack();
            let mut prototype = item_prototype(&format!("test:{suffix}"));
            prototype.transfer_policy = transfer_policy;
            candidate.item_prototypes.push(prototype);
            candidate.validate().unwrap();

            let encoded = serde_json::to_vec(&candidate).unwrap();
            let decoded: WorldPack = serde_json::from_slice(&encoded).unwrap();
            assert_eq!(decoded.item_prototypes[0].transfer_policy, transfer_policy);
        }

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
    fn item_instance_ids_share_the_runtime_identifier_contract() {
        let prototype_id = "test:bounded-instance";
        for valid_id in [
            format!("i{}", "a".repeat(127)),
            "runtime:1".into(),
            "runtime:18446744073709551615".into(),
        ] {
            let mut candidate = pack();
            candidate.item_prototypes.push(item_prototype(prototype_id));
            candidate.initial_item_instances.push(InitialItemInstance {
                id: valid_id,
                prototype_id: prototype_id.into(),
                location: InitialItemLocation::Room {
                    room_id: "test:1".into(),
                },
            });
            candidate.validate().unwrap();
        }

        for invalid_id in [
            format!("i{}", "a".repeat(128)),
            "runtime:".into(),
            "runtime:0".into(),
            "runtime:01".into(),
            "runtime:-1".into(),
            "runtime:+1".into(),
            "runtime:18446744073709551616".into(),
        ] {
            let mut candidate = pack();
            candidate.item_prototypes.push(item_prototype(prototype_id));
            candidate.initial_item_instances.push(InitialItemInstance {
                id: invalid_id,
                prototype_id: prototype_id.into(),
                location: InitialItemLocation::Room {
                    room_id: "test:1".into(),
                },
            });
            let messages = candidate
                .validate()
                .unwrap_err()
                .0
                .into_iter()
                .map(|error| error.to_string())
                .collect::<Vec<_>>()
                .join("\n");
            assert!(messages.contains("canonical runtime id"), "{messages}");
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
