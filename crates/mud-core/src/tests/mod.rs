//! Unit tests for the world model. Shared fixtures live here; the tests
//! are grouped by subsystem in child modules.

use super::*;

mod areas;
mod gameplay;
mod items;
mod loot;
mod parse;
mod perception;
mod quests;
mod snapshot;
mod staff;
mod trade;
mod world;

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
    let (mut world, _) = World::from_initial_prepared_areas("area-a:start", full_batch).unwrap();
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
