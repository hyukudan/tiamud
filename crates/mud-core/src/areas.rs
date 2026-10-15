//! Area residency types plus additive install, retirement and
//! rehydration of prepared core areas.

use super::*;

/// Content-addressed identity of the one area catalog fixed for this process.
///
/// The value is the lower-case SHA-256 of the exact canonical catalog bytes,
/// not its editorial revision or filesystem path.
#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct AreaGenerationId(pub(crate) String);

impl AreaGenerationId {
    pub fn from_catalog_sha256(value: impl Into<String>) -> Result<Self, WorldError> {
        let value = value.into();
        if !valid_digest_sha256(&value) {
            return Err(WorldError::InvalidAreaLifecycle);
        }
        Ok(Self(value))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// Runtime-only state used by the server's bounded area coordinator.
///
/// Phase 6B only transitions `Unloaded -> Loading -> Active`; no unload or
/// draining transition is implemented by this type.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum AreaResidenceState {
    Unloaded,
    Loading,
    Active,
}

pub const MAX_RUNTIME_AREA_RESIDENCY_ENTRIES: usize = 256;
pub const AREA_DYNAMIC_CHECKPOINT_VERSION: u32 = 1;

/// Durable selector for one area revision within the catalog generation fixed
/// by the server. Dynamic state exists in exactly one place: a resident area's
/// state is in [`WorldRuntimeSnapshot`], while a retired area's state is in the
/// checkpoint named by `checkpoint_sha256`.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "state", rename_all = "snake_case", deny_unknown_fields)]
pub enum AreaRuntimeResidence {
    Resident {
        revision: u64,
        bundle_sha256: String,
        residency_epoch: u64,
    },
    Retired {
        revision: u64,
        bundle_sha256: String,
        residency_epoch: u64,
        checkpoint_version: u32,
        checkpoint_sha256: String,
    },
}

impl AreaRuntimeResidence {
    pub fn revision(&self) -> u64 {
        match self {
            Self::Resident { revision, .. } | Self::Retired { revision, .. } => *revision,
        }
    }

    pub fn bundle_sha256(&self) -> &str {
        match self {
            Self::Resident { bundle_sha256, .. } | Self::Retired { bundle_sha256, .. } => {
                bundle_sha256
            }
        }
    }

    pub fn residency_epoch(&self) -> u64 {
        match self {
            Self::Resident {
                residency_epoch, ..
            }
            | Self::Retired {
                residency_epoch, ..
            } => *residency_epoch,
        }
    }
}

/// Catalog-owned IDs needed by the core to prove and extract one retirement
/// unit. Reusable definitions deliberately remain generation-global: objects
/// carried out of an area must keep working without pinning their origin.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AreaRuntimeOwnership {
    pub area_id: String,
    pub revision: u64,
    pub bundle_sha256: String,
    pub rooms: BTreeSet<String>,
    pub initial_item_instances: BTreeSet<ItemInstanceId>,
    pub npc_spawns: BTreeSet<String>,
}

/// One reverse-closed SCC (or a sorted set of SCCs) selected by the server.
/// `boundary_exits` contains every resident exit from outside the unit into a
/// room owned by it, allowing retirement to recreate exact `DeferredExit`s.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AreaRetirementUnit {
    pub generation_id: AreaGenerationId,
    pub areas: Vec<AreaRuntimeOwnership>,
    pub boundary_exits: Vec<AreaLoadRequest>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AreaRetirementPin {
    StartRoom,
    ConnectedCharacter,
    Combat,
    Trade,
    ScriptCallback,
    PendingTick,
    TimedNpcState,
    CrossAreaReference,
}

#[derive(Debug, Error, Eq, PartialEq)]
pub enum AreaRetirementError {
    #[error("the area retirement unit is invalid")]
    Invalid,
    #[error("the area retirement unit is pinned by {0:?}")]
    Busy(AreaRetirementPin),
    #[error("the area retirement preparation is stale")]
    Stale,
}

/// Pure retirement result. The server serializes and hashes `checkpoint`,
/// commits it together with the post-retirement v8 snapshot, and only then
/// publishes the candidate `World` that consumed this value.
#[derive(Clone, Debug)]
pub struct PreparedAreaRetirement {
    pub checkpoint: AreaDynamicCheckpoint,
    pub(crate) unit: AreaRetirementUnit,
    pub(crate) retired_room_ids: BTreeSet<String>,
    pub(crate) retired_npc_ids: BTreeSet<String>,
    pub(crate) retired_item_ids: BTreeSet<ItemInstanceId>,
    pub(crate) expected_world_revision: u64,
    pub(crate) expected_logical_tick: u64,
    pub(crate) expected_rng_state: u64,
    pub(crate) expected_next_item_instance_sequence: u64,
    pub(crate) expected_next_loot_drop_id: LootDropId,
    pub(crate) expected_next_reward_open_id: RewardOpenId,
}

/// Internal demand produced when movement reaches a valid non-resident portal.
/// Stable content identifiers in this value are operational data and must not
/// be serialized to a player protocol.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AreaLoadRequest {
    pub generation_id: AreaGenerationId,
    pub source_room_id: String,
    pub direction: Direction,
    pub target_room_id: String,
    pub target_area_id: String,
    pub minimum_target_revision: u64,
}

/// Core-owned part of one fully verified and privately prepared area.
///
/// Progression, player abilities, authored emotes and the RNG seed are fixed
/// globally before listeners open and therefore deliberately absent here.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PreparedCoreArea {
    pub area_id: String,
    pub revision: u64,
    pub bundle_sha256: String,
    pub rooms: Vec<RoomDefinition>,
    /// Additive presentation owned by this area for rooms that may be owned
    /// by another already-resident hard dependency.
    pub room_presentations: Vec<RoomPresentationDefinition>,
    pub item_prototypes: Vec<ItemPrototypeDefinition>,
    pub initial_item_instances: Vec<InitialItemInstanceDefinition>,
    pub npcs: Vec<NpcDefinition>,
    pub npc_ability_sets: Vec<NpcAbilitySetDefinition>,
    pub loot_tables: Vec<LootTableDefinition>,
    pub loot_sources: Vec<LootSourceDefinition>,
    pub affixes: Vec<AffixDefinition>,
    pub affix_pools: Vec<AffixPoolDefinition>,
    pub room_light: Vec<(String, LightLevel)>,
    pub hidden_items: Vec<(String, u16)>,
    pub hidden_npcs: Vec<(String, u16)>,
    pub secret_exits: Vec<(String, Direction, u16)>,
}

/// Presentation-only patch whose ownership is independent from room topology.
///
/// At least one medium must be present. Installation can fill an empty medium
/// or replay the exact same value, but never replace already-published media.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RoomPresentationDefinition {
    pub room_id: String,
    pub image: Option<SceneImage>,
    pub audio: Option<SceneAudio>,
}

/// All-or-nothing core staging result for one bounded hard-dependency closure.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PreparedCoreAreaBatch {
    pub generation_id: AreaGenerationId,
    pub areas: Vec<PreparedCoreArea>,
    /// Cross-area portals whose target may still be absent after this batch.
    pub deferred_exits: Vec<AreaLoadRequest>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CoreAreaInstallOutcome {
    pub installed_areas: usize,
    pub installed_area_ids: Vec<String>,
    pub created_dynamic_state: bool,
}

