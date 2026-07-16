use std::collections::BTreeMap;

use tiamud_core::{
    AbilityCategory, AbilityDamageDefinition, AbilityEffectTarget, AbilityHealDefinition,
    AbilityStatusEffectDefinition, AbilityTargeting, AccountId, ActiveStatusEffect,
    ActiveStatusKind, ArchetypeDefinition, CharacterId, CharacterSnapshot, DamageType, DomainEvent,
    GameplayEvent, LevelProgressionDefinition, NpcAbilitySetDefinition, NpcDefinition,
    PlayerAction, ResourcePool, RoomDefinition, SessionId, World, WorldDefinition, WorldError,
    parse_action,
};

const ROOM_ID: &str = "test:arena";
const ARCHETYPE_ID: &str = "test:hero";
const NPC_ID: &str = "test:sentinel";

fn empty_world() -> World {
    World::from_definition(WorldDefinition {
        start_room_id: ROOM_ID.into(),
        items: Vec::new(),
        rooms: vec![RoomDefinition {
            stable_id: ROOM_ID.into(),
            title: "Test Arena".into(),
            prose: vec!["A bounded room for deterministic combat tests.".into()],
            image: None,
            audio: None,
            exits: Vec::new(),
            item_ids: Vec::new(),
        }],
    })
    .unwrap()
}

fn base_archetype(ability_ids: &[&str]) -> ArchetypeDefinition {
    ArchetypeDefinition {
        id: ARCHETYPE_ID.into(),
        name: "Test Hero".into(),
        description: "A deterministic test archetype.".into(),
        initial_attributes: BTreeMap::from([
            ("agility".into(), 4),
            ("defence".into(), 3),
            ("max_health".into(), 30),
            ("strength".into(), 6),
        ]),
        initial_skills: BTreeMap::from([("combat".into(), 1)]),
        initial_resources: BTreeMap::new(),
        resource_regeneration: BTreeMap::new(),
        initial_resistances: BTreeMap::new(),
        initial_ability_ids: ability_ids.iter().map(|id| (*id).to_owned()).collect(),
        level_progression: Vec::new(),
    }
}

fn target_damage_ability(
    id: &str,
    base_amount: u32,
    damage_type: DamageType,
) -> tiamud_core::AbilityDefinition {
    tiamud_core::AbilityDefinition {
        id: id.into(),
        name: id.rsplit(':').next().unwrap_or(id).replace('-', " "),
        description: "A deterministic test attack.".into(),
        category: AbilityCategory::Martial,
        resource_costs: BTreeMap::new(),
        cooldown_ticks: 0,
        targeting: AbilityTargeting::SingleEnemy,
        damage: Some(AbilityDamageDefinition {
            target: AbilityEffectTarget::Target,
            damage_type,
            base_amount,
            scaling_attribute: None,
            scaling_percent: 0,
        }),
        heal: None,
        status_effects: Vec::new(),
    }
}

fn configure_catalog(
    world: &mut World,
    archetype: ArchetypeDefinition,
    abilities: Vec<tiamud_core::AbilityDefinition>,
) {
    world
        .configure_abilities(archetype.id.clone(), vec![archetype], abilities)
        .unwrap();
}

fn spawn_npc(
    world: &mut World,
    id: &str,
    name: &str,
    max_health: i32,
    strength: i32,
    defence: i32,
    experience_reward: u64,
) {
    world
        .spawn_npc(NpcDefinition {
            stable_id: id.into(),
            name: name.into(),
            description: "A deterministic test opponent.".into(),
            room_id: ROOM_ID.into(),
            level: 1,
            max_health,
            strength,
            defence,
            experience_reward,
            loot_item_ids: Vec::new(),
            respawn_ticks: 5,
        })
        .unwrap();
}

fn enter_character(world: &mut World, archetype_id: &str) -> (SessionId, CharacterId) {
    let session = SessionId(1);
    let character_id = CharacterId(1);
    let gameplay = world.initial_gameplay(Some(archetype_id)).unwrap();
    world
        .restore_character_gameplay(character_id, gameplay)
        .unwrap();
    world
        .connect(
            session,
            CharacterSnapshot {
                id: character_id,
                account_id: AccountId(1),
                name: "Aria".into(),
                room_id: ROOM_ID.into(),
                item_locations: BTreeMap::new(),
            },
        )
        .unwrap();
    (session, character_id)
}

fn use_ability(world: &mut World, session: SessionId, id: &str, target: Option<&str>) {
    world
        .apply(
            session,
            PlayerAction::Ability {
                ability_id: id.into(),
                target: target.map(str::to_owned),
            },
        )
        .unwrap();
}

fn npc_health(world: &World, npc_id: &str) -> i32 {
    world.runtime_snapshot().npcs[npc_id].health
}

