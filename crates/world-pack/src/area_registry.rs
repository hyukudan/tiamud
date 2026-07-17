//! Read-only, content-addressed access to one local area generation.
//!
//! Opening a generation verifies its exact catalog path and monolithic
//! fallback, but deliberately leaves area bundles unopened. Callers prepare
//! only the requested hard-dependency closure and every used bundle is
//! revalidated against the fixed catalog and fallback.

use std::{
    collections::{BTreeMap, BTreeSet, VecDeque},
    fs::{self, OpenOptions},
    io::Read,
    path::{Component, Path, PathBuf},
};

use sha2::{Digest, Sha256};
use thiserror::Error;

use crate::{
    AreaBundle, AreaCatalog, AreaCatalogEntry, AreaCatalogError, MAX_COMPOSED_WORLD_BYTES,
    MAX_TOTAL_AREA_BYTES, WorldPackDocument,
    area_catalog::{
        MAX_AREA_CATALOG_BYTES, verify_area_bundle_against_catalog, verify_area_catalog_fallback,
    },
    parse_area_catalog, verify_area_release,
};

pub const MAX_LOCAL_HARD_DEPENDENCY_CLOSURE: usize = 32;

#[derive(Debug, Error)]
pub enum LocalAreaRegistryError {
    #[error("could not inspect {path:?}: {source}")]
    Inspect {
        path: PathBuf,
        source: std::io::Error,
    },
    #[error("could not read {path:?}: {source}")]
    Read {
        path: PathBuf,
        source: std::io::Error,
    },
    #[error("{kind} {path:?} contains {size} bytes; the limit is {maximum} bytes")]
    TooLarge {
        kind: &'static str,
        path: PathBuf,
        size: u64,
        maximum: u64,
    },
    #[error("invalid local area registry: {0}")]
    Invalid(String),
    #[error("area catalog verification failed: {0}")]
    Catalog(#[from] AreaCatalogError),
}

#[derive(Debug, Clone)]
pub struct LocalAreaGeneration {
    registry: PathBuf,
    catalog_path: PathBuf,
    generation_id: String,
    catalog: AreaCatalog,
    world_artifact: Vec<u8>,
    fallback: WorldPackDocument,
    area_index: BTreeMap<String, usize>,
    room_owners: BTreeMap<String, String>,
}

impl LocalAreaGeneration {
    pub fn open_exact(
        registry: impl AsRef<Path>,
        catalog_path: impl AsRef<Path>,
    ) -> Result<Self, LocalAreaRegistryError> {
        let registry = registry.as_ref().to_owned();
        let catalog_path = catalog_path.as_ref().to_owned();
        let catalog_bytes = read_bounded_local_registry_file(
            &registry,
            &catalog_path,
            MAX_AREA_CATALOG_BYTES,
            "area catalog",
        )?;
        let generation_id = sha256_hex(&catalog_bytes);
        let catalog = parse_area_catalog(&catalog_bytes)?;
        preflight_catalog_read_budget(&catalog)?;
        let expected_path = local_area_catalog_path(&registry, &catalog.world_id, &generation_id);
        if catalog_path != expected_path {
            return Err(LocalAreaRegistryError::Invalid(
                "catalog path is not its exact content-addressed registry path".into(),
            ));
        }

        let world_path =
            local_area_world_path(&registry, &catalog.world_id, &catalog.world_artifact_sha256);
        let world_artifact = read_bounded_local_registry_file(
            &registry,
            &world_path,
            MAX_COMPOSED_WORLD_BYTES,
            "monolithic WorldPack fallback",
        )?;
        let fallback = verify_area_catalog_fallback(&catalog, &world_artifact)?;
        let area_index = catalog
            .entries
            .iter()
            .enumerate()
            .map(|(index, entry)| (entry.area_id.clone(), index))
            .collect::<BTreeMap<_, _>>();
        let room_owners = catalog
            .entries
            .iter()
            .flat_map(|entry| {
                entry
                    .ownership
                    .rooms
                    .iter()
                    .map(move |room_id| (room_id.clone(), entry.area_id.clone()))
            })
            .collect::<BTreeMap<_, _>>();
        let generation = Self {
            registry,
            catalog_path,
            generation_id,
            catalog,
            world_artifact,
            fallback,
            area_index,
            room_owners,
        };
        for entry in &generation.catalog.entries {
            generation.hard_dependency_closure(entry.area_id.as_str())?;
        }
        Ok(generation)
    }

