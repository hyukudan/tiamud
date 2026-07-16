use std::{
    collections::HashMap,
    fs::File,
    io::{self, Read},
    path::{Path, PathBuf},
};

use crate::runtime_content::{RuntimeContent, RuntimeScript};
use thiserror::Error;
use tiamud_core::{
    ActiveEffect, Direction as CoreDirection, EffectKind as CoreEffectKind,
    EquipmentSlot as CoreEquipmentSlot, ExitDefinition, ItemDefinition, ItemGameplayDefinition,
    LightLevel as CoreLightLevel, NpcDefinition, RoomDefinition, SceneAudio, SceneImage, World,
    WorldBuildError, WorldDefinition, WorldError,
};
use world_pack::{
    AudioCue, Direction as PackDirection, EffectKind as PackEffectKind,
    EquipmentSlot as PackEquipmentSlot, ExitTarget, LightLevel as PackLightLevel,
    RoomMediaDefinition, RoomPerceptionDefinition, ValidationErrors, WorldPack, WorldPackDocument,
};

const MAX_WORLD_PACK_BYTES: usize = 16 * 1024 * 1024;
const DEFAULT_WORLD_PACK: &[u8] = include_bytes!("../../../worlds/lantern-square.world.json");

#[cfg(test)]
pub fn default_world_definition() -> Result<WorldDefinition, WorldLoadError> {
    parse_world_definition(DEFAULT_WORLD_PACK)
}

#[derive(Debug)]
pub struct LoadedWorldDefinition {
    pub definition: WorldDefinition,
    pub rng_seed: Option<u64>,
    pub item_rules: Vec<ItemGameplayDefinition>,
    pub npcs: Vec<NpcDefinition>,
    pub room_light: Vec<(String, CoreLightLevel)>,
    pub hidden_items: Vec<(String, u16)>,
    pub hidden_npcs: Vec<(String, u16)>,
    pub secret_exits: Vec<(String, CoreDirection, u16)>,
    pub runtime_content: RuntimeContent,
}

