use std::{
    collections::{BTreeMap, BTreeSet, HashMap},
    fs::File,
    io::{self, Read},
    path::{Path, PathBuf},
};

use crate::runtime_content::{RuntimeContent, RuntimeScript};
use thiserror::Error;
use tiamud_core::{
    AbilityCategory as CoreAbilityCategory,
    AbilityCommunicationDefinition as CoreAbilityCommunicationDefinition,
    AbilityCommunicationKind as CoreAbilityCommunicationKind,
    AbilityCommunicationScope as CoreAbilityCommunicationScope,
    AbilityDamageDefinition as CoreAbilityDamageDefinition,
    AbilityDefinition as CoreAbilityDefinition, AbilityEffectTarget as CoreAbilityEffectTarget,
    AbilityHealDefinition as CoreAbilityHealDefinition,
    AbilityStatusEffectDefinition as CoreAbilityStatusEffectDefinition,
    AbilityTargeting as CoreAbilityTargeting, ActiveEffect,
    AdvancedClassDefinition as CoreAdvancedClassDefinition, AffixDefinition as CoreAffixDefinition,
    AffixPoolDefinition as CoreAffixPoolDefinition, AffixPoolEntry as CoreAffixPoolEntry,
    AffixPoolEntryKind as CoreAffixPoolEntryKind, AffixPosition as CoreAffixPosition,
    BackgroundDefinition as CoreBackgroundDefinition,
    BaseClassDefinition as CoreBaseClassDefinition,
    CharacterDefaultsDefinition as CoreCharacterDefaultsDefinition,
    CharacterProgressionDefinition as CoreCharacterProgressionDefinition,
    DamageType as CoreDamageType, Direction as CoreDirection, EffectKind as CoreEffectKind,
    EquipmentSlot as CoreEquipmentSlot, ExitDefinition, GrantDefinition as CoreGrantDefinition,
    InitialItemInstanceDefinition as CoreInitialItemInstanceDefinition,
    InitialItemLocation as CoreInitialItemLocation,
    ItemContainerDefinition as CoreItemContainerDefinition, ItemInstanceId,
    ItemPrototypeDefinition as CoreItemPrototypeDefinition, ItemRarity as CoreItemRarity,
    ItemStackPolicy as CoreItemStackPolicy, ItemTransferPolicy as CoreItemTransferPolicy,
    LevelGrantDefinition as CoreLevelGrantDefinition,
    LevelThresholdDefinition as CoreLevelThresholdDefinition, LightLevel as CoreLightLevel,
    LineageDefinition as CoreLineageDefinition, LootEntryDefinition as CoreLootEntryDefinition,
    LootEntryKind as CoreLootEntryKind, LootGuaranteedDefinition as CoreLootGuaranteedDefinition,
    LootPoolDefinition as CoreLootPoolDefinition, LootRightsPolicy as CoreLootRightsPolicy,
    LootSourceDefinition as CoreLootSourceDefinition, LootSourceKind as CoreLootSourceKind,
    LootTableDefinition as CoreLootTableDefinition,
    NpcAbilitySetDefinition as CoreNpcAbilitySetDefinition, NpcDefinition,
    RequirementDefinition as CoreRequirementDefinition,
    RequirementDisclosure as CoreRequirementDisclosure,
    RequirementPredicate as CoreRequirementPredicate, RoomDefinition, SceneAudio, SceneImage,
    TalentGraphDefinition as CoreTalentGraphDefinition,
    TalentNodeDefinition as CoreTalentNodeDefinition,
    TalentRankDefinition as CoreTalentRankDefinition, World, WorldBuildError, WorldDefinition,
    WorldError,
};
use world_pack::{
    AudioCue, Direction as PackDirection, EffectKind as PackEffectKind,
    EquipmentSlot as PackEquipmentSlot, ExitTarget, InitialItemLocation as PackInitialItemLocation,
    ItemContainerDefinition as PackItemContainerDefinition, ItemRarity as PackItemRarity,
    ItemStackPolicy as PackItemStackPolicy, ItemTransferPolicy as PackItemTransferPolicy,
    LightLevel as PackLightLevel, RoomMediaDefinition, RoomPerceptionDefinition, ValidationErrors,
    WorldPack, WorldPackDocument,
};

const MAX_WORLD_PACK_BYTES: usize = 16 * 1024 * 1024;
const DEFAULT_WORLD_PACK: &[u8] = include_bytes!("../../../worlds/lantern-square.world.json");

#[cfg(test)]
pub fn default_world_definition() -> Result<WorldDefinition, WorldLoadError> {
    parse_world_definition(DEFAULT_WORLD_PACK)
}

#[derive(Debug)]
pub struct LoadedWorldDefinition {
    pub definition: WorldDefinition,
    pub rng_seed: Option<u64>,
    pub character_progression: CoreCharacterProgressionDefinition,
    pub abilities: Vec<CoreAbilityDefinition>,
    pub npcs: Vec<NpcDefinition>,
    pub npc_ability_sets: Vec<CoreNpcAbilitySetDefinition>,
    pub loot_tables: Vec<CoreLootTableDefinition>,
    pub loot_sources: Vec<CoreLootSourceDefinition>,
    pub affixes: Vec<CoreAffixDefinition>,
    pub affix_pools: Vec<CoreAffixPoolDefinition>,
    pub room_light: Vec<(String, CoreLightLevel)>,
    pub hidden_items: Vec<(String, u16)>,
    pub hidden_npcs: Vec<(String, u16)>,
    pub secret_exits: Vec<(String, CoreDirection, u16)>,
    pub runtime_content: RuntimeContent,
}

