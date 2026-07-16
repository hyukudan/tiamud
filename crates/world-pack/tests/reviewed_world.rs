use world_pack::{
    AbilityCategory, AbilityStatusEffect, CURRENT_SCHEMA_VERSION, ExitTarget, WorldPack,
};

const REVIEWED_WORLD: &[u8] = include_bytes!("../../../worlds/lantern-square.world.json");

#[test]
fn reviewed_runtime_fixture_matches_world_pack_v2() {
    let pack: WorldPack = serde_json::from_slice(REVIEWED_WORLD).unwrap();

    pack.validate().unwrap();
    assert_eq!(pack.schema_version, CURRENT_SCHEMA_VERSION);
    assert_eq!(pack.start_room_id, "demo:lantern-square");
    assert!(pack.rooms.iter().any(|room| room.id == pack.start_room_id));
    assert_eq!(pack.items.len(), 6);
    assert!(pack.items.iter().all(|item| item.id.starts_with("tiamud:")));
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
    assert_eq!(pack.gameplay.item_rules.len(), 3);
    assert_eq!(pack.gameplay.npc_prototypes.len(), 1);
    assert_eq!(pack.gameplay.npc_spawns.len(), 1);
    assert_eq!(
        pack.gameplay.default_archetype_id.as_deref(),
        Some("tiamud:wanderer")
    );
    assert_eq!(pack.gameplay.archetypes.len(), 3);
    assert_eq!(pack.gameplay.abilities.len(), 3);
    assert_eq!(pack.gameplay.npc_ability_sets.len(), 1);
    let arcanist = pack
        .gameplay
        .archetypes
        .iter()
        .find(|archetype| archetype.id == "tiamud:arcanist")
        .unwrap();
    assert_eq!(arcanist.initial_attributes["intellect"], 8);
    assert_eq!(arcanist.initial_resources["mana"], 14);
    assert_eq!(arcanist.resource_regeneration["mana"], 2);
    assert_eq!(arcanist.level_progression[0].total_experience_required, 100);
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