#[derive(Debug, Error)]
pub enum WorldInitializationError {
    #[error(transparent)]
    Build(#[from] WorldBuildError),
    #[error(transparent)]
    Configure(#[from] WorldError),
}

/// Applies the complete authored startup contract to the runtime world. NPC
/// loot is explicitly staged outside all rooms, then transferred to its spawn
/// owner; it is never briefly visible as duplicate room content.
pub fn build_loaded_world(
    loaded: &LoadedWorldDefinition,
) -> Result<World, WorldInitializationError> {
    let staged_loot = loaded
        .npcs
        .iter()
        .flat_map(|npc| npc.loot_item_ids.iter().cloned())
        .collect();
    let mut world =
        World::from_definition_with_staged_items(loaded.definition.clone(), staged_loot)?;
    if let Some(seed) = loaded.rng_seed {
        world.set_rng_seed(seed);
    }
    for rule in &loaded.item_rules {
        world.configure_item_gameplay(rule.clone())?;
    }
    for (room_id, light) in &loaded.room_light {
        world.configure_room_light(room_id, *light)?;
    }
    for (item_id, difficulty) in &loaded.hidden_items {
        world.configure_hidden_item(item_id, *difficulty)?;
    }
    for (room_id, direction, difficulty) in &loaded.secret_exits {
        world.configure_secret_exit(room_id, *direction, *difficulty)?;
    }
    for npc in &loaded.npcs {
        world.spawn_npc(npc.clone())?;
    }
    for (npc_id, difficulty) in &loaded.hidden_npcs {
        world.configure_hidden_npc(npc_id, *difficulty)?;
    }
    Ok(world)
}

pub fn default_loaded_world() -> Result<LoadedWorldDefinition, WorldLoadError> {
    parse_loaded_world(DEFAULT_WORLD_PACK)
}

pub fn load_loaded_world(path: impl AsRef<Path>) -> Result<LoadedWorldDefinition, WorldLoadError> {
    let bytes = read_world_pack(path.as_ref())?;
    parse_loaded_world(&bytes)
}

/// Load and validate an authored WorldPack before it reaches the world actor.
#[cfg(test)]
pub fn load_world_definition(path: impl AsRef<Path>) -> Result<WorldDefinition, WorldLoadError> {
    let bytes = read_world_pack(path.as_ref())?;
    parse_world_definition(&bytes)
}

fn read_world_pack(path: &Path) -> Result<Vec<u8>, WorldLoadError> {
    let mut file = File::open(path).map_err(|source| WorldLoadError::Open {
        path: path.to_owned(),
        source,
    })?;
    let mut bytes = Vec::new();
    file.by_ref()
        .take((MAX_WORLD_PACK_BYTES + 1) as u64)
        .read_to_end(&mut bytes)
        .map_err(|source| WorldLoadError::Read {
            path: path.to_owned(),
            source,
        })?;
    Ok(bytes)
}

/// Parse a WorldPack from memory using the same limits and runtime checks as
/// [`load_world_definition`]. This is also useful to embedders and tests.
#[cfg(test)]
pub fn parse_world_definition(bytes: &[u8]) -> Result<WorldDefinition, WorldLoadError> {
    Ok(parse_loaded_world(bytes)?.definition)
}

pub fn parse_loaded_world(bytes: &[u8]) -> Result<LoadedWorldDefinition, WorldLoadError> {
    if bytes.len() > MAX_WORLD_PACK_BYTES {
        return Err(WorldLoadError::TooLarge {
            size: bytes.len(),
            max: MAX_WORLD_PACK_BYTES,
        });
    }

    let document: WorldPackDocument = serde_json::from_slice(bytes)?;
    document.validate()?;
    convert_document(document)
}

fn convert_document(
    mut document: WorldPackDocument,
) -> Result<LoadedWorldDefinition, WorldLoadError> {
    let seed = document.world.gameplay.rng_seed.unwrap_or(0x5449_414d_5544);
    for variants in &document.scene_variants {
        for variant in &variants.variants {
            if let Some(path) = &variant.asset_manifest
                && !runtime_asset_manifest_path(path)
            {
                return Err(WorldLoadError::UnsupportedAssetManifestPath {
                    room_id: variants.room_id.clone(),
                    path: path.clone(),
                });
            }
        }
    }
    let mut scripts = Vec::with_capacity(document.scripts.len());
    for definition in std::mem::take(&mut document.scripts) {
        let script_id = definition.id.clone();
        let script = RuntimeScript::from_definition(definition);
        tiamud_scripting::validate_module(&script.source, &script.hooks, script.limits)
            .map_err(|source| WorldLoadError::Script { script_id, source })?;
        scripts.push(script);
    }
    let runtime_content =
        RuntimeContent::new(std::mem::take(&mut document.scene_variants), scripts, seed);
    let mut loaded = convert_pack(
        document.world,
        std::mem::take(&mut document.room_media),
        std::mem::take(&mut document.room_perception),
    )?;
    loaded.runtime_content = runtime_content;
    Ok(loaded)
}

fn convert_pack(
    mut pack: WorldPack,
    room_media: Vec<RoomMediaDefinition>,
    room_perception: Vec<RoomPerceptionDefinition>,
) -> Result<LoadedWorldDefinition, WorldLoadError> {
    let gameplay = std::mem::take(&mut pack.gameplay);
    let start_room_id = pack.start_room_id.clone();
    let items = pack
        .items
        .into_iter()
        .map(|item| ItemDefinition {
            stable_id: item.id,
            name: item.name,
            description: item.description,
            aliases: item.aliases,
        })
        .collect();
    let room_ids_by_vnum = pack
        .rooms
        .iter()
        .filter_map(|room| room.legacy_vnum.map(|vnum| (vnum, room.id.clone())))
        .collect::<HashMap<_, _>>();
    let mut room_media = room_media
        .into_iter()
        .map(|media| (media.room_id.clone(), media))
        .collect::<HashMap<_, _>>();

    let mut rooms = Vec::with_capacity(pack.rooms.len());
    for room in pack.rooms {
        let media = room_media.remove(&room.id);
        let mut exits = Vec::with_capacity(room.exits.len());
        for exit in room.exits {
            let target_room_id = match exit.target {
                ExitTarget::Room { room_id } => room_id,
                ExitTarget::LegacyVnum { vnum } => room_ids_by_vnum
                    .get(&vnum)
                    .cloned()
                    .ok_or_else(|| WorldLoadError::UnresolvedLegacyVnum {
                        room_id: room.id.clone(),
                        direction: exit.direction,
                        vnum,
                    })?,
            };
            exits.push(ExitDefinition {
                direction: map_direction(exit.direction),
                target_room_id,
            });
        }

        let image = match room.asset_manifest {
            Some(path) if runtime_asset_manifest_path(&path) => Some(SceneImage {
                alt: media
                    .as_ref()
                    .and_then(|media| media.image_alt.resolve("en"))
                    .expect("validated image manifests always have localized alt text")
                    .to_owned(),
                localized_alt: media
                    .as_ref()
                    .map(|media| media.image_alt.0.clone())
                    .unwrap_or_default(),
                manifest: Some(format!("/{path}")),
                avif: None,
                webp: None,
                jpeg: None,
                width: None,
                height: None,
            }),
            Some(path) => {
                return Err(WorldLoadError::UnsupportedAssetManifestPath {
                    room_id: room.id,
                    path,
                });
            }
            None => None,
        };
        let audio = media
            .as_ref()
            .and_then(|media| media.audio.as_ref())
            .map(scene_audio);

        rooms.push(RoomDefinition {
            stable_id: room.id,
            title: room.title,
            prose: split_prose(&room.description),
            image,
            audio,
            exits,
            item_ids: room.items,
        });
    }

    let definition = WorldDefinition {
        start_room_id,
        items,
        rooms,
    };
    let item_rules = gameplay
        .item_rules
        .into_iter()
        .map(|rule| ItemGameplayDefinition {
            item_id: rule.item_id,
            equipment_slot: rule.equipment_slot.map(map_equipment_slot),
            attack_bonus: rule.attack_bonus,
            heal_amount: rule.heal_amount,
            consumable: rule.consumable,
            effect: rule.effect.map(|effect| ActiveEffect {
                kind: map_effect_kind(effect.kind),
                magnitude: effect.magnitude,
                remaining_ticks: effect.remaining_ticks,
            }),
        })
        .collect();
    let prototypes = gameplay
        .npc_prototypes
        .into_iter()
        .map(|prototype| (prototype.id.clone(), prototype))
        .collect::<HashMap<_, _>>();
    let npcs = gameplay
        .npc_spawns
        .into_iter()
        .filter_map(|spawn| {
            let prototype = prototypes.get(&spawn.prototype_id)?;
            Some(NpcDefinition {
                stable_id: spawn.id,
                name: prototype.name.clone(),
                description: prototype.description.clone(),
                room_id: spawn.room_id,
                level: prototype.level,
                max_health: prototype.max_health,
                strength: prototype.strength,
                defence: prototype.defence,
                experience_reward: prototype.experience_reward,
                loot_item_ids: prototype.loot_item_ids.clone(),
                respawn_ticks: prototype.respawn_ticks,
            })
        })
        .collect();
    let mut room_light = Vec::with_capacity(room_perception.len());
    let mut hidden_items = Vec::new();
    let mut hidden_npcs = Vec::new();
    let mut secret_exits = Vec::new();
    for perception in room_perception {
        room_light.push((perception.room_id.clone(), map_light(perception.light)));
        hidden_items.extend(perception.hidden_items);
        hidden_npcs.extend(perception.hidden_npcs);
        secret_exits.extend(
            perception
                .secret_exits
                .into_iter()
                .map(|(direction, difficulty)| {
                    (
                        perception.room_id.clone(),
                        map_direction(direction),
                        difficulty,
                    )
                }),
        );
    }
    Ok(LoadedWorldDefinition {
        definition,
        rng_seed: gameplay.rng_seed,
        item_rules,
        npcs,
        room_light,
        hidden_items,
        hidden_npcs,
        secret_exits,
        runtime_content: RuntimeContent::default(),
    })
}

fn map_light(light: PackLightLevel) -> CoreLightLevel {
    match light {
        PackLightLevel::Bright => CoreLightLevel::Bright,
        PackLightLevel::Dim => CoreLightLevel::Dim,
        PackLightLevel::Dark => CoreLightLevel::Dark,
    }
}

fn scene_audio(cue: &AudioCue) -> SceneAudio {
    SceneAudio {
        channel: cue.channel.as_str().to_owned(),
        url: format!("/{}", cue.path),
        looped: cue.looped,
    }
}

fn map_equipment_slot(slot: PackEquipmentSlot) -> CoreEquipmentSlot {
    match slot {
        PackEquipmentSlot::Weapon => CoreEquipmentSlot::Weapon,
        PackEquipmentSlot::Body => CoreEquipmentSlot::Body,
        PackEquipmentSlot::Trinket => CoreEquipmentSlot::Trinket,
    }
}

fn map_effect_kind(kind: PackEffectKind) -> CoreEffectKind {
    match kind {
        PackEffectKind::Regeneration => CoreEffectKind::Regeneration,
        PackEffectKind::Strength => CoreEffectKind::Strength,
    }
}

fn runtime_asset_manifest_path(path: &str) -> bool {
    let mut components = path.split('/');
    let (Some("assets"), Some(hash), Some("manifest.json"), None) = (
        components.next(),
        components.next(),
        components.next(),
        components.next(),
    ) else {
        return false;
    };
    hash.len() == 64
        && hash
            .bytes()
            .all(|byte| byte.is_ascii_digit() || matches!(byte, b'a'..=b'f'))
}

fn split_prose(description: &str) -> Vec<String> {
    description
        .replace("\r\n", "\n")
        .replace('\r', "\n")
        .split("\n\n")
        .map(str::trim)
        .filter(|paragraph| !paragraph.is_empty())
        .map(str::to_owned)
        .collect()
}

fn map_direction(direction: PackDirection) -> CoreDirection {
    match direction {
        PackDirection::North => CoreDirection::North,
        PackDirection::East => CoreDirection::East,
        PackDirection::South => CoreDirection::South,
        PackDirection::West => CoreDirection::West,
        PackDirection::Up => CoreDirection::Up,
        PackDirection::Down => CoreDirection::Down,
    }
}

#[derive(Debug, Error)]
pub enum WorldLoadError {
    #[error("could not open WorldPack {path:?}: {source}")]
    Open {
        path: PathBuf,
        #[source]
        source: io::Error,
    },
    #[error("could not read WorldPack {path:?}: {source}")]
    Read {
        path: PathBuf,
        #[source]
        source: io::Error,
    },
    #[error("WorldPack contains {size} bytes; the runtime limit is {max} bytes")]
    TooLarge { size: usize, max: usize },
    #[error("WorldPack is not valid JSON: {0}")]
    Json(#[from] serde_json::Error),
    #[error(transparent)]
    Validation(#[from] ValidationErrors),
    #[error(
        "room {room_id:?} exit {direction:?} points to legacy VNUM {vnum}, which is not present in this WorldPack"
    )]
    UnresolvedLegacyVnum {
        room_id: String,
        direction: PackDirection,
        vnum: i64,
    },
    #[error(
        "room {room_id:?} asset manifest {path:?} is not a content-addressed assets/<sha256>/manifest.json path"
    )]
    UnsupportedAssetManifestPath { room_id: String, path: String },
    #[error("WorldPack script {script_id:?} is invalid: {source}")]
    Script {
        script_id: String,
        #[source]
        source: tiamud_scripting::ScriptError,
    },
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reviewed_pack_loads_as_a_runtime_definition() {
        let path =
            Path::new(env!("CARGO_MANIFEST_DIR")).join("../../worlds/lantern-square.world.json");
        let definition = load_world_definition(path).unwrap();

        assert_eq!(definition.start_room_id, "demo:lantern-square");
        assert_eq!(definition.rooms.len(), 3);
        assert_eq!(definition.rooms[0].prose.len(), 2);
        assert_eq!(definition.rooms[0].exits.len(), 2);
        assert_eq!(
            definition.rooms[0]
                .image
                .as_ref()
                .and_then(|image| image.manifest.as_deref()),
            Some(
                "/assets/8ade88f0df1adb294333e72d3fa2d0863543dc0263ebe3038208f3c9e415924c/manifest.json"
            )
        );
        assert_eq!(
            definition.rooms[0]
                .image
                .as_ref()
                .map(|image| image.alt.as_str()),
            Some(
                "Lantern Square after rain, its dark stones lit by a blue-glass lantern beneath the clock tower."
            )
        );
        assert_eq!(
            definition.rooms[0]
                .image
                .as_ref()
                .and_then(|image| image.localized_alt.get("es"))
                .map(String::as_str),
            Some(
                "La Plaza del Farol tras la lluvia, con las piedras oscuras iluminadas por un farol de cristal azul bajo la torre del reloj."
            )
        );
    }