#[derive(Debug, Error)]
pub enum WorldInitializationError {
    #[error(transparent)]
    Build(#[from] WorldBuildError),
    #[error(transparent)]
    Configure(#[from] WorldError),
}

/// Applies the complete authored startup contract to the runtime world.
/// Initial instances assigned to an NPC spawn remain staged until that NPC is
/// created, so prototype reuse never makes loot briefly visible in a room.
pub fn build_loaded_world(
    loaded: &LoadedWorldDefinition,
) -> Result<World, WorldInitializationError> {
    let mut world = World::from_definition(loaded.definition.clone())?;
    if let Some(seed) = loaded.rng_seed {
        world.set_rng_seed(seed);
    }
    for (room_id, light) in &loaded.room_light {
        world.configure_room_light(room_id, *light)?;
    }
    for (item_id, difficulty) in &loaded.hidden_items {
        world.configure_hidden_item(item_id, *difficulty)?;
    }
    for (room_id, direction, difficulty) in &loaded.secret_exits {
        world.configure_secret_exit(room_id, *direction, *difficulty)?;
    }
    world.configure_progression(
        loaded.character_progression.clone(),
        loaded.abilities.clone(),
    )?;
    for npc in &loaded.npcs {
        world.spawn_npc(npc.clone())?;
    }
    world.configure_affixes(loaded.affixes.clone(), loaded.affix_pools.clone())?;
    world.configure_loot(loaded.loot_tables.clone(), loaded.loot_sources.clone())?;
    for npc_ability_set in &loaded.npc_ability_sets {
        world.configure_npc_abilities(npc_ability_set.clone())?;
    }
    for (npc_id, difficulty) in &loaded.hidden_npcs {
        world.configure_hidden_npc(npc_id, *difficulty)?;
    }
    Ok(world)
}

pub fn default_loaded_world() -> Result<LoadedWorldDefinition, WorldLoadError> {
    parse_loaded_world(DEFAULT_WORLD_PACK)
}

pub fn load_loaded_world(path: impl AsRef<Path>) -> Result<LoadedWorldDefinition, WorldLoadError> {
    let bytes = read_world_pack(path.as_ref())?;
    parse_loaded_world(&bytes)
}

/// Load and validate an authored WorldPack before it reaches the world actor.
#[cfg(test)]
pub fn load_world_definition(path: impl AsRef<Path>) -> Result<WorldDefinition, WorldLoadError> {
    let bytes = read_world_pack(path.as_ref())?;
    parse_world_definition(&bytes)
}

fn read_world_pack(path: &Path) -> Result<Vec<u8>, WorldLoadError> {
    let mut file = File::open(path).map_err(|source| WorldLoadError::Open {
        path: path.to_owned(),
        source,
    })?;
    let mut bytes = Vec::new();
    file.by_ref()
        .take((MAX_WORLD_PACK_BYTES + 1) as u64)
        .read_to_end(&mut bytes)
        .map_err(|source| WorldLoadError::Read {
            path: path.to_owned(),
            source,
        })?;
    Ok(bytes)
}

/// Parse a WorldPack from memory using the same limits and runtime checks as
/// [`load_world_definition`]. This is also useful to embedders and tests.
#[cfg(test)]
pub fn parse_world_definition(bytes: &[u8]) -> Result<WorldDefinition, WorldLoadError> {
    Ok(parse_loaded_world(bytes)?.definition)
}

pub fn parse_loaded_world(bytes: &[u8]) -> Result<LoadedWorldDefinition, WorldLoadError> {
    if bytes.len() > MAX_WORLD_PACK_BYTES {
        return Err(WorldLoadError::TooLarge {
            size: bytes.len(),
            max: MAX_WORLD_PACK_BYTES,
        });
    }

    let document = WorldPackDocument::from_json(bytes)?;
    document.validate()?;
    convert_document(document)
}

fn convert_document(
    mut document: WorldPackDocument,
) -> Result<LoadedWorldDefinition, WorldLoadError> {
    let seed = document.world.gameplay.rng_seed.unwrap_or(0x5449_414d_5544);
    for variants in &document.scene_variants {
        for variant in &variants.variants {
            if let Some(path) = &variant.asset_manifest
                && !runtime_asset_manifest_path(path)
            {
                return Err(WorldLoadError::UnsupportedAssetManifestPath {
                    room_id: variants.room_id.clone(),
                    path: path.clone(),
                });
            }
        }
    }
    let mut scripts = Vec::with_capacity(document.scripts.len());
    for definition in std::mem::take(&mut document.scripts) {
        let script_id = definition.id.clone();
        let script = RuntimeScript::from_definition(definition);
        tiamud_scripting::validate_module(&script.source, &script.hooks, script.limits)
            .map_err(|source| WorldLoadError::Script { script_id, source })?;
        scripts.push(script);
    }
    let runtime_content =
        RuntimeContent::new(std::mem::take(&mut document.scene_variants), scripts, seed)
            .with_entity_media(std::mem::take(&mut document.entity_media));
    let mut loaded = convert_pack(
        document.world,
        std::mem::take(&mut document.room_media),
        std::mem::take(&mut document.room_perception),
    )?;
    loaded.runtime_content = runtime_content;
    Ok(loaded)
}

fn convert_pack(
    mut pack: WorldPack,
    room_media: Vec<RoomMediaDefinition>,
    room_perception: Vec<RoomPerceptionDefinition>,
) -> Result<LoadedWorldDefinition, WorldLoadError> {
    let world_pack::GameplayDefinition {
        rng_seed,
        npc_prototypes,
        npc_spawns,
        character_progression,
        abilities: pack_abilities,
        npc_ability_sets: pack_npc_ability_sets,
        loot_tables: pack_loot_tables,
        loot_sources: pack_loot_sources,
        affixes: pack_affixes,
        affix_pools: pack_affix_pools,
        ..
    } = std::mem::take(&mut pack.gameplay);
    let start_room_id = pack.start_room_id.clone();
    let item_prototypes = std::mem::take(&mut pack.item_prototypes)
        .into_iter()
        .map(|item| CoreItemPrototypeDefinition {
            prototype_id: item.id,
            name: item.name,
            description: item.description,
            aliases: item.aliases,
            equipment_slot: item.equipment_slot.map(map_equipment_slot),
            attack_bonus: item.attack_bonus,
            heal_amount: item.heal_amount,
            consumable: item.consumable,
            effect: item.effect.map(|effect| ActiveEffect {
                kind: map_effect_kind(effect.kind),
                magnitude: effect.magnitude,
                remaining_ticks: effect.remaining_ticks,
            }),
            weight_grams: item.weight_grams,
            slot_cost: item.slot_cost,
            rarity: map_item_rarity(item.rarity),
            item_level: item.item_level,
            stack_policy: map_item_stack_policy(item.stack_policy),
            max_stack_units: item.max_stack_units,
            container: item.container.map(map_item_container),
            transfer_policy: map_item_transfer_policy(item.transfer_policy),
            sealed_loot_table_id: item.sealed_loot_table_id,
        })
        .collect::<Vec<_>>();
    let initial_item_instances = std::mem::take(&mut pack.initial_item_instances)
        .into_iter()
        .map(|instance| CoreInitialItemInstanceDefinition {
            instance_id: ItemInstanceId(instance.id),
            prototype_id: instance.prototype_id,
            location: match instance.location {
                PackInitialItemLocation::Room { room_id } => CoreInitialItemLocation::Room(room_id),
                PackInitialItemLocation::NpcSpawn { npc_spawn_id } => {
                    CoreInitialItemLocation::NpcSpawn(npc_spawn_id)
                }
                PackInitialItemLocation::Container {
                    container_instance_id,
                } => CoreInitialItemLocation::Container(ItemInstanceId(container_instance_id)),
                PackInitialItemLocation::Staged => CoreInitialItemLocation::Staged,
            },
        })
        .collect::<Vec<_>>();
    let mut loot_by_npc = HashMap::<String, Vec<ItemInstanceId>>::new();
    for instance in &initial_item_instances {
        if let CoreInitialItemLocation::NpcSpawn(npc_id) = &instance.location {
            loot_by_npc
                .entry(npc_id.clone())
                .or_default()
                .push(instance.instance_id.clone());
        }
    }
    let room_ids_by_vnum = pack
        .rooms
        .iter()
        .filter_map(|room| room.legacy_vnum.map(|vnum| (vnum, room.id.clone())))
        .collect::<HashMap<_, _>>();
    let mut room_media = room_media
        .into_iter()
        .map(|media| (media.room_id.clone(), media))
        .collect::<HashMap<_, _>>();

    let mut rooms = Vec::with_capacity(pack.rooms.len());
    for room in pack.rooms {
        let media = room_media.remove(&room.id);
        let mut exits = Vec::with_capacity(room.exits.len());
        for exit in room.exits {
            let target_room_id = match exit.target {
                ExitTarget::Room { room_id } => room_id,
                ExitTarget::LegacyVnum { vnum } => room_ids_by_vnum
                    .get(&vnum)
                    .cloned()
                    .ok_or_else(|| WorldLoadError::UnresolvedLegacyVnum {
                        room_id: room.id.clone(),
                        direction: exit.direction,
                        vnum,
                    })?,
            };
            exits.push(ExitDefinition {
                direction: map_direction(exit.direction),
                target_room_id,
            });
        }

        let image = match room.asset_manifest {
            Some(path) if runtime_asset_manifest_path(&path) => Some(SceneImage {
                alt: media
                    .as_ref()
                    .and_then(|media| media.image_alt.resolve("en"))
                    .expect("validated image manifests always have localized alt text")
                    .to_owned(),
                localized_alt: media
                    .as_ref()
                    .map(|media| media.image_alt.0.clone())
                    .unwrap_or_default(),
                manifest: Some(format!("/{path}")),
                avif: None,
                webp: None,
                jpeg: None,
                width: None,
                height: None,
            }),
            Some(path) => {
                return Err(WorldLoadError::UnsupportedAssetManifestPath {
                    room_id: room.id,
                    path,
                });
            }
            None => None,
        };
        let audio = media
            .as_ref()
            .and_then(|media| media.audio.as_ref())
            .map(scene_audio);

        rooms.push(RoomDefinition {
            stable_id: room.id,
            title: room.title,
            prose: split_prose(&room.description),
            image,
            audio,
            exits,
            item_ids: Vec::new(),
        });
    }

    let definition = WorldDefinition {
        start_room_id,
        item_prototypes,
        initial_item_instances,
        rooms,
    };
    let character_progression = character_progression
        .map(map_character_progression)
        .ok_or(WorldLoadError::MissingCharacterProgression)?;
    let abilities = pack_abilities.into_iter().map(map_ability).collect();
    let loot_tables = pack_loot_tables.into_iter().map(map_loot_table).collect();
    let loot_sources = pack_loot_sources.into_iter().map(map_loot_source).collect();
    let affixes = pack_affixes.into_iter().map(map_affix).collect();
    let affix_pools = pack_affix_pools.into_iter().map(map_affix_pool).collect();
    let prototypes = npc_prototypes
        .into_iter()
        .map(|prototype| (prototype.id.clone(), prototype))
        .collect::<HashMap<_, _>>();
    let npc_ability_sets_by_prototype = pack_npc_ability_sets
        .into_iter()
        .map(|definition| (definition.npc_prototype_id.clone(), definition))
        .collect::<HashMap<_, _>>();
    let mut npc_ability_sets = Vec::new();
    let npcs = npc_spawns
        .into_iter()
        .filter_map(|spawn| {
            let prototype = prototypes.get(&spawn.prototype_id)?;
            let loot_item_ids = loot_by_npc.remove(&spawn.id).unwrap_or_default();
            if let Some(definition) = npc_ability_sets_by_prototype.get(&spawn.prototype_id) {
                npc_ability_sets.push(CoreNpcAbilitySetDefinition {
                    npc_id: spawn.id.clone(),
                    resource_pools: definition.resource_pools.clone(),
                    resource_regeneration: definition.resource_regeneration.clone(),
                    resistances: map_resistances(definition.resistances.clone()),
                    ability_ids: definition.ability_ids.clone(),
                });
            }
            Some(NpcDefinition {
                stable_id: spawn.id,
                name: prototype.name.clone(),
                description: prototype.description.clone(),
                room_id: spawn.room_id,
                level: prototype.level,
                max_health: prototype.max_health,
                strength: prototype.strength,
                defence: prototype.defence,
                experience_reward: prototype.experience_reward,
                loot_item_ids,
                respawn_ticks: prototype.respawn_ticks,
            })
        })
        .collect();
    let mut room_light = Vec::with_capacity(room_perception.len());
    let mut hidden_items = Vec::new();
    let mut hidden_npcs = Vec::new();
    let mut secret_exits = Vec::new();
    for perception in room_perception {
        room_light.push((perception.room_id.clone(), map_light(perception.light)));
        hidden_items.extend(perception.hidden_items);
        hidden_npcs.extend(perception.hidden_npcs);
        secret_exits.extend(
            perception
                .secret_exits
                .into_iter()
                .map(|(direction, difficulty)| {
                    (
                        perception.room_id.clone(),
                        map_direction(direction),
                        difficulty,
                    )
                }),
        );
    }
    Ok(LoadedWorldDefinition {
        definition,
        rng_seed,
        character_progression,
        abilities,
        npcs,
        npc_ability_sets,
        loot_tables,
        loot_sources,
        affixes,
        affix_pools,
        room_light,
        hidden_items,
        hidden_npcs,
        secret_exits,
        runtime_content: RuntimeContent::default(),
    })
}

fn map_loot_table(table: world_pack::LootTableDefinition) -> CoreLootTableDefinition {
    let digest_sha256 = table.canonical_sha256();
    CoreLootTableDefinition {
        id: table.id,
        digest_sha256,
        guaranteed: table
            .guaranteed
            .into_iter()
            .map(|entry| CoreLootGuaranteedDefinition {
                id: entry.id,
                item_prototype_id: entry.item_prototype_id,
                quantity: entry.quantity,
                affix_pool_id: entry.affix_pool_id,
            })
            .collect(),
        pools: table
            .pools
            .into_iter()
            .map(|pool| CoreLootPoolDefinition {
                id: pool.id,
                rolls: pool.rolls,
                entries: pool
                    .entries
                    .into_iter()
                    .map(|entry| CoreLootEntryDefinition {
                        kind: match entry.kind {
                            world_pack::LootEntryKind::Item => CoreLootEntryKind::Item,
                            world_pack::LootEntryKind::Nothing => CoreLootEntryKind::Nothing,
                        },
                        id: entry.id,
                        weight: entry.weight,
                        item_prototype_id: entry.item_prototype_id,
                        min_quantity: entry.min_quantity,
                        max_quantity: entry.max_quantity,
                        affix_pool_id: entry.affix_pool_id,
                    })
                    .collect(),
            })
            .collect(),
    }
}

fn map_affix(affix: world_pack::AffixDefinition) -> CoreAffixDefinition {
    CoreAffixDefinition {
        id: affix.id,
        label: affix.label,
        position: match affix.position {
            world_pack::AffixPosition::Prefix => CoreAffixPosition::Prefix,
            world_pack::AffixPosition::Suffix => CoreAffixPosition::Suffix,
        },
        attack_bonus_delta: affix.attack_bonus_delta,
        heal_amount_delta: affix.heal_amount_delta,
    }
}

fn map_affix_pool(pool: world_pack::AffixPoolDefinition) -> CoreAffixPoolDefinition {
    CoreAffixPoolDefinition {
        id: pool.id,
        entries: pool
            .entries
            .into_iter()
            .map(|entry| CoreAffixPoolEntry {
                kind: match entry.kind {
                    world_pack::AffixPoolEntryKind::Affix => CoreAffixPoolEntryKind::Affix,
                    world_pack::AffixPoolEntryKind::Nothing => CoreAffixPoolEntryKind::Nothing,
                },
                id: entry.id,
                weight: entry.weight,
                affix_id: entry.affix_id,
            })
            .collect(),
    }
}

fn map_loot_source(source: world_pack::LootSourceDefinition) -> CoreLootSourceDefinition {
    CoreLootSourceDefinition {
        kind: match source.kind {
            world_pack::LootSourceKind::NpcSpawn => CoreLootSourceKind::NpcSpawn,
        },
        id: source.id,
        npc_spawn_id: source.npc_spawn_id,
        loot_table_id: source.loot_table_id,
        rights: match source.rights {
            world_pack::LootRightsPolicy::CreditedCharacter => {
                CoreLootRightsPolicy::CreditedCharacter
            }
            world_pack::LootRightsPolicy::PartyRoundRobin => CoreLootRightsPolicy::PartyRoundRobin,
        },
    }
}

fn map_character_progression(
    definition: world_pack::CharacterProgressionDefinition,
) -> CoreCharacterProgressionDefinition {
    CoreCharacterProgressionDefinition {
        defaults: CoreCharacterDefaultsDefinition {
            lineage_id: definition.defaults.lineage_id,
            background_id: definition.defaults.background_id,
            base_class_id: definition.defaults.base_class_id,
        },
        level_thresholds: definition
            .level_thresholds
            .into_iter()
            .map(|entry| CoreLevelThresholdDefinition {
                level: entry.level,
                total_experience_required: entry.total_experience_required,
                point_awards: entry.point_awards,
            })
            .collect(),
        lineages: definition
            .lineages
            .into_iter()
            .map(|entry| CoreLineageDefinition {
                id: entry.id,
                name: entry.name,
                description: entry.description,
                ui_order: entry.ui_order,
                creation_grants: map_grants(entry.creation_grants),
                talent_graph_ids: entry.talent_graph_ids,
            })
            .collect(),
        backgrounds: definition
            .backgrounds
            .into_iter()
            .map(|entry| CoreBackgroundDefinition {
                id: entry.id,
                name: entry.name,
                description: entry.description,
                ui_order: entry.ui_order,
                creation_grants: map_grants(entry.creation_grants),
                talent_graph_ids: entry.talent_graph_ids,
            })
            .collect(),
        base_classes: definition
            .base_classes
            .into_iter()
            .map(|entry| CoreBaseClassDefinition {
                id: entry.id,
                name: entry.name,
                description: entry.description,
                ui_order: entry.ui_order,
                creation_grants: map_grants(entry.creation_grants),
                level_grants: entry
                    .level_grants
                    .into_iter()
                    .map(map_level_grant)
                    .collect(),
                talent_graph_ids: entry.talent_graph_ids,
            })
            .collect(),
        advanced_classes: definition
            .advanced_classes
            .into_iter()
            .map(|entry| CoreAdvancedClassDefinition {
                id: entry.id,
                name: entry.name,
                description: entry.description,
                ui_order: entry.ui_order,
                from_class_ids: entry.from_class_ids,
                exclusive_group: entry.exclusive_group,
                requirements: entry
                    .requirements
                    .into_iter()
                    .map(map_requirement)
                    .collect(),
                promotion_grants: map_grants(entry.promotion_grants),
                level_grants: entry
                    .level_grants
                    .into_iter()
                    .map(map_level_grant)
                    .collect(),
                talent_graph_ids: entry.talent_graph_ids,
            })
            .collect(),
        talent_graphs: definition
            .talent_graphs
            .into_iter()
            .map(|graph| CoreTalentGraphDefinition {
                id: graph.id,
                name: graph.name,
                description: graph.description,
                ui_order: graph.ui_order,
                point_pool_id: graph.point_pool_id,
                nodes: graph
                    .nodes
                    .into_iter()
                    .map(|node| CoreTalentNodeDefinition {
                        id: node.id,
                        name: node.name,
                        description: node.description,
                        ui_order: node.ui_order,
                        exclusion_group: node.exclusion_group,
                        ranks: node
                            .ranks
                            .into_iter()
                            .map(|rank| CoreTalentRankDefinition {
                                rank: rank.rank,
                                cost: rank.cost,
                                requirements: rank
                                    .requirements
                                    .into_iter()
                                    .map(map_requirement)
                                    .collect(),
                                grants: map_grants(rank.grants),
                            })
                            .collect(),
                    })
                    .collect(),
            })
            .collect(),
    }
}

fn map_level_grant(definition: world_pack::LevelGrantDefinition) -> CoreLevelGrantDefinition {
    CoreLevelGrantDefinition {
        level: definition.level,
        grants: map_grants(definition.grants),
    }
}

fn map_grants(definitions: Vec<world_pack::GrantDefinition>) -> Vec<CoreGrantDefinition> {
    definitions.into_iter().map(map_grant).collect()
}

fn map_grant(definition: world_pack::GrantDefinition) -> CoreGrantDefinition {
    match definition {
        world_pack::GrantDefinition::Attribute {
            attribute_id,
            amount,
        } => CoreGrantDefinition::Attribute {
            attribute_id,
            amount,
        },
        world_pack::GrantDefinition::Skill { skill_id, amount } => {
            CoreGrantDefinition::Skill { skill_id, amount }
        }
        world_pack::GrantDefinition::ResourceMax {
            resource_id,
            amount,
        } => CoreGrantDefinition::ResourceMax {
            resource_id,
            amount,
        },
        world_pack::GrantDefinition::ResourceRegeneration {
            resource_id,
            amount,
        } => CoreGrantDefinition::ResourceRegeneration {
            resource_id,
            amount,
        },
        world_pack::GrantDefinition::Ability { ability_id } => {
            CoreGrantDefinition::Ability { ability_id }
        }
        world_pack::GrantDefinition::Resistance {
            damage_type,
            percent,
        } => CoreGrantDefinition::Resistance {
            damage_type: map_damage_type(damage_type),
            percent,
        },
        world_pack::GrantDefinition::Points { pool_id, amount } => {
            CoreGrantDefinition::Points { pool_id, amount }
        }
        world_pack::GrantDefinition::Tag { tag_id } => CoreGrantDefinition::Tag { tag_id },
    }
}

fn map_requirement(definition: world_pack::RequirementDefinition) -> CoreRequirementDefinition {
    CoreRequirementDefinition {
        disclosure: match definition.disclosure {
            world_pack::RequirementDisclosure::Public => CoreRequirementDisclosure::Public,
            world_pack::RequirementDisclosure::Hidden => CoreRequirementDisclosure::Hidden,
        },
        hint: definition.hint,
        predicate: map_requirement_predicate(definition.predicate),
    }
}

fn map_requirement_predicate(
    predicate: world_pack::RequirementPredicate,
) -> CoreRequirementPredicate {
    match predicate {
        world_pack::RequirementPredicate::Level { minimum } => {
            CoreRequirementPredicate::Level { minimum }
        }
        world_pack::RequirementPredicate::PermanentAttribute {
            attribute_id,
            minimum,
        } => CoreRequirementPredicate::PermanentAttribute {
            attribute_id,
            minimum,
        },
        world_pack::RequirementPredicate::Skill { skill_id, minimum } => {
            CoreRequirementPredicate::Skill { skill_id, minimum }
        }
        world_pack::RequirementPredicate::CompletedQuest { quest_id } => {
            CoreRequirementPredicate::CompletedQuest { quest_id }
        }
        world_pack::RequirementPredicate::ReceivedSignal { signal_id } => {
            CoreRequirementPredicate::ReceivedSignal { signal_id }
        }
        world_pack::RequirementPredicate::CharacterFlag { flag_id, equals } => {
            CoreRequirementPredicate::CharacterFlag { flag_id, equals }
        }
        world_pack::RequirementPredicate::WorldFlag { flag_id, equals } => {
            CoreRequirementPredicate::WorldFlag { flag_id, equals }
        }
        world_pack::RequirementPredicate::Lineage { lineage_id } => {
            CoreRequirementPredicate::Lineage { lineage_id }
        }
        world_pack::RequirementPredicate::CurrentClass { class_id } => {
            CoreRequirementPredicate::CurrentClass { class_id }
        }
        world_pack::RequirementPredicate::HistoricalClass { class_id } => {
            CoreRequirementPredicate::HistoricalClass { class_id }
        }
        world_pack::RequirementPredicate::NodeRank {
            graph_id,
            node_id,
            minimum_rank,
        } => CoreRequirementPredicate::NodeRank {
            graph_id,
            node_id,
            minimum_rank,
        },
        world_pack::RequirementPredicate::Ability { ability_id } => {
            CoreRequirementPredicate::Ability { ability_id }
        }
        world_pack::RequirementPredicate::All { requirements } => CoreRequirementPredicate::All {
            requirements: requirements.into_iter().map(map_requirement).collect(),
        },
        world_pack::RequirementPredicate::Any { requirements } => CoreRequirementPredicate::Any {
            requirements: requirements.into_iter().map(map_requirement).collect(),
        },
        world_pack::RequirementPredicate::Not { requirement } => CoreRequirementPredicate::Not {
            requirement: Box::new(map_requirement(*requirement)),
        },
    }
}

fn map_ability(definition: world_pack::AbilityDefinition) -> CoreAbilityDefinition {
    CoreAbilityDefinition {
        id: definition.id,
        name: definition.name,
        description: definition.description,
        category: map_ability_category(definition.category),
        resource_costs: definition.resource_costs,
        cooldown_ticks: definition.cooldown_ticks,
        targeting: map_ability_targeting(definition.targeting),
        damage: definition.damage.map(|damage| CoreAbilityDamageDefinition {
            target: map_ability_effect_target(damage.target),
            damage_type: map_damage_type(damage.damage_type),
            base_amount: damage.base_amount,
            scaling_attribute: damage.scaling_attribute,
            scaling_percent: damage.scaling_percent,
        }),
        heal: definition.heal.map(|heal| CoreAbilityHealDefinition {
            target: map_ability_effect_target(heal.target),
            base_amount: heal.base_amount,
            scaling_attribute: heal.scaling_attribute,
            scaling_percent: heal.scaling_percent,
        }),
        status_effects: definition
            .status_effects
            .into_iter()
            .map(map_ability_status_effect)
            .collect(),
        communication: definition.communication.map(|communication| {
            CoreAbilityCommunicationDefinition {
                kind: match communication.kind {
                    world_pack::AbilityCommunicationKind::Telepathy => {
                        CoreAbilityCommunicationKind::Telepathy
                    }
                },
                scope: match communication.scope {
                    world_pack::AbilityCommunicationScope::DirectOnline => {
                        CoreAbilityCommunicationScope::DirectOnline
                    }
                },
                blocked_room_ids: communication
                    .blocked_room_ids
                    .into_iter()
                    .collect::<BTreeSet<_>>(),
            }
        }),
    }
}

fn map_ability_category(category: world_pack::AbilityCategory) -> CoreAbilityCategory {
    match category {
        world_pack::AbilityCategory::Martial => CoreAbilityCategory::Martial,
        world_pack::AbilityCategory::Spell => CoreAbilityCategory::Spell,
        world_pack::AbilityCategory::Utility => CoreAbilityCategory::Utility,
    }
}

fn map_ability_targeting(targeting: world_pack::AbilityTargeting) -> CoreAbilityTargeting {
    match targeting {
        world_pack::AbilityTargeting::SelfOnly => CoreAbilityTargeting::SelfOnly,
        world_pack::AbilityTargeting::SingleEnemy => CoreAbilityTargeting::SingleEnemy,
        world_pack::AbilityTargeting::SingleAlly => CoreAbilityTargeting::SingleAlly,
    }
}

fn map_ability_effect_target(target: world_pack::AbilityEffectTarget) -> CoreAbilityEffectTarget {
    match target {
        world_pack::AbilityEffectTarget::Caster => CoreAbilityEffectTarget::Caster,
        world_pack::AbilityEffectTarget::Target => CoreAbilityEffectTarget::Target,
    }
}

fn map_damage_type(damage_type: world_pack::DamageType) -> CoreDamageType {
    match damage_type {
        world_pack::DamageType::Physical => CoreDamageType::Physical,
        world_pack::DamageType::Arcane => CoreDamageType::Arcane,
        world_pack::DamageType::Fire => CoreDamageType::Fire,
        world_pack::DamageType::Frost => CoreDamageType::Frost,
        world_pack::DamageType::Lightning => CoreDamageType::Lightning,
    }
}

fn map_resistances(
    resistances: BTreeMap<world_pack::DamageType, u16>,
) -> BTreeMap<CoreDamageType, u16> {
    resistances
        .into_iter()
        .map(|(damage_type, percent)| (map_damage_type(damage_type), percent))
        .collect()
}

fn map_ability_status_effect(
    status: world_pack::AbilityStatusEffect,
) -> CoreAbilityStatusEffectDefinition {
    match status {
        world_pack::AbilityStatusEffect::AttributeModifier {
            target,
            attribute,
            magnitude,
            duration_ticks,
        } => CoreAbilityStatusEffectDefinition::AttributeModifier {
            target: map_ability_effect_target(target),
            attribute,
            magnitude,
            duration_ticks,
        },
        world_pack::AbilityStatusEffect::Regeneration {
            target,
            magnitude,
            duration_ticks,
        } => CoreAbilityStatusEffectDefinition::Regeneration {
            target: map_ability_effect_target(target),
            magnitude,
            duration_ticks,
        },
        world_pack::AbilityStatusEffect::DamageOverTime {
            target,
            damage_type,
            magnitude,
            duration_ticks,
        } => CoreAbilityStatusEffectDefinition::DamageOverTime {
            target: map_ability_effect_target(target),
            damage_type: map_damage_type(damage_type),
            magnitude,
            duration_ticks,
        },
        world_pack::AbilityStatusEffect::Stun {
            target,
            duration_ticks,
        } => CoreAbilityStatusEffectDefinition::Stun {
            target: map_ability_effect_target(target),
            duration_ticks,
        },
    }
}

fn map_light(light: PackLightLevel) -> CoreLightLevel {
    match light {
        PackLightLevel::Bright => CoreLightLevel::Bright,
        PackLightLevel::Dim => CoreLightLevel::Dim,
        PackLightLevel::Dark => CoreLightLevel::Dark,
    }
}

fn scene_audio(cue: &AudioCue) -> SceneAudio {
    SceneAudio {
        channel: cue.channel.as_str().to_owned(),
        url: format!("/{}", cue.path),
        looped: cue.looped,
    }
}

fn map_equipment_slot(slot: PackEquipmentSlot) -> CoreEquipmentSlot {
    match slot {
        PackEquipmentSlot::Weapon => CoreEquipmentSlot::Weapon,
        PackEquipmentSlot::Body => CoreEquipmentSlot::Body,
        PackEquipmentSlot::Trinket => CoreEquipmentSlot::Trinket,
    }
}

fn map_effect_kind(kind: PackEffectKind) -> CoreEffectKind {
    match kind {
        PackEffectKind::Regeneration => CoreEffectKind::Regeneration,
        PackEffectKind::Strength => CoreEffectKind::Strength,
    }
}

fn map_item_rarity(rarity: PackItemRarity) -> CoreItemRarity {
    match rarity {
        PackItemRarity::Common => CoreItemRarity::Common,
        PackItemRarity::Uncommon => CoreItemRarity::Uncommon,
        PackItemRarity::Rare => CoreItemRarity::Rare,
        PackItemRarity::Epic => CoreItemRarity::Epic,
        PackItemRarity::Legendary => CoreItemRarity::Legendary,
    }
}

fn map_item_stack_policy(policy: PackItemStackPolicy) -> CoreItemStackPolicy {
    match policy {
        PackItemStackPolicy::Unique => CoreItemStackPolicy::Unique,
        PackItemStackPolicy::Fungible => CoreItemStackPolicy::Fungible,
    }
}

fn map_item_container(container: PackItemContainerDefinition) -> CoreItemContainerDefinition {
    CoreItemContainerDefinition {
        max_slots: container.max_slots,
        max_weight_grams: container.max_weight_grams,
    }
}

fn map_item_transfer_policy(policy: PackItemTransferPolicy) -> CoreItemTransferPolicy {
    match policy {
        PackItemTransferPolicy::Transferable => CoreItemTransferPolicy::Transferable,
        PackItemTransferPolicy::CharacterBound => CoreItemTransferPolicy::CharacterBound,
        PackItemTransferPolicy::NonTransferable => CoreItemTransferPolicy::NonTransferable,
    }
}

fn runtime_asset_manifest_path(path: &str) -> bool {
    let mut components = path.split('/');
    let (Some("assets"), Some(hash), Some("manifest.json"), None) = (
        components.next(),
        components.next(),
        components.next(),
        components.next(),
    ) else {
        return false;
    };
    hash.len() == 64
        && hash
            .bytes()
            .all(|byte| byte.is_ascii_digit() || matches!(byte, b'a'..=b'f'))
}

fn split_prose(description: &str) -> Vec<String> {
    description
        .replace("\r\n", "\n")
        .replace('\r', "\n")
        .split("\n\n")
        .map(str::trim)
        .filter(|paragraph| !paragraph.is_empty())
        .map(str::to_owned)
        .collect()
}

fn map_direction(direction: PackDirection) -> CoreDirection {
    match direction {
        PackDirection::North => CoreDirection::North,
        PackDirection::East => CoreDirection::East,
        PackDirection::South => CoreDirection::South,
        PackDirection::West => CoreDirection::West,
        PackDirection::Up => CoreDirection::Up,
        PackDirection::Down => CoreDirection::Down,
    }
}

#[derive(Debug, Error)]
pub enum WorldLoadError {
    #[error("could not open WorldPack {path:?}: {source}")]
    Open {
        path: PathBuf,
        #[source]
        source: io::Error,
    },
    #[error("could not read WorldPack {path:?}: {source}")]
    Read {
        path: PathBuf,
        #[source]
        source: io::Error,
    },
    #[error("WorldPack contains {size} bytes; the runtime limit is {max} bytes")]
    TooLarge { size: usize, max: usize },
    #[error("WorldPack v3 requires a character progression definition")]
    MissingCharacterProgression,
    #[error("WorldPack is not valid JSON: {0}")]
    Json(#[from] serde_json::Error),
    #[error("WorldPack could not be decoded or normalized: {0}")]
    Document(#[from] world_pack::DocumentToolError),
    #[error(transparent)]
    Validation(#[from] ValidationErrors),
    #[error(
        "room {room_id:?} exit {direction:?} points to legacy VNUM {vnum}, which is not present in this WorldPack"
    )]
    UnresolvedLegacyVnum {
        room_id: String,
        direction: PackDirection,
        vnum: i64,
    },
    #[error(
        "room {room_id:?} asset manifest {path:?} is not a content-addressed assets/<sha256>/manifest.json path"
    )]
    UnsupportedAssetManifestPath { room_id: String, path: String },
    #[error("WorldPack script {script_id:?} is invalid: {source}")]
    Script {
        script_id: String,
        #[source]
        source: tiamud_scripting::ScriptError,
    },
}

#[cfg(test)]
mod tests {
    use super::*;