    pub fn generation_id(&self) -> &str {
        &self.generation_id
    }

    pub fn catalog(&self) -> &AreaCatalog {
        &self.catalog
    }

    pub fn catalog_path(&self) -> &Path {
        &self.catalog_path
    }

    pub fn world_artifact(&self) -> &[u8] {
        &self.world_artifact
    }

    pub fn fallback(&self) -> &WorldPackDocument {
        &self.fallback
    }

    pub fn entry(&self, area_id: &str) -> Option<&AreaCatalogEntry> {
        self.area_index
            .get(area_id)
            .map(|index| &self.catalog.entries[*index])
    }

    pub fn room_owner(&self, room_id: &str) -> Option<&str> {
        self.room_owners.get(room_id).map(String::as_str)
    }

    pub fn hard_dependency_closure(
        &self,
        area_id: &str,
    ) -> Result<Vec<&AreaCatalogEntry>, LocalAreaRegistryError> {
        if self.entry(area_id).is_none() {
            return Err(LocalAreaRegistryError::Invalid(format!(
                "unknown area {area_id:?}"
            )));
        }
        let mut pending = VecDeque::from([area_id]);
        let mut closure = BTreeSet::new();
        while let Some(current) = pending.pop_front() {
            if !closure.insert(current) {
                continue;
            }
            if closure.len() > MAX_LOCAL_HARD_DEPENDENCY_CLOSURE {
                return Err(LocalAreaRegistryError::Invalid(format!(
                    "hard dependency closure for area {area_id:?} exceeds {MAX_LOCAL_HARD_DEPENDENCY_CLOSURE} areas"
                )));
            }
            let entry = self.entry(current).ok_or_else(|| {
                LocalAreaRegistryError::Invalid(format!(
                    "hard dependency closure names unknown area {current:?}"
                ))
            })?;
            pending.extend(entry.hard_dependencies.iter().map(String::as_str));
        }
        Ok(closure
            .into_iter()
            .map(|id| {
                self.entry(id)
                    .expect("catalog dependencies were validated when the generation opened")
            })
            .collect())
    }

    pub fn read_bundle_exact(&self, area_id: &str) -> Result<AreaBundle, LocalAreaRegistryError> {
        let entry = self
            .entry(area_id)
            .ok_or_else(|| LocalAreaRegistryError::Invalid(format!("unknown area {area_id:?}")))?;
        let bytes = self.read_bundle_bytes(entry)?;
        verify_area_bundle_against_catalog(&self.catalog, &self.fallback, entry, &bytes)
            .map_err(Into::into)
    }

    pub fn read_hard_dependency_closure(
        &self,
        area_id: &str,
    ) -> Result<Vec<AreaBundle>, LocalAreaRegistryError> {
        let closure = self.hard_dependency_closure(area_id)?;
        let mut total = 0_u64;
        let mut bundles = Vec::with_capacity(closure.len());
        for entry in closure {
            total = total.checked_add(entry.stored_bytes).ok_or_else(|| {
                LocalAreaRegistryError::Invalid("area bundle closure byte sum overflows".into())
            })?;
            if total > MAX_TOTAL_AREA_BYTES {
                return Err(LocalAreaRegistryError::Invalid(format!(
                    "area bundle closure declares {total} bytes; the limit is {MAX_TOTAL_AREA_BYTES} bytes"
                )));
            }
            bundles.push(self.read_bundle_exact(&entry.area_id)?);
        }
        Ok(bundles)
    }

