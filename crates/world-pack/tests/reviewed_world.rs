use world_pack::{
    AbilityCategory, AbilityStatusEffect, CURRENT_SCHEMA_VERSION, ExitTarget, GrantDefinition,
    InitialItemLocation, ItemStackPolicy, WorldPackDocument,
};

const REVIEWED_WORLD: &[u8] = include_bytes!("../../../worlds/lantern-square.world.json");

#[test]
fn reviewed_runtime_fixture_matches_world_pack_v5() {
    let document = WorldPackDocument::from_json(REVIEWED_WORLD).unwrap();
    let pack = document.world;

    pack.validate().unwrap();
    assert_eq!(pack.schema_version, CURRENT_SCHEMA_VERSION);
    assert_eq!(pack.start_room_id, "demo:lantern-square");
    assert!(pack.rooms.iter().any(|room| room.id == pack.start_room_id));
    assert_eq!(pack.item_prototypes.len(), 7);
    assert_eq!(pack.initial_item_instances.len(), 8);
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
        3
    );
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
