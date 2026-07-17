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

use serde::{Deserialize, Serialize};
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
pub const MAX_LOCAL_EVICTION_UNITS: usize = crate::MAX_AREA_COUNT;
pub const MAX_LOCAL_AREA_POINTER_BYTES: usize = 16 * 1024;

#[derive(Debug, Clone, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ActiveAreaCatalog {
    pub schema: u8,
    pub world_id: String,
    pub catalog_sha256: String,
    pub catalog: String,
}

/// Exact bytes observed while resolving one cooperative `area-active` update.
///
/// The catalog digest is the runtime generation identity. The pointer digest
/// additionally lets a caller compare-and-swap a candidate acquired through
/// `open_active` without treating the mutable editorial pointer as authority.
#[derive(Debug, Clone, Eq, PartialEq)]
pub struct ActiveAreaCatalogIdentity {
    pointer_path: PathBuf,
    pointer_sha256: String,
    catalog_sha256: String,
}

impl ActiveAreaCatalogIdentity {
    pub fn pointer_path(&self) -> &Path {
        &self.pointer_path
    }

    pub fn pointer_sha256(&self) -> &str {
        &self.pointer_sha256
    }

    pub fn catalog_sha256(&self) -> &str {
        &self.catalog_sha256
    }
}

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
    #[error("the cooperating area publication lock is busy")]
    PublicationLockBusy,
    #[error("area catalog verification failed: {0}")]
    Catalog(#[from] AreaCatalogError),
}

/// Owned shared advisory lock for one world's mutable editorial pointer.
/// Cooperating `area-publish` and `area-select` writers cannot move
/// `area-active` until this value is dropped.
#[cfg(unix)]
#[derive(Debug)]
pub struct LocalAreaPublicationReadGuard {
    file: fs::File,
}

#[cfg(unix)]
impl Drop for LocalAreaPublicationReadGuard {
    fn drop(&mut self) {
        let _ = compatible_unix_lock(&self.file, rustix::fs::FlockOperation::Unlock);
    }
}

#[cfg(unix)]
fn compatible_unix_lock(
    file: &fs::File,
    operation: rustix::fs::FlockOperation,
) -> rustix::io::Result<()> {
    #[cfg(not(target_os = "solaris"))]
    return rustix::fs::flock(file, operation);

    #[cfg(target_os = "solaris")]
    return rustix::fs::fcntl_lock(file, operation);
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
    eviction_index: LocalAreaEvictionIndex,
    room_owners: BTreeMap<String, String>,
    active_pointer_identity: Option<ActiveAreaCatalogIdentity>,
}

/// Generation-local index used to keep hard-dependency cycles indivisible.
///
/// Unit numbers are deliberately private implementation details. Public
/// queries return only catalog entries in canonical order, so this index does
/// not create another stable identifier or protocol surface.
#[derive(Debug, Clone, Eq, PartialEq)]
struct LocalAreaEvictionIndex {
    area_to_unit: Vec<usize>,
    units: Vec<LocalAreaEvictionUnit>,
}

#[derive(Debug, Clone, Eq, PartialEq)]
struct LocalAreaEvictionUnit {
    area_indices: Vec<usize>,
    hard_dependency_units: Vec<usize>,
    hard_dependent_units: Vec<usize>,
    eviction_closure: Vec<usize>,
}

impl LocalAreaEvictionIndex {
    fn build(
        catalog: &AreaCatalog,
        area_index: &BTreeMap<String, usize>,
    ) -> Result<Self, LocalAreaRegistryError> {
        let area_count = catalog.entries.len();
        if area_count == 0
            || area_count > MAX_LOCAL_EVICTION_UNITS
            || area_index.len() != area_count
        {
            return Err(LocalAreaRegistryError::Invalid(format!(
                "eviction-unit index requires 1..={MAX_LOCAL_EVICTION_UNITS} unique areas"
            )));
        }

        // With at most 256 catalog entries, an explicit transitive-reachability
        // matrix is smaller and easier to audit than an unbounded recursive SCC
        // walk. It also makes the inverse eviction closure deterministic.
        let mut reachable = vec![vec![false; area_count]; area_count];
        for (source_index, entry) in catalog.entries.iter().enumerate() {
            if area_index.get(&entry.area_id) != Some(&source_index) {
                return Err(LocalAreaRegistryError::Invalid(
                    "eviction-unit area index does not match canonical catalog order".into(),
                ));
            }
            if entry
                .hard_dependencies
                .windows(2)
                .any(|pair| pair[0] >= pair[1])
            {
                return Err(LocalAreaRegistryError::Invalid(format!(
                    "hard dependencies for area {:?} are not strictly sorted",
                    entry.area_id
                )));
            }
            reachable[source_index][source_index] = true;
            for dependency in &entry.hard_dependencies {
                let dependency_index = area_index.get(dependency).copied().ok_or_else(|| {
                    LocalAreaRegistryError::Invalid(format!(
                        "hard dependency {dependency:?} for area {:?} is unknown",
                        entry.area_id
                    ))
                })?;
                if dependency_index == source_index {
                    return Err(LocalAreaRegistryError::Invalid(format!(
                        "area {:?} cannot hard-depend on itself",
                        entry.area_id
                    )));
                }
                reachable[source_index][dependency_index] = true;
            }
        }

        for intermediate in 0..area_count {
            let intermediate_reachability = reachable[intermediate].clone();
            for source_reachability in &mut reachable {
                if !source_reachability[intermediate] {
                    continue;
                }
                for (target, intermediate_target) in source_reachability
                    .iter_mut()
                    .zip(&intermediate_reachability)
                {
                    *target |= *intermediate_target;
                }
            }
        }

        let mut area_to_unit = vec![usize::MAX; area_count];
        let mut unit_areas = Vec::new();
        for source in 0..area_count {
            if area_to_unit[source] != usize::MAX {
                continue;
            }
            let unit_index = unit_areas.len();
            let mut areas = Vec::new();
            for candidate in source..area_count {
                if area_to_unit[candidate] == usize::MAX
                    && reachable[source][candidate]
                    && reachable[candidate][source]
                {
                    area_to_unit[candidate] = unit_index;
                    areas.push(candidate);
                }
            }
            if areas.is_empty() {
                return Err(LocalAreaRegistryError::Invalid(
                    "eviction-unit SCC construction produced an empty unit".into(),
                ));
            }
            unit_areas.push(areas);
        }
        if unit_areas.is_empty() || unit_areas.len() > MAX_LOCAL_EVICTION_UNITS {
            return Err(LocalAreaRegistryError::Invalid(
                "eviction-unit count exceeds its catalog bound".into(),
            ));
        }

        let mut dependency_units = vec![BTreeSet::new(); unit_areas.len()];
        let mut dependent_units = vec![BTreeSet::new(); unit_areas.len()];
        for (source_index, entry) in catalog.entries.iter().enumerate() {
            let source_unit = area_to_unit[source_index];
            for dependency in &entry.hard_dependencies {
                let dependency_index = area_index[dependency];
                let dependency_unit = area_to_unit[dependency_index];
                if source_unit != dependency_unit {
                    dependency_units[source_unit].insert(dependency_unit);
                    dependent_units[dependency_unit].insert(source_unit);
                }
            }
        }

        let mut units = Vec::with_capacity(unit_areas.len());
        for (unit_index, area_indices) in unit_areas.into_iter().enumerate() {
            let mut pending = VecDeque::from([unit_index]);
            let mut eviction_closure = BTreeSet::new();
            while let Some(current) = pending.pop_front() {
                if !eviction_closure.insert(current) {
                    continue;
                }
                if eviction_closure.len() > MAX_LOCAL_EVICTION_UNITS {
                    return Err(LocalAreaRegistryError::Invalid(
                        "inverse eviction closure exceeds its catalog bound".into(),
                    ));
                }
                pending.extend(dependent_units[current].iter().copied());
            }
            units.push(LocalAreaEvictionUnit {
                area_indices,
                hard_dependency_units: dependency_units[unit_index].iter().copied().collect(),
                hard_dependent_units: dependent_units[unit_index].iter().copied().collect(),
                eviction_closure: eviction_closure.into_iter().collect(),
            });
        }

        let index = Self {
            area_to_unit,
            units,
        };
        index.validate(catalog, area_index)?;
        Ok(index)
    }