    /// Revalidates every immutable bundle and their complete union. Runtime
    /// loading remains lazy; the offline `area-verify` command uses this gate.
    pub fn verify_complete_release(&self) -> Result<(), LocalAreaRegistryError> {
        let mut bundles = BTreeMap::new();
        for entry in &self.catalog.entries {
            let bytes = self.read_bundle_bytes(entry)?;
            bundles.insert(entry.area_id.clone(), bytes);
        }
        verify_area_release(&self.catalog, &bundles, &self.world_artifact).map_err(Into::into)
    }

    fn read_bundle_bytes(
        &self,
        entry: &AreaCatalogEntry,
    ) -> Result<Vec<u8>, LocalAreaRegistryError> {
        let path = local_area_bundle_path(
            &self.registry,
            &self.catalog.world_id,
            &entry.area_id,
            &entry.bundle_sha256,
        );
        read_bounded_local_registry_file(
            &self.registry,
            &path,
            entry.stored_bytes as usize,
            "area bundle",
        )
    }
}

pub fn local_area_catalog_path(registry: &Path, world_id: &str, digest: &str) -> PathBuf {
    registry
        .join("area-catalogs")
        .join(storage_id(world_id))
        .join(format!("{digest}.catalog.json"))
}

pub fn local_area_bundle_path(
    registry: &Path,
    world_id: &str,
    area_id: &str,
    digest: &str,
) -> PathBuf {
    registry
        .join("area-bundles")
        .join(storage_id(world_id))
        .join(storage_id(area_id))
        .join(format!("{digest}.area.json"))
}

pub fn local_area_world_path(registry: &Path, world_id: &str, digest: &str) -> PathBuf {
    registry
        .join("area-worlds")
        .join(storage_id(world_id))
        .join(format!("{digest}.world.json"))
}

#[doc(hidden)]
pub fn read_bounded_local_registry_file(
    registry: &Path,
    path: &Path,
    maximum: usize,
    kind: &'static str,
) -> Result<Vec<u8>, LocalAreaRegistryError> {
    if !path.starts_with(registry) {
        return Err(LocalAreaRegistryError::Invalid(format!(
            "{kind} path escapes its configured registry"
        )));
    }
    let parent = path
        .parent()
        .ok_or_else(|| LocalAreaRegistryError::Invalid(format!("{kind} has no parent")))?;
    verify_existing_registry_directory(registry, parent)?;
    read_bounded_regular_file(path, maximum, kind)
}

#[doc(hidden)]
pub fn verify_existing_registry_directory(
    registry: &Path,
    directory: &Path,
) -> Result<(), LocalAreaRegistryError> {
    if !directory.starts_with(registry) {
        return Err(LocalAreaRegistryError::Invalid(
            "registry read path escapes its configured root".into(),
        ));
    }
    verify_existing_directory(registry)?;
    let relative = directory.strip_prefix(registry).map_err(|_| {
        LocalAreaRegistryError::Invalid("registry read path escapes its configured root".into())
    })?;
    let mut current = registry.to_owned();
    for component in relative.components() {
        let Component::Normal(component) = component else {
            return Err(LocalAreaRegistryError::Invalid(
                "registry read path contains a traversal component".into(),
            ));
        };
        current.push(component);
        verify_existing_directory(&current)?;
    }
    Ok(())
}

#[doc(hidden)]
pub fn read_bounded_regular_file(
    path: &Path,
    maximum: usize,
    kind: &'static str,
) -> Result<Vec<u8>, LocalAreaRegistryError> {
    let before = fs::symlink_metadata(path).map_err(|source| LocalAreaRegistryError::Inspect {
        path: path.to_owned(),
        source,
    })?;
    if before.file_type().is_symlink() || !before.file_type().is_file() {
        return Err(LocalAreaRegistryError::Invalid(format!(
            "{kind} is not a regular non-symlink file"
        )));
    }
    if before.len() > maximum as u64 {
        return Err(LocalAreaRegistryError::TooLarge {
            kind,
            path: path.to_owned(),
            size: before.len(),
            maximum: maximum as u64,
        });
    }
    let mut file = OpenOptions::new().read(true).open(path).map_err(|source| {
        LocalAreaRegistryError::Read {
            path: path.to_owned(),
            source,
        }
    })?;
    let opened = file
        .metadata()
        .map_err(|source| LocalAreaRegistryError::Inspect {
            path: path.to_owned(),
            source,
        })?;
    if !opened.file_type().is_file() || opened.len() > maximum as u64 {
        return Err(LocalAreaRegistryError::Invalid(format!(
            "{kind} changed while it was opened"
        )));
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        if before.dev() != opened.dev() || before.ino() != opened.ino() {
            return Err(LocalAreaRegistryError::Invalid(format!(
                "{kind} changed while it was opened"
            )));
        }
    }
    let mut bytes = Vec::with_capacity(opened.len() as usize);
    Read::by_ref(&mut file)
        .take((maximum + 1) as u64)
        .read_to_end(&mut bytes)
        .map_err(|source| LocalAreaRegistryError::Read {
            path: path.to_owned(),
            source,
        })?;
    if bytes.len() > maximum {
        return Err(LocalAreaRegistryError::TooLarge {
            kind,
            path: path.to_owned(),
            size: bytes.len() as u64,
            maximum: maximum as u64,
        });
    }
    Ok(bytes)
}

fn verify_existing_directory(path: &Path) -> Result<(), LocalAreaRegistryError> {
    let metadata =
        fs::symlink_metadata(path).map_err(|source| LocalAreaRegistryError::Inspect {
            path: path.to_owned(),
            source,
        })?;
    if metadata.file_type().is_dir() && !metadata.file_type().is_symlink() {
        Ok(())
    } else {
        Err(LocalAreaRegistryError::Invalid(format!(
            "registry component {path:?} is not a real directory"
        )))
    }
}

fn preflight_catalog_read_budget(catalog: &AreaCatalog) -> Result<(), LocalAreaRegistryError> {
    let mut total = 0_u64;
    for entry in &catalog.entries {
        total = total.checked_add(entry.stored_bytes).ok_or_else(|| {
            LocalAreaRegistryError::Invalid("area bundle byte sum overflows".into())
        })?;
        if total > MAX_TOTAL_AREA_BYTES {
            return Err(LocalAreaRegistryError::Invalid(format!(
                "area bundle set declares {total} stored bytes; the limit is {MAX_TOTAL_AREA_BYTES} bytes"
            )));
        }
    }
    Ok(())
}

fn storage_id(id: &str) -> String {
    sha256_hex(id.as_bytes())
}

fn sha256_hex(bytes: &[u8]) -> String {
    hex::encode(Sha256::digest(bytes))
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::*;
    use crate::{
        AREA_RELEASE_SCHEMA_VERSION, AreaOwnership, AreaReleaseEntry, AreaReleaseManifest,
        ComposedWorld, ReleaseTrustMode, VerifiedAreaRelease, build_area_release,
        compose_world_project,
    };

    fn project_path() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../..")
            .join("worlds/lantern-square.project.json")
    }

