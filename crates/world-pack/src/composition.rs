//! Strict, bounded, offline composition of modular authoring areas.
//!
//! AreaPack v8 is deliberately an authoring format, not a runtime hot-load
//! protocol. Composition resolves and validates the complete graph, then emits
//! one validated [`WorldPackDocument`] v9 consumed by the runtime.
//! Area identities, source digests, ownership and cross-area edges are exposed
//! in [`CompositionReport`] so a later catalog/activation design can build on
//! stable facts without weakening today's all-or-nothing runtime validation.

use std::{
    collections::{HashMap, HashSet},
    fs,
    io::Read,
    path::{Component, Path, PathBuf},
};

use serde::{Deserialize, Deserializer, Serialize, de::Error as _};
use sha2::{Digest, Sha256};
use thiserror::Error;

use crate::{
    AbilityDefinition, AdvancedClassDefinition, AffixDefinition, AffixPoolDefinition,
    ArchetypeDefinition, BackgroundDefinition, BaseClassDefinition, CharacterDefaultsDefinition,
    CharacterProgressionDefinition, CommodityDefinition, ContentTest, EntityMediaDefinition,
    EntityMediaKind, ExitTarget, GameplayDefinition, GrantDefinition, InitialItemInstance,
    ItemPrototype, LevelThresholdDefinition, LineageDefinition, LootSourceDefinition,
    LootTableDefinition, NpcAbilitySet, NpcPrototype, NpcSpawn, QuestDefinition,
    RequirementDefinition, RequirementPredicate, Room, RoomMediaDefinition,
    RoomPerceptionDefinition, RoomSceneVariants, ScriptDefinition, ShopDefinition,
    SocialEmoteDefinition, SourceMetadata, TalentGraphDefinition, ValidationErrors, WorldPack,
    WorldPackDocument,
};

pub const WORLD_PROJECT_SCHEMA_VERSION: u32 = 2;
pub const AREA_PACK_SCHEMA_VERSION: u32 = 9;
pub const MAX_PROJECT_MANIFEST_BYTES: u64 = 1024 * 1024;
pub const MAX_AREA_COUNT: usize = 256;
pub const MAX_AREA_FILE_BYTES: u64 = 16 * 1024 * 1024;
pub const MAX_TOTAL_AREA_BYTES: u64 = 16 * 1024 * 1024;
pub const MAX_COMPOSED_WORLD_BYTES: usize = 16 * 1024 * 1024;

/// Project-level identity and the complete set of offline authoring modules.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WorldProjectManifest {
    pub schema_version: u32,
    pub id: String,
    pub name: String,
    pub start_room_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rng_seed: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default_lineage_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default_background_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default_base_class_id: Option<String>,
    pub areas: Vec<ProjectArea>,
}

