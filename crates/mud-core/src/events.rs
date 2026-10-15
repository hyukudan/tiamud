//! World errors, authoritative domain events, gameplay events and view
//! events.

use super::*;

#[derive(Debug, Error)]
pub enum WorldError {
    #[error("No character is attached to this session.")]
    UnknownSession,
    #[error("That character is already connected.")]
    CharacterAlreadyConnected,
    #[error("There is no exit in that direction.")]
    NoExit,
    #[error("The destination area must be loaded before movement can continue.")]
    AreaLoadRequired(AreaLoadRequest),
    #[error("The prepared area lifecycle transition is invalid.")]
    InvalidAreaLifecycle,
    #[error("The world contains an invalid entity reference.")]
    InvalidEntity,
    #[error("No accessible item matches {0:?}.")]
    ItemNotFound(String),
    #[error("{query:?} is ambiguous; it could mean: {matches}.")]
    AmbiguousItem { query: String, matches: String },
    #[error("A persisted domain event references invalid world data.")]
    InvalidDomainEvent,
    #[error("A persisted domain event is inconsistent with authoritative world state.")]
    InvalidDomainTransition,
    #[error("That gameplay action is not available.")]
    GameplayUnavailable,
    #[error("The gameplay definition is invalid.")]
    InvalidGameplayDefinition,
    #[error("No visible NPC matches that target.")]
    NpcNotFound,
    #[error("That NPC name is ambiguous.")]
    AmbiguousNpc,
    #[error("That item cannot be equipped.")]
    ItemNotEquippable,
    #[error("That item cannot be used.")]
    ItemNotUsable,
    #[error("That item would exceed inventory slot or weight capacity.")]
    InventoryCapacityExceeded,
    #[error("That item cannot change ownership.")]
    ItemTransferForbidden,
    #[error("That recipient is not available for an item transfer.")]
    GiveTargetUnavailable,
    #[error("That trade action is not available.")]
    TradeUnavailable,
    #[error("That trade participant is not available.")]
    TradeTargetUnavailable,
    #[error("That item is reserved for a trade.")]
    ItemReservedForTrade,
    #[error("That loot action is not available.")]
    LootUnavailable,
    #[error("That sealed reward is not available.")]
    RewardUnavailable,
    #[error("That trade confirmation refers to an obsolete revision.")]
    StaleTradeRevision,
    #[error("Nothing is equipped in that slot.")]
    EquipmentSlotEmpty,
    #[error("Combat cannot be started right now.")]
    CombatUnavailable,
    #[error("No lineage matches that identifier.")]
    LineageNotFound,
    #[error("No background matches that identifier.")]
    BackgroundNotFound,
    #[error("No class matches that identifier.")]
    ClassNotFound,
    #[error("That class promotion is not available.")]
    PromotionUnavailable,
    #[error("That talent rank is not available.")]
    TalentUnavailable,
    #[error("There are not enough progression points.")]
    InsufficientProgressionPoints,
    #[error("A progression grant would exceed an engine cap.")]
    ProgressionCapExceeded,
    #[error("The authored progression configuration is invalid.")]
    InvalidProgressionDefinition,
    #[error("The progression transition is invalid.")]
    InvalidProgressionTransition,
    #[error("The authored ability configuration is invalid.")]
    InvalidAbilityDefinition,
    #[error("That ability is unknown or has not been learned.")]
    AbilityNotKnown,
    #[error("That ability cannot be used right now.")]
    AbilityUnavailable,
    #[error("There are not enough resources to use that ability.")]
    InsufficientResource,
    #[error("That ability target is not valid.")]
    InvalidAbilityTarget,
    #[error("That telepathic message is not valid.")]
    InvalidTelepathyRequest,
    #[error("The authored social-emote configuration is invalid.")]
    InvalidEmoteDefinition,
    #[error("That social emote is not available.")]
    EmoteUnavailable,
    #[error("That social-emote target is not available.")]
    EmoteTargetUnavailable,
    #[error("The persisted world runtime snapshot is invalid or incompatible.")]
    InvalidRuntimeSnapshot,
    #[error("That party action is not available.")]
    PartyUnavailable,
    #[error("Only the party leader may do that.")]
    PartyForbidden,
    #[error("No connected character matches that party target.")]
    PartyTargetNotFound,
    #[error("That quest action is not available.")]
    QuestUnavailable,
    #[error("That shop action is not available here.")]
    ShopUnavailable,
    #[error("The shop does not carry that commodity.")]
    CommodityNotFound,
    #[error("You do not have enough coin.")]
    InsufficientFunds,
    #[error("You do not own that commodity.")]
    CommodityNotOwned,
    #[error("A script requested an invalid or excessive authorized flag.")]
    InvalidScriptFlag,
    #[error("A script requested an invalid or excessive authorized effect.")]
    InvalidScriptEffect,
}

