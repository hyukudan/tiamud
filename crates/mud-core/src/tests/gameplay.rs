use super::*;

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