    fn v3_fixture(bytes: &[u8]) -> Vec<u8> {
        let mut value: serde_json::Value = serde_json::from_slice(bytes).unwrap();
        value["schemaVersion"] = serde_json::json!(3);
        value["gameplay"]["characterProgression"] = serde_json::json!({
            "defaults": {
                "lineageId": "test:mortal",
                "baseClassId": "test:wanderer"
            },
            "levelThresholds": [{
                "level": 2,
                "totalExperienceRequired": 100
            }],
            "lineages": [{
                "id": "test:mortal",
                "name": "Mortal",
                "description": "A test lineage.",
                "uiOrder": 0
            }],
            "baseClasses": [{
                "id": "test:wanderer",
                "name": "Wanderer",
                "description": "A test class.",
                "uiOrder": 0,
                "creationGrants": [{
                    "kind": "attribute",
                    "attributeId": "max_health",
                    "amount": 20
                }]
            }]
        });
        serde_json::to_vec(&value).unwrap()
    }

    #[test]
    fn progression_authoring_and_runtime_caps_match() {
        macro_rules! same_cap {
            ($name:ident) => {
                assert_eq!(world_pack::$name, tiamud_core::$name, stringify!($name));
            };
        }
        same_cap!(MAX_LINEAGES);
        same_cap!(MAX_BACKGROUNDS);
        same_cap!(MAX_BASE_CLASSES);
        same_cap!(MAX_ADVANCED_CLASSES);
        same_cap!(MAX_TALENT_GRAPHS);
        same_cap!(MAX_NODES_PER_GRAPH);
        same_cap!(MAX_RANKS_PER_NODE);
        same_cap!(MAX_GRANTS_PER_ENTRY);
        same_cap!(MAX_REQUIREMENT_DEPTH);
        same_cap!(MAX_REQUIREMENT_NODES);
        same_cap!(MAX_PROGRESSION_POINTS);
        same_cap!(MAX_PROGRESSION_POINT_POOLS);
    }