/// Authoritative facts emitted after a successful world-state transition.
/// These IDs belong to the domain and must never be sent directly to clients;
/// use [`World::project_event`] to obtain an observer-specific [`ViewEvent`].
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum DomainEvent {
    CharacterMoved {
        actor_id: CharacterId,
        actor_name: String,
        from_room_id: String,
        to_room_id: String,
        direction: Direction,
    },
    CharacterRelocated {
        actor_id: CharacterId,
        actor_name: String,
        from_room_id: String,
        to_room_id: String,
    },
    ItemTaken {
        actor_id: CharacterId,
        actor_name: String,
        item_id: ItemInstanceId,
        item_name: String,
        room_id: String,
    },
    ItemDropped {
        actor_id: CharacterId,
        actor_name: String,
        item_id: ItemInstanceId,
        item_name: String,
        room_id: String,
    },
    ItemsMoved {
        actor_id: CharacterId,
        actor_name: String,
        item_ids: Vec<ItemInstanceId>,
        item_name: String,
        container_id: Option<ItemInstanceId>,
        operation: ContainerMoveOperation,
        room_id: String,
    },
    ItemsGiven {
        giver_id: CharacterId,
        giver_name: String,
        recipient_id: CharacterId,
        recipient_name: String,
        item_ids: Vec<ItemInstanceId>,
        item_name: String,
        room_id: String,
    },
    TradeRequested {
        trade: TradeSnapshot,
    },
    TradeOpened {
        previous: TradeSnapshot,
        current: TradeSnapshot,
    },
    TradeOfferSet {
        previous: TradeSnapshot,
        current: TradeSnapshot,
    },
    TradeConfirmed {
        participant: CharacterId,
        confirmation: TradeConfirmation,
    },
    TradeInvalidated {
        previous: TradeSnapshot,
        current: TradeSnapshot,
    },
    TradeCancelled {
        previous_trade: TradeSnapshot,
        reason: TradeCancellationReason,
    },
    TradeCompleted {
        previous_trade: TradeSnapshot,
    },
    Gameplay {
        event: GameplayEvent,
    },
    Adventure {
        event: AdventureEvent,
    },
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum ContainerMoveOperation {
    Take,
    Drop,
    Put,
    Get,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "eventKind", rename_all = "camelCase")]
