use super::*;

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