#[test]
fn authored_catalog_builds_the_complete_initial_character_sheet() {
    let strike = target_damage_ability("test:strike", 3, DamageType::Physical);
    let mut wanderer = base_archetype(&["test:strike"]);
    wanderer.id = "test:wanderer".into();
    wanderer.name = "Wanderer".into();
    wanderer.initial_resources = BTreeMap::from([("stamina".into(), 8)]);
    wanderer.resource_regeneration = BTreeMap::from([("stamina".into(), 2)]);
    wanderer.initial_resistances = BTreeMap::from([(DamageType::Physical, 10)]);

    let mut vanguard = base_archetype(&["test:strike"]);
    vanguard.id = "test:vanguard".into();
    vanguard.name = "Vanguard".into();

    let mut world = empty_world();
    world
        .configure_abilities(
            wanderer.id.clone(),
            vec![vanguard, wanderer.clone()],
            vec![strike],
        )
        .unwrap();

    assert_eq!(world.default_archetype_id(), "test:wanderer");
    assert_eq!(world.archetype_catalog().len(), 2);
    assert!(
        world
            .archetype_catalog()
            .iter()
            .any(|choice| choice.id == "test:vanguard")
    );

    let sheet = world.initial_gameplay(None).unwrap();
    assert_eq!(sheet.archetype_id, "test:wanderer");
    assert_eq!(sheet.vitals.health, 30);
    assert_eq!(sheet.vitals.max_health, 30);
    assert_eq!(sheet.attributes.strength, 6);
    assert_eq!(sheet.attributes.defence, 3);
    assert_eq!(sheet.attributes.additional["agility"], 4);
    assert_eq!(sheet.skills["combat"], 1);
    assert_eq!(
        sheet.resources["stamina"],
        ResourcePool { current: 8, max: 8 }
    );
    assert_eq!(sheet.resistances[&DamageType::Physical], 10);
    assert_eq!(sheet.ability_ids, ["test:strike"]);

    let (session, _) = enter_character(&mut world, "test:wanderer");
    let player = world.player_state_view(session).unwrap();
    assert_eq!(player.character.archetype.id, "test:wanderer");
    assert_eq!(player.character.attributes.len(), 3);
    assert_eq!(player.character.resources[0].id, "stamina");
    assert_eq!(player.character.abilities[0].id, "test:strike");
}

#[test]
fn parser_accepts_ability_skill_and_cast_aliases() {
    let expected = PlayerAction::Ability {
        ability_id: "test:arcane-bolt".into(),
        target: Some("gate sentinel".into()),
    };

    for alias in ["ability", "skill", "cast"] {
        assert_eq!(
            parse_action(&format!("{alias} test:arcane-bolt gate sentinel")).unwrap(),
            expected
        );
    }
}

#[test]
fn authoritative_ability_lookup_rejects_oversized_transport_queries() {
    let strike = target_damage_ability("test:strike", 3, DamageType::Physical);
    let mut world = empty_world();
    configure_catalog(&mut world, base_archetype(&["test:strike"]), vec![strike]);
    let (session, _) = enter_character(&mut world, ARCHETYPE_ID);
    let before_runtime = world.runtime_snapshot();
    let before_gameplay = world.character_gameplay_snapshot(session).unwrap();

    let result = world.apply(
        session,
        PlayerAction::Ability {
            ability_id: "x".repeat(129),
            target: None,
        },
    );
    assert!(matches!(result, Err(WorldError::AbilityNotKnown)));
    assert_eq!(world.runtime_snapshot(), before_runtime);
    assert_eq!(
        world.character_gameplay_snapshot(session).unwrap(),
        before_gameplay
    );
}

#[test]
fn global_cooldown_blocks_same_tick_spam_and_the_ability_replaces_autoattack() {
    let strike = target_damage_ability("test:strike", 5, DamageType::Physical);
    let mut world = empty_world();
    configure_catalog(&mut world, base_archetype(&["test:strike"]), vec![strike]);
    spawn_npc(&mut world, NPC_ID, "Training Sentinel", 100, 1, 100, 0);
    let (session, _) = enter_character(&mut world, ARCHETYPE_ID);

    use_ability(
        &mut world,
        session,
        "test:strike",
        Some("Training Sentinel"),
    );
    assert_eq!(npc_health(&world, NPC_ID), 95);

    let repeated = world.apply(
        session,
        PlayerAction::Ability {
            ability_id: "test:strike".into(),
            target: Some("Training Sentinel".into()),
        },
    );
    assert!(matches!(repeated, Err(WorldError::AbilityUnavailable)));
    assert_eq!(npc_health(&world, NPC_ID), 95);

    world.advance_tick().unwrap();
    assert_eq!(
        npc_health(&world, NPC_ID),
        95,
        "the normal player attack must not stack with the preceding ability"
    );

    world.advance_tick().unwrap();
    assert_eq!(npc_health(&world, NPC_ID), 94);
}