    fn release_for(composed: &ComposedWorld) -> AreaReleaseManifest {
        AreaReleaseManifest {
            schema_version: AREA_RELEASE_SCHEMA_VERSION,
            trust_mode: ReleaseTrustMode::UnsignedLocal,
            catalog_revision: 1,
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
                    provenance: "Reviewed original Tiamud source.".into(),
                })
                .collect(),
        }
    }

    fn built_release() -> VerifiedAreaRelease {
        let composed = compose_world_project(project_path()).unwrap();
        build_area_release(&composed, &release_for(&composed), None).unwrap()
    }

    fn write_release(registry: &Path, release: &VerifiedAreaRelease) -> PathBuf {
        let world_path = local_area_world_path(
            registry,
            &release.catalog.world_id,
            &release.catalog.world_artifact_sha256,
        );
        fs::create_dir_all(world_path.parent().unwrap()).unwrap();
        fs::write(world_path, &release.world_artifact).unwrap();
        for (area_id, bytes) in &release.bundles {
            let entry = release
                .catalog
                .entries
                .iter()
                .find(|entry| &entry.area_id == area_id)
                .unwrap();
            let path = local_area_bundle_path(
                registry,
                &release.catalog.world_id,
                area_id,
                &entry.bundle_sha256,
            );
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(path, bytes).unwrap();
        }
        write_catalog(registry, &release.catalog)
    }

    fn write_catalog(registry: &Path, catalog: &AreaCatalog) -> PathBuf {
        let bytes = serde_json::to_vec(catalog).unwrap();
        let digest = sha256_hex(&bytes);
        let path = local_area_catalog_path(registry, &catalog.world_id, &digest);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(&path, bytes).unwrap();
        path
    }

    #[test]
    fn opens_lazy_generation_and_indexes_exact_catalog_and_rooms() {
        let temp = tempfile::tempdir().unwrap();
        let release = built_release();
        let catalog_path = write_release(temp.path(), &release);
        let missing_entry = &release.catalog.entries[0];
        let missing_bundle = local_area_bundle_path(
            temp.path(),
            &release.catalog.world_id,
            &missing_entry.area_id,
            &missing_entry.bundle_sha256,
        );
        fs::remove_file(&missing_bundle).unwrap();

        let generation = LocalAreaGeneration::open_exact(temp.path(), &catalog_path).unwrap();
        assert_eq!(generation.generation_id(), release.catalog_sha256);
        assert_eq!(generation.catalog_path(), catalog_path);
        assert_eq!(generation.fallback().world.id, "lantern-square");
        assert_eq!(generation.room_owner("demo:lantern-square"), Some("square"));
        assert_eq!(generation.room_owner("unknown:room"), None);
        assert!(generation.entry("square").is_some());
        assert!(generation.entry("unknown-area").is_none());
        assert!(
            generation
                .read_bundle_exact(&missing_entry.area_id)
                .is_err()
        );
        assert!(generation.read_bundle_exact("unknown-area").is_err());
        assert!(generation.hard_dependency_closure("unknown-area").is_err());
    }

    #[test]
    fn reads_exact_bundle_and_rejects_tamper_or_raw_semantic_smuggling() {
        let temp = tempfile::tempdir().unwrap();
        let release = built_release();
        let catalog_path = write_release(temp.path(), &release);
        let generation = LocalAreaGeneration::open_exact(temp.path(), &catalog_path).unwrap();
        let square = generation.read_bundle_exact("square").unwrap();
        assert_eq!(square.area_id, "square");
        let closure = generation.read_hard_dependency_closure("square").unwrap();
        assert!(!closure.is_empty());
        assert!(
            closure
                .windows(2)
                .all(|pair| pair[0].area_id < pair[1].area_id)
        );

        let entry = generation.entry("square").unwrap();
        let bundle_path = local_area_bundle_path(
            temp.path(),
            &release.catalog.world_id,
            "square",
            &entry.bundle_sha256,
        );
        let original = fs::read(&bundle_path).unwrap();
        fs::write(&bundle_path, [original.as_slice(), b" "].concat()).unwrap();
        assert!(generation.read_bundle_exact("square").is_err());
        #[cfg(unix)]
        {
            use std::os::unix::fs::symlink;

            let outside = temp.path().join("outside.area.json");
            fs::write(&outside, &original).unwrap();
            fs::remove_file(&bundle_path).unwrap();
            symlink(outside, &bundle_path).unwrap();
            let error = generation.read_bundle_exact("square").unwrap_err();
            assert!(error.to_string().contains("non-symlink"));
        }

        let second = tempfile::tempdir().unwrap();
        let mut catalog = release.catalog.clone();
        let mut bundle = crate::parse_area_bundle(&release.bundles["square"]).unwrap();
        bundle.area_pack.rooms[0].description = "Smuggled room prose.".into();
        let area_pack_bytes = serde_json::to_vec(&bundle.area_pack).unwrap();
        bundle.canonical_area_pack_sha256 = sha256_hex(&area_pack_bytes);
        let bundle_bytes = serde_json::to_vec(&bundle).unwrap();
        let entry = catalog
            .entries
            .iter_mut()
            .find(|entry| entry.area_id == "square")
            .unwrap();
        entry.bundle_sha256 = sha256_hex(&bundle_bytes);
        entry.stored_bytes = bundle_bytes.len() as u64;
        entry.expanded_bytes = bundle_bytes.len() as u64;
        entry.estimated_memory_bytes = (area_pack_bytes.len() as u64).saturating_mul(2);
        let world_path = local_area_world_path(
            second.path(),
            &catalog.world_id,
            &catalog.world_artifact_sha256,
        );
        fs::create_dir_all(world_path.parent().unwrap()).unwrap();
        fs::write(world_path, &release.world_artifact).unwrap();
        let malicious_path = local_area_bundle_path(
            second.path(),
            &catalog.world_id,
            "square",
            &entry.bundle_sha256,
        );
        fs::create_dir_all(malicious_path.parent().unwrap()).unwrap();
        fs::write(malicious_path, bundle_bytes).unwrap();
        let catalog_path = write_catalog(second.path(), &catalog);
        let generation = LocalAreaGeneration::open_exact(second.path(), catalog_path).unwrap();
        let error = generation.read_bundle_exact("square").unwrap_err();
        assert!(error.to_string().contains("bundle/WorldPack mismatch"));
    }

    #[test]
    fn rejects_aggregate_budget_before_fallback_or_bundle_io() {
        let temp = tempfile::tempdir().unwrap();
        let release = built_release();
        let mut catalog = release.catalog;
        for entry in &mut catalog.entries {
            entry.stored_bytes = MAX_TOTAL_AREA_BYTES / 2;
            entry.expanded_bytes = entry.stored_bytes;
        }
        let path = write_catalog(temp.path(), &catalog);
        let error = LocalAreaGeneration::open_exact(temp.path(), path).unwrap_err();
        assert!(error.to_string().contains("area bundle set declares"));
        assert!(!temp.path().join("area-worlds").exists());
        assert!(!temp.path().join("area-bundles").exists());
    }

    #[test]
    fn dependency_closure_deduplicates_cycles_and_rejects_more_than_thirty_two() {
        let temp = tempfile::tempdir().unwrap();
        let release = built_release();
        let mut cyclic = release.catalog.clone();
        cyclic.entries[0].hard_dependencies = vec![cyclic.entries[1].area_id.clone()];
        cyclic.entries[1].hard_dependencies = vec![cyclic.entries[2].area_id.clone()];
        cyclic.entries[2].hard_dependencies = vec![cyclic.entries[0].area_id.clone()];
        let world_path =
            local_area_world_path(temp.path(), &cyclic.world_id, &cyclic.world_artifact_sha256);
        fs::create_dir_all(world_path.parent().unwrap()).unwrap();
        fs::write(world_path, &release.world_artifact).unwrap();
        let path = write_catalog(temp.path(), &cyclic);
        let generation = LocalAreaGeneration::open_exact(temp.path(), path).unwrap();
        let closure = generation
            .hard_dependency_closure(&cyclic.entries[0].area_id)
            .unwrap();
        assert_eq!(closure.len(), 3);
        assert!(
            closure
                .windows(2)
                .all(|pair| pair[0].area_id < pair[1].area_id)
        );

        let second = tempfile::tempdir().unwrap();
        let mut oversized = release.catalog.clone();
        let template = oversized.entries[0].clone();
        for index in 0..30 {
            let mut entry = template.clone();
            entry.area_id = format!("extra-{index:02}");
            entry.revision = (index + 10) as u64;
            entry.ownership = empty_ownership();
            entry.hard_dependencies.clear();
            entry.portals.clear();
            entry.asset_manifests.clear();
            entry.lua_modules.clear();
            oversized.entries.push(entry);
        }
        oversized
            .entries
            .sort_by(|left, right| left.area_id.cmp(&right.area_id));
        let ids = oversized
            .entries
            .iter()
            .map(|entry| entry.area_id.clone())
            .collect::<Vec<_>>();
        for (index, entry) in oversized.entries.iter_mut().enumerate() {
            entry.hard_dependencies = ids.get(index + 1).cloned().into_iter().collect();
        }
        let world_path = local_area_world_path(
            second.path(),
            &oversized.world_id,
            &oversized.world_artifact_sha256,
        );
        fs::create_dir_all(world_path.parent().unwrap()).unwrap();
        fs::write(world_path, &release.world_artifact).unwrap();
        let path = write_catalog(second.path(), &oversized);
        let error = LocalAreaGeneration::open_exact(second.path(), path).unwrap_err();
        assert!(error.to_string().contains("exceeds 32 areas"));
    }

    #[test]
    fn rejects_non_exact_catalog_path_and_symlinked_files() {
        let temp = tempfile::tempdir().unwrap();
        let release = built_release();
        let catalog_path = write_release(temp.path(), &release);
        let wrong_path = catalog_path.parent().unwrap().join("wrong.catalog.json");
        fs::write(&wrong_path, fs::read(&catalog_path).unwrap()).unwrap();
        let error = LocalAreaGeneration::open_exact(temp.path(), wrong_path).unwrap_err();
        assert!(error.to_string().contains("exact content-addressed"));
        let outside_path = temp.path().parent().unwrap().join("outside.catalog.json");
        let error = LocalAreaGeneration::open_exact(temp.path(), outside_path).unwrap_err();
        assert!(
            error
                .to_string()
                .contains("escapes its configured registry")
        );

        #[cfg(unix)]
        {
            use std::os::unix::fs::symlink;

            let outside = temp.path().join("outside.catalog.json");
            fs::write(&outside, fs::read(&catalog_path).unwrap()).unwrap();
            fs::remove_file(&catalog_path).unwrap();
            symlink(outside, &catalog_path).unwrap();
            let error = LocalAreaGeneration::open_exact(temp.path(), &catalog_path).unwrap_err();
            assert!(error.to_string().contains("non-symlink"));
        }
    }

    fn empty_ownership() -> AreaOwnership {
        AreaOwnership {
            rooms: Vec::new(),
            item_prototypes: Vec::new(),
            initial_item_instances: Vec::new(),
            npc_prototypes: Vec::new(),
            npc_spawns: Vec::new(),
            loot_tables: Vec::new(),
            loot_sources: Vec::new(),
            affixes: Vec::new(),
            affix_pools: Vec::new(),
            archetypes: Vec::new(),
            lineages: Vec::new(),
            backgrounds: Vec::new(),
            base_classes: Vec::new(),
            advanced_classes: Vec::new(),
            talent_graphs: Vec::new(),
            talent_nodes: Vec::new(),
            abilities: Vec::new(),
            social_emotes: Vec::new(),
            npc_ability_sets: Vec::new(),
            room_media: Vec::new(),
            entity_media: Vec::new(),
            room_perception: Vec::new(),
            scene_variant_sets: Vec::new(),
            scene_variants: Vec::new(),
            scripts: Vec::new(),
            content_tests: Vec::new(),
        }
    }
}
