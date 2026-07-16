//! Bounded, transport-independent character creation and progression runtime.
//!
//! Authoring formats are compiled into these types by the server. The core
//! deliberately owns progression semantics so every transport observes the
//! same atomic promotion, talent and level transitions.

use serde::Serialize;
use std::collections::{BTreeMap, BTreeSet};

use crate::{
    AbilityDefinition, CharacterGameplaySnapshot, DamageType, MAX_KNOWN_ABILITIES,
    MAX_RESISTANCE_PERCENT, MAX_RESOURCE_POOLS, MAX_RESOURCE_VALUE, MAX_SKILL_VALUE,
    MAX_STAT_VALUE, QuestStatus, ResourcePool, WorldError, humanize_id, valid_definition_id,
    valid_stat_id,
};

pub const MAX_LINEAGES: usize = 16;
pub const MAX_BACKGROUNDS: usize = 16;
pub const MAX_BASE_CLASSES: usize = 32;
pub const MAX_ADVANCED_CLASSES: usize = 64;
pub const MAX_TALENT_GRAPHS: usize = 16;
pub const MAX_NODES_PER_GRAPH: usize = 32;
pub const MAX_RANKS_PER_NODE: usize = 5;
pub const MAX_GRANTS_PER_ENTRY: usize = 32;
pub const MAX_REQUIREMENT_DEPTH: usize = 8;
pub const MAX_REQUIREMENT_NODES: usize = 128;
pub const MAX_PROGRESSION_POINTS: u16 = 10_000;
pub const MAX_PROGRESSION_POINT_POOLS: usize = 4;
pub const MAX_SELECTED_TALENT_NODES: usize = 32;
pub const MAX_CLASS_HISTORY: usize = 8;
pub const MAX_PROGRESSION_TAGS: usize = 64;
pub const MAX_RECEIVED_SIGNALS: usize = 64;
const MAX_REQUIREMENT_TEXT_BYTES: usize = 1_024;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CharacterProgressionDefinition {
    pub defaults: CharacterDefaultsDefinition,
    pub level_thresholds: Vec<LevelThresholdDefinition>,
    pub lineages: Vec<LineageDefinition>,
    pub backgrounds: Vec<BackgroundDefinition>,
    pub base_classes: Vec<BaseClassDefinition>,
    pub advanced_classes: Vec<AdvancedClassDefinition>,
    pub talent_graphs: Vec<TalentGraphDefinition>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CharacterDefaultsDefinition {
    pub lineage_id: String,
    pub background_id: Option<String>,
    pub base_class_id: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct LevelThresholdDefinition {
    pub level: u32,
    pub total_experience_required: u64,
    pub point_awards: BTreeMap<String, u16>,
}

macro_rules! origin_definition {
    ($name:ident) => {
        #[derive(Clone, Debug, Eq, PartialEq)]
        pub struct $name {
            pub id: String,
            pub name: String,
            pub description: String,
            pub ui_order: u16,
            pub creation_grants: Vec<GrantDefinition>,
            pub talent_graph_ids: Vec<String>,
        }
    };
}

origin_definition!(LineageDefinition);
origin_definition!(BackgroundDefinition);

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct BaseClassDefinition {
    pub id: String,
    pub name: String,
    pub description: String,
    pub ui_order: u16,
    pub creation_grants: Vec<GrantDefinition>,
    pub level_grants: Vec<LevelGrantDefinition>,
    pub talent_graph_ids: Vec<String>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AdvancedClassDefinition {
    pub id: String,
    pub name: String,
    pub description: String,
    pub ui_order: u16,
    pub from_class_ids: Vec<String>,
    pub exclusive_group: Option<String>,
    pub requirements: Vec<RequirementDefinition>,
    pub promotion_grants: Vec<GrantDefinition>,
    pub level_grants: Vec<LevelGrantDefinition>,
    pub talent_graph_ids: Vec<String>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct LevelGrantDefinition {
    pub level: u32,
    pub grants: Vec<GrantDefinition>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TalentGraphDefinition {
    pub id: String,
    pub name: String,
    pub description: String,
    pub ui_order: u16,
    pub point_pool_id: String,
    pub nodes: Vec<TalentNodeDefinition>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TalentNodeDefinition {
    pub id: String,
    pub name: String,
    pub description: String,
    pub ui_order: u16,
    pub exclusion_group: Option<String>,
    pub ranks: Vec<TalentRankDefinition>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TalentRankDefinition {
    pub rank: u8,
    pub cost: u16,
    pub requirements: Vec<RequirementDefinition>,
    pub grants: Vec<GrantDefinition>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
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

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum RequirementDisclosure {
    #[default]
    Public,
    Hidden,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RequirementDefinition {
    pub disclosure: RequirementDisclosure,
    pub hint: Option<String>,
    pub predicate: RequirementPredicate,
}

#[derive(Clone, Debug, Eq, PartialEq)]
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

#[derive(Clone, Debug)]
pub(crate) struct ProgressionCatalog {
    pub defaults: CharacterDefaultsDefinition,
    pub level_thresholds: Vec<LevelThresholdDefinition>,
    pub lineages: BTreeMap<String, LineageDefinition>,
    pub backgrounds: BTreeMap<String, BackgroundDefinition>,
    pub base_classes: BTreeMap<String, BaseClassDefinition>,
    pub advanced_classes: BTreeMap<String, AdvancedClassDefinition>,
    pub talent_graphs: BTreeMap<String, TalentGraphDefinition>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProgressionOptionView {
    pub id: String,
    pub label: String,
    pub description: String,
}

pub type LineageView = ProgressionOptionView;
pub type BackgroundView = ProgressionOptionView;
pub type ClassView = ProgressionOptionView;

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CharacterCreationCatalogView {
    pub default_lineage_id: String,
    pub default_background_id: Option<String>,
    pub default_base_class_id: String,
    pub lineages: Vec<LineageView>,
    pub backgrounds: Vec<BackgroundView>,
    pub base_classes: Vec<ClassView>,
}

/// An explicit optional-background choice. `Default` and `None` are distinct:
/// a pack may recommend a background without making it mandatory.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum BackgroundSelection<'a> {
    Default,
    None,
    Id(&'a str),
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RequirementStatusView {
    pub met: bool,
    pub text: String,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PromotionView {
    pub class: ClassView,
    pub available: bool,
    pub requirements: Vec<RequirementStatusView>,
    pub command: String,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TalentNodeView {
    pub id: String,
    pub label: String,
    pub description: String,
    pub rank: u8,
    pub max_rank: u8,
    pub next_cost: Option<u16>,
    pub available: bool,
    pub requirements: Vec<RequirementStatusView>,
    pub command: String,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TalentGraphView {
    pub id: String,
    pub label: String,
    pub description: String,
    pub point_pool_id: String,
    pub available_points: u16,
    pub nodes: Vec<TalentNodeView>,
}

impl ProgressionCatalog {
    pub fn compile(
        mut definition: CharacterProgressionDefinition,
        abilities: &BTreeMap<String, AbilityDefinition>,
    ) -> Result<Self, WorldError> {
        if definition.lineages.is_empty()
            || definition.lineages.len() > MAX_LINEAGES
            || definition.backgrounds.len() > MAX_BACKGROUNDS
            || definition.base_classes.is_empty()
            || definition.base_classes.len() > MAX_BASE_CLASSES
            || definition.advanced_classes.len() > MAX_ADVANCED_CLASSES
            || definition.talent_graphs.len() > MAX_TALENT_GRAPHS
        {
            return Err(WorldError::InvalidProgressionDefinition);
        }
        definition.level_thresholds.sort_by_key(|entry| entry.level);
        let mut prior_level = 1;
        let mut prior_xp = 0;
        let mut awarded_pools = BTreeSet::new();
        for threshold in &definition.level_thresholds {
            if threshold.level != prior_level + 1
                || threshold.total_experience_required <= prior_xp
                || threshold.level > crate::MAX_PROGRESSION_LEVEL
                || threshold.point_awards.iter().any(|(id, amount)| {
                    !valid_stat_id(id) || *amount == 0 || *amount > MAX_PROGRESSION_POINTS
                })
            {
                return Err(WorldError::InvalidProgressionDefinition);
            }
            awarded_pools.extend(threshold.point_awards.keys().cloned());
            prior_level = threshold.level;
            prior_xp = threshold.total_experience_required;
        }
        if awarded_pools.len() > MAX_PROGRESSION_POINT_POOLS {
            return Err(WorldError::InvalidProgressionDefinition);
        }

        let lineages = collect_unique(definition.lineages, |value| &value.id)?;
        let backgrounds = collect_unique(definition.backgrounds, |value| &value.id)?;
        let base_classes = collect_unique(definition.base_classes, |value| &value.id)?;
        let advanced_classes = collect_unique(definition.advanced_classes, |value| &value.id)?;
        let talent_graphs = collect_unique(definition.talent_graphs, |value| &value.id)?;
        if !lineages.contains_key(&definition.defaults.lineage_id)
            || !base_classes.contains_key(&definition.defaults.base_class_id)
            || definition
                .defaults
                .background_id
                .as_ref()
                .is_some_and(|id| !backgrounds.contains_key(id))
        {
            return Err(WorldError::InvalidProgressionDefinition);
        }

        let mut global_ids = BTreeSet::new();
        for id in lineages
            .keys()
            .chain(backgrounds.keys())
            .chain(base_classes.keys())
            .chain(advanced_classes.keys())
            .chain(talent_graphs.keys())
        {
            if !valid_definition_id(id) || !global_ids.insert(id) {
                return Err(WorldError::InvalidProgressionDefinition);
            }
        }
        let class_ids = base_classes
            .keys()
            .chain(advanced_classes.keys())
            .cloned()
            .collect::<BTreeSet<_>>();
        let graph_ids = talent_graphs.keys().cloned().collect::<BTreeSet<_>>();
        let max_level = definition
            .level_thresholds
            .last()
            .map_or(1, |entry| entry.level);

        for lineage in lineages.values() {
            validate_named(&lineage.name, &lineage.description)?;
            validate_grants(&lineage.creation_grants, abilities, &mut awarded_pools)?;
            validate_graph_ids(&lineage.talent_graph_ids, &graph_ids)?;
        }
        for background in backgrounds.values() {
            validate_named(&background.name, &background.description)?;
            validate_grants(&background.creation_grants, abilities, &mut awarded_pools)?;
            validate_graph_ids(&background.talent_graph_ids, &graph_ids)?;
        }
        for class in base_classes.values() {
            validate_named(&class.name, &class.description)?;
            validate_grants(&class.creation_grants, abilities, &mut awarded_pools)?;
            if !class.creation_grants.iter().any(|grant| {
                matches!(grant, GrantDefinition::Attribute { attribute_id, amount }
                    if attribute_id == "max_health" && *amount > 0)
            }) {
                return Err(WorldError::InvalidProgressionDefinition);
            }
            validate_level_grants(
                &class.level_grants,
                max_level,
                abilities,
                &mut awarded_pools,
            )?;
            validate_graph_ids(&class.talent_graph_ids, &graph_ids)?;
        }

        for class in advanced_classes.values() {
            validate_named(&class.name, &class.description)?;
            if class.from_class_ids.is_empty()
                || class
                    .from_class_ids
                    .iter()
                    .any(|id| id == &class.id || !class_ids.contains(id))
                || class.from_class_ids.iter().collect::<BTreeSet<_>>().len()
                    != class.from_class_ids.len()
            {
                return Err(WorldError::InvalidProgressionDefinition);
            }
            validate_grants(&class.promotion_grants, abilities, &mut awarded_pools)?;
            validate_level_grants(
                &class.level_grants,
                max_level,
                abilities,
                &mut awarded_pools,
            )?;
            validate_graph_ids(&class.talent_graph_ids, &graph_ids)?;
        }
        validate_class_reachability(&base_classes, &advanced_classes)?;
        if awarded_pools.len() > MAX_PROGRESSION_POINT_POOLS {
            return Err(WorldError::InvalidProgressionDefinition);
        }

        let mut nodes = BTreeMap::<(String, String), u8>::new();
        let mut global_nodes = BTreeSet::new();
        for graph in talent_graphs.values() {
            validate_named(&graph.name, &graph.description)?;
            if !valid_stat_id(&graph.point_pool_id)
                || !awarded_pools.contains(&graph.point_pool_id)
                || graph.nodes.is_empty()
                || graph.nodes.len() > MAX_NODES_PER_GRAPH
            {
                return Err(WorldError::InvalidProgressionDefinition);
            }
            for node in &graph.nodes {
                validate_named(&node.name, &node.description)?;
                if !valid_definition_id(&node.id)
                    || !global_nodes.insert(node.id.clone())
                    || node.ranks.is_empty()
                    || node.ranks.len() > MAX_RANKS_PER_NODE
                {
                    return Err(WorldError::InvalidProgressionDefinition);
                }
                for (index, rank) in node.ranks.iter().enumerate() {
                    if usize::from(rank.rank) != index + 1
                        || rank.cost == 0
                        || rank.cost > MAX_PROGRESSION_POINTS
                    {
                        return Err(WorldError::InvalidProgressionDefinition);
                    }
                    validate_grants(&rank.grants, abilities, &mut awarded_pools)?;
                }
                nodes.insert((graph.id.clone(), node.id.clone()), node.ranks.len() as u8);
            }
        }
        if awarded_pools.len() > MAX_PROGRESSION_POINT_POOLS {
            return Err(WorldError::InvalidProgressionDefinition);
        }
        for class in advanced_classes.values() {
            validate_requirements(
                &class.requirements,
                1,
                &lineages,
                &class_ids,
                &talent_graphs,
                &nodes,
                abilities,
            )?;
        }
        let mut dependencies = nodes
            .keys()
            .cloned()
            .map(|key| (key, BTreeSet::new()))
            .collect::<BTreeMap<_, _>>();
        for graph in talent_graphs.values() {
            for node in &graph.nodes {
                for rank in &node.ranks {
                    validate_requirements(
                        &rank.requirements,
                        1,
                        &lineages,
                        &class_ids,
                        &talent_graphs,
                        &nodes,
                        abilities,
                    )?;
                    if self_rank_requirement_is_invalid(
                        &rank.requirements,
                        &graph.id,
                        &node.id,
                        rank.rank,
                    ) {
                        return Err(WorldError::InvalidProgressionDefinition);
                    }
                    collect_node_dependencies(
                        &rank.requirements,
                        dependencies
                            .get_mut(&(graph.id.clone(), node.id.clone()))
                            .expect("every validated talent node is indexed"),
                    );
                }
                dependencies
                    .get_mut(&(graph.id.clone(), node.id.clone()))
                    .expect("every validated talent node is indexed")
                    .remove(&(graph.id.clone(), node.id.clone()));
            }
        }
        validate_dependency_graph(&dependencies)?;

        let catalog = Self {
            defaults: definition.defaults,
            level_thresholds: definition.level_thresholds,
            lineages,
            backgrounds,
            base_classes,
            advanced_classes,
            talent_graphs,
        };
        // Every independent creation choice is selectable. Validate their
        // composition at catalog install time; authors never maintain a
        // lineage × background × class matrix.
        let background_ids = std::iter::once(None)
            .chain(catalog.backgrounds.keys().map(|id| Some(id.as_str())))
            .collect::<Vec<_>>();
        for lineage_id in catalog.lineages.keys() {
            for background_id in &background_ids {
                for class_id in catalog.base_classes.keys() {
                    let background =
                        background_id.map_or(BackgroundSelection::None, BackgroundSelection::Id);
                    catalog
                        .initial_gameplay(Some(lineage_id), background, Some(class_id), abilities)
                        .map_err(|_| WorldError::InvalidProgressionDefinition)?;
                }
            }
        }
        Ok(catalog)
    }

    pub fn creation_view(&self) -> CharacterCreationCatalogView {
        CharacterCreationCatalogView {
            default_lineage_id: self.defaults.lineage_id.clone(),
            default_background_id: self.defaults.background_id.clone(),
            default_base_class_id: self.defaults.base_class_id.clone(),
            lineages: ordered_options(
                self.lineages
                    .values()
                    .map(|entry| (entry.ui_order, &entry.id, &entry.name, &entry.description)),
            ),
            backgrounds: ordered_options(
                self.backgrounds
                    .values()
                    .map(|entry| (entry.ui_order, &entry.id, &entry.name, &entry.description)),
            ),
            base_classes: ordered_options(
                self.base_classes
                    .values()
                    .map(|entry| (entry.ui_order, &entry.id, &entry.name, &entry.description)),
            ),
        }
    }

    pub fn initial_gameplay(
        &self,
        lineage_id: Option<&str>,
        background: BackgroundSelection<'_>,
        base_class_id: Option<&str>,
        abilities: &BTreeMap<String, AbilityDefinition>,
    ) -> Result<CharacterGameplaySnapshot, WorldError> {
        let lineage_id = lineage_id.unwrap_or(&self.defaults.lineage_id);
        let background_id = match background {
            BackgroundSelection::Default => self.defaults.background_id.as_deref(),
            BackgroundSelection::None => None,
            BackgroundSelection::Id(id) => Some(id),
        };
        let class_id = base_class_id.unwrap_or(&self.defaults.base_class_id);
        let lineage = self
            .lineages
            .get(lineage_id)
            .ok_or(WorldError::LineageNotFound)?;
        let background = background_id
            .map(|id| {
                self.backgrounds
                    .get(id)
                    .ok_or(WorldError::BackgroundNotFound)
            })
            .transpose()?;
        let class = self
            .base_classes
            .get(class_id)
            .ok_or(WorldError::ClassNotFound)?;
        let mut snapshot = CharacterGameplaySnapshot {
            lineage_id: lineage.id.clone(),
            background_id: background.map(|entry| entry.id.clone()),
            base_class_id: class.id.clone(),
            active_class_id: class.id.clone(),
            class_history: vec![class.id.clone()],
            ..CharacterGameplaySnapshot::default()
        };
        // Creation grants are the complete authored sheet, not bonuses on top
        // of legacy engine defaults. Keep non-sheet defaults such as level,
        // coins and the starter quest, but compose every stat from zero.
        snapshot.vitals.health = 0;
        snapshot.vitals.max_health = 0;
        snapshot.attributes.strength = 0;
        snapshot.attributes.defence = 0;
        snapshot.attributes.additional.clear();
        snapshot.skills.clear();
        snapshot.resources.clear();
        snapshot.resource_regeneration.clear();
        snapshot.ability_ids.clear();
        snapshot.ability_cooldowns.clear();
        snapshot.resistances.clear();
        snapshot.statuses.clear();
        snapshot.progression_points.clear();
        snapshot.talent_node_ranks.clear();
        snapshot.tags.clear();
        snapshot.received_signals.clear();
        snapshot.equipment.clear();
        snapshot.effects.clear();
        let mut grants = Vec::new();
        grants.extend(lineage.creation_grants.iter().cloned());
        if let Some(background) = background {
            grants.extend(background.creation_grants.iter().cloned());
        }
        grants.extend(class.creation_grants.iter().cloned());
        apply_grants_checked(&mut snapshot, &grants, abilities)?;
        if snapshot.vitals.max_health <= 0 {
            return Err(WorldError::InvalidProgressionDefinition);
        }
        // Creation is the one explicit initialization boundary. Capacity
        // grants themselves never alter current values during live play.
        snapshot.vitals.health = snapshot.vitals.max_health;
        for pool in snapshot.resources.values_mut() {
            pool.current = pool.max;
        }
        Ok(snapshot)
    }

    pub fn next_level(&self, current: u32, experience: u64) -> Option<&LevelThresholdDefinition> {
        self.level_thresholds.iter().find(|entry| {
            entry.level == current.saturating_add(1)
                && experience >= entry.total_experience_required
        })
    }

    pub fn next_level_experience(&self, current: u32) -> u64 {
        self.level_thresholds
            .iter()
            .find(|entry| entry.level == current.saturating_add(1))
            .map_or(0, |entry| entry.total_experience_required)
    }

    pub fn class_level_grants(&self, class_id: &str, level: u32) -> &[GrantDefinition] {
        if let Some(class) = self.base_classes.get(class_id) {
            return class
                .level_grants
                .iter()
                .find(|entry| entry.level == level)
                .map_or(&[], |entry| entry.grants.as_slice());
        }
        self.advanced_classes
            .get(class_id)
            .and_then(|class| class.level_grants.iter().find(|entry| entry.level == level))
            .map_or(&[], |entry| entry.grants.as_slice())
    }

    pub fn class_view(&self, id: &str) -> ClassView {
        self.base_classes
            .get(id)
            .map(|entry| option_view(&entry.id, &entry.name, &entry.description))
            .or_else(|| {
                self.advanced_classes
                    .get(id)
                    .map(|entry| option_view(&entry.id, &entry.name, &entry.description))
            })
            .unwrap_or_else(|| option_view(id, &humanize_id(id), "Unavailable class definition."))
    }

    pub fn lineage_view(&self, id: &str) -> LineageView {
        self.lineages
            .get(id)
            .map(|entry| option_view(&entry.id, &entry.name, &entry.description))
            .unwrap_or_else(|| option_view(id, &humanize_id(id), "Unavailable lineage definition."))
    }

    pub fn background_view(&self, id: &str) -> BackgroundView {
        self.backgrounds
            .get(id)
            .map(|entry| option_view(&entry.id, &entry.name, &entry.description))
            .unwrap_or_else(|| {
                option_view(id, &humanize_id(id), "Unavailable background definition.")
            })
    }

    pub fn promotions(
        &self,
        snapshot: &CharacterGameplaySnapshot,
        world_flags: &BTreeSet<String>,
    ) -> Vec<PromotionView> {
        let mut values = self
            .advanced_classes
            .values()
            .filter(|class| class.from_class_ids.contains(&snapshot.active_class_id))
            .map(|class| {
                let mut requirements =
                    requirement_views(&class.requirements, snapshot, world_flags);
                let exclusive = class.exclusive_group.as_ref().is_some_and(|group| {
                    snapshot.class_history.iter().any(|id| {
                        self.advanced_classes
                            .get(id)
                            .and_then(|entry| entry.exclusive_group.as_ref())
                            == Some(group)
                    })
                });
                if exclusive {
                    requirements.push(RequirementStatusView {
                        met: false,
                        text: "A mutually exclusive class path has already been chosen.".into(),
                    });
                }
                PromotionView {
                    class: self.class_view(&class.id),
                    available: !exclusive && requirements.iter().all(|entry| entry.met),
                    requirements,
                    command: format!("promote {}", class.id),
                }
            })
            .collect::<Vec<_>>();
        values.sort_by(|left, right| left.class.id.cmp(&right.class.id));
        values
    }

    pub fn talent_views(
        &self,
        snapshot: &CharacterGameplaySnapshot,
        world_flags: &BTreeSet<String>,
    ) -> Vec<TalentGraphView> {
        let owned = self.owned_graph_ids(snapshot);
        let mut graphs = owned
            .into_iter()
            .filter_map(|id| self.talent_graphs.get(&id))
            .map(|graph| {
                let mut nodes = graph
                    .nodes
                    .iter()
                    .map(|node| {
                        let rank = snapshot
                            .talent_node_ranks
                            .get(&graph.id)
                            .and_then(|values| values.get(&node.id))
                            .copied()
                            .unwrap_or(0);
                        let next = node.ranks.get(usize::from(rank));
                        let mut requirements = next.map_or_else(Vec::new, |next| {
                            requirement_views(&next.requirements, snapshot, world_flags)
                        });
                        let excluded = node.exclusion_group.as_ref().is_some_and(|group| {
                            graph.nodes.iter().any(|peer| {
                                peer.id != node.id
                                    && peer.exclusion_group.as_ref() == Some(group)
                                    && snapshot
                                        .talent_node_ranks
                                        .get(&graph.id)
                                        .and_then(|values| values.get(&peer.id))
                                        .copied()
                                        .unwrap_or(0)
                                        > 0
                            })
                        });
                        if excluded {
                            requirements.push(RequirementStatusView {
                                met: false,
                                text: "A mutually exclusive talent has already been chosen.".into(),
                            });
                        }
                        let has_points = next.is_some_and(|next| {
                            snapshot
                                .progression_points
                                .get(&graph.point_pool_id)
                                .copied()
                                .unwrap_or(0)
                                >= next.cost
                        });
                        TalentNodeView {
                            id: node.id.clone(),
                            label: node.name.clone(),
                            description: node.description.clone(),
                            rank,
                            max_rank: node.ranks.len() as u8,
                            next_cost: next.map(|entry| entry.cost),
                            available: next.is_some()
                                && !excluded
                                && has_points
                                && requirements.iter().all(|entry| entry.met),
                            requirements,
                            command: format!("talent learn {} {}", graph.id, node.id),
                        }
                    })
                    .collect::<Vec<_>>();
                nodes.sort_by(|left, right| left.id.cmp(&right.id));
                TalentGraphView {
                    id: graph.id.clone(),
                    label: graph.name.clone(),
                    description: graph.description.clone(),
                    point_pool_id: graph.point_pool_id.clone(),
                    available_points: snapshot
                        .progression_points
                        .get(&graph.point_pool_id)
                        .copied()
                        .unwrap_or(0),
                    nodes,
                }
            })
            .collect::<Vec<_>>();
        graphs.sort_by(|left, right| left.id.cmp(&right.id));
        graphs
    }

    pub fn promote(
        &self,
        snapshot: &CharacterGameplaySnapshot,
        class_id: &str,
        world_flags: &BTreeSet<String>,
        abilities: &BTreeMap<String, AbilityDefinition>,
    ) -> Result<CharacterGameplaySnapshot, WorldError> {
        let class = self
            .advanced_classes
            .get(class_id)
            .ok_or(WorldError::ClassNotFound)?;
        if !class.from_class_ids.contains(&snapshot.active_class_id)
            || snapshot.class_history.contains(&class.id)
            || snapshot.class_history.len() >= MAX_CLASS_HISTORY
            || !requirements_met(&class.requirements, snapshot, world_flags)
            || class.exclusive_group.as_ref().is_some_and(|group| {
                snapshot.class_history.iter().any(|id| {
                    self.advanced_classes
                        .get(id)
                        .and_then(|entry| entry.exclusive_group.as_ref())
                        == Some(group)
                })
            })
        {
            return Err(WorldError::PromotionUnavailable);
        }
        let mut next = snapshot.clone();
        apply_grants_checked(&mut next, &class.promotion_grants, abilities)?;
        // Global level is never reset by promotion. Catch up every authored
        // target-class milestone already reached so a delayed choice cannot
        // permanently lose class growth. The class-history guard above makes
        // this one atomic, exactly-once transition.
        for milestone in class
            .level_grants
            .iter()
            .filter(|milestone| milestone.level <= snapshot.level)
        {
            apply_grants_checked(&mut next, &milestone.grants, abilities)?;
        }
        next.active_class_id = class.id.clone();
        next.class_history.push(class.id.clone());
        Ok(next)
    }

    pub fn acquire_talent(
        &self,
        snapshot: &CharacterGameplaySnapshot,
        graph_id: &str,
        node_id: &str,
        world_flags: &BTreeSet<String>,
        abilities: &BTreeMap<String, AbilityDefinition>,
    ) -> Result<CharacterGameplaySnapshot, WorldError> {
        if !self.owned_graph_ids(snapshot).contains(graph_id) {
            return Err(WorldError::TalentUnavailable);
        }
        let graph = self
            .talent_graphs
            .get(graph_id)
            .ok_or(WorldError::TalentUnavailable)?;
        let node = graph
            .nodes
            .iter()
            .find(|entry| entry.id == node_id)
            .ok_or(WorldError::TalentUnavailable)?;
        let current_rank = snapshot
            .talent_node_ranks
            .get(graph_id)
            .and_then(|values| values.get(node_id))
            .copied()
            .unwrap_or(0);
        let selected_nodes = snapshot
            .talent_node_ranks
            .values()
            .map(BTreeMap::len)
            .sum::<usize>();
        if current_rank == 0 && selected_nodes >= MAX_SELECTED_TALENT_NODES {
            return Err(WorldError::ProgressionCapExceeded);
        }
        let rank = node
            .ranks
            .get(usize::from(current_rank))
            .ok_or(WorldError::TalentUnavailable)?;
        if !requirements_met(&rank.requirements, snapshot, world_flags)
            || node.exclusion_group.as_ref().is_some_and(|group| {
                graph.nodes.iter().any(|peer| {
                    peer.id != node.id
                        && peer.exclusion_group.as_ref() == Some(group)
                        && snapshot
                            .talent_node_ranks
                            .get(graph_id)
                            .and_then(|values| values.get(&peer.id))
                            .copied()
                            .unwrap_or(0)
                            > 0
                })
            })
        {
            return Err(WorldError::TalentUnavailable);
        }
        let points = snapshot
            .progression_points
            .get(&graph.point_pool_id)
            .copied()
            .unwrap_or(0);
        if points < rank.cost {
            return Err(WorldError::InsufficientProgressionPoints);
        }
        let mut next = snapshot.clone();
        apply_grants_checked(&mut next, &rank.grants, abilities)?;
        next.progression_points
            .insert(graph.point_pool_id.clone(), points - rank.cost);
        next.talent_node_ranks
            .entry(graph.id.clone())
            .or_default()
            .insert(node.id.clone(), rank.rank);
        Ok(next)
    }

    pub fn apply_level(
        &self,
        snapshot: &CharacterGameplaySnapshot,
        threshold: &LevelThresholdDefinition,
        abilities: &BTreeMap<String, AbilityDefinition>,
    ) -> Result<CharacterGameplaySnapshot, WorldError> {
        if threshold.level != snapshot.level.saturating_add(1)
            || snapshot.experience < threshold.total_experience_required
        {
            return Err(WorldError::InvalidProgressionTransition);
        }
        let mut next = snapshot.clone();
        let mut grants = threshold
            .point_awards
            .iter()
            .map(|(pool_id, amount)| GrantDefinition::Points {
                pool_id: pool_id.clone(),
                amount: *amount,
            })
            .collect::<Vec<_>>();
        grants.extend(
            self.class_level_grants(&snapshot.active_class_id, threshold.level)
                .iter()
                .cloned(),
        );
        apply_grants_checked(&mut next, &grants, abilities)?;
        next.level = threshold.level;
        Ok(next)
    }

    pub fn validate_snapshot(
        &self,
        snapshot: &CharacterGameplaySnapshot,
        abilities: &BTreeMap<String, AbilityDefinition>,
    ) -> bool {
        self.lineages.contains_key(&snapshot.lineage_id)
            && snapshot
                .background_id
                .as_ref()
                .is_none_or(|id| self.backgrounds.contains_key(id))
            && self.base_classes.contains_key(&snapshot.base_class_id)
            && (self.base_classes.contains_key(&snapshot.active_class_id)
                || self
                    .advanced_classes
                    .contains_key(&snapshot.active_class_id))
            && self.valid_class_history(snapshot)
            && self.level_matches_experience(snapshot.level, snapshot.experience)
            && snapshot.progression_points.len() <= MAX_PROGRESSION_POINT_POOLS
            && snapshot
                .progression_points
                .iter()
                .all(|(id, value)| valid_stat_id(id) && *value <= MAX_PROGRESSION_POINTS)
            && snapshot.talent_node_ranks.len() <= self.talent_graphs.len()
            && snapshot
                .talent_node_ranks
                .values()
                .map(BTreeMap::len)
                .sum::<usize>()
                <= MAX_SELECTED_TALENT_NODES
            && self.valid_talent_selection(snapshot)
            && snapshot
                .ability_ids
                .iter()
                .all(|id| abilities.contains_key(id))
            && snapshot.tags.len() <= MAX_PROGRESSION_TAGS
            && snapshot.tags.iter().all(|id| valid_definition_id(id))
            && snapshot.received_signals.len() <= MAX_RECEIVED_SIGNALS
            && snapshot
                .received_signals
                .iter()
                .all(|id| valid_definition_id(id))
    }

    fn valid_class_history(&self, snapshot: &CharacterGameplaySnapshot) -> bool {
        if snapshot.class_history.first() != Some(&snapshot.base_class_id)
            || snapshot.class_history.last() != Some(&snapshot.active_class_id)
            || snapshot.class_history.len() > MAX_CLASS_HISTORY
            || snapshot.class_history.iter().collect::<BTreeSet<_>>().len()
                != snapshot.class_history.len()
        {
            return false;
        }
        let mut exclusive_groups = BTreeSet::new();
        snapshot.class_history.windows(2).all(|pair| {
            self.advanced_classes.get(&pair[1]).is_some_and(|class| {
                class.from_class_ids.contains(&pair[0])
                    && class
                        .exclusive_group
                        .as_ref()
                        .is_none_or(|group| exclusive_groups.insert(group))
            })
        })
    }

    fn level_matches_experience(&self, level: u32, experience: u64) -> bool {
        let expected = self
            .level_thresholds
            .iter()
            .take_while(|entry| experience >= entry.total_experience_required)
            .last()
            .map_or(1, |entry| entry.level);
        level == expected
    }

    fn valid_talent_selection(&self, snapshot: &CharacterGameplaySnapshot) -> bool {
        let owned = self.owned_graph_ids(snapshot);
        snapshot.talent_node_ranks.iter().all(|(graph_id, ranks)| {
            let Some(graph) = self.talent_graphs.get(graph_id) else {
                return false;
            };
            if !owned.contains(graph_id) || ranks.len() > graph.nodes.len() {
                return false;
            }
            let mut exclusion_groups = BTreeSet::new();
            ranks.iter().all(|(node_id, rank)| {
                graph
                    .nodes
                    .iter()
                    .find(|node| node.id == *node_id)
                    .is_some_and(|node| {
                        *rank > 0
                            && usize::from(*rank) <= node.ranks.len()
                            && node
                                .exclusion_group
                                .as_ref()
                                .is_none_or(|group| exclusion_groups.insert(group))
                    })
            })
        })
    }

    fn owned_graph_ids(&self, snapshot: &CharacterGameplaySnapshot) -> BTreeSet<String> {
        let mut ids = self
            .lineages
            .get(&snapshot.lineage_id)
            .into_iter()
            .flat_map(|entry| entry.talent_graph_ids.iter().cloned())
            .collect::<BTreeSet<_>>();
        if let Some(background) = snapshot
            .background_id
            .as_ref()
            .and_then(|id| self.backgrounds.get(id))
        {
            ids.extend(background.talent_graph_ids.iter().cloned());
        }
        for class_id in &snapshot.class_history {
            if let Some(class) = self.base_classes.get(class_id) {
                ids.extend(class.talent_graph_ids.iter().cloned());
            }
            if let Some(class) = self.advanced_classes.get(class_id) {
                ids.extend(class.talent_graph_ids.iter().cloned());
            }
        }
        ids
    }
}

fn apply_grants_checked(
    snapshot: &mut CharacterGameplaySnapshot,
    grants: &[GrantDefinition],
    abilities: &BTreeMap<String, AbilityDefinition>,
) -> Result<(), WorldError> {
    if grants.len() > MAX_GRANTS_PER_ENTRY.saturating_mul(4) {
        return Err(WorldError::InvalidProgressionTransition);
    }
    // Capacity is resolved first so one declarative GrantSet is independent
    // of authoring order (for example regeneration may precede its capacity).
    for grant in grants {
        if let GrantDefinition::ResourceMax {
            resource_id,
            amount,
        } = grant
        {
            if *amount == 0
                || !valid_stat_id(resource_id)
                || snapshot.resources.len() >= MAX_RESOURCE_POOLS
                    && !snapshot.resources.contains_key(resource_id)
            {
                return Err(WorldError::InvalidProgressionTransition);
            }
            let pool = snapshot
                .resources
                .entry(resource_id.clone())
                .or_insert(ResourcePool { current: 0, max: 0 });
            pool.max = pool
                .max
                .checked_add(*amount)
                .filter(|value| *value <= MAX_RESOURCE_VALUE)
                .ok_or(WorldError::ProgressionCapExceeded)?;
            pool.current = pool.current.min(pool.max);
        }
    }
    for grant in grants {
        match grant {
            GrantDefinition::Attribute {
                attribute_id,
                amount,
            } => {
                if *amount == 0 || !valid_stat_id(attribute_id) {
                    return Err(WorldError::InvalidProgressionTransition);
                }
                let limit = i32::try_from(MAX_STAT_VALUE).unwrap_or(i32::MAX);
                let amount =
                    i32::try_from(*amount).map_err(|_| WorldError::InvalidProgressionTransition)?;
                let value = match attribute_id.as_str() {
                    "max_health" => &mut snapshot.vitals.max_health,
                    "strength" => &mut snapshot.attributes.strength,
                    "defence" => &mut snapshot.attributes.defence,
                    _ => snapshot
                        .attributes
                        .additional
                        .entry(attribute_id.clone())
                        .or_default(),
                };
                *value = value
                    .checked_add(amount)
                    .filter(|value| *value <= limit)
                    .ok_or(WorldError::ProgressionCapExceeded)?;
            }
            GrantDefinition::Skill { skill_id, amount } => {
                if *amount == 0 || !valid_stat_id(skill_id) {
                    return Err(WorldError::InvalidProgressionTransition);
                }
                let value = snapshot.skills.entry(skill_id.clone()).or_default();
                *value = value
                    .checked_add(*amount)
                    .filter(|value| *value <= MAX_SKILL_VALUE)
                    .ok_or(WorldError::ProgressionCapExceeded)?;
            }
            GrantDefinition::ResourceMax {
                resource_id: _,
                amount: _,
            } => {}
            GrantDefinition::ResourceRegeneration {
                resource_id,
                amount,
            } => {
                if *amount == 0
                    || !valid_stat_id(resource_id)
                    || !snapshot.resources.contains_key(resource_id)
                {
                    return Err(WorldError::InvalidProgressionTransition);
                }
                let value = snapshot
                    .resource_regeneration
                    .entry(resource_id.clone())
                    .or_default();
                *value = value
                    .checked_add(*amount)
                    .filter(|value| *value <= MAX_RESOURCE_VALUE)
                    .ok_or(WorldError::ProgressionCapExceeded)?;
            }
            GrantDefinition::Ability { ability_id } => {
                if !abilities.is_empty() && !abilities.contains_key(ability_id) {
                    return Err(WorldError::InvalidProgressionTransition);
                }
                if !snapshot.ability_ids.contains(ability_id) {
                    if snapshot.ability_ids.len() >= MAX_KNOWN_ABILITIES {
                        return Err(WorldError::ProgressionCapExceeded);
                    }
                    snapshot.ability_ids.push(ability_id.clone());
                    snapshot.ability_ids.sort();
                }
            }
            GrantDefinition::Resistance {
                damage_type,
                percent,
            } => {
                if *percent == 0 {
                    return Err(WorldError::InvalidProgressionTransition);
                }
                let value = snapshot.resistances.entry(*damage_type).or_default();
                *value = value
                    .checked_add(*percent)
                    .filter(|value| *value <= MAX_RESISTANCE_PERCENT)
                    .ok_or(WorldError::ProgressionCapExceeded)?;
            }
            GrantDefinition::Points { pool_id, amount } => {
                if *amount == 0 || !valid_stat_id(pool_id) {
                    return Err(WorldError::InvalidProgressionTransition);
                }
                let value = snapshot
                    .progression_points
                    .entry(pool_id.clone())
                    .or_default();
                *value = value
                    .checked_add(*amount)
                    .filter(|value| *value <= MAX_PROGRESSION_POINTS)
                    .ok_or(WorldError::ProgressionCapExceeded)?;
            }
            GrantDefinition::Tag { tag_id } => {
                if !valid_definition_id(tag_id)
                    || snapshot.tags.len() >= MAX_PROGRESSION_TAGS
                        && !snapshot.tags.contains(tag_id)
                {
                    return Err(WorldError::InvalidProgressionTransition);
                }
                snapshot.tags.insert(tag_id.clone());
            }
        }
    }
    Ok(())
}

fn requirements_met(
    requirements: &[RequirementDefinition],
    snapshot: &CharacterGameplaySnapshot,
    world_flags: &BTreeSet<String>,
) -> bool {
    requirements
        .iter()
        .all(|entry| requirement_met(&entry.predicate, snapshot, world_flags))
}

fn requirement_met(
    predicate: &RequirementPredicate,
    snapshot: &CharacterGameplaySnapshot,
    world_flags: &BTreeSet<String>,
) -> bool {
    match predicate {
        RequirementPredicate::Level { minimum } => snapshot.level >= *minimum,
        RequirementPredicate::PermanentAttribute {
            attribute_id,
            minimum,
        } => {
            crate::permanent_gameplay_attribute(snapshot, attribute_id)
                >= i32::try_from(*minimum).unwrap_or(i32::MAX)
        }
        RequirementPredicate::Skill { skill_id, minimum } => {
            snapshot.skills.get(skill_id).copied().unwrap_or(0) >= *minimum
        }
        RequirementPredicate::CompletedQuest { quest_id } => snapshot
            .quests
            .get(quest_id)
            .is_some_and(|entry| entry.status == QuestStatus::Completed),
        RequirementPredicate::ReceivedSignal { signal_id } => {
            snapshot.received_signals.contains(signal_id)
        }
        RequirementPredicate::CharacterFlag { flag_id, equals } => {
            snapshot.script_flags.contains(flag_id) == *equals
        }
        RequirementPredicate::WorldFlag { flag_id, equals } => {
            world_flags.contains(flag_id) == *equals
        }
        RequirementPredicate::Lineage { lineage_id } => snapshot.lineage_id == *lineage_id,
        RequirementPredicate::CurrentClass { class_id } => snapshot.active_class_id == *class_id,
        RequirementPredicate::HistoricalClass { class_id } => {
            snapshot.class_history.contains(class_id)
        }
        RequirementPredicate::NodeRank {
            graph_id,
            node_id,
            minimum_rank,
        } => {
            snapshot
                .talent_node_ranks
                .get(graph_id)
                .and_then(|nodes| nodes.get(node_id))
                .copied()
                .unwrap_or(0)
                >= *minimum_rank
        }
        RequirementPredicate::Ability { ability_id } => snapshot.ability_ids.contains(ability_id),
        RequirementPredicate::All { requirements } => {
            requirements_met(requirements, snapshot, world_flags)
        }
        RequirementPredicate::Any { requirements } => requirements
            .iter()
            .any(|entry| requirement_met(&entry.predicate, snapshot, world_flags)),
        RequirementPredicate::Not { requirement } => {
            !requirement_met(&requirement.predicate, snapshot, world_flags)
        }
    }
}

fn requirement_views(
    requirements: &[RequirementDefinition],
    snapshot: &CharacterGameplaySnapshot,
    world_flags: &BTreeSet<String>,
) -> Vec<RequirementStatusView> {
    requirements
        .iter()
        .map(|entry| RequirementStatusView {
            met: requirement_met(&entry.predicate, snapshot, world_flags),
            text: disclosed_requirement_text(entry),
        })
        .collect()
}

fn disclosed_requirement_text(requirement: &RequirementDefinition) -> String {
    if requirement.disclosure == RequirementDisclosure::Hidden {
        return bounded_requirement_text(
            requirement
                .hint
                .clone()
                .unwrap_or_else(|| "A hidden condition must be fulfilled.".into()),
        );
    }
    let generated = match &requirement.predicate {
        RequirementPredicate::All { requirements } => format!(
            "All of: {}",
            requirements
                .iter()
                .map(disclosed_requirement_text)
                .collect::<Vec<_>>()
                .join(" AND ")
        ),
        RequirementPredicate::Any { requirements } => format!(
            "One of: {}",
            requirements
                .iter()
                .map(disclosed_requirement_text)
                .collect::<Vec<_>>()
                .join(" OR ")
        ),
        RequirementPredicate::Not { requirement } => {
            format!("Not: {}", disclosed_requirement_text(requirement))
        }
        predicate => public_requirement_text(predicate),
    };
    bounded_requirement_text(requirement.hint.as_ref().map_or(generated.clone(), |hint| {
        if matches!(
            &requirement.predicate,
            RequirementPredicate::All { .. }
                | RequirementPredicate::Any { .. }
                | RequirementPredicate::Not { .. }
        ) {
            format!("{hint} {generated}")
        } else {
            hint.clone()
        }
    }))
}

fn bounded_requirement_text(mut text: String) -> String {
    if text.len() <= MAX_REQUIREMENT_TEXT_BYTES {
        return text;
    }
    let maximum_prefix = MAX_REQUIREMENT_TEXT_BYTES.saturating_sub('…'.len_utf8());
    let boundary = text
        .char_indices()
        .map(|(index, _)| index)
        .take_while(|index| *index <= maximum_prefix)
        .last()
        .unwrap_or(0);
    text.truncate(boundary);
    text.push('…');
    text
}

fn public_requirement_text(predicate: &RequirementPredicate) -> String {
    match predicate {
        RequirementPredicate::Level { minimum } => format!("Reach level {minimum}."),
        RequirementPredicate::PermanentAttribute {
            attribute_id,
            minimum,
        } => {
            format!("Raise {} to {minimum}.", humanize_id(attribute_id))
        }
        RequirementPredicate::Skill { skill_id, minimum } => {
            format!("Raise {} to {minimum}.", humanize_id(skill_id))
        }
        RequirementPredicate::CompletedQuest { quest_id } => {
            format!("Complete {}.", humanize_id(quest_id))
        }
        RequirementPredicate::ReceivedSignal { signal_id } => {
            format!("Fulfil {}.", humanize_id(signal_id))
        }
        RequirementPredicate::CharacterFlag { flag_id, equals }
        | RequirementPredicate::WorldFlag { flag_id, equals } => {
            format!(
                "{} must be {}.",
                humanize_id(flag_id),
                if *equals { "active" } else { "inactive" }
            )
        }
        RequirementPredicate::Lineage { lineage_id } => {
            format!("Be of the {} lineage.", humanize_id(lineage_id))
        }
        RequirementPredicate::CurrentClass { class_id } => {
            format!("Be a {}.", humanize_id(class_id))
        }
        RequirementPredicate::HistoricalClass { class_id } => {
            format!("Have followed the {} path.", humanize_id(class_id))
        }
        RequirementPredicate::NodeRank {
            node_id,
            minimum_rank,
            ..
        } => format!("Reach rank {minimum_rank} in {}.", humanize_id(node_id)),
        RequirementPredicate::Ability { ability_id } => {
            format!("Learn {}.", humanize_id(ability_id))
        }
        RequirementPredicate::All { .. } => "Fulfil every listed condition.".into(),
        RequirementPredicate::Any { .. } => "Fulfil at least one listed condition.".into(),
        RequirementPredicate::Not { .. } => "Avoid the listed condition.".into(),
    }
}

fn validate_named(name: &str, description: &str) -> Result<(), WorldError> {
    if name.trim().is_empty()
        || name.len() > 128
        || name.chars().any(char::is_control)
        || description.trim().is_empty()
        || description.len() > 1_024
        || description.chars().any(char::is_control)
    {
        return Err(WorldError::InvalidProgressionDefinition);
    }
    Ok(())
}

fn validate_grants(
    grants: &[GrantDefinition],
    abilities: &BTreeMap<String, AbilityDefinition>,
    awarded_pools: &mut BTreeSet<String>,
) -> Result<(), WorldError> {
    if grants.len() > MAX_GRANTS_PER_ENTRY {
        return Err(WorldError::InvalidProgressionDefinition);
    }
    for grant in grants {
        let valid = match grant {
            GrantDefinition::Attribute {
                attribute_id,
                amount,
            }
            | GrantDefinition::ResourceMax {
                resource_id: attribute_id,
                amount,
            }
            | GrantDefinition::ResourceRegeneration {
                resource_id: attribute_id,
                amount,
            } => valid_stat_id(attribute_id) && (1..=MAX_STAT_VALUE).contains(amount),
            GrantDefinition::Skill { skill_id, amount } => {
                valid_stat_id(skill_id) && (1..=MAX_SKILL_VALUE).contains(amount)
            }
            GrantDefinition::Ability { ability_id } => abilities.contains_key(ability_id),
            GrantDefinition::Resistance { percent, .. } => {
                (1..=MAX_RESISTANCE_PERCENT).contains(percent)
            }
            GrantDefinition::Points { pool_id, amount } => {
                awarded_pools.insert(pool_id.clone());
                valid_stat_id(pool_id) && (1..=MAX_PROGRESSION_POINTS).contains(amount)
            }
            GrantDefinition::Tag { tag_id } => valid_definition_id(tag_id),
        };
        if !valid {
            return Err(WorldError::InvalidProgressionDefinition);
        }
    }
    Ok(())
}

fn validate_level_grants(
    entries: &[LevelGrantDefinition],
    max_level: u32,
    abilities: &BTreeMap<String, AbilityDefinition>,
    pools: &mut BTreeSet<String>,
) -> Result<(), WorldError> {
    let mut levels = BTreeSet::new();
    for entry in entries {
        if entry.level < 2 || entry.level > max_level || !levels.insert(entry.level) {
            return Err(WorldError::InvalidProgressionDefinition);
        }
        validate_grants(&entry.grants, abilities, pools)?;
    }
    Ok(())
}

fn validate_graph_ids(ids: &[String], known: &BTreeSet<String>) -> Result<(), WorldError> {
    if ids.iter().all(|id| known.contains(id))
        && ids.iter().collect::<BTreeSet<_>>().len() == ids.len()
    {
        Ok(())
    } else {
        Err(WorldError::InvalidProgressionDefinition)
    }
}

#[allow(clippy::too_many_arguments)]
fn validate_requirements(
    values: &[RequirementDefinition],
    depth: usize,
    lineages: &BTreeMap<String, LineageDefinition>,
    classes: &BTreeSet<String>,
    graphs: &BTreeMap<String, TalentGraphDefinition>,
    nodes: &BTreeMap<(String, String), u8>,
    abilities: &BTreeMap<String, AbilityDefinition>,
) -> Result<(), WorldError> {
    if depth > MAX_REQUIREMENT_DEPTH
        || values.len() > MAX_REQUIREMENT_NODES
        || requirement_node_count(values) > MAX_REQUIREMENT_NODES
    {
        return Err(WorldError::InvalidProgressionDefinition);
    }
    for value in values {
        if value.hint.as_ref().is_some_and(|hint| {
            hint.trim().is_empty()
                || hint.len() > MAX_REQUIREMENT_TEXT_BYTES
                || hint.chars().any(char::is_control)
        }) {
            return Err(WorldError::InvalidProgressionDefinition);
        }
        if value.disclosure == RequirementDisclosure::Hidden
            && value
                .hint
                .as_ref()
                .is_none_or(|hint| hint.trim().is_empty())
        {
            return Err(WorldError::InvalidProgressionDefinition);
        }
        let valid = match &value.predicate {
            RequirementPredicate::Level { minimum } => {
                (1..=crate::MAX_PROGRESSION_LEVEL).contains(minimum)
            }
            RequirementPredicate::PermanentAttribute {
                attribute_id,
                minimum,
            } => valid_stat_id(attribute_id) && (1..=MAX_STAT_VALUE).contains(minimum),
            RequirementPredicate::Skill { skill_id, minimum } => {
                valid_stat_id(skill_id) && (1..=MAX_SKILL_VALUE).contains(minimum)
            }
            RequirementPredicate::CompletedQuest { quest_id }
            | RequirementPredicate::ReceivedSignal {
                signal_id: quest_id,
            }
            | RequirementPredicate::CharacterFlag {
                flag_id: quest_id, ..
            }
            | RequirementPredicate::WorldFlag {
                flag_id: quest_id, ..
            } => valid_definition_id(quest_id),
            RequirementPredicate::Lineage { lineage_id } => lineages.contains_key(lineage_id),
            RequirementPredicate::CurrentClass { class_id }
            | RequirementPredicate::HistoricalClass { class_id } => classes.contains(class_id),
            RequirementPredicate::NodeRank {
                graph_id,
                node_id,
                minimum_rank,
            } => {
                graphs.contains_key(graph_id)
                    && nodes
                        .get(&(graph_id.clone(), node_id.clone()))
                        .is_some_and(|max| *minimum_rank > 0 && minimum_rank <= max)
            }
            RequirementPredicate::Ability { ability_id } => abilities.contains_key(ability_id),
            RequirementPredicate::All { requirements }
            | RequirementPredicate::Any { requirements } => {
                if requirements.is_empty() || requirements.len() > 32 {
                    false
                } else {
                    validate_requirements(
                        requirements,
                        depth + 1,
                        lineages,
                        classes,
                        graphs,
                        nodes,
                        abilities,
                    )
                    .is_ok()
                }
            }
            RequirementPredicate::Not { requirement } => validate_requirements(
                std::slice::from_ref(requirement),
                depth + 1,
                lineages,
                classes,
                graphs,
                nodes,
                abilities,
            )
            .is_ok(),
        };
        if !valid {
            return Err(WorldError::InvalidProgressionDefinition);
        }
    }
    Ok(())
}

fn requirement_node_count(values: &[RequirementDefinition]) -> usize {
    values.iter().fold(0_usize, |count, value| {
        let children = match &value.predicate {
            RequirementPredicate::All { requirements }
            | RequirementPredicate::Any { requirements } => requirement_node_count(requirements),
            RequirementPredicate::Not { requirement } => {
                requirement_node_count(std::slice::from_ref(requirement))
            }
            _ => 0,
        };
        count.saturating_add(1).saturating_add(children)
    })
}

fn collect_node_dependencies(
    values: &[RequirementDefinition],
    output: &mut BTreeSet<(String, String)>,
) {
    for value in values {
        match &value.predicate {
            RequirementPredicate::NodeRank {
                graph_id, node_id, ..
            } => {
                output.insert((graph_id.clone(), node_id.clone()));
            }
            RequirementPredicate::All { requirements }
            | RequirementPredicate::Any { requirements } => {
                collect_node_dependencies(requirements, output);
            }
            RequirementPredicate::Not { requirement } => {
                collect_node_dependencies(std::slice::from_ref(requirement), output);
            }
            _ => {}
        }
    }
}

fn self_rank_requirement_is_invalid(
    values: &[RequirementDefinition],
    graph_id: &str,
    node_id: &str,
    rank: u8,
) -> bool {
    values.iter().any(|value| match &value.predicate {
        RequirementPredicate::NodeRank {
            graph_id: required_graph,
            node_id: required_node,
            minimum_rank,
        } => required_graph == graph_id && required_node == node_id && *minimum_rank >= rank,
        RequirementPredicate::All { requirements } | RequirementPredicate::Any { requirements } => {
            self_rank_requirement_is_invalid(requirements, graph_id, node_id, rank)
        }
        RequirementPredicate::Not { requirement } => self_rank_requirement_is_invalid(
            std::slice::from_ref(requirement),
            graph_id,
            node_id,
            rank,
        ),
        _ => false,
    })
}

fn validate_dependency_graph(
    dependencies: &BTreeMap<(String, String), BTreeSet<(String, String)>>,
) -> Result<(), WorldError> {
    fn visit(
        node: &(String, String),
        dependencies: &BTreeMap<(String, String), BTreeSet<(String, String)>>,
        visiting: &mut BTreeSet<(String, String)>,
        visited: &mut BTreeSet<(String, String)>,
    ) -> bool {
        if visited.contains(node) {
            return true;
        }
        if !visiting.insert(node.clone()) {
            return false;
        }
        if dependencies
            .get(node)
            .into_iter()
            .flatten()
            .filter(|dependency| dependencies.contains_key(*dependency))
            .any(|dependency| !visit(dependency, dependencies, visiting, visited))
        {
            return false;
        }
        visiting.remove(node);
        visited.insert(node.clone());
        true
    }

    let mut visiting = BTreeSet::new();
    let mut visited = BTreeSet::new();
    if dependencies
        .keys()
        .all(|node| visit(node, dependencies, &mut visiting, &mut visited))
    {
        Ok(())
    } else {
        Err(WorldError::InvalidProgressionDefinition)
    }
}

fn validate_class_reachability(
    bases: &BTreeMap<String, BaseClassDefinition>,
    advanced: &BTreeMap<String, AdvancedClassDefinition>,
) -> Result<(), WorldError> {
    let mut reachable = bases.keys().cloned().collect::<BTreeSet<_>>();
    let mut remaining = advanced.keys().cloned().collect::<BTreeSet<_>>();
    loop {
        let found = advanced
            .values()
            .filter(|class| {
                remaining.contains(&class.id)
                    && class.from_class_ids.iter().any(|id| reachable.contains(id))
            })
            .map(|class| class.id.clone())
            .collect::<Vec<_>>();
        if found.is_empty() {
            break;
        }
        for id in found {
            remaining.remove(&id);
            reachable.insert(id);
        }
    }
    if remaining.is_empty() {
        Ok(())
    } else {
        Err(WorldError::InvalidProgressionDefinition)
    }
}

fn collect_unique<T>(
    values: Vec<T>,
    id: impl Fn(&T) -> &String,
) -> Result<BTreeMap<String, T>, WorldError> {
    let count = values.len();
    let values = values
        .into_iter()
        .map(|value| (id(&value).clone(), value))
        .collect::<BTreeMap<_, _>>();
    if values.len() == count {
        Ok(values)
    } else {
        Err(WorldError::InvalidProgressionDefinition)
    }
}

fn option_view(id: &str, name: &str, description: &str) -> ProgressionOptionView {
    ProgressionOptionView {
        id: id.into(),
        label: name.into(),
        description: description.into(),
    }
}

fn ordered_options<'a>(
    values: impl Iterator<Item = (u16, &'a String, &'a String, &'a String)>,
) -> Vec<ProgressionOptionView> {
    let mut values = values.collect::<Vec<_>>();
    values.sort_by(|left, right| (left.0, left.1).cmp(&(right.0, right.1)));
    values
        .into_iter()
        .map(|(_, id, name, description)| option_view(id, name, description))
        .collect()
}