impl World {
    /// Constructs the first resident closure of a fixed catalog generation.
    ///
    /// The temporary anchor never leaves this function. It exists solely so
    /// the ordinary world invariants can stay non-empty while the same atomic
    /// installation path used at runtime installs the initial batch.
    pub fn from_initial_prepared_areas(
        start_room_id: &str,
        batch: PreparedCoreAreaBatch,
    ) -> Result<(Self, CoreAreaInstallOutcome), WorldError> {
        Self::from_initial_prepared_areas_with_configuration(start_room_id, batch, |_| Ok(()))
    }

    /// Initial-area constructor with a global-content configuration barrier.
    ///
    /// The callback runs after the generation is fixed but before any lazy NPC
    /// ability set is installed. Servers use it to pin RNG, progression,
    /// player abilities and social emotes from the verified monolithic
    /// fallback. A callback or area failure publishes no world.
    pub fn from_initial_prepared_areas_with_configuration(
        start_room_id: &str,
        batch: PreparedCoreAreaBatch,
        configure_globals: impl FnOnce(&mut Self) -> Result<(), WorldError>,
    ) -> Result<(Self, CoreAreaInstallOutcome), WorldError> {
        const BOOTSTRAP_ROOM_ID: &str = "\0tiamud-internal-area-bootstrap";
        if start_room_id.is_empty()
            || batch.areas.is_empty()
            || batch.areas.iter().any(|area| {
                area.rooms
                    .iter()
                    .any(|room| room.stable_id == BOOTSTRAP_ROOM_ID)
            })
        {
            return Err(WorldError::InvalidAreaLifecycle);
        }
        let mut world = Self::from_definition(WorldDefinition {
            start_room_id: BOOTSTRAP_ROOM_ID.into(),
            item_prototypes: Vec::new(),
            initial_item_instances: Vec::new(),
            rooms: vec![RoomDefinition {
                stable_id: BOOTSTRAP_ROOM_ID.into(),
                title: String::new(),
                prose: Vec::new(),
                image: None,
                audio: None,
                exits: Vec::new(),
                item_ids: Vec::new(),
            }],
        })
        .map_err(|_| WorldError::InvalidAreaLifecycle)?;
        world.area_generation = Some(batch.generation_id.clone());
        configure_globals(&mut world)?;
        let outcome = world.install_prepared_areas(batch)?;
        let start_room = world
            .stable_rooms
            .get(start_room_id)
            .copied()
            .ok_or(WorldError::InvalidAreaLifecycle)?;
        room_checked(&world.entities, start_room)?;
        let bootstrap = world
            .stable_rooms
            .remove(BOOTSTRAP_ROOM_ID)
            .ok_or(WorldError::InvalidAreaLifecycle)?;
        world
            .entities
            .remove(bootstrap)
            .ok_or(WorldError::InvalidAreaLifecycle)?;
        world.start_room = start_room;
        world.start_room_stable_id = start_room_id.to_owned();
        Ok((world, outcome))
    }

    pub fn area_generation_id(&self) -> Option<&AreaGenerationId> {
        self.area_generation.as_ref()
    }

    pub fn area_generation(&self) -> Option<&AreaGenerationId> {
        self.area_generation.as_ref()
    }

    pub fn active_area_count(&self) -> usize {
        self.active_areas.len()
    }

    pub fn deferred_area_exit_count(&self) -> usize {
        self.entities
            .values()
            .filter_map(|entity| match entity {
                Entity::Room(room) => Some(room),
                Entity::Character(_) => None,
            })
            .map(|room| {
                room.exits
                    .values()
                    .filter(|exit| matches!(exit, RoomExit::Deferred(_)))
                    .count()
            })
            .sum()
    }

    pub fn area_is_active(&self, area_id: &str) -> bool {
        self.active_areas.contains_key(area_id)
    }

    pub fn active_area_matches(&self, area_id: &str, revision: u64, bundle_sha256: &str) -> bool {
        self.active_areas
            .get(area_id)
            .is_some_and(|area| area.revision == revision && area.bundle_sha256 == bundle_sha256)
    }

    /// Read-only movement preflight used before any authored `OnLeave` hook.
    ///
    /// `None` means the direction is not currently a visible deferred portal;
    /// ordinary movement still owns final validation. The same check is
    /// repeated defensively by [`World::apply`].
    pub fn deferred_area_load_request(
        &self,
        session: SessionId,
        direction: Direction,
    ) -> Result<Option<AreaLoadRequest>, WorldError> {
        let location = self.character(session)?.location;
        let source_room = room_checked(&self.entities, location)?;
        if !self.exit_visible_to(session, &source_room.stable_id, direction)? {
            return Ok(None);
        }
        match source_room.exits.get(&direction) {
            Some(RoomExit::Deferred(request)) => Ok(Some(request.clone())),
            Some(RoomExit::Resident(target)) => {
                room_checked(&self.entities, *target)?;
                Ok(None)
            }
            None => Ok(None),
        }
    }

