use serde_json::json;
use world_pack::{
    AreaPack, GrantDefinition, MAX_GAMEPLAY_VALUE, RequirementDefinition, RequirementDisclosure,
    RequirementPredicate, ValidationError, WorldPack, WorldPackDocument,
};

const REVIEWED_WORLD: &[u8] = include_bytes!("../../../worlds/lantern-square.world.json");

fn reviewed() -> WorldPack {
    WorldPackDocument::from_json(REVIEWED_WORLD).unwrap().world
}

fn invalid_messages(pack: &WorldPack) -> String {
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

#[test]
fn reviewed_v3_catalog_has_one_global_auditable_curve_and_initial_choices() {
    let pack = reviewed();
    pack.validate().unwrap();
    assert_eq!(pack.schema_version, 4);
    assert!(pack.gameplay.archetypes.is_empty());
    let progression = pack.gameplay.character_progression.unwrap();

    assert_eq!(
        progression
            .lineages
            .iter()
            .map(|entry| entry.name.as_str())
            .collect::<Vec<_>>(),
        ["Mortal", "Stonekin", "Embermarked"]
    );
    assert_eq!(progression.backgrounds.len(), 3);
    assert_eq!(progression.base_classes.len(), 3);
    assert_eq!(progression.advanced_classes.len(), 4);
    assert_eq!(progression.talent_graphs.len(), 6);
    assert!(
        progression
            .lineages
            .iter()
            .all(|lineage| lineage.talent_graph_ids.len() == 1)
    );
    assert!(progression.advanced_classes.iter().any(|class| {
        class.name == "Spellblade"
            && class.from_class_ids.len() == 2
            && class.requirements.iter().any(|requirement| {
                matches!(
                    requirement.predicate,
                    RequirementPredicate::Level { minimum: 6 }
                )
            })
    }));

    for threshold in &progression.level_thresholds {
        let previous = u64::from(threshold.level - 1);
        assert_eq!(
            threshold.total_experience_required,
            100 * previous * previous
        );
        assert_eq!(threshold.point_awards.get("talent"), Some(&1));
    }
}

#[test]
fn v2_archetypes_normalize_to_v3_without_guessing_a_second_xp_system() {
    let bytes = serde_json::to_vec(&json!({
        "schemaVersion": 2,
        "id": "legacy:world",
        "name": "Legacy",
        "startRoomId": "legacy:start",
        "rooms": [{"id":"legacy:start","title":"Start","description":"Start."}],
        "gameplay": {
            "defaultArchetypeId": "legacy:warden",
            "archetypes": [{
                "id": "legacy:warden",
                "name": "Warden",
                "description": "A legacy class.",
                "initialAttributes": {"max_health": 20},
                "initialAbilityIds": [],
                "levelProgression": [{
                    "level": 2,
                    "totalExperienceRequired": 100,
                    "attributeGains": {"max_health": 3}
                }]
            }]
        }
    }))
    .unwrap();

    let document = WorldPackDocument::from_json(&bytes).unwrap();
    document.validate().unwrap();
    assert_eq!(document.world.schema_version, 4);
    assert!(document.world.gameplay.archetypes.is_empty());
    let progression = document.world.gameplay.character_progression.unwrap();
    assert_eq!(progression.defaults.lineage_id, "legacy:mortal");
    assert_eq!(progression.defaults.base_class_id, "legacy:warden");
    assert_eq!(progression.level_thresholds.len(), 1);
    assert_eq!(progression.base_classes[0].level_grants[0].level, 2);
}

#[test]
fn v2_normalization_rejects_conflicting_per_class_xp_curves() {
    let mut value = json!({
        "schemaVersion": 2,
        "id": "legacy:world",
        "name": "Legacy",
        "startRoomId": "legacy:start",
        "rooms": [{"id":"legacy:start","title":"Start","description":"Start."}],
        "gameplay": {
            "defaultArchetypeId": "legacy:a",
            "archetypes": []
        }
    });
    value["gameplay"]["archetypes"] = json!([
        {"id":"legacy:a","name":"A","description":"A.","levelProgression":[{"level":2,"totalExperienceRequired":100}]},
        {"id":"legacy:b","name":"B","description":"B.","levelProgression":[{"level":2,"totalExperienceRequired":200}]}
    ]);
    let error = WorldPackDocument::from_json(&serde_json::to_vec(&value).unwrap()).unwrap_err();
    assert!(error.to_string().contains("disagree on cumulative XP"));
}

#[test]
fn area_v1_archetypes_have_a_bounded_import_path_to_native_v3_authoring() {
    let area: AreaPack = serde_json::from_value(json!({
        "schemaVersion": 1,
        "id": "legacy-area",
        "archetypes": [{
            "id": "legacy:scout",
            "name": "Scout",
            "description": "A scout.",
            "initialAttributes": {"max_health": 10},
            "levelProgression": [{"level":2,"totalExperienceRequired":100}]
        }]
    }))
    .unwrap();
    assert_eq!(area.schema_version, 3);
    assert!(area.archetypes.is_empty());
    assert_eq!(area.lineages[0].id, "legacy:mortal");
    assert_eq!(area.base_classes[0].id, "legacy:scout");
}

#[test]
fn progression_fails_closed_on_cycles_hidden_hints_and_missing_references() {
    let mut cyclic = reviewed();
    let progression = cyclic.gameplay.character_progression.as_mut().unwrap();
    let graph = &mut progression.talent_graphs[0];
    let target_node = graph.nodes[1].id.clone();
    graph.nodes[0].ranks[0]
        .requirements
        .push(RequirementDefinition {
            disclosure: RequirementDisclosure::Public,
            hint: None,
            predicate: RequirementPredicate::NodeRank {
                graph_id: graph.id.clone(),
                node_id: target_node,
                minimum_rank: 1,
            },
        });
    assert!(invalid_messages(&cyclic).contains("cycle"));

    let mut invalid = reviewed();
    let progression = invalid.gameplay.character_progression.as_mut().unwrap();
    progression.level_thresholds.remove(1);
    let advanced = &mut progression.advanced_classes[0];
    advanced.from_class_ids = vec!["missing:class".to_owned()];
    advanced.requirements.push(RequirementDefinition {
        disclosure: RequirementDisclosure::Hidden,
        hint: None,
        predicate: RequirementPredicate::CompletedQuest {
            quest_id: "tiamud:secret".to_owned(),
        },
    });
    advanced.promotion_grants.push(GrantDefinition::Ability {
        ability_id: "missing:ability".to_owned(),
    });
    let messages = invalid_messages(&invalid);
    assert!(messages.contains("contiguous levels"));
    assert!(messages.contains("invalid source class"));
    assert!(messages.contains("hidden requirement needs a safe player-facing hint"));
    assert!(messages.contains("missing ability"));
}

#[test]
fn progression_publication_caps_persistent_point_pools() {
    let mut pack = reviewed();
    let progression = pack.gameplay.character_progression.as_mut().unwrap();
    progression.level_thresholds[0].point_awards = [
        ("pool_a".to_owned(), 1),
        ("pool_b".to_owned(), 1),
        ("pool_c".to_owned(), 1),
        ("pool_d".to_owned(), 1),
        ("pool_e".to_owned(), 1),
    ]
    .into_iter()
    .collect();

    assert!(
        invalid_messages(&pack).contains("point pools; the maximum is 4"),
        "a valid authored pack must always fit the bounded durable character snapshot"
    );
}

#[test]
fn creation_requires_class_health_and_caps_cross_choice_totals() {
    let mut missing_health = reviewed();
    missing_health
        .gameplay
        .character_progression
        .as_mut()
        .unwrap()
        .base_classes[0]
        .creation_grants
        .retain(|grant| {
            !matches!(grant, GrantDefinition::Attribute { attribute_id, .. }
                if attribute_id == "max_health")
        });
    assert!(
        invalid_messages(&missing_health).contains("must grant positive max_health at creation")
    );

    let mut overflowing_combination = reviewed();
    overflowing_combination
        .gameplay
        .character_progression
        .as_mut()
        .unwrap()
        .lineages[0]
        .creation_grants
        .push(GrantDefinition::Attribute {
            attribute_id: "strength".into(),
            amount: MAX_GAMEPLAY_VALUE,
        });
    assert!(invalid_messages(&overflowing_combination).contains("creation combination lineage"));
}

#[test]
fn requirement_records_reject_unknown_fields_and_artifacts_canonicalize_ui_lists() {
    let malformed = serde_json::from_value::<RequirementDefinition>(json!({
        "kind": "level",
        "minimum": 5,
        "arbitraryExpression": "player.gold > 3"
    }));
    assert!(malformed.is_err());

    let left = WorldPackDocument::from_json(REVIEWED_WORLD).unwrap();
    let mut right = left.clone();
    let progression = right.world.gameplay.character_progression.as_mut().unwrap();
    progression.level_thresholds.reverse();
    progression.lineages.reverse();
    progression.backgrounds.reverse();
    progression.base_classes.reverse();
    progression.advanced_classes.reverse();
    progression.talent_graphs.reverse();
    for graph in &mut progression.talent_graphs {
        graph.nodes.reverse();
        for node in &mut graph.nodes {
            node.ranks.reverse();
        }
    }
    assert_eq!(
        left.deterministic_artifact().unwrap(),
        right.deterministic_artifact().unwrap()
    );
}