#[test]
fn resource_cost_cooldown_and_authored_regeneration_share_world_ticks() {
    let focus = tiamud_core::AbilityDefinition {
        id: "test:focus".into(),
        name: "Focus".into(),
        description: "Spend mana and recover a little health.".into(),
        category: AbilityCategory::Spell,
        resource_costs: BTreeMap::from([("mana".into(), 3)]),
        cooldown_ticks: 2,
        targeting: AbilityTargeting::SelfOnly,
        damage: None,
        heal: Some(AbilityHealDefinition {
            target: AbilityEffectTarget::Caster,
            base_amount: 1,
            scaling_attribute: None,
            scaling_percent: 0,
        }),
        status_effects: Vec::new(),
    };
    let mut archetype = base_archetype(&["test:focus"]);
    archetype.initial_resources = BTreeMap::from([("mana".into(), 10)]);
    archetype.resource_regeneration = BTreeMap::from([("mana".into(), 2)]);
    let mut world = empty_world();
    configure_catalog(&mut world, archetype, vec![focus]);
    let (session, character_id) = enter_character(&mut world, ARCHETYPE_ID);

    use_ability(&mut world, session, "test:focus", None);
    let spent = world.character_gameplay_snapshot(session).unwrap();
    assert_eq!(spent.resources["mana"].current, 7);
    assert_eq!(spent.ability_cooldowns["test:focus"], 2);

    world.advance_tick().unwrap();
    assert_eq!(
        world
            .character_gameplay_snapshot(session)
            .unwrap()
            .resources["mana"]
            .current,
        9
    );
    let cooling_down = world.apply(
        session,
        PlayerAction::Ability {
            ability_id: "test:focus".into(),
            target: None,
        },
    );
    assert!(matches!(cooling_down, Err(WorldError::AbilityUnavailable)));

    world.advance_tick().unwrap();
    assert_eq!(
        world
            .character_gameplay_snapshot(session)
            .unwrap()
            .resources["mana"]
            .current,
        10
    );
    use_ability(&mut world, session, "test:focus", None);
    assert_eq!(
        world
            .character_gameplay_snapshot(session)
            .unwrap()
            .resources["mana"]
            .current,
        7
    );

    // Keep the durable ID exercised by this integration test rather than only
    // checking the session-scoped projection.
    assert_eq!(character_id, CharacterId(1));
}

#[test]
fn npc_resistance_reduces_ability_damage_by_the_authored_percentage() {
    let fire = target_damage_ability("test:fire", 10, DamageType::Fire);
    let mut world = empty_world();
    configure_catalog(&mut world, base_archetype(&["test:fire"]), vec![fire]);
    spawn_npc(&mut world, NPC_ID, "Fire Sentinel", 50, 1, 0, 0);
    world
        .configure_npc_abilities(NpcAbilitySetDefinition {
            npc_id: NPC_ID.into(),
            resource_pools: BTreeMap::new(),
            resource_regeneration: BTreeMap::new(),
            resistances: BTreeMap::from([(DamageType::Fire, 50)]),
            ability_ids: Vec::new(),
        })
        .unwrap();
    let (session, _) = enter_character(&mut world, ARCHETYPE_ID);

    use_ability(&mut world, session, "test:fire", Some("Fire Sentinel"));
    assert_eq!(npc_health(&world, NPC_ID), 45);
}

#[test]
fn defeating_an_npc_applies_authored_level_progression_and_unlocks() {
    let tap = target_damage_ability("test:tap", 5, DamageType::Physical);
    let mut power = target_damage_ability("test:power", 10, DamageType::Physical);
    power.resource_costs = BTreeMap::from([("stamina".into(), 5)]);

    let mut archetype = base_archetype(&["test:tap"]);
    archetype.initial_attributes = BTreeMap::from([
        ("agility".into(), 2),
        ("defence".into(), 0),
        ("max_health".into(), 20),
        ("strength".into(), 1),
    ]);
    archetype.initial_resources = BTreeMap::from([("stamina".into(), 2)]);
    archetype.level_progression = vec![LevelProgressionDefinition {
        level: 2,
        total_experience_required: 10,
        attribute_gains: BTreeMap::from([
            ("agility".into(), 1),
            ("max_health".into(), 4),
            ("strength".into(), 2),
        ]),
        skill_gains: BTreeMap::from([("combat".into(), 2)]),
        resource_gains: BTreeMap::from([("stamina".into(), 3)]),
        ability_unlock_ids: vec!["test:power".into()],
    }];

    let mut world = empty_world();
    configure_catalog(&mut world, archetype, vec![tap, power]);
    spawn_npc(&mut world, NPC_ID, "Frail Sentinel", 1, 1, 0, 10);
    let (session, _) = enter_character(&mut world, ARCHETYPE_ID);

    let outcome = world
        .apply(
            session,
            PlayerAction::Ability {
                ability_id: "test:tap".into(),
                target: Some("Frail Sentinel".into()),
            },
        )
        .unwrap();
    assert!(outcome.domain_events.iter().any(|event| matches!(
        event,
        DomainEvent::Gameplay {
            event: GameplayEvent::CharacterLeveled { level: 2, .. }
        }
    )));

    let sheet = world.character_gameplay_snapshot(session).unwrap();
    assert_eq!(sheet.level, 2);
    assert_eq!(sheet.experience, 10);
    assert_eq!(
        sheet.vitals,
        tiamud_core::Vitals {
            health: 24,
            max_health: 24
        }
    );
    assert_eq!(sheet.attributes.strength, 3);
    assert_eq!(sheet.attributes.additional["agility"], 3);
    assert_eq!(sheet.skills["combat"], 3);
    assert_eq!(
        sheet.resources["stamina"],
        ResourcePool { current: 5, max: 5 }
    );
    assert_eq!(sheet.ability_ids, ["test:tap", "test:power"]);
}