    fn validate(
        &self,
        catalog: &AreaCatalog,
        area_index: &BTreeMap<String, usize>,
    ) -> Result<(), LocalAreaRegistryError> {
        if self.area_to_unit.len() != catalog.entries.len()
            || self.units.is_empty()
            || self.units.len() > MAX_LOCAL_EVICTION_UNITS
        {
            return Err(LocalAreaRegistryError::Invalid(
                "eviction-unit index has invalid bounded cardinality".into(),
            ));
        }

        let mut covered_areas = BTreeSet::new();
        for (unit_index, unit) in self.units.iter().enumerate() {
            if unit.area_indices.is_empty()
                || unit.area_indices.windows(2).any(|pair| pair[0] >= pair[1])
                || unit
                    .hard_dependency_units
                    .windows(2)
                    .any(|pair| pair[0] >= pair[1])
                || unit
                    .hard_dependent_units
                    .windows(2)
                    .any(|pair| pair[0] >= pair[1])
                || unit
                    .eviction_closure
                    .windows(2)
                    .any(|pair| pair[0] >= pair[1])
                || unit.eviction_closure.binary_search(&unit_index).is_err()
            {
                return Err(LocalAreaRegistryError::Invalid(
                    "eviction-unit index is not canonical".into(),
                ));
            }
            for &area in &unit.area_indices {
                if area >= catalog.entries.len()
                    || self.area_to_unit[area] != unit_index
                    || !covered_areas.insert(area)
                {
                    return Err(LocalAreaRegistryError::Invalid(
                        "eviction-unit index does not partition catalog areas".into(),
                    ));
                }
            }
            for &dependency in &unit.hard_dependency_units {
                if dependency >= self.units.len()
                    || dependency == unit_index
                    || self.units[dependency]
                        .hard_dependent_units
                        .binary_search(&unit_index)
                        .is_err()
                {
                    return Err(LocalAreaRegistryError::Invalid(
                        "eviction-unit dependency inverse is inconsistent".into(),
                    ));
                }
            }
            for &dependent in &unit.hard_dependent_units {
                if dependent >= self.units.len()
                    || dependent == unit_index
                    || self.units[dependent]
                        .hard_dependency_units
                        .binary_search(&unit_index)
                        .is_err()
                {
                    return Err(LocalAreaRegistryError::Invalid(
                        "eviction-unit dependent inverse is inconsistent".into(),
                    ));
                }
            }
        }
        if covered_areas.len() != catalog.entries.len() {
            return Err(LocalAreaRegistryError::Invalid(
                "eviction-unit index omits catalog areas".into(),
            ));
        }

        // No unit outside an eviction closure may retain a hard dependency on
        // a unit inside it. That property is the future runtime safety boundary
        // this generation-local index supplies; it does not itself unload.
        for (unit_index, unit) in self.units.iter().enumerate() {
            for outside in 0..self.units.len() {
                if unit.eviction_closure.binary_search(&outside).is_ok() {
                    continue;
                }
                if self.units[outside]
                    .hard_dependency_units
                    .iter()
                    .any(|dependency| unit.eviction_closure.binary_search(dependency).is_ok())
                {
                    return Err(LocalAreaRegistryError::Invalid(format!(
                        "eviction closure for canonical unit {unit_index} leaves an active hard dependent"
                    )));
                }
            }
        }

        for (area_id, &area) in area_index {
            if catalog
                .entries
                .get(area)
                .map(|entry| entry.area_id.as_str())
                != Some(area_id.as_str())
            {
                return Err(LocalAreaRegistryError::Invalid(
                    "eviction-unit validation found a non-canonical area index".into(),
                ));
            }
        }
        Ok(())
    }
}

#[derive(Debug, Clone)]
pub struct StagedLocalAreaGenerationReplacement {
    kind: GenerationReplacementKind,
    expected_current_generation_id: String,
    candidate_generation_id: String,
    candidate_active_pointer_identity: Option<ActiveAreaCatalogIdentity>,
    changed_area_ids: Vec<String>,
    candidate: LocalAreaGeneration,
}

impl StagedLocalAreaGenerationReplacement {
    pub fn kind(&self) -> GenerationReplacementKind {
        self.kind
    }

    pub fn expected_current_generation_id(&self) -> &str {
        &self.expected_current_generation_id
    }

    pub fn candidate_generation_id(&self) -> &str {
        &self.candidate_generation_id
    }

