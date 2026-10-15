//! Private in-memory world model: compiled prototypes, instances, rooms,
//! NPCs, the deterministic RNG and entity helpers.

use super::*;

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct ItemPrototype {
    pub(crate) name: String,
    pub(crate) description: String,
    pub(crate) aliases: Vec<String>,
    pub(crate) equipment_slot: Option<EquipmentSlot>,
    pub(crate) attack_bonus: i32,
    pub(crate) heal_amount: i32,
    pub(crate) consumable: bool,
    pub(crate) effect: Option<ActiveEffect>,
    pub(crate) weight_grams: u32,
    pub(crate) slot_cost: u16,
    pub(crate) rarity: ItemRarity,
    pub(crate) item_level: u16,
    pub(crate) stack_policy: ItemStackPolicy,
    pub(crate) max_stack_units: u16,
    pub(crate) container: Option<ItemContainerDefinition>,
    pub(crate) transfer_policy: ItemTransferPolicy,
    pub(crate) sealed_loot_table_id: Option<String>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct ItemInstance {
    pub(crate) prototype_id: String,
    pub(crate) location: SharedItemLocation,
    pub(crate) binding: ItemBinding,
    pub(crate) item_affixes: Vec<AppliedItemAffix>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct AffixPool {
    pub(crate) id: String,
    pub(crate) digest_sha256: String,
    pub(crate) entries: Vec<AffixPoolEntry>,
    pub(crate) total_weight: u64,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct LootTable {
    pub(crate) id: String,
    pub(crate) digest_sha256: String,
    pub(crate) guaranteed: Vec<LootGuaranteedDefinition>,
    pub(crate) pools: Vec<LootPoolDefinition>,
    pub(crate) worst_case_units: usize,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct LootSource {
    pub(crate) loot_table_id: String,
    pub(crate) rights: LootRightsPolicy,
}

#[derive(Clone, Debug)]
pub(crate) struct RolledLootUnit {
    pub(crate) prototype_id: String,
    pub(crate) entry_id: String,
    pub(crate) pool_id: Option<String>,
    pub(crate) roll_index: u8,
    pub(crate) quantity_index: u16,
    pub(crate) affix_pool_id: Option<String>,
}

#[derive(Clone, Debug)]
pub(crate) struct Room {
    pub(crate) stable_id: String,
    pub(crate) title: String,
    pub(crate) prose: Vec<String>,
    pub(crate) image: Option<SceneImage>,
    pub(crate) audio: Option<SceneAudio>,
    pub(crate) exits: BTreeMap<Direction, RoomExit>,
}

#[derive(Clone, Debug)]
pub(crate) enum RoomExit {
    Resident(EntityId),
    Deferred(AreaLoadRequest),
}

impl RoomExit {
    pub(crate) fn resident_target(&self) -> Option<EntityId> {
        match self {
            Self::Resident(target) => Some(*target),
            Self::Deferred(_) => None,
        }
    }
}

#[derive(Clone, Debug)]
pub(crate) struct Character {
    pub(crate) id: CharacterId,
    pub(crate) account_id: AccountId,
    pub(crate) name: String,
    pub(crate) location: EntityId,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum SharedItemLocation {
    Staged,
    Carried(CharacterId),
    Room(String),
    Equipped(CharacterId, EquipmentSlot),
    HeldByNpc(String),
    Contained(ItemInstanceId),
    Consumed,
}

#[derive(Clone, Debug)]
pub(crate) struct Npc {
    pub(crate) stable_id: String,
    pub(crate) name: String,
    pub(crate) description: String,
    pub(crate) spawn_room_id: String,
    pub(crate) level: u32,
    pub(crate) health: i32,
    pub(crate) max_health: i32,
    pub(crate) strength: i32,
    pub(crate) defence: i32,
    pub(crate) experience_reward: u64,
    pub(crate) loot_item_ids: Vec<ItemInstanceId>,
    pub(crate) respawn_ticks: u64,
    pub(crate) respawn_at_tick: Option<u64>,
    pub(crate) engaged_character: Option<CharacterId>,
    pub(crate) resources: BTreeMap<String, ResourcePool>,
    pub(crate) resource_regeneration: BTreeMap<String, u32>,
    pub(crate) ability_ids: Vec<String>,
    pub(crate) ability_cooldowns: BTreeMap<String, u64>,
    pub(crate) resistances: BTreeMap<DamageType, u16>,
    pub(crate) statuses: Vec<ActiveStatusEffect>,
}

impl Npc {
    pub(crate) fn alive(&self) -> bool {
        self.health > 0 && self.respawn_at_tick.is_none()
    }
}

pub(crate) fn runtime_item_instance_snapshot(
    instance: &ItemInstance,
) -> RuntimeItemInstanceSnapshot {
    let location = match &instance.location {
        SharedItemLocation::Staged => RuntimeItemLocation::Staged,
        SharedItemLocation::Carried(character_id) => RuntimeItemLocation::Carried {
            character_id: *character_id,
        },
        SharedItemLocation::Room(room_id) => RuntimeItemLocation::Room {
            room_id: room_id.clone(),
        },
        SharedItemLocation::Equipped(character_id, slot) => RuntimeItemLocation::Equipped {
            character_id: *character_id,
            slot: *slot,
        },
        SharedItemLocation::HeldByNpc(npc_id) => RuntimeItemLocation::HeldByNpc {
            npc_id: npc_id.clone(),
        },
        SharedItemLocation::Contained(container_id) => RuntimeItemLocation::Contained {
            container_id: container_id.clone(),
        },
        SharedItemLocation::Consumed => RuntimeItemLocation::Consumed,
    };
    RuntimeItemInstanceSnapshot {
        prototype_id: instance.prototype_id.clone(),
        location,
        binding: instance.binding,
        item_affixes: instance.item_affixes.clone(),
    }
}

pub(crate) fn npc_runtime_snapshot(npc: &Npc) -> NpcRuntimeSnapshot {
    NpcRuntimeSnapshot {
        stable_id: npc.stable_id.clone(),
        name: npc.name.clone(),
        description: npc.description.clone(),
        spawn_room_id: npc.spawn_room_id.clone(),
        level: npc.level,
        health: npc.health,
        max_health: npc.max_health,
        strength: npc.strength,
        defence: npc.defence,
        experience_reward: npc.experience_reward,
        loot_item_ids: npc.loot_item_ids.clone(),
        respawn_ticks: npc.respawn_ticks,
        respawn_at_tick: npc.respawn_at_tick,
        engaged_character: npc.engaged_character,
        resources: npc.resources.clone(),
        resource_regeneration: npc.resource_regeneration.clone(),
        ability_ids: npc.ability_ids.clone(),
        ability_cooldowns: npc.ability_cooldowns.clone(),
        resistances: npc.resistances.clone(),
        statuses: npc.statuses.clone(),
    }
}

#[derive(Clone, Debug)]
pub(crate) struct DeterministicRng {
    pub(crate) state: u64,
}

impl DeterministicRng {
    pub(crate) fn new(seed: u64) -> Self {
        Self { state: seed.max(1) }
    }

    pub(crate) fn next_u64(&mut self) -> u64 {
        let mut value = self.state;
        value ^= value << 13;
        value ^= value >> 7;
        value ^= value << 17;
        self.state = value;
        value
    }

    pub(crate) fn range_inclusive(&mut self, minimum: i32, maximum: i32) -> i32 {
        // Compute the span in wider arithmetic so a misuse (minimum > maximum)
        // fails closed to `minimum` instead of relying on a release-stripped
        // debug assertion or underflowing `i32`. For valid inputs the span
        // equals `maximum - minimum + 1`, so the distribution is unchanged.
        let span = i64::from(maximum)
            .saturating_sub(i64::from(minimum))
            .saturating_add(1)
            .max(1) as u64;
        let offset = self.next_u64() % span;
        i64::from(minimum).saturating_add(offset as i64) as i32
    }

    pub(crate) fn uniform_below(&mut self, upper_exclusive: u64) -> Result<u64, WorldError> {
        if upper_exclusive == 0 {
            return Err(WorldError::InvalidGameplayDefinition);
        }
        let threshold = upper_exclusive.wrapping_neg() % upper_exclusive;
        loop {
            let value = self.next_u64();
            if value >= threshold {
                return Ok(value % upper_exclusive);
            }
        }
    }
}

#[derive(Clone, Debug)]
pub(crate) struct ObserverView {
    pub(crate) salt: [u8; 32],
    pub(crate) revision: u64,
    pub(crate) discovered_rooms: BTreeSet<String>,
    pub(crate) role: AccountRole,
}

#[derive(Clone, Debug)]
pub(crate) enum Entity {
    Room(Box<Room>),
    Character(Character),
}

pub(crate) fn room(entities: &SlotMap<EntityId, Entity>, id: EntityId) -> &Room {
    match &entities[id] {
        Entity::Room(room) => room,
        Entity::Character(_) => unreachable!("room reference points to a character"),
    }
}

pub(crate) fn room_mut(entities: &mut SlotMap<EntityId, Entity>, id: EntityId) -> &mut Room {
    match &mut entities[id] {
        Entity::Room(room) => room,
        Entity::Character(_) => unreachable!("room reference points to a character"),
    }
}

pub(crate) fn room_checked(
    entities: &SlotMap<EntityId, Entity>,
    id: EntityId,
) -> Result<&Room, WorldError> {
    match entities.get(id) {
        Some(Entity::Room(room)) => Ok(room),
        Some(Entity::Character(_)) | None => Err(WorldError::InvalidEntity),
    }
}
