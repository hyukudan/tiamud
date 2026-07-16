//! Closed, data-driven character creation and progression schema for WorldPack v3.
//!
//! The runtime deliberately receives bounded predicates and grants rather than
//! scripts. A single global XP curve advances one active class; talent points
//! and promotion choices are explicit authored data.

use std::collections::{BTreeMap, HashMap, HashSet, VecDeque};

use serde::{Deserialize, Deserializer, Serialize, de::Error as _};

use crate::{
    AbilityDefinition, ArchetypeDefinition, DamageType, MAX_ABILITIES_PER_ACTOR,
    MAX_GAMEPLAY_VALUE, MAX_PROGRESSION_LEVEL, MAX_RESISTANCE_PERCENT, MAX_RESOURCES_PER_ARCHETYPE,
    MAX_SKILL_VALUE, MAX_STATS_PER_ARCHETYPE, ValidationError,
};

/// Upgrade the v2 archetype-shaped character data into native v3 records.
/// Conflicting per-archetype XP thresholds are rejected: v3 has one global
/// curve, so guessing would change progression semantics.
pub(crate) fn normalize_v2_character_progression(
    archetypes: Vec<ArchetypeDefinition>,
    default_archetype_id: Option<String>,
) -> Result<Option<CharacterProgressionDefinition>, String> {
    if archetypes.is_empty() {
        return Ok(None);
    }
    let mut thresholds = BTreeMap::<u32, u64>::new();
    let mut base_classes = Vec::with_capacity(archetypes.len());
    for (ui_order, archetype) in archetypes.into_iter().enumerate() {
        let mut creation_grants = Vec::new();
        creation_grants.extend(archetype.initial_attributes.into_iter().map(
            |(attribute_id, amount)| GrantDefinition::Attribute {
                attribute_id,
                amount,
            },
        ));
        creation_grants.extend(
            archetype
                .initial_skills
                .into_iter()
                .map(|(skill_id, amount)| GrantDefinition::Skill { skill_id, amount }),
        );
        creation_grants.extend(archetype.initial_resources.into_iter().map(
            |(resource_id, amount)| GrantDefinition::ResourceMax {
                resource_id,
                amount,
            },
        ));
        creation_grants.extend(archetype.resource_regeneration.into_iter().map(
            |(resource_id, amount)| GrantDefinition::ResourceRegeneration {
                resource_id,
                amount,
            },
        ));
        creation_grants.extend(archetype.initial_resistances.into_iter().map(
            |(damage_type, percent)| GrantDefinition::Resistance {
                damage_type,
                percent,
            },
        ));
        creation_grants.extend(
            archetype
                .initial_ability_ids
                .into_iter()
                .map(|ability_id| GrantDefinition::Ability { ability_id }),
        );
        let mut level_grants = Vec::new();
        for milestone in archetype.level_progression {
            if let Some(previous) =
                thresholds.insert(milestone.level, milestone.total_experience_required)
                && previous != milestone.total_experience_required
            {
                return Err(format!(
                    "legacy archetypes disagree on cumulative XP for level {} ({previous} versus {})",
                    milestone.level, milestone.total_experience_required
                ));
            }
            let mut grants = Vec::new();
            grants.extend(
                milestone
                    .attribute_gains
                    .into_iter()
                    .map(|(attribute_id, amount)| GrantDefinition::Attribute {
                        attribute_id,
                        amount,
                    }),
            );
            grants.extend(
                milestone
                    .skill_gains
                    .into_iter()
                    .map(|(skill_id, amount)| GrantDefinition::Skill { skill_id, amount }),
            );
            grants.extend(
                milestone
                    .resource_gains
                    .into_iter()
                    .map(|(resource_id, amount)| GrantDefinition::ResourceMax {
                        resource_id,
                        amount,
                    }),
            );
            grants.extend(
                milestone
                    .ability_unlock_ids
                    .into_iter()
                    .map(|ability_id| GrantDefinition::Ability { ability_id }),
            );
            level_grants.push(LevelGrantDefinition {
                level: milestone.level,
                grants,
            });
        }
        base_classes.push(BaseClassDefinition {
            id: archetype.id,
            name: archetype.name,
            description: archetype.description,
            ui_order: ui_order as u16,
            creation_grants,
            level_grants,
            talent_graph_ids: Vec::new(),
        });
    }
    let base_class_id = default_archetype_id.unwrap_or_else(|| base_classes[0].id.clone());
    Ok(Some(CharacterProgressionDefinition {
        defaults: CharacterDefaultsDefinition {
            lineage_id: "legacy:mortal".to_owned(),
            background_id: None,
            base_class_id,
        },
        level_thresholds: thresholds
            .into_iter()
            .map(
                |(level, total_experience_required)| LevelThresholdDefinition {
                    level,
                    total_experience_required,
                    point_awards: BTreeMap::new(),
                },
            )
            .collect(),
        lineages: vec![LineageDefinition {
            id: "legacy:mortal".to_owned(),
            name: "Mortal".to_owned(),
            description: "Compatibility lineage assigned to imported WorldPack v2 characters."
                .to_owned(),
            ui_order: 0,
            creation_grants: Vec::new(),
            talent_graph_ids: Vec::new(),
        }],
        backgrounds: Vec::new(),
        base_classes,
        advanced_classes: Vec::new(),
        talent_graphs: Vec::new(),
    }))
}