    #[test]
    fn sealed_reward_authoring_and_runtime_caps_match() {
        assert_eq!(
            usize::try_from(world_pack::MAX_SEALED_REWARD_UNITS).unwrap(),
            tiamud_core::MAX_SEALED_REWARD_UNITS
        );
        assert_eq!(
            world_pack::MAX_AFFIX_DEFINITIONS,
            tiamud_core::MAX_AFFIX_DEFINITIONS
        );
        assert_eq!(world_pack::MAX_AFFIX_POOLS, tiamud_core::MAX_AFFIX_POOLS);
        assert_eq!(
            world_pack::MAX_AFFIX_ENTRIES_PER_POOL,
            tiamud_core::MAX_AFFIX_POOL_ENTRIES
        );
        assert_eq!(
            world_pack::MAX_AFFIX_ENTRIES,
            tiamud_core::MAX_TOTAL_AFFIX_POOL_ENTRIES
        );
        assert_eq!(
            world_pack::MAX_AFFIX_MODIFIER,
            tiamud_core::MAX_AFFIX_MODIFIER
        );
        assert_eq!(
            world_pack::MAX_REWARD_ODDS_ROWS,
            tiamud_core::MAX_REWARD_ODDS_ROWS
        );
        assert_eq!(
            world_pack::MAX_COMMUNICATION_ABILITIES,
            tiamud_core::MAX_COMMUNICATION_ABILITIES
        );
        assert_eq!(
            world_pack::MAX_COMMUNICATION_BLOCKED_ROOMS,
            tiamud_core::MAX_COMMUNICATION_BLOCKED_ROOMS
        );
    }

