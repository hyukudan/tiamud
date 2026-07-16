use std::collections::{BTreeMap, BTreeSet};

use tiamud_core::{
    AbilityCategory, AbilityDamageDefinition, AbilityDefinition, AbilityEffectTarget,
    AbilityTargeting, AccountId, ActiveEffect, ActiveStatusEffect, ActiveStatusKind,
    AdvancedClassDefinition, BackgroundDefinition, BackgroundSelection, BaseClassDefinition,
    CharacterDefaultsDefinition, CharacterGameplaySnapshot, CharacterId,
    CharacterProgressionDefinition, CharacterSnapshot, DamageType, DomainEvent, EffectKind,
    GameplayEvent, GrantDefinition, LevelGrantDefinition, LevelThresholdDefinition,
    LineageDefinition, NpcDefinition, PlayerAction, RequirementDefinition, RequirementDisclosure,
    RequirementPredicate, ResourcePool, RoomDefinition, SessionId, TalentGraphDefinition,
    TalentNodeDefinition, TalentRankDefinition, World, WorldDefinition, WorldError, parse_action,
};

const ROOM_ID: &str = "test:arena";
const LINEAGE_ID: &str = "test:human";
const CLASS_ID: &str = "test:warden";
const NPC_ID: &str = "test:sentinel";

fn empty_world() -> World {
    World::from_definition(WorldDefinition {
        start_room_id: ROOM_ID.into(),
        item_prototypes: Vec::new(),
        initial_item_instances: Vec::new(),
        rooms: vec![RoomDefinition {
            stable_id: ROOM_ID.into(),
            title: "Test Arena".into(),
            prose: vec!["A bounded room for deterministic progression tests.".into()],
            image: None,
            audio: None,
            exits: Vec::new(),
            item_ids: Vec::new(),
        }],
    })
    .unwrap()
}

fn origin_lineage(id: &str, grants: Vec<GrantDefinition>) -> LineageDefinition {
    LineageDefinition {
        id: id.into(),
        name: id.rsplit(':').next().unwrap().into(),
        description: "A deterministic test lineage.".into(),
        ui_order: 0,
        creation_grants: grants,
        talent_graph_ids: Vec::new(),
    }
}

fn background(id: &str, grants: Vec<GrantDefinition>) -> BackgroundDefinition {
    BackgroundDefinition {
        id: id.into(),
        name: id.rsplit(':').next().unwrap().into(),
        description: "A deterministic test background.".into(),
        ui_order: 0,
        creation_grants: grants,
        talent_graph_ids: Vec::new(),
    }
}

fn base_class(
    id: &str,
    creation_grants: Vec<GrantDefinition>,
    level_grants: Vec<LevelGrantDefinition>,
) -> BaseClassDefinition {
    BaseClassDefinition {
        id: id.into(),
        name: id.rsplit(':').next().unwrap().into(),
        description: "A deterministic test class.".into(),
        ui_order: 0,
        creation_grants,
        level_grants,
        talent_graph_ids: Vec::new(),
    }
}

fn basic_definition() -> CharacterProgressionDefinition {
    CharacterProgressionDefinition {
        defaults: CharacterDefaultsDefinition {
            lineage_id: LINEAGE_ID.into(),
            background_id: None,
            base_class_id: CLASS_ID.into(),
        },
        level_thresholds: vec![
            LevelThresholdDefinition {
                level: 2,
                total_experience_required: 10,
                point_awards: BTreeMap::new(),
            },
            LevelThresholdDefinition {
                level: 3,
                total_experience_required: 30,
                point_awards: BTreeMap::new(),
            },
        ],
        lineages: vec![origin_lineage(LINEAGE_ID, Vec::new())],
        backgrounds: Vec::new(),
        base_classes: vec![base_class(
            CLASS_ID,
            vec![
                grant_attribute("max_health", 20),
                grant_attribute("strength", 5),
                grant_attribute("defence", 2),
                GrantDefinition::Skill {
                    skill_id: "combat".into(),
                    amount: 10,
                },
                GrantDefinition::Skill {
                    skill_id: "exploration".into(),
                    amount: 5,
                },
            ],
            Vec::new(),
        )],
        advanced_classes: Vec::new(),
        talent_graphs: Vec::new(),
    }
}