impl<'de> Deserialize<'de> for WorldProjectManifest {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        #[derive(Deserialize)]
        #[serde(rename_all = "camelCase", deny_unknown_fields)]
        struct Raw {
            schema_version: u32,
            id: String,
            name: String,
            start_room_id: String,
            #[serde(default)]
            rng_seed: Option<u64>,
            #[serde(default)]
            default_archetype_id: Option<String>,
            #[serde(default)]
            default_lineage_id: Option<String>,
            #[serde(default)]
            default_background_id: Option<String>,
            #[serde(default)]
            default_base_class_id: Option<String>,
            areas: Vec<ProjectArea>,
        }
        let raw = Raw::deserialize(deserializer)?;
        if raw.schema_version != 1 && raw.schema_version != WORLD_PROJECT_SCHEMA_VERSION {
            return Err(D::Error::custom(format!(
                "unsupported project schema {}",
                raw.schema_version
            )));
        }
        let legacy = raw.schema_version == 1;
        Ok(Self {
            schema_version: WORLD_PROJECT_SCHEMA_VERSION,
            id: raw.id,
            name: raw.name,
            start_room_id: raw.start_room_id,
            rng_seed: raw.rng_seed,
            default_lineage_id: raw
                .default_lineage_id
                .or_else(|| legacy.then(|| "legacy:mortal".to_owned())),
            default_background_id: raw.default_background_id,
            default_base_class_id: raw.default_base_class_id.or(raw.default_archetype_id),
            areas: raw.areas,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ProjectArea {
    pub id: String,
    pub path: String,
}

/// One independently owned authoring module.
///
/// `dependsOn` declares every other area referenced by this area's structured
/// records. Dependencies may be cyclic: mutual room exits are a normal case.
/// Lua source remains opaque and cannot create a verified structured edge.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AreaPack {
    pub schema_version: u32,
    pub id: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub depends_on: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub item_prototypes: Vec<ItemPrototype>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub initial_item_instances: Vec<InitialItemInstance>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub rooms: Vec<Room>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub npc_prototypes: Vec<NpcPrototype>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub npc_spawns: Vec<NpcSpawn>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub loot_tables: Vec<LootTableDefinition>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub loot_sources: Vec<LootSourceDefinition>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub affixes: Vec<AffixDefinition>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub affix_pools: Vec<AffixPoolDefinition>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub archetypes: Vec<ArchetypeDefinition>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub level_thresholds: Vec<LevelThresholdDefinition>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub lineages: Vec<LineageDefinition>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub backgrounds: Vec<BackgroundDefinition>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub base_classes: Vec<BaseClassDefinition>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub advanced_classes: Vec<AdvancedClassDefinition>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub talent_graphs: Vec<TalentGraphDefinition>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub abilities: Vec<AbilityDefinition>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub social_emotes: Vec<SocialEmoteDefinition>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub commodities: Vec<CommodityDefinition>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub quests: Vec<QuestDefinition>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub shops: Vec<ShopDefinition>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub npc_ability_sets: Vec<NpcAbilitySet>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub room_media: Vec<RoomMediaDefinition>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub entity_media: Vec<EntityMediaDefinition>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub room_perception: Vec<RoomPerceptionDefinition>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub scene_variants: Vec<RoomSceneVariants>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub scripts: Vec<ScriptDefinition>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub content_tests: Vec<ContentTest>,
}

fn normalize_legacy_area_items(value: &mut serde_json::Value) -> Result<(), String> {
    let root = value
        .as_object_mut()
        .ok_or_else(|| "AreaPack root must be an object".to_owned())?;
    let mut gameplay = serde_json::Map::new();
    for (area_key, gameplay_key) in [
        ("itemRules", "itemRules"),
        ("npcPrototypes", "npcPrototypes"),
        ("npcSpawns", "npcSpawns"),
    ] {
        if let Some(field) = root.remove(area_key) {
            gameplay.insert(gameplay_key.to_owned(), field);
        }
    }
    root.insert("gameplay".to_owned(), serde_json::Value::Object(gameplay));
    crate::authoring::normalize_legacy_items(value)?;
    let root = value
        .as_object_mut()
        .expect("legacy normalizer preserves the AreaPack root object");
    let gameplay = root
        .remove("gameplay")
        .and_then(|value| value.as_object().cloned())
        .ok_or_else(|| "legacy item normalization lost gameplay fields".to_owned())?;
    for key in ["npcPrototypes", "npcSpawns"] {
        if let Some(field) = gameplay.get(key) {
            root.insert(key.to_owned(), field.clone());
        }
    }
    Ok(())
}

impl<'de> Deserialize<'de> for AreaPack {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        #[derive(Deserialize)]
        #[serde(rename_all = "camelCase", deny_unknown_fields)]
        struct RawAreaPack {
            schema_version: u32,
            id: String,
            #[serde(default)]
            depends_on: Vec<String>,
            #[serde(default)]
            item_prototypes: Vec<ItemPrototype>,
            #[serde(default)]
            initial_item_instances: Vec<InitialItemInstance>,
            #[serde(default)]
            rooms: Vec<Room>,
            #[serde(default)]
            npc_prototypes: Vec<NpcPrototype>,
            #[serde(default)]
            npc_spawns: Vec<NpcSpawn>,
            #[serde(default)]
            loot_tables: Vec<LootTableDefinition>,
            #[serde(default)]
            loot_sources: Vec<LootSourceDefinition>,
            #[serde(default)]
            affixes: Vec<AffixDefinition>,
            #[serde(default)]
            affix_pools: Vec<AffixPoolDefinition>,
            #[serde(default)]
            archetypes: Vec<ArchetypeDefinition>,
            #[serde(default)]
            level_thresholds: Vec<LevelThresholdDefinition>,
            #[serde(default)]
            lineages: Vec<LineageDefinition>,
            #[serde(default)]
            backgrounds: Vec<BackgroundDefinition>,
            #[serde(default)]
            base_classes: Vec<BaseClassDefinition>,
            #[serde(default)]
            advanced_classes: Vec<AdvancedClassDefinition>,
            #[serde(default)]
            talent_graphs: Vec<TalentGraphDefinition>,
            #[serde(default)]
            abilities: Vec<AbilityDefinition>,
            #[serde(default)]
            social_emotes: Vec<SocialEmoteDefinition>,
            #[serde(default)]
            commodities: Vec<CommodityDefinition>,
            #[serde(default)]
            quests: Vec<QuestDefinition>,
            #[serde(default)]
            shops: Vec<ShopDefinition>,
            #[serde(default)]
            npc_ability_sets: Vec<NpcAbilitySet>,
            #[serde(default)]
            room_media: Vec<RoomMediaDefinition>,
            #[serde(default)]
            entity_media: Vec<EntityMediaDefinition>,
            #[serde(default)]
            room_perception: Vec<RoomPerceptionDefinition>,
            #[serde(default)]
            scene_variants: Vec<RoomSceneVariants>,
            #[serde(default)]
            scripts: Vec<ScriptDefinition>,
            #[serde(default)]
            content_tests: Vec<ContentTest>,
        }

        let mut value = serde_json::Value::deserialize(deserializer)?;
        audit_area_pack_keys(&value).map_err(D::Error::custom)?;
        let schema_version = value
            .get("schemaVersion")
            .and_then(serde_json::Value::as_u64)
            .ok_or_else(|| D::Error::custom("AreaPack schemaVersion is required"))?;
        if (1..=6).contains(&schema_version) {
            reject_legacy_area_communication_fields(&value, schema_version)
                .map_err(D::Error::custom)?;
        }
        if (1..=7).contains(&schema_version) {
            reject_legacy_area_social_emote_fields(&value, schema_version)
                .map_err(D::Error::custom)?;
        }
        if (1..u64::from(AREA_PACK_SCHEMA_VERSION)).contains(&schema_version) {
            reject_legacy_area_adventure_fields(&value, schema_version)
                .map_err(D::Error::custom)?;
        }
        if schema_version == 1 || schema_version == 2 {
            reject_legacy_area_loot_fields(&value, schema_version).map_err(D::Error::custom)?;
            reject_legacy_area_reward_fields(&value, schema_version).map_err(D::Error::custom)?;
            normalize_legacy_area_items(&mut value).map_err(D::Error::custom)?;
        } else if schema_version == 3 {
            reject_legacy_area_loot_fields(&value, schema_version).map_err(D::Error::custom)?;
            reject_legacy_area_reward_fields(&value, schema_version).map_err(D::Error::custom)?;
            normalize_v3_area_items(&mut value).map_err(D::Error::custom)?;
        } else if schema_version == 4 {
            reject_legacy_area_loot_fields(&value, schema_version).map_err(D::Error::custom)?;
            reject_legacy_area_reward_fields(&value, schema_version).map_err(D::Error::custom)?;
            normalize_v4_area_loot(&mut value).map_err(D::Error::custom)?;
        } else if schema_version == 5 {
            reject_legacy_area_reward_fields(&value, schema_version).map_err(D::Error::custom)?;
            normalize_v5_area_rewards(&mut value).map_err(D::Error::custom)?;
        } else if schema_version == 6 || schema_version == 7 {
            normalize_v5_area_rewards(&mut value).map_err(D::Error::custom)?;
        } else if schema_version == 8 || schema_version == u64::from(AREA_PACK_SCHEMA_VERSION) {
            reject_native_area_legacy_item_fields(&value).map_err(D::Error::custom)?;
        }
        let raw: RawAreaPack = serde_json::from_value(value).map_err(D::Error::custom)?;
        if raw.schema_version != 1
            && raw.schema_version != 2
            && raw.schema_version != 3
            && raw.schema_version != 4
            && raw.schema_version != 5
            && raw.schema_version != 6
            && raw.schema_version != 7
            && raw.schema_version != 8
            && raw.schema_version != AREA_PACK_SCHEMA_VERSION
        {
            return Err(D::Error::custom(format!(
                "unsupported area schema {}",
                raw.schema_version
            )));
        }
        let mut level_thresholds = raw.level_thresholds;
        let mut lineages = raw.lineages;
        let mut base_classes = raw.base_classes;
        if raw.schema_version <= 2 && !raw.archetypes.is_empty() {
            let imported = crate::progression::normalize_v2_character_progression(
                raw.archetypes.clone(),
                None,
            )
            .map_err(D::Error::custom)?
            .expect("non-empty archetypes produce progression");
            level_thresholds.extend(imported.level_thresholds);
            lineages.extend(imported.lineages);
            base_classes.extend(imported.base_classes);
        }
        let archetypes = if raw.schema_version <= 2 {
            Vec::new()
        } else {
            raw.archetypes
        };
        Ok(Self {
            schema_version: AREA_PACK_SCHEMA_VERSION,
            id: raw.id,
            depends_on: raw.depends_on,
            item_prototypes: raw.item_prototypes,
            initial_item_instances: raw.initial_item_instances,
            rooms: raw.rooms,
            npc_prototypes: raw.npc_prototypes,
            npc_spawns: raw.npc_spawns,
            loot_tables: raw.loot_tables,
            loot_sources: raw.loot_sources,
            affixes: raw.affixes,
            affix_pools: raw.affix_pools,
            archetypes,
            level_thresholds,
            lineages,
            backgrounds: raw.backgrounds,
            base_classes,
            advanced_classes: raw.advanced_classes,
            talent_graphs: raw.talent_graphs,
            abilities: raw.abilities,
            social_emotes: raw.social_emotes,
            commodities: raw.commodities,
            quests: raw.quests,
            shops: raw.shops,
            npc_ability_sets: raw.npc_ability_sets,
            room_media: raw.room_media,
            entity_media: raw.entity_media,
            room_perception: raw.room_perception,
            scene_variants: raw.scene_variants,
            scripts: raw.scripts,
            content_tests: raw.content_tests,
        })
    }
}

fn reject_native_area_legacy_item_fields(value: &serde_json::Value) -> Result<(), String> {
    let Some(root) = value.as_object() else {
        return Ok(());
    };
    if root.contains_key("items") || root.contains_key("itemRules") {
        return Err("AreaPack v8 must use itemPrototypes and initialItemInstances".to_owned());
    }
    if root
        .get("rooms")
        .and_then(serde_json::Value::as_array)
        .is_some_and(|rooms| rooms.iter().any(|room| room.get("items").is_some()))
    {
        return Err("AreaPack v8 rooms cannot declare legacy items".to_owned());
    }
    if root
        .get("npcPrototypes")
        .and_then(serde_json::Value::as_array)
        .is_some_and(|prototypes| {
            prototypes
                .iter()
                .any(|prototype| prototype.get("lootItemIds").is_some())
        })
    {
        return Err("AreaPack v8 NPC prototypes cannot declare legacy lootItemIds".to_owned());
    }
    Ok(())
}

fn reject_legacy_area_loot_fields(
    value: &serde_json::Value,
    schema_version: u64,
) -> Result<(), String> {
    if value.get("lootTables").is_some() || value.get("lootSources").is_some() {
        return Err(format!(
            "AreaPack v{schema_version} cannot declare lootTables or lootSources"
        ));
    }
    Ok(())
}

fn reject_legacy_area_reward_fields(
    value: &serde_json::Value,
    schema_version: u64,
) -> Result<(), String> {
    let has_sealed = ["itemPrototypes", "items"].into_iter().any(|field| {
        value
            .get(field)
            .and_then(serde_json::Value::as_array)
            .is_some_and(|prototypes| {
                prototypes
                    .iter()
                    .any(|prototype| prototype.get("sealedLootTableId").is_some())
            })
    });
    let has_affix_pool_reference = value
        .get("lootTables")
        .and_then(serde_json::Value::as_array)
        .is_some_and(|tables| {
            tables.iter().any(|table| {
                table
                    .get("guaranteed")
                    .and_then(serde_json::Value::as_array)
                    .is_some_and(|entries| {
                        entries
                            .iter()
                            .any(|entry| entry.get("affixPoolId").is_some())
                    })
                    || table
                        .get("pools")
                        .and_then(serde_json::Value::as_array)
                        .is_some_and(|pools| {
                            pools.iter().any(|pool| {
                                pool.get("entries")
                                    .and_then(serde_json::Value::as_array)
                                    .is_some_and(|entries| {
                                        entries
                                            .iter()
                                            .any(|entry| entry.get("affixPoolId").is_some())
                                    })
                            })
                        })
            })
        });
    if value.get("affixes").is_some()
        || value.get("affixPools").is_some()
        || has_sealed
        || has_affix_pool_reference
    {
        return Err(format!(
            "AreaPack v{schema_version} cannot declare v6 sealed rewards or affixes"
        ));
    }
    Ok(())
}

fn reject_legacy_area_communication_fields(
    value: &serde_json::Value,
    schema_version: u64,
) -> Result<(), String> {
    let has_communication = value
        .get("abilities")
        .and_then(serde_json::Value::as_array)
        .is_some_and(|abilities| {
            abilities
                .iter()
                .any(|ability| ability.get("communication").is_some())
        });
    if has_communication {
        return Err(format!(
            "AreaPack v{schema_version} cannot declare v7 communication abilities"
        ));
    }
    Ok(())
}

fn reject_legacy_area_adventure_fields(
    value: &serde_json::Value,
    schema_version: u64,
) -> Result<(), String> {
    if ["commodities", "quests", "shops"]
        .into_iter()
        .any(|field| value.get(field).is_some())
    {
        return Err(format!(
            "AreaPack v{schema_version} cannot declare v9 commodities, quests or shops"
        ));
    }
    Ok(())
}

fn reject_legacy_area_social_emote_fields(
    value: &serde_json::Value,
    schema_version: u64,
) -> Result<(), String> {
    if value.get("socialEmotes").is_some() {
        return Err(format!(
            "AreaPack v{schema_version} cannot declare v8 socialEmotes"
        ));
    }
    Ok(())
}

fn normalize_v4_area_loot(value: &mut serde_json::Value) -> Result<(), String> {
    reject_native_area_legacy_item_fields(value)?;
    let root = value
        .as_object_mut()
        .ok_or_else(|| "AreaPack root must be an object".to_owned())?;
    root.insert(
        "schemaVersion".to_owned(),
        serde_json::json!(AREA_PACK_SCHEMA_VERSION),
    );
    Ok(())
}

fn normalize_v5_area_rewards(value: &mut serde_json::Value) -> Result<(), String> {
    reject_native_area_legacy_item_fields(value)?;
    let root = value
        .as_object_mut()
        .ok_or_else(|| "AreaPack root must be an object".to_owned())?;
    root.insert(
        "schemaVersion".to_owned(),
        serde_json::json!(AREA_PACK_SCHEMA_VERSION),
    );
    Ok(())
}

fn normalize_v3_area_items(value: &mut serde_json::Value) -> Result<(), String> {
    reject_native_area_legacy_item_fields(value)?;
    let root = value
        .as_object_mut()
        .ok_or_else(|| "AreaPack root must be an object".to_owned())?;
    let prototypes = root
        .entry("itemPrototypes")
        .or_insert_with(|| serde_json::Value::Array(Vec::new()))
        .as_array_mut()
        .ok_or_else(|| "AreaPack v3 itemPrototypes must be an array".to_owned())?;
    for prototype in prototypes {
        let prototype = prototype
            .as_object_mut()
            .ok_or_else(|| "AreaPack v3 item prototype must be an object".to_owned())?;
        if prototype.contains_key("maxStackUnits") || prototype.contains_key("container") {
            return Err(
                "AreaPack v3 cannot declare v4 maxStackUnits or container fields".to_owned(),
            );
        }
        if prototype
            .get("stackPolicy")
            .and_then(serde_json::Value::as_str)
            != Some("unique")
        {
            return Err("AreaPack v3 item prototypes must use stackPolicy unique".to_owned());
        }
        if prototype
            .get("transferPolicy")
            .and_then(serde_json::Value::as_str)
            != Some("transferable")
        {
            return Err(
                "AreaPack v3 item prototypes must use transferPolicy transferable".to_owned(),
            );
        }
        prototype.insert("maxStackUnits".to_owned(), serde_json::json!(1));
    }
    let instances = root
        .entry("initialItemInstances")
        .or_insert_with(|| serde_json::Value::Array(Vec::new()))
        .as_array()
        .ok_or_else(|| "AreaPack v3 initialItemInstances must be an array".to_owned())?;
    if instances.iter().any(|instance| {
        instance.get("location").is_some_and(|location| {
            location.get("kind").and_then(serde_json::Value::as_str) == Some("container")
                || location.get("containerInstanceId").is_some()
        })
    }) {
        return Err("AreaPack v3 cannot declare v4 container locations".to_owned());
    }
    root.insert(
        "schemaVersion".to_owned(),
        serde_json::json!(AREA_PACK_SCHEMA_VERSION),
    );
    Ok(())
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AreaOwnership {
    pub rooms: Vec<String>,
    pub item_prototypes: Vec<String>,
    pub initial_item_instances: Vec<String>,
    pub npc_prototypes: Vec<String>,
    pub npc_spawns: Vec<String>,
    pub loot_tables: Vec<String>,
    pub loot_sources: Vec<String>,
    pub affixes: Vec<String>,
    pub affix_pools: Vec<String>,
    pub archetypes: Vec<String>,
    pub lineages: Vec<String>,
    pub backgrounds: Vec<String>,
    pub base_classes: Vec<String>,
    pub advanced_classes: Vec<String>,
    pub talent_graphs: Vec<String>,
    pub talent_nodes: Vec<String>,
    pub abilities: Vec<String>,
    pub social_emotes: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub commodities: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub quests: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub shops: Vec<String>,
    pub npc_ability_sets: Vec<String>,
    pub room_media: Vec<String>,
    pub entity_media: Vec<String>,
    pub room_perception: Vec<String>,
    pub scene_variant_sets: Vec<String>,
    pub scene_variants: Vec<String>,
    pub scripts: Vec<String>,
    pub content_tests: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AreaComposition {
    pub id: String,
    /// SHA-256 of the exact reviewed AreaPack source bytes.
    pub source_sha256: String,
    pub depends_on: Vec<String>,
    pub asset_manifests: Vec<AreaAssetManifestDigest>,
    pub ownership: AreaOwnership,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AreaAssetManifestDigest {
    pub path: String,
    pub sha256: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum AreaCrossReferenceKind {
    RoomExit,
    ItemInstancePrototype,
    ItemInstanceRoom,
    ItemInstanceNpcSpawn,
    ItemInstanceContainer,
    NpcSpawnPrototype,
    NpcSpawnRoom,
    LootGuaranteedPrototype,
    LootGuaranteedAffixPool,
    LootEntryPrototype,
    LootEntryAffixPool,
    LootSourceNpcSpawn,
    LootSourceTable,
    ItemSealedLootTable,
    AffixPoolAffix,
    ArchetypeAbility,
    AbilityBlockedRoom,
    ProgressionReference,
    NpcAbilitySetPrototype,
    NpcAbility,
    RoomMedia,
    EntityMedia,
    RoomPerception,
    HiddenItem,
    HiddenNpc,
    SceneVariants,
    ScriptRoom,
    ContentTestRoom,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AreaCrossReference {
    pub from_area: String,
    pub to_area: String,
    pub kind: AreaCrossReferenceKind,
    pub source_id: String,
    pub target_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CompositionReport {
    pub project_id: String,
    /// SHA-256 of a canonical manifest representation; area entry order does not affect it.
    pub project_manifest_sha256: String,
    pub artifact_sha256: String,
    pub artifact_bytes: usize,
    pub areas: Vec<AreaComposition>,
    pub cross_area_references: Vec<AreaCrossReference>,
}

#[derive(Debug, Clone)]
pub struct ComposedWorld {
    pub document: WorldPackDocument,
    pub artifact: Vec<u8>,
    pub report: CompositionReport,
    /// Canonical AreaPack v8 inputs, ordered by stable area id. This remains an
    /// offline authoring surface and is never consumed by the game runtime.
    pub area_packs: Vec<AreaPack>,
}

#[derive(Debug, Error)]
pub enum CompositionError {
    #[error("invalid character progression project default: {0}")]
    InvalidCharacterProgressionDefault(String),
    #[error("could not inspect {path:?}: {source}")]
    Inspect {
        path: PathBuf,
        source: std::io::Error,
    },
    #[error("{kind} {path:?} contains {size} bytes; the limit is {maximum} bytes")]
    FileTooLarge {
        kind: &'static str,
        path: PathBuf,
        size: u64,
        maximum: u64,
    },
    #[error("could not read {path:?}: {source}")]
    Read {
        path: PathBuf,
        source: std::io::Error,
    },
    #[error("{kind} {path:?} is not a regular file")]
    NotRegularFile { kind: &'static str, path: PathBuf },
    #[error("invalid {kind} JSON in {path:?}: {source}")]
    Json {
        kind: &'static str,
        path: PathBuf,
        source: serde_json::Error,
    },
    #[error("area pack contains unknown field {field:?} at {json_path}")]
    UnknownAreaField { json_path: String, field: String },
    #[error("project schema version {found} is unsupported; this build supports {supported}")]
    UnsupportedProjectSchema { found: u32, supported: u32 },
    #[error(
        "area {area_id:?} schema version {found} is unsupported; this build supports {supported}"
    )]
    UnsupportedAreaSchema {
        area_id: String,
        found: u32,
        supported: u32,
    },
    #[error("project must declare between 1 and {maximum} areas (found {found})")]
    InvalidAreaCount { found: usize, maximum: usize },
    #[error("area id must contain 1..=256 safe bytes")]
    InvalidAreaId,
    #[error("area id {0:?} appears more than once in the project")]
    DuplicateAreaId(String),
    #[error("area path {0:?} appears more than once in the project")]
    DuplicateAreaPath(PathBuf),
    #[error("area {area_id:?} has unsafe relative path {path:?}")]
    UnsafeAreaPath { area_id: String, path: String },
    #[error("area {area_id:?} path {path:?} escapes project root {project_root:?}")]
    AreaPathEscape {
        area_id: String,
        path: PathBuf,
        project_root: PathBuf,
    },
    #[error("area {area_id:?} path {path:?} is not a regular file")]
    AreaNotFile { area_id: String, path: PathBuf },
    #[error("area file {path:?} declares id {found:?}, expected {expected:?}")]
    AreaIdMismatch {
        path: PathBuf,
        expected: String,
        found: String,
    },
    #[error("area {area_id:?} repeats dependency {dependency:?}")]
    DuplicateDependency { area_id: String, dependency: String },
    #[error("area {area_id:?} cannot depend on itself")]
    SelfDependency { area_id: String },
    #[error("area {area_id:?} depends on missing area {dependency:?}")]
    MissingDependency { area_id: String, dependency: String },
    #[error("area sources contain {size} bytes in total; the limit is {maximum} bytes")]
    TotalAreaBytesTooLarge { size: u64, maximum: u64 },
    #[error(
        "global entity id {id:?} is shared by {first_kind} in area {first_area:?} and {second_kind} in area {second_area:?}"
    )]
    DuplicateGlobalEntityId {
        id: String,
        first_kind: &'static str,
        first_area: String,
        second_kind: &'static str,
        second_area: String,
    },
    #[error("composed WorldPack is invalid: {0}")]
    InvalidWorld(#[from] ValidationErrors),
    #[error("could not serialize composed WorldPack: {0}")]
    Artifact(serde_json::Error),
    #[error("composed WorldPack contains {size} bytes; the limit is {maximum} bytes")]
    ComposedWorldTooLarge { size: usize, maximum: usize },
    #[error(
        "area {area_id:?} references area {target_area:?} via {kind:?} ({source_id:?} -> {target_id:?}) but does not list it in dependsOn"
    )]
    UndeclaredDependency {
        area_id: String,
        target_area: String,
        kind: AreaCrossReferenceKind,
        source_id: String,
        target_id: String,
    },
    #[error("asset path {path:?} is unsafe or escapes the project asset roots")]
    UnsafeAssetPath { path: String },
    #[error("asset {path:?} does not exist as a regular file under the project asset roots")]
    MissingAsset { path: String },
    #[error("image manifest {path:?} is invalid JSON: {source}")]
    ImageManifestJson {
        path: PathBuf,
        source: serde_json::Error,
    },
    #[error("image manifest {path:?} alt does not exactly match reviewed English imageAlt")]
    ImageAltMismatch { path: PathBuf },
}

#[derive(Debug)]
struct LoadedArea {
    source_sha256: String,
    pack: AreaPack,
}

/// Compose and validate a project without writing any output.
pub fn compose_world_project(
    manifest_path: impl AsRef<Path>,
) -> Result<ComposedWorld, CompositionError> {
    let manifest_path = manifest_path.as_ref();
    let canonical_manifest =
        fs::canonicalize(manifest_path).map_err(|source| CompositionError::Inspect {
            path: manifest_path.to_owned(),
            source,
        })?;
    let manifest_dir = canonical_manifest
        .parent()
        .unwrap_or_else(|| Path::new("/"))
        .to_owned();
    let manifest_bytes = read_bounded(
        &canonical_manifest,
        MAX_PROJECT_MANIFEST_BYTES,
        "project manifest",
    )?;
    let manifest: WorldProjectManifest =
        serde_json::from_slice(&manifest_bytes).map_err(|source| CompositionError::Json {
            kind: "project manifest",
            path: canonical_manifest.clone(),
            source,
        })?;
    validate_manifest(&manifest)?;

    let mut entries = manifest.areas.clone();
    entries.sort_by(|left, right| left.id.cmp(&right.id).then(left.path.cmp(&right.path)));
    let mut canonical_paths = HashSet::new();
    let mut total_area_bytes = 0_u64;
    let mut loaded = Vec::with_capacity(entries.len());
    for entry in entries {
        if !safe_relative_path(&entry.path) {
            return Err(CompositionError::UnsafeAreaPath {
                area_id: entry.id,
                path: entry.path,
            });
        }
        let requested_path = manifest_dir.join(&entry.path);
        let canonical_path =
            fs::canonicalize(&requested_path).map_err(|source| CompositionError::Inspect {
                path: requested_path.clone(),
                source,
            })?;
        if !canonical_path.starts_with(&manifest_dir) {
            return Err(CompositionError::AreaPathEscape {
                area_id: entry.id,
                path: canonical_path,
                project_root: manifest_dir,
            });
        }
        if !canonical_paths.insert(canonical_path.clone()) {
            return Err(CompositionError::DuplicateAreaPath(canonical_path));
        }
        let metadata =
            fs::metadata(&canonical_path).map_err(|source| CompositionError::Inspect {
                path: canonical_path.clone(),
                source,
            })?;
        if !metadata.is_file() {
            return Err(CompositionError::AreaNotFile {
                area_id: entry.id,
                path: canonical_path,
            });
        }
        let bytes = read_bounded(&canonical_path, MAX_AREA_FILE_BYTES, "area file")?;
        total_area_bytes = total_area_bytes.saturating_add(bytes.len() as u64);
        if total_area_bytes > MAX_TOTAL_AREA_BYTES {
            return Err(CompositionError::TotalAreaBytesTooLarge {
                size: total_area_bytes,
                maximum: MAX_TOTAL_AREA_BYTES,
            });
        }
        let area_value: serde_json::Value =
            serde_json::from_slice(&bytes).map_err(|source| CompositionError::Json {
                kind: "area pack",
                path: canonical_path.clone(),
                source,
            })?;
        audit_area_pack_keys(&area_value)?;
        let pack: AreaPack =
            serde_json::from_value(area_value).map_err(|source| CompositionError::Json {
                kind: "area pack",
                path: canonical_path.clone(),
                source,
            })?;
        if pack.schema_version != AREA_PACK_SCHEMA_VERSION {
            return Err(CompositionError::UnsupportedAreaSchema {
                area_id: entry.id,
                found: pack.schema_version,
                supported: AREA_PACK_SCHEMA_VERSION,
            });
        }
        if pack.id != entry.id {
            return Err(CompositionError::AreaIdMismatch {
                path: canonical_path,
                expected: entry.id,
                found: pack.id,
            });
        }
        loaded.push(LoadedArea {
            source_sha256: sha256_hex(&bytes),
            pack,
        });
    }

    validate_dependencies(&loaded)?;
    compose_loaded(manifest, loaded, &canonical_manifest, &manifest_dir)
}

fn validate_manifest(manifest: &WorldProjectManifest) -> Result<(), CompositionError> {
    if manifest.schema_version != WORLD_PROJECT_SCHEMA_VERSION {
        return Err(CompositionError::UnsupportedProjectSchema {
            found: manifest.schema_version,
            supported: WORLD_PROJECT_SCHEMA_VERSION,
        });
    }
    if manifest.areas.is_empty() || manifest.areas.len() > MAX_AREA_COUNT {
        return Err(CompositionError::InvalidAreaCount {
            found: manifest.areas.len(),
            maximum: MAX_AREA_COUNT,
        });
    }
    let mut ids = HashSet::new();
    let mut lexical_paths = HashSet::new();
    for area in &manifest.areas {
        if !safe_identifier(&area.id) {
            return Err(CompositionError::InvalidAreaId);
        }
        if !ids.insert(area.id.as_str()) {
            return Err(CompositionError::DuplicateAreaId(area.id.clone()));
        }
        if !lexical_paths.insert(area.path.as_str()) {
            return Err(CompositionError::DuplicateAreaPath(PathBuf::from(
                &area.path,
            )));
        }
    }
    Ok(())
}

fn validate_dependencies(areas: &[LoadedArea]) -> Result<(), CompositionError> {
    let area_ids = areas
        .iter()
        .map(|area| area.pack.id.as_str())
        .collect::<HashSet<_>>();
    for area in areas {
        let mut dependencies = HashSet::new();
        for dependency in &area.pack.depends_on {
            if !dependencies.insert(dependency.as_str()) {
                return Err(CompositionError::DuplicateDependency {
                    area_id: area.pack.id.clone(),
                    dependency: dependency.clone(),
                });
            }
            if dependency == &area.pack.id {
                return Err(CompositionError::SelfDependency {
                    area_id: area.pack.id.clone(),
                });
            }
            if !area_ids.contains(dependency.as_str()) {
                return Err(CompositionError::MissingDependency {
                    area_id: area.pack.id.clone(),
                    dependency: dependency.clone(),
                });
            }
        }
    }
    Ok(())
}

fn compose_loaded(
    manifest: WorldProjectManifest,
    areas: Vec<LoadedArea>,
    manifest_path: &Path,
    manifest_dir: &Path,
) -> Result<ComposedWorld, CompositionError> {
    let project_manifest_sha256 = canonical_project_digest(&manifest)?;
    let mut document = WorldPackDocument {
        world: WorldPack {
            schema_version: crate::CURRENT_SCHEMA_VERSION,
            id: manifest.id.clone(),
            name: manifest.name,
            start_room_id: manifest.start_room_id,
            source: Some(SourceMetadata {
                format: "world-pack".to_owned(),
                dialect: None,
                file: None,
            }),
            item_prototypes: Vec::new(),
            initial_item_instances: Vec::new(),
            rooms: Vec::new(),
            gameplay: GameplayDefinition {
                rng_seed: manifest.rng_seed,
                character_progression: None,
                ..GameplayDefinition::default()
            },
        },
        room_media: Vec::new(),
        entity_media: Vec::new(),
        room_perception: Vec::new(),
        scene_variants: Vec::new(),
        scripts: Vec::new(),
        content_tests: Vec::new(),
    };
    let mut global_ids: HashMap<String, Vec<(&'static str, String)>> = HashMap::new();
    let mut ownership = Vec::with_capacity(areas.len());

    for area in &areas {
        let pack = &area.pack;
        register_entities(
            &mut global_ids,
            "room",
            &pack.id,
            pack.rooms.iter().map(|v| &v.id),
        )?;
        register_entities(
            &mut global_ids,
            "item prototype",
            &pack.id,
            pack.item_prototypes.iter().map(|v| &v.id),
        )?;
        register_entities(
            &mut global_ids,
            "initial item instance",
            &pack.id,
            pack.initial_item_instances.iter().map(|v| &v.id),
        )?;
        register_entities(
            &mut global_ids,
            "NPC prototype",
            &pack.id,
            pack.npc_prototypes.iter().map(|v| &v.id),
        )?;
        register_entities(
            &mut global_ids,
            "NPC spawn",
            &pack.id,
            pack.npc_spawns.iter().map(|v| &v.id),
        )?;
        register_entities(
            &mut global_ids,
            "loot table",
            &pack.id,
            pack.loot_tables.iter().map(|value| &value.id),
        )?;
        register_entities(
            &mut global_ids,
            "loot source",
            &pack.id,
            pack.loot_sources.iter().map(|value| &value.id),
        )?;
        register_entities(
            &mut global_ids,
            "affix",
            &pack.id,
            pack.affixes.iter().map(|value| &value.id),
        )?;
        register_entities(
            &mut global_ids,
            "affix pool",
            &pack.id,
            pack.affix_pools.iter().map(|value| &value.id),
        )?;
        register_entities(
            &mut global_ids,
            "archetype",
            &pack.id,
            pack.archetypes.iter().map(|v| &v.id),
        )?;
        register_entities(
            &mut global_ids,
            "lineage",
            &pack.id,
            pack.lineages.iter().map(|v| &v.id),
        )?;
        register_entities(
            &mut global_ids,
            "background",
            &pack.id,
            pack.backgrounds.iter().map(|v| &v.id),
        )?;
        register_entities(
            &mut global_ids,
            "base class",
            &pack.id,
            pack.base_classes.iter().map(|v| &v.id),
        )?;
        register_entities(
            &mut global_ids,
            "advanced class",
            &pack.id,
            pack.advanced_classes.iter().map(|v| &v.id),
        )?;
        register_entities(
            &mut global_ids,
            "talent graph",
            &pack.id,
            pack.talent_graphs.iter().map(|v| &v.id),
        )?;
        register_entities(
            &mut global_ids,
            "talent node",
            &pack.id,
            pack.talent_graphs
                .iter()
                .flat_map(|graph| graph.nodes.iter().map(|node| &node.id)),
        )?;
        register_entities(
            &mut global_ids,
            "ability",
            &pack.id,
            pack.abilities.iter().map(|v| &v.id),
        )?;
        register_entities(
            &mut global_ids,
            "social emote",
            &pack.id,
            pack.social_emotes.iter().map(|v| &v.id),
        )?;
        register_entities(
            &mut global_ids,
            "commodity",
            &pack.id,
            pack.commodities.iter().map(|v| &v.id),
        )?;
        register_entities(
            &mut global_ids,
            "quest",
            &pack.id,
            pack.quests.iter().map(|v| &v.id),
        )?;
        register_entities(
            &mut global_ids,
            "shop",
            &pack.id,
            pack.shops.iter().map(|v| &v.id),
        )?;
        register_entities(
            &mut global_ids,
            "script",
            &pack.id,
            pack.scripts.iter().map(|v| &v.id),
        )?;
        register_entities(
            &mut global_ids,
            "scene variant",
            &pack.id,
            pack.scene_variants
                .iter()
                .flat_map(|set| set.variants.iter().map(|variant| &variant.id)),
        )?;

        ownership.push(AreaComposition {
            id: pack.id.clone(),
            source_sha256: area.source_sha256.clone(),
            depends_on: sorted(pack.depends_on.clone()),
            asset_manifests: area_asset_manifest_digests(manifest_path, manifest_dir, pack)?,
            ownership: AreaOwnership {
                rooms: sorted(pack.rooms.iter().map(|value| value.id.clone()).collect()),
                item_prototypes: sorted(
                    pack.item_prototypes
                        .iter()
                        .map(|value| value.id.clone())
                        .collect(),
                ),
                initial_item_instances: sorted(
                    pack.initial_item_instances
                        .iter()
                        .map(|value| value.id.clone())
                        .collect(),
                ),
                npc_prototypes: sorted(
                    pack.npc_prototypes
                        .iter()
                        .map(|value| value.id.clone())
                        .collect(),
                ),
                npc_spawns: sorted(
                    pack.npc_spawns
                        .iter()
                        .map(|value| value.id.clone())
                        .collect(),
                ),
                loot_tables: sorted(
                    pack.loot_tables
                        .iter()
                        .map(|value| value.id.clone())
                        .collect(),
                ),
                loot_sources: sorted(
                    pack.loot_sources
                        .iter()
                        .map(|value| value.id.clone())
                        .collect(),
                ),
                affixes: sorted(pack.affixes.iter().map(|value| value.id.clone()).collect()),
                affix_pools: sorted(
                    pack.affix_pools
                        .iter()
                        .map(|value| value.id.clone())
                        .collect(),
                ),
                archetypes: sorted(
                    pack.archetypes
                        .iter()
                        .map(|value| value.id.clone())
                        .collect(),
                ),
                lineages: sorted(pack.lineages.iter().map(|value| value.id.clone()).collect()),
                backgrounds: sorted(
                    pack.backgrounds
                        .iter()
                        .map(|value| value.id.clone())
                        .collect(),
                ),
                base_classes: sorted(
                    pack.base_classes
                        .iter()
                        .map(|value| value.id.clone())
                        .collect(),
                ),
                advanced_classes: sorted(
                    pack.advanced_classes
                        .iter()
                        .map(|value| value.id.clone())
                        .collect(),
                ),
                talent_graphs: sorted(
                    pack.talent_graphs
                        .iter()
                        .map(|value| value.id.clone())
                        .collect(),
                ),
                talent_nodes: sorted(
                    pack.talent_graphs
                        .iter()
                        .flat_map(|graph| graph.nodes.iter().map(|node| node.id.clone()))
                        .collect(),
                ),
                abilities: sorted(
                    pack.abilities
                        .iter()
                        .map(|value| value.id.clone())
                        .collect(),
                ),
                social_emotes: sorted(
                    pack.social_emotes
                        .iter()
                        .map(|value| value.id.clone())
                        .collect(),
                ),
                commodities: sorted(
                    pack.commodities
                        .iter()
                        .map(|value| value.id.clone())
                        .collect(),
                ),
                quests: sorted(pack.quests.iter().map(|value| value.id.clone()).collect()),
                shops: sorted(pack.shops.iter().map(|value| value.id.clone()).collect()),
                npc_ability_sets: sorted(
                    pack.npc_ability_sets
                        .iter()
                        .map(|value| value.npc_prototype_id.clone())
                        .collect(),
                ),
                room_media: sorted(
                    pack.room_media
                        .iter()
                        .map(|value| value.room_id.clone())
                        .collect(),
                ),
                entity_media: sorted(
                    pack.entity_media
                        .iter()
                        .map(EntityMediaDefinition::key)
                        .collect(),
                ),
                room_perception: sorted(
                    pack.room_perception
                        .iter()
                        .map(|value| value.room_id.clone())
                        .collect(),
                ),
                scene_variant_sets: sorted(
                    pack.scene_variants
                        .iter()
                        .map(|value| value.room_id.clone())
                        .collect(),
                ),
                scene_variants: sorted(
                    pack.scene_variants
                        .iter()
                        .flat_map(|set| set.variants.iter().map(|variant| variant.id.clone()))
                        .collect(),
                ),
                scripts: sorted(pack.scripts.iter().map(|value| value.id.clone()).collect()),
                content_tests: sorted(
                    pack.content_tests
                        .iter()
                        .map(|value| value.name.clone())
                        .collect(),
                ),
            },
        });

        document
            .world
            .item_prototypes
            .extend(pack.item_prototypes.clone());
        document
            .world
            .initial_item_instances
            .extend(pack.initial_item_instances.clone());
        document.world.rooms.extend(pack.rooms.clone());
        document
            .world
            .gameplay
            .npc_prototypes
            .extend(pack.npc_prototypes.clone());
        document
            .world
            .gameplay
            .npc_spawns
            .extend(pack.npc_spawns.clone());
        document
            .world
            .gameplay
            .loot_tables
            .extend(pack.loot_tables.clone());
        document
            .world
            .gameplay
            .loot_sources
            .extend(pack.loot_sources.clone());
        document.world.gameplay.affixes.extend(pack.affixes.clone());
        document
            .world
            .gameplay
            .affix_pools
            .extend(pack.affix_pools.clone());
        document
            .world
            .gameplay
            .archetypes
            .extend(pack.archetypes.clone());
        document
            .world
            .gameplay
            .abilities
            .extend(pack.abilities.clone());
        document
            .world
            .gameplay
            .social_emotes
            .extend(pack.social_emotes.clone());
        document
            .world
            .gameplay
            .commodities
            .extend(pack.commodities.clone());
        document.world.gameplay.quests.extend(pack.quests.clone());
        document.world.gameplay.shops.extend(pack.shops.clone());
        document
            .world
            .gameplay
            .npc_ability_sets
            .extend(pack.npc_ability_sets.clone());
        document.room_media.extend(pack.room_media.clone());
        document.entity_media.extend(pack.entity_media.clone());
        document
            .room_perception
            .extend(pack.room_perception.clone());
        document.scene_variants.extend(pack.scene_variants.clone());
        document.scripts.extend(pack.scripts.clone());
        document.content_tests.extend(pack.content_tests.clone());
    }

    let has_progression = areas.iter().any(|area| {
        let pack = &area.pack;
        !pack.level_thresholds.is_empty()
            || !pack.lineages.is_empty()
            || !pack.backgrounds.is_empty()
            || !pack.base_classes.is_empty()
            || !pack.advanced_classes.is_empty()
            || !pack.talent_graphs.is_empty()
    });
    if has_progression {
        document.world.gameplay.character_progression = Some(CharacterProgressionDefinition {
            defaults: CharacterDefaultsDefinition {
                lineage_id: manifest.default_lineage_id.clone().ok_or_else(|| {
                    CompositionError::InvalidCharacterProgressionDefault(
                        "defaultLineageId is required".to_owned(),
                    )
                })?,
                background_id: manifest.default_background_id.clone(),
                base_class_id: manifest.default_base_class_id.clone().ok_or_else(|| {
                    CompositionError::InvalidCharacterProgressionDefault(
                        "defaultBaseClassId is required".to_owned(),
                    )
                })?,
            },
            level_thresholds: areas
                .iter()
                .flat_map(|area| area.pack.level_thresholds.clone())
                .collect(),
            lineages: areas
                .iter()
                .flat_map(|area| area.pack.lineages.clone())
                .collect(),
            backgrounds: areas
                .iter()
                .flat_map(|area| area.pack.backgrounds.clone())
                .collect(),
            base_classes: areas
                .iter()
                .flat_map(|area| area.pack.base_classes.clone())
                .collect(),
            advanced_classes: areas
                .iter()
                .flat_map(|area| area.pack.advanced_classes.clone())
                .collect(),
            talent_graphs: areas
                .iter()
                .flat_map(|area| area.pack.talent_graphs.clone())
                .collect(),
        });
    }

    document.validate()?;
    let mut cross_area_references = derive_cross_area_references(&areas);
    cross_area_references.sort();
    cross_area_references.dedup();
    let declared = areas
        .iter()
        .map(|area| {
            (
                area.pack.id.as_str(),
                area.pack
                    .depends_on
                    .iter()
                    .map(String::as_str)
                    .collect::<HashSet<_>>(),
            )
        })
        .collect::<HashMap<_, _>>();
    for edge in &cross_area_references {
        if !declared
            .get(edge.from_area.as_str())
            .is_some_and(|dependencies| dependencies.contains(edge.to_area.as_str()))
        {
            return Err(CompositionError::UndeclaredDependency {
                area_id: edge.from_area.clone(),
                target_area: edge.to_area.clone(),
                kind: edge.kind,
                source_id: edge.source_id.clone(),
                target_id: edge.target_id.clone(),
            });
        }
    }
    validate_project_assets(manifest_path, manifest_dir, &document)?;
    let artifact = document
        .deterministic_artifact()
        .map_err(|error| match error {
            crate::DocumentToolError::InvalidDocument(error) => {
                CompositionError::InvalidWorld(error)
            }
            crate::DocumentToolError::Json(error) => CompositionError::Artifact(error),
            crate::DocumentToolError::InvalidScene(error) => {
                unreachable!("artifact generation does not resolve scenes: {error}")
            }
            crate::DocumentToolError::LegacyProgression(error) => {
                unreachable!("composed artifacts are already WorldPack v9: {error}")
            }
            crate::DocumentToolError::LegacyItems(error) => {
                unreachable!("composed artifacts already use native v5 items: {error}")
            }
        })?;
    if artifact.len() > MAX_COMPOSED_WORLD_BYTES {
        return Err(CompositionError::ComposedWorldTooLarge {
            size: artifact.len(),
            maximum: MAX_COMPOSED_WORLD_BYTES,
        });
    }
    ownership.sort_by(|left, right| left.id.cmp(&right.id));
    let report = CompositionReport {
        project_id: manifest.id,
        project_manifest_sha256,
        artifact_sha256: sha256_hex(&artifact),
        artifact_bytes: artifact.len(),
        areas: ownership,
        cross_area_references,
    };
    let area_packs = areas.iter().map(|area| area.pack.clone()).collect();
    Ok(ComposedWorld {
        document,
        artifact,
        report,
        area_packs,
    })
}

pub(crate) fn ownership_for_area(pack: &AreaPack) -> AreaOwnership {
    AreaOwnership {
        rooms: sorted(pack.rooms.iter().map(|value| value.id.clone()).collect()),
        item_prototypes: sorted(
            pack.item_prototypes
                .iter()
                .map(|value| value.id.clone())
                .collect(),
        ),
        initial_item_instances: sorted(
            pack.initial_item_instances
                .iter()
                .map(|value| value.id.clone())
                .collect(),
        ),
        npc_prototypes: sorted(
            pack.npc_prototypes
                .iter()
                .map(|value| value.id.clone())
                .collect(),
        ),
        npc_spawns: sorted(
            pack.npc_spawns
                .iter()
                .map(|value| value.id.clone())
                .collect(),
        ),
        loot_tables: sorted(
            pack.loot_tables
                .iter()
                .map(|value| value.id.clone())
                .collect(),
        ),
        loot_sources: sorted(
            pack.loot_sources
                .iter()
                .map(|value| value.id.clone())
                .collect(),
        ),
        affixes: sorted(pack.affixes.iter().map(|value| value.id.clone()).collect()),
        affix_pools: sorted(
            pack.affix_pools
                .iter()
                .map(|value| value.id.clone())
                .collect(),
        ),
        archetypes: sorted(
            pack.archetypes
                .iter()
                .map(|value| value.id.clone())
                .collect(),
        ),
        lineages: sorted(pack.lineages.iter().map(|value| value.id.clone()).collect()),
        backgrounds: sorted(
            pack.backgrounds
                .iter()
                .map(|value| value.id.clone())
                .collect(),
        ),
        base_classes: sorted(
            pack.base_classes
                .iter()
                .map(|value| value.id.clone())
                .collect(),
        ),
        advanced_classes: sorted(
            pack.advanced_classes
                .iter()
                .map(|value| value.id.clone())
                .collect(),
        ),
        talent_graphs: sorted(
            pack.talent_graphs
                .iter()
                .map(|value| value.id.clone())
                .collect(),
        ),
        talent_nodes: sorted(
            pack.talent_graphs
                .iter()
                .flat_map(|graph| graph.nodes.iter().map(|node| node.id.clone()))
                .collect(),
        ),
        abilities: sorted(
            pack.abilities
                .iter()
                .map(|value| value.id.clone())
                .collect(),
        ),
        social_emotes: sorted(
            pack.social_emotes
                .iter()
                .map(|value| value.id.clone())
                .collect(),
        ),
        commodities: sorted(
            pack.commodities
                .iter()
                .map(|value| value.id.clone())
                .collect(),
        ),
        quests: sorted(pack.quests.iter().map(|value| value.id.clone()).collect()),
        shops: sorted(pack.shops.iter().map(|value| value.id.clone()).collect()),
        npc_ability_sets: sorted(
            pack.npc_ability_sets
                .iter()
                .map(|value| value.npc_prototype_id.clone())
                .collect(),
        ),
        room_media: sorted(
            pack.room_media
                .iter()
                .map(|value| value.room_id.clone())
                .collect(),
        ),
        entity_media: sorted(
            pack.entity_media
                .iter()
                .map(EntityMediaDefinition::key)
                .collect(),
        ),
        room_perception: sorted(
            pack.room_perception
                .iter()
                .map(|value| value.room_id.clone())
                .collect(),
        ),
        scene_variant_sets: sorted(
            pack.scene_variants
                .iter()
                .map(|value| value.room_id.clone())
                .collect(),
        ),
        scene_variants: sorted(
            pack.scene_variants
                .iter()
                .flat_map(|set| set.variants.iter().map(|variant| variant.id.clone()))
                .collect(),
        ),
        scripts: sorted(pack.scripts.iter().map(|value| value.id.clone()).collect()),
        content_tests: sorted(
            pack.content_tests
                .iter()
                .map(|value| value.name.clone())
                .collect(),
        ),
    }
}

pub(crate) fn cross_area_references_for_packs(packs: &[AreaPack]) -> Vec<AreaCrossReference> {
    let loaded = packs
        .iter()
        .cloned()
        .map(|pack| LoadedArea {
            source_sha256: String::new(),
            pack,
        })
        .collect::<Vec<_>>();
    let mut references = derive_cross_area_references(&loaded);
    references.sort();
    references.dedup();
    references
}

fn canonical_project_digest(manifest: &WorldProjectManifest) -> Result<String, CompositionError> {
    let mut canonical = manifest.clone();
    canonical
        .areas
        .sort_by(|left, right| left.id.cmp(&right.id).then(left.path.cmp(&right.path)));
    serde_json::to_vec(&canonical)
        .map(|bytes| sha256_hex(&bytes))
        .map_err(CompositionError::Artifact)
}

fn register_entities<'a>(
    global: &mut HashMap<String, Vec<(&'static str, String)>>,
    kind: &'static str,
    area: &str,
    ids: impl Iterator<Item = &'a String>,
) -> Result<(), CompositionError> {
    for id in ids {
        let registrations = global.entry(id.clone()).or_default();
        if let Some((first_kind, first_area)) = registrations.first() {
            let typed_legacy_pair = registrations.len() == 1
                && matches!(
                    (*first_kind, kind),
                    ("item prototype", "initial item instance")
                        | ("initial item instance", "item prototype")
                );
            if !typed_legacy_pair {
                return Err(CompositionError::DuplicateGlobalEntityId {
                    id: id.clone(),
                    first_kind,
                    first_area: first_area.clone(),
                    second_kind: kind,
                    second_area: area.to_owned(),
                });
            }
        }
        registrations.push((kind, area.to_owned()));
    }
    Ok(())
}

fn derive_cross_area_references(areas: &[LoadedArea]) -> Vec<AreaCrossReference> {
    let room_owner = owner_map(areas, |area| {
        area.rooms.iter().map(|value| value.id.as_str())
    });
    let item_prototype_owner = owner_map(areas, |area| {
        area.item_prototypes.iter().map(|value| value.id.as_str())
    });
    let item_instance_owner = owner_map(areas, |area| {
        area.initial_item_instances
            .iter()
            .map(|value| value.id.as_str())
    });
    let prototype_owner = owner_map(areas, |area| {
        area.npc_prototypes.iter().map(|value| value.id.as_str())
    });
    let ability_owner = owner_map(areas, |area| {
        area.abilities.iter().map(|value| value.id.as_str())
    });
    let progression_owner = areas
        .iter()
        .flat_map(|area| {
            let owner = area.pack.id.as_str();
            area.pack
                .lineages
                .iter()
                .map(move |value| (value.id.as_str(), owner))
                .chain(
                    area.pack
                        .backgrounds
                        .iter()
                        .map(move |value| (value.id.as_str(), owner)),
                )
                .chain(
                    area.pack
                        .base_classes
                        .iter()
                        .map(move |value| (value.id.as_str(), owner)),
                )
                .chain(
                    area.pack
                        .advanced_classes
                        .iter()
                        .map(move |value| (value.id.as_str(), owner)),
                )
                .chain(area.pack.talent_graphs.iter().flat_map(move |graph| {
                    std::iter::once((graph.id.as_str(), owner)).chain(
                        graph
                            .nodes
                            .iter()
                            .map(move |node| (node.id.as_str(), owner)),
                    )
                }))
        })
        .collect::<HashMap<_, _>>();
    let spawn_owner = owner_map(areas, |area| {
        area.npc_spawns.iter().map(|value| value.id.as_str())
    });
    let loot_table_owner = owner_map(areas, |area| {
        area.loot_tables.iter().map(|value| value.id.as_str())
    });
    let affix_owner = owner_map(areas, |area| {
        area.affixes.iter().map(|value| value.id.as_str())
    });
    let affix_pool_owner = owner_map(areas, |area| {
        area.affix_pools.iter().map(|value| value.id.as_str())
    });
    let legacy_owner = areas
        .iter()
        .flat_map(|area| {
            area.pack
                .rooms
                .iter()
                .filter_map(|room| room.legacy_vnum.map(|vnum| (vnum, area.pack.id.as_str())))
        })
        .collect::<HashMap<_, _>>();
    let mut edges = Vec::new();
    for area in areas {
        let from = area.pack.id.as_str();
        for prototype in &area.pack.item_prototypes {
            if let Some(table_id) = prototype.sealed_loot_table_id.as_deref()
                && let Some(owner) = loot_table_owner.get(table_id)
            {
                add_edge(
                    &mut edges,
                    from,
                    owner,
                    AreaCrossReferenceKind::ItemSealedLootTable,
                    &prototype.id,
                    table_id,
                );
            }
        }
        for room in &area.pack.rooms {
            for exit in &room.exits {
                let target = match &exit.target {
                    ExitTarget::Room { room_id } => room_owner
                        .get(room_id.as_str())
                        .map(|owner| (*owner, room_id.as_str())),
                    ExitTarget::LegacyVnum { vnum } => legacy_owner.get(vnum).and_then(|owner| {
                        areas
                            .iter()
                            .flat_map(|area| &area.pack.rooms)
                            .find(|room| room.legacy_vnum == Some(*vnum))
                            .map(|room| (*owner, room.id.as_str()))
                    }),
                };
                if let Some((owner, target_id)) = target {
                    add_edge(
                        &mut edges,
                        from,
                        owner,
                        AreaCrossReferenceKind::RoomExit,
                        &room.id,
                        target_id,
                    );
                }
            }
        }
        for instance in &area.pack.initial_item_instances {
            if let Some(owner) = item_prototype_owner.get(instance.prototype_id.as_str()) {
                add_edge(
                    &mut edges,
                    from,
                    owner,
                    AreaCrossReferenceKind::ItemInstancePrototype,
                    &instance.id,
                    &instance.prototype_id,
                );
            }
            match &instance.location {
                crate::InitialItemLocation::Room { room_id } => {
                    if let Some(owner) = room_owner.get(room_id.as_str()) {
                        add_edge(
                            &mut edges,
                            from,
                            owner,
                            AreaCrossReferenceKind::ItemInstanceRoom,
                            &instance.id,
                            room_id,
                        );
                    }
                }
                crate::InitialItemLocation::NpcSpawn { npc_spawn_id } => {
                    if let Some(owner) = spawn_owner.get(npc_spawn_id.as_str()) {
                        add_edge(
                            &mut edges,
                            from,
                            owner,
                            AreaCrossReferenceKind::ItemInstanceNpcSpawn,
                            &instance.id,
                            npc_spawn_id,
                        );
                    }
                }
                crate::InitialItemLocation::Container {
                    container_instance_id,
                } => {
                    if let Some(owner) = item_instance_owner.get(container_instance_id.as_str()) {
                        add_edge(
                            &mut edges,
                            from,
                            owner,
                            AreaCrossReferenceKind::ItemInstanceContainer,
                            &instance.id,
                            container_instance_id,
                        );
                    }
                }
                crate::InitialItemLocation::Staged => {}
            }
        }
        for spawn in &area.pack.npc_spawns {
            if let Some(owner) = prototype_owner.get(spawn.prototype_id.as_str()) {
                add_edge(
                    &mut edges,
                    from,
                    owner,
                    AreaCrossReferenceKind::NpcSpawnPrototype,
                    &spawn.id,
                    &spawn.prototype_id,
                );
            }
            if let Some(owner) = room_owner.get(spawn.room_id.as_str()) {
                add_edge(
                    &mut edges,
                    from,
                    owner,
                    AreaCrossReferenceKind::NpcSpawnRoom,
                    &spawn.id,
                    &spawn.room_id,
                );
            }
        }
        for table in &area.pack.loot_tables {
            for guaranteed in &table.guaranteed {
                if let Some(owner) = item_prototype_owner.get(guaranteed.item_prototype_id.as_str())
                {
                    add_edge(
                        &mut edges,
                        from,
                        owner,
                        AreaCrossReferenceKind::LootGuaranteedPrototype,
                        &guaranteed.id,
                        &guaranteed.item_prototype_id,
                    );
                }
                if let Some(pool_id) = guaranteed.affix_pool_id.as_deref()
                    && let Some(owner) = affix_pool_owner.get(pool_id)
                {
                    add_edge(
                        &mut edges,
                        from,
                        owner,
                        AreaCrossReferenceKind::LootGuaranteedAffixPool,
                        &guaranteed.id,
                        pool_id,
                    );
                }
            }
            for entry in table.pools.iter().flat_map(|pool| &pool.entries) {
                if let Some(prototype_id) = entry.item_prototype_id.as_deref()
                    && let Some(owner) = item_prototype_owner.get(prototype_id)
                {
                    add_edge(
                        &mut edges,
                        from,
                        owner,
                        AreaCrossReferenceKind::LootEntryPrototype,
                        &entry.id,
                        prototype_id,
                    );
                }
                if let Some(pool_id) = entry.affix_pool_id.as_deref()
                    && let Some(owner) = affix_pool_owner.get(pool_id)
                {
                    add_edge(
                        &mut edges,
                        from,
                        owner,
                        AreaCrossReferenceKind::LootEntryAffixPool,
                        &entry.id,
                        pool_id,
                    );
                }
            }
        }
        for pool in &area.pack.affix_pools {
            for entry in &pool.entries {
                if let Some(affix_id) = entry.affix_id.as_deref()
                    && let Some(owner) = affix_owner.get(affix_id)
                {
                    add_edge(
                        &mut edges,
                        from,
                        owner,
                        AreaCrossReferenceKind::AffixPoolAffix,
                        &pool.id,
                        affix_id,
                    );
                }
            }
        }
        for source in &area.pack.loot_sources {
            if let Some(owner) = spawn_owner.get(source.npc_spawn_id.as_str()) {
                add_edge(
                    &mut edges,
                    from,
                    owner,
                    AreaCrossReferenceKind::LootSourceNpcSpawn,
                    &source.id,
                    &source.npc_spawn_id,
                );
            }
            if let Some(owner) = loot_table_owner.get(source.loot_table_id.as_str()) {
                add_edge(
                    &mut edges,
                    from,
                    owner,
                    AreaCrossReferenceKind::LootSourceTable,
                    &source.id,
                    &source.loot_table_id,
                );
            }
        }
        for archetype in &area.pack.archetypes {
            for ability_id in archetype.initial_ability_ids.iter().chain(
                archetype
                    .level_progression
                    .iter()
                    .flat_map(|level| &level.ability_unlock_ids),
            ) {
                if let Some(owner) = ability_owner.get(ability_id.as_str()) {
                    add_edge(
                        &mut edges,
                        from,
                        owner,
                        AreaCrossReferenceKind::ArchetypeAbility,
                        &archetype.id,
                        ability_id,
                    );
                }
            }
        }
        for ability in &area.pack.abilities {
            if let Some(communication) = &ability.communication {
                for room_id in &communication.blocked_room_ids {
                    if let Some(owner) = room_owner.get(room_id.as_str()) {
                        add_edge(
                            &mut edges,
                            from,
                            owner,
                            AreaCrossReferenceKind::AbilityBlockedRoom,
                            &ability.id,
                            room_id,
                        );
                    }
                }
            }
        }
        for lineage in &area.pack.lineages {
            add_progression_references(
                &mut edges,
                from,
                &lineage.id,
                lineage.talent_graph_ids.iter().map(String::as_str),
                &progression_owner,
            );
            add_grant_references(
                &mut edges,
                from,
                &lineage.id,
                &lineage.creation_grants,
                &ability_owner,
            );
        }
        for background in &area.pack.backgrounds {
            add_progression_references(
                &mut edges,
                from,
                &background.id,
                background.talent_graph_ids.iter().map(String::as_str),
                &progression_owner,
            );
            add_grant_references(
                &mut edges,
                from,
                &background.id,
                &background.creation_grants,
                &ability_owner,
            );
        }
        for class in &area.pack.base_classes {
            add_progression_references(
                &mut edges,
                from,
                &class.id,
                class.talent_graph_ids.iter().map(String::as_str),
                &progression_owner,
            );
            add_grant_references(
                &mut edges,
                from,
                &class.id,
                &class.creation_grants,
                &ability_owner,
            );
            for milestone in &class.level_grants {
                add_grant_references(
                    &mut edges,
                    from,
                    &class.id,
                    &milestone.grants,
                    &ability_owner,
                );
            }
        }
        for class in &area.pack.advanced_classes {
            add_progression_references(
                &mut edges,
                from,
                &class.id,
                class
                    .from_class_ids
                    .iter()
                    .chain(&class.talent_graph_ids)
                    .map(String::as_str),
                &progression_owner,
            );
            add_requirement_references(
                &mut edges,
                from,
                &class.id,
                &class.requirements,
                &progression_owner,
                &ability_owner,
            );
            add_grant_references(
                &mut edges,
                from,
                &class.id,
                &class.promotion_grants,
                &ability_owner,
            );
            for milestone in &class.level_grants {
                add_grant_references(
                    &mut edges,
                    from,
                    &class.id,
                    &milestone.grants,
                    &ability_owner,
                );
            }
        }
        for graph in &area.pack.talent_graphs {
            for node in &graph.nodes {
                for rank in &node.ranks {
                    add_requirement_references(
                        &mut edges,
                        from,
                        &node.id,
                        &rank.requirements,
                        &progression_owner,
                        &ability_owner,
                    );
                    add_grant_references(&mut edges, from, &node.id, &rank.grants, &ability_owner);
                }
            }
        }
        for set in &area.pack.npc_ability_sets {
            if let Some(owner) = prototype_owner.get(set.npc_prototype_id.as_str()) {
                add_edge(
                    &mut edges,
                    from,
                    owner,
                    AreaCrossReferenceKind::NpcAbilitySetPrototype,
                    &set.npc_prototype_id,
                    &set.npc_prototype_id,
                );
            }
            for ability_id in &set.ability_ids {
                if let Some(owner) = ability_owner.get(ability_id.as_str()) {
                    add_edge(
                        &mut edges,
                        from,
                        owner,
                        AreaCrossReferenceKind::NpcAbility,
                        &set.npc_prototype_id,
                        ability_id,
                    );
                }
            }
        }
        for media in &area.pack.room_media {
            if let Some(owner) = room_owner.get(media.room_id.as_str()) {
                add_edge(
                    &mut edges,
                    from,
                    owner,
                    AreaCrossReferenceKind::RoomMedia,
                    &media.room_id,
                    &media.room_id,
                );
            }
        }
        for media in &area.pack.entity_media {
            let owner = match media.kind {
                EntityMediaKind::Item => item_prototype_owner.get(media.subject_id.as_str()),
                EntityMediaKind::Npc => spawn_owner.get(media.subject_id.as_str()),
            };
            if let Some(owner) = owner {
                add_edge(
                    &mut edges,
                    from,
                    owner,
                    AreaCrossReferenceKind::EntityMedia,
                    &media.key(),
                    &media.subject_id,
                );
            }
        }
        for perception in &area.pack.room_perception {
            if let Some(owner) = room_owner.get(perception.room_id.as_str()) {
                add_edge(
                    &mut edges,
                    from,
                    owner,
                    AreaCrossReferenceKind::RoomPerception,
                    &perception.room_id,
                    &perception.room_id,
                );
            }
            for item in perception.hidden_items.keys() {
                if let Some(owner) = item_instance_owner.get(item.as_str()) {
                    add_edge(
                        &mut edges,
                        from,
                        owner,
                        AreaCrossReferenceKind::HiddenItem,
                        &perception.room_id,
                        item,
                    );
                }
            }
            for spawn in perception.hidden_npcs.keys() {
                if let Some(owner) = spawn_owner.get(spawn.as_str()) {
                    add_edge(
                        &mut edges,
                        from,
                        owner,
                        AreaCrossReferenceKind::HiddenNpc,
                        &perception.room_id,
                        spawn,
                    );
                }
            }
        }
        for variants in &area.pack.scene_variants {
            if let Some(owner) = room_owner.get(variants.room_id.as_str()) {
                add_edge(
                    &mut edges,
                    from,
                    owner,
                    AreaCrossReferenceKind::SceneVariants,
                    &variants.room_id,
                    &variants.room_id,
                );
            }
        }
        for script in &area.pack.scripts {
            if let Some(owner) = room_owner.get(script.room_id.as_str()) {
                add_edge(
                    &mut edges,
                    from,
                    owner,
                    AreaCrossReferenceKind::ScriptRoom,
                    &script.id,
                    &script.room_id,
                );
            }
        }
        for test in &area.pack.content_tests {
            if let Some(owner) = room_owner.get(test.room_id.as_str()) {
                add_edge(
                    &mut edges,
                    from,
                    owner,
                    AreaCrossReferenceKind::ContentTestRoom,
                    &test.name,
                    &test.room_id,
                );
            }
        }
    }
    edges
}

fn owner_map<'a, I>(
    areas: &'a [LoadedArea],
    values: impl Fn(&'a AreaPack) -> I,
) -> HashMap<&'a str, &'a str>
where
    I: Iterator<Item = &'a str>,
{
    areas
        .iter()
        .flat_map(|area| values(&area.pack).map(|id| (id, area.pack.id.as_str())))
        .collect()
}

fn add_progression_references<'a>(
    edges: &mut Vec<AreaCrossReference>,
    from: &str,
    source_id: &str,
    targets: impl Iterator<Item = &'a str>,
    owners: &HashMap<&str, &str>,
) {
    for target in targets {
        if let Some(owner) = owners.get(target) {
            add_edge(
                edges,
                from,
                owner,
                AreaCrossReferenceKind::ProgressionReference,
                source_id,
                target,
            );
        }
    }
}

fn add_grant_references(
    edges: &mut Vec<AreaCrossReference>,
    from: &str,
    source_id: &str,
    grants: &[GrantDefinition],
    ability_owners: &HashMap<&str, &str>,
) {
    for grant in grants {
        if let GrantDefinition::Ability { ability_id } = grant
            && let Some(owner) = ability_owners.get(ability_id.as_str())
        {
            add_edge(
                edges,
                from,
                owner,
                AreaCrossReferenceKind::ProgressionReference,
                source_id,
                ability_id,
            );
        }
    }
}

fn add_requirement_references(
    edges: &mut Vec<AreaCrossReference>,
    from: &str,
    source_id: &str,
    requirements: &[RequirementDefinition],
    progression_owners: &HashMap<&str, &str>,
    ability_owners: &HashMap<&str, &str>,
) {
    for requirement in requirements {
        match &requirement.predicate {
            RequirementPredicate::Lineage { lineage_id } => add_progression_references(
                edges,
                from,
                source_id,
                std::iter::once(lineage_id.as_str()),
                progression_owners,
            ),
            RequirementPredicate::CurrentClass { class_id }
            | RequirementPredicate::HistoricalClass { class_id } => add_progression_references(
                edges,
                from,
                source_id,
                std::iter::once(class_id.as_str()),
                progression_owners,
            ),
            RequirementPredicate::NodeRank {
                graph_id, node_id, ..
            } => add_progression_references(
                edges,
                from,
                source_id,
                [graph_id.as_str(), node_id.as_str()].into_iter(),
                progression_owners,
            ),
            RequirementPredicate::Ability { ability_id } => {
                if let Some(owner) = ability_owners.get(ability_id.as_str()) {
                    add_edge(
                        edges,
                        from,
                        owner,
                        AreaCrossReferenceKind::ProgressionReference,
                        source_id,
                        ability_id,
                    );
                }
            }
            RequirementPredicate::All { requirements }
            | RequirementPredicate::Any { requirements } => add_requirement_references(
                edges,
                from,
                source_id,
                requirements,
                progression_owners,
                ability_owners,
            ),
            RequirementPredicate::Not { requirement } => add_requirement_references(
                edges,
                from,
                source_id,
                std::slice::from_ref(requirement),
                progression_owners,
                ability_owners,
            ),
            RequirementPredicate::Level { .. }
            | RequirementPredicate::PermanentAttribute { .. }
            | RequirementPredicate::Skill { .. }
            | RequirementPredicate::CompletedQuest { .. }
            | RequirementPredicate::ReceivedSignal { .. }
            | RequirementPredicate::CharacterFlag { .. }
            | RequirementPredicate::WorldFlag { .. } => {}
        }
    }
}

fn add_edge(
    edges: &mut Vec<AreaCrossReference>,
    from: &str,
    to: &str,
    kind: AreaCrossReferenceKind,
    source_id: &str,
    target_id: &str,
) {
    if from != to {
        edges.push(AreaCrossReference {
            from_area: from.to_owned(),
            to_area: to.to_owned(),
            kind,
            source_id: source_id.to_owned(),
            target_id: target_id.to_owned(),
        });
    }
}

fn area_asset_manifest_digests(
    manifest_path: &Path,
    manifest_dir: &Path,
    pack: &AreaPack,
) -> Result<Vec<AreaAssetManifestDigest>, CompositionError> {
    let roots = project_asset_roots(manifest_path, manifest_dir);
    let mut paths = pack
        .rooms
        .iter()
        .filter_map(|room| room.asset_manifest.clone())
        .chain(
            pack.scene_variants
                .iter()
                .flat_map(|set| &set.variants)
                .filter_map(|variant| variant.asset_manifest.clone()),
        )
        .collect::<Vec<_>>();
    paths.sort();
    paths.dedup();
    paths
        .into_iter()
        .map(|path| {
            let resolved = locate_asset(&roots, &path)?;
            let bytes = read_bounded(&resolved, 256 * 1024, "image manifest")?;
            Ok(AreaAssetManifestDigest {
                path,
                sha256: sha256_hex(&bytes),
            })
        })
        .collect()
}

fn validate_project_assets(
    manifest_path: &Path,
    manifest_dir: &Path,
    document: &WorldPackDocument,
) -> Result<(), CompositionError> {
    let roots = project_asset_roots(manifest_path, manifest_dir);
    let media = document
        .room_media
        .iter()
        .map(|value| (value.room_id.as_str(), value))
        .collect::<HashMap<_, _>>();
    for room in &document.world.rooms {
        if let Some(relative) = &room.asset_manifest {
            let expected_alt = media
                .get(room.id.as_str())
                .and_then(|value| value.image_alt.resolve("en"))
                .expect("WorldPack validation requires English alt text");
            validate_image_manifest(&roots, relative, expected_alt)?;
        }
        if let Some(audio) = media
            .get(room.id.as_str())
            .and_then(|value| value.audio.as_ref())
        {
            locate_asset(&roots, &audio.path)?;
        }
    }
    for variants in &document.scene_variants {
        for variant in &variants.variants {
            if let Some(relative) = &variant.asset_manifest {
                let expected_alt = variant
                    .image_alt
                    .resolve("en")
                    .expect("WorldPack validation requires English alt text");
                validate_image_manifest(&roots, relative, expected_alt)?;
            }
            if let Some(audio) = &variant.audio {
                locate_asset(&roots, &audio.path)?;
            }
        }
    }
    for media in &document.entity_media {
        let sources = std::iter::once(&media.poses.idle)
            .chain(media.poses.attack.iter())
            .chain(media.poses.hit.iter())
            .chain(media.poses.defeated.iter());
        for source in sources {
            for path in [source.webp.as_deref(), source.png.as_deref()]
                .into_iter()
                .flatten()
            {
                locate_asset(&roots, path)?;
            }
        }
    }
    Ok(())
}

fn project_asset_roots(manifest_path: &Path, manifest_dir: &Path) -> Vec<PathBuf> {
    let project_root = manifest_path
        .ancestors()
        .skip(1)
        .find(|ancestor| {
            ancestor.join(".git").exists()
                || (ancestor.join("Cargo.toml").is_file()
                    && ancestor.join("apps/web/public").is_dir())
        })
        .unwrap_or(manifest_dir);
    let mut candidates = vec![
        manifest_dir.to_owned(),
        project_root.to_owned(),
        project_root.join("apps/web/public"),
    ];
    candidates.retain(|path| path.is_dir());
    let mut roots = candidates
        .into_iter()
        .filter_map(|path| fs::canonicalize(path).ok())
        .collect::<Vec<_>>();
    roots.sort();
    roots.dedup();
    roots
}

fn validate_image_manifest(
    roots: &[PathBuf],
    relative: &str,
    expected_alt: &str,
) -> Result<(), CompositionError> {
    let path = locate_asset(roots, relative)?;
    let bytes = read_bounded(&path, 256 * 1024, "image manifest")?;
    let value: serde_json::Value =
        serde_json::from_slice(&bytes).map_err(|source| CompositionError::ImageManifestJson {
            path: path.clone(),
            source,
        })?;
    if value.get("alt").and_then(serde_json::Value::as_str) != Some(expected_alt) {
        return Err(CompositionError::ImageAltMismatch { path });
    }
    Ok(())
}

fn locate_asset(roots: &[PathBuf], relative: &str) -> Result<PathBuf, CompositionError> {
    if !safe_relative_path(relative) {
        return Err(CompositionError::UnsafeAssetPath {
            path: relative.to_owned(),
        });
    }
    for root in roots {
        let requested = root.join(relative);
        let Ok(canonical) = fs::canonicalize(&requested) else {
            continue;
        };
        if canonical.starts_with(root) && canonical.is_file() {
            return Ok(canonical);
        }
    }
    Err(CompositionError::MissingAsset {
        path: relative.to_owned(),
    })
}

fn read_bounded(
    path: &Path,
    maximum: u64,
    kind: &'static str,
) -> Result<Vec<u8>, CompositionError> {
    // Reject static FIFOs/devices before opening them: opening a FIFO for read
    // can block, while devices can report a zero length and stream forever.
    let path_metadata = fs::metadata(path).map_err(|source| CompositionError::Inspect {
        path: path.to_owned(),
        source,
    })?;
    if !path_metadata.is_file() {
        return Err(CompositionError::NotRegularFile {
            kind,
            path: path.to_owned(),
        });
    }
    if path_metadata.len() > maximum {
        return Err(CompositionError::FileTooLarge {
            kind,
            path: path.to_owned(),
            size: path_metadata.len(),
            maximum,
        });
    }

    // Keep one handle from inspection through read, then cap the read itself.
    // The second regular-file check catches replacement between metadata and
    // open; max + 1 catches a regular file that grows after either check.
    let mut file = fs::File::open(path).map_err(|source| CompositionError::Read {
        path: path.to_owned(),
        source,
    })?;
    let metadata = file
        .metadata()
        .map_err(|source| CompositionError::Inspect {
            path: path.to_owned(),
            source,
        })?;
    if !metadata.is_file() {
        return Err(CompositionError::NotRegularFile {
            kind,
            path: path.to_owned(),
        });
    }
    if metadata.len() > maximum {
        return Err(CompositionError::FileTooLarge {
            kind,
            path: path.to_owned(),
            size: metadata.len(),
            maximum,
        });
    }

    let mut bytes = Vec::with_capacity(metadata.len().min(maximum) as usize);
    file.by_ref()
        .take(maximum.saturating_add(1))
        .read_to_end(&mut bytes)
        .map_err(|source| CompositionError::Read {
            path: path.to_owned(),
            source,
        })?;
    if bytes.len() as u64 > maximum {
        return Err(CompositionError::FileTooLarge {
            kind,
            path: path.to_owned(),
            size: bytes.len() as u64,
            maximum,
        });
    }
    Ok(bytes)
}

fn safe_identifier(value: &str) -> bool {
    let trimmed = value.trim();
    !trimmed.is_empty() && trimmed.len() <= 256 && !trimmed.chars().any(char::is_control)
}

/// Existing runtime WorldPack readers intentionally tolerate future fields.
/// AreaPack v8 does not: this format-specific key audit makes every structured
/// authoring record fail closed without changing runtime deserialization.
pub(crate) fn audit_area_pack_keys(value: &serde_json::Value) -> Result<(), CompositionError> {
    let Some(area) = audit_object(
        value,
        &[
            "schemaVersion",
            "id",
            "dependsOn",
            "itemPrototypes",
            "initialItemInstances",
            "items",
            "rooms",
            "itemRules",
            "npcPrototypes",
            "npcSpawns",
            "lootTables",
            "lootSources",
            "affixes",
            "affixPools",
            "archetypes",
            "levelThresholds",
            "lineages",
            "backgrounds",
            "baseClasses",
            "advancedClasses",
            "talentGraphs",
            "abilities",
            "socialEmotes",
            "commodities",
            "quests",
            "shops",
            "npcAbilitySets",
            "roomMedia",
            "entityMedia",
            "roomPerception",
            "sceneVariants",
            "scripts",
            "contentTests",
        ],
        "$",
    )?
    else {
        return Ok(());
    };
    audit_array(
        area.get("itemPrototypes"),
        "$.itemPrototypes",
        audit_item_prototype,
    )?;
    audit_array(
        area.get("initialItemInstances"),
        "$.initialItemInstances",
        audit_initial_item_instance,
    )?;
    audit_array(area.get("items"), "$.items", audit_item)?;
    audit_array(area.get("rooms"), "$.rooms", audit_room)?;
    audit_array(area.get("itemRules"), "$.itemRules", audit_item_rule)?;
    audit_array(
        area.get("npcPrototypes"),
        "$.npcPrototypes",
        audit_npc_prototype,
    )?;
    audit_array(area.get("npcSpawns"), "$.npcSpawns", audit_npc_spawn)?;
    audit_array(area.get("lootTables"), "$.lootTables", audit_loot_table)?;
    audit_array(area.get("lootSources"), "$.lootSources", audit_loot_source)?;
    audit_array(area.get("affixes"), "$.affixes", audit_affix)?;
    audit_array(area.get("affixPools"), "$.affixPools", audit_affix_pool)?;
    audit_array(area.get("archetypes"), "$.archetypes", audit_archetype)?;
    audit_array(area.get("abilities"), "$.abilities", audit_ability)?;
    audit_array(
        area.get("socialEmotes"),
        "$.socialEmotes",
        audit_social_emote,
    )?;
    audit_array(
        area.get("npcAbilitySets"),
        "$.npcAbilitySets",
        audit_npc_ability_set,
    )?;
    audit_array(area.get("roomMedia"), "$.roomMedia", audit_room_media)?;
    audit_array(area.get("entityMedia"), "$.entityMedia", audit_entity_media)?;
    audit_array(
        area.get("roomPerception"),
        "$.roomPerception",
        audit_room_perception,
    )?;
    audit_array(
        area.get("sceneVariants"),
        "$.sceneVariants",
        audit_scene_variants,
    )?;
    audit_array(area.get("scripts"), "$.scripts", audit_script)?;
    audit_array(
        area.get("contentTests"),
        "$.contentTests",
        audit_content_test,
    )?;
    Ok(())
}

fn audit_item(value: &serde_json::Value, path: &str) -> Result<(), CompositionError> {
    audit_object(value, &["id", "name", "description", "aliases"], path).map(|_| ())
}

fn audit_item_prototype(value: &serde_json::Value, path: &str) -> Result<(), CompositionError> {
    let Some(prototype) = audit_object(
        value,
        &[
            "id",
            "name",
            "description",
            "aliases",
            "equipmentSlot",
            "attackBonus",
            "healAmount",
            "consumable",
            "effect",
            "weightGrams",
            "slotCost",
            "rarity",
            "itemLevel",
            "stackPolicy",
            "maxStackUnits",
            "container",
            "transferPolicy",
            "sealedLootTableId",
        ],
        path,
    )?
    else {
        return Ok(());
    };
    if let Some(effect) = prototype.get("effect") {
        audit_object(
            effect,
            &["kind", "magnitude", "remainingTicks"],
            &format!("{path}.effect"),
        )?;
    }
    if let Some(container) = prototype.get("container") {
        audit_object(
            container,
            &["maxSlots", "maxWeightGrams"],
            &format!("{path}.container"),
        )?;
    }
    Ok(())
}

fn audit_initial_item_instance(
    value: &serde_json::Value,
    path: &str,
) -> Result<(), CompositionError> {
    let Some(instance) = audit_object(value, &["id", "prototypeId", "location"], path)? else {
        return Ok(());
    };
    if let Some(location) = instance.get("location") {
        audit_object(
            location,
            &["kind", "roomId", "npcSpawnId", "containerInstanceId"],
            &format!("{path}.location"),
        )?;
    }
    Ok(())
}

fn audit_room(value: &serde_json::Value, path: &str) -> Result<(), CompositionError> {
    let Some(room) = audit_object(
        value,
        &[
            "id",
            "legacyVnum",
            "title",
            "description",
            "assetManifest",
            "exits",
            "items",
            "legacy",
        ],
        path,
    )?
    else {
        return Ok(());
    };
    audit_array(room.get("exits"), &format!("{path}.exits"), audit_exit)?;
    if let Some(legacy) = room.get("legacy") {
        audit_object(
            legacy,
            &["flags", "sector", "prefixValues"],
            &format!("{path}.legacy"),
        )?;
    }
    Ok(())
}

fn audit_exit(value: &serde_json::Value, path: &str) -> Result<(), CompositionError> {
    let Some(exit) = audit_object(
        value,
        &[
            "direction",
            "target",
            "description",
            "keywords",
            "legacyValues",
        ],
        path,
    )?
    else {
        return Ok(());
    };
    let Some(target) = exit.get("target") else {
        return Ok(());
    };
    let allowed = match target.get("kind").and_then(serde_json::Value::as_str) {
        Some("room") => &["kind", "room_id"][..],
        Some("legacy_vnum") => &["kind", "vnum"][..],
        _ => &["kind"][..],
    };
    audit_object(target, allowed, &format!("{path}.target"))?;
    Ok(())
}

fn audit_item_rule(value: &serde_json::Value, path: &str) -> Result<(), CompositionError> {
    let Some(rule) = audit_object(
        value,
        &[
            "itemId",
            "equipmentSlot",
            "attackBonus",
            "healAmount",
            "consumable",
            "effect",
        ],
        path,
    )?
    else {
        return Ok(());
    };
    if let Some(effect) = rule.get("effect") {
        audit_object(
            effect,
            &["kind", "magnitude", "remainingTicks"],
            &format!("{path}.effect"),
        )?;
    }
    Ok(())
}

fn audit_npc_prototype(value: &serde_json::Value, path: &str) -> Result<(), CompositionError> {
    audit_object(
        value,
        &[
            "id",
            "name",
            "description",
            "level",
            "maxHealth",
            "strength",
            "defence",
            "experienceReward",
            "respawnTicks",
            "lootItemIds",
        ],
        path,
    )
    .map(|_| ())
}

fn audit_archetype(value: &serde_json::Value, path: &str) -> Result<(), CompositionError> {
    let Some(archetype) = audit_object(
        value,
        &[
            "id",
            "name",
            "description",
            "initialAttributes",
            "initialSkills",
            "initialResources",
            "resourceRegeneration",
            "initialResistances",
            "initialAbilityIds",
            "levelProgression",
        ],
        path,
    )?
    else {
        return Ok(());
    };
    audit_array(
        archetype.get("levelProgression"),
        &format!("{path}.levelProgression"),
        audit_level_progression,
    )?;
    Ok(())
}

fn audit_level_progression(value: &serde_json::Value, path: &str) -> Result<(), CompositionError> {
    audit_object(
        value,
        &[
            "level",
            "totalExperienceRequired",
            "attributeGains",
            "skillGains",
            "resourceGains",
            "abilityUnlockIds",
        ],
        path,
    )
    .map(|_| ())
}

fn audit_ability(value: &serde_json::Value, path: &str) -> Result<(), CompositionError> {
    let Some(ability) = audit_object(
        value,
        &[
            "id",
            "name",
            "description",
            "category",
            "resourceCosts",
            "cooldownTicks",
            "targeting",
            "damage",
            "heal",
            "statusEffects",
            "communication",
        ],
        path,
    )?
    else {
        return Ok(());
    };
    if let Some(damage) = ability.get("damage") {
        audit_object(
            damage,
            &[
                "target",
                "damageType",
                "baseAmount",
                "scalingAttribute",
                "scalingPercent",
            ],
            &format!("{path}.damage"),
        )?;
    }
    if let Some(heal) = ability.get("heal") {
        audit_object(
            heal,
            &["target", "baseAmount", "scalingAttribute", "scalingPercent"],
            &format!("{path}.heal"),
        )?;
    }
    if let Some(communication) = ability.get("communication") {
        audit_object(
            communication,
            &["kind", "scope", "blockedRoomIds"],
            &format!("{path}.communication"),
        )?;
    }
    audit_array(
        ability.get("statusEffects"),
        &format!("{path}.statusEffects"),
        audit_ability_status,
    )?;
    Ok(())
}

fn audit_social_emote(value: &serde_json::Value, path: &str) -> Result<(), CompositionError> {
    let Some(emote) = audit_object(
        value,
        &[
            "id",
            "command",
            "name",
            "audience",
            "targeting",
            "templates",
        ],
        path,
    )?
    else {
        return Ok(());
    };
    let Some(templates) = emote.get("templates") else {
        return Ok(());
    };
    let Some(templates) = audit_object(
        templates,
        &["untargeted", "targeted"],
        &format!("{path}.templates"),
    )?
    else {
        return Ok(());
    };
    if let Some(untargeted) = templates.get("untargeted") {
        audit_object(
            untargeted,
            &["actor", "others"],
            &format!("{path}.templates.untargeted"),
        )?;
    }
    if let Some(targeted) = templates.get("targeted") {
        audit_object(
            targeted,
            &["actor", "target", "others"],
            &format!("{path}.templates.targeted"),
        )?;
    }
    Ok(())
}

fn audit_ability_status(value: &serde_json::Value, path: &str) -> Result<(), CompositionError> {
    let allowed = match value.get("kind").and_then(serde_json::Value::as_str) {
        Some("attribute_modifier") => {
            &["kind", "target", "attribute", "magnitude", "durationTicks"][..]
        }
        Some("regeneration") => &["kind", "target", "magnitude", "durationTicks"][..],
        Some("damage_over_time") => {
            &["kind", "target", "damageType", "magnitude", "durationTicks"][..]
        }
        Some("stun") => &["kind", "target", "durationTicks"][..],
        _ => &["kind"][..],
    };
    audit_object(value, allowed, path).map(|_| ())
}

fn audit_npc_ability_set(value: &serde_json::Value, path: &str) -> Result<(), CompositionError> {
    audit_object(
        value,
        &[
            "npcPrototypeId",
            "resourcePools",
            "resourceRegeneration",
            "resistances",
            "abilityIds",
        ],
        path,
    )
    .map(|_| ())
}

fn audit_npc_spawn(value: &serde_json::Value, path: &str) -> Result<(), CompositionError> {
    audit_object(value, &["id", "prototypeId", "roomId"], path).map(|_| ())
}

fn audit_loot_table(value: &serde_json::Value, path: &str) -> Result<(), CompositionError> {
    let Some(table) = audit_object(value, &["id", "guaranteed", "pools"], path)? else {
        return Ok(());
    };
    audit_array(
        table.get("guaranteed"),
        &format!("{path}.guaranteed"),
        |value, path| {
            audit_object(
                value,
                &["id", "itemPrototypeId", "quantity", "affixPoolId"],
                path,
            )
            .map(|_| ())
        },
    )?;
    audit_array(
        table.get("pools"),
        &format!("{path}.pools"),
        |value, path| {
            let Some(pool) = audit_object(value, &["id", "rolls", "entries"], path)? else {
                return Ok(());
            };
            audit_array(
                pool.get("entries"),
                &format!("{path}.entries"),
                |value, path| {
                    audit_object(
                        value,
                        &[
                            "kind",
                            "id",
                            "weight",
                            "itemPrototypeId",
                            "minQuantity",
                            "maxQuantity",
                            "affixPoolId",
                        ],
                        path,
                    )
                    .map(|_| ())
                },
            )
        },
    )?;
    Ok(())
}

fn audit_affix(value: &serde_json::Value, path: &str) -> Result<(), CompositionError> {
    audit_object(
        value,
        &[
            "id",
            "label",
            "position",
            "attackBonusDelta",
            "healAmountDelta",
        ],
        path,
    )
    .map(|_| ())
}

fn audit_affix_pool(value: &serde_json::Value, path: &str) -> Result<(), CompositionError> {
    let Some(pool) = audit_object(value, &["id", "entries"], path)? else {
        return Ok(());
    };
    audit_array(
        pool.get("entries"),
        &format!("{path}.entries"),
        |value, path| audit_object(value, &["kind", "id", "weight", "affixId"], path).map(|_| ()),
    )
}

fn audit_loot_source(value: &serde_json::Value, path: &str) -> Result<(), CompositionError> {
    audit_object(
        value,
        &["kind", "id", "npcSpawnId", "lootTableId", "rights"],
        path,
    )
    .map(|_| ())
}

fn audit_room_media(value: &serde_json::Value, path: &str) -> Result<(), CompositionError> {
    let Some(media) = audit_object(value, &["roomId", "imageAlt", "audio"], path)? else {
        return Ok(());
    };
    if let Some(audio) = media.get("audio") {
        audit_audio(audio, &format!("{path}.audio"))?;
    }
    Ok(())
}

fn audit_entity_media(value: &serde_json::Value, path: &str) -> Result<(), CompositionError> {
    let Some(media) = audit_object(
        value,
        &[
            "kind",
            "subjectId",
            "alt",
            "decorative",
            "footprint",
            "preferredAnchor",
            "priority",
            "layer",
            "poses",
        ],
        path,
    )?
    else {
        return Ok(());
    };
    if let Some(footprint) = media.get("footprint") {
        audit_object(
            footprint,
            &["widthMilli", "heightMilli"],
            &format!("{path}.footprint"),
        )?;
    }
    if let Some(anchor) = media.get("preferredAnchor") {
        audit_object(
            anchor,
            &["xMilli", "yMilli"],
            &format!("{path}.preferredAnchor"),
        )?;
    }
    if let Some(poses) = media.get("poses") {
        let Some(poses) = audit_object(
            poses,
            &["idle", "attack", "hit", "defeated"],
            &format!("{path}.poses"),
        )?
        else {
            return Ok(());
        };
        for pose in ["idle", "attack", "hit", "defeated"] {
            if let Some(source) = poses.get(pose) {
                audit_object(source, &["webp", "png"], &format!("{path}.poses.{pose}"))?;
            }
        }
    }
    Ok(())
}

fn audit_audio(value: &serde_json::Value, path: &str) -> Result<(), CompositionError> {
    audit_object(value, &["channel", "path", "loop"], path).map(|_| ())
}

fn audit_room_perception(value: &serde_json::Value, path: &str) -> Result<(), CompositionError> {
    audit_object(
        value,
        &[
            "roomId",
            "light",
            "hiddenItems",
            "hiddenNpcs",
            "secretExits",
        ],
        path,
    )
    .map(|_| ())
}

fn audit_scene_variants(value: &serde_json::Value, path: &str) -> Result<(), CompositionError> {
    let Some(variants) = audit_object(value, &["roomId", "variants"], path)? else {
        return Ok(());
    };
    audit_array(
        variants.get("variants"),
        &format!("{path}.variants"),
        audit_scene_variant,
    )
}

fn audit_scene_variant(value: &serde_json::Value, path: &str) -> Result<(), CompositionError> {
    let Some(variant) = audit_object(
        value,
        &[
            "id",
            "priority",
            "when",
            "title",
            "description",
            "assetManifest",
            "imageAlt",
            "audio",
        ],
        path,
    )?
    else {
        return Ok(());
    };
    if let Some(condition) = variant.get("when") {
        audit_condition(condition, &format!("{path}.when"))?;
    }
    if let Some(audio) = variant.get("audio") {
        audit_audio(audio, &format!("{path}.audio"))?;
    }
    Ok(())
}

fn audit_condition(value: &serde_json::Value, path: &str) -> Result<(), CompositionError> {
    let Some(condition) = audit_object(
        value,
        &[
            "all", "any", "not", "eq", "ne", "in", "gt", "gte", "lt", "lte", "contains",
        ],
        path,
    )?
    else {
        return Ok(());
    };
    for operator in ["all", "any"] {
        audit_array(
            condition.get(operator),
            &format!("{path}.{operator}"),
            audit_condition,
        )?;
    }
    if let Some(nested) = condition.get("not") {
        audit_condition(nested, &format!("{path}.not"))?;
    }
    Ok(())
}

fn audit_script(value: &serde_json::Value, path: &str) -> Result<(), CompositionError> {
    let Some(script) = audit_object(
        value,
        &["id", "roomId", "hooks", "capabilities", "limits", "source"],
        path,
    )?
    else {
        return Ok(());
    };
    if let Some(limits) = script.get("limits") {
        audit_object(
            limits,
            &[
                "maxSourceBytes",
                "maxMemoryBytes",
                "maxInstructions",
                "maxWallTimeMillis",
                "maxEvents",
                "maxEventFieldBytes",
            ],
            &format!("{path}.limits"),
        )?;
    }
    Ok(())
}

fn audit_content_test(value: &serde_json::Value, path: &str) -> Result<(), CompositionError> {
    let Some(test) = audit_object(value, &["name", "roomId", "context", "expect"], path)? else {
        return Ok(());
    };
    if let Some(context) = test.get("context") {
        audit_object(
            context,
            &[
                "worldInstance",
                "worldTimePeriod",
                "worldWeather",
                "worldTimeHour",
                "actorLevel",
                "actorIsAdmin",
                "actorFlags",
                "actorQuestStates",
                "actorContentPreferences",
                "actorSenses",
            ],
            &format!("{path}.context"),
        )?;
    }
    if let Some(expect) = test.get("expect") {
        audit_object(
            expect,
            &[
                "title",
                "descriptionContains",
                "outputExcludes",
                "assetManifest",
                "exits",
                "items",
            ],
            &format!("{path}.expect"),
        )?;
    }
    Ok(())
}

fn audit_object<'a>(
    value: &'a serde_json::Value,
    allowed: &[&str],
    path: &str,
) -> Result<Option<&'a serde_json::Map<String, serde_json::Value>>, CompositionError> {
    let Some(object) = value.as_object() else {
        return Ok(None);
    };
    if let Some(field) = object
        .keys()
        .find(|field| !allowed.contains(&field.as_str()))
    {
        return Err(CompositionError::UnknownAreaField {
            json_path: path.to_owned(),
            field: field.clone(),
        });
    }
    Ok(Some(object))
}

fn audit_array(
    value: Option<&serde_json::Value>,
    path: &str,
    audit: fn(&serde_json::Value, &str) -> Result<(), CompositionError>,
) -> Result<(), CompositionError> {
    let Some(values) = value.and_then(serde_json::Value::as_array) else {
        return Ok(());
    };
    for (index, value) in values.iter().enumerate() {
        audit(value, &format!("{path}[{index}]"))?;
    }
    Ok(())
}

fn safe_relative_path(value: &str) -> bool {
    !value.trim().is_empty()
        && value.len() <= 4096
        && !value.contains('\\')
        && !looks_like_windows_drive(value)
        && Path::new(value)
            .components()
            .all(|component| matches!(component, Component::Normal(_)))
}

fn looks_like_windows_drive(value: &str) -> bool {
    matches!(value.as_bytes(), [drive, b':', ..] if drive.is_ascii_alphabetic())
}

fn sorted(mut values: Vec<String>) -> Vec<String> {
    values.sort();
    values
}

fn sha256_hex(bytes: &[u8]) -> String {
    hex::encode(Sha256::digest(bytes))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strict_top_level_schemas_reject_unknown_fields() {
        let project = br#"{"schemaVersion":1,"id":"demo","name":"Demo","startRoomId":"a:start","areas":[],"extra":true}"#;
        assert!(serde_json::from_slice::<WorldProjectManifest>(project).is_err());
        let area = br#"{"schemaVersion":1,"id":"a","rooms":[],"extra":true}"#;
        assert!(serde_json::from_slice::<AreaPack>(area).is_err());
    }

    #[test]
    fn relative_paths_are_strict_and_portable() {
        for valid in ["areas/a.json", "a.json"] {
            assert!(safe_relative_path(valid), "{valid}");
        }
        for invalid in [
            "",
            ".",
            "../a.json",
            "areas/../a.json",
            "/a.json",
            "C:/a.json",
            "C:\\a.json",
        ] {
            assert!(!safe_relative_path(invalid), "{invalid}");
        }
    }
}