#[test]
fn one_experience_reward_advances_through_every_reached_milestone() {
    let tap = target_damage_ability("test:tap", 5, DamageType::Physical);
    let first_unlock = target_damage_ability("test:first", 1, DamageType::Physical);
    let second_unlock = target_damage_ability("test:second", 1, DamageType::Physical);
    let mut archetype = base_archetype(&["test:tap"]);
    archetype.level_progression = vec![
        LevelProgressionDefinition {
            level: 2,
            total_experience_required: 10,
            attribute_gains: BTreeMap::from([("strength".into(), 1)]),
            skill_gains: BTreeMap::new(),
            resource_gains: BTreeMap::new(),
            ability_unlock_ids: vec!["test:first".into()],
        },
        LevelProgressionDefinition {
            level: 3,
            total_experience_required: 20,
            attribute_gains: BTreeMap::from([("strength".into(), 2)]),
            skill_gains: BTreeMap::new(),
            resource_gains: BTreeMap::new(),
            ability_unlock_ids: vec!["test:second".into()],
        },
    ];
    let mut world = empty_world();
    configure_catalog(
        &mut world,
        archetype,
        vec![tap, first_unlock, second_unlock],
    );
    spawn_npc(&mut world, NPC_ID, "Reward Sentinel", 1, 1, 0, 30);
    let (session, _) = enter_character(&mut world, ARCHETYPE_ID);

    let outcome = world
        .apply(
            session,
            PlayerAction::Ability {
                ability_id: "test:tap".into(),
                target: Some("Reward Sentinel".into()),
            },
        )
        .unwrap();
    assert_eq!(
        outcome
            .domain_events
            .iter()
            .filter(|event| matches!(
                event,
                DomainEvent::Gameplay {
                    event: GameplayEvent::CharacterLeveled { .. }
                }
            ))
            .count(),
        2
    );
    let sheet = world.character_gameplay_snapshot(session).unwrap();
    assert_eq!(sheet.level, 3);
    assert_eq!(sheet.experience, 30);
    assert_eq!(sheet.attributes.strength, 9);
    assert_eq!(sheet.ability_ids, ["test:tap", "test:first", "test:second"]);
}

#[test]
fn npc_ai_prefers_the_stronger_eligible_technique_and_spends_its_resource() {
    let basic = target_damage_ability("test:basic", 1, DamageType::Physical);
    let mut technique = target_damage_ability("test:technique", 7, DamageType::Physical);
    technique.resource_costs = BTreeMap::from([("stamina".into(), 2)]);
    technique.cooldown_ticks = 2;

    let mut world = empty_world();
    configure_catalog(&mut world, base_archetype(&[]), vec![basic, technique]);
    spawn_npc(&mut world, NPC_ID, "Battle Sentinel", 100, 1, 100, 0);
    world
        .configure_npc_abilities(NpcAbilitySetDefinition {
            npc_id: NPC_ID.into(),
            resource_pools: BTreeMap::from([("stamina".into(), 4)]),
            resource_regeneration: BTreeMap::new(),
            resistances: BTreeMap::new(),
            ability_ids: vec!["test:basic".into(), "test:technique".into()],
        })
        .unwrap();
    let (session, _) = enter_character(&mut world, ARCHETYPE_ID);
    world
        .apply(session, PlayerAction::Attack("Battle Sentinel".into()))
        .unwrap();

    let events = world.advance_tick().unwrap();
    assert!(events.iter().any(|event| matches!(
        event,
        DomainEvent::Gameplay {
            event: GameplayEvent::NpcAbilityUsed { ability_id, .. }
        } if ability_id == "test:technique"
    )));
    let snapshot = world.runtime_snapshot();
    assert_eq!(snapshot.npcs[NPC_ID].resources["stamina"].current, 2);
    assert_eq!(
        world
            .character_gameplay_snapshot(session)
            .unwrap()
            .vitals
            .health,
        23
    );
}

#[test]
fn npc_damage_over_time_can_defeat_and_attribute_the_character() {
    let affliction = tiamud_core::AbilityDefinition {
        id: "test:npc-affliction".into(),
        name: "Withering mark".into(),
        description: "A deterministic hostile periodic effect.".into(),
        category: AbilityCategory::Innate,
        resource_costs: BTreeMap::new(),
        cooldown_ticks: 3,
        targeting: AbilityTargeting::SingleEnemy,
        damage: None,
        heal: None,
        status_effects: vec![AbilityStatusEffectDefinition::DamageOverTime {
            target: AbilityEffectTarget::Target,
            damage_type: DamageType::Arcane,
            magnitude: 30,
            duration_ticks: 1,
        }],
    };
    let mut world = empty_world();
    configure_catalog(&mut world, base_archetype(&[]), vec![affliction]);
    spawn_npc(&mut world, NPC_ID, "Withering Sentinel", 100, 1, 100, 0);
    world
        .configure_npc_abilities(NpcAbilitySetDefinition {
            npc_id: NPC_ID.into(),
            resource_pools: BTreeMap::new(),
            resource_regeneration: BTreeMap::new(),
            resistances: BTreeMap::new(),
            ability_ids: vec!["test:npc-affliction".into()],
        })
        .unwrap();
    let (session, _) = enter_character(&mut world, ARCHETYPE_ID);
    world
        .apply(session, PlayerAction::Attack("Withering Sentinel".into()))
        .unwrap();

    world.advance_tick().unwrap();
    let events = world.advance_tick().unwrap();
    assert!(events.iter().any(|event| matches!(
        event,
        DomainEvent::Gameplay {
            event: GameplayEvent::CharacterDefeated { npc_id, .. }
        } if npc_id == NPC_ID
    )));
    let sheet = world.character_gameplay_snapshot(session).unwrap();
    assert_eq!(sheet.vitals.health, 0);
    assert!(sheet.recovery_at_tick.is_some());
}