    /// Atomically incorporates one privately prepared additive area closure.
    ///
    /// This method never changes the fixed generation, emits domain events or
    /// leaves observer revisions bumped. The caller must still compose its
    /// returned checkpoint boundary with runtime-content/residency state and
    /// durably confirm the snapshot when `created_dynamic_state` is true.
    pub fn install_prepared_areas(
        &mut self,
        batch: PreparedCoreAreaBatch,
    ) -> Result<CoreAreaInstallOutcome, WorldError> {
        if self.area_generation.as_ref() != Some(&batch.generation_id)
            || batch.areas.is_empty()
            || batch.areas.len() > MAX_PREPARED_AREAS_PER_BATCH
        {
            return Err(WorldError::InvalidAreaLifecycle);
        }
        let checkpoint = self.lifecycle_checkpoint();
        let observable_revision = self.revision;
        let observable_views = self.observer_views.clone();
        let observable_last_bump = self.last_view_bump.clone();
        let result = (|| {
            let mut seen_areas = BTreeSet::new();
            let mut rehydrating_areas = BTreeSet::new();
            let mut new_areas = Vec::new();
            for area in batch.areas {
                if !valid_definition_id(&area.area_id)
                    || area.revision == 0
                    || !valid_digest_sha256(&area.bundle_sha256)
                    || !seen_areas.insert(area.area_id.clone())
                {
                    return Err(WorldError::InvalidAreaLifecycle);
                }
                match self.active_areas.get(&area.area_id) {
                    Some(active)
                        if active.revision == area.revision
                            && active.bundle_sha256 == area.bundle_sha256 => {}
                    Some(_) => return Err(WorldError::InvalidAreaLifecycle),
                    None => {
                        match self.retired_areas.get(&area.area_id) {
                            Some(retired)
                                if retired.revision == area.revision
                                    && retired.bundle_sha256 == area.bundle_sha256 =>
                            {
                                rehydrating_areas.insert(area.area_id.clone());
                            }
                            Some(_) => return Err(WorldError::InvalidAreaLifecycle),
                            None => {}
                        }
                        new_areas.push(area);
                    }
                }
            }
            if new_areas.is_empty() {
                for request in &batch.deferred_exits {
                    validate_area_load_request(request, &batch.generation_id)?;
                    let source = self
                        .stable_rooms
                        .get(&request.source_room_id)
                        .copied()
                        .ok_or(WorldError::InvalidAreaLifecycle)?;
                    match room_checked(&self.entities, source)?
                        .exits
                        .get(&request.direction)
                    {
                        Some(RoomExit::Deferred(existing)) if existing == request => {}
                        Some(RoomExit::Resident(target))
                            if room_checked(&self.entities, *target)?.stable_id
                                == request.target_room_id
                                && self.room_areas.get(&request.target_room_id)
                                    == Some(&request.target_area_id)
                                && self.active_areas.get(&request.target_area_id).is_some_and(
                                    |area| area.revision >= request.minimum_target_revision,
                                ) => {}
                        _ => return Err(WorldError::InvalidAreaLifecycle),
                    }
                }
                return Ok(CoreAreaInstallOutcome {
                    installed_areas: 0,
                    installed_area_ids: Vec::new(),
                    created_dynamic_state: false,
                });
            }

            let mut portal_requests = BTreeMap::new();
            for request in batch.deferred_exits {
                validate_area_load_request(&request, &batch.generation_id)?;
                let key = (
                    request.source_room_id.clone(),
                    request.direction,
                    request.target_room_id.clone(),
                );
                if portal_requests.insert(key, request).is_some() {
                    return Err(WorldError::InvalidAreaLifecycle);
                }
            }

            let mut created_dynamic_state = new_areas
                .iter()
                .any(|area| !area.initial_item_instances.is_empty() || !area.npcs.is_empty());
            let mut new_room_ids = BTreeSet::new();
            for area in &new_areas {
                for definition in &area.rooms {
                    if !valid_definition_id(&definition.stable_id)
                        || self.stable_rooms.contains_key(&definition.stable_id)
                        || !new_room_ids.insert(definition.stable_id.clone())
                    {
                        return Err(WorldError::InvalidAreaLifecycle);
                    }
                    let room_id = self.entities.insert(Entity::Room(Shared::new(Room {
                        stable_id: definition.stable_id.clone(),
                        title: definition.title.clone(),
                        prose: definition.prose.clone(),
                        image: definition.image.clone(),
                        audio: definition.audio.clone(),
                        exits: BTreeMap::new(),
                    })));
                    self.stable_rooms
                        .insert(definition.stable_id.clone(), room_id);
                    self.room_areas
                        .insert(definition.stable_id.clone(), area.area_id.clone());
                }
                let residency_epoch = match self.retired_areas.remove(&area.area_id) {
                    Some(retired) => retired
                        .residency_epoch
                        .checked_add(1)
                        .ok_or(WorldError::InvalidAreaLifecycle)?,
                    None => 1,
                };
                self.active_areas.insert(
                    area.area_id.clone(),
                    ActiveAreaRevision {
                        revision: area.revision,
                        bundle_sha256: area.bundle_sha256.clone(),
                        residency_epoch,
                    },
                );
            }

            let mut presented_rooms = BTreeSet::new();
            for area in &new_areas {
                for presentation in &area.room_presentations {
                    if !valid_definition_id(&presentation.room_id)
                        || (presentation.image.is_none() && presentation.audio.is_none())
                        || !presented_rooms.insert(presentation.room_id.clone())
                    {
                        return Err(WorldError::InvalidAreaLifecycle);
                    }
                    let room_id = self
                        .stable_rooms
                        .get(&presentation.room_id)
                        .copied()
                        .ok_or(WorldError::InvalidAreaLifecycle)?;
                    let room = room_mut(&mut self.entities, room_id);
                    if let Some(image) = &presentation.image {
                        match &room.image {
                            Some(existing) if existing != image => {
                                return Err(WorldError::InvalidAreaLifecycle);
                            }
                            None => room.image = Some(image.clone()),
                            Some(_) => {}
                        }
                    }
                    if let Some(audio) = &presentation.audio {
                        match &room.audio {
                            Some(existing) if existing != audio => {
                                return Err(WorldError::InvalidAreaLifecycle);
                            }
                            None => room.audio = Some(audio.clone()),
                            Some(_) => {}
                        }
                    }
                }
            }

            for area in &new_areas {
                for definition in &area.rooms {
                    let source = self
                        .stable_rooms
                        .get(&definition.stable_id)
                        .copied()
                        .ok_or(WorldError::InvalidAreaLifecycle)?;
                    let mut seen_directions = BTreeSet::new();
                    for exit in &definition.exits {
                        if !seen_directions.insert(exit.direction) {
                            return Err(WorldError::InvalidAreaLifecycle);
                        }
                        let room_exit = if let Some(target) =
                            self.stable_rooms.get(&exit.target_room_id).copied()
                        {
                            RoomExit::Resident(target)
                        } else {
                            let request = portal_requests
                                .get(&(
                                    definition.stable_id.clone(),
                                    exit.direction,
                                    exit.target_room_id.clone(),
                                ))
                                .cloned()
                                .ok_or(WorldError::InvalidAreaLifecycle)?;
                            RoomExit::Deferred(request)
                        };
                        if room_mut(&mut self.entities, source)
                            .exits
                            .insert(exit.direction, room_exit)
                            .is_some()
                        {
                            return Err(WorldError::InvalidAreaLifecycle);
                        }
                    }
                }
            }

            // Every portal request must annotate an exit already supplied by
            // its owning room definition (possibly in an earlier batch).
            for request in portal_requests.values() {
                let source = self
                    .stable_rooms
                    .get(&request.source_room_id)
                    .copied()
                    .ok_or(WorldError::InvalidAreaLifecycle)?;
                let existing = room_checked(&self.entities, source)?
                    .exits
                    .get(&request.direction)
                    .cloned();
                match existing {
                    Some(RoomExit::Deferred(existing)) if &existing == request => {}
                    Some(RoomExit::Resident(target))
                        if room_checked(&self.entities, target)?.stable_id
                            == request.target_room_id
                            && self.room_areas.get(&request.target_room_id)
                                == Some(&request.target_area_id)
                            && self.active_areas.get(&request.target_area_id).is_some_and(
                                |area| area.revision >= request.minimum_target_revision,
                            ) => {}
                    Some(_) => return Err(WorldError::InvalidAreaLifecycle),
                    None => return Err(WorldError::InvalidAreaLifecycle),
                }
            }
            self.relink_resident_area_exits()?;

            for area in &new_areas {
                for definition in &area.item_prototypes {
                    let (prototype_id, prototype) = compile_item_prototype(definition.clone())?;
                    match self.item_prototypes.get(&prototype_id) {
                        Some(existing)
                            if rehydrating_areas.contains(&area.area_id)
                                && existing == &prototype => {}
                        Some(_) => return Err(WorldError::InvalidAreaLifecycle),
                        None => {
                            self.item_prototypes.insert(prototype_id, prototype);
                        }
                    }
                }
            }
            for area in &new_areas {
                for definition in &area.initial_item_instances {
                    if !valid_item_instance_id(definition.instance_id.as_str()) {
                        return Err(WorldError::InvalidAreaLifecycle);
                    }
                    if let Some(existing) = self.item_instances.get(&definition.instance_id) {
                        if !rehydrating_areas.contains(&area.area_id)
                            || existing.prototype_id != definition.prototype_id
                            || self.initial_item_prototypes.get(&definition.instance_id)
                                != Some(&definition.prototype_id)
                        {
                            return Err(WorldError::InvalidAreaLifecycle);
                        }
                        continue;
                    }
                    let prototype = self
                        .item_prototypes
                        .get(&definition.prototype_id)
                        .ok_or(WorldError::InvalidAreaLifecycle)?;
                    if prototype.sealed_loot_table_id.is_some() {
                        return Err(WorldError::InvalidAreaLifecycle);
                    }
                    let location = match &definition.location {
                        InitialItemLocation::Room(room_id) => {
                            if !self.stable_rooms.contains_key(room_id) {
                                return Err(WorldError::InvalidAreaLifecycle);
                            }
                            SharedItemLocation::Room(room_id.clone())
                        }
                        InitialItemLocation::NpcSpawn(npc_id) => {
                            SharedItemLocation::HeldByNpc(npc_id.clone())
                        }
                        InitialItemLocation::Container(container_id) => {
                            SharedItemLocation::Contained(container_id.clone())
                        }
                        InitialItemLocation::Staged => SharedItemLocation::Staged,
                    };
                    self.item_instances.insert(
                        definition.instance_id.clone(),
                        ItemInstance {
                            prototype_id: definition.prototype_id.clone(),
                            location,
                            binding: ItemBinding::Unbound,
                            item_affixes: Vec::new(),
                        },
                    );
                    match self.initial_item_prototypes.get(&definition.instance_id) {
                        Some(existing) if existing == &definition.prototype_id => {}
                        Some(_) => return Err(WorldError::InvalidAreaLifecycle),
                        None => {
                            self.initial_item_prototypes.insert(
                                definition.instance_id.clone(),
                                definition.prototype_id.clone(),
                            );
                        }
                    }
                    self.item_provenance.insert(
                        definition.instance_id.clone(),
                        ItemProvenance::AuthoredInitial,
                    );
                    if let Some(sequence) = runtime_item_sequence(&definition.instance_id) {
                        self.next_item_instance_sequence = self.next_item_instance_sequence.max(
                            sequence
                                .checked_add(1)
                                .ok_or(WorldError::InvalidAreaLifecycle)?,
                        );
                    }
                }
            }
            if self.item_instances.len() > MAX_ITEM_INSTANCES
                || validate_item_graph(&self.item_instances, &self.item_prototypes).is_err()
            {
                return Err(WorldError::InvalidAreaLifecycle);
            }

            let mut authored_npc_items = BTreeMap::<String, Vec<ItemInstanceId>>::new();
            for area in &new_areas {
                for item in &area.initial_item_instances {
                    if let InitialItemLocation::NpcSpawn(npc_id) = &item.location
                        && (matches!(
                            self.item_location(&item.instance_id),
                            Some(SharedItemLocation::Staged)
                        ) || matches!(
                            self.item_location(&item.instance_id),
                            Some(SharedItemLocation::HeldByNpc(owner)) if owner == npc_id
                        ))
                    {
                        authored_npc_items
                            .entry(npc_id.clone())
                            .or_default()
                            .push(item.instance_id.clone());
                    }
                }
            }
            for item_ids in authored_npc_items.values_mut() {
                item_ids.sort();
                item_ids.dedup();
            }
            let resident_npc_ids = authored_npc_items
                .keys()
                .filter(|npc_id| self.npcs.contains_key(*npc_id))
                .cloned()
                .collect::<Vec<_>>();
            for npc_id in resident_npc_ids {
                let item_ids = authored_npc_items
                    .remove(&npc_id)
                    .ok_or(WorldError::InvalidAreaLifecycle)?;
                let npc = self
                    .npcs
                    .get_mut(&npc_id)
                    .ok_or(WorldError::InvalidAreaLifecycle)?;
                npc.loot_item_ids.extend(item_ids);
                npc.loot_item_ids.sort();
                npc.loot_item_ids.dedup();
                if npc.loot_item_ids.len() > MAX_LOOT_UNITS_PER_RESOLUTION {
                    return Err(WorldError::InvalidAreaLifecycle);
                }
                created_dynamic_state = true;
            }

            let mut affixes = self.affixes.values().cloned().collect::<Vec<_>>();
            let mut affix_pools = self
                .affix_pools
                .values()
                .map(|pool| AffixPoolDefinition {
                    id: pool.id.clone(),
                    entries: pool.entries.clone(),
                })
                .collect::<Vec<_>>();
            for area in &new_areas {
                for definition in &area.affixes {
                    match self.affixes.get(&definition.id) {
                        Some(existing)
                            if rehydrating_areas.contains(&area.area_id)
                                && existing == definition => {}
                        Some(_) => return Err(WorldError::InvalidAreaLifecycle),
                        None => affixes.push(definition.clone()),
                    }
                }
                for pool in &area.affix_pools {
                    match self.affix_pools.get(&pool.id) {
                        Some(existing)
                            if rehydrating_areas.contains(&area.area_id)
                                && existing.digest_sha256 == canonical_sha256(pool)? => {}
                        Some(_) => return Err(WorldError::InvalidAreaLifecycle),
                        None => affix_pools.push(pool.clone()),
                    }
                }
            }
            self.configure_affixes(affixes, affix_pools)?;

            for area in &new_areas {
                for definition in &area.npcs {
                    let mut npc = definition.clone();
                    if let Some(item_ids) = authored_npc_items.remove(&npc.stable_id) {
                        npc.loot_item_ids.extend(item_ids);
                        npc.loot_item_ids.sort();
                        npc.loot_item_ids.dedup();
                    }
                    self.spawn_npc(npc)?;
                }
            }
            if !authored_npc_items.is_empty() {
                return Err(WorldError::InvalidAreaLifecycle);
            }
            let spawned_npc_ids = new_areas
                .iter()
                .flat_map(|area| area.npcs.iter().map(|npc| npc.stable_id.clone()))
                .collect::<Vec<_>>();
            for npc_id in spawned_npc_ids {
                if let Some(definition) = self.pending_npc_ability_sets.remove(&npc_id) {
                    self.configure_npc_abilities(definition)?;
                }
            }
            for area in &new_areas {
                for item in &area.initial_item_instances {
                    if let InitialItemLocation::NpcSpawn(npc_id) = &item.location
                        && self
                            .npcs
                            .get(npc_id)
                            .is_none_or(|npc| !npc.loot_item_ids.contains(&item.instance_id))
                    {
                        return Err(WorldError::InvalidAreaLifecycle);
                    }
                }
            }
            for area in &new_areas {
                for definition in &area.npc_ability_sets {
                    self.validate_npc_ability_set(definition)?;
                    if self.npcs.contains_key(&definition.npc_id) {
                        self.configure_npc_abilities(definition.clone())?;
                        created_dynamic_state = true;
                    } else {
                        match self.pending_npc_ability_sets.get(&definition.npc_id) {
                            Some(existing) if existing == definition => {}
                            Some(_) => return Err(WorldError::InvalidAreaLifecycle),
                            None => {
                                self.pending_npc_ability_sets
                                    .insert(definition.npc_id.clone(), definition.clone());
                            }
                        }
                    }
                }
            }

            let mut loot_tables = self
                .loot_tables
                .values()
                .map(|table| LootTableDefinition {
                    id: table.id.clone(),
                    digest_sha256: table.digest_sha256.clone(),
                    guaranteed: table.guaranteed.clone(),
                    pools: table.pools.clone(),
                })
                .collect::<Vec<_>>();
            let mut loot_sources = self
                .loot_sources
                .iter()
                .map(|(npc_spawn_id, source)| LootSourceDefinition {
                    kind: LootSourceKind::NpcSpawn,
                    // The runtime does not retain the editorial source ID;
                    // the globally unique NPC ID is a valid deterministic
                    // reconstruction key for revalidating the merged catalog.
                    id: npc_spawn_id.clone(),
                    npc_spawn_id: npc_spawn_id.clone(),
                    loot_table_id: source.loot_table_id.clone(),
                    rights: source.rights,
                })
                .collect::<Vec<_>>();
            for area in &new_areas {
                for table in &area.loot_tables {
                    match self.loot_tables.get(&table.id) {
                        Some(existing)
                            if rehydrating_areas.contains(&area.area_id)
                                && existing.id == table.id
                                && existing.digest_sha256 == table.digest_sha256
                                && existing.guaranteed == table.guaranteed
                                && existing.pools == table.pools => {}
                        Some(_) => return Err(WorldError::InvalidAreaLifecycle),
                        None => loot_tables.push(table.clone()),
                    }
                }
                for source in &area.loot_sources {
                    match self.loot_sources.get(&source.npc_spawn_id) {
                        Some(existing)
                            if rehydrating_areas.contains(&area.area_id)
                                && existing.loot_table_id == source.loot_table_id
                                && existing.rights == source.rights => {}
                        Some(_) => return Err(WorldError::InvalidAreaLifecycle),
                        None => loot_sources.push(source.clone()),
                    }
                }
            }
            self.configure_loot(loot_tables, loot_sources)?;

            for area in &new_areas {
                for (room_id, light) in &area.room_light {
                    self.configure_room_light(room_id, *light)?;
                }
                for (item_id, difficulty) in &area.hidden_items {
                    self.configure_hidden_item(item_id, *difficulty)?;
                }
                for (npc_id, difficulty) in &area.hidden_npcs {
                    self.configure_hidden_npc(npc_id, *difficulty)?;
                }
                for (room_id, direction, difficulty) in &area.secret_exits {
                    self.configure_secret_exit(room_id, *direction, *difficulty)?;
                }
            }

            self.revision = observable_revision;
            self.observer_views = observable_views.clone();
            self.last_view_bump = observable_last_bump.clone();
            Ok(CoreAreaInstallOutcome {
                installed_areas: new_areas.len(),
                installed_area_ids: new_areas.iter().map(|area| area.area_id.clone()).collect(),
                created_dynamic_state,
            })
        })();
        match result {
            Ok(outcome) => Ok(outcome),
            Err(error) => {
                self.restore_lifecycle_checkpoint(checkpoint);
                Err(error)
            }
        }
    }