    #[test]
    fn reviewed_pack_loads_as_a_runtime_definition() {
        let path =
            Path::new(env!("CARGO_MANIFEST_DIR")).join("../../worlds/lantern-square.world.json");
        let definition = load_world_definition(path).unwrap();

        assert_eq!(definition.start_room_id, "demo:lantern-square");
        assert_eq!(definition.rooms.len(), 3);
        let start = definition
            .rooms
            .iter()
            .find(|room| room.stable_id == definition.start_room_id)
            .unwrap();
        assert_eq!(start.prose.len(), 2);
        assert_eq!(start.exits.len(), 2);
        assert_eq!(
            start
                .image
                .as_ref()
                .and_then(|image| image.manifest.as_deref()),
            Some(
                "/assets/8ade88f0df1adb294333e72d3fa2d0863543dc0263ebe3038208f3c9e415924c/manifest.json"
            )
        );
        assert_eq!(
            start.image.as_ref().map(|image| image.alt.as_str()),
            Some(
                "Lantern Square after rain, its dark stones lit by a blue-glass lantern beneath the clock tower."
            )
        );
        assert_eq!(
            start
                .image
                .as_ref()
                .and_then(|image| image.localized_alt.get("es"))
                .map(String::as_str),
            Some(
                "La Plaza del Farol tras la lluvia, con las piedras oscuras iluminadas por un farol de cristal azul bajo la torre del reloj."
            )
        );
    }