    #[test]
    fn embedded_default_pack_loads_without_a_working_directory_dependency() {
        let definition = default_world_definition().unwrap();

        assert_eq!(definition.start_room_id, "demo:lantern-square");
        assert_eq!(definition.rooms.len(), 3);
    }

    #[test]
    fn reviewed_pack_loads_item_rules_and_an_npc_spawn() {
        let loaded = default_loaded_world().unwrap();
        assert_eq!(loaded.item_rules.len(), 3);
        assert_eq!(loaded.npcs.len(), 1);
        assert_eq!(loaded.npcs[0].stable_id, "tiamud:gate-stalker-1");
        assert_eq!(loaded.npcs[0].loot_item_ids, ["tiamud:wolf-clasp"]);
        assert_eq!(loaded.rng_seed, Some(6_073_466_303_471_576_068));
        assert!(
            loaded
                .room_light
                .contains(&("demo:north-gate".into(), CoreLightLevel::Bright))
        );
        assert_eq!(loaded.hidden_items, [("tiamud:road-token".into(), 8)]);
        assert_eq!(loaded.hidden_npcs, [("tiamud:gate-stalker-1".into(), 8)]);
        assert_eq!(
            loaded.secret_exits,
            [("demo:lantern-square".into(), CoreDirection::Down, 8)]
        );
    }

