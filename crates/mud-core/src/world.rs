//! The single-writer `World`, its checkpoints, construction, static
//! configuration and the `apply` dispatch.

use super::*;

/// A single-owner world graph. Callers are expected to place it behind an actor.
pub struct World {
    pub(crate) entities: Shared<SlotMap<EntityId, Entity>>,
    pub(crate) stable_rooms: HashMap<String, EntityId>,
    pub(crate) area_generation: Option<AreaGenerationId>,
    pub(crate) active_areas: BTreeMap<String, ActiveAreaRevision>,
    pub(crate) retired_areas: BTreeMap<String, RetiredAreaRevision>,
    pub(crate) room_areas: HashMap<String, String>,
    pub(crate) item_prototypes: HashMap<String, ItemPrototype>,
    /// Immutable authored identity mapping used to merge an older runtime
    /// snapshot with newly published initial instances without allowing a
    /// snapshot to retarget an existing authored ID to another prototype.
    pub(crate) initial_item_prototypes: BTreeMap<ItemInstanceId, String>,
    pub(crate) item_instances: ItemInstances,
    pub(crate) next_item_instance_sequence: u64,
    pub(crate) item_provenance: Shared<BTreeMap<ItemInstanceId, ItemProvenance>>,
    pub(crate) affixes: BTreeMap<String, AffixDefinition>,
    pub(crate) affix_pools: BTreeMap<String, AffixPool>,
    pub(crate) loot_tables: BTreeMap<String, LootTable>,
    pub(crate) loot_sources: BTreeMap<String, LootSource>,
    pub(crate) next_loot_drop_id: LootDropId,
    pub(crate) next_reward_open_id: RewardOpenId,
    pub(crate) loot_encounters: Shared<BTreeMap<String, LootEncounter>>,
    pub(crate) ground_loot: Shared<BTreeMap<ItemInstanceId, GroundLootReservation>>,
    pub(crate) party_loot_cursors: Shared<BTreeMap<u64, u64>>,
    pub(crate) room_light: HashMap<String, LightLevel>,
    pub(crate) hidden_items: HashMap<String, u16>,
    pub(crate) hidden_npcs: HashMap<String, u16>,
    pub(crate) secret_exits: HashMap<(String, Direction), u16>,
    pub(crate) pending_npc_ability_sets: BTreeMap<String, NpcAbilitySetDefinition>,
    pub(crate) npcs: Shared<BTreeMap<String, Npc>>,
    pub(crate) abilities: BTreeMap<String, AbilityDefinition>,
    pub(crate) social_emotes: BTreeMap<String, SocialEmoteDefinition>,
    /// Immutable WorldPack v10 quest, commodity and shop catalog.
    pub(crate) adventure_content: std::sync::Arc<AdventureContent>,
    pub(crate) progression: ProgressionCatalog,
    pub(crate) progression_catalog: CharacterCreationCatalogView,
    pub(crate) world_progression_flags: Shared<BTreeSet<String>>,
    pub(crate) character_gameplay: Shared<HashMap<CharacterId, CharacterGameplaySnapshot>>,
    pub(crate) sessions: HashMap<SessionId, EntityId>,
    pub(crate) observer_views: HashMap<SessionId, ObserverView>,
    pub(crate) last_view_bump: Option<(u64, Vec<(SessionId, u64)>)>,
    pub(crate) start_room: EntityId,
    pub(crate) start_room_stable_id: String,
    pub(crate) revision: u64,
    pub(crate) logical_tick: u64,
    pub(crate) rng: DeterministicRng,
    pub(crate) adventure: Shared<AdventureRuntimeSnapshot>,
    pub(crate) next_trade_id: TradeId,
    pub(crate) trades: Shared<BTreeMap<TradeId, TradeSnapshot>>,
    /// Ephemeral abuse control. This deliberately does not enter the portable
    /// runtime snapshot and may reset on process restart.
    pub(crate) last_trade_request_ticks: HashMap<CharacterId, u64>,
    pub(crate) tick_resolution_pending: bool,
    /// What the backend last confirmed. Not part of any checkpoint: rolling
    /// world state back never changes what the store holds.
    pub(crate) durable: DurableBaseline,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct ActiveAreaRevision {
    pub(crate) revision: u64,
    pub(crate) bundle_sha256: String,
    pub(crate) residency_epoch: u64,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct RetiredAreaRevision {
    pub(crate) revision: u64,
    pub(crate) bundle_sha256: String,
    pub(crate) residency_epoch: u64,
    pub(crate) checkpoint_sha256: String,
}

/// Exact in-memory checkpoint used around one durable commit. Unlike the
/// portable snapshots this also contains observer revision bookkeeping, so a
/// failed database transaction is invisible to connected clients.
#[derive(Clone)]
pub struct WorldCheckpoint {
    pub(crate) entities: Shared<SlotMap<EntityId, Entity>>,
    pub(crate) item_instances: ItemInstances,
    pub(crate) next_item_instance_sequence: u64,
    pub(crate) item_provenance: Shared<BTreeMap<ItemInstanceId, ItemProvenance>>,
    pub(crate) next_reward_open_id: RewardOpenId,
    pub(crate) next_loot_drop_id: LootDropId,
    pub(crate) loot_encounters: Shared<BTreeMap<String, LootEncounter>>,
    pub(crate) ground_loot: Shared<BTreeMap<ItemInstanceId, GroundLootReservation>>,
    pub(crate) party_loot_cursors: Shared<BTreeMap<u64, u64>>,
    pub(crate) npcs: Shared<BTreeMap<String, Npc>>,
    pub(crate) character_gameplay: Shared<HashMap<CharacterId, CharacterGameplaySnapshot>>,
    pub(crate) observer_views: HashMap<SessionId, ObserverView>,
    pub(crate) last_view_bump: Option<(u64, Vec<(SessionId, u64)>)>,
    pub(crate) revision: u64,
    pub(crate) logical_tick: u64,
    pub(crate) rng: DeterministicRng,
    pub(crate) adventure: Shared<AdventureRuntimeSnapshot>,
    pub(crate) world_progression_flags: Shared<BTreeSet<String>>,
    pub(crate) next_trade_id: TradeId,
    pub(crate) trades: Shared<BTreeMap<TradeId, TradeSnapshot>>,
    pub(crate) last_trade_request_ticks: HashMap<CharacterId, u64>,
    pub(crate) tick_resolution_pending: bool,
}

/// Exact checkpoint for an actor-owned topology/content installation.
///
/// The ordinary [`WorldCheckpoint`] remains the cheaper durable-action
/// boundary. This wider checkpoint additionally owns every static core index
/// that an additive area batch may change. The server composes it with its
/// `RuntimeContent` and residency checkpoints before publishing a batch.
#[derive(Clone)]
pub struct WorldLifecycleCheckpoint {
    pub(crate) world: WorldCheckpoint,
    pub(crate) stable_rooms: HashMap<String, EntityId>,
    pub(crate) area_generation: Option<AreaGenerationId>,
    pub(crate) active_areas: BTreeMap<String, ActiveAreaRevision>,
    pub(crate) retired_areas: BTreeMap<String, RetiredAreaRevision>,
    pub(crate) room_areas: HashMap<String, String>,
    pub(crate) item_prototypes: HashMap<String, ItemPrototype>,
    pub(crate) initial_item_prototypes: BTreeMap<ItemInstanceId, String>,
    pub(crate) affixes: BTreeMap<String, AffixDefinition>,
    pub(crate) affix_pools: BTreeMap<String, AffixPool>,
    pub(crate) loot_tables: BTreeMap<String, LootTable>,
    pub(crate) loot_sources: BTreeMap<String, LootSource>,
    pub(crate) room_light: HashMap<String, LightLevel>,
    pub(crate) hidden_items: HashMap<String, u16>,
    pub(crate) hidden_npcs: HashMap<String, u16>,
    pub(crate) secret_exits: HashMap<(String, Direction), u16>,
    pub(crate) pending_npc_ability_sets: BTreeMap<String, NpcAbilitySetDefinition>,
}

impl World {
    pub fn from_definition(definition: WorldDefinition) -> Result<Self, WorldBuildError> {
        Self::from_definition_with_staged_items(definition, Vec::new())
    }

    /// Builds a world while reserving explicitly named items for initial
    /// ownership assignment (for example, NPC loot). A staged item is not
    /// visible in any room and can only be claimed by an NPC spawn.
    pub fn from_definition_with_staged_items(
        definition: WorldDefinition,
        staged_item_ids: Vec<String>,
    ) -> Result<Self, WorldBuildError> {
        if definition.rooms.is_empty() {
            return Err(WorldBuildError::EmptyWorld);
        }

        let mut entities = SlotMap::with_key();
        let mut stable_rooms = HashMap::with_capacity(definition.rooms.len());
        let mut item_prototypes = HashMap::with_capacity(definition.item_prototypes.len());
        for item in definition.item_prototypes {
            if item.prototype_id.trim().is_empty() {
                return Err(WorldBuildError::EmptyItemId);
            }
            if item.name.trim().is_empty() {
                return Err(WorldBuildError::EmptyItemName(item.prototype_id));
            }
            let stack_valid = match item.stack_policy {
                ItemStackPolicy::Unique => item.max_stack_units == 1,
                ItemStackPolicy::Fungible => (2..=1_000).contains(&item.max_stack_units),
            };
            let container_valid = item.container.as_ref().is_none_or(|container| {
                item.stack_policy == ItemStackPolicy::Unique
                    && item.max_stack_units == 1
                    && !item.consumable
                    && (1..=1_000).contains(&container.max_slots)
                    && container.max_weight_grams <= 1_000_000
            });
            let presentation_valid = valid_definition_id(&item.prototype_id)
                && safe_definition_text(&item.name, 128)
                && safe_definition_multiline_text(&item.description, 2_048)
                && item.aliases.len() <= 32
                && item
                    .aliases
                    .iter()
                    .all(|alias| safe_definition_text(alias, 64));
            let sealed_valid = item.sealed_loot_table_id.as_ref().is_none_or(|table_id| {
                valid_definition_id(table_id)
                    && item.stack_policy == ItemStackPolicy::Unique
                    && item.max_stack_units == 1
                    && item.consumable
                    && item.container.is_none()
                    && item.equipment_slot.is_none()
                    && item.attack_bonus == 0
                    && item.heal_amount == 0
                    && item.effect.is_none()
                    && item.transfer_policy == ItemTransferPolicy::CharacterBound
            });
            if !presentation_valid
                || item.weight_grams > 1_000_000
                || item.slot_cost == 0
                || item.slot_cost > 1_000
                || item.item_level == 0
                || item.item_level > 1_000
                || !stack_valid
                || !container_valid
                || !sealed_valid
            {
                return Err(WorldBuildError::InvalidItemPrototype(item.prototype_id));
            }
            let prototype_id = item.prototype_id;
            if item_prototypes.contains_key(&prototype_id) {
                return Err(WorldBuildError::DuplicateItemId(prototype_id));
            }
            item_prototypes.insert(
                prototype_id.clone(),
                ItemPrototype {
                    name: item.name,
                    description: item.description,
                    aliases: item.aliases,
                    equipment_slot: item.equipment_slot,
                    attack_bonus: item.attack_bonus,
                    heal_amount: item.heal_amount,
                    consumable: item.consumable,
                    effect: item.effect,
                    weight_grams: item.weight_grams,
                    slot_cost: item.slot_cost,
                    rarity: item.rarity,
                    item_level: item.item_level,
                    stack_policy: item.stack_policy,
                    max_stack_units: item.max_stack_units,
                    container: item.container,
                    transfer_policy: item.transfer_policy,
                    sealed_loot_table_id: item.sealed_loot_table_id,
                },
            );
        }
        let mut unresolved_exits = Vec::with_capacity(definition.rooms.len());

        for definition in definition.rooms {
            if definition.stable_id.trim().is_empty() {
                return Err(WorldBuildError::EmptyRoomId);
            }
            if stable_rooms.contains_key(&definition.stable_id) {
                return Err(WorldBuildError::DuplicateRoomId(definition.stable_id));
            }

            let stable_id = definition.stable_id;
            let room_id = entities.insert(Entity::Room(Shared::new(Room {
                stable_id: stable_id.clone(),
                title: definition.title,
                prose: definition.prose,
                image: definition.image,
                audio: definition.audio,
                exits: BTreeMap::new(),
            })));
            stable_rooms.insert(stable_id, room_id);
            unresolved_exits.push((room_id, definition.exits));
        }

        let start_room = stable_rooms
            .get(&definition.start_room_id)
            .copied()
            .ok_or_else(|| WorldBuildError::UnknownStartRoom(definition.start_room_id.clone()))?;

        for (room_id, exits) in unresolved_exits {
            let source_id = room(&entities, room_id).stable_id.clone();
            for exit in exits {
                let destination =
                    stable_rooms
                        .get(&exit.target_room_id)
                        .copied()
                        .ok_or_else(|| WorldBuildError::UnknownExitTarget {
                            room_id: source_id.clone(),
                            target_room_id: exit.target_room_id.clone(),
                        })?;
                let previous = room_mut(&mut entities, room_id)
                    .exits
                    .insert(exit.direction, RoomExit::Resident(destination));
                if previous.is_some() {
                    return Err(WorldBuildError::DuplicateExitDirection {
                        room_id: source_id.clone(),
                        direction: exit.direction,
                    });
                }
            }
        }

        let mut item_instances = BTreeMap::new();
        for instance in definition.initial_item_instances {
            let instance_id = instance.instance_id;
            if !valid_item_instance_id(instance_id.as_str()) {
                return Err(WorldBuildError::EmptyItemId);
            }
            if !item_prototypes.contains_key(&instance.prototype_id) {
                return Err(WorldBuildError::UnknownItemPrototype(instance.prototype_id));
            }
            if item_prototypes
                .get(&instance.prototype_id)
                .is_some_and(|prototype| prototype.sealed_loot_table_id.is_some())
            {
                return Err(WorldBuildError::InvalidItemPrototype(instance.prototype_id));
            }
            if item_instances.contains_key(&instance_id) {
                return Err(WorldBuildError::DuplicateItemId(instance_id.0));
            }
            let location = match instance.location {
                InitialItemLocation::Room(room_id) => {
                    if !stable_rooms.contains_key(&room_id) {
                        return Err(WorldBuildError::UnknownRoomItem {
                            room_id,
                            item_id: instance_id.0,
                        });
                    }
                    SharedItemLocation::Room(room_id)
                }
                InitialItemLocation::NpcSpawn(npc_id) => SharedItemLocation::HeldByNpc(npc_id),
                InitialItemLocation::Container(container_id) => {
                    SharedItemLocation::Contained(container_id)
                }
                InitialItemLocation::Staged => SharedItemLocation::Staged,
            };
            item_instances.insert(
                instance_id.clone(),
                ItemInstance {
                    prototype_id: instance.prototype_id,
                    location,
                    binding: ItemBinding::Unbound,
                    item_affixes: Vec::new(),
                },
            );
        }
        if item_instances.len() > MAX_ITEM_INSTANCES {
            return Err(WorldBuildError::TooManyItemInstances);
        }
        if validate_item_graph(&item_instances, &item_prototypes).is_err() {
            return Err(WorldBuildError::InvalidItemPrototype(
                "container graph".into(),
            ));
        }
        let mut staged = std::collections::HashSet::new();
        for item_id in staged_item_ids {
            let item_id = ItemInstanceId(item_id);
            if !staged.insert(item_id.clone()) {
                return Err(WorldBuildError::DuplicateStagedItem(item_id.0));
            }
            if !item_instances.contains_key(&item_id) {
                return Err(WorldBuildError::UnknownStagedItem(item_id.0));
            }
            if let Some(ItemInstance {
                location: SharedItemLocation::Room(room_id),
                ..
            }) = item_instances.get(&item_id)
            {
                return Err(WorldBuildError::ItemPlacedAndStaged {
                    item_id: item_id.0,
                    room_id: room_id.clone(),
                });
            }
            item_instances
                .get_mut(&item_id)
                .expect("validated staged instance exists")
                .location = SharedItemLocation::Staged;
        }

        let progression =
            ProgressionCatalog::compile(builtin_progression_definition(), &BTreeMap::new())
                .expect("the built-in progression is valid");
        let progression_catalog = progression.creation_view();
        let initial_item_prototypes = item_instances
            .iter()
            .map(|(id, instance)| (id.clone(), instance.prototype_id.clone()))
            .collect();
        let item_provenance = item_instances
            .keys()
            .cloned()
            .map(|id| (id, ItemProvenance::AuthoredInitial))
            .collect::<BTreeMap<_, _>>();
        let next_item_instance_sequence = item_instances
            .keys()
            .filter_map(runtime_item_sequence)
            .max()
            .unwrap_or(0)
            .checked_add(1)
            .ok_or_else(|| WorldBuildError::InvalidItemPrototype("runtime:*".into()))?;

        Ok(Self {
            entities: entities.into(),
            stable_rooms,
            area_generation: None,
            active_areas: BTreeMap::new(),
            retired_areas: BTreeMap::new(),
            room_areas: HashMap::new(),
            item_prototypes,
            initial_item_prototypes,
            item_instances: item_instances.into(),
            next_item_instance_sequence,
            item_provenance: item_provenance.into(),
            affixes: BTreeMap::new(),
            affix_pools: BTreeMap::new(),
            loot_tables: BTreeMap::new(),
            loot_sources: BTreeMap::new(),
            next_loot_drop_id: LootDropId(1),
            next_reward_open_id: RewardOpenId(1),
            loot_encounters: BTreeMap::new().into(),
            ground_loot: BTreeMap::new().into(),
            party_loot_cursors: BTreeMap::new().into(),
            room_light: HashMap::new(),
            hidden_items: HashMap::new(),
            hidden_npcs: HashMap::new(),
            secret_exits: HashMap::new(),
            pending_npc_ability_sets: BTreeMap::new(),
            npcs: BTreeMap::new().into(),
            abilities: BTreeMap::new(),
            social_emotes: BTreeMap::new(),
            adventure_content: std::sync::Arc::new(AdventureContent::legacy(
                &definition.start_room_id,
            )),
            progression,
            progression_catalog,
            world_progression_flags: BTreeSet::new().into(),
            character_gameplay: HashMap::new().into(),
            sessions: HashMap::new(),
            observer_views: HashMap::new(),
            last_view_bump: None,
            start_room,
            start_room_stable_id: definition.start_room_id,
            revision: 1,
            logical_tick: 0,
            rng: DeterministicRng::new(0x5449_414d_5544),
            adventure: AdventureRuntimeSnapshot::default().into(),
            next_trade_id: TradeId(1),
            trades: BTreeMap::new().into(),
            last_trade_request_ticks: HashMap::new(),
            tick_resolution_pending: false,
            durable: DurableBaseline::default(),
        })
    }

    pub fn demo() -> Self {
        Self::from_definition(WorldDefinition {
            start_room_id: "demo:lantern-square".into(),
            item_prototypes: vec![ItemPrototypeDefinition {
                prototype_id: "demo:blue-lantern".into(),
                name: "blue-glass lantern".into(),
                description: "Cold blue light pools inside the rain-speckled glass.".into(),
                aliases: vec!["lantern".into(), "lamp".into()],
                equipment_slot: None,
                attack_bonus: 0,
                heal_amount: 0,
                consumable: false,
                effect: None,
                weight_grams: 1_000,
                slot_cost: 1,
                rarity: ItemRarity::Common,
                item_level: 1,
                stack_policy: ItemStackPolicy::Unique,
                max_stack_units: 1,
                container: None,
                transfer_policy: ItemTransferPolicy::Transferable,
                sealed_loot_table_id: None,
            }],
            initial_item_instances: vec![InitialItemInstanceDefinition {
                instance_id: ItemInstanceId("demo:blue-lantern".into()),
                prototype_id: "demo:blue-lantern".into(),
                location: InitialItemLocation::Room("demo:lantern-square".into()),
            }],
            rooms: vec![
                RoomDefinition {
                    stable_id: "demo:lantern-square".into(),
                    title: "Lantern Square".into(),
                    prose: vec![
                        "Rain has polished the old stones to a dark shine. Beneath the clock tower, a few late travellers gather around a blue-glass lantern.".into(),
                        "The illustration, when available, is atmospheric only; the text remains the complete source of playable information.".into(),
                    ],
                    image: None,
                    audio: None,
                    item_ids: Vec::new(),
                    exits: vec![
                        ExitDefinition {
                            direction: Direction::North,
                            target_room_id: "demo:north-gate".into(),
                        },
                        ExitDefinition {
                            direction: Direction::Down,
                            target_room_id: "demo:archive".into(),
                        },
                    ],
                },
                RoomDefinition {
                    stable_id: "demo:north-gate".into(),
                    title: "The North Gate".into(),
                    prose: vec![
                        "An iron-bound gate stands open between squat stone towers. The road continues into fields silvered by rain.".into(),
                    ],
                    image: None,
                    audio: None,
                    item_ids: Vec::new(),
                    exits: vec![ExitDefinition {
                        direction: Direction::South,
                        target_room_id: "demo:lantern-square".into(),
                    }],
                },
                RoomDefinition {
                    stable_id: "demo:archive".into(),
                    title: "The Cartographer's Archive".into(),
                    prose: vec![
                        "Shelves of annotated maps surround a broad oak table. A dry stair climbs back toward the square.".into(),
                    ],
                    image: None,
                    audio: None,
                    item_ids: Vec::new(),
                    exits: vec![ExitDefinition {
                        direction: Direction::Up,
                        target_room_id: "demo:lantern-square".into(),
                    }],
                },
            ],
        })
        .expect("the built-in demo world is valid")
    }

    /// Atomically installs one reviewed character progression and ability catalog.
    pub fn configure_progression(
        &mut self,
        definition: CharacterProgressionDefinition,
        abilities: Vec<AbilityDefinition>,
    ) -> Result<(), WorldError> {
        if abilities.len() > MAX_ABILITIES {
            return Err(WorldError::InvalidAbilityDefinition);
        }
        if abilities
            .iter()
            .filter(|ability| ability.communication.is_some())
            .count()
            > MAX_COMMUNICATION_ABILITIES
        {
            return Err(WorldError::InvalidAbilityDefinition);
        }
        let ability_count = abilities.len();
        let unique_ability_names = abilities
            .iter()
            .map(|ability| ability.name.to_lowercase())
            .collect::<BTreeSet<_>>();
        if unique_ability_names.len() != ability_count {
            return Err(WorldError::InvalidAbilityDefinition);
        }
        let ability_map = abilities
            .into_iter()
            .map(|ability| {
                validate_ability_definition(&ability)?;
                if ability.communication.as_ref().is_some_and(|communication| {
                    communication.blocked_room_ids.iter().any(|room_id| {
                        !self.stable_rooms.contains_key(room_id)
                            && (self.area_generation.is_none() || !valid_definition_id(room_id))
                    })
                }) {
                    return Err(WorldError::InvalidAbilityDefinition);
                }
                Ok((ability.id.clone(), ability))
            })
            .collect::<Result<BTreeMap<_, _>, WorldError>>()?;
        if ability_map.len() != ability_count || ability_map.len() > MAX_ABILITIES {
            return Err(WorldError::InvalidAbilityDefinition);
        }

        let progression = ProgressionCatalog::compile(definition, &ability_map)?;
        self.progression_catalog = progression.creation_view();
        self.progression = progression;
        self.abilities = ability_map;
        Ok(())
    }

    /// Atomically installs the reviewed, bounded local-emote catalog.
    ///
    /// The catalog is immutable authored configuration. It is deliberately
    /// absent from runtime snapshots and checkpoints, and no template is
    /// accepted unless every perspective has a bounded worst-case rendering.
    pub fn configure_social_emotes(
        &mut self,
        definitions: Vec<SocialEmoteDefinition>,
    ) -> Result<(), WorldError> {
        if definitions.len() > MAX_SOCIAL_EMOTES
            || definitions
                .iter()
                .any(|definition| !emote::validate_definition(definition))
        {
            return Err(WorldError::InvalidEmoteDefinition);
        }
        let definition_count = definitions.len();
        let unique_ids = definitions
            .iter()
            .map(|definition| definition.id.as_str())
            .collect::<BTreeSet<_>>();
        if unique_ids.len() != definition_count {
            return Err(WorldError::InvalidEmoteDefinition);
        }
        let catalog = definitions
            .into_iter()
            .map(|definition| (definition.command.clone(), definition))
            .collect::<BTreeMap<_, _>>();
        if catalog.len() != definition_count {
            return Err(WorldError::InvalidEmoteDefinition);
        }
        self.social_emotes = catalog;
        Ok(())
    }

    /// Returns command-discovery metadata in deterministic command order.
    ///
    /// Authored templates never leave core through this view.
    pub fn social_emotes(&self) -> Vec<SocialEmoteView> {
        self.social_emotes.values().map(emote::view).collect()
    }

    pub fn progression_catalog(&self) -> &CharacterCreationCatalogView {
        &self.progression_catalog
    }

    pub fn initial_gameplay(
        &self,
        lineage_id: Option<&str>,
        background: BackgroundSelection<'_>,
        base_class_id: Option<&str>,
    ) -> Result<CharacterGameplaySnapshot, WorldError> {
        self.progression
            .initial_gameplay(lineage_id, background, base_class_id, &self.abilities)
    }

    pub fn configure_npc_abilities(
        &mut self,
        definition: NpcAbilitySetDefinition,
    ) -> Result<(), WorldError> {
        self.validate_npc_ability_set(&definition)?;
        let npc = self
            .npcs
            .get_mut(&definition.npc_id)
            .ok_or(WorldError::NpcNotFound)?;
        npc.resources = definition
            .resource_pools
            .into_iter()
            .map(|(id, maximum)| {
                (
                    id,
                    ResourcePool {
                        current: maximum,
                        max: maximum,
                    },
                )
            })
            .collect();
        npc.resource_regeneration = definition.resource_regeneration;
        npc.resistances = definition.resistances;
        npc.ability_ids = definition.ability_ids;
        npc.ability_cooldowns.clear();
        Ok(())
    }

    pub(crate) fn validate_npc_ability_set(
        &self,
        definition: &NpcAbilitySetDefinition,
    ) -> Result<(), WorldError> {
        let unique_abilities = definition
            .ability_ids
            .iter()
            .collect::<std::collections::HashSet<_>>()
            .len()
            == definition.ability_ids.len();
        let abilities_usable = definition.ability_ids.iter().all(|id| {
            self.abilities.get(id).is_some_and(|ability| {
                ability.communication.is_none()
                    && ability
                        .resource_costs
                        .keys()
                        .all(|resource| definition.resource_pools.contains_key(resource))
            })
        });
        if definition.resource_pools.len() > MAX_RESOURCE_POOLS
            || definition.resource_regeneration.len() > MAX_RESOURCE_POOLS
            || definition.resistances.len() > MAX_RESISTANCES
            || definition.ability_ids.len() > MAX_KNOWN_ABILITIES
            || !unique_abilities
            || !abilities_usable
            || definition
                .resource_pools
                .iter()
                .any(|(id, value)| !valid_stat_id(id) || *value == 0 || *value > MAX_RESOURCE_VALUE)
            || definition.resource_regeneration.iter().any(|(id, value)| {
                !valid_stat_id(id)
                    || !definition.resource_pools.contains_key(id)
                    || *value > MAX_RESOURCE_VALUE
            })
            || definition
                .resistances
                .values()
                .any(|value| *value > MAX_RESISTANCE_PERCENT)
        {
            return Err(WorldError::InvalidAbilityDefinition);
        }
        Ok(())
    }

    pub fn start_room_id(&self) -> &str {
        &self.start_room_stable_id
    }

    pub fn revision(&self) -> u64 {
        self.revision
    }

    pub fn logical_tick(&self) -> u64 {
        self.logical_tick
    }

    pub fn set_rng_seed(&mut self, seed: u64) {
        self.rng = DeterministicRng::new(seed);
    }

    pub fn configure_affixes(
        &mut self,
        mut definitions: Vec<AffixDefinition>,
        mut pools: Vec<AffixPoolDefinition>,
    ) -> Result<(), WorldError> {
        if definitions.len() > MAX_AFFIX_DEFINITIONS || pools.len() > MAX_AFFIX_POOLS {
            return Err(WorldError::InvalidGameplayDefinition);
        }
        definitions.sort_by(|left, right| left.id.cmp(&right.id));
        pools.sort_by(|left, right| left.id.cmp(&right.id));
        let mut compiled_definitions = BTreeMap::new();
        for definition in definitions {
            let valid_delta = (0..=MAX_AFFIX_MODIFIER).contains(&definition.attack_bonus_delta)
                && (0..=MAX_AFFIX_MODIFIER).contains(&definition.heal_amount_delta)
                && (definition.attack_bonus_delta > 0 || definition.heal_amount_delta > 0);
            if !valid_definition_id(&definition.id)
                || !safe_definition_text(&definition.label, 64)
                || !valid_delta
                || compiled_definitions
                    .insert(definition.id.clone(), definition)
                    .is_some()
            {
                return Err(WorldError::InvalidGameplayDefinition);
            }
        }
        let mut compiled_pools = BTreeMap::new();
        let mut total_entries = 0_usize;
        for mut pool in pools {
            pool.entries.sort_by(|left, right| left.id.cmp(&right.id));
            total_entries = total_entries
                .checked_add(pool.entries.len())
                .ok_or(WorldError::InvalidGameplayDefinition)?;
            if !valid_definition_id(&pool.id)
                || pool.entries.is_empty()
                || pool.entries.len() > MAX_AFFIX_POOL_ENTRIES
                || total_entries > MAX_TOTAL_AFFIX_POOL_ENTRIES
            {
                return Err(WorldError::InvalidGameplayDefinition);
            }
            let digest_sha256 = canonical_sha256(&pool)?;
            let mut ids = BTreeSet::new();
            let mut total_weight = 0_u64;
            for entry in &pool.entries {
                if !ids.insert(entry.id.clone())
                    || !valid_definition_id(&entry.id)
                    || !(1..=1_000_000).contains(&entry.weight)
                {
                    return Err(WorldError::InvalidGameplayDefinition);
                }
                total_weight = total_weight
                    .checked_add(u64::from(entry.weight))
                    .ok_or(WorldError::InvalidGameplayDefinition)?;
                match entry.kind {
                    AffixPoolEntryKind::Affix => {
                        if entry
                            .affix_id
                            .as_ref()
                            .is_none_or(|id| !compiled_definitions.contains_key(id))
                        {
                            return Err(WorldError::InvalidGameplayDefinition);
                        }
                    }
                    AffixPoolEntryKind::Nothing if entry.affix_id.is_none() => {}
                    AffixPoolEntryKind::Nothing => {
                        return Err(WorldError::InvalidGameplayDefinition);
                    }
                }
            }
            if compiled_pools
                .insert(
                    pool.id.clone(),
                    AffixPool {
                        id: pool.id,
                        digest_sha256,
                        entries: pool.entries,
                        total_weight,
                    },
                )
                .is_some()
            {
                return Err(WorldError::InvalidGameplayDefinition);
            }
        }
        self.affixes = compiled_definitions;
        self.affix_pools = compiled_pools;
        Ok(())
    }

    pub fn configure_loot(
        &mut self,
        mut tables: Vec<LootTableDefinition>,
        mut sources: Vec<LootSourceDefinition>,
    ) -> Result<(), WorldError> {
        if tables.len() > 256 || sources.len() > 1_024 {
            return Err(WorldError::InvalidGameplayDefinition);
        }
        tables.sort_by(|left, right| left.id.cmp(&right.id));
        sources.sort_by(|left, right| left.id.cmp(&right.id));
        let sealed_table_ids = self
            .item_prototypes
            .values()
            .filter_map(|prototype| prototype.sealed_loot_table_id.clone())
            .collect::<BTreeSet<_>>();
        let mut compiled = BTreeMap::new();
        let mut total_entries = 0_usize;
        for mut table in tables {
            if !valid_definition_id(&table.id)
                || table.digest_sha256.len() != 64
                || !table
                    .digest_sha256
                    .bytes()
                    .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
                || table.guaranteed.len() > 8
                || table.pools.len() > 8
            {
                return Err(WorldError::InvalidGameplayDefinition);
            }
            table
                .guaranteed
                .sort_by(|left, right| left.id.cmp(&right.id));
            table.pools.sort_by(|left, right| left.id.cmp(&right.id));
            let mut ids = BTreeSet::new();
            let mut worst_case_units = 0_usize;
            let mut minimum_units = 0_usize;
            let mut odds_rows = 0_usize;
            let sealed_table = sealed_table_ids.contains(&table.id);
            for guaranteed in &table.guaranteed {
                let prototype = self
                    .item_prototypes
                    .get(&guaranteed.item_prototype_id)
                    .ok_or(WorldError::InvalidGameplayDefinition)?;
                if !ids.insert(guaranteed.id.clone())
                    || !valid_definition_id(&guaranteed.id)
                    || !(1..=32).contains(&guaranteed.quantity)
                    || guaranteed.affix_pool_id.as_ref().is_some_and(|pool_id| {
                        guaranteed.quantity != 1
                            || !affix_pool_applies(
                                pool_id,
                                prototype,
                                &self.affix_pools,
                                &self.affixes,
                            )
                    })
                    || (sealed_table && prototype.sealed_loot_table_id.is_some())
                {
                    return Err(WorldError::InvalidGameplayDefinition);
                }
                worst_case_units = worst_case_units
                    .checked_add(usize::from(guaranteed.quantity))
                    .ok_or(WorldError::InvalidGameplayDefinition)?;
                minimum_units = minimum_units
                    .checked_add(usize::from(guaranteed.quantity))
                    .ok_or(WorldError::InvalidGameplayDefinition)?;
                if sealed_table {
                    odds_rows = odds_rows
                        .checked_add(1)
                        .and_then(|rows| {
                            guaranteed
                                .affix_pool_id
                                .as_ref()
                                .map_or(Some(rows), |pool_id| {
                                    rows.checked_add(self.affix_pools.get(pool_id)?.entries.len())
                                })
                        })
                        .ok_or(WorldError::InvalidGameplayDefinition)?;
                }
            }
            for pool in &mut table.pools {
                if !ids.insert(pool.id.clone())
                    || !valid_definition_id(&pool.id)
                    || !(1..=8).contains(&pool.rolls)
                    || pool.entries.is_empty()
                    || pool.entries.len() > 64
                {
                    return Err(WorldError::InvalidGameplayDefinition);
                }
                pool.entries.sort_by(|left, right| left.id.cmp(&right.id));
                total_entries = total_entries
                    .checked_add(pool.entries.len())
                    .ok_or(WorldError::InvalidGameplayDefinition)?;
                let mut total_weight = 0_u64;
                let mut maximum_quantity = 0_u16;
                let mut minimum_quantity = u16::MAX;
                let mut can_roll_nothing = false;
                for entry in &pool.entries {
                    if !ids.insert(entry.id.clone())
                        || !valid_definition_id(&entry.id)
                        || !(1..=1_000_000).contains(&entry.weight)
                    {
                        return Err(WorldError::InvalidGameplayDefinition);
                    }
                    total_weight = total_weight
                        .checked_add(u64::from(entry.weight))
                        .ok_or(WorldError::InvalidGameplayDefinition)?;
                    if sealed_table {
                        odds_rows = odds_rows
                            .checked_add(1)
                            .and_then(|rows| {
                                entry.affix_pool_id.as_ref().map_or(Some(rows), |pool_id| {
                                    rows.checked_add(self.affix_pools.get(pool_id)?.entries.len())
                                })
                            })
                            .ok_or(WorldError::InvalidGameplayDefinition)?;
                    }
                    match entry.kind {
                        LootEntryKind::Item => {
                            let prototype_id = entry
                                .item_prototype_id
                                .as_ref()
                                .ok_or(WorldError::InvalidGameplayDefinition)?;
                            let minimum = entry
                                .min_quantity
                                .ok_or(WorldError::InvalidGameplayDefinition)?;
                            let maximum = entry
                                .max_quantity
                                .ok_or(WorldError::InvalidGameplayDefinition)?;
                            let prototype = self
                                .item_prototypes
                                .get(prototype_id)
                                .ok_or(WorldError::InvalidGameplayDefinition)?;
                            if minimum == 0
                                || minimum > maximum
                                || maximum > 32
                                || entry.affix_pool_id.as_ref().is_some_and(|pool_id| {
                                    minimum != 1
                                        || maximum != 1
                                        || !affix_pool_applies(
                                            pool_id,
                                            prototype,
                                            &self.affix_pools,
                                            &self.affixes,
                                        )
                                })
                                || (sealed_table && prototype.sealed_loot_table_id.is_some())
                            {
                                return Err(WorldError::InvalidGameplayDefinition);
                            }
                            maximum_quantity = maximum_quantity.max(maximum);
                            minimum_quantity = minimum_quantity.min(minimum);
                        }
                        LootEntryKind::Nothing => {
                            if entry.item_prototype_id.is_some()
                                || entry.min_quantity.is_some()
                                || entry.max_quantity.is_some()
                                || entry.affix_pool_id.is_some()
                            {
                                return Err(WorldError::InvalidGameplayDefinition);
                            }
                            can_roll_nothing = true;
                        }
                    }
                }
                if total_weight == 0 {
                    return Err(WorldError::InvalidGameplayDefinition);
                }
                worst_case_units = worst_case_units
                    .checked_add(usize::from(pool.rolls) * usize::from(maximum_quantity))
                    .ok_or(WorldError::InvalidGameplayDefinition)?;
                if !can_roll_nothing {
                    minimum_units = minimum_units
                        .checked_add(usize::from(pool.rolls) * usize::from(minimum_quantity))
                        .ok_or(WorldError::InvalidGameplayDefinition)?;
                }
            }
            if total_entries > 4_096
                || worst_case_units > MAX_LOOT_UNITS_PER_RESOLUTION
                || (sealed_table
                    && (minimum_units == 0
                        || worst_case_units > MAX_SEALED_REWARD_UNITS
                        || odds_rows > MAX_REWARD_ODDS_ROWS))
            {
                return Err(WorldError::InvalidGameplayDefinition);
            }
            let id = table.id.clone();
            if compiled
                .insert(
                    id.clone(),
                    LootTable {
                        id,
                        digest_sha256: table.digest_sha256,
                        guaranteed: table.guaranteed,
                        pools: table.pools,
                        worst_case_units,
                    },
                )
                .is_some()
            {
                return Err(WorldError::InvalidGameplayDefinition);
            }
        }
        let mut compiled_sources = BTreeMap::new();
        let mut used_tables = BTreeSet::new();
        let mut source_ids = BTreeSet::new();
        for source in sources {
            if source.kind != LootSourceKind::NpcSpawn
                || !valid_definition_id(&source.id)
                || !source_ids.insert(source.id.clone())
                || !valid_definition_id(&source.npc_spawn_id)
                || !compiled.contains_key(&source.loot_table_id)
                || sealed_table_ids.contains(&source.loot_table_id)
                || !self.npcs.contains_key(&source.npc_spawn_id)
            {
                return Err(WorldError::InvalidGameplayDefinition);
            }
            let combined_worst_case = self
                .npcs
                .get(&source.npc_spawn_id)
                .ok_or(WorldError::InvalidGameplayDefinition)?
                .loot_item_ids
                .len()
                .checked_add(
                    compiled
                        .get(&source.loot_table_id)
                        .ok_or(WorldError::InvalidGameplayDefinition)?
                        .worst_case_units,
                )
                .ok_or(WorldError::InvalidGameplayDefinition)?;
            if combined_worst_case > MAX_LOOT_UNITS_PER_RESOLUTION {
                return Err(WorldError::InvalidGameplayDefinition);
            }
            used_tables.insert(source.loot_table_id.clone());
            let npc_spawn_id = source.npc_spawn_id;
            if compiled_sources
                .insert(
                    npc_spawn_id,
                    LootSource {
                        loot_table_id: source.loot_table_id,
                        rights: source.rights,
                    },
                )
                .is_some()
            {
                return Err(WorldError::InvalidGameplayDefinition);
            }
        }
        used_tables.extend(sealed_table_ids.iter().cloned());
        if sealed_table_ids.iter().any(|id| !compiled.contains_key(id))
            || compiled.keys().any(|id| !used_tables.contains(id))
        {
            return Err(WorldError::InvalidGameplayDefinition);
        }
        self.loot_tables = compiled;
        self.loot_sources = compiled_sources;
        Ok(())
    }

    /// Allocates one durable repeatable instance without publishing it into a
    /// room or inventory. The caller must include creation/assignment in the
    /// same checkpointed durable commit before making the instance visible.
    pub fn create_staged_item_instance(
        &mut self,
        prototype_id: &str,
    ) -> Result<ItemInstanceId, WorldError> {
        if !self.item_prototypes.contains_key(prototype_id)
            || self.item_instances.len() >= MAX_ITEM_INSTANCES
        {
            return Err(WorldError::InvalidGameplayDefinition);
        }
        loop {
            let sequence = self.next_item_instance_sequence;
            self.next_item_instance_sequence = sequence
                .checked_add(1)
                .ok_or(WorldError::InvalidGameplayDefinition)?;
            let instance_id = ItemInstanceId(format!("runtime:{sequence}"));
            if self.item_instances.contains_key(&instance_id) {
                continue;
            }
            self.item_instances.insert(
                instance_id.clone(),
                ItemInstance {
                    prototype_id: prototype_id.to_owned(),
                    location: SharedItemLocation::Staged,
                    binding: ItemBinding::Unbound,
                    item_affixes: Vec::new(),
                },
            );
            self.item_provenance.insert(
                instance_id.clone(),
                ItemProvenance::RuntimeCreated {
                    created_tick: self.logical_tick,
                },
            );
            return Ok(instance_id);
        }
    }

    pub(crate) fn item_prototype_for(
        &self,
        instance_id: &ItemInstanceId,
    ) -> Result<&ItemPrototype, WorldError> {
        let instance = self
            .item_instances
            .get(instance_id)
            .ok_or(WorldError::InvalidEntity)?;
        let prototype = self
            .item_prototypes
            .get(&instance.prototype_id)
            .ok_or(WorldError::InvalidEntity)?;
        debug_assert!(matches!(
            prototype.transfer_policy,
            ItemTransferPolicy::Transferable
                | ItemTransferPolicy::CharacterBound
                | ItemTransferPolicy::NonTransferable
        ));
        Ok(prototype)
    }

    pub(crate) fn item_gameplay_for(
        &self,
        instance_id: &ItemInstanceId,
    ) -> Result<&ItemPrototype, WorldError> {
        self.item_prototype_for(instance_id)
    }

    pub(crate) fn item_affix(&self, instance_id: &ItemInstanceId) -> Option<&AppliedItemAffix> {
        self.item_instances
            .get(instance_id)
            .and_then(|instance| instance.item_affixes.first())
    }

    pub(crate) fn item_affix_view(&self, instance_id: &ItemInstanceId) -> Option<ItemAffixView> {
        self.item_affix(instance_id).map(|affix| ItemAffixView {
            label: affix.label.clone(),
            position: affix.position,
            attack_bonus_delta: affix.attack_bonus_delta,
            heal_amount_delta: affix.heal_amount_delta,
        })
    }

    pub(crate) fn item_effective_name(
        &self,
        instance_id: &ItemInstanceId,
    ) -> Result<String, WorldError> {
        let prototype = self.item_prototype_for(instance_id)?;
        Ok(match self.item_affix(instance_id) {
            Some(affix) if affix.position == AffixPosition::Prefix => {
                format!("{} {}", affix.label, prototype.name)
            }
            Some(affix) => format!("{} {}", prototype.name, affix.label),
            None => prototype.name.clone(),
        })
    }

    pub(crate) fn item_effective_attack_bonus(
        &self,
        instance_id: &ItemInstanceId,
    ) -> Result<i32, WorldError> {
        let prototype = self.item_prototype_for(instance_id)?;
        prototype
            .attack_bonus
            .checked_add(
                self.item_affix(instance_id)
                    .map_or(0, |affix| affix.attack_bonus_delta),
            )
            .ok_or(WorldError::InvalidEntity)
    }

    pub(crate) fn item_effective_heal_amount(
        &self,
        instance_id: &ItemInstanceId,
    ) -> Result<i32, WorldError> {
        let prototype = self.item_prototype_for(instance_id)?;
        prototype
            .heal_amount
            .checked_add(
                self.item_affix(instance_id)
                    .map_or(0, |affix| affix.heal_amount_delta),
            )
            .ok_or(WorldError::InvalidEntity)
    }

    pub(crate) fn item_is_openable_reward(
        &self,
        instance_id: &ItemInstanceId,
        character_id: CharacterId,
    ) -> bool {
        let Some(instance) = self.item_instances.get(instance_id) else {
            return false;
        };
        let Some(prototype) = self.item_prototypes.get(&instance.prototype_id) else {
            return false;
        };
        matches!(instance.location, SharedItemLocation::Carried(owner) if owner == character_id)
            && instance.binding == (ItemBinding::Character { character_id })
            && instance.item_affixes.is_empty()
            && prototype.transfer_policy == ItemTransferPolicy::CharacterBound
            && prototype.stack_policy == ItemStackPolicy::Unique
            && prototype.max_stack_units == 1
            && prototype.consumable
            && prototype.container.is_none()
            && prototype.equipment_slot.is_none()
            && prototype.attack_bonus == 0
            && prototype.heal_amount == 0
            && prototype.effect.is_none()
            && prototype
                .sealed_loot_table_id
                .as_ref()
                .is_some_and(|table_id| {
                    self.loot_tables.contains_key(table_id)
                        && matches!(
                            self.item_provenance.get(instance_id),
                            Some(ItemProvenance::NpcLoot {
                                sealed_loot_table_id: Some(authored_table_id),
                                ..
                            }) if authored_table_id == table_id
                        )
                })
    }

    pub(crate) fn item_location(
        &self,
        instance_id: &ItemInstanceId,
    ) -> Option<&SharedItemLocation> {
        self.item_instances
            .get(instance_id)
            .map(|instance| &instance.location)
    }

    pub(crate) fn set_item_location(
        &mut self,
        instance_id: &ItemInstanceId,
        location: SharedItemLocation,
    ) -> Result<(), WorldError> {
        self.item_instances
            .get_mut(instance_id)
            .ok_or(WorldError::InvalidEntity)?
            .location = location;
        Ok(())
    }

    pub fn checkpoint(&self) -> WorldCheckpoint {
        WorldCheckpoint {
            entities: self.entities.clone(),
            item_instances: self.item_instances.clone(),
            next_item_instance_sequence: self.next_item_instance_sequence,
            item_provenance: self.item_provenance.clone(),
            next_reward_open_id: self.next_reward_open_id,
            next_loot_drop_id: self.next_loot_drop_id,
            loot_encounters: self.loot_encounters.clone(),
            ground_loot: self.ground_loot.clone(),
            party_loot_cursors: self.party_loot_cursors.clone(),
            npcs: self.npcs.clone(),
            character_gameplay: self.character_gameplay.clone(),
            observer_views: self.observer_views.clone(),
            last_view_bump: self.last_view_bump.clone(),
            revision: self.revision,
            logical_tick: self.logical_tick,
            rng: self.rng.clone(),
            adventure: self.adventure.clone(),
            world_progression_flags: self.world_progression_flags.clone(),
            next_trade_id: self.next_trade_id,
            trades: self.trades.clone(),
            last_trade_request_ticks: self.last_trade_request_ticks.clone(),
            tick_resolution_pending: self.tick_resolution_pending,
        }
    }

    pub fn restore_checkpoint(&mut self, checkpoint: WorldCheckpoint) {
        self.entities = checkpoint.entities;
        self.item_instances = checkpoint.item_instances;
        self.next_item_instance_sequence = checkpoint.next_item_instance_sequence;
        self.item_provenance = checkpoint.item_provenance;
        self.next_reward_open_id = checkpoint.next_reward_open_id;
        self.next_loot_drop_id = checkpoint.next_loot_drop_id;
        self.loot_encounters = checkpoint.loot_encounters;
        self.ground_loot = checkpoint.ground_loot;
        self.party_loot_cursors = checkpoint.party_loot_cursors;
        self.npcs = checkpoint.npcs;
        self.character_gameplay = checkpoint.character_gameplay;
        self.observer_views = checkpoint.observer_views;
        self.last_view_bump = checkpoint.last_view_bump;
        self.revision = checkpoint.revision;
        self.logical_tick = checkpoint.logical_tick;
        self.rng = checkpoint.rng;
        self.adventure = checkpoint.adventure;
        self.world_progression_flags = checkpoint.world_progression_flags;
        self.next_trade_id = checkpoint.next_trade_id;
        self.trades = checkpoint.trades;
        self.last_trade_request_ticks = checkpoint.last_trade_request_ticks;
        self.tick_resolution_pending = checkpoint.tick_resolution_pending;
    }

    pub fn lifecycle_checkpoint(&self) -> WorldLifecycleCheckpoint {
        WorldLifecycleCheckpoint {
            world: self.checkpoint(),
            stable_rooms: self.stable_rooms.clone(),
            area_generation: self.area_generation.clone(),
            active_areas: self.active_areas.clone(),
            retired_areas: self.retired_areas.clone(),
            room_areas: self.room_areas.clone(),
            item_prototypes: self.item_prototypes.clone(),
            initial_item_prototypes: self.initial_item_prototypes.clone(),
            affixes: self.affixes.clone(),
            affix_pools: self.affix_pools.clone(),
            loot_tables: self.loot_tables.clone(),
            loot_sources: self.loot_sources.clone(),
            room_light: self.room_light.clone(),
            hidden_items: self.hidden_items.clone(),
            hidden_npcs: self.hidden_npcs.clone(),
            secret_exits: self.secret_exits.clone(),
            pending_npc_ability_sets: self.pending_npc_ability_sets.clone(),
        }
    }

    pub fn restore_lifecycle_checkpoint(&mut self, checkpoint: WorldLifecycleCheckpoint) {
        // Lifecycle rollbacks follow failed or ambiguous area commits whose
        // durable outcome the caller reconciles separately.
        self.invalidate_durable_baseline();
        self.restore_checkpoint(checkpoint.world);
        self.stable_rooms = checkpoint.stable_rooms;
        self.area_generation = checkpoint.area_generation;
        self.active_areas = checkpoint.active_areas;
        self.retired_areas = checkpoint.retired_areas;
        self.room_areas = checkpoint.room_areas;
        self.item_prototypes = checkpoint.item_prototypes;
        self.initial_item_prototypes = checkpoint.initial_item_prototypes;
        self.affixes = checkpoint.affixes;
        self.affix_pools = checkpoint.affix_pools;
        self.loot_tables = checkpoint.loot_tables;
        self.loot_sources = checkpoint.loot_sources;
        self.room_light = checkpoint.room_light;
        self.hidden_items = checkpoint.hidden_items;
        self.hidden_npcs = checkpoint.hidden_npcs;
        self.secret_exits = checkpoint.secret_exits;
        self.pending_npc_ability_sets = checkpoint.pending_npc_ability_sets;
    }
}

impl World {
    pub fn configure_item_gameplay(
        &mut self,
        definition: ItemGameplayDefinition,
    ) -> Result<(), WorldError> {
        let max_stat_value = MAX_STAT_VALUE_I32;
        let valid = self.item_prototypes.contains_key(&definition.item_id)
            && (0..=max_stat_value).contains(&definition.attack_bonus)
            && (0..=max_stat_value).contains(&definition.heal_amount)
            && definition.effect.as_ref().is_none_or(|effect| {
                (1..=MAX_STATUS_MAGNITUDE).contains(&effect.magnitude)
                    && (1..=MAX_STATUS_TICKS).contains(&effect.remaining_ticks)
            });
        if !valid {
            return Err(WorldError::InvalidGameplayDefinition);
        }
        let prototype = self
            .item_prototypes
            .get_mut(&definition.item_id)
            .ok_or(WorldError::InvalidGameplayDefinition)?;
        prototype.equipment_slot = definition.equipment_slot;
        prototype.attack_bonus = definition.attack_bonus;
        prototype.heal_amount = definition.heal_amount;
        prototype.consumable = definition.consumable;
        prototype.effect = definition.effect;
        Ok(())
    }

    /// Sets the ambient light used by the perception projection. Omitted rooms
    /// are bright by default so existing packs retain their behavior.
    pub fn configure_room_light(
        &mut self,
        room_id: &str,
        light: LightLevel,
    ) -> Result<(), WorldError> {
        if !self.stable_rooms.contains_key(room_id) {
            return Err(WorldError::InvalidGameplayDefinition);
        }
        self.room_light.insert(room_id.to_owned(), light);
        Ok(())
    }

    pub fn configure_hidden_item(
        &mut self,
        item_id: &str,
        difficulty: u16,
    ) -> Result<(), WorldError> {
        if !self.item_instances.contains_key(item_id) || !(1..=100).contains(&difficulty) {
            return Err(WorldError::InvalidGameplayDefinition);
        }
        self.hidden_items.insert(item_id.to_owned(), difficulty);
        Ok(())
    }

    pub fn configure_hidden_npc(
        &mut self,
        npc_id: &str,
        difficulty: u16,
    ) -> Result<(), WorldError> {
        if !self.npcs.contains_key(npc_id) || !(1..=100).contains(&difficulty) {
            return Err(WorldError::InvalidGameplayDefinition);
        }
        self.hidden_npcs.insert(npc_id.to_owned(), difficulty);
        Ok(())
    }

    pub fn configure_secret_exit(
        &mut self,
        room_id: &str,
        direction: Direction,
        difficulty: u16,
    ) -> Result<(), WorldError> {
        let room = self
            .stable_rooms
            .get(room_id)
            .copied()
            .and_then(|id| self.entities.get(id));
        let valid_exit =
            matches!(room, Some(Entity::Room(room)) if room.exits.contains_key(&direction));
        if !valid_exit || !(1..=100).contains(&difficulty) {
            return Err(WorldError::InvalidGameplayDefinition);
        }
        self.secret_exits
            .insert((room_id.to_owned(), direction), difficulty);
        Ok(())
    }

    pub fn spawn_npc(&mut self, definition: NpcDefinition) -> Result<DomainEvent, WorldError> {
        let loot_unique = definition
            .loot_item_ids
            .iter()
            .collect::<std::collections::HashSet<_>>()
            .len()
            == definition.loot_item_ids.len();
        let max_stat_value = MAX_STAT_VALUE_I32;
        let valid = !definition.stable_id.trim().is_empty()
            && !definition.name.trim().is_empty()
            && !self.npcs.contains_key(&definition.stable_id)
            && self.stable_rooms.contains_key(&definition.room_id)
            && (1..=MAX_PROGRESSION_LEVEL).contains(&definition.level)
            && (1..=max_stat_value).contains(&definition.max_health)
            && (1..=max_stat_value).contains(&definition.strength)
            && (0..=max_stat_value).contains(&definition.defence)
            && definition.respawn_ticks > 0
            && loot_unique
            && definition.loot_item_ids.len() <= MAX_LOOT_UNITS_PER_RESOLUTION
            && definition.loot_item_ids.iter().all(|item_id| {
                self.item_instances.contains_key(item_id)
                    && (matches!(
                        self.item_location(item_id),
                        Some(SharedItemLocation::Staged)
                    ) || matches!(
                        self.item_location(item_id),
                        Some(SharedItemLocation::HeldByNpc(owner))
                            if owner == &definition.stable_id
                    ))
            });
        if !valid {
            return Err(WorldError::InvalidGameplayDefinition);
        }
        for item_id in &definition.loot_item_ids {
            self.set_item_location(
                item_id,
                SharedItemLocation::HeldByNpc(definition.stable_id.clone()),
            )?;
        }
        let event = DomainEvent::Gameplay {
            event: GameplayEvent::NpcSpawned {
                npc_id: definition.stable_id.clone(),
                npc_name: definition.name.clone(),
                room_id: definition.room_id.clone(),
            },
        };
        self.npcs.insert(
            definition.stable_id.clone(),
            Npc {
                stable_id: definition.stable_id,
                name: definition.name,
                description: definition.description,
                spawn_room_id: definition.room_id,
                level: definition.level,
                health: definition.max_health,
                max_health: definition.max_health,
                strength: definition.strength,
                defence: definition.defence,
                experience_reward: definition.experience_reward,
                loot_item_ids: definition.loot_item_ids,
                respawn_ticks: definition.respawn_ticks,
                respawn_at_tick: None,
                engaged_character: None,
                resources: BTreeMap::new(),
                resource_regeneration: BTreeMap::new(),
                ability_ids: Vec::new(),
                ability_cooldowns: BTreeMap::new(),
                resistances: BTreeMap::new(),
                statuses: Vec::new(),
            },
        );
        self.revision = self.revision.wrapping_add(1).max(1);
        self.bump_view_revisions(&event)?;
        Ok(event)
    }
}

impl World {
    pub fn apply(
        &mut self,
        session: SessionId,
        action: PlayerAction,
    ) -> Result<ActionOutcome, WorldError> {
        let mut outcome = self.apply_action(session, action)?;
        self.refresh_state_quests(session, &mut outcome)?;
        Ok(outcome)
    }

    fn apply_action(
        &mut self,
        session: SessionId,
        action: PlayerAction,
    ) -> Result<ActionOutcome, WorldError> {
        let gameplay = self
            .character_gameplay
            .get(&self.character(session)?.id)
            .ok_or(WorldError::InvalidEntity)?;
        let defeated = gameplay.recovery_at_tick.is_some();
        let stunned = gameplay
            .statuses
            .iter()
            .any(|status| status.kind == ActiveStatusKind::Stun);
        if defeated
            && !matches!(
                &action,
                PlayerAction::Look
                    | PlayerAction::Inventory
                    | PlayerAction::Examine(_)
                    | PlayerAction::Contents { .. }
                    | PlayerAction::Loot(_)
                    | PlayerAction::RewardOdds { .. }
                    | PlayerAction::Party(_)
                    | PlayerAction::Quest(_)
                    | PlayerAction::Economy(EconomyAction::Balance)
                    | PlayerAction::Weather
                    | PlayerAction::Time
            )
        {
            return Err(WorldError::GameplayUnavailable);
        }
        if stunned
            && !matches!(
                &action,
                PlayerAction::Look
                    | PlayerAction::Inventory
                    | PlayerAction::Examine(_)
                    | PlayerAction::Contents { .. }
                    | PlayerAction::Loot(_)
                    | PlayerAction::RewardOdds { .. }
                    | PlayerAction::Weather
                    | PlayerAction::Time
            )
        {
            return Err(WorldError::GameplayUnavailable);
        }
        match action {
            PlayerAction::Look => Ok(ActionOutcome {
                scene: self.scene_for(session)?,
                container_contents: None,
                trade_contents: None,
                reward_odds: None,
                output: None,
                changed_character: None,
                domain_events: Vec::new(),
            }),
            PlayerAction::Inventory => Ok(ActionOutcome {
                scene: self.scene_for(session)?,
                container_contents: None,
                trade_contents: None,
                reward_odds: None,
                output: Some(self.inventory_as_plain_text(session)?),
                changed_character: None,
                domain_events: Vec::new(),
            }),
            PlayerAction::Examine(query) => {
                let item_id =
                    self.resolve_accessible_item(session, &query, ItemScope::Accessible)?;
                let item = self.item_prototype_for(&item_id)?;
                let name = self.item_effective_name(&item_id)?;
                let affix = self.item_affix(&item_id);
                let affix_text = affix.map_or_else(String::new, |affix| {
                    format!(
                        "\r\nAfijo: {} (ataque {:+}, curación {:+}).",
                        affix.label, affix.attack_bonus_delta, affix.heal_amount_delta
                    )
                });
                Ok(ActionOutcome {
                    scene: self.scene_for(session)?,
                    container_contents: None,
                    trade_contents: None,
                    reward_odds: None,
                    output: Some(format!("{name}: {}{affix_text}\r\n", item.description)),
                    changed_character: None,
                    domain_events: Vec::new(),
                })
            }
            PlayerAction::Take(selection) => self.move_item(session, &selection, true),
            PlayerAction::Drop(selection) => self.move_item(session, &selection, false),
            PlayerAction::Contents { container, page } => {
                self.inspect_container(session, &container, page)
            }
            PlayerAction::Put {
                selection,
                container,
            } => self.move_item_to_container(session, &selection, &container, true),
            PlayerAction::Get {
                selection,
                container,
            } => self.move_item_to_container(session, &selection, &container, false),
            PlayerAction::Give {
                selection,
                recipient,
            } => self.give_item(session, &selection, &recipient),
            PlayerAction::Loot(action) => self.apply_loot(session, action),
            PlayerAction::Trade(action) => self.apply_trade(session, action),
            PlayerAction::OpenReward(query) => self.open_reward(session, &query),
            PlayerAction::RewardOdds { target, page } => self.reward_odds(session, &target, page),
            PlayerAction::Attack(query) => self.start_combat(session, &query),
            PlayerAction::Ability { ability_id, target } => {
                self.use_ability(session, &ability_id, target.as_deref())
            }
            PlayerAction::Promote { class_id } => self.promote_character(session, &class_id),
            PlayerAction::AcquireTalent { graph_id, node_id } => {
                self.acquire_talent(session, &graph_id, &node_id)
            }
            PlayerAction::Use(query) => self.use_item(session, &query),
            PlayerAction::Equip(query) => self.equip_item(session, &query),
            PlayerAction::Unequip(query) => self.unequip_item(session, &query),
            PlayerAction::Party(action) => self.apply_party(session, action),
            PlayerAction::Quest(action) => self.apply_quest(session, action),
            PlayerAction::Economy(action) => self.apply_economy(session, action),
            PlayerAction::Weather => Ok(ActionOutcome {
                scene: self.scene_for(session)?,
                container_contents: None,
                trade_contents: None,
                reward_odds: None,
                output: Some(format!(
                    "The weather is {}.\r\n",
                    weather_label(self.adventure.weather)
                )),
                changed_character: None,
                domain_events: Vec::new(),
            }),
            PlayerAction::Time => Ok(ActionOutcome {
                scene: self.scene_for(session)?,
                container_contents: None,
                trade_contents: None,
                reward_odds: None,
                output: Some(format!(
                    "It is hour {} of day {} (world tick {}).\r\n",
                    self.logical_tick % 24,
                    self.logical_tick / 24 + 1,
                    self.logical_tick
                )),
                changed_character: None,
                domain_events: Vec::new(),
            }),
            PlayerAction::Telepathy { .. }
            | PlayerAction::Social(_)
            | PlayerAction::Mail(_)
            | PlayerAction::Board(_)
            | PlayerAction::Staff(_) => Err(WorldError::GameplayUnavailable),
            PlayerAction::Move(direction) => {
                if let Some(request) = self.deferred_area_load_request(session, direction)? {
                    return Err(WorldError::AreaLoadRequired(request));
                }
                let location = self.character(session)?.location;
                let source_room = room_checked(&self.entities, location)?;
                let from_room_id = source_room.stable_id.clone();
                if !self.exit_visible_to(session, &from_room_id, direction)? {
                    return Err(WorldError::NoExit);
                }
                let destination = match source_room.exits.get(&direction) {
                    Some(RoomExit::Resident(destination)) => *destination,
                    Some(RoomExit::Deferred(request)) => {
                        return Err(WorldError::AreaLoadRequired(request.clone()));
                    }
                    None => return Err(WorldError::NoExit),
                };
                // Validate the destination before mutating character state. This
                // turns a corrupt graph edge into a recoverable world error.
                let to_room_id = room_checked(&self.entities, destination)?.stable_id.clone();
                let character_id = *self
                    .sessions
                    .get(&session)
                    .ok_or(WorldError::UnknownSession)?;
                let (actor_id, actor_name) = match self.entities.get_mut(character_id) {
                    Some(Entity::Character(character)) => {
                        character.location = destination;
                        (character.id, character.name.clone())
                    }
                    _ => return Err(WorldError::InvalidEntity),
                };
                self.revision = self.revision.wrapping_add(1).max(1);
                let event = DomainEvent::CharacterMoved {
                    actor_id,
                    actor_name,
                    from_room_id,
                    to_room_id: to_room_id.clone(),
                    direction,
                };
                let mut events = vec![event];
                let mut released_npcs = Vec::new();
                // Copy-on-write: borrow NPCs mutably only when one is engaged.
                if self
                    .npcs
                    .values()
                    .any(|npc| npc.engaged_character == Some(actor_id))
                {
                    for npc in self.npcs.values_mut() {
                        if npc.engaged_character == Some(actor_id) {
                            npc.engaged_character = None;
                            released_npcs.push((
                                npc.stable_id.clone(),
                                npc.name.clone(),
                                npc.spawn_room_id.clone(),
                            ));
                        }
                    }
                }
                for (npc_id, npc_name, room_id) in released_npcs {
                    self.loot_encounters.remove(&npc_id);
                    events.push(DomainEvent::Gameplay {
                        event: GameplayEvent::LootEncounterCancelled {
                            npc_id,
                            npc_name,
                            room_id,
                        },
                    });
                }
                self.bump_view_revisions_for_events(&events)?;
                self.observer_views
                    .get_mut(&session)
                    .ok_or(WorldError::InvalidEntity)?
                    .discovered_rooms
                    .insert(to_room_id);
                Ok(ActionOutcome {
                    scene: self.scene_for(session)?,
                    container_contents: None,
                    trade_contents: None,
                    reward_odds: None,
                    output: None,
                    changed_character: Some(self.character_snapshot(session)?),
                    domain_events: events,
                })
            }
        }
    }
}

impl World {
    pub(crate) fn observer(&self, session: SessionId) -> Result<&ObserverView, WorldError> {
        self.observer_views
            .get(&session)
            .ok_or(WorldError::UnknownSession)
    }

    pub(crate) fn session_for_character(&self, character_id: CharacterId) -> Option<SessionId> {
        self.sessions.iter().find_map(|(session, entity_id)| {
            matches!(
                self.entities.get(*entity_id),
                Some(Entity::Character(character)) if character.id == character_id
            )
            .then_some(*session)
        })
    }

    pub(crate) fn character(&self, session: SessionId) -> Result<&Character, WorldError> {
        let character_id = *self
            .sessions
            .get(&session)
            .ok_or(WorldError::UnknownSession)?;
        match self.entities.get(character_id) {
            Some(Entity::Character(character)) => Ok(character),
            _ => Err(WorldError::InvalidEntity),
        }
    }
}