#[test]
fn stun_suppresses_the_npc_turn_while_damage_over_time_keeps_attribution() {
    let affliction = tiamud_core::AbilityDefinition {
        id: "test:affliction".into(),
        name: "Affliction".into(),
        description: "Stun an enemy while deterministic damage resolves.".into(),
        category: AbilityCategory::Spell,
        resource_costs: BTreeMap::new(),
        cooldown_ticks: 0,
        targeting: AbilityTargeting::SingleEnemy,
        damage: None,
        heal: None,
        status_effects: vec![
            AbilityStatusEffectDefinition::Stun {
                target: AbilityEffectTarget::Target,
                duration_ticks: 1,
            },
            AbilityStatusEffectDefinition::DamageOverTime {
                target: AbilityEffectTarget::Target,
                damage_type: DamageType::Arcane,
                magnitude: 4,
                duration_ticks: 2,
            },
        ],
    };
    let mut world = empty_world();
    configure_catalog(
        &mut world,
        base_archetype(&["test:affliction"]),
        vec![affliction],
    );
    spawn_npc(&mut world, NPC_ID, "Cursed Sentinel", 8, 10, 100, 5);
    let (session, _) = enter_character(&mut world, ARCHETYPE_ID);

    use_ability(
        &mut world,
        session,
        "test:affliction",
        Some("Cursed Sentinel"),
    );
    world.advance_tick().unwrap();
    assert_eq!(npc_health(&world, NPC_ID), 4);
    assert_eq!(
        world
            .character_gameplay_snapshot(session)
            .unwrap()
            .vitals
            .health,
        30,
        "stun must suppress the NPC response on the first status tick"
    );

    let events = world.advance_tick().unwrap();
    assert_eq!(npc_health(&world, NPC_ID), 0);
    assert!(events.iter().any(|event| matches!(
        event,
        DomainEvent::Gameplay {
            event: GameplayEvent::NpcDefeated { character_id, .. }
        } if *character_id == CharacterId(1)
    )));
    assert_eq!(
        world
            .character_gameplay_snapshot(session)
            .unwrap()
            .experience,
        5
    );
}

#[test]
fn stun_suppresses_the_players_automatic_combat_turn() {
    let stun = tiamud_core::AbilityDefinition {
        id: "test:stun".into(),
        name: "Stun".into(),
        description: "Prevent an enemy from taking its next automatic turns.".into(),
        category: AbilityCategory::Martial,
        resource_costs: BTreeMap::new(),
        cooldown_ticks: 3,
        targeting: AbilityTargeting::SingleEnemy,
        damage: None,
        heal: None,
        status_effects: vec![AbilityStatusEffectDefinition::Stun {
            target: AbilityEffectTarget::Target,
            duration_ticks: 2,
        }],
    };
    let mut world = empty_world();
    configure_catalog(&mut world, base_archetype(&[]), vec![stun]);
    spawn_npc(&mut world, NPC_ID, "Stunning Sentinel", 100, 1, 100, 0);
    world
        .configure_npc_abilities(NpcAbilitySetDefinition {
            npc_id: NPC_ID.into(),
            resource_pools: BTreeMap::new(),
            resource_regeneration: BTreeMap::new(),
            resistances: BTreeMap::new(),
            ability_ids: vec!["test:stun".into()],
        })
        .unwrap();
    let (session, _) = enter_character(&mut world, ARCHETYPE_ID);
    world
        .apply(session, PlayerAction::Attack("Stunning Sentinel".into()))
        .unwrap();

    let first_turn = world.advance_tick().unwrap();
    assert!(first_turn.iter().any(|event| matches!(
        event,
        DomainEvent::Gameplay {
            event: GameplayEvent::NpcAbilityUsed { ability_id, .. }
        } if ability_id == "test:stun"
    )));
    let health_after_first_turn = npc_health(&world, NPC_ID);

    let stunned_turn = world.advance_tick().unwrap();
    assert_eq!(
        npc_health(&world, NPC_ID),
        health_after_first_turn,
        "a stunned character must not perform its automatic attack"
    );
    assert!(!stunned_turn.iter().any(|event| matches!(
        event,
        DomainEvent::Gameplay {
            event: GameplayEvent::NpcDamaged { character_id, .. }
        } if *character_id == CharacterId(1)
    )));
}

