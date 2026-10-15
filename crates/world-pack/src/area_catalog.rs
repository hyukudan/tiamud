//! Strict, bounded, offline catalog and immutable bundle formats.
//!
//! These records deliberately do not implement hot loading. They are the
//! content-addressed publication boundary required before runtime lifecycle
//! work can safely begin.

use std::collections::{BTreeMap, HashMap, HashSet};

use serde::{
    Deserialize, Deserializer, Serialize,
    de::{MapAccess, SeqAccess, Visitor},
};
use serde_json::{Map, Value};
use sha2::{Digest, Sha256};
use thiserror::Error;

use crate::{
    AREA_PACK_SCHEMA_VERSION, AreaCrossReferenceKind, AreaOwnership, AreaPack,
    CURRENT_SCHEMA_VERSION, ComposedWorld,
    composition::{cross_area_references_for_packs, ownership_for_area},
};

pub const AREA_RELEASE_SCHEMA_VERSION: u32 = 1;
pub const AREA_BUNDLE_SCHEMA_VERSION: u32 = 1;
pub const AREA_CATALOG_SCHEMA_VERSION: u32 = 1;
pub const MAX_AREA_CATALOG_BYTES: usize = 1024 * 1024;
pub const MAX_AREA_BUNDLE_BYTES: usize = 16 * 1024 * 1024;
pub const MAX_AREA_RELEASE_BYTES: usize = 256 * 1024;
const MAX_JSON_DEPTH: usize = 16;
const MAX_REVIEWER_BYTES: usize = 128;
const MAX_LICENSE_BYTES: usize = 128;
const MAX_PROVENANCE_BYTES: usize = 2_048;
const CONTENT_LICENSE: &str = "CC-BY-4.0";
const SCRIPT_LICENSE: &str = "MIT OR Apache-2.0";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReleaseTrustMode {
    UnsignedLocal,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AreaReleaseManifest {
    pub schema_version: u32,
    pub trust_mode: ReleaseTrustMode,
    pub catalog_revision: u64,
    pub areas: Vec<AreaReleaseEntry>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AreaReleaseEntry {
    pub area_id: String,
    pub revision: u64,
    pub content_license: String,
    pub script_license: String,
    pub reviewer: String,
    pub reviewed_at: String,
    pub provenance: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ContentDigest {
    pub id: String,
    pub sha256: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AreaCompatibility {
    pub world_pack_schema_version: u32,
    pub persistent_migration_version: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AreaBundle {
    pub schema_version: u32,
    pub trust_mode: ReleaseTrustMode,
    pub world_id: String,
    pub area_id: String,
    pub revision: u64,
    pub area_pack_schema_version: u32,
    pub source_sha256: String,
    pub canonical_area_pack_sha256: String,
    pub compiled_area_sha256: String,
    pub content_license: String,
    pub script_license: String,
    pub reviewer: String,
    pub reviewed_at: String,
    pub provenance: String,
    pub compatibility: AreaCompatibility,
    pub asset_manifests: Vec<ContentDigest>,
    pub lua_modules: Vec<ContentDigest>,
    pub compiled_area: CompiledArea,
    pub area_pack: AreaPack,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CompiledArea {
    pub item_prototypes: Vec<crate::ItemPrototype>,
    pub initial_item_instances: Vec<crate::InitialItemInstance>,
    pub rooms: Vec<crate::Room>,
    pub npc_prototypes: Vec<crate::NpcPrototype>,
    pub npc_spawns: Vec<crate::NpcSpawn>,
    pub loot_tables: Vec<crate::LootTableDefinition>,
    pub loot_sources: Vec<crate::LootSourceDefinition>,
    pub affixes: Vec<crate::AffixDefinition>,
    pub affix_pools: Vec<crate::AffixPoolDefinition>,
    pub archetypes: Vec<crate::ArchetypeDefinition>,
    pub level_thresholds: Vec<crate::LevelThresholdDefinition>,
    pub lineages: Vec<crate::LineageDefinition>,
    pub backgrounds: Vec<crate::BackgroundDefinition>,
    pub base_classes: Vec<crate::BaseClassDefinition>,
    pub advanced_classes: Vec<crate::AdvancedClassDefinition>,
    pub talent_graphs: Vec<crate::TalentGraphDefinition>,
    pub abilities: Vec<crate::AbilityDefinition>,
    pub social_emotes: Vec<crate::SocialEmoteDefinition>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub commodities: Vec<crate::CommodityDefinition>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub quests: Vec<crate::QuestDefinition>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub shops: Vec<crate::ShopDefinition>,
    pub npc_ability_sets: Vec<crate::NpcAbilitySet>,
    pub room_media: Vec<crate::RoomMediaDefinition>,
    pub entity_media: Vec<crate::EntityMediaDefinition>,
    pub room_perception: Vec<crate::RoomPerceptionDefinition>,
    pub scene_variants: Vec<crate::RoomSceneVariants>,
    pub scripts: Vec<crate::ScriptDefinition>,
    pub content_tests: Vec<crate::ContentTest>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AreaPortalDirection {
    Incoming,
    Outgoing,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AreaPortal {
    pub direction: AreaPortalDirection,
    pub source_exit: crate::Direction,
    pub peer_area_id: String,
    pub source_room_id: String,
    pub target_room_id: String,
    pub minimum_peer_revision: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AreaCatalogEntry {
    pub area_id: String,
    pub revision: u64,
    pub bundle_sha256: String,
    pub stored_bytes: u64,
    pub expanded_bytes: u64,
    pub estimated_memory_bytes: u64,
    pub area_pack_schema_version: u32,
    pub compatibility: AreaCompatibility,
    pub source_sha256: String,
    pub compiled_area_sha256: String,
    pub content_license: String,
    pub script_license: String,
    pub reviewer: String,
    pub reviewed_at: String,
    pub provenance: String,
    pub ownership: AreaOwnership,
    pub hard_dependencies: Vec<String>,
    pub soft_dependencies: Vec<String>,
    pub portals: Vec<AreaPortal>,
    pub asset_manifests: Vec<ContentDigest>,
    pub lua_modules: Vec<ContentDigest>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AreaCatalog {
    pub schema_version: u32,
    pub trust_mode: ReleaseTrustMode,
    pub catalog_revision: u64,
    pub world_id: String,
    pub project_manifest_sha256: String,
    pub world_artifact_sha256: String,
    pub world_artifact_bytes: u64,
    pub start_room_id: String,
    pub entries: Vec<AreaCatalogEntry>,
}

#[derive(Debug, Clone)]
pub struct VerifiedAreaRelease {
    pub catalog: AreaCatalog,
    pub catalog_bytes: Vec<u8>,
    pub catalog_sha256: String,
    pub world_artifact: Vec<u8>,
    pub bundles: BTreeMap<String, Vec<u8>>,
}

#[derive(Debug, Error)]
pub enum AreaCatalogError {
    #[error("{kind} contains {size} bytes; the limit is {maximum} bytes")]
    TooLarge {
        kind: &'static str,
        size: usize,
        maximum: usize,
    },
    #[error("invalid {kind} JSON: {source}")]
    Json {
        kind: &'static str,
        source: serde_json::Error,
    },
    #[error("invalid area release: {0}")]
    Invalid(String),
    #[error("release metadata is missing for area {0:?}")]
    MissingReleaseMetadata(String),
    #[error("release metadata names unknown area {0:?}")]
    UnknownReleaseArea(String),
    #[error("bundle is missing for area {0:?}")]
    MissingBundle(String),
    #[error("bundle digest mismatch for area {0:?}")]
    BundleDigest(String),
    #[error("area {area_id:?} changed without a strictly newer revision")]
    RevisionNotAdvanced { area_id: String },
    #[error("could not serialize canonical {kind}: {source}")]
    Encode {
        kind: &'static str,
        source: serde_json::Error,
    },
}

pub fn parse_area_release_manifest(bytes: &[u8]) -> Result<AreaReleaseManifest, AreaCatalogError> {
    parse_bounded(bytes, MAX_AREA_RELEASE_BYTES, "area release manifest")
}

pub fn parse_area_bundle(bytes: &[u8]) -> Result<AreaBundle, AreaCatalogError> {
    let value = parse_bounded_value(bytes, MAX_AREA_BUNDLE_BYTES, "area bundle")?;
    let area = value
        .get("areaPack")
        .ok_or_else(|| AreaCatalogError::Invalid("area bundle is missing areaPack".into()))?;
    if area.get("schemaVersion").and_then(Value::as_u64) != Some(AREA_PACK_SCHEMA_VERSION as u64) {
        return Err(AreaCatalogError::Invalid(
            "area bundle must contain raw AreaPack v8".into(),
        ));
    }
    crate::composition::audit_area_pack_keys(area)
        .map_err(|error| AreaCatalogError::Invalid(format!("invalid bundled AreaPack: {error}")))?;
    let bundle: AreaBundle =
        serde_json::from_value(value).map_err(|source| AreaCatalogError::Json {
            kind: "area bundle",
            source,
        })?;
    if canonical_json(&bundle, "area bundle")? != bytes {
        return Err(AreaCatalogError::Invalid(
            "area bundle bytes are not canonical".into(),
        ));
    }
    Ok(bundle)
}

pub fn parse_area_catalog(bytes: &[u8]) -> Result<AreaCatalog, AreaCatalogError> {
    let catalog: AreaCatalog = parse_bounded(bytes, MAX_AREA_CATALOG_BYTES, "area catalog")?;
    if canonical_json(&catalog, "area catalog")? != bytes {
        return Err(AreaCatalogError::Invalid(
            "area catalog bytes are not canonical".into(),
        ));
    }
    Ok(catalog)
}

pub fn build_area_release(
    composed: &ComposedWorld,
    release: &AreaReleaseManifest,
    previous: Option<&AreaCatalog>,
) -> Result<VerifiedAreaRelease, AreaCatalogError> {
    validate_release_manifest(release, &composed.area_packs)?;
    if let Some(previous) = previous {
        if previous.world_id != composed.report.project_id {
            return Err(AreaCatalogError::Invalid(
                "previous catalog belongs to another world".into(),
            ));
        }
        if release.catalog_revision <= previous.catalog_revision {
            return Err(AreaCatalogError::Invalid(format!(
                "catalog revision {} must be newer than active revision {}",
                release.catalog_revision, previous.catalog_revision
            )));
        }
    }
    let metadata = release
        .areas
        .iter()
        .map(|entry| (entry.area_id.as_str(), entry))
        .collect::<HashMap<_, _>>();
    let reports = composed
        .report
        .areas
        .iter()
        .map(|entry| (entry.id.as_str(), entry))
        .collect::<HashMap<_, _>>();
    let canonical_document =
        crate::WorldPackDocument::from_json(&composed.artifact).map_err(|error| {
            AreaCatalogError::Invalid(format!("could not decode composed WorldPack: {error}"))
        })?;
    let mut bundles = BTreeMap::new();
    let mut entries = Vec::with_capacity(composed.area_packs.len());

    for pack in &composed.area_packs {
        let release_entry = metadata[pack.id.as_str()];
        let report = reports[pack.id.as_str()];
        let area_bytes = canonical_json(pack, "AreaPack")?;
        let compiled_area = world_projection(&canonical_document, &report.ownership, pack);
        let compiled_area_bytes = canonical_json(&compiled_area, "compiled area")?;
        let mut lua_modules = pack
            .scripts
            .iter()
            .map(|script| ContentDigest {
                id: script.id.clone(),
                sha256: sha256_hex(script.source.as_bytes()),
            })
            .collect::<Vec<_>>();
        lua_modules.sort_by(|a, b| a.id.cmp(&b.id));
        let bundle = AreaBundle {
            schema_version: AREA_BUNDLE_SCHEMA_VERSION,
            trust_mode: release.trust_mode,
            world_id: composed.report.project_id.clone(),
            area_id: pack.id.clone(),
            revision: release_entry.revision,
            area_pack_schema_version: AREA_PACK_SCHEMA_VERSION,
            source_sha256: report.source_sha256.clone(),
            canonical_area_pack_sha256: sha256_hex(&area_bytes),
            compiled_area_sha256: sha256_hex(&compiled_area_bytes),
            content_license: release_entry.content_license.clone(),
            script_license: release_entry.script_license.clone(),
            reviewer: release_entry.reviewer.clone(),
            reviewed_at: release_entry.reviewed_at.clone(),
            provenance: release_entry.provenance.clone(),
            compatibility: AreaCompatibility {
                world_pack_schema_version: CURRENT_SCHEMA_VERSION,
                persistent_migration_version: 0,
            },
            asset_manifests: report
                .asset_manifests
                .iter()
                .map(|manifest| ContentDigest {
                    id: manifest.path.clone(),
                    sha256: manifest.sha256.clone(),
                })
                .collect(),
            lua_modules: lua_modules.clone(),
            compiled_area,
            area_pack: pack.clone(),
        };
        let bundle_bytes = canonical_json(&bundle, "area bundle")?;
        if bundle_bytes.len() > MAX_AREA_BUNDLE_BYTES {
            return Err(AreaCatalogError::TooLarge {
                kind: "area bundle",
                size: bundle_bytes.len(),
                maximum: MAX_AREA_BUNDLE_BYTES,
            });
        }
        let bundle_sha256 = sha256_hex(&bundle_bytes);
        if let Some(old) = previous.and_then(|catalog| {
            catalog
                .entries
                .iter()
                .find(|entry| entry.area_id == pack.id)
        }) {
            if old.bundle_sha256 != bundle_sha256 && release_entry.revision <= old.revision {
                return Err(AreaCatalogError::RevisionNotAdvanced {
                    area_id: pack.id.clone(),
                });
            }
            if old.bundle_sha256 == bundle_sha256 && release_entry.revision != old.revision {
                return Err(AreaCatalogError::Invalid(format!(
                    "unchanged area {:?} must preserve revision {}",
                    pack.id, old.revision
                )));
            }
        }
        entries.push(AreaCatalogEntry {
            area_id: pack.id.clone(),
            revision: release_entry.revision,
            bundle_sha256: bundle_sha256.clone(),
            stored_bytes: bundle_bytes.len() as u64,
            expanded_bytes: bundle_bytes.len() as u64,
            estimated_memory_bytes: (area_bytes.len() as u64).saturating_mul(2),
            area_pack_schema_version: AREA_PACK_SCHEMA_VERSION,
            compatibility: bundle.compatibility,
            source_sha256: report.source_sha256.clone(),
            compiled_area_sha256: bundle.compiled_area_sha256.clone(),
            content_license: release_entry.content_license.clone(),
            script_license: release_entry.script_license.clone(),
            reviewer: release_entry.reviewer.clone(),
            reviewed_at: release_entry.reviewed_at.clone(),
            provenance: release_entry.provenance.clone(),
            ownership: report.ownership.clone(),
            hard_dependencies: Vec::new(),
            soft_dependencies: Vec::new(),
            portals: Vec::new(),
            asset_manifests: bundle.asset_manifests.clone(),
            lua_modules,
        });
        bundles.insert(pack.id.clone(), bundle_bytes);
    }
    entries.sort_by(|a, b| a.area_id.cmp(&b.area_id));
    attach_runtime_relationships(
        &mut entries,
        &composed.report.cross_area_references,
        &composed.area_packs,
    )?;
    let catalog = AreaCatalog {
        schema_version: AREA_CATALOG_SCHEMA_VERSION,
        trust_mode: release.trust_mode,
        catalog_revision: release.catalog_revision,
        world_id: composed.report.project_id.clone(),
        project_manifest_sha256: composed.report.project_manifest_sha256.clone(),
        world_artifact_sha256: composed.report.artifact_sha256.clone(),
        world_artifact_bytes: composed.artifact.len() as u64,
        start_room_id: composed.document.world.start_room_id.clone(),
        entries,
    };
    let catalog_bytes = canonical_json(&catalog, "area catalog")?;
    if catalog_bytes.len() > MAX_AREA_CATALOG_BYTES {
        return Err(AreaCatalogError::TooLarge {
            kind: "area catalog",
            size: catalog_bytes.len(),
            maximum: MAX_AREA_CATALOG_BYTES,
        });
    }
    let verified = VerifiedAreaRelease {
        catalog,
        catalog_sha256: sha256_hex(&catalog_bytes),
        catalog_bytes,
        world_artifact: composed.artifact.clone(),
        bundles,
    };
    verify_area_release(
        &verified.catalog,
        &verified.bundles,
        &verified.world_artifact,
    )?;
    Ok(verified)
}

pub fn verify_area_release(
    catalog: &AreaCatalog,
    bundles: &BTreeMap<String, Vec<u8>>,
    world_artifact: &[u8],
) -> Result<(), AreaCatalogError> {
    let document = verify_area_catalog_fallback(catalog, world_artifact)?;
    if bundles.len() != catalog.entries.len() {
        return Err(AreaCatalogError::Invalid(
            "bundle set does not exactly match catalog entries".into(),
        ));
    }
    let mut packs = Vec::with_capacity(catalog.entries.len());
    let mut total_bundle_bytes = 0usize;
    for entry in &catalog.entries {
        let bytes = bundles
            .get(&entry.area_id)
            .ok_or_else(|| AreaCatalogError::MissingBundle(entry.area_id.clone()))?;
        total_bundle_bytes = total_bundle_bytes
            .checked_add(bytes.len())
            .ok_or_else(|| AreaCatalogError::Invalid("bundle byte sum overflow".into()))?;
        if total_bundle_bytes > crate::MAX_TOTAL_AREA_BYTES as usize {
            return Err(AreaCatalogError::TooLarge {
                kind: "area bundle set",
                size: total_bundle_bytes,
                maximum: crate::MAX_TOTAL_AREA_BYTES as usize,
            });
        }
        let bundle = verify_area_bundle_against_catalog(catalog, &document, entry, bytes)?;
        packs.push(bundle.area_pack);
    }
    // The compiled projections prove what a future area runtime would load,
    // but the embedded raw AreaPack is also a reviewed publication
    // surface. Prove that every authored domain in those packs is exactly the
    // content from which the monolithic fallback was composed. Without this
    // check a self-consistent catalog could retain the expected compiled area
    // while smuggling different room prose, progression, media or tests in its
    // raw AreaPack copy.
    verify_monolithic_content(&document, &packs)?;
    let references = cross_area_references_for_packs(&packs);
    validate_declared_dependencies(&packs, &references)?;
    let mut expected_entries = catalog.entries.clone();
    for entry in &mut expected_entries {
        entry.hard_dependencies.clear();
        entry.portals.clear();
    }
    attach_runtime_relationships(&mut expected_entries, &references, &packs)?;
    for (actual, expected) in catalog.entries.iter().zip(expected_entries.iter()) {
        if actual.portals != expected.portals
            || actual.hard_dependencies != expected.hard_dependencies
        {
            return Err(AreaCatalogError::Invalid(format!(
                "runtime relationship mismatch for area {:?}",
                actual.area_id
            )));
        }
    }
    Ok(())
}

pub(crate) fn verify_area_catalog_fallback(
    catalog: &AreaCatalog,
    world_artifact: &[u8],
) -> Result<crate::WorldPackDocument, AreaCatalogError> {
    validate_catalog_header(catalog)?;
    if world_artifact.len() as u64 != catalog.world_artifact_bytes
        || sha256_hex(world_artifact) != catalog.world_artifact_sha256
    {
        return Err(AreaCatalogError::Invalid(
            "monolithic WorldPack fallback digest/size mismatch".into(),
        ));
    }
    let document = crate::WorldPackDocument::from_json(world_artifact).map_err(|error| {
        AreaCatalogError::Invalid(format!("invalid monolithic WorldPack fallback: {error}"))
    })?;
    document.validate().map_err(|error| {
        AreaCatalogError::Invalid(format!("invalid monolithic WorldPack fallback: {error}"))
    })?;
    let canonical_world = document.deterministic_artifact().map_err(|error| {
        AreaCatalogError::Invalid(format!(
            "could not canonicalize WorldPack fallback: {error}"
        ))
    })?;
    if canonical_world != world_artifact {
        return Err(AreaCatalogError::Invalid(
            "monolithic WorldPack fallback bytes are not canonical".into(),
        ));
    }
    if document.world.id != catalog.world_id
        || document.world.start_room_id != catalog.start_room_id
    {
        return Err(AreaCatalogError::Invalid(
            "monolithic WorldPack identity/start room mismatch".into(),
        ));
    }
    validate_catalog_structure(catalog, &document)?;
    validate_container_materialization_closures(catalog, &document)?;
    Ok(document)
}

fn validate_container_materialization_closures(
    catalog: &AreaCatalog,
    document: &crate::WorldPackDocument,
) -> Result<(), AreaCatalogError> {
    let owners = catalog
        .entries
        .iter()
        .flat_map(|entry| {
            entry
                .ownership
                .initial_item_instances
                .iter()
                .map(move |item_id| (item_id.as_str(), entry.area_id.as_str()))
        })
        .collect::<HashMap<_, _>>();
    let entries = catalog
        .entries
        .iter()
        .map(|entry| (entry.area_id.as_str(), entry))
        .collect::<HashMap<_, _>>();
    for item in &document.world.initial_item_instances {
        let crate::InitialItemLocation::Container {
            container_instance_id,
        } = &item.location
        else {
            continue;
        };
        let child_owner = owners.get(item.id.as_str()).copied().ok_or_else(|| {
            AreaCatalogError::Invalid("container child has no catalog owner".into())
        })?;
        let container_owner = owners
            .get(container_instance_id.as_str())
            .copied()
            .ok_or_else(|| {
                AreaCatalogError::Invalid("container instance has no catalog owner".into())
            })?;
        if child_owner == container_owner {
            continue;
        }
        let child = entries[child_owner];
        let container = entries[container_owner];
        if !child
            .hard_dependencies
            .iter()
            .any(|area| area == container_owner)
            || !container
                .hard_dependencies
                .iter()
                .any(|area| area == child_owner)
        {
            return Err(AreaCatalogError::Invalid(format!(
                "cross-area container {:?} and child {:?} must share one atomic hard-dependency closure",
                container_instance_id, item.id
            )));
        }
    }
    Ok(())
}

pub(crate) fn verify_area_bundle_against_catalog(
    catalog: &AreaCatalog,
    document: &crate::WorldPackDocument,
    entry: &AreaCatalogEntry,
    bytes: &[u8],
) -> Result<AreaBundle, AreaCatalogError> {
    if bytes.len() as u64 != entry.stored_bytes || sha256_hex(bytes) != entry.bundle_sha256 {
        return Err(AreaCatalogError::BundleDigest(entry.area_id.clone()));
    }
    let bundle = parse_area_bundle(bytes)?;
    validate_bundle(&bundle, entry, catalog, document)?;
    if ownership_for_area(&bundle.area_pack) != entry.ownership {
        return Err(AreaCatalogError::Invalid(format!(
            "ownership mismatch for area {:?}",
            entry.area_id
        )));
    }
    verify_single_area_content(document, entry, &bundle.area_pack)?;
    Ok(bundle)
}

fn validate_catalog_structure(
    catalog: &AreaCatalog,
    document: &crate::WorldPackDocument,
) -> Result<(), AreaCatalogError> {
    let mut area_ids = HashSet::new();
    for entry in &catalog.entries {
        if !area_ids.insert(entry.area_id.as_str()) {
            return Err(AreaCatalogError::Invalid(format!(
                "duplicate catalog area {:?}",
                entry.area_id
            )));
        }
        validate_catalog_entry(entry)?;
        validate_ownership_order(entry)?;
    }
    validate_world_coverage(document, &catalog.entries)?;
    validate_global_ownership(&catalog.entries)?;
    validate_catalog_relationships(catalog)?;
    let owners = catalog
        .entries
        .iter()
        .filter(|entry| entry.ownership.rooms.contains(&catalog.start_room_id))
        .count();
    if owners != 1 {
        return Err(AreaCatalogError::Invalid(
            "start room must have exactly one area owner".into(),
        ));
    }
    Ok(())
}

fn validate_ownership_order(entry: &AreaCatalogEntry) -> Result<(), AreaCatalogError> {
    for ids in [
        &entry.ownership.rooms,
        &entry.ownership.item_prototypes,
        &entry.ownership.initial_item_instances,
        &entry.ownership.npc_prototypes,
        &entry.ownership.npc_spawns,
        &entry.ownership.loot_tables,
        &entry.ownership.loot_sources,
        &entry.ownership.affixes,
        &entry.ownership.affix_pools,
        &entry.ownership.archetypes,
        &entry.ownership.lineages,
        &entry.ownership.backgrounds,
        &entry.ownership.base_classes,
        &entry.ownership.advanced_classes,
        &entry.ownership.talent_graphs,
        &entry.ownership.talent_nodes,
        &entry.ownership.abilities,
        &entry.ownership.social_emotes,
        &entry.ownership.commodities,
        &entry.ownership.quests,
        &entry.ownership.shops,
        &entry.ownership.npc_ability_sets,
        &entry.ownership.room_media,
        &entry.ownership.entity_media,
        &entry.ownership.room_perception,
        &entry.ownership.scene_variant_sets,
        &entry.ownership.scene_variants,
        &entry.ownership.scripts,
        &entry.ownership.content_tests,
    ] {
        if ids.windows(2).any(|pair| pair[0] >= pair[1]) {
            return Err(AreaCatalogError::Invalid(format!(
                "ownership for area {:?} must be strictly sorted",
                entry.area_id
            )));
        }
    }
    Ok(())
}

fn validate_release_manifest(
    release: &AreaReleaseManifest,
    packs: &[AreaPack],
) -> Result<(), AreaCatalogError> {
    if release.schema_version != AREA_RELEASE_SCHEMA_VERSION || release.catalog_revision == 0 {
        return Err(AreaCatalogError::Invalid(
            "unsupported release schema or zero catalog revision".into(),
        ));
    }
    if release.areas.len() != packs.len()
        || release.areas.is_empty()
        || release.areas.len() > crate::MAX_AREA_COUNT
    {
        return Err(AreaCatalogError::Invalid(
            "release must name every composed area exactly once".into(),
        ));
    }
    let pack_ids = packs
        .iter()
        .map(|pack| pack.id.as_str())
        .collect::<HashSet<_>>();
    let mut ids = HashSet::new();
    for entry in &release.areas {
        if !ids.insert(entry.area_id.as_str()) {
            return Err(AreaCatalogError::Invalid(format!(
                "duplicate release area {:?}",
                entry.area_id
            )));
        }
        if !pack_ids.contains(entry.area_id.as_str()) {
            return Err(AreaCatalogError::UnknownReleaseArea(entry.area_id.clone()));
        }
        if entry.revision == 0
            || !bounded_text(&entry.content_license, MAX_LICENSE_BYTES)
            || !bounded_text(&entry.script_license, MAX_LICENSE_BYTES)
            || entry.content_license != CONTENT_LICENSE
            || entry.script_license != SCRIPT_LICENSE
            || !bounded_text(&entry.reviewer, MAX_REVIEWER_BYTES)
            || !valid_reviewed_at(&entry.reviewed_at)
            || !bounded_text(&entry.provenance, MAX_PROVENANCE_BYTES)
        {
            return Err(AreaCatalogError::Invalid(format!(
                "invalid release metadata for area {:?}",
                entry.area_id
            )));
        }
    }
    if let Some(pack) = packs.iter().find(|pack| !ids.contains(pack.id.as_str())) {
        return Err(AreaCatalogError::MissingReleaseMetadata(pack.id.clone()));
    }
    Ok(())
}

fn validate_catalog_header(catalog: &AreaCatalog) -> Result<(), AreaCatalogError> {
    if catalog.schema_version != AREA_CATALOG_SCHEMA_VERSION
        || catalog.catalog_revision == 0
        || catalog.entries.is_empty()
        || catalog.entries.len() > crate::MAX_AREA_COUNT
        || !valid_digest(&catalog.project_manifest_sha256)
        || !valid_digest(&catalog.world_artifact_sha256)
        || catalog.world_artifact_bytes == 0
        || catalog.world_artifact_bytes > crate::MAX_COMPOSED_WORLD_BYTES as u64
        || !safe_identifier(&catalog.world_id)
        || !safe_identifier(&catalog.start_room_id)
    {
        return Err(AreaCatalogError::Invalid("invalid catalog header".into()));
    }
    if catalog
        .entries
        .windows(2)
        .any(|pair| pair[0].area_id >= pair[1].area_id)
    {
        return Err(AreaCatalogError::Invalid(
            "catalog entries must be strictly sorted".into(),
        ));
    }
    Ok(())
}

fn validate_catalog_entry(entry: &AreaCatalogEntry) -> Result<(), AreaCatalogError> {
    if !safe_identifier(&entry.area_id)
        || entry.revision == 0
        || entry.stored_bytes == 0
        || entry.stored_bytes > MAX_AREA_BUNDLE_BYTES as u64
        || entry.expanded_bytes != entry.stored_bytes
        || entry.estimated_memory_bytes == 0
        || entry.area_pack_schema_version != AREA_PACK_SCHEMA_VERSION
        || entry.compatibility.world_pack_schema_version != CURRENT_SCHEMA_VERSION
        || entry.compatibility.persistent_migration_version != 0
        || !entry.soft_dependencies.is_empty()
        || !bounded_text(&entry.content_license, MAX_LICENSE_BYTES)
        || !bounded_text(&entry.script_license, MAX_LICENSE_BYTES)
        || entry.content_license != CONTENT_LICENSE
        || entry.script_license != SCRIPT_LICENSE
        || !bounded_text(&entry.reviewer, MAX_REVIEWER_BYTES)
        || !valid_reviewed_at(&entry.reviewed_at)
        || !bounded_text(&entry.provenance, MAX_PROVENANCE_BYTES)
        || !valid_digest(&entry.bundle_sha256)
        || !valid_digest(&entry.source_sha256)
        || !valid_digest(&entry.compiled_area_sha256)
        || !strict_content_digests(&entry.asset_manifests)
        || !strict_content_digests(&entry.lua_modules)
    {
        return Err(AreaCatalogError::Invalid(format!(
            "invalid bounded catalog metadata for area {:?}",
            entry.area_id
        )));
    }
    Ok(())
}

fn validate_catalog_relationships(catalog: &AreaCatalog) -> Result<(), AreaCatalogError> {
    let entries = catalog
        .entries
        .iter()
        .map(|entry| (entry.area_id.as_str(), entry))
        .collect::<HashMap<_, _>>();
    for entry in &catalog.entries {
        if entry
            .hard_dependencies
            .windows(2)
            .any(|pair| pair[0] >= pair[1])
            || entry.portals.windows(2).any(|pair| pair[0] >= pair[1])
        {
            return Err(AreaCatalogError::Invalid(format!(
                "runtime relationships for area {:?} must be strictly sorted",
                entry.area_id
            )));
        }
        for dependency in &entry.hard_dependencies {
            if dependency == &entry.area_id || !entries.contains_key(dependency.as_str()) {
                return Err(AreaCatalogError::Invalid(format!(
                    "invalid hard dependency {dependency:?} for area {:?}",
                    entry.area_id
                )));
            }
        }
        for portal in &entry.portals {
            let Some(peer) = entries.get(portal.peer_area_id.as_str()).copied() else {
                return Err(AreaCatalogError::Invalid(format!(
                    "portal from area {:?} names unknown peer {:?}",
                    entry.area_id, portal.peer_area_id
                )));
            };
            if peer.area_id == entry.area_id
                || portal.minimum_peer_revision != peer.revision
                || !safe_identifier(&portal.source_room_id)
                || !safe_identifier(&portal.target_room_id)
            {
                return Err(AreaCatalogError::Invalid(format!(
                    "invalid portal metadata for area {:?}",
                    entry.area_id
                )));
            }
            let (source_owner, target_owner) = match portal.direction {
                AreaPortalDirection::Outgoing => (entry, peer),
                AreaPortalDirection::Incoming => (peer, entry),
            };
            if source_owner
                .ownership
                .rooms
                .binary_search(&portal.source_room_id)
                .is_err()
                || target_owner
                    .ownership
                    .rooms
                    .binary_search(&portal.target_room_id)
                    .is_err()
            {
                return Err(AreaCatalogError::Invalid(format!(
                    "portal room ownership mismatch for area {:?}",
                    entry.area_id
                )));
            }
            let reciprocal_direction = match portal.direction {
                AreaPortalDirection::Incoming => AreaPortalDirection::Outgoing,
                AreaPortalDirection::Outgoing => AreaPortalDirection::Incoming,
            };
            if !peer.portals.iter().any(|candidate| {
                candidate.direction == reciprocal_direction
                    && candidate.source_exit == portal.source_exit
                    && candidate.peer_area_id == entry.area_id
                    && candidate.source_room_id == portal.source_room_id
                    && candidate.target_room_id == portal.target_room_id
                    && candidate.minimum_peer_revision == entry.revision
            }) {
                return Err(AreaCatalogError::Invalid(format!(
                    "portal from area {:?} has no exact reciprocal entry",
                    entry.area_id
                )));
            }
        }
    }
    Ok(())
}

fn validate_bundle(
    bundle: &AreaBundle,
    entry: &AreaCatalogEntry,
    catalog: &AreaCatalog,
    document: &crate::WorldPackDocument,
) -> Result<(), AreaCatalogError> {
    validate_catalog_entry(entry)?;
    if bundle.schema_version != AREA_BUNDLE_SCHEMA_VERSION
        || bundle.trust_mode != catalog.trust_mode
        || bundle.area_pack_schema_version != AREA_PACK_SCHEMA_VERSION
        || bundle.area_pack.schema_version != AREA_PACK_SCHEMA_VERSION
        || bundle.area_id != bundle.area_pack.id
        || bundle.area_id != entry.area_id
        || bundle.revision != entry.revision
        || bundle.world_id != catalog.world_id
        || bundle.compatibility != entry.compatibility
        || bundle.source_sha256 != entry.source_sha256
        || bundle.compiled_area_sha256 != entry.compiled_area_sha256
        || bundle.content_license != entry.content_license
        || bundle.script_license != entry.script_license
        || bundle.reviewer != entry.reviewer
        || bundle.reviewed_at != entry.reviewed_at
        || bundle.provenance != entry.provenance
        || bundle.asset_manifests != entry.asset_manifests
        || bundle.lua_modules != entry.lua_modules
    {
        return Err(AreaCatalogError::Invalid(format!(
            "bundle metadata mismatch for area {:?}",
            entry.area_id
        )));
    }
    if bundle.compatibility.world_pack_schema_version != CURRENT_SCHEMA_VERSION
        || bundle.compatibility.persistent_migration_version != 0
        || !valid_digest(&bundle.source_sha256)
        || !valid_digest(&bundle.canonical_area_pack_sha256)
    {
        return Err(AreaCatalogError::Invalid(format!(
            "invalid bundle compatibility/digest for area {:?}",
            entry.area_id
        )));
    }
    let area_bytes = canonical_json(&bundle.area_pack, "AreaPack")?;
    if sha256_hex(&area_bytes) != bundle.canonical_area_pack_sha256 {
        return Err(AreaCatalogError::Invalid(format!(
            "canonical AreaPack digest mismatch for area {:?}",
            entry.area_id
        )));
    }
    let compiled_bytes = canonical_json(&bundle.compiled_area, "compiled area")?;
    if sha256_hex(&compiled_bytes) != bundle.compiled_area_sha256
        || bundle.compiled_area != world_projection(document, &entry.ownership, &bundle.area_pack)
    {
        return Err(AreaCatalogError::Invalid(format!(
            "compiled area/WorldPack mismatch for area {:?}",
            entry.area_id
        )));
    }
    let expected_lua = bundle
        .area_pack
        .scripts
        .iter()
        .map(|script| ContentDigest {
            id: script.id.clone(),
            sha256: sha256_hex(script.source.as_bytes()),
        })
        .collect::<Vec<_>>();
    let mut expected_lua = expected_lua;
    expected_lua.sort_by(|a, b| a.id.cmp(&b.id));
    if expected_lua != bundle.lua_modules {
        return Err(AreaCatalogError::Invalid(format!(
            "Lua digest mismatch for area {:?}",
            entry.area_id
        )));
    }
    let mut expected_asset_ids = bundle
        .area_pack
        .rooms
        .iter()
        .filter_map(|room| room.asset_manifest.clone())
        .chain(
            bundle
                .area_pack
                .scene_variants
                .iter()
                .flat_map(|set| &set.variants)
                .filter_map(|variant| variant.asset_manifest.clone()),
        )
        .collect::<Vec<_>>();
    expected_asset_ids.sort();
    expected_asset_ids.dedup();
    if expected_asset_ids
        != bundle
            .asset_manifests
            .iter()
            .map(|manifest| manifest.id.clone())
            .collect::<Vec<_>>()
    {
        return Err(AreaCatalogError::Invalid(format!(
            "asset manifest references mismatch for area {:?}",
            entry.area_id
        )));
    }
    let expected_expanded = entry.stored_bytes;
    let expected_memory = (area_bytes.len() as u64).saturating_mul(2);
    if entry.expanded_bytes != expected_expanded
        || entry.estimated_memory_bytes != expected_memory
        || entry.area_pack_schema_version != AREA_PACK_SCHEMA_VERSION
    {
        return Err(AreaCatalogError::Invalid(format!(
            "size policy mismatch for area {:?}",
            entry.area_id
        )));
    }
    Ok(())
}

fn attach_runtime_relationships(
    entries: &mut [AreaCatalogEntry],
    references: &[crate::AreaCrossReference],
    packs: &[AreaPack],
) -> Result<(), AreaCatalogError> {
    let revisions = entries
        .iter()
        .map(|entry| (entry.area_id.clone(), entry.revision))
        .collect::<HashMap<_, _>>();
    let indexes = entries
        .iter()
        .enumerate()
        .map(|(index, entry)| (entry.area_id.clone(), index))
        .collect::<HashMap<_, _>>();
    for edge in references
        .iter()
        .filter(|edge| edge.kind != AreaCrossReferenceKind::RoomExit)
    {
        let from = *indexes.get(&edge.from_area).ok_or_else(|| {
            AreaCatalogError::Invalid("hard dependency source area missing".into())
        })?;
        let to = *indexes.get(&edge.to_area).ok_or_else(|| {
            AreaCatalogError::Invalid("hard dependency target area missing".into())
        })?;
        entries[from].hard_dependencies.push(edge.to_area.clone());
        // Authored children must materialize atomically with their container.
        // The ordinary child -> container edge resolves the reference; this
        // reverse runtime edge reserves capacity before players can mutate a
        // partially loaded container. The resulting two-area closure is
        // intentional and cycle-safe.
        if edge.kind == AreaCrossReferenceKind::ItemInstanceContainer {
            entries[to].hard_dependencies.push(edge.from_area.clone());
        }
    }
    for portal in derive_portal_edges(packs) {
        let edge = &portal.reference;
        let from = *indexes
            .get(&edge.from_area)
            .ok_or_else(|| AreaCatalogError::Invalid("portal source area missing".into()))?;
        let to = *indexes
            .get(&edge.to_area)
            .ok_or_else(|| AreaCatalogError::Invalid("portal target area missing".into()))?;
        entries[from].portals.push(AreaPortal {
            direction: AreaPortalDirection::Outgoing,
            source_exit: portal.source_exit,
            peer_area_id: edge.to_area.clone(),
            source_room_id: edge.source_id.clone(),
            target_room_id: edge.target_id.clone(),
            minimum_peer_revision: revisions[&edge.to_area],
        });
        entries[to].portals.push(AreaPortal {
            direction: AreaPortalDirection::Incoming,
            source_exit: portal.source_exit,
            peer_area_id: edge.from_area.clone(),
            source_room_id: edge.source_id.clone(),
            target_room_id: edge.target_id.clone(),
            minimum_peer_revision: revisions[&edge.from_area],
        });
    }
    for entry in entries {
        entry.hard_dependencies.sort();
        entry.hard_dependencies.dedup();
        entry.portals.sort();
        entry.portals.dedup();
    }
    Ok(())
}

struct PortalEdge {
    reference: crate::AreaCrossReference,
    source_exit: crate::Direction,
}

fn derive_portal_edges(packs: &[AreaPack]) -> Vec<PortalEdge> {
    let room_owners = packs
        .iter()
        .flat_map(|pack| {
            pack.rooms
                .iter()
                .map(move |room| (room.id.as_str(), pack.id.as_str()))
        })
        .collect::<HashMap<_, _>>();
    let legacy_rooms = packs
        .iter()
        .flat_map(|pack| {
            pack.rooms.iter().filter_map(move |room| {
                room.legacy_vnum
                    .map(|vnum| (vnum, (room.id.as_str(), pack.id.as_str())))
            })
        })
        .collect::<HashMap<_, _>>();
    let mut portals = Vec::new();
    for pack in packs {
        for room in &pack.rooms {
            for exit in &room.exits {
                let target = match &exit.target {
                    crate::ExitTarget::Room { room_id } => room_owners
                        .get(room_id.as_str())
                        .map(|owner| (room_id.as_str(), *owner)),
                    crate::ExitTarget::LegacyVnum { vnum } => legacy_rooms.get(vnum).copied(),
                };
                if let Some((target_room, target_area)) = target
                    && target_area != pack.id
                {
                    portals.push(PortalEdge {
                        reference: crate::AreaCrossReference {
                            from_area: pack.id.clone(),
                            to_area: target_area.to_owned(),
                            kind: AreaCrossReferenceKind::RoomExit,
                            source_id: room.id.clone(),
                            target_id: target_room.to_owned(),
                        },
                        source_exit: exit.direction,
                    });
                }
            }
        }
    }
    portals.sort_by(|left, right| {
        (&left.reference, left.source_exit).cmp(&(&right.reference, right.source_exit))
    });
    portals
}

fn validate_declared_dependencies(
    packs: &[AreaPack],
    references: &[crate::AreaCrossReference],
) -> Result<(), AreaCatalogError> {
    let area_ids = packs
        .iter()
        .map(|pack| pack.id.as_str())
        .collect::<HashSet<_>>();
    let mut declared = HashMap::new();
    for pack in packs {
        let dependencies = pack
            .depends_on
            .iter()
            .map(String::as_str)
            .collect::<HashSet<_>>();
        if dependencies.len() != pack.depends_on.len()
            || dependencies.contains(pack.id.as_str())
            || dependencies
                .iter()
                .any(|dependency| !area_ids.contains(dependency))
        {
            return Err(AreaCatalogError::Invalid(format!(
                "invalid editorial dependsOn set for area {:?}",
                pack.id
            )));
        }
        declared.insert(pack.id.as_str(), dependencies);
    }
    for edge in references {
        if !declared
            .get(edge.from_area.as_str())
            .is_some_and(|dependencies| dependencies.contains(edge.to_area.as_str()))
        {
            return Err(AreaCatalogError::Invalid(format!(
                "area {:?} does not declare cross-area {:?} reference to {:?}",
                edge.from_area, edge.kind, edge.to_area
            )));
        }
    }
    Ok(())
}

fn strict_content_digests(values: &[ContentDigest]) -> bool {
    values
        .iter()
        .all(|value| bounded_text(&value.id, 512) && valid_digest(&value.sha256))
        && values.windows(2).all(|pair| pair[0].id < pair[1].id)
}

fn world_projection(
    document: &crate::WorldPackDocument,
    ownership: &AreaOwnership,
    pack: &AreaPack,
) -> CompiledArea {
    let contains =
        |ids: &[String], id: &str| ids.binary_search_by(|value| value.as_str().cmp(id)).is_ok();
    let progression = document.world.gameplay.character_progression.as_ref();
    let threshold_levels = pack
        .level_thresholds
        .iter()
        .map(|threshold| threshold.level)
        .collect::<HashSet<_>>();
    CompiledArea {
        item_prototypes: document
            .world
            .item_prototypes
            .iter()
            .filter(|value| contains(&ownership.item_prototypes, &value.id))
            .cloned()
            .collect(),
        initial_item_instances: document
            .world
            .initial_item_instances
            .iter()
            .filter(|value| contains(&ownership.initial_item_instances, &value.id))
            .cloned()
            .collect(),
        rooms: document
            .world
            .rooms
            .iter()
            .filter(|value| contains(&ownership.rooms, &value.id))
            .cloned()
            .collect(),
        npc_prototypes: document
            .world
            .gameplay
            .npc_prototypes
            .iter()
            .filter(|value| contains(&ownership.npc_prototypes, &value.id))
            .cloned()
            .collect(),
        npc_spawns: document
            .world
            .gameplay
            .npc_spawns
            .iter()
            .filter(|value| contains(&ownership.npc_spawns, &value.id))
            .cloned()
            .collect(),
        loot_tables: document
            .world
            .gameplay
            .loot_tables
            .iter()
            .filter(|value| contains(&ownership.loot_tables, &value.id))
            .cloned()
            .collect(),
        loot_sources: document
            .world
            .gameplay
            .loot_sources
            .iter()
            .filter(|value| contains(&ownership.loot_sources, &value.id))
            .cloned()
            .collect(),
        affixes: document
            .world
            .gameplay
            .affixes
            .iter()
            .filter(|value| contains(&ownership.affixes, &value.id))
            .cloned()
            .collect(),
        affix_pools: document
            .world
            .gameplay
            .affix_pools
            .iter()
            .filter(|value| contains(&ownership.affix_pools, &value.id))
            .cloned()
            .collect(),
        archetypes: document
            .world
            .gameplay
            .archetypes
            .iter()
            .filter(|value| contains(&ownership.archetypes, &value.id))
            .cloned()
            .collect(),
        level_thresholds: progression
            .into_iter()
            .flat_map(|value| &value.level_thresholds)
            .filter(|value| threshold_levels.contains(&value.level))
            .cloned()
            .collect(),
        lineages: progression
            .into_iter()
            .flat_map(|value| &value.lineages)
            .filter(|value| contains(&ownership.lineages, &value.id))
            .cloned()
            .collect(),
        backgrounds: progression
            .into_iter()
            .flat_map(|value| &value.backgrounds)
            .filter(|value| contains(&ownership.backgrounds, &value.id))
            .cloned()
            .collect(),
        base_classes: progression
            .into_iter()
            .flat_map(|value| &value.base_classes)
            .filter(|value| contains(&ownership.base_classes, &value.id))
            .cloned()
            .collect(),
        advanced_classes: progression
            .into_iter()
            .flat_map(|value| &value.advanced_classes)
            .filter(|value| contains(&ownership.advanced_classes, &value.id))
            .cloned()
            .collect(),
        talent_graphs: progression
            .into_iter()
            .flat_map(|value| &value.talent_graphs)
            .filter(|value| contains(&ownership.talent_graphs, &value.id))
            .cloned()
            .collect(),
        abilities: document
            .world
            .gameplay
            .abilities
            .iter()
            .filter(|value| contains(&ownership.abilities, &value.id))
            .cloned()
            .collect(),
        social_emotes: document
            .world
            .gameplay
            .social_emotes
            .iter()
            .filter(|value| contains(&ownership.social_emotes, &value.id))
            .cloned()
            .collect(),
        commodities: document
            .world
            .gameplay
            .commodities
            .iter()
            .filter(|value| contains(&ownership.commodities, &value.id))
            .cloned()
            .collect(),
        quests: document
            .world
            .gameplay
            .quests
            .iter()
            .filter(|value| contains(&ownership.quests, &value.id))
            .cloned()
            .collect(),
        shops: document
            .world
            .gameplay
            .shops
            .iter()
            .filter(|value| contains(&ownership.shops, &value.id))
            .cloned()
            .collect(),
        npc_ability_sets: document
            .world
            .gameplay
            .npc_ability_sets
            .iter()
            .filter(|value| contains(&ownership.npc_ability_sets, &value.npc_prototype_id))
            .cloned()
            .collect(),
        room_media: document
            .room_media
            .iter()
            .filter(|value| contains(&ownership.room_media, &value.room_id))
            .cloned()
            .collect(),
        entity_media: document
            .entity_media
            .iter()
            .filter(|value| contains(&ownership.entity_media, &value.key()))
            .cloned()
            .collect(),
        room_perception: document
            .room_perception
            .iter()
            .filter(|value| contains(&ownership.room_perception, &value.room_id))
            .cloned()
            .collect(),
        scene_variants: document
            .scene_variants
            .iter()
            .filter(|value| contains(&ownership.scene_variant_sets, &value.room_id))
            .cloned()
            .collect(),
        scripts: document
            .scripts
            .iter()
            .filter(|value| contains(&ownership.scripts, &value.id))
            .cloned()
            .collect(),
        content_tests: document
            .content_tests
            .iter()
            .filter(|value| contains(&ownership.content_tests, &value.name))
            .cloned()
            .collect(),
    }
}

/// A monolithic fallback published before WorldPack v10 normalizes with the
/// implied legacy quest and market, which no pre-v9 AreaPack can own. That
/// exact content, with no adventure ownership anywhere in the catalog, is the
/// only unowned adventure content a catalog may carry.
fn implicit_legacy_adventure(
    document: &crate::WorldPackDocument,
    entries: &[AreaCatalogEntry],
) -> bool {
    let owned = entries.iter().any(|entry| {
        !entry.ownership.commodities.is_empty()
            || !entry.ownership.quests.is_empty()
            || !entry.ownership.shops.is_empty()
    });
    let (commodities, quests, shops) =
        crate::legacy_adventure_content(&document.world.start_room_id);
    !owned
        && document.world.gameplay.commodities == commodities
        && document.world.gameplay.quests == quests
        && document.world.gameplay.shops == shops
}

fn validate_world_coverage(
    document: &crate::WorldPackDocument,
    entries: &[AreaCatalogEntry],
) -> Result<(), AreaCatalogError> {
    let compiled = entries.iter().map(|entry| &entry.ownership);
    macro_rules! exact_ids {
        ($actual:expr, $field:ident, $id:expr, $label:literal) => {{
            let mut actual = $actual.map($id).collect::<Vec<_>>();
            let mut owned = compiled
                .clone()
                .flat_map(|value| value.$field.clone())
                .collect::<Vec<_>>();
            actual.sort();
            owned.sort();
            if actual != owned {
                return Err(AreaCatalogError::Invalid(
                    concat!("ownership/WorldPack mismatch: ", $label).into(),
                ));
            }
        }};
    }
    exact_ids!(
        document.world.item_prototypes.iter(),
        item_prototypes,
        |value: &crate::ItemPrototype| value.id.clone(),
        "item prototypes"
    );
    exact_ids!(
        document.world.initial_item_instances.iter(),
        initial_item_instances,
        |value: &crate::InitialItemInstance| value.id.clone(),
        "item instances"
    );
    exact_ids!(
        document.world.rooms.iter(),
        rooms,
        |value: &crate::Room| value.id.clone(),
        "rooms"
    );
    exact_ids!(
        document.world.gameplay.npc_prototypes.iter(),
        npc_prototypes,
        |value: &crate::NpcPrototype| value.id.clone(),
        "NPC prototypes"
    );
    exact_ids!(
        document.world.gameplay.npc_spawns.iter(),
        npc_spawns,
        |value: &crate::NpcSpawn| value.id.clone(),
        "NPC spawns"
    );
    exact_ids!(
        document.world.gameplay.loot_tables.iter(),
        loot_tables,
        |value: &crate::LootTableDefinition| value.id.clone(),
        "loot tables"
    );
    exact_ids!(
        document.world.gameplay.loot_sources.iter(),
        loot_sources,
        |value: &crate::LootSourceDefinition| value.id.clone(),
        "loot sources"
    );
    exact_ids!(
        document.world.gameplay.affixes.iter(),
        affixes,
        |value: &crate::AffixDefinition| value.id.clone(),
        "affixes"
    );
    exact_ids!(
        document.world.gameplay.affix_pools.iter(),
        affix_pools,
        |value: &crate::AffixPoolDefinition| value.id.clone(),
        "affix pools"
    );
    exact_ids!(
        document.world.gameplay.archetypes.iter(),
        archetypes,
        |value: &crate::ArchetypeDefinition| value.id.clone(),
        "archetypes"
    );
    exact_ids!(
        document.world.gameplay.abilities.iter(),
        abilities,
        |value: &crate::AbilityDefinition| value.id.clone(),
        "abilities"
    );
    exact_ids!(
        document.world.gameplay.social_emotes.iter(),
        social_emotes,
        |value: &crate::SocialEmoteDefinition| value.id.clone(),
        "social emotes"
    );
    if !implicit_legacy_adventure(document, entries) {
        exact_ids!(
            document.world.gameplay.commodities.iter(),
            commodities,
            |value: &crate::CommodityDefinition| value.id.clone(),
            "commodities"
        );
        exact_ids!(
            document.world.gameplay.quests.iter(),
            quests,
            |value: &crate::QuestDefinition| value.id.clone(),
            "quests"
        );
        exact_ids!(
            document.world.gameplay.shops.iter(),
            shops,
            |value: &crate::ShopDefinition| value.id.clone(),
            "shops"
        );
    }
    exact_ids!(
        document.world.gameplay.npc_ability_sets.iter(),
        npc_ability_sets,
        |value: &crate::NpcAbilitySet| value.npc_prototype_id.clone(),
        "NPC ability sets"
    );
    let progression = document.world.gameplay.character_progression.as_ref();
    exact_ids!(
        progression.into_iter().flat_map(|value| &value.lineages),
        lineages,
        |value: &crate::LineageDefinition| value.id.clone(),
        "lineages"
    );
    exact_ids!(
        progression.into_iter().flat_map(|value| &value.backgrounds),
        backgrounds,
        |value: &crate::BackgroundDefinition| value.id.clone(),
        "backgrounds"
    );
    exact_ids!(
        progression
            .into_iter()
            .flat_map(|value| &value.base_classes),
        base_classes,
        |value: &crate::BaseClassDefinition| value.id.clone(),
        "base classes"
    );
    exact_ids!(
        progression
            .into_iter()
            .flat_map(|value| &value.advanced_classes),
        advanced_classes,
        |value: &crate::AdvancedClassDefinition| value.id.clone(),
        "advanced classes"
    );
    exact_ids!(
        progression
            .into_iter()
            .flat_map(|value| &value.talent_graphs),
        talent_graphs,
        |value: &crate::TalentGraphDefinition| value.id.clone(),
        "talent graphs"
    );
    exact_ids!(
        progression
            .into_iter()
            .flat_map(|value| &value.talent_graphs)
            .flat_map(|graph| &graph.nodes),
        talent_nodes,
        |value: &crate::TalentNodeDefinition| value.id.clone(),
        "talent nodes"
    );
    exact_ids!(
        document.room_media.iter(),
        room_media,
        |value: &crate::RoomMediaDefinition| value.room_id.clone(),
        "room media"
    );
    exact_ids!(
        document.entity_media.iter(),
        entity_media,
        |value: &crate::EntityMediaDefinition| value.key(),
        "entity media"
    );
    exact_ids!(
        document.room_perception.iter(),
        room_perception,
        |value: &crate::RoomPerceptionDefinition| value.room_id.clone(),
        "room perception"
    );
    exact_ids!(
        document.scene_variants.iter(),
        scene_variant_sets,
        |value: &crate::RoomSceneVariants| value.room_id.clone(),
        "scene variant sets"
    );
    exact_ids!(
        document.scene_variants.iter().flat_map(|set| &set.variants),
        scene_variants,
        |value: &crate::SceneVariant| value.id.clone(),
        "scene variants"
    );
    exact_ids!(
        document.scripts.iter(),
        scripts,
        |value: &crate::ScriptDefinition| value.id.clone(),
        "scripts"
    );
    exact_ids!(
        document.content_tests.iter(),
        content_tests,
        |value: &crate::ContentTest| value.name.clone(),
        "content tests"
    );
    Ok(())
}

fn verify_monolithic_content(
    document: &crate::WorldPackDocument,
    packs: &[AreaPack],
) -> Result<(), AreaCatalogError> {
    let mut rebuilt = document.clone();
    macro_rules! rebuild_flattened {
        ($target:expr, $field:ident) => {
            $target = packs.iter().flat_map(|pack| pack.$field.clone()).collect()
        };
    }
    rebuild_flattened!(rebuilt.world.item_prototypes, item_prototypes);
    rebuild_flattened!(rebuilt.world.initial_item_instances, initial_item_instances);
    rebuild_flattened!(rebuilt.world.rooms, rooms);
    rebuild_flattened!(rebuilt.world.gameplay.npc_prototypes, npc_prototypes);
    rebuild_flattened!(rebuilt.world.gameplay.npc_spawns, npc_spawns);
    rebuild_flattened!(rebuilt.world.gameplay.loot_tables, loot_tables);
    rebuild_flattened!(rebuilt.world.gameplay.loot_sources, loot_sources);
    rebuild_flattened!(rebuilt.world.gameplay.affixes, affixes);
    rebuild_flattened!(rebuilt.world.gameplay.affix_pools, affix_pools);
    rebuild_flattened!(rebuilt.world.gameplay.archetypes, archetypes);
    rebuild_flattened!(rebuilt.world.gameplay.abilities, abilities);
    rebuild_flattened!(rebuilt.world.gameplay.social_emotes, social_emotes);
    rebuild_flattened!(rebuilt.world.gameplay.commodities, commodities);
    rebuild_flattened!(rebuilt.world.gameplay.quests, quests);
    rebuild_flattened!(rebuilt.world.gameplay.shops, shops);
    rebuild_flattened!(rebuilt.world.gameplay.npc_ability_sets, npc_ability_sets);
    rebuild_flattened!(rebuilt.room_media, room_media);
    rebuild_flattened!(rebuilt.entity_media, entity_media);
    rebuild_flattened!(rebuilt.room_perception, room_perception);
    rebuild_flattened!(rebuilt.scene_variants, scene_variants);
    rebuild_flattened!(rebuilt.scripts, scripts);
    rebuild_flattened!(rebuilt.content_tests, content_tests);

    let has_progression = packs.iter().any(|pack| {
        !pack.level_thresholds.is_empty()
            || !pack.lineages.is_empty()
            || !pack.backgrounds.is_empty()
            || !pack.base_classes.is_empty()
            || !pack.advanced_classes.is_empty()
            || !pack.talent_graphs.is_empty()
    });
    rebuilt.world.gameplay.character_progression = if has_progression {
        let Some(mut progression) = document.world.gameplay.character_progression.clone() else {
            return Err(AreaCatalogError::Invalid(
                "bundle/WorldPack mismatch: character progression presence".into(),
            ));
        };
        rebuild_flattened!(progression.level_thresholds, level_thresholds);
        rebuild_flattened!(progression.lineages, lineages);
        rebuild_flattened!(progression.backgrounds, backgrounds);
        rebuild_flattened!(progression.base_classes, base_classes);
        rebuild_flattened!(progression.advanced_classes, advanced_classes);
        rebuild_flattened!(progression.talent_graphs, talent_graphs);
        Some(progression)
    } else {
        None
    };

    rebuilt.validate().map_err(|error| {
        AreaCatalogError::Invalid(format!("invalid bundled AreaPack projection: {error}"))
    })?;
    // Use the same canonicalizer as normal WorldPack composition after the
    // explicit validation above.
    // Authoring order for aliases, exits, grants and other set-like fields is
    // intentionally irrelevant, while every semantic value remains exact.
    let rebuilt_bytes = rebuilt.deterministic_artifact().map_err(|error| {
        AreaCatalogError::Invalid(format!("invalid bundled AreaPack projection: {error}"))
    })?;
    let rebuilt = crate::WorldPackDocument::from_json(&rebuilt_bytes).map_err(|error| {
        AreaCatalogError::Invalid(format!(
            "could not decode canonical bundled AreaPack projection: {error}"
        ))
    })?;

    macro_rules! exact_content {
        ($actual:expr, $expected:expr, $label:literal) => {
            if $actual != $expected {
                return Err(AreaCatalogError::Invalid(
                    concat!("bundle/WorldPack mismatch: ", $label).into(),
                ));
            }
        };
    }
    exact_content!(
        document.world.item_prototypes,
        rebuilt.world.item_prototypes,
        "item prototypes"
    );
    exact_content!(
        document.world.initial_item_instances,
        rebuilt.world.initial_item_instances,
        "initial item instances"
    );
    exact_content!(document.world.rooms, rebuilt.world.rooms, "rooms");
    exact_content!(
        document.world.gameplay.npc_prototypes,
        rebuilt.world.gameplay.npc_prototypes,
        "NPC prototypes"
    );
    exact_content!(
        document.world.gameplay.npc_spawns,
        rebuilt.world.gameplay.npc_spawns,
        "NPC spawns"
    );
    exact_content!(
        document.world.gameplay.loot_tables,
        rebuilt.world.gameplay.loot_tables,
        "loot tables"
    );
    exact_content!(
        document.world.gameplay.loot_sources,
        rebuilt.world.gameplay.loot_sources,
        "loot sources"
    );
    exact_content!(
        document.world.gameplay.affixes,
        rebuilt.world.gameplay.affixes,
        "affixes"
    );
    exact_content!(
        document.world.gameplay.affix_pools,
        rebuilt.world.gameplay.affix_pools,
        "affix pools"
    );
    exact_content!(
        document.world.gameplay.archetypes,
        rebuilt.world.gameplay.archetypes,
        "archetypes"
    );
    exact_content!(
        document.world.gameplay.abilities,
        rebuilt.world.gameplay.abilities,
        "abilities"
    );
    exact_content!(
        document.world.gameplay.social_emotes,
        rebuilt.world.gameplay.social_emotes,
        "social emotes"
    );
    exact_content!(
        document.world.gameplay.commodities,
        rebuilt.world.gameplay.commodities,
        "commodities"
    );
    exact_content!(
        document.world.gameplay.quests,
        rebuilt.world.gameplay.quests,
        "quests"
    );
    exact_content!(
        document.world.gameplay.shops,
        rebuilt.world.gameplay.shops,
        "shops"
    );
    exact_content!(
        document.world.gameplay.npc_ability_sets,
        rebuilt.world.gameplay.npc_ability_sets,
        "NPC ability sets"
    );
    exact_content!(document.room_media, rebuilt.room_media, "room media");
    exact_content!(document.entity_media, rebuilt.entity_media, "entity media");
    exact_content!(
        document.room_perception,
        rebuilt.room_perception,
        "room perception"
    );
    exact_content!(
        document.scene_variants,
        rebuilt.scene_variants,
        "scene variants"
    );
    exact_content!(document.scripts, rebuilt.scripts, "scripts");
    exact_content!(
        document.content_tests,
        rebuilt.content_tests,
        "content tests"
    );

    match (
        has_progression,
        document.world.gameplay.character_progression.as_ref(),
        rebuilt.world.gameplay.character_progression.as_ref(),
    ) {
        (false, None, None) => {}
        (true, Some(progression), Some(rebuilt_progression)) => {
            exact_content!(
                progression.level_thresholds,
                rebuilt_progression.level_thresholds,
                "level thresholds"
            );
            exact_content!(
                progression.lineages,
                rebuilt_progression.lineages,
                "lineages"
            );
            exact_content!(
                progression.backgrounds,
                rebuilt_progression.backgrounds,
                "backgrounds"
            );
            exact_content!(
                progression.base_classes,
                rebuilt_progression.base_classes,
                "base classes"
            );
            exact_content!(
                progression.advanced_classes,
                rebuilt_progression.advanced_classes,
                "advanced classes"
            );
            exact_content!(
                progression.talent_graphs,
                rebuilt_progression.talent_graphs,
                "talent graphs"
            );
        }
        _ => {
            return Err(AreaCatalogError::Invalid(
                "bundle/WorldPack mismatch: character progression presence".into(),
            ));
        }
    }
    Ok(())
}

fn verify_single_area_content(
    document: &crate::WorldPackDocument,
    entry: &AreaCatalogEntry,
    pack: &AreaPack,
) -> Result<(), AreaCatalogError> {
    let owns = |ids: &[String], id: &str| {
        ids.binary_search_by(|candidate| candidate.as_str().cmp(id))
            .is_ok()
    };
    let progression = document.world.gameplay.character_progression.as_ref();
    let owned_thresholds = pack
        .level_thresholds
        .iter()
        .map(|threshold| threshold.level)
        .collect::<HashSet<_>>();
    let remainder = AreaPack {
        schema_version: AREA_PACK_SCHEMA_VERSION,
        id: "tiamud:verified-remainder".into(),
        depends_on: Vec::new(),
        item_prototypes: document
            .world
            .item_prototypes
            .iter()
            .filter(|value| !owns(&entry.ownership.item_prototypes, &value.id))
            .cloned()
            .collect(),
        initial_item_instances: document
            .world
            .initial_item_instances
            .iter()
            .filter(|value| !owns(&entry.ownership.initial_item_instances, &value.id))
            .cloned()
            .collect(),
        rooms: document
            .world
            .rooms
            .iter()
            .filter(|value| !owns(&entry.ownership.rooms, &value.id))
            .cloned()
            .collect(),
        npc_prototypes: document
            .world
            .gameplay
            .npc_prototypes
            .iter()
            .filter(|value| !owns(&entry.ownership.npc_prototypes, &value.id))
            .cloned()
            .collect(),
        npc_spawns: document
            .world
            .gameplay
            .npc_spawns
            .iter()
            .filter(|value| !owns(&entry.ownership.npc_spawns, &value.id))
            .cloned()
            .collect(),
        loot_tables: document
            .world
            .gameplay
            .loot_tables
            .iter()
            .filter(|value| !owns(&entry.ownership.loot_tables, &value.id))
            .cloned()
            .collect(),
        loot_sources: document
            .world
            .gameplay
            .loot_sources
            .iter()
            .filter(|value| !owns(&entry.ownership.loot_sources, &value.id))
            .cloned()
            .collect(),
        affixes: document
            .world
            .gameplay
            .affixes
            .iter()
            .filter(|value| !owns(&entry.ownership.affixes, &value.id))
            .cloned()
            .collect(),
        affix_pools: document
            .world
            .gameplay
            .affix_pools
            .iter()
            .filter(|value| !owns(&entry.ownership.affix_pools, &value.id))
            .cloned()
            .collect(),
        archetypes: document
            .world
            .gameplay
            .archetypes
            .iter()
            .filter(|value| !owns(&entry.ownership.archetypes, &value.id))
            .cloned()
            .collect(),
        level_thresholds: progression
            .into_iter()
            .flat_map(|value| &value.level_thresholds)
            .filter(|value| !owned_thresholds.contains(&value.level))
            .cloned()
            .collect(),
        lineages: progression
            .into_iter()
            .flat_map(|value| &value.lineages)
            .filter(|value| !owns(&entry.ownership.lineages, &value.id))
            .cloned()
            .collect(),
        backgrounds: progression
            .into_iter()
            .flat_map(|value| &value.backgrounds)
            .filter(|value| !owns(&entry.ownership.backgrounds, &value.id))
            .cloned()
            .collect(),
        base_classes: progression
            .into_iter()
            .flat_map(|value| &value.base_classes)
            .filter(|value| !owns(&entry.ownership.base_classes, &value.id))
            .cloned()
            .collect(),
        advanced_classes: progression
            .into_iter()
            .flat_map(|value| &value.advanced_classes)
            .filter(|value| !owns(&entry.ownership.advanced_classes, &value.id))
            .cloned()
            .collect(),
        talent_graphs: progression
            .into_iter()
            .flat_map(|value| &value.talent_graphs)
            .filter(|value| !owns(&entry.ownership.talent_graphs, &value.id))
            .cloned()
            .collect(),
        abilities: document
            .world
            .gameplay
            .abilities
            .iter()
            .filter(|value| !owns(&entry.ownership.abilities, &value.id))
            .cloned()
            .collect(),
        social_emotes: document
            .world
            .gameplay
            .social_emotes
            .iter()
            .filter(|value| !owns(&entry.ownership.social_emotes, &value.id))
            .cloned()
            .collect(),
        commodities: document
            .world
            .gameplay
            .commodities
            .iter()
            .filter(|value| !owns(&entry.ownership.commodities, &value.id))
            .cloned()
            .collect(),
        quests: document
            .world
            .gameplay
            .quests
            .iter()
            .filter(|value| !owns(&entry.ownership.quests, &value.id))
            .cloned()
            .collect(),
        shops: document
            .world
            .gameplay
            .shops
            .iter()
            .filter(|value| !owns(&entry.ownership.shops, &value.id))
            .cloned()
            .collect(),
        npc_ability_sets: document
            .world
            .gameplay
            .npc_ability_sets
            .iter()
            .filter(|value| !owns(&entry.ownership.npc_ability_sets, &value.npc_prototype_id))
            .cloned()
            .collect(),
        room_media: document
            .room_media
            .iter()
            .filter(|value| !owns(&entry.ownership.room_media, &value.room_id))
            .cloned()
            .collect(),
        entity_media: document
            .entity_media
            .iter()
            .filter(|value| !owns(&entry.ownership.entity_media, &value.key()))
            .cloned()
            .collect(),
        room_perception: document
            .room_perception
            .iter()
            .filter(|value| !owns(&entry.ownership.room_perception, &value.room_id))
            .cloned()
            .collect(),
        scene_variants: document
            .scene_variants
            .iter()
            .filter(|value| !owns(&entry.ownership.scene_variant_sets, &value.room_id))
            .cloned()
            .collect(),
        scripts: document
            .scripts
            .iter()
            .filter(|value| !owns(&entry.ownership.scripts, &value.id))
            .cloned()
            .collect(),
        content_tests: document
            .content_tests
            .iter()
            .filter(|value| !owns(&entry.ownership.content_tests, &value.name))
            .cloned()
            .collect(),
    };
    verify_monolithic_content(document, &[pack.clone(), remainder])
}

fn validate_global_ownership(entries: &[AreaCatalogEntry]) -> Result<(), AreaCatalogError> {
    let mut owners: HashMap<&str, Vec<(&'static str, &str)>> = HashMap::new();
    for entry in entries {
        let area = entry.area_id.as_str();
        for (kind, ids) in [
            ("room", &entry.ownership.rooms),
            ("item prototype", &entry.ownership.item_prototypes),
            (
                "initial item instance",
                &entry.ownership.initial_item_instances,
            ),
            ("NPC prototype", &entry.ownership.npc_prototypes),
            ("NPC spawn", &entry.ownership.npc_spawns),
            ("loot table", &entry.ownership.loot_tables),
            ("loot source", &entry.ownership.loot_sources),
            ("affix", &entry.ownership.affixes),
            ("affix pool", &entry.ownership.affix_pools),
            ("archetype", &entry.ownership.archetypes),
            ("lineage", &entry.ownership.lineages),
            ("background", &entry.ownership.backgrounds),
            ("base class", &entry.ownership.base_classes),
            ("advanced class", &entry.ownership.advanced_classes),
            ("talent graph", &entry.ownership.talent_graphs),
            ("talent node", &entry.ownership.talent_nodes),
            ("ability", &entry.ownership.abilities),
            ("social emote", &entry.ownership.social_emotes),
            ("commodity", &entry.ownership.commodities),
            ("quest", &entry.ownership.quests),
            ("shop", &entry.ownership.shops),
            ("script", &entry.ownership.scripts),
            ("scene variant", &entry.ownership.scene_variants),
        ] {
            if ids.windows(2).any(|pair| pair[0] >= pair[1]) {
                return Err(AreaCatalogError::Invalid(format!(
                    "{kind} ownership for area {area:?} must be strictly sorted"
                )));
            }
            for id in ids {
                let registrations = owners.entry(id).or_default();
                if let Some((first_kind, first_area)) = registrations.first().copied() {
                    let typed_legacy_pair = registrations.len() == 1
                        && matches!(
                            (first_kind, kind),
                            ("item prototype", "initial item instance")
                                | ("initial item instance", "item prototype")
                        );
                    if !typed_legacy_pair {
                        return Err(AreaCatalogError::Invalid(format!(
                            "global id {id:?} is owned as {first_kind} by {first_area:?} and as {kind} by {area:?}"
                        )));
                    }
                }
                registrations.push((kind, area));
            }
        }
    }
    Ok(())
}

fn canonical_json<T: Serialize>(
    value: &T,
    kind: &'static str,
) -> Result<Vec<u8>, AreaCatalogError> {
    serde_json::to_vec(value).map_err(|source| AreaCatalogError::Encode { kind, source })
}

pub(crate) fn parse_bounded<T: for<'de> Deserialize<'de>>(
    bytes: &[u8],
    maximum: usize,
    kind: &'static str,
) -> Result<T, AreaCatalogError> {
    let value = parse_bounded_value(bytes, maximum, kind)?;
    serde_json::from_value(value).map_err(|source| AreaCatalogError::Json { kind, source })
}

fn parse_bounded_value(
    bytes: &[u8],
    maximum: usize,
    kind: &'static str,
) -> Result<Value, AreaCatalogError> {
    if bytes.len() > maximum {
        return Err(AreaCatalogError::TooLarge {
            kind,
            size: bytes.len(),
            maximum,
        });
    }
    let text = std::str::from_utf8(bytes)
        .map_err(|source| AreaCatalogError::Invalid(format!("{kind} is not UTF-8: {source}")))?;
    if !json_depth_within(text, MAX_JSON_DEPTH) {
        return Err(AreaCatalogError::Invalid(format!(
            "{kind} exceeds JSON depth {MAX_JSON_DEPTH}"
        )));
    }
    parse_unique_json(text).map_err(|source| AreaCatalogError::Json { kind, source })
}

struct UniqueJsonValue(Value);
impl<'de> Deserialize<'de> for UniqueJsonValue {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        deserializer.deserialize_any(UniqueJsonVisitor)
    }
}
struct UniqueJsonVisitor;
impl<'de> Visitor<'de> for UniqueJsonVisitor {
    type Value = UniqueJsonValue;
    fn expecting(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("JSON without duplicate object keys")
    }
    fn visit_bool<E>(self, value: bool) -> Result<Self::Value, E> {
        Ok(UniqueJsonValue(Value::Bool(value)))
    }
    fn visit_i64<E>(self, value: i64) -> Result<Self::Value, E> {
        Ok(UniqueJsonValue(Value::Number(value.into())))
    }
    fn visit_u64<E>(self, value: u64) -> Result<Self::Value, E> {
        Ok(UniqueJsonValue(Value::Number(value.into())))
    }
    fn visit_f64<E: serde::de::Error>(self, value: f64) -> Result<Self::Value, E> {
        serde_json::Number::from_f64(value)
            .map(Value::Number)
            .map(UniqueJsonValue)
            .ok_or_else(|| E::custom("non-finite JSON number"))
    }
    fn visit_str<E>(self, value: &str) -> Result<Self::Value, E> {
        Ok(UniqueJsonValue(Value::String(value.to_owned())))
    }
    fn visit_string<E>(self, value: String) -> Result<Self::Value, E> {
        Ok(UniqueJsonValue(Value::String(value)))
    }
    fn visit_none<E>(self) -> Result<Self::Value, E> {
        Ok(UniqueJsonValue(Value::Null))
    }
    fn visit_unit<E>(self) -> Result<Self::Value, E> {
        Ok(UniqueJsonValue(Value::Null))
    }
    fn visit_seq<A: SeqAccess<'de>>(self, mut sequence: A) -> Result<Self::Value, A::Error> {
        let mut values = Vec::new();
        while let Some(value) = sequence.next_element::<UniqueJsonValue>()? {
            values.push(value.0);
        }
        Ok(UniqueJsonValue(Value::Array(values)))
    }
    fn visit_map<A: MapAccess<'de>>(self, mut object: A) -> Result<Self::Value, A::Error> {
        let mut values = Map::new();
        while let Some(key) = object.next_key::<String>()? {
            if values.contains_key(&key) {
                return Err(serde::de::Error::custom(format!(
                    "duplicate JSON key: {key}"
                )));
            }
            values.insert(key, object.next_value::<UniqueJsonValue>()?.0);
        }
        Ok(UniqueJsonValue(Value::Object(values)))
    }
}
fn parse_unique_json(text: &str) -> Result<Value, serde_json::Error> {
    let mut deserializer = serde_json::Deserializer::from_str(text);
    let value = UniqueJsonValue::deserialize(&mut deserializer)?.0;
    deserializer.end()?;
    Ok(value)
}
fn json_depth_within(text: &str, maximum: usize) -> bool {
    let (mut depth, mut in_string, mut escaped) = (0usize, false, false);
    for byte in text.bytes() {
        if in_string {
            if escaped {
                escaped = false;
            } else if byte == b'\\' {
                escaped = true;
            } else if byte == b'"' {
                in_string = false;
            }
            continue;
        }
        match byte {
            b'"' => in_string = true,
            b'{' | b'[' => {
                depth += 1;
                if depth > maximum {
                    return false;
                }
            }
            b'}' | b']' => {
                let Some(next) = depth.checked_sub(1) else {
                    return false;
                };
                depth = next;
            }
            _ => {}
        }
    }
    depth == 0 && !in_string && !escaped
}
fn bounded_text(value: &str, maximum: usize) -> bool {
    !value.is_empty()
        && value.len() <= maximum
        && value.trim() == value
        && !value.chars().any(|character| {
            character.is_control()
                || matches!(
                    character,
                    '\u{061c}'
                        | '\u{200b}'
                        | '\u{200e}'
                        | '\u{200f}'
                        | '\u{202a}'..='\u{202e}'
                        | '\u{2066}'..='\u{2069}'
                        | '\u{feff}'
                )
        })
}
fn safe_identifier(value: &str) -> bool {
    bounded_text(value, 256)
}
fn valid_reviewed_at(value: &str) -> bool {
    let bytes = value.as_bytes();
    bytes.len() == 20
        && matches!(bytes[4], b'-')
        && matches!(bytes[7], b'-')
        && matches!(bytes[10], b'T')
        && matches!(bytes[13], b':')
        && matches!(bytes[16], b':')
        && matches!(bytes[19], b'Z')
        && bytes
            .iter()
            .enumerate()
            .filter(|(index, _)| !matches!(index, 4 | 7 | 10 | 13 | 16 | 19))
            .all(|(_, byte)| byte.is_ascii_digit())
}
fn valid_digest(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || matches!(byte, b'a'..=b'f'))
}
fn sha256_hex(bytes: &[u8]) -> String {
    hex::encode(Sha256::digest(bytes))
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::*;
    use crate::compose_world_project;

    fn project_path() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../..")
            .join("worlds/lantern-square.project.json")
    }

    fn release_for(composed: &ComposedWorld, catalog_revision: u64) -> AreaReleaseManifest {
        AreaReleaseManifest {
            schema_version: AREA_RELEASE_SCHEMA_VERSION,
            trust_mode: ReleaseTrustMode::UnsignedLocal,
            catalog_revision,
            areas: composed
                .area_packs
                .iter()
                .enumerate()
                .map(|(index, pack)| AreaReleaseEntry {
                    area_id: pack.id.clone(),
                    revision: (index + 1) as u64,
                    content_license: "CC-BY-4.0".into(),
                    script_license: "MIT OR Apache-2.0".into(),
                    reviewer: "Tiamud maintainers".into(),
                    reviewed_at: "2026-07-17T00:00:00Z".into(),
                    provenance: "Reviewed source authored for Tiamud.".into(),
                })
                .collect(),
        }
    }

    #[test]
    fn real_project_builds_a_verified_catalog_with_resources_and_portals() {
        let composed = compose_world_project(project_path()).unwrap();
        let release = release_for(&composed, 7);
        let built = build_area_release(&composed, &release, None).unwrap();

        verify_area_release(&built.catalog, &built.bundles, &built.world_artifact).unwrap();
        assert_eq!(
            parse_area_catalog(&built.catalog_bytes).unwrap(),
            built.catalog
        );
        assert_eq!(built.catalog.entries.len(), 3);
        assert!(
            built
                .catalog
                .entries
                .iter()
                .any(|entry| !entry.portals.is_empty())
        );
        let north_gate = built
            .catalog
            .entries
            .iter()
            .find(|entry| entry.area_id == "north-gate")
            .unwrap();
        assert!(
            !north_gate
                .hard_dependencies
                .iter()
                .any(|area| area == "archive"),
            "portal-only dependency must not become a hard dependency: {:?}",
            north_gate.hard_dependencies
        );
        let square = built
            .catalog
            .entries
            .iter()
            .find(|entry| entry.area_id == "square")
            .unwrap();
        assert!(
            !square
                .hard_dependencies
                .iter()
                .any(|area| area == "north-gate")
        );
        assert_eq!(square.asset_manifests.len(), 1);
        assert_eq!(
            square.asset_manifests[0].sha256,
            "ab383069fd1e4aed4fd4064ad3c7001e192a52a22b9c5f9515f4932a95d1a1f1"
        );
        assert!(
            square
                .lua_modules
                .iter()
                .any(|module| module.id == "lantern-whisper")
        );
        assert_eq!(square.content_license, "CC-BY-4.0");
        assert_eq!(square.script_license, "MIT OR Apache-2.0");
    }

    #[test]
    fn pre_v10_fallback_may_carry_only_the_implied_unowned_legacy_adventure() {
        let composed = compose_world_project(project_path()).unwrap();
        let release = release_for(&composed, 7);
        let built = build_area_release(&composed, &release, None).unwrap();
        let document = crate::WorldPackDocument::from_json(&built.world_artifact).unwrap();
        let square = built
            .catalog
            .entries
            .iter()
            .find(|entry| entry.area_id == "square")
            .unwrap();
        assert_eq!(square.ownership.quests, ["first-hunt"]);
        assert_eq!(square.ownership.shops, ["lantern-market"]);
        validate_world_coverage(&document, &built.catalog.entries).unwrap();

        // A catalog published before AreaPack v9 owns no adventure content,
        // while its v9 fallback normalizes with the implied legacy content.
        let mut legacy_entries = built.catalog.entries.clone();
        for entry in &mut legacy_entries {
            entry.ownership.commodities.clear();
            entry.ownership.quests.clear();
            entry.ownership.shops.clear();
        }
        validate_world_coverage(&document, &legacy_entries).unwrap();

        // Any other unowned adventure content still fails closed.
        let mut altered = document.clone();
        altered.world.gameplay.shops[0].stock[0].buy_price = 13;
        assert!(validate_world_coverage(&altered, &legacy_entries).is_err());
        assert!(validate_world_coverage(&altered, &built.catalog.entries).is_ok());
        let mut extra = document;
        extra.world.gameplay.quests[0].id = "second-hunt".into();
        assert!(validate_world_coverage(&extra, &built.catalog.entries).is_err());
    }

    #[test]
    fn cross_area_container_contents_form_one_atomic_runtime_closure() {
        let composed = compose_world_project(project_path()).unwrap();
        let release = release_for(&composed, 7);
        let built = build_area_release(&composed, &release, None).unwrap();
        let mut entries = built.catalog.entries[..2].to_vec();
        for entry in &mut entries {
            entry.hard_dependencies.clear();
            entry.portals.clear();
        }
        let container_area = entries[0].area_id.clone();
        let child_area = entries[1].area_id.clone();
        attach_runtime_relationships(
            &mut entries,
            &[crate::AreaCrossReference {
                from_area: child_area.clone(),
                to_area: container_area.clone(),
                kind: AreaCrossReferenceKind::ItemInstanceContainer,
                source_id: "test:child".into(),
                target_id: "test:container".into(),
            }],
            &[],
        )
        .unwrap();

        let container = entries
            .iter()
            .find(|entry| entry.area_id == container_area)
            .unwrap();
        let child = entries
            .iter()
            .find(|entry| entry.area_id == child_area)
            .unwrap();
        assert_eq!(container.hard_dependencies, vec![child_area]);
        assert_eq!(child.hard_dependencies, vec![container_area]);
    }

    #[test]
    fn parser_rejects_recursive_duplicates_depth_bombs_and_raw_legacy_area_pack() {
        let duplicate = br#"{"schemaVersion":1,"trustMode":"unsigned_local","catalogRevision":1,"areas":[{"areaId":"a","revision":1,"contentLicense":"CC-BY-4.0","scriptLicense":"MIT OR Apache-2.0","reviewer":"r","reviewedAt":"t","provenance":"p","nested":{"key":1,"key":2}}]}"#;
        assert!(matches!(
            parse_area_release_manifest(duplicate),
            Err(AreaCatalogError::Json { .. })
        ));
        let deep = format!(
            "{}0{}",
            "[".repeat(MAX_JSON_DEPTH + 1),
            "]".repeat(MAX_JSON_DEPTH + 1)
        );
        assert!(matches!(
            parse_area_catalog(deep.as_bytes()),
            Err(AreaCatalogError::Invalid(_))
        ));

        let composed = compose_world_project(project_path()).unwrap();
        let built = build_area_release(&composed, &release_for(&composed, 1), None).unwrap();
        let bytes = built.bundles.values().next().unwrap();
        let mut value: Value = serde_json::from_slice(bytes).unwrap();
        value["areaPack"]["schemaVersion"] = Value::from(7);
        assert!(matches!(
            parse_area_bundle(&serde_json::to_vec(&value).unwrap()),
            Err(AreaCatalogError::Invalid(_))
        ));
    }

    #[test]
    fn verification_fails_closed_on_tampering_and_revision_reuse() {
        let composed = compose_world_project(project_path()).unwrap();
        let release = release_for(&composed, 1);
        let built = build_area_release(&composed, &release, None).unwrap();
        let mut tampered = built.bundles.clone();
        tampered.get_mut("square").unwrap().push(b' ');
        assert!(matches!(
            verify_area_release(&built.catalog, &tampered, &built.world_artifact),
            Err(AreaCatalogError::BundleDigest(_))
        ));
        let mut wrong_world = built.world_artifact.clone();
        wrong_world.push(b' ');
        assert!(verify_area_release(&built.catalog, &built.bundles, &wrong_world).is_err());

        let mut changed = release_for(&composed, 2);
        changed.areas[0].reviewer = "Another reviewer".into();
        assert!(matches!(
            build_area_release(&composed, &changed, Some(&built.catalog)),
            Err(AreaCatalogError::RevisionNotAdvanced { .. })
        ));
        changed.areas[0].revision += 1;
        let second = build_area_release(&composed, &changed, Some(&built.catalog)).unwrap();
        assert_eq!(second.catalog.catalog_revision, 2);
    }

    fn replace_bundle_after_raw_area_mutation(
        built: &mut VerifiedAreaRelease,
        area_id: &str,
        mutate: impl FnOnce(&mut AreaPack),
    ) {
        let bytes = built.bundles.get(area_id).unwrap();
        let mut bundle = parse_area_bundle(bytes).unwrap();
        mutate(&mut bundle.area_pack);
        let area_bytes = canonical_json(&bundle.area_pack, "AreaPack").unwrap();
        bundle.canonical_area_pack_sha256 = sha256_hex(&area_bytes);
        let bundle_bytes = canonical_json(&bundle, "area bundle").unwrap();
        let entry = built
            .catalog
            .entries
            .iter_mut()
            .find(|entry| entry.area_id == area_id)
            .unwrap();
        entry.bundle_sha256 = sha256_hex(&bundle_bytes);
        entry.stored_bytes = bundle_bytes.len() as u64;
        entry.expanded_bytes = bundle_bytes.len() as u64;
        entry.estimated_memory_bytes = (area_bytes.len() as u64).saturating_mul(2);
        built.bundles.insert(area_id.to_owned(), bundle_bytes);
    }

    #[test]
    fn verification_rejects_raw_room_content_that_differs_from_compiled_fallback() {
        let composed = compose_world_project(project_path()).unwrap();
        let mut built = build_area_release(&composed, &release_for(&composed, 1), None).unwrap();
        replace_bundle_after_raw_area_mutation(&mut built, "square", |pack| {
            pack.rooms[0].title.push_str(" alterado");
        });

        assert!(matches!(
            verify_area_release(&built.catalog, &built.bundles, &built.world_artifact),
            Err(AreaCatalogError::Invalid(reason))
                if reason == "bundle/WorldPack mismatch: rooms"
        ));
    }

    #[test]
    fn verification_rejects_raw_progression_that_differs_from_compiled_fallback() {
        let composed = compose_world_project(project_path()).unwrap();
        let mut built = build_area_release(&composed, &release_for(&composed, 1), None).unwrap();
        replace_bundle_after_raw_area_mutation(&mut built, "square", |pack| {
            pack.level_thresholds[0].total_experience_required += 1;
        });

        assert!(matches!(
            verify_area_release(&built.catalog, &built.bundles, &built.world_artifact),
            Err(AreaCatalogError::Invalid(reason))
                if reason == "bundle/WorldPack mismatch: level thresholds"
        ));
    }

    #[test]
    fn release_metadata_and_catalog_fields_are_closed_and_bounded() {
        let composed = compose_world_project(project_path()).unwrap();
        let mut release = release_for(&composed, 1);
        release.areas[0].script_license.clear();
        assert!(build_area_release(&composed, &release, None).is_err());
        let mut release = release_for(&composed, 1);
        release.areas[0].content_license = "CC0-1.0".into();
        assert!(build_area_release(&composed, &release, None).is_err());
        let mut release = release_for(&composed, 1);
        release.areas[0].script_license = "Apache-2.0".into();
        assert!(build_area_release(&composed, &release, None).is_err());
        assert!(matches!(
            parse_area_catalog(&vec![b' '; MAX_AREA_CATALOG_BYTES + 1]),
            Err(AreaCatalogError::TooLarge { .. })
        ));
        assert!(parse_area_release_manifest(br#"{"schemaVersion":1,"trustMode":"unsigned_local","catalogRevision":1,"areas":[],"signature":null}"#).is_err());
    }
}
