use std::collections::HashSet;

use tiamud_core::{
    AccountId, ActiveEffect, CharacterId, CharacterSnapshot, Direction, EconomyAction, EffectKind,
    EquipmentSlot, InitialItemInstanceDefinition, InitialItemLocation, ItemGameplayDefinition,
    ItemInstanceId, ItemPrototypeDefinition, ItemRarity, ItemStackPolicy, ItemTransferPolicy,
    PlayerAction, RoomDefinition, RuntimeItemLocation, SessionId, World, WorldDefinition,
};

const ITEM_COUNT: usize = 3;

fn property_world() -> (World, SessionId) {
    let prototype =
        |id: &str, name: &str, description: &str, alias: &str| ItemPrototypeDefinition {
            prototype_id: id.into(),
            name: name.into(),
            description: description.into(),
            aliases: vec![alias.into()],
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
        };
    let mut world = World::from_definition(WorldDefinition {
        start_room_id: "property:market".into(),
        item_prototypes: vec![
            prototype(
                "property:sword",
                "test sword",
                "A sword used to exercise equipment invariants.",
                "sword",
            ),
            prototype(
                "property:charm",
                "test charm",
                "A reusable charm used to exercise effect bounds.",
                "charm",
            ),
            prototype(
                "property:potion",
                "test potion",
                "A consumable used to exercise item transitions.",
                "potion",
            ),
        ],
        initial_item_instances: ["property:sword", "property:charm", "property:potion"]
            .into_iter()
            .map(|id| InitialItemInstanceDefinition {
                instance_id: ItemInstanceId(id.into()),
                prototype_id: id.into(),
                location: InitialItemLocation::Room("property:market".into()),
            })
            .collect(),
        rooms: vec![RoomDefinition {
            stable_id: "property:market".into(),
            title: "Property Market".into(),
            prose: vec!["A deterministic test market.".into()],
            image: None,
            audio: None,
            exits: Vec::new(),
            item_ids: vec![
                "property:sword".into(),
                "property:charm".into(),
                "property:potion".into(),
            ],
        }],
    })
    .unwrap();
    world
        .configure_item_gameplay(ItemGameplayDefinition {
            item_id: "property:sword".into(),
            equipment_slot: Some(EquipmentSlot::Weapon),
            attack_bonus: 3,
            heal_amount: 0,
            consumable: false,
            effect: None,
        })
        .unwrap();
    world
        .configure_item_gameplay(ItemGameplayDefinition {
            item_id: "property:charm".into(),
            equipment_slot: None,
            attack_bonus: 0,
            heal_amount: 0,
            consumable: false,
            effect: Some(ActiveEffect {
                kind: EffectKind::Strength,
                magnitude: 1,
                remaining_ticks: 3,
            }),
        })
        .unwrap();
    world
        .configure_item_gameplay(ItemGameplayDefinition {
            item_id: "property:potion".into(),
            equipment_slot: None,
            attack_bonus: 0,
            heal_amount: 5,
            consumable: true,
            effect: Some(ActiveEffect {
                kind: EffectKind::Regeneration,
                magnitude: 1,
                remaining_ticks: 2,
            }),
        })
        .unwrap();

    let session = SessionId(1);
    world
        .connect(
            session,
            CharacterSnapshot {
                id: CharacterId(1),
                account_id: AccountId(1),
                name: "Invariant".into(),
                room_id: "property:market".into(),
                item_locations: Default::default(),
            },
        )
        .unwrap();
    (world, session)
}

fn next_random(seed: &mut u64) -> u64 {
    *seed ^= *seed << 13;
    *seed ^= *seed >> 7;
    *seed ^= *seed << 17;
    *seed
}