// These publication limits are intentionally conservative. Every acquired
// choice becomes durable character state inside the 64 KiB snapshot budget,
// so accepting enormous graphs would make an otherwise valid pack impossible
// to persist safely.
pub const MAX_LINEAGES: usize = 16;
pub const MAX_BACKGROUNDS: usize = 16;
pub const MAX_BASE_CLASSES: usize = 32;
pub const MAX_ADVANCED_CLASSES: usize = 64;
pub const MAX_TALENT_GRAPHS: usize = 16;
pub const MAX_NODES_PER_GRAPH: usize = 32;
pub const MAX_RANKS_PER_NODE: usize = 5;
pub const MAX_PROGRESSION_POINT_POOLS: usize = 4;
pub const MAX_GRANTS_PER_ENTRY: usize = 32;
pub const MAX_REQUIREMENT_DEPTH: usize = 8;
pub const MAX_REQUIREMENT_NODES: usize = 128;
pub const MAX_UI_TEXT_BYTES: usize = 1_024;
pub const MAX_PROGRESSION_POINTS: u16 = 10_000;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CharacterProgressionDefinition {
    pub defaults: CharacterDefaultsDefinition,
    pub level_thresholds: Vec<LevelThresholdDefinition>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub lineages: Vec<LineageDefinition>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub backgrounds: Vec<BackgroundDefinition>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub base_classes: Vec<BaseClassDefinition>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub advanced_classes: Vec<AdvancedClassDefinition>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub talent_graphs: Vec<TalentGraphDefinition>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CharacterDefaultsDefinition {
    pub lineage_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub background_id: Option<String>,
    pub base_class_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct LevelThresholdDefinition {
    pub level: u32,
    pub total_experience_required: u64,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub point_awards: BTreeMap<String, u16>,
}

macro_rules! common_origin {
    ($name:ident) => {
        #[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
        #[serde(rename_all = "camelCase", deny_unknown_fields)]
        pub struct $name {
            pub id: String,
            pub name: String,
            pub description: String,
            pub ui_order: u16,
            #[serde(default, skip_serializing_if = "Vec::is_empty")]
            pub creation_grants: Vec<GrantDefinition>,
            #[serde(default, skip_serializing_if = "Vec::is_empty")]
            pub talent_graph_ids: Vec<String>,
        }
    };
}

common_origin!(LineageDefinition);
common_origin!(BackgroundDefinition);

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct BaseClassDefinition {
    pub id: String,
    pub name: String,
    pub description: String,
    pub ui_order: u16,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub creation_grants: Vec<GrantDefinition>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub level_grants: Vec<LevelGrantDefinition>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub talent_graph_ids: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AdvancedClassDefinition {
    pub id: String,
    pub name: String,
    pub description: String,
    pub ui_order: u16,
    pub from_class_ids: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub exclusive_group: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub requirements: Vec<RequirementDefinition>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub promotion_grants: Vec<GrantDefinition>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub level_grants: Vec<LevelGrantDefinition>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub talent_graph_ids: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct LevelGrantDefinition {
    pub level: u32,
    pub grants: Vec<GrantDefinition>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct TalentGraphDefinition {
    pub id: String,
    pub name: String,
    pub description: String,
    pub ui_order: u16,
    pub point_pool_id: String,
    pub nodes: Vec<TalentNodeDefinition>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct TalentNodeDefinition {
    pub id: String,
    pub name: String,
    pub description: String,
    pub ui_order: u16,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub exclusion_group: Option<String>,
    pub ranks: Vec<TalentRankDefinition>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct TalentRankDefinition {
    pub rank: u8,
    pub cost: u16,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub requirements: Vec<RequirementDefinition>,
    pub grants: Vec<GrantDefinition>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(
    tag = "kind",
    rename_all = "snake_case",
    rename_all_fields = "camelCase",
    deny_unknown_fields
)]
pub enum GrantDefinition {
    Attribute {
        attribute_id: String,
        amount: u32,
    },
    Skill {
        skill_id: String,
        amount: u16,
    },
    ResourceMax {
        resource_id: String,
        amount: u32,
    },
    ResourceRegeneration {
        resource_id: String,
        amount: u32,
    },
    Ability {
        ability_id: String,
    },
    Resistance {
        damage_type: DamageType,
        percent: u16,
    },
    Points {
        pool_id: String,
        amount: u16,
    },
    Tag {
        tag_id: String,
    },
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RequirementDisclosure {
    #[default]
    Public,
    Hidden,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RequirementDefinition {
    #[serde(default)]
    pub disclosure: RequirementDisclosure,
    /// A bounded, player-safe explanation; hidden predicates are never sent.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hint: Option<String>,
    #[serde(flatten)]
    pub predicate: RequirementPredicate,
}

impl<'de> Deserialize<'de> for RequirementDefinition {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        #[derive(Deserialize)]
        #[serde(rename_all = "camelCase")]
        struct Raw {
            #[serde(default)]
            disclosure: RequirementDisclosure,
            #[serde(default)]
            hint: Option<String>,
            #[serde(flatten)]
            predicate: RequirementPredicate,
        }
        let value = serde_json::Value::deserialize(deserializer)?;
        let object = value
            .as_object()
            .ok_or_else(|| D::Error::custom("requirement must be an object"))?;
        let kind = object
            .get("kind")
            .and_then(serde_json::Value::as_str)
            .ok_or_else(|| D::Error::custom("requirement kind is required"))?;
        let variant_fields: &[&str] = match kind {
            "level" => &["minimum"],
            "permanent_attribute" => &["attributeId", "minimum"],
            "skill" => &["skillId", "minimum"],
            "completed_quest" => &["questId"],
            "received_signal" => &["signalId"],
            "character_flag" | "world_flag" => &["flagId", "equals"],
            "lineage" => &["lineageId"],
            "current_class" | "historical_class" => &["classId"],
            "node_rank" => &["graphId", "nodeId", "minimumRank"],
            "ability" => &["abilityId"],
            "all" | "any" => &["requirements"],
            "not" => &["requirement"],
            other => {
                return Err(D::Error::custom(format!(
                    "unknown requirement kind {other:?}"
                )));
            }
        };
        for key in object.keys() {
            if !matches!(key.as_str(), "kind" | "disclosure" | "hint")
                && !variant_fields.contains(&key.as_str())
            {
                return Err(D::Error::custom(format!(
                    "unknown requirement field {key:?} for kind {kind:?}"
                )));
            }
        }
        let raw: Raw = serde_json::from_value(value).map_err(D::Error::custom)?;
        Ok(Self {
            disclosure: raw.disclosure,
            hint: raw.hint,
            predicate: raw.predicate,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(
    tag = "kind",
    rename_all = "snake_case",
    rename_all_fields = "camelCase"
)]
pub enum RequirementPredicate {
    Level {
        minimum: u32,
    },
    PermanentAttribute {
        attribute_id: String,
        minimum: u32,
    },
    Skill {
        skill_id: String,
        minimum: u16,
    },
    CompletedQuest {
        quest_id: String,
    },
    ReceivedSignal {
        signal_id: String,
    },
    CharacterFlag {
        flag_id: String,
        equals: bool,
    },
    WorldFlag {
        flag_id: String,
        equals: bool,
    },
    Lineage {
        lineage_id: String,
    },
    CurrentClass {
        class_id: String,
    },
    HistoricalClass {
        class_id: String,
    },
    NodeRank {
        graph_id: String,
        node_id: String,
        minimum_rank: u8,
    },
    Ability {
        ability_id: String,
    },
    All {
        requirements: Vec<RequirementDefinition>,
    },
    Any {
        requirements: Vec<RequirementDefinition>,
    },
    Not {
        requirement: Box<RequirementDefinition>,
    },
}

pub(crate) fn validate_character_progression(
    value: &CharacterProgressionDefinition,
    abilities: &[AbilityDefinition],
    errors: &mut Vec<ValidationError>,
) {
    let mut report = |message: String| errors.push(ValidationError::InvalidGameplay(message));
    if value.lineages.is_empty() || value.lineages.len() > MAX_LINEAGES {
        report(format!(
            "character progression must define 1..={MAX_LINEAGES} lineages"
        ));
    }
    if value.base_classes.is_empty() || value.base_classes.len() > MAX_BASE_CLASSES {
        report(format!(
            "character progression must define 1..={MAX_BASE_CLASSES} base classes"
        ));
    }
    if value.backgrounds.len() > MAX_BACKGROUNDS
        || value.advanced_classes.len() > MAX_ADVANCED_CLASSES
        || value.talent_graphs.len() > MAX_TALENT_GRAPHS
    {
        report("character progression collection limit exceeded".to_owned());
    }

    validate_level_curve(&value.level_thresholds, &mut report);
    let max_level = value
        .level_thresholds
        .iter()
        .map(|entry| entry.level)
        .max()
        .unwrap_or(1);
    let ability_ids = abilities
        .iter()
        .map(|v| v.id.as_str())
        .collect::<HashSet<_>>();
    let lineage_ids = collect_entity_ids(
        "lineage",
        value.lineages.iter().map(|v| (&v.id, v.ui_order)),
        &mut report,
    );
    let background_ids = collect_entity_ids(
        "background",
        value.backgrounds.iter().map(|v| (&v.id, v.ui_order)),
        &mut report,
    );
    let base_ids = collect_entity_ids(
        "base class",
        value.base_classes.iter().map(|v| (&v.id, v.ui_order)),
        &mut report,
    );
    let advanced_ids = collect_entity_ids(
        "advanced class",
        value.advanced_classes.iter().map(|v| (&v.id, v.ui_order)),
        &mut report,
    );
    let graph_ids = collect_entity_ids(
        "talent graph",
        value.talent_graphs.iter().map(|v| (&v.id, v.ui_order)),
        &mut report,
    );
    let class_ids = base_ids
        .union(&advanced_ids)
        .copied()
        .collect::<HashSet<_>>();
    let total_typed_ids = lineage_ids.len()
        + background_ids.len()
        + base_ids.len()
        + advanced_ids.len()
        + graph_ids.len();
    let globally_unique_ids = lineage_ids
        .iter()
        .chain(background_ids.iter())
        .chain(base_ids.iter())
        .chain(advanced_ids.iter())
        .chain(graph_ids.iter())
        .copied()
        .collect::<HashSet<_>>();
    if globally_unique_ids.len() != total_typed_ids {
        report(
            "lineage, background, class and talent graph ids must be globally unique".to_owned(),
        );
    }

    check_ref(
        "default lineage",
        &value.defaults.lineage_id,
        &lineage_ids,
        &mut report,
    );
    if let Some(id) = &value.defaults.background_id {
        check_ref("default background", id, &background_ids, &mut report);
    }
    check_ref(
        "default base class",
        &value.defaults.base_class_id,
        &base_ids,
        &mut report,
    );

    let mut resources = HashSet::new();
    for lineage in &value.lineages {
        validate_common(
            &lineage.name,
            &lineage.description,
            &format!("lineage {:?}", lineage.id),
            &mut report,
        );
        validate_grants(
            &lineage.creation_grants,
            &ability_ids,
            &mut resources,
            &format!("lineage {:?}", lineage.id),
            &mut report,
        );
        validate_graph_refs(
            &lineage.talent_graph_ids,
            &graph_ids,
            &format!("lineage {:?}", lineage.id),
            &mut report,
        );
    }
    for background in &value.backgrounds {
        validate_common(
            &background.name,
            &background.description,
            &format!("background {:?}", background.id),
            &mut report,
        );
        validate_grants(
            &background.creation_grants,
            &ability_ids,
            &mut resources,
            &format!("background {:?}", background.id),
            &mut report,
        );
        validate_graph_refs(
            &background.talent_graph_ids,
            &graph_ids,
            &format!("background {:?}", background.id),
            &mut report,
        );
    }
    for class in &value.base_classes {
        validate_common(
            &class.name,
            &class.description,
            &format!("base class {:?}", class.id),
            &mut report,
        );
        validate_grants(
            &class.creation_grants,
            &ability_ids,
            &mut resources,
            &format!("base class {:?}", class.id),
            &mut report,
        );
        if !class.creation_grants.iter().any(|grant| {
            matches!(grant, GrantDefinition::Attribute { attribute_id, amount }
                if attribute_id == "max_health" && *amount > 0)
        }) {
            report(format!(
                "base class {:?} must grant positive max_health at creation",
                class.id
            ));
        }
        validate_level_grants(
            &class.level_grants,
            max_level,
            &ability_ids,
            &mut resources,
            &format!("base class {:?}", class.id),
            &mut report,
        );
        validate_graph_refs(
            &class.talent_graph_ids,
            &graph_ids,
            &format!("base class {:?}", class.id),
            &mut report,
        );
    }

    validate_creation_combinations(value, &mut report);

    for class in &value.advanced_classes {
        let context = format!("advanced class {:?}", class.id);
        validate_common(&class.name, &class.description, &context, &mut report);
        if class.from_class_ids.is_empty() {
            report(format!("{context} must declare at least one source class"));
        }
        let mut sources = HashSet::new();
        for source in &class.from_class_ids {
            if !sources.insert(source.as_str()) {
                report(format!("{context} repeats source class {source:?}"));
            }
            if !class_ids.contains(source.as_str()) || source == &class.id {
                report(format!("{context} has invalid source class {source:?}"));
            }
        }
        if let Some(group) = &class.exclusive_group {
            validate_local_id(group, &format!("{context} exclusive group"), &mut report);
        }
        validate_grants(
            &class.promotion_grants,
            &ability_ids,
            &mut resources,
            &context,
            &mut report,
        );
        validate_level_grants(
            &class.level_grants,
            max_level,
            &ability_ids,
            &mut resources,
            &context,
            &mut report,
        );
        validate_graph_refs(&class.talent_graph_ids, &graph_ids, &context, &mut report);
    }
    validate_class_dag(&value.base_classes, &value.advanced_classes, &mut report);

    let awarded_point_pools = value
        .level_thresholds
        .iter()
        .flat_map(|entry| entry.point_awards.keys().map(String::as_str))
        .chain(all_grants(value).filter_map(|grant| match grant {
            GrantDefinition::Points { pool_id, .. } => Some(pool_id.as_str()),
            _ => None,
        }))
        .collect::<HashSet<_>>();
    if awarded_point_pools.len() > MAX_PROGRESSION_POINT_POOLS {
        report(format!(
            "character progression awards {} point pools; the maximum is {MAX_PROGRESSION_POINT_POOLS}",
            awarded_point_pools.len()
        ));
    }

    let mut node_ranks = HashMap::new();
    let mut node_orders = HashSet::new();
    let mut global_node_ids = HashSet::new();
    for graph in &value.talent_graphs {
        validate_common(
            &graph.name,
            &graph.description,
            &format!("talent graph {:?}", graph.id),
            &mut report,
        );
        validate_local_id(
            &graph.point_pool_id,
            &format!("talent graph {:?} point pool", graph.id),
            &mut report,
        );
        if !awarded_point_pools.contains(graph.point_pool_id.as_str()) {
            report(format!(
                "talent graph {:?} uses point pool {:?} that is never awarded",
                graph.id, graph.point_pool_id
            ));
        }
        if graph.nodes.is_empty() || graph.nodes.len() > MAX_NODES_PER_GRAPH {
            report(format!(
                "talent graph {:?} must contain 1..={MAX_NODES_PER_GRAPH} nodes",
                graph.id
            ));
        }
        node_orders.clear();
        let mut local_ids = HashSet::new();
        for node in &graph.nodes {
            if !valid_namespaced_id(&node.id) || !local_ids.insert(node.id.as_str()) {
                report(format!(
                    "talent graph {:?} has invalid or duplicate node id {:?}",
                    graph.id, node.id
                ));
            }
            if !global_node_ids.insert(node.id.as_str()) {
                report(format!(
                    "talent node id {:?} is not globally unique",
                    node.id
                ));
            }
            if !node_orders.insert(node.ui_order) {
                report(format!(
                    "talent graph {:?} repeats node uiOrder {}",
                    graph.id, node.ui_order
                ));
            }
            validate_common(
                &node.name,
                &node.description,
                &format!("talent node {:?}", node.id),
                &mut report,
            );
            if let Some(group) = &node.exclusion_group {
                validate_local_id(
                    group,
                    &format!("talent node {:?} exclusion group", node.id),
                    &mut report,
                );
            }
            if node.ranks.is_empty() || node.ranks.len() > MAX_RANKS_PER_NODE {
                report(format!(
                    "talent node {:?} must contain 1..={MAX_RANKS_PER_NODE} ranks",
                    node.id
                ));
            }
            let authored_ranks = node
                .ranks
                .iter()
                .map(|rank| rank.rank)
                .collect::<HashSet<_>>();
            for rank in &node.ranks {
                if rank.rank == 0
                    || rank.rank as usize > node.ranks.len()
                    || authored_ranks.len() != node.ranks.len()
                    || rank.cost == 0
                    || rank.cost > MAX_PROGRESSION_POINTS
                {
                    report(format!(
                        "talent node {:?} ranks must be contiguous and have a bounded positive cost",
                        node.id
                    ));
                }
            }
            node_ranks.insert(
                (graph.id.as_str(), node.id.as_str()),
                node.ranks.len() as u8,
            );
        }
    }

    for class in &value.advanced_classes {
        validate_requirements(
            &class.requirements,
            &lineage_ids,
            &class_ids,
            &graph_ids,
            &ability_ids,
            &node_ranks,
            &format!("advanced class {:?}", class.id),
            &mut report,
        );
    }

    let mut dependencies: HashMap<(&str, &str), HashSet<(&str, &str)>> = HashMap::new();
    for graph in &value.talent_graphs {
        for node in &graph.nodes {
            let key = (graph.id.as_str(), node.id.as_str());
            let entry = dependencies.entry(key).or_default();
            for rank in &node.ranks {
                let context = format!("talent node {:?} rank {}", node.id, rank.rank);
                validate_requirements(
                    &rank.requirements,
                    &lineage_ids,
                    &class_ids,
                    &graph_ids,
                    &ability_ids,
                    &node_ranks,
                    &context,
                    &mut report,
                );
                collect_node_dependencies(&rank.requirements, entry);
                if self_rank_requirement_is_invalid(
                    &rank.requirements,
                    graph.id.as_str(),
                    node.id.as_str(),
                    rank.rank,
                ) {
                    report(format!(
                        "{context} cannot require its own current or future rank"
                    ));
                }
                validate_grants(
                    &rank.grants,
                    &ability_ids,
                    &mut resources,
                    &context,
                    &mut report,
                );
            }
            entry.remove(&key);
        }
    }
    validate_node_dag(&dependencies, &mut report);

    // A regeneration grant without any capacity grant is unfulfillable. Ability
    // costs likewise must name a resource introduced by progression data.
    for grant in all_grants(value) {
        if let GrantDefinition::ResourceRegeneration { resource_id, .. } = grant
            && !resources.contains(resource_id.as_str())
        {
            report(format!(
                "resource regeneration references undeclared capacity {resource_id:?}"
            ));
        }
    }
    for ability in abilities {
        for resource in ability.resource_costs.keys() {
            if !resources.contains(resource.as_str()) {
                report(format!(
                    "ability {:?} costs resource {resource:?} not granted by character progression",
                    ability.id
                ));
            }
        }
    }
}

pub(crate) fn canonicalize_character_progression(value: &mut CharacterProgressionDefinition) {
    value.level_thresholds.sort_by_key(|entry| entry.level);
    value
        .lineages
        .sort_by(|a, b| a.ui_order.cmp(&b.ui_order).then(a.id.cmp(&b.id)));
    value
        .backgrounds
        .sort_by(|a, b| a.ui_order.cmp(&b.ui_order).then(a.id.cmp(&b.id)));
    value
        .base_classes
        .sort_by(|a, b| a.ui_order.cmp(&b.ui_order).then(a.id.cmp(&b.id)));
    value
        .advanced_classes
        .sort_by(|a, b| a.ui_order.cmp(&b.ui_order).then(a.id.cmp(&b.id)));
    value
        .talent_graphs
        .sort_by(|a, b| a.ui_order.cmp(&b.ui_order).then(a.id.cmp(&b.id)));
    for lineage in &mut value.lineages {
        lineage.talent_graph_ids.sort();
    }
    for background in &mut value.backgrounds {
        background.talent_graph_ids.sort();
    }
    for class in &mut value.base_classes {
        class.level_grants.sort_by_key(|entry| entry.level);
        class.talent_graph_ids.sort();
    }
    for class in &mut value.advanced_classes {
        class.from_class_ids.sort();
        class.level_grants.sort_by_key(|entry| entry.level);
        class.talent_graph_ids.sort();
    }
    for graph in &mut value.talent_graphs {
        graph
            .nodes
            .sort_by(|a, b| a.ui_order.cmp(&b.ui_order).then(a.id.cmp(&b.id)));
        for node in &mut graph.nodes {
            node.ranks.sort_by_key(|rank| rank.rank);
        }
    }
}

fn validate_level_curve(entries: &[LevelThresholdDefinition], report: &mut impl FnMut(String)) {
    if entries.is_empty() {
        report("global level threshold curve must not be empty".to_owned());
        return;
    }
    let mut entries = entries.iter().collect::<Vec<_>>();
    entries.sort_by_key(|entry| entry.level);
    let mut previous = 0;
    for (index, entry) in entries.into_iter().enumerate() {
        let expected = index as u32 + 2;
        if entry.level != expected
            || entry.level > MAX_PROGRESSION_LEVEL
            || entry.total_experience_required <= previous
        {
            report(format!(
                "global XP curve must contain contiguous levels from 2 with strictly increasing cumulative thresholds; invalid level {}",
                entry.level
            ));
        }
        previous = entry.total_experience_required;
        for (pool, amount) in &entry.point_awards {
            validate_local_id(pool, "level point award pool", report);
            if *amount == 0 || *amount > MAX_PROGRESSION_POINTS {
                report(format!(
                    "level {} point award for {pool:?} is not bounded and positive",
                    entry.level
                ));
            }
        }
    }
}

fn collect_entity_ids<'a>(
    kind: &str,
    values: impl Iterator<Item = (&'a String, u16)>,
    report: &mut impl FnMut(String),
) -> HashSet<&'a str> {
    let mut ids = HashSet::new();
    let mut orders = HashSet::new();
    for (id, order) in values {
        if !valid_namespaced_id(id) || !ids.insert(id.as_str()) {
            report(format!("{kind} id {id:?} is invalid or duplicated"));
        }
        if !orders.insert(order) {
            report(format!("{kind} uiOrder {order} is duplicated"));
        }
    }
    ids
}

fn validate_common(name: &str, description: &str, context: &str, report: &mut impl FnMut(String)) {
    for (field, value) in [("name", name), ("description", description)] {
        if value.trim().is_empty()
            || value.len() > MAX_UI_TEXT_BYTES
            || value.chars().any(char::is_control)
        {
            report(format!("{context} has invalid {field}"));
        }
    }
}

fn validate_level_grants(
    entries: &[LevelGrantDefinition],
    max_level: u32,
    abilities: &HashSet<&str>,
    resources: &mut HashSet<String>,
    context: &str,
    report: &mut impl FnMut(String),
) {
    let mut levels = HashSet::new();
    for entry in entries {
        if entry.level < 2 || entry.level > max_level || !levels.insert(entry.level) {
            report(format!(
                "{context} has invalid or duplicate automatic grant level {}",
                entry.level
            ));
        }
        validate_grants(
            &entry.grants,
            abilities,
            resources,
            &format!("{context} level {}", entry.level),
            report,
        );
    }
}

fn validate_grants(
    grants: &[GrantDefinition],
    abilities: &HashSet<&str>,
    resources: &mut HashSet<String>,
    context: &str,
    report: &mut impl FnMut(String),
) {
    if grants.len() > MAX_GRANTS_PER_ENTRY {
        report(format!("{context} exceeds {MAX_GRANTS_PER_ENTRY} grants"));
    }
    let mut numeric_totals = HashMap::<(&str, &str), u64>::new();
    let mut resistance_totals = BTreeMap::<DamageType, u64>::new();
    let mut unique_abilities = HashSet::new();
    let mut unique_tags = HashSet::new();
    for grant in grants {
        match grant {
            GrantDefinition::Attribute {
                attribute_id,
                amount,
            } => {
                validate_local_id(attribute_id, context, report);
                accumulate_grant(
                    &mut numeric_totals,
                    ("attribute", attribute_id),
                    u64::from(*amount),
                    u64::from(MAX_GAMEPLAY_VALUE),
                    context,
                    report,
                );
            }
            GrantDefinition::Skill { skill_id, amount } => {
                validate_local_id(skill_id, context, report);
                accumulate_grant(
                    &mut numeric_totals,
                    ("skill", skill_id),
                    u64::from(*amount),
                    u64::from(MAX_SKILL_VALUE),
                    context,
                    report,
                );
            }
            GrantDefinition::ResourceMax {
                resource_id,
                amount,
            } => {
                validate_local_id(resource_id, context, report);
                accumulate_grant(
                    &mut numeric_totals,
                    ("resource_max", resource_id),
                    u64::from(*amount),
                    u64::from(MAX_GAMEPLAY_VALUE),
                    context,
                    report,
                );
                if *amount > 0 {
                    resources.insert(resource_id.clone());
                }
            }
            GrantDefinition::ResourceRegeneration {
                resource_id,
                amount,
            } => {
                validate_local_id(resource_id, context, report);
                accumulate_grant(
                    &mut numeric_totals,
                    ("resource_regeneration", resource_id),
                    u64::from(*amount),
                    u64::from(MAX_GAMEPLAY_VALUE),
                    context,
                    report,
                );
            }
            GrantDefinition::Ability { ability_id } => {
                if !abilities.contains(ability_id.as_str()) {
                    report(format!(
                        "{context} references missing ability {ability_id:?}"
                    ));
                }
                if !unique_abilities.insert(ability_id) {
                    report(format!("{context} repeats ability grant {ability_id:?}"));
                }
            }
            GrantDefinition::Resistance {
                damage_type,
                percent,
            } => {
                let total = resistance_totals.entry(*damage_type).or_default();
                *total += u64::from(*percent);
                if *percent == 0 || *total > u64::from(MAX_RESISTANCE_PERCENT) {
                    report(format!(
                        "{context} has cumulative resistance outside 1..={MAX_RESISTANCE_PERCENT}"
                    ));
                }
            }
            GrantDefinition::Points { pool_id, amount } => {
                validate_local_id(pool_id, context, report);
                accumulate_grant(
                    &mut numeric_totals,
                    ("points", pool_id),
                    u64::from(*amount),
                    u64::from(MAX_PROGRESSION_POINTS),
                    context,
                    report,
                );
            }
            GrantDefinition::Tag { tag_id } => {
                validate_namespaced_id(tag_id, context, report);
                if !unique_tags.insert(tag_id) {
                    report(format!("{context} repeats tag grant {tag_id:?}"));
                }
            }
        }
    }
}

fn validate_creation_combinations(
    value: &CharacterProgressionDefinition,
    report: &mut impl FnMut(String),
) {
    let backgrounds = std::iter::once(None)
        .chain(value.backgrounds.iter().map(Some))
        .collect::<Vec<_>>();
    'combinations: for lineage in &value.lineages {
        for background in &backgrounds {
            for class in &value.base_classes {
                let mut grants = lineage.creation_grants.iter().chain(&class.creation_grants);
                let mut combined = Vec::new();
                combined.extend(grants.by_ref());
                if let Some(background) = background {
                    combined.extend(&background.creation_grants);
                }
                if let Some(reason) = invalid_creation_combination(&combined) {
                    report(format!(
                        "creation combination lineage {:?}, background {:?}, base class {:?} is invalid: {reason}",
                        lineage.id,
                        background.map(|entry| entry.id.as_str()),
                        class.id
                    ));
                    break 'combinations;
                }
            }
        }
    }
}

fn invalid_creation_combination(grants: &[&GrantDefinition]) -> Option<String> {
    let mut attributes = HashMap::<&str, u64>::new();
    let mut skills = HashMap::<&str, u64>::new();
    let mut resource_max = HashMap::<&str, u64>::new();
    let mut resource_regeneration = HashMap::<&str, u64>::new();
    let mut points = HashMap::<&str, u64>::new();
    let mut resistances = BTreeMap::<DamageType, u64>::new();
    let mut abilities = HashSet::new();
    for grant in grants {
        match grant {
            GrantDefinition::Attribute {
                attribute_id,
                amount,
            } => {
                let total = attributes.entry(attribute_id).or_default();
                *total = total.saturating_add(u64::from(*amount));
            }
            GrantDefinition::Skill { skill_id, amount } => {
                let total = skills.entry(skill_id).or_default();
                *total = total.saturating_add(u64::from(*amount));
            }
            GrantDefinition::ResourceMax {
                resource_id,
                amount,
            } => {
                let total = resource_max.entry(resource_id).or_default();
                *total = total.saturating_add(u64::from(*amount));
            }
            GrantDefinition::ResourceRegeneration {
                resource_id,
                amount,
            } => {
                let total = resource_regeneration.entry(resource_id).or_default();
                *total = total.saturating_add(u64::from(*amount));
            }
            GrantDefinition::Ability { ability_id } => {
                abilities.insert(ability_id.as_str());
            }
            GrantDefinition::Resistance {
                damage_type,
                percent,
            } => {
                let total = resistances.entry(*damage_type).or_default();
                *total = total.saturating_add(u64::from(*percent));
            }
            GrantDefinition::Points { pool_id, amount } => {
                let total = points.entry(pool_id).or_default();
                *total = total.saturating_add(u64::from(*amount));
            }
            GrantDefinition::Tag { .. } => {}
        }
    }
    if attributes.get("max_health").copied().unwrap_or(0) == 0 {
        return Some("max_health must be positive".to_owned());
    }
    if attributes
        .values()
        .any(|total| *total > u64::from(MAX_GAMEPLAY_VALUE))
    {
        return Some("an attribute exceeds the gameplay cap".to_owned());
    }
    let durable_attribute_count = attributes.keys().filter(|id| **id != "max_health").count();
    if durable_attribute_count > MAX_STATS_PER_ARCHETYPE {
        return Some("too many attributes".to_owned());
    }
    if skills.len() > MAX_STATS_PER_ARCHETYPE
        || skills
            .values()
            .any(|total| *total > u64::from(MAX_SKILL_VALUE))
    {
        return Some("skills exceed durable snapshot bounds".to_owned());
    }
    if resource_max.len() > MAX_RESOURCES_PER_ARCHETYPE
        || resource_max
            .values()
            .any(|total| *total > u64::from(MAX_GAMEPLAY_VALUE))
        || resource_regeneration.iter().any(|(id, total)| {
            !resource_max.contains_key(id) || *total > u64::from(MAX_GAMEPLAY_VALUE)
        })
    {
        return Some("resources exceed durable snapshot bounds".to_owned());
    }
    if abilities.len() > MAX_ABILITIES_PER_ACTOR {
        return Some("too many initial abilities".to_owned());
    }
    if resistances
        .values()
        .any(|total| *total > u64::from(MAX_RESISTANCE_PERCENT))
    {
        return Some("a resistance exceeds the gameplay cap".to_owned());
    }
    if points.len() > MAX_PROGRESSION_POINT_POOLS
        || points
            .values()
            .any(|total| *total > u64::from(MAX_PROGRESSION_POINTS))
    {
        return Some("progression points exceed durable snapshot bounds".to_owned());
    }
    None
}

fn accumulate_grant<'a>(
    totals: &mut HashMap<(&'static str, &'a str), u64>,
    key: (&'static str, &'a str),
    amount: u64,
    maximum: u64,
    context: &str,
    report: &mut impl FnMut(String),
) {
    let total = totals.entry(key).or_default();
    *total = total.saturating_add(amount);
    if amount == 0 || *total > maximum {
        report(format!(
            "{context} has invalid or overflowing cumulative {} grant for {:?}",
            key.0, key.1
        ));
    }
}

#[allow(clippy::too_many_arguments)]
fn validate_requirements(
    requirements: &[RequirementDefinition],
    lineages: &HashSet<&str>,
    classes: &HashSet<&str>,
    graphs: &HashSet<&str>,
    abilities: &HashSet<&str>,
    node_ranks: &HashMap<(&str, &str), u8>,
    context: &str,
    report: &mut impl FnMut(String),
) {
    let mut nodes = 0;
    for requirement in requirements {
        validate_requirement(
            requirement,
            1,
            &mut nodes,
            lineages,
            classes,
            graphs,
            abilities,
            node_ranks,
            context,
            report,
        );
    }
}

#[allow(clippy::too_many_arguments)]
fn validate_requirement(
    requirement: &RequirementDefinition,
    depth: usize,
    nodes: &mut usize,
    lineages: &HashSet<&str>,
    classes: &HashSet<&str>,
    graphs: &HashSet<&str>,
    abilities: &HashSet<&str>,
    node_ranks: &HashMap<(&str, &str), u8>,
    context: &str,
    report: &mut impl FnMut(String),
) {
    *nodes += 1;
    if depth > MAX_REQUIREMENT_DEPTH || *nodes > MAX_REQUIREMENT_NODES {
        report(format!(
            "{context} requirement expression exceeds structural limits"
        ));
        return;
    }
    if requirement.disclosure == RequirementDisclosure::Hidden
        && requirement
            .hint
            .as_ref()
            .is_none_or(|v| v.trim().is_empty())
    {
        report(format!(
            "{context} hidden requirement needs a safe player-facing hint"
        ));
    }
    if let Some(hint) = &requirement.hint
        && (hint.len() > 200 || hint.chars().any(char::is_control))
    {
        report(format!("{context} has invalid requirement hint"));
    }
    match &requirement.predicate {
        RequirementPredicate::Level { minimum } => {
            if *minimum == 0 || *minimum > MAX_PROGRESSION_LEVEL {
                report(format!("{context} has invalid level requirement"));
            }
        }
        RequirementPredicate::PermanentAttribute {
            attribute_id,
            minimum,
        } => {
            validate_local_id(attribute_id, context, report);
            if *minimum == 0 || *minimum > MAX_GAMEPLAY_VALUE {
                report(format!("{context} has invalid attribute requirement"));
            }
        }
        RequirementPredicate::Skill { skill_id, minimum } => {
            validate_local_id(skill_id, context, report);
            if *minimum == 0 || *minimum > MAX_SKILL_VALUE {
                report(format!("{context} has invalid skill requirement"));
            }
        }
        RequirementPredicate::CompletedQuest { quest_id } => {
            validate_namespaced_id(quest_id, context, report)
        }
        RequirementPredicate::ReceivedSignal { signal_id } => {
            validate_namespaced_id(signal_id, context, report)
        }
        RequirementPredicate::CharacterFlag { flag_id, .. }
        | RequirementPredicate::WorldFlag { flag_id, .. } => {
            // Script flags are character-local runtime keys, not published
            // content identities. They intentionally use the existing safe
            // lowercase flag grammar and therefore cannot contain `:`.
            validate_local_id(flag_id, context, report)
        }
        RequirementPredicate::Lineage { lineage_id } => {
            check_ref(context, lineage_id, lineages, report)
        }
        RequirementPredicate::CurrentClass { class_id }
        | RequirementPredicate::HistoricalClass { class_id } => {
            check_ref(context, class_id, classes, report)
        }
        RequirementPredicate::NodeRank {
            graph_id,
            node_id,
            minimum_rank,
        } => {
            check_ref(context, graph_id, graphs, report);
            match node_ranks.get(&(graph_id.as_str(), node_id.as_str())) {
                Some(max) if *minimum_rank > 0 && minimum_rank <= max => {}
                _ => report(format!(
                    "{context} references invalid node rank {graph_id:?}/{node_id:?}:{minimum_rank}"
                )),
            }
        }
        RequirementPredicate::Ability { ability_id } => {
            check_ref(context, ability_id, abilities, report)
        }
        RequirementPredicate::All { requirements } | RequirementPredicate::Any { requirements } => {
            if requirements.is_empty() || requirements.len() > 32 {
                report(format!(
                    "{context} all/any requirement must have 1..=32 children"
                ));
            }
            for child in requirements {
                validate_requirement(
                    child,
                    depth + 1,
                    nodes,
                    lineages,
                    classes,
                    graphs,
                    abilities,
                    node_ranks,
                    context,
                    report,
                );
            }
        }
        RequirementPredicate::Not { requirement } => validate_requirement(
            requirement,
            depth + 1,
            nodes,
            lineages,
            classes,
            graphs,
            abilities,
            node_ranks,
            context,
            report,
        ),
    }
}

fn collect_node_dependencies<'a>(
    requirements: &'a [RequirementDefinition],
    output: &mut HashSet<(&'a str, &'a str)>,
) {
    for requirement in requirements {
        match &requirement.predicate {
            RequirementPredicate::NodeRank {
                graph_id, node_id, ..
            } => {
                output.insert((graph_id, node_id));
            }
            RequirementPredicate::All { requirements }
            | RequirementPredicate::Any { requirements } => {
                collect_node_dependencies(requirements, output)
            }
            RequirementPredicate::Not { requirement } => {
                collect_node_dependencies(std::slice::from_ref(requirement), output)
            }
            _ => {}
        }
    }
}

fn self_rank_requirement_is_invalid(
    requirements: &[RequirementDefinition],
    graph_id: &str,
    node_id: &str,
    current_rank: u8,
) -> bool {
    requirements
        .iter()
        .any(|requirement| match &requirement.predicate {
            RequirementPredicate::NodeRank {
                graph_id: required_graph,
                node_id: required_node,
                minimum_rank,
            } => {
                required_graph == graph_id
                    && required_node == node_id
                    && *minimum_rank >= current_rank
            }
            RequirementPredicate::All { requirements }
            | RequirementPredicate::Any { requirements } => {
                self_rank_requirement_is_invalid(requirements, graph_id, node_id, current_rank)
            }
            RequirementPredicate::Not { requirement } => self_rank_requirement_is_invalid(
                std::slice::from_ref(requirement),
                graph_id,
                node_id,
                current_rank,
            ),
            _ => false,
        })
}

fn validate_node_dag<'a>(
    dependencies: &HashMap<(&'a str, &'a str), HashSet<(&'a str, &'a str)>>,
    report: &mut impl FnMut(String),
) {
    let mut indegree = dependencies
        .keys()
        .map(|key| (*key, 0_usize))
        .collect::<HashMap<_, _>>();
    let mut dependents: HashMap<(&str, &str), Vec<(&str, &str)>> = HashMap::new();
    for (node, prerequisites) in dependencies {
        for prerequisite in prerequisites {
            if dependencies.contains_key(prerequisite) {
                *indegree.entry(*node).or_default() += 1;
                dependents.entry(*prerequisite).or_default().push(*node);
            }
        }
    }
    let mut queue = indegree
        .iter()
        .filter_map(|(node, degree)| (*degree == 0).then_some(*node))
        .collect::<VecDeque<_>>();
    let mut visited = 0;
    while let Some(node) = queue.pop_front() {
        visited += 1;
        for dependent in dependents.get(&node).into_iter().flatten() {
            let degree = indegree.get_mut(dependent).expect("known node");
            *degree -= 1;
            if *degree == 0 {
                queue.push_back(*dependent);
            }
        }
    }
    if visited != dependencies.len() {
        report("talent node prerequisites contain a cycle and make nodes unreachable".to_owned());
    }
}

fn validate_class_dag(
    base_classes: &[BaseClassDefinition],
    advanced_classes: &[AdvancedClassDefinition],
    report: &mut impl FnMut(String),
) {
    let base_ids = base_classes
        .iter()
        .map(|class| class.id.as_str())
        .collect::<HashSet<_>>();
    let mut reachable = base_ids.clone();
    let mut remaining = advanced_classes
        .iter()
        .map(|class| class.id.as_str())
        .collect::<HashSet<_>>();
    loop {
        let newly_reachable = advanced_classes
            .iter()
            .filter(|class| {
                remaining.contains(class.id.as_str())
                    && class
                        .from_class_ids
                        .iter()
                        .any(|source| reachable.contains(source.as_str()))
            })
            .map(|class| class.id.as_str())
            .collect::<Vec<_>>();
        if newly_reachable.is_empty() {
            break;
        }
        for id in newly_reachable {
            remaining.remove(id);
            reachable.insert(id);
        }
    }
    if !remaining.is_empty() {
        let mut ids = remaining.into_iter().collect::<Vec<_>>();
        ids.sort_unstable();
        report(format!(
            "advanced class promotion graph contains a cycle or classes unreachable from any base class: {}",
            ids.join(", ")
        ));
    }
}

fn validate_graph_refs(
    ids: &[String],
    known: &HashSet<&str>,
    context: &str,
    report: &mut impl FnMut(String),
) {
    let mut unique = HashSet::new();
    for id in ids {
        if !unique.insert(id.as_str()) {
            report(format!("{context} repeats talent graph {id:?}"));
        }
        check_ref(context, id, known, report);
    }
}

fn check_ref(context: &str, id: &str, known: &HashSet<&str>, report: &mut impl FnMut(String)) {
    if !known.contains(id) {
        report(format!("{context} references missing id {id:?}"));
    }
}
fn validate_namespaced_id(id: &str, context: &str, report: &mut impl FnMut(String)) {
    if !valid_namespaced_id(id) {
        report(format!(
            "{context} contains invalid stable namespaced id {id:?}"
        ));
    }
}
fn validate_local_id(id: &str, context: &str, report: &mut impl FnMut(String)) {
    if id.is_empty()
        || id.len() > 64
        || !id.bytes().all(|c| {
            c.is_ascii_lowercase() || c.is_ascii_digit() || matches!(c, b'_' | b'-' | b'.')
        })
    {
        report(format!("{context} contains invalid lowercase id {id:?}"));
    }
}
fn valid_namespaced_id(id: &str) -> bool {
    let mut parts = id.split(':');
    matches!((parts.next(), parts.next(), parts.next()), (Some(namespace), Some(local), None) if !namespace.is_empty() && !local.is_empty() && id.len() <= 96 && id.bytes().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || matches!(c, b':' | b'_' | b'-' | b'.')))
}

fn all_grants(value: &CharacterProgressionDefinition) -> impl Iterator<Item = &GrantDefinition> {
    value
        .lineages
        .iter()
        .flat_map(|v| v.creation_grants.iter())
        .chain(
            value
                .backgrounds
                .iter()
                .flat_map(|v| v.creation_grants.iter()),
        )
        .chain(value.base_classes.iter().flat_map(|v| {
            v.creation_grants
                .iter()
                .chain(v.level_grants.iter().flat_map(|m| m.grants.iter()))
        }))
        .chain(value.advanced_classes.iter().flat_map(|v| {
            v.promotion_grants
                .iter()
                .chain(v.level_grants.iter().flat_map(|m| m.grants.iter()))
        }))
        .chain(value.talent_graphs.iter().flat_map(|g| {
            g.nodes
                .iter()
                .flat_map(|n| n.ranks.iter().flat_map(|r| r.grants.iter()))
        }))
}