fn configure(
    world: &mut World,
    definition: CharacterProgressionDefinition,
    abilities: Vec<AbilityDefinition>,
) {
    world.configure_progression(definition, abilities).unwrap();
}

fn enter(world: &mut World) -> (SessionId, CharacterId) {
    let session = SessionId(1);
    let character_id = CharacterId(1);
    let gameplay = world
        .initial_gameplay(None, BackgroundSelection::Default, None)
        .unwrap();
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

fn spawn_reward(world: &mut World, reward: u64) {
    world
        .spawn_npc(NpcDefinition {
            stable_id: NPC_ID.into(),
            name: "Sentinel".into(),
            description: "A deterministic opponent.".into(),
            room_id: ROOM_ID.into(),
            level: 1,
            max_health: 1,
            strength: 1,
            defence: 0,
            experience_reward: reward,
            loot_item_ids: Vec::new(),
            respawn_ticks: 5,
        })
        .unwrap();
}

fn grant_attribute(id: &str, amount: u32) -> GrantDefinition {
    GrantDefinition::Attribute {
        attribute_id: id.into(),
        amount,
    }
}

fn public(predicate: RequirementPredicate) -> RequirementDefinition {
    RequirementDefinition {
        disclosure: RequirementDisclosure::Public,
        hint: None,
        predicate,
    }
}

#[test]
fn character_creation_composes_independent_choices_without_a_matrix() {
    let mut definition = basic_definition();
    definition.lineages = vec![
        origin_lineage("test:human", vec![grant_attribute("agility", 1)]),
        origin_lineage("test:elf", vec![grant_attribute("agility", 2)]),
    ];
    definition.backgrounds = vec![
        background(
            "test:scholar",
            vec![GrantDefinition::Skill {
                skill_id: "lore".into(),
                amount: 3,
            }],
        ),
        background(
            "test:scout",
            vec![GrantDefinition::Skill {
                skill_id: "survival".into(),
                amount: 4,
            }],
        ),
    ];
    definition.defaults.background_id = Some("test:scholar".into());
    definition.base_classes = vec![
        base_class(
            "test:warden",
            vec![
                grant_attribute("max_health", 20),
                grant_attribute("strength", 2),
            ],
            Vec::new(),
        ),
        base_class(
            "test:mystic",
            vec![
                grant_attribute("max_health", 20),
                grant_attribute("defence", 3),
            ],
            Vec::new(),
        ),
    ];
    let mut world = empty_world();
    configure(&mut world, definition, Vec::new());

    for lineage in ["test:human", "test:elf"] {
        for background in ["test:scholar", "test:scout"] {
            for class in ["test:warden", "test:mystic"] {
                let first = world
                    .initial_gameplay(
                        Some(lineage),
                        BackgroundSelection::Id(background),
                        Some(class),
                    )
                    .unwrap();
                let second = world
                    .initial_gameplay(
                        Some(lineage),
                        BackgroundSelection::Id(background),
                        Some(class),
                    )
                    .unwrap();
                assert_eq!(first, second);
                assert_eq!(first.lineage_id, lineage);
                assert_eq!(first.background_id.as_deref(), Some(background));
                assert_eq!(first.active_class_id, class);
                assert_eq!(first.class_history, [class]);
            }
        }
    }

    let catalog = world.progression_catalog();
    assert_eq!(catalog.lineages.len(), 2);
    assert_eq!(catalog.backgrounds.len(), 2);
    assert_eq!(catalog.base_classes.len(), 2);
    assert_eq!(
        world
            .initial_gameplay(None, BackgroundSelection::Default, None)
            .unwrap()
            .background_id
            .as_deref(),
        Some("test:scholar")
    );
    assert_eq!(
        world
            .initial_gameplay(None, BackgroundSelection::None, None)
            .unwrap()
            .background_id,
        None
    );
}

#[test]
fn character_creation_sheet_is_composed_from_authored_zero() {
    let mut definition = basic_definition();
    definition.lineages[0].creation_grants = vec![
        grant_attribute("max_health", 2),
        grant_attribute("strength", 1),
        GrantDefinition::Skill {
            skill_id: "combat".into(),
            amount: 2,
        },
    ];
    definition.backgrounds = vec![background(
        "test:guard",
        vec![
            grant_attribute("max_health", 3),
            grant_attribute("defence", 4),
            GrantDefinition::Skill {
                skill_id: "combat".into(),
                amount: 3,
            },
        ],
    )];
    definition.base_classes[0].creation_grants = vec![
        grant_attribute("max_health", 10),
        grant_attribute("strength", 2),
        GrantDefinition::Skill {
            skill_id: "exploration".into(),
            amount: 1,
        },
    ];
    let mut world = empty_world();
    configure(&mut world, definition, Vec::new());

    let sheet = world
        .initial_gameplay(
            Some(LINEAGE_ID),
            BackgroundSelection::Id("test:guard"),
            Some(CLASS_ID),
        )
        .unwrap();

    assert_eq!(sheet.vitals.max_health, 15);
    assert_eq!(sheet.vitals.health, 15);
    assert_eq!(sheet.attributes.strength, 3);
    assert_eq!(sheet.attributes.defence, 4);
    assert_eq!(sheet.skills.get("combat"), Some(&5));
    assert_eq!(sheet.skills.get("exploration"), Some(&1));
}

#[test]
fn base_class_must_author_positive_creation_health() {
    let mut definition = basic_definition();
    definition.base_classes[0].creation_grants.retain(|grant| {
        !matches!(grant, GrantDefinition::Attribute { attribute_id, .. }
                if attribute_id == "max_health")
    });
    let mut world = empty_world();

    assert!(matches!(
        world.configure_progression(definition, Vec::new()),
        Err(WorldError::InvalidProgressionDefinition)
    ));
}

#[test]
fn global_xp_advances_without_requiring_a_class_grant_at_every_level() {
    let mut definition = basic_definition();
    definition.base_classes[0].level_grants = vec![LevelGrantDefinition {
        level: 2,
        grants: vec![grant_attribute("strength", 2)],
    }];
    let mut world = empty_world();
    configure(&mut world, definition, Vec::new());
    spawn_reward(&mut world, 30);
    let (session, _) = enter(&mut world);

    world
        .apply(session, PlayerAction::Attack("Sentinel".into()))
        .unwrap();
    let events = world.advance_tick().unwrap();
    assert_eq!(
        events
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
    assert_eq!(sheet.attributes.strength, 7);
}

#[test]
fn capacity_grants_never_refill_health_or_resources() {
    let mut definition = basic_definition();
    definition.base_classes[0].creation_grants = vec![
        grant_attribute("max_health", 20),
        GrantDefinition::ResourceMax {
            resource_id: "mana".into(),
            amount: 10,
        },
    ];
    definition.base_classes[0].level_grants = vec![LevelGrantDefinition {
        level: 2,
        grants: vec![
            grant_attribute("max_health", 5),
            GrantDefinition::ResourceMax {
                resource_id: "mana".into(),
                amount: 5,
            },
        ],
    }];
    let mut world = empty_world();
    configure(&mut world, definition, Vec::new());
    spawn_reward(&mut world, 10);
    let (session, character_id) = enter(&mut world);
    let mut wounded = world.character_gameplay_snapshot(session).unwrap();
    wounded.vitals.health = 7;
    wounded.resources.get_mut("mana").unwrap().current = 3;
    world
        .restore_character_gameplay(character_id, wounded)
        .unwrap();

    world
        .apply(session, PlayerAction::Attack("Sentinel".into()))
        .unwrap();
    world.advance_tick().unwrap();
    let sheet = world.character_gameplay_snapshot(session).unwrap();
    assert_eq!(sheet.vitals.health, 7);
    assert_eq!(sheet.vitals.max_health, 25);
    assert_eq!(
        sheet.resources["mana"],
        ResourcePool {
            current: 3,
            max: 15
        }
    );
}

fn progression_with_talents() -> CharacterProgressionDefinition {
    let mut definition = basic_definition();
    definition.level_thresholds[0].point_awards = BTreeMap::from([("class".into(), 3)]);
    definition.base_classes[0].talent_graph_ids = vec!["test:warden-talents".into()];
    definition.talent_graphs = vec![TalentGraphDefinition {
        id: "test:warden-talents".into(),
        name: "Warden talents".into(),
        description: "A bounded talent graph.".into(),
        ui_order: 0,
        point_pool_id: "class".into(),
        nodes: vec![
            TalentNodeDefinition {
                id: "test:bulwark".into(),
                name: "Bulwark".into(),
                description: "Defensive training.".into(),
                ui_order: 0,
                exclusion_group: Some("stance".into()),
                ranks: vec![
                    TalentRankDefinition {
                        rank: 1,
                        cost: 1,
                        requirements: Vec::new(),
                        grants: vec![grant_attribute("defence", 1)],
                    },
                    TalentRankDefinition {
                        rank: 2,
                        cost: 2,
                        requirements: vec![public(RequirementPredicate::NodeRank {
                            graph_id: "test:warden-talents".into(),
                            node_id: "test:bulwark".into(),
                            minimum_rank: 1,
                        })],
                        grants: vec![grant_attribute("defence", 2)],
                    },
                ],
            },
            TalentNodeDefinition {
                id: "test:assault".into(),
                name: "Assault".into(),
                description: "Aggressive training.".into(),
                ui_order: 1,
                exclusion_group: Some("stance".into()),
                ranks: vec![TalentRankDefinition {
                    rank: 1,
                    cost: 1,
                    requirements: Vec::new(),
                    grants: vec![grant_attribute("strength", 1)],
                }],
            },
        ],
    }];
    definition
}

#[test]
fn talent_ranks_deduct_points_exactly_once_and_enforce_exclusion() {
    let definition = progression_with_talents();
    let mut world = empty_world();
    configure(&mut world, definition, Vec::new());
    spawn_reward(&mut world, 10);
    let (session, _) = enter(&mut world);
    world
        .apply(session, PlayerAction::Attack("Sentinel".into()))
        .unwrap();
    world.advance_tick().unwrap();

    world
        .apply(
            session,
            PlayerAction::AcquireTalent {
                graph_id: "test:warden-talents".into(),
                node_id: "test:bulwark".into(),
            },
        )
        .unwrap();
    assert!(matches!(
        world.apply(
            session,
            PlayerAction::AcquireTalent {
                graph_id: "test:warden-talents".into(),
                node_id: "test:assault".into(),
            }
        ),
        Err(WorldError::TalentUnavailable)
    ));
    world
        .apply(
            session,
            PlayerAction::AcquireTalent {
                graph_id: "test:warden-talents".into(),
                node_id: "test:bulwark".into(),
            },
        )
        .unwrap();
    let sheet = world.character_gameplay_snapshot(session).unwrap();
    assert_eq!(sheet.progression_points["class"], 0);
    assert_eq!(
        sheet.talent_node_ranks["test:warden-talents"]["test:bulwark"],
        2
    );
    assert_eq!(sheet.attributes.defence, 5);
    assert!(matches!(
        world.apply(
            session,
            PlayerAction::AcquireTalent {
                graph_id: "test:warden-talents".into(),
                node_id: "test:bulwark".into(),
            }
        ),
        Err(WorldError::TalentUnavailable)
    ));
}

fn advanced_class(
    id: &str,
    from: &[&str],
    requirements: Vec<RequirementDefinition>,
) -> AdvancedClassDefinition {
    AdvancedClassDefinition {
        id: id.into(),
        name: id.rsplit(':').next().unwrap().into(),
        description: "An advanced deterministic class.".into(),
        ui_order: 0,
        from_class_ids: from.iter().map(|value| (*value).into()).collect(),
        exclusive_group: None,
        requirements,
        promotion_grants: vec![grant_attribute("strength", 2)],
        level_grants: Vec::new(),
        talent_graph_ids: Vec::new(),
    }
}

#[test]
fn promotion_is_atomic_and_supports_cross_entry_advanced_classes() {
    let mut definition = basic_definition();
    definition.base_classes.push(base_class(
        "test:mage",
        vec![grant_attribute("max_health", 16)],
        Vec::new(),
    ));
    definition.advanced_classes = vec![advanced_class(
        "test:spellblade",
        &[CLASS_ID, "test:mage"],
        vec![
            public(RequirementPredicate::Level { minimum: 2 }),
            public(RequirementPredicate::Skill {
                skill_id: "arcana".into(),
                minimum: 3,
            }),
            public(RequirementPredicate::ReceivedSignal {
                signal_id: "test:blade-oath".into(),
            }),
        ],
    )];
    let mut world = empty_world();
    configure(&mut world, definition, Vec::new());
    let (session, character_id) = enter(&mut world);
    let before = world.character_gameplay_snapshot(session).unwrap();
    assert!(matches!(
        world.apply(
            session,
            PlayerAction::Promote {
                class_id: "test:spellblade".into(),
            }
        ),
        Err(WorldError::PromotionUnavailable)
    ));
    assert_eq!(world.character_gameplay_snapshot(session).unwrap(), before);

    let mut eligible = before;
    eligible.level = 2;
    eligible.experience = 10;
    eligible.skills.insert("arcana".into(), 3);
    world
        .restore_character_gameplay(character_id, eligible)
        .unwrap();
    world
        .receive_progression_signal(character_id, "test:blade-oath")
        .unwrap();
    let outcome = world
        .apply(
            session,
            PlayerAction::Promote {
                class_id: "test:spellblade".into(),
            },
        )
        .unwrap();
    assert!(matches!(
        outcome.domain_events.as_slice(),
        [DomainEvent::Gameplay {
            event: GameplayEvent::CharacterPromoted { to_class_id, .. }
        }] if to_class_id == "test:spellblade"
    ));
    let promoted = world.character_gameplay_snapshot(session).unwrap();
    assert_eq!(promoted.active_class_id, "test:spellblade");
    assert_eq!(promoted.class_history, [CLASS_ID, "test:spellblade"]);
    assert_eq!(promoted.level, 2);
    assert_eq!(promoted.experience, 10);
    assert_eq!(promoted.attributes.strength, 7);
    let view = world.player_state_view(session).unwrap();
    assert_eq!(view.character.base_class.id, CLASS_ID);
    assert_eq!(view.character.active_class.id, "test:spellblade");
}

#[test]
fn delayed_promotion_applies_reached_class_milestones_exactly_once() {
    let mut definition = basic_definition();
    definition
        .level_thresholds
        .extend((4..=8).map(|level| LevelThresholdDefinition {
            level,
            total_experience_required: u64::from(level) * 10,
            point_awards: BTreeMap::new(),
        }));
    let mut advanced = advanced_class(
        "test:bastion",
        &[CLASS_ID],
        vec![public(RequirementPredicate::Level { minimum: 8 })],
    );
    advanced.level_grants = vec![LevelGrantDefinition {
        level: 7,
        grants: vec![grant_attribute("strength", 3)],
    }];
    definition.advanced_classes = vec![advanced];
    let mut world = empty_world();
    configure(&mut world, definition, Vec::new());
    let (session, character_id) = enter(&mut world);
    let mut eligible = world.character_gameplay_snapshot(session).unwrap();
    eligible.level = 8;
    eligible.experience = 80;
    world
        .restore_character_gameplay(character_id, eligible)
        .unwrap();

    world
        .apply(
            session,
            PlayerAction::Promote {
                class_id: "test:bastion".into(),
            },
        )
        .unwrap();
    let promoted = world.character_gameplay_snapshot(session).unwrap();
    assert_eq!(promoted.attributes.strength, 10);
    assert_eq!(promoted.class_history, [CLASS_ID, "test:bastion"]);

    assert!(matches!(
        world.apply(
            session,
            PlayerAction::Promote {
                class_id: "test:bastion".into(),
            }
        ),
        Err(WorldError::PromotionUnavailable)
    ));
    assert_eq!(
        world.character_gameplay_snapshot(session).unwrap(),
        promoted
    );
}

#[test]
fn hidden_requirement_projects_only_its_safe_hint() {
    let secret = "test:never-leak-this-flag";
    let mut definition = basic_definition();
    definition.advanced_classes = vec![advanced_class(
        "test:hidden-path",
        &[CLASS_ID],
        vec![RequirementDefinition {
            disclosure: RequirementDisclosure::Hidden,
            hint: Some("Earn the trust of the silent order.".into()),
            predicate: RequirementPredicate::CharacterFlag {
                flag_id: secret.into(),
                equals: true,
            },
        }],
    )];
    let mut world = empty_world();
    configure(&mut world, definition, Vec::new());
    let (session, _) = enter(&mut world);
    let json = serde_json::to_string(&world.player_state_view(session).unwrap()).unwrap();
    assert!(json.contains("Earn the trust of the silent order."));
    assert!(!json.contains(secret));
}

#[test]
fn nested_requirement_text_exposes_safe_branch_details_only() {
    let secret = "test:spellblade-secret-flag";
    let mut definition = basic_definition();
    definition.advanced_classes = vec![advanced_class(
        "test:spellblade",
        &[CLASS_ID],
        vec![public(RequirementPredicate::Any {
            requirements: vec![
                public(RequirementPredicate::Level { minimum: 8 }),
                RequirementDefinition {
                    disclosure: RequirementDisclosure::Hidden,
                    hint: Some("Hear the veiled call.".into()),
                    predicate: RequirementPredicate::CharacterFlag {
                        flag_id: secret.into(),
                        equals: true,
                    },
                },
            ],
        })],
    )];
    let mut world = empty_world();
    configure(&mut world, definition, Vec::new());
    let (session, _) = enter(&mut world);

    let text = &world
        .player_state_view(session)
        .unwrap()
        .character
        .promotions[0]
        .requirements[0]
        .text;
    assert!(text.contains("One of:"));
    assert!(text.contains("Reach level 8."));
    assert!(text.contains("Hear the veiled call."));
    assert!(!text.contains(secret));
    assert!(text.len() <= 1_024);
}

#[test]
fn restore_rejects_unknown_progression_references() {
    let mut world = empty_world();
    configure(&mut world, basic_definition(), Vec::new());
    for invalid in [
        {
            let mut value = world
                .initial_gameplay(None, BackgroundSelection::Default, None)
                .unwrap();
            value.lineage_id = "test:missing".into();
            value
        },
        {
            let mut value = world
                .initial_gameplay(None, BackgroundSelection::Default, None)
                .unwrap();
            value.active_class_id = "test:missing".into();
            value.class_history.push("test:missing".into());
            value
        },
        {
            let mut value = world
                .initial_gameplay(None, BackgroundSelection::Default, None)
                .unwrap();
            value.talent_node_ranks.insert(
                "test:missing".into(),
                BTreeMap::from([("test:nope".into(), 1)]),
            );
            value
        },
    ] {
        assert!(matches!(
            world.restore_character_gameplay(CharacterId(1), invalid),
            Err(WorldError::InvalidGameplayDefinition)
        ));
    }
}

#[test]
fn snapshot_roundtrip_is_bounded_and_lossless() {
    let mut snapshot = CharacterGameplaySnapshot::default();
    snapshot.class_history = (0..8).map(|index| format!("test:class-{index}")).collect();
    snapshot.base_class_id = snapshot.class_history[0].clone();
    snapshot.active_class_id = snapshot.class_history[7].clone();
    snapshot.attributes.additional = (0..62)
        .map(|index| (format!("attribute-{index}"), 999_999))
        .collect();
    snapshot.skills = (0..64)
        .map(|index| (format!("skill-{index}"), 1_000))
        .collect();
    snapshot.resources = (0..16)
        .map(|index| {
            (
                format!("resource-{index}"),
                ResourcePool {
                    current: 999_999,
                    max: 1_000_000,
                },
            )
        })
        .collect();
    snapshot.resource_regeneration = (0..16)
        .map(|index| (format!("resource-{index}"), 1_000_000))
        .collect();
    snapshot.ability_ids = (0..64)
        .map(|index| format!("test:ability-{index}"))
        .collect();
    snapshot.ability_cooldowns = snapshot
        .ability_ids
        .iter()
        .cloned()
        .map(|id| (id, 10_000))
        .collect();
    snapshot.statuses = (0..32)
        .map(|index| ActiveStatusEffect {
            source_ability_id: format!("test:ability-{index}"),
            source_character_id: Some(CharacterId(9_999)),
            source_npc_id: Some(format!("test:npc-{index}")),
            kind: ActiveStatusKind::Stun,
            magnitude: 0,
            remaining_ticks: 10_000,
            attribute: None,
            damage_type: None,
        })
        .collect();
    snapshot.effects = (0..16)
        .map(|_| ActiveEffect {
            kind: EffectKind::Strength,
            magnitude: 100_000,
            remaining_ticks: 10_000,
        })
        .collect();
    snapshot.progression_points = (0..4).map(|index| (format!("pool-{index}"), 100)).collect();
    snapshot.talent_node_ranks = (0..16)
        .map(|graph| {
            (
                format!("test:graph-{graph}"),
                (0..2)
                    .map(|node| (format!("test:node-{graph}-{node}"), 5))
                    .collect(),
            )
        })
        .collect();
    snapshot.tags = (0..64)
        .map(|index| format!("test:tag-{index}-{}", "x".repeat(64)))
        .collect();
    snapshot.received_signals = (0..64)
        .map(|index| format!("test:signal-{index}-{}", "x".repeat(60)))
        .collect::<BTreeSet<_>>();
    snapshot.script_flags = (0..64)
        .map(|index| format!("test:flag-{index}-{}", "x".repeat(64)))
        .collect();
    let bytes = serde_json::to_vec(&snapshot).unwrap();
    assert!(bytes.len() < 65_536, "snapshot used {} bytes", bytes.len());
    assert_eq!(
        serde_json::from_slice::<CharacterGameplaySnapshot>(&bytes).unwrap(),
        snapshot
    );
}

#[test]
fn parser_exposes_explicit_promotion_and_talent_actions() {
    assert_eq!(
        parse_action("promote test:spellblade").unwrap(),
        PlayerAction::Promote {
            class_id: "test:spellblade".into()
        }
    );
    assert_eq!(
        parse_action("talent learn test:warden-talents test:bulwark").unwrap(),
        PlayerAction::AcquireTalent {
            graph_id: "test:warden-talents".into(),
            node_id: "test:bulwark".into(),
        }
    );
}

#[test]
fn authored_ability_grants_remain_usable() {
    let strike = AbilityDefinition {
        id: "test:strike".into(),
        name: "Strike".into(),
        description: "A deterministic attack.".into(),
        category: AbilityCategory::Martial,
        resource_costs: BTreeMap::new(),
        cooldown_ticks: 0,
        targeting: AbilityTargeting::SingleEnemy,
        damage: Some(AbilityDamageDefinition {
            target: AbilityEffectTarget::Target,
            damage_type: DamageType::Physical,
            base_amount: 2,
            scaling_attribute: None,
            scaling_percent: 0,
        }),
        heal: None,
        status_effects: Vec::new(),
    };
    let mut definition = basic_definition();
    definition.base_classes[0].creation_grants = vec![
        grant_attribute("max_health", 20),
        GrantDefinition::Ability {
            ability_id: strike.id.clone(),
        },
    ];
    let mut world = empty_world();
    configure(&mut world, definition, vec![strike]);
    spawn_reward(&mut world, 0);
    let (session, _) = enter(&mut world);
    world
        .apply(
            session,
            PlayerAction::Ability {
                ability_id: "test:strike".into(),
                target: Some("Sentinel".into()),
            },
        )
        .unwrap();
}
