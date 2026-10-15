//! WorldPack v10 data-driven quests, commodities and shops (ADR 0032).
//!
//! Every collection is closed, bounded and validated before the runtime sees
//! it. Display text is localized and resolved by the loader; identifiers are
//! durable once published because character quest progress, commodity
//! balances and runtime shop stock are keyed by them.

use std::collections::{HashMap, HashSet};

use serde::{Deserialize, Serialize};

use crate::{
    Condition, ConditionField, ConditionValue, LocalizedText, ValidationError, invalid_gameplay,
    valid_gameplay_identifier,
};

pub const MAX_COMMODITIES: usize = 64;
pub const MAX_COMMODITY_OWNED: u32 = 999;
pub const MAX_QUESTS: usize = 256;
pub const MAX_QUEST_ALIASES: usize = 8;
pub const MAX_QUEST_ALIAS_BYTES: usize = 64;
pub const MAX_QUEST_STAGES: usize = 16;
pub const MAX_QUEST_OBJECTIVES_PER_STAGE: usize = 8;
pub const MAX_QUEST_OBJECTIVE_COUNT: u32 = 1_000;
pub const MAX_QUEST_REWARD_COINS: u64 = 1_000_000;
pub const MAX_QUEST_REWARD_EXPERIENCE: u64 = 1_000_000;
pub const MAX_QUEST_PREREQUISITE_NODES: usize = 32;
pub const MAX_QUEST_PREREQUISITE_DEPTH: usize = 4;
pub const MAX_SHOPS: usize = 64;
pub const MAX_SHOP_STOCK_ENTRIES: usize = 32;
pub const MAX_SHOP_PRICE: u64 = 1_000_000;
pub const MAX_SHOP_STOCK: u32 = 10_000;
pub const MAX_SHOP_RESTOCK_INTERVAL_TICKS: u32 = 100_000;
pub const MAX_ADVENTURE_LABEL_BYTES: usize = 128;
pub const MAX_ADVENTURE_SUMMARY_BYTES: usize = 512;
const MAX_ADVENTURE_LOCALES: usize = 16;

