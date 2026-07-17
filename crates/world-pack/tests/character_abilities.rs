use std::collections::BTreeMap;

use serde_json::json;
use world_pack::{
    AbilityCategory, AbilityCommunicationDefinition, AbilityCommunicationKind,
    AbilityCommunicationScope, AbilityEffectTarget, AbilityStatusEffect, AbilityTargeting,
    ArchetypeDefinition, AreaPack, DamageType, GrantDefinition, LevelProgressionDefinition,
    MAX_ABILITY_COOLDOWN_TICKS, MAX_ABILITY_EFFECT_DURATION_TICKS, MAX_COMMUNICATION_ABILITIES,
    MAX_COMMUNICATION_BLOCKED_ROOMS, MAX_GAMEPLAY_VALUE, MAX_RESISTANCE_PERCENT,
    MAX_RESOURCES_PER_ARCHETYPE, MAX_SKILL_VALUE, MAX_STATS_PER_ARCHETYPE, ValidationError,
    WorldPack, WorldPackDocument,
};

const REVIEWED_WORLD: &[u8] = include_bytes!("../../../worlds/lantern-square.world.json");

fn reviewed_world() -> WorldPack {
    let mut pack = WorldPackDocument::from_json(REVIEWED_WORLD).unwrap().world;
    let progression = pack.gameplay.character_progression.take().unwrap();
    pack.gameplay.default_archetype_id = Some(progression.defaults.base_class_id);
    pack.gameplay.archetypes = progression
        .base_classes
        .into_iter()
        .map(|class| {
            let mut archetype = ArchetypeDefinition {
                id: class.id,
                name: class.name,
                description: class.description,
                initial_attributes: BTreeMap::new(),
                initial_skills: BTreeMap::new(),
                initial_resources: BTreeMap::new(),
                resource_regeneration: BTreeMap::new(),
                initial_resistances: BTreeMap::new(),
                initial_ability_ids: Vec::new(),
                level_progression: Vec::new(),
            };
            apply_legacy_grants(
                class.creation_grants,
                &mut archetype.initial_attributes,
                &mut archetype.initial_skills,
                &mut archetype.initial_resources,
                &mut archetype.resource_regeneration,
                &mut archetype.initial_resistances,
                &mut archetype.initial_ability_ids,
            );
            for milestone in class.level_grants {
                let mut attributes = BTreeMap::new();
                let mut skills = BTreeMap::new();
                let mut resources = BTreeMap::new();
                let mut regeneration = BTreeMap::new();
                let mut resistances = BTreeMap::new();
                let mut abilities = Vec::new();
                apply_legacy_grants(
                    milestone.grants,
                    &mut attributes,
                    &mut skills,
                    &mut resources,
                    &mut regeneration,
                    &mut resistances,
                    &mut abilities,
                );
                archetype
                    .level_progression
                    .push(LevelProgressionDefinition {
                        level: milestone.level,
                        total_experience_required: progression
                            .level_thresholds
                            .iter()
                            .find(|entry| entry.level == milestone.level)
                            .unwrap()
                            .total_experience_required,
                        attribute_gains: attributes,
                        skill_gains: skills,
                        resource_gains: resources,
                        ability_unlock_ids: abilities,
                    });
            }
            archetype
        })
        .collect();
    pack
}

#[allow(clippy::too_many_arguments)]
fn apply_legacy_grants(
    grants: Vec<GrantDefinition>,
    attributes: &mut BTreeMap<String, u32>,
    skills: &mut BTreeMap<String, u16>,
    resources: &mut BTreeMap<String, u32>,
    regeneration: &mut BTreeMap<String, u32>,
    resistances: &mut BTreeMap<DamageType, u16>,
    abilities: &mut Vec<String>,
) {
    for grant in grants {
        match grant {
            GrantDefinition::Attribute {
                attribute_id,
                amount,
            } => {
                attributes.insert(attribute_id, amount);
            }
            GrantDefinition::Skill { skill_id, amount } => {
                skills.insert(skill_id, amount);
            }
            GrantDefinition::ResourceMax {
                resource_id,
                amount,
            } => {
                resources.insert(resource_id, amount);
            }
            GrantDefinition::ResourceRegeneration {
                resource_id,
                amount,
            } => {
                regeneration.insert(resource_id, amount);
            }
            GrantDefinition::Ability { ability_id } => abilities.push(ability_id),
            GrantDefinition::Resistance {
                damage_type,
                percent,
            } => {
                resistances.insert(damage_type, percent);
            }
            GrantDefinition::Points { .. } | GrantDefinition::Tag { .. } => {}
        }
    }
}