#[test]
fn legacy_level_three_migration_replays_all_reached_authored_milestones_once() {
    let tap = target_damage_ability("test:tap", 2, DamageType::Physical);
    let power = target_damage_ability("test:power", 4, DamageType::Physical);
    let finisher = target_damage_ability("test:finisher", 6, DamageType::Physical);
    let mut archetype = base_archetype(&["test:tap"]);
    archetype.initial_attributes.insert("agility".into(), 2);
    archetype.initial_resources = BTreeMap::from([("stamina".into(), 2)]);
    archetype.level_progression = vec![
        LevelProgressionDefinition {
            level: 2,
            total_experience_required: 10,
            attribute_gains: BTreeMap::from([
                ("agility".into(), 1),
                ("max_health".into(), 4),
                ("strength".into(), 2),
            ]),
            skill_gains: BTreeMap::from([("combat".into(), 2)]),
            resource_gains: BTreeMap::from([("stamina".into(), 3)]),
            ability_unlock_ids: vec!["test:power".into()],
        },
        LevelProgressionDefinition {
            level: 3,
            total_experience_required: 30,
            attribute_gains: BTreeMap::from([
                ("agility".into(), 2),
                ("defence".into(), 3),
                ("max_health".into(), 5),
            ]),
            skill_gains: BTreeMap::from([("combat".into(), 1), ("lore".into(), 4)]),
            resource_gains: BTreeMap::from([("stamina".into(), 2)]),
            ability_unlock_ids: vec!["test:finisher".into()],
        },
    ];
    let mut world = empty_world();
    configure_catalog(&mut world, archetype, vec![tap, power, finisher]);

    let mut legacy = tiamud_core::CharacterGameplaySnapshot {
        level: 3,
        experience: 30,
        ..tiamud_core::CharacterGameplaySnapshot::default()
    };
    // The pre-archetype snapshot shape has the old default ID and none of the
    // additive collections. Preserve a non-full health value across migration.
    legacy.vitals.health = 17;
    world
        .restore_character_gameplay(CharacterId(1), legacy)
        .unwrap();
    world
        .connect(
            SessionId(1),
            CharacterSnapshot {
                id: CharacterId(1),
                account_id: AccountId(1),
                name: "Aria".into(),
                room_id: ROOM_ID.into(),
                item_locations: BTreeMap::new(),
            },
        )
        .unwrap();

    let migrated = world.character_gameplay_snapshot(SessionId(1)).unwrap();
    assert_eq!(migrated.archetype_id, ARCHETYPE_ID);
    assert_eq!(migrated.vitals.health, 17);
    assert_eq!(migrated.vitals.max_health, 29);
    assert_eq!(migrated.attributes.strength, 7);
    assert_eq!(migrated.attributes.defence, 5);
    assert_eq!(migrated.attributes.additional["agility"], 5);
    assert_eq!(migrated.skills["combat"], 13);
    assert_eq!(migrated.skills["lore"], 4);
    assert_eq!(
        migrated.resources["stamina"],
        ResourcePool { current: 7, max: 7 }
    );
    assert_eq!(
        migrated.ability_ids,
        ["test:tap", "test:power", "test:finisher"]
    );

    // A current-shape snapshot carries additive fields, so restoring it must
    // not replay the migration milestones a second time.
    world
        .restore_character_gameplay(CharacterId(1), migrated.clone())
        .unwrap();
    assert_eq!(
        world.character_gameplay_snapshot(SessionId(1)).unwrap(),
        migrated
    );
}

#[test]
fn current_snapshot_with_empty_optional_collections_does_not_replay_milestones() {
    let mut archetype = base_archetype(&[]);
    archetype.level_progression = vec![LevelProgressionDefinition {
        level: 2,
        total_experience_required: 10,
        attribute_gains: BTreeMap::from([("strength".into(), 2)]),
        skill_gains: BTreeMap::from([("combat".into(), 3)]),
        resource_gains: BTreeMap::new(),
        ability_unlock_ids: Vec::new(),
    }];
    let mut world = empty_world();
    configure_catalog(&mut world, archetype, Vec::new());

    let mut current = world.initial_gameplay(Some(ARCHETYPE_ID)).unwrap();
    current.level = 2;
    current.experience = 10;
    current.attributes.strength += 2;
    *current.skills.get_mut("combat").unwrap() += 3;
    assert!(current.resources.is_empty());
    assert!(current.ability_ids.is_empty());
    let expected = current.clone();

    world
        .restore_character_gameplay(CharacterId(1), current)
        .unwrap();
    world
        .connect(
            SessionId(1),
            CharacterSnapshot {
                id: CharacterId(1),
                account_id: AccountId(1),
                name: "Aria".into(),
                room_id: ROOM_ID.into(),
                item_locations: BTreeMap::new(),
            },
        )
        .unwrap();

    assert_eq!(
        world.character_gameplay_snapshot(SessionId(1)).unwrap(),
        expected
    );
}

