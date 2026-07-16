use std::collections::BTreeMap;

use serde_json::json;
use world_pack::{
    AbilityEffectTarget, AbilityStatusEffect, AreaPack, DamageType, LevelProgressionDefinition,
    MAX_ABILITY_COOLDOWN_TICKS, MAX_ABILITY_EFFECT_DURATION_TICKS, MAX_GAMEPLAY_VALUE,
    MAX_RESISTANCE_PERCENT, MAX_RESOURCES_PER_ARCHETYPE, MAX_STATS_PER_ARCHETYPE, ValidationError,
    WorldPack, WorldPackDocument,
};

const REVIEWED_WORLD: &[u8] = include_bytes!("../../../worlds/lantern-square.world.json");

fn reviewed_world() -> WorldPack {
    serde_json::from_slice(REVIEWED_WORLD).unwrap()
}

#[test]
fn old_world_pack_v2_defaults_to_no_character_catalog() {
    let pack: WorldPack = serde_json::from_value(json!({
        "schemaVersion": 2,
        "id": "legacy-v2",
        "name": "Legacy v2",
        "startRoomId": "legacy:start",
        "rooms": [{
            "id": "legacy:start",
            "title": "Start",
            "description": "An old package without authored archetypes."
        }]
    }))
    .unwrap();

    pack.validate().unwrap();
    assert!(pack.gameplay.default_archetype_id.is_none());
    assert!(pack.gameplay.archetypes.is_empty());
    assert!(pack.gameplay.abilities.is_empty());
    assert!(pack.gameplay.npc_ability_sets.is_empty());
}

#[test]
fn modular_authoring_rejects_nested_ability_typos() {
    let error = serde_json::from_value::<AreaPack>(json!({
        "schemaVersion": 1,
        "id": "abilities",
        "abilities": [{
            "id": "demo:strike",
            "name": "Strike",
            "description": "A strike.",
            "category": "martial",
            "targeting": "single_enemy",
            "cooldownTick": 2,
            "damage": {
                "target": "target",
                "damageType": "physical",
                "baseAmount": 1
            }
        }]
    }))
    .unwrap_err();

    let message = error.to_string();
    assert!(message.contains("$.abilities[0]"));
    assert!(message.contains("cooldownTick"));
}

#[test]
fn reviewed_catalog_rejects_broken_references_and_hard_bounds() {
    let mut pack = reviewed_world();
    pack.gameplay.default_archetype_id = Some("missing:class".to_owned());
    pack.gameplay.archetypes[0]
        .initial_resistances
        .insert(DamageType::Fire, MAX_RESISTANCE_PERCENT + 1);
    pack.gameplay.archetypes[0]
        .resource_regeneration
        .insert("missing_pool".to_owned(), 1);
    pack.gameplay.abilities[0].cooldown_ticks = MAX_ABILITY_COOLDOWN_TICKS + 1;
    pack.gameplay.abilities[0]
        .resource_costs
        .insert("missing_pool".to_owned(), 1);
    pack.gameplay.abilities[0]
        .status_effects
        .push(AbilityStatusEffect::Stun {
            target: world_pack::AbilityEffectTarget::Target,
            duration_ticks: MAX_ABILITY_EFFECT_DURATION_TICKS + 1,
        });
    let duplicate_name = pack.gameplay.abilities[0].name.to_uppercase();
    pack.gameplay.abilities[1].name = duplicate_name;
    pack.gameplay.npc_ability_sets[0]
        .ability_ids
        .push("missing:ability".to_owned());
    pack.gameplay.npc_ability_sets[0]
        .resource_regeneration
        .insert("missing_pool".to_owned(), 1);

    let errors = pack.validate().unwrap_err().0;
    let messages = errors
        .iter()
        .filter_map(|error| match error {
            ValidationError::InvalidGameplay(message) => Some(message.as_str()),
            _ => None,
        })
        .collect::<Vec<_>>()
        .join("\n");
    assert!(messages.contains("default archetype"));
    assert!(messages.contains("exceeds 90 percent"));
    assert!(messages.contains("regenerates missing initial resource"));
    assert!(messages.contains("cooldown exceeds"));
    assert!(messages.contains("resource \"missing_pool\" is unavailable"));
    assert!(messages.contains("status duration"));
    assert!(messages.contains("names must be unique ignoring case"));
    assert!(messages.contains("missing ability \"missing:ability\""));
    assert!(messages.contains("regenerates missing resource \"missing_pool\""));
}