pub enum AdventureEvent {
    PartyChanged {
        character_id: CharacterId,
        party_id: u64,
        #[serde(default)]
        audience: Vec<CharacterId>,
        summary: String,
    },
    QuestChanged {
        character_id: CharacterId,
        quest_id: String,
        summary: String,
    },
    EconomyChanged {
        character_id: CharacterId,
        commodity_id: Option<String>,
        summary: String,
    },
    WeatherChanged {
        weather: WeatherKind,
        tick: u64,
    },
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "eventKind", rename_all = "camelCase")]
pub enum GameplayEvent {
    TickAdvanced {
        tick: u64,
    },
    NpcSpawned {
        npc_id: String,
        npc_name: String,
        room_id: String,
    },
    CombatStarted {
        character_id: CharacterId,
        character_name: String,
        npc_id: String,
        npc_name: String,
        room_id: String,
    },
    AbilityUsed {
        character_id: CharacterId,
        character_name: String,
        ability_id: String,
        ability_name: String,
        npc_id: Option<String>,
        npc_name: Option<String>,
        room_id: String,
    },
    NpcAbilityUsed {
        npc_id: String,
        npc_name: String,
        character_id: CharacterId,
        character_name: String,
        ability_id: String,
        ability_name: String,
        room_id: String,
    },
    NpcDamaged {
        character_id: CharacterId,
        character_name: String,
        npc_id: String,
        npc_name: String,
        amount: i32,
        remaining_health: i32,
        room_id: String,
    },
    CharacterDamaged {
        character_id: CharacterId,
        character_name: String,
        npc_id: String,
        npc_name: String,
        amount: i32,
        remaining_health: i32,
        room_id: String,
    },
    NpcDefeated {
        character_id: CharacterId,
        character_name: String,
        npc_id: String,
        npc_name: String,
        respawn_at_tick: u64,
        room_id: String,
    },
    LootDropped {
        npc_id: String,
        npc_name: String,
        item_id: ItemInstanceId,
        item_name: String,
        room_id: String,
    },
    LootGenerated {
        drop_id: LootDropId,
        npc_id: String,
        npc_name: String,
        item_ids: Vec<ItemInstanceId>,
        room_id: String,
    },
    LootRightsChanged {
        item_id: ItemInstanceId,
        room_id: String,
        summary: String,
    },
    LootGenerationSuppressed {
        drop_id: LootDropId,
        npc_id: String,
        npc_name: String,
        reason: String,
        room_id: String,
    },
    LootDespawned {
        item_ids: Vec<ItemInstanceId>,
        room_id: String,
    },
    LootEncounterCancelled {
        npc_id: String,
        npc_name: String,
        room_id: String,
    },
    ExperienceGained {
        character_id: CharacterId,
        character_name: String,
        amount: u64,
        total: u64,
        room_id: String,
    },
    CharacterLeveled {
        character_id: CharacterId,
        character_name: String,
        level: u32,
        room_id: String,
    },
    CharacterPromoted {
        character_id: CharacterId,
        character_name: String,
        from_class_id: String,
        to_class_id: String,
        room_id: String,
    },
    TalentRankAcquired {
        character_id: CharacterId,
        character_name: String,
        graph_id: String,
        node_id: String,
        rank: u8,
        room_id: String,
    },
    CharacterDefeated {
        character_id: CharacterId,
        character_name: String,
        npc_id: String,
        npc_name: String,
        recover_at_tick: u64,
        room_id: String,
    },
    CharacterCollapsed {
        character_id: CharacterId,
        character_name: String,
        recover_at_tick: u64,
        room_id: String,
    },
    CharacterRecovered {
        character_id: CharacterId,
        character_name: String,
        room_id: String,
    },
    NpcRespawned {
        npc_id: String,
        npc_name: String,
        room_id: String,
    },
    ItemEquipped {
        character_id: CharacterId,
        character_name: String,
        item_id: ItemInstanceId,
        item_name: String,
        slot: EquipmentSlot,
        room_id: String,
    },
    ItemUnequipped {
        character_id: CharacterId,
        character_name: String,
        item_id: ItemInstanceId,
        item_name: String,
        slot: EquipmentSlot,
        room_id: String,
    },
    ItemUsed {
        character_id: CharacterId,
        character_name: String,
        item_id: ItemInstanceId,
        item_name: String,
        room_id: String,
    },
    RewardOpened {
        open_id: RewardOpenId,
        character_id: CharacterId,
        character_name: String,
        source_item_id: ItemInstanceId,
        source_item_name: String,
        item_ids: Vec<ItemInstanceId>,
        item_names: Vec<String>,
        room_id: String,
    },
    CharacterHealed {
        character_id: CharacterId,
        character_name: String,
        amount: i32,
        health: i32,
        room_id: String,
    },
    CharacterRestored {
        character_id: CharacterId,
        character_name: String,
        health: i32,
        room_id: String,
    },
    EffectApplied {
        character_id: CharacterId,
        character_name: String,
        effect: ActiveEffect,
        room_id: String,
    },
    EffectExpired {
        character_id: CharacterId,
        character_name: String,
        kind: EffectKind,
        room_id: String,
    },
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum ViewEventKind {
    CharacterMoved,
    ItemTaken,
    ItemDropped,
    ItemGiven,
    TradeRequested,
    TradeChanged,
    TradeCompleted,
    TradeCancelled,
    Gameplay,
    Adventure,
}

/// Perception-safe projection of one domain event for one observer.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ViewEvent {
    pub revision: u64,
    pub kind: ViewEventKind,
    pub text: String,
    pub actor_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub target_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub item_id: Option<String>,
}

#[derive(Clone, Debug)]
pub struct ActionOutcome {
    pub scene: SceneView,
    /// Present only for an explicit, paginated container inspection.
    pub container_contents: Option<ContainerContentsView>,
    /// Present only for an explicit inspection of an offered container.
    pub trade_contents: Option<TradeContentsView>,
    /// Present only for an explicit, bounded sealed-reward odds request.
    pub reward_odds: Option<RewardOddsView>,
    /// Optional transport-independent, plain-text response to the command.
    pub output: Option<String>,
    /// Present when the action changed persistent character state.
    pub changed_character: Option<CharacterSnapshot>,
    /// Authoritative events to persist and then project to connected views.
    pub domain_events: Vec<DomainEvent>,
}
