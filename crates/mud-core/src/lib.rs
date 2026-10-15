//! Transport-independent world model for Tiamud.

mod ability;
mod actions;
mod adventure;
mod areas;
mod character;
mod definitions;
mod emote;
mod events;
mod gameplay;
mod items;
mod loot;
pub mod loot_distribution;
mod model;
mod parse;
mod perception;
mod persistence;
mod progression;
mod projection;
mod replay;
mod runtime;
mod sessions;
mod snapshot;
mod tick;
mod trade;
mod validation;
mod views;
mod world;

pub use ability::*;
pub use actions::*;
use adventure::*;
pub use areas::*;
pub use definitions::*;
pub use emote::*;
pub use events::*;
use gameplay::*;
use items::*;
pub use loot::*;
use model::*;
pub use parse::*;
use perception::*;
pub use progression::*;
use projection::*;
pub use runtime::*;
pub use snapshot::*;
pub use trade::*;
pub use validation::*;
pub use views::*;
pub use world::*;

use rand_core::{OsRng, RngCore};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use slotmap::{SlotMap, new_key_type};
use std::borrow::Borrow;
use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet, VecDeque};
use std::fmt::Write as _;
use thiserror::Error;

/// `MAX_STAT_VALUE` (the stat cap) expressed once as `i32`. The cap sits far
/// below `i32::MAX`, so the conversion is lossless; the const assertion below
/// enforces that invariant at compile time.
const MAX_STAT_VALUE_I32: i32 = MAX_STAT_VALUE as i32;
const _: () = assert!(MAX_STAT_VALUE <= i32::MAX as u32);

/// Zero-based page index for a one-based `page` selector. Callers that must
/// reject `page == 0` still do so explicitly before calling this; the
/// `saturating_sub` only keeps the index computation itself underflow-free.
fn page_index(page: u16) -> usize {
    usize::from(page.saturating_sub(1))
}