/// Abstract counted goods held per character. They are not item instances.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CommodityDefinition {
    pub id: String,
    pub name: LocalizedText,
    pub max_owned: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum QuestRepeatability {
    /// The reward can be claimed exactly once per character.
    Once,
    /// After the reward is claimed the quest may be accepted again; each
    /// claim is still a single atomic, exactly-once transition.
    Repeatable,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct QuestRewardDefinition {
    #[serde(default)]
    pub coins: u64,
    #[serde(default)]
    pub experience: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(
    tag = "kind",
    rename_all = "snake_case",
    rename_all_fields = "camelCase",
    deny_unknown_fields
)]
pub enum QuestObjectiveDefinition {
    /// Defeat `count` NPCs. Without a prototype any defeated NPC counts.
    DefeatNpc {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        npc_prototype_id: Option<String>,
        count: u32,
    },
    /// Carry `count` units of one item prototype at the same time.
    CollectItem {
        item_prototype_id: String,
        count: u32,
    },
    /// Stand in one room.
    VisitRoom { room_id: String },
    /// Use `talk <npc>` on a living NPC of one prototype.
    TalkToNpc { npc_prototype_id: String },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct QuestStageDefinition {
    pub id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub summary: Option<LocalizedText>,
    pub objectives: Vec<QuestObjectiveDefinition>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct QuestDefinition {
    pub id: String,
    pub name: LocalizedText,
    pub summary: LocalizedText,
    /// Extra case-insensitive command names accepted by `quest accept`.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub aliases: Vec<String>,
    /// Closed subset of [`Condition`]: `all`, `any`, `not`, integer
    /// comparisons on `actor.level` and `contains` on `actor.flags` or
    /// `actor.quest_states`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub prerequisite: Option<Condition>,
    pub repeatability: QuestRepeatability,
    pub stages: Vec<QuestStageDefinition>,
    #[serde(default)]
    pub rewards: QuestRewardDefinition,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(
    tag = "kind",
    rename_all = "snake_case",
    rename_all_fields = "camelCase",
    deny_unknown_fields
)]
pub enum ShopRestockPolicy {
    None,
    /// Every `everyTicks` logical ticks add `amount`, never above `ceiling`.
    Interval {
        every_ticks: u32,
        amount: u32,
        ceiling: u32,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ShopStockEntryDefinition {
    /// Durable runtime stock key, unique across every shop in the world.
    pub stock_id: String,
    pub commodity_id: String,
    pub buy_price: u64,
    pub sell_price: u64,
    pub initial_stock: u32,
    pub max_stock: u32,
    pub restock: ShopRestockPolicy,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ShopDefinition {
    pub id: String,
    pub name: LocalizedText,
    pub room_id: String,
    /// Optional NPC spawn that must be alive in the shop room to trade.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub keeper_npc_spawn_id: Option<String>,
    pub stock: Vec<ShopStockEntryDefinition>,
}

/// Content implied by every WorldPack before v10. Normalizing an older pack
/// makes the formerly hardcoded runtime behaviour explicit instead of
/// silently dropping the starter quest and market.
pub fn legacy_adventure_content(
    start_room_id: &str,
) -> (
    Vec<CommodityDefinition>,
    Vec<QuestDefinition>,
    Vec<ShopDefinition>,
) {
    let english = |value: &str| LocalizedText([("en".to_owned(), value.to_owned())].into());
    let commodities = vec![
        CommodityDefinition {
            id: "tonic".into(),
            name: english("healing tonic"),
            max_owned: 99,
        },
        CommodityDefinition {
            id: "trail-ration".into(),
            name: english("trail ration"),
            max_owned: 99,
        },
    ];
    let quests = vec![QuestDefinition {
        id: "first-hunt".into(),
        name: english("First Hunt"),
        summary: english("Defeat one hostile creature; reward 25 coin and 50 experience."),
        aliases: vec!["first hunt".into(), "hunt".into()],
        prerequisite: None,
        repeatability: QuestRepeatability::Once,
        stages: vec![QuestStageDefinition {
            id: "hunt".into(),
            summary: None,
            objectives: vec![QuestObjectiveDefinition::DefeatNpc {
                npc_prototype_id: None,
                count: 1,
            }],
        }],
        rewards: QuestRewardDefinition {
            coins: 25,
            experience: 50,
        },
    }];
    let stock = |stock_id: &str, buy_price, sell_price| ShopStockEntryDefinition {
        stock_id: stock_id.into(),
        commodity_id: stock_id.into(),
        buy_price,
        sell_price,
        initial_stock: 20,
        max_stock: 100,
        restock: ShopRestockPolicy::None,
    };
    let shops = if start_room_id.trim().is_empty() {
        Vec::new()
    } else {
        vec![ShopDefinition {
            id: "lantern-market".into(),
            name: english("Lantern Market"),
            room_id: start_room_id.to_owned(),
            keeper_npc_spawn_id: None,
            stock: vec![stock("tonic", 12, 6), stock("trail-ration", 8, 4)],
        }]
    };
    (commodities, quests, shops)
}

fn valid_localized(text: &LocalizedText, maximum_bytes: usize) -> bool {
    !text.0.is_empty()
        && text.0.len() <= MAX_ADVENTURE_LOCALES
        && text.0.iter().all(|(locale, value)| {
            (2..=35).contains(&locale.len())
                && locale
                    .bytes()
                    .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-')
                && !value.trim().is_empty()
                && value.trim() == value
                && value.len() <= maximum_bytes
                && !value.chars().any(char::is_control)
        })
}

pub(crate) struct AdventureReferences<'a> {
    pub(crate) rooms: &'a HashSet<&'a str>,
    pub(crate) item_prototypes: &'a HashSet<&'a str>,
    pub(crate) npc_prototypes: &'a HashSet<&'a str>,
    /// Spawn id to its room id.
    pub(crate) npc_spawns: &'a HashMap<&'a str, &'a str>,
}

pub(crate) fn validate_adventure(
    commodities: &[CommodityDefinition],
    quests: &[QuestDefinition],
    shops: &[ShopDefinition],
    references: &AdventureReferences<'_>,
    errors: &mut Vec<ValidationError>,
) {
    if commodities.len() > MAX_COMMODITIES {
        invalid_gameplay(
            errors,
            format!("at most {MAX_COMMODITIES} commodities are allowed"),
        );
    }
    let mut commodity_ids = HashSet::new();
    for commodity in commodities {
        if !valid_gameplay_identifier(&commodity.id) || !commodity_ids.insert(commodity.id.as_str())
        {
            invalid_gameplay(
                errors,
                format!(
                    "commodity {:?} has an invalid or duplicate id",
                    commodity.id
                ),
            );
        }
        if !valid_localized(&commodity.name, MAX_ADVENTURE_LABEL_BYTES) {
            invalid_gameplay(
                errors,
                format!(
                    "commodity {:?} name must be safe localized text",
                    commodity.id
                ),
            );
        }
        if !(1..=MAX_COMMODITY_OWNED).contains(&commodity.max_owned) {
            invalid_gameplay(
                errors,
                format!(
                    "commodity {:?} maxOwned must be 1..={MAX_COMMODITY_OWNED}",
                    commodity.id
                ),
            );
        }
    }

    if quests.len() > MAX_QUESTS {
        invalid_gameplay(errors, format!("at most {MAX_QUESTS} quests are allowed"));
    }
    let mut quest_ids = HashSet::new();
    let mut quest_names = HashSet::new();
    for quest in quests {
        validate_quest(quest, references, errors);
        if !quest_ids.insert(quest.id.as_str()) {
            invalid_gameplay(
                errors,
                format!("quest {:?} appears more than once", quest.id),
            );
        }
        for name in std::iter::once(&quest.id).chain(&quest.aliases) {
            if !quest_names.insert(name.to_ascii_lowercase()) {
                invalid_gameplay(
                    errors,
                    format!("quest command name {name:?} is ambiguous across quests or aliases"),
                );
            }
        }
    }

    if shops.len() > MAX_SHOPS {
        invalid_gameplay(errors, format!("at most {MAX_SHOPS} shops are allowed"));
    }
    let mut shop_ids = HashSet::new();
    let mut shop_rooms = HashSet::new();
    let mut stock_ids = HashSet::new();
    for shop in shops {
        if !valid_gameplay_identifier(&shop.id) || !shop_ids.insert(shop.id.as_str()) {
            invalid_gameplay(
                errors,
                format!("shop {:?} has an invalid or duplicate id", shop.id),
            );
        }
        if !valid_localized(&shop.name, MAX_ADVENTURE_LABEL_BYTES) {
            invalid_gameplay(
                errors,
                format!("shop {:?} name must be safe localized text", shop.id),
            );
        }
        if !references.rooms.contains(shop.room_id.as_str()) {
            invalid_gameplay(
                errors,
                format!(
                    "shop {:?} references unknown room {:?}",
                    shop.id, shop.room_id
                ),
            );
        } else if !shop_rooms.insert(shop.room_id.as_str()) {
            invalid_gameplay(
                errors,
                format!("room {:?} hosts more than one shop", shop.room_id),
            );
        }
        if let Some(keeper) = &shop.keeper_npc_spawn_id
            && references.npc_spawns.get(keeper.as_str()) != Some(&shop.room_id.as_str())
        {
            invalid_gameplay(
                errors,
                format!(
                    "shop {:?} keeper {keeper:?} must be an NPC spawn in the shop room",
                    shop.id
                ),
            );
        }
        if shop.stock.is_empty() || shop.stock.len() > MAX_SHOP_STOCK_ENTRIES {
            invalid_gameplay(
                errors,
                format!(
                    "shop {:?} must list 1..={MAX_SHOP_STOCK_ENTRIES} stock entries",
                    shop.id
                ),
            );
        }
        let mut shop_commodities = HashSet::new();
        for entry in &shop.stock {
            validate_stock_entry(
                shop,
                entry,
                &commodity_ids,
                &mut stock_ids,
                &mut shop_commodities,
                errors,
            );
        }
    }
}

fn validate_stock_entry<'a>(
    shop: &ShopDefinition,
    entry: &'a ShopStockEntryDefinition,
    commodity_ids: &HashSet<&str>,
    stock_ids: &mut HashSet<&'a str>,
    shop_commodities: &mut HashSet<&'a str>,
    errors: &mut Vec<ValidationError>,
) {
    let label = format!("shop {:?} stock {:?}", shop.id, entry.stock_id);
    if !valid_gameplay_identifier(&entry.stock_id) || !stock_ids.insert(entry.stock_id.as_str()) {
        invalid_gameplay(
            errors,
            format!("{label} has an invalid or world-wide duplicate stockId"),
        );
    }
    if !commodity_ids.contains(entry.commodity_id.as_str()) {
        invalid_gameplay(
            errors,
            format!(
                "{label} references unknown commodity {:?}",
                entry.commodity_id
            ),
        );
    } else if !shop_commodities.insert(entry.commodity_id.as_str()) {
        invalid_gameplay(errors, format!("{label} repeats a commodity in one shop"));
    }
    if !(1..=MAX_SHOP_PRICE).contains(&entry.buy_price) || entry.sell_price > entry.buy_price {
        invalid_gameplay(
            errors,
            format!("{label} needs buyPrice 1..={MAX_SHOP_PRICE} and sellPrice <= buyPrice"),
        );
    }
    if !(1..=MAX_SHOP_STOCK).contains(&entry.max_stock) || entry.initial_stock > entry.max_stock {
        invalid_gameplay(
            errors,
            format!("{label} needs maxStock 1..={MAX_SHOP_STOCK} and initialStock <= maxStock"),
        );
    }
    if let ShopRestockPolicy::Interval {
        every_ticks,
        amount,
        ceiling,
    } = entry.restock
        && (!(1..=MAX_SHOP_RESTOCK_INTERVAL_TICKS).contains(&every_ticks)
            || amount == 0
            || amount > entry.max_stock
            || ceiling == 0
            || ceiling > entry.max_stock)
    {
        invalid_gameplay(
            errors,
            format!(
                "{label} interval restock needs everyTicks 1..={MAX_SHOP_RESTOCK_INTERVAL_TICKS} and amount/ceiling 1..=maxStock"
            ),
        );
    }
}

fn validate_quest(
    quest: &QuestDefinition,
    references: &AdventureReferences<'_>,
    errors: &mut Vec<ValidationError>,
) {
    let label = format!("quest {:?}", quest.id);
    if !valid_gameplay_identifier(&quest.id) {
        invalid_gameplay(errors, format!("{label} has an invalid id"));
    }
    if !valid_localized(&quest.name, MAX_ADVENTURE_LABEL_BYTES)
        || !valid_localized(&quest.summary, MAX_ADVENTURE_SUMMARY_BYTES)
    {
        invalid_gameplay(
            errors,
            format!("{label} name and summary must be safe localized text"),
        );
    }
    if quest.aliases.len() > MAX_QUEST_ALIASES
        || quest.aliases.iter().any(|alias| {
            alias.trim() != alias
                || alias.is_empty()
                || alias.len() > MAX_QUEST_ALIAS_BYTES
                || alias.chars().any(char::is_control)
                || alias.to_ascii_lowercase() != *alias
        })
    {
        invalid_gameplay(
            errors,
            format!(
                "{label} allows at most {MAX_QUEST_ALIASES} trimmed lowercase aliases of 1..={MAX_QUEST_ALIAS_BYTES} bytes"
            ),
        );
    }
    if let Some(prerequisite) = &quest.prerequisite {
        if let Err(error) = prerequisite.validate() {
            invalid_gameplay(errors, format!("{label} prerequisite is invalid: {error}"));
        }
        let mut nodes = 0;
        if !supported_prerequisite(prerequisite, 0, &mut nodes) {
            invalid_gameplay(
                errors,
                format!(
                    "{label} prerequisite may only use all/any/not, integer comparisons on actor.level and contains on actor.flags or actor.quest_states (at most {MAX_QUEST_PREREQUISITE_NODES} nodes, depth {MAX_QUEST_PREREQUISITE_DEPTH})"
                ),
            );
        }
    }
    if quest.rewards.coins > MAX_QUEST_REWARD_COINS
        || quest.rewards.experience > MAX_QUEST_REWARD_EXPERIENCE
    {
        invalid_gameplay(
            errors,
            format!(
                "{label} rewards are limited to {MAX_QUEST_REWARD_COINS} coin and {MAX_QUEST_REWARD_EXPERIENCE} experience"
            ),
        );
    }
    if quest.stages.is_empty() || quest.stages.len() > MAX_QUEST_STAGES {
        invalid_gameplay(
            errors,
            format!("{label} must declare 1..={MAX_QUEST_STAGES} stages"),
        );
    }
    let mut stage_ids = HashSet::new();
    for stage in &quest.stages {
        if !valid_gameplay_identifier(&stage.id) || !stage_ids.insert(stage.id.as_str()) {
            invalid_gameplay(
                errors,
                format!(
                    "{label} stage {:?} has an invalid or duplicate id",
                    stage.id
                ),
            );
        }
        if stage
            .summary
            .as_ref()
            .is_some_and(|summary| !valid_localized(summary, MAX_ADVENTURE_SUMMARY_BYTES))
        {
            invalid_gameplay(
                errors,
                format!(
                    "{label} stage {:?} summary must be safe localized text",
                    stage.id
                ),
            );
        }
        if stage.objectives.is_empty() || stage.objectives.len() > MAX_QUEST_OBJECTIVES_PER_STAGE {
            invalid_gameplay(
                errors,
                format!(
                    "{label} stage {:?} must declare 1..={MAX_QUEST_OBJECTIVES_PER_STAGE} objectives",
                    stage.id
                ),
            );
        }
        for objective in &stage.objectives {
            if let Some(message) = invalid_objective(objective, references) {
                invalid_gameplay(
                    errors,
                    format!("{label} stage {:?} objective {message}", stage.id),
                );
            }
        }
    }
}

fn invalid_objective(
    objective: &QuestObjectiveDefinition,
    references: &AdventureReferences<'_>,
) -> Option<String> {
    let valid_count = |count: u32| (1..=MAX_QUEST_OBJECTIVE_COUNT).contains(&count);
    match objective {
        QuestObjectiveDefinition::DefeatNpc {
            npc_prototype_id,
            count,
        } => {
            if !valid_count(*count) {
                return Some(format!("count must be 1..={MAX_QUEST_OBJECTIVE_COUNT}"));
            }
            npc_prototype_id
                .as_ref()
                .filter(|id| !references.npc_prototypes.contains(id.as_str()))
                .map(|id| format!("references unknown NPC prototype {id:?}"))
        }
        QuestObjectiveDefinition::CollectItem {
            item_prototype_id,
            count,
        } => {
            if !valid_count(*count) {
                return Some(format!("count must be 1..={MAX_QUEST_OBJECTIVE_COUNT}"));
            }
            (!references
                .item_prototypes
                .contains(item_prototype_id.as_str()))
            .then(|| format!("references unknown item prototype {item_prototype_id:?}"))
        }
        QuestObjectiveDefinition::VisitRoom { room_id } => {
            (!references.rooms.contains(room_id.as_str()))
                .then(|| format!("references unknown room {room_id:?}"))
        }
        QuestObjectiveDefinition::TalkToNpc { npc_prototype_id } => (!references
            .npc_prototypes
            .contains(npc_prototype_id.as_str()))
        .then(|| format!("references unknown NPC prototype {npc_prototype_id:?}")),
    }
}

fn supported_prerequisite(condition: &Condition, depth: usize, nodes: &mut usize) -> bool {
    *nodes += 1;
    if *nodes > MAX_QUEST_PREREQUISITE_NODES || depth > MAX_QUEST_PREREQUISITE_DEPTH {
        return false;
    }
    match condition {
        Condition::All(children) | Condition::Any(children) => {
            !children.is_empty()
                && children
                    .iter()
                    .all(|child| supported_prerequisite(child, depth + 1, nodes))
        }
        Condition::Not(child) => supported_prerequisite(child, depth + 1, nodes),
        Condition::Eq(ConditionField::ActorLevel, ConditionValue::Integer(_))
        | Condition::Ne(ConditionField::ActorLevel, ConditionValue::Integer(_))
        | Condition::Gt(ConditionField::ActorLevel, _)
        | Condition::Gte(ConditionField::ActorLevel, _)
        | Condition::Lt(ConditionField::ActorLevel, _)
        | Condition::Lte(ConditionField::ActorLevel, _)
        | Condition::Contains(ConditionField::ActorFlags | ConditionField::ActorQuestStates, _) => {
            true
        }
        _ => false,
    }
}