    pub(crate) fn relink_resident_area_exits(&mut self) -> Result<(), WorldError> {
        let stable_rooms = self.stable_rooms.clone();
        let room_areas = self.room_areas.clone();
        let active_areas = self.active_areas.clone();
        for entity in self.entities.values_mut() {
            let Entity::Room(room) = entity else {
                continue;
            };
            for room_exit in room.exits.values_mut() {
                let RoomExit::Deferred(request) = room_exit else {
                    continue;
                };
                let Some(target) = stable_rooms.get(&request.target_room_id).copied() else {
                    continue;
                };
                if room_areas.get(&request.target_room_id) != Some(&request.target_area_id)
                    || active_areas
                        .get(&request.target_area_id)
                        .is_none_or(|area| area.revision < request.minimum_target_revision)
                {
                    return Err(WorldError::InvalidAreaLifecycle);
                }
                *room_exit = RoomExit::Resident(target);
            }
        }
        Ok(())
    }

    /// Builds a complete, mutation-free retirement plan for one server-proved
    /// reverse-closed SCC unit.
    pub fn prepare_area_retirement(
        &self,
        unit: AreaRetirementUnit,
    ) -> Result<PreparedAreaRetirement, AreaRetirementError> {
        if self.tick_resolution_pending {
            return Err(AreaRetirementError::Busy(AreaRetirementPin::PendingTick));
        }
        if self.area_generation.as_ref() != Some(&unit.generation_id)
            || unit.areas.is_empty()
            || unit.areas.len() > MAX_RUNTIME_AREA_RESIDENCY_ENTRIES
            || unit
                .areas
                .windows(2)
                .any(|pair| pair[0].area_id >= pair[1].area_id)
        {
            return Err(AreaRetirementError::Invalid);
        }

        let mut retired_room_ids = BTreeSet::new();
        let mut retired_npc_ids = BTreeSet::new();
        let mut owned_initial_item_ids = BTreeSet::new();
        let mut identities = Vec::with_capacity(unit.areas.len());
        for ownership in &unit.areas {
            let active = self
                .active_areas
                .get(&ownership.area_id)
                .ok_or(AreaRetirementError::Invalid)?;
            if !valid_definition_id(&ownership.area_id)
                || ownership.revision != active.revision
                || ownership.bundle_sha256 != active.bundle_sha256
                || ownership.rooms.iter().any(|room_id| {
                    self.room_areas.get(room_id) != Some(&ownership.area_id)
                        || !self.stable_rooms.contains_key(room_id)
                })
                || ownership.npc_spawns.iter().any(|npc_id| {
                    !self.npcs.contains_key(npc_id) || self.npcs[npc_id].stable_id != *npc_id
                })
                || ownership.initial_item_instances.iter().any(|item_id| {
                    !self.initial_item_prototypes.contains_key(item_id)
                        || !self.item_instances.contains_key(item_id)
                })
            {
                return Err(AreaRetirementError::Invalid);
            }
            if !retired_room_ids.is_disjoint(&ownership.rooms)
                || !retired_npc_ids.is_disjoint(&ownership.npc_spawns)
                || !owned_initial_item_ids.is_disjoint(&ownership.initial_item_instances)
            {
                return Err(AreaRetirementError::Invalid);
            }
            retired_room_ids.extend(ownership.rooms.iter().cloned());
            retired_npc_ids.extend(ownership.npc_spawns.iter().cloned());
            owned_initial_item_ids.extend(ownership.initial_item_instances.iter().cloned());
            identities.push(AreaDynamicCheckpointIdentity {
                area_id: ownership.area_id.clone(),
                revision: active.revision,
                bundle_sha256: active.bundle_sha256.clone(),
                residency_epoch: active.residency_epoch,
            });
        }
        let unit_residency_epoch = identities
            .iter()
            .map(|identity| identity.residency_epoch)
            .max()
            .and_then(|epoch| epoch.checked_add(1))
            .ok_or(AreaRetirementError::Invalid)?;
        for identity in &mut identities {
            identity.residency_epoch = unit_residency_epoch;
        }
        if retired_room_ids.contains(&self.start_room_stable_id) {
            return Err(AreaRetirementError::Busy(AreaRetirementPin::StartRoom));
        }

        let retired_entities = retired_room_ids
            .iter()
            .map(|room_id| {
                self.stable_rooms
                    .get(room_id)
                    .copied()
                    .ok_or(AreaRetirementError::Invalid)
            })
            .collect::<Result<BTreeSet<_>, _>>()?;
        if self.entities.values().any(|entity| {
            matches!(entity, Entity::Character(character) if retired_entities.contains(&character.location))
        }) {
            return Err(AreaRetirementError::Busy(
                AreaRetirementPin::ConnectedCharacter,
            ));
        }

        for npc_id in &retired_npc_ids {
            let npc = self.npcs.get(npc_id).ok_or(AreaRetirementError::Invalid)?;
            if npc.engaged_character.is_some() || self.loot_encounters.contains_key(npc_id) {
                return Err(AreaRetirementError::Busy(AreaRetirementPin::Combat));
            }
            let idle_alive = npc.alive()
                && npc.health == npc.max_health
                && npc
                    .resources
                    .values()
                    .all(|resource| resource.current == resource.max);
            let waiting_to_respawn = npc.health == 0 && npc.respawn_at_tick.is_some();
            if (!idle_alive && !waiting_to_respawn)
                || !npc.statuses.is_empty()
                || !npc.ability_cooldowns.is_empty()
            {
                return Err(AreaRetirementError::Busy(AreaRetirementPin::TimedNpcState));
            }
        }
        if self.character_gameplay.values().any(|gameplay| {
            gameplay.statuses.iter().any(|status| {
                status
                    .source_npc_id
                    .as_ref()
                    .is_some_and(|npc_id| retired_npc_ids.contains(npc_id))
            })
        }) {
            return Err(AreaRetirementError::Busy(
                AreaRetirementPin::CrossAreaReference,
            ));
        }

        let mut retired_item_ids = BTreeSet::new();
        for item_id in self.item_instances.keys() {
            let (root_id, root_location) = self
                .item_root_location(item_id)
                .map_err(|_| AreaRetirementError::Invalid)?;
            let belongs = match root_location {
                SharedItemLocation::Room(room_id) => retired_room_ids.contains(room_id),
                SharedItemLocation::HeldByNpc(npc_id) => retired_npc_ids.contains(npc_id),
                SharedItemLocation::Staged => owned_initial_item_ids.contains(root_id),
                SharedItemLocation::Carried(_)
                | SharedItemLocation::Equipped(_, _)
                | SharedItemLocation::Consumed => false,
                SharedItemLocation::Contained(_) => return Err(AreaRetirementError::Invalid),
            };
            if belongs {
                retired_item_ids.insert(item_id.clone());
            }
        }
        if self.npcs.iter().any(|(npc_id, npc)| {
            !retired_npc_ids.contains(npc_id)
                && npc
                    .loot_item_ids
                    .iter()
                    .any(|item_id| retired_item_ids.contains(item_id))
        }) {
            return Err(AreaRetirementError::Busy(
                AreaRetirementPin::CrossAreaReference,
            ));
        }
        if self.trades.values().any(|trade| {
            retired_room_ids.contains(&trade.room_id)
                || trade.offers.values().any(|offer| {
                    offer.lines.iter().any(|line| {
                        line.item_ids
                            .iter()
                            .any(|item_id| retired_item_ids.contains(item_id))
                    })
                })
        }) {
            return Err(AreaRetirementError::Busy(AreaRetirementPin::Trade));
        }

        let mut boundary_keys = BTreeSet::new();
        for request in &unit.boundary_exits {
            if request.generation_id != unit.generation_id
                || retired_room_ids.contains(&request.source_room_id)
                || !retired_room_ids.contains(&request.target_room_id)
                || self.room_areas.get(&request.target_room_id) != Some(&request.target_area_id)
                || self
                    .active_areas
                    .get(&request.target_area_id)
                    .is_none_or(|area| area.revision < request.minimum_target_revision)
                || !boundary_keys.insert((
                    request.source_room_id.clone(),
                    request.direction,
                    request.target_room_id.clone(),
                ))
            {
                return Err(AreaRetirementError::Invalid);
            }
            let source = self
                .stable_rooms
                .get(&request.source_room_id)
                .copied()
                .ok_or(AreaRetirementError::Invalid)?;
            let target = self
                .stable_rooms
                .get(&request.target_room_id)
                .copied()
                .ok_or(AreaRetirementError::Invalid)?;
            if !matches!(
                room_checked(&self.entities, source)
                    .map_err(|_| AreaRetirementError::Invalid)?
                    .exits
                    .get(&request.direction),
                Some(RoomExit::Resident(actual)) if *actual == target
            ) {
                return Err(AreaRetirementError::Invalid);
            }
        }
        for entity in self.entities.values() {
            let Entity::Room(room) = entity else {
                continue;
            };
            if retired_room_ids.contains(&room.stable_id) {
                continue;
            }
            for (direction, exit) in &room.exits {
                let Some(target) = exit.resident_target() else {
                    continue;
                };
                let target_room = room_checked(&self.entities, target)
                    .map_err(|_| AreaRetirementError::Invalid)?;
                if retired_room_ids.contains(&target_room.stable_id)
                    && !boundary_keys.contains(&(
                        room.stable_id.clone(),
                        *direction,
                        target_room.stable_id.clone(),
                    ))
                {
                    return Err(AreaRetirementError::Busy(
                        AreaRetirementPin::CrossAreaReference,
                    ));
                }
            }
        }

        let npcs = retired_npc_ids
            .iter()
            .map(|npc_id| {
                let npc = self.npcs.get(npc_id).expect("validated retired NPC");
                (npc_id.clone(), npc_runtime_snapshot(npc))
            })
            .collect();
        let item_instances = retired_item_ids
            .iter()
            .map(|item_id| {
                let instance = self
                    .item_instances
                    .get(item_id)
                    .expect("validated retired item");
                (item_id.clone(), runtime_item_instance_snapshot(instance))
            })
            .collect();
        let item_provenance = retired_item_ids
            .iter()
            .map(|item_id| {
                self.item_provenance
                    .get(item_id)
                    .cloned()
                    .map(|provenance| (item_id.clone(), provenance))
                    .ok_or(AreaRetirementError::Invalid)
            })
            .collect::<Result<_, _>>()?;
        let ground_loot = self
            .ground_loot
            .iter()
            .filter(|(root_id, _)| retired_item_ids.contains(*root_id))
            .map(|(root_id, reservation)| (root_id.clone(), reservation.clone()))
            .collect();
        if self.ground_loot.iter().any(|(root_id, reservation)| {
            retired_room_ids.contains(&reservation.room_id) != retired_item_ids.contains(root_id)
        }) {
            return Err(AreaRetirementError::Invalid);
        }

        let checkpoint = AreaDynamicCheckpoint {
            checkpoint_version: AREA_DYNAMIC_CHECKPOINT_VERSION,
            generation_sha256: unit.generation_id.as_str().to_owned(),
            areas: identities,
            captured_world_revision: self.revision,
            captured_logical_tick: self.logical_tick,
            captured_rng_state: self.rng.state,
            captured_next_item_instance_sequence: self.next_item_instance_sequence,
            captured_next_loot_drop_id: self.next_loot_drop_id,
            captured_next_reward_open_id: self.next_reward_open_id,
            npcs,
            item_instances,
            item_provenance,
            ground_loot,
        };
        validate_area_dynamic_checkpoint(&checkpoint).map_err(|_| AreaRetirementError::Invalid)?;
        Ok(PreparedAreaRetirement {
            checkpoint,
            unit,
            retired_room_ids,
            retired_npc_ids,
            retired_item_ids,
            expected_world_revision: self.revision,
            expected_logical_tick: self.logical_tick,
            expected_rng_state: self.rng.state,
            expected_next_item_instance_sequence: self.next_item_instance_sequence,
            expected_next_loot_drop_id: self.next_loot_drop_id,
            expected_next_reward_open_id: self.next_reward_open_id,
        })
    }