new_key_type! {
    /// Internal, generational entity identifier. Never exposed as a persistent ID.
    struct EntityId;
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct SessionId(pub u64);

/// Durable database identity for an account.
///
/// Unlike [`SessionId`], this value survives reconnects and server restarts.
#[derive(Clone, Copy, Debug, Deserialize, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(transparent)]
pub struct AccountId(pub i64);

/// Durable database identity for a character.
///
/// Character names are presentation data and are not used as identity.
#[derive(Clone, Copy, Debug, Deserialize, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(transparent)]
pub struct CharacterId(pub i64);

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct AccountSnapshot {
    pub id: AccountId,
    pub name: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn item_prototype(
        id: &str,
        name: &str,
        description: &str,
        aliases: &[&str],
    ) -> ItemPrototypeDefinition {
        ItemPrototypeDefinition {
            prototype_id: id.into(),
            name: name.into(),
            description: description.into(),
            aliases: aliases.iter().map(|alias| (*alias).into()).collect(),
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
        }
    }

    fn item_instance(
        id: &str,
        prototype_id: &str,
        location: InitialItemLocation,
    ) -> InitialItemInstanceDefinition {
        InitialItemInstanceDefinition {
            instance_id: id.into(),
            prototype_id: prototype_id.into(),
            location,
        }
    }

    fn area_generation() -> AreaGenerationId {
        AreaGenerationId::from_catalog_sha256("1".repeat(64)).unwrap()
    }

    fn prepared_room(
        stable_id: &str,
        exits: impl IntoIterator<Item = (Direction, &'static str)>,
    ) -> RoomDefinition {
        RoomDefinition {
            stable_id: stable_id.into(),
            title: stable_id.into(),
            prose: vec![format!("Area room {stable_id}.")],
            image: None,
            audio: None,
            exits: exits
                .into_iter()
                .map(|(direction, target_room_id)| ExitDefinition {
                    direction,
                    target_room_id: target_room_id.into(),
                })
                .collect(),
            item_ids: Vec::new(),
        }
    }

    fn prepared_area(area_id: &str, revision: u64, rooms: Vec<RoomDefinition>) -> PreparedCoreArea {
        PreparedCoreArea {
            area_id: area_id.into(),
            revision,
            bundle_sha256: format!("{revision:064x}"),
            rooms,
            room_presentations: Vec::new(),
            item_prototypes: Vec::new(),
            initial_item_instances: Vec::new(),
            npcs: Vec::new(),
            npc_ability_sets: Vec::new(),
            loot_tables: Vec::new(),
            loot_sources: Vec::new(),
            affixes: Vec::new(),
            affix_pools: Vec::new(),
            room_light: Vec::new(),
            hidden_items: Vec::new(),
            hidden_npcs: Vec::new(),
            secret_exits: Vec::new(),
        }
    }

    fn deferred_portal(
        generation_id: &AreaGenerationId,
        source_room_id: &str,
        direction: Direction,
        target_room_id: &str,
        target_area_id: &str,
        minimum_target_revision: u64,
    ) -> AreaLoadRequest {
        AreaLoadRequest {
            generation_id: generation_id.clone(),
            source_room_id: source_room_id.into(),
            direction,
            target_room_id: target_room_id.into(),
            target_area_id: target_area_id.into(),
            minimum_target_revision,
        }
    }

    fn retirement_fixture() -> (World, PreparedCoreAreaBatch, AreaRetirementUnit, SessionId) {
        let generation = area_generation();
        let inbound = deferred_portal(
            &generation,
            "area-a:start",
            Direction::North,
            "area-b:start",
            "area-b",
            1,
        );
        let outbound = deferred_portal(
            &generation,
            "area-b:start",
            Direction::South,
            "area-a:start",
            "area-a",
            1,
        );
        let area_a = prepared_area(
            "area-a",
            1,
            vec![prepared_room(
                "area-a:start",
                [(Direction::North, "area-b:start")],
            )],
        );
        let mut area_b = prepared_area(
            "area-b",
            1,
            vec![prepared_room(
                "area-b:start",
                [(Direction::South, "area-a:start")],
            )],
        );
        let carried = item_prototype(
            "area-b:carried",
            "travelling token",
            "A token that may leave its authored area.",
            &["token"],
        );
        let mut chest = item_prototype(
            "area-b:chest",
            "field chest",
            "A ground-loot container.",
            &["chest"],
        );
        chest.container = Some(ItemContainerDefinition {
            max_slots: 8,
            max_weight_grams: 20_000,
        });
        let child = item_prototype(
            "area-b:child",
            "chest shard",
            "An item contained by the chest.",
            &["shard"],
        );
        let npc_token = item_prototype(
            "area-b:npc-token",
            "keeper token",
            "An authored NPC possession.",
            &["keeper-token"],
        );
        area_b.item_prototypes = vec![carried, chest, child, npc_token];
        area_b.initial_item_instances = vec![
            item_instance(
                "area-b:carried-instance",
                "area-b:carried",
                InitialItemLocation::Room("area-b:start".into()),
            ),
            item_instance(
                "area-b:chest-instance",
                "area-b:chest",
                InitialItemLocation::Room("area-b:start".into()),
            ),
            item_instance(
                "area-b:child-instance",
                "area-b:child",
                InitialItemLocation::Container(ItemInstanceId("area-b:chest-instance".into())),
            ),
            item_instance(
                "area-b:npc-token-instance",
                "area-b:npc-token",
                InitialItemLocation::NpcSpawn("area-b:keeper".into()),
            ),
        ];
        area_b.npcs.push(NpcDefinition {
            stable_id: "area-b:keeper".into(),
            name: "keeper".into(),
            description: "A quiet keeper used by retirement tests.".into(),
            room_id: "area-b:start".into(),
            level: 1,
            max_health: 10,
            strength: 1,
            defence: 0,
            experience_reward: 1,
            loot_item_ids: Vec::new(),
            respawn_ticks: 2,
        });
        let full_batch = PreparedCoreAreaBatch {
            generation_id: generation.clone(),
            areas: vec![area_a, area_b.clone()],
            deferred_exits: vec![inbound.clone(), outbound.clone()],
        };
        let (mut world, _) =
            World::from_initial_prepared_areas("area-a:start", full_batch).unwrap();
        let session = SessionId(612);
        world
            .connect(
                session,
                CharacterSnapshot {
                    id: CharacterId(612),
                    account_id: AccountId(612),
                    name: "Retirement Tester".into(),
                    room_id: "area-a:start".into(),
                    item_locations: BTreeMap::new(),
                },
            )
            .unwrap();
        world
            .set_item_location(
                &ItemInstanceId("area-b:carried-instance".into()),
                SharedItemLocation::Carried(CharacterId(612)),
            )
            .unwrap();
        world.next_loot_drop_id = LootDropId(2);
        world.ground_loot.insert(
            ItemInstanceId("area-b:chest-instance".into()),
            GroundLootReservation {
                drop_id: LootDropId(1),
                root_item_id: ItemInstanceId("area-b:chest-instance".into()),
                room_id: "area-b:start".into(),
                cohort: vec![CharacterId(612)],
                beneficiary: Some(CharacterId(612)),
                passed: BTreeSet::new(),
                created_tick: 0,
                claim_until_tick: LOOT_EXCLUSIVE_TICKS,
                despawn_at_tick: LOOT_DESPAWN_TICKS,
            },
        );
        let rehydrate_batch = PreparedCoreAreaBatch {
            generation_id: generation.clone(),
            areas: vec![area_b],
            deferred_exits: vec![inbound.clone(), outbound],
        };
        let ownership = AreaRuntimeOwnership {
            area_id: "area-b".into(),
            revision: 1,
            bundle_sha256: format!("{:064x}", 1),
            rooms: BTreeSet::from(["area-b:start".into()]),
            initial_item_instances: BTreeSet::from([
                ItemInstanceId("area-b:carried-instance".into()),
                ItemInstanceId("area-b:chest-instance".into()),
                ItemInstanceId("area-b:child-instance".into()),
                ItemInstanceId("area-b:npc-token-instance".into()),
            ]),
            npc_spawns: BTreeSet::from(["area-b:keeper".into()]),
        };
        let unit = AreaRetirementUnit {
            generation_id: generation,
            areas: vec![ownership],
            boundary_exits: vec![inbound],
        };
        (world, rehydrate_batch, unit, session)
    }

    fn sealed_reward_definition() -> WorldDefinition {
        let mut seal = item_prototype(
            "reward:sealed-cache",
            "sealed cache",
            "A reward earned in combat and sealed against rerolls.",
            &["cache", "seal"],
        );
        seal.consumable = true;
        seal.transfer_policy = ItemTransferPolicy::CharacterBound;
        seal.sealed_loot_table_id = Some("reward:cache-table".into());
        let mut sword = item_prototype(
            "reward:sword",
            "iron sword",
            "A deterministic reward blade.",
            &["sword", "blade"],
        );
        sword.equipment_slot = Some(EquipmentSlot::Weapon);
        sword.attack_bonus = 1;
        sword.slot_cost = 2;
        WorldDefinition {
            start_room_id: "test:start".into(),
            item_prototypes: vec![seal, sword],
            initial_item_instances: Vec::new(),
            rooms: vec![RoomDefinition {
                stable_id: "test:start".into(),
                title: "Reward Test".into(),
                prose: vec!["A bounded reward test room.".into()],
                image: None,
                audio: None,
                item_ids: Vec::new(),
                exits: Vec::new(),
            }],
        }
    }

    fn sealed_reward_world(seed: u64) -> (World, SessionId, ItemInstanceId) {
        let (mut world, session) = connect_definition(sealed_reward_definition());
        world.set_rng_seed(seed);
        world
            .configure_affixes(
                vec![AffixDefinition {
                    id: "reward:keen".into(),
                    label: "afilada".into(),
                    position: AffixPosition::Prefix,
                    attack_bonus_delta: 2,
                    heal_amount_delta: 0,
                }],
                vec![AffixPoolDefinition {
                    id: "reward:weapon-affix".into(),
                    entries: vec![AffixPoolEntry {
                        kind: AffixPoolEntryKind::Affix,
                        id: "reward:keen-outcome".into(),
                        weight: 3,
                        affix_id: Some("reward:keen".into()),
                    }],
                }],
            )
            .unwrap();
        world
            .configure_loot(
                vec![LootTableDefinition {
                    id: "reward:cache-table".into(),
                    digest_sha256: "d".repeat(64),
                    guaranteed: vec![LootGuaranteedDefinition {
                        id: "reward:sword-outcome".into(),
                        item_prototype_id: "reward:sword".into(),
                        quantity: 1,
                        affix_pool_id: Some("reward:weapon-affix".into()),
                    }],
                    pools: vec![LootPoolDefinition {
                        id: "reward:empty-draw".into(),
                        rolls: 2,
                        entries: vec![LootEntryDefinition {
                            kind: LootEntryKind::Nothing,
                            id: "reward:nothing".into(),
                            weight: 2,
                            item_prototype_id: None,
                            min_quantity: None,
                            max_quantity: None,
                            affix_pool_id: None,
                        }],
                    }],
                }],
                Vec::new(),
            )
            .unwrap();
        let source = world
            .create_staged_item_instance("reward:sealed-cache")
            .unwrap();
        let instance = world.item_instances.get_mut(&source).unwrap();
        instance.location = SharedItemLocation::Carried(CharacterId(701));
        instance.binding = ItemBinding::Character {
            character_id: CharacterId(701),
        };
        world.item_provenance.insert(
            source.clone(),
            ItemProvenance::NpcLoot {
                drop_id: LootDropId(1),
                npc_spawn_id: "reward:earned-source".into(),
                loot_table_id: "reward:earned-table".into(),
                table_digest_sha256: "e".repeat(64),
                entry_id: "reward:earned-cache".into(),
                pool_id: Some("reward:earned-pool".into()),
                sealed_loot_table_id: Some("reward:cache-table".into()),
                roll_index: 0,
                quantity_index: 0,
                generated_tick: 0,
            },
        );
        world.next_loot_drop_id = LootDropId(2);
        (world, session, source)
    }

    fn redact_and_validate_opaque_item_refs(mut text: &str) -> String {
        const PREFIX: &str = "; ref o";
        let mut redacted = String::with_capacity(text.len());
        while let Some(index) = text.find(PREFIX) {
            redacted.push_str(&text[..index]);
            let suffix = &text[index + PREFIX.len()..];
            assert!(suffix.len() >= 16, "truncated opaque reference in {text:?}");
            let token = &suffix[..16];
            assert!(
                token.bytes().all(|byte| byte.is_ascii_hexdigit()),
                "invalid opaque reference in {text:?}"
            );
            text = &suffix[16..];
        }
        redacted.push_str(text);
        redacted
    }

    fn connected_world() -> (World, SessionId) {
        let mut world = World::demo();
        let session = SessionId(7);
        world
            .connect(
                session,
                CharacterSnapshot {
                    id: CharacterId(101),
                    account_id: AccountId(10),
                    name: "Tester".into(),
                    room_id: world.start_room_id().to_owned(),
                    item_locations: BTreeMap::new(),
                },
            )
            .unwrap();
        (world, session)
    }

    fn runtime_snapshot_json_for_version(
        snapshot: &WorldRuntimeSnapshot,
        version: u32,
    ) -> serde_json::Value {
        let mut value = serde_json::to_value(snapshot).unwrap();
        value["snapshotVersion"] = serde_json::json!(version);
        if version <= 8
            && let Some(encounters) = value
                .get_mut("lootEncounters")
                .and_then(|v| v.as_object_mut())
        {
            for encounter in encounters.values_mut() {
                if let Some(object) = encounter.as_object_mut() {
                    object.remove("distribution");
                    object.remove("lootDeclarations");
                    object.remove("masterAssignments");
                }
            }
        }
        if version <= 7 {
            value.as_object_mut().unwrap().remove("areaResidency");
        }
        if version <= 6 {
            value.as_object_mut().unwrap().remove("nextRewardOpenId");
            for instance in value["itemInstances"].as_object_mut().unwrap().values_mut() {
                instance.as_object_mut().unwrap().remove("itemAffixes");
            }
            for provenance in value["itemProvenance"]
                .as_object_mut()
                .unwrap()
                .values_mut()
            {
                provenance
                    .as_object_mut()
                    .unwrap()
                    .remove("sealedLootTableId");
            }
        }
        if version <= 4 {
            let root = value.as_object_mut().unwrap();
            root.remove("nextTradeId");
            root.remove("trades");
        }
        if version <= 5 {
            let root = value.as_object_mut().unwrap();
            root.remove("nextLootDropId");
            root.remove("itemProvenance");
            root.remove("lootEncounters");
            root.remove("groundLoot");
            root.remove("partyLootCursors");
        }
        match version {
            1 => {
                let instances = value
                    .as_object_mut()
                    .unwrap()
                    .remove("itemInstances")
                    .unwrap();
                let locations = instances
                    .as_object()
                    .unwrap()
                    .iter()
                    .map(|(item_id, instance)| (item_id.clone(), instance["location"].clone()))
                    .collect::<serde_json::Map<_, _>>();
                let root = value.as_object_mut().unwrap();
                root.remove("nextItemInstanceSequence");
                root.insert("itemLocations".into(), serde_json::Value::Object(locations));
            }
            2 | 3 => {
                for instance in value["itemInstances"].as_object_mut().unwrap().values_mut() {
                    instance.as_object_mut().unwrap().remove("binding");
                }
            }
            4..=9 => {}
            _ => panic!("test helper only supports runtime snapshot v1-v9"),
        }
        value
    }

    fn first_runtime_npc(value: &mut serde_json::Value) -> &mut serde_json::Value {
        value["npcs"]
            .as_object_mut()
            .unwrap()
            .values_mut()
            .next()
            .expect("runtime snapshot fixture contains an NPC")
    }

    fn connect_definition(definition: WorldDefinition) -> (World, SessionId) {
        let mut world = World::from_definition(definition).unwrap();
        let session = SessionId(71);
        world
            .connect(
                session,
                CharacterSnapshot {
                    id: CharacterId(701),
                    account_id: AccountId(70),
                    name: "Stack Tester".into(),
                    room_id: "test:start".into(),
                    item_locations: BTreeMap::new(),
                },
            )
            .unwrap();
        (world, session)
    }

    fn policy_definition() -> WorldDefinition {
        let mut bag = item_prototype(
            "policy:bag",
            "traveller bag",
            "A bag used to test ownership policy.",
            &["bag"],
        );
        bag.container = Some(ItemContainerDefinition {
            max_slots: 20,
            max_weight_grams: 20_000,
        });
        let mut chest = item_prototype(
            "policy:chest",
            "stone chest",
            "A room-accessible container.",
            &["chest"],
        );
        chest.container = Some(ItemContainerDefinition {
            max_slots: 20,
            max_weight_grams: 20_000,
        });
        let mut clasp = item_prototype(
            "policy:clasp",
            "wolf clasp",
            "A character-bound clasp.",
            &["clasp"],
        );
        clasp.transfer_policy = ItemTransferPolicy::CharacterBound;
        clasp.weight_grams = 10;
        let mut token = item_prototype(
            "policy:token",
            "road token",
            "A non-transferable token.",
            &["token"],
        );
        token.transfer_policy = ItemTransferPolicy::NonTransferable;
        token.weight_grams = 10;
        let mut scripted = item_prototype(
            "policy:scripted",
            "scripted sigil",
            "A staged character-bound grant.",
            &["sigil"],
        );
        scripted.transfer_policy = ItemTransferPolicy::CharacterBound;
        scripted.weight_grams = 10;
        WorldDefinition {
            start_room_id: "test:start".into(),
            item_prototypes: vec![bag, chest, clasp, token, scripted],
            initial_item_instances: vec![
                item_instance(
                    "policy:bag-instance",
                    "policy:bag",
                    InitialItemLocation::Room("test:start".into()),
                ),
                item_instance(
                    "policy:clasp-instance",
                    "policy:clasp",
                    InitialItemLocation::Container("policy:bag-instance".into()),
                ),
                item_instance(
                    "policy:token-instance",
                    "policy:token",
                    InitialItemLocation::Room("test:start".into()),
                ),
                item_instance(
                    "policy:chest-instance",
                    "policy:chest",
                    InitialItemLocation::Room("test:start".into()),
                ),
                item_instance(
                    "policy:scripted-instance",
                    "policy:scripted",
                    InitialItemLocation::Room("test:start".into()),
                ),
            ],
            rooms: vec![RoomDefinition {
                stable_id: "test:start".into(),
                title: "Start".into(),
                prose: vec!["A test room.".into()],
                image: None,
                audio: None,
                item_ids: Vec::new(),
                exits: Vec::new(),
            }],
        }
    }

    fn give_definition() -> WorldDefinition {
        let mut token = item_prototype(
            "give:token",
            "moon token",
            "A fungible transferable token.",
            &["token"],
        );
        token.stack_policy = ItemStackPolicy::Fungible;
        token.max_stack_units = 2;
        token.weight_grams = 100;
        WorldDefinition {
            start_room_id: "give:start".into(),
            item_prototypes: vec![token],
            initial_item_instances: (0..3)
                .map(|index| {
                    item_instance(
                        &format!("give:token-{index}"),
                        "give:token",
                        InitialItemLocation::Room("give:start".into()),
                    )
                })
                .collect(),
            rooms: vec![
                RoomDefinition {
                    stable_id: "give:start".into(),
                    title: "Start".into(),
                    prose: vec!["A shared test room.".into()],
                    image: None,
                    audio: None,
                    item_ids: Vec::new(),
                    exits: vec![ExitDefinition {
                        direction: Direction::North,
                        target_room_id: "give:remote".into(),
                    }],
                },
                RoomDefinition {
                    stable_id: "give:remote".into(),
                    title: "Remote".into(),
                    prose: vec!["A distant test room.".into()],
                    image: None,
                    audio: None,
                    item_ids: Vec::new(),
                    exits: vec![ExitDefinition {
                        direction: Direction::South,
                        target_room_id: "give:start".into(),
                    }],
                },
            ],
        }
    }

    fn phase_two_world(seed: u64, npc_strength: i32, npc_health: i32) -> (World, SessionId) {
        let mut world = World::from_definition_with_staged_items(
            WorldDefinition {
                start_room_id: "game:start".into(),
                item_prototypes: vec![
                    item_prototype(
                        "game:sword",
                        "iron sword",
                        "A balanced beginner's blade.",
                        &["sword", "blade"],
                    ),
                    item_prototype(
                        "game:potion",
                        "red potion",
                        "A restorative draught.",
                        &["potion"],
                    ),
                    item_prototype(
                        "game:fang",
                        "silver fang",
                        "Proof of victory over the guardian.",
                        &["fang"],
                    ),
                ],
                initial_item_instances: vec![
                    item_instance(
                        "game:sword",
                        "game:sword",
                        InitialItemLocation::Room("game:start".into()),
                    ),
                    item_instance(
                        "game:potion",
                        "game:potion",
                        InitialItemLocation::Room("game:start".into()),
                    ),
                    item_instance("game:fang", "game:fang", InitialItemLocation::Staged),
                ],
                rooms: vec![
                    RoomDefinition {
                        stable_id: "game:start".into(),
                        title: "Sanctuary".into(),
                        prose: vec!["A quiet place to prepare.".into()],
                        image: None,
                        audio: None,
                        exits: vec![ExitDefinition {
                            direction: Direction::North,
                            target_room_id: "game:arena".into(),
                        }],
                        item_ids: vec!["game:sword".into(), "game:potion".into()],
                    },
                    RoomDefinition {
                        stable_id: "game:arena".into(),
                        title: "Old Arena".into(),
                        prose: vec!["Claw marks cross the sand.".into()],
                        image: None,
                        audio: None,
                        exits: vec![ExitDefinition {
                            direction: Direction::South,
                            target_room_id: "game:start".into(),
                        }],
                        item_ids: Vec::new(),
                    },
                ],
            },
            vec!["game:fang".into()],
        )
        .unwrap();
        world.set_rng_seed(seed);
        world
            .configure_item_gameplay(ItemGameplayDefinition {
                item_id: "game:sword".into(),
                equipment_slot: Some(EquipmentSlot::Weapon),
                attack_bonus: 4,
                heal_amount: 0,
                consumable: false,
                effect: None,
            })
            .unwrap();
        world
            .configure_item_gameplay(ItemGameplayDefinition {
                item_id: "game:potion".into(),
                equipment_slot: None,
                attack_bonus: 0,
                heal_amount: 5,
                consumable: true,
                effect: Some(ActiveEffect {
                    kind: EffectKind::Regeneration,
                    magnitude: 2,
                    remaining_ticks: 2,
                }),
            })
            .unwrap();
        world
            .spawn_npc(NpcDefinition {
                stable_id: "game:guardian".into(),
                name: "fang guardian".into(),
                description: "A scarred beast guards the arena.".into(),
                room_id: "game:arena".into(),
                level: 2,
                max_health: npc_health,
                strength: npc_strength,
                defence: 1,
                experience_reward: 120,
                loot_item_ids: vec![ItemInstanceId("game:fang".into())],
                respawn_ticks: 3,
            })
            .unwrap();
        let session = SessionId(900);
        world
            .connect(
                session,
                CharacterSnapshot {
                    id: CharacterId(900),
                    account_id: AccountId(90),
                    name: "Adventurer".into(),
                    room_id: "game:start".into(),
                    item_locations: BTreeMap::new(),
                },
            )
            .unwrap();
        (world, session)
    }

    #[test]
    fn runtime_gameplay_definitions_enforce_numeric_caps() {
        let (mut world, _) = phase_two_world(1, 2, 10);
        let max_stat_value = MAX_STAT_VALUE_I32;
        let base_item = ItemGameplayDefinition {
            item_id: "game:sword".into(),
            equipment_slot: Some(EquipmentSlot::Weapon),
            attack_bonus: 1,
            heal_amount: 0,
            consumable: false,
            effect: None,
        };
        for invalid_item in [
            ItemGameplayDefinition {
                attack_bonus: max_stat_value + 1,
                ..base_item.clone()
            },
            ItemGameplayDefinition {
                heal_amount: max_stat_value + 1,
                ..base_item.clone()
            },
            ItemGameplayDefinition {
                effect: Some(ActiveEffect {
                    kind: EffectKind::Strength,
                    magnitude: MAX_STATUS_MAGNITUDE + 1,
                    remaining_ticks: 1,
                }),
                ..base_item
            },
        ] {
            assert!(matches!(
                world.configure_item_gameplay(invalid_item),
                Err(WorldError::InvalidGameplayDefinition)
            ));
        }

        let base_npc = NpcDefinition {
            stable_id: "game:bounded-sentinel".into(),
            name: "bounded sentinel".into(),
            description: "A sentinel used to verify runtime bounds.".into(),
            room_id: "game:start".into(),
            level: 1,
            max_health: 10,
            strength: 2,
            defence: 1,
            experience_reward: 1,
            loot_item_ids: Vec::new(),
            respawn_ticks: 1,
        };
        let mut invalid_npcs = Vec::new();
        let mut invalid = base_npc.clone();
        invalid.level = MAX_PROGRESSION_LEVEL + 1;
        invalid_npcs.push(invalid);
        let mut invalid = base_npc.clone();
        invalid.max_health = max_stat_value + 1;
        invalid_npcs.push(invalid);
        let mut invalid = base_npc.clone();
        invalid.strength = max_stat_value + 1;
        invalid_npcs.push(invalid);
        let mut invalid = base_npc;
        invalid.defence = max_stat_value + 1;
        invalid_npcs.push(invalid);
        for invalid_npc in invalid_npcs {
            assert!(matches!(
                world.spawn_npc(invalid_npc),
                Err(WorldError::InvalidGameplayDefinition)
            ));
        }
    }

    #[test]
    fn restored_item_effects_enforce_numeric_caps() {
        let (mut world, session) = phase_two_world(1, 2, 10);
        let mut snapshot = world.character_gameplay_snapshot(session).unwrap();
        snapshot.effects.push(ActiveEffect {
            kind: EffectKind::Strength,
            magnitude: MAX_STATUS_MAGNITUDE + 1,
            remaining_ticks: 1,
        });

        assert!(matches!(
            world.restore_character_gameplay(CharacterId(900), snapshot),
            Err(WorldError::InvalidGameplayDefinition)
        ));
    }

    #[test]
    fn combat_saturates_equipment_and_strength_effect_bonuses() {
        let (mut world, session) = phase_two_world(7, 2, 10);
        world
            .apply(session, PlayerAction::Move(Direction::North))
            .unwrap();

        for (item_id, slot) in [
            ("game:sword", EquipmentSlot::Weapon),
            ("game:potion", EquipmentSlot::Body),
            ("game:fang", EquipmentSlot::Trinket),
        ] {
            let prototype = world.item_prototypes.get_mut(item_id).unwrap();
            prototype.equipment_slot = Some(slot);
            prototype.attack_bonus = i32::MAX;
            world
                .character_gameplay
                .get_mut(&CharacterId(900))
                .unwrap()
                .equipment
                .insert(slot, item_id.into());
        }
        world
            .character_gameplay
            .get_mut(&CharacterId(900))
            .unwrap()
            .effects = vec![
            ActiveEffect {
                kind: EffectKind::Strength,
                magnitude: i32::MAX,
                remaining_ticks: 2,
            },
            ActiveEffect {
                kind: EffectKind::Strength,
                magnitude: i32::MAX,
                remaining_ticks: 2,
            },
        ];
        world.npcs.get_mut("game:guardian").unwrap().defence = 0;

        world
            .apply(session, PlayerAction::Attack("guardian".into()))
            .unwrap();
        let events = world.advance_tick().unwrap();
        let damage = events.iter().find_map(|event| match event {
            DomainEvent::Gameplay {
                event:
                    GameplayEvent::NpcDamaged {
                        amount,
                        remaining_health,
                        ..
                    },
            } => Some((*amount, *remaining_health)),
            _ => None,
        });

        assert_eq!(damage, Some((i32::MAX, 0)));
        assert_eq!(world.npcs["game:guardian"].health, 0);
    }

    #[test]
    fn parses_legacy_friendly_movement_commands() {
        assert_eq!(parse_action("l"), Ok(PlayerAction::Look));
        assert_eq!(
            parse_action("go north"),
            Ok(PlayerAction::Move(Direction::North))
        );
    }

    #[test]
    fn parses_inventory_and_item_commands_with_safe_missing_targets() {
        assert_eq!(parse_action("inventory"), Ok(PlayerAction::Inventory));
        assert_eq!(
            parse_action("get Blue Lantern"),
            Ok(PlayerAction::Take("blue lantern".into()))
        );
        assert_eq!(
            parse_action("drop lamp"),
            Ok(PlayerAction::Drop("lamp".into()))
        );
        assert_eq!(
            parse_action("x lantern"),
            Ok(PlayerAction::Examine("lantern".into()))
        );
        assert_eq!(
            parse_action("open o0123456789abcdef"),
            Ok(PlayerAction::OpenReward("o0123456789abcdef".into()))
        );
        assert_eq!(
            parse_action("reward odds o0123456789abcdef"),
            Ok(PlayerAction::RewardOdds {
                target: "o0123456789abcdef".into(),
                page: 1,
            })
        );
        assert_eq!(
            parse_action("reward odds sealed cache 12"),
            Ok(PlayerAction::RewardOdds {
                target: "sealed cache".into(),
                page: 12,
            })
        );
        assert!(parse_action("reward odds sealed cache 0").is_err());
        assert_eq!(
            parse_action("take   "),
            Err(ParseActionError::MissingTarget { command: "take" })
        );
        assert_eq!(
            parse_action("examine"),
            Err(ParseActionError::MissingTarget { command: "examine" })
        );
    }

    #[test]
    fn parses_social_and_moderation_commands_without_lowercasing_messages() {
        assert_eq!(
            parse_action("say Hold the North Gate!"),
            Ok(PlayerAction::Social(SocialAction::Say(
                "Hold the North Gate!".into()
            )))
        );
        assert_eq!(
            parse_action("yell Hold the Near Gate!"),
            Ok(PlayerAction::Social(SocialAction::Yell(
                "Hold the Near Gate!".into()
            )))
        );
        assert_eq!(
            parse_action("shout Hold Every Gate!"),
            Ok(PlayerAction::Social(SocialAction::Shout(
                "Hold Every Gate!".into()
            )))
        );
        assert_eq!(
            parse_action("tell Aria Meet Me There"),
            Ok(PlayerAction::Social(SocialAction::Tell {
                target: "Aria".into(),
                message: "Meet Me There".into(),
            }))
        );
        assert_eq!(
            parse_action("party say Hold the North Gate!"),
            Ok(PlayerAction::Social(SocialAction::PartySay(
                "Hold the North Gate!".into()
            )))
        );
        assert_eq!(
            parse_action("reply I Am On My Way"),
            Ok(PlayerAction::Social(SocialAction::Reply(
                "I Am On My Way".into()
            )))
        );
        assert_eq!(
            parse_action("channel list"),
            Ok(PlayerAction::Social(SocialAction::ChannelList))
        );
        assert_eq!(
            parse_action("channel join newbie"),
            Ok(PlayerAction::Social(SocialAction::ChannelJoin(
                SocialChannelName::Newbie
            )))
        );
        assert_eq!(
            parse_action("channel leave general"),
            Ok(PlayerAction::Social(SocialAction::ChannelLeave(
                SocialChannelName::General
            )))
        );
        assert_eq!(
            parse_action("channel say general Keep This Case"),
            Ok(PlayerAction::Social(SocialAction::ChannelSay {
                channel: SocialChannelName::General,
                message: "Keep This Case".into(),
            }))
        );
        assert_eq!(
            parse_action("party revoke Aria"),
            Ok(PlayerAction::Party(PartyAction::Revoke("Aria".into())))
        );
        assert_eq!(
            parse_action("party kick Aria"),
            Ok(PlayerAction::Party(PartyAction::Kick("Aria".into())))
        );
        assert_eq!(
            parse_action("party transfer Aria"),
            Ok(PlayerAction::Party(PartyAction::Transfer("Aria".into())))
        );
        assert_eq!(
            parse_action("party disband"),
            Ok(PlayerAction::Party(PartyAction::Disband))
        );
        for command in [
            "yell",
            "shout",
            "party say",
            "reply",
            "channel join",
            "channel say general",
        ] {
            assert!(matches!(
                parse_action(command),
                Err(ParseActionError::MissingTarget { .. })
            ));
        }
        assert_eq!(
            parse_action("mute Griefer 2h repeated flooding"),
            Ok(PlayerAction::Social(SocialAction::Mute {
                target: "Griefer".into(),
                duration_seconds: Some(7_200),
                reason: "repeated flooding".into(),
            }))
        );
        assert_eq!(
            parse_action("ban Griefer permanent severe abuse"),
            Ok(PlayerAction::Social(SocialAction::Ban {
                target: "Griefer".into(),
                duration_seconds: None,
                reason: "severe abuse".into(),
            }))
        );
        assert_eq!(
            parse_action("role Aria moderator operational promotion"),
            Ok(PlayerAction::Social(SocialAction::SetRole {
                target: "Aria".into(),
                role: AccountRole::Moderator,
                reason: "operational promotion".into(),
            }))
        );
        assert_eq!(
            parse_action("unmute Griefer appeal accepted"),
            Ok(PlayerAction::Social(SocialAction::Unmute {
                target: "Griefer".into(),
                reason: "appeal accepted".into(),
            }))
        );
    }

    #[test]
    fn parses_staff_commands_with_closed_values_and_mandatory_reasons() {
        assert_eq!(
            parse_action("staff inspect Aria | investigate stuck character"),
            Ok(PlayerAction::Staff(StaffAction::Inspect {
                target: "Aria".into(),
                reason: "investigate stuck character".into(),
            }))
        );
        assert_eq!(
            parse_action("staff announce Server restart in ten minutes | planned maintenance"),
            Ok(PlayerAction::Staff(StaffAction::Announce {
                message: "Server restart in ten minutes".into(),
                reason: "planned maintenance".into(),
            }))
        );
        assert_eq!(
            parse_action("staff resolve 7 action_taken verified harassment"),
            Ok(PlayerAction::Staff(StaffAction::ResolveReport {
                report_id: 7,
                resolution: StaffReportResolution::ActionTaken,
                reason: "verified harassment".into(),
            }))
        );
        assert_eq!(
            parse_action("staff profile Aria helper Angel of Welcome | onboarding assignment"),
            Ok(PlayerAction::Staff(StaffAction::SetProfile {
                target: "Aria".into(),
                profile: Some(StaffProfileKey::Helper),
                lore_title: Some("Angel of Welcome".into()),
                reason: "onboarding assignment".into(),
            }))
        );
        assert_eq!(
            parse_action("staff capability grant Aria reports.view | support rotation"),
            Ok(PlayerAction::Staff(StaffAction::SetCapability {
                target: "Aria".into(),
                capability: StaffCapability::ReportsView,
                enabled: true,
                reason: "support rotation".into(),
            }))
        );
        for invalid in [
            "staff inspect Aria",
            "staff announce notice without reason",
            "staff resolve 0 action_taken invalid id",
            "staff resolve 7 arbitrary invalid resolution",
            "staff capability grant Aria unknown | invalid capability",
        ] {
            assert!(parse_action(invalid).is_err(), "{invalid}");
        }
    }

    #[test]
    fn parses_staff_area_lifecycle_commands_with_mandatory_reasons() {
        assert_eq!(
            parse_action("staff area retire square | draining for maintenance"),
            Ok(PlayerAction::Staff(StaffAction::RetireArea {
                area: "square".into(),
                reason: "draining for maintenance".into(),
            }))
        );
        assert_eq!(
            parse_action("staff area rehydrate square | restoring after maintenance"),
            Ok(PlayerAction::Staff(StaffAction::RehydrateArea {
                area: "square".into(),
                reason: "restoring after maintenance".into(),
            }))
        );
        assert_eq!(
            parse_action("staff area retire square"),
            Err(ParseActionError::MissingTarget { command: "staff" })
        );
        assert_eq!(
            parse_action("STAFF AREA RETIRE Square | why"),
            Ok(PlayerAction::Staff(StaffAction::RetireArea {
                area: "Square".into(),
                reason: "why".into(),
            }))
        );
    }

    #[test]
    fn staff_profiles_use_explicit_ranks_and_closed_default_bundles() {
        assert!(
            StaffProfileKey::Helper.authority_rank() < StaffProfileKey::Moderator.authority_rank()
        );
        assert!(
            StaffProfileKey::Administrator.authority_rank()
                < StaffProfileKey::Implementor.authority_rank()
        );
        assert_eq!(StaffProfileKey::Builder.default_capabilities(), &[]);
        assert!(
            StaffProfileKey::Moderator
                .default_capabilities()
                .contains(&StaffCapability::ModerationBan)
        );
        assert_eq!(
            StaffProfileKey::Administrator.default_capabilities(),
            &StaffCapability::ALL
        );
    }

    #[test]
    fn staff_relocation_and_restore_are_checkpoint_safe_and_project_no_stable_ids() {
        let mut world = World::demo();
        let actor = SessionId(501);
        let target = SessionId(502);
        world
            .connect(
                actor,
                CharacterSnapshot {
                    id: CharacterId(501),
                    account_id: AccountId(501),
                    name: "Keeper".into(),
                    room_id: "demo:lantern-square".into(),
                    item_locations: BTreeMap::new(),
                },
            )
            .unwrap();
        world
            .connect(
                target,
                CharacterSnapshot {
                    id: CharacterId(502),
                    account_id: AccountId(502),
                    name: "Aria".into(),
                    room_id: "demo:north-gate".into(),
                    item_locations: BTreeMap::new(),
                },
            )
            .unwrap();

        let inspection = world.staff_inspection_as_plain_text(target).unwrap();
        assert!(inspection.contains("Character: Aria"));
        assert!(!inspection.contains("demo:north-gate"));
        assert!(!inspection.contains("account #"));

        let checkpoint = world.checkpoint();
        let relocated = world.staff_goto(actor, target).unwrap();
        assert!(matches!(
            relocated.domain_events.as_slice(),
            [DomainEvent::CharacterRelocated { actor_id, .. }] if *actor_id == CharacterId(501)
        ));
        assert_eq!(
            world.character_snapshot(actor).unwrap().room_id,
            "demo:north-gate"
        );
        world.restore_checkpoint(checkpoint);
        assert_eq!(
            world.character_snapshot(actor).unwrap().room_id,
            "demo:lantern-square"
        );

        let mut gameplay = world.character_gameplay_snapshot(target).unwrap();
        gameplay.vitals.health = 0;
        gameplay.recovery_at_tick = Some(world.logical_tick().saturating_add(3));
        world
            .restore_character_gameplay(CharacterId(502), gameplay)
            .unwrap();
        let restored = world.staff_restore(actor, target).unwrap();
        assert!(matches!(
            restored.domain_events.as_slice(),
            [DomainEvent::Gameplay {
                event: GameplayEvent::CharacterRestored { character_id, .. }
            }] if *character_id == CharacterId(502)
        ));
        let restored_gameplay = world.character_gameplay_snapshot(target).unwrap();
        assert_eq!(
            restored_gameplay.vitals.health,
            restored_gameplay.vitals.max_health
        );
        assert_eq!(restored_gameplay.recovery_at_tick, None);
    }

    #[test]
    fn movement_changes_the_stable_room_reference() {
        let (mut world, session) = connected_world();
        let outcome = world
            .apply(session, PlayerAction::Move(Direction::North))
            .unwrap();
        assert_ne!(outcome.scene.view_id, "demo:north-gate");
        assert_eq!(outcome.scene.title, "The North Gate");
        assert_eq!(
            outcome.changed_character.unwrap().room_id,
            "demo:north-gate"
        );
    }

    #[test]
    fn failed_movement_does_not_change_state() {
        let (mut world, session) = connected_world();
        let before = world.character_snapshot(session).unwrap();
        assert!(matches!(
            world.apply(session, PlayerAction::Move(Direction::West)),
            Err(WorldError::NoExit)
        ));
        assert_eq!(world.character_snapshot(session).unwrap(), before);
    }

    #[test]
    fn deferred_portal_preflight_is_idempotent_secret_safe_and_non_mutating() {
        let generation = area_generation();
        let request = deferred_portal(
            &generation,
            "area-a:start",
            Direction::North,
            "area-b:start",
            "area-b",
            2,
        );
        let initial_batch = PreparedCoreAreaBatch {
            generation_id: generation.clone(),
            areas: vec![prepared_area(
                "area-a",
                1,
                vec![prepared_room(
                    "area-a:start",
                    [(Direction::North, "area-b:start")],
                )],
            )],
            deferred_exits: vec![request.clone()],
        };
        let duplicate_batch = initial_batch.clone();
        let (mut world, installed) =
            World::from_initial_prepared_areas("area-a:start", initial_batch).unwrap();
        assert_eq!(installed.installed_area_ids, vec!["area-a"]);
        assert_eq!(world.area_generation(), Some(&generation));
        assert_eq!(world.active_area_count(), 1);
        assert!(world.active_area_matches("area-a", 1, &format!("{:064x}", 1)));

        // A coalesced duplicate completion is a strict no-op even while its
        // outgoing portal remains deferred.
        assert_eq!(
            world.install_prepared_areas(duplicate_batch).unwrap(),
            CoreAreaInstallOutcome {
                installed_areas: 0,
                installed_area_ids: Vec::new(),
                created_dynamic_state: false,
            }
        );

        let session = SessionId(71);
        world
            .connect(
                session,
                CharacterSnapshot {
                    id: CharacterId(71),
                    account_id: AccountId(71),
                    name: "Portal Tester".into(),
                    room_id: "area-a:start".into(),
                    item_locations: BTreeMap::new(),
                },
            )
            .unwrap();
        let before_character = world.character_snapshot(session).unwrap();
        let before_revision = world.revision();
        let before_view_revision = world.observer(session).unwrap().revision;
        let scene = world.scene_for(session).unwrap();
        assert_eq!(scene.exits.len(), 1);
        assert!(!scene.exits[0].id.contains("area-b"));
        let player = world.player_state_view(session).unwrap();
        assert_eq!(player.map.paths.len(), 1);
        assert_eq!(player.map.paths[0].to, None);
        assert_eq!(
            world
                .deferred_area_load_request(session, Direction::North)
                .unwrap(),
            Some(request.clone())
        );
        assert!(matches!(
            world.apply(session, PlayerAction::Move(Direction::North)),
            Err(WorldError::AreaLoadRequired(actual)) if actual == request
        ));
        assert_eq!(world.character_snapshot(session).unwrap(), before_character);
        assert_eq!(world.revision(), before_revision);
        assert_eq!(
            world.observer(session).unwrap().revision,
            before_view_revision
        );
        assert_eq!(world.shout_audience(session).unwrap().len(), 1);

        world
            .configure_secret_exit("area-a:start", Direction::North, 100)
            .unwrap();
        assert!(world.scene_for(session).unwrap().exits.is_empty());
        assert_eq!(
            world
                .deferred_area_load_request(session, Direction::North)
                .unwrap(),
            None
        );
    }

    #[test]
    fn snapshot_v7_requires_an_explicit_catalog_manifest_but_monolithic_upgrade_is_empty() {
        let (world, _) = connected_world();
        let legacy = runtime_snapshot_json_for_version(&world.runtime_snapshot(), 7);
        let json = serde_json::to_string(&legacy).unwrap();
        let exact = decode_world_runtime_snapshot_v7(7, &json).unwrap();
        let monolithic = decode_world_runtime_snapshot(7, &json).unwrap();
        assert_eq!(monolithic.snapshot_version, WORLD_RUNTIME_SNAPSHOT_VERSION);
        assert!(monolithic.area_residency.is_empty());

        let manifest = BTreeMap::from([(
            "area-a".into(),
            AreaRuntimeResidence::Resident {
                revision: 1,
                bundle_sha256: "a".repeat(64),
                residency_epoch: 1,
            },
        )]);
        let catalog = WorldRuntimeSnapshot::upgrade_v7(exact.clone(), manifest.clone()).unwrap();
        assert_eq!(catalog.area_residency, manifest);
        let retired = BTreeMap::from([(
            "area-a".into(),
            AreaRuntimeResidence::Retired {
                revision: 1,
                bundle_sha256: "a".repeat(64),
                residency_epoch: 2,
                checkpoint_version: AREA_DYNAMIC_CHECKPOINT_VERSION,
                checkpoint_sha256: "b".repeat(64),
            },
        )]);
        assert!(WorldRuntimeSnapshot::upgrade_v7(exact, retired).is_err());
    }

    #[test]
    fn area_retirement_is_partitioned_restart_safe_and_rehydrates_without_global_rewind() {
        let (mut world, rehydrate_batch, unit, _) = retirement_fixture();
        let carried_id = ItemInstanceId("area-b:carried-instance".into());
        let chest_id = ItemInstanceId("area-b:chest-instance".into());
        let child_id = ItemInstanceId("area-b:child-instance".into());
        let npc_item_id = ItemInstanceId("area-b:npc-token-instance".into());
        let npc = world.npcs.get_mut("area-b:keeper").unwrap();
        npc.health = 0;
        npc.respawn_at_tick = Some(1);

        let prepared = world.prepare_area_retirement(unit.clone()).unwrap();
        assert!(prepared.checkpoint.item_instances.contains_key(&chest_id));
        assert!(prepared.checkpoint.item_instances.contains_key(&child_id));
        assert!(
            prepared
                .checkpoint
                .item_instances
                .contains_key(&npc_item_id)
        );
        assert!(!prepared.checkpoint.item_instances.contains_key(&carried_id));
        assert!(prepared.checkpoint.ground_loot.contains_key(&chest_id));
        assert_eq!(prepared.checkpoint.npcs["area-b:keeper"].health, 0);
        let checkpoint = prepared.checkpoint.clone();
        let checkpoint_sha256 = canonical_sha256(&checkpoint).unwrap();
        let before_retirement = (
            world.revision,
            world.logical_tick,
            world.rng.state,
            world.next_item_instance_sequence,
            world.next_loot_drop_id,
            world.next_reward_open_id,
        );
        world
            .apply_prepared_area_retirement(prepared, checkpoint_sha256.clone())
            .unwrap();
        assert_eq!(
            (
                world.revision,
                world.logical_tick,
                world.rng.state,
                world.next_item_instance_sequence,
                world.next_loot_drop_id,
                world.next_reward_open_id,
            ),
            before_retirement
        );
        assert!(!world.area_is_active("area-b"));
        assert!(!world.stable_rooms.contains_key("area-b:start"));
        assert!(!world.npcs.contains_key("area-b:keeper"));
        assert!(!world.item_instances.contains_key(&chest_id));
        assert!(!world.item_instances.contains_key(&child_id));
        assert!(matches!(
            world.item_location(&carried_id),
            Some(SharedItemLocation::Carried(CharacterId(612)))
        ));
        let retired_snapshot = world.runtime_snapshot();
        assert!(matches!(
            retired_snapshot.area_residency.get("area-b"),
            Some(AreaRuntimeResidence::Retired { checkpoint_sha256: digest, .. })
                if digest == &checkpoint_sha256
        ));

        let (mut restarted, _, restart_unit, session) = retirement_fixture();
        restarted.disconnect(session);
        restarted
            .set_item_location(&carried_id, SharedItemLocation::Room("area-b:start".into()))
            .unwrap();
        let before_failed_restore = restarted.runtime_snapshot();
        assert!(
            restarted
                .restore_runtime_snapshot(retired_snapshot.clone())
                .is_err()
        );
        assert_eq!(restarted.runtime_snapshot(), before_failed_restore);
        restarted
            .restore_runtime_snapshot_with_retired_areas(
                retired_snapshot.clone(),
                vec![restart_unit],
            )
            .unwrap();
        assert!(!restarted.area_is_active("area-b"));
        assert!(!restarted.stable_rooms.contains_key("area-b:start"));
        assert!(!restarted.item_instances.contains_key(&chest_id));
        assert!(matches!(
            restarted.item_location(&carried_id),
            Some(SharedItemLocation::Carried(CharacterId(612)))
        ));

        world.advance_tick().unwrap();
        let before_rehydrate = (
            world.revision,
            world.logical_tick,
            world.rng.state,
            world.next_item_instance_sequence,
            world.next_loot_drop_id,
            world.next_reward_open_id,
        );
        let outcome = world
            .rehydrate_retired_areas(rehydrate_batch, checkpoint, &checkpoint_sha256)
            .unwrap();
        assert_eq!(outcome.installed_area_ids, vec!["area-b"]);
        assert!(outcome.created_dynamic_state);
        assert_eq!(
            (
                world.revision,
                world.logical_tick,
                world.rng.state,
                world.next_item_instance_sequence,
                world.next_loot_drop_id,
                world.next_reward_open_id,
            ),
            before_rehydrate
        );
        assert!(world.area_is_active("area-b"));
        assert!(world.item_instances.contains_key(&chest_id));
        assert!(matches!(
            world.item_location(&child_id),
            Some(SharedItemLocation::Contained(container)) if container == &chest_id
        ));
        assert!(world.ground_loot.contains_key(&chest_id));
        assert!(matches!(
            world.item_location(&carried_id),
            Some(SharedItemLocation::Carried(CharacterId(612)))
        ));
        let npc = &world.npcs["area-b:keeper"];
        assert_eq!(npc.health, npc.max_health);
        assert_eq!(npc.respawn_at_tick, None);
    }

    #[test]
    fn area_retirement_prunes_discovered_rooms_so_projection_never_dangles() {
        let (mut world, _rehydrate_batch, unit, session) = retirement_fixture();
        // The observer previously visited the retired unit's room, so its map
        // memory references a room that retirement removes from `stable_rooms`.
        world
            .observer_views
            .get_mut(&session)
            .expect("connected observer")
            .discovered_rooms
            .insert("area-b:start".into());
        // Both projections resolve `discovered_rooms` strictly and would fail
        // with `InvalidEntity` if the retired room lingered in the map.
        assert!(world.player_state_view(session).is_ok());

        let prepared = world.prepare_area_retirement(unit).unwrap();
        let checkpoint_sha256 = canonical_sha256(&prepared.checkpoint).unwrap();
        world
            .apply_prepared_area_retirement(prepared, checkpoint_sha256)
            .unwrap();

        assert!(!world.stable_rooms.contains_key("area-b:start"));
        assert!(
            !world
                .observer_views
                .get(&session)
                .expect("observer survives retirement")
                .discovered_rooms
                .contains("area-b:start"),
            "retired room must be pruned from the observer's map memory"
        );
        // The dangling reference is gone: the full player projection succeeds.
        world.player_state_view(session).unwrap();
        world.scene_for(session).unwrap();
    }

    #[test]
    fn area_retirement_pins_non_convertible_state_without_mutation() {
        let (mut world, _, unit, session) = retirement_fixture();
        let before = world.runtime_snapshot();
        let b_room = world.stable_rooms["area-b:start"];
        if let Entity::Character(character) = world
            .entities
            .get_mut(world.sessions[&session])
            .expect("connected character")
        {
            character.location = b_room;
        }
        assert_eq!(
            world.prepare_area_retirement(unit.clone()).unwrap_err(),
            AreaRetirementError::Busy(AreaRetirementPin::ConnectedCharacter)
        );
        if let Entity::Character(character) = world
            .entities
            .get_mut(world.sessions[&session])
            .expect("connected character")
        {
            character.location = world.stable_rooms["area-a:start"];
        }
        world
            .npcs
            .get_mut("area-b:keeper")
            .unwrap()
            .engaged_character = Some(CharacterId(612));
        let combat_before = world.runtime_snapshot();
        assert_eq!(
            world.prepare_area_retirement(unit.clone()).unwrap_err(),
            AreaRetirementError::Busy(AreaRetirementPin::Combat)
        );
        // Runtime snapshots deliberately exclude connected positions, but all
        // durable state is byte-for-byte unchanged by both failed preflights.
        assert_ne!(combat_before, before);
        assert_eq!(world.runtime_snapshot(), combat_before);

        world
            .npcs
            .get_mut("area-b:keeper")
            .unwrap()
            .engaged_character = None;
        world.trades.insert(
            TradeId(1),
            TradeSnapshot {
                id: TradeId(1),
                requester: CharacterId(612),
                recipient: CharacterId(613),
                room_id: "area-b:start".into(),
                phase: TradePhase::Negotiating,
                created_tick: 0,
                opened_tick: Some(0),
                expires_at_tick: TRADE_NEGOTIATION_TICKS,
                state_revision: 1,
                offers: BTreeMap::from([
                    (CharacterId(612), TradeOffer::default()),
                    (CharacterId(613), TradeOffer::default()),
                ]),
                confirmations: BTreeMap::new(),
            },
        );
        let trade_before = world.runtime_snapshot();
        assert_eq!(
            world.prepare_area_retirement(unit.clone()).unwrap_err(),
            AreaRetirementError::Busy(AreaRetirementPin::Trade)
        );
        assert_eq!(world.runtime_snapshot(), trade_before);
        world.trades.clear();

        let stable_before_restore = world.runtime_snapshot();
        let mut late_invalid = stable_before_restore.clone();
        if let Some(AreaRuntimeResidence::Resident {
            residency_epoch, ..
        }) = late_invalid.area_residency.get_mut("area-b")
        {
            *residency_epoch = 99;
        }
        late_invalid
            .item_instances
            .get_mut(&ItemInstanceId("area-b:carried-instance".into()))
            .unwrap()
            .prototype_id = "missing:prototype".into();
        assert!(world.restore_runtime_snapshot(late_invalid).is_err());
        assert_eq!(world.runtime_snapshot(), stable_before_restore);
    }

    #[test]
    fn multi_area_retirement_uses_one_epoch_above_the_unit_maximum() {
        let (mut world, _, mut unit, _) = retirement_fixture();
        world
            .install_prepared_areas(PreparedCoreAreaBatch {
                generation_id: area_generation(),
                areas: vec![prepared_area(
                    "area-c",
                    1,
                    vec![prepared_room("area-c:start", [])],
                )],
                deferred_exits: Vec::new(),
            })
            .unwrap();
        world
            .active_areas
            .get_mut("area-b")
            .unwrap()
            .residency_epoch = 3;
        world
            .active_areas
            .get_mut("area-c")
            .unwrap()
            .residency_epoch = 8;
        unit.areas.push(AreaRuntimeOwnership {
            area_id: "area-c".into(),
            revision: 1,
            bundle_sha256: format!("{:064x}", 1),
            rooms: BTreeSet::from(["area-c:start".into()]),
            initial_item_instances: BTreeSet::new(),
            npc_spawns: BTreeSet::new(),
        });
        unit.areas
            .sort_by(|left, right| left.area_id.cmp(&right.area_id));

        let prepared = world.prepare_area_retirement(unit).unwrap();
        assert!(
            prepared
                .checkpoint
                .areas
                .iter()
                .all(|area| area.residency_epoch == 9)
        );
        let digest = canonical_sha256(&prepared.checkpoint).unwrap();
        world
            .apply_prepared_area_retirement(prepared, digest)
            .unwrap();
        let snapshot = world.runtime_snapshot();
        assert!(matches!(
            snapshot.area_residency.get("area-b"),
            Some(AreaRuntimeResidence::Retired {
                residency_epoch: 9,
                ..
            })
        ));
        assert!(matches!(
            snapshot.area_residency.get("area-c"),
            Some(AreaRuntimeResidence::Retired {
                residency_epoch: 9,
                ..
            })
        ));
    }

    #[test]
    fn initial_area_configuration_pins_abilities_before_lazy_npcs_install() {
        let generation = area_generation();
        let mut area = prepared_area("area-a", 1, vec![prepared_room("area-a:start", [])]);
        area.npcs.push(NpcDefinition {
            stable_id: "area-a:sentinel".into(),
            name: "sentinel".into(),
            description: "A sentinel with a pinned global ability.".into(),
            room_id: "area-a:start".into(),
            level: 1,
            max_health: 10,
            strength: 1,
            defence: 0,
            experience_reward: 0,
            loot_item_ids: Vec::new(),
            respawn_ticks: 10,
        });
        area.npc_ability_sets.push(NpcAbilitySetDefinition {
            npc_id: "area-a:sentinel".into(),
            resource_pools: BTreeMap::new(),
            resource_regeneration: BTreeMap::new(),
            resistances: BTreeMap::new(),
            ability_ids: vec!["global:jab".into()],
        });
        let (world, outcome) = World::from_initial_prepared_areas_with_configuration(
            "area-a:start",
            PreparedCoreAreaBatch {
                generation_id: generation,
                areas: vec![area],
                deferred_exits: Vec::new(),
            },
            |world| {
                world.configure_progression(
                    builtin_progression_definition(),
                    vec![AbilityDefinition {
                        id: "global:jab".into(),
                        name: "Jab".into(),
                        description: "A pinned global strike.".into(),
                        category: AbilityCategory::Martial,
                        resource_costs: BTreeMap::new(),
                        cooldown_ticks: 0,
                        targeting: AbilityTargeting::SingleEnemy,
                        damage: Some(AbilityDamageDefinition {
                            target: AbilityEffectTarget::Target,
                            damage_type: DamageType::Physical,
                            base_amount: 1,
                            scaling_attribute: None,
                            scaling_percent: 0,
                        }),
                        heal: None,
                        status_effects: Vec::new(),
                        communication: None,
                    }],
                )
            },
        )
        .unwrap();
        assert!(outcome.created_dynamic_state);
        assert_eq!(world.revision(), 1);
        assert_eq!(
            world.npcs["area-a:sentinel"].ability_ids,
            vec!["global:jab"]
        );
    }

    #[test]
    fn additive_cross_area_presentation_and_pending_npc_abilities_are_atomic() {
        let generation = area_generation();
        let mut start = prepared_area("area-a", 1, vec![prepared_room("area-a:start", [])]);
        let image = SceneImage {
            alt: "A verified cross-area scene.".into(),
            localized_alt: BTreeMap::from([(
                "es".into(),
                "Una escena verificada entre áreas.".into(),
            )]),
            manifest: Some(format!("/assets/{}/manifest.json", "a".repeat(64))),
            avif: None,
            webp: None,
            jpeg: None,
            width: None,
            height: None,
        };
        start.room_presentations.push(RoomPresentationDefinition {
            room_id: "area-a:start".into(),
            image: Some(image.clone()),
            audio: None,
        });
        let mut rules = prepared_area("area-rules", 1, Vec::new());
        rules.npc_ability_sets.push(NpcAbilitySetDefinition {
            npc_id: "area-npcs:sentinel".into(),
            resource_pools: BTreeMap::new(),
            resource_regeneration: BTreeMap::new(),
            resistances: BTreeMap::new(),
            ability_ids: vec!["global:jab".into()],
        });
        let (mut world, initial) = World::from_initial_prepared_areas_with_configuration(
            "area-a:start",
            PreparedCoreAreaBatch {
                generation_id: generation.clone(),
                areas: vec![start, rules],
                deferred_exits: Vec::new(),
            },
            |world| {
                world.configure_progression(
                    builtin_progression_definition(),
                    vec![AbilityDefinition {
                        id: "global:jab".into(),
                        name: "Jab".into(),
                        description: "A pinned global strike.".into(),
                        category: AbilityCategory::Martial,
                        resource_costs: BTreeMap::new(),
                        cooldown_ticks: 0,
                        targeting: AbilityTargeting::SingleEnemy,
                        damage: Some(AbilityDamageDefinition {
                            target: AbilityEffectTarget::Target,
                            damage_type: DamageType::Physical,
                            base_amount: 1,
                            scaling_attribute: None,
                            scaling_percent: 0,
                        }),
                        heal: None,
                        status_effects: Vec::new(),
                        communication: None,
                    }],
                )
            },
        )
        .unwrap();
        assert!(!initial.created_dynamic_state);
        let room = room_checked(&world.entities, world.stable_rooms["area-a:start"]).unwrap();
        assert_eq!(room.image.as_ref(), Some(&image));
        assert!(
            world
                .pending_npc_ability_sets
                .contains_key("area-npcs:sentinel")
        );

        let mut npcs = prepared_area("area-npcs", 1, Vec::new());
        npcs.npcs.push(NpcDefinition {
            stable_id: "area-npcs:sentinel".into(),
            name: "sentinel".into(),
            description: "A sentinel configured from another area.".into(),
            room_id: "area-a:start".into(),
            level: 1,
            max_health: 10,
            strength: 1,
            defence: 0,
            experience_reward: 0,
            loot_item_ids: Vec::new(),
            respawn_ticks: 10,
        });
        let installed = world
            .install_prepared_areas(PreparedCoreAreaBatch {
                generation_id: generation.clone(),
                areas: vec![npcs],
                deferred_exits: Vec::new(),
            })
            .unwrap();
        assert!(installed.created_dynamic_state);
        assert_eq!(
            world.npcs["area-npcs:sentinel"].ability_ids,
            vec!["global:jab"]
        );
        assert!(world.pending_npc_ability_sets.is_empty());

        let mut loot = prepared_area("area-loot", 1, Vec::new());
        loot.item_prototypes.push(item_prototype(
            "area-loot:token",
            "token",
            "A cross-area NPC possession.",
            &[],
        ));
        loot.initial_item_instances
            .push(InitialItemInstanceDefinition {
                instance_id: ItemInstanceId("area-loot:token-1".into()),
                prototype_id: "area-loot:token".into(),
                location: InitialItemLocation::NpcSpawn("area-npcs:sentinel".into()),
            });
        let installed_loot = world
            .install_prepared_areas(PreparedCoreAreaBatch {
                generation_id: generation.clone(),
                areas: vec![loot],
                deferred_exits: Vec::new(),
            })
            .unwrap();
        assert!(installed_loot.created_dynamic_state);
        assert!(
            world.npcs["area-npcs:sentinel"]
                .loot_item_ids
                .contains(&ItemInstanceId("area-loot:token-1".into()))
        );
        assert_eq!(
            world.item_location(&ItemInstanceId("area-loot:token-1".into())),
            Some(&SharedItemLocation::HeldByNpc("area-npcs:sentinel".into()))
        );

        let checkpoint = world.lifecycle_checkpoint();
        let mut conflict = prepared_area("area-conflict", 1, Vec::new());
        let mut conflicting_image = image;
        conflicting_image.alt = "Conflicting replacement.".into();
        conflict
            .room_presentations
            .push(RoomPresentationDefinition {
                room_id: "area-a:start".into(),
                image: Some(conflicting_image),
                audio: None,
            });
        assert!(matches!(
            world.install_prepared_areas(PreparedCoreAreaBatch {
                generation_id: generation,
                areas: vec![conflict],
                deferred_exits: Vec::new(),
            }),
            Err(WorldError::InvalidAreaLifecycle)
        ));
        let after = world.lifecycle_checkpoint();
        assert_eq!(checkpoint.active_areas, after.active_areas);
        assert_eq!(
            checkpoint.pending_npc_ability_sets,
            after.pending_npc_ability_sets
        );
        let room = room_checked(&world.entities, world.stable_rooms["area-a:start"]).unwrap();
        assert_eq!(
            room.image.as_ref().map(|image| image.alt.as_str()),
            Some("A verified cross-area scene.")
        );
    }

    #[test]
    fn additive_install_relinks_portals_reports_dynamic_state_and_rolls_back_exactly() {
        let generation = area_generation();
        let first_request = deferred_portal(
            &generation,
            "area-a:start",
            Direction::North,
            "area-b:start",
            "area-b",
            2,
        );
        let (mut world, _) = World::from_initial_prepared_areas(
            "area-a:start",
            PreparedCoreAreaBatch {
                generation_id: generation.clone(),
                areas: vec![prepared_area(
                    "area-a",
                    1,
                    vec![prepared_room(
                        "area-a:start",
                        [(Direction::North, "area-b:start")],
                    )],
                )],
                deferred_exits: vec![first_request],
            },
        )
        .unwrap();
        let session = SessionId(72);
        world
            .connect(
                session,
                CharacterSnapshot {
                    id: CharacterId(72),
                    account_id: AccountId(72),
                    name: "Lifecycle Tester".into(),
                    room_id: "area-a:start".into(),
                    item_locations: BTreeMap::new(),
                },
            )
            .unwrap();
        let second_request = deferred_portal(
            &generation,
            "area-b:start",
            Direction::East,
            "area-c:start",
            "area-c",
            3,
        );
        let before_install_revision = world.revision();
        let installed = world
            .install_prepared_areas(PreparedCoreAreaBatch {
                generation_id: generation.clone(),
                areas: vec![prepared_area(
                    "area-b",
                    2,
                    vec![prepared_room(
                        "area-b:start",
                        [
                            (Direction::South, "area-a:start"),
                            (Direction::East, "area-c:start"),
                        ],
                    )],
                )],
                deferred_exits: vec![second_request.clone()],
            })
            .unwrap();
        assert_eq!(installed.installed_area_ids, vec!["area-b"]);
        assert!(!installed.created_dynamic_state);
        assert_eq!(world.revision(), before_install_revision);
        assert_eq!(
            world
                .deferred_area_load_request(session, Direction::North)
                .unwrap(),
            None
        );
        world
            .apply(session, PlayerAction::Move(Direction::North))
            .unwrap();
        assert_eq!(
            world
                .deferred_area_load_request(session, Direction::East)
                .unwrap(),
            Some(second_request)
        );

        let lifecycle_checkpoint = world.lifecycle_checkpoint();
        let mut invalid_c = prepared_area(
            "area-c",
            3,
            vec![prepared_room(
                "area-c:start",
                [(Direction::West, "area-b:start")],
            )],
        );
        let duplicate = item_prototype("area-c:item", "area item", "An item.", &[]);
        invalid_c.item_prototypes = vec![duplicate.clone(), duplicate];
        assert!(matches!(
            world.install_prepared_areas(PreparedCoreAreaBatch {
                generation_id: generation.clone(),
                areas: vec![invalid_c],
                deferred_exits: Vec::new(),
            }),
            Err(WorldError::InvalidAreaLifecycle)
        ));
        assert!(!world.area_is_active("area-c"));
        assert_eq!(
            world
                .deferred_area_load_request(session, Direction::East)
                .unwrap(),
            Some(deferred_portal(
                &generation,
                "area-b:start",
                Direction::East,
                "area-c:start",
                "area-c",
                3,
            ))
        );

        let mut valid_c = prepared_area(
            "area-c",
            3,
            vec![prepared_room(
                "area-c:start",
                [(Direction::West, "area-b:start")],
            )],
        );
        valid_c.item_prototypes = vec![item_prototype(
            "area-c:item",
            "area item",
            "An item installed with its area.",
            &[],
        )];
        valid_c.initial_item_instances = vec![item_instance(
            "area-c:item-instance",
            "area-c:item",
            InitialItemLocation::Room("area-c:start".into()),
        )];
        let revision_before_dynamic_install = world.revision();
        let installed = world
            .install_prepared_areas(PreparedCoreAreaBatch {
                generation_id: generation,
                areas: vec![valid_c],
                deferred_exits: Vec::new(),
            })
            .unwrap();
        assert_eq!(installed.installed_area_ids, vec!["area-c"]);
        assert!(installed.created_dynamic_state);
        assert_eq!(world.revision(), revision_before_dynamic_install);
        assert_eq!(
            world
                .deferred_area_load_request(session, Direction::East)
                .unwrap(),
            None
        );
        let scene = world
            .apply(session, PlayerAction::Move(Direction::East))
            .unwrap()
            .scene;
        assert_eq!(scene.title, "area-c:start");
        assert_eq!(scene.items.len(), 1);

        world.restore_lifecycle_checkpoint(lifecycle_checkpoint);
        assert!(!world.area_is_active("area-c"));
        assert_eq!(
            world.character_snapshot(session).unwrap().room_id,
            "area-b:start"
        );
        assert!(matches!(
            world.apply(session, PlayerAction::Move(Direction::East)),
            Err(WorldError::AreaLoadRequired(_))
        ));
    }

    #[test]
    fn unknown_persisted_rooms_fail_closed_without_relocating() {
        let mut world = World::demo();
        let session = SessionId(11);
        assert!(matches!(
            world.connect(
                session,
                CharacterSnapshot {
                    id: CharacterId(102),
                    account_id: AccountId(10),
                    name: "Old Save".into(),
                    room_id: "missing:room".into(),
                    item_locations: BTreeMap::new(),
                },
            ),
            Err(WorldError::InvalidEntity)
        ));
        assert!(!world.sessions.contains_key(&session));
        assert!(!world.character_gameplay.contains_key(&CharacterId(102)));
    }

    #[test]
    fn corrupt_exit_references_return_an_error_instead_of_panicking() {
        let (mut world, session) = connected_world();
        let character_id = world.sessions[&session];
        room_mut(&mut world.entities, world.start_room)
            .exits
            .insert(Direction::West, RoomExit::Resident(character_id));

        assert!(matches!(
            world.scene_for(session),
            Err(WorldError::InvalidEntity)
        ));
        assert!(matches!(
            world.apply(session, PlayerAction::Move(Direction::West)),
            Err(WorldError::InvalidEntity)
        ));
        assert_eq!(
            world.character_snapshot(session).unwrap().room_id,
            "demo:lantern-square"
        );
    }

    #[test]
    fn durable_character_identity_survives_world_changes() {
        let (mut world, session) = connected_world();
        world
            .apply(session, PlayerAction::Move(Direction::North))
            .unwrap();

        let snapshot = world.character_snapshot(session).unwrap();
        assert_eq!(snapshot.id, CharacterId(101));
        assert_eq!(snapshot.account_id, AccountId(10));
        assert_eq!(snapshot.name, "Tester");
    }

    #[test]
    fn a_character_can_reconnect_after_disconnect() {
        let (mut world, first_session) = connected_world();
        let first_scene = world.scene_for(first_session).unwrap();
        let snapshot = world.character_snapshot(first_session).unwrap();
        world.disconnect(first_session);

        let scene = world.connect(SessionId(9), snapshot.clone()).unwrap();
        assert_ne!(scene.view_id, "demo:lantern-square");
        assert_ne!(scene.view_id, first_scene.view_id);
        assert_ne!(scene.items[0].id, first_scene.items[0].id);
        assert_eq!(scene.title, "Lantern Square");

        let mut restarted = World::demo();
        let restarted_scene = restarted.connect(SessionId(1), snapshot).unwrap();
        assert_ne!(restarted_scene.view_id, first_scene.view_id);
        assert_ne!(restarted_scene.items[0].id, first_scene.items[0].id);
    }

    #[test]
    fn a_character_cannot_be_connected_by_two_sessions_at_once() {
        let (mut world, first_session) = connected_world();
        let snapshot = world.character_snapshot(first_session).unwrap();

        assert!(matches!(
            world.connect(SessionId(9), snapshot),
            Err(WorldError::CharacterAlreadyConnected)
        ));
        assert_eq!(
            world.character_snapshot(first_session).unwrap().id,
            CharacterId(101)
        );
    }

    #[test]
    fn rollback_cannot_replace_a_sessions_durable_identity() {
        let (mut world, session) = connected_world();
        let mut foreign = world.character_snapshot(session).unwrap();
        foreign.id = CharacterId(999);
        let revision = world.revision();

        assert!(matches!(
            world.restore_character(session, &foreign, revision),
            Err(WorldError::InvalidEntity)
        ));
        assert_eq!(
            world.character_snapshot(session).unwrap().id,
            CharacterId(101)
        );
    }

    #[test]
    fn builds_a_world_from_a_transport_independent_definition() {
        let image = SceneImage {
            alt: String::new(),
            localized_alt: BTreeMap::new(),
            manifest: Some("/assets/aaaaaaaa/manifest.json".into()),
            avif: None,
            webp: None,
            jpeg: None,
            width: None,
            height: None,
        };
        let mut world = World::from_definition(WorldDefinition {
            start_room_id: "test:start".into(),
            item_prototypes: Vec::new(),
            initial_item_instances: Vec::new(),
            rooms: vec![
                RoomDefinition {
                    stable_id: "test:start".into(),
                    title: "Start".into(),
                    prose: vec!["A complete textual scene.".into()],
                    image: Some(image.clone()),
                    audio: None,
                    exits: vec![ExitDefinition {
                        direction: Direction::North,
                        target_room_id: "test:north".into(),
                    }],
                    item_ids: Vec::new(),
                },
                RoomDefinition {
                    stable_id: "test:north".into(),
                    title: "North".into(),
                    prose: vec!["The northern room.".into()],
                    image: None,
                    audio: None,
                    exits: Vec::new(),
                    item_ids: Vec::new(),
                },
            ],
        })
        .unwrap();
        let session = SessionId(42);
        let scene = world
            .connect(
                session,
                CharacterSnapshot {
                    id: CharacterId(7),
                    account_id: AccountId(3),
                    name: "Builder".into(),
                    room_id: "test:start".into(),
                    item_locations: BTreeMap::new(),
                },
            )
            .unwrap();

        assert_eq!(scene.image, Some(image));
        assert_eq!(scene.exits[0].command, "north");
        assert_eq!(
            world
                .apply(session, PlayerAction::Move(Direction::North))
                .unwrap()
                .scene
                .title,
            "North"
        );
    }

    #[test]
    fn rejects_invalid_world_definition_references() {
        let missing_default = World::from_definition(WorldDefinition {
            start_room_id: "missing".into(),
            item_prototypes: Vec::new(),
            initial_item_instances: Vec::new(),
            rooms: vec![RoomDefinition {
                stable_id: "only".into(),
                title: "Only".into(),
                prose: Vec::new(),
                image: None,
                audio: None,
                exits: Vec::new(),
                item_ids: Vec::new(),
            }],
        });
        assert!(matches!(
            missing_default,
            Err(WorldBuildError::UnknownStartRoom(room)) if room == "missing"
        ));

        let missing_exit = World::from_definition(WorldDefinition {
            start_room_id: "only".into(),
            item_prototypes: Vec::new(),
            initial_item_instances: Vec::new(),
            rooms: vec![RoomDefinition {
                stable_id: "only".into(),
                title: "Only".into(),
                prose: Vec::new(),
                image: None,
                audio: None,
                exits: vec![ExitDefinition {
                    direction: Direction::Down,
                    target_room_id: "void".into(),
                }],
                item_ids: Vec::new(),
            }],
        });
        assert!(matches!(
            missing_exit,
            Err(WorldBuildError::UnknownExitTarget { target_room_id, .. }) if target_room_id == "void"
        ));

        let duplicate_direction = World::from_definition(WorldDefinition {
            start_room_id: "only".into(),
            item_prototypes: Vec::new(),
            initial_item_instances: Vec::new(),
            rooms: vec![RoomDefinition {
                stable_id: "only".into(),
                title: "Only".into(),
                prose: Vec::new(),
                image: None,
                audio: None,
                exits: vec![
                    ExitDefinition {
                        direction: Direction::North,
                        target_room_id: "only".into(),
                    },
                    ExitDefinition {
                        direction: Direction::North,
                        target_room_id: "only".into(),
                    },
                ],
                item_ids: Vec::new(),
            }],
        });
        assert!(matches!(
            duplicate_direction,
            Err(WorldBuildError::DuplicateExitDirection {
                direction: Direction::North,
                ..
            })
        ));
    }

    #[test]
    fn legacy_character_item_state_is_ignored_and_scene_ids_are_opaque() {
        let (world, session) = connected_world();

        let snapshot = world.character_snapshot(session).unwrap();
        assert!(snapshot.item_locations.is_empty());
        let scene = world.scene_for(session).unwrap();
        assert_eq!(scene.items.len(), 1);
        assert_eq!(scene.items[0].label, "blue-glass lantern");
        assert_ne!(scene.items[0].id, "demo:blue-lantern");
        assert!(!scene.items[0].actions[0].command.contains("demo:"));
        assert!(
            scene
                .as_plain_text()
                .contains("You see here: blue-glass lantern.")
        );
    }

    #[test]
    fn take_inventory_and_drop_form_a_shared_world_loop() {
        let (mut world, session) = connected_world();

        let taken = world
            .apply(session, PlayerAction::Take("lantern".into()))
            .unwrap();
        assert_eq!(
            taken.output.as_deref(),
            Some("You take the blue-glass lantern.\r\n")
        );
        assert!(taken.scene.items.is_empty());
        assert!(taken.changed_character.is_none());
        assert!(matches!(
            taken.domain_events.as_slice(),
            [DomainEvent::ItemTaken { .. }]
        ));
        let inventory = world
            .apply(session, PlayerAction::Inventory)
            .unwrap()
            .output
            .unwrap();
        assert_eq!(
            redact_and_validate_opaque_item_refs(&inventory),
            "Capacity: 1/24 slots; 1000/50000 g.\r\nYou are carrying:\r\n- 1 × blue-glass lantern [common; level 1; 1000 g each; 1 stack slot; 1000 g total; policy transferable]\r\n"
        );

        world
            .apply(session, PlayerAction::Move(Direction::North))
            .unwrap();
        let dropped = world
            .apply(session, PlayerAction::Drop("lamp".into()))
            .unwrap();
        assert_ne!(dropped.scene.items[0].id, "demo:blue-lantern");
        assert!(dropped.changed_character.is_none());
        let saved = world.character_snapshot(session).unwrap();

        world.disconnect(session);
        let reconnected = world.connect(SessionId(77), saved).unwrap();
        assert_eq!(reconnected.title, "The North Gate");
        assert_ne!(reconnected.items[0].id, "demo:blue-lantern");
    }

    #[test]
    fn examine_is_read_only_and_works_for_visible_or_carried_items() {
        let (mut world, session) = connected_world();
        let before = world.character_snapshot(session).unwrap();
        let examined = world
            .apply(session, PlayerAction::Examine("blue".into()))
            .unwrap();
        assert!(examined.output.unwrap().contains("Cold blue light"));
        assert!(examined.changed_character.is_none());
        assert_eq!(world.character_snapshot(session).unwrap(), before);

        world
            .apply(session, PlayerAction::Take("lantern".into()))
            .unwrap();
        assert!(
            world
                .apply(session, PlayerAction::Examine("lamp".into()))
                .unwrap()
                .output
                .unwrap()
                .contains("rain-speckled glass")
        );
    }

    #[test]
    fn absent_items_fail_without_mutation() {
        let (mut world, session) = connected_world();
        let before = world.character_snapshot(session).unwrap();
        let revision = world.revision();

        assert!(matches!(
            world.apply(session, PlayerAction::Take("sword".into())),
            Err(WorldError::ItemNotFound(query)) if query == "sword"
        ));
        assert!(matches!(
            world.apply(session, PlayerAction::Drop("lantern".into())),
            Err(WorldError::ItemNotFound(query)) if query == "lantern"
        ));
        assert_eq!(world.character_snapshot(session).unwrap(), before);
        assert_eq!(world.revision(), revision);
    }

    #[test]
    fn ambiguous_item_names_fail_without_mutation() {
        let mut world = World::from_definition(WorldDefinition {
            start_room_id: "test:start".into(),
            item_prototypes: vec![
                item_prototype("test:blue", "blue lantern", "Blue.", &["lantern"]),
                item_prototype("test:brass", "brass lantern", "Brass.", &["lantern"]),
            ],
            initial_item_instances: vec![
                item_instance(
                    "test:blue-lantern",
                    "test:blue",
                    InitialItemLocation::Room("test:start".into()),
                ),
                item_instance(
                    "test:brass-lantern",
                    "test:brass",
                    InitialItemLocation::Room("test:start".into()),
                ),
            ],
            rooms: vec![RoomDefinition {
                stable_id: "test:start".into(),
                title: "Start".into(),
                prose: Vec::new(),
                image: None,
                audio: None,
                exits: Vec::new(),
                item_ids: vec!["test:blue-lantern".into(), "test:brass-lantern".into()],
            }],
        })
        .unwrap();
        let session = SessionId(8);
        world
            .connect(
                session,
                CharacterSnapshot {
                    id: CharacterId(8),
                    account_id: AccountId(8),
                    name: "Chooser".into(),
                    room_id: "test:start".into(),
                    item_locations: BTreeMap::new(),
                },
            )
            .unwrap();
        let before = world.character_snapshot(session).unwrap();

        assert!(matches!(
            world.apply(session, PlayerAction::Take("lantern".into())),
            Err(WorldError::AmbiguousItem { .. })
        ));
        assert_eq!(world.character_snapshot(session).unwrap(), before);
    }

    #[test]
    fn taking_an_item_is_observed_globally() {
        let (mut world, first) = connected_world();
        let second = SessionId(17);
        world
            .connect(
                second,
                CharacterSnapshot {
                    id: CharacterId(202),
                    account_id: AccountId(20),
                    name: "Second".into(),
                    room_id: world.start_room_id().to_owned(),
                    item_locations: BTreeMap::new(),
                },
            )
            .unwrap();

        world
            .apply(first, PlayerAction::Take("lantern".into()))
            .unwrap();
        assert!(world.scene_for(first).unwrap().items.is_empty());
        assert!(world.scene_for(second).unwrap().items.is_empty());
        assert_eq!(
            world.inventory_as_plain_text(second).unwrap(),
            "Capacity: 0/24 slots; 0/50000 g.\r\nYou are carrying nothing.\r\n"
        );
    }

    #[test]
    fn one_domain_event_projects_differently_by_perception() {
        let (mut world, actor) = connected_world();
        let peer = SessionId(17);
        world
            .connect(
                peer,
                CharacterSnapshot {
                    id: CharacterId(202),
                    account_id: AccountId(20),
                    name: "Witness".into(),
                    room_id: world.start_room_id().to_owned(),
                    item_locations: BTreeMap::new(),
                },
            )
            .unwrap();
        let distant = SessionId(18);
        world
            .connect(
                distant,
                CharacterSnapshot {
                    id: CharacterId(303),
                    account_id: AccountId(30),
                    name: "Archivist".into(),
                    room_id: "demo:archive".into(),
                    item_locations: BTreeMap::new(),
                },
            )
            .unwrap();

        let actor_scene = world.scene_for(actor).unwrap();
        let peer_scene = world.scene_for(peer).unwrap();
        assert_ne!(actor_scene.view_id, peer_scene.view_id);
        assert_ne!(actor_scene.items[0].id, peer_scene.items[0].id);
        assert!(!actor_scene.items[0].id.contains("demo:"));
        assert!(!actor_scene.items[0].actions[1].command.contains("demo:"));
        let distant_revision = world.scene_for(distant).unwrap().revision;

        let action = parse_action(&actor_scene.items[0].actions[1].command).unwrap();
        let outcome = world.apply(actor, action).unwrap();
        let event = outcome.domain_events.first().unwrap();
        let actor_view = world.project_event(actor, event).unwrap().unwrap();
        let peer_view = world.project_event(peer, event).unwrap().unwrap();

        assert_eq!(actor_view.text, "You take the blue-glass lantern.");
        assert_eq!(peer_view.text, "Tester takes the blue-glass lantern.");
        assert_ne!(actor_view.actor_id, peer_view.actor_id);
        assert_ne!(actor_view.item_id, peer_view.item_id);
        assert!(!actor_view.actor_id.contains("101"));
        assert!(!actor_view.item_id.as_deref().unwrap().contains("demo:"));
        assert!(world.project_event(distant, event).unwrap().is_none());
        assert_eq!(world.scene_for(actor).unwrap().revision, 2);
        assert_eq!(world.scene_for(peer).unwrap().revision, 2);
        assert_eq!(world.scene_for(distant).unwrap().revision, distant_revision);
    }

    #[test]
    fn a_dropped_item_becomes_visible_to_other_characters() {
        let (mut world, actor) = connected_world();
        let peer = SessionId(17);
        world
            .connect(
                peer,
                CharacterSnapshot {
                    id: CharacterId(202),
                    account_id: AccountId(20),
                    name: "Witness".into(),
                    room_id: world.start_room_id().to_owned(),
                    item_locations: BTreeMap::new(),
                },
            )
            .unwrap();
        world
            .apply(actor, PlayerAction::Take("lantern".into()))
            .unwrap();
        world
            .apply(actor, PlayerAction::Move(Direction::North))
            .unwrap();
        world
            .apply(actor, PlayerAction::Drop("lantern".into()))
            .unwrap();

        assert_eq!(world.scene_for(actor).unwrap().items.len(), 1);
        assert!(world.scene_for(peer).unwrap().items.is_empty());
        world
            .apply(peer, PlayerAction::Move(Direction::North))
            .unwrap();
        assert_eq!(world.scene_for(peer).unwrap().items.len(), 1);
        assert_eq!(
            world.scene_for(peer).unwrap().items[0].label,
            "blue-glass lantern"
        );
    }

    #[test]
    fn rollback_restores_movement_and_observer_revision() {
        let (mut world, session) = connected_world();
        let before = world.character_snapshot(session).unwrap();
        let view_revision = world.scene_for(session).unwrap().revision;
        let revision = world.revision();
        world
            .apply(session, PlayerAction::Move(Direction::North))
            .unwrap();

        world.restore_character(session, &before, revision).unwrap();

        assert_eq!(world.character_snapshot(session).unwrap(), before);
        assert_eq!(world.revision(), revision);
        assert_eq!(world.scene_for(session).unwrap().revision, view_revision);
        assert_eq!(world.scene_for(session).unwrap().items.len(), 1);
        assert_eq!(
            world.inventory_as_plain_text(session).unwrap(),
            "Capacity: 0/24 slots; 0/50000 g.\r\nYou are carrying nothing.\r\n"
        );
    }

    #[test]
    fn replay_reconstructs_take_and_drop_with_the_owner_disconnected() {
        let owner = CharacterId(404);
        let taken = DomainEvent::ItemTaken {
            actor_id: owner,
            actor_name: "Offline Hero".into(),
            item_id: "demo:blue-lantern".into(),
            item_name: "blue-glass lantern".into(),
            room_id: "demo:lantern-square".into(),
        };
        let dropped = DomainEvent::ItemDropped {
            actor_id: owner,
            actor_name: "Offline Hero".into(),
            item_id: "demo:blue-lantern".into(),
            item_name: "blue-glass lantern".into(),
            room_id: "demo:north-gate".into(),
        };

        let mut carried_world = World::demo();
        carried_world.replay_persisted_event(&taken).unwrap();
        let owner_session = SessionId(404);
        carried_world
            .connect(
                owner_session,
                CharacterSnapshot {
                    id: owner,
                    account_id: AccountId(40),
                    name: "Offline Hero".into(),
                    room_id: "demo:archive".into(),
                    item_locations: BTreeMap::new(),
                },
            )
            .unwrap();
        let inventory = carried_world
            .inventory_as_plain_text(owner_session)
            .unwrap();
        assert_eq!(
            redact_and_validate_opaque_item_refs(&inventory),
            "Capacity: 1/24 slots; 1000/50000 g.\r\nYou are carrying:\r\n- 1 × blue-glass lantern [common; level 1; 1000 g each; 1 stack slot; 1000 g total; policy transferable]\r\n"
        );

        let mut dropped_world = World::demo();
        dropped_world.replay_persisted_event(&taken).unwrap();
        dropped_world.replay_persisted_event(&dropped).unwrap();
        let witness = SessionId(405);
        let scene = dropped_world
            .connect(
                witness,
                CharacterSnapshot {
                    id: CharacterId(405),
                    account_id: AccountId(40),
                    name: "Witness".into(),
                    room_id: "demo:north-gate".into(),
                    item_locations: BTreeMap::new(),
                },
            )
            .unwrap();
        assert_eq!(scene.items.len(), 1);
        assert_eq!(scene.items[0].label, "blue-glass lantern");
        assert_eq!(dropped_world.revision(), 3);
    }

    #[test]
    fn replay_rejects_invalid_metadata_and_transitions_atomically() {
        let mut world = World::demo();
        let invalid_name = DomainEvent::ItemTaken {
            actor_id: CharacterId(404),
            actor_name: "Offline Hero".into(),
            item_id: "demo:blue-lantern".into(),
            item_name: "forged name".into(),
            room_id: "demo:lantern-square".into(),
        };
        let invalid_transition = DomainEvent::ItemDropped {
            actor_id: CharacterId(404),
            actor_name: "Offline Hero".into(),
            item_id: "demo:blue-lantern".into(),
            item_name: "blue-glass lantern".into(),
            room_id: "demo:lantern-square".into(),
        };
        let unknown_item = DomainEvent::ItemTaken {
            actor_id: CharacterId(404),
            actor_name: "Offline Hero".into(),
            item_id: "demo:forged-item".into(),
            item_name: "blue-glass lantern".into(),
            room_id: "demo:lantern-square".into(),
        };
        let unknown_room = DomainEvent::ItemTaken {
            actor_id: CharacterId(404),
            actor_name: "Offline Hero".into(),
            item_id: "demo:blue-lantern".into(),
            item_name: "blue-glass lantern".into(),
            room_id: "demo:forged-room".into(),
        };

        for invalid in [&invalid_name, &unknown_item, &unknown_room] {
            assert!(matches!(
                world.replay_persisted_event(invalid),
                Err(WorldError::InvalidDomainEvent)
            ));
        }
        assert!(matches!(
            world.replay_persisted_event(&invalid_transition),
            Err(WorldError::InvalidDomainTransition)
        ));
        assert_eq!(world.revision(), 1);
        let session = SessionId(406);
        assert_eq!(
            world
                .connect(
                    session,
                    CharacterSnapshot {
                        id: CharacterId(406),
                        account_id: AccountId(40),
                        name: "Witness".into(),
                        room_id: "demo:lantern-square".into(),
                        item_locations: BTreeMap::new(),
                    },
                )
                .unwrap()
                .items
                .len(),
            1
        );
    }

    #[test]
    fn rollback_domain_events_restores_shared_state_and_view_revisions() {
        let (mut world, actor) = connected_world();
        let peer = SessionId(407);
        world
            .connect(
                peer,
                CharacterSnapshot {
                    id: CharacterId(407),
                    account_id: AccountId(40),
                    name: "Witness".into(),
                    room_id: "demo:lantern-square".into(),
                    item_locations: BTreeMap::new(),
                },
            )
            .unwrap();
        let domain_revision = world.revision();
        let actor_revision = world.scene_for(actor).unwrap().revision;
        let peer_revision = world.scene_for(peer).unwrap().revision;
        let outcome = world
            .apply(actor, PlayerAction::Take("lantern".into()))
            .unwrap();
        assert!(world.scene_for(peer).unwrap().items.is_empty());

        world
            .rollback_domain_events(&outcome.domain_events)
            .unwrap();

        assert_eq!(world.revision(), domain_revision);
        assert_eq!(world.scene_for(actor).unwrap().revision, actor_revision);
        assert_eq!(world.scene_for(peer).unwrap().revision, peer_revision);
        assert_eq!(world.scene_for(actor).unwrap().items.len(), 1);
        assert_eq!(world.scene_for(peer).unwrap().items.len(), 1);
        assert_eq!(
            world.inventory_as_plain_text(actor).unwrap(),
            "Capacity: 0/24 slots; 0/50000 g.\r\nYou are carrying nothing.\r\n"
        );
    }

    #[test]
    fn invalid_rollback_batch_does_not_partially_mutate_state() {
        let (mut world, actor) = connected_world();
        let outcome = world
            .apply(actor, PlayerAction::Take("lantern".into()))
            .unwrap();
        let revision = world.revision();
        let view_revision = world.scene_for(actor).unwrap().revision;
        let mut forged = outcome.domain_events.clone();
        if let DomainEvent::ItemTaken { item_name, .. } = &mut forged[0] {
            *item_name = "forged name".into();
        }

        assert!(matches!(
            world.rollback_domain_events(&forged),
            Err(WorldError::InvalidDomainEvent)
        ));
        assert_eq!(world.revision(), revision);
        assert_eq!(world.scene_for(actor).unwrap().revision, view_revision);
        let inventory = world.inventory_as_plain_text(actor).unwrap();
        assert_eq!(
            redact_and_validate_opaque_item_refs(&inventory),
            "Capacity: 1/24 slots; 1000/50000 g.\r\nYou are carrying:\r\n- 1 × blue-glass lantern [common; level 1; 1000 g each; 1 stack slot; 1000 g total; policy transferable]\r\n"
        );
    }

    #[test]
    fn parses_phase_two_gameplay_commands() {
        assert_eq!(
            parse_action("attack guardian"),
            Ok(PlayerAction::Attack("guardian".into()))
        );
        assert_eq!(
            parse_action("use potion"),
            Ok(PlayerAction::Use("potion".into()))
        );
        assert_eq!(
            parse_action("wear sword"),
            Ok(PlayerAction::Equip("sword".into()))
        );
        assert_eq!(
            parse_action("remove weapon"),
            Ok(PlayerAction::Unequip("weapon".into()))
        );
        assert_eq!(
            parse_action("attack"),
            Err(ParseActionError::MissingTarget { command: "attack" })
        );
    }

    #[test]
    fn deterministic_explore_combat_loot_and_progression_cycle() {
        let (mut world, session) = phase_two_world(42, 2, 14);
        assert_eq!(world.scene_for(session).unwrap().title, "Sanctuary");
        world
            .apply(session, PlayerAction::Take("sword".into()))
            .unwrap();
        let equipped = world
            .apply(session, PlayerAction::Equip("sword".into()))
            .unwrap();
        assert!(matches!(
            &equipped.domain_events[0],
            DomainEvent::Gameplay {
                event: GameplayEvent::ItemEquipped { .. }
            }
        ));
        world
            .apply(session, PlayerAction::Move(Direction::North))
            .unwrap();
        let arena = world.scene_for(session).unwrap();
        assert_eq!(arena.title, "Old Arena");
        assert_eq!(arena.npcs.len(), 1);
        assert!(!arena.npcs[0].id.contains("game:"));
        assert!(!arena.npcs[0].actions[0].command.contains("game:"));
        let attack = parse_action(&arena.npcs[0].actions[0].command).unwrap();
        world.apply(session, attack).unwrap();

        let mut combat_events = Vec::new();
        for _ in 0..5 {
            let events = world.advance_tick().unwrap();
            combat_events.extend(events);
            if world.scene_for(session).unwrap().npcs.is_empty() {
                break;
            }
        }
        assert!(combat_events.iter().any(|event| matches!(
            event,
            DomainEvent::Gameplay {
                event: GameplayEvent::NpcDefeated { .. }
            }
        )));
        assert!(combat_events.iter().any(|event| matches!(
            event,
            DomainEvent::Gameplay {
                event: GameplayEvent::LootDropped { .. }
            }
        )));
        let gameplay = world.character_gameplay_snapshot(session).unwrap();
        assert_eq!(gameplay.level, 2);
        assert_eq!(gameplay.experience, 120);
        assert_eq!(gameplay.skills.get("combat"), Some(&11));
        assert!(
            world
                .player_state_view(session)
                .unwrap()
                .character
                .skills
                .iter()
                .any(|skill| skill.id == "combat" && skill.label == "Combat" && skill.rank == 11)
        );
        let loot_scene = world.scene_for(session).unwrap();
        assert_eq!(loot_scene.items.len(), 1);
        assert_eq!(loot_scene.items[0].label, "silver fang");
        world
            .apply(session, PlayerAction::Take("fang".into()))
            .unwrap();
        assert!(
            world
                .inventory_as_plain_text(session)
                .unwrap()
                .contains("silver fang")
        );

        for _ in 0..3 {
            world.advance_tick().unwrap();
        }
        assert_eq!(world.scene_for(session).unwrap().npcs.len(), 1);
    }

    #[test]
    fn seeded_rng_produces_identical_combat_events() {
        let (mut first, first_session) = phase_two_world(77, 3, 30);
        let (mut second, second_session) = phase_two_world(77, 3, 30);
        for (world, session) in [(&mut first, first_session), (&mut second, second_session)] {
            world
                .apply(session, PlayerAction::Move(Direction::North))
                .unwrap();
            world
                .apply(session, PlayerAction::Attack("guardian".into()))
                .unwrap();
        }

        assert_eq!(
            first.advance_tick().unwrap(),
            second.advance_tick().unwrap()
        );
        assert_eq!(
            first.advance_tick().unwrap(),
            second.advance_tick().unwrap()
        );
        assert_eq!(first.logical_tick(), second.logical_tick());
    }

    #[test]
    fn use_effect_equip_and_unequip_update_gameplay_state() {
        let (mut world, session) = phase_two_world(9, 2, 20);
        let mut wounded = world.character_gameplay_snapshot(session).unwrap();
        wounded.vitals.health = 8;
        world
            .restore_character_gameplay(CharacterId(900), wounded)
            .unwrap();
        world
            .apply(session, PlayerAction::Take("potion".into()))
            .unwrap();
        let used = world
            .apply(session, PlayerAction::Use("potion".into()))
            .unwrap();
        assert!(used.domain_events.iter().any(|event| matches!(
            event,
            DomainEvent::Gameplay {
                event: GameplayEvent::EffectApplied { .. }
            }
        )));
        assert_eq!(
            world
                .character_gameplay_snapshot(session)
                .unwrap()
                .vitals
                .health,
            13
        );
        world.advance_tick().unwrap();
        let second_tick = world.advance_tick().unwrap();
        assert_eq!(
            world
                .character_gameplay_snapshot(session)
                .unwrap()
                .vitals
                .health,
            17
        );
        assert!(second_tick.iter().any(|event| matches!(
            event,
            DomainEvent::Gameplay {
                event: GameplayEvent::EffectExpired { .. }
            }
        )));

        world
            .apply(session, PlayerAction::Take("sword".into()))
            .unwrap();
        world
            .apply(session, PlayerAction::Equip("sword".into()))
            .unwrap();
        assert_eq!(
            world
                .character_gameplay_snapshot(session)
                .unwrap()
                .equipment[&EquipmentSlot::Weapon],
            ItemInstanceId("game:sword".into())
        );
        world
            .apply(session, PlayerAction::Unequip("weapon".into()))
            .unwrap();
        assert!(
            world
                .character_gameplay_snapshot(session)
                .unwrap()
                .equipment
                .is_empty()
        );
    }

    #[test]
    fn defeat_recovers_at_start_after_deterministic_ticks() {
        let (mut world, session) = phase_two_world(5, 50, 100);
        world
            .apply(session, PlayerAction::Move(Direction::North))
            .unwrap();
        world
            .apply(session, PlayerAction::Attack("guardian".into()))
            .unwrap();
        let defeat_tick = world.advance_tick().unwrap();
        assert!(defeat_tick.iter().any(|event| matches!(
            event,
            DomainEvent::Gameplay {
                event: GameplayEvent::CharacterDefeated { .. }
            }
        )));
        assert_eq!(
            world
                .character_gameplay_snapshot(session)
                .unwrap()
                .vitals
                .health,
            0
        );
        assert!(matches!(
            world.apply(session, PlayerAction::Move(Direction::South)),
            Err(WorldError::GameplayUnavailable)
        ));
        for _ in 0..3 {
            world.advance_tick().unwrap();
        }
        let recovered = world.character_gameplay_snapshot(session).unwrap();
        assert_eq!(recovered.vitals.health, recovered.vitals.max_health);
        assert!(recovered.recovery_at_tick.is_none());
        assert_eq!(world.scene_for(session).unwrap().title, "Sanctuary");
    }

    #[test]
    fn portable_snapshots_resume_combat_loot_xp_equipment_effects_and_rng() {
        let (mut original, session) = phase_two_world(41, 2, 3);
        original
            .apply(session, PlayerAction::Take("potion".into()))
            .unwrap();
        original
            .apply(session, PlayerAction::Use("potion".into()))
            .unwrap();
        original
            .apply(session, PlayerAction::Take("sword".into()))
            .unwrap();
        original
            .apply(session, PlayerAction::Equip("sword".into()))
            .unwrap();
        original
            .apply(session, PlayerAction::Move(Direction::North))
            .unwrap();
        original
            .apply(session, PlayerAction::Attack("guardian".into()))
            .unwrap();
        original.advance_tick().unwrap();

        let runtime_json = serde_json::to_string(&original.runtime_snapshot()).unwrap();
        let durable_json =
            serde_json::to_string(&original.durable_character_snapshot(session).unwrap()).unwrap();
        let runtime: WorldRuntimeSnapshot = serde_json::from_str(&runtime_json).unwrap();
        let durable: DurableCharacterSnapshot = serde_json::from_str(&durable_json).unwrap();

        let (mut resumed, resumed_session) = phase_two_world(999, 2, 3);
        let revision = runtime.revision;
        resumed.restore_runtime_snapshot(runtime).unwrap();
        resumed
            .restore_character(resumed_session, &durable.character, revision)
            .unwrap();
        resumed
            .restore_character_gameplay(CharacterId(900), durable.gameplay)
            .unwrap();

        assert_eq!(resumed.runtime_snapshot(), original.runtime_snapshot());
        assert_eq!(
            resumed
                .character_gameplay_snapshot(resumed_session)
                .unwrap(),
            original.character_gameplay_snapshot(session).unwrap()
        );
        let resumed_inventory = resumed.inventory_as_plain_text(resumed_session).unwrap();
        let original_inventory = original.inventory_as_plain_text(session).unwrap();
        assert_ne!(resumed_inventory, original_inventory);
        assert_eq!(
            redact_and_validate_opaque_item_refs(&resumed_inventory),
            redact_and_validate_opaque_item_refs(&original_inventory)
        );
        assert_eq!(
            resumed.advance_tick().unwrap(),
            original.advance_tick().unwrap()
        );
    }

    #[test]
    fn portable_snapshots_resume_defeat_recovery_and_npc_respawn_ticks() {
        let (mut original, session) = phase_two_world(5, 50, 100);
        original
            .apply(session, PlayerAction::Move(Direction::North))
            .unwrap();
        original
            .apply(session, PlayerAction::Attack("guardian".into()))
            .unwrap();
        original.advance_tick().unwrap();
        assert!(
            original
                .character_gameplay_snapshot(session)
                .unwrap()
                .recovery_at_tick
                .is_some()
        );

        let runtime = original.runtime_snapshot();
        let durable = original.durable_character_snapshot(session).unwrap();
        let (mut resumed, resumed_session) = phase_two_world(999, 50, 100);
        let revision = runtime.revision;
        resumed.restore_runtime_snapshot(runtime).unwrap();
        resumed
            .restore_character(resumed_session, &durable.character, revision)
            .unwrap();
        resumed
            .restore_character_gameplay(CharacterId(900), durable.gameplay)
            .unwrap();

        for _ in 0..4 {
            assert_eq!(
                resumed.advance_tick().unwrap(),
                original.advance_tick().unwrap()
            );
        }
        assert_eq!(
            resumed
                .character_gameplay_snapshot(resumed_session)
                .unwrap(),
            original.character_gameplay_snapshot(session).unwrap()
        );
        assert_eq!(
            resumed.scene_for(resumed_session).unwrap().title,
            "Sanctuary"
        );
    }

    #[test]
    fn checkpoint_restores_all_gameplay_and_observer_state() {
        let (mut world, session) = phase_two_world(17, 2, 20);
        world
            .apply(session, PlayerAction::Take("potion".into()))
            .unwrap();
        let before_runtime = world.runtime_snapshot();
        let before_gameplay = world.character_gameplay_snapshot(session).unwrap();
        let before_scene = world.scene_for(session).unwrap();
        let checkpoint = world.checkpoint();

        world
            .apply(session, PlayerAction::Use("potion".into()))
            .unwrap();
        world.advance_tick().unwrap();
        world.restore_checkpoint(checkpoint);

        assert_eq!(world.runtime_snapshot(), before_runtime);
        assert_eq!(
            world.character_gameplay_snapshot(session).unwrap(),
            before_gameplay
        );
        let restored_scene = world.scene_for(session).unwrap();
        assert_eq!(restored_scene.view_id, before_scene.view_id);
        assert_eq!(restored_scene.revision, before_scene.revision);
        assert_eq!(restored_scene.title, before_scene.title);
        assert_eq!(restored_scene.items, before_scene.items);
        assert_eq!(restored_scene.npcs, before_scene.npcs);
    }

    #[test]
    fn combat_view_events_are_opaque_and_perception_scoped() {
        let (mut world, session) = phase_two_world(13, 2, 20);
        world
            .apply(session, PlayerAction::Move(Direction::North))
            .unwrap();
        world
            .apply(session, PlayerAction::Attack("guardian".into()))
            .unwrap();
        let events = world.advance_tick().unwrap();
        let damage = events
            .iter()
            .find(|event| {
                matches!(
                    event,
                    DomainEvent::Gameplay {
                        event: GameplayEvent::NpcDamaged { .. }
                    }
                )
            })
            .unwrap();
        let view = world.project_event(session, damage).unwrap().unwrap();
        assert_ne!(view.actor_id, "900");
        assert!(!view.target_id.as_deref().unwrap().contains("game:"));
        assert!(!view.text.contains("game:"));
    }

    #[test]
    fn private_narration_uses_only_the_authorized_observers_opaque_identity() {
        let (world, session) = phase_two_world(13, 2, 20);
        let view = world
            .project_private_narration(session, "Only you hear the archive whisper.")
            .unwrap();

        assert_eq!(view.kind, ViewEventKind::Gameplay);
        assert_eq!(view.text, "Only you hear the archive whisper.");
        assert!(!view.actor_id.contains("900"));
        assert!(view.target_id.is_none());
        assert!(view.item_id.is_none());
        assert!(matches!(
            world.project_private_narration(session, "\u{1b}[31munsafe"),
            Err(WorldError::InvalidScriptEffect)
        ));
    }

    #[test]
    fn internal_clock_ticks_refresh_but_do_not_narrate_observer_views() {
        let (mut world, session) = phase_two_world(13, 2, 20);
        let before = world.scene_for(session).unwrap().revision;

        let events = world.advance_tick().unwrap();
        let tick = events
            .iter()
            .find(|event| {
                matches!(
                    event,
                    DomainEvent::Gameplay {
                        event: GameplayEvent::TickAdvanced { .. }
                    }
                )
            })
            .unwrap();

        assert_eq!(world.project_event(session, tick).unwrap(), None);
        assert!(world.scene_for(session).unwrap().revision > before);
    }

    #[test]
    fn hidden_item_policy_targets_one_instance_not_its_reusable_prototype() {
        let mut world = World::from_definition(WorldDefinition {
            start_room_id: "test:start".into(),
            item_prototypes: vec![item_prototype(
                "test:lantern",
                "blue lantern",
                "A cold blue lantern.",
                &["lantern"],
            )],
            initial_item_instances: vec![item_instance(
                "test:lantern:one",
                "test:lantern",
                InitialItemLocation::Room("test:start".into()),
            )],
            rooms: vec![RoomDefinition {
                stable_id: "test:start".into(),
                title: "Start".into(),
                prose: Vec::new(),
                image: None,
                audio: None,
                exits: Vec::new(),
                item_ids: Vec::new(),
            }],
        })
        .unwrap();
        assert!(matches!(
            world.configure_hidden_item("test:lantern", 100),
            Err(WorldError::InvalidGameplayDefinition)
        ));
        world
            .configure_hidden_item("test:lantern:one", 100)
            .unwrap();
        let session = SessionId(777);
        world
            .connect(
                session,
                CharacterSnapshot {
                    id: CharacterId(777),
                    account_id: AccountId(777),
                    name: "Observer".into(),
                    room_id: "test:start".into(),
                    item_locations: BTreeMap::new(),
                },
            )
            .unwrap();

        assert!(world.scene_for(session).unwrap().items.is_empty());
    }

    #[test]
    fn perception_authorizes_scenes_actions_maps_and_events_before_projection() {
        let (mut world, actor) = phase_two_world(13, 2, 20);
        world.configure_hidden_item("game:sword", 20).unwrap();
        world
            .configure_secret_exit("game:start", Direction::North, 20)
            .unwrap();
        world.configure_hidden_npc("game:guardian", 20).unwrap();
        world
            .configure_room_light("game:arena", LightLevel::Dark)
            .unwrap();
        let arena_entity = world.stable_rooms["game:arena"];
        let arena = room_mut(&mut world.entities, arena_entity);
        arena.image = Some(SceneImage {
            alt: "A moonless arena.".into(),
            localized_alt: BTreeMap::from([("en".into(), "A moonless arena.".into())]),
            manifest: None,
            avif: Some("/assets/arena.avif".into()),
            webp: None,
            jpeg: None,
            width: Some(640),
            height: Some(360),
        });
        arena.audio = Some(SceneAudio {
            channel: "ambience".into(),
            url: format!("/assets/{}/arena.opus", "a".repeat(64)),
            looped: true,
        });

        let start_observer = SessionId(901);
        world
            .connect(
                start_observer,
                CharacterSnapshot {
                    id: CharacterId(901),
                    account_id: AccountId(91),
                    name: "Scout".into(),
                    room_id: "game:start".into(),
                    item_locations: BTreeMap::new(),
                },
            )
            .unwrap();
        let low_scene = world.scene_for(start_observer).unwrap();
        assert!(low_scene.exits.is_empty());
        assert_eq!(
            low_scene
                .items
                .iter()
                .map(|item| item.label.as_str())
                .collect::<Vec<_>>(),
            vec!["red potion"]
        );
        assert!(matches!(
            world.apply(start_observer, PlayerAction::Move(Direction::North)),
            Err(WorldError::NoExit)
        ));
        assert!(matches!(
            world.apply(start_observer, PlayerAction::Take("sword".into())),
            Err(WorldError::ItemNotFound(_))
        ));
        assert!(
            world
                .player_state_view(start_observer)
                .unwrap()
                .map
                .paths
                .is_empty()
        );

        world.set_observer_role(actor, AccountRole::Admin).unwrap();
        let admin_scene = world.scene_for(actor).unwrap();
        assert_eq!(admin_scene.exits.len(), 1);
        assert!(
            admin_scene
                .items
                .iter()
                .any(|item| item.label == "iron sword")
        );
        assert!(!admin_scene.exits[0].id.contains("game:"));

        let item_event = world
            .apply(actor, PlayerAction::Take("sword".into()))
            .unwrap()
            .domain_events
            .remove(0);
        assert_eq!(
            world.project_event(start_observer, &item_event).unwrap(),
            None
        );
        let move_event = world
            .apply(actor, PlayerAction::Move(Direction::North))
            .unwrap()
            .domain_events
            .remove(0);
        assert_eq!(
            world.project_event(start_observer, &move_event).unwrap(),
            None
        );

        let arena_observer = SessionId(902);
        world
            .connect(
                arena_observer,
                CharacterSnapshot {
                    id: CharacterId(902),
                    account_id: AccountId(92),
                    name: "Listener".into(),
                    room_id: "game:arena".into(),
                    item_locations: BTreeMap::new(),
                },
            )
            .unwrap();
        let dark_scene = world.scene_for(arena_observer).unwrap();
        assert!(dark_scene.npcs.is_empty());
        assert!(!dark_scene.prose.join(" ").contains("Claw marks"));
        assert!(dark_scene.image.is_none());
        assert!(dark_scene.audio.is_some());

        let combat_event = world
            .apply(actor, PlayerAction::Attack("guardian".into()))
            .unwrap()
            .domain_events
            .remove(0);
        assert_eq!(
            world.project_event(arena_observer, &combat_event).unwrap(),
            None
        );

        let mut perceptive = world.character_gameplay_snapshot(arena_observer).unwrap();
        perceptive.skills.insert("perception".into(), 100);
        perceptive.script_flags.insert("sense.darkvision".into());
        world
            .restore_character_gameplay(CharacterId(902), perceptive.clone())
            .unwrap();
        let darkvision_scene = world.scene_for(arena_observer).unwrap();
        assert_eq!(darkvision_scene.npcs.len(), 1);
        assert!(darkvision_scene.prose.join(" ").contains("Claw marks"));
        assert!(darkvision_scene.image.is_some());

        perceptive.script_flags.insert("condition.blind".into());
        world
            .restore_character_gameplay(CharacterId(902), perceptive.clone())
            .unwrap();
        let blind_scene = world.scene_for(arena_observer).unwrap();
        assert!(blind_scene.npcs.is_empty());
        assert!(blind_scene.image.is_none());
        assert!(blind_scene.audio.is_some());
        perceptive.script_flags.insert("sense.tremorsense".into());
        world
            .restore_character_gameplay(CharacterId(902), perceptive.clone())
            .unwrap();
        assert_eq!(world.scene_for(arena_observer).unwrap().npcs.len(), 1);
        perceptive.script_flags.insert("condition.deaf".into());
        world
            .restore_character_gameplay(CharacterId(902), perceptive)
            .unwrap();
        assert!(world.scene_for(arena_observer).unwrap().audio.is_none());
    }

    #[test]
    fn spatial_voice_is_directed_bounded_secret_safe_and_deafness_only_filters_receivers() {
        let room = |id: &str, exits: Vec<(Direction, &str)>| RoomDefinition {
            stable_id: id.into(),
            title: id.into(),
            prose: vec![format!("Room {id}.")],
            image: None,
            audio: None,
            item_ids: Vec::new(),
            exits: exits
                .into_iter()
                .map(|(direction, target_room_id)| ExitDefinition {
                    direction,
                    target_room_id: target_room_id.into(),
                })
                .collect(),
        };
        let mut world = World::from_definition(WorldDefinition {
            start_room_id: "voice:r0".into(),
            item_prototypes: Vec::new(),
            initial_item_instances: Vec::new(),
            rooms: vec![
                room("voice:r0", vec![(Direction::East, "voice:r1")]),
                room(
                    "voice:r1",
                    vec![(Direction::West, "voice:r0"), (Direction::East, "voice:r2")],
                ),
                room(
                    "voice:r2",
                    vec![(Direction::West, "voice:r1"), (Direction::East, "voice:r3")],
                ),
                room(
                    "voice:r3",
                    vec![(Direction::West, "voice:r2"), (Direction::East, "voice:r4")],
                ),
                room("voice:r4", vec![(Direction::West, "voice:r3")]),
                // This one-way edge points toward the origin. A directed BFS
                // from r0 must not infer an unauthored reverse connection.
                room("voice:inbound", vec![(Direction::West, "voice:r0")]),
            ],
        })
        .unwrap();
        let connect =
            |world: &mut World, session: u64, character: i64, name: &str, room_id: &str| {
                world
                    .connect(
                        SessionId(session),
                        CharacterSnapshot {
                            id: CharacterId(character),
                            account_id: AccountId(character),
                            name: name.into(),
                            room_id: room_id.into(),
                            item_locations: BTreeMap::new(),
                        },
                    )
                    .unwrap();
            };
        connect(&mut world, 1, 1, "Actor", "voice:r0");
        connect(&mut world, 2, 2, "Local", "voice:r0");
        connect(&mut world, 3, 3, "Near", "voice:r1");
        connect(&mut world, 4, 4, "Mid", "voice:r2");
        connect(&mut world, 5, 5, "Far", "voice:r3");
        connect(&mut world, 6, 6, "Beyond", "voice:r4");
        connect(&mut world, 7, 7, "Inbound", "voice:inbound");
        connect(&mut world, 8, 8, "Deaf", "voice:r1");
        let mut deaf = world.character_gameplay_snapshot(SessionId(8)).unwrap();
        deaf.script_flags.insert("condition.deaf".into());
        world
            .restore_character_gameplay(CharacterId(8), deaf)
            .unwrap();

        let names = |audience: Vec<ConnectedCharacter>| {
            audience
                .into_iter()
                .map(|candidate| candidate.name)
                .collect::<Vec<_>>()
        };
        assert_eq!(
            names(world.local_voice_audience(SessionId(1)).unwrap()),
            vec!["Actor", "Local"]
        );
        assert_eq!(
            names(world.yell_audience(SessionId(1)).unwrap()),
            vec!["Actor", "Local", "Near"]
        );
        assert_eq!(
            names(world.shout_audience(SessionId(1)).unwrap()),
            vec!["Actor", "Local", "Near", "Mid", "Far"]
        );

        world
            .configure_secret_exit("voice:r1", Direction::East, 1)
            .unwrap();
        assert_eq!(
            names(world.shout_audience(SessionId(1)).unwrap()),
            vec!["Actor", "Local", "Near"]
        );

        // Deafness does not prevent emission or remove the actor's private
        // acknowledgement path: only receivers are filtered.
        assert_eq!(
            names(world.shout_audience(SessionId(8)).unwrap()),
            vec!["Actor", "Local", "Near"]
        );
    }

    #[test]
    fn shout_traversal_accepts_the_exact_structural_room_cap() {
        let directions = [
            Direction::North,
            Direction::East,
            Direction::South,
            Direction::West,
            Direction::Up,
            Direction::Down,
        ];
        let mut exits = (0..MAX_SPATIAL_VOICE_ROOMS)
            .map(|_| Vec::new())
            .collect::<Vec<Vec<ExitDefinition>>>();
        let mut next_room = 1_usize;
        let mut level = vec![0_usize];
        for _ in 0..SHOUT_MAX_ROOM_HOPS {
            let mut next_level = Vec::new();
            for parent in level {
                for direction in directions {
                    let child = next_room;
                    next_room += 1;
                    exits[parent].push(ExitDefinition {
                        direction,
                        target_room_id: format!("cap:r{child}"),
                    });
                    next_level.push(child);
                }
            }
            level = next_level;
        }
        assert_eq!(next_room, MAX_SPATIAL_VOICE_ROOMS);
        let mut world = World::from_definition(WorldDefinition {
            start_room_id: "cap:r0".into(),
            item_prototypes: Vec::new(),
            initial_item_instances: Vec::new(),
            rooms: exits
                .into_iter()
                .enumerate()
                .map(|(index, exits)| RoomDefinition {
                    stable_id: format!("cap:r{index}"),
                    title: format!("Cap room {index}"),
                    prose: Vec::new(),
                    image: None,
                    audio: None,
                    item_ids: Vec::new(),
                    exits,
                })
                .collect(),
        })
        .unwrap();
        for index in 0..MAX_SPATIAL_VOICE_ROOMS {
            let identity = i64::try_from(index + 1).unwrap();
            world
                .connect(
                    SessionId(u64::try_from(index + 1).unwrap()),
                    CharacterSnapshot {
                        id: CharacterId(identity),
                        account_id: AccountId(identity),
                        name: format!("Listener {index}"),
                        room_id: format!("cap:r{index}"),
                        item_locations: BTreeMap::new(),
                    },
                )
                .unwrap();
        }

        assert_eq!(
            world.shout_audience(SessionId(1)).unwrap().len(),
            MAX_SPATIAL_VOICE_ROOMS
        );
    }

    #[test]
    fn spatial_voice_visits_cycles_and_convergent_paths_only_once() {
        let room = |id: &str, exits: Vec<(Direction, &str)>| RoomDefinition {
            stable_id: id.into(),
            title: id.into(),
            prose: Vec::new(),
            image: None,
            audio: None,
            item_ids: Vec::new(),
            exits: exits
                .into_iter()
                .map(|(direction, target)| ExitDefinition {
                    direction,
                    target_room_id: target.into(),
                })
                .collect(),
        };
        let mut world = World::from_definition(WorldDefinition {
            start_room_id: "diamond:origin".into(),
            item_prototypes: Vec::new(),
            initial_item_instances: Vec::new(),
            rooms: vec![
                room(
                    "diamond:origin",
                    vec![
                        (Direction::North, "diamond:left"),
                        (Direction::East, "diamond:right"),
                    ],
                ),
                room("diamond:left", vec![(Direction::East, "diamond:join")]),
                room("diamond:right", vec![(Direction::North, "diamond:join")]),
                // This edge closes a cycle back to the origin after the two
                // branches converge at the same room.
                room("diamond:join", vec![(Direction::South, "diamond:origin")]),
            ],
        })
        .unwrap();
        for (session, name, room_id) in [
            (1_u64, "Actor", "diamond:origin"),
            (2, "At join", "diamond:join"),
        ] {
            world
                .connect(
                    SessionId(session),
                    CharacterSnapshot {
                        id: CharacterId(i64::try_from(session).unwrap()),
                        account_id: AccountId(i64::try_from(session).unwrap()),
                        name: name.into(),
                        room_id: room_id.into(),
                        item_locations: BTreeMap::new(),
                    },
                )
                .unwrap();
        }

        let audience = world.shout_audience(SessionId(1)).unwrap();
        assert_eq!(
            audience
                .iter()
                .map(|candidate| candidate.name.as_str())
                .collect::<Vec<_>>(),
            vec!["Actor", "At join"]
        );
        assert_eq!(
            audience
                .iter()
                .filter(|candidate| candidate.room_id == "diamond:join")
                .count(),
            1
        );
    }

    #[test]
    fn rejects_invalid_item_definitions_and_room_references() {
        let room = || RoomDefinition {
            stable_id: "test:start".into(),
            title: "Start".into(),
            prose: Vec::new(),
            image: None,
            audio: None,
            exits: Vec::new(),
            item_ids: Vec::new(),
        };
        let item = item_prototype("test:item", "item", "An item.", &[]);

        assert!(matches!(
            World::from_definition(WorldDefinition {
                start_room_id: "test:start".into(),
                item_prototypes: Vec::new(),
                initial_item_instances: vec![item_instance(
                    "test:instance",
                    "missing",
                    InitialItemLocation::Room("test:start".into()),
                )],
                rooms: vec![room()],
            }),
            Err(WorldBuildError::UnknownItemPrototype(id)) if id == "missing"
        ));
        assert!(
            World::from_definition(WorldDefinition {
                start_room_id: "test:start".into(),
                item_prototypes: vec![item.clone()],
                initial_item_instances: Vec::new(),
                rooms: vec![room()],
            })
            .is_ok()
        );
        assert!(matches!(
            World::from_definition(WorldDefinition {
                start_room_id: "test:start".into(),
                item_prototypes: vec![item.clone()],
                initial_item_instances: vec![
                    item_instance("duplicate", "test:item", InitialItemLocation::Staged),
                    item_instance("duplicate", "test:item", InitialItemLocation::Staged),
                ],
                rooms: vec![room()],
            }),
            Err(WorldBuildError::DuplicateItemId(id)) if id == "duplicate"
        ));

        assert!(matches!(
            World::from_definition_with_staged_items(
                WorldDefinition {
                    start_room_id: "test:start".into(),
                    item_prototypes: vec![item.clone()],
                    initial_item_instances: vec![item_instance(
                        "test:item-instance",
                        "test:item",
                        InitialItemLocation::Room("test:start".into()),
                    )],
                    rooms: vec![room()],
                },
                vec!["test:item-instance".into()],
            ),
            Err(WorldBuildError::ItemPlacedAndStaged { item_id, .. })
                if item_id == "test:item-instance"
        ));

        let mut weightless = item;
        weightless.weight_grams = 0;
        assert!(
            World::from_definition(WorldDefinition {
                start_room_id: "test:start".into(),
                item_prototypes: vec![weightless],
                initial_item_instances: Vec::new(),
                rooms: vec![room()],
            })
            .is_ok()
        );

        let mut reserved_transfer = item_prototype("test:bound", "bound", "Bound.", &[]);
        reserved_transfer.transfer_policy = ItemTransferPolicy::CharacterBound;
        assert!(
            World::from_definition(WorldDefinition {
                start_room_id: "test:start".into(),
                item_prototypes: vec![reserved_transfer],
                initial_item_instances: Vec::new(),
                rooms: vec![room()],
            })
            .is_ok()
        );
    }

    #[test]
    fn repeatable_instances_are_distinct_opaque_and_capacity_failures_are_atomic() {
        let mut prototype = item_prototype(
            "test:repeated",
            "etched token",
            "One concrete token from a reusable design.",
            &["token"],
        );
        prototype.weight_grams = 600;
        prototype.rarity = ItemRarity::Rare;
        prototype.item_level = 7;
        let mut world = World::from_definition(WorldDefinition {
            start_room_id: "test:start".into(),
            item_prototypes: vec![prototype],
            initial_item_instances: vec![
                item_instance(
                    "durable:first",
                    "test:repeated",
                    InitialItemLocation::Room("test:start".into()),
                ),
                item_instance(
                    "durable:second",
                    "test:repeated",
                    InitialItemLocation::Room("test:start".into()),
                ),
            ],
            rooms: vec![RoomDefinition {
                stable_id: "test:start".into(),
                title: "Start".into(),
                prose: Vec::new(),
                image: None,
                audio: None,
                exits: Vec::new(),
                item_ids: Vec::new(),
            }],
        })
        .unwrap();
        let session = SessionId(500);
        world
            .connect(
                session,
                CharacterSnapshot {
                    id: CharacterId(500),
                    account_id: AccountId(500),
                    name: "Carrier".into(),
                    room_id: "test:start".into(),
                    item_locations: BTreeMap::new(),
                },
            )
            .unwrap();
        let mut gameplay = world.character_gameplay_snapshot(session).unwrap();
        gameplay.inventory_capacity = InventoryCapacity {
            max_slots: 1,
            max_weight_grams: 1_000,
        };
        world
            .restore_character_gameplay(CharacterId(500), gameplay)
            .unwrap();

        let scene = world.scene_for(session).unwrap();
        assert_eq!(scene.items.len(), 2);
        assert_ne!(scene.items[0].id, scene.items[1].id);
        assert!(scene.items.iter().all(|item| item.rarity == "rare"));
        let scene_json = serde_json::to_string(&scene).unwrap();
        assert!(!scene_json.contains("durable:first"));
        assert!(!scene_json.contains("durable:second"));
        assert!(!scene_json.contains("test:repeated"));

        world
            .apply(
                session,
                PlayerAction::Take(scene.items[0].id.clone().into()),
            )
            .unwrap();
        let before = world.runtime_snapshot();
        let revision = world.revision();
        let remaining = world.scene_for(session).unwrap().items[0].id.clone();
        assert!(matches!(
            world.apply(session, PlayerAction::Take(remaining.into())),
            Err(WorldError::InventoryCapacityExceeded)
        ));
        assert_eq!(world.runtime_snapshot(), before);
        assert_eq!(world.revision(), revision);
        let text = world.inventory_as_plain_text(session).unwrap();
        assert!(text.contains("1/1 slots"));
        assert!(text.contains(
            "[rare; level 7; 600 g each; 1 stack slot; 600 g total; policy transferable; ref o"
        ));
        assert!(redact_and_validate_opaque_item_refs(&text).contains(
            "[rare; level 7; 600 g each; 1 stack slot; 600 g total; policy transferable]"
        ));
    }

    #[test]
    fn unequip_and_equipment_swap_preflight_slot_capacity() {
        let mut bulky = item_prototype("test:bulky", "bulky blade", "Bulky.", &["bulky"]);
        bulky.equipment_slot = Some(EquipmentSlot::Weapon);
        bulky.slot_cost = 2;
        let mut light = item_prototype("test:light", "light blade", "Light.", &["light"]);
        light.equipment_slot = Some(EquipmentSlot::Weapon);
        let mut world = World::from_definition(WorldDefinition {
            start_room_id: "test:start".into(),
            item_prototypes: vec![bulky, light],
            initial_item_instances: vec![
                item_instance(
                    "test:bulky-instance",
                    "test:bulky",
                    InitialItemLocation::Room("test:start".into()),
                ),
                item_instance(
                    "test:light-instance",
                    "test:light",
                    InitialItemLocation::Room("test:start".into()),
                ),
            ],
            rooms: vec![RoomDefinition {
                stable_id: "test:start".into(),
                title: "Start".into(),
                prose: Vec::new(),
                image: None,
                audio: None,
                exits: Vec::new(),
                item_ids: Vec::new(),
            }],
        })
        .unwrap();
        let session = SessionId(501);
        world
            .connect(
                session,
                CharacterSnapshot {
                    id: CharacterId(501),
                    account_id: AccountId(501),
                    name: "Carrier".into(),
                    room_id: "test:start".into(),
                    item_locations: BTreeMap::new(),
                },
            )
            .unwrap();
        world
            .apply(session, PlayerAction::Take("bulky".into()))
            .unwrap();
        world
            .apply(session, PlayerAction::Equip("bulky".into()))
            .unwrap();
        world
            .apply(session, PlayerAction::Take("light".into()))
            .unwrap();
        let mut gameplay = world.character_gameplay_snapshot(session).unwrap();
        gameplay.inventory_capacity.max_slots = 1;
        world
            .restore_character_gameplay(CharacterId(501), gameplay)
            .unwrap();
        let before = world.runtime_snapshot();
        let gameplay_before = world.character_gameplay_snapshot(session).unwrap();
        assert!(matches!(
            world.apply(session, PlayerAction::Unequip("weapon".into())),
            Err(WorldError::InventoryCapacityExceeded)
        ));
        assert!(matches!(
            world.apply(session, PlayerAction::Equip("light".into())),
            Err(WorldError::InventoryCapacityExceeded)
        ));
        assert_eq!(world.runtime_snapshot(), before);
        assert_eq!(
            world.character_gameplay_snapshot(session).unwrap(),
            gameplay_before
        );
    }

    #[test]
    fn snapshot_v1_upgrade_event_json_and_allocator_remain_compatible() {
        let world = World::demo();
        let current = world.runtime_snapshot();
        let legacy = WorldRuntimeSnapshotV1 {
            snapshot_version: 1,
            revision: current.revision,
            logical_tick: current.logical_tick,
            rng_state: current.rng_state,
            item_locations: current
                .item_instances
                .iter()
                .map(|(id, instance)| {
                    let location = match &instance.location {
                        RuntimeItemLocation::Staged => RuntimeItemLocationV2::Staged,
                        RuntimeItemLocation::Carried { character_id } => {
                            RuntimeItemLocationV2::Carried {
                                character_id: *character_id,
                            }
                        }
                        RuntimeItemLocation::Room { room_id } => RuntimeItemLocationV2::Room {
                            room_id: room_id.clone(),
                        },
                        RuntimeItemLocation::Equipped { character_id, slot } => {
                            RuntimeItemLocationV2::Equipped {
                                character_id: *character_id,
                                slot: *slot,
                            }
                        }
                        RuntimeItemLocation::HeldByNpc { npc_id } => {
                            RuntimeItemLocationV2::HeldByNpc {
                                npc_id: npc_id.clone(),
                            }
                        }
                        RuntimeItemLocation::Consumed => RuntimeItemLocationV2::Consumed,
                        RuntimeItemLocation::Contained { .. } => {
                            panic!("v1 fixture must not contain nested items")
                        }
                    };
                    (id.0.clone(), location)
                })
                .collect(),
            npcs: current.npcs,
            adventure: current.adventure,
            progression_flags: current.progression_flags,
        };
        let encoded = serde_json::to_string(&legacy).unwrap();
        let compatible: CompatibleWorldRuntimeSnapshot = serde_json::from_str(&encoded).unwrap();
        let upgraded = compatible.into_current().unwrap();
        assert_eq!(upgraded.snapshot_version, WORLD_RUNTIME_SNAPSHOT_VERSION);
        assert_eq!(upgraded.item_instances.len(), 1);
        assert_eq!(
            upgraded.item_instances[&ItemInstanceId("demo:blue-lantern".into())].prototype_id,
            "demo:blue-lantern"
        );

        let event = DomainEvent::ItemTaken {
            actor_id: CharacterId(1),
            actor_name: "Carrier".into(),
            item_id: ItemInstanceId("demo:blue-lantern".into()),
            item_name: "blue-glass lantern".into(),
            room_id: "demo:lantern-square".into(),
        };
        let event_json = serde_json::to_string(&event).unwrap();
        assert!(
            event_json.contains("\"item_id\":\"demo:blue-lantern\""),
            "{event_json}"
        );
        assert_eq!(
            serde_json::from_str::<DomainEvent>(&event_json).unwrap(),
            event
        );

        let mut collision_prototype = item_prototype(
            "runtime:1",
            "legacy token",
            "Legacy runtime-looking identity.",
            &["token"],
        );
        collision_prototype.weight_grams = 0;
        let mut collision_world = World::from_definition(WorldDefinition {
            start_room_id: "test:start".into(),
            item_prototypes: vec![collision_prototype],
            initial_item_instances: vec![item_instance(
                "runtime:1",
                "runtime:1",
                InitialItemLocation::Staged,
            )],
            rooms: vec![RoomDefinition {
                stable_id: "test:start".into(),
                title: "Start".into(),
                prose: Vec::new(),
                image: None,
                audio: None,
                exits: Vec::new(),
                item_ids: Vec::new(),
            }],
        })
        .unwrap();
        let before = collision_world.runtime_snapshot();
        let checkpoint = collision_world.checkpoint();
        assert_eq!(
            collision_world
                .create_staged_item_instance("runtime:1")
                .unwrap(),
            ItemInstanceId("runtime:2".into())
        );
        collision_world.restore_checkpoint(checkpoint);
        assert_eq!(collision_world.runtime_snapshot(), before);
    }

    #[test]
    fn runtime_restore_merges_new_authored_instances_and_rejects_identity_drift() {
        let prototypes = || {
            vec![
                item_prototype("test:token", "token", "A token.", &["token"]),
                item_prototype("test:other", "other", "Another token.", &["other"]),
            ]
        };
        let room = || RoomDefinition {
            stable_id: "test:start".into(),
            title: "Start".into(),
            prose: Vec::new(),
            image: None,
            audio: None,
            exits: Vec::new(),
            item_ids: Vec::new(),
        };
        let build = |include_new: bool| {
            let mut instances = vec![item_instance(
                "test:token:one",
                "test:token",
                InitialItemLocation::Room("test:start".into()),
            )];
            if include_new {
                instances.push(item_instance(
                    "test:token:two",
                    "test:token",
                    InitialItemLocation::Room("test:start".into()),
                ));
            }
            World::from_definition(WorldDefinition {
                start_room_id: "test:start".into(),
                item_prototypes: prototypes(),
                initial_item_instances: instances,
                rooms: vec![room()],
            })
            .unwrap()
        };

        let old_snapshot = build(false).runtime_snapshot();
        let mut upgraded_world = build(true);
        upgraded_world
            .restore_runtime_snapshot(old_snapshot.clone())
            .unwrap();
        assert!(
            upgraded_world
                .runtime_snapshot()
                .item_instances
                .contains_key(&ItemInstanceId("test:token:two".into()))
        );

        let mut retargeted = old_snapshot.clone();
        retargeted
            .item_instances
            .get_mut(&ItemInstanceId("test:token:one".into()))
            .unwrap()
            .prototype_id = "test:other".into();
        assert!(matches!(
            build(true).restore_runtime_snapshot(retargeted),
            Err(WorldError::InvalidRuntimeSnapshot)
        ));

        let mut foreign_id = old_snapshot.clone();
        foreign_id.item_instances.insert(
            ItemInstanceId("phantom:anything".into()),
            RuntimeItemInstanceSnapshot {
                prototype_id: "test:token".into(),
                location: RuntimeItemLocation::Staged,
                binding: ItemBinding::Unbound,
                item_affixes: Vec::new(),
            },
        );
        assert!(matches!(
            build(true).restore_runtime_snapshot(foreign_id),
            Err(WorldError::InvalidRuntimeSnapshot)
        ));

        let mut runtime_authored = World::from_definition(WorldDefinition {
            start_room_id: "test:start".into(),
            item_prototypes: prototypes(),
            initial_item_instances: vec![
                item_instance(
                    "test:token:one",
                    "test:token",
                    InitialItemLocation::Room("test:start".into()),
                ),
                item_instance("runtime:100", "test:token", InitialItemLocation::Staged),
            ],
            rooms: vec![room()],
        })
        .unwrap();
        runtime_authored
            .restore_runtime_snapshot(old_snapshot.clone())
            .unwrap();
        assert_eq!(
            runtime_authored
                .create_staged_item_instance("test:token")
                .unwrap(),
            ItemInstanceId("runtime:101".into())
        );

        let mut stale_sequence = old_snapshot;
        stale_sequence.item_instances.insert(
            ItemInstanceId("runtime:100".into()),
            RuntimeItemInstanceSnapshot {
                prototype_id: "test:token".into(),
                location: RuntimeItemLocation::Staged,
                binding: ItemBinding::Unbound,
                item_affixes: Vec::new(),
            },
        );
        stale_sequence.next_item_instance_sequence = 100;
        assert!(matches!(
            build(true).restore_runtime_snapshot(stale_sequence),
            Err(WorldError::InvalidRuntimeSnapshot)
        ));
    }

    #[test]
    fn staged_npc_loot_is_neither_visible_nor_interactable_before_spawn() {
        let mut world = World::from_definition_with_staged_items(
            WorldDefinition {
                start_room_id: "test:start".into(),
                item_prototypes: vec![item_prototype(
                    "test:loot",
                    "silver fang",
                    "A trophy carried by a guardian.",
                    &["fang"],
                )],
                initial_item_instances: vec![item_instance(
                    "test:loot",
                    "test:loot",
                    InitialItemLocation::Staged,
                )],
                rooms: vec![RoomDefinition {
                    stable_id: "test:start".into(),
                    title: "Start".into(),
                    prose: Vec::new(),
                    image: None,
                    audio: None,
                    exits: Vec::new(),
                    item_ids: Vec::new(),
                }],
            },
            vec!["test:loot".into()],
        )
        .unwrap();
        let session = SessionId(91);
        world
            .connect(
                session,
                CharacterSnapshot {
                    id: CharacterId(92),
                    account_id: AccountId(93),
                    name: "Observer".into(),
                    room_id: "test:start".into(),
                    item_locations: BTreeMap::new(),
                },
            )
            .unwrap();

        assert!(world.scene_for(session).unwrap().items.is_empty());
        assert!(matches!(
            world.apply(session, PlayerAction::Take("fang".into())),
            Err(WorldError::ItemNotFound(query)) if query == "fang"
        ));
        world
            .spawn_npc(NpcDefinition {
                stable_id: "test:guardian".into(),
                name: "guardian".into(),
                description: "A silent guardian.".into(),
                room_id: "test:start".into(),
                level: 1,
                max_health: 1,
                strength: 1,
                defence: 0,
                experience_reward: 1,
                loot_item_ids: vec![ItemInstanceId("test:loot".into())],
                respawn_ticks: 1,
            })
            .unwrap();
        assert_eq!(
            world.runtime_snapshot().item_instances[&ItemInstanceId("test:loot".into())].location,
            RuntimeItemLocation::HeldByNpc {
                npc_id: "test:guardian".into(),
            }
        );
        world
            .spawn_npc(NpcDefinition {
                stable_id: "test:bystander".into(),
                name: "bystander".into(),
                description: "Another NPC.".into(),
                room_id: "test:start".into(),
                level: 1,
                max_health: 1,
                strength: 1,
                defence: 0,
                experience_reward: 0,
                loot_item_ids: Vec::new(),
                respawn_ticks: 1,
            })
            .unwrap();
        let mut wrong_owner = world.runtime_snapshot();
        wrong_owner
            .item_instances
            .get_mut(&ItemInstanceId("test:loot".into()))
            .unwrap()
            .location = RuntimeItemLocation::HeldByNpc {
            npc_id: "test:bystander".into(),
        };
        assert!(matches!(
            world.restore_runtime_snapshot(wrong_owner),
            Err(WorldError::InvalidRuntimeSnapshot)
        ));
    }

    #[test]
    fn parses_adventure_commands_without_losing_targets() {
        assert_eq!(
            parse_action("party invite Aria").unwrap(),
            PlayerAction::Party(PartyAction::Invite("Aria".into()))
        );
        assert_eq!(
            parse_action("quest accept first hunt").unwrap(),
            PlayerAction::Quest(QuestAction::Accept("first hunt".into()))
        );
        assert_eq!(
            parse_action("buy healing tonic").unwrap(),
            PlayerAction::Economy(EconomyAction::Buy("healing tonic".into()))
        );
        assert_eq!(parse_action("weather").unwrap(), PlayerAction::Weather);
    }

    #[test]
    fn party_loop_enforces_leader_authority_and_projects_opaque_ids() {
        let (mut world, leader_session) = connected_world();
        let member_session = SessionId(8);
        world
            .connect(
                member_session,
                CharacterSnapshot {
                    id: CharacterId(102),
                    account_id: AccountId(11),
                    name: "Aria".into(),
                    room_id: world.start_room_id().into(),
                    item_locations: BTreeMap::new(),
                },
            )
            .unwrap();
        let outsider_session = SessionId(9);
        world
            .connect(
                outsider_session,
                CharacterSnapshot {
                    id: CharacterId(103),
                    account_id: AccountId(12),
                    name: "Bram".into(),
                    room_id: world.start_room_id().into(),
                    item_locations: BTreeMap::new(),
                },
            )
            .unwrap();

        let created = world
            .apply(leader_session, PlayerAction::Party(PartyAction::Create))
            .unwrap();
        let view = world
            .project_event(leader_session, &created.domain_events[0])
            .unwrap()
            .unwrap();
        assert_eq!(view.kind, ViewEventKind::Adventure);
        assert!(!view.actor_id.contains("101"));
        assert_ne!(view.target_id.unwrap(), "1");

        world
            .apply(
                leader_session,
                PlayerAction::Party(PartyAction::Invite("Bram".into())),
            )
            .unwrap();
        let invited_member = world
            .apply(
                leader_session,
                PlayerAction::Party(PartyAction::Invite("Aria".into())),
            )
            .unwrap();
        assert!(
            world
                .project_event(outsider_session, &invited_member.domain_events[0])
                .unwrap()
                .is_none()
        );
        let revoked = world
            .apply(
                leader_session,
                PlayerAction::Party(PartyAction::Revoke("Bram".into())),
            )
            .unwrap();
        assert!(
            world
                .project_event(outsider_session, &revoked.domain_events[0])
                .unwrap()
                .is_some()
        );
        let accepted = world
            .apply(
                member_session,
                PlayerAction::Party(PartyAction::Accept("Tester".into())),
            )
            .unwrap();
        assert!(
            world
                .project_event(leader_session, &accepted.domain_events[0])
                .unwrap()
                .is_some()
        );
        assert!(
            world
                .project_event(outsider_session, &accepted.domain_events[0])
                .unwrap()
                .is_none()
        );
        assert!(matches!(
            world.apply(
                member_session,
                PlayerAction::Party(PartyAction::Invite("Bram".into()))
            ),
            Err(WorldError::PartyForbidden)
        ));
        assert!(
            world
                .apply(member_session, PlayerAction::Party(PartyAction::Status))
                .unwrap()
                .output
                .unwrap()
                .contains("Tester, Aria")
        );
        let json = serde_json::to_string(&world.runtime_snapshot()).unwrap();
        let restored: WorldRuntimeSnapshot = serde_json::from_str(&json).unwrap();
        assert_eq!(restored.adventure.parties.len(), 1);

        assert!(matches!(
            world.apply(
                member_session,
                PlayerAction::Party(PartyAction::Kick("Tester".into()))
            ),
            Err(WorldError::PartyForbidden)
        ));
        world
            .apply(
                leader_session,
                PlayerAction::Party(PartyAction::Transfer("Aria".into())),
            )
            .unwrap();
        assert!(
            world
                .player_state_view(member_session)
                .unwrap()
                .party
                .unwrap()
                .leader
        );
        assert!(matches!(
            world.apply(
                leader_session,
                PlayerAction::Party(PartyAction::Kick("Aria".into()))
            ),
            Err(WorldError::PartyForbidden)
        ));
        let kicked = world
            .apply(
                member_session,
                PlayerAction::Party(PartyAction::Kick("Tester".into())),
            )
            .unwrap();
        assert!(
            world
                .project_event(leader_session, &kicked.domain_events[0])
                .unwrap()
                .is_some()
        );
        assert!(
            world
                .player_state_view(leader_session)
                .unwrap()
                .party
                .is_none()
        );
        world
            .apply(member_session, PlayerAction::Party(PartyAction::Disband))
            .unwrap();
        assert!(
            world
                .player_state_view(member_session)
                .unwrap()
                .party
                .is_none()
        );
    }

    #[test]
    fn economy_buy_sell_and_checkpoint_are_bounded_and_reversible() {
        let (mut world, session) = connected_world();
        let checkpoint = world.checkpoint();
        world
            .apply(
                session,
                PlayerAction::Economy(EconomyAction::Buy("tonic".into())),
            )
            .unwrap();
        let after_buy = world.character_gameplay_snapshot(session).unwrap();
        assert_eq!(after_buy.coins, 28);
        assert_eq!(after_buy.commodities["tonic"], 1);
        assert_eq!(world.runtime_snapshot().adventure.shop_stock["tonic"], 19);
        world
            .apply(
                session,
                PlayerAction::Economy(EconomyAction::Sell("tonic".into())),
            )
            .unwrap();
        assert_eq!(
            world.character_gameplay_snapshot(session).unwrap().coins,
            34
        );
        world.restore_checkpoint(checkpoint);
        assert_eq!(
            world.character_gameplay_snapshot(session).unwrap().coins,
            40
        );
        assert_eq!(world.runtime_snapshot().adventure.shop_stock["tonic"], 20);
    }

    #[test]
    fn quest_progresses_from_combat_and_reward_can_only_be_claimed_once() {
        let (mut world, session) = phase_two_world(5, 1, 1);
        world
            .apply(
                session,
                PlayerAction::Quest(QuestAction::Accept("first-hunt".into())),
            )
            .unwrap();
        world
            .apply(session, PlayerAction::Move(Direction::North))
            .unwrap();
        world
            .apply(session, PlayerAction::Attack("guardian".into()))
            .unwrap();
        world.advance_tick().unwrap();
        let ready = world.character_gameplay_snapshot(session).unwrap();
        assert_eq!(ready.quests["first-hunt"].status, QuestStatus::ReadyToClaim);
        let coins = ready.coins;
        let experience = ready.experience;
        world
            .apply(
                session,
                PlayerAction::Quest(QuestAction::Complete("first-hunt".into())),
            )
            .unwrap();
        let completed = world.character_gameplay_snapshot(session).unwrap();
        assert_eq!(completed.coins, coins + 25);
        assert_eq!(completed.experience, experience + 50);
        assert!(matches!(
            world.apply(
                session,
                PlayerAction::Quest(QuestAction::Complete("first-hunt".into()))
            ),
            Err(WorldError::QuestUnavailable)
        ));
    }

    #[test]
    fn weather_timer_is_deterministic_and_survives_runtime_restore() {
        let (mut first, _) = connected_world();
        let (mut second, _) = connected_world();
        first.set_rng_seed(777);
        second.set_rng_seed(777);
        for _ in 0..5 {
            assert_eq!(
                first.advance_tick().unwrap(),
                second.advance_tick().unwrap()
            );
        }
        assert_ne!(
            first.runtime_snapshot().adventure.weather,
            WeatherKind::Rain
        );

        let snapshot = first.runtime_snapshot();
        let (mut restored, _) = connected_world();
        restored.restore_runtime_snapshot(snapshot).unwrap();
        assert_eq!(
            restored.runtime_snapshot().adventure,
            first.runtime_snapshot().adventure
        );
    }

    #[test]
    fn player_state_is_complete_bounded_and_never_serialises_durable_entity_ids() {
        let (mut world, session) = connected_world();
        let initial = world.player_state_view(session).unwrap();
        assert_eq!(initial.character.name, "Tester");
        assert_eq!(initial.character.health, 20);
        assert_eq!(initial.map.rooms.len(), 1);
        assert!(initial.map.rooms[0].current);
        assert_eq!(initial.quests[0].status, "available");
        assert_eq!(initial.economy.coins, 40);
        assert_eq!(initial.social.online.len(), 1);

        world
            .apply(session, PlayerAction::Take("lantern".into()))
            .unwrap();
        world
            .apply(session, PlayerAction::Move(Direction::North))
            .unwrap();
        let discovered = world.player_state_view(session).unwrap();
        assert_eq!(discovered.inventory.len(), 1);
        assert_eq!(discovered.map.rooms.len(), 2);
        assert_eq!(
            discovered
                .map
                .rooms
                .iter()
                .filter(|room| room.current)
                .count(),
            1
        );
        assert!(
            discovered
                .inventory
                .iter()
                .all(|item| item.id.starts_with('o') && item.id.len() == 17)
        );
        assert!(
            discovered
                .equipment
                .iter()
                .all(|item| item.id.starts_with('o') && item.id.len() == 17)
        );

        let json = serde_json::to_string(&discovered).unwrap();
        assert!(!json.contains("demo:lantern-square"));
        assert!(!json.contains("demo:north-gate"));
        assert!(!json.contains("demo:blue-lantern"));
        assert!(!json.contains("accountId"));
        assert!(!json.contains("characterId"));
        assert!(!json.contains("partyId"));
        assert!(!json.contains("\"id\":101"));
    }

    #[test]
    fn parses_bounded_stack_and_container_commands() {
        assert_eq!(
            parse_action("take 3 tonic").unwrap(),
            PlayerAction::Take(ItemSelection {
                query: "tonic".into(),
                quantity: 3,
            })
        );
        assert_eq!(
            parse_action("get 2 tonic from SATCHEL").unwrap(),
            PlayerAction::Get {
                selection: ItemSelection {
                    query: "tonic".into(),
                    quantity: 2,
                },
                container: "satchel".into(),
            }
        );
        assert_eq!(
            parse_action("put tonic in satchel").unwrap(),
            PlayerAction::Put {
                selection: ItemSelection::one("tonic"),
                container: "satchel".into(),
            }
        );
        assert_eq!(
            parse_action("contents satchel 2").unwrap(),
            PlayerAction::Contents {
                container: "satchel".into(),
                page: 2,
            }
        );
        assert_eq!(
            parse_action("give 2 Moon Tonic to Aria").unwrap(),
            PlayerAction::Give {
                selection: ItemSelection {
                    query: "Moon Tonic".into(),
                    quantity: 2,
                },
                recipient: "Aria".into(),
            }
        );
        assert_eq!(
            parse_action("trade offer item 12 Moon Tonic").unwrap(),
            PlayerAction::Trade(TradeAction::OfferItem(ItemSelection {
                query: "Moon Tonic".into(),
                quantity: 12,
            }))
        );
        assert_eq!(
            parse_action("trade offer coins 1000000000").unwrap(),
            PlayerAction::Trade(TradeAction::OfferCoins(1_000_000_000))
        );
        assert_eq!(
            parse_action("trade confirm 42").unwrap(),
            PlayerAction::Trade(TradeAction::Confirm { state_revision: 42 })
        );
        assert_eq!(
            parse_action("trade contents offered bag 3").unwrap(),
            PlayerAction::Trade(TradeAction::Contents {
                container: "offered bag".into(),
                page: 3,
            })
        );
        assert!(parse_action("take 257 tonic").is_err());
        assert!(parse_action("take 0 tonic").is_err());
        assert!(parse_action("give tonic").is_err());
        assert!(parse_action("give 257 tonic to Aria").is_err());
        assert!(parse_action("trade offer item 129 tonic").is_err());
        assert!(parse_action("trade offer coins 1000000001").is_err());
        assert!(parse_action("trade confirm 0").is_err());
    }

    #[test]
    fn fungible_stacks_and_containers_are_derived_atomic_and_opaque() {
        let mut tonic = item_prototype("test:tonic", "healing tonic", "Restorative.", &["tonic"]);
        tonic.stack_policy = ItemStackPolicy::Fungible;
        tonic.max_stack_units = 2;
        tonic.weight_grams = 100;
        let mut satchel = item_prototype(
            "test:satchel",
            "archive satchel",
            "A bounded satchel.",
            &["satchel"],
        );
        satchel.weight_grams = 500;
        satchel.container = Some(ItemContainerDefinition {
            max_slots: 2,
            max_weight_grams: 500,
        });
        let instances = vec![
            item_instance(
                "test:satchel-instance",
                "test:satchel",
                InitialItemLocation::Room("test:start".into()),
            ),
            item_instance(
                "test:tonic-a",
                "test:tonic",
                InitialItemLocation::Room("test:start".into()),
            ),
            item_instance(
                "test:tonic-b",
                "test:tonic",
                InitialItemLocation::Room("test:start".into()),
            ),
            item_instance(
                "test:tonic-c",
                "test:tonic",
                InitialItemLocation::Room("test:start".into()),
            ),
        ];
        let (mut world, session) = connect_definition(WorldDefinition {
            start_room_id: "test:start".into(),
            item_prototypes: vec![tonic, satchel],
            initial_item_instances: instances,
            rooms: vec![RoomDefinition {
                stable_id: "test:start".into(),
                title: "Start".into(),
                prose: vec!["A test room.".into()],
                image: None,
                audio: None,
                item_ids: Vec::new(),
                exits: Vec::new(),
            }],
        });

        let scene = world.scene_for(session).unwrap();
        let tonic_groups = scene
            .items
            .iter()
            .filter(|item| item.label == "healing tonic")
            .collect::<Vec<_>>();
        assert_eq!(tonic_groups.len(), 2);
        assert_eq!(
            tonic_groups.iter().map(|item| item.quantity).sum::<u16>(),
            3
        );
        assert!(tonic_groups.iter().all(|item| item.id.starts_with('o')));

        let before_take = world.runtime_snapshot();
        let before_take_revision = world.revision();
        let take = world
            .apply(
                session,
                PlayerAction::Take(ItemSelection {
                    query: "tonic".into(),
                    quantity: 3,
                }),
            )
            .unwrap();
        assert_eq!(world.revision(), before_take_revision + 1);
        assert!(matches!(
            take.domain_events.as_slice(),
            [DomainEvent::ItemsMoved {
                item_ids,
                operation: ContainerMoveOperation::Take,
                container_id: None,
                ..
            }] if item_ids.len() == 3
        ));
        let projected = world
            .project_event(session, &take.domain_events[0])
            .unwrap()
            .unwrap();
        assert_eq!(projected.kind, ViewEventKind::ItemTaken);
        assert!(projected.target_id.is_none());
        let after_take = world.runtime_snapshot();
        world.rollback_domain_events(&take.domain_events).unwrap();
        assert_eq!(world.runtime_snapshot(), before_take);
        let mut forged = take.domain_events[0].clone();
        if let DomainEvent::ItemsMoved { item_ids, .. } = &mut forged {
            item_ids[1] = item_ids[0].clone();
        }
        assert!(matches!(
            world.replay_persisted_event(&forged),
            Err(WorldError::InvalidDomainEvent)
        ));
        assert_eq!(world.runtime_snapshot(), before_take);
        world
            .replay_persisted_event(&take.domain_events[0])
            .unwrap();
        assert_eq!(world.runtime_snapshot(), after_take);
        world
            .apply(session, PlayerAction::Take(ItemSelection::one("satchel")))
            .unwrap();
        let player = world.player_state_view(session).unwrap();
        assert_eq!(player.inventory_capacity.used_slots, 3);
        assert_eq!(player.inventory_capacity.used_weight_grams, 800);

        world
            .apply(
                session,
                PlayerAction::Put {
                    selection: ItemSelection {
                        query: "tonic".into(),
                        quantity: 2,
                    },
                    container: "satchel".into(),
                },
            )
            .unwrap();
        let contents = world
            .apply(
                session,
                PlayerAction::Contents {
                    container: "satchel".into(),
                    page: 1,
                },
            )
            .unwrap()
            .container_contents
            .unwrap();
        assert!(contents.selector.starts_with('o'));
        assert_eq!(contents.items.len(), 1);
        assert_eq!(contents.items[0].quantity, 2);
        assert_eq!(contents.capacity.used_slots, 1);
        assert_eq!(contents.capacity.used_weight_grams, 200);

        let before_get = world.runtime_snapshot();
        let get = world
            .apply(
                session,
                PlayerAction::Get {
                    selection: ItemSelection {
                        query: "tonic".into(),
                        quantity: 2,
                    },
                    container: "satchel".into(),
                },
            )
            .unwrap();
        assert!(
            matches!(get.domain_events.as_slice(), [DomainEvent::ItemsMoved { item_ids, .. }] if item_ids.len() == 2)
        );
        world.rollback_domain_events(&get.domain_events).unwrap();
        assert_eq!(world.runtime_snapshot(), before_get);
    }

    #[test]
    fn container_text_and_actions_keep_capacity_stack_and_weight_facts() {
        let mut satchel = item_prototype(
            "test:satchel",
            "archive satchel",
            "A bounded satchel.",
            &["satchel"],
        );
        satchel.weight_grams = 500;
        satchel.equipment_slot = Some(EquipmentSlot::Trinket);
        satchel.container = Some(ItemContainerDefinition {
            max_slots: 4,
            max_weight_grams: 1_000,
        });
        let mut box_item = item_prototype("test:box", "map box", "A smaller container.", &["box"]);
        box_item.weight_grams = 100;
        box_item.container = Some(ItemContainerDefinition {
            max_slots: 2,
            max_weight_grams: 500,
        });
        let mut tonic = item_prototype("test:tonic", "healing tonic", "Restorative.", &["tonic"]);
        tonic.weight_grams = 100;
        let (mut world, session) = connect_definition(WorldDefinition {
            start_room_id: "test:start".into(),
            item_prototypes: vec![satchel, box_item, tonic],
            initial_item_instances: vec![
                item_instance(
                    "test:satchel-instance",
                    "test:satchel",
                    InitialItemLocation::Room("test:start".into()),
                ),
                item_instance(
                    "test:box-instance",
                    "test:box",
                    InitialItemLocation::Container(ItemInstanceId("test:satchel-instance".into())),
                ),
                item_instance(
                    "test:tonic-instance",
                    "test:tonic",
                    InitialItemLocation::Container(ItemInstanceId("test:box-instance".into())),
                ),
            ],
            rooms: vec![RoomDefinition {
                stable_id: "test:start".into(),
                title: "Start".into(),
                prose: vec!["A test room.".into()],
                image: None,
                audio: None,
                item_ids: Vec::new(),
                exits: Vec::new(),
            }],
        });

        let scene_satchel = world
            .scene_for(session)
            .unwrap()
            .items
            .into_iter()
            .find(|item| item.label == "archive satchel")
            .unwrap();
        assert_eq!(scene_satchel.quantity, 1);
        assert_eq!(scene_satchel.stack_slots, 1);
        assert_eq!(scene_satchel.total_weight_grams, 700);
        assert!(
            scene_satchel
                .actions
                .iter()
                .any(|action| action.label == "Contents"
                    && action.command == format!("contents {}", scene_satchel.id))
        );

        world
            .apply(session, PlayerAction::Take(ItemSelection::one("satchel")))
            .unwrap();
        let carried = world.inventory_as_plain_text(session).unwrap();
        assert!(carried.contains("1 × archive satchel"));
        assert!(carried.contains("1 stack slot"));
        assert!(carried.contains("700 g total"));
        assert!(carried.contains("container 1/4 slots; 200/1000 g"));

        world
            .apply(session, PlayerAction::Equip("satchel".into()))
            .unwrap();
        let equipped = world.inventory_as_plain_text(session).unwrap();
        assert!(equipped.contains("trinket: 1 × archive satchel"));
        assert!(equipped.contains("1 stack slot"));
        assert!(equipped.contains("container 1/4 slots; 200/1000 g"));

        let contents = world
            .apply(
                session,
                PlayerAction::Contents {
                    container: "satchel".into(),
                    page: 1,
                },
            )
            .unwrap();
        let text = contents.output.unwrap();
        assert!(text.contains("1 × map box"));
        assert!(text.contains("1 stack slot"));
        assert!(text.contains("200 g total"));
        assert!(text.contains("container 1/2 slots; 100/500 g"));
    }

    #[test]
    fn container_cycles_and_capacity_fail_without_partial_mutation() {
        let mut bag = item_prototype("test:bag", "canvas bag", "A bag.", &["bag"]);
        bag.container = Some(ItemContainerDefinition {
            max_slots: 2,
            max_weight_grams: 10_000,
        });
        let (mut world, session) = connect_definition(WorldDefinition {
            start_room_id: "test:start".into(),
            item_prototypes: vec![bag],
            initial_item_instances: vec![
                item_instance(
                    "test:bag-a",
                    "test:bag",
                    InitialItemLocation::Room("test:start".into()),
                ),
                item_instance(
                    "test:bag-b",
                    "test:bag",
                    InitialItemLocation::Room("test:start".into()),
                ),
            ],
            rooms: vec![RoomDefinition {
                stable_id: "test:start".into(),
                title: "Start".into(),
                prose: vec!["A test room.".into()],
                image: None,
                audio: None,
                item_ids: Vec::new(),
                exits: Vec::new(),
            }],
        });
        let ids = world.scene_for(session).unwrap().items;
        world
            .apply(session, PlayerAction::Take(ids[0].id.clone().into()))
            .unwrap();
        world
            .apply(session, PlayerAction::Take(ids[1].id.clone().into()))
            .unwrap();
        let carried = world.player_state_view(session).unwrap().inventory;
        world
            .apply(
                session,
                PlayerAction::Put {
                    selection: ItemSelection::one(carried[0].id.clone()),
                    container: carried[1].id.clone(),
                },
            )
            .unwrap();
        let before = world.runtime_snapshot();
        let nested = world
            .apply(
                session,
                PlayerAction::Contents {
                    container: carried[1].id.clone(),
                    page: 1,
                },
            )
            .unwrap()
            .container_contents
            .unwrap()
            .items[0]
            .id
            .clone();
        assert!(
            world
                .apply(
                    session,
                    PlayerAction::Put {
                        selection: ItemSelection::one(carried[1].id.clone()),
                        container: nested,
                    },
                )
                .is_err()
        );
        assert_eq!(world.runtime_snapshot(), before);
    }

    #[test]
    fn fungible_equipment_uses_prospective_stack_capacity() {
        let mut blade = item_prototype("test:blade", "thin blade", "A blade.", &["blade"]);
        blade.stack_policy = ItemStackPolicy::Fungible;
        blade.max_stack_units = 10;
        blade.equipment_slot = Some(EquipmentSlot::Weapon);
        let (mut world, session) = connect_definition(WorldDefinition {
            start_room_id: "test:start".into(),
            item_prototypes: vec![blade],
            initial_item_instances: vec![
                item_instance(
                    "test:blade-a",
                    "test:blade",
                    InitialItemLocation::Room("test:start".into()),
                ),
                item_instance(
                    "test:blade-b",
                    "test:blade",
                    InitialItemLocation::Room("test:start".into()),
                ),
            ],
            rooms: vec![RoomDefinition {
                stable_id: "test:start".into(),
                title: "Start".into(),
                prose: vec!["A test room.".into()],
                image: None,
                audio: None,
                item_ids: Vec::new(),
                exits: Vec::new(),
            }],
        });
        world
            .character_gameplay
            .get_mut(&CharacterId(701))
            .unwrap()
            .inventory_capacity
            .max_slots = 1;
        world
            .apply(
                session,
                PlayerAction::Take(ItemSelection {
                    query: "blade".into(),
                    quantity: 2,
                }),
            )
            .unwrap();
        let selector = world.player_state_view(session).unwrap().inventory[0]
            .id
            .clone();
        world.apply(session, PlayerAction::Equip(selector)).unwrap();
        world
            .apply(session, PlayerAction::Unequip("weapon".into()))
            .unwrap();
        let player = world.player_state_view(session).unwrap();
        assert_eq!(player.inventory.len(), 1);
        assert_eq!(player.inventory[0].quantity, 2);
        assert_eq!(player.inventory_capacity.used_slots, 1);
    }

    #[test]
    fn snapshot_decoder_is_version_directed_and_rejects_v2_v3_smuggling() {
        let (world, _) = connected_world();
        let json = serde_json::to_string(&world.runtime_snapshot()).unwrap();
        assert_eq!(
            decode_world_runtime_snapshot(WORLD_RUNTIME_SNAPSHOT_VERSION, &json)
                .unwrap()
                .snapshot_version,
            WORLD_RUNTIME_SNAPSHOT_VERSION
        );
        assert!(decode_world_runtime_snapshot(4, &json).is_err());
        let legacy_v3 = runtime_snapshot_json_for_version(&world.runtime_snapshot(), 3);
        let legacy_v3_json = serde_json::to_string(&legacy_v3).unwrap();
        assert_eq!(
            decode_world_runtime_snapshot(3, &legacy_v3_json)
                .unwrap()
                .snapshot_version,
            WORLD_RUNTIME_SNAPSHOT_VERSION
        );
        let mut v3_binding_smuggling = legacy_v3;
        v3_binding_smuggling["itemInstances"]
            .as_object_mut()
            .unwrap()
            .values_mut()
            .next()
            .unwrap()["binding"] = serde_json::json!({"kind": "unbound"});
        assert!(
            decode_world_runtime_snapshot(
                3,
                &serde_json::to_string(&v3_binding_smuggling).unwrap()
            )
            .is_err()
        );
        let mut smuggled: serde_json::Value = serde_json::from_str(&json).unwrap();
        smuggled["snapshotVersion"] = serde_json::json!(2);
        let instance = smuggled["itemInstances"]
            .as_object_mut()
            .unwrap()
            .values_mut()
            .next()
            .unwrap();
        instance["location"] = serde_json::json!({
            "kind": "contained",
            "containerId": "demo:blue-lantern"
        });
        let smuggled = serde_json::to_string(&smuggled).unwrap();
        assert!(decode_world_runtime_snapshot(2, &smuggled).is_err());

        let mut unknown_root: serde_json::Value = serde_json::from_str(&json).unwrap();
        unknown_root["quantity"] = serde_json::json!(1);
        assert!(
            decode_world_runtime_snapshot(6, &serde_json::to_string(&unknown_root).unwrap())
                .is_err()
        );

        let mut unknown_instance: serde_json::Value = serde_json::from_str(&json).unwrap();
        let instance = unknown_instance["itemInstances"]
            .as_object_mut()
            .unwrap()
            .values_mut()
            .next()
            .unwrap();
        instance["quantity"] = serde_json::json!(1);
        assert!(
            decode_world_runtime_snapshot(6, &serde_json::to_string(&unknown_instance).unwrap())
                .is_err()
        );

        let mut unknown_location: serde_json::Value = serde_json::from_str(&json).unwrap();
        let location = &mut unknown_location["itemInstances"]
            .as_object_mut()
            .unwrap()
            .values_mut()
            .next()
            .unwrap()["location"];
        location["containerId"] = serde_json::json!("demo:blue-lantern");
        assert!(
            decode_world_runtime_snapshot(6, &serde_json::to_string(&unknown_location).unwrap())
                .is_err()
        );

        let legacy_v4 = runtime_snapshot_json_for_version(&world.runtime_snapshot(), 4);
        let upgraded =
            decode_world_runtime_snapshot(4, &serde_json::to_string(&legacy_v4).unwrap()).unwrap();
        assert_eq!(upgraded.snapshot_version, WORLD_RUNTIME_SNAPSHOT_VERSION);
        assert_eq!(upgraded.next_trade_id, TradeId(1));
        assert!(upgraded.trades.is_empty());
        let mut smuggled_v4 = legacy_v4;
        smuggled_v4["trades"] = serde_json::json!({});
        assert!(
            decode_world_runtime_snapshot(4, &serde_json::to_string(&smuggled_v4).unwrap())
                .is_err()
        );

        let mut missing_trade_fields: serde_json::Value = serde_json::from_str(&json).unwrap();
        missing_trade_fields
            .as_object_mut()
            .unwrap()
            .remove("trades");
        assert!(
            decode_world_runtime_snapshot(
                6,
                &serde_json::to_string(&missing_trade_fields).unwrap()
            )
            .is_err()
        );
        let mut nested_trade = runtime_snapshot_json_for_version(&world.runtime_snapshot(), 6);
        nested_trade["nextTradeId"] = serde_json::json!(2);
        nested_trade["trades"]["1"] = serde_json::to_value(TradeSnapshot {
            id: TradeId(1),
            requester: CharacterId(101),
            recipient: CharacterId(102),
            room_id: world.start_room_id().into(),
            phase: TradePhase::Requested,
            created_tick: 0,
            opened_tick: None,
            expires_at_tick: TRADE_REQUEST_TICKS,
            state_revision: 1,
            offers: BTreeMap::new(),
            confirmations: BTreeMap::new(),
        })
        .unwrap();
        assert!(
            decode_world_runtime_snapshot(6, &serde_json::to_string(&nested_trade).unwrap())
                .is_ok()
        );
        nested_trade["trades"]["1"]["futureTradeField"] = serde_json::json!(true);
        assert!(
            decode_world_runtime_snapshot(6, &serde_json::to_string(&nested_trade).unwrap())
                .is_err()
        );

        assert!(valid_item_instance_id("runtime:1"));
        assert!(valid_item_instance_id("runtime:18446744073709551615"));
        assert!(valid_item_instance_id(&"i".repeat(128)));
        for invalid_id in [
            "runtime:",
            "runtime:0",
            "runtime:01",
            "runtime:-1",
            "runtime:+1",
            "runtime:18446744073709551616",
            &"i".repeat(129),
        ] {
            assert!(!valid_item_instance_id(invalid_id), "{invalid_id}");
        }
    }

    #[test]
    fn runtime_snapshot_nested_records_reject_unknown_fields_across_versions() {
        let mut world = World::demo();
        let room_id = world.start_room_id().to_owned();
        world
            .spawn_npc(NpcDefinition {
                stable_id: "test:snapshot-npc".into(),
                name: "snapshot sentinel".into(),
                description: "An NPC retained only to exercise the durable snapshot shape.".into(),
                room_id,
                level: 1,
                max_health: 10,
                strength: 1,
                defence: 0,
                experience_reward: 1,
                loot_item_ids: Vec::new(),
                respawn_ticks: 2,
            })
            .unwrap();
        let snapshot = world.runtime_snapshot();
        for version in 1..=WORLD_RUNTIME_SNAPSHOT_VERSION {
            let clean = runtime_snapshot_json_for_version(&snapshot, version);
            let clean_json = serde_json::to_string(&clean).unwrap();
            assert!(
                decode_world_runtime_snapshot(version, &clean_json).is_ok(),
                "valid v{version} fixture must decode"
            );

            let mut npc_smuggling = clean.clone();
            first_runtime_npc(&mut npc_smuggling)["futureNpcField"] = serde_json::json!(true);
            assert!(
                decode_world_runtime_snapshot(
                    version,
                    &serde_json::to_string(&npc_smuggling).unwrap()
                )
                .is_err(),
                "v{version} must reject an unknown NPC field"
            );

            let mut adventure_smuggling = clean.clone();
            adventure_smuggling["adventure"]["futureAdventureField"] = serde_json::json!(true);
            assert!(
                decode_world_runtime_snapshot(
                    version,
                    &serde_json::to_string(&adventure_smuggling).unwrap()
                )
                .is_err(),
                "v{version} must reject an unknown adventure field"
            );

            let mut party_smuggling = clean.clone();
            party_smuggling["adventure"]["parties"]["77"] = serde_json::json!({
                "id": 77,
                "leader": 701,
                "members": [701],
                "invitations": {},
                "futurePartyField": true
            });
            assert!(
                decode_world_runtime_snapshot(
                    version,
                    &serde_json::to_string(&party_smuggling).unwrap()
                )
                .is_err(),
                "v{version} must reject an unknown party field"
            );

            let mut resource_smuggling = clean.clone();
            first_runtime_npc(&mut resource_smuggling)["resources"]["test:focus"] = serde_json::json!({
                "current": 1,
                "max": 2,
                "futureResourceField": true
            });
            assert!(
                decode_world_runtime_snapshot(
                    version,
                    &serde_json::to_string(&resource_smuggling).unwrap()
                )
                .is_err(),
                "v{version} must reject an unknown resource-pool field"
            );

            let mut status_smuggling = clean.clone();
            first_runtime_npc(&mut status_smuggling)["statuses"] = serde_json::json!([{
                "sourceAbilityId": "test:stun",
                "kind": "stun",
                "magnitude": 1,
                "remainingTicks": 1,
                "futureStatusField": true
            }]);
            assert!(
                decode_world_runtime_snapshot(
                    version,
                    &serde_json::to_string(&status_smuggling).unwrap()
                )
                .is_err(),
                "v{version} must reject an unknown active-status field"
            );

            let mut legacy_defaults = clean;
            legacy_defaults.as_object_mut().unwrap().remove("adventure");
            let npc = first_runtime_npc(&mut legacy_defaults)
                .as_object_mut()
                .unwrap();
            for optional in [
                "resources",
                "resourceRegeneration",
                "abilityIds",
                "abilityCooldowns",
                "resistances",
                "statuses",
            ] {
                npc.remove(optional);
            }
            assert!(
                decode_world_runtime_snapshot(
                    version,
                    &serde_json::to_string(&legacy_defaults).unwrap()
                )
                .is_ok(),
                "v{version} must retain defaults for absent additive fields"
            );
        }
    }

    #[test]
    fn batch_item_projection_filters_hidden_units_and_private_container_selectors() {
        let mut tonic = item_prototype("test:tonic", "healing tonic", "Restorative.", &["tonic"]);
        tonic.stack_policy = ItemStackPolicy::Fungible;
        tonic.max_stack_units = 20;
        let mut satchel = item_prototype(
            "test:satchel",
            "archive satchel",
            "A bounded satchel.",
            &["satchel"],
        );
        satchel.container = Some(ItemContainerDefinition {
            max_slots: 20,
            max_weight_grams: 20_000,
        });
        let (mut world, actor) = connect_definition(WorldDefinition {
            start_room_id: "test:start".into(),
            item_prototypes: vec![tonic, satchel],
            initial_item_instances: vec![
                item_instance(
                    "test:tonic-visible",
                    "test:tonic",
                    InitialItemLocation::Room("test:start".into()),
                ),
                item_instance(
                    "test:tonic-hidden",
                    "test:tonic",
                    InitialItemLocation::Room("test:start".into()),
                ),
                item_instance(
                    "test:satchel-instance",
                    "test:satchel",
                    InitialItemLocation::Room("test:start".into()),
                ),
            ],
            rooms: vec![RoomDefinition {
                stable_id: "test:start".into(),
                title: "Start".into(),
                prose: vec!["A test room.".into()],
                image: None,
                audio: None,
                item_ids: Vec::new(),
                exits: Vec::new(),
            }],
        });
        world
            .configure_hidden_item("test:tonic-hidden", 20)
            .unwrap();
        world.set_observer_role(actor, AccountRole::Admin).unwrap();
        let peer = SessionId(72);
        world
            .connect(
                peer,
                CharacterSnapshot {
                    id: CharacterId(702),
                    account_id: AccountId(72),
                    name: "Witness".into(),
                    room_id: "test:start".into(),
                    item_locations: BTreeMap::new(),
                },
            )
            .unwrap();
        let peer_visible_tonic = world
            .scene_for(peer)
            .unwrap()
            .items
            .into_iter()
            .find(|item| item.label == "healing tonic")
            .unwrap()
            .id;

        let take = world
            .apply(
                actor,
                PlayerAction::Take(ItemSelection {
                    query: "tonic".into(),
                    quantity: 2,
                }),
            )
            .unwrap();
        let actor_event = world
            .project_event(actor, &take.domain_events[0])
            .unwrap()
            .unwrap();
        let peer_event = world
            .project_event(peer, &take.domain_events[0])
            .unwrap()
            .unwrap();
        assert!(actor_event.text.contains("2 × healing tonic"));
        assert!(peer_event.text.contains("1 × healing tonic"));
        assert_eq!(
            peer_event.item_id.as_deref(),
            Some(peer_visible_tonic.as_str())
        );

        world
            .apply(actor, PlayerAction::Take(ItemSelection::one("satchel")))
            .unwrap();
        let put = world
            .apply(
                actor,
                PlayerAction::Put {
                    selection: ItemSelection::one("tonic"),
                    container: "satchel".into(),
                },
            )
            .unwrap();
        let peer_put = world
            .project_event(peer, &put.domain_events[0])
            .unwrap()
            .unwrap();
        assert!(peer_put.text.contains("puts 1 × healing tonic"));
        assert!(peer_put.target_id.is_none());
        assert!(peer_put.item_id.is_none());
    }

    #[test]
    fn batch_replay_supports_offline_owners_but_rejects_foreign_container_roots() {
        let mut tonic = item_prototype("test:tonic", "healing tonic", "Restorative.", &["tonic"]);
        tonic.stack_policy = ItemStackPolicy::Fungible;
        tonic.max_stack_units = 20;
        let mut satchel = item_prototype(
            "test:satchel",
            "archive satchel",
            "A bounded satchel.",
            &["satchel"],
        );
        satchel.container = Some(ItemContainerDefinition {
            max_slots: 20,
            max_weight_grams: 20_000,
        });
        let definition = WorldDefinition {
            start_room_id: "test:start".into(),
            item_prototypes: vec![tonic, satchel],
            initial_item_instances: vec![
                item_instance(
                    "test:tonic-a",
                    "test:tonic",
                    InitialItemLocation::Room("test:start".into()),
                ),
                item_instance(
                    "test:tonic-b",
                    "test:tonic",
                    InitialItemLocation::Room("test:start".into()),
                ),
                item_instance(
                    "test:satchel-instance",
                    "test:satchel",
                    InitialItemLocation::Room("test:start".into()),
                ),
            ],
            rooms: vec![RoomDefinition {
                stable_id: "test:start".into(),
                title: "Start".into(),
                prose: vec!["A test room.".into()],
                image: None,
                audio: None,
                item_ids: Vec::new(),
                exits: Vec::new(),
            }],
        };
        let offline_owner = CharacterId(900);
        let offline_take = DomainEvent::ItemsMoved {
            actor_id: offline_owner,
            actor_name: "Offline owner".into(),
            item_ids: vec!["test:tonic-a".into(), "test:tonic-b".into()],
            item_name: "healing tonic".into(),
            container_id: None,
            operation: ContainerMoveOperation::Take,
            room_id: "test:start".into(),
        };
        let mut offline_world = World::from_definition(definition.clone()).unwrap();
        offline_world.replay_persisted_event(&offline_take).unwrap();
        assert_eq!(
            offline_world.runtime_item_owner_ids(),
            BTreeSet::from([offline_owner])
        );

        let (mut world, actor) = connect_definition(definition);
        let victim = SessionId(73);
        world
            .connect(
                victim,
                CharacterSnapshot {
                    id: CharacterId(703),
                    account_id: AccountId(73),
                    name: "Other owner".into(),
                    room_id: "test:start".into(),
                    item_locations: BTreeMap::new(),
                },
            )
            .unwrap();
        world
            .apply(actor, PlayerAction::Take(ItemSelection::one("tonic")))
            .unwrap();
        world
            .apply(victim, PlayerAction::Take(ItemSelection::one("satchel")))
            .unwrap();
        let forged = DomainEvent::ItemsMoved {
            actor_id: CharacterId(701),
            actor_name: "Stack Tester".into(),
            item_ids: vec!["test:tonic-a".into()],
            item_name: "healing tonic".into(),
            container_id: Some("test:satchel-instance".into()),
            operation: ContainerMoveOperation::Put,
            room_id: "test:start".into(),
        };
        let before = world.runtime_snapshot();
        assert!(matches!(
            world.replay_persisted_event(&forged),
            Err(WorldError::InvalidDomainEvent)
        ));
        assert_eq!(world.runtime_snapshot(), before);
    }

    #[test]
    fn rollback_revalidates_inventory_capacity_before_publishing_locations() {
        let tonic = item_prototype("test:tonic", "healing tonic", "Restorative.", &["tonic"]);
        let mut satchel = item_prototype(
            "test:satchel",
            "archive satchel",
            "A bounded satchel.",
            &["satchel"],
        );
        satchel.container = Some(ItemContainerDefinition {
            max_slots: 20,
            max_weight_grams: 20_000,
        });
        let (mut world, actor) = connect_definition(WorldDefinition {
            start_room_id: "test:start".into(),
            item_prototypes: vec![tonic, satchel],
            initial_item_instances: vec![
                item_instance(
                    "test:tonic-a",
                    "test:tonic",
                    InitialItemLocation::Room("test:start".into()),
                ),
                item_instance(
                    "test:satchel-instance",
                    "test:satchel",
                    InitialItemLocation::Room("test:start".into()),
                ),
            ],
            rooms: vec![RoomDefinition {
                stable_id: "test:start".into(),
                title: "Start".into(),
                prose: vec!["A test room.".into()],
                image: None,
                audio: None,
                item_ids: Vec::new(),
                exits: Vec::new(),
            }],
        });
        world
            .apply(actor, PlayerAction::Take(ItemSelection::one("tonic")))
            .unwrap();
        world
            .apply(actor, PlayerAction::Take(ItemSelection::one("satchel")))
            .unwrap();
        let put = world
            .apply(
                actor,
                PlayerAction::Put {
                    selection: ItemSelection::one("tonic"),
                    container: "satchel".into(),
                },
            )
            .unwrap();
        world
            .character_gameplay
            .get_mut(&CharacterId(701))
            .unwrap()
            .inventory_capacity
            .max_slots = 1;
        let before = world.runtime_snapshot();
        assert!(matches!(
            world.rollback_domain_events(&put.domain_events),
            Err(WorldError::InventoryCapacityExceeded)
        ));
        assert_eq!(world.runtime_snapshot(), before);
    }

    #[test]
    fn acquisition_binds_recursively_and_restricted_subtrees_cannot_escape_owner() {
        let (mut world, actor) = connect_definition(policy_definition());
        let actor_id = CharacterId(701);
        world
            .connect(
                SessionId(72),
                CharacterSnapshot {
                    id: CharacterId(702),
                    account_id: AccountId(72),
                    name: "Aria".into(),
                    room_id: "test:start".into(),
                    item_locations: BTreeMap::new(),
                },
            )
            .unwrap();
        let clasp_id = ItemInstanceId("policy:clasp-instance".into());
        let token_id = ItemInstanceId("policy:token-instance".into());
        assert_eq!(
            world.runtime_snapshot().item_instances[&clasp_id].binding,
            ItemBinding::Unbound
        );

        let original_max_weight = world
            .character_gameplay
            .get(&actor_id)
            .unwrap()
            .inventory_capacity
            .max_weight_grams;
        world
            .character_gameplay
            .get_mut(&actor_id)
            .unwrap()
            .inventory_capacity
            .max_weight_grams = 1_000;
        let before_failed_script_grant = world.runtime_snapshot();
        assert!(matches!(
            world.give_authorized_script_item(actor, "policy:bag-instance"),
            Err(WorldError::InventoryCapacityExceeded)
        ));
        assert_eq!(world.runtime_snapshot(), before_failed_script_grant);
        assert_eq!(
            world.runtime_snapshot().item_instances[&clasp_id].binding,
            ItemBinding::Unbound
        );
        world
            .character_gameplay
            .get_mut(&actor_id)
            .unwrap()
            .inventory_capacity
            .max_weight_grams = original_max_weight;

        let take_bag = world
            .apply(actor, PlayerAction::Take(ItemSelection::one("bag")))
            .unwrap();
        assert_eq!(
            world.runtime_snapshot().item_instances[&clasp_id].binding,
            ItemBinding::Character {
                character_id: actor_id
            }
        );
        assert_eq!(world.runtime_item_owner_ids(), BTreeSet::from([actor_id]));
        let bag = world
            .player_state_view(actor)
            .unwrap()
            .inventory
            .into_iter()
            .find(|item| item.label == "traveller bag")
            .unwrap();
        assert_eq!(bag.transfer_policy, ItemTransferPolicy::Transferable);
        assert!(!bag.bound_to_you);
        assert!(!bag.can_give);
        assert!(!bag.actions.iter().any(|action| action.label == "Drop"));

        let before_forbidden = world.runtime_snapshot();
        let forged_restricted_give = DomainEvent::ItemsGiven {
            giver_id: actor_id,
            giver_name: "Stack Tester".into(),
            recipient_id: CharacterId(702),
            recipient_name: "Aria".into(),
            item_ids: vec!["policy:bag-instance".into()],
            item_name: "traveller bag".into(),
            room_id: "test:start".into(),
        };
        assert!(matches!(
            world.replay_persisted_event(&forged_restricted_give),
            Err(WorldError::InvalidDomainEvent)
        ));
        assert_eq!(world.runtime_snapshot(), before_forbidden);
        assert!(matches!(
            world.apply(actor, PlayerAction::Drop(ItemSelection::one("bag"))),
            Err(WorldError::ItemTransferForbidden)
        ));
        assert!(matches!(
            world.apply(
                actor,
                PlayerAction::Give {
                    selection: ItemSelection::one("bag"),
                    recipient: "Aria".into(),
                }
            ),
            Err(WorldError::ItemTransferForbidden)
        ));
        assert_eq!(world.runtime_snapshot(), before_forbidden);

        world
            .apply(actor, PlayerAction::Take(ItemSelection::one("token")))
            .unwrap();
        assert_eq!(
            world.runtime_snapshot().item_instances[&token_id].binding,
            ItemBinding::Unbound
        );
        let restricted = world.runtime_snapshot();
        assert!(matches!(
            world.apply(actor, PlayerAction::Drop(ItemSelection::one("token"))),
            Err(WorldError::ItemTransferForbidden)
        ));
        assert!(matches!(
            world.apply(
                actor,
                PlayerAction::Give {
                    selection: ItemSelection::one("token"),
                    recipient: "Aria".into(),
                }
            ),
            Err(WorldError::ItemTransferForbidden)
        ));
        assert!(matches!(
            world.apply(
                actor,
                PlayerAction::Put {
                    selection: ItemSelection::one("token"),
                    container: "chest".into(),
                }
            ),
            Err(WorldError::ItemTransferForbidden)
        ));
        assert_eq!(world.runtime_snapshot(), restricted);

        world
            .configure_item_gameplay(ItemGameplayDefinition {
                item_id: "policy:token".into(),
                equipment_slot: None,
                attack_bonus: 0,
                heal_amount: 1,
                consumable: true,
                effect: None,
            })
            .unwrap();
        world
            .apply(actor, PlayerAction::Use("token".into()))
            .unwrap();
        let consumed_token_snapshot = world.runtime_snapshot();
        let consumed_token = &consumed_token_snapshot.item_instances[&token_id];
        assert_eq!(consumed_token.location, RuntimeItemLocation::Consumed);
        assert_eq!(consumed_token.binding, ItemBinding::Unbound);
        validate_item_graph(&world.item_instances, &world.item_prototypes).unwrap();
        let mut restored_consumed = World::from_definition(policy_definition()).unwrap();
        restored_consumed
            .restore_runtime_snapshot(consumed_token_snapshot)
            .unwrap();

        world
            .apply(actor, PlayerAction::Take(ItemSelection::one("chest")))
            .unwrap();
        world
            .apply(
                actor,
                PlayerAction::Put {
                    selection: ItemSelection::one("bag"),
                    container: "chest".into(),
                },
            )
            .unwrap();
        world
            .apply(
                actor,
                PlayerAction::Get {
                    selection: ItemSelection::one("bag"),
                    container: "chest".into(),
                },
            )
            .unwrap();
        assert_eq!(
            world.runtime_snapshot().item_instances[&clasp_id].binding,
            ItemBinding::Character {
                character_id: actor_id
            }
        );

        world
            .configure_item_gameplay(ItemGameplayDefinition {
                item_id: "policy:scripted".into(),
                equipment_slot: None,
                attack_bonus: 0,
                heal_amount: 1,
                consumable: true,
                effect: None,
            })
            .unwrap();
        let scripted_event = world
            .give_authorized_script_item(actor, "policy:scripted-instance")
            .unwrap()
            .unwrap();
        assert!(matches!(scripted_event, DomainEvent::ItemTaken { .. }));
        world
            .apply(actor, PlayerAction::Use("sigil".into()))
            .unwrap();
        let consumed_snapshot = world.runtime_snapshot();
        let scripted =
            &consumed_snapshot.item_instances[&ItemInstanceId("policy:scripted-instance".into())];
        assert_eq!(scripted.location, RuntimeItemLocation::Consumed);
        assert_eq!(
            scripted.binding,
            ItemBinding::Character {
                character_id: actor_id
            }
        );
        assert!(world.runtime_item_owner_ids().contains(&actor_id));

        let mut replay = World::from_definition(policy_definition()).unwrap();
        replay
            .replay_persisted_event(&take_bag.domain_events[0])
            .unwrap();
        assert_eq!(
            replay.runtime_snapshot().item_instances[&clasp_id].binding,
            ItemBinding::Character {
                character_id: actor_id
            }
        );
        replay
            .rollback_domain_events(&take_bag.domain_events)
            .unwrap();
        assert_eq!(
            replay.runtime_snapshot().item_instances[&clasp_id].binding,
            ItemBinding::Unbound
        );
    }

    #[test]
    fn give_is_atomic_capacity_bounded_opaque_and_replayable() {
        let definition = give_definition();
        let mut world = World::from_definition(definition.clone()).unwrap();
        let connect = |world: &mut World,
                       session: SessionId,
                       character_id: CharacterId,
                       name: &str,
                       room_id: &str| {
            world
                .connect(
                    session,
                    CharacterSnapshot {
                        id: character_id,
                        account_id: AccountId(character_id.0),
                        name: name.into(),
                        room_id: room_id.into(),
                        item_locations: BTreeMap::new(),
                    },
                )
                .unwrap();
        };
        let giver_session = SessionId(71);
        let recipient_session = SessionId(72);
        let observer_session = SessionId(73);
        let remote_session = SessionId(74);
        connect(
            &mut world,
            giver_session,
            CharacterId(701),
            "Stack Tester",
            "give:start",
        );
        connect(
            &mut world,
            recipient_session,
            CharacterId(702),
            "Aria",
            "give:start",
        );
        connect(
            &mut world,
            observer_session,
            CharacterId(703),
            "Bram",
            "give:start",
        );
        connect(
            &mut world,
            remote_session,
            CharacterId(704),
            "Distant",
            "give:remote",
        );

        let social = world.player_state_view(giver_session).unwrap().social;
        let aria = social
            .online
            .iter()
            .find(|entry| entry.name == "Aria")
            .unwrap();
        let aria_selector = aria.id.clone();
        // Selector hashing is skipped only for queries that cannot be one.
        assert!(is_opaque_id_shaped(&aria_selector));
        assert!(!is_opaque_id_shaped(&aria_selector.to_ascii_uppercase()));
        assert!(!is_opaque_id_shaped("Aria"));
        assert!(matches!(
            world.resolve_give_recipient(giver_session, &aria_selector.to_ascii_uppercase()),
            Err(WorldError::GiveTargetUnavailable)
        ));
        assert!(aria.nearby);
        assert!(
            !social
                .online
                .iter()
                .find(|entry| entry.name == "Distant")
                .unwrap()
                .nearby
        );
        assert_eq!(
            world
                .resolve_give_recipient(giver_session, &aria_selector)
                .unwrap()
                .character_id,
            CharacterId(702)
        );
        assert_eq!(
            world
                .resolve_give_recipient(giver_session, "aRiA")
                .unwrap()
                .character_id,
            CharacterId(702)
        );
        connect(
            &mut world,
            SessionId(75),
            CharacterId(705),
            "ARIA",
            "give:start",
        );
        assert!(matches!(
            world.resolve_give_recipient(giver_session, "aria"),
            Err(WorldError::GiveTargetUnavailable)
        ));
        for invalid in ["Stack Tester", "Distant", "Offline"] {
            assert!(matches!(
                world.resolve_give_recipient(giver_session, invalid),
                Err(WorldError::GiveTargetUnavailable)
            ));
        }

        let take = world
            .apply(
                giver_session,
                PlayerAction::Take(ItemSelection {
                    query: "token".into(),
                    quantity: 3,
                }),
            )
            .unwrap();
        assert!(world.player_state_view(giver_session).unwrap().inventory[0].can_give);
        world
            .character_gameplay
            .get_mut(&CharacterId(702))
            .unwrap()
            .inventory_capacity
            .max_slots = 1;
        let before_capacity_failure = world.runtime_snapshot();
        assert!(matches!(
            world.apply(
                giver_session,
                PlayerAction::Give {
                    selection: ItemSelection {
                        query: "token".into(),
                        quantity: 3,
                    },
                    recipient: aria_selector.clone(),
                }
            ),
            Err(WorldError::InventoryCapacityExceeded)
        ));
        assert_eq!(world.runtime_snapshot(), before_capacity_failure);

        let given = world
            .apply(
                giver_session,
                PlayerAction::Give {
                    selection: ItemSelection {
                        query: "token".into(),
                        quantity: 2,
                    },
                    recipient: aria_selector,
                },
            )
            .unwrap();
        let event = given.domain_events.first().unwrap();
        assert!(matches!(
            event,
            DomainEvent::ItemsGiven {
                giver_id: CharacterId(701),
                recipient_id: CharacterId(702),
                item_ids,
                ..
            } if item_ids.len() == 2
        ));
        let giver_view = world.project_event(giver_session, event).unwrap().unwrap();
        let recipient_view = world
            .project_event(recipient_session, event)
            .unwrap()
            .unwrap();
        let observer_view = world
            .project_event(observer_session, event)
            .unwrap()
            .unwrap();
        assert_eq!(giver_view.kind, ViewEventKind::ItemGiven);
        assert!(giver_view.text.contains("to Aria"));
        assert!(recipient_view.text.contains("gives you"));
        assert!(observer_view.text.contains("to Aria"));
        assert!(giver_view.item_id.is_some());
        assert!(recipient_view.item_id.is_some());
        assert_ne!(giver_view.item_id, recipient_view.item_id);
        assert!(observer_view.item_id.is_none());
        assert!(
            world
                .project_event(remote_session, event)
                .unwrap()
                .is_none()
        );
        assert_eq!(
            world.player_state_view(giver_session).unwrap().inventory[0].quantity,
            1
        );
        assert_eq!(
            world
                .player_state_view(recipient_session)
                .unwrap()
                .inventory[0]
                .quantity,
            2
        );

        let mut replay = World::from_definition(definition.clone()).unwrap();
        replay
            .replay_persisted_event(&take.domain_events[0])
            .unwrap();
        replay.replay_persisted_event(event).unwrap();
        assert_eq!(
            replay
                .runtime_snapshot()
                .item_instances
                .values()
                .filter(|instance| {
                    instance.location
                        == (RuntimeItemLocation::Carried {
                            character_id: CharacterId(702),
                        })
                })
                .count(),
            2
        );
        replay
            .rollback_domain_events(std::slice::from_ref(event))
            .unwrap();
        replay.rollback_domain_events(&take.domain_events).unwrap();
        let initial = World::from_definition(definition)
            .unwrap()
            .runtime_snapshot();
        assert_eq!(replay.runtime_snapshot(), initial);

        world.disconnect(recipient_session);
        assert_eq!(
            world
                .runtime_snapshot()
                .item_instances
                .values()
                .filter(|instance| {
                    instance.location
                        == (RuntimeItemLocation::Carried {
                            character_id: CharacterId(702),
                        })
                })
                .count(),
            2
        );
    }

    #[test]
    fn confirmed_trade_reserves_invalidates_and_settles_both_sides_atomically() {
        let mut definition = give_definition();
        let mut bag = item_prototype(
            "give:bag",
            "trade satchel",
            "A container offered with visible contents.",
            &["satchel"],
        );
        bag.container = Some(ItemContainerDefinition {
            max_slots: 4,
            max_weight_grams: 2_000,
        });
        let pebble = item_prototype(
            "give:pebble",
            "moon pebble",
            "A small transferable stone.",
            &["pebble"],
        );
        definition.item_prototypes.extend([bag, pebble]);
        definition.initial_item_instances.extend([
            item_instance(
                "give:bag-instance",
                "give:bag",
                InitialItemLocation::Room("give:start".into()),
            ),
            item_instance(
                "give:pebble-instance",
                "give:pebble",
                InitialItemLocation::Container("give:bag-instance".into()),
            ),
        ]);
        let mut world = World::from_definition(definition.clone()).unwrap();
        let connect = |world: &mut World, session, id, name: &str| {
            world
                .connect(
                    session,
                    CharacterSnapshot {
                        id,
                        account_id: AccountId(id.0),
                        name: name.into(),
                        room_id: "give:start".into(),
                        item_locations: BTreeMap::new(),
                    },
                )
                .unwrap();
        };
        let a = SessionId(801);
        let b = SessionId(802);
        connect(&mut world, a, CharacterId(801), "Aster");
        connect(&mut world, b, CharacterId(802), "Bryn");
        let take = world
            .apply(
                a,
                PlayerAction::Take(ItemSelection {
                    query: "token".into(),
                    quantity: 2,
                }),
            )
            .unwrap();
        let take_bag = world
            .apply(a, PlayerAction::Take(ItemSelection::one("satchel")))
            .unwrap();

        let requested = world
            .apply(a, PlayerAction::Trade(TradeAction::Request("Bryn".into())))
            .unwrap();
        assert!(matches!(
            requested.domain_events.as_slice(),
            [DomainEvent::TradeRequested { trade }] if trade.state_revision == 1
        ));
        assert_eq!(
            world.runtime_snapshot().snapshot_version,
            WORLD_RUNTIME_SNAPSHOT_VERSION
        );
        assert_eq!(world.runtime_snapshot().next_trade_id, TradeId(2));
        let mut replay_events = requested.domain_events.clone();
        let opened = world
            .apply(b, PlayerAction::Trade(TradeAction::Accept))
            .unwrap();
        replay_events.extend(opened.domain_events);
        let item_offer = world
            .apply(
                a,
                PlayerAction::Trade(TradeAction::OfferItem(ItemSelection::one("token"))),
            )
            .unwrap();
        replay_events.extend(item_offer.domain_events);
        let coin_offer = world
            .apply(b, PlayerAction::Trade(TradeAction::OfferCoins(5)))
            .unwrap();
        replay_events.extend(coin_offer.domain_events);
        let bag_offer = world
            .apply(
                a,
                PlayerAction::Trade(TradeAction::OfferItem(ItemSelection::one("satchel"))),
            )
            .unwrap();
        replay_events.extend(bag_offer.domain_events);

        let a_view = world.player_state_view(a).unwrap();
        assert_eq!(
            a_view
                .inventory
                .iter()
                .map(|item| item.quantity)
                .sum::<u16>(),
            3
        );
        let available = a_view
            .inventory
            .iter()
            .find(|item| item.available_quantity == 1)
            .unwrap();
        let reserved = a_view
            .inventory
            .iter()
            .find(|item| item.reserved_for_trade == 1)
            .unwrap();
        assert!(available.can_trade);
        assert!(!reserved.can_trade);
        assert_ne!(available.id, reserved.id);
        let trade_view = a_view.trade.as_ref().unwrap();
        assert_eq!(trade_view.state_revision, 5);
        assert!(trade_view.counterparty.starts_with('o'));
        assert!(!trade_view.counterparty.contains("Bryn"));
        assert_ne!(
            trade_view.counterparty,
            world
                .player_state_view(b)
                .unwrap()
                .trade
                .unwrap()
                .counterparty
        );
        assert_eq!(world.reserved_coins(CharacterId(802)), 5);
        assert_eq!(world.spendable_coins(CharacterId(802)).unwrap(), 35);
        let status = world
            .apply(a, PlayerAction::Trade(TradeAction::Status))
            .unwrap()
            .output
            .unwrap();
        assert!(status.contains("Trade o"));
        assert!(status.contains("with Bryn"));
        assert!(status.contains("Your offer"));
        assert!(status.contains("Counterpart offer: 5 coin"));
        assert!(status.contains("reserved"));
        assert!(status.contains("spendable"));
        assert!(status.contains("ref o"));
        assert!(!status.contains("give:token"));
        assert!(!status.contains("give:bag"));
        let bag_target = trade_view
            .own_offer
            .lines
            .iter()
            .find(|line| line.container)
            .unwrap()
            .target
            .clone();
        let contents = world
            .apply(
                a,
                PlayerAction::Trade(TradeAction::Contents {
                    container: bag_target,
                    page: 1,
                }),
            )
            .unwrap()
            .output
            .unwrap();
        assert!(contents.contains("Offered container contents, page 1/1"));
        assert!(contents.contains("moon pebble"));
        assert!(contents.contains("ref o"));
        assert!(!contents.contains("give:pebble-instance"));
        assert!(matches!(
            world.apply(
                a,
                PlayerAction::Drop(ItemSelection::one(reserved.id.clone()))
            ),
            Err(WorldError::ItemReservedForTrade)
        ));
        assert!(matches!(
            world.apply(
                a,
                PlayerAction::Trade(TradeAction::Confirm { state_revision: 3 })
            ),
            Err(WorldError::StaleTradeRevision)
        ));

        let first_confirmation = world
            .apply(
                a,
                PlayerAction::Trade(TradeAction::Confirm { state_revision: 5 }),
            )
            .unwrap();
        replay_events.extend(first_confirmation.domain_events);
        let checkpoint = world.checkpoint();
        let completed = world
            .apply(
                b,
                PlayerAction::Trade(TradeAction::Confirm { state_revision: 5 }),
            )
            .unwrap();
        assert!(matches!(
            completed.domain_events.as_slice(),
            [DomainEvent::TradeCompleted { .. }]
        ));
        replay_events.extend(completed.domain_events.clone());
        assert!(world.trades.is_empty());
        assert_eq!(
            world
                .character_gameplay
                .get(&CharacterId(801))
                .unwrap()
                .coins,
            45
        );
        assert_eq!(
            world
                .character_gameplay
                .get(&CharacterId(802))
                .unwrap()
                .coins,
            35
        );
        assert_eq!(
            world
                .runtime_snapshot()
                .item_instances
                .values()
                .filter(|instance| instance.location
                    == RuntimeItemLocation::Carried {
                        character_id: CharacterId(802)
                    })
                .count(),
            2
        );

        let mut replay = World::from_definition(definition).unwrap();
        replay
            .replay_persisted_event(&take.domain_events[0])
            .unwrap();
        replay
            .replay_persisted_event(&take_bag.domain_events[0])
            .unwrap();
        for event in &replay_events {
            replay.replay_persisted_event(event).unwrap();
        }
        assert!(replay.runtime_snapshot().trades.is_empty());
        assert_eq!(
            replay
                .runtime_snapshot()
                .item_instances
                .values()
                .filter(|instance| instance.location
                    == RuntimeItemLocation::Carried {
                        character_id: CharacterId(802)
                    })
                .count(),
            2
        );

        world.restore_checkpoint(checkpoint);
        assert_eq!(world.trades.len(), 1);
        assert_eq!(
            world
                .character_gameplay
                .get(&CharacterId(801))
                .unwrap()
                .coins,
            40
        );
        assert_eq!(
            world
                .character_gameplay
                .get(&CharacterId(802))
                .unwrap()
                .coins,
            40
        );
    }

    #[test]
    fn trade_candidate_enforces_the_global_reservation_cap_and_portable_semantics() {
        let mut prototype = item_prototype(
            "cap:item",
            "capacity token",
            "A token used to exercise the global reservation boundary.",
            &["token"],
        );
        prototype.stack_policy = ItemStackPolicy::Fungible;
        prototype.max_stack_units = 256;
        let item_ids = (0..=MAX_RESERVED_TRADE_ROOTS)
            .map(|index| ItemInstanceId(format!("cap:item-{index:04}")))
            .collect::<Vec<_>>();
        let mut world = World::from_definition(WorldDefinition {
            start_room_id: "cap:start".into(),
            item_prototypes: vec![prototype],
            initial_item_instances: item_ids
                .iter()
                .map(|item_id| {
                    item_instance(
                        item_id.as_str(),
                        "cap:item",
                        InitialItemLocation::Room("cap:start".into()),
                    )
                })
                .collect(),
            rooms: vec![RoomDefinition {
                stable_id: "cap:start".into(),
                title: "Capacity".into(),
                prose: vec!["A bounded reservation fixture.".into()],
                image: None,
                audio: None,
                exits: Vec::new(),
                item_ids: Vec::new(),
            }],
        })
        .unwrap();
        let make_trade = |id: u64, roots: Vec<ItemInstanceId>| {
            let requester = CharacterId(10_000 + i64::try_from(id).unwrap() * 2);
            let recipient = CharacterId(requester.0 + 1);
            let lines = (!roots.is_empty())
                .then(|| TradeOfferLine {
                    key: TradeOfferKey::Fungible {
                        prototype_id: "cap:item".into(),
                    },
                    item_ids: roots,
                })
                .into_iter()
                .collect();
            TradeSnapshot {
                id: TradeId(id),
                requester,
                recipient,
                room_id: "cap:start".into(),
                phase: TradePhase::Negotiating,
                created_tick: 0,
                opened_tick: Some(0),
                expires_at_tick: TRADE_NEGOTIATION_TICKS,
                state_revision: 2,
                offers: BTreeMap::from([
                    (requester, TradeOffer { lines, coins: 0 }),
                    (recipient, TradeOffer::default()),
                ]),
                confirmations: BTreeMap::new(),
            }
        };
        for trade_index in 0..16_usize {
            let start = trade_index * usize::from(MAX_TRADE_ITEM_QUANTITY);
            let end = start + usize::from(MAX_TRADE_ITEM_QUANTITY);
            let roots = item_ids[start..end].to_vec();
            let trade = make_trade(u64::try_from(trade_index + 1).unwrap(), roots.clone());
            for root in roots {
                world.item_instances.get_mut(&root).unwrap().location =
                    SharedItemLocation::Carried(trade.requester);
            }
            world.trades.insert(trade.id, trade);
        }
        let previous = make_trade(17, Vec::new());
        world.trades.insert(previous.id, previous.clone());
        world.next_trade_id = TradeId(18);
        validate_trade_state(
            world.next_trade_id,
            &world.trades,
            &world.item_instances,
            &world.item_prototypes,
            &world.stable_rooms,
        )
        .unwrap();
        validate_world_runtime_snapshot_semantics(&world.runtime_snapshot()).unwrap();

        let overflow_root = item_ids[MAX_RESERVED_TRADE_ROOTS].clone();
        world
            .item_instances
            .get_mut(&overflow_root)
            .unwrap()
            .location = SharedItemLocation::Carried(previous.requester);
        let current = make_trade(17, vec![overflow_root]);
        let revision = world.revision;
        assert!(matches!(
            world.publish_trade_offer_change(SessionId(99_999), previous.clone(), current.clone()),
            Err(WorldError::InvalidRuntimeSnapshot)
        ));
        assert_eq!(world.trades.get(&TradeId(17)), Some(&previous));
        assert_eq!(world.revision, revision);

        let mut overflow_snapshot = world.runtime_snapshot();
        overflow_snapshot.trades.insert(TradeId(17), current);
        assert!(matches!(
            validate_world_runtime_snapshot_semantics(&overflow_snapshot),
            Err(WorldError::InvalidRuntimeSnapshot)
        ));
    }

    #[test]
    fn trade_replay_rejects_id_reuse_and_forged_transitions_or_confirmations() {
        let requested = TradeSnapshot {
            id: TradeId(1),
            requester: CharacterId(901),
            recipient: CharacterId(902),
            room_id: "give:start".into(),
            phase: TradePhase::Requested,
            created_tick: 0,
            opened_tick: None,
            expires_at_tick: TRADE_REQUEST_TICKS,
            state_revision: 1,
            offers: BTreeMap::new(),
            confirmations: BTreeMap::new(),
        };
        let mut anti_reuse = World::from_definition(give_definition()).unwrap();
        anti_reuse
            .replay_persisted_event(&DomainEvent::TradeRequested {
                trade: requested.clone(),
            })
            .unwrap();
        anti_reuse
            .replay_persisted_event(&DomainEvent::TradeCancelled {
                previous_trade: requested.clone(),
                reason: TradeCancellationReason::CancelledByParticipant,
            })
            .unwrap();
        assert!(matches!(
            anti_reuse.replay_persisted_event(&DomainEvent::TradeRequested {
                trade: requested.clone(),
            }),
            Err(WorldError::InvalidDomainTransition)
        ));

        let mut replay = World::from_definition(give_definition()).unwrap();
        for (session, character_id, name) in [
            (SessionId(901), CharacterId(901), "Aster"),
            (SessionId(902), CharacterId(902), "Bryn"),
        ] {
            replay
                .connect(
                    session,
                    CharacterSnapshot {
                        id: character_id,
                        account_id: AccountId(character_id.0),
                        name: name.into(),
                        room_id: "give:start".into(),
                        item_locations: BTreeMap::new(),
                    },
                )
                .unwrap();
        }
        replay
            .replay_persisted_event(&DomainEvent::TradeRequested {
                trade: requested.clone(),
            })
            .unwrap();
        let mut opened = requested.clone();
        opened.phase = TradePhase::Negotiating;
        opened.opened_tick = Some(0);
        opened.expires_at_tick = TRADE_NEGOTIATION_TICKS;
        opened.state_revision = 2;
        opened.offers = BTreeMap::from([
            (opened.requester, TradeOffer::default()),
            (opened.recipient, TradeOffer::default()),
        ]);
        let mut opened_after_expiry = opened.clone();
        opened_after_expiry.opened_tick = Some(TRADE_REQUEST_TICKS);
        opened_after_expiry.expires_at_tick = TRADE_REQUEST_TICKS + TRADE_NEGOTIATION_TICKS;
        assert!(matches!(
            replay.replay_persisted_event(&DomainEvent::TradeOpened {
                previous: requested.clone(),
                current: opened_after_expiry,
            }),
            Err(WorldError::InvalidDomainTransition)
        ));
        let mut forged_open = opened.clone();
        forged_open.room_id = "give:remote".into();
        assert!(matches!(
            replay.replay_persisted_event(&DomainEvent::TradeOpened {
                previous: requested.clone(),
                current: forged_open,
            }),
            Err(WorldError::InvalidDomainTransition)
        ));
        replay
            .replay_persisted_event(&DomainEvent::TradeOpened {
                previous: requested,
                current: opened.clone(),
            })
            .unwrap();

        let mut offered = opened.clone();
        offered.state_revision = 3;
        offered.offers.get_mut(&offered.requester).unwrap().coins = 1;
        let mut forged_offer = offered.clone();
        forged_offer
            .offers
            .get_mut(&forged_offer.recipient)
            .unwrap()
            .coins = 1;
        assert!(matches!(
            replay.replay_persisted_event(&DomainEvent::TradeOfferSet {
                previous: opened.clone(),
                current: forged_offer,
            }),
            Err(WorldError::InvalidDomainTransition)
        ));
        replay
            .replay_persisted_event(&DomainEvent::TradeOfferSet {
                previous: opened,
                current: offered.clone(),
            })
            .unwrap();

        let mut forged_invalidation = offered.clone();
        forged_invalidation.state_revision = 4;
        forged_invalidation
            .offers
            .get_mut(&forged_invalidation.requester)
            .unwrap()
            .coins = 2;
        assert!(matches!(
            replay.replay_persisted_event(&DomainEvent::TradeInvalidated {
                previous: offered.clone(),
                current: forged_invalidation,
            }),
            Err(WorldError::InvalidDomainTransition)
        ));

        let basis = replay.trade_basis_sha256(&offered).unwrap();
        let confirmation = TradeConfirmation {
            state_revision: offered.state_revision,
            basis_sha256: basis,
        };
        let mut forged_confirmation = confirmation.clone();
        forged_confirmation.basis_sha256 = [0x5a; 32];
        assert!(matches!(
            replay.replay_persisted_event(&DomainEvent::TradeConfirmed {
                participant: offered.requester,
                confirmation: forged_confirmation,
            }),
            Err(WorldError::InvalidDomainTransition)
        ));
        replay
            .replay_persisted_event(&DomainEvent::TradeConfirmed {
                participant: offered.requester,
                confirmation: confirmation.clone(),
            })
            .unwrap();

        let mut completed = offered;
        completed
            .confirmations
            .insert(completed.requester, confirmation.clone());
        completed
            .confirmations
            .insert(completed.recipient, confirmation.clone());
        let mut outsider = completed.clone();
        outsider.confirmations.remove(&outsider.recipient);
        outsider
            .confirmations
            .insert(CharacterId(999), confirmation.clone());
        assert!(matches!(
            replay.replay_persisted_event(&DomainEvent::TradeCompleted {
                previous_trade: outsider,
            }),
            Err(WorldError::InvalidDomainTransition)
        ));
        let mut bad_digest = completed.clone();
        bad_digest
            .confirmations
            .get_mut(&bad_digest.recipient)
            .unwrap()
            .basis_sha256 = [0xa5; 32];
        assert!(matches!(
            replay.replay_persisted_event(&DomainEvent::TradeCompleted {
                previous_trade: bad_digest,
            }),
            Err(WorldError::InvalidDomainTransition)
        ));
        replay
            .replay_persisted_event(&DomainEvent::TradeCompleted {
                previous_trade: completed,
            })
            .unwrap();
        assert!(replay.trades.is_empty());
    }

    #[test]
    fn cold_trade_replay_rejects_assetless_arbitrary_confirmations() {
        let requested = TradeSnapshot {
            id: TradeId(1),
            requester: CharacterId(961),
            recipient: CharacterId(962),
            room_id: "give:start".into(),
            phase: TradePhase::Requested,
            created_tick: 0,
            opened_tick: None,
            expires_at_tick: TRADE_REQUEST_TICKS,
            state_revision: 1,
            offers: BTreeMap::new(),
            confirmations: BTreeMap::new(),
        };
        let mut opened = requested.clone();
        opened.phase = TradePhase::Negotiating;
        opened.opened_tick = Some(0);
        opened.expires_at_tick = TRADE_NEGOTIATION_TICKS;
        opened.state_revision = 2;
        opened.offers = BTreeMap::from([
            (opened.requester, TradeOffer::default()),
            (opened.recipient, TradeOffer::default()),
        ]);
        let mut replay = World::from_definition(give_definition()).unwrap();
        replay
            .replay_persisted_event(&DomainEvent::TradeRequested {
                trade: requested.clone(),
            })
            .unwrap();
        replay
            .replay_persisted_event(&DomainEvent::TradeOpened {
                previous: requested,
                current: opened.clone(),
            })
            .unwrap();
        assert!(replay.character_gameplay.is_empty());
        let arbitrary = TradeConfirmation {
            state_revision: opened.state_revision,
            basis_sha256: [0x42; 32],
        };
        assert!(matches!(
            replay.replay_persisted_event(&DomainEvent::TradeConfirmed {
                participant: opened.requester,
                confirmation: arbitrary.clone(),
            }),
            Err(WorldError::InvalidDomainTransition)
        ));

        let mut one_confirmation = opened.clone();
        one_confirmation
            .confirmations
            .insert(opened.requester, arbitrary.clone());
        replay.trades.insert(opened.id, one_confirmation);
        let mut completed = opened;
        completed
            .confirmations
            .insert(completed.requester, arbitrary.clone());
        completed
            .confirmations
            .insert(completed.recipient, arbitrary);
        assert!(matches!(
            replay.replay_persisted_event(&DomainEvent::TradeCompleted {
                previous_trade: completed,
            }),
            Err(WorldError::InvalidDomainTransition)
        ));
        assert_eq!(replay.trades.len(), 1);
    }

    #[test]
    fn trade_rate_expiry_and_tick_counters_fail_closed() {
        let mut world = World::from_definition(give_definition()).unwrap();
        for (session, character_id, name) in [
            (SessionId(911), CharacterId(911), "Aster"),
            (SessionId(912), CharacterId(912), "Bryn"),
        ] {
            world
                .connect(
                    session,
                    CharacterSnapshot {
                        id: character_id,
                        account_id: AccountId(character_id.0),
                        name: name.into(),
                        room_id: "give:start".into(),
                        item_locations: BTreeMap::new(),
                    },
                )
                .unwrap();
        }
        world
            .apply(
                SessionId(911),
                PlayerAction::Trade(TradeAction::Request("Bryn".into())),
            )
            .unwrap();
        world
            .apply(SessionId(911), PlayerAction::Trade(TradeAction::Cancel))
            .unwrap();
        let rate_checkpoint = world.checkpoint();
        assert!(matches!(
            world.apply(
                SessionId(911),
                PlayerAction::Trade(TradeAction::Request("Bryn".into()))
            ),
            Err(WorldError::TradeUnavailable)
        ));
        for _ in 0..TRADE_REQUEST_RATE_TICKS {
            world.advance_tick().unwrap();
        }
        world.restore_checkpoint(rate_checkpoint);
        assert!(matches!(
            world.apply(
                SessionId(911),
                PlayerAction::Trade(TradeAction::Request("Bryn".into()))
            ),
            Err(WorldError::TradeUnavailable)
        ));
        for _ in 0..TRADE_REQUEST_RATE_TICKS {
            world.advance_tick().unwrap();
        }
        world
            .apply(
                SessionId(911),
                PlayerAction::Trade(TradeAction::Request("Bryn".into())),
            )
            .unwrap();

        world.logical_tick = TRADE_REQUEST_RATE_TICKS + TRADE_REQUEST_TICKS;
        world.revision = u64::MAX;
        let live_trade = world.trades.values().next().unwrap().clone();
        assert!(matches!(
            world.cancel_expired_trades(),
            Err(WorldError::InvalidDomainTransition)
        ));
        assert_eq!(world.trades.get(&live_trade.id), Some(&live_trade));
        world.revision = 1;
        let events = world.cancel_expired_trades().unwrap();
        assert!(matches!(
            events.as_slice(),
            [DomainEvent::TradeCancelled {
                reason: TradeCancellationReason::Expired,
                ..
            }]
        ));
        assert!(world.trades.is_empty());

        let mut overflow_tick = World::from_definition(give_definition()).unwrap();
        overflow_tick.logical_tick = u64::MAX;
        let before = overflow_tick.runtime_snapshot();
        assert!(matches!(
            overflow_tick.advance_tick(),
            Err(WorldError::InvalidDomainTransition)
        ));
        assert_eq!(overflow_tick.runtime_snapshot(), before);
    }

    #[test]
    fn trade_status_is_text_bounded_and_keeps_opaque_actionable_inventory() {
        let prototypes = (0..1_000)
            .map(|index| {
                let mut prototype = item_prototype(
                    &format!("status:item-{index:04}"),
                    &format!("status item {index:04}"),
                    "A unique item used to bound trade status output.",
                    &[],
                );
                prototype.weight_grams = 1;
                prototype
            })
            .collect::<Vec<_>>();
        let instances = (0..1_000)
            .map(|index| {
                item_instance(
                    &format!("status:instance-{index:04}"),
                    &format!("status:item-{index:04}"),
                    InitialItemLocation::Room("status:start".into()),
                )
            })
            .collect::<Vec<_>>();
        let mut world = World::from_definition(WorldDefinition {
            start_room_id: "status:start".into(),
            item_prototypes: prototypes,
            initial_item_instances: instances,
            rooms: vec![RoomDefinition {
                stable_id: "status:start".into(),
                title: "Status".into(),
                prose: vec!["A bounded text fixture.".into()],
                image: None,
                audio: None,
                exits: Vec::new(),
                item_ids: Vec::new(),
            }],
        })
        .unwrap();
        for (session, character_id, name) in [
            (SessionId(921), CharacterId(921), "Aster"),
            (SessionId(922), CharacterId(922), "Bryn"),
        ] {
            world
                .connect(
                    session,
                    CharacterSnapshot {
                        id: character_id,
                        account_id: AccountId(character_id.0),
                        name: name.into(),
                        room_id: "status:start".into(),
                        item_locations: BTreeMap::new(),
                    },
                )
                .unwrap();
        }
        for instance in world.item_instances.values_mut() {
            instance.location = SharedItemLocation::Carried(CharacterId(921));
        }
        world
            .character_gameplay
            .get_mut(&CharacterId(921))
            .unwrap()
            .inventory_capacity = InventoryCapacity {
            max_slots: 1_000,
            max_weight_grams: 1_000_000,
        };
        world
            .apply(
                SessionId(921),
                PlayerAction::Trade(TradeAction::Request("Bryn".into())),
            )
            .unwrap();
        let snapshot = world.player_state_view(SessionId(921)).unwrap();
        let counterparty = &snapshot.trade.as_ref().unwrap().counterparty;
        assert!(counterparty.starts_with('o'));
        assert!(!counterparty.contains("Bryn"));
        let status = world
            .apply(SessionId(921), PlayerAction::Trade(TradeAction::Status))
            .unwrap()
            .output
            .unwrap();
        assert!(status.len() <= MAX_TRADE_STATUS_TEXT_BYTES);
        assert!(status.contains("with Bryn"));
        assert!(status.contains("980 more inventory groups omitted"));
        assert!(status.contains("trade contents <offered-container> <page>"));
        assert!(status.contains("ref o"));
        assert!(!status.contains("status:instance-"));
    }

    #[test]
    fn trade_settlement_uses_net_capacity_and_reserved_subtrees_block_every_item_bypass() {
        let swap_definition = || WorldDefinition {
            start_room_id: "swap:start".into(),
            item_prototypes: vec![
                item_prototype(
                    "swap:amber",
                    "amber token",
                    "A unique amber token.",
                    &["amber"],
                ),
                item_prototype(
                    "swap:azure",
                    "azure token",
                    "A unique azure token.",
                    &["azure"],
                ),
            ],
            initial_item_instances: vec![
                item_instance(
                    "swap:amber-instance",
                    "swap:amber",
                    InitialItemLocation::Room("swap:start".into()),
                ),
                item_instance(
                    "swap:azure-instance",
                    "swap:azure",
                    InitialItemLocation::Room("swap:start".into()),
                ),
            ],
            rooms: vec![RoomDefinition {
                stable_id: "swap:start".into(),
                title: "Swap".into(),
                prose: vec!["A net-capacity fixture.".into()],
                image: None,
                audio: None,
                exits: Vec::new(),
                item_ids: Vec::new(),
            }],
        };
        let connect_swap = |world: &mut World| {
            for (session, character_id, name) in [
                (SessionId(931), CharacterId(931), "Aster"),
                (SessionId(932), CharacterId(932), "Bryn"),
            ] {
                world
                    .connect(
                        session,
                        CharacterSnapshot {
                            id: character_id,
                            account_id: AccountId(character_id.0),
                            name: name.into(),
                            room_id: "swap:start".into(),
                            item_locations: BTreeMap::new(),
                        },
                    )
                    .unwrap();
                world
                    .character_gameplay
                    .get_mut(&character_id)
                    .unwrap()
                    .inventory_capacity = InventoryCapacity {
                    max_slots: 1,
                    max_weight_grams: 2_000,
                };
            }
            world
                .apply(
                    SessionId(931),
                    PlayerAction::Take(ItemSelection::one("amber")),
                )
                .unwrap();
            world
                .apply(
                    SessionId(932),
                    PlayerAction::Take(ItemSelection::one("azure")),
                )
                .unwrap();
        };
        let mut swap = World::from_definition(swap_definition()).unwrap();
        connect_swap(&mut swap);
        swap.apply(
            SessionId(931),
            PlayerAction::Trade(TradeAction::Request("Bryn".into())),
        )
        .unwrap();
        swap.apply(SessionId(932), PlayerAction::Trade(TradeAction::Accept))
            .unwrap();
        swap.apply(
            SessionId(931),
            PlayerAction::Trade(TradeAction::OfferItem(ItemSelection::one("amber"))),
        )
        .unwrap();
        swap.apply(
            SessionId(932),
            PlayerAction::Trade(TradeAction::OfferItem(ItemSelection::one("azure"))),
        )
        .unwrap();
        swap.apply(
            SessionId(931),
            PlayerAction::Trade(TradeAction::Confirm { state_revision: 4 }),
        )
        .unwrap();
        swap.apply(
            SessionId(932),
            PlayerAction::Trade(TradeAction::Confirm { state_revision: 4 }),
        )
        .unwrap();
        assert_eq!(
            swap.item_location(&ItemInstanceId("swap:amber-instance".into())),
            Some(&SharedItemLocation::Carried(CharacterId(932)))
        );
        assert_eq!(
            swap.item_location(&ItemInstanceId("swap:azure-instance".into())),
            Some(&SharedItemLocation::Carried(CharacterId(931)))
        );

        let mut over_capacity = World::from_definition(swap_definition()).unwrap();
        connect_swap(&mut over_capacity);
        over_capacity
            .apply(
                SessionId(931),
                PlayerAction::Trade(TradeAction::Request("Bryn".into())),
            )
            .unwrap();
        over_capacity
            .apply(SessionId(932), PlayerAction::Trade(TradeAction::Accept))
            .unwrap();
        over_capacity
            .apply(
                SessionId(931),
                PlayerAction::Trade(TradeAction::OfferItem(ItemSelection::one("amber"))),
            )
            .unwrap();
        over_capacity
            .apply(
                SessionId(931),
                PlayerAction::Trade(TradeAction::Confirm { state_revision: 3 }),
            )
            .unwrap();
        assert!(matches!(
            over_capacity.apply(
                SessionId(932),
                PlayerAction::Trade(TradeAction::Confirm { state_revision: 3 })
            ),
            Err(WorldError::InventoryCapacityExceeded)
        ));
        assert_eq!(over_capacity.trades.len(), 1);

        let mut bag = item_prototype(
            "reserved:bag",
            "trade satchel",
            "A reserved container.",
            &["satchel"],
        );
        bag.container = Some(ItemContainerDefinition {
            max_slots: 4,
            max_weight_grams: 4_000,
        });
        let mut herb = item_prototype(
            "reserved:herb",
            "healing herb",
            "A herb inside the reserved container.",
            &["herb"],
        );
        herb.heal_amount = 1;
        herb.consumable = true;
        let spare = item_prototype(
            "reserved:spare",
            "spare stone",
            "A free item that must not enter a reserved container.",
            &["spare"],
        );
        let mut reserved = World::from_definition(WorldDefinition {
            start_room_id: "reserved:start".into(),
            item_prototypes: vec![bag, herb, spare],
            initial_item_instances: vec![
                item_instance(
                    "reserved:bag-instance",
                    "reserved:bag",
                    InitialItemLocation::Room("reserved:start".into()),
                ),
                item_instance(
                    "reserved:herb-instance",
                    "reserved:herb",
                    InitialItemLocation::Container("reserved:bag-instance".into()),
                ),
                item_instance(
                    "reserved:spare-instance",
                    "reserved:spare",
                    InitialItemLocation::Room("reserved:start".into()),
                ),
            ],
            rooms: vec![RoomDefinition {
                stable_id: "reserved:start".into(),
                title: "Reserved".into(),
                prose: vec!["A reservation bypass fixture.".into()],
                image: None,
                audio: None,
                exits: Vec::new(),
                item_ids: Vec::new(),
            }],
        })
        .unwrap();
        for (session, character_id, name) in [
            (SessionId(941), CharacterId(941), "Aster"),
            (SessionId(942), CharacterId(942), "Bryn"),
        ] {
            reserved
                .connect(
                    session,
                    CharacterSnapshot {
                        id: character_id,
                        account_id: AccountId(character_id.0),
                        name: name.into(),
                        room_id: "reserved:start".into(),
                        item_locations: BTreeMap::new(),
                    },
                )
                .unwrap();
        }
        reserved
            .apply(
                SessionId(941),
                PlayerAction::Take(ItemSelection::one("satchel")),
            )
            .unwrap();
        reserved
            .apply(
                SessionId(941),
                PlayerAction::Take(ItemSelection::one("spare")),
            )
            .unwrap();
        reserved
            .apply(
                SessionId(941),
                PlayerAction::Trade(TradeAction::Request("Bryn".into())),
            )
            .unwrap();
        reserved
            .apply(SessionId(942), PlayerAction::Trade(TradeAction::Accept))
            .unwrap();
        reserved
            .apply(
                SessionId(941),
                PlayerAction::Trade(TradeAction::OfferItem(ItemSelection::one("satchel"))),
            )
            .unwrap();
        let bag_id = ItemInstanceId("reserved:bag-instance".into());
        let herb_id = ItemInstanceId("reserved:herb-instance".into());
        assert!(reserved.item_reserved_for_trade(&bag_id).unwrap());
        assert!(reserved.item_reserved_for_trade(&herb_id).unwrap());
        let contents = reserved
            .apply(
                SessionId(941),
                PlayerAction::Contents {
                    container: "satchel".into(),
                    page: 1,
                },
            )
            .unwrap();
        let contents_view = contents.container_contents.as_ref().unwrap();
        let herb_view = contents_view
            .items
            .iter()
            .find(|item| item.label == "healing herb")
            .unwrap();
        assert_eq!(herb_view.quantity, 1);
        assert_eq!(herb_view.available_quantity, 0);
        assert_eq!(herb_view.reserved_for_trade, 1);
        assert!(!herb_view.can_trade);
        assert!(
            herb_view
                .actions
                .iter()
                .all(|action| action.label != "Get" && !action.command.starts_with("get "))
        );
        assert!(
            contents
                .output
                .as_deref()
                .unwrap()
                .contains("1 reserved for trade; 0 available")
        );
        let blocked = [
            PlayerAction::Drop(ItemSelection::one("satchel")),
            PlayerAction::Give {
                selection: ItemSelection::one("satchel"),
                recipient: "Bryn".into(),
            },
            PlayerAction::Put {
                selection: ItemSelection::one("spare"),
                container: "satchel".into(),
            },
            PlayerAction::Get {
                selection: ItemSelection::one("herb"),
                container: "satchel".into(),
            },
            PlayerAction::Use("satchel".into()),
            PlayerAction::Equip("satchel".into()),
        ];
        for action in blocked {
            assert!(matches!(
                reserved.apply(SessionId(941), action),
                Err(WorldError::ItemReservedForTrade)
            ));
        }
    }

    #[test]
    fn trade_revision_id_and_coin_overflows_are_atomic_and_shop_uses_spendable_coins() {
        let connect_pair = |world: &mut World| {
            for (session, character_id, name) in [
                (SessionId(951), CharacterId(951), "Aster"),
                (SessionId(952), CharacterId(952), "Bryn"),
            ] {
                world
                    .connect(
                        session,
                        CharacterSnapshot {
                            id: character_id,
                            account_id: AccountId(character_id.0),
                            name: name.into(),
                            room_id: "give:start".into(),
                            item_locations: BTreeMap::new(),
                        },
                    )
                    .unwrap();
            }
        };
        let mut revision_overflow = World::from_definition(give_definition()).unwrap();
        connect_pair(&mut revision_overflow);
        revision_overflow
            .apply(
                SessionId(951),
                PlayerAction::Trade(TradeAction::Request("Bryn".into())),
            )
            .unwrap();
        revision_overflow
            .apply(SessionId(952), PlayerAction::Trade(TradeAction::Accept))
            .unwrap();
        revision_overflow
            .trades
            .get_mut(&TradeId(1))
            .unwrap()
            .state_revision = u64::MAX;
        let before = revision_overflow.runtime_snapshot();
        assert!(matches!(
            revision_overflow.apply(
                SessionId(951),
                PlayerAction::Trade(TradeAction::OfferCoins(1))
            ),
            Err(WorldError::InvalidDomainTransition)
        ));
        assert_eq!(revision_overflow.runtime_snapshot(), before);

        let mut id_overflow = World::from_definition(give_definition()).unwrap();
        connect_pair(&mut id_overflow);
        id_overflow.next_trade_id = TradeId(u64::MAX);
        assert!(matches!(
            id_overflow.apply(
                SessionId(951),
                PlayerAction::Trade(TradeAction::Request("Bryn".into()))
            ),
            Err(WorldError::InvalidDomainTransition)
        ));
        assert!(id_overflow.trades.is_empty());
        assert_eq!(id_overflow.next_trade_id, TradeId(u64::MAX));

        let mut coin_overflow = World::from_definition(give_definition()).unwrap();
        connect_pair(&mut coin_overflow);
        coin_overflow
            .character_gameplay
            .get_mut(&CharacterId(951))
            .unwrap()
            .coins = u64::MAX;
        coin_overflow
            .apply(
                SessionId(951),
                PlayerAction::Trade(TradeAction::Request("Bryn".into())),
            )
            .unwrap();
        coin_overflow
            .apply(SessionId(952), PlayerAction::Trade(TradeAction::Accept))
            .unwrap();
        coin_overflow
            .apply(
                SessionId(952),
                PlayerAction::Trade(TradeAction::OfferCoins(1)),
            )
            .unwrap();
        coin_overflow
            .apply(
                SessionId(951),
                PlayerAction::Trade(TradeAction::Confirm { state_revision: 3 }),
            )
            .unwrap();
        assert!(matches!(
            coin_overflow.apply(
                SessionId(952),
                PlayerAction::Trade(TradeAction::Confirm { state_revision: 3 })
            ),
            Err(WorldError::InsufficientFunds)
        ));
        assert_eq!(coin_overflow.trades.len(), 1);

        let mut shop = World::from_definition(give_definition()).unwrap();
        connect_pair(&mut shop);
        shop.apply(
            SessionId(951),
            PlayerAction::Trade(TradeAction::Request("Bryn".into())),
        )
        .unwrap();
        shop.apply(SessionId(952), PlayerAction::Trade(TradeAction::Accept))
            .unwrap();
        shop.apply(
            SessionId(951),
            PlayerAction::Trade(TradeAction::OfferCoins(35)),
        )
        .unwrap();
        let tonic = shop
            .player_state_view(SessionId(951))
            .unwrap()
            .economy
            .shop
            .into_iter()
            .find(|item| item.id == "tonic")
            .unwrap();
        assert!(
            !tonic
                .actions
                .iter()
                .any(|action| action.command == "buy tonic")
        );
        assert!(matches!(
            shop.apply(
                SessionId(951),
                PlayerAction::Economy(EconomyAction::Buy("tonic".into()))
            ),
            Err(WorldError::InsufficientFunds)
        ));
    }

    #[test]
    fn snapshot_v4_requires_valid_binding_and_rejects_policy_drift() {
        let (mut world, actor) = connect_definition(policy_definition());
        world
            .apply(actor, PlayerAction::Take(ItemSelection::one("bag")))
            .unwrap();
        let snapshot = world.runtime_snapshot();
        let encoded = serde_json::to_string(&snapshot).unwrap();
        assert!(encoded.contains("\"snapshotVersion\":9"));
        assert!(encoded.contains("\"binding\":{\"kind\":\"character\",\"characterId\":701}"));
        let mut restored = World::from_definition(policy_definition()).unwrap();
        restored.restore_runtime_snapshot(snapshot.clone()).unwrap();
        assert!(
            restored
                .runtime_item_owner_ids()
                .contains(&CharacterId(701))
        );

        let clasp_id = ItemInstanceId("policy:clasp-instance".into());
        let mut zero = snapshot.clone();
        zero.item_instances.get_mut(&clasp_id).unwrap().binding = ItemBinding::Character {
            character_id: CharacterId(0),
        };
        assert!(matches!(
            World::from_definition(policy_definition())
                .unwrap()
                .restore_runtime_snapshot(zero),
            Err(WorldError::InvalidRuntimeSnapshot)
        ));
        let mut zero_location = snapshot.clone();
        zero_location
            .item_instances
            .get_mut(&ItemInstanceId("policy:bag-instance".into()))
            .unwrap()
            .location = RuntimeItemLocation::Carried {
            character_id: CharacterId(0),
        };
        assert!(matches!(
            World::from_definition(policy_definition())
                .unwrap()
                .restore_runtime_snapshot(zero_location),
            Err(WorldError::InvalidRuntimeSnapshot)
        ));
        let mut unbound = snapshot.clone();
        unbound.item_instances.get_mut(&clasp_id).unwrap().binding = ItemBinding::Unbound;
        assert!(matches!(
            World::from_definition(policy_definition())
                .unwrap()
                .restore_runtime_snapshot(unbound),
            Err(WorldError::InvalidRuntimeSnapshot)
        ));
        let mut escaped = snapshot.clone();
        escaped.item_instances.get_mut(&clasp_id).unwrap().location = RuntimeItemLocation::Room {
            room_id: "test:start".into(),
        };
        assert!(matches!(
            World::from_definition(policy_definition())
                .unwrap()
                .restore_runtime_snapshot(escaped),
            Err(WorldError::InvalidRuntimeSnapshot)
        ));
        let mut forged_transferable = snapshot.clone();
        forged_transferable
            .item_instances
            .get_mut(&ItemInstanceId("policy:bag-instance".into()))
            .unwrap()
            .binding = ItemBinding::Character {
            character_id: CharacterId(701),
        };
        assert!(matches!(
            World::from_definition(policy_definition())
                .unwrap()
                .restore_runtime_snapshot(forged_transferable),
            Err(WorldError::InvalidRuntimeSnapshot)
        ));
        let mut drifted_definition = policy_definition();
        drifted_definition
            .item_prototypes
            .iter_mut()
            .find(|prototype| prototype.prototype_id == "policy:clasp")
            .unwrap()
            .transfer_policy = ItemTransferPolicy::Transferable;
        assert!(matches!(
            World::from_definition(drifted_definition)
                .unwrap()
                .restore_runtime_snapshot(snapshot),
            Err(WorldError::InvalidRuntimeSnapshot)
        ));

        let mut missing_binding: serde_json::Value = serde_json::from_str(&encoded).unwrap();
        missing_binding["itemInstances"]
            .as_object_mut()
            .unwrap()
            .values_mut()
            .next()
            .unwrap()
            .as_object_mut()
            .unwrap()
            .remove("binding");
        assert!(
            decode_world_runtime_snapshot(4, &serde_json::to_string(&missing_binding).unwrap())
                .is_err()
        );
    }

    #[test]
    fn item_graph_index_handles_thousands_of_empty_containers_in_one_pass() {
        let mut container = item_prototype(
            "test:container",
            "empty container",
            "A bounded container.",
            &["container"],
        );
        container.container = Some(ItemContainerDefinition {
            max_slots: 1,
            max_weight_grams: 1,
        });
        let instances = (0..4_096)
            .map(|index| {
                item_instance(
                    &format!("test:container-{index}"),
                    "test:container",
                    InitialItemLocation::Staged,
                )
            })
            .collect();
        let world = World::from_definition(WorldDefinition {
            start_room_id: "test:start".into(),
            item_prototypes: vec![container],
            initial_item_instances: instances,
            rooms: vec![RoomDefinition {
                stable_id: "test:start".into(),
                title: "Start".into(),
                prose: vec!["A test room.".into()],
                image: None,
                audio: None,
                item_ids: Vec::new(),
                exits: Vec::new(),
            }],
        })
        .unwrap();
        let graph = build_item_graph_index(&world.item_instances, &world.item_prototypes).unwrap();
        assert_eq!(graph.subtree_weights.len(), 4_096);
        assert!(graph.children.is_empty());
    }

    #[test]
    fn item_presentation_text_is_bounded_before_it_reaches_transports() {
        let definition_for = |item: ItemPrototypeDefinition| WorldDefinition {
            start_room_id: "test:start".into(),
            item_prototypes: vec![item],
            initial_item_instances: Vec::new(),
            rooms: vec![RoomDefinition {
                stable_id: "test:start".into(),
                title: "Start".into(),
                prose: vec!["A test room.".into()],
                image: None,
                audio: None,
                item_ids: Vec::new(),
                exits: Vec::new(),
            }],
        };
        let mut oversized = item_prototype("test:item", "safe", "Safe.", &["item"]);
        oversized.name = "x".repeat(129);
        assert!(matches!(
            World::from_definition(definition_for(oversized)),
            Err(WorldBuildError::InvalidItemPrototype(_))
        ));

        let mut control = item_prototype("test:item", "safe", "Safe.", &["item"]);
        control.description = "unsafe\u{1b}line".into();
        assert!(matches!(
            World::from_definition(definition_for(control)),
            Err(WorldBuildError::InvalidItemPrototype(_))
        ));

        let mut aliases = item_prototype("test:item", "safe", "Safe.", &["item"]);
        aliases.aliases = (0..33).map(|index| format!("alias-{index}")).collect();
        assert!(matches!(
            World::from_definition(definition_for(aliases)),
            Err(WorldBuildError::InvalidItemPrototype(_))
        ));

        let mut multiline = item_prototype("test:item", "safe", "Safe.", &["item"]);
        multiline.description = "First line.\n\tSecond line.".into();
        assert!(World::from_definition(definition_for(multiline)).is_ok());
    }

    #[test]
    fn offensive_ability_freezes_a_bounded_cohort_without_dropping_the_initiator() {
        let (mut world, initiator_session) = phase_two_world(9, 1, 100);
        world
            .spawn_npc(NpcDefinition {
                stable_id: "game:training-target".into(),
                name: "training target".into(),
                description: "A target sturdy enough to retain an encounter.".into(),
                room_id: "game:start".into(),
                level: 1,
                max_health: 100,
                strength: 1,
                defence: 0,
                experience_reward: 0,
                loot_item_ids: Vec::new(),
                respawn_ticks: 1,
            })
            .unwrap();
        for id in 1..=6 {
            world
                .connect(
                    SessionId(1_000 + id as u64),
                    CharacterSnapshot {
                        id: CharacterId(id),
                        account_id: AccountId(id),
                        name: format!("Peer {id}"),
                        room_id: "game:start".into(),
                        item_locations: BTreeMap::new(),
                    },
                )
                .unwrap();
        }
        world.adventure.next_party_id = 78;
        world.adventure.parties.insert(
            77,
            PartySnapshot {
                id: 77,
                leader: CharacterId(900),
                members: vec![
                    CharacterId(1),
                    CharacterId(2),
                    CharacterId(3),
                    CharacterId(4),
                    CharacterId(5),
                    CharacterId(6),
                    CharacterId(900),
                ],
                invitations: BTreeMap::new(),
            },
        );
        world.abilities.insert(
            "game:jab".into(),
            AbilityDefinition {
                id: "game:jab".into(),
                name: "Jab".into(),
                description: "A controlled strike.".into(),
                category: AbilityCategory::Martial,
                resource_costs: BTreeMap::new(),
                cooldown_ticks: 0,
                targeting: AbilityTargeting::SingleEnemy,
                damage: Some(AbilityDamageDefinition {
                    target: AbilityEffectTarget::Target,
                    damage_type: DamageType::Physical,
                    base_amount: 1,
                    scaling_attribute: None,
                    scaling_percent: 0,
                }),
                heal: None,
                status_effects: Vec::new(),
                communication: None,
            },
        );
        world
            .character_gameplay
            .get_mut(&CharacterId(900))
            .unwrap()
            .ability_ids
            .push("game:jab".into());

        world
            .apply(
                initiator_session,
                PlayerAction::Ability {
                    ability_id: "game:jab".into(),
                    target: Some("training target".into()),
                },
            )
            .unwrap();
        let encounter = &world.loot_encounters["game:training-target"];
        assert_eq!(encounter.initiator, CharacterId(900));
        assert_eq!(encounter.cohort.len(), 6);
        assert!(encounter.cohort.contains(&CharacterId(900)));
        assert_eq!(
            encounter.cohort,
            vec![
                CharacterId(1),
                CharacterId(2),
                CharacterId(3),
                CharacterId(4),
                CharacterId(5),
                CharacterId(900),
            ]
        );
    }

    #[test]
    fn snapshot_v9_round_trips_and_decodes_at_current_version() {
        let (world, _) = connected_world();
        let snapshot = world.runtime_snapshot();
        assert_eq!(snapshot.snapshot_version, WORLD_RUNTIME_SNAPSHOT_VERSION);
        assert_eq!(WORLD_RUNTIME_SNAPSHOT_VERSION, 9);
        let json = serde_json::to_string(&snapshot).unwrap();
        // A direct serde round-trip is lossless.
        let restored: WorldRuntimeSnapshot = serde_json::from_str(&json).unwrap();
        assert_eq!(restored, snapshot);
        // The version-directed decoder accepts the current payload verbatim.
        let decoded = decode_world_runtime_snapshot(WORLD_RUNTIME_SNAPSHOT_VERSION, &json).unwrap();
        assert_eq!(decoded, snapshot);
        // A column/payload version disagreement still fails closed.
        assert!(decode_world_runtime_snapshot(8, &json).is_err());
    }

    #[test]
    fn snapshot_upgrade_v8_normalizes_loot_distribution_and_preserves_state() {
        // The v8 -> v9 loot-encounter conversion defaults to round-robin with no
        // declarations or assignments.
        let converted: LootEncounter = LootEncounterV8 {
            npc_spawn_id: "game:guardian".into(),
            initiator: CharacterId(910),
            party_id: Some(5),
            cohort: vec![CharacterId(910), CharacterId(911)],
            room_id: "game:arena".into(),
            started_tick: 3,
        }
        .into();
        assert_eq!(
            converted.distribution,
            loot_distribution::LootDistributionMode::RoundRobin
        );
        assert!(converted.loot_declarations.is_empty());
        assert!(converted.master_assignments.is_empty());
        assert_eq!(converted.initiator, CharacterId(910));
        assert_eq!(converted.cohort, vec![CharacterId(910), CharacterId(911)]);

        // An exact v8 payload upgrades to v9 with the rest of the state intact.
        let (world, _) = connected_world();
        let snapshot = world.runtime_snapshot();
        let legacy_v8 = runtime_snapshot_json_for_version(&snapshot, 8);
        let upgraded =
            decode_world_runtime_snapshot(8, &serde_json::to_string(&legacy_v8).unwrap()).unwrap();
        assert_eq!(upgraded.snapshot_version, WORLD_RUNTIME_SNAPSHOT_VERSION);
        assert!(upgraded.loot_encounters.values().all(|encounter| {
            encounter.distribution == loot_distribution::LootDistributionMode::RoundRobin
                && encounter.loot_declarations.is_empty()
                && encounter.master_assignments.is_empty()
        }));
        assert_eq!(upgraded.item_instances, snapshot.item_instances);
        assert_eq!(upgraded.item_provenance, snapshot.item_provenance);
        assert_eq!(upgraded.npcs, snapshot.npcs);
        assert_eq!(upgraded.revision, snapshot.revision);
        assert_eq!(upgraded.rng_state, snapshot.rng_state);
        assert_eq!(upgraded.area_residency, snapshot.area_residency);
        assert_eq!(upgraded, snapshot);
    }

    #[test]
    fn snapshot_v8_rejects_smuggled_v9_loot_distribution_field() {
        let (world, _) = connected_world();
        let snapshot = world.runtime_snapshot();
        // A well-formed v8 loot encounter carrying the v9 `distribution` field.
        let smuggled_encounter = serde_json::json!({
            "npcSpawnId": "ghost:target",
            "initiator": 1,
            "cohort": [1],
            "roomId": "ghost:room",
            "startedTick": 0,
            "distribution": "need_greed",
        });
        // Declared as v8, the field is unknown and deserialization fails closed
        // before any semantic validation runs.
        let mut v8 = runtime_snapshot_json_for_version(&snapshot, 8);
        v8["lootEncounters"]["ghost:target"] = smuggled_encounter.clone();
        assert!(decode_world_runtime_snapshot(8, &serde_json::to_string(&v8).unwrap()).is_err());
        // The same field is legitimate under v9, proving the rejection is
        // version-specific rather than a generic parse failure.
        let mut v9 = serde_json::to_value(&snapshot).unwrap();
        v9["lootEncounters"]["ghost:target"] = smuggled_encounter;
        let decoded = decode_world_runtime_snapshot(
            WORLD_RUNTIME_SNAPSHOT_VERSION,
            &serde_json::to_string(&v9).unwrap(),
        )
        .unwrap();
        assert_eq!(
            decoded.loot_encounters["ghost:target"].distribution,
            loot_distribution::LootDistributionMode::NeedGreed
        );
    }

    #[test]
    fn need_greed_governs_reservation_while_round_robin_stays_unchanged() {
        let (mut world, _) = phase_two_world(1, 1, 1);
        for id in [910_i64, 911] {
            world
                .connect(
                    SessionId(id as u64),
                    CharacterSnapshot {
                        id: CharacterId(id),
                        account_id: AccountId(id),
                        name: format!("Cohort {id}"),
                        room_id: "game:arena".into(),
                        item_locations: BTreeMap::new(),
                    },
                )
                .unwrap();
        }
        world.adventure.next_party_id = 6;
        world.adventure.parties.insert(
            5,
            PartySnapshot {
                id: 5,
                leader: CharacterId(910),
                members: vec![CharacterId(910), CharacterId(911)],
                invitations: BTreeMap::new(),
            },
        );

        // Need/greed: member 911 needs, member 910 only greeds. Need must win
        // regardless of the round-robin cursor, and the advanced path must not
        // perturb that cursor.
        let need_root = world.create_staged_item_instance("game:sword").unwrap();
        world
            .set_item_location(&need_root, SharedItemLocation::Room("game:arena".into()))
            .unwrap();
        let mut declarations = BTreeMap::new();
        declarations.insert(
            need_root.clone(),
            BTreeMap::from([
                (CharacterId(910), loot_distribution::LootDecision::Greed),
                (CharacterId(911), loot_distribution::LootDecision::Need),
            ]),
        );
        let need_greed_encounter = LootEncounter {
            npc_spawn_id: "game:guardian".into(),
            initiator: CharacterId(910),
            party_id: Some(5),
            cohort: vec![CharacterId(910), CharacterId(911)],
            room_id: "game:arena".into(),
            started_tick: 0,
            distribution: loot_distribution::LootDistributionMode::NeedGreed,
            loot_declarations: declarations,
            master_assignments: BTreeMap::new(),
        };
        world
            .reserve_ground_loot(
                LootDropId(1),
                std::slice::from_ref(&need_root),
                &need_greed_encounter,
                LootRightsPolicy::PartyRoundRobin,
                "game:arena",
                5,
            )
            .unwrap();
        assert_eq!(
            world.ground_loot[&need_root].beneficiary,
            Some(CharacterId(911))
        );
        assert!(!world.party_loot_cursors.contains_key(&5));

        // A plain round-robin encounter keeps the exact phase 4A behaviour: the
        // first root goes to the smallest cohort identity at cursor 0 and the
        // cursor advances by one.
        let rr_root = world.create_staged_item_instance("game:sword").unwrap();
        world
            .set_item_location(&rr_root, SharedItemLocation::Room("game:arena".into()))
            .unwrap();
        let round_robin_encounter = LootEncounter {
            distribution: loot_distribution::LootDistributionMode::RoundRobin,
            loot_declarations: BTreeMap::new(),
            ..need_greed_encounter.clone()
        };
        world
            .reserve_ground_loot(
                LootDropId(2),
                std::slice::from_ref(&rr_root),
                &round_robin_encounter,
                LootRightsPolicy::PartyRoundRobin,
                "game:arena",
                6,
            )
            .unwrap();
        assert_eq!(
            world.ground_loot[&rr_root].beneficiary,
            Some(CharacterId(910))
        );
        assert_eq!(world.party_loot_cursors.get(&5), Some(&1));
    }

    /// Freeze a two-member party encounter in the arena: the initiator (900,
    /// "Adventurer", party leader) and a partner (901, "Partner"). The guardian
    /// carries `game:fang`, which becomes the single ground-loot root at defeat.
    fn party_arena_world(seed: u64) -> (World, SessionId, SessionId) {
        let (mut world, session) = phase_two_world(seed, 1, 1);
        world
            .configure_loot(
                vec![LootTableDefinition {
                    id: "game:group-table".into(),
                    digest_sha256: "a".repeat(64),
                    guaranteed: Vec::new(),
                    pools: Vec::new(),
                }],
                vec![LootSourceDefinition {
                    kind: LootSourceKind::NpcSpawn,
                    id: "game:group-source".into(),
                    npc_spawn_id: "game:guardian".into(),
                    loot_table_id: "game:group-table".into(),
                    rights: LootRightsPolicy::PartyRoundRobin,
                }],
            )
            .unwrap();
        world
            .apply(session, PlayerAction::Move(Direction::North))
            .unwrap();
        let partner = SessionId(901);
        world
            .connect(
                partner,
                CharacterSnapshot {
                    id: CharacterId(901),
                    account_id: AccountId(91),
                    name: "Partner".into(),
                    room_id: "game:arena".into(),
                    item_locations: BTreeMap::new(),
                },
            )
            .unwrap();
        world.adventure.next_party_id = 6;
        world.adventure.parties.insert(
            5,
            PartySnapshot {
                id: 5,
                leader: CharacterId(900),
                members: vec![CharacterId(900), CharacterId(901)],
                invitations: BTreeMap::new(),
            },
        );
        world
            .apply(session, PlayerAction::Attack("guardian".into()))
            .unwrap();
        // The cohort must include both party members before any decision.
        assert_eq!(
            world.loot_encounters["game:guardian"].cohort,
            vec![CharacterId(900), CharacterId(901)]
        );
        (world, session, partner)
    }

    fn drive_to_loot_resolution(world: &mut World) {
        let fang = ItemInstanceId("game:fang".into());
        for _ in 0..16 {
            world.advance_tick().unwrap();
            if world.ground_loot.contains_key(&fang) {
                return;
            }
        }
        panic!("guardian loot was never resolved");
    }

    #[test]
    fn parses_group_loot_distribution_commands() {
        use loot_distribution::{LootDecision, LootDistributionMode};
        assert_eq!(
            parse_action("loot mode need_greed"),
            Ok(PlayerAction::Loot(LootAction::Mode(
                LootDistributionMode::NeedGreed
            )))
        );
        assert_eq!(
            parse_action("loot mode master"),
            Ok(PlayerAction::Loot(LootAction::Mode(
                LootDistributionMode::MasterLoot
            )))
        );
        assert_eq!(
            parse_action("loot mode rr"),
            Ok(PlayerAction::Loot(LootAction::Mode(
                LootDistributionMode::RoundRobin
            )))
        );
        assert_eq!(
            parse_action("loot roll"),
            Ok(PlayerAction::Loot(LootAction::Roll))
        );
        assert_eq!(
            parse_action("need silver fang"),
            Ok(PlayerAction::Loot(LootAction::Declare {
                decision: LootDecision::Need,
                query: "silver fang".into()
            }))
        );
        assert_eq!(
            parse_action("greed fang"),
            Ok(PlayerAction::Loot(LootAction::Declare {
                decision: LootDecision::Greed,
                query: "fang".into()
            }))
        );
        assert_eq!(
            parse_action("pass fang"),
            Ok(PlayerAction::Loot(LootAction::Declare {
                decision: LootDecision::Pass,
                query: "fang".into()
            }))
        );
        assert_eq!(
            parse_action("loot master fang to Partner"),
            Ok(PlayerAction::Loot(LootAction::Master {
                query: "fang".into(),
                member: "Partner".into()
            }))
        );
        // The pre-existing reservation `loot pass` still routes to `Pass`, never
        // to a need/greed declaration.
        assert_eq!(
            parse_action("loot pass fang"),
            Ok(PlayerAction::Loot(LootAction::Pass("fang".into())))
        );
        assert!(matches!(
            parse_action("loot mode bogus"),
            Err(ParseActionError::Unknown(_))
        ));
        assert!(matches!(
            parse_action("loot mode"),
            Err(ParseActionError::MissingTarget { command: "loot" })
        ));
        assert!(matches!(
            parse_action("need"),
            Err(ParseActionError::MissingTarget { command: "loot" })
        ));
    }

    #[test]
    fn need_greed_flow_resolves_the_needer_and_projects_state() {
        let (mut world, s900, s901) = party_arena_world(7);
        world
            .apply(
                s900,
                PlayerAction::Loot(LootAction::Mode(
                    loot_distribution::LootDistributionMode::NeedGreed,
                )),
            )
            .unwrap();
        // The cohort-private projection shows the mode and the carried root, with
        // opaque refs only (never the internal instance id).
        let roll = world
            .apply(s901, PlayerAction::Loot(LootAction::Roll))
            .unwrap();
        let projection = roll.output.unwrap();
        assert!(projection.contains("need/greed"));
        assert!(projection.contains("silver fang"));
        assert!(!projection.contains("game:fang"));
        // 901 needs, 900 only greeds: the needer must win regardless of seed.
        world
            .apply(
                s901,
                PlayerAction::Loot(LootAction::Declare {
                    decision: loot_distribution::LootDecision::Need,
                    query: "fang".into(),
                }),
            )
            .unwrap();
        world
            .apply(
                s900,
                PlayerAction::Loot(LootAction::Declare {
                    decision: loot_distribution::LootDecision::Greed,
                    query: "fang".into(),
                }),
            )
            .unwrap();
        drive_to_loot_resolution(&mut world);
        let fang = ItemInstanceId("game:fang".into());
        assert_eq!(world.ground_loot[&fang].beneficiary, Some(CharacterId(901)));
    }

    #[test]
    fn need_greed_declarations_survive_a_snapshot_round_trip() {
        let (mut world, s900, s901) = party_arena_world(11);
        world
            .apply(
                s900,
                PlayerAction::Loot(LootAction::Mode(
                    loot_distribution::LootDistributionMode::NeedGreed,
                )),
            )
            .unwrap();
        world
            .apply(
                s901,
                PlayerAction::Loot(LootAction::Declare {
                    decision: loot_distribution::LootDecision::Need,
                    query: "fang".into(),
                }),
            )
            .unwrap();
        // A restart mid-window preserves the durable declaration in snapshot v9,
        // and resolution stays exactly-once: 901 still wins after the round trip.
        let snapshot = world.runtime_snapshot();
        let encoded = serde_json::to_string(&snapshot).unwrap();
        let decoded =
            decode_world_runtime_snapshot(WORLD_RUNTIME_SNAPSHOT_VERSION, &encoded).unwrap();
        let fang = ItemInstanceId("game:fang".into());
        assert_eq!(
            decoded.loot_encounters["game:guardian"].loot_declarations[&fang][&CharacterId(901)],
            loot_distribution::LootDecision::Need
        );
        drive_to_loot_resolution(&mut world);
        assert_eq!(world.ground_loot[&fang].beneficiary, Some(CharacterId(901)));
    }

    #[test]
    fn master_loot_flow_assigns_to_the_chosen_member() {
        let (mut world, s900, _s901) = party_arena_world(3);
        world
            .apply(
                s900,
                PlayerAction::Loot(LootAction::Mode(
                    loot_distribution::LootDistributionMode::MasterLoot,
                )),
            )
            .unwrap();
        world
            .apply(
                s900,
                PlayerAction::Loot(LootAction::Master {
                    query: "fang".into(),
                    member: "Partner".into(),
                }),
            )
            .unwrap();
        drive_to_loot_resolution(&mut world);
        let fang = ItemInstanceId("game:fang".into());
        assert_eq!(world.ground_loot[&fang].beneficiary, Some(CharacterId(901)));
    }

    #[test]
    fn group_loot_rejects_non_cohort_double_declaration_and_closed_window() {
        let (mut world, s900, s901) = party_arena_world(9);
        // Declaring before the leader opts into need/greed is refused.
        assert!(
            world
                .apply(
                    s901,
                    PlayerAction::Loot(LootAction::Declare {
                        decision: loot_distribution::LootDecision::Need,
                        query: "fang".into(),
                    })
                )
                .is_err()
        );
        world
            .apply(
                s900,
                PlayerAction::Loot(LootAction::Mode(
                    loot_distribution::LootDistributionMode::NeedGreed,
                )),
            )
            .unwrap();
        // An outsider present in the room but outside the frozen cohort cannot
        // declare.
        let outsider = SessionId(902);
        world
            .connect(
                outsider,
                CharacterSnapshot {
                    id: CharacterId(902),
                    account_id: AccountId(92),
                    name: "Outsider".into(),
                    room_id: "game:arena".into(),
                    item_locations: BTreeMap::new(),
                },
            )
            .unwrap();
        assert!(
            world
                .apply(
                    outsider,
                    PlayerAction::Loot(LootAction::Declare {
                        decision: loot_distribution::LootDecision::Need,
                        query: "fang".into(),
                    })
                )
                .is_err()
        );
        // Exactly one declaration per (root, member): a second is refused.
        world
            .apply(
                s901,
                PlayerAction::Loot(LootAction::Declare {
                    decision: loot_distribution::LootDecision::Need,
                    query: "fang".into(),
                }),
            )
            .unwrap();
        assert!(
            world
                .apply(
                    s901,
                    PlayerAction::Loot(LootAction::Declare {
                        decision: loot_distribution::LootDecision::Greed,
                        query: "fang".into(),
                    })
                )
                .is_err()
        );
        // Master assignment to a non-cohort member is refused.
        world
            .apply(
                s900,
                PlayerAction::Loot(LootAction::Mode(
                    loot_distribution::LootDistributionMode::MasterLoot,
                )),
            )
            .unwrap();
        assert!(
            world
                .apply(
                    s900,
                    PlayerAction::Loot(LootAction::Master {
                        query: "fang".into(),
                        member: "Outsider".into(),
                    })
                )
                .is_err()
        );
        // Once the encounter resolves at defeat, the decision window is closed.
        drive_to_loot_resolution(&mut world);
        assert!(
            world
                .apply(
                    s900,
                    PlayerAction::Loot(LootAction::Mode(
                        loot_distribution::LootDistributionMode::NeedGreed,
                    ))
                )
                .is_err()
        );
    }

    #[test]
    fn group_loot_cannot_override_credited_rights_or_leadership() {
        use loot_distribution::LootDistributionMode;
        let (mut world, leader, _) = party_arena_world(4);
        world.loot_sources.get_mut("game:guardian").unwrap().rights =
            LootRightsPolicy::CreditedCharacter;
        let before = world.runtime_snapshot();
        for mode in [
            LootDistributionMode::NeedGreed,
            LootDistributionMode::MasterLoot,
        ] {
            assert!(
                world
                    .apply(leader, PlayerAction::Loot(LootAction::Mode(mode)))
                    .is_err()
            );
            assert_eq!(world.runtime_snapshot(), before);
        }
        // Even an already durable legacy advanced decision cannot override
        // authored credited-character rights when the NPC is defeated.
        let encounter = world.loot_encounters.get_mut("game:guardian").unwrap();
        encounter.distribution = LootDistributionMode::MasterLoot;
        encounter
            .master_assignments
            .insert(ItemInstanceId("game:fang".into()), CharacterId(901));
        drive_to_loot_resolution(&mut world);
        assert_eq!(
            world.ground_loot[&ItemInstanceId("game:fang".into())].beneficiary,
            Some(CharacterId(900))
        );

        let (mut world, leader, partner) = party_arena_world(5);
        world.adventure.parties.get_mut(&5).unwrap().leader = CharacterId(901);
        // Neither an initiating non-leader nor a later leader can take over the
        // frozen initiating leader's authority.
        for session in [leader, partner] {
            assert!(
                world
                    .apply(
                        session,
                        PlayerAction::Loot(LootAction::Mode(LootDistributionMode::MasterLoot))
                    )
                    .is_err()
            );
        }
    }

    #[test]
    fn round_robin_default_group_loot_is_unchanged() {
        // With no mode selected and no declarations, the fixed root follows the
        // unchanged phase 4A path: it is reserved for the initiator.
        let (mut world, _s900, _s901) = party_arena_world(4);
        drive_to_loot_resolution(&mut world);
        let fang = ItemInstanceId("game:fang".into());
        assert_eq!(
            world.loot_encounters.get("game:guardian"),
            None,
            "the encounter is consumed at defeat"
        );
        assert_eq!(world.ground_loot[&fang].beneficiary, Some(CharacterId(900)));
    }

    #[test]
    fn loot_definition_caps_include_fixed_roots_and_source_ids() {
        let (mut world, _) = phase_two_world(1, 1, 10);
        let table = LootTableDefinition {
            id: "game:oversized".into(),
            digest_sha256: "a".repeat(64),
            guaranteed: vec![
                LootGuaranteedDefinition {
                    id: "game:first".into(),
                    item_prototype_id: "game:potion".into(),
                    quantity: 32,
                    affix_pool_id: None,
                },
                LootGuaranteedDefinition {
                    id: "game:second".into(),
                    item_prototype_id: "game:potion".into(),
                    quantity: 32,
                    affix_pool_id: None,
                },
            ],
            pools: Vec::new(),
        };
        assert!(matches!(
            world.configure_loot(
                vec![table],
                vec![LootSourceDefinition {
                    kind: LootSourceKind::NpcSpawn,
                    id: "game:guardian-source".into(),
                    npc_spawn_id: "game:guardian".into(),
                    loot_table_id: "game:oversized".into(),
                    rights: LootRightsPolicy::CreditedCharacter,
                }],
            ),
            Err(WorldError::InvalidGameplayDefinition)
        ));

        let mut roots = Vec::new();
        for _ in 0..=MAX_LOOT_UNITS_PER_RESOLUTION {
            roots.push(world.create_staged_item_instance("game:sword").unwrap());
        }
        assert!(matches!(
            world.spawn_npc(NpcDefinition {
                stable_id: "game:too-much-fixed-loot".into(),
                name: "overburdened target".into(),
                description: "An invalid fixed-loot source.".into(),
                room_id: "game:start".into(),
                level: 1,
                max_health: 1,
                strength: 1,
                defence: 0,
                experience_reward: 0,
                loot_item_ids: roots,
                respawn_ticks: 1,
            }),
            Err(WorldError::InvalidGameplayDefinition)
        ));
    }

    #[test]
    fn fixed_loot_wins_the_last_claim_and_random_capacity_never_blocks_defeat() {
        let (mut world, _) = phase_two_world(3, 1, 1);
        world
            .configure_loot(
                vec![LootTableDefinition {
                    id: "game:guardian-table".into(),
                    digest_sha256: "b".repeat(64),
                    guaranteed: vec![LootGuaranteedDefinition {
                        id: "game:potion-drop".into(),
                        item_prototype_id: "game:potion".into(),
                        quantity: 1,
                        affix_pool_id: None,
                    }],
                    pools: Vec::new(),
                }],
                vec![LootSourceDefinition {
                    kind: LootSourceKind::NpcSpawn,
                    id: "game:guardian-source".into(),
                    npc_spawn_id: "game:guardian".into(),
                    loot_table_id: "game:guardian-table".into(),
                    rights: LootRightsPolicy::CreditedCharacter,
                }],
            )
            .unwrap();
        world.next_loot_drop_id = LootDropId(2);
        for _ in 0..MAX_GROUND_LOOT_RESERVATIONS - 1 {
            let item_id = world.create_staged_item_instance("game:sword").unwrap();
            world
                .set_item_location(&item_id, SharedItemLocation::Room("game:arena".into()))
                .unwrap();
            world.ground_loot.insert(
                item_id.clone(),
                GroundLootReservation {
                    drop_id: LootDropId(1),
                    root_item_id: item_id,
                    room_id: "game:arena".into(),
                    cohort: vec![CharacterId(900)],
                    beneficiary: Some(CharacterId(900)),
                    passed: BTreeSet::new(),
                    created_tick: 0,
                    claim_until_tick: LOOT_EXCLUSIVE_TICKS,
                    despawn_at_tick: LOOT_DESPAWN_TICKS,
                },
            );
        }
        assert!(
            serde_json::to_vec(&world.runtime_snapshot()).unwrap().len()
                <= MAX_WORLD_RUNTIME_SNAPSHOT_BYTES
        );
        let mut events = Vec::new();
        world
            .resolve_npc_defeat(
                0,
                "game:guardian",
                CharacterId(900),
                "Adventurer",
                "game:arena",
                &mut events,
            )
            .unwrap();
        let fang = ItemInstanceId("game:fang".into());
        assert_eq!(
            world.item_location(&fang),
            Some(&SharedItemLocation::Room("game:arena".into()))
        );
        assert!(world.ground_loot.contains_key(&fang));
        assert_eq!(world.ground_loot.len(), MAX_GROUND_LOOT_RESERVATIONS);
        assert_eq!(world.next_loot_drop_id, LootDropId(3));
        assert!(events.iter().any(|event| matches!(
            event,
            DomainEvent::Gameplay {
                event: GameplayEvent::NpcDefeated { .. }
            }
        )));
        assert!(events.iter().any(|event| matches!(
            event,
            DomainEvent::Gameplay {
                event: GameplayEvent::LootGenerationSuppressed { reason, .. }
            } if reason == "capacity"
        )));
        assert_eq!(world.character_gameplay[&CharacterId(900)].experience, 120);
    }

    #[test]
    fn random_loot_is_suppressed_atomically_at_the_exact_snapshot_byte_budget() {
        let (mut world, _) = phase_two_world(11, 1, 10);
        world
            .spawn_npc(NpcDefinition {
                stable_id: "game:byte-target".into(),
                name: "byte target".into(),
                description: "A target for durable admission testing.".into(),
                room_id: "game:start".into(),
                level: 1,
                max_health: 1,
                strength: 1,
                defence: 0,
                experience_reward: 7,
                loot_item_ids: Vec::new(),
                respawn_ticks: 2,
            })
            .unwrap();
        world
            .configure_loot(
                vec![LootTableDefinition {
                    id: "game:byte-table".into(),
                    digest_sha256: "c".repeat(64),
                    guaranteed: Vec::new(),
                    pools: vec![LootPoolDefinition {
                        id: "game:byte-pool".into(),
                        rolls: 1,
                        entries: vec![LootEntryDefinition {
                            kind: LootEntryKind::Item,
                            id: "game:byte-entry".into(),
                            weight: 1,
                            item_prototype_id: Some("game:potion".into()),
                            min_quantity: Some(32),
                            max_quantity: Some(32),
                            affix_pool_id: None,
                        }],
                    }],
                }],
                vec![LootSourceDefinition {
                    kind: LootSourceKind::NpcSpawn,
                    id: "game:byte-source".into(),
                    npc_spawn_id: "game:byte-target".into(),
                    loot_table_id: "game:byte-table".into(),
                    rights: LootRightsPolicy::CreditedCharacter,
                }],
            )
            .unwrap();
        let mut encoded_len = serde_json::to_vec(&world.runtime_snapshot()).unwrap().len();
        while encoded_len < MAX_WORLD_RUNTIME_SNAPSHOT_BYTES - 40_000 {
            for _ in 0..100 {
                world.create_staged_item_instance("game:sword").unwrap();
            }
            encoded_len = serde_json::to_vec(&world.runtime_snapshot()).unwrap().len();
        }
        while encoded_len < MAX_WORLD_RUNTIME_SNAPSHOT_BYTES - 8_000 {
            world.create_staged_item_instance("game:sword").unwrap();
            encoded_len = serde_json::to_vec(&world.runtime_snapshot()).unwrap().len();
        }
        assert!(encoded_len <= MAX_WORLD_RUNTIME_SNAPSHOT_BYTES);
        let before_rng = world.rng.state;
        let mut events = Vec::new();
        world
            .resolve_npc_defeat(
                0,
                "game:byte-target",
                CharacterId(900),
                "Adventurer",
                "game:start",
                &mut events,
            )
            .unwrap();

        assert_eq!(world.next_loot_drop_id, LootDropId(2));
        assert_ne!(world.rng.state, before_rng);
        assert!(!world.item_provenance.values().any(|provenance| matches!(
            provenance,
            ItemProvenance::NpcLoot {
                npc_spawn_id, ..
            } if npc_spawn_id == "game:byte-target"
        )));
        assert!(events.iter().any(|event| matches!(
            event,
            DomainEvent::Gameplay {
                event: GameplayEvent::LootGenerationSuppressed { reason, .. }
            } if reason == "snapshot_bytes"
        )));
        assert!(
            serde_json::to_vec(&world.runtime_snapshot()).unwrap().len()
                <= MAX_WORLD_RUNTIME_SNAPSHOT_BYTES
        );
        assert_eq!(world.character_gameplay[&CharacterId(900)].experience, 7);
    }

    #[test]
    fn near_limit_v5_snapshot_upgrades_losslessly_below_the_v6_byte_budget() {
        let (mut world, _) = phase_two_world(13, 1, 10);
        let mut legacy = runtime_snapshot_json_for_version(&world.runtime_snapshot(), 5);
        let mut encoded_len = serde_json::to_vec(&legacy).unwrap().len();
        while encoded_len < MAX_LEGACY_WORLD_RUNTIME_SNAPSHOT_BYTES - 40_000 {
            for _ in 0..100 {
                world.create_staged_item_instance("game:sword").unwrap();
            }
            legacy = runtime_snapshot_json_for_version(&world.runtime_snapshot(), 5);
            encoded_len = serde_json::to_vec(&legacy).unwrap().len();
        }
        while encoded_len < MAX_LEGACY_WORLD_RUNTIME_SNAPSHOT_BYTES - 8_000 {
            world.create_staged_item_instance("game:sword").unwrap();
            legacy = runtime_snapshot_json_for_version(&world.runtime_snapshot(), 5);
            encoded_len = serde_json::to_vec(&legacy).unwrap().len();
        }
        assert!(encoded_len <= MAX_LEGACY_WORLD_RUNTIME_SNAPSHOT_BYTES);
        let legacy_json = serde_json::to_string(&legacy).unwrap();
        let upgraded = decode_world_runtime_snapshot(5, &legacy_json).unwrap();
        assert_eq!(upgraded.snapshot_version, WORLD_RUNTIME_SNAPSHOT_VERSION);
        assert_eq!(
            upgraded.item_provenance.len(),
            upgraded.item_instances.len()
        );
        assert!(upgraded.item_provenance.values().all(|provenance| matches!(
            provenance,
            ItemProvenance::LegacySnapshot { source_version: 5 }
        )));
        assert!(serde_json::to_vec(&upgraded).unwrap().len() <= MAX_WORLD_RUNTIME_SNAPSHOT_BYTES);
    }

    #[test]
    fn loot_views_and_pass_keep_reserved_ground_items_enforced_server_side() {
        let (mut world, actor) = connected_world();
        let other = SessionId(8);
        world
            .connect(
                other,
                CharacterSnapshot {
                    id: CharacterId(102),
                    account_id: AccountId(11),
                    name: "Other".into(),
                    room_id: world.start_room_id().to_owned(),
                    item_locations: BTreeMap::new(),
                },
            )
            .unwrap();
        let item_id = ItemInstanceId("demo:blue-lantern".into());
        world.next_loot_drop_id = LootDropId(2);
        world.ground_loot.insert(
            item_id.clone(),
            GroundLootReservation {
                drop_id: LootDropId(1),
                root_item_id: item_id,
                room_id: world.start_room_id().to_owned(),
                cohort: vec![CharacterId(101), CharacterId(102)],
                beneficiary: Some(CharacterId(101)),
                passed: BTreeSet::new(),
                created_tick: 0,
                claim_until_tick: LOOT_EXCLUSIVE_TICKS,
                despawn_at_tick: LOOT_DESPAWN_TICKS,
            },
        );
        let own = world.scene_for(actor).unwrap().items.remove(0);
        assert_eq!(
            own.loot.as_ref().map(|loot| loot.access),
            Some(LootAccessView::Yours)
        );
        assert!(own.actions.iter().any(|action| action.label == "Pass"));
        let reserved = world.scene_for(other).unwrap().items.remove(0);
        assert_eq!(
            reserved.loot.as_ref().map(|loot| loot.access),
            Some(LootAccessView::Reserved)
        );
        assert!(!reserved.actions.iter().any(|action| action.label == "Take"));
        assert!(matches!(
            world.apply(other, PlayerAction::Take(ItemSelection::one("lantern"))),
            Err(WorldError::LootUnavailable)
        ));

        world
            .apply(actor, PlayerAction::Loot(LootAction::Pass(own.id)))
            .unwrap();
        assert_eq!(
            world.ground_loot[&ItemInstanceId("demo:blue-lantern".into())].beneficiary,
            Some(CharacterId(102))
        );
        world
            .apply(other, PlayerAction::Take(ItemSelection::one("lantern")))
            .unwrap();
        assert!(world.ground_loot.is_empty());
    }

    #[test]
    fn reserved_ground_container_contents_are_private_to_the_beneficiary() {
        let (mut world, beneficiary) = connect_definition(policy_definition());
        world
            .item_prototypes
            .get_mut("policy:clasp")
            .unwrap()
            .container = Some(ItemContainerDefinition {
            max_slots: 2,
            max_weight_grams: 100,
        });
        let other = SessionId(72);
        world
            .connect(
                other,
                CharacterSnapshot {
                    id: CharacterId(702),
                    account_id: AccountId(71),
                    name: "Other".into(),
                    room_id: "test:start".into(),
                    item_locations: BTreeMap::new(),
                },
            )
            .unwrap();
        let bag_id = ItemInstanceId("policy:bag-instance".into());
        world.next_loot_drop_id = LootDropId(2);
        world.ground_loot.insert(
            bag_id.clone(),
            GroundLootReservation {
                drop_id: LootDropId(1),
                root_item_id: bag_id,
                room_id: "test:start".into(),
                cohort: vec![CharacterId(701), CharacterId(702)],
                beneficiary: Some(CharacterId(701)),
                passed: BTreeSet::new(),
                created_tick: 0,
                claim_until_tick: LOOT_EXCLUSIVE_TICKS,
                despawn_at_tick: LOOT_DESPAWN_TICKS,
            },
        );

        let own = world
            .apply(
                beneficiary,
                PlayerAction::Contents {
                    container: "bag".into(),
                    page: 1,
                },
            )
            .unwrap();
        assert_eq!(
            own.container_contents.as_ref().unwrap().items[0].label,
            "wolf clasp"
        );
        assert!(
            world
                .apply(beneficiary, PlayerAction::Examine("clasp".into()))
                .unwrap()
                .output
                .unwrap()
                .contains("character-bound clasp")
        );
        assert!(
            world
                .apply(
                    beneficiary,
                    PlayerAction::Contents {
                        container: "clasp".into(),
                        page: 1,
                    },
                )
                .is_ok()
        );
        assert!(matches!(
            world.apply(
                other,
                PlayerAction::Contents {
                    container: "bag".into(),
                    page: 1,
                },
            ),
            Err(WorldError::LootUnavailable)
        ));
        assert!(matches!(
            world.apply(
                other,
                PlayerAction::Contents {
                    container: "clasp".into(),
                    page: 1,
                },
            ),
            Err(WorldError::ItemNotFound(_))
        ));
        assert!(matches!(
            world.apply(
                other,
                PlayerAction::Contents {
                    container: "not-there".into(),
                    page: 1,
                },
            ),
            Err(WorldError::ItemNotFound(_))
        ));
        assert!(matches!(
            world.apply(
                other,
                PlayerAction::Get {
                    selection: ItemSelection::one("clasp"),
                    container: "bag".into(),
                },
            ),
            Err(WorldError::LootUnavailable)
        ));
        assert!(matches!(
            world.apply(
                other,
                PlayerAction::Get {
                    selection: ItemSelection::one("not-there"),
                    container: "bag".into(),
                },
            ),
            Err(WorldError::LootUnavailable)
        ));
        assert!(matches!(
            world.apply(
                other,
                PlayerAction::Put {
                    selection: ItemSelection::one("not-there"),
                    container: "bag".into(),
                },
            ),
            Err(WorldError::LootUnavailable)
        ));
        assert!(matches!(
            world.apply(other, PlayerAction::Examine("clasp".into())),
            Err(WorldError::ItemNotFound(_))
        ));
    }

    #[test]
    fn despawning_runtime_root_preserves_authored_descendants_as_tombstones() {
        let (mut world, _) = connect_definition(policy_definition());
        let root = world.create_staged_item_instance("policy:bag").unwrap();
        world
            .set_item_location(&root, SharedItemLocation::Room("test:start".into()))
            .unwrap();
        let authored = ItemInstanceId("policy:clasp-instance".into());
        world.item_instances.get_mut(&authored).unwrap().location =
            SharedItemLocation::Contained(root.clone());
        world.next_loot_drop_id = LootDropId(2);
        world.ground_loot.insert(
            root.clone(),
            GroundLootReservation {
                drop_id: LootDropId(1),
                root_item_id: root.clone(),
                room_id: "test:start".into(),
                cohort: vec![CharacterId(701)],
                beneficiary: Some(CharacterId(701)),
                passed: BTreeSet::new(),
                created_tick: 0,
                claim_until_tick: LOOT_EXCLUSIVE_TICKS,
                despawn_at_tick: LOOT_DESPAWN_TICKS,
            },
        );
        world.logical_tick = LOOT_DESPAWN_TICKS;

        world.expire_ground_loot().unwrap();
        assert!(!world.item_instances.contains_key(&root));
        assert_eq!(
            world.item_location(&authored),
            Some(&SharedItemLocation::Staged)
        );
        assert_eq!(
            world.item_provenance.get(&authored),
            Some(&ItemProvenance::AuthoredInitial)
        );
    }

    #[test]
    fn legacy_fixed_loot_despawn_stays_staged_after_the_next_restart() {
        let (source, _) = phase_two_world(17, 1, 10);
        let legacy = runtime_snapshot_json_for_version(&source.runtime_snapshot(), 5);
        let upgraded = decode_world_runtime_snapshot(5, &legacy.to_string()).unwrap();
        let fang = ItemInstanceId("game:fang".into());
        assert_eq!(
            upgraded.item_provenance.get(&fang),
            Some(&ItemProvenance::LegacySnapshot { source_version: 5 })
        );

        let (mut restored, _) = phase_two_world(17, 1, 10);
        restored.restore_runtime_snapshot(upgraded).unwrap();
        restored
            .set_item_location(&fang, SharedItemLocation::Room("game:start".into()))
            .unwrap();
        restored.next_loot_drop_id = LootDropId(2);
        restored.ground_loot.insert(
            fang.clone(),
            GroundLootReservation {
                drop_id: LootDropId(1),
                root_item_id: fang.clone(),
                room_id: "game:start".into(),
                cohort: vec![CharacterId(900)],
                beneficiary: Some(CharacterId(900)),
                passed: BTreeSet::new(),
                created_tick: 0,
                claim_until_tick: LOOT_EXCLUSIVE_TICKS,
                despawn_at_tick: LOOT_DESPAWN_TICKS,
            },
        );
        restored.logical_tick = LOOT_DESPAWN_TICKS;
        restored.expire_ground_loot().unwrap();
        assert_eq!(
            restored.item_location(&fang),
            Some(&SharedItemLocation::Staged)
        );
        assert!(restored.ground_loot.is_empty());

        let post_despawn = restored.runtime_snapshot();
        let (mut restarted, _) = phase_two_world(17, 1, 10);
        restarted.restore_runtime_snapshot(post_despawn).unwrap();
        assert_eq!(
            restarted.item_location(&fang),
            Some(&SharedItemLocation::Staged),
            "restart must not resurrect fixed loot at its authored NPC"
        );
        assert_eq!(
            restarted.item_provenance.get(&fang),
            Some(&ItemProvenance::LegacySnapshot { source_version: 5 })
        );
    }

    #[test]
    fn restart_cancellation_clears_lethal_npc_status_before_the_next_tick() {
        let (mut source, session) = phase_two_world(5, 1, 10);
        source
            .apply(session, PlayerAction::Move(Direction::North))
            .unwrap();
        source
            .apply(session, PlayerAction::Attack("guardian".into()))
            .unwrap();
        source.npcs.get_mut("game:guardian").unwrap().statuses = vec![ActiveStatusEffect {
            source_ability_id: "game:old-dot".into(),
            source_character_id: Some(CharacterId(900)),
            source_npc_id: None,
            kind: ActiveStatusKind::DamageOverTime,
            magnitude: 100,
            remaining_ticks: 1,
            attribute: None,
            damage_type: Some(DamageType::Physical),
        }];
        let snapshot = source.runtime_snapshot();
        let (mut restored, _) = phase_two_world(5, 1, 10);
        restored.restore_runtime_snapshot(snapshot).unwrap();

        let cancelled = restored.cancel_all_loot_encounters_for_restart().unwrap();
        assert_eq!(cancelled.len(), 1);
        assert!(restored.loot_encounters.is_empty());
        assert!(restored.npcs["game:guardian"].statuses.is_empty());
        let events = restored.advance_tick().unwrap();
        assert!(restored.npcs["game:guardian"].alive());
        assert!(!events.iter().any(|event| matches!(
            event,
            DomainEvent::Gameplay {
                event: GameplayEvent::NpcDefeated { .. }
            }
        )));
    }

    #[test]
    fn sealed_reward_open_and_odds_are_atomic_opaque_and_exact() {
        let (mut world, session, source) = sealed_reward_world(41);
        let before = world.runtime_snapshot();
        let inventory = world.player_state_view(session).unwrap().inventory;
        let seal = inventory
            .iter()
            .find(|item| item.label == "sealed cache")
            .unwrap();
        assert!(seal.actions.iter().any(|action| {
            action.label == "Abrir" && action.command == format!("open {}", seal.id)
        }));
        assert!(seal.actions.iter().any(|action| {
            action.label == "Probabilidades"
                && action.command == format!("reward odds {} 1", seal.id)
        }));

        let odds_outcome = world
            .apply(
                session,
                PlayerAction::RewardOdds {
                    target: seal.id.clone(),
                    page: 1,
                },
            )
            .unwrap();
        let odds_text = odds_outcome.output.unwrap();
        assert!(odds_text.contains("garantizado; afijo aleatorio"));
        assert!(odds_text.contains(
            "Tirada 1: 2 tirada(s); sin recompensa; cantidad 0..=0; probabilidad 2/2 por tirada"
        ));
        assert!(
            odds_text
                .contains("posición prefix; ataque +2; curación +0; probabilidad 3/3 condicional")
        );
        let odds = odds_outcome.reward_odds.unwrap();
        assert_eq!(odds.guaranteed.len(), 1);
        assert_eq!(odds.weighted.len(), 1);
        assert_eq!(odds.affixes.len(), 1);
        assert_eq!(odds.affixes[0].conditional_probability_numerator, 3);
        assert_eq!(odds.affixes[0].conditional_probability_denominator, 3);
        let encoded_odds = serde_json::to_string(&odds).unwrap();
        assert!(!encoded_odds.contains("reward:weapon-affix"));
        assert!(!encoded_odds.contains("reward:keen"));

        let opened = world
            .apply(session, PlayerAction::OpenReward(seal.id.clone()))
            .unwrap();
        assert!(matches!(
            opened.domain_events.as_slice(),
            [DomainEvent::Gameplay {
                event: GameplayEvent::RewardOpened {
                    open_id: RewardOpenId(1),
                    ..
                }
            }]
        ));
        assert_eq!(
            world.item_instances[&source].location,
            SharedItemLocation::Consumed
        );
        let reward_id = world
            .item_provenance
            .iter()
            .find_map(|(item_id, provenance)| {
                matches!(provenance, ItemProvenance::SealedReward { .. }).then_some(item_id.clone())
            })
            .unwrap();
        assert_eq!(
            world.item_effective_name(&reward_id).unwrap(),
            "afilada iron sword"
        );
        assert_eq!(world.item_effective_attack_bonus(&reward_id).unwrap(), 3);
        assert_eq!(world.item_instances[&reward_id].item_affixes.len(), 1);
        let after = world.runtime_snapshot();
        assert!(matches!(
            world.apply(session, PlayerAction::OpenReward(seal.id.clone())),
            Err(WorldError::ItemNotFound(_))
        ));
        assert_eq!(world.runtime_snapshot(), after);

        world.restore_checkpoint({
            let mut replay = sealed_reward_world(41).0;
            let checkpoint = replay.checkpoint();
            let replay_session = SessionId(71);
            replay
                .apply(
                    replay_session,
                    PlayerAction::OpenReward("sealed cache".into()),
                )
                .unwrap();
            checkpoint
        });
        assert_eq!(world.runtime_snapshot(), before);
    }

    #[test]
    fn sealed_reward_capacity_failure_cannot_reroll_or_consume() {
        let (mut world, session, source) = sealed_reward_world(73);
        world
            .character_gameplay
            .get_mut(&CharacterId(701))
            .unwrap()
            .inventory_capacity
            .max_slots = 1;
        let before = world.runtime_snapshot();
        assert!(matches!(
            world.apply(session, PlayerAction::OpenReward("sealed cache".into())),
            Err(WorldError::InventoryCapacityExceeded)
        ));
        assert_eq!(world.runtime_snapshot(), before);
        assert_eq!(
            world.item_instances[&source].location,
            SharedItemLocation::Carried(CharacterId(701))
        );
        assert_eq!(world.next_reward_open_id, RewardOpenId(1));
    }

    #[test]
    fn sealed_reward_survives_source_pruning_and_affix_authoring_changes() {
        let (mut world, session, source) = sealed_reward_world(91);
        world
            .apply(session, PlayerAction::OpenReward("sealed cache".into()))
            .unwrap();
        let reward_id = world
            .item_provenance
            .iter()
            .find_map(|(item_id, provenance)| {
                matches!(provenance, ItemProvenance::SealedReward { .. }).then_some(item_id.clone())
            })
            .unwrap();
        let source_digest = match &world.item_provenance[&reward_id] {
            ItemProvenance::SealedReward {
                source_npc_loot_digest_sha256,
                ..
            } => source_npc_loot_digest_sha256.clone(),
            _ => unreachable!(),
        };
        world.prune_consumed_runtime_items().unwrap();
        assert!(!world.item_instances.contains_key(&source));
        let snapshot = world.runtime_snapshot();
        assert!(valid_digest_sha256(&source_digest));

        let mut restored = World::from_definition(sealed_reward_definition()).unwrap();
        restored.restore_runtime_snapshot(snapshot).unwrap();
        assert_eq!(
            restored.item_effective_name(&reward_id).unwrap(),
            "afilada iron sword"
        );
        assert!(matches!(
            &restored.item_provenance[&reward_id],
            ItemProvenance::SealedReward {
                source_npc_loot_digest_sha256,
                ..
            } if source_npc_loot_digest_sha256 == &source_digest
        ));
    }

    #[test]
    fn snapshot_v6_rejects_affix_and_sealed_provenance_smuggling() {
        let (world, _, _) = sealed_reward_world(11);
        let mut legacy = runtime_snapshot_json_for_version(&world.runtime_snapshot(), 6);
        legacy["itemProvenance"]
            .as_object_mut()
            .unwrap()
            .values_mut()
            .next()
            .unwrap()["sealedLootTableId"] = serde_json::json!("reward:cache-table");
        assert!(decode_world_runtime_snapshot(6, &legacy.to_string()).is_err());
        legacy["itemProvenance"]
            .as_object_mut()
            .unwrap()
            .values_mut()
            .next()
            .unwrap()
            .as_object_mut()
            .unwrap()
            .remove("sealedLootTableId");
        let instance = legacy["itemInstances"]
            .as_object_mut()
            .unwrap()
            .values_mut()
            .next()
            .unwrap();
        instance["itemAffixes"] = serde_json::json!([]);
        assert!(decode_world_runtime_snapshot(6, &legacy.to_string()).is_err());

        let mut forged = world.runtime_snapshot();
        let source = forged.item_instances.keys().next().unwrap().clone();
        forged.item_instances.get_mut(&source).unwrap().item_affixes = vec![AppliedItemAffix {
            affix_id: "reward:forged".into(),
            definition_digest_sha256: "0".repeat(64),
            pool_id: "reward:forged-pool".into(),
            pool_digest_sha256: "1".repeat(64),
            record_digest_sha256: "2".repeat(64),
            label: "forged".into(),
            position: AffixPosition::Prefix,
            attack_bonus_delta: 1,
            heal_amount_delta: 0,
        }];
        assert!(matches!(
            validate_world_runtime_snapshot_semantics(&forged),
            Err(WorldError::InvalidRuntimeSnapshot)
        ));
    }

    #[test]
    fn sealed_restore_rejects_forged_provenance_location_binding_and_reservation() {
        fn assert_restore_rejected(snapshot: WorldRuntimeSnapshot) {
            validate_world_runtime_snapshot_semantics(&snapshot).unwrap();
            let mut restored = World::from_definition(sealed_reward_definition()).unwrap();
            assert!(matches!(
                restored.restore_runtime_snapshot(snapshot),
                Err(WorldError::InvalidRuntimeSnapshot)
            ));
        }

        let (world, _, source) = sealed_reward_world(13);
        let base = world.runtime_snapshot();
        let mut correct = World::from_definition(sealed_reward_definition()).unwrap();
        correct.restore_runtime_snapshot(base.clone()).unwrap();
        for marker in [None, Some("reward:other-table".to_owned())] {
            let mut forged = base.clone();
            let ItemProvenance::NpcLoot {
                sealed_loot_table_id,
                ..
            } = forged.item_provenance.get_mut(&source).unwrap()
            else {
                unreachable!();
            };
            *sealed_loot_table_id = marker;
            assert_restore_rejected(forged);
        }
        for forged_provenance in [
            ItemProvenance::AuthoredInitial,
            ItemProvenance::LegacySnapshot { source_version: 5 },
            ItemProvenance::RuntimeCreated { created_tick: 0 },
            ItemProvenance::SealedReward {
                open_id: RewardOpenId(1),
                source_item_id: ItemInstanceId("runtime:999".into()),
                source_npc_loot_digest_sha256: "a".repeat(64),
                sealed_prototype_id: "reward:former-seal".into(),
                loot_table_id: "reward:forged-table".into(),
                table_digest_sha256: "b".repeat(64),
                entry_id: "reward:forged-entry".into(),
                pool_id: None,
                roll_index: 0,
                quantity_index: 0,
                generated_tick: 0,
            },
        ] {
            let mut forged = base.clone();
            if matches!(forged_provenance, ItemProvenance::SealedReward { .. }) {
                forged.next_reward_open_id = RewardOpenId(2);
            }
            forged
                .item_provenance
                .insert(source.clone(), forged_provenance);
            assert_restore_rejected(forged);
        }

        let invalid_states = [
            (RuntimeItemLocation::Staged, ItemBinding::Unbound),
            (
                RuntimeItemLocation::Room {
                    room_id: "test:start".into(),
                },
                ItemBinding::Unbound,
            ),
            (
                RuntimeItemLocation::Carried {
                    character_id: CharacterId(701),
                },
                ItemBinding::Unbound,
            ),
            (
                RuntimeItemLocation::Carried {
                    character_id: CharacterId(701),
                },
                ItemBinding::Character {
                    character_id: CharacterId(702),
                },
            ),
            (
                RuntimeItemLocation::Equipped {
                    character_id: CharacterId(701),
                    slot: EquipmentSlot::Weapon,
                },
                ItemBinding::Character {
                    character_id: CharacterId(701),
                },
            ),
            (
                RuntimeItemLocation::HeldByNpc {
                    npc_id: "reward:holder".into(),
                },
                ItemBinding::Unbound,
            ),
            (RuntimeItemLocation::Consumed, ItemBinding::Unbound),
        ];
        for (location, binding) in invalid_states {
            let mut forged = base.clone();
            let instance = forged.item_instances.get_mut(&source).unwrap();
            instance.location = location;
            instance.binding = binding;
            assert_restore_rejected(forged);
        }

        let mut wrong_reservation = base;
        let instance = wrong_reservation.item_instances.get_mut(&source).unwrap();
        instance.location = RuntimeItemLocation::Room {
            room_id: "test:start".into(),
        };
        instance.binding = ItemBinding::Unbound;
        wrong_reservation.next_loot_drop_id = LootDropId(3);
        wrong_reservation.ground_loot.insert(
            source.clone(),
            GroundLootReservation {
                drop_id: LootDropId(2),
                root_item_id: source,
                room_id: "test:start".into(),
                cohort: vec![CharacterId(701)],
                beneficiary: Some(CharacterId(701)),
                passed: BTreeSet::new(),
                created_tick: 0,
                claim_until_tick: LOOT_EXCLUSIVE_TICKS,
                despawn_at_tick: LOOT_DESPAWN_TICKS,
            },
        );
        assert_restore_rejected(wrong_reservation);
    }

    #[test]
    fn sealed_snapshot_groups_are_bounded_consistent_and_affix_records_self_verify() {
        let (mut world, session, _) = sealed_reward_world(29);
        world
            .apply(session, PlayerAction::OpenReward("sealed cache".into()))
            .unwrap();
        let base = world.runtime_snapshot();
        let reward_id = base
            .item_provenance
            .iter()
            .find_map(|(item_id, provenance)| {
                matches!(provenance, ItemProvenance::SealedReward { .. }).then_some(item_id.clone())
            })
            .unwrap();

        let mut sixteen = base.clone();
        for quantity_index in 1..MAX_SEALED_REWARD_UNITS {
            let sequence = sixteen.next_item_instance_sequence;
            sixteen.next_item_instance_sequence += 1;
            let item_id = ItemInstanceId(format!("runtime:{sequence}"));
            sixteen
                .item_instances
                .insert(item_id.clone(), sixteen.item_instances[&reward_id].clone());
            let mut provenance = sixteen.item_provenance[&reward_id].clone();
            match &mut provenance {
                ItemProvenance::SealedReward {
                    quantity_index: index,
                    ..
                } => *index = u16::try_from(quantity_index).unwrap(),
                _ => unreachable!(),
            }
            sixteen.item_provenance.insert(item_id, provenance);
        }
        validate_world_runtime_snapshot_semantics(&sixteen).unwrap();

        let mut seventeen = sixteen.clone();
        let sequence = seventeen.next_item_instance_sequence;
        seventeen.next_item_instance_sequence += 1;
        let extra_id = ItemInstanceId(format!("runtime:{sequence}"));
        seventeen.item_instances.insert(
            extra_id.clone(),
            seventeen.item_instances[&reward_id].clone(),
        );
        let mut extra_provenance = seventeen.item_provenance[&reward_id].clone();
        match &mut extra_provenance {
            ItemProvenance::SealedReward {
                entry_id,
                quantity_index,
                ..
            } => {
                *entry_id = "reward:seventeenth".into();
                *quantity_index = 0;
            }
            _ => unreachable!(),
        }
        seventeen.item_provenance.insert(extra_id, extra_provenance);
        assert!(validate_world_runtime_snapshot_semantics(&seventeen).is_err());

        let mut mixed = base.clone();
        let sequence = mixed.next_item_instance_sequence;
        mixed.next_item_instance_sequence += 1;
        let mixed_id = ItemInstanceId(format!("runtime:{sequence}"));
        mixed
            .item_instances
            .insert(mixed_id.clone(), mixed.item_instances[&reward_id].clone());
        let mut mixed_provenance = mixed.item_provenance[&reward_id].clone();
        match &mut mixed_provenance {
            ItemProvenance::SealedReward {
                loot_table_id,
                entry_id,
                ..
            } => {
                *loot_table_id = "reward:other-table".into();
                *entry_id = "reward:other-entry".into();
            }
            _ => unreachable!(),
        }
        mixed.item_provenance.insert(mixed_id, mixed_provenance);
        assert!(validate_world_runtime_snapshot_semantics(&mixed).is_err());

        let mut corrupted_pool = base.clone();
        corrupted_pool
            .item_instances
            .get_mut(&reward_id)
            .unwrap()
            .item_affixes[0]
            .pool_digest_sha256 = "0".repeat(64);
        assert!(validate_world_runtime_snapshot_semantics(&corrupted_pool).is_err());
        let mut corrupted_record = base;
        corrupted_record
            .item_instances
            .get_mut(&reward_id)
            .unwrap()
            .item_affixes[0]
            .record_digest_sha256 = "0".repeat(64);
        assert!(validate_world_runtime_snapshot_semantics(&corrupted_record).is_err());
    }

    #[test]
    fn affix_modifier_caps_match_authoring_at_zero_one_thousand_and_overflow() {
        let mut world = World::from_definition(sealed_reward_definition()).unwrap();
        for invalid in [
            AffixDefinition {
                id: "reward:negative".into(),
                label: "negative".into(),
                position: AffixPosition::Prefix,
                attack_bonus_delta: -1,
                heal_amount_delta: 0,
            },
            AffixDefinition {
                id: "reward:empty".into(),
                label: "empty".into(),
                position: AffixPosition::Prefix,
                attack_bonus_delta: 0,
                heal_amount_delta: 0,
            },
            AffixDefinition {
                id: "reward:too-large".into(),
                label: "too large".into(),
                position: AffixPosition::Prefix,
                attack_bonus_delta: MAX_AFFIX_MODIFIER + 1,
                heal_amount_delta: 0,
            },
        ] {
            assert!(matches!(
                world.configure_affixes(vec![invalid], Vec::new()),
                Err(WorldError::InvalidGameplayDefinition)
            ));
        }
        world
            .configure_affixes(
                vec![AffixDefinition {
                    id: "reward:cap".into(),
                    label: "maximum".into(),
                    position: AffixPosition::Prefix,
                    attack_bonus_delta: MAX_AFFIX_MODIFIER,
                    heal_amount_delta: 0,
                }],
                Vec::new(),
            )
            .unwrap();
    }

    #[test]
    fn suppressed_and_admitted_npc_affixes_consume_identical_rng() {
        let (mut admitted, _) = phase_two_world(123, 1, 10);
        let (mut suppressed, _) = phase_two_world(123, 1, 10);
        for world in [&mut admitted, &mut suppressed] {
            world
                .configure_affixes(
                    vec![AffixDefinition {
                        id: "game:keen".into(),
                        label: "keen".into(),
                        position: AffixPosition::Prefix,
                        attack_bonus_delta: 1,
                        heal_amount_delta: 0,
                    }],
                    vec![AffixPoolDefinition {
                        id: "game:weapon-affix".into(),
                        entries: vec![AffixPoolEntry {
                            kind: AffixPoolEntryKind::Affix,
                            id: "game:keen-outcome".into(),
                            weight: 1,
                            affix_id: Some("game:keen".into()),
                        }],
                    }],
                )
                .unwrap();
            world
                .configure_loot(
                    vec![LootTableDefinition {
                        id: "game:guardian-table".into(),
                        digest_sha256: "f".repeat(64),
                        guaranteed: vec![LootGuaranteedDefinition {
                            id: "game:sword-outcome".into(),
                            item_prototype_id: "game:sword".into(),
                            quantity: 1,
                            affix_pool_id: Some("game:weapon-affix".into()),
                        }],
                        pools: Vec::new(),
                    }],
                    vec![LootSourceDefinition {
                        kind: LootSourceKind::NpcSpawn,
                        id: "game:guardian-source".into(),
                        npc_spawn_id: "game:guardian".into(),
                        loot_table_id: "game:guardian-table".into(),
                        rights: LootRightsPolicy::CreditedCharacter,
                    }],
                )
                .unwrap();
        }
        for index in 0..MAX_GROUND_LOOT_RESERVATIONS {
            let item_id = ItemInstanceId(format!("test:saturated-{index}"));
            suppressed.ground_loot.insert(
                item_id.clone(),
                GroundLootReservation {
                    drop_id: LootDropId(99),
                    root_item_id: item_id,
                    room_id: "game:arena".into(),
                    cohort: vec![CharacterId(900)],
                    beneficiary: Some(CharacterId(900)),
                    passed: BTreeSet::new(),
                    created_tick: 0,
                    claim_until_tick: LOOT_EXCLUSIVE_TICKS,
                    despawn_at_tick: LOOT_DESPAWN_TICKS,
                },
            );
        }
        let mut admitted_events = Vec::new();
        admitted
            .resolve_npc_defeat(
                0,
                "game:guardian",
                CharacterId(900),
                "Adventurer",
                "game:arena",
                &mut admitted_events,
            )
            .unwrap();
        let mut suppressed_events = Vec::new();
        suppressed
            .resolve_npc_defeat(
                0,
                "game:guardian",
                CharacterId(900),
                "Adventurer",
                "game:arena",
                &mut suppressed_events,
            )
            .unwrap();
        assert_eq!(admitted.rng.state, suppressed.rng.state);
        assert_eq!(admitted.next_loot_drop_id, suppressed.next_loot_drop_id);
        assert_eq!(admitted.rng.next_u64(), suppressed.rng.next_u64());
        let generated = admitted
            .item_instances
            .values()
            .find(|instance| {
                instance.prototype_id == "game:sword" && !instance.item_affixes.is_empty()
            })
            .unwrap();
        assert_eq!(generated.item_affixes[0].label, "keen");
        assert!(suppressed_events.iter().any(|event| matches!(
            event,
            DomainEvent::Gameplay {
                event: GameplayEvent::LootGenerationSuppressed { .. }
            }
        )));
    }

    #[test]
    fn affixed_items_project_effective_use_and_equip_actions() {
        let (mut reward_world, reward_session, _) = sealed_reward_world(17);
        reward_world
            .apply(
                reward_session,
                PlayerAction::OpenReward("sealed cache".into()),
            )
            .unwrap();
        let reward = reward_world
            .player_state_view(reward_session)
            .unwrap()
            .inventory
            .into_iter()
            .find(|item| item.affix.is_some())
            .unwrap();
        assert_eq!(reward.label, "afilada iron sword");
        assert!(reward.actions.iter().any(|action| action.label == "Equip"));
        let equipped = reward_world
            .apply(reward_session, PlayerAction::Equip(reward.id))
            .unwrap();
        assert!(equipped.output.unwrap().contains("afilada iron sword"));
        assert!(matches!(
            equipped.domain_events.last(),
            Some(DomainEvent::Gameplay {
                event: GameplayEvent::ItemEquipped { item_name, .. }
            }) if item_name == "afilada iron sword"
        ));

        let (mut consumable_world, consumable_session) = phase_two_world(19, 1, 10);
        consumable_world
            .apply(
                consumable_session,
                PlayerAction::Take(ItemSelection::one("potion")),
            )
            .unwrap();
        let potion_id = ItemInstanceId("game:potion".into());
        let definition = AffixDefinition {
            id: "game:restorative".into(),
            label: "restorative".into(),
            position: AffixPosition::Suffix,
            attack_bonus_delta: 0,
            heal_amount_delta: 2,
        };
        let pool = AffixPoolDefinition {
            id: "game:tonic-affix".into(),
            entries: vec![AffixPoolEntry {
                kind: AffixPoolEntryKind::Affix,
                id: "game:restorative-outcome".into(),
                weight: 1,
                affix_id: Some(definition.id.clone()),
            }],
        };
        consumable_world
            .item_prototypes
            .get_mut("game:potion")
            .unwrap()
            .heal_amount = 0;
        consumable_world
            .item_prototypes
            .get_mut("game:potion")
            .unwrap()
            .effect = None;
        let mut applied = AppliedItemAffix {
            affix_id: definition.id.clone(),
            definition_digest_sha256: canonical_sha256(&definition).unwrap(),
            pool_id: pool.id.clone(),
            pool_digest_sha256: canonical_sha256(&pool).unwrap(),
            record_digest_sha256: String::new(),
            label: definition.label,
            position: definition.position,
            attack_bonus_delta: 0,
            heal_amount_delta: 2,
        };
        applied.record_digest_sha256 = applied_affix_record_sha256(&applied).unwrap();
        consumable_world
            .item_instances
            .get_mut(&potion_id)
            .unwrap()
            .item_affixes = vec![applied];
        let potion = consumable_world
            .player_state_view(consumable_session)
            .unwrap()
            .inventory
            .into_iter()
            .find(|item| item.affix.is_some())
            .unwrap();
        assert_eq!(potion.label, "red potion restorative");
        assert!(potion.actions.iter().any(|action| action.label == "Use"));
        let used = consumable_world
            .apply(consumable_session, PlayerAction::Use(potion.id))
            .unwrap();
        assert!(used.output.unwrap().contains("red potion restorative"));
    }

    #[test]
    fn cold_replay_fails_closed_for_every_snapshot_v7_loot_ledger_event() {
        let (mut world, _) = connected_world();
        let item_id = ItemInstanceId("demo:blue-lantern".into());
        let events = vec![
            GameplayEvent::LootGenerated {
                drop_id: LootDropId(1),
                npc_id: "demo:npc".into(),
                npc_name: "npc".into(),
                item_ids: vec![item_id.clone()],
                room_id: "demo:lantern-square".into(),
            },
            GameplayEvent::LootRightsChanged {
                item_id: item_id.clone(),
                room_id: "demo:lantern-square".into(),
                summary: "changed".into(),
            },
            GameplayEvent::LootGenerationSuppressed {
                drop_id: LootDropId(1),
                npc_id: "demo:npc".into(),
                npc_name: "npc".into(),
                reason: "capacity".into(),
                room_id: "demo:lantern-square".into(),
            },
            GameplayEvent::LootDespawned {
                item_ids: vec![item_id],
                room_id: "demo:lantern-square".into(),
            },
            GameplayEvent::LootEncounterCancelled {
                npc_id: "demo:npc".into(),
                npc_name: "npc".into(),
                room_id: "demo:lantern-square".into(),
            },
            GameplayEvent::RewardOpened {
                open_id: RewardOpenId(1),
                character_id: CharacterId(101),
                character_name: "Tester".into(),
                source_item_id: ItemInstanceId("runtime:1".into()),
                source_item_name: "sealed cache".into(),
                item_ids: vec![ItemInstanceId("runtime:2".into())],
                item_names: vec!["forged reward".into()],
                room_id: "demo:lantern-square".into(),
            },
        ];
        for event in events {
            assert!(matches!(
                world.replay_persisted_event(&DomainEvent::Gameplay { event }),
                Err(WorldError::InvalidDomainTransition)
            ));
        }
    }
}
