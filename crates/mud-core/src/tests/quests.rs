//! Data-driven quests, commodities and shops (WorldPack v10, ADR 0032).

use super::*;

fn npc(stable_id: &str, name: &str, room_id: &str, health: i32) -> NpcDefinition {
    NpcDefinition {
        stable_id: stable_id.into(),
        name: name.into(),
        description: format!("{name} waits here."),
        room_id: room_id.into(),
        level: 1,
        max_health: health,
        strength: 1,
        defence: 0,
        experience_reward: 0,
        loot_item_ids: Vec::new(),
        respawn_ticks: 5,
    }
}

fn errand_content() -> AdventureContent {
    AdventureContent {
        commodities: vec![CommodityDefinition {
            id: "game:salt".into(),
            label: "sea salt".into(),
            max_owned: 2,
        }],
        quests: vec![
            QuestDefinition {
                id: "game:errand".into(),
                name: "Herb Errand".into(),
                summary: "Gather herbs and clear the yard.".into(),
                aliases: vec!["errand".into()],
                prerequisite: Some(QuestRequirement::HasQuestState(
                    "game:intro:completed".into(),
                )),
                repeatable: true,
                stages: vec![
                    QuestStageDefinition {
                        id: "approach".into(),
                        summary: Some("Walk to the arena.".into()),
                        objectives: vec![QuestObjective::VisitRoom {
                            room_id: "game:arena".into(),
                        }],
                    },
                    QuestStageDefinition {
                        id: "gather".into(),
                        summary: None,
                        objectives: vec![
                            QuestObjective::CollectItem {
                                item_prototype_id: "game:herb".into(),
                                count: 2,
                            },
                            QuestObjective::DefeatNpc {
                                npc_ids: vec!["game:rat".into()],
                                count: 1,
                            },
                        ],
                    },
                ],
                reward_coins: 10,
                reward_experience: 20,
            },
            QuestDefinition {
                id: "game:intro".into(),
                name: "Introductions".into(),
                summary: "Talk with the sage.".into(),
                aliases: Vec::new(),
                prerequisite: None,
                repeatable: false,
                stages: vec![QuestStageDefinition {
                    id: "greet".into(),
                    summary: None,
                    objectives: vec![QuestObjective::TalkToNpc {
                        npc_ids: vec!["game:sage".into()],
                    }],
                }],
                reward_coins: 5,
                reward_experience: 0,
            },
        ],
        shops: vec![ShopDefinition {
            id: "game:stall".into(),
            name: "Salt Stall".into(),
            room_id: "game:start".into(),
            keeper_npc_id: Some("game:sage".into()),
            stock: vec![ShopStockDefinition {
                stock_id: "game:stall-salt".into(),
                commodity_id: "game:salt".into(),
                buy_price: 3,
                sell_price: 1,
                initial_stock: 1,
                max_stock: 4,
                restock: ShopRestock::Interval {
                    every_ticks: 2,
                    amount: 1,
                    ceiling: 2,
                },
            }],
        }],
    }
}

fn quest_world(content: AdventureContent) -> (World, SessionId) {
    let mut world = World::from_definition(WorldDefinition {
        start_room_id: "game:start".into(),
        item_prototypes: vec![ItemPrototypeDefinition {
            stack_policy: ItemStackPolicy::Fungible,
            max_stack_units: 10,
            ..item_prototype("game:herb", "bitter herb", "A fragrant herb.", &["herb"])
        }],
        initial_item_instances: vec![
            item_instance(
                "game:herb-1",
                "game:herb",
                InitialItemLocation::Room("game:arena".into()),
            ),
            item_instance(
                "game:herb-2",
                "game:herb",
                InitialItemLocation::Room("game:arena".into()),
            ),
        ],
        rooms: vec![
            prepared_room("game:start", [(Direction::North, "game:arena")]),
            prepared_room("game:arena", [(Direction::South, "game:start")]),
        ],
    })
    .unwrap();
    world.configure_adventure_content(content).unwrap();
    world
        .spawn_npc(npc("game:sage", "patient sage", "game:start", 50))
        .unwrap();
    world
        .spawn_npc(npc("game:rat", "yard rat", "game:arena", 1))
        .unwrap();
    let session = SessionId(77);
    world
        .connect(
            session,
            CharacterSnapshot {
                id: CharacterId(77),
                account_id: AccountId(7),
                name: "Seeker".into(),
                room_id: "game:start".into(),
                item_locations: BTreeMap::new(),
            },
        )
        .unwrap();
    (world, session)
}

