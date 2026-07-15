use world_pack::{CURRENT_SCHEMA_VERSION, ExitTarget, WorldPack};

const REVIEWED_WORLD: &[u8] = include_bytes!("../../../worlds/lantern-square.world.json");

#[test]
fn reviewed_runtime_fixture_matches_world_pack_v2() {
    let pack: WorldPack = serde_json::from_slice(REVIEWED_WORLD).unwrap();

    pack.validate().unwrap();
    assert_eq!(pack.schema_version, CURRENT_SCHEMA_VERSION);
    assert_eq!(pack.start_room_id, "demo:lantern-square");
    assert_eq!(pack.rooms.first().unwrap().id, "demo:lantern-square");
    assert_eq!(pack.items.len(), 6);
    assert!(pack.items.iter().all(|item| item.id.starts_with("tiamud:")));
    let manifest = pack
        .rooms
        .first()
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