    #[test]
    fn reviewed_pack_builds_and_assigns_loot_to_its_npc_without_room_placement() {
        let path =
            Path::new(env!("CARGO_MANIFEST_DIR")).join("../../worlds/lantern-square.world.json");
        let loaded = load_loaded_world(path).unwrap();
        assert!(
            loaded
                .definition
                .rooms
                .iter()
                .all(|room| !room.item_ids.iter().any(|item| item == "tiamud:wolf-clasp"))
        );

        let world = build_loaded_world(&loaded).unwrap();
        let snapshot = world.runtime_snapshot();
        assert!(snapshot.npcs.contains_key("tiamud:gate-stalker-1"));
        assert_eq!(
            snapshot.item_locations.get("tiamud:wolf-clasp"),
            Some(&tiamud_core::RuntimeItemLocation::HeldByNpc {
                npc_id: "tiamud:gate-stalker-1".into(),
            })
        );
    }

    #[test]
    fn empty_world_is_rejected_for_runtime() {
        let error = parse_world_definition(
            br#"{"schemaVersion":2,"id":"empty","name":"Empty","startRoomId":"missing","rooms":[]}"#,
        )
        .err()
        .unwrap();

        assert!(matches!(error, WorldLoadError::Validation(_)));
    }

    #[test]
    fn unresolved_external_legacy_vnum_is_rejected_for_runtime() {
        let error = parse_world_definition(
            br#"{
                "schemaVersion": 2,
                "id": "legacy",
                "name": "Legacy",
                "startRoomId": "legacy:1",
                "rooms": [{
                    "id": "legacy:1",
                    "legacyVnum": 1,
                    "title": "Room",
                    "description": "Description",
                    "exits": [{
                        "direction": "north",
                        "target": {"kind": "legacy_vnum", "vnum": 999}
                    }]
                }]
            }"#,
        )
        .err()
        .unwrap();

