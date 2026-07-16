//! Data-driven resources, abilities and status effects.
//!
//! These types deliberately do not depend on WorldPack. The server converts a
//! reviewed authoring document into these bounded runtime definitions.

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

use crate::CharacterId;

pub const MAX_ABILITIES: usize = 512;
pub const MAX_KNOWN_ABILITIES: usize = 64;
pub const MAX_ABILITY_EFFECTS: usize = 8;
pub const MAX_ABILITY_RESOURCE_COSTS: usize = 4;
pub const MAX_RESOURCE_POOLS: usize = 16;
pub const MAX_RESISTANCES: usize = 16;
pub const MAX_ACTIVE_STATUSES: usize = 32;
pub const MAX_PROGRESSION_LEVEL: u32 = 100;
pub const MAX_STATS: usize = 64;
pub const MAX_SKILL_VALUE: u16 = 1_000;
pub const MAX_STAT_VALUE: u32 = 1_000_000;
pub const MAX_RESOURCE_VALUE: u32 = 1_000_000;
pub const MAX_COOLDOWN_TICKS: u32 = 10_000;
pub const MAX_STATUS_TICKS: u32 = 10_000;
pub const MAX_STATUS_MAGNITUDE: i32 = 100_000;
pub const MAX_RESISTANCE_PERCENT: u16 = 90;

#[derive(Clone, Copy, Debug, Deserialize, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum DamageType {
    Physical,
    Arcane,
    Fire,
    Frost,
    Lightning,
}

impl DamageType {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Physical => "physical",
            Self::Arcane => "arcane",
            Self::Fire => "fire",
            Self::Frost => "frost",
            Self::Lightning => "lightning",
        }
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum AbilityCategory {
    Martial,
    Spell,
    Utility,
}

impl AbilityCategory {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Martial => "martial",
            Self::Spell => "spell",
            Self::Utility => "utility",
        }
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum AbilityTargeting {
    SelfOnly,
    SingleEnemy,
    SingleAlly,
}

impl AbilityTargeting {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::SelfOnly => "self",
            Self::SingleEnemy => "explicitTarget",
            Self::SingleAlly => "optionalTarget",
        }
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum AbilityEffectTarget {
    Caster,
    Target,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AbilityDamageDefinition {
    pub target: AbilityEffectTarget,
    pub damage_type: DamageType,
    pub base_amount: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub scaling_attribute: Option<String>,
    #[serde(default)]
    pub scaling_percent: u16,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AbilityHealDefinition {
    pub target: AbilityEffectTarget,
    pub base_amount: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub scaling_attribute: Option<String>,
    #[serde(default)]
    pub scaling_percent: u16,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum AbilityStatusEffectDefinition {
    AttributeModifier {
        target: AbilityEffectTarget,
        attribute: String,
        magnitude: i32,
        duration_ticks: u32,
    },
    Regeneration {
        target: AbilityEffectTarget,
        magnitude: u32,
        duration_ticks: u32,
    },
    DamageOverTime {
        target: AbilityEffectTarget,
        damage_type: DamageType,
        magnitude: u32,
        duration_ticks: u32,
    },
    Stun {
        target: AbilityEffectTarget,
        duration_ticks: u32,
    },
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AbilityDefinition {
    pub id: String,
    pub name: String,
    pub description: String,
    pub category: AbilityCategory,
    #[serde(default)]
    pub resource_costs: BTreeMap<String, u32>,
    #[serde(default)]
    pub cooldown_ticks: u32,
    pub targeting: AbilityTargeting,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub damage: Option<AbilityDamageDefinition>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub heal: Option<AbilityHealDefinition>,
    #[serde(default)]
    pub status_effects: Vec<AbilityStatusEffectDefinition>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct NpcAbilitySetDefinition {
    pub npc_id: String,
    pub resource_pools: BTreeMap<String, u32>,
    pub resource_regeneration: BTreeMap<String, u32>,
    pub resistances: BTreeMap<DamageType, u16>,
    pub ability_ids: Vec<String>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ResourcePool {
    pub current: u32,
    pub max: u32,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ActiveStatusKind {
    AttributeModifier,
    Regeneration,
    DamageOverTime,
    Stun,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ActiveStatusEffect {
    pub source_ability_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_character_id: Option<CharacterId>,
    /// Stable content ID of the NPC that applied the status, when applicable.
    /// This lets delayed damage preserve defeat attribution without exposing
    /// the ID directly to clients.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_npc_id: Option<String>,
    pub kind: ActiveStatusKind,
    pub magnitude: i32,
    pub remaining_ticks: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub attribute: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub damage_type: Option<DamageType>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AttributeView {
    pub id: String,
    pub label: String,
    pub value: i32,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ResourceView {
    pub id: String,
    pub label: String,
    pub current: u32,
    pub max: u32,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AbilityResourceCostView {
    pub resource_id: String,
    pub label: String,
    pub amount: u32,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AbilityView {
    pub id: String,
    pub name: String,
    pub description: String,
    pub kind: String,
    pub resource_costs: Vec<AbilityResourceCostView>,
    pub cooldown_remaining: u64,
    pub usable: bool,
    pub targeting: String,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ResistanceView {
    pub damage_type: DamageType,
    pub percent: u16,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StatusView {
    pub id: String,
    pub label: String,
    pub kind: ActiveStatusKind,
    pub magnitude: i32,
    pub remaining_ticks: u32,
}

pub(crate) fn valid_definition_id(value: &str) -> bool {
    let bytes = value.as_bytes();
    !bytes.is_empty()
        && bytes.len() <= 128
        && bytes[0].is_ascii_alphanumeric()
        && bytes[bytes.len() - 1].is_ascii_alphanumeric()
        && value.bytes().all(|byte| {
            byte.is_ascii_lowercase()
                || byte.is_ascii_digit()
                || matches!(byte, b'-' | b'_' | b'.' | b':')
        })
}

pub(crate) fn valid_stat_id(value: &str) -> bool {
    value.len() <= 64 && valid_definition_id(value)
}

pub(crate) fn humanize_id(value: &str) -> String {
    let tail = value.rsplit([':', '.']).next().unwrap_or(value);
    let mut output = String::with_capacity(tail.len());
    let mut uppercase = true;
    for character in tail.chars() {
        if matches!(character, '-' | '_') {
            output.push(' ');
            uppercase = true;
        } else if uppercase {
            output.extend(character.to_uppercase());
            uppercase = false;
        } else {
            output.push(character);
        }
    }
    output
}