    pub fn candidate_active_pointer_identity(&self) -> Option<&ActiveAreaCatalogIdentity> {
        self.candidate_active_pointer_identity.as_ref()
    }

    pub fn changed_area_ids(&self) -> &[String] {
        &self.changed_area_ids
    }

    pub fn candidate(&self) -> &LocalAreaGeneration {
        &self.candidate
    }

    pub fn into_candidate(self) -> LocalAreaGeneration {
        self.candidate
    }
}

#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub enum GenerationReplacementKind {
    Activate,
    Rollback,
    Noop,
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

        let area_index = catalog
            .entries
            .iter()
            .enumerate()
            .map(|(index, entry)| (entry.area_id.clone(), index))
            .collect::<BTreeMap<_, _>>();
        let eviction_index = LocalAreaEvictionIndex::build(&catalog, &area_index)?;

        let world_path =
            local_area_world_path(&registry, &catalog.world_id, &catalog.world_artifact_sha256);
        let world_artifact = read_bounded_local_registry_file(
            &registry,
            &world_path,
            MAX_COMPOSED_WORLD_BYTES,
            "monolithic WorldPack fallback",
        )?;
        let fallback = verify_area_catalog_fallback(&catalog, &world_artifact)?;
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
            eviction_index,
            room_owners,
            active_pointer_identity: None,
        };
        for entry in &generation.catalog.entries {
            generation.hard_dependency_closure(entry.area_id.as_str())?;
        }
        Ok(generation)
    }

    /// Resolves one mutable local `area-active` pointer into an immutable exact
    /// generation. The pointer is read strictly before and after opening the
    /// catalog and fallback; a cooperating publication that changes during
    /// acquisition is rejected rather than mixed with the candidate.
    pub fn open_active(
        registry: impl AsRef<Path>,
        world_id: &str,
    ) -> Result<Self, LocalAreaRegistryError> {
        Self::open_active_with_observer(registry.as_ref(), world_id, || {})
    }

    fn open_active_with_observer(
        registry: &Path,
        world_id: &str,
        after_generation_open: impl FnOnce(),
    ) -> Result<Self, LocalAreaRegistryError> {
        let pointer_path = local_active_area_catalog_path(registry, world_id);
        let pointer_bytes = read_bounded_local_registry_file(
            registry,
            &pointer_path,
            MAX_LOCAL_AREA_POINTER_BYTES,
            "active area catalog pointer",
        )?;
        let pointer: ActiveAreaCatalog = crate::area_catalog::parse_bounded(
            &pointer_bytes,
            MAX_LOCAL_AREA_POINTER_BYTES,
            "active area catalog pointer",
        )
        .map_err(|error| LocalAreaRegistryError::Invalid(error.to_string()))?;
        if pointer.schema != 1 || pointer.world_id != world_id {
            return Err(LocalAreaRegistryError::Invalid(
                "active catalog pointer has the wrong schema or world".into(),
            ));
        }
        if !valid_digest(&pointer.catalog_sha256) {
            return Err(LocalAreaRegistryError::Invalid(
                "active catalog pointer has an invalid catalog digest".into(),
            ));
        }
        let catalog_path = local_area_catalog_path(registry, world_id, &pointer.catalog_sha256);
        let expected_relative = registry_relative_path(registry, &catalog_path)?;
        if pointer.catalog != expected_relative {
            return Err(LocalAreaRegistryError::Invalid(
                "active catalog pointer does not name its content-addressed catalog".into(),
            ));
        }
        let mut generation = Self::open_exact(registry, &catalog_path)?;
        if generation.generation_id != pointer.catalog_sha256
            || generation.catalog.world_id != world_id
        {
            return Err(LocalAreaRegistryError::Invalid(
                "active catalog pointer digest/world mismatch".into(),
            ));
        }

        after_generation_open();
        let final_pointer_bytes = read_bounded_local_registry_file(
            registry,
            &pointer_path,
            MAX_LOCAL_AREA_POINTER_BYTES,
            "active area catalog pointer",
        )?;
        if final_pointer_bytes != pointer_bytes {
            return Err(LocalAreaRegistryError::Invalid(
                "active catalog pointer changed while the generation was opened".into(),
            ));
        }
        generation.active_pointer_identity = Some(ActiveAreaCatalogIdentity {
            pointer_path,
            pointer_sha256: sha256_hex(&pointer_bytes),
            catalog_sha256: pointer.catalog_sha256,
        });
        Ok(generation)
    }

    pub fn generation_id(&self) -> &str {
        &self.generation_id
    }

    pub fn registry_path(&self) -> &Path {
        &self.registry
    }

    pub fn active_pointer_identity(&self) -> Option<&ActiveAreaCatalogIdentity> {
        self.active_pointer_identity.as_ref()
    }

    /// Confirms that the mutable editorial pointer still contains the exact
    /// bytes from which this candidate was acquired. Exact-path generations do
    /// not carry a pointer identity and therefore cannot pass this CAS gate.
    pub fn revalidate_active_pointer_identity(&self) -> Result<(), LocalAreaRegistryError> {
        let identity = self.active_pointer_identity.as_ref().ok_or_else(|| {
            LocalAreaRegistryError::Invalid(
                "generation was not acquired through an active catalog pointer".into(),
            )
        })?;
        let pointer_bytes = read_bounded_local_registry_file(
            &self.registry,
            &identity.pointer_path,
            MAX_LOCAL_AREA_POINTER_BYTES,
            "active area catalog pointer",
        )?;
        if sha256_hex(&pointer_bytes) != identity.pointer_sha256 {
            return Err(LocalAreaRegistryError::Invalid(
                "active catalog pointer changed after candidate acquisition".into(),
            ));
        }
        Ok(())
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

    /// Resolves the indivisible SCC that contains `area_id`.
    ///
    /// Entries are returned in catalog order. The unit has no public ID and is
    /// valid only for this immutable generation.
    pub fn eviction_unit_for_area(
        &self,
        area_id: &str,
    ) -> Result<Vec<&AreaCatalogEntry>, LocalAreaRegistryError> {
        let area =
            self.area_index.get(area_id).copied().ok_or_else(|| {
                LocalAreaRegistryError::Invalid(format!("unknown area {area_id:?}"))
            })?;
        let unit = self.eviction_index.area_to_unit[area];
        Ok(self.eviction_index.units[unit]
            .area_indices
            .iter()
            .map(|&index| &self.catalog.entries[index])
            .collect())
    }

    /// Resolves the complete inverse hard-dependency closure for eviction.
    ///
    /// The result includes the area's SCC and every SCC that depends on it,
    /// directly or transitively. A future runtime must ensure every active unit
    /// in this set leaves before or atomically with the target; units that are
    /// already absent need no transition. No unit outside this closure has a
    /// hard dependency on one inside it. Both units and entries inside each
    /// unit use canonical catalog order.
    pub fn eviction_closure_for_area(
        &self,
        area_id: &str,
    ) -> Result<Vec<Vec<&AreaCatalogEntry>>, LocalAreaRegistryError> {
        let area =
            self.area_index.get(area_id).copied().ok_or_else(|| {
                LocalAreaRegistryError::Invalid(format!("unknown area {area_id:?}"))
            })?;
        let unit = self.eviction_index.area_to_unit[area];
        Ok(self.eviction_index.units[unit]
            .eviction_closure
            .iter()
            .map(|&closure_unit| {
                self.eviction_index.units[closure_unit]
                    .area_indices
                    .iter()
                    .map(|&index| &self.catalog.entries[index])
                    .collect()
            })
            .collect())
    }

    /// Rebuilds and revalidates the bounded SCC partition and its
    /// direct/inverse indexes against the immutable catalog.
    ///
    /// This is a pure generation-local check and does not perform eviction.
    pub fn validate_eviction_units(&self) -> Result<(), LocalAreaRegistryError> {
        let rebuilt = LocalAreaEvictionIndex::build(&self.catalog, &self.area_index)?;
        if rebuilt != self.eviction_index {
            return Err(LocalAreaRegistryError::Invalid(
                "eviction-unit index differs from its immutable catalog".into(),
            ));
        }
        Ok(())
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

/// Performs the deliberately narrow 6C full-generation compatibility gate.
///
/// Both releases are re-opened completely before comparison. Runtime gameplay,
/// durable authored items and NPC/loot/affix definitions must remain byte-for-
/// byte equivalent. A candidate may change room prose/topology/assets plus the
/// reviewed presentation, perception and script layers, provided stable room
/// identity and legacy VNUM mapping do not change.
pub fn validate_generation_replacement(
    current: &LocalAreaGeneration,
    next: &LocalAreaGeneration,
) -> Result<GenerationReplacementKind, LocalAreaRegistryError> {
    current.verify_complete_release()?;
    next.verify_complete_release()?;
    let current_world = &current.fallback.world;
    let next_world = &next.fallback.world;
    if current.catalog.world_id != next.catalog.world_id
        || current_world.id != next_world.id
        || current_world.name != next_world.name
        || current_world.start_room_id != next_world.start_room_id
    {
        return Err(LocalAreaRegistryError::Invalid(
            "generation replacement changes world or start-room identity".into(),
        ));
    }
    if current_world.item_prototypes != next_world.item_prototypes
        || current_world.initial_item_instances != next_world.initial_item_instances
        || current_world.gameplay != next_world.gameplay
    {
        return Err(LocalAreaRegistryError::Invalid(
            "generation replacement changes durable gameplay content".into(),
        ));
    }
    let current_rooms = room_identity_map(&current_world.rooms)?;
    let next_rooms = room_identity_map(&next_world.rooms)?;
    if current_rooms != next_rooms {
        return Err(LocalAreaRegistryError::Invalid(
            "generation replacement changes stable room or legacy VNUM identity".into(),
        ));
    }
    let current_areas = current
        .catalog
        .entries
        .iter()
        .map(|entry| (entry.area_id.as_str(), entry))
        .collect::<BTreeMap<_, _>>();
    let next_areas = next
        .catalog
        .entries
        .iter()
        .map(|entry| (entry.area_id.as_str(), entry))
        .collect::<BTreeMap<_, _>>();
    if current_areas.len() != next_areas.len()
        || current_areas.keys().ne(next_areas.keys())
        || current_areas.iter().any(|(area_id, current_entry)| {
            next_areas
                .get(area_id)
                .is_none_or(|next_entry| current_entry.ownership != next_entry.ownership)
        })
    {
        return Err(LocalAreaRegistryError::Invalid(
            "generation replacement changes area or stable-ID ownership".into(),
        ));
    }
    if current
        .catalog
        .entries
        .iter()
        .chain(next.catalog.entries.iter())
        .any(|entry| entry.compatibility.persistent_migration_version != 0)
    {
        return Err(LocalAreaRegistryError::Invalid(
            "generation replacement requires a persistent migration".into(),
        ));
    }

    let kind = match next
        .catalog
        .catalog_revision
        .cmp(&current.catalog.catalog_revision)
    {
        std::cmp::Ordering::Greater => GenerationReplacementKind::Activate,
        std::cmp::Ordering::Less => GenerationReplacementKind::Rollback,
        std::cmp::Ordering::Equal if next.generation_id == current.generation_id => {
            GenerationReplacementKind::Noop
        }
        std::cmp::Ordering::Equal => {
            return Err(LocalAreaRegistryError::Invalid(
                "different generations reuse one catalog revision".into(),
            ));
        }
    };
    for (area_id, current_entry) in current_areas {
        let next_entry = next_areas[area_id];
        let revision_order = next_entry.revision.cmp(&current_entry.revision);
        let bundle_changed = next_entry.bundle_sha256 != current_entry.bundle_sha256;
        let valid = match kind {
            GenerationReplacementKind::Activate => {
                (bundle_changed && revision_order.is_gt())
                    || (!bundle_changed && revision_order.is_eq())
            }
            GenerationReplacementKind::Rollback => {
                (bundle_changed && revision_order.is_lt())
                    || (!bundle_changed && revision_order.is_eq())
            }
            GenerationReplacementKind::Noop => !bundle_changed && revision_order.is_eq(),
        };
        if !valid {
            return Err(LocalAreaRegistryError::Invalid(
                "area revision/digest does not follow generation direction".into(),
            ));
        }
    }
    Ok(kind)
}

/// Verifies and packages all immutable inputs required by a server-side 6C
/// compare-and-swap. It deliberately returns world-pack pieces rather than
/// depending on server runtime types.
pub fn stage_generation_replacement(
    current: &LocalAreaGeneration,
    candidate: LocalAreaGeneration,
) -> Result<StagedLocalAreaGenerationReplacement, LocalAreaRegistryError> {
    stage_generation_with_kind(current, candidate, GenerationReplacementKind::Activate)
}

/// Explicit reverse-editorial path. A lower catalog revision is never accepted
/// through the ordinary activation API.
pub fn stage_generation_rollback(
    current: &LocalAreaGeneration,
    candidate: LocalAreaGeneration,
) -> Result<StagedLocalAreaGenerationReplacement, LocalAreaRegistryError> {
    stage_generation_with_kind(current, candidate, GenerationReplacementKind::Rollback)
}

fn stage_generation_with_kind(
    current: &LocalAreaGeneration,
    candidate: LocalAreaGeneration,
    expected: GenerationReplacementKind,
) -> Result<StagedLocalAreaGenerationReplacement, LocalAreaRegistryError> {
    let kind = validate_generation_replacement(current, &candidate)?;
    if candidate.active_pointer_identity.is_some() {
        candidate.revalidate_active_pointer_identity()?;
    }
    if kind != expected && kind != GenerationReplacementKind::Noop {
        return Err(LocalAreaRegistryError::Invalid(
            match kind {
                GenerationReplacementKind::Activate => {
                    "newer generation requires the activation staging API"
                }
                GenerationReplacementKind::Rollback => {
                    "older generation requires the explicit rollback staging API"
                }
                GenerationReplacementKind::Noop => unreachable!(),
            }
            .into(),
        ));
    }
    let current_entries = current
        .catalog
        .entries
        .iter()
        .map(|entry| (entry.area_id.as_str(), entry))
        .collect::<BTreeMap<_, _>>();
    let candidate_entries = candidate
        .catalog
        .entries
        .iter()
        .map(|entry| (entry.area_id.as_str(), entry))
        .collect::<BTreeMap<_, _>>();
    let changed_area_ids = current_entries
        .keys()
        .chain(candidate_entries.keys())
        .copied()
        .collect::<BTreeSet<_>>()
        .into_iter()
        .filter(|area_id| current_entries.get(area_id) != candidate_entries.get(area_id))
        .map(str::to_owned)
        .collect();
    Ok(StagedLocalAreaGenerationReplacement {
        kind,
        expected_current_generation_id: current.generation_id.clone(),
        candidate_generation_id: candidate.generation_id.clone(),
        candidate_active_pointer_identity: candidate.active_pointer_identity.clone(),
        changed_area_ids,
        candidate,
    })
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

pub fn local_active_area_catalog_path(registry: &Path, world_id: &str) -> PathBuf {
    registry
        .join("area-active")
        .join(format!("{}.json", storage_id(world_id)))
}

pub fn local_area_publication_lock_path(registry: &Path, world_id: &str) -> PathBuf {
    registry
        .join("area-locks")
        .join(format!("{}.lock", storage_id(world_id)))
}

/// Attempts to pin the cooperative editorial pointer without blocking a
/// runtime thread. The lock file is created durably by `area-publish`; runtime
/// reconciliation refuses missing, replaced or symlinked lock identities.
#[cfg(unix)]
pub fn try_acquire_local_area_publication_read_lock(
    registry: &Path,
    world_id: &str,
) -> Result<LocalAreaPublicationReadGuard, LocalAreaRegistryError> {
    use std::os::unix::fs::MetadataExt;

    let path = local_area_publication_lock_path(registry, world_id);
    let parent = path.parent().ok_or_else(|| {
        LocalAreaRegistryError::Invalid("area publication lock has no parent".into())
    })?;
    verify_existing_registry_directory(registry, parent)?;
    let before = fs::symlink_metadata(&path).map_err(|source| LocalAreaRegistryError::Inspect {
        path: path.clone(),
        source,
    })?;
    if before.file_type().is_symlink() || !before.file_type().is_file() {
        return Err(LocalAreaRegistryError::Invalid(
            "area publication lock is not a regular non-symlink file".into(),
        ));
    }
    let file = OpenOptions::new()
        .read(true)
        .write(true)
        .open(&path)
        .map_err(|source| LocalAreaRegistryError::Read {
            path: path.clone(),
            source,
        })?;
    let opened = file
        .metadata()
        .map_err(|source| LocalAreaRegistryError::Inspect {
            path: path.clone(),
            source,
        })?;
    if !opened.file_type().is_file() || before.dev() != opened.dev() || before.ino() != opened.ino()
    {
        return Err(LocalAreaRegistryError::Invalid(
            "area publication lock changed while it was opened".into(),
        ));
    }
    compatible_unix_lock(&file, rustix::fs::FlockOperation::NonBlockingLockShared).map_err(
        |error| {
            if error.kind() == std::io::ErrorKind::WouldBlock {
                LocalAreaRegistryError::PublicationLockBusy
            } else {
                LocalAreaRegistryError::Read {
                    path: path.clone(),
                    source: error.into(),
                }
            }
        },
    )?;
    let current =
        fs::symlink_metadata(&path).map_err(|source| LocalAreaRegistryError::Inspect {
            path: path.clone(),
            source,
        })?;
    if current.file_type().is_symlink()
        || !current.file_type().is_file()
        || current.dev() != opened.dev()
        || current.ino() != opened.ino()
    {
        let _ = compatible_unix_lock(&file, rustix::fs::FlockOperation::Unlock);
        return Err(LocalAreaRegistryError::Invalid(
            "area publication lock changed while it was acquired".into(),
        ));
    }
    Ok(LocalAreaPublicationReadGuard { file })
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

fn registry_relative_path(registry: &Path, path: &Path) -> Result<String, LocalAreaRegistryError> {
    let relative = path.strip_prefix(registry).map_err(|_| {
        LocalAreaRegistryError::Invalid("registry pointer path escapes its root".into())
    })?;
    if relative
        .components()
        .any(|component| !matches!(component, Component::Normal(_)))
    {
        return Err(LocalAreaRegistryError::Invalid(
            "registry pointer path contains traversal".into(),
        ));
    }
    Ok(relative.to_string_lossy().replace('\\', "/"))
}

fn room_identity_map(
    rooms: &[crate::Room],
) -> Result<BTreeMap<&str, Option<i64>>, LocalAreaRegistryError> {
    let mut identities = BTreeMap::new();
    for room in rooms {
        if identities
            .insert(room.id.as_str(), room.legacy_vnum)
            .is_some()
        {
            return Err(LocalAreaRegistryError::Invalid(
                "verified fallback contains duplicate stable room identity".into(),
            ));
        }
    }
    Ok(identities)
}

fn valid_digest(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || matches!(byte, b'a'..=b'f'))
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
        ComposedWorld, GenerationReplacementKind, ReleaseTrustMode, VerifiedAreaRelease,
        build_area_release, compose_world_project,
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

    fn write_active_pointer(
        registry: &Path,
        catalog_path: &Path,
        catalog: &AreaCatalog,
        catalog_sha256: &str,
    ) -> PathBuf {
        let path = local_active_area_catalog_path(registry, &catalog.world_id);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        let pointer = ActiveAreaCatalog {
            schema: 1,
            world_id: catalog.world_id.clone(),
            catalog_sha256: catalog_sha256.to_owned(),
            catalog: registry_relative_path(registry, catalog_path).unwrap(),
        };
        let mut bytes = serde_json::to_vec_pretty(&pointer).unwrap();
        bytes.push(b'\n');
        fs::write(&path, bytes).unwrap();
        path
    }

    fn finish_changed_release(
        mut composed: ComposedWorld,
        changed_area_ids: &[&str],
        catalog_revision: u64,
    ) -> VerifiedAreaRelease {
        composed.artifact = composed.document.deterministic_artifact().unwrap();
        composed.report.artifact_bytes = composed.artifact.len();
        composed.report.artifact_sha256 = sha256_hex(&composed.artifact);
        for area_id in changed_area_ids {
            let pack = composed
                .area_packs
                .iter()
                .find(|pack| pack.id == *area_id)
                .unwrap();
            let report = composed
                .report
                .areas
                .iter_mut()
                .find(|report| report.id == *area_id)
                .unwrap();
            report.source_sha256 = sha256_hex(&serde_json::to_vec(pack).unwrap());
        }
        let mut release = release_for(&composed);
        release.catalog_revision = catalog_revision;
        for area_id in changed_area_ids {
            release
                .areas
                .iter_mut()
                .find(|entry| entry.area_id == *area_id)
                .unwrap()
                .revision += 1;
        }
        build_area_release(&composed, &release, None).unwrap()
    }

    fn release_with_changed_room_title(catalog_revision: u64) -> VerifiedAreaRelease {
        let mut composed = compose_world_project(project_path()).unwrap();
        let room_id = "demo:lantern-square";
        composed
            .area_packs
            .iter_mut()
            .find(|pack| pack.id == "square")
            .unwrap()
            .rooms
            .iter_mut()
            .find(|room| room.id == room_id)
            .unwrap()
            .title = "Lantern Square after the rain".into();
        composed
            .document
            .world
            .rooms
            .iter_mut()
            .find(|room| room.id == room_id)
            .unwrap()
            .title = "Lantern Square after the rain".into();
        finish_changed_release(composed, &["square"], catalog_revision)
    }

    fn release_with_changed_rng() -> VerifiedAreaRelease {
        let mut composed = compose_world_project(project_path()).unwrap();
        composed.document.world.gameplay.rng_seed = Some(1);
        finish_changed_release(composed, &[], 2)
    }

    fn release_with_changed_room_identity() -> VerifiedAreaRelease {
        let mut composed = compose_world_project(project_path()).unwrap();
        let room_id = "demo:lantern-square";
        composed
            .area_packs
            .iter_mut()
            .find(|pack| pack.id == "square")
            .unwrap()
            .rooms
            .iter_mut()
            .find(|room| room.id == room_id)
            .unwrap()
            .legacy_vnum = Some(9_999);
        composed
            .document
            .world
            .rooms
            .iter_mut()
            .find(|room| room.id == room_id)
            .unwrap()
            .legacy_vnum = Some(9_999);
        finish_changed_release(composed, &["square"], 2)
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
    fn opens_a_strict_stable_active_pointer_and_exposes_its_cas_identity() {
        let temp = tempfile::tempdir().unwrap();
        let release = built_release();
        let catalog_path = write_release(temp.path(), &release);
        let pointer_path = write_active_pointer(
            temp.path(),
            &catalog_path,
            &release.catalog,
            &release.catalog_sha256,
        );

        let generation =
            LocalAreaGeneration::open_active(temp.path(), &release.catalog.world_id).unwrap();
        let identity = generation.active_pointer_identity().unwrap();
        assert_eq!(generation.generation_id(), release.catalog_sha256);
        assert_eq!(identity.pointer_path(), pointer_path);
        assert_eq!(identity.catalog_sha256(), release.catalog_sha256);
        assert_eq!(
            identity.pointer_sha256(),
            sha256_hex(&fs::read(identity.pointer_path()).unwrap())
        );
    }

    #[test]
    fn active_pointer_rejects_tamper_recursive_duplicates_wrong_world_and_symlink() {
        let release = built_release();

        let tampered = tempfile::tempdir().unwrap();
        let catalog_path = write_release(tampered.path(), &release);
        write_active_pointer(
            tampered.path(),
            &catalog_path,
            &release.catalog,
            &release.catalog_sha256,
        );
        fs::write(
            &catalog_path,
            [release.catalog_bytes.as_slice(), b" "].concat(),
        )
        .unwrap();
        assert!(
            LocalAreaGeneration::open_active(tampered.path(), &release.catalog.world_id).is_err()
        );

        let duplicate = tempfile::tempdir().unwrap();
        let catalog_path = write_release(duplicate.path(), &release);
        let pointer_path = write_active_pointer(
            duplicate.path(),
            &catalog_path,
            &release.catalog,
            &release.catalog_sha256,
        );
        let relative = registry_relative_path(duplicate.path(), &catalog_path).unwrap();
        fs::write(
            &pointer_path,
            format!(
                "{{\"schema\":1,\"schema\":1,\"worldId\":\"{}\",\"catalogSha256\":\"{}\",\"catalog\":\"{}\"}}",
                release.catalog.world_id, release.catalog_sha256, relative
            ),
        )
        .unwrap();
        let error = LocalAreaGeneration::open_active(duplicate.path(), &release.catalog.world_id)
            .unwrap_err();
        assert!(error.to_string().contains("duplicate JSON key"));

        let wrong_world = tempfile::tempdir().unwrap();
        let catalog_path = write_release(wrong_world.path(), &release);
        let pointer_path = write_active_pointer(
            wrong_world.path(),
            &catalog_path,
            &release.catalog,
            &release.catalog_sha256,
        );
        let mut pointer: ActiveAreaCatalog =
            serde_json::from_slice(&fs::read(&pointer_path).unwrap()).unwrap();
        pointer.world_id = "another-world".into();
        fs::write(&pointer_path, serde_json::to_vec(&pointer).unwrap()).unwrap();
        assert!(
            LocalAreaGeneration::open_active(wrong_world.path(), &release.catalog.world_id)
                .unwrap_err()
                .to_string()
                .contains("wrong schema or world")
        );

        #[cfg(unix)]
        {
            use std::os::unix::fs::symlink;

            let linked = tempfile::tempdir().unwrap();
            let catalog_path = write_release(linked.path(), &release);
            let pointer_path = write_active_pointer(
                linked.path(),
                &catalog_path,
                &release.catalog,
                &release.catalog_sha256,
            );
            let outside = linked.path().join("outside-pointer.json");
            fs::write(&outside, fs::read(&pointer_path).unwrap()).unwrap();
            fs::remove_file(&pointer_path).unwrap();
            symlink(outside, &pointer_path).unwrap();
            assert!(
                LocalAreaGeneration::open_active(linked.path(), &release.catalog.world_id)
                    .unwrap_err()
                    .to_string()
                    .contains("non-symlink")
            );
        }
    }

    #[test]
    fn active_pointer_change_during_generation_open_is_rejected() {
        let temp = tempfile::tempdir().unwrap();
        let release = built_release();
        let catalog_path = write_release(temp.path(), &release);
        let pointer_path = write_active_pointer(
            temp.path(),
            &catalog_path,
            &release.catalog,
            &release.catalog_sha256,
        );
        let pointer_before = fs::read(&pointer_path).unwrap();
        let error = LocalAreaGeneration::open_active_with_observer(
            temp.path(),
            &release.catalog.world_id,
            || {
                let mut changed = pointer_before;
                changed.push(b'\n');
                fs::write(&pointer_path, changed).unwrap();
            },
        )
        .unwrap_err();
        assert!(error.to_string().contains("changed while"));
    }

    #[test]
    fn replacement_gate_is_symmetric_and_requires_explicit_rollback() {
        let current_root = tempfile::tempdir().unwrap();
        let current_release = built_release();
        let current_path = write_release(current_root.path(), &current_release);
        let current = LocalAreaGeneration::open_exact(current_root.path(), current_path).unwrap();

        let next_root = tempfile::tempdir().unwrap();
        let next_release = release_with_changed_room_title(2);
        let next_path = write_release(next_root.path(), &next_release);
        write_active_pointer(
            next_root.path(),
            &next_path,
            &next_release.catalog,
            &next_release.catalog_sha256,
        );
        let next =
            LocalAreaGeneration::open_active(next_root.path(), &next_release.catalog.world_id)
                .unwrap();

        assert_eq!(
            validate_generation_replacement(&current, &next).unwrap(),
            GenerationReplacementKind::Activate
        );
        assert_eq!(
            validate_generation_replacement(&next, &current).unwrap(),
            GenerationReplacementKind::Rollback
        );
        assert_eq!(
            validate_generation_replacement(&current, &current).unwrap(),
            GenerationReplacementKind::Noop
        );
        let staged = stage_generation_replacement(&current, next.clone()).unwrap();
        assert_eq!(staged.kind(), GenerationReplacementKind::Activate);
        assert_eq!(
            staged.expected_current_generation_id(),
            current.generation_id()
        );
        assert_eq!(staged.candidate_generation_id(), next.generation_id());
        assert_eq!(
            staged.changed_area_ids(),
            ["archive", "north-gate", "square"]
        );
        assert!(staged.candidate_active_pointer_identity().is_some());
        let candidate_pointer = next.active_pointer_identity().unwrap().pointer_path();
        let mut changed_pointer = fs::read(candidate_pointer).unwrap();
        changed_pointer.push(b'\n');
        fs::write(candidate_pointer, changed_pointer).unwrap();
        assert!(
            stage_generation_replacement(&current, next.clone())
                .unwrap_err()
                .to_string()
                .contains("changed after candidate acquisition")
        );
        assert!(stage_generation_replacement(&next, current.clone()).is_err());
        assert_eq!(
            stage_generation_rollback(&next, current).unwrap().kind(),
            GenerationReplacementKind::Rollback
        );
    }

    #[test]
    fn replacement_gate_rejects_gameplay_room_identity_and_revision_reuse_both_ways() {
        let current_root = tempfile::tempdir().unwrap();
        let current_release = built_release();
        let current_path = write_release(current_root.path(), &current_release);
        let current = LocalAreaGeneration::open_exact(current_root.path(), current_path).unwrap();

        for incompatible_release in [
            release_with_changed_rng(),
            release_with_changed_room_identity(),
            release_with_changed_room_title(1),
        ] {
            let next_root = tempfile::tempdir().unwrap();
            let next_path = write_release(next_root.path(), &incompatible_release);
            let next = LocalAreaGeneration::open_exact(next_root.path(), next_path).unwrap();
            assert!(validate_generation_replacement(&current, &next).is_err());
            assert!(validate_generation_replacement(&next, &current).is_err());
        }
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
        let eviction_unit = generation
            .eviction_unit_for_area(&cyclic.entries[1].area_id)
            .unwrap();
        assert_eq!(
            eviction_unit
                .iter()
                .map(|entry| entry.area_id.as_str())
                .collect::<Vec<_>>(),
            cyclic
                .entries
                .iter()
                .map(|entry| entry.area_id.as_str())
                .collect::<Vec<_>>()
        );
        let eviction_closure = generation
            .eviction_closure_for_area(&cyclic.entries[2].area_id)
            .unwrap();
        assert_eq!(eviction_closure.len(), 1);
        assert_eq!(eviction_closure[0], eviction_unit);
        generation.validate_eviction_units().unwrap();

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
    fn eviction_units_follow_sccs_and_inverse_closure_in_canonical_order() {
        let temp = tempfile::tempdir().unwrap();
        let release = built_release();
        let mut catalog = release.catalog.clone();
        let ids = catalog
            .entries
            .iter()
            .map(|entry| entry.area_id.clone())
            .collect::<Vec<_>>();
        assert_eq!(ids, ["archive", "north-gate", "square"]);
        catalog.entries[0].hard_dependencies = vec![ids[1].clone()];
        catalog.entries[1].hard_dependencies = vec![ids[2].clone()];
        catalog.entries[2].hard_dependencies.clear();

        let world_path = local_area_world_path(
            temp.path(),
            &catalog.world_id,
            &catalog.world_artifact_sha256,
        );
        fs::create_dir_all(world_path.parent().unwrap()).unwrap();
        fs::write(world_path, &release.world_artifact).unwrap();
        let path = write_catalog(temp.path(), &catalog);
        let generation = LocalAreaGeneration::open_exact(temp.path(), path).unwrap();

        for area_id in &ids {
            assert_eq!(
                generation
                    .eviction_unit_for_area(area_id)
                    .unwrap()
                    .iter()
                    .map(|entry| entry.area_id.as_str())
                    .collect::<Vec<_>>(),
                [area_id.as_str()]
            );
        }
        let closure_ids = |area_id: &str| {
            generation
                .eviction_closure_for_area(area_id)
                .unwrap()
                .into_iter()
                .map(|unit| {
                    unit.into_iter()
                        .map(|entry| entry.area_id.as_str())
                        .collect::<Vec<_>>()
                })
                .collect::<Vec<_>>()
        };
        assert_eq!(
            closure_ids(&ids[0]),
            vec![vec![ids[0].as_str()]],
            "evicting a dependent does not require its dependencies to leave"
        );
        assert_eq!(
            closure_ids(&ids[1]),
            vec![vec![ids[0].as_str()], vec![ids[1].as_str()]],
            "the direct dependent must leave with its dependency"
        );
        assert_eq!(
            closure_ids(&ids[2]),
            vec![
                vec![ids[0].as_str()],
                vec![ids[1].as_str()],
                vec![ids[2].as_str()]
            ],
            "the inverse closure includes transitive dependents"
        );
        assert!(generation.eviction_unit_for_area("unknown-area").is_err());
        assert!(
            generation
                .eviction_closure_for_area("unknown-area")
                .is_err()
        );
        generation.validate_eviction_units().unwrap();
    }

    #[test]
    fn eviction_index_rejects_self_unknown_unsorted_and_over_limit_catalogs() {
        let release = built_release();
        let first_id = release.catalog.entries[0].area_id.clone();

        for (hard_dependencies, expected) in [
            (vec![first_id.clone()], "cannot hard-depend on itself"),
            (vec!["missing-area".to_owned()], "is unknown"),
            (
                vec![
                    release.catalog.entries[2].area_id.clone(),
                    release.catalog.entries[1].area_id.clone(),
                ],
                "not strictly sorted",
            ),
        ] {
            let temp = tempfile::tempdir().unwrap();
            let mut invalid = release.catalog.clone();
            invalid.entries[0].hard_dependencies = hard_dependencies;
            let path = write_catalog(temp.path(), &invalid);
            let error = LocalAreaGeneration::open_exact(temp.path(), path).unwrap_err();
            assert!(error.to_string().contains(expected), "{error}");
        }

        let at_limit = tempfile::tempdir().unwrap();
        let mut bounded = release.catalog.clone();
        let template = bounded.entries[0].clone();
        for index in bounded.entries.len()..MAX_LOCAL_EVICTION_UNITS {
            let mut entry = template.clone();
            entry.area_id = format!("limit-area-{index:03}");
            entry.revision = (index + 1) as u64;
            entry.ownership = empty_ownership();
            entry.hard_dependencies.clear();
            entry.portals.clear();
            entry.asset_manifests.clear();
            entry.lua_modules.clear();
            bounded.entries.push(entry);
        }
        bounded
            .entries
            .sort_by(|left, right| left.area_id.cmp(&right.area_id));
        let world_path = local_area_world_path(
            at_limit.path(),
            &bounded.world_id,
            &bounded.world_artifact_sha256,
        );
        fs::create_dir_all(world_path.parent().unwrap()).unwrap();
        fs::write(world_path, &release.world_artifact).unwrap();
        let path = write_catalog(at_limit.path(), &bounded);
        let generation = LocalAreaGeneration::open_exact(at_limit.path(), path).unwrap();
        generation.validate_eviction_units().unwrap();
        assert_eq!(
            generation.eviction_index.units.len(),
            MAX_LOCAL_EVICTION_UNITS
        );

        let over_limit = tempfile::tempdir().unwrap();
        let mut oversized = bounded;
        let mut extra = template;
        extra.area_id = "zz-over-limit-area".into();
        extra.revision = (MAX_LOCAL_EVICTION_UNITS + 1) as u64;
        extra.ownership = empty_ownership();
        extra.hard_dependencies.clear();
        extra.portals.clear();
        extra.asset_manifests.clear();
        extra.lua_modules.clear();
        oversized.entries.push(extra);
        let path = write_catalog(over_limit.path(), &oversized);
        let error = LocalAreaGeneration::open_exact(over_limit.path(), path).unwrap_err();
        assert!(error.to_string().contains("1..=256 unique areas"));
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

    #[cfg(unix)]
    #[test]
    fn shared_runtime_lock_excludes_cooperating_pointer_writers() {
        let temp = tempfile::tempdir().unwrap();
        let lock_path = local_area_publication_lock_path(temp.path(), "lantern-square");
        fs::create_dir_all(lock_path.parent().unwrap()).unwrap();
        fs::File::create(&lock_path).unwrap();

        let read_guard =
            try_acquire_local_area_publication_read_lock(temp.path(), "lantern-square").unwrap();
        let writer_file = OpenOptions::new()
            .read(true)
            .write(true)
            .open(&lock_path)
            .unwrap();
        let mut writer = fd_lock::RwLock::new(writer_file);
        assert_eq!(
            writer.try_write().unwrap_err().kind(),
            std::io::ErrorKind::WouldBlock
        );

        drop(read_guard);
        let writer_guard = writer.try_write().unwrap();
        drop(writer_guard);

        use std::os::unix::fs::symlink;
        let escaped = tempfile::tempdir().unwrap();
        let escaped_lock = local_area_publication_lock_path(escaped.path(), "lantern-square");
        fs::create_dir_all(escaped_lock.parent().unwrap()).unwrap();
        fs::File::create(&escaped_lock).unwrap();
        let registry = temp.path().join("symlink-registry");
        fs::create_dir(&registry).unwrap();
        symlink(
            escaped.path().join("area-locks"),
            registry.join("area-locks"),
        )
        .unwrap();
        assert!(try_acquire_local_area_publication_read_lock(&registry, "lantern-square").is_err());
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