    #[test]
    fn embedded_default_pack_loads_without_a_working_directory_dependency() {
        let definition = default_world_definition().unwrap();

        assert_eq!(definition.start_room_id, "demo:lantern-square");
        assert_eq!(definition.rooms.len(), 3);
    }

    #[test]
    fn reviewed_pack_loads_item_prototypes_instances_and_an_npc_spawn() {
        let loaded = default_loaded_world().unwrap();
        assert_eq!(loaded.definition.item_prototypes.len(), 10);
        assert_eq!(loaded.definition.initial_item_instances.len(), 9);
        let tonic = loaded
            .definition
            .item_prototypes
            .iter()
            .find(|prototype| prototype.prototype_id == "tiamud:ember-tonic")
            .unwrap();
        assert!(matches!(
            tonic.rarity,
            CoreItemRarity::Common
                | CoreItemRarity::Uncommon
                | CoreItemRarity::Rare
                | CoreItemRarity::Epic
                | CoreItemRarity::Legendary
        ));
        assert!(tonic.item_level > 0);
        assert!(tonic.weight_grams > 0);
        assert_eq!(tonic.stack_policy, CoreItemStackPolicy::Fungible);
        assert_eq!(tonic.max_stack_units, 20);
        let satchel = loaded
            .definition
            .item_prototypes
            .iter()
            .find(|prototype| prototype.prototype_id == "tiamud:archive-satchel")
            .unwrap();
        assert_eq!(satchel.container.as_ref().unwrap().max_slots, 8);
        let rain_writ = loaded
            .definition
            .item_prototypes
            .iter()
            .find(|prototype| prototype.prototype_id == "tiamud:rain-writ")
            .unwrap();
        assert_eq!(
            rain_writ.transfer_policy,
            CoreItemTransferPolicy::NonTransferable
        );
        let stormglass_talon = loaded
            .definition
            .item_prototypes
            .iter()
            .find(|prototype| prototype.prototype_id == "tiamud:stormglass-talon")
            .unwrap();
        assert_eq!(
            stormglass_talon.transfer_policy,
            CoreItemTransferPolicy::CharacterBound
        );
        let storm_seal = loaded
            .definition
            .item_prototypes
            .iter()
            .find(|prototype| prototype.prototype_id == "tiamud:storm-seal")
            .unwrap();
        assert_eq!(
            storm_seal.sealed_loot_table_id.as_deref(),
            Some("tiamud:storm-oath-rewards")
        );
        assert!(storm_seal.consumable);
        assert!(
            loaded
                .definition
                .initial_item_instances
                .iter()
                .any(|instance| {
                    instance.instance_id == ItemInstanceId("tiamud:weathered-map".into())
                        && instance.location
                            == CoreInitialItemLocation::Container(ItemInstanceId(
                                "tiamud:archive-satchel".into(),
                            ))
                })
        );
        assert_eq!(
            loaded.character_progression.defaults.lineage_id,
            "tiamud:mortal"
        );
        assert_eq!(
            loaded
                .character_progression
                .defaults
                .background_id
                .as_deref(),
            Some("tiamud:caravan-scout")
        );
        assert_eq!(
            loaded.character_progression.defaults.base_class_id,
            "tiamud:wanderer"
        );
        assert_eq!(loaded.character_progression.lineages.len(), 3);
        assert_eq!(loaded.character_progression.base_classes.len(), 3);
        assert_eq!(loaded.character_progression.advanced_classes.len(), 4);
        assert_eq!(loaded.abilities.len(), 4);
        let whisper = loaded
            .abilities
            .iter()
            .find(|ability| ability.id == "tiamud:mind-whisper")
            .unwrap();
        assert_eq!(whisper.resource_costs.get("stamina"), Some(&2));
        assert_eq!(whisper.cooldown_ticks, 2);
        let communication = whisper.communication.as_ref().unwrap();
        assert_eq!(communication.kind, CoreAbilityCommunicationKind::Telepathy);
        assert_eq!(
            communication.scope,
            CoreAbilityCommunicationScope::DirectOnline
        );
        assert_eq!(
            communication.blocked_room_ids,
            BTreeSet::from(["demo:archive".to_owned()])
        );
        assert_eq!(loaded.npcs.len(), 1);
        assert_eq!(loaded.npc_ability_sets.len(), 1);
        assert_eq!(loaded.loot_tables.len(), 2);
        assert_eq!(loaded.loot_sources.len(), 1);
        assert_eq!(loaded.affixes.len(), 1);
        assert_eq!(loaded.affix_pools.len(), 1);
        assert_eq!(loaded.loot_tables[0].id, "tiamud:gate-stalker-drops");
        assert_eq!(loaded.loot_tables[0].digest_sha256.len(), 64);
        assert!(
            loaded.loot_tables[0]
                .digest_sha256
                .bytes()
                .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
        );
        assert_eq!(
            loaded.loot_tables[0].pools[0]
                .entries
                .iter()
                .find(|entry| { entry.item_prototype_id.as_deref() == Some("tiamud:wolf-clasp") })
                .unwrap()
                .affix_pool_id
                .as_deref(),
            Some("tiamud:storm-talon-affixes")
        );
        let reward_table = loaded
            .loot_tables
            .iter()
            .find(|table| table.id == "tiamud:storm-oath-rewards")
            .unwrap();
        assert_eq!(
            reward_table.guaranteed[0].affix_pool_id.as_deref(),
            Some("tiamud:storm-talon-affixes")
        );
        assert_eq!(loaded.affixes[0].id, "tiamud:affix:storm-bitten");
        assert_eq!(loaded.affixes[0].position, CoreAffixPosition::Prefix);
        assert_eq!(loaded.affixes[0].attack_bonus_delta, 1);
        assert_eq!(loaded.affix_pools[0].id, "tiamud:storm-talon-affixes");
        assert_eq!(loaded.affix_pools[0].entries.len(), 2);
        assert_eq!(loaded.loot_sources[0].npc_spawn_id, "tiamud:gate-stalker-1");
        assert_eq!(
            loaded.loot_sources[0].loot_table_id,
            "tiamud:gate-stalker-drops"
        );
        assert_eq!(
            loaded.loot_sources[0].rights,
            CoreLootRightsPolicy::PartyRoundRobin
        );
        assert_eq!(loaded.npcs[0].stable_id, "tiamud:gate-stalker-1");
        assert_eq!(
            loaded.npcs[0].loot_item_ids,
            [ItemInstanceId("tiamud:stormglass-talon".into())]
        );
        assert_eq!(loaded.rng_seed, Some(6_073_466_303_471_576_068));
        assert!(
            loaded
                .room_light
                .contains(&("demo:north-gate".into(), CoreLightLevel::Bright))
        );
        assert_eq!(loaded.hidden_items, [("tiamud:road-token".into(), 8)]);
        assert_eq!(loaded.hidden_npcs, [("tiamud:gate-stalker-1".into(), 8)]);
        assert_eq!(
            loaded.secret_exits,
            [("demo:lantern-square".into(), CoreDirection::Down, 8)]
        );
    }