#[test]
fn character_catalog_serialization_is_order_independent() {
    let left = WorldPackDocument::from_json(REVIEWED_WORLD).unwrap();
    let mut right = left.clone();
    right.world.gameplay.archetypes.reverse();
    right.world.gameplay.abilities.reverse();
    right.world.gameplay.npc_ability_sets.reverse();
    for archetype in &mut right.world.gameplay.archetypes {
        archetype.initial_ability_ids.reverse();
        archetype.level_progression.reverse();
        for level in &mut archetype.level_progression {
            level.ability_unlock_ids.reverse();
        }
    }
    for ability in &mut right.world.gameplay.abilities {
        ability.status_effects.reverse();
    }
    for set in &mut right.world.gameplay.npc_ability_sets {
        set.ability_ids.reverse();
    }

    assert_eq!(
        left.deterministic_artifact().unwrap(),
        right.deterministic_artifact().unwrap()
    );
}

#[test]
fn cumulative_progression_limits_count_unique_stats_and_resources_across_milestones() {
    let u32_gains = |prefix: &str, start: usize, end: usize| {
        (start..end)
            .map(|index| (format!("{prefix}_{index:02}"), 1_u32))
            .collect::<BTreeMap<_, _>>()
    };
    let u16_gains = |prefix: &str, start: usize, end: usize| {
        (start..end)
            .map(|index| (format!("{prefix}_{index:02}"), 1_u16))
            .collect::<BTreeMap<_, _>>()
    };

    let mut at_limit = reviewed_world();
    let archetype = &mut at_limit.gameplay.archetypes[0];
    archetype.initial_attributes = BTreeMap::from([
        ("defence".to_owned(), 1),
        ("max_health".to_owned(), 10),
        ("strength".to_owned(), 1),
    ]);
    archetype.initial_skills.clear();
    archetype.initial_resources.clear();
    archetype.resource_regeneration.clear();
    archetype.initial_ability_ids.clear();
    archetype.level_progression = vec![
        LevelProgressionDefinition {
            level: 2,
            total_experience_required: 100,
            attribute_gains: u32_gains("attribute", 0, 31),
            skill_gains: u16_gains("skill", 0, 32),
            resource_gains: u32_gains("resource", 0, 8),
            ability_unlock_ids: Vec::new(),
        },
        LevelProgressionDefinition {
            level: 3,
            total_experience_required: 300,
            attribute_gains: u32_gains("attribute", 31, 62),
            skill_gains: u16_gains("skill", 32, 64),
            resource_gains: u32_gains("resource", 8, 16),
            ability_unlock_ids: Vec::new(),
        },
    ];

    at_limit.validate().unwrap();
    assert_eq!(
        2 + at_limit.gameplay.archetypes[0]
            .level_progression
            .iter()
            .map(|level| level.attribute_gains.len())
            .sum::<usize>(),
        MAX_STATS_PER_ARCHETYPE
    );
    assert_eq!(
        at_limit.gameplay.archetypes[0]
            .level_progression
            .iter()
            .map(|level| level.skill_gains.len())
            .sum::<usize>(),
        MAX_STATS_PER_ARCHETYPE
    );
    assert_eq!(
        at_limit.gameplay.archetypes[0]
            .level_progression
            .iter()
            .map(|level| level.resource_gains.len())
            .sum::<usize>(),
        MAX_RESOURCES_PER_ARCHETYPE
    );

    let mut over_limit = at_limit;
    over_limit.gameplay.archetypes[0]
        .level_progression
        .push(LevelProgressionDefinition {
            level: 4,
            total_experience_required: 600,
            attribute_gains: u32_gains("attribute", 62, 63),
            skill_gains: u16_gains("skill", 64, 65),
            resource_gains: u32_gains("resource", 16, 17),
            ability_unlock_ids: Vec::new(),
        });

    let messages = invalid_gameplay_messages(&over_limit);
    assert!(messages.contains("can project more than 64 attributes"));
    assert!(messages.contains("can learn more than 64 skills"));
    assert!(messages.contains("can own more than 16 resources"));
}

