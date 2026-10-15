//! Transport-neutral view DTOs projected to observers (scene, player
//! state, inventory, trade, map, quest, party, economy and social views).

use super::*;

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "kind", content = "roomId", rename_all = "camelCase")]
pub enum ItemLocationSnapshot {
    Carried,
    Room(String),
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CharacterSnapshot {
    pub id: CharacterId,
    pub account_id: AccountId,
    pub name: String,
    pub room_id: String,
    /// Legacy v2 per-character item state. Shared-world runtimes ignore this
    /// field, but retain it so existing persisted characters remain readable.
    #[serde(default)]
    pub item_locations: BTreeMap<String, ItemLocationSnapshot>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ConnectedCharacter {
    pub session_id: SessionId,
    pub character_id: CharacterId,
    pub account_id: AccountId,
    pub name: String,
    pub room_id: String,
}

pub const YELL_MAX_ROOM_HOPS: u8 = 1;
pub const SHOUT_MAX_ROOM_HOPS: u8 = 3;
pub const MAX_SPATIAL_VOICE_ROOMS: usize = 1 + 6 + 36 + 216;

/// Server-internal result of an authoritative telepathy ability preflight.
///
/// The destination and message are deliberately absent: they are transient
/// social-routing inputs and must never become a durable domain event.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TelepathyPlan {
    pub sender_character_id: CharacterId,
    pub sender_name: String,
    pub sender_room_id: String,
    pub ability_id: String,
    pub ability_name: String,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SceneImage {
    pub alt: String,
    #[serde(skip_serializing_if = "BTreeMap::is_empty")]
    pub localized_alt: BTreeMap<String, String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub manifest: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub avif: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub webp: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub jpeg: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub width: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub height: Option<u32>,
}

/// Optional, non-authoritative ambient audio attached to a projected scene.
/// URLs always refer to reviewed, immutable assets; transports still require
/// explicit client consent before sending or playing a cue.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SceneAudio {
    pub channel: String,
    pub url: String,
    #[serde(rename = "loop")]
    pub looped: bool,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SceneExit {
    pub id: String,
    pub label: String,
    pub command: String,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SceneAction {
    pub label: String,
    pub command: String,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum LootAccessView {
    Yours,
    Reserved,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SceneLootAccessView {
    pub access: LootAccessView,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub expires_in_ticks: Option<u64>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SceneItem {
    pub id: String,
    pub label: String,
    pub description: String,
    pub rarity: String,
    pub item_level: u16,
    pub weight_grams: u32,
    pub quantity: u16,
    pub stack_slots: u32,
    pub total_weight_grams: u64,
    pub transfer_policy: ItemTransferPolicy,
    pub bound_to_you: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub affix: Option<ItemAffixView>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub container_capacity: Option<InventoryCapacityView>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub loot: Option<SceneLootAccessView>,
    pub actions: Vec<SceneAction>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SceneNpc {
    pub id: String,
    pub label: String,
    pub description: String,
    pub health: i32,
    pub max_health: i32,
    pub actions: Vec<SceneAction>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WeatherView {
    pub condition: String,
    pub day: u64,
    pub hour: u64,
    pub next_change_in_ticks: u64,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SceneView {
    pub view_id: String,
    pub revision: u64,
    pub title: String,
    pub prose: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub image: Option<SceneImage>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub audio: Option<SceneAudio>,
    pub exits: Vec<SceneExit>,
    pub items: Vec<SceneItem>,
    pub npcs: Vec<SceneNpc>,
    pub weather: WeatherView,
}

/// Complete, bounded and perception-safe state for one connected player.
///
/// Every entity identifier in this projection is salted for the transport
/// session. Content identifiers (quest and commodity slugs) are intentionally
/// stable because clients need them to submit their corresponding actions.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PlayerStateView {
    pub revision: u64,
    pub character: PlayerCharacterView,
    pub inventory: Vec<PlayerItemView>,
    pub equipment: Vec<EquippedItemView>,
    pub inventory_capacity: InventoryCapacityView,
    pub map: DiscoveredMapView,
    pub quests: Vec<QuestView>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub party: Option<PartyView>,
    pub party_invites: Vec<PartyInviteView>,
    pub economy: EconomyView,
    pub social: SocialView,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub trade: Option<TradeView>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PlayerCharacterView {
    pub name: String,
    pub lineage: LineageView,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub background: Option<BackgroundView>,
    pub base_class: ClassView,
    pub active_class: ClassView,
    pub class_history: Vec<ClassView>,
    pub level: u32,
    pub experience: u64,
    pub next_level_experience: u64,
    pub health: i32,
    pub max_health: i32,
    pub strength: i32,
    pub defence: i32,
    pub attributes: Vec<AttributeView>,
    pub skills: Vec<SkillView>,
    pub resources: Vec<ResourceView>,
    pub abilities: Vec<AbilityView>,
    pub resistances: Vec<ResistanceView>,
    pub statuses: Vec<StatusView>,
    pub progression_points: BTreeMap<String, u16>,
    pub talents: Vec<TalentGraphView>,
    pub promotions: Vec<PromotionView>,
    pub effects: Vec<ActiveEffect>,
    pub recovery_in_ticks: Option<u64>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SkillView {
    pub id: String,
    pub label: String,
    pub rank: u16,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PlayerItemView {
    pub id: String,
    pub label: String,
    pub description: String,
    pub rarity: String,
    pub item_level: u16,
    pub weight_grams: u32,
    pub quantity: u16,
    pub available_quantity: u16,
    pub reserved_for_trade: u16,
    pub stack_slots: u32,
    pub total_weight_grams: u64,
    pub transfer_policy: ItemTransferPolicy,
    pub bound_to_you: bool,
    pub can_give: bool,
    pub can_trade: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub affix: Option<ItemAffixView>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub container_capacity: Option<InventoryCapacityView>,
    pub actions: Vec<SceneAction>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct EquippedItemView {
    pub id: String,
    pub slot: String,
    pub label: String,
    pub rarity: String,
    pub item_level: u16,
    pub weight_grams: u32,
    pub quantity: u16,
    pub stack_slots: u32,
    pub total_weight_grams: u64,
    pub transfer_policy: ItemTransferPolicy,
    pub bound_to_you: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub affix: Option<ItemAffixView>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub container_capacity: Option<InventoryCapacityView>,
    pub actions: Vec<SceneAction>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct InventoryCapacityView {
    pub used_slots: u16,
    pub max_slots: u16,
    pub used_weight_grams: u32,
    pub max_weight_grams: u32,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ItemAffixView {
    pub label: String,
    pub position: AffixPosition,
    pub attack_bonus_delta: i32,
    pub heal_amount_delta: i32,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RewardOddsView {
    pub selector: String,
    pub page: u16,
    pub total_pages: u16,
    pub guaranteed: Vec<RewardOddsGuaranteedRow>,
    pub weighted: Vec<RewardOddsWeightedRow>,
    pub affixes: Vec<RewardOddsAffixRow>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RewardOddsGuaranteedRow {
    pub label: String,
    pub quantity: u16,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub affix_pool_label: Option<String>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RewardOddsWeightedRow {
    pub pool_label: String,
    pub rolls: u8,
    pub outcome_label: String,
    pub min_quantity: u16,
    pub max_quantity: u16,
    pub probability_numerator: u64,
    pub probability_denominator: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub affix_pool_label: Option<String>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RewardOddsAffixRow {
    pub parent_outcome_label: String,
    pub affix_label: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub position: Option<AffixPosition>,
    pub attack_bonus_delta: i32,
    pub heal_amount_delta: i32,
    pub conditional_probability_numerator: u64,
    pub conditional_probability_denominator: u64,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ContainerContentsView {
    pub selector: String,
    pub items: Vec<PlayerItemView>,
    pub page: u16,
    pub total_pages: u16,
    pub capacity: InventoryCapacityView,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TradeOfferLineView {
    pub target: String,
    pub label: String,
    pub quantity: u16,
    pub container: bool,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TradeOfferView {
    pub coins: u64,
    pub lines: Vec<TradeOfferLineView>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TradeView {
    pub id: String,
    pub phase: TradePhase,
    pub counterparty: String,
    pub expires_in_ticks: u64,
    pub state_revision: u64,
    pub own_confirmed: bool,
    pub counterparty_confirmed: bool,
    pub own_offer: TradeOfferView,
    pub counterparty_offer: TradeOfferView,
    pub can_accept: bool,
    pub can_confirm: bool,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TradeContentsView {
    pub trade_id: String,
    pub container: String,
    pub items: Vec<PlayerItemView>,
    pub page: u16,
    pub total_pages: u16,
    pub capacity: InventoryCapacityView,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DiscoveredMapView {
    pub current_room_id: String,
    pub rooms: Vec<MapRoomView>,
    pub paths: Vec<MapPathView>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MapRoomView {
    pub id: String,
    pub title: String,
    pub current: bool,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MapPathView {
    pub from: String,
    pub direction: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub to: Option<String>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct QuestView {
    pub id: String,
    pub label: String,
    pub summary: String,
    pub status: String,
    pub current: u32,
    pub target: u32,
    pub actions: Vec<SceneAction>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PartyView {
    pub leader: bool,
    pub members: Vec<PartyMemberView>,
    pub actions: Vec<SceneAction>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PartyMemberView {
    pub id: String,
    pub name: String,
    pub leader: bool,
    pub online: bool,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PartyInviteView {
    pub leader_name: String,
    pub expires_in_ticks: u64,
    pub actions: Vec<SceneAction>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct EconomyView {
    pub coins: u64,
    pub reserved_coins: u64,
    pub spendable_coins: u64,
    pub commodities: Vec<CommodityView>,
    pub shop_available: bool,
    pub shop: Vec<ShopItemView>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CommodityView {
    pub id: String,
    pub label: String,
    pub quantity: u32,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ShopItemView {
    pub id: String,
    pub label: String,
    pub buy_price: u64,
    pub sell_price: u64,
    pub stock: u32,
    pub owned: u32,
    pub actions: Vec<SceneAction>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SocialView {
    pub online: Vec<OnlineCharacterView>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OnlineCharacterView {
    pub id: String,
    pub name: String,
    pub self_character: bool,
    pub nearby: bool,
}

impl SceneView {
    pub fn as_plain_text(&self) -> String {
        let mut output = format!("\r\n{}\r\n", self.title);
        for paragraph in &self.prose {
            output.push_str(paragraph);
            output.push_str("\r\n");
        }
        if !self.items.is_empty() {
            output.push_str("You see here: ");
            output.push_str(
                &self
                    .items
                    .iter()
                    .map(|item| item.label.as_str())
                    .collect::<Vec<_>>()
                    .join(", "),
            );
            output.push_str(".\r\n");
        }
        for npc in &self.npcs {
            output.push_str(&format!(
                "{} is here ({}/{} health).\r\n",
                npc.label, npc.health, npc.max_health
            ));
        }
        output.push_str(&format!(
            "Weather: {}; day {}, hour {}.\r\n",
            self.weather.condition, self.weather.day, self.weather.hour
        ));
        if self.exits.is_empty() {
            output.push_str("Exits: none\r\n");
        } else {
            let exits = self
                .exits
                .iter()
                .map(|exit| exit.label.as_str())
                .collect::<Vec<_>>()
                .join(", ");
            output.push_str(&format!("Exits: {exits}\r\n"));
        }
        output
    }
}