    #[test]
    fn reviewed_pack_builds_the_creation_catalog_and_composed_initial_gameplay() {
        let loaded = default_loaded_world().unwrap();
        let world = build_loaded_world(&loaded).unwrap();

        let catalog = world.progression_catalog();
        assert_eq!(catalog.default_lineage_id, "tiamud:mortal");
        assert_eq!(
            catalog.default_background_id.as_deref(),
            Some("tiamud:caravan-scout")
        );
        assert_eq!(catalog.default_base_class_id, "tiamud:wanderer");
        assert_eq!(catalog.lineages.len(), 3);
        assert_eq!(catalog.backgrounds.len(), 3);
        assert_eq!(catalog.base_classes.len(), 3);
        assert!(
            catalog
                .base_classes
                .iter()
                .any(|class| { class.id == "tiamud:arcanist" && class.label == "Arcanist" })
        );

        let initial = world
            .initial_gameplay(
                Some("tiamud:mortal"),
                tiamud_core::BackgroundSelection::Id("tiamud:caravan-scout"),
                Some("tiamud:arcanist"),
            )
            .unwrap();
        assert_eq!(initial.lineage_id, "tiamud:mortal");
        assert_eq!(
            initial.background_id.as_deref(),
            Some("tiamud:caravan-scout")
        );
        assert_eq!(initial.base_class_id, "tiamud:arcanist");
        assert_eq!(initial.active_class_id, "tiamud:arcanist");
        assert_eq!(initial.vitals.health, 16);
        assert_eq!(initial.attributes.additional.get("intellect"), Some(&8));
        assert_eq!(initial.skills.get("perception"), Some(&12));
        assert_eq!(
            initial.resources.get("mana"),
            Some(&tiamud_core::ResourcePool {
                current: 14,
                max: 14,
            })
        );
        assert_eq!(initial.ability_ids, ["tiamud:basic-strike"]);
        assert_eq!(initial.resistances.get(&CoreDamageType::Arcane), Some(&5));

        let wanderer = world
            .initial_gameplay(
                Some("tiamud:mortal"),
                tiamud_core::BackgroundSelection::Id("tiamud:caravan-scout"),
                Some("tiamud:wanderer"),
            )
            .unwrap();
        assert_eq!(
            wanderer.ability_ids,
            ["tiamud:basic-strike", "tiamud:mind-whisper"]
        );
        assert_eq!(
            wanderer.resources.get("stamina"),
            Some(&tiamud_core::ResourcePool { current: 8, max: 8 })
        );
    }