#[test]
fn one_tick_attribute_modifier_affects_combat_before_expiring() {
    let weaken = tiamud_core::AbilityDefinition {
        id: "test:weaken".into(),
        name: "Weaken".into(),
        description: "Reduce an enemy's strength for one deterministic turn.".into(),
        category: AbilityCategory::Spell,
        resource_costs: BTreeMap::new(),
        cooldown_ticks: 0,
        targeting: AbilityTargeting::SingleEnemy,
        damage: None,
        heal: None,
        status_effects: vec![AbilityStatusEffectDefinition::AttributeModifier {
            target: AbilityEffectTarget::Target,
            attribute: "strength".into(),
            magnitude: -100,
            duration_ticks: 1,
        }],
    };
    let mut world = empty_world();
    configure_catalog(&mut world, base_archetype(&["test:weaken"]), vec![weaken]);
    spawn_npc(&mut world, NPC_ID, "Weakened Sentinel", 100, 10, 100, 5);
    let (session, _) = enter_character(&mut world, ARCHETYPE_ID);

    use_ability(
        &mut world,
        session,
        "test:weaken",
        Some("Weakened Sentinel"),
    );
    world.advance_tick().unwrap();

    assert_eq!(
        world
            .character_gameplay_snapshot(session)
            .unwrap()
            .vitals
            .health,
        29,
        "the duration-one strength penalty must affect the NPC's next attack"
    );
    assert!(
        world.runtime_snapshot().npcs[NPC_ID].statuses.is_empty(),
        "the modifier must expire after affecting exactly one combat turn"
    );
}

#[test]
fn corrupt_runtime_snapshots_are_rejected_without_partial_mutation() {
    let technique = target_damage_ability("test:technique", 5, DamageType::Physical);
    let mut world = empty_world();
    configure_catalog(&mut world, base_archetype(&[]), vec![technique]);
    spawn_npc(&mut world, NPC_ID, "Snapshot Sentinel", 20, 2, 1, 0);
    world
        .configure_npc_abilities(NpcAbilitySetDefinition {
            npc_id: NPC_ID.into(),
            resource_pools: BTreeMap::from([("stamina".into(), 6)]),
            resource_regeneration: BTreeMap::from([("stamina".into(), 1)]),
            resistances: BTreeMap::new(),
            ability_ids: vec!["test:technique".into()],
        })
        .unwrap();
    let baseline = world.runtime_snapshot();

    let mut overfilled = baseline.clone();
    overfilled
        .npcs
        .get_mut(NPC_ID)
        .unwrap()
        .resources
        .insert("stamina".into(), ResourcePool { current: 7, max: 6 });
    assert!(matches!(
        world.restore_runtime_snapshot(overfilled),
        Err(WorldError::InvalidRuntimeSnapshot)
    ));
    assert_eq!(world.runtime_snapshot(), baseline);

    let mut forged_status = baseline.clone();
    forged_status
        .npcs
        .get_mut(NPC_ID)
        .unwrap()
        .statuses
        .push(ActiveStatusEffect {
            source_ability_id: "test:technique".into(),
            source_character_id: Some(CharacterId(1)),
            source_npc_id: None,
            kind: ActiveStatusKind::DamageOverTime,
            magnitude: 100_000,
            remaining_ticks: 1,
            attribute: None,
            damage_type: Some(DamageType::Physical),
        });
    world.restore_runtime_snapshot(forged_status).unwrap();
    assert!(world.runtime_snapshot().npcs[NPC_ID].statuses.is_empty());
    assert_eq!(world.runtime_snapshot(), baseline);

    let mut forged_npc = baseline.clone();
    let mut ghost = forged_npc.npcs[NPC_ID].clone();
    ghost.stable_id = "test:ghost".into();
    forged_npc.npcs.insert("test:ghost".into(), ghost);
    assert!(matches!(
        world.restore_runtime_snapshot(forged_npc),
        Err(WorldError::InvalidRuntimeSnapshot)
    ));
    assert_eq!(world.runtime_snapshot(), baseline);
}

#[test]
fn disconnect_cancels_character_sourced_npc_statuses_before_offline_ticks() {
    let affliction = tiamud_core::AbilityDefinition {
        id: "test:affliction".into(),
        name: "Affliction".into(),
        description: "A pending effect that must not outlive its combat session.".into(),
        category: AbilityCategory::Spell,
        resource_costs: BTreeMap::new(),
        cooldown_ticks: 0,
        targeting: AbilityTargeting::SingleEnemy,
        damage: None,
        heal: None,
        status_effects: vec![AbilityStatusEffectDefinition::DamageOverTime {
            target: AbilityEffectTarget::Target,
            damage_type: DamageType::Arcane,
            magnitude: 5,
            duration_ticks: 2,
        }],
    };
    let mut world = empty_world();
    configure_catalog(
        &mut world,
        base_archetype(&["test:affliction"]),
        vec![affliction],
    );
    spawn_npc(&mut world, NPC_ID, "Lingering Sentinel", 5, 1, 100, 10);
    let (session, _) = enter_character(&mut world, ARCHETYPE_ID);

    use_ability(
        &mut world,
        session,
        "test:affliction",
        Some("Lingering Sentinel"),
    );
    assert_eq!(world.runtime_snapshot().npcs[NPC_ID].statuses.len(), 1);

    world.disconnect(session);
    let after_disconnect = world.runtime_snapshot();
    assert!(after_disconnect.npcs[NPC_ID].statuses.is_empty());
    assert_eq!(after_disconnect.npcs[NPC_ID].engaged_character, None);

    let events = world.advance_tick().unwrap();
    assert_eq!(npc_health(&world, NPC_ID), 5);
    assert!(!events.iter().any(|event| matches!(
        event,
        DomainEvent::Gameplay {
            event: GameplayEvent::NpcDefeated { .. }
        }
    )));
}