fn quest_action(world: &mut World, session: SessionId, action: QuestAction) -> ActionOutcome {
    world.apply(session, PlayerAction::Quest(action)).unwrap()
}

fn quest_events(outcome: &ActionOutcome) -> Vec<String> {
    outcome
        .domain_events
        .iter()
        .filter_map(|event| match event {
            DomainEvent::Adventure {
                event: AdventureEvent::QuestChanged { summary, .. },
            } => Some(summary.clone()),
            _ => None,
        })
        .collect()
}

#[test]
fn legacy_default_content_matches_the_formerly_hardcoded_quest_and_market() {
    let world = World::demo();
    let content = world.adventure_content();
    assert_eq!(*content, AdventureContent::legacy("demo:lantern-square"));
    assert_eq!(content.quests.len(), 1);
    assert_eq!(content.quests[0].id, "first-hunt");
    assert_eq!(
        world.runtime_snapshot().adventure.shop_stock,
        AdventureRuntimeSnapshot::default().shop_stock
    );
}

#[test]
fn legacy_first_hunt_outputs_are_byte_identical() {
    let (mut world, session) = connected_world();
    let list = quest_action(&mut world, session, QuestAction::List);
    assert_eq!(
        list.output.as_deref(),
        Some(
            "Available quest: first-hunt — defeat one hostile creature; reward 25 coin and 50 experience.\r\n"
        )
    );
    let status = quest_action(&mut world, session, QuestAction::Status);
    assert_eq!(
        status.output.as_deref(),
        Some("The first-hunt quest has not been accepted.\r\n")
    );
    let accepted = quest_action(
        &mut world,
        session,
        QuestAction::Accept("First Hunt".into()),
    );
    assert_eq!(
        accepted.output.as_deref(),
        Some("You accept First Hunt.\r\n")
    );
    assert_eq!(quest_events(&accepted), vec!["First Hunt accepted."]);
    let status = quest_action(&mut world, session, QuestAction::Status);
    assert_eq!(
        status.output.as_deref(),
        Some("First Hunt: Active (0/1).\r\n")
    );
    let view = world.player_state_view(session).unwrap();
    assert_eq!(view.quests.len(), 1);
    assert_eq!(view.quests[0].id, "first-hunt");
    assert_eq!(view.quests[0].label, "First Hunt");
    assert_eq!(view.quests[0].status, "active");
    assert_eq!(view.quests[0].target, 1);
    let shop = world
        .apply(session, PlayerAction::Economy(EconomyAction::Shop))
        .unwrap();
    assert_eq!(
        shop.output.as_deref(),
        Some(
            "Lantern Market:\r\n- healing tonic: buy 12, sell 6, stock 20\r\n- trail ration: buy 8, sell 4, stock 20\r\n"
        )
    );
    // The legacy JSON shape carries no WorldPack v10 stage state.
    let json = serde_json::to_value(world.character_gameplay_snapshot(session).unwrap()).unwrap();
    assert!(json.get("questStages").is_none());
}