    #[test]
    fn reviewed_pack_configures_and_executes_the_npc_ability_set() {
        let loaded = default_loaded_world().unwrap();
        let mut world = build_loaded_world(&loaded).unwrap();

        let snapshot = world.runtime_snapshot();
        let npc = snapshot.npcs.get("tiamud:gate-stalker-1").unwrap();
        assert_eq!(
            npc.resources.get("stamina"),
            Some(&tiamud_core::ResourcePool { current: 6, max: 6 })
        );
        assert_eq!(npc.resource_regeneration.get("stamina"), Some(&1));
        assert_eq!(
            npc.ability_ids,
            ["tiamud:basic-strike", "tiamud:lunging-strike"]
        );
        assert_eq!(npc.resistances.get(&CoreDamageType::Physical), Some(&5));

        let session = tiamud_core::SessionId(91);
        let character_id = tiamud_core::CharacterId(42);
        let gameplay = world
            .initial_gameplay(
                None,
                tiamud_core::BackgroundSelection::Default,
                Some("tiamud:arcanist"),
            )
            .unwrap();
        world
            .restore_character_gameplay(character_id, gameplay)
            .unwrap();
        world
            .connect(
                session,
                tiamud_core::CharacterSnapshot {
                    id: character_id,
                    account_id: tiamud_core::AccountId(7),
                    name: "Mara".into(),
                    room_id: "demo:north-gate".into(),
                    item_locations: BTreeMap::new(),
                },
            )
            .unwrap();
        world
            .apply(session, tiamud_core::PlayerAction::Attack("gate".into()))
            .unwrap();

        let events = world.advance_tick().unwrap();
        assert!(events.iter().any(|event| matches!(
            event,
            tiamud_core::DomainEvent::Gameplay {
                event: tiamud_core::GameplayEvent::NpcAbilityUsed {
                    npc_id,
                    ability_id,
                    ..
                }
            } if npc_id == "tiamud:gate-stalker-1" && ability_id == "tiamud:lunging-strike"
        )));
        let npc = &world.runtime_snapshot().npcs["tiamud:gate-stalker-1"];
        assert_eq!(npc.resources["stamina"].current, 3);
        assert!(npc.ability_cooldowns.contains_key("tiamud:lunging-strike"));
    }

    #[test]
    fn reviewed_pack_stages_fixed_loot_and_configures_repeatable_loot() {
        let path =
            Path::new(env!("CARGO_MANIFEST_DIR")).join("../../worlds/lantern-square.world.json");
        let loaded = load_loaded_world(path).unwrap();
        assert!(
            loaded
                .definition
                .rooms
                .iter()
                .all(|room| !room.item_ids.iter().any(|item| item == "tiamud:wolf-clasp"))
        );

        let world = build_loaded_world(&loaded).unwrap();
        let snapshot = world.runtime_snapshot();
        assert!(snapshot.npcs.contains_key("tiamud:gate-stalker-1"));
        assert_eq!(
            snapshot
                .item_instances
                .get(&ItemInstanceId("tiamud:stormglass-talon".into()))
                .map(|instance| &instance.location),
            Some(&tiamud_core::RuntimeItemLocation::HeldByNpc {
                npc_id: "tiamud:gate-stalker-1".into(),
            })
        );
        assert!(
            !snapshot
                .item_instances
                .contains_key(&ItemInstanceId("tiamud:wolf-clasp".into())),
            "repeatable loot must not exist before the NPC is defeated"
        );
    }

    #[test]
    fn empty_world_is_rejected_for_runtime() {
        let error = parse_world_definition(&v3_fixture(
            br#"{"schemaVersion":2,"id":"empty","name":"Empty","startRoomId":"missing","rooms":[]}"#,
        ))
        .err()
        .unwrap();

        assert!(matches!(error, WorldLoadError::Validation(_)));
    }

    #[test]
    fn unresolved_external_legacy_vnum_is_rejected_for_runtime() {
        let error = parse_world_definition(&v3_fixture(
            br#"{
                "schemaVersion": 2,
                "id": "legacy",
                "name": "Legacy",
                "startRoomId": "legacy:1",
                "rooms": [{
                    "id": "legacy:1",
                    "legacyVnum": 1,
                    "title": "Room",
                    "description": "Description",
                    "exits": [{
                        "direction": "north",
                        "target": {"kind": "legacy_vnum", "vnum": 999}
                    }]
                }]
            }"#,
        ))
        .err()
        .unwrap();

        assert!(matches!(
            error,
            WorldLoadError::UnresolvedLegacyVnum { vnum: 999, .. }
        ));
    }

    #[test]
    fn internal_legacy_vnum_is_resolved_to_a_stable_room_id() {
        let definition = parse_world_definition(&v3_fixture(
            br#"{
                "schemaVersion": 2,
                "id": "legacy",
                "name": "Legacy",
                "startRoomId": "legacy:1",
                "rooms": [
                    {
                        "id": "legacy:1",
                        "legacyVnum": 1,
                        "title": "One",
                        "description": "First",
                        "exits": [{
                            "direction": "north",
                            "target": {"kind": "legacy_vnum", "vnum": 2}
                        }]
                    },
                    {
                        "id": "legacy:2",
                        "legacyVnum": 2,
                        "title": "Two",
                        "description": "Second"
                    }
                ]
            }"#,
        ))
        .unwrap();

        assert_eq!(definition.rooms[0].exits[0].target_room_id, "legacy:2");
    }

    #[test]
    fn structural_validation_errors_are_preserved() {
        let error = parse_world_definition(&v3_fixture(
            br#"{
                "schemaVersion": 2,
                "id": "broken",
                "name": "Broken",
                "startRoomId": "broken:1",
                "rooms": [{
                    "id": "broken:1",
                    "title": "Room",
                    "description": "Description",
                    "exits": [{
                        "direction": "north",
                        "target": {"kind": "room", "room_id": "broken:missing"}
                    }]
                }]
            }"#,
        ))
        .err()
        .unwrap();

        assert!(matches!(error, WorldLoadError::Validation(_)));
    }

    #[test]
    fn oversized_input_is_rejected_before_json_parsing() {
        let bytes = vec![b' '; MAX_WORLD_PACK_BYTES + 1];
        let error = parse_world_definition(&bytes).err().unwrap();

        assert!(matches!(error, WorldLoadError::TooLarge { .. }));
    }

    #[test]
    fn runtime_requires_a_content_addressed_asset_manifest() {
        let error = parse_world_definition(&v3_fixture(
            br#"{
                "schemaVersion": 2,
                "id": "unsafe-media",
                "name": "Unsafe media",
                "startRoomId": "unsafe-media:1",
                "rooms": [{
                    "id": "unsafe-media:1",
                    "title": "Room",
                    "description": "Description",
                    "assetManifest": "assets/latest/manifest.json"
                }],
                "roomMedia": [{
                    "roomId": "unsafe-media:1",
                    "imageAlt": {"en": "A safe description."}
                }]
            }"#,
        ))
        .unwrap_err();

        assert!(matches!(
            error,
            WorldLoadError::UnsupportedAssetManifestPath { .. }
        ));
    }

    #[test]
    fn localized_alt_and_content_addressed_audio_reach_the_runtime_scene() {
        let digest = "a".repeat(64);
        let json = format!(
            r#"{{
                "schemaVersion": 2,
                "id": "media",
                "name": "Media",
                "startRoomId": "media:1",
                "rooms": [{{
                    "id": "media:1",
                    "title": "Rain Hall",
                    "description": "Rain sounds beyond the roof.",
                    "assetManifest": "assets/{digest}/manifest.json"
                }}],
                "roomMedia": [{{
                    "roomId": "media:1",
                    "imageAlt": {{
                        "es": "Un salón bajo la lluvia.",
                        "en": "A hall beneath the rain."
                    }},
                    "audio": {{
                        "channel": "ambience",
                        "path": "assets/{digest}/audio-rain.opus",
                        "loop": true
                    }}
                }}]
            }}"#
        );
        let definition = parse_world_definition(&v3_fixture(json.as_bytes())).unwrap();
        let room = &definition.rooms[0];
        assert_eq!(
            room.image.as_ref().map(|image| image.alt.as_str()),
            Some("A hall beneath the rain.")
        );
        assert_eq!(
            room.audio.as_ref(),
            Some(&SceneAudio {
                channel: "ambience".into(),
                url: format!("/assets/{digest}/audio-rain.opus"),
                looped: true,
            })
        );
    }

    #[test]
    fn prose_normalises_windows_and_classic_mac_line_endings() {
        assert_eq!(
            split_prose("First.\r\n\r\nSecond.\r\rThird."),
            ["First.", "Second.", "Third."]
        );
    }
}
