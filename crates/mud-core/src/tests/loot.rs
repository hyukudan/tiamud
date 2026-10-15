use super::*;

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
    let decoded = decode_world_runtime_snapshot(WORLD_RUNTIME_SNAPSHOT_VERSION, &encoded).unwrap();
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
        action.label == "Probabilidades" && action.command == format!("reward odds {} 1", seal.id)
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
        odds_text.contains("posición prefix; ataque +2; curación +0; probabilidad 3/3 condicional")
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
        .find(|instance| instance.prototype_id == "game:sword" && !instance.item_affixes.is_empty())
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
