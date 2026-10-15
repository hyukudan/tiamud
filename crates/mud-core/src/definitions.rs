//! Authored world definitions: items, affixes, rooms, NPCs, exits,
//! character gameplay state and build errors.

use super::*;

#[derive(Clone, Copy, Debug, Deserialize, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum EquipmentSlot {
    Weapon,
    Body,
    Trinket,
}

impl EquipmentSlot {
    pub(crate) fn parse(value: &str) -> Option<Self> {
        match value {
            "weapon" => Some(Self::Weapon),
            "body" | "armour" | "armor" => Some(Self::Body),
            "trinket" => Some(Self::Trinket),
            _ => None,
        }
    }

    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::Weapon => "weapon",
            Self::Body => "body",
            Self::Trinket => "trinket",
        }
    }
}

/// Durable server-side identity of one concrete item.
///
/// The wrapped value is intentionally never projected directly to a client.
/// Transport views derive a session-salted opaque identifier from it.
#[derive(Clone, Debug, Deserialize, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(transparent)]
pub struct ItemInstanceId(pub String);

impl ItemInstanceId {
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl From<String> for ItemInstanceId {
    fn from(value: String) -> Self {
        Self(value)
    }
}

impl From<&str> for ItemInstanceId {
    fn from(value: &str) -> Self {
        Self(value.to_owned())
    }
}

impl Borrow<str> for ItemInstanceId {
    fn borrow(&self) -> &str {
        self.as_str()
    }
}

pub fn valid_item_instance_id(value: &str) -> bool {
    let bytes = value.as_bytes();
    if bytes.is_empty() || bytes.len() > MAX_ITEM_INSTANCE_ID_BYTES {
        return false;
    }
    if value.starts_with("runtime:") {
        return runtime_item_sequence(&ItemInstanceId(value.to_owned())).is_some();
    }
    bytes[0].is_ascii_alphanumeric()
        && bytes[bytes.len() - 1].is_ascii_alphanumeric()
        && bytes.iter().all(|byte| {
            byte.is_ascii_lowercase()
                || byte.is_ascii_digit()
                || matches!(byte, b':' | b'-' | b'_' | b'.')
        })
}

pub(crate) fn runtime_item_sequence(item_id: &ItemInstanceId) -> Option<u64> {
    let value = item_id.as_str().strip_prefix("runtime:")?;
    if value.is_empty()
        || !value.bytes().all(|byte| byte.is_ascii_digit())
        || (value.len() > 1 && value.starts_with('0'))
    {
        return None;
    }
    value.parse::<u64>().ok().filter(|sequence| *sequence > 0)
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ItemRarity {
    Common,
    Uncommon,
    Rare,
    Epic,
    Legendary,
}

impl ItemRarity {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Common => "common",
            Self::Uncommon => "uncommon",
            Self::Rare => "rare",
            Self::Epic => "epic",
            Self::Legendary => "legendary",
        }
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, Hash, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ItemStackPolicy {
    Unique,
    Fungible,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ItemContainerDefinition {
    pub max_slots: u16,
    pub max_weight_grams: u32,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, Hash, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ItemTransferPolicy {
    Transferable,
    CharacterBound,
    NonTransferable,
}

impl ItemTransferPolicy {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Transferable => "transferable",
            Self::CharacterBound => "character_bound",
            Self::NonTransferable => "non_transferable",
        }
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(
    tag = "kind",
    rename_all = "camelCase",
    rename_all_fields = "camelCase",
    deny_unknown_fields
)]
pub enum ItemBinding {
    Unbound,
    Character { character_id: CharacterId },
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct InventoryCapacity {
    pub max_slots: u16,
    pub max_weight_grams: u32,
}

impl Default for InventoryCapacity {
    fn default() -> Self {
        Self {
            max_slots: 24,
            max_weight_grams: 50_000,
        }
    }
}

/// Transport- and authoring-format-independent reusable item content.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ItemPrototypeDefinition {
    pub prototype_id: String,
    pub name: String,
    pub description: String,
    pub aliases: Vec<String>,
    pub equipment_slot: Option<EquipmentSlot>,
    pub attack_bonus: i32,
    pub heal_amount: i32,
    pub consumable: bool,
    pub effect: Option<ActiveEffect>,
    pub weight_grams: u32,
    pub slot_cost: u16,
    pub rarity: ItemRarity,
    pub item_level: u16,
    pub stack_policy: ItemStackPolicy,
    pub max_stack_units: u16,
    pub container: Option<ItemContainerDefinition>,
    pub transfer_policy: ItemTransferPolicy,
    pub sealed_loot_table_id: Option<String>,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum AffixPosition {
    Prefix,
    Suffix,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AffixDefinition {
    pub id: String,
    pub label: String,
    pub position: AffixPosition,
    pub attack_bonus_delta: i32,
    pub heal_amount_delta: i32,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum AffixPoolEntryKind {
    Affix,
    Nothing,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AffixPoolEntry {
    pub kind: AffixPoolEntryKind,
    pub id: String,
    pub weight: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub affix_id: Option<String>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AffixPoolDefinition {
    pub id: String,
    pub entries: Vec<AffixPoolEntry>,
}

/// Immutable, self-contained result of one reviewed affix roll.
#[derive(Clone, Debug, Deserialize, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AppliedItemAffix {
    pub affix_id: String,
    pub definition_digest_sha256: String,
    pub pool_id: String,
    pub pool_digest_sha256: String,
    pub record_digest_sha256: String,
    pub label: String,
    pub position: AffixPosition,
    pub attack_bonus_delta: i32,
    pub heal_amount_delta: i32,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum InitialItemLocation {
    Room(String),
    NpcSpawn(String),
    Container(ItemInstanceId),
    Staged,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct InitialItemInstanceDefinition {
    pub instance_id: ItemInstanceId,
    pub prototype_id: String,
    pub location: InitialItemLocation,
}

/// Transport- and authoring-format-independent input used to construct a world.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct WorldDefinition {
    pub start_room_id: String,
    pub item_prototypes: Vec<ItemPrototypeDefinition>,
    pub initial_item_instances: Vec<InitialItemInstanceDefinition>,
    pub rooms: Vec<RoomDefinition>,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum EffectKind {
    Regeneration,
    Strength,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ActiveEffect {
    pub kind: EffectKind,
    pub magnitude: i32,
    pub remaining_ticks: u32,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Vitals {
    pub health: i32,
    pub max_health: i32,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Attributes {
    pub strength: i32,
    pub defence: i32,
    #[serde(default)]
    pub additional: BTreeMap<String, i32>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CharacterGameplaySnapshot {
    pub lineage_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub background_id: Option<String>,
    pub base_class_id: String,
    pub active_class_id: String,
    pub class_history: Vec<String>,
    pub level: u32,
    pub experience: u64,
    pub vitals: Vitals,
    pub attributes: Attributes,
    #[serde(default)]
    pub resources: BTreeMap<String, ResourcePool>,
    #[serde(default)]
    pub resource_regeneration: BTreeMap<String, u32>,
    #[serde(default)]
    pub ability_ids: Vec<String>,
    /// Absolute logical tick at which an ability becomes available again.
    #[serde(default)]
    pub ability_cooldowns: BTreeMap<String, u64>,
    /// One bounded action slot per world tick prevents command spam from
    /// bypassing deterministic combat turns.
    #[serde(default)]
    pub ability_global_ready_tick: u64,
    /// Used by the combat loop to replace, rather than stack with, the normal
    /// player attack on the following resolved turn.
    #[serde(default)]
    pub last_offensive_ability_tick: Option<u64>,
    #[serde(default)]
    pub resistances: BTreeMap<DamageType, u16>,
    #[serde(default)]
    pub statuses: Vec<ActiveStatusEffect>,
    #[serde(default = "default_skills")]
    pub skills: BTreeMap<String, u16>,
    #[serde(default)]
    pub progression_points: BTreeMap<String, u16>,
    #[serde(default)]
    pub talent_node_ranks: BTreeMap<String, BTreeMap<String, u8>>,
    #[serde(default)]
    pub tags: BTreeSet<String>,
    #[serde(default)]
    pub received_signals: BTreeSet<String>,
    pub equipment: BTreeMap<EquipmentSlot, ItemInstanceId>,
    #[serde(default)]
    pub inventory_capacity: InventoryCapacity,
    pub effects: Vec<ActiveEffect>,
    pub recovery_at_tick: Option<u64>,
    #[serde(default = "default_coin_balance")]
    pub coins: u64,
    #[serde(default)]
    pub commodities: BTreeMap<String, u32>,
    #[serde(default)]
    pub quests: BTreeMap<String, QuestProgress>,
    /// Server-authorized script state. Flags are never projected directly to
    /// clients; they may only influence declarative content conditions.
    #[serde(default)]
    pub script_flags: BTreeSet<String>,
    #[serde(default)]
    pub scheduled_script_flags: Vec<ScheduledScriptFlag>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ScheduledScriptFlag {
    pub flag: String,
    pub due_tick: u64,
}

pub(crate) fn default_coin_balance() -> u64 {
    40
}

pub(crate) fn default_skills() -> BTreeMap<String, u16> {
    BTreeMap::from([
        ("combat".to_owned(), 10),
        ("perception".to_owned(), 10),
        ("survival".to_owned(), 10),
    ])
}

pub(crate) fn builtin_progression_definition() -> CharacterProgressionDefinition {
    CharacterProgressionDefinition {
        defaults: CharacterDefaultsDefinition {
            lineage_id: "core:human".into(),
            background_id: None,
            base_class_id: "core:wanderer".into(),
        },
        level_thresholds: (2..=MAX_PROGRESSION_LEVEL)
            .map(|level| LevelThresholdDefinition {
                level,
                total_experience_required: u64::from(level - 1) * 100,
                point_awards: BTreeMap::new(),
            })
            .collect(),
        lineages: vec![LineageDefinition {
            id: "core:human".into(),
            name: "Human".into(),
            description: "A versatile mortal lineage.".into(),
            ui_order: 0,
            creation_grants: Vec::new(),
            talent_graph_ids: Vec::new(),
        }],
        backgrounds: Vec::new(),
        base_classes: vec![BaseClassDefinition {
            id: "core:wanderer".into(),
            name: "Wanderer".into(),
            description: "A flexible adventurer with a balanced foundation.".into(),
            ui_order: 0,
            creation_grants: vec![
                GrantDefinition::Attribute {
                    attribute_id: "max_health".into(),
                    amount: 20,
                },
                GrantDefinition::Attribute {
                    attribute_id: "strength".into(),
                    amount: 5,
                },
                GrantDefinition::Attribute {
                    attribute_id: "defence".into(),
                    amount: 2,
                },
                GrantDefinition::Skill {
                    skill_id: "combat".into(),
                    amount: 10,
                },
                GrantDefinition::Skill {
                    skill_id: "exploration".into(),
                    amount: 5,
                },
            ],
            level_grants: (2..=MAX_PROGRESSION_LEVEL)
                .map(|level| LevelGrantDefinition {
                    level,
                    grants: vec![
                        GrantDefinition::Attribute {
                            attribute_id: "max_health".into(),
                            amount: 5,
                        },
                        GrantDefinition::Attribute {
                            attribute_id: "strength".into(),
                            amount: 2,
                        },
                        GrantDefinition::Attribute {
                            attribute_id: "defence".into(),
                            amount: 1,
                        },
                    ],
                })
                .collect(),
            talent_graph_ids: Vec::new(),
        }],
        advanced_classes: Vec::new(),
        talent_graphs: Vec::new(),
    }
}

pub(crate) fn valid_script_flag(flag: &str) -> bool {
    valid_definition_id(flag)
}

pub(crate) fn perception_senses(gameplay: &CharacterGameplaySnapshot) -> BTreeSet<Sense> {
    let mut senses = BTreeSet::from([Sense::Vision, Sense::Hearing]);
    if gameplay.script_flags.contains("condition.blind") {
        senses.remove(&Sense::Vision);
    }
    if gameplay.script_flags.contains("condition.deaf") {
        senses.remove(&Sense::Hearing);
    }
    if gameplay.script_flags.contains("sense.darkvision")
        && !gameplay.script_flags.contains("condition.blind")
    {
        senses.insert(Sense::Darkvision);
    }
    if gameplay.script_flags.contains("sense.tremorsense") {
        senses.insert(Sense::Tremorsense);
    }
    senses
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum QuestStatus {
    Active,
    ReadyToClaim,
    Completed,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct QuestProgress {
    pub status: QuestStatus,
    pub current: u32,
    pub target: u32,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum WeatherKind {
    Clear,
    Rain,
    Wind,
    Fog,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PartySnapshot {
    pub id: u64,
    pub leader: CharacterId,
    pub members: Vec<CharacterId>,
    pub invitations: BTreeMap<CharacterId, u64>,
}

/// Whitelisted runtime facts available to the server-side content engine.
/// Stable IDs remain server-internal and this type is deliberately not serializable.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AuthorizedRuntimeContext {
    pub character_id: CharacterId,
    pub character_name: String,
    pub room_id: String,
    pub actor_level: u32,
    pub actor_flags: BTreeSet<String>,
    pub world_instance: String,
    pub actor_quest_states: BTreeSet<String>,
    pub actor_content_preferences: BTreeSet<String>,
    pub actor_senses: BTreeSet<String>,
    pub logical_tick: u64,
    pub weather: WeatherKind,
}

/// Server-internal join key for presentation annotations. This type is
/// intentionally not serializable: stable IDs must never cross a transport
/// boundary, while `opaque_id` is safe only for this observer session.
#[derive(Clone, Debug, Eq, PartialEq)]
#[doc(hidden)]
pub struct VisibleSceneEntityId {
    /// Durable instance ID used only to derive the observer-salted identity.
    pub stable_id: String,
    /// Authored prototype join key for reviewed presentation metadata.
    pub prototype_id: String,
    pub opaque_id: String,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
#[doc(hidden)]
pub struct VisibleSceneEntityIds {
    pub items: Vec<VisibleSceneEntityId>,
    pub npcs: Vec<VisibleSceneEntityId>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ItemGameplayDefinition {
    pub item_id: String,
    pub equipment_slot: Option<EquipmentSlot>,
    pub attack_bonus: i32,
    pub heal_amount: i32,
    pub consumable: bool,
    pub effect: Option<ActiveEffect>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct NpcDefinition {
    pub stable_id: String,
    pub name: String,
    pub description: String,
    pub room_id: String,
    pub level: u32,
    pub max_health: i32,
    pub strength: i32,
    pub defence: i32,
    pub experience_reward: u64,
    pub loot_item_ids: Vec<ItemInstanceId>,
    pub respawn_ticks: u64,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RoomDefinition {
    pub stable_id: String,
    pub title: String,
    pub prose: Vec<String>,
    pub image: Option<SceneImage>,
    pub audio: Option<SceneAudio>,
    pub exits: Vec<ExitDefinition>,
    /// Transitional compatibility field. Canonical v4 placement lives in
    /// `WorldDefinition::initial_item_instances`.
    pub item_ids: Vec<String>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ExitDefinition {
    pub direction: Direction,
    pub target_room_id: String,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum LightLevel {
    Bright,
    Dim,
    Dark,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Sense {
    Vision,
    Hearing,
    Darkvision,
    Tremorsense,
}

#[derive(Debug, Error, Eq, PartialEq)]
pub enum WorldBuildError {
    #[error("A world must contain at least one room.")]
    EmptyWorld,
    #[error("A room has an empty stable ID.")]
    EmptyRoomId,
    #[error("Room ID {0:?} appears more than once.")]
    DuplicateRoomId(String),
    #[error("Start room {0:?} does not exist.")]
    UnknownStartRoom(String),
    #[error("Room {room_id:?} points to missing room {target_room_id:?}.")]
    UnknownExitTarget {
        room_id: String,
        target_room_id: String,
    },
    #[error("Room {room_id:?} defines direction {direction:?} more than once.")]
    DuplicateExitDirection {
        room_id: String,
        direction: Direction,
    },
    #[error("An item has an empty stable ID.")]
    EmptyItemId,
    #[error("Item ID {0:?} appears more than once.")]
    DuplicateItemId(String),
    #[error("A world declares more than {MAX_ITEM_INSTANCES} item instances.")]
    TooManyItemInstances,
    #[error("Item prototype {0:?} is invalid.")]
    InvalidItemPrototype(String),
    #[error("Item instance points to missing prototype {0:?}.")]
    UnknownItemPrototype(String),
    #[error("Item {0:?} must have a name.")]
    EmptyItemName(String),
    #[error("Room {room_id:?} points to missing item {item_id:?}.")]
    UnknownRoomItem { room_id: String, item_id: String },
    #[error("Room {room_id:?} lists item {item_id:?} more than once.")]
    DuplicateRoomItem { room_id: String, item_id: String },
    #[error("Item {item_id:?} is placed in both {first_room_id:?} and {second_room_id:?}.")]
    ItemPlacedMoreThanOnce {
        item_id: String,
        first_room_id: String,
        second_room_id: String,
    },
    #[error("Item {0:?} has no initial room.")]
    UnplacedItem(String),
    #[error("Staged item {0:?} is not declared by the world.")]
    UnknownStagedItem(String),
    #[error("Staged item {0:?} appears more than once.")]
    DuplicateStagedItem(String),
    #[error("Item {item_id:?} is both placed in room {room_id:?} and staged for an initial owner.")]
    ItemPlacedAndStaged { item_id: String, room_id: String },
}
