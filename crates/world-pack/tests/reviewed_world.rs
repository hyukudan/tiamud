use world_pack::{
    AbilityCategory, AbilityCommunicationKind, AbilityCommunicationScope, AbilityStatusEffect,
    AbilityTargeting, AffixPosition, CURRENT_SCHEMA_VERSION, EquipmentSlot, ExitTarget,
    GrantDefinition, InitialItemLocation, ItemStackPolicy, ItemTransferPolicy, WorldPackDocument,
};

const REVIEWED_WORLD: &[u8] = include_bytes!("../../../worlds/lantern-square.world.json");

#[test]
fn reviewed_runtime_fixture_matches_world_pack_v8() {
    let document = WorldPackDocument::from_json(REVIEWED_WORLD).unwrap();
    let pack = document.world;

    pack.validate().unwrap();
    assert_eq!(pack.schema_version, CURRENT_SCHEMA_VERSION);
    assert_eq!(pack.start_room_id, "demo:lantern-square");
    assert!(pack.rooms.iter().any(|room| room.id == pack.start_room_id));
    assert_eq!(pack.item_prototypes.len(), 10);
    assert_eq!(pack.initial_item_instances.len(), 9);
    assert!(
        pack.item_prototypes
            .iter()
            .all(|item| item.id.starts_with("tiamud:"))
    );
    let manifest = pack
        .rooms
        .iter()
        .find(|room| room.id == pack.start_room_id)
        .unwrap()
        .asset_manifest
        .as_deref()
        .unwrap();
    let components: Vec<_> = manifest.split('/').collect();
    assert_eq!(components.len(), 3);
    assert_eq!(components[0], "assets");
    assert_eq!(components[2], "manifest.json");
    assert_eq!(components[1].len(), 64);
    assert!(
        components[1]
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    );
    assert_eq!(
        pack.item_prototypes
            .iter()
            .filter(|item| item.equipment_slot.is_some() || item.consumable)
            .count(),
        7
    );
    for stable_id in ["tiamud:road-token", "tiamud:wolf-clasp"] {
        assert_eq!(
            pack.item_prototypes
                .iter()
                .find(|prototype| prototype.id == stable_id)
                .unwrap()
                .transfer_policy,
            ItemTransferPolicy::Transferable
        );
    }
    let wolf_clasp = pack
        .item_prototypes
        .iter()
        .find(|prototype| prototype.id == "tiamud:wolf-clasp")
        .unwrap();
    assert_eq!(wolf_clasp.equipment_slot, Some(EquipmentSlot::Trinket));
    let rain_writ = pack
        .item_prototypes
        .iter()
        .find(|prototype| prototype.id == "tiamud:rain-writ")
        .unwrap();
    assert!(rain_writ.consumable);
    assert_eq!(rain_writ.heal_amount, 1);
    assert_eq!(
        rain_writ.transfer_policy,
        ItemTransferPolicy::NonTransferable
    );
    assert!(pack.initial_item_instances.iter().any(|instance| {
        instance.id == rain_writ.id
            && matches!(
                &instance.location,
                InitialItemLocation::Room { room_id } if room_id == "demo:north-gate"
            )
    }));
    let storm_seal = pack
        .item_prototypes
        .iter()
        .find(|prototype| prototype.id == "tiamud:storm-seal")
        .unwrap();
    assert!(storm_seal.consumable);
    assert_eq!(storm_seal.stack_policy, ItemStackPolicy::Unique);
    assert_eq!(
        storm_seal.transfer_policy,
        ItemTransferPolicy::CharacterBound
    );
    assert_eq!(
        storm_seal.sealed_loot_table_id.as_deref(),
        Some("tiamud:storm-oath-rewards")
    );
    assert!(
        pack.initial_item_instances
            .iter()
            .all(|instance| instance.prototype_id != storm_seal.id)
    );
    let stormglass = pack
        .item_prototypes
        .iter()
        .find(|prototype| prototype.id == "tiamud:stormglass-talon")
        .unwrap();
    assert_eq!(stormglass.equipment_slot, Some(EquipmentSlot::Trinket));
    assert_eq!(
        stormglass.transfer_policy,
        ItemTransferPolicy::CharacterBound
    );
    assert!(pack.initial_item_instances.iter().any(|instance| {
        instance.id == stormglass.id
            && matches!(
                &instance.location,
                InitialItemLocation::NpcSpawn { npc_spawn_id }
                    if npc_spawn_id == "tiamud:gate-stalker-1"
            )
    }));
    let tonic = pack
        .item_prototypes
        .iter()
        .find(|prototype| prototype.id == "tiamud:ember-tonic")
        .unwrap();
    assert_eq!(tonic.stack_policy, ItemStackPolicy::Fungible);
    assert_eq!(tonic.max_stack_units, 20);
    assert_eq!(
        pack.initial_item_instances
            .iter()
            .filter(|instance| instance.prototype_id == tonic.id)
            .count(),
        2
    );
    let satchel = pack
        .item_prototypes
        .iter()
        .find(|prototype| prototype.id == "tiamud:archive-satchel")
        .unwrap();
    assert_eq!(satchel.container.unwrap().max_slots, 8);
    assert!(pack.initial_item_instances.iter().any(|instance| {
        instance.id == "tiamud:weathered-map"
            && matches!(
                &instance.location,
                InitialItemLocation::Container {
                    container_instance_id
                } if container_instance_id == "tiamud:archive-satchel"
            )
    }));
    assert_eq!(pack.gameplay.npc_prototypes.len(), 1);
    assert_eq!(pack.gameplay.npc_spawns.len(), 1);
    assert!(pack.gameplay.default_archetype_id.is_none());
    assert!(pack.gameplay.archetypes.is_empty());
    assert_eq!(pack.gameplay.abilities.len(), 4);
    assert_eq!(pack.gameplay.npc_ability_sets.len(), 1);
    assert_eq!(pack.gameplay.loot_tables.len(), 2);
    assert_eq!(pack.gameplay.loot_sources.len(), 1);
    let loot_table = &pack.gameplay.loot_tables[0];
    assert_eq!(loot_table.id, "tiamud:gate-stalker-drops");
    assert_eq!(
        loot_table.guaranteed[0].item_prototype_id,
        "tiamud:road-token"
    );
    assert_eq!(loot_table.pools[0].entries.len(), 2);
    assert_eq!(
        loot_table.pools[0]
            .entries
            .iter()
            .find(|entry| entry.item_prototype_id.as_deref() == Some("tiamud:wolf-clasp"))
            .unwrap()
            .affix_pool_id
            .as_deref(),
        Some("tiamud:storm-talon-affixes")
    );
    assert_eq!(loot_table.canonical_sha256().len(), 64);
    assert!(loot_table.guaranteed.iter().any(|entry| {
        entry.item_prototype_id == storm_seal.id && entry.affix_pool_id.is_none()
    }));
    let reward_table = pack
        .gameplay
        .loot_tables
        .iter()
        .find(|table| table.id == "tiamud:storm-oath-rewards")
        .unwrap();
    assert_eq!(reward_table.guaranteed.len(), 1);
    assert_eq!(
        reward_table.guaranteed[0].affix_pool_id.as_deref(),
        Some("tiamud:storm-talon-affixes")
    );
    assert_eq!(pack.gameplay.affixes.len(), 1);
    assert_eq!(pack.gameplay.affix_pools.len(), 1);
    assert_eq!(pack.gameplay.affixes[0].position, AffixPosition::Prefix);
    assert_eq!(pack.gameplay.affixes[0].attack_bonus_delta, 1);
    assert_eq!(pack.gameplay.affix_pools[0].entries.len(), 2);
    let loot_source = &pack.gameplay.loot_sources[0];
    assert_eq!(loot_source.npc_spawn_id, "tiamud:gate-stalker-1");
    assert_eq!(
        loot_source.rights,
        world_pack::LootRightsPolicy::PartyRoundRobin
    );
    let progression = pack.gameplay.character_progression.as_ref().unwrap();
    assert_eq!(progression.defaults.base_class_id, "tiamud:wanderer");
    assert_eq!(progression.lineages.len(), 3);
    assert_eq!(progression.backgrounds.len(), 3);
    assert_eq!(progression.base_classes.len(), 3);
    assert_eq!(progression.advanced_classes.len(), 4);
    let arcanist = progression
        .base_classes
        .iter()
        .find(|class| class.id == "tiamud:arcanist")
        .unwrap();
    assert!(
        arcanist
            .creation_grants
            .contains(&GrantDefinition::Attribute {
                attribute_id: "intellect".to_owned(),
                amount: 8,
            })
    );
    assert!(
        arcanist
            .creation_grants
            .contains(&GrantDefinition::ResourceMax {
                resource_id: "mana".to_owned(),
                amount: 14,
            })
    );
    assert_eq!(
        progression.level_thresholds[0].total_experience_required,
        100
    );
    let martial = pack
        .gameplay
        .abilities
        .iter()
        .find(|ability| ability.id == "tiamud:lunging-strike")
        .unwrap();
    assert_eq!(martial.category, AbilityCategory::Martial);
    assert!(matches!(
        martial.status_effects.as_slice(),
        [AbilityStatusEffect::AttributeModifier { magnitude: -1, .. }]
    ));
    let spell = pack
        .gameplay
        .abilities
        .iter()
        .find(|ability| ability.id == "tiamud:arcane-bolt")
        .unwrap();
    assert_eq!(spell.category, AbilityCategory::Spell);
    let whisper = pack
        .gameplay
        .abilities
        .iter()
        .find(|ability| ability.id == "tiamud:mind-whisper")
        .unwrap();
    assert_eq!(whisper.category, AbilityCategory::Utility);
    assert_eq!(whisper.targeting, AbilityTargeting::SingleAlly);
    assert_eq!(whisper.resource_costs.get("stamina"), Some(&2));
    assert_eq!(whisper.cooldown_ticks, 2);
    assert!(whisper.damage.is_none());
    assert!(whisper.heal.is_none());
    assert!(whisper.status_effects.is_empty());
    let communication = whisper.communication.as_ref().unwrap();
    assert_eq!(communication.kind, AbilityCommunicationKind::Telepathy);
    assert_eq!(communication.scope, AbilityCommunicationScope::DirectOnline);
    assert_eq!(communication.blocked_room_ids, ["demo:archive"]);
    let wanderer = progression
        .base_classes
        .iter()
        .find(|class| class.id == "tiamud:wanderer")
        .unwrap();
    assert!(
        wanderer
            .creation_grants
            .contains(&GrantDefinition::Ability {
                ability_id: "tiamud:mind-whisper".to_owned(),
            })
    );
    assert_eq!(
        pack.gameplay.npc_spawns[0].prototype_id,
        "tiamud:gate-stalker"
    );
    assert!(
        pack.rooms
            .iter()
            .flat_map(|room| &room.exits)
            .all(|exit| { matches!(exit.target, ExitTarget::Room { .. }) })
    );
}