#[test]
fn multi_stage_quest_progresses_from_data_with_prerequisites_and_repeats() {
    let (mut world, session) = quest_world(errand_content());
    let list = quest_action(&mut world, session, QuestAction::List);
    assert_eq!(
        list.output.as_deref(),
        Some("Available quest: game:intro — talk with the sage.\r\n")
    );
    assert!(matches!(
        world.apply(
            session,
            PlayerAction::Quest(QuestAction::Accept("errand".into()))
        ),
        Err(WorldError::QuestUnavailable)
    ));

    quest_action(
        &mut world,
        session,
        QuestAction::Accept("game:intro".into()),
    );
    let talked = world
        .apply(session, parse_action("talk to sage").unwrap())
        .unwrap();
    assert_eq!(
        talked.output.as_deref(),
        Some("You talk with patient sage.\r\n")
    );
    assert_eq!(
        quest_events(&talked),
        vec!["Introductions is ready to claim."]
    );
    let completed = quest_action(
        &mut world,
        session,
        QuestAction::Complete("game:intro".into()),
    );
    assert_eq!(
        completed.output.as_deref(),
        Some("Introductions complete: you receive 5 coin.\r\n")
    );
    assert!(matches!(
        world.apply(
            session,
            PlayerAction::Quest(QuestAction::Complete("game:intro".into()))
        ),
        Err(WorldError::QuestUnavailable)
    ));
    assert!(matches!(
        world.apply(
            session,
            PlayerAction::Quest(QuestAction::Accept("game:intro".into()))
        ),
        Err(WorldError::QuestUnavailable)
    ));

    quest_action(&mut world, session, QuestAction::Accept("errand".into()));
    let context = world.authorized_runtime_context(session).unwrap();
    assert!(context.actor_quest_states.contains("game:errand:active"));
    assert!(
        context
            .actor_quest_states
            .contains("game:errand:stage:approach")
    );
    let view = world.player_state_view(session).unwrap();
    let errand = view
        .quests
        .iter()
        .find(|quest| quest.id == "game:errand")
        .unwrap();
    assert_eq!(errand.summary, "Walk to the arena.");

    let moved = world
        .apply(session, PlayerAction::Move(Direction::North))
        .unwrap();
    assert_eq!(
        quest_events(&moved),
        vec!["Herb Errand advances to a new stage."]
    );
    let gameplay = world.character_gameplay_snapshot(session).unwrap();
    assert_eq!(
        gameplay.quest_stages["game:errand"],
        QuestStageProgress {
            stage: 1,
            objectives: vec![0, 0],
            completions: 0,
        }
    );
    assert_eq!(gameplay.quests["game:errand"].target, 3);

    let taken = world
        .apply(
            session,
            PlayerAction::Take(ItemSelection {
                query: "herb".into(),
                quantity: 2,
            }),
        )
        .unwrap();
    assert_eq!(quest_events(&taken), vec!["Herb Errand progresses."]);
    let gameplay = world.character_gameplay_snapshot(session).unwrap();
    assert_eq!(gameplay.quest_stages["game:errand"].objectives, vec![2, 0]);
    assert_eq!(gameplay.quests["game:errand"].current, 2);

    world
        .apply(session, PlayerAction::Attack("rat".into()))
        .unwrap();
    let mut ready = false;
    for _ in 0..10 {
        let events = world.advance_tick().unwrap();
        if events.iter().any(|event| {
            matches!(
                event,
                DomainEvent::Adventure {
                    event: AdventureEvent::QuestChanged { summary, .. }
                } if summary == "Herb Errand is ready to claim."
            )
        }) {
            ready = true;
            break;
        }
    }
    assert!(ready);
    let before = world.character_gameplay_snapshot(session).unwrap();
    assert_eq!(
        before.quests["game:errand"].status,
        QuestStatus::ReadyToClaim
    );
    let claimed = quest_action(&mut world, session, QuestAction::Complete("errand".into()));
    assert_eq!(
        claimed.output.as_deref(),
        Some("Herb Errand complete: you receive 10 coin and 20 experience.\r\n")
    );
    let after = world.character_gameplay_snapshot(session).unwrap();
    assert_eq!(after.coins, before.coins + 10);
    assert_eq!(after.experience, before.experience + 20);
    assert_eq!(after.quest_stages["game:errand"].completions, 1);
    assert!(
        world
            .validate_character_gameplay_snapshot(CharacterId(77), &after)
            .is_ok()
    );

    // Repeatable: a new run restarts at the first stage, keeping the count.
    quest_action(&mut world, session, QuestAction::Accept("errand".into()));
    let restarted = world.character_gameplay_snapshot(session).unwrap();
    // Already standing in the arena, the visit completes immediately.
    assert_eq!(restarted.quest_stages["game:errand"].stage, 1);
    assert_eq!(restarted.quest_stages["game:errand"].completions, 1);
}