fn assert_domain_invariants(world: &World, session: SessionId) {
    let gameplay = world.character_gameplay_snapshot(session).unwrap();
    assert!((0..=gameplay.vitals.max_health).contains(&gameplay.vitals.health));
    assert!(gameplay.effects.len() <= 32);
    assert!(
        gameplay
            .effects
            .iter()
            .all(|effect| effect.magnitude > 0 && effect.remaining_ticks > 0)
    );
    assert!(
        gameplay
            .commodities
            .values()
            .all(|quantity| *quantity <= 99)
    );
    assert!(gameplay.coins <= 40);

    let equipped = gameplay.equipment.values().collect::<HashSet<_>>();
    assert_eq!(equipped.len(), gameplay.equipment.len());

    let runtime = world.runtime_snapshot();
    assert_eq!(runtime.item_instances.len(), ITEM_COUNT);
    for (slot, item_id) in &gameplay.equipment {
        assert_eq!(
            runtime
                .item_instances
                .get(item_id)
                .map(|instance| &instance.location),
            Some(&RuntimeItemLocation::Equipped {
                character_id: CharacterId(1),
                slot: *slot,
            })
        );
    }
    for (commodity, initial_stock) in [("tonic", 20_u32), ("trail-ration", 20_u32)] {
        let stock = runtime.adventure.shop_stock[commodity];
        let owned = gameplay.commodities.get(commodity).copied().unwrap_or(0);
        assert_eq!(stock + owned, initial_stock);
    }

    let player = world.player_state_view(session).unwrap();
    assert_eq!(player.character.health, gameplay.vitals.health);
    assert_eq!(player.character.max_health, gameplay.vitals.max_health);
}

#[test]
fn generated_inventory_economy_and_effect_sequences_preserve_invariants() {
    for case in 1..=128_u64 {
        let (mut world, session) = property_world();
        let mut seed = case.wrapping_mul(0x9e37_79b9_7f4a_7c15);
        for _step in 0..256 {
            let action = match next_random(&mut seed) % 12 {
                0 => Some(PlayerAction::Take("sword".into())),
                1 => Some(PlayerAction::Drop("sword".into())),
                2 => Some(PlayerAction::Equip("sword".into())),
                3 => Some(PlayerAction::Unequip("weapon".into())),
                4 => Some(PlayerAction::Take("charm".into())),
                5 => Some(PlayerAction::Drop("charm".into())),
                6 => Some(PlayerAction::Use("charm".into())),
                7 => Some(PlayerAction::Take("potion".into())),
                8 => Some(PlayerAction::Use("potion".into())),
                9 => Some(PlayerAction::Economy(EconomyAction::Buy("tonic".into()))),
                10 => Some(PlayerAction::Economy(EconomyAction::Sell("tonic".into()))),
                _ => None,
            };
            if let Some(action) = action {
                let _ = world.apply(session, action);
            } else {
                world.advance_tick().unwrap();
            }
            assert_domain_invariants(&world, session);
        }
    }
}

#[test]
fn reusable_effect_items_fail_atomically_at_the_active_effect_cap() {
    let (mut world, session) = property_world();
    world
        .apply(session, PlayerAction::Take("charm".into()))
        .unwrap();
    for _ in 0..32 {
        world
            .apply(session, PlayerAction::Use("charm".into()))
            .unwrap();
    }
    let before = (
        world.character_snapshot(session).unwrap(),
        world.character_gameplay_snapshot(session).unwrap(),
        world.runtime_snapshot(),
    );
    assert!(
        world
            .apply(session, PlayerAction::Use("charm".into()))
            .is_err()
    );
    assert_eq!(
        (
            world.character_snapshot(session).unwrap(),
            world.character_gameplay_snapshot(session).unwrap(),
            world.runtime_snapshot(),
        ),
        before
    );
    assert_domain_invariants(&world, session);
}

#[test]
fn generated_actions_do_not_create_exits_or_move_the_single_room_character() {
    let (mut world, session) = property_world();
    for direction in [
        Direction::North,
        Direction::East,
        Direction::South,
        Direction::West,
        Direction::Up,
        Direction::Down,
    ] {
        assert!(world.apply(session, PlayerAction::Move(direction)).is_err());
        assert_eq!(
            world.character_snapshot(session).unwrap().room_id,
            "property:market"
        );
    }
}
