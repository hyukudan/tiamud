//! Data-driven quests, commodities and shops (WorldPack v10, ADR 0032).
//!
//! Definitions are immutable authored configuration installed before any
//! character enters. They are deliberately absent from runtime snapshots and
//! checkpoints; only per-character progress and per-stock-entry counters are
//! durable, keyed by the durable authored IDs.

use super::*;

pub const MAX_ADVENTURE_COMMODITIES: usize = 64;
pub const MAX_ADVENTURE_COMMODITY_OWNED: u32 = 999;
pub const MAX_ADVENTURE_QUESTS: usize = 256;
pub const MAX_ADVENTURE_QUEST_STAGES: usize = 16;
pub const MAX_ADVENTURE_QUEST_OBJECTIVES: usize = 8;
pub const MAX_ADVENTURE_QUEST_OBJECTIVE_COUNT: u32 = 1_000;
pub const MAX_ADVENTURE_QUEST_ALIASES: usize = 8;
pub const MAX_ADVENTURE_QUEST_REQUIREMENT_NODES: usize = 32;
pub const MAX_ADVENTURE_QUEST_REWARD: u64 = 1_000_000;
pub const MAX_ADVENTURE_SHOPS: usize = 64;
pub const MAX_ADVENTURE_SHOP_STOCK_ENTRIES: usize = 32;
pub const MAX_ADVENTURE_SHOP_PRICE: u64 = 1_000_000;
pub const MAX_ADVENTURE_SHOP_STOCK: u32 = 10_000;
/// Quests a character may hold at once, unchanged from the legacy cap.
pub const MAX_CHARACTER_QUESTS: usize = 32;
/// Quests projected into one scene snapshot, bounding the protocol frame.
pub const MAX_PROJECTED_QUESTS: usize = 16;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CommodityDefinition {
    pub id: String,
    pub label: String,
    pub max_owned: u32,
}