#[test]
fn persisted_quest_state_round_trips_and_fails_closed() {
    let (mut world, session) = quest_world(errand_content());
    quest_action(
        &mut world,
        session,
        QuestAction::Accept("game:intro".into()),
    );
    world
        .apply(session, parse_action("talk sage").unwrap())
        .unwrap();
    quest_action(
        &mut world,
        session,
        QuestAction::Complete("game:intro".into()),
    );
    quest_action(&mut world, session, QuestAction::Accept("errand".into()));
    world
        .apply(session, PlayerAction::Move(Direction::North))
        .unwrap();
    let snapshot = world.character_gameplay_snapshot(session).unwrap();
    let json = serde_json::to_string(&snapshot).unwrap();
    assert!(json.contains("\"questStages\""));
    let decoded: CharacterGameplaySnapshot = serde_json::from_str(&json).unwrap();
    assert_eq!(decoded, snapshot);
    world
        .validate_character_gameplay_snapshot(CharacterId(77), &decoded)
        .unwrap();

    let mut out_of_range = decoded.clone();
    out_of_range
        .quest_stages
        .get_mut("game:errand")
        .unwrap()
        .stage = 9;
    let mut wrong_counts = decoded.clone();
    wrong_counts
        .quest_stages
        .get_mut("game:errand")
        .unwrap()
        .objectives = vec![5, 0];
    let mut unknown = decoded.clone();
    let progress = unknown.quests["game:errand"].clone();
    unknown.quests.insert("game:missing".into(), progress);
    let mut orphan = decoded.clone();
    orphan
        .quest_stages
        .insert("game:intro".into(), QuestStageProgress::default());
    let mut once_repeated = decoded;
    once_repeated.quest_stages.insert(
        "game:intro".into(),
        QuestStageProgress {
            stage: 0,
            objectives: Vec::new(),
            completions: 2,
        },
    );
    for invalid in [out_of_range, wrong_counts, unknown, orphan, once_repeated] {
        assert!(
            world
                .validate_character_gameplay_snapshot(CharacterId(77), &invalid)
                .is_err()
        );
    }
}