        assert!(matches!(
            error,
            WorldLoadError::UnresolvedLegacyVnum { vnum: 999, .. }
        ));
    }

    #[test]
    fn internal_legacy_vnum_is_resolved_to_a_stable_room_id() {
        let definition = parse_world_definition(
            br#"{
                "schemaVersion": 2,
                "id": "legacy",
                "name": "Legacy",
                "startRoomId": "legacy:1",
                "rooms": [
                    {
                        "id": "legacy:1",
                        "legacyVnum": 1,
                        "title": "One",
                        "description": "First",
                        "exits": [{
                            "direction": "north",
                            "target": {"kind": "legacy_vnum", "vnum": 2}
                        }]
                    },
                    {
                        "id": "legacy:2",
                        "legacyVnum": 2,
                        "title": "Two",
                        "description": "Second"
                    }
                ]
            }"#,
        )
        .unwrap();

        assert_eq!(definition.rooms[0].exits[0].target_room_id, "legacy:2");
    }

    #[test]
    fn structural_validation_errors_are_preserved() {
        let error = parse_world_definition(
            br#"{
                "schemaVersion": 2,
                "id": "broken",
                "name": "Broken",
                "startRoomId": "broken:1",
                "rooms": [{
                    "id": "broken:1",
                    "title": "Room",
                    "description": "Description",
                    "exits": [{
                        "direction": "north",
                        "target": {"kind": "room", "room_id": "broken:missing"}
                    }]
                }]
            }"#,
        )
        .err()
        .unwrap();

        assert!(matches!(error, WorldLoadError::Validation(_)));
    }

    #[test]
    fn oversized_input_is_rejected_before_json_parsing() {
        let bytes = vec![b' '; MAX_WORLD_PACK_BYTES + 1];
        let error = parse_world_definition(&bytes).err().unwrap();

        assert!(matches!(error, WorldLoadError::TooLarge { .. }));
    }

    #[test]
    fn runtime_requires_a_content_addressed_asset_manifest() {
        let error = parse_world_definition(
            br#"{
                "schemaVersion": 2,
                "id": "unsafe-media",
                "name": "Unsafe media",
                "startRoomId": "unsafe-media:1",
                "rooms": [{
                    "id": "unsafe-media:1",
                    "title": "Room",
                    "description": "Description",
                    "assetManifest": "assets/latest/manifest.json"
                }],
                "roomMedia": [{
                    "roomId": "unsafe-media:1",
                    "imageAlt": {"en": "A safe description."}
                }]
            }"#,
        )
        .unwrap_err();

        assert!(matches!(
            error,
            WorldLoadError::UnsupportedAssetManifestPath { .. }
        ));
    }

    #[test]
    fn localized_alt_and_content_addressed_audio_reach_the_runtime_scene() {
        let digest = "a".repeat(64);
        let json = format!(
            r#"{{
                "schemaVersion": 2,
                "id": "media",
                "name": "Media",
                "startRoomId": "media:1",
                "rooms": [{{
                    "id": "media:1",
                    "title": "Rain Hall",
                    "description": "Rain sounds beyond the roof.",
                    "assetManifest": "assets/{digest}/manifest.json"
                }}],
                "roomMedia": [{{
                    "roomId": "media:1",
                    "imageAlt": {{
                        "es": "Un salón bajo la lluvia.",
                        "en": "A hall beneath the rain."
                    }},
                    "audio": {{
                        "channel": "ambience",
                        "path": "assets/{digest}/audio-rain.opus",
                        "loop": true
                    }}
                }}]
            }}"#
        );
        let definition = parse_world_definition(json.as_bytes()).unwrap();
        let room = &definition.rooms[0];
        assert_eq!(
            room.image.as_ref().map(|image| image.alt.as_str()),
            Some("A hall beneath the rain.")
        );
        assert_eq!(
            room.audio.as_ref(),
            Some(&SceneAudio {
                channel: "ambience".into(),
                url: format!("/assets/{digest}/audio-rain.opus"),
                looped: true,
            })
        );
    }

    #[test]
    fn prose_normalises_windows_and_classic_mac_line_endings() {
        assert_eq!(
            split_prose("First.\r\n\r\nSecond.\r\rThird."),
            ["First.", "Second.", "Third."]
        );
    }
}