/// Closed prerequisite vocabulary translated from the authored WorldPack
/// `Condition` subset. It reads only the actor's own durable state.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum QuestRequirement {
    All(Vec<QuestRequirement>),
    Any(Vec<QuestRequirement>),
    Not(Box<QuestRequirement>),
    Level(QuestComparison, i64),
    HasFlag(String),
    HasQuestState(String),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum QuestComparison {
    Eq,
    Ne,
    Gt,
    Gte,
    Lt,
    Lte,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum QuestObjective {
    /// Empty `npc_ids` accepts any defeated NPC.
    DefeatNpc {
        npc_ids: Vec<String>,
        count: u32,
    },
    CollectItem {
        item_prototype_id: String,
        count: u32,
    },
    VisitRoom {
        room_id: String,
    },
    TalkToNpc {
        npc_ids: Vec<String>,
    },
}

impl QuestObjective {
    pub fn target(&self) -> u32 {
        match self {
            Self::DefeatNpc { count, .. } | Self::CollectItem { count, .. } => *count,
            Self::VisitRoom { .. } | Self::TalkToNpc { .. } => 1,
        }
    }

    fn state_based(&self) -> bool {
        matches!(self, Self::CollectItem { .. } | Self::VisitRoom { .. })
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct QuestStageDefinition {
    pub id: String,
    pub summary: Option<String>,
    pub objectives: Vec<QuestObjective>,
}

impl QuestStageDefinition {
    fn target(&self) -> u32 {
        self.objectives
            .iter()
            .map(QuestObjective::target)
            .fold(0_u32, u32::saturating_add)
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct QuestDefinition {
    pub id: String,
    pub name: String,
    pub summary: String,
    pub aliases: Vec<String>,
    pub prerequisite: Option<QuestRequirement>,
    pub repeatable: bool,
    pub stages: Vec<QuestStageDefinition>,
    pub reward_coins: u64,
    pub reward_experience: u64,
}

impl QuestDefinition {
    fn matches(&self, query: &str) -> bool {
        let query = query.trim().to_ascii_lowercase();
        self.id == query || self.aliases.contains(&query)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ShopRestock {
    None,
    Interval {
        every_ticks: u64,
        amount: u32,
        ceiling: u32,
    },
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ShopStockDefinition {
    pub stock_id: String,
    pub commodity_id: String,
    pub buy_price: u64,
    pub sell_price: u64,
    pub initial_stock: u32,
    pub max_stock: u32,
    pub restock: ShopRestock,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ShopDefinition {
    pub id: String,
    pub name: String,
    pub room_id: String,
    pub keeper_npc_id: Option<String>,
    pub stock: Vec<ShopStockDefinition>,
}

/// Complete immutable adventure catalog for one world.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct AdventureContent {
    pub commodities: Vec<CommodityDefinition>,
    pub quests: Vec<QuestDefinition>,
    pub shops: Vec<ShopDefinition>,
}

impl AdventureContent {
    /// The quest and market every runtime hardcoded before WorldPack v10.
    /// It remains the default for a `World` built directly from a
    /// `WorldDefinition`; loaded WorldPacks always replace it.
    pub fn legacy(start_room_id: &str) -> Self {
        let stock = |id: &str, buy_price, sell_price| ShopStockDefinition {
            stock_id: id.into(),
            commodity_id: id.into(),
            buy_price,
            sell_price,
            initial_stock: 20,
            max_stock: 100,
            restock: ShopRestock::None,
        };
        Self {
            commodities: vec![
                CommodityDefinition {
                    id: "tonic".into(),
                    label: "healing tonic".into(),
                    max_owned: 99,
                },
                CommodityDefinition {
                    id: "trail-ration".into(),
                    label: "trail ration".into(),
                    max_owned: 99,
                },
            ],
            quests: vec![QuestDefinition {
                id: "first-hunt".into(),
                name: "First Hunt".into(),
                summary: "Defeat one hostile creature; reward 25 coin and 50 experience.".into(),
                aliases: vec!["first hunt".into(), "hunt".into()],
                prerequisite: None,
                repeatable: false,
                stages: vec![QuestStageDefinition {
                    id: "hunt".into(),
                    summary: None,
                    objectives: vec![QuestObjective::DefeatNpc {
                        npc_ids: Vec::new(),
                        count: 1,
                    }],
                }],
                reward_coins: 25,
                reward_experience: 50,
            }],
            shops: vec![ShopDefinition {
                id: "lantern-market".into(),
                name: "Lantern Market".into(),
                room_id: start_room_id.into(),
                keeper_npc_id: None,
                stock: vec![stock("tonic", 12, 6), stock("trail-ration", 8, 4)],
            }],
        }
    }

    pub fn commodity(&self, id: &str) -> Option<&CommodityDefinition> {
        self.commodities.iter().find(|commodity| commodity.id == id)
    }

    pub fn quest(&self, id: &str) -> Option<&QuestDefinition> {
        self.quests.iter().find(|quest| quest.id == id)
    }

    fn stock_entries(&self) -> impl Iterator<Item = &ShopStockDefinition> {
        self.shops.iter().flat_map(|shop| &shop.stock)
    }

    fn initial_stock(&self) -> BTreeMap<String, u32> {
        self.stock_entries()
            .map(|entry| (entry.stock_id.clone(), entry.initial_stock))
            .collect()
    }

    fn validate(&self) -> bool {
        let label_ok = |value: &str| {
            !value.trim().is_empty() && value.len() <= 512 && !value.chars().any(char::is_control)
        };
        let mut commodity_ids = BTreeSet::new();
        let commodities_ok = self.commodities.len() <= MAX_ADVENTURE_COMMODITIES
            && self.commodities.iter().all(|commodity| {
                valid_definition_id(&commodity.id)
                    && commodity_ids.insert(commodity.id.as_str())
                    && label_ok(&commodity.label)
                    && (1..=MAX_ADVENTURE_COMMODITY_OWNED).contains(&commodity.max_owned)
            });
        let mut quest_names = BTreeSet::new();
        let quests_ok = self.quests.len() <= MAX_ADVENTURE_QUESTS
            && self.quests.iter().all(|quest| {
                valid_definition_id(&quest.id)
                    && label_ok(&quest.name)
                    && label_ok(&quest.summary)
                    && quest.aliases.len() <= MAX_ADVENTURE_QUEST_ALIASES
                    && std::iter::once(&quest.id)
                        .chain(&quest.aliases)
                        .all(|name| label_ok(name) && quest_names.insert(name.as_str()))
                    && quest
                        .prerequisite
                        .as_ref()
                        .is_none_or(|requirement| requirement_size(requirement, 0).is_some())
                    && quest.reward_coins <= MAX_ADVENTURE_QUEST_REWARD
                    && quest.reward_experience <= MAX_ADVENTURE_QUEST_REWARD
                    && (1..=MAX_ADVENTURE_QUEST_STAGES).contains(&quest.stages.len())
                    && quest.stages.iter().all(|stage| {
                        valid_definition_id(&stage.id)
                            && stage.summary.as_deref().is_none_or(label_ok)
                            && (1..=MAX_ADVENTURE_QUEST_OBJECTIVES)
                                .contains(&stage.objectives.len())
                            && stage.objectives.iter().all(|objective| {
                                (1..=MAX_ADVENTURE_QUEST_OBJECTIVE_COUNT)
                                    .contains(&objective.target())
                                    && match objective {
                                        QuestObjective::TalkToNpc { npc_ids } => {
                                            !npc_ids.is_empty()
                                        }
                                        _ => true,
                                    }
                            })
                    })
            });
        let mut shop_rooms = BTreeSet::new();
        let mut stock_ids = BTreeSet::new();
        let shops_ok = self.shops.len() <= MAX_ADVENTURE_SHOPS
            && self.shops.iter().all(|shop| {
                valid_definition_id(&shop.id)
                    && label_ok(&shop.name)
                    && shop_rooms.insert(shop.room_id.as_str())
                    && (1..=MAX_ADVENTURE_SHOP_STOCK_ENTRIES).contains(&shop.stock.len())
                    && shop.stock.iter().all(|entry| {
                        valid_definition_id(&entry.stock_id)
                            && stock_ids.insert(entry.stock_id.as_str())
                            && commodity_ids.contains(entry.commodity_id.as_str())
                            && (1..=MAX_ADVENTURE_SHOP_PRICE).contains(&entry.buy_price)
                            && entry.sell_price <= entry.buy_price
                            && (1..=MAX_ADVENTURE_SHOP_STOCK).contains(&entry.max_stock)
                            && entry.initial_stock <= entry.max_stock
                            && match entry.restock {
                                ShopRestock::None => true,
                                ShopRestock::Interval {
                                    every_ticks,
                                    amount,
                                    ceiling,
                                } => {
                                    every_ticks > 0
                                        && amount > 0
                                        && (1..=entry.max_stock).contains(&ceiling)
                                }
                            }
                    })
                    && {
                        let mut seen = BTreeSet::new();
                        shop.stock
                            .iter()
                            .all(|entry| seen.insert(entry.commodity_id.as_str()))
                    }
            });
        commodities_ok && quests_ok && shops_ok
    }
}

fn requirement_size(requirement: &QuestRequirement, depth: usize) -> Option<usize> {
    if depth > 8 {
        return None;
    }
    let size = match requirement {
        QuestRequirement::All(children) | QuestRequirement::Any(children) => {
            if children.is_empty() {
                return None;
            }
            children.iter().try_fold(1_usize, |total, child| {
                requirement_size(child, depth + 1).map(|size| total.saturating_add(size))
            })?
        }
        QuestRequirement::Not(child) => requirement_size(child, depth + 1)?.saturating_add(1),
        QuestRequirement::Level(..) => 1,
        QuestRequirement::HasFlag(value) | QuestRequirement::HasQuestState(value) => {
            if value.trim().is_empty() || value.len() > 256 {
                return None;
            }
            1
        }
    };
    (size <= MAX_ADVENTURE_QUEST_REQUIREMENT_NODES).then_some(size)
}

fn status_label(status: QuestStatus) -> &'static str {
    match status {
        QuestStatus::Active => "active",
        QuestStatus::ReadyToClaim => "ready_to_claim",
        QuestStatus::Completed => "completed",
    }
}

/// Opaque per-character state strings readable by content conditions:
/// `<quest>:<status>` for every held quest plus `<quest>:stage:<stage>` while
/// the quest is active or ready to claim.
pub(crate) fn quest_state_strings(
    content: &AdventureContent,
    gameplay: &CharacterGameplaySnapshot,
) -> BTreeSet<String> {
    let mut states = BTreeSet::new();
    for (id, progress) in &gameplay.quests {
        states.insert(format!("{id}:{}", status_label(progress.status)));
        if progress.status != QuestStatus::Completed
            && let Some(stage) = content.quest(id).and_then(|quest| {
                let index = gameplay.quest_stages.get(id).map_or(0, |extra| extra.stage);
                quest.stages.get(usize::try_from(index).ok()?)
            })
        {
            states.insert(format!("{id}:stage:{}", stage.id));
        }
    }
    states
}

fn requirement_met(
    requirement: &QuestRequirement,
    gameplay: &CharacterGameplaySnapshot,
    quest_states: &BTreeSet<String>,
) -> bool {
    match requirement {
        QuestRequirement::All(children) => children
            .iter()
            .all(|child| requirement_met(child, gameplay, quest_states)),
        QuestRequirement::Any(children) => children
            .iter()
            .any(|child| requirement_met(child, gameplay, quest_states)),
        QuestRequirement::Not(child) => !requirement_met(child, gameplay, quest_states),
        QuestRequirement::Level(comparison, value) => {
            let level = i64::from(gameplay.level);
            match comparison {
                QuestComparison::Eq => level == *value,
                QuestComparison::Ne => level != *value,
                QuestComparison::Gt => level > *value,
                QuestComparison::Gte => level >= *value,
                QuestComparison::Lt => level < *value,
                QuestComparison::Lte => level <= *value,
            }
        }
        QuestRequirement::HasFlag(flag) => gameplay.script_flags.contains(flag),
        QuestRequirement::HasQuestState(state) => quest_states.contains(state),
    }
}

/// One authoritative fact that may advance an active objective.
#[derive(Clone, Copy, Debug)]
pub(crate) enum QuestTrigger<'a> {
    /// Re-evaluate only state-based objectives (visit and collect).
    State,
    DefeatedNpc(&'a str),
    TalkedTo(&'a str),
}

/// Validates a persisted character's quest and commodity state against the
/// configured catalog.
pub(crate) fn valid_character_adventure_state(
    content: &AdventureContent,
    snapshot: &CharacterGameplaySnapshot,
) -> bool {
    let commodities_ok = snapshot.commodities.len() <= content.commodities.len()
        && snapshot.commodities.iter().all(|(id, amount)| {
            content
                .commodity(id)
                .is_some_and(|commodity| *amount <= commodity.max_owned)
        });
    let quests_ok = snapshot.quests.len() <= MAX_CHARACTER_QUESTS
        && snapshot
            .quest_stages
            .keys()
            .all(|id| snapshot.quests.contains_key(id))
        && snapshot
            .quest_stages
            .values()
            .all(|extra| !extra.is_default())
        && snapshot.quests.iter().all(|(id, progress)| {
            let Some(quest) = content.quest(id) else {
                return false;
            };
            let extra = snapshot.quest_stages.get(id).cloned().unwrap_or_default();
            let Some(stage) = usize::try_from(extra.stage)
                .ok()
                .and_then(|index| quest.stages.get(index))
            else {
                return false;
            };
            let last_stage = usize::try_from(extra.stage).ok() == Some(quest.stages.len() - 1);
            let counts_ok = if stage.objectives.len() == 1 {
                extra.objectives.is_empty()
            } else {
                extra.objectives.len() == stage.objectives.len()
                    && extra
                        .objectives
                        .iter()
                        .zip(&stage.objectives)
                        .all(|(count, objective)| *count <= objective.target())
                    && progress.current
                        == extra
                            .objectives
                            .iter()
                            .fold(0_u32, |total, count| total.saturating_add(*count))
            };
            (quest.repeatable || extra.completions == 0)
                && progress.target == stage.target()
                && progress.current <= progress.target
                && counts_ok
                && match progress.status {
                    QuestStatus::Active => progress.current < progress.target,
                    QuestStatus::ReadyToClaim | QuestStatus::Completed => {
                        last_stage && progress.current == progress.target
                    }
                }
        });
    commodities_ok && quests_ok
}

fn new_progress(stage: &QuestStageDefinition) -> (QuestProgress, Vec<u32>) {
    (
        QuestProgress {
            status: QuestStatus::Active,
            current: 0,
            target: stage.target(),
        },
        if stage.objectives.len() > 1 {
            vec![0; stage.objectives.len()]
        } else {
            Vec::new()
        },
    )
}

fn reward_text(quest: &QuestDefinition) -> Option<String> {
    match (quest.reward_coins, quest.reward_experience) {
        (0, 0) => None,
        (coins, 0) => Some(format!("{coins} coin")),
        (0, experience) => Some(format!("{experience} experience")),
        (coins, experience) => Some(format!("{coins} coin and {experience} experience")),
    }
}

fn lower_first(value: &str) -> String {
    let mut characters = value.chars();
    characters.next().map_or_else(String::new, |first| {
        first.to_lowercase().collect::<String>() + characters.as_str()
    })
}

impl World {
    /// Installs the immutable quest, commodity and shop catalog and resets
    /// every shop stock counter to its authored initial value. It must run
    /// before characters enter and before a runtime snapshot is restored.
    pub fn configure_adventure_content(
        &mut self,
        content: AdventureContent,
    ) -> Result<(), WorldError> {
        if !content.validate() || !self.character_gameplay.is_empty() {
            return Err(WorldError::InvalidGameplayDefinition);
        }
        let stock = content.initial_stock();
        if self.adventure.shop_stock != stock {
            self.adventure.shop_stock = stock;
        }
        self.adventure_content = std::sync::Arc::new(content);
        Ok(())
    }

    pub fn adventure_content(&self) -> &AdventureContent {
        &self.adventure_content
    }

    /// Restored stock must cover exactly the configured stock entries and
    /// stay within each entry's authored maximum.
    pub(crate) fn valid_shop_stock(&self, stock: &BTreeMap<String, u32>) -> bool {
        let entries = self.adventure_content.stock_entries().collect::<Vec<_>>();
        stock.len() == entries.len()
            && entries.iter().all(|entry| {
                stock
                    .get(&entry.stock_id)
                    .is_some_and(|value| *value <= entry.max_stock)
            })
    }

    pub(crate) fn shop_in_room(&self, room_id: &str) -> Option<&ShopDefinition> {
        self.adventure_content
            .shops
            .iter()
            .find(|shop| shop.room_id == room_id)
            .filter(|shop| {
                shop.keeper_npc_id.as_ref().is_none_or(|keeper| {
                    self.npcs
                        .get(keeper)
                        .is_some_and(|npc| npc.alive() && npc.spawn_room_id == room_id)
                })
            })
    }

    pub(crate) fn resolve_shop_stock<'a>(
        shop: &'a ShopDefinition,
        content: &'a AdventureContent,
        query: &str,
    ) -> Result<(&'a ShopStockDefinition, &'a CommodityDefinition), WorldError> {
        let query = query.trim().to_ascii_lowercase();
        shop.stock
            .iter()
            .filter_map(|entry| {
                content
                    .commodity(&entry.commodity_id)
                    .map(|commodity| (entry, commodity))
            })
            .find(|(_, commodity)| {
                commodity.id == query
                    || commodity.label.eq_ignore_ascii_case(&query)
                    || commodity.label.starts_with(&query)
            })
            .ok_or(WorldError::CommodityNotFound)
    }

    pub(crate) fn quest_prerequisite_met(
        &self,
        quest: &QuestDefinition,
        gameplay: &CharacterGameplaySnapshot,
    ) -> bool {
        quest.prerequisite.as_ref().is_none_or(|requirement| {
            let states = quest_state_strings(&self.adventure_content, gameplay);
            requirement_met(requirement, gameplay, &states)
        })
    }

    /// Quests a character can see: held ones plus those whose prerequisite
    /// passes, in authored (canonical ID) order.
    pub(crate) fn visible_quests<'a>(
        &'a self,
        gameplay: &'a CharacterGameplaySnapshot,
    ) -> impl Iterator<Item = &'a QuestDefinition> + 'a {
        self.adventure_content.quests.iter().filter(move |quest| {
            gameplay.quests.contains_key(&quest.id) || self.quest_prerequisite_met(quest, gameplay)
        })
    }

    pub(crate) fn quest_list_text(&self, gameplay: &CharacterGameplaySnapshot) -> String {
        let lines = self
            .visible_quests(gameplay)
            .map(|quest| {
                format!(
                    "Available quest: {} — {}\r\n",
                    quest.id,
                    lower_first(&quest.summary)
                )
            })
            .collect::<String>();
        if lines.is_empty() {
            "No quests are available.\r\n".into()
        } else {
            lines
        }
    }

    pub(crate) fn quest_status_text(&self, gameplay: &CharacterGameplaySnapshot) -> String {
        if gameplay.quests.is_empty() {
            return match self.adventure_content.quests.as_slice() {
                [quest] => format!("The {} quest has not been accepted.\r\n", quest.id),
                _ => "You have not accepted any quests.\r\n".into(),
            };
        }
        gameplay
            .quests
            .iter()
            .map(|(id, progress)| {
                let quest = self.adventure_content.quest(id);
                let name = quest.map_or(id.as_str(), |quest| quest.name.as_str());
                let stage = quest
                    .filter(|quest| quest.stages.len() > 1)
                    .map(|quest| {
                        let index = gameplay.quest_stages.get(id).map_or(0, |extra| extra.stage);
                        format!(", stage {}/{}", index.saturating_add(1), quest.stages.len())
                    })
                    .unwrap_or_default();
                format!(
                    "{name}: {:?} ({}/{}){stage}.\r\n",
                    progress.status, progress.current, progress.target
                )
            })
            .collect()
    }

    pub(crate) fn accept_quest(
        &mut self,
        session: SessionId,
        query: &str,
    ) -> Result<ActionOutcome, WorldError> {
        let character_id = self.character(session)?.id;
        let content = std::sync::Arc::clone(&self.adventure_content);
        let quest = content
            .quests
            .iter()
            .find(|quest| quest.matches(query))
            .ok_or(WorldError::QuestUnavailable)?;
        let gameplay = self
            .character_gameplay
            .get(&character_id)
            .ok_or(WorldError::InvalidEntity)?;
        let previous_completions = match gameplay.quests.get(&quest.id) {
            None if gameplay.quests.len() >= MAX_CHARACTER_QUESTS => {
                return Err(WorldError::QuestUnavailable);
            }
            None => 0,
            Some(progress) if quest.repeatable && progress.status == QuestStatus::Completed => {
                gameplay
                    .quest_stages
                    .get(&quest.id)
                    .map_or(0, |extra| extra.completions)
            }
            Some(_) => return Err(WorldError::QuestUnavailable),
        };
        if !self.quest_prerequisite_met(quest, gameplay) {
            return Err(WorldError::QuestUnavailable);
        }
        let (progress, objectives) = new_progress(&quest.stages[0]);
        let extra = QuestStageProgress {
            stage: 0,
            objectives,
            completions: previous_completions,
        };
        let state = self
            .character_gameplay
            .get_mut(&character_id)
            .ok_or(WorldError::InvalidEntity)?;
        state.quests.insert(quest.id.clone(), progress);
        if extra.is_default() {
            state.quest_stages.remove(&quest.id);
        } else {
            state.quest_stages.insert(quest.id.clone(), extra);
        }
        self.finish_adventure_action(
            session,
            format!("You accept {}.\r\n", quest.name),
            AdventureEvent::QuestChanged {
                character_id,
                quest_id: quest.id.clone(),
                summary: format!("{} accepted.", quest.name),
            },
        )
    }

    pub(crate) fn complete_quest(
        &mut self,
        session: SessionId,
        query: &str,
    ) -> Result<ActionOutcome, WorldError> {
        let character = self.character(session)?;
        let character_id = character.id;
        let character_name = character.name.clone();
        let room_id = room_checked(&self.entities, character.location)?
            .stable_id
            .clone();
        let content = std::sync::Arc::clone(&self.adventure_content);
        let quest = content
            .quests
            .iter()
            .find(|quest| quest.matches(query))
            .ok_or(WorldError::QuestUnavailable)?;
        let rewarded_experience = {
            let state = self
                .character_gameplay
                .get_mut(&character_id)
                .ok_or(WorldError::InvalidEntity)?;
            if state
                .quests
                .get(&quest.id)
                .is_none_or(|progress| progress.status != QuestStatus::ReadyToClaim)
            {
                return Err(WorldError::QuestUnavailable);
            }
            let coins = state
                .coins
                .checked_add(quest.reward_coins)
                .ok_or(WorldError::QuestUnavailable)?;
            let experience = state
                .experience
                .checked_add(quest.reward_experience)
                .ok_or(WorldError::QuestUnavailable)?;
            let completions = if quest.repeatable {
                let previous = state
                    .quest_stages
                    .get(&quest.id)
                    .map_or(0, |extra| extra.completions);
                Some(
                    previous
                        .checked_add(1)
                        .ok_or(WorldError::QuestUnavailable)?,
                )
            } else {
                None
            };
            state.coins = coins;
            state.experience = experience;
            if let Some(progress) = state.quests.get_mut(&quest.id) {
                progress.status = QuestStatus::Completed;
            }
            if let Some(completions) = completions {
                let extra = state.quest_stages.entry(quest.id.clone()).or_default();
                extra.completions = completions;
            }
            state.experience
        };
        let mut progression_events = Vec::new();
        if quest.reward_experience > 0 {
            progression_events.push(DomainEvent::Gameplay {
                event: GameplayEvent::ExperienceGained {
                    character_id,
                    character_name: character_name.clone(),
                    amount: quest.reward_experience,
                    total: rewarded_experience,
                    room_id: room_id.clone(),
                },
            });
            self.advance_character_progression(
                character_id,
                &character_name,
                &room_id,
                &mut progression_events,
            )?;
        }
        let output = match reward_text(quest) {
            Some(reward) => format!("{} complete: you receive {reward}.\r\n", quest.name),
            None => format!("{} complete.\r\n", quest.name),
        };
        let mut outcome = self.finish_adventure_action(
            session,
            output,
            AdventureEvent::QuestChanged {
                character_id,
                quest_id: quest.id.clone(),
                summary: format!("{} completed and its reward claimed.", quest.name),
            },
        )?;
        outcome.domain_events.extend(progression_events);
        Ok(outcome)
    }

    pub(crate) fn talk_to_npc(
        &mut self,
        session: SessionId,
        query: &str,
    ) -> Result<ActionOutcome, WorldError> {
        let character_id = self.character(session)?.id;
        if self
            .character_gameplay
            .get(&character_id)
            .ok_or(WorldError::InvalidEntity)?
            .recovery_at_tick
            .is_some()
        {
            return Err(WorldError::GameplayUnavailable);
        }
        let npc_id = self.resolve_visible_npc(session, query)?;
        let npc_name = self
            .npcs
            .get(&npc_id)
            .ok_or(WorldError::NpcNotFound)?
            .name
            .clone();
        let events = self.advance_quests(character_id, QuestTrigger::TalkedTo(&npc_id))?;
        let output = format!("You talk with {npc_name}.\r\n");
        if events.is_empty() {
            return Ok(ActionOutcome {
                scene: self.scene_for(session)?,
                container_contents: None,
                trade_contents: None,
                reward_odds: None,
                output: Some(output),
                changed_character: None,
                domain_events: Vec::new(),
            });
        }
        self.revision = self.revision.wrapping_add(1).max(1);
        self.bump_view_revisions_for_events(&events)?;
        Ok(ActionOutcome {
            scene: self.scene_for(session)?,
            container_contents: None,
            trade_contents: None,
            reward_odds: None,
            output: Some(output),
            changed_character: None,
            domain_events: events,
        })
    }

    /// Re-evaluates state-based objectives after a successful action so a
    /// visit or collection completes in the same atomic transition.
    pub(crate) fn refresh_state_quests(
        &mut self,
        session: SessionId,
        outcome: &mut ActionOutcome,
    ) -> Result<(), WorldError> {
        let Ok(character) = self.character(session) else {
            return Ok(());
        };
        let character_id = character.id;
        let needs_refresh = self
            .character_gameplay
            .get(&character_id)
            .is_some_and(|gameplay| {
                gameplay.quests.iter().any(|(id, progress)| {
                    progress.status == QuestStatus::Active
                        && self.current_stage(id, gameplay).is_some_and(|stage| {
                            stage.objectives.iter().any(QuestObjective::state_based)
                        })
                })
            });
        if !needs_refresh {
            return Ok(());
        }
        let events = self.advance_quests(character_id, QuestTrigger::State)?;
        if events.is_empty() {
            return Ok(());
        }
        self.revision = self.revision.wrapping_add(1).max(1);
        self.bump_view_revisions_for_events(&events)?;
        outcome.scene = self.scene_for(session)?;
        outcome.domain_events.extend(events);
        Ok(())
    }

    fn current_stage<'a>(
        &'a self,
        quest_id: &str,
        gameplay: &CharacterGameplaySnapshot,
    ) -> Option<&'a QuestStageDefinition> {
        let index = gameplay
            .quest_stages
            .get(quest_id)
            .map_or(0, |extra| extra.stage);
        self.adventure_content
            .quest(quest_id)?
            .stages
            .get(usize::try_from(index).ok()?)
    }

    fn carried_units(&self, character_id: CharacterId, prototype_id: &str) -> u32 {
        let mut frontier = self
            .item_instances
            .ids_at(&SharedItemLocation::Carried(character_id))
            .cloned()
            .collect::<Vec<_>>();
        if let Some(gameplay) = self.character_gameplay.get(&character_id) {
            frontier.extend(gameplay.equipment.values().cloned());
        }
        let mut count = 0_u32;
        for _ in 0..=MAX_CONTAINER_DEPTH {
            if frontier.is_empty() {
                break;
            }
            let mut next = Vec::new();
            for item_id in &frontier {
                if self
                    .item_instances
                    .get(item_id)
                    .is_some_and(|instance| instance.prototype_id == prototype_id)
                {
                    count = count.saturating_add(1);
                }
                next.extend(
                    self.item_instances
                        .ids_at(&SharedItemLocation::Contained(item_id.clone()))
                        .cloned(),
                );
            }
            frontier = next;
        }
        count
    }

    /// Applies one trigger to every active quest of a character. Mutable
    /// access happens only when a quest's progress actually changes, so an
    /// irrelevant trigger never dirties durable state.
    pub(crate) fn advance_quests(
        &mut self,
        character_id: CharacterId,
        trigger: QuestTrigger<'_>,
    ) -> Result<Vec<DomainEvent>, WorldError> {
        let Some(gameplay) = self.character_gameplay.get(&character_id) else {
            return Ok(Vec::new());
        };
        let room_id = self
            .session_for_character(character_id)
            .and_then(|session| self.character(session).ok())
            .and_then(|character| room_checked(&self.entities, character.location).ok())
            .map(|room| room.stable_id.clone());
        let content = std::sync::Arc::clone(&self.adventure_content);
        let mut updates = Vec::new();
        for (quest_id, progress) in &gameplay.quests {
            if progress.status != QuestStatus::Active {
                continue;
            }
            let Some(quest) = content.quest(quest_id) else {
                continue;
            };
            let mut extra = gameplay
                .quest_stages
                .get(quest_id)
                .cloned()
                .unwrap_or_default();
            let mut next = progress.clone();
            let mut trigger = Some(trigger);
            let mut changed = false;
            let mut advanced_stage = false;
            loop {
                let Some(stage) = usize::try_from(extra.stage)
                    .ok()
                    .and_then(|index| quest.stages.get(index))
                else {
                    return Err(WorldError::InvalidEntity);
                };
                let mut counts = if stage.objectives.len() == 1 {
                    vec![next.current]
                } else {
                    extra.objectives.clone()
                };
                if counts.len() != stage.objectives.len() {
                    return Err(WorldError::InvalidEntity);
                }
                for (count, objective) in counts.iter_mut().zip(&stage.objectives) {
                    let target = objective.target();
                    let updated = match (objective, trigger) {
                        (
                            QuestObjective::DefeatNpc { npc_ids, .. },
                            Some(QuestTrigger::DefeatedNpc(npc_id)),
                        ) if npc_ids.is_empty() || npc_ids.iter().any(|id| id == npc_id) => {
                            count.saturating_add(1)
                        }
                        (
                            QuestObjective::TalkToNpc { npc_ids },
                            Some(QuestTrigger::TalkedTo(npc_id)),
                        ) if npc_ids.iter().any(|id| id == npc_id) => 1,
                        (
                            QuestObjective::VisitRoom {
                                room_id: target_room,
                            },
                            _,
                        ) if room_id.as_deref() == Some(target_room.as_str()) => 1,
                        (
                            QuestObjective::CollectItem {
                                item_prototype_id, ..
                            },
                            _,
                        ) => self.carried_units(character_id, item_prototype_id),
                        _ => *count,
                    }
                    .min(target);
                    if updated != *count {
                        *count = updated;
                        changed = true;
                    }
                }
                let complete = counts
                    .iter()
                    .zip(&stage.objectives)
                    .all(|(count, objective)| *count >= objective.target());
                let total = counts
                    .iter()
                    .fold(0_u32, |sum, count| sum.saturating_add(*count));
                next.current = total.min(next.target);
                extra.objectives = if stage.objectives.len() > 1 {
                    counts
                } else {
                    Vec::new()
                };
                if !complete {
                    break;
                }
                let last = usize::try_from(extra.stage).ok() == Some(quest.stages.len() - 1);
                if last {
                    next.status = QuestStatus::ReadyToClaim;
                    changed = true;
                    break;
                }
                extra.stage = extra.stage.saturating_add(1);
                let following = &quest.stages
                    [usize::try_from(extra.stage).map_err(|_| WorldError::InvalidEntity)?];
                let (fresh, objectives) = new_progress(following);
                next = fresh;
                extra.objectives = objectives;
                changed = true;
                advanced_stage = true;
                // A single authoritative fact advances at most one stage;
                // later stages only re-check state-based objectives.
                trigger = None;
            }
            if changed {
                let summary = match next.status {
                    QuestStatus::ReadyToClaim => format!("{} is ready to claim.", quest.name),
                    _ if advanced_stage => {
                        format!("{} advances to a new stage.", quest.name)
                    }
                    _ => format!("{} progresses.", quest.name),
                };
                updates.push((quest_id.clone(), next, extra, summary));
            }
        }
        if updates.is_empty() {
            return Ok(Vec::new());
        }
        let state = self
            .character_gameplay
            .get_mut(&character_id)
            .ok_or(WorldError::InvalidEntity)?;
        let mut events = Vec::with_capacity(updates.len());
        for (quest_id, progress, extra, summary) in updates {
            state.quests.insert(quest_id.clone(), progress);
            if extra.is_default() {
                state.quest_stages.remove(&quest_id);
            } else {
                state.quest_stages.insert(quest_id.clone(), extra);
            }
            events.push(DomainEvent::Adventure {
                event: AdventureEvent::QuestChanged {
                    character_id,
                    quest_id,
                    summary,
                },
            });
        }
        Ok(events)
    }

    /// Applies authored interval restocks for one logical tick.
    pub(crate) fn restock_shops(&mut self, tick: u64) {
        let content = std::sync::Arc::clone(&self.adventure_content);
        let updates = content
            .stock_entries()
            .filter_map(|entry| match entry.restock {
                ShopRestock::Interval {
                    every_ticks,
                    amount,
                    ceiling,
                } if every_ticks > 0 && tick.is_multiple_of(every_ticks) => {
                    let current = self.adventure.shop_stock.get(&entry.stock_id).copied()?;
                    let restocked = if current >= ceiling {
                        current
                    } else {
                        current.saturating_add(amount).min(ceiling)
                    };
                    (restocked != current).then(|| (entry.stock_id.clone(), restocked))
                }
                _ => None,
            })
            .collect::<Vec<_>>();
        for (stock_id, value) in updates {
            self.adventure.shop_stock.insert(stock_id, value);
        }
    }
}