#[test]
fn shop_keeper_binding_stock_limits_and_restock_are_data_driven() {
    let (mut world, session) = quest_world(errand_content());
    assert_eq!(
        world.runtime_snapshot().adventure.shop_stock,
        BTreeMap::from([("game:stall-salt".to_owned(), 1)])
    );
    let shop = world
        .apply(session, PlayerAction::Economy(EconomyAction::Shop))
        .unwrap();
    assert_eq!(
        shop.output.as_deref(),
        Some("Salt Stall:\r\n- sea salt: buy 3, sell 1, stock 1\r\n")
    );
    world
        .apply(
            session,
            PlayerAction::Economy(EconomyAction::Buy("sea".into())),
        )
        .unwrap();
    assert!(matches!(
        world.apply(
            session,
            PlayerAction::Economy(EconomyAction::Buy("sea".into()))
        ),
        Err(WorldError::ShopUnavailable)
    ));
    // Interval restock: every two ticks add one, never above the ceiling.
    for _ in 0..6 {
        world.advance_tick().unwrap();
    }
    assert_eq!(
        world.runtime_snapshot().adventure.shop_stock["game:stall-salt"],
        2
    );
    world
        .apply(
            session,
            PlayerAction::Economy(EconomyAction::Buy("sea".into())),
        )
        .unwrap();
    // maxOwned is two.
    assert!(matches!(
        world.apply(
            session,
            PlayerAction::Economy(EconomyAction::Buy("sea".into()))
        ),
        Err(WorldError::ShopUnavailable)
    ));
    // No shop in the arena, and the stall closes without its keeper.
    world
        .apply(session, PlayerAction::Move(Direction::North))
        .unwrap();
    assert!(matches!(
        world.apply(session, PlayerAction::Economy(EconomyAction::Shop)),
        Err(WorldError::ShopUnavailable)
    ));
    assert!(
        !world
            .player_state_view(session)
            .unwrap()
            .economy
            .shop_available
    );

    // Restored stock must match the configured entries exactly.
    let mut snapshot = world.runtime_snapshot();
    snapshot.adventure.shop_stock.insert("tonic".into(), 1);
    assert!(matches!(
        world.restore_runtime_snapshot(snapshot),
        Err(WorldError::InvalidRuntimeSnapshot)
    ));
    let mut snapshot = world.runtime_snapshot();
    snapshot
        .adventure
        .shop_stock
        .insert("game:stall-salt".into(), 5);
    assert!(world.restore_runtime_snapshot(snapshot).is_err());
}

#[test]
fn adventure_configuration_is_bounded_and_fails_closed() {
    let (mut world, _) = connected_world();
    assert!(matches!(
        world.configure_adventure_content(AdventureContent::default()),
        Err(WorldError::InvalidGameplayDefinition)
    ));

    let mut world = World::demo();
    let mut too_many = errand_content();
    too_many.quests[0].stages = (0..=MAX_ADVENTURE_QUEST_STAGES)
        .map(|index| QuestStageDefinition {
            id: format!("stage-{index}"),
            summary: None,
            objectives: vec![QuestObjective::VisitRoom {
                room_id: "game:arena".into(),
            }],
        })
        .collect();
    let mut sell_above_buy = errand_content();
    sell_above_buy.shops[0].stock[0].sell_price = 4;
    let mut zero_count = errand_content();
    zero_count.quests[0].stages[1].objectives[0] = QuestObjective::CollectItem {
        item_prototype_id: "game:herb".into(),
        count: 0,
    };
    let mut unknown_commodity = errand_content();
    unknown_commodity.shops[0].stock[0].commodity_id = "game:pepper".into();
    let mut ambiguous_alias = errand_content();
    ambiguous_alias.quests[1].aliases = vec!["errand".into()];
    for invalid in [
        too_many,
        sell_above_buy,
        zero_count,
        unknown_commodity,
        ambiguous_alias,
    ] {
        assert!(matches!(
            world.configure_adventure_content(invalid),
            Err(WorldError::InvalidGameplayDefinition)
        ));
    }
    world.configure_adventure_content(errand_content()).unwrap();
}

#[test]
fn talk_requires_a_visible_npc_and_parses_both_forms() {
    assert_eq!(
        parse_action("talk sage"),
        Ok(PlayerAction::Quest(QuestAction::Talk("sage".into())))
    );
    assert_eq!(
        parse_action("Talk to the sage"),
        Ok(PlayerAction::Quest(QuestAction::Talk("the sage".into())))
    );
    assert!(parse_action("talk").is_err());
    assert!(parse_action("talk to").is_err());
    let (mut world, session) = quest_world(errand_content());
    assert!(matches!(
        world.apply(session, parse_action("talk rat").unwrap()),
        Err(WorldError::NpcNotFound)
    ));
    let outcome = world
        .apply(session, parse_action("talk sage").unwrap())
        .unwrap();
    assert!(outcome.domain_events.is_empty());
}