#[test]
fn old_world_pack_v2_is_normalized_without_inventing_a_character_catalog() {
    let bytes = serde_json::to_vec(&json!({
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
    let pack = WorldPackDocument::from_json(&bytes).unwrap().world;

    pack.validate().unwrap();
    assert_eq!(pack.schema_version, world_pack::CURRENT_SCHEMA_VERSION);
    assert!(pack.gameplay.character_progression.is_none());
    assert!(pack.gameplay.default_archetype_id.is_none());
    assert!(pack.gameplay.archetypes.is_empty());
    assert!(pack.gameplay.abilities.is_empty());
    assert!(pack.gameplay.npc_ability_sets.is_empty());
}

#[test]
fn world_pack_v2_through_v7_normalize_without_inventing_communication_and_reject_smuggling() {
    for schema_version in 2..=7 {
        let legacy = json!({
            "schemaVersion": schema_version,
            "id": "legacy",
            "name": "Legacy",
            "startRoomId": "legacy:start",
            "rooms": [{
                "id": "legacy:start",
                "title": "Start",
                "description": "A legacy room."
            }],
            "gameplay": {
                "abilities": [{
                    "id": "legacy:strike",
                    "name": "Strike",
                    "description": "An ordinary legacy ability.",
                    "category": "martial",
                    "targeting": "single_enemy",
                    "damage": {
                        "target": "target",
                        "damageType": "physical",
                        "baseAmount": 1
                    }
                }]
            }
        });
        let normalized =
            WorldPackDocument::from_json(&serde_json::to_vec(&legacy).unwrap()).unwrap();
        assert_eq!(
            normalized.world.schema_version,
            world_pack::CURRENT_SCHEMA_VERSION
        );
        assert_eq!(normalized.world.gameplay.abilities.len(), 1);
        assert!(
            normalized.world.gameplay.abilities[0]
                .communication
                .is_none()
        );

        let mut smuggled = legacy;
        smuggled["gameplay"] = json!({
            "abilities": [{
                "id": "legacy:whisper",
                "name": "Whisper",
                "description": "A field from a later schema.",
                "category": "utility",
                "resourceCosts": {"stamina": 1},
                "cooldownTicks": 1,
                "targeting": "single_ally",
                "communication": {
                    "kind": "telepathy",
                    "scope": "direct_online",
                    "blockedRoomIds": []
                }
            }]
        });
        let error =
            WorldPackDocument::from_json(&serde_json::to_vec(&smuggled).unwrap()).unwrap_err();
        assert!(error.to_string().contains("cannot declare v8"), "{error}");
    }
}

fn communication_ability(pack: &WorldPack) -> world_pack::AbilityDefinition {
    let mut ability = pack.gameplay.abilities[0].clone();
    ability.id = "demo:whisper".to_owned();
    ability.name = "Whisper".to_owned();
    ability.category = AbilityCategory::Utility;
    ability.resource_costs = BTreeMap::from([("stamina".to_owned(), 1)]);
    ability.cooldown_ticks = 1;
    ability.targeting = AbilityTargeting::SingleAlly;
    ability.damage = None;
    ability.heal = None;
    ability.status_effects.clear();
    ability.communication = Some(AbilityCommunicationDefinition {
        kind: AbilityCommunicationKind::Telepathy,
        scope: AbilityCommunicationScope::DirectOnline,
        blocked_room_ids: vec![pack.rooms[0].id.clone()],
    });
    ability
}

#[test]
fn communication_abilities_enforce_closed_shape_bounds_rooms_and_npc_exclusion() {
    let mut valid = reviewed_world();
    let communication = communication_ability(&valid);
    valid.gameplay.abilities.push(communication.clone());
    valid.validate().unwrap();

    let mut invalid = valid.clone();
    let damage = invalid.gameplay.abilities[0].damage.clone();
    let communication = invalid.gameplay.abilities.last_mut().unwrap();
    communication.category = AbilityCategory::Martial;
    communication.targeting = AbilityTargeting::SelfOnly;
    communication.resource_costs.clear();
    communication.cooldown_ticks = 0;
    communication.damage = damage;
    communication
        .communication
        .as_mut()
        .unwrap()
        .blocked_room_ids = vec!["missing:room".to_owned(), "missing:room".to_owned()];
    invalid.gameplay.npc_ability_sets[0]
        .ability_ids
        .push("demo:whisper".to_owned());
    let messages = invalid
        .validate()
        .unwrap_err()
        .0
        .into_iter()
        .map(|error| error.to_string())
        .collect::<Vec<_>>()
        .join("\n");
    assert!(messages.contains("spend a resource"), "{messages}");
    assert!(messages.contains("repeats blocked room"), "{messages}");
    assert!(messages.contains("missing blocked room"), "{messages}");
    assert!(
        messages.contains("cannot use communication ability"),
        "{messages}"
    );

    let mut too_many_rooms = valid.clone();
    too_many_rooms
        .gameplay
        .abilities
        .last_mut()
        .unwrap()
        .communication
        .as_mut()
        .unwrap()
        .blocked_room_ids = (0..=MAX_COMMUNICATION_BLOCKED_ROOMS)
        .map(|index| format!("missing:{index}"))
        .collect();
    let messages = too_many_rooms
        .validate()
        .unwrap_err()
        .0
        .into_iter()
        .map(|error| error.to_string())
        .collect::<Vec<_>>()
        .join("\n");
    assert!(messages.contains("blocks 65 rooms"), "{messages}");

    let mut too_many_abilities = reviewed_world();
    let prototype = communication_ability(&too_many_abilities);
    for index in 0..=MAX_COMMUNICATION_ABILITIES {
        let mut ability = prototype.clone();
        ability.id = format!("demo:whisper-{index}");
        ability.name = format!("Whisper {index}");
        too_many_abilities.gameplay.abilities.push(ability);
    }
    let messages = too_many_abilities
        .validate()
        .unwrap_err()
        .0
        .into_iter()
        .map(|error| error.to_string())
        .collect::<Vec<_>>()
        .join("\n");
    assert!(
        messages.contains("communication abilities; the maximum is 32"),
        "{messages}"
    );
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
    let mut left = WorldPackDocument::from_json(REVIEWED_WORLD).unwrap();
    left.world
        .gameplay
        .abilities
        .iter_mut()
        .find(|ability| ability.id == "tiamud:mind-whisper")
        .unwrap()
        .communication
        .as_mut()
        .unwrap()
        .blocked_room_ids = vec!["demo:north-gate".to_owned(), "demo:archive".to_owned()];
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
        if let Some(communication) = &mut ability.communication {
            communication.blocked_room_ids.reverse();
        }
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
fn cumulative_attributes_and_skills_accept_the_caps_and_reject_excess_or_overflow() {
    let mut at_cap = reviewed_world();
    let archetype = &mut at_cap.gameplay.archetypes[0];
    archetype.initial_attributes = BTreeMap::from([
        ("agility".to_owned(), MAX_GAMEPLAY_VALUE - 2),
        ("defence".to_owned(), MAX_GAMEPLAY_VALUE - 2),
        ("max_health".to_owned(), MAX_GAMEPLAY_VALUE - 2),
        ("strength".to_owned(), MAX_GAMEPLAY_VALUE - 2),
    ]);
    archetype.initial_skills = BTreeMap::from([("combat".to_owned(), MAX_SKILL_VALUE - 2)]);
    archetype.initial_resources.clear();
    archetype.resource_regeneration.clear();
    archetype.initial_ability_ids.clear();
    archetype.level_progression = vec![
        LevelProgressionDefinition {
            level: 2,
            total_experience_required: 100,
            attribute_gains: BTreeMap::from([
                ("agility".to_owned(), 1),
                ("defence".to_owned(), 1),
                ("max_health".to_owned(), 1),
                ("strength".to_owned(), 1),
            ]),
            skill_gains: BTreeMap::from([("combat".to_owned(), 1)]),
            resource_gains: BTreeMap::new(),
            ability_unlock_ids: Vec::new(),
        },
        LevelProgressionDefinition {
            level: 3,
            total_experience_required: 300,
            attribute_gains: BTreeMap::from([
                ("agility".to_owned(), 1),
                ("defence".to_owned(), 1),
                ("max_health".to_owned(), 1),
                ("strength".to_owned(), 1),
            ]),
            skill_gains: BTreeMap::from([("combat".to_owned(), 1)]),
            resource_gains: BTreeMap::new(),
            ability_unlock_ids: Vec::new(),
        },
    ];
    at_cap.validate().unwrap();

    let mut over_cap = at_cap.clone();
    let last = &mut over_cap.gameplay.archetypes[0].level_progression[1];
    last.attribute_gains.values_mut().for_each(|gain| *gain = 2);
    last.skill_gains.insert("combat".to_owned(), 2);
    let messages = invalid_gameplay_messages(&over_cap);
    for attribute in ["agility", "defence", "max_health", "strength"] {
        assert!(messages.contains(&format!(
            "attribute \"{attribute}\" exceeds {MAX_GAMEPLAY_VALUE} during progression"
        )));
    }
    assert!(messages.contains(&format!(
        "skill \"combat\" exceeds {MAX_SKILL_VALUE} during progression"
    )));

    let mut overflowing = at_cap;
    let archetype = &mut overflowing.gameplay.archetypes[0];
    archetype
        .initial_attributes
        .insert("agility".to_owned(), u32::MAX);
    archetype
        .initial_skills
        .insert("combat".to_owned(), u16::MAX);
    let messages = invalid_gameplay_messages(&overflowing);
    assert!(messages.contains("attribute \"agility\" overflows during progression"));
    assert!(messages.contains("skill \"combat\" overflows during progression"));
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