#[test]
fn retired_abilities_do_not_block_character_or_runtime_restore() {
    let retired = tiamud_core::AbilityDefinition {
        id: "test:retired".into(),
        name: "Retired art".into(),
        description: "A content definition removed by the next reviewed pack.".into(),
        category: AbilityCategory::Innate,
        resource_costs: BTreeMap::new(),
        cooldown_ticks: 5,
        targeting: AbilityTargeting::SelfOnly,
        damage: None,
        heal: None,
        status_effects: vec![AbilityStatusEffectDefinition::Regeneration {
            target: AbilityEffectTarget::Caster,
            magnitude: 1,
            duration_ticks: 3,
        }],
    };
    let mut old_world = empty_world();
    configure_catalog(
        &mut old_world,
        base_archetype(&["test:retired"]),
        vec![retired],
    );
    spawn_npc(&mut old_world, NPC_ID, "Old Sentinel", 20, 1, 1, 0);
    let (session, character_id) = enter_character(&mut old_world, ARCHETYPE_ID);
    use_ability(&mut old_world, session, "test:retired", None);
    let saved_character = old_world.character_gameplay_snapshot(session).unwrap();
    let mut saved_runtime = old_world.runtime_snapshot();
    saved_runtime
        .npcs
        .get_mut(NPC_ID)
        .unwrap()
        .ability_ids
        .push("test:retired".into());
    saved_runtime
        .npcs
        .get_mut(NPC_ID)
        .unwrap()
        .ability_cooldowns
        .insert("test:retired".into(), 3);

    let mut current_world = empty_world();
    configure_catalog(&mut current_world, base_archetype(&[]), Vec::new());
    spawn_npc(&mut current_world, NPC_ID, "Current Sentinel", 20, 1, 1, 0);
    current_world
        .restore_character_gameplay(character_id, saved_character)
        .unwrap();
    current_world
        .restore_runtime_snapshot(saved_runtime)
        .unwrap();
    current_world
        .connect(
            SessionId(2),
            CharacterSnapshot {
                id: character_id,
                account_id: AccountId(1),
                name: "Aria".into(),
                room_id: ROOM_ID.into(),
                item_locations: BTreeMap::new(),
            },
        )
        .unwrap();

    let restored = current_world
        .character_gameplay_snapshot(SessionId(2))
        .unwrap();
    assert!(restored.ability_ids.is_empty());
    assert!(restored.ability_cooldowns.is_empty());
    assert!(restored.statuses.is_empty());
    assert!(
        current_world.runtime_snapshot().npcs[NPC_ID]
            .ability_cooldowns
            .is_empty()
    );
}

#[test]
fn runtime_restore_overlays_dynamic_state_onto_new_authored_npc_configuration() {
    let technique = target_damage_ability("test:technique", 5, DamageType::Physical);

    let mut old_world = empty_world();
    configure_catalog(&mut old_world, base_archetype(&[]), vec![technique.clone()]);
    spawn_npc(&mut old_world, NPC_ID, "Old Sentinel", 30, 1, 0, 0);
    let old_snapshot = old_world.runtime_snapshot();
    assert!(old_snapshot.npcs[NPC_ID].ability_ids.is_empty());

    let mut current_world = empty_world();
    configure_catalog(&mut current_world, base_archetype(&[]), vec![technique]);
    spawn_npc(&mut current_world, NPC_ID, "Current Sentinel", 50, 4, 2, 0);
    current_world
        .configure_npc_abilities(NpcAbilitySetDefinition {
            npc_id: NPC_ID.into(),
            resource_pools: BTreeMap::from([("stamina".into(), 6)]),
            resource_regeneration: BTreeMap::from([("stamina".into(), 1)]),
            resistances: BTreeMap::from([(DamageType::Fire, 25)]),
            ability_ids: vec!["test:technique".into()],
        })
        .unwrap();
    spawn_npc(&mut current_world, "test:newcomer", "Newcomer", 15, 2, 1, 0);

    current_world
        .restore_runtime_snapshot(old_snapshot)
        .unwrap();
    let restored = current_world.runtime_snapshot();
    let sentinel = &restored.npcs[NPC_ID];
    assert_eq!(sentinel.name, "Current Sentinel");
    assert_eq!(sentinel.max_health, 50);
    assert_eq!(sentinel.health, 30);
    assert_eq!(sentinel.strength, 4);
    assert_eq!(sentinel.defence, 2);
    assert_eq!(sentinel.ability_ids, ["test:technique"]);
    assert_eq!(
        sentinel.resources["stamina"],
        ResourcePool { current: 6, max: 6 }
    );
    assert_eq!(sentinel.resource_regeneration["stamina"], 1);
    assert_eq!(sentinel.resistances[&DamageType::Fire], 25);
    assert!(restored.npcs.contains_key("test:newcomer"));
}
