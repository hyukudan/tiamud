//! Strict, bounded, offline composition of modular authoring areas.
//!
//! AreaPack v1 is deliberately an authoring format, not a runtime hot-load
//! protocol. Composition resolves and validates the complete graph, then emits
//! the same single [`WorldPackDocument`] v2 consumed by the current runtime.
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
    ContentTest, ExitTarget, GameplayDefinition, Item, ItemRule, NpcPrototype, NpcSpawn, Room,
    RoomMediaDefinition, RoomPerceptionDefinition, RoomSceneVariants, ScriptDefinition,
    SourceMetadata, ValidationErrors, WorldPack, WorldPackDocument,
};

pub const WORLD_PROJECT_SCHEMA_VERSION: u32 = 1;
pub const AREA_PACK_SCHEMA_VERSION: u32 = 1;
pub const MAX_PROJECT_MANIFEST_BYTES: u64 = 1024 * 1024;
pub const MAX_AREA_COUNT: usize = 256;
pub const MAX_AREA_FILE_BYTES: u64 = 16 * 1024 * 1024;
pub const MAX_TOTAL_AREA_BYTES: u64 = 16 * 1024 * 1024;
pub const MAX_COMPOSED_WORLD_BYTES: usize = 16 * 1024 * 1024;

/// Project-level identity and the complete set of offline authoring modules.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct WorldProjectManifest {
    pub schema_version: u32,
    pub id: String,
    pub name: String,
    pub start_room_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rng_seed: Option<u64>,
    pub areas: Vec<ProjectArea>,
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
    pub items: Vec<Item>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub rooms: Vec<Room>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub item_rules: Vec<ItemRule>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub npc_prototypes: Vec<NpcPrototype>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub npc_spawns: Vec<NpcSpawn>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub room_media: Vec<RoomMediaDefinition>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub room_perception: Vec<RoomPerceptionDefinition>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub scene_variants: Vec<RoomSceneVariants>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub scripts: Vec<ScriptDefinition>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub content_tests: Vec<ContentTest>,
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
            items: Vec<Item>,
            #[serde(default)]
            rooms: Vec<Room>,
            #[serde(default)]
            item_rules: Vec<ItemRule>,
            #[serde(default)]
            npc_prototypes: Vec<NpcPrototype>,
            #[serde(default)]
            npc_spawns: Vec<NpcSpawn>,
            #[serde(default)]
            room_media: Vec<RoomMediaDefinition>,
            #[serde(default)]
            room_perception: Vec<RoomPerceptionDefinition>,
            #[serde(default)]
            scene_variants: Vec<RoomSceneVariants>,
            #[serde(default)]
            scripts: Vec<ScriptDefinition>,
            #[serde(default)]
            content_tests: Vec<ContentTest>,
        }

        let value = serde_json::Value::deserialize(deserializer)?;
        audit_area_pack_keys(&value).map_err(D::Error::custom)?;
        let raw: RawAreaPack = serde_json::from_value(value).map_err(D::Error::custom)?;
        Ok(Self {
            schema_version: raw.schema_version,
            id: raw.id,
            depends_on: raw.depends_on,
            items: raw.items,
            rooms: raw.rooms,
            item_rules: raw.item_rules,
            npc_prototypes: raw.npc_prototypes,
            npc_spawns: raw.npc_spawns,
            room_media: raw.room_media,
            room_perception: raw.room_perception,
            scene_variants: raw.scene_variants,
            scripts: raw.scripts,
            content_tests: raw.content_tests,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AreaOwnership {
    pub rooms: Vec<String>,
    pub items: Vec<String>,
    pub item_rules: Vec<String>,
    pub npc_prototypes: Vec<String>,
    pub npc_spawns: Vec<String>,
    pub room_media: Vec<String>,
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
    pub ownership: AreaOwnership,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum AreaCrossReferenceKind {
    RoomExit,
    RoomItem,
    ItemRule,
    NpcLoot,
    NpcSpawnPrototype,
    NpcSpawnRoom,
    RoomMedia,
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
}

#[derive(Debug, Error)]
pub enum CompositionError {
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
            items: Vec::new(),
            rooms: Vec::new(),
            gameplay: GameplayDefinition {
                rng_seed: manifest.rng_seed,
                ..GameplayDefinition::default()
            },
        },
        room_media: Vec::new(),
        room_perception: Vec::new(),
        scene_variants: Vec::new(),
        scripts: Vec::new(),
        content_tests: Vec::new(),
    };
    let mut global_ids: HashMap<String, (&'static str, String)> = HashMap::new();
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
            "item",
            &pack.id,
            pack.items.iter().map(|v| &v.id),
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
            ownership: AreaOwnership {
                rooms: sorted(pack.rooms.iter().map(|value| value.id.clone()).collect()),
                items: sorted(pack.items.iter().map(|value| value.id.clone()).collect()),
                item_rules: sorted(
                    pack.item_rules
                        .iter()
                        .map(|value| value.item_id.clone())
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
                room_media: sorted(
                    pack.room_media
                        .iter()
                        .map(|value| value.room_id.clone())
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

        document.world.items.extend(pack.items.clone());
        document.world.rooms.extend(pack.rooms.clone());
        document
            .world
            .gameplay
            .item_rules
            .extend(pack.item_rules.clone());
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
        document.room_media.extend(pack.room_media.clone());
        document
            .room_perception
            .extend(pack.room_perception.clone());
        document.scene_variants.extend(pack.scene_variants.clone());
        document.scripts.extend(pack.scripts.clone());
        document.content_tests.extend(pack.content_tests.clone());
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
    Ok(ComposedWorld {
        document,
        artifact,
        report,
    })
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
    global: &mut HashMap<String, (&'static str, String)>,
    kind: &'static str,
    area: &str,
    ids: impl Iterator<Item = &'a String>,
) -> Result<(), CompositionError> {
    for id in ids {
        if let Some((first_kind, first_area)) = global.insert(id.clone(), (kind, area.to_owned())) {
            return Err(CompositionError::DuplicateGlobalEntityId {
                id: id.clone(),
                first_kind,
                first_area,
                second_kind: kind,
                second_area: area.to_owned(),
            });
        }
    }
    Ok(())
}

fn derive_cross_area_references(areas: &[LoadedArea]) -> Vec<AreaCrossReference> {
    let room_owner = owner_map(areas, |area| {
        area.rooms.iter().map(|value| value.id.as_str())
    });
    let item_owner = owner_map(areas, |area| {
        area.items.iter().map(|value| value.id.as_str())
    });
    let prototype_owner = owner_map(areas, |area| {
        area.npc_prototypes.iter().map(|value| value.id.as_str())
    });
    let spawn_owner = owner_map(areas, |area| {
        area.npc_spawns.iter().map(|value| value.id.as_str())
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
            for item in &room.items {
                if let Some(owner) = item_owner.get(item.as_str()) {
                    add_edge(
                        &mut edges,
                        from,
                        owner,
                        AreaCrossReferenceKind::RoomItem,
                        &room.id,
                        item,
                    );
                }
            }
        }
        for rule in &area.pack.item_rules {
            if let Some(owner) = item_owner.get(rule.item_id.as_str()) {
                add_edge(
                    &mut edges,
                    from,
                    owner,
                    AreaCrossReferenceKind::ItemRule,
                    &rule.item_id,
                    &rule.item_id,
                );
            }
        }
        for prototype in &area.pack.npc_prototypes {
            for item in &prototype.loot_item_ids {
                if let Some(owner) = item_owner.get(item.as_str()) {
                    add_edge(
                        &mut edges,
                        from,
                        owner,
                        AreaCrossReferenceKind::NpcLoot,
                        &prototype.id,
                        item,
                    );
                }
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
                if let Some(owner) = item_owner.get(item.as_str()) {
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
/// AreaPack v1 does not: this format-specific key audit makes every structured
/// authoring record fail closed without changing runtime deserialization.
fn audit_area_pack_keys(value: &serde_json::Value) -> Result<(), CompositionError> {
    let Some(area) = audit_object(
        value,
        &[
            "schemaVersion",
            "id",
            "dependsOn",
            "items",
            "rooms",
            "itemRules",
            "npcPrototypes",
            "npcSpawns",
            "roomMedia",
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
    audit_array(area.get("items"), "$.items", audit_item)?;
    audit_array(area.get("rooms"), "$.rooms", audit_room)?;
    audit_array(area.get("itemRules"), "$.itemRules", audit_item_rule)?;
    audit_array(
        area.get("npcPrototypes"),
        "$.npcPrototypes",
        audit_npc_prototype,
    )?;
    audit_array(area.get("npcSpawns"), "$.npcSpawns", audit_npc_spawn)?;
    audit_array(area.get("roomMedia"), "$.roomMedia", audit_room_media)?;
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

fn audit_npc_spawn(value: &serde_json::Value, path: &str) -> Result<(), CompositionError> {
    audit_object(value, &["id", "prototypeId", "roomId"], path).map(|_| ())
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