    pub(crate) fn item_root_location<'a>(
        &'a self,
        item_id: &'a ItemInstanceId,
    ) -> Result<(&'a ItemInstanceId, &'a SharedItemLocation), WorldError> {
        let mut current = item_id;
        for _ in 0..=MAX_CONTAINER_DEPTH {
            let instance = self
                .item_instances
                .get(current)
                .ok_or(WorldError::InvalidEntity)?;
            match &instance.location {
                SharedItemLocation::Contained(parent) => current = parent,
                location => return Ok((current, location)),
            }
        }
        Err(WorldError::InvalidEntity)
    }

    /// Applies a prepared retirement to a private actor candidate. It performs
    /// no I/O and changes no global counter, tick, revision or RNG state.
    pub fn apply_prepared_area_retirement(
        &mut self,
        prepared: PreparedAreaRetirement,
        checkpoint_sha256: String,
    ) -> Result<(), AreaRetirementError> {
        if !valid_digest_sha256(&checkpoint_sha256)
            || canonical_sha256(&prepared.checkpoint).map_err(|_| AreaRetirementError::Invalid)?
                != checkpoint_sha256
            || self.area_generation.as_ref() != Some(&prepared.unit.generation_id)
            || self.revision != prepared.expected_world_revision
            || self.logical_tick != prepared.expected_logical_tick
            || self.rng.state != prepared.expected_rng_state
            || self.next_item_instance_sequence != prepared.expected_next_item_instance_sequence
            || self.next_loot_drop_id != prepared.expected_next_loot_drop_id
            || self.next_reward_open_id != prepared.expected_next_reward_open_id
        {
            return Err(AreaRetirementError::Stale);
        }
        let current = self.prepare_area_retirement(prepared.unit.clone())?;
        if current.checkpoint != prepared.checkpoint
            || current.retired_room_ids != prepared.retired_room_ids
            || current.retired_npc_ids != prepared.retired_npc_ids
            || current.retired_item_ids != prepared.retired_item_ids
        {
            return Err(AreaRetirementError::Stale);
        }

        let rollback = self.lifecycle_checkpoint();
        let applied = (|| {
            for request in &prepared.unit.boundary_exits {
                let source = self
                    .stable_rooms
                    .get(&request.source_room_id)
                    .copied()
                    .ok_or(AreaRetirementError::Invalid)?;
                let target = self
                    .stable_rooms
                    .get(&request.target_room_id)
                    .copied()
                    .ok_or(AreaRetirementError::Invalid)?;
                let exit = room_mut(&mut self.entities, source)
                    .exits
                    .get_mut(&request.direction)
                    .ok_or(AreaRetirementError::Invalid)?;
                if !matches!(exit, RoomExit::Resident(actual) if *actual == target) {
                    return Err(AreaRetirementError::Invalid);
                }
                *exit = RoomExit::Deferred(request.clone());
            }

            for room_id in &prepared.retired_room_ids {
                let entity_id = self
                    .stable_rooms
                    .remove(room_id)
                    .ok_or(AreaRetirementError::Invalid)?;
                if !matches!(self.entities.remove(entity_id), Some(Entity::Room(_))) {
                    return Err(AreaRetirementError::Invalid);
                }
                self.room_areas.remove(room_id);
                self.room_light.remove(room_id);
                self.secret_exits
                    .retain(|(secret_room_id, _), _| secret_room_id != room_id);
            }
            // Per-session map memory is durable runtime state, not a durable
            // room. A retired room leaves `stable_rooms`, so any observer that
            // discovered it would otherwise resolve a dangling id when the map
            // is projected. Reconcile it at this teardown boundary exactly like
            // the other resident indexes above; a rehydrated room is
            // rediscovered on re-entry. `observer_views` is part of the
            // lifecycle checkpoint, so a later failure rolls this back too.
            for observer in self.observer_views.values_mut() {
                observer
                    .discovered_rooms
                    .retain(|room_id| !prepared.retired_room_ids.contains(room_id));
            }
            for npc_id in &prepared.retired_npc_ids {
                self.npcs
                    .remove(npc_id)
                    .ok_or(AreaRetirementError::Invalid)?;
                self.hidden_npcs.remove(npc_id);
                self.loot_sources.remove(npc_id);
            }
            for item_id in &prepared.retired_item_ids {
                self.item_instances
                    .remove(item_id)
                    .ok_or(AreaRetirementError::Invalid)?;
                self.item_provenance
                    .remove(item_id)
                    .ok_or(AreaRetirementError::Invalid)?;
                self.hidden_items.remove(item_id.as_str());
                self.ground_loot.remove(item_id);
            }
            for identity in &prepared.checkpoint.areas {
                let active = self
                    .active_areas
                    .remove(&identity.area_id)
                    .ok_or(AreaRetirementError::Invalid)?;
                if active.revision != identity.revision
                    || active.bundle_sha256 != identity.bundle_sha256
                    || identity.residency_epoch <= active.residency_epoch
                {
                    return Err(AreaRetirementError::Invalid);
                }
                self.retired_areas.insert(
                    identity.area_id.clone(),
                    RetiredAreaRevision {
                        revision: identity.revision,
                        bundle_sha256: identity.bundle_sha256.clone(),
                        residency_epoch: identity.residency_epoch,
                        checkpoint_sha256: checkpoint_sha256.clone(),
                    },
                );
            }
            if validate_item_graph(&self.item_instances, &self.item_prototypes).is_err()
                || self.npcs.values().any(|npc| {
                    npc.loot_item_ids
                        .iter()
                        .any(|item_id| !self.item_instances.contains_key(item_id))
                })
            {
                return Err(AreaRetirementError::Invalid);
            }
            Ok(())
        })();
        if let Err(error) = applied {
            self.restore_lifecycle_checkpoint(rollback);
            return Err(error);
        }
        debug_assert_eq!(self.revision, prepared.expected_world_revision);
        debug_assert_eq!(self.logical_tick, prepared.expected_logical_tick);
        debug_assert_eq!(self.rng.state, prepared.expected_rng_state);
        Ok(())
    }

    /// Reinstalls one retired unit from the same immutable generation and
    /// overlays its checkpoint without rewinding any global clock, RNG or
    /// allocator. The caller applies this to an actor-private candidate and
    /// commits the resulting v8 snapshot plus residency CAS before publishing.
    pub fn rehydrate_retired_areas(
        &mut self,
        batch: PreparedCoreAreaBatch,
        mut checkpoint: AreaDynamicCheckpoint,
        checkpoint_sha256: &str,
    ) -> Result<CoreAreaInstallOutcome, AreaRetirementError> {
        validate_area_dynamic_checkpoint(&checkpoint).map_err(|_| AreaRetirementError::Invalid)?;
        if self.tick_resolution_pending {
            return Err(AreaRetirementError::Busy(AreaRetirementPin::PendingTick));
        }
        if !valid_digest_sha256(checkpoint_sha256)
            || canonical_sha256(&checkpoint).map_err(|_| AreaRetirementError::Invalid)?
                != checkpoint_sha256
            || self.area_generation.as_ref() != Some(&batch.generation_id)
            || checkpoint.generation_sha256 != batch.generation_id.as_str()
            || checkpoint.captured_logical_tick > self.logical_tick
            || checkpoint.captured_world_revision > self.revision
            || checkpoint.captured_next_item_instance_sequence > self.next_item_instance_sequence
            || checkpoint.captured_next_loot_drop_id > self.next_loot_drop_id
            || checkpoint.captured_next_reward_open_id > self.next_reward_open_id
        {
            return Err(AreaRetirementError::Invalid);
        }

        let batch_identities = batch
            .areas
            .iter()
            .map(|area| {
                (
                    area.area_id.clone(),
                    (area.revision, area.bundle_sha256.as_str()),
                )
            })
            .collect::<BTreeMap<_, _>>();
        if batch_identities.len() != checkpoint.areas.len()
            || checkpoint.areas.iter().any(|identity| {
                batch_identities.get(&identity.area_id)
                    != Some(&(identity.revision, identity.bundle_sha256.as_str()))
                    || self
                        .retired_areas
                        .get(&identity.area_id)
                        .is_none_or(|retired| {
                            retired.revision != identity.revision
                                || retired.bundle_sha256 != identity.bundle_sha256
                                || retired.residency_epoch != identity.residency_epoch
                                || retired.checkpoint_sha256 != checkpoint_sha256
                        })
            })
        {
            return Err(AreaRetirementError::Invalid);
        }

        // Apply only deterministic time catch-up. Expired ground roots take
        // their complete contained subtree with them; exclusivity simply
        // becomes public. No random roll is replayed.
        let expired_roots = checkpoint
            .ground_loot
            .iter()
            .filter(|(_, reservation)| reservation.despawn_at_tick <= self.logical_tick)
            .map(|(root_id, _)| root_id.clone())
            .collect::<BTreeSet<_>>();
        let mut discarded_item_ids = expired_roots.clone();
        loop {
            let before = discarded_item_ids.len();
            for (item_id, item) in &checkpoint.item_instances {
                if matches!(
                    &item.location,
                    RuntimeItemLocation::Contained { container_id }
                        if discarded_item_ids.contains(container_id)
                ) {
                    discarded_item_ids.insert(item_id.clone());
                }
            }
            if discarded_item_ids.len() == before {
                break;
            }
        }
        for item_id in &discarded_item_ids {
            checkpoint.item_instances.remove(item_id);
            checkpoint.item_provenance.remove(item_id);
            checkpoint.ground_loot.remove(item_id);
        }
        for reservation in checkpoint.ground_loot.values_mut() {
            if reservation.claim_until_tick <= self.logical_tick {
                reservation.beneficiary = None;
            }
        }
        for npc in checkpoint.npcs.values_mut() {
            npc.loot_item_ids
                .retain(|item_id| checkpoint.item_instances.contains_key(item_id));
        }

        let rollback = self.lifecycle_checkpoint();
        let expected_revision = self.revision;
        let expected_tick = self.logical_tick;
        let expected_rng_state = self.rng.state;
        let expected_item_sequence = self.next_item_instance_sequence;
        let expected_drop_id = self.next_loot_drop_id;
        let expected_reward_open_id = self.next_reward_open_id;
        let restored = (|| {
            let mut outcome = self
                .install_prepared_areas(batch)
                .map_err(|_| AreaRetirementError::Invalid)?;

            let checkpoint_item_ids = checkpoint
                .item_instances
                .keys()
                .cloned()
                .chain(discarded_item_ids.iter().cloned())
                .collect::<BTreeSet<_>>();
            for item_id in &checkpoint_item_ids {
                if let Some(existing) = self.item_instances.remove(item_id)
                    && self.initial_item_prototypes.get(item_id) != Some(&existing.prototype_id)
                {
                    return Err(AreaRetirementError::Invalid);
                }
                self.item_provenance.remove(item_id);
                self.hidden_items.remove(item_id.as_str());
                self.ground_loot.remove(item_id);
            }
            for (item_id, saved) in &checkpoint.item_instances {
                let location = match &saved.location {
                    RuntimeItemLocation::Staged => SharedItemLocation::Staged,
                    RuntimeItemLocation::Room { room_id } => {
                        if !self.stable_rooms.contains_key(room_id) {
                            return Err(AreaRetirementError::Invalid);
                        }
                        SharedItemLocation::Room(room_id.clone())
                    }
                    RuntimeItemLocation::HeldByNpc { npc_id } => {
                        if !checkpoint.npcs.contains_key(npc_id) {
                            return Err(AreaRetirementError::Invalid);
                        }
                        SharedItemLocation::HeldByNpc(npc_id.clone())
                    }
                    RuntimeItemLocation::Contained { container_id } => {
                        if !checkpoint.item_instances.contains_key(container_id)
                            && !self.item_instances.contains_key(container_id)
                        {
                            return Err(AreaRetirementError::Invalid);
                        }
                        SharedItemLocation::Contained(container_id.clone())
                    }
                    RuntimeItemLocation::Carried { .. }
                    | RuntimeItemLocation::Equipped { .. }
                    | RuntimeItemLocation::Consumed => {
                        // Those roots are global and therefore cannot belong
                        // to an area checkpoint.
                        return Err(AreaRetirementError::Invalid);
                    }
                };
                if !self.item_prototypes.contains_key(&saved.prototype_id)
                    || self
                        .item_instances
                        .insert(
                            item_id.clone(),
                            ItemInstance {
                                prototype_id: saved.prototype_id.clone(),
                                location,
                                binding: saved.binding,
                                item_affixes: saved.item_affixes.clone(),
                            },
                        )
                        .is_some()
                {
                    return Err(AreaRetirementError::Invalid);
                }
                let provenance = checkpoint
                    .item_provenance
                    .get(item_id)
                    .cloned()
                    .ok_or(AreaRetirementError::Invalid)?;
                self.item_provenance.insert(item_id.clone(), provenance);
            }

            for (npc_id, saved) in &checkpoint.npcs {
                let authored = self
                    .npcs
                    .get(npc_id)
                    .cloned()
                    .ok_or(AreaRetirementError::Invalid)?;
                let static_matches = authored.stable_id == saved.stable_id
                    && authored.name == saved.name
                    && authored.description == saved.description
                    && authored.spawn_room_id == saved.spawn_room_id
                    && authored.level == saved.level
                    && authored.max_health == saved.max_health
                    && authored.strength == saved.strength
                    && authored.defence == saved.defence
                    && authored.experience_reward == saved.experience_reward
                    && authored.respawn_ticks == saved.respawn_ticks
                    && authored.resource_regeneration == saved.resource_regeneration
                    && authored.ability_ids == saved.ability_ids
                    && authored.resistances == saved.resistances
                    && authored.resources.keys().eq(saved.resources.keys())
                    && authored.resources.iter().all(|(resource_id, configured)| {
                        saved
                            .resources
                            .get(resource_id)
                            .is_some_and(|runtime| runtime.max == configured.max)
                    });
                if !static_matches {
                    return Err(AreaRetirementError::Invalid);
                }
                let due_to_respawn = saved.health == 0
                    && saved
                        .respawn_at_tick
                        .is_some_and(|tick| tick <= self.logical_tick);
                let npc = self
                    .npcs
                    .get_mut(npc_id)
                    .ok_or(AreaRetirementError::Invalid)?;
                npc.health = if due_to_respawn {
                    npc.max_health
                } else {
                    saved.health
                };
                npc.respawn_at_tick = if due_to_respawn {
                    None
                } else {
                    saved.respawn_at_tick
                };
                npc.engaged_character = None;
                npc.loot_item_ids = saved.loot_item_ids.clone();
                npc.resources = authored
                    .resources
                    .iter()
                    .map(|(resource_id, configured)| {
                        let current = if due_to_respawn {
                            configured.max
                        } else {
                            saved.resources[resource_id].current
                        };
                        (
                            resource_id.clone(),
                            ResourcePool {
                                current,
                                max: configured.max,
                            },
                        )
                    })
                    .collect();
                npc.ability_cooldowns.clear();
                npc.statuses.clear();
            }
            for (root_id, reservation) in &checkpoint.ground_loot {
                if self
                    .ground_loot
                    .insert(root_id.clone(), reservation.clone())
                    .is_some()
                {
                    return Err(AreaRetirementError::Invalid);
                }
            }
            if validate_item_graph(&self.item_instances, &self.item_prototypes).is_err()
                || validate_restored_item_affixes(&self.item_instances, &self.item_prototypes)
                    .is_err()
                || validate_restored_sealed_items(
                    &self.item_instances,
                    &self.item_provenance,
                    &self.ground_loot,
                    &self.item_prototypes,
                )
                .is_err()
                || self.npcs.values().any(|npc| {
                    npc.loot_item_ids.iter().any(|item_id| {
                        self.item_instances.get(item_id).is_none_or(|instance| {
                            instance.location
                                != SharedItemLocation::HeldByNpc(npc.stable_id.clone())
                        })
                    })
                })
                || self.revision != expected_revision
                || self.logical_tick != expected_tick
                || self.rng.state != expected_rng_state
                || self.next_item_instance_sequence != expected_item_sequence
                || self.next_loot_drop_id != expected_drop_id
                || self.next_reward_open_id != expected_reward_open_id
            {
                return Err(AreaRetirementError::Invalid);
            }
            outcome.created_dynamic_state = true;
            Ok(outcome)
        })();
        match restored {
            Ok(outcome) => Ok(outcome),
            Err(error) => {
                self.restore_lifecycle_checkpoint(rollback);
                Err(error)
            }
        }
    }
}