#[test]
fn cumulative_resource_capacity_accepts_the_cap_and_rejects_excess_or_overflow() {
    let mut at_cap = reviewed_world();
    let arcanist = at_cap
        .gameplay
        .archetypes
        .iter_mut()
        .find(|archetype| archetype.id == "tiamud:arcanist")
        .unwrap();
    arcanist
        .initial_resources
        .insert("mana".to_owned(), MAX_GAMEPLAY_VALUE - 1);
    arcanist.level_progression[0].resource_gains = BTreeMap::from([("mana".to_owned(), 1)]);
    at_cap.validate().unwrap();

    let mut over_cap = at_cap.clone();
    over_cap
        .gameplay
        .archetypes
        .iter_mut()
        .find(|archetype| archetype.id == "tiamud:arcanist")
        .unwrap()
        .level_progression[0]
        .resource_gains = BTreeMap::from([("mana".to_owned(), 2)]);
    let messages = invalid_gameplay_messages(&over_cap);
    assert!(messages.contains(&format!(
        "resource \"mana\" exceeds {MAX_GAMEPLAY_VALUE} during progression"
    )));

    let mut overflowing = at_cap;
    let arcanist = overflowing
        .gameplay
        .archetypes
        .iter_mut()
        .find(|archetype| archetype.id == "tiamud:arcanist")
        .unwrap();
    arcanist
        .initial_resources
        .insert("mana".to_owned(), u32::MAX);
    arcanist.level_progression[0].resource_gains = BTreeMap::from([("mana".to_owned(), 1)]);
    let messages = invalid_gameplay_messages(&overflowing);
    assert!(messages.contains("resource \"mana\" overflows during progression"));
}

#[test]
fn max_health_is_valid_for_scaling_but_not_for_attribute_modifier_statuses() {
    let mut scaling = reviewed_world();
    let ability = scaling
        .gameplay
        .abilities
        .iter_mut()
        .find(|ability| ability.id == "tiamud:arcane-bolt")
        .unwrap();
    let damage = ability.damage.as_mut().unwrap();
    damage.scaling_attribute = Some("max_health".to_owned());
    damage.scaling_percent = 100;
    scaling.validate().unwrap();

    let ability = scaling
        .gameplay
        .abilities
        .iter_mut()
        .find(|ability| ability.id == "tiamud:arcane-bolt")
        .unwrap();
    ability
        .status_effects
        .push(AbilityStatusEffect::AttributeModifier {
            target: AbilityEffectTarget::Target,
            attribute: "max_health".to_owned(),
            magnitude: -1,
            duration_ticks: 2,
        });

    let messages = invalid_gameplay_messages(&scaling);
    assert!(messages.contains("has an invalid attribute modifier"));
    assert!(
        !messages.contains("scaling requires"),
        "max_health must remain a valid scaling attribute"
    );
}

fn invalid_gameplay_messages(pack: &WorldPack) -> String {
    pack.validate()
        .unwrap_err()
        .0
        .iter()
        .filter_map(|error| match error {
            ValidationError::InvalidGameplay(message) => Some(message.as_str()),
            _ => None,
        })
        .collect::<Vec<_>>()
        .join("\n")
}
