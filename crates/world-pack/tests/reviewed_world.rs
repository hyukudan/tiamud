use world_pack::{
    AbilityCategory, AbilityStatusEffect, CURRENT_SCHEMA_VERSION, EquipmentSlot, ExitTarget,
    GrantDefinition, InitialItemLocation, ItemStackPolicy, ItemTransferPolicy, WorldPackDocument,
};

const REVIEWED_WORLD: &[u8] = include_bytes!("../../../worlds/lantern-square.world.json");

#[test]
fn reviewed_runtime_fixture_matches_world_pack_v6() {
    let document = WorldPackDocument::from_json(REVIEWED_WORLD).unwrap();
    let pack = document.world;

    pack.validate().unwrap();
    assert_eq!(pack.schema_version, CURRENT_SCHEMA_VERSION);
    assert_eq!(pack.start_room_id, "demo:lantern-square");
    assert!(pack.rooms.iter().any(|room| room.id == pack.start_room_id));
    assert_eq!(pack.item_prototypes.len(), 9);
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
        5
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
    assert_eq!(pack.gameplay.abilities.len(), 3);
    assert_eq!(pack.gameplay.npc_ability_sets.len(), 1);
    assert_eq!(pack.gameplay.loot_tables.len(), 1);
    assert_eq!(pack.gameplay.loot_sources.len(), 1);
    let loot_table = &pack.gameplay.loot_tables[0];
    assert_eq!(loot_table.id, "tiamud:gate-stalker-drops");
    assert_eq!(
        loot_table.guaranteed[0].item_prototype_id,
        "tiamud:road-token"
    );
    assert_eq!(loot_table.pools[0].entries.len(), 2);
    assert_eq!(loot_table.canonical_sha256().len(), 64);
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
