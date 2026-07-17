use std::{
    collections::{BTreeMap, BTreeSet, HashMap, HashSet},
    fmt,
    time::Duration,
};

use serde::{Deserialize, Serialize};
use thiserror::Error;
use tiamud_scripting::{MAX_MODULES_PER_ROOM_HOOK_BATCH, ScriptLimits, validate_module};
pub use tiamud_scripting::{ScriptCapability, ScriptHook};

use super::{
    Direction, EffectRule, EquipmentSlot, InitialItemLocation, MAX_ROOM_DESCRIPTION_BYTES,
    MAX_ROOM_TITLE_BYTES, ValidationError, ValidationErrors, WorldPack, safe_gameplay_label,
    safe_gameplay_text, valid_content_addressed_audio_path, valid_relative_asset_path,
};

const MAX_CONDITION_DEPTH: usize = 32;
const MAX_CONDITION_NODES: usize = 1_024;

/// A WorldPack v7 plus optional authoring data ignored by older runtime readers.
///
/// Keeping the additions at the top level preserves Rust source compatibility for
/// existing `WorldPack` and `Room` struct literals. Serde's default unknown-field
/// behavior also lets existing runtime readers consume a built document safely.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WorldPackDocument {
    #[serde(flatten)]
    pub world: WorldPack,
    /// Localized, presentation-only media metadata keyed by stable room ID.
    /// Keeping it outside `Room` preserves the portable gameplay schema.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub room_media: Vec<RoomMediaDefinition>,
    /// Optional presentation annotations for item instances and NPC spawns.
    /// These definitions never affect visibility or gameplay and are joined to
    /// observer-safe scene entities only after the core has filtered them.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub entity_media: Vec<EntityMediaDefinition>,
    /// Gameplay-facing perception policy, grouped per room so hidden state is
    /// validated before the runtime can project it.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub room_perception: Vec<RoomPerceptionDefinition>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub scene_variants: Vec<RoomSceneVariants>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub scripts: Vec<ScriptDefinition>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub content_tests: Vec<ContentTest>,
}

impl From<WorldPack> for WorldPackDocument {
    fn from(world: WorldPack) -> Self {
        Self {
            world,
            room_media: Vec::new(),
            entity_media: Vec::new(),
            room_perception: Vec::new(),
            scene_variants: Vec::new(),
            scripts: Vec::new(),
            content_tests: Vec::new(),
        }
    }
}

pub const MAX_ENTITY_MEDIA_DEFINITIONS: usize = 512;
pub const MAX_PROJECTED_ENTITY_MEDIA: usize = 12;
pub const MAX_PROJECTED_ENTITY_POSES: usize = 32;

#[derive(Debug, Clone, Copy, Hash, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EntityMediaKind {
    Item,
    Npc,
}

impl EntityMediaKind {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Item => "item",
            Self::Npc => "npc",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EntityMediaLayer {
    Midground,
    Foreground,
    Overlay,
}

impl EntityMediaLayer {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Midground => "midground",
            Self::Foreground => "foreground",
            Self::Overlay => "overlay",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NormalizedFootprint {
    /// Width in thousandths of the scene, constrained to 20..=600.
    pub width_milli: u16,
    /// Height in thousandths of the scene, constrained to 20..=800.
    pub height_milli: u16,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NormalizedAnchor {
    /// Horizontal bottom-centre anchor in thousandths of the scene.
    pub x_milli: u16,
    /// Vertical bottom-centre anchor in thousandths, constrained to 200..=1000.
    pub y_milli: u16,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct EntitySpriteSource {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub webp: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub png: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct EntitySpritePoses {
    pub idle: EntitySpriteSource,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub attack: Option<EntitySpriteSource>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hit: Option<EntitySpriteSource>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub defeated: Option<EntitySpriteSource>,
}

impl EntitySpritePoses {
    pub fn count(&self) -> usize {
        1 + usize::from(self.attack.is_some())
            + usize::from(self.hit.is_some())
            + usize::from(self.defeated.is_some())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct EntityMediaDefinition {
    pub kind: EntityMediaKind,
    /// Stable item-prototype ID for `item`, or stable NPC-spawn ID for `npc`.
    pub subject_id: String,
    /// Localized reviewed alternative text. Decorative sprites must omit it.
    #[serde(default, skip_serializing_if = "LocalizedText::is_empty")]
    pub alt: LocalizedText,
    #[serde(default)]
    pub decorative: bool,
    pub footprint: NormalizedFootprint,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub preferred_anchor: Option<NormalizedAnchor>,
    #[serde(default)]
    pub priority: i16,
    pub layer: EntityMediaLayer,
    pub poses: EntitySpritePoses,
}

impl EntityMediaDefinition {
    pub fn key(&self) -> String {
        format!("{}:{}", self.kind.as_str(), self.subject_id)
    }
}

/// Bounded BCP-47-like locale keys mapped to plain, non-empty display text.
/// Selection is deterministic: exact locale, language, English, then the first
/// lexicographic entry.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct LocalizedText(pub BTreeMap<String, String>);

impl LocalizedText {
    pub fn resolve(&self, locale: &str) -> Option<&str> {
        let normalized = locale.trim().to_ascii_lowercase();
        self.0
            .iter()
            .find(|(key, _)| key.eq_ignore_ascii_case(&normalized))
            .or_else(|| {
                normalized.split_once('-').and_then(|(language, _)| {
                    self.0
                        .iter()
                        .find(|(key, _)| key.eq_ignore_ascii_case(language))
                })
            })
            .or_else(|| {
                self.0
                    .iter()
                    .find(|(key, _)| key.eq_ignore_ascii_case("en"))
            })
            .or_else(|| self.0.first_key_value())
            .map(|(_, value)| value.as_str())
    }

    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    fn validate(&self) -> Result<(), String> {
        if self.0.is_empty() {
            return Err("at least one locale is required".to_owned());
        }
        if self.0.len() > 16 {
            return Err("at most 16 locales are allowed".to_owned());
        }
        let mut normalized_locales = HashSet::new();
        for (locale, text) in &self.0 {
            if !valid_locale(locale) {
                return Err(format!("locale {locale:?} is invalid"));
            }
            if !normalized_locales.insert(locale.to_ascii_lowercase()) {
                return Err(format!(
                    "locale {locale:?} is duplicated case-insensitively"
                ));
            }
            let trimmed = text.trim();
            if trimmed.is_empty() || trimmed.len() > 512 || trimmed.chars().any(char::is_control) {
                return Err(format!(
                    "alt text for locale {locale:?} must contain 1..=512 safe bytes"
                ));
            }
        }
        Ok(())
    }
}

fn valid_locale(locale: &str) -> bool {
    (2..=35).contains(&locale.len())
        && locale
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-')
        && locale
            .as_bytes()
            .first()
            .is_some_and(u8::is_ascii_alphanumeric)
        && locale
            .as_bytes()
            .last()
            .is_some_and(u8::is_ascii_alphanumeric)
        && !locale.contains("--")
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AudioChannel {
    Ambience,
    Effects,
    Music,
}

impl AudioChannel {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Ambience => "ambience",
            Self::Effects => "effects",
            Self::Music => "music",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AudioCue {
    pub channel: AudioChannel,
    pub path: String,
    #[serde(default, rename = "loop")]
    pub looped: bool,
}

impl AudioCue {
    fn validate(&self) -> Result<(), String> {
        valid_content_addressed_audio_path(&self.path)
            .then_some(())
            .ok_or_else(|| {
                "path must be assets/<lowercase-sha256>/audio-<name>.{opus,ogg,mp3}".to_owned()
            })
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RoomMediaDefinition {
    pub room_id: String,
    #[serde(default, skip_serializing_if = "LocalizedText::is_empty")]
    pub image_alt: LocalizedText,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub audio: Option<AudioCue>,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LightLevel {
    #[default]
    Bright,
    Dim,
    Dark,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RoomPerceptionDefinition {
    pub room_id: String,
    #[serde(default)]
    pub light: LightLevel,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub hidden_items: BTreeMap<String, u16>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub hidden_npcs: BTreeMap<String, u16>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub secret_exits: BTreeMap<Direction, u16>,
}

/// A server-side Lua module attached to one room. Only the named hooks are
/// callable and every effect must pass through one of the declared capabilities.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ScriptDefinition {
    pub id: String,
    pub room_id: String,
    #[serde(default)]
    pub hooks: Vec<ScriptHook>,
    #[serde(default)]
    pub capabilities: Vec<ScriptCapability>,
    #[serde(default)]
    pub limits: ScriptLimitsDefinition,
    pub source: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ScriptLimitsDefinition {
    #[serde(default = "default_script_source_bytes")]
    pub max_source_bytes: usize,
    #[serde(default = "default_script_memory_bytes")]
    pub max_memory_bytes: usize,
    #[serde(default = "default_script_instructions")]
    pub max_instructions: u64,
    #[serde(default = "default_script_wall_time_millis")]
    pub max_wall_time_millis: u64,
    #[serde(default = "default_script_events")]
    pub max_events: usize,
    #[serde(default = "default_script_event_field_bytes")]
    pub max_event_field_bytes: usize,
}

const fn default_script_source_bytes() -> usize {
    64 * 1024
}

const fn default_script_memory_bytes() -> usize {
    2 * 1024 * 1024
}

const fn default_script_instructions() -> u64 {
    100_000
}

const fn default_script_wall_time_millis() -> u64 {
    25
}

const fn default_script_events() -> usize {
    32
}

const fn default_script_event_field_bytes() -> usize {
    1024
}

impl Default for ScriptLimitsDefinition {
    fn default() -> Self {
        Self {
            max_source_bytes: default_script_source_bytes(),
            max_memory_bytes: default_script_memory_bytes(),
            max_instructions: default_script_instructions(),
            max_wall_time_millis: default_script_wall_time_millis(),
            max_events: default_script_events(),
            max_event_field_bytes: default_script_event_field_bytes(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RoomSceneVariants {
    pub room_id: String,
    #[serde(default)]
    pub variants: Vec<SceneVariant>,
}

/// Author-only scene choice. Its id and condition are deliberately absent from
/// `ResolvedScene`, the only public result of scene selection.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SceneVariant {
    pub id: String,
    #[serde(default)]
    pub priority: i32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub when: Option<Condition>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub asset_manifest: Option<String>,
    #[serde(default, skip_serializing_if = "LocalizedText::is_empty")]
    pub image_alt: LocalizedText,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub audio: Option<AudioCue>,
}

/// Declarative, bounded condition AST. The externally-tagged representation is
/// intentionally close to the authoring syntax in the specification, for example
/// `{"eq":["world.time.period","night"]}`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Condition {
    All(Vec<Condition>),
    Any(Vec<Condition>),
    Not(Box<Condition>),
    Eq(ConditionField, ConditionValue),
    Ne(ConditionField, ConditionValue),
    In(ConditionField, Vec<ConditionValue>),
    Gt(ConditionField, i64),
    Gte(ConditionField, i64),
    Lt(ConditionField, i64),
    Lte(ConditionField, i64),
    Contains(ConditionField, String),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum ConditionField {
    #[serde(rename = "world.instance")]
    WorldInstance,
    #[serde(rename = "world.time.period")]
    WorldTimePeriod,
    #[serde(rename = "world.weather")]
    WorldWeather,
    #[serde(rename = "world.time.hour")]
    WorldTimeHour,
    #[serde(rename = "actor.level")]
    ActorLevel,
    #[serde(rename = "actor.is_admin")]
    ActorIsAdmin,
    #[serde(rename = "actor.flags")]
    ActorFlags,
    #[serde(rename = "actor.quest_states")]
    ActorQuestStates,
    #[serde(rename = "actor.content_preferences")]
    ActorContentPreferences,
    #[serde(rename = "actor.senses")]
    ActorSenses,
}

impl fmt::Display for ConditionField {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::WorldInstance => "world.instance",
            Self::WorldTimePeriod => "world.time.period",
            Self::WorldWeather => "world.weather",
            Self::WorldTimeHour => "world.time.hour",
            Self::ActorLevel => "actor.level",
            Self::ActorIsAdmin => "actor.is_admin",
            Self::ActorFlags => "actor.flags",
            Self::ActorQuestStates => "actor.quest_states",
            Self::ActorContentPreferences => "actor.content_preferences",
            Self::ActorSenses => "actor.senses",
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(untagged)]
pub enum ConditionValue {
    Boolean(bool),
    Integer(i64),
    String(String),
}

impl ConditionValue {
    fn kind_name(&self) -> &'static str {
        match self {
            Self::Boolean(_) => "boolean",
            Self::Integer(_) => "integer",
            Self::String(_) => "string",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum TimePeriod {
    Dawn,
    #[default]
    Day,
    Dusk,
    Night,
}

impl TimePeriod {
    fn as_str(self) -> &'static str {
        match self {
            Self::Dawn => "dawn",
            Self::Day => "day",
            Self::Dusk => "dusk",
            Self::Night => "night",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum Weather {
    #[default]
    Clear,
    Cloudy,
    Rain,
    Storm,
    Snow,
    Fog,
}

impl Weather {
    fn as_str(self) -> &'static str {
        match self {
            Self::Clear => "clear",
            Self::Cloudy => "cloudy",
            Self::Rain => "rain",
            Self::Storm => "storm",
            Self::Snow => "snow",
            Self::Fog => "fog",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ConditionContext {
    #[serde(default = "default_world_instance")]
    pub world_instance: String,
    #[serde(default)]
    pub world_time_period: TimePeriod,
    #[serde(default)]
    pub world_weather: Weather,
    #[serde(default = "default_world_hour")]
    pub world_time_hour: u8,
    #[serde(default = "default_actor_level")]
    pub actor_level: u32,
    #[serde(default)]
    pub actor_is_admin: bool,
    #[serde(default, skip_serializing_if = "BTreeSet::is_empty")]
    pub actor_flags: BTreeSet<String>,
    #[serde(default, skip_serializing_if = "BTreeSet::is_empty")]
    pub actor_quest_states: BTreeSet<String>,
    #[serde(default, skip_serializing_if = "BTreeSet::is_empty")]
    pub actor_content_preferences: BTreeSet<String>,
    #[serde(default, skip_serializing_if = "BTreeSet::is_empty")]
    pub actor_senses: BTreeSet<String>,
}

fn default_world_instance() -> String {
    "primary".to_owned()
}

const fn default_world_hour() -> u8 {
    12
}

const fn default_actor_level() -> u32 {
    1
}

impl Default for ConditionContext {
    fn default() -> Self {
        Self {
            world_instance: default_world_instance(),
            world_time_period: TimePeriod::default(),
            world_weather: Weather::default(),
            world_time_hour: default_world_hour(),
            actor_level: default_actor_level(),
            actor_is_admin: false,
            actor_flags: BTreeSet::new(),
            actor_quest_states: BTreeSet::new(),
            actor_content_preferences: BTreeSet::new(),
            actor_senses: BTreeSet::new(),
        }
    }
}

impl ConditionContext {
    fn validate(&self) -> Result<(), String> {
        if self.world_instance.trim().is_empty() || self.world_instance.len() > 64 {
            return Err("worldInstance must contain 1..=64 safe bytes".into());
        }
        if self.world_time_hour > 23 {
            return Err(format!(
                "worldTimeHour {} is outside 0..=23",
                self.world_time_hour
            ));
        }
        if let Some(flag) = self.actor_flags.iter().find(|flag| flag.trim().is_empty()) {
            return Err(format!("actorFlags contains an empty flag {flag:?}"));
        }
        for (label, values) in [
            ("actorQuestStates", &self.actor_quest_states),
            ("actorContentPreferences", &self.actor_content_preferences),
            ("actorSenses", &self.actor_senses),
        ] {
            if values.len() > 64
                || values
                    .iter()
                    .any(|value| value.trim().is_empty() || value.len() > 128)
            {
                return Err(format!("{label} contains invalid or excessive values"));
            }
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum ConditionValidationError {
    #[error("condition nesting exceeds {maximum} levels")]
    TooDeep { maximum: usize },
    #[error("condition contains more than {maximum} AST nodes")]
    TooManyNodes { maximum: usize },
    #[error("operator {operator} requires at least one operand")]
    EmptyOperands { operator: &'static str },
    #[error(
        "operator {operator} cannot be used with {field}; expected {expected} field, found {found}"
    )]
    InvalidOperator {
        operator: &'static str,
        field: ConditionField,
        expected: &'static str,
        found: &'static str,
    },
    #[error(
        "operator {operator} compares {field} with {found}; the field requires {expected} values"
    )]
    TypeMismatch {
        operator: &'static str,
        field: ConditionField,
        expected: &'static str,
        found: &'static str,
    },
    #[error("operator {operator} has an empty value list for {field}")]
    EmptyValueList {
        operator: &'static str,
        field: ConditionField,
    },
    #[error("value {value:?} is not valid for {field}; expected {expected}")]
    InvalidEnumeratedValue {
        field: ConditionField,
        value: String,
        expected: &'static str,
    },
    #[error("value {value} is outside {minimum}..={maximum} for {field}")]
    IntegerOutOfRange {
        field: ConditionField,
        value: i64,
        minimum: i64,
        maximum: i64,
    },
    #[error("operator contains requires a non-empty value for {field}")]
    EmptyContainsValue { field: ConditionField },
    #[error("all contains contradictory equality values for {field}")]
    ContradictoryEquality { field: ConditionField },
}

#[derive(Debug, Clone, PartialEq, Eq, Error)]
#[error("condition validation failed with {} error(s)", .0.len())]
pub struct ConditionValidationErrors(pub Vec<ConditionValidationError>);

impl Condition {
    pub fn validate(&self) -> Result<(), ConditionValidationErrors> {
        let mut errors = Vec::new();
        let mut nodes = 0;
        validate_condition(self, 0, &mut nodes, &mut errors);
        if errors.is_empty() {
            Ok(())
        } else {
            Err(ConditionValidationErrors(errors))
        }
    }

    pub fn evaluate(&self, context: &ConditionContext) -> Result<bool, ConditionValidationErrors> {
        self.validate()?;
        Ok(self.evaluate_validated(context))
    }

    fn evaluate_validated(&self, context: &ConditionContext) -> bool {
        match self {
            Self::All(conditions) => conditions
                .iter()
                .all(|condition| condition.evaluate_validated(context)),
            Self::Any(conditions) => conditions
                .iter()
                .any(|condition| condition.evaluate_validated(context)),
            Self::Not(condition) => !condition.evaluate_validated(context),
            Self::Eq(field, value) => context_value_equals(*field, value, context),
            Self::Ne(field, value) => !context_value_equals(*field, value, context),
            Self::In(field, values) => values
                .iter()
                .any(|value| context_value_equals(*field, value, context)),
            Self::Gt(field, value) => context_integer(*field, context) > *value,
            Self::Gte(field, value) => context_integer(*field, context) >= *value,
            Self::Lt(field, value) => context_integer(*field, context) < *value,
            Self::Lte(field, value) => context_integer(*field, context) <= *value,
            Self::Contains(ConditionField::ActorFlags, value) => {
                context.actor_flags.contains(value)
            }
            Self::Contains(ConditionField::ActorQuestStates, value) => {
                context.actor_quest_states.contains(value)
            }
            Self::Contains(ConditionField::ActorContentPreferences, value) => {
                context.actor_content_preferences.contains(value)
            }
            Self::Contains(ConditionField::ActorSenses, value) => {
                context.actor_senses.contains(value)
            }
            Self::Contains(_, _) => false,
        }
    }

    fn canonicalize(&mut self) {
        match self {
            Self::All(conditions) | Self::Any(conditions) => {
                for condition in conditions.iter_mut() {
                    condition.canonicalize();
                }
                conditions.sort_by_cached_key(condition_sort_key);
            }
            Self::Not(condition) => condition.canonicalize(),
            Self::In(_, values) => values.sort_by_cached_key(condition_value_sort_key),
            _ => {}
        }
    }
}

fn validate_condition(
    condition: &Condition,
    depth: usize,
    nodes: &mut usize,
    errors: &mut Vec<ConditionValidationError>,
) {
    *nodes += 1;
    if *nodes == MAX_CONDITION_NODES + 1 {
        errors.push(ConditionValidationError::TooManyNodes {
            maximum: MAX_CONDITION_NODES,
        });
    }
    if *nodes > MAX_CONDITION_NODES {
        return;
    }
    if depth > MAX_CONDITION_DEPTH {
        errors.push(ConditionValidationError::TooDeep {
            maximum: MAX_CONDITION_DEPTH,
        });
        return;
    }

    match condition {
        Condition::All(conditions) => {
            validate_condition_list("all", conditions, depth, nodes, errors);
            let mut equalities: HashMap<ConditionField, &ConditionValue> = HashMap::new();
            for child in conditions {
                if let Condition::Eq(field, value) = child
                    && let Some(previous) = equalities.insert(*field, value)
                    && previous != value
                {
                    errors.push(ConditionValidationError::ContradictoryEquality { field: *field });
                }
            }
        }
        Condition::Any(conditions) => {
            validate_condition_list("any", conditions, depth, nodes, errors);
        }
        Condition::Not(condition) => {
            validate_condition(condition, depth + 1, nodes, errors);
        }
        Condition::Eq(field, value) => validate_scalar("eq", *field, value, errors),
        Condition::Ne(field, value) => validate_scalar("ne", *field, value, errors),
        Condition::In(field, values) => {
            if values.is_empty() {
                errors.push(ConditionValidationError::EmptyValueList {
                    operator: "in",
                    field: *field,
                });
            }
            for value in values {
                validate_scalar("in", *field, value, errors);
            }
        }
        Condition::Gt(field, value)
        | Condition::Gte(field, value)
        | Condition::Lt(field, value)
        | Condition::Lte(field, value) => {
            let operator = match condition {
                Condition::Gt(_, _) => "gt",
                Condition::Gte(_, _) => "gte",
                Condition::Lt(_, _) => "lt",
                Condition::Lte(_, _) => "lte",
                _ => unreachable!(),
            };
            if !matches!(
                field,
                ConditionField::WorldTimeHour | ConditionField::ActorLevel
            ) {
                errors.push(ConditionValidationError::InvalidOperator {
                    operator,
                    field: *field,
                    expected: "integer",
                    found: field_kind_name(*field),
                });
            } else {
                validate_integer_domain(*field, *value, errors);
            }
        }
        Condition::Contains(field, value) => {
            if !matches!(
                field,
                ConditionField::ActorFlags
                    | ConditionField::ActorQuestStates
                    | ConditionField::ActorContentPreferences
                    | ConditionField::ActorSenses
            ) {
                errors.push(ConditionValidationError::InvalidOperator {
                    operator: "contains",
                    field: *field,
                    expected: "string-set",
                    found: field_kind_name(*field),
                });
            }
            if value.trim().is_empty() {
                errors.push(ConditionValidationError::EmptyContainsValue { field: *field });
            }
        }
    }
}

fn validate_condition_list(
    operator: &'static str,
    conditions: &[Condition],
    depth: usize,
    nodes: &mut usize,
    errors: &mut Vec<ConditionValidationError>,
) {
    if conditions.is_empty() {
        errors.push(ConditionValidationError::EmptyOperands { operator });
    }
    for condition in conditions {
        validate_condition(condition, depth + 1, nodes, errors);
    }
}

fn validate_scalar(
    operator: &'static str,
    field: ConditionField,
    value: &ConditionValue,
    errors: &mut Vec<ConditionValidationError>,
) {
    let expected = field_kind_name(field);
    if matches!(
        field,
        ConditionField::ActorFlags
            | ConditionField::ActorQuestStates
            | ConditionField::ActorContentPreferences
            | ConditionField::ActorSenses
    ) {
        errors.push(ConditionValidationError::InvalidOperator {
            operator,
            field,
            expected: "scalar",
            found: expected,
        });
        return;
    }
    let type_matches = matches!(
        (field, value),
        (
            ConditionField::WorldInstance
                | ConditionField::WorldTimePeriod
                | ConditionField::WorldWeather,
            ConditionValue::String(_)
        ) | (
            ConditionField::WorldTimeHour | ConditionField::ActorLevel,
            ConditionValue::Integer(_)
        ) | (ConditionField::ActorIsAdmin, ConditionValue::Boolean(_))
    );
    if !type_matches {
        errors.push(ConditionValidationError::TypeMismatch {
            operator,
            field,
            expected,
            found: value.kind_name(),
        });
        return;
    }
    match (field, value) {
        (ConditionField::WorldTimePeriod, ConditionValue::String(value))
            if !matches!(value.as_str(), "dawn" | "day" | "dusk" | "night") =>
        {
            errors.push(ConditionValidationError::InvalidEnumeratedValue {
                field,
                value: value.clone(),
                expected: "dawn, day, dusk, or night",
            });
        }
        (ConditionField::WorldWeather, ConditionValue::String(value))
            if !matches!(
                value.as_str(),
                "clear" | "cloudy" | "rain" | "storm" | "snow" | "fog"
            ) =>
        {
            errors.push(ConditionValidationError::InvalidEnumeratedValue {
                field,
                value: value.clone(),
                expected: "clear, cloudy, rain, storm, snow, or fog",
            });
        }
        (
            field @ (ConditionField::WorldTimeHour | ConditionField::ActorLevel),
            ConditionValue::Integer(value),
        ) => validate_integer_domain(field, *value, errors),
        _ => {}
    }
}

fn validate_integer_domain(
    field: ConditionField,
    value: i64,
    errors: &mut Vec<ConditionValidationError>,
) {
    let range = match field {
        ConditionField::WorldTimeHour => Some((0, 23)),
        ConditionField::ActorLevel => Some((0, i64::from(u32::MAX))),
        _ => None,
    };
    if let Some((minimum, maximum)) = range
        && !(minimum..=maximum).contains(&value)
    {
        errors.push(ConditionValidationError::IntegerOutOfRange {
            field,
            value,
            minimum,
            maximum,
        });
    }
}

fn field_kind_name(field: ConditionField) -> &'static str {
    match field {
        ConditionField::WorldInstance
        | ConditionField::WorldTimePeriod
        | ConditionField::WorldWeather => "string",
        ConditionField::WorldTimeHour | ConditionField::ActorLevel => "integer",
        ConditionField::ActorIsAdmin => "boolean",
        ConditionField::ActorFlags
        | ConditionField::ActorQuestStates
        | ConditionField::ActorContentPreferences
        | ConditionField::ActorSenses => "string-set",
    }
}

fn context_value_equals(
    field: ConditionField,
    value: &ConditionValue,
    context: &ConditionContext,
) -> bool {
    match (field, value) {
        (ConditionField::WorldInstance, ConditionValue::String(value)) => {
            context.world_instance == *value
        }
        (ConditionField::WorldTimePeriod, ConditionValue::String(value)) => {
            context.world_time_period.as_str() == value
        }
        (ConditionField::WorldWeather, ConditionValue::String(value)) => {
            context.world_weather.as_str() == value
        }
        (ConditionField::WorldTimeHour, ConditionValue::Integer(value)) => {
            i64::from(context.world_time_hour) == *value
        }
        (ConditionField::ActorLevel, ConditionValue::Integer(value)) => {
            i64::from(context.actor_level) == *value
        }
        (ConditionField::ActorIsAdmin, ConditionValue::Boolean(value)) => {
            context.actor_is_admin == *value
        }
        _ => false,
    }
}

fn context_integer(field: ConditionField, context: &ConditionContext) -> i64 {
    match field {
        ConditionField::WorldTimeHour => i64::from(context.world_time_hour),
        ConditionField::ActorLevel => i64::from(context.actor_level),
        _ => 0,
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ContentTest {
    pub name: String,
    pub room_id: String,
    #[serde(default)]
    pub context: ConditionContext,
    pub expect: ContentTestExpectation,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct ContentTestExpectation {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub description_contains: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub output_excludes: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub asset_manifest: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub exits: Option<Vec<Direction>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub items: Option<Vec<String>>,
}

impl ContentTestExpectation {
    fn is_empty(&self) -> bool {
        self.title.is_none()
            && self.description_contains.is_empty()
            && self.output_excludes.is_empty()
            && self.asset_manifest.is_none()
            && self.exits.is_none()
            && self.items.is_none()
    }

    fn canonicalize(&mut self) {
        self.description_contains.sort();
        self.output_excludes.sort();
        if let Some(exits) = &mut self.exits {
            exits.sort_by_key(|direction| direction_rank(*direction));
        }
        if let Some(items) = &mut self.items {
            items.sort();
        }
    }
}

/// Authorized scene output. It intentionally contains neither a room/variant id
/// nor the condition that selected it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ResolvedScene {
    pub title: String,
    pub description: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub asset_manifest: Option<String>,
    #[serde(default, skip_serializing_if = "LocalizedText::is_empty")]
    pub image_alt: LocalizedText,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub audio: Option<AudioCue>,
    pub exits: Vec<Direction>,
    pub items: Vec<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PreviewFormat {
    Text,
    Ansi,
}

#[derive(Debug, Error)]
pub enum SceneResolutionError {
    #[error(transparent)]
    InvalidDocument(#[from] ValidationErrors),
    #[error("room {0:?} does not exist")]
    UnknownRoom(String),
    #[error("condition context is invalid: {0}")]
    InvalidContext(String),
}

#[derive(Debug, Error)]
pub enum DocumentToolError {
    #[error(transparent)]
    InvalidDocument(#[from] ValidationErrors),
    #[error(transparent)]
    InvalidScene(#[from] SceneResolutionError),
    #[error("WorldPack JSON is invalid: {0}")]
    Json(#[from] serde_json::Error),
    #[error("WorldPack v2 character progression cannot be normalized safely: {0}")]
    LegacyProgression(String),
    #[error("legacy WorldPack items cannot be normalized safely: {0}")]
    LegacyItems(String),
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct LegacyItem {
    id: String,
    name: String,
    description: String,
    #[serde(default)]
    aliases: Vec<String>,
}

#[derive(Clone, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct LegacyItemRule {
    item_id: String,
    #[serde(default)]
    equipment_slot: Option<EquipmentSlot>,
    #[serde(default)]
    attack_bonus: i32,
    #[serde(default)]
    heal_amount: i32,
    #[serde(default)]
    consumable: bool,
    #[serde(default)]
    effect: Option<EffectRule>,
}

pub(crate) fn normalize_legacy_items(value: &mut serde_json::Value) -> Result<(), String> {
    let root = value
        .as_object_mut()
        .ok_or_else(|| "WorldPack root must be an object".to_owned())?;
    if root.contains_key("itemPrototypes") || root.contains_key("initialItemInstances") {
        return Err("legacy schemas cannot also declare native item fields".to_owned());
    }

    let legacy_items: Vec<LegacyItem> = serde_json::from_value(
        root.remove("items")
            .unwrap_or_else(|| serde_json::Value::Array(Vec::new())),
    )
    .map_err(|error| format!("invalid legacy items: {error}"))?;
    let (legacy_rules, npc_spawns_by_prototype) = {
        let gameplay = root
            .entry("gameplay")
            .or_insert_with(|| serde_json::Value::Object(serde_json::Map::new()))
            .as_object_mut()
            .ok_or_else(|| "gameplay must be an object".to_owned())?;
        let legacy_rules: Vec<LegacyItemRule> = serde_json::from_value(
            gameplay
                .remove("itemRules")
                .unwrap_or_else(|| serde_json::Value::Array(Vec::new())),
        )
        .map_err(|error| format!("invalid legacy item rules: {error}"))?;
        let mut npc_spawns_by_prototype: HashMap<String, Vec<String>> = HashMap::new();
        if let Some(spawns) = gameplay
            .get("npcSpawns")
            .and_then(serde_json::Value::as_array)
        {
            for spawn in spawns {
                let Some(spawn) = spawn.as_object() else {
                    continue;
                };
                let (Some(id), Some(prototype_id)) = (
                    spawn.get("id").and_then(serde_json::Value::as_str),
                    spawn.get("prototypeId").and_then(serde_json::Value::as_str),
                ) else {
                    continue;
                };
                npc_spawns_by_prototype
                    .entry(prototype_id.to_owned())
                    .or_default()
                    .push(id.to_owned());
            }
        }
        (legacy_rules, npc_spawns_by_prototype)
    };
    let mut rules = HashMap::new();
    for rule in legacy_rules {
        if rules.insert(rule.item_id.clone(), rule).is_some() {
            return Err("a legacy item has more than one gameplay rule".to_owned());
        }
    }

    let mut locations = HashMap::new();
    if let Some(rooms) = root
        .get_mut("rooms")
        .and_then(serde_json::Value::as_array_mut)
    {
        for room in rooms {
            let Some(room) = room.as_object_mut() else {
                continue;
            };
            let room_id = room
                .get("id")
                .and_then(serde_json::Value::as_str)
                .unwrap_or_default()
                .to_owned();
            let placed = room
                .remove("items")
                .unwrap_or_else(|| serde_json::Value::Array(Vec::new()));
            let placed = placed
                .as_array()
                .ok_or_else(|| format!("room {room_id:?} items must be an array"))?;
            for item in placed {
                let item_id = item
                    .as_str()
                    .ok_or_else(|| format!("room {room_id:?} has a non-string item id"))?;
                insert_legacy_location(
                    &mut locations,
                    item_id,
                    serde_json::json!({"kind":"room", "roomId":room_id}),
                )?;
            }
        }
    }

    if let Some(prototypes) = root
        .get_mut("gameplay")
        .and_then(serde_json::Value::as_object_mut)
        .and_then(|gameplay| gameplay.get_mut("npcPrototypes"))
        .and_then(serde_json::Value::as_array_mut)
    {
        for prototype in prototypes {
            let Some(prototype) = prototype.as_object_mut() else {
                continue;
            };
            let prototype_id = prototype
                .get("id")
                .and_then(serde_json::Value::as_str)
                .unwrap_or_default()
                .to_owned();
            let loot = prototype
                .remove("lootItemIds")
                .unwrap_or_else(|| serde_json::Value::Array(Vec::new()));
            let loot = loot
                .as_array()
                .ok_or_else(|| format!("NPC prototype {prototype_id:?} loot must be an array"))?;
            if loot.is_empty() {
                continue;
            }
            let spawns = npc_spawns_by_prototype
                .get(&prototype_id)
                .filter(|spawns| spawns.len() == 1)
                .ok_or_else(|| {
                    format!(
                        "NPC prototype {prototype_id:?} with legacy loot must have exactly one spawn"
                    )
                })?;
            for item in loot {
                let item_id = item.as_str().ok_or_else(|| {
                    format!("NPC prototype {prototype_id:?} has a non-string loot item id")
                })?;
                insert_legacy_location(
                    &mut locations,
                    item_id,
                    serde_json::json!({"kind":"npc_spawn", "npcSpawnId":spawns[0]}),
                )?;
            }
        }
    }

    let mut seen_items = HashSet::new();
    let mut item_prototypes = Vec::with_capacity(legacy_items.len());
    let mut initial_instances = Vec::with_capacity(legacy_items.len());
    for item in legacy_items {
        if !crate::valid_item_instance_identifier(&item.id) {
            return Err(format!(
                "legacy item id {:?} must be a canonical runtime id or contain 1..=128 lowercase namespaced identifier bytes",
                item.id
            ));
        }
        if !crate::safe_gameplay_label(&item.name, crate::MAX_ITEM_NAME_BYTES) {
            return Err(format!(
                "legacy item {:?} name must contain 1..={} safe bytes without control characters",
                item.id,
                crate::MAX_ITEM_NAME_BYTES
            ));
        }
        if !crate::safe_gameplay_text(&item.description, crate::MAX_ITEM_DESCRIPTION_BYTES) {
            return Err(format!(
                "legacy item {:?} description must contain 1..={} safe bytes",
                item.id,
                crate::MAX_ITEM_DESCRIPTION_BYTES
            ));
        }
        if item
            .aliases
            .iter()
            .any(|alias| !crate::safe_gameplay_label(alias, crate::MAX_ITEM_ALIAS_BYTES))
        {
            return Err(format!(
                "legacy item {:?} alias must contain 1..={} safe bytes without control characters",
                item.id,
                crate::MAX_ITEM_ALIAS_BYTES
            ));
        }
        if !seen_items.insert(item.id.clone()) {
            return Err(format!(
                "legacy item id {:?} appears more than once",
                item.id
            ));
        }
        let rule = rules.remove(&item.id).unwrap_or(LegacyItemRule {
            item_id: item.id.clone(),
            equipment_slot: None,
            attack_bonus: 0,
            heal_amount: 0,
            consumable: false,
            effect: None,
        });
        let location = locations
            .remove(&item.id)
            .ok_or_else(|| format!("legacy item {:?} has no unique initial location", item.id))?;
        item_prototypes.push(serde_json::json!({
            "id": item.id,
            "name": item.name,
            "description": item.description,
            "aliases": item.aliases,
            "equipmentSlot": rule.equipment_slot,
            "attackBonus": rule.attack_bonus,
            "healAmount": rule.heal_amount,
            "consumable": rule.consumable,
            "effect": rule.effect,
            "weightGrams": 0,
            "slotCost": 1,
            "rarity": "common",
            "itemLevel": 1,
            "stackPolicy": "unique",
            "maxStackUnits": 1,
            "transferPolicy": "transferable"
        }));
        initial_instances.push(serde_json::json!({
            "id": item.id,
            "prototypeId": item.id,
            "location": location
        }));
    }
    if let Some(item_id) = rules.keys().next() {
        return Err(format!(
            "legacy item rule references missing item {item_id:?}"
        ));
    }
    if let Some(item_id) = locations.keys().next() {
        return Err(format!(
            "legacy placement references missing item {item_id:?}"
        ));
    }
    root.insert(
        "itemPrototypes".to_owned(),
        serde_json::Value::Array(item_prototypes),
    );
    root.insert(
        "initialItemInstances".to_owned(),
        serde_json::Value::Array(initial_instances),
    );
    Ok(())
}

fn insert_legacy_location(
    locations: &mut HashMap<String, serde_json::Value>,
    item_id: &str,
    location: serde_json::Value,
) -> Result<(), String> {
    if locations.insert(item_id.to_owned(), location).is_some() {
        return Err(format!(
            "legacy item {item_id:?} has more than one initial location"
        ));
    }
    Ok(())
}

fn reject_native_legacy_item_fields(value: &serde_json::Value) -> Result<(), String> {
    let Some(root) = value.as_object() else {
        return Ok(());
    };
    if root.contains_key("items")
        || root
            .get("gameplay")
            .is_some_and(|gameplay| gameplay.get("itemRules").is_some())
    {
        return Err("WorldPack v7 must use itemPrototypes and initialItemInstances".to_owned());
    }
    if root
        .get("rooms")
        .and_then(serde_json::Value::as_array)
        .is_some_and(|rooms| rooms.iter().any(|room| room.get("items").is_some()))
    {
        return Err("WorldPack v7 rooms cannot declare legacy items".to_owned());
    }
    if root
        .get("gameplay")
        .and_then(|gameplay| gameplay.get("npcPrototypes"))
        .and_then(serde_json::Value::as_array)
        .is_some_and(|prototypes| {
            prototypes
                .iter()
                .any(|prototype| prototype.get("lootItemIds").is_some())
        })
    {
        return Err("WorldPack v7 NPC prototypes cannot declare legacy lootItemIds".to_owned());
    }
    Ok(())
}

fn reject_legacy_loot_fields(value: &serde_json::Value, schema_version: u64) -> Result<(), String> {
    if value.get("gameplay").is_some_and(|gameplay| {
        gameplay.get("lootTables").is_some() || gameplay.get("lootSources").is_some()
    }) {
        return Err(format!(
            "WorldPack v{schema_version} cannot declare v6 lootTables or lootSources"
        ));
    }
    Ok(())
}

fn reject_legacy_reward_fields(
    value: &serde_json::Value,
    schema_version: u64,
) -> Result<(), String> {
    let prototypes_have_sealed = ["itemPrototypes", "items"].into_iter().any(|field| {
        value
            .get(field)
            .and_then(serde_json::Value::as_array)
            .is_some_and(|prototypes| {
                prototypes
                    .iter()
                    .any(|prototype| prototype.get("sealedLootTableId").is_some())
            })
    });
    let gameplay = value.get("gameplay");
    let gameplay_has_affixes = gameplay.is_some_and(|gameplay| {
        gameplay.get("affixes").is_some() || gameplay.get("affixPools").is_some()
    });
    let loot_has_affix_pool = gameplay
        .and_then(|gameplay| gameplay.get("lootTables"))
        .and_then(serde_json::Value::as_array)
        .is_some_and(|tables| {
            tables.iter().any(|table| {
                table
                    .get("guaranteed")
                    .and_then(serde_json::Value::as_array)
                    .is_some_and(|entries| {
                        entries
                            .iter()
                            .any(|entry| entry.get("affixPoolId").is_some())
                    })
                    || table
                        .get("pools")
                        .and_then(serde_json::Value::as_array)
                        .is_some_and(|pools| {
                            pools.iter().any(|pool| {
                                pool.get("entries")
                                    .and_then(serde_json::Value::as_array)
                                    .is_some_and(|entries| {
                                        entries
                                            .iter()
                                            .any(|entry| entry.get("affixPoolId").is_some())
                                    })
                            })
                        })
            })
        });
    if prototypes_have_sealed || gameplay_has_affixes || loot_has_affix_pool {
        return Err(format!(
            "WorldPack v{schema_version} cannot declare v7 sealed rewards or affixes"
        ));
    }
    Ok(())
}

fn normalize_v6_rewards(value: &mut serde_json::Value) -> Result<(), String> {
    reject_native_legacy_item_fields(value)?;
    let root = value
        .as_object_mut()
        .ok_or_else(|| "WorldPack root must be an object".to_owned())?;
    root.insert(
        "schemaVersion".to_owned(),
        serde_json::json!(crate::CURRENT_SCHEMA_VERSION),
    );
    Ok(())
}

fn normalize_v4_items(value: &mut serde_json::Value) -> Result<(), String> {
    reject_native_legacy_item_fields(value)?;
    let root = value
        .as_object_mut()
        .ok_or_else(|| "WorldPack root must be an object".to_owned())?;
    let prototypes = root
        .entry("itemPrototypes")
        .or_insert_with(|| serde_json::Value::Array(Vec::new()))
        .as_array_mut()
        .ok_or_else(|| "WorldPack v4 itemPrototypes must be an array".to_owned())?;
    for prototype in prototypes {
        let prototype = prototype
            .as_object_mut()
            .ok_or_else(|| "WorldPack v4 item prototype must be an object".to_owned())?;
        if prototype.contains_key("maxStackUnits") || prototype.contains_key("container") {
            return Err(
                "WorldPack v4 cannot declare v5 maxStackUnits or container fields".to_owned(),
            );
        }
        if prototype
            .get("stackPolicy")
            .and_then(serde_json::Value::as_str)
            != Some("unique")
        {
            return Err("WorldPack v4 item prototypes must use stackPolicy unique".to_owned());
        }
        if prototype
            .get("transferPolicy")
            .and_then(serde_json::Value::as_str)
            != Some("transferable")
        {
            return Err(
                "WorldPack v4 item prototypes must use transferPolicy transferable".to_owned(),
            );
        }
        prototype.insert("maxStackUnits".to_owned(), serde_json::json!(1));
    }
    let instances = root
        .entry("initialItemInstances")
        .or_insert_with(|| serde_json::Value::Array(Vec::new()))
        .as_array()
        .ok_or_else(|| "WorldPack v4 initialItemInstances must be an array".to_owned())?;
    if instances.iter().any(|instance| {
        instance.get("location").is_some_and(|location| {
            location.get("kind").and_then(serde_json::Value::as_str) == Some("container")
                || location.get("containerInstanceId").is_some()
        })
    }) {
        return Err("WorldPack v4 cannot declare v5 container locations".to_owned());
    }
    root.insert(
        "schemaVersion".to_owned(),
        serde_json::json!(crate::CURRENT_SCHEMA_VERSION),
    );
    Ok(())
}

impl WorldPackDocument {
    pub fn from_json(bytes: &[u8]) -> Result<Self, DocumentToolError> {
        let mut value: serde_json::Value = serde_json::from_slice(bytes)?;
        let schema_version = value
            .get("schemaVersion")
            .and_then(serde_json::Value::as_u64)
            .unwrap_or_default();
        let legacy_schema = schema_version == 2 || schema_version == 3;
        if legacy_schema {
            reject_legacy_loot_fields(&value, schema_version)
                .map_err(DocumentToolError::LegacyItems)?;
            reject_legacy_reward_fields(&value, schema_version)
                .map_err(DocumentToolError::LegacyItems)?;
            normalize_legacy_items(&mut value).map_err(DocumentToolError::LegacyItems)?;
        } else if schema_version == 4 {
            reject_legacy_loot_fields(&value, schema_version)
                .map_err(DocumentToolError::LegacyItems)?;
            reject_legacy_reward_fields(&value, schema_version)
                .map_err(DocumentToolError::LegacyItems)?;
            normalize_v4_items(&mut value).map_err(DocumentToolError::LegacyItems)?;
        } else if schema_version == 5 {
            reject_legacy_loot_fields(&value, schema_version)
                .map_err(DocumentToolError::LegacyItems)?;
            reject_legacy_reward_fields(&value, schema_version)
                .map_err(DocumentToolError::LegacyItems)?;
            normalize_v6_rewards(&mut value).map_err(DocumentToolError::LegacyItems)?;
        } else if schema_version == 6 {
            reject_legacy_reward_fields(&value, schema_version)
                .map_err(DocumentToolError::LegacyItems)?;
            normalize_v6_rewards(&mut value).map_err(DocumentToolError::LegacyItems)?;
        } else if schema_version == u64::from(crate::CURRENT_SCHEMA_VERSION) {
            reject_native_legacy_item_fields(&value).map_err(DocumentToolError::LegacyItems)?;
        }
        let mut document: Self = serde_json::from_value(value)?;
        if document.world.schema_version == 2 {
            document.world.gameplay.character_progression =
                crate::progression::normalize_v2_character_progression(
                    std::mem::take(&mut document.world.gameplay.archetypes),
                    document.world.gameplay.default_archetype_id.take(),
                )
                .map_err(DocumentToolError::LegacyProgression)?;
        }
        if legacy_schema {
            document.world.schema_version = crate::CURRENT_SCHEMA_VERSION;
        }
        Ok(document)
    }

    pub fn validate(&self) -> Result<(), ValidationErrors> {
        let mut errors = self
            .world
            .validate()
            .err()
            .map_or_else(Vec::new, |error| error.0);
        let room_ids: HashSet<&str> = self
            .world
            .rooms
            .iter()
            .map(|room| room.id.as_str())
            .collect();

        let mut media_rooms = HashSet::new();
        for media in &self.room_media {
            let room = self
                .world
                .rooms
                .iter()
                .find(|room| room.id == media.room_id);
            if room.is_none() {
                errors.push(ValidationError::MissingRoomMediaRoom {
                    room_id: media.room_id.clone(),
                });
            }
            if !media_rooms.insert(media.room_id.as_str()) {
                errors.push(ValidationError::DuplicateRoomMedia {
                    room_id: media.room_id.clone(),
                });
            }
            if !media.image_alt.is_empty()
                && let Err(reason) = media.image_alt.validate()
            {
                errors.push(ValidationError::InvalidRoomImageAlt {
                    room_id: media.room_id.clone(),
                    reason,
                });
            }
            if let Some(audio) = &media.audio
                && let Err(reason) = audio.validate()
            {
                errors.push(ValidationError::InvalidRoomAudio {
                    room_id: media.room_id.clone(),
                    reason,
                });
            }
            if let Some(room) = room {
                match (room.asset_manifest.is_some(), media.image_alt.is_empty()) {
                    (true, true) => errors.push(ValidationError::MissingRoomImageAlt {
                        room_id: room.id.clone(),
                    }),
                    (false, false) => {
                        errors.push(ValidationError::RoomImageAltWithoutManifest {
                            room_id: room.id.clone(),
                        });
                    }
                    _ => {}
                }
            }
        }
        for room in &self.world.rooms {
            if room.asset_manifest.is_some() && !media_rooms.contains(room.id.as_str()) {
                errors.push(ValidationError::MissingRoomImageAlt {
                    room_id: room.id.clone(),
                });
            }
        }

        if self.entity_media.len() > MAX_ENTITY_MEDIA_DEFINITIONS {
            errors.push(ValidationError::TooManyEntityMedia);
        }
        let item_locations = self
            .world
            .initial_item_instances
            .iter()
            .map(|item| (item.id.as_str(), &item.location))
            .collect::<HashMap<_, _>>();
        let item_prototype_ids = self
            .world
            .item_prototypes
            .iter()
            .map(|prototype| prototype.id.as_str())
            .collect::<HashSet<_>>();
        let npc_spawn_ids = self
            .world
            .gameplay
            .npc_spawns
            .iter()
            .map(|spawn| spawn.id.as_str())
            .collect::<HashSet<_>>();
        let mut entity_media_subjects = HashSet::new();
        for media in &self.entity_media {
            let key = (media.kind, media.subject_id.as_str());
            if !entity_media_subjects.insert(key) {
                errors.push(ValidationError::DuplicateEntityMedia {
                    kind: media.kind.as_str().to_owned(),
                    subject_id: media.subject_id.clone(),
                });
                continue;
            }
            let subject_exists = match media.kind {
                EntityMediaKind::Item => item_prototype_ids.contains(media.subject_id.as_str()),
                EntityMediaKind::Npc => npc_spawn_ids.contains(media.subject_id.as_str()),
            };
            if !subject_exists {
                invalid_entity_media(
                    media,
                    "the referenced item prototype or NPC spawn does not exist",
                    &mut errors,
                );
            }
            if media.decorative {
                if !media.alt.is_empty() {
                    invalid_entity_media(
                        media,
                        "decorative sprites must have empty alt text",
                        &mut errors,
                    );
                }
            } else if let Err(reason) = media.alt.validate() {
                invalid_entity_media(
                    media,
                    &format!("non-decorative sprites require valid localized alt text: {reason}"),
                    &mut errors,
                );
            } else if media.alt.0.values().any(|text| text.len() > 256) {
                invalid_entity_media(
                    media,
                    "entity sprite alt text is limited to 256 bytes per locale",
                    &mut errors,
                );
            }
            if !(20..=600).contains(&media.footprint.width_milli)
                || !(20..=800).contains(&media.footprint.height_milli)
            {
                invalid_entity_media(
                    media,
                    "footprint must be within width 20..=600 and height 20..=800 thousandths",
                    &mut errors,
                );
            }
            if let Some(anchor) = media.preferred_anchor
                && (anchor.x_milli > 1_000 || !(200..=1_000).contains(&anchor.y_milli))
            {
                invalid_entity_media(
                    media,
                    "preferred anchor must be within x 0..=1000 and y 200..=1000 thousandths",
                    &mut errors,
                );
            }
            if !(-100..=100).contains(&media.priority) {
                invalid_entity_media(media, "priority must be within -100..=100", &mut errors);
            }
            validate_entity_sprite_source(media, "idle", &media.poses.idle, &mut errors);
            for (pose, source) in [
                ("attack", media.poses.attack.as_ref()),
                ("hit", media.poses.hit.as_ref()),
                ("defeated", media.poses.defeated.as_ref()),
            ] {
                if let Some(source) = source {
                    validate_entity_sprite_source(media, pose, source, &mut errors);
                }
            }
        }

        let npc_rooms = self
            .world
            .gameplay
            .npc_spawns
            .iter()
            .map(|spawn| (spawn.id.as_str(), spawn.room_id.as_str()))
            .collect::<HashMap<_, _>>();
        let mut perception_rooms = HashSet::new();
        for perception in &self.room_perception {
            let room = self
                .world
                .rooms
                .iter()
                .find(|room| room.id == perception.room_id);
            if room.is_none() {
                errors.push(ValidationError::MissingRoomPerceptionRoom {
                    room_id: perception.room_id.clone(),
                });
            }
            if !perception_rooms.insert(perception.room_id.as_str()) {
                errors.push(ValidationError::DuplicateRoomPerception {
                    room_id: perception.room_id.clone(),
                });
            }
            for (item_id, difficulty) in &perception.hidden_items {
                let is_top_level_in_room =
                    item_locations
                        .get(item_id.as_str())
                        .is_some_and(|location| match location {
                            InitialItemLocation::Room { room_id } => room_id == &perception.room_id,
                            InitialItemLocation::NpcSpawn { .. }
                            | InitialItemLocation::Container { .. }
                            | InitialItemLocation::Staged => false,
                        });
                if !is_top_level_in_room || !(1..=100).contains(difficulty) {
                    errors.push(ValidationError::InvalidPerception {
                        room_id: perception.room_id.clone(),
                        reason: format!(
                            "hidden item {item_id:?} must be a top-level item in this room and have difficulty 1..=100"
                        ),
                    });
                }
            }
            for (npc_id, difficulty) in &perception.hidden_npcs {
                if npc_rooms.get(npc_id.as_str()).copied() != Some(perception.room_id.as_str())
                    || !(1..=100).contains(difficulty)
                {
                    errors.push(ValidationError::InvalidPerception {
                        room_id: perception.room_id.clone(),
                        reason: format!(
                            "hidden NPC {npc_id:?} must spawn in this room and have difficulty 1..=100"
                        ),
                    });
                }
            }
            for (direction, difficulty) in &perception.secret_exits {
                let exists = room
                    .is_some_and(|room| room.exits.iter().any(|exit| exit.direction == *direction));
                if !exists || !(1..=100).contains(difficulty) {
                    errors.push(ValidationError::InvalidPerception {
                        room_id: perception.room_id.clone(),
                        reason: format!(
                            "secret exit {direction:?} must exist and have difficulty 1..=100"
                        ),
                    });
                }
            }
        }

        let mut variant_rooms = HashSet::new();
        for set in &self.scene_variants {
            if !room_ids.contains(set.room_id.as_str()) {
                errors.push(ValidationError::MissingSceneVariantRoom {
                    room_id: set.room_id.clone(),
                });
            }
            if !variant_rooms.insert(set.room_id.as_str()) {
                errors.push(ValidationError::DuplicateSceneVariantSet {
                    room_id: set.room_id.clone(),
                });
            }
            let base_has_manifest = self
                .world
                .rooms
                .iter()
                .find(|room| room.id == set.room_id)
                .is_some_and(|room| room.asset_manifest.is_some());
            validate_scene_variants(set, base_has_manifest, &mut errors);
        }

        if self.scripts.len() > 256 {
            errors.push(ValidationError::TooManyScripts);
        }
        let mut script_ids = HashSet::new();
        let mut room_hook_module_counts = BTreeMap::new();
        for script in &self.scripts {
            if script.id.trim().is_empty() {
                errors.push(ValidationError::EmptyScriptId);
            } else if !script_ids.insert(script.id.as_str()) {
                errors.push(ValidationError::DuplicateScriptId(script.id.clone()));
            }
            if !room_ids.contains(script.room_id.as_str()) {
                errors.push(ValidationError::MissingScriptRoom {
                    script_id: script.id.clone(),
                    room_id: script.room_id.clone(),
                });
            }
            for hook in script.hooks.iter().copied().collect::<BTreeSet<_>>() {
                *room_hook_module_counts
                    .entry((script.room_id.as_str(), hook))
                    .or_insert(0_usize) += 1;
            }
            validate_script(script, &mut errors);
        }
        for ((room_id, hook), actual) in room_hook_module_counts {
            if actual > MAX_MODULES_PER_ROOM_HOOK_BATCH {
                errors.push(ValidationError::TooManyScriptsForRoomHook {
                    room_id: room_id.to_owned(),
                    hook,
                    actual,
                    maximum: MAX_MODULES_PER_ROOM_HOOK_BATCH,
                });
            }
        }

        let mut test_names = HashSet::new();
        for test in &self.content_tests {
            if test.name.trim().is_empty() {
                errors.push(ValidationError::EmptyContentTestName);
            } else if !test_names.insert(test.name.as_str()) {
                errors.push(ValidationError::DuplicateContentTestName(test.name.clone()));
            }
            if !room_ids.contains(test.room_id.as_str()) {
                errors.push(ValidationError::MissingContentTestRoom {
                    test_name: test.name.clone(),
                    room_id: test.room_id.clone(),
                });
            }
            if test.expect.is_empty() {
                errors.push(ValidationError::EmptyContentTestExpectation(
                    test.name.clone(),
                ));
            }
            if let Err(reason) = test.context.validate() {
                errors.push(ValidationError::InvalidContentTestContext {
                    test_name: test.name.clone(),
                    reason,
                });
            }
        }

        if errors.is_empty() {
            Ok(())
        } else {
            Err(ValidationErrors(errors))
        }
    }

    pub fn resolve_scene(
        &self,
        room_id: &str,
        context: &ConditionContext,
    ) -> Result<ResolvedScene, SceneResolutionError> {
        self.validate()?;
        context
            .validate()
            .map_err(SceneResolutionError::InvalidContext)?;
        let room = self
            .world
            .rooms
            .iter()
            .find(|room| room.id == room_id)
            .ok_or_else(|| SceneResolutionError::UnknownRoom(room_id.to_owned()))?;

        let variant = self
            .scene_variants
            .iter()
            .find(|set| set.room_id == room_id)
            .and_then(|set| {
                set.variants
                    .iter()
                    .filter(|variant| {
                        variant
                            .when
                            .as_ref()
                            .is_some_and(|condition| condition.evaluate_validated(context))
                    })
                    .max_by_key(|variant| variant.priority)
                    .or_else(|| set.variants.iter().find(|variant| variant.when.is_none()))
            });

        let item_names: HashMap<&str, &str> = self
            .world
            .item_prototypes
            .iter()
            .map(|item| (item.id.as_str(), item.name.as_str()))
            .collect();
        Ok(ResolvedScene {
            title: variant
                .and_then(|variant| variant.title.clone())
                .unwrap_or_else(|| room.title.clone()),
            description: variant
                .and_then(|variant| variant.description.clone())
                .unwrap_or_else(|| room.description.clone()),
            asset_manifest: variant
                .and_then(|variant| variant.asset_manifest.clone())
                .or_else(|| room.asset_manifest.clone()),
            image_alt: variant
                .filter(|variant| !variant.image_alt.is_empty())
                .map(|variant| variant.image_alt.clone())
                .or_else(|| {
                    self.room_media
                        .iter()
                        .find(|media| media.room_id == room_id)
                        .map(|media| media.image_alt.clone())
                })
                .unwrap_or_default(),
            audio: variant
                .and_then(|variant| variant.audio.clone())
                .or_else(|| {
                    self.room_media
                        .iter()
                        .find(|media| media.room_id == room_id)
                        .and_then(|media| media.audio.clone())
                }),
            exits: room.exits.iter().map(|exit| exit.direction).collect(),
            items: self
                .world
                .initial_item_instances
                .iter()
                .filter(|instance| {
                    matches!(
                        &instance.location,
                        crate::InitialItemLocation::Room { room_id: item_room_id }
                            if item_room_id == room_id
                    )
                })
                .filter_map(|instance| {
                    item_names
                        .get(instance.prototype_id.as_str())
                        .map(ToString::to_string)
                })
                .collect(),
        })
    }

    pub fn render_preview(
        &self,
        room_id: &str,
        context: &ConditionContext,
        format: PreviewFormat,
    ) -> Result<String, SceneResolutionError> {
        let scene = self.resolve_scene(room_id, context)?;
        Ok(match format {
            PreviewFormat::Text => render_text_preview(&scene),
            PreviewFormat::Ansi => render_ansi_preview(&scene),
        })
    }

    pub fn run_content_tests(&self) -> Result<ContentTestReport, DocumentToolError> {
        self.validate()?;
        let mut failures = Vec::new();
        for test in &self.content_tests {
            let scene = self.resolve_scene(&test.room_id, &test.context)?;
            let mut messages = Vec::new();
            if let Some(expected) = &test.expect.title
                && scene.title != *expected
            {
                messages.push(format!(
                    "title was {:?}, expected {:?}",
                    scene.title, expected
                ));
            }
            for needle in &test.expect.description_contains {
                if !scene.description.contains(needle) {
                    messages.push(format!("description does not contain {needle:?}"));
                }
            }
            let serialized = serde_json::to_string(&scene)?;
            for needle in &test.expect.output_excludes {
                if serialized.contains(needle) {
                    messages.push(format!("authorized output leaks forbidden text {needle:?}"));
                }
            }
            if let Some(expected) = &test.expect.asset_manifest
                && scene.asset_manifest.as_ref() != Some(expected)
            {
                messages.push(format!(
                    "asset manifest was {:?}, expected {:?}",
                    scene.asset_manifest, expected
                ));
            }
            if let Some(expected) = &test.expect.exits
                && scene.exits != *expected
            {
                messages.push(format!(
                    "exits were {:?}, expected {:?}",
                    scene.exits, expected
                ));
            }
            if let Some(expected) = &test.expect.items
                && scene.items != *expected
            {
                messages.push(format!(
                    "items were {:?}, expected {:?}",
                    scene.items, expected
                ));
            }
            if !messages.is_empty() {
                failures.push(ContentTestFailure {
                    name: test.name.clone(),
                    messages,
                });
            }
        }
        Ok(ContentTestReport {
            total: self.content_tests.len(),
            passed: self.content_tests.len() - failures.len(),
            failures,
        })
    }

    pub fn deterministic_artifact(&self) -> Result<Vec<u8>, DocumentToolError> {
        self.validate()?;
        let canonical = self.canonicalized();
        let mut bytes = serde_json::to_vec_pretty(&canonical)?;
        bytes.push(b'\n');
        Ok(bytes)
    }

    pub fn diff(&self, other: &Self) -> Result<WorldPackDiff, DocumentToolError> {
        self.validate()?;
        other.validate()?;

        let metadata_changed = self.world.schema_version != other.world.schema_version
            || self.world.id != other.world.id
            || self.world.name != other.world.name
            || self.world.start_room_id != other.world.start_room_id
            || self.world.source != other.world.source
            || self.world.gameplay.rng_seed != other.world.gameplay.rng_seed
            || self.world.gameplay.default_archetype_id
                != other.world.gameplay.default_archetype_id;
        Ok(WorldPackDiff {
            metadata_changed,
            room_media: diff_entities(
                self.room_media.iter().map(|media| (&media.room_id, media)),
                other.room_media.iter().map(|media| (&media.room_id, media)),
            ),
            entity_media: diff_entities_owned(
                self.entity_media.iter().map(|media| (media.key(), media)),
                other.entity_media.iter().map(|media| (media.key(), media)),
            ),
            room_perception: diff_entities(
                self.room_perception
                    .iter()
                    .map(|perception| (&perception.room_id, perception)),
                other
                    .room_perception
                    .iter()
                    .map(|perception| (&perception.room_id, perception)),
            ),
            rooms: diff_entities(
                self.world.rooms.iter().map(|room| (&room.id, room)),
                other.world.rooms.iter().map(|room| (&room.id, room)),
            ),
            item_prototypes: diff_entities(
                self.world
                    .item_prototypes
                    .iter()
                    .map(|item| (&item.id, item)),
                other
                    .world
                    .item_prototypes
                    .iter()
                    .map(|item| (&item.id, item)),
            ),
            initial_item_instances: diff_entities(
                self.world
                    .initial_item_instances
                    .iter()
                    .map(|instance| (&instance.id, instance)),
                other
                    .world
                    .initial_item_instances
                    .iter()
                    .map(|instance| (&instance.id, instance)),
            ),
            npc_prototypes: diff_entities(
                self.world
                    .gameplay
                    .npc_prototypes
                    .iter()
                    .map(|prototype| (&prototype.id, prototype)),
                other
                    .world
                    .gameplay
                    .npc_prototypes
                    .iter()
                    .map(|prototype| (&prototype.id, prototype)),
            ),
            npc_spawns: diff_entities(
                self.world
                    .gameplay
                    .npc_spawns
                    .iter()
                    .map(|spawn| (&spawn.id, spawn)),
                other
                    .world
                    .gameplay
                    .npc_spawns
                    .iter()
                    .map(|spawn| (&spawn.id, spawn)),
            ),
            archetypes: diff_entities(
                self.world
                    .gameplay
                    .archetypes
                    .iter()
                    .map(|archetype| (&archetype.id, archetype)),
                other
                    .world
                    .gameplay
                    .archetypes
                    .iter()
                    .map(|archetype| (&archetype.id, archetype)),
            ),
            abilities: diff_entities(
                self.world
                    .gameplay
                    .abilities
                    .iter()
                    .map(|ability| (&ability.id, ability)),
                other
                    .world
                    .gameplay
                    .abilities
                    .iter()
                    .map(|ability| (&ability.id, ability)),
            ),
            npc_ability_sets: diff_entities(
                self.world
                    .gameplay
                    .npc_ability_sets
                    .iter()
                    .map(|set| (&set.npc_prototype_id, set)),
                other
                    .world
                    .gameplay
                    .npc_ability_sets
                    .iter()
                    .map(|set| (&set.npc_prototype_id, set)),
            ),
            loot_tables: diff_entities(
                self.world
                    .gameplay
                    .loot_tables
                    .iter()
                    .map(|table| (&table.id, table)),
                other
                    .world
                    .gameplay
                    .loot_tables
                    .iter()
                    .map(|table| (&table.id, table)),
            ),
            loot_sources: diff_entities(
                self.world
                    .gameplay
                    .loot_sources
                    .iter()
                    .map(|source| (&source.id, source)),
                other
                    .world
                    .gameplay
                    .loot_sources
                    .iter()
                    .map(|source| (&source.id, source)),
            ),
            affixes: diff_entities(
                self.world
                    .gameplay
                    .affixes
                    .iter()
                    .map(|affix| (&affix.id, affix)),
                other
                    .world
                    .gameplay
                    .affixes
                    .iter()
                    .map(|affix| (&affix.id, affix)),
            ),
            affix_pools: diff_entities(
                self.world
                    .gameplay
                    .affix_pools
                    .iter()
                    .map(|pool| (&pool.id, pool)),
                other
                    .world
                    .gameplay
                    .affix_pools
                    .iter()
                    .map(|pool| (&pool.id, pool)),
            ),
            scene_variants: diff_entities(
                self.scene_variants
                    .iter()
                    .map(|variants| (&variants.room_id, variants)),
                other
                    .scene_variants
                    .iter()
                    .map(|variants| (&variants.room_id, variants)),
            ),
            scripts: diff_entities(
                self.scripts.iter().map(|script| (&script.id, script)),
                other.scripts.iter().map(|script| (&script.id, script)),
            ),
            content_tests: diff_entities(
                self.content_tests.iter().map(|test| (&test.name, test)),
                other.content_tests.iter().map(|test| (&test.name, test)),
            ),
        })
    }

    fn canonicalized(&self) -> Self {
        let mut document = self.clone();
        if let Some(progression) = &mut document.world.gameplay.character_progression {
            crate::progression::canonicalize_character_progression(progression);
        }
        document
            .world
            .item_prototypes
            .sort_by(|left, right| left.id.cmp(&right.id));
        for item in &mut document.world.item_prototypes {
            item.aliases.sort();
        }
        document
            .world
            .initial_item_instances
            .sort_by(|left, right| left.id.cmp(&right.id));
        document
            .world
            .gameplay
            .npc_prototypes
            .sort_by(|left, right| left.id.cmp(&right.id));
        document
            .world
            .gameplay
            .npc_spawns
            .sort_by(|left, right| left.id.cmp(&right.id));
        document
            .world
            .gameplay
            .archetypes
            .sort_by(|left, right| left.id.cmp(&right.id));
        for archetype in &mut document.world.gameplay.archetypes {
            archetype.initial_ability_ids.sort();
            archetype
                .level_progression
                .sort_by_key(|progression| progression.level);
            for progression in &mut archetype.level_progression {
                progression.ability_unlock_ids.sort();
            }
        }
        document
            .world
            .gameplay
            .abilities
            .sort_by(|left, right| left.id.cmp(&right.id));
        for ability in &mut document.world.gameplay.abilities {
            ability.status_effects.sort();
        }
        document
            .world
            .gameplay
            .npc_ability_sets
            .sort_by(|left, right| left.npc_prototype_id.cmp(&right.npc_prototype_id));
        for set in &mut document.world.gameplay.npc_ability_sets {
            set.ability_ids.sort();
        }
        document
            .world
            .gameplay
            .loot_tables
            .sort_by(|left, right| left.id.cmp(&right.id));
        for table in &mut document.world.gameplay.loot_tables {
            table
                .guaranteed
                .sort_by(|left, right| left.id.cmp(&right.id));
            table.pools.sort_by(|left, right| left.id.cmp(&right.id));
            for pool in &mut table.pools {
                pool.entries.sort_by(|left, right| left.id.cmp(&right.id));
            }
        }
        document
            .world
            .gameplay
            .loot_sources
            .sort_by(|left, right| left.id.cmp(&right.id));
        document
            .world
            .gameplay
            .affixes
            .sort_by(|left, right| left.id.cmp(&right.id));
        document
            .world
            .gameplay
            .affix_pools
            .sort_by(|left, right| left.id.cmp(&right.id));
        for pool in &mut document.world.gameplay.affix_pools {
            pool.entries.sort_by(|left, right| left.id.cmp(&right.id));
        }
        document
            .world
            .rooms
            .sort_by(|left, right| left.id.cmp(&right.id));
        for room in &mut document.world.rooms {
            room.exits
                .sort_by_key(|exit| direction_rank(exit.direction));
        }
        document
            .room_media
            .sort_by(|left, right| left.room_id.cmp(&right.room_id));
        document.entity_media.sort_by(|left, right| {
            left.kind
                .cmp(&right.kind)
                .then_with(|| left.subject_id.cmp(&right.subject_id))
        });
        document
            .room_perception
            .sort_by(|left, right| left.room_id.cmp(&right.room_id));
        document
            .scene_variants
            .sort_by(|left, right| left.room_id.cmp(&right.room_id));
        for set in &mut document.scene_variants {
            for variant in &mut set.variants {
                if let Some(condition) = &mut variant.when {
                    condition.canonicalize();
                }
            }
            set.variants.sort_by(|left, right| {
                right
                    .priority
                    .cmp(&left.priority)
                    .then_with(|| left.id.cmp(&right.id))
            });
        }
        document
            .scripts
            .sort_by(|left, right| left.id.cmp(&right.id));
        for script in &mut document.scripts {
            script.hooks.sort();
            script.capabilities.sort();
        }
        document
            .content_tests
            .sort_by(|left, right| left.name.cmp(&right.name));
        for test in &mut document.content_tests {
            test.expect.canonicalize();
        }
        document
    }
}

fn invalid_entity_media(
    media: &EntityMediaDefinition,
    reason: &str,
    errors: &mut Vec<ValidationError>,
) {
    errors.push(ValidationError::InvalidEntityMedia {
        kind: media.kind.as_str().to_owned(),
        subject_id: media.subject_id.clone(),
        reason: reason.to_owned(),
    });
}

fn validate_entity_sprite_source(
    media: &EntityMediaDefinition,
    pose: &str,
    source: &EntitySpriteSource,
    errors: &mut Vec<ValidationError>,
) {
    if source.webp.is_none() && source.png.is_none() {
        invalid_entity_media(media, &format!("{pose} pose has no raster source"), errors);
        return;
    }
    for (format, path) in [("webp", &source.webp), ("png", &source.png)] {
        if let Some(path) = path
            && !valid_entity_sprite_path(path, pose, format)
        {
            invalid_entity_media(
                media,
                &format!(
                    "{pose} {format} path must be assets/<lowercase-sha256>/sprite-{pose}-{{320|640}}.{format}"
                ),
                errors,
            );
        }
    }
}

fn valid_entity_sprite_path(path: &str, pose: &str, format: &str) -> bool {
    let mut components = path.split('/');
    let (Some("assets"), Some(hash), Some(file), None) = (
        components.next(),
        components.next(),
        components.next(),
        components.next(),
    ) else {
        return false;
    };
    hash.len() == 64
        && hash
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
        && [320, 640]
            .iter()
            .any(|size| file == format!("sprite-{pose}-{size}.{format}"))
}

fn validate_script(script: &ScriptDefinition, errors: &mut Vec<ValidationError>) {
    let mut can_compile = true;
    if script.hooks.is_empty() {
        errors.push(ValidationError::EmptyScriptHooks(script.id.clone()));
        can_compile = false;
    }
    let mut hooks = BTreeSet::new();
    for hook in &script.hooks {
        if !hooks.insert(*hook) {
            errors.push(ValidationError::DuplicateScriptHook {
                script_id: script.id.clone(),
                hook: *hook,
            });
            can_compile = false;
        }
    }
    let mut capabilities = BTreeSet::new();
    for capability in &script.capabilities {
        if !capabilities.insert(*capability) {
            errors.push(ValidationError::DuplicateScriptCapability {
                script_id: script.id.clone(),
                capability: *capability,
            });
        }
    }
    if script.source.trim().is_empty() {
        errors.push(ValidationError::EmptyScriptSource(script.id.clone()));
        can_compile = false;
    }
    if script.source.len() > script.limits.max_source_bytes {
        errors.push(ValidationError::ScriptSourceTooLarge {
            script_id: script.id.clone(),
            actual: script.source.len(),
            maximum: script.limits.max_source_bytes,
        });
        can_compile = false;
    }
    let limits = script.limits;
    let invalid_limits = limits.max_source_bytes == 0
        || limits.max_source_bytes > default_script_source_bytes()
        || limits.max_memory_bytes < 64 * 1024
        || limits.max_memory_bytes > 8 * 1024 * 1024
        || limits.max_instructions == 0
        || limits.max_instructions > 1_000_000
        || limits.max_wall_time_millis == 0
        || limits.max_wall_time_millis > 100
        || limits.max_events == 0
        || limits.max_events > 64
        || limits.max_event_field_bytes == 0
        || limits.max_event_field_bytes > 2_048;
    if invalid_limits {
        errors.push(ValidationError::InvalidScriptLimits(script.id.clone()));
        can_compile = false;
    }
    if can_compile {
        let declared_hooks = script.hooks.iter().copied().collect::<BTreeSet<_>>();
        let runtime_limits = ScriptLimits {
            max_source_bytes: limits.max_source_bytes,
            max_memory_bytes: limits.max_memory_bytes,
            max_instructions: limits.max_instructions,
            max_wall_time: Duration::from_millis(limits.max_wall_time_millis),
            max_events: limits.max_events,
            max_event_field_bytes: limits.max_event_field_bytes,
        };
        if let Err(error) = validate_module(&script.source, &declared_hooks, runtime_limits) {
            errors.push(ValidationError::InvalidScriptModule {
                script_id: script.id.clone(),
                reason: error.to_string(),
            });
        }
    }
}

fn validate_scene_variants(
    set: &RoomSceneVariants,
    base_has_manifest: bool,
    errors: &mut Vec<ValidationError>,
) {
    let mut variant_ids = HashSet::new();
    let mut priorities: HashMap<i32, &str> = HashMap::new();
    let mut default: Option<&str> = None;
    for variant in &set.variants {
        if variant.id.trim().is_empty() {
            errors.push(ValidationError::EmptySceneVariantId {
                room_id: set.room_id.clone(),
            });
        } else if !variant_ids.insert(variant.id.as_str()) {
            errors.push(ValidationError::DuplicateSceneVariantId {
                room_id: set.room_id.clone(),
                variant_id: variant.id.clone(),
            });
        }
        if let Some(first_variant) = priorities.insert(variant.priority, variant.id.as_str()) {
            errors.push(ValidationError::DuplicateSceneVariantPriority {
                room_id: set.room_id.clone(),
                priority: variant.priority,
                first_variant: first_variant.to_owned(),
                second_variant: variant.id.clone(),
            });
        }
        if variant.when.is_none()
            && let Some(first_variant) = default.replace(variant.id.as_str())
        {
            errors.push(ValidationError::DuplicateDefaultSceneVariant {
                room_id: set.room_id.clone(),
                first_variant: first_variant.to_owned(),
                second_variant: variant.id.clone(),
            });
        }
        if variant.title.is_none()
            && variant.description.is_none()
            && variant.asset_manifest.is_none()
            && variant.image_alt.is_empty()
            && variant.audio.is_none()
        {
            errors.push(ValidationError::EmptySceneVariant {
                room_id: set.room_id.clone(),
                variant_id: variant.id.clone(),
            });
        }
        if let Some(title) = &variant.title {
            if title.trim().is_empty() {
                errors.push(ValidationError::EmptySceneVariantTitle {
                    room_id: set.room_id.clone(),
                    variant_id: variant.id.clone(),
                });
            } else if !safe_gameplay_label(title, MAX_ROOM_TITLE_BYTES) {
                errors.push(ValidationError::InvalidGameplay(format!(
                    "room {:?} scene variant {:?} title must contain 1..={MAX_ROOM_TITLE_BYTES} safe bytes without control characters",
                    set.room_id, variant.id
                )));
            }
        }
        if let Some(description) = &variant.description {
            if description.trim().is_empty() {
                errors.push(ValidationError::EmptySceneVariantDescription {
                    room_id: set.room_id.clone(),
                    variant_id: variant.id.clone(),
                });
            } else if !safe_gameplay_text(description, MAX_ROOM_DESCRIPTION_BYTES) {
                errors.push(ValidationError::InvalidGameplay(format!(
                    "room {:?} scene variant {:?} description must contain 1..={MAX_ROOM_DESCRIPTION_BYTES} safe bytes",
                    set.room_id, variant.id
                )));
            }
        }
        if let Some(path) = &variant.asset_manifest
            && !valid_relative_asset_path(path)
        {
            errors.push(ValidationError::InvalidSceneVariantAssetManifestPath {
                room_id: set.room_id.clone(),
                variant_id: variant.id.clone(),
                path: path.clone(),
            });
        }
        if !variant.image_alt.is_empty()
            && let Err(reason) = variant.image_alt.validate()
        {
            errors.push(ValidationError::InvalidSceneVariantImageAlt {
                room_id: set.room_id.clone(),
                variant_id: variant.id.clone(),
                reason,
            });
        }
        if variant.asset_manifest.is_some() && variant.image_alt.is_empty() {
            errors.push(ValidationError::MissingSceneVariantImageAlt {
                room_id: set.room_id.clone(),
                variant_id: variant.id.clone(),
            });
        } else if !variant.image_alt.is_empty()
            && variant.asset_manifest.is_none()
            && !base_has_manifest
        {
            errors.push(ValidationError::SceneVariantImageAltWithoutManifest {
                room_id: set.room_id.clone(),
                variant_id: variant.id.clone(),
            });
        }
        if let Some(audio) = &variant.audio
            && let Err(reason) = audio.validate()
        {
            errors.push(ValidationError::InvalidSceneVariantAudio {
                room_id: set.room_id.clone(),
                variant_id: variant.id.clone(),
                reason,
            });
        }
        if let Some(condition) = &variant.when
            && let Err(condition_errors) = condition.validate()
        {
            errors.extend(condition_errors.0.into_iter().map(|error| {
                ValidationError::InvalidSceneVariantCondition {
                    room_id: set.room_id.clone(),
                    variant_id: variant.id.clone(),
                    reason: error.to_string(),
                }
            }));
        }
    }
    if !set.variants.is_empty() && default.is_none() {
        errors.push(ValidationError::MissingDefaultSceneVariant {
            room_id: set.room_id.clone(),
        });
    }
}

fn render_text_preview(scene: &ResolvedScene) -> String {
    let title = sanitize_terminal_text(&scene.title);
    let description = sanitize_terminal_text(&scene.description);
    let exits = join_directions(&scene.exits);
    let items = if scene.items.is_empty() {
        "none".to_owned()
    } else {
        scene
            .items
            .iter()
            .map(|item| sanitize_terminal_text(item))
            .collect::<Vec<_>>()
            .join(", ")
    };
    let illustration = if scene.asset_manifest.is_some() {
        "\nIllustration: atmospheric asset available"
    } else {
        ""
    };
    format!(
        "{title}\n{}\n\n{description}\n\nExits: {exits}\nItems: {items}{illustration}\n",
        "=".repeat(title.chars().count().min(72))
    )
}

fn render_ansi_preview(scene: &ResolvedScene) -> String {
    let title = sanitize_terminal_text(&scene.title);
    let description = sanitize_terminal_text(&scene.description);
    let exits = join_directions(&scene.exits);
    let items = if scene.items.is_empty() {
        "none".to_owned()
    } else {
        scene
            .items
            .iter()
            .map(|item| sanitize_terminal_text(item))
            .collect::<Vec<_>>()
            .join(", ")
    };
    let illustration = if scene.asset_manifest.is_some() {
        "\n\x1b[2mIllustration: atmospheric asset available\x1b[0m"
    } else {
        ""
    };
    format!(
        "\x1b[1;36m{title}\x1b[0m\n\n{description}\n\n\x1b[1;33mExits:\x1b[0m {exits}\n\x1b[1;32mItems:\x1b[0m {items}{illustration}\n"
    )
}

fn sanitize_terminal_text(value: &str) -> String {
    value
        .chars()
        .filter(|character| {
            *character == '\n'
                || *character == '\t'
                || (!character.is_control() && *character != '\u{1b}')
        })
        .collect()
}

fn join_directions(directions: &[Direction]) -> String {
    if directions.is_empty() {
        return "none".to_owned();
    }
    directions
        .iter()
        .map(|direction| match direction {
            Direction::North => "north",
            Direction::East => "east",
            Direction::South => "south",
            Direction::West => "west",
            Direction::Up => "up",
            Direction::Down => "down",
        })
        .collect::<Vec<_>>()
        .join(", ")
}

fn direction_rank(direction: Direction) -> u8 {
    match direction {
        Direction::North => 0,
        Direction::East => 1,
        Direction::South => 2,
        Direction::West => 3,
        Direction::Up => 4,
        Direction::Down => 5,
    }
}

fn condition_sort_key(condition: &Condition) -> String {
    serde_json::to_string(condition).unwrap_or_default()
}

fn condition_value_sort_key(value: &ConditionValue) -> String {
    serde_json::to_string(value).unwrap_or_default()
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct EntityDiff {
    pub added: Vec<String>,
    pub removed: Vec<String>,
    pub changed: Vec<String>,
}

impl EntityDiff {
    pub fn is_empty(&self) -> bool {
        self.added.is_empty() && self.removed.is_empty() && self.changed.is_empty()
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct WorldPackDiff {
    pub metadata_changed: bool,
    pub room_media: EntityDiff,
    pub entity_media: EntityDiff,
    pub room_perception: EntityDiff,
    pub rooms: EntityDiff,
    pub item_prototypes: EntityDiff,
    pub initial_item_instances: EntityDiff,
    pub npc_prototypes: EntityDiff,
    pub npc_spawns: EntityDiff,
    pub archetypes: EntityDiff,
    pub abilities: EntityDiff,
    pub npc_ability_sets: EntityDiff,
    pub loot_tables: EntityDiff,
    pub loot_sources: EntityDiff,
    pub affixes: EntityDiff,
    pub affix_pools: EntityDiff,
    pub scene_variants: EntityDiff,
    pub scripts: EntityDiff,
    pub content_tests: EntityDiff,
}

impl WorldPackDiff {
    pub fn is_empty(&self) -> bool {
        !self.metadata_changed
            && self.room_media.is_empty()
            && self.entity_media.is_empty()
            && self.room_perception.is_empty()
            && self.rooms.is_empty()
            && self.item_prototypes.is_empty()
            && self.initial_item_instances.is_empty()
            && self.npc_prototypes.is_empty()
            && self.npc_spawns.is_empty()
            && self.archetypes.is_empty()
            && self.abilities.is_empty()
            && self.npc_ability_sets.is_empty()
            && self.loot_tables.is_empty()
            && self.loot_sources.is_empty()
            && self.affixes.is_empty()
            && self.affix_pools.is_empty()
            && self.scene_variants.is_empty()
            && self.scripts.is_empty()
            && self.content_tests.is_empty()
    }
}

impl fmt::Display for WorldPackDiff {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.is_empty() {
            return formatter.write_str("no differences\n");
        }
        if self.metadata_changed {
            writeln!(formatter, "metadata: changed")?;
        }
        write_entity_diff(formatter, "room media", &self.room_media)?;
        write_entity_diff(formatter, "entity media", &self.entity_media)?;
        write_entity_diff(formatter, "room perception", &self.room_perception)?;
        write_entity_diff(formatter, "rooms", &self.rooms)?;
        write_entity_diff(formatter, "item prototypes", &self.item_prototypes)?;
        write_entity_diff(
            formatter,
            "initial item instances",
            &self.initial_item_instances,
        )?;
        write_entity_diff(formatter, "NPC prototypes", &self.npc_prototypes)?;
        write_entity_diff(formatter, "NPC spawns", &self.npc_spawns)?;
        write_entity_diff(formatter, "archetypes", &self.archetypes)?;
        write_entity_diff(formatter, "abilities", &self.abilities)?;
        write_entity_diff(formatter, "NPC ability sets", &self.npc_ability_sets)?;
        write_entity_diff(formatter, "loot tables", &self.loot_tables)?;
        write_entity_diff(formatter, "loot sources", &self.loot_sources)?;
        write_entity_diff(formatter, "affixes", &self.affixes)?;
        write_entity_diff(formatter, "affix pools", &self.affix_pools)?;
        write_entity_diff(formatter, "scene variants", &self.scene_variants)?;
        write_entity_diff(formatter, "scripts", &self.scripts)?;
        write_entity_diff(formatter, "content tests", &self.content_tests)
    }
}

fn write_entity_diff(
    formatter: &mut fmt::Formatter<'_>,
    label: &str,
    diff: &EntityDiff,
) -> fmt::Result {
    for id in &diff.added {
        writeln!(formatter, "{label}: + {id}")?;
    }
    for id in &diff.removed {
        writeln!(formatter, "{label}: - {id}")?;
    }
    for id in &diff.changed {
        writeln!(formatter, "{label}: ~ {id}")?;
    }
    Ok(())
}

fn diff_entities<'a, T: PartialEq + 'a>(
    before: impl Iterator<Item = (&'a String, &'a T)>,
    after: impl Iterator<Item = (&'a String, &'a T)>,
) -> EntityDiff {
    let before: BTreeMap<&str, &T> = before.map(|(id, value)| (id.as_str(), value)).collect();
    let after: BTreeMap<&str, &T> = after.map(|(id, value)| (id.as_str(), value)).collect();
    let mut diff = EntityDiff::default();
    for (id, value) in &before {
        match after.get(id) {
            None => diff.removed.push((*id).to_owned()),
            Some(other) if *value != *other => diff.changed.push((*id).to_owned()),
            Some(_) => {}
        }
    }
    for id in after.keys() {
        if !before.contains_key(id) {
            diff.added.push((*id).to_owned());
        }
    }
    diff
}

fn diff_entities_owned<'a, T: PartialEq + 'a>(
    before: impl Iterator<Item = (String, &'a T)>,
    after: impl Iterator<Item = (String, &'a T)>,
) -> EntityDiff {
    let before: BTreeMap<String, &T> = before.collect();
    let after: BTreeMap<String, &T> = after.collect();
    let mut diff = EntityDiff::default();
    for (id, value) in &before {
        match after.get(id) {
            None => diff.removed.push(id.clone()),
            Some(other) if *value != *other => diff.changed.push(id.clone()),
            Some(_) => {}
        }
    }
    for id in after.keys() {
        if !before.contains_key(id) {
            diff.added.push(id.clone());
        }
    }
    diff
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ContentTestFailure {
    pub name: String,
    pub messages: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ContentTestReport {
    pub total: usize,
    pub passed: usize,
    pub failures: Vec<ContentTestFailure>,
}

impl ContentTestReport {
    pub fn is_success(&self) -> bool {
        self.failures.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        CURRENT_SCHEMA_VERSION, Exit, ExitTarget, InitialItemInstance, InitialItemLocation,
        ItemPrototype, ItemRarity, ItemStackPolicy, ItemTransferPolicy, Room,
    };

    fn room(id: &str, title: &str) -> Room {
        Room {
            id: id.to_owned(),
            legacy_vnum: None,
            title: title.to_owned(),
            description: format!("A complete description of {title}."),
            asset_manifest: None,
            exits: Vec::new(),
            legacy: None,
        }
    }

    fn document() -> WorldPackDocument {
        let mut square = room("demo:square", "Lantern Square");
        square.exits.push(Exit {
            direction: Direction::North,
            target: ExitTarget::Room {
                room_id: "demo:gate".into(),
            },
            description: String::new(),
            keywords: String::new(),
            legacy_values: Vec::new(),
        });
        let mut gate = room("demo:gate", "North Gate");
        gate.exits.push(Exit {
            direction: Direction::South,
            target: ExitTarget::Room {
                room_id: "demo:square".into(),
            },
            description: String::new(),
            keywords: String::new(),
            legacy_values: Vec::new(),
        });
        WorldPackDocument {
            world: WorldPack {
                schema_version: CURRENT_SCHEMA_VERSION,
                id: "demo".into(),
                name: "Demo".into(),
                start_room_id: "demo:square".into(),
                source: None,
                item_prototypes: vec![ItemPrototype {
                    id: "demo:lantern".into(),
                    name: "blue lantern".into(),
                    description: "A cold blue lantern.".into(),
                    aliases: vec!["lamp".into(), "lantern".into()],
                    equipment_slot: None,
                    attack_bonus: 0,
                    heal_amount: 0,
                    consumable: false,
                    effect: None,
                    weight_grams: 500,
                    slot_cost: 1,
                    rarity: ItemRarity::Common,
                    item_level: 1,
                    stack_policy: ItemStackPolicy::Unique,
                    max_stack_units: 1,
                    container: None,
                    transfer_policy: ItemTransferPolicy::Transferable,
                    sealed_loot_table_id: None,
                }],
                initial_item_instances: vec![InitialItemInstance {
                    id: "demo:lantern".into(),
                    prototype_id: "demo:lantern".into(),
                    location: InitialItemLocation::Room {
                        room_id: "demo:square".into(),
                    },
                }],
                rooms: vec![square, gate],
                gameplay: crate::GameplayDefinition::default(),
            },
            room_media: Vec::new(),
            entity_media: Vec::new(),
            room_perception: Vec::new(),
            scene_variants: Vec::new(),
            scripts: Vec::new(),
            content_tests: Vec::new(),
        }
    }

    fn night_rain_condition() -> Condition {
        Condition::All(vec![
            Condition::Eq(
                ConditionField::WorldTimePeriod,
                ConditionValue::String("night".into()),
            ),
            Condition::In(
                ConditionField::WorldWeather,
                vec![
                    ConditionValue::String("rain".into()),
                    ConditionValue::String("storm".into()),
                ],
            ),
        ])
    }

    fn add_repeatable_loot(document: &mut WorldPackDocument) {
        document
            .world
            .gameplay
            .npc_prototypes
            .push(crate::NpcPrototype {
                id: "demo:wolf".into(),
                name: "wolf".into(),
                description: "A test wolf.".into(),
                level: 1,
                max_health: 10,
                strength: 2,
                defence: 0,
                experience_reward: 10,
                respawn_ticks: 2,
            });
        document.world.gameplay.npc_spawns.push(crate::NpcSpawn {
            id: "demo:wolf-1".into(),
            prototype_id: "demo:wolf".into(),
            room_id: "demo:gate".into(),
        });
        document
            .world
            .gameplay
            .loot_tables
            .push(crate::LootTableDefinition {
                id: "demo:wolf-drops".into(),
                guaranteed: Vec::new(),
                pools: vec![crate::LootPoolDefinition {
                    id: "demo:trophy".into(),
                    rolls: 1,
                    entries: vec![
                        crate::LootEntryDefinition {
                            kind: crate::LootEntryKind::Nothing,
                            id: "demo:nothing".into(),
                            weight: 4_000,
                            item_prototype_id: None,
                            min_quantity: None,
                            max_quantity: None,
                            affix_pool_id: None,
                        },
                        crate::LootEntryDefinition {
                            kind: crate::LootEntryKind::Item,
                            id: "demo:lantern-drop".into(),
                            weight: 6_000,
                            item_prototype_id: Some("demo:lantern".into()),
                            min_quantity: Some(1),
                            max_quantity: Some(1),
                            affix_pool_id: None,
                        },
                    ],
                }],
            });
        document
            .world
            .gameplay
            .loot_sources
            .push(crate::LootSourceDefinition {
                kind: crate::LootSourceKind::NpcSpawn,
                id: "demo:wolf-source".into(),
                npc_spawn_id: "demo:wolf-1".into(),
                loot_table_id: "demo:wolf-drops".into(),
                rights: crate::LootRightsPolicy::PartyRoundRobin,
            });
    }

    #[test]
    fn perception_policy_is_cross_referenced_and_bounded() {
        let mut document = document();
        document.room_perception.push(RoomPerceptionDefinition {
            room_id: "demo:square".into(),
            light: LightLevel::Dim,
            hidden_items: BTreeMap::from([("demo:lantern".into(), 10)]),
            hidden_npcs: BTreeMap::new(),
            secret_exits: BTreeMap::from([(Direction::North, 20)]),
        });
        document.validate().unwrap();

        document.room_perception[0]
            .secret_exits
            .insert(Direction::Down, 0);
        assert!(document.validate().unwrap_err().0.iter().any(|error| {
            matches!(error, ValidationError::InvalidPerception { room_id, .. } if room_id == "demo:square")
        }));
    }

    #[test]
    fn hidden_items_must_be_top_level_instances_in_the_declared_room() {
        let mut document = document();
        document.room_perception.push(RoomPerceptionDefinition {
            room_id: "demo:square".into(),
            light: LightLevel::Bright,
            hidden_items: BTreeMap::from([("demo:lantern".into(), 10)]),
            hidden_npcs: BTreeMap::new(),
            secret_exits: BTreeMap::new(),
        });

        let mut bag = document.world.item_prototypes[0].clone();
        bag.id = "demo:bag".into();
        bag.name = "canvas bag".into();
        bag.description = "A canvas bag.".into();
        bag.aliases = vec!["bag".into()];
        bag.container = Some(crate::ItemContainerDefinition {
            max_slots: 4,
            max_weight_grams: 2_000,
        });
        document.world.item_prototypes.push(bag);
        document
            .world
            .initial_item_instances
            .push(InitialItemInstance {
                id: "demo:bag".into(),
                prototype_id: "demo:bag".into(),
                location: InitialItemLocation::Room {
                    room_id: "demo:square".into(),
                },
            });
        document
            .world
            .gameplay
            .npc_prototypes
            .push(crate::NpcPrototype {
                id: "demo:keeper".into(),
                name: "keeper".into(),
                description: "A quiet keeper.".into(),
                level: 1,
                max_health: 10,
                strength: 1,
                defence: 0,
                experience_reward: 0,
                respawn_ticks: 1,
            });
        document.world.gameplay.npc_spawns.push(crate::NpcSpawn {
            id: "demo:keeper-1".into(),
            prototype_id: "demo:keeper".into(),
            room_id: "demo:square".into(),
        });
        document.validate().unwrap();

        for location in [
            InitialItemLocation::Room {
                room_id: "demo:gate".into(),
            },
            InitialItemLocation::Staged,
            InitialItemLocation::NpcSpawn {
                npc_spawn_id: "demo:keeper-1".into(),
            },
            InitialItemLocation::Container {
                container_instance_id: "demo:bag".into(),
            },
        ] {
            let mut candidate = document.clone();
            candidate.world.initial_item_instances[0].location = location;
            assert!(candidate.validate().unwrap_err().0.iter().any(|error| {
                matches!(error, ValidationError::InvalidPerception { room_id, reason }
                    if room_id == "demo:square" && reason.contains("top-level item"))
            }));
        }
    }

    fn add_variants(document: &mut WorldPackDocument) {
        document.scene_variants.push(RoomSceneVariants {
            room_id: "demo:square".into(),
            variants: vec![
                SceneVariant {
                    id: "author-only-night-rain".into(),
                    priority: 20,
                    when: Some(night_rain_condition()),
                    title: Some("Rain-dark Lantern Square".into()),
                    description: Some("Rain needles the square beneath a moonless sky.".into()),
                    asset_manifest: Some("assets/night-rain/manifest.json".into()),
                    image_alt: LocalizedText(BTreeMap::from([
                        (
                            "en".into(),
                            "Rain-dark Lantern Square beneath the clock.".into(),
                        ),
                        (
                            "es".into(),
                            "La Plaza del Farol oscura bajo el reloj.".into(),
                        ),
                    ])),
                    audio: None,
                },
                SceneVariant {
                    id: "author-only-default".into(),
                    priority: 0,
                    when: None,
                    title: Some("Lantern Square".into()),
                    description: None,
                    asset_manifest: None,
                    image_alt: LocalizedText::default(),
                    audio: None,
                },
            ],
        });
    }

    #[test]
    fn localized_alt_falls_back_deterministically_and_audio_is_content_addressed() {
        let mut document = document();
        document.world.rooms[0].asset_manifest =
            Some(format!("assets/{}/manifest.json", "a".repeat(64)));
        document.room_media.push(RoomMediaDefinition {
            room_id: "demo:square".into(),
            image_alt: LocalizedText(BTreeMap::from([
                ("en".into(), "Lantern Square after rain.".into()),
                ("es".into(), "La Plaza del Farol tras la lluvia.".into()),
            ])),
            audio: Some(AudioCue {
                channel: AudioChannel::Ambience,
                path: format!("assets/{}/audio-rain.opus", "b".repeat(64)),
                looped: true,
            }),
        });

        document.validate().unwrap();
        let alt = &document.room_media[0].image_alt;
        assert_eq!(
            alt.resolve("es-MX"),
            Some("La Plaza del Farol tras la lluvia.")
        );
        assert_eq!(alt.resolve("fr"), Some("Lantern Square after rain."));
        let resolved = document
            .resolve_scene("demo:square", &ConditionContext::default())
            .unwrap();
        assert_eq!(
            resolved.image_alt.resolve("fr"),
            Some("Lantern Square after rain.")
        );
        assert_eq!(resolved.audio.unwrap().channel, AudioChannel::Ambience);
    }

    #[test]
    fn manifests_require_alt_and_audio_paths_fail_closed() {
        let mut missing_alt = document();
        missing_alt.world.rooms[0].asset_manifest =
            Some(format!("assets/{}/manifest.json", "a".repeat(64)));
        assert!(missing_alt.validate().unwrap_err().0.contains(
            &ValidationError::MissingRoomImageAlt {
                room_id: "demo:square".into(),
            }
        ));

        let mut invalid_audio = document();
        invalid_audio.room_media.push(RoomMediaDefinition {
            room_id: "demo:square".into(),
            image_alt: LocalizedText::default(),
            audio: Some(AudioCue {
                channel: AudioChannel::Effects,
                path: "https://tracker.invalid/audio.mp3".into(),
                looped: false,
            }),
        });
        assert!(
            invalid_audio
                .validate()
                .unwrap_err()
                .0
                .iter()
                .any(|error| { matches!(error, ValidationError::InvalidRoomAudio { .. }) })
        );
    }

    fn night_rain_context() -> ConditionContext {
        ConditionContext {
            world_time_period: TimePeriod::Night,
            world_weather: Weather::Rain,
            world_time_hour: 23,
            ..ConditionContext::default()
        }
    }

    #[test]
    fn condition_json_matches_the_documented_ast_and_is_typed() {
        let condition: Condition = serde_json::from_str(
            r#"{"all":[{"eq":["world.time.period","night"]},{"in":["world.weather",["rain","storm"]]}]}"#,
        )
        .unwrap();
        assert_eq!(condition, night_rain_condition());
        condition.validate().unwrap();
        assert!(condition.evaluate(&night_rain_context()).unwrap());

        let wrong_type = Condition::Eq(
            ConditionField::ActorLevel,
            ConditionValue::String("experienced".into()),
        );
        assert!(matches!(
            wrong_type.validate().unwrap_err().0.as_slice(),
            [ConditionValidationError::TypeMismatch { .. }]
        ));
    }

    #[test]
    fn extended_condition_context_evaluates_runtime_identity_state_preferences_and_senses() {
        let context = ConditionContext {
            world_instance: "primary".into(),
            actor_quest_states: BTreeSet::from(["pilgrimage:ready_to_claim".into()]),
            actor_content_preferences: BTreeSet::from(["reduced_motion".into()]),
            actor_senses: BTreeSet::from(["hearing".into(), "tremorsense".into()]),
            ..ConditionContext::default()
        };
        let condition = Condition::All(vec![
            Condition::Eq(
                ConditionField::WorldInstance,
                ConditionValue::String("primary".into()),
            ),
            Condition::Contains(
                ConditionField::ActorQuestStates,
                "pilgrimage:ready_to_claim".into(),
            ),
            Condition::Contains(
                ConditionField::ActorContentPreferences,
                "reduced_motion".into(),
            ),
            Condition::Contains(ConditionField::ActorSenses, "tremorsense".into()),
        ]);

        condition.validate().unwrap();
        assert!(condition.evaluate(&context).unwrap());
        assert!(
            !Condition::Contains(ConditionField::ActorSenses, "darkvision".into())
                .evaluate(&context)
                .unwrap()
        );
    }

    #[test]
    fn condition_validation_rejects_impossible_and_unbounded_shapes() {
        let contradiction = Condition::All(vec![
            Condition::Eq(
                ConditionField::WorldWeather,
                ConditionValue::String("rain".into()),
            ),
            Condition::Eq(
                ConditionField::WorldWeather,
                ConditionValue::String("clear".into()),
            ),
        ]);
        assert!(contradiction.validate().unwrap_err().0.contains(
            &ConditionValidationError::ContradictoryEquality {
                field: ConditionField::WorldWeather,
            }
        ));

        let invalid_hour =
            Condition::Eq(ConditionField::WorldTimeHour, ConditionValue::Integer(24));
        assert!(matches!(
            invalid_hour.validate().unwrap_err().0.as_slice(),
            [ConditionValidationError::IntegerOutOfRange { .. }]
        ));

        let mut too_deep =
            Condition::Eq(ConditionField::ActorIsAdmin, ConditionValue::Boolean(true));
        for _ in 0..=MAX_CONDITION_DEPTH {
            too_deep = Condition::Not(Box::new(too_deep));
        }
        assert!(
            too_deep
                .validate()
                .unwrap_err()
                .0
                .iter()
                .any(|error| matches!(error, ConditionValidationError::TooDeep { .. }))
        );
    }

    #[test]
    fn current_world_pack_json_is_compatible_with_the_extended_document() {
        let plain = serde_json::to_vec(&document().world).unwrap();
        let decoded = WorldPackDocument::from_json(&plain).unwrap();
        assert!(decoded.scene_variants.is_empty());
        assert!(decoded.content_tests.is_empty());
        decoded.validate().unwrap();

        let artifact = decoded.deterministic_artifact().unwrap();
        let runtime_reader: WorldPack = serde_json::from_slice(&artifact).unwrap();
        runtime_reader.validate().unwrap();
    }

    #[test]
    fn v3_items_normalize_to_typed_v7_identity_without_duplicate_authorities() {
        let legacy = serde_json::json!({
            "schemaVersion": 3,
            "id": "legacy",
            "name": "Legacy",
            "startRoomId": "legacy:start",
            "items": [{
                "id": "legacy:blade",
                "name": "blade",
                "description": "A legacy blade.",
                "aliases": ["sword"]
            }],
            "rooms": [{
                "id": "legacy:start",
                "title": "Start",
                "description": "A starting room.",
                "items": ["legacy:blade"]
            }],
            "gameplay": {
                "itemRules": [{
                    "itemId": "legacy:blade",
                    "equipmentSlot": "weapon",
                    "attackBonus": 4
                }]
            }
        });
        let document = WorldPackDocument::from_json(&serde_json::to_vec(&legacy).unwrap()).unwrap();
        document.validate().unwrap();
        assert_eq!(document.world.schema_version, crate::CURRENT_SCHEMA_VERSION);
        assert_eq!(document.world.item_prototypes[0].id, "legacy:blade");
        assert_eq!(document.world.initial_item_instances[0].id, "legacy:blade");
        assert_eq!(
            document.world.initial_item_instances[0].prototype_id,
            "legacy:blade"
        );
        assert_eq!(document.world.item_prototypes[0].attack_bonus, 4);
        assert_eq!(document.world.item_prototypes[0].max_stack_units, 1);
        assert!(document.world.item_prototypes[0].container.is_none());

        let artifact = document.deterministic_artifact().unwrap();
        let artifact: serde_json::Value = serde_json::from_slice(&artifact).unwrap();
        assert!(artifact.get("items").is_none());
        assert!(artifact["rooms"][0].get("items").is_none());
        assert!(artifact["gameplay"].get("itemRules").is_none());

        for schema_version in [2, 3] {
            let mut smuggled_item = legacy.clone();
            smuggled_item["schemaVersion"] = serde_json::json!(schema_version);
            smuggled_item["items"][0]["maxStackUnits"] = serde_json::json!(20);
            assert!(matches!(
                WorldPackDocument::from_json(&serde_json::to_vec(&smuggled_item).unwrap()),
                Err(DocumentToolError::LegacyItems(_))
            ));

            let mut smuggled_policy = legacy.clone();
            smuggled_policy["schemaVersion"] = serde_json::json!(schema_version);
            smuggled_policy["items"][0]["transferPolicy"] = serde_json::json!("character_bound");
            assert!(matches!(
                WorldPackDocument::from_json(&serde_json::to_vec(&smuggled_policy).unwrap()),
                Err(DocumentToolError::LegacyItems(_))
            ));

            let mut smuggled_rule = legacy.clone();
            smuggled_rule["schemaVersion"] = serde_json::json!(schema_version);
            smuggled_rule["gameplay"]["itemRules"][0]["container"] =
                serde_json::json!({"maxSlots": 4, "maxWeightGrams": 1000});
            assert!(matches!(
                WorldPackDocument::from_json(&serde_json::to_vec(&smuggled_rule).unwrap()),
                Err(DocumentToolError::LegacyItems(_))
            ));
        }
    }

    #[test]
    fn world_pack_v2_through_v6_normalize_without_inventing_rewards() {
        let current = serde_json::to_value(document()).unwrap();
        for schema_version in [2, 3, 4, 5, 6] {
            let mut legacy = current.clone();
            legacy["schemaVersion"] = serde_json::json!(schema_version);
            if schema_version <= 3 {
                legacy.as_object_mut().unwrap().remove("itemPrototypes");
                legacy
                    .as_object_mut()
                    .unwrap()
                    .remove("initialItemInstances");
                legacy["items"] = serde_json::json!([]);
            } else if schema_version == 4 {
                legacy["itemPrototypes"][0]
                    .as_object_mut()
                    .unwrap()
                    .remove("maxStackUnits");
                legacy["itemPrototypes"][0]
                    .as_object_mut()
                    .unwrap()
                    .remove("container");
            }
            let normalized =
                WorldPackDocument::from_json(&serde_json::to_vec(&legacy).unwrap()).unwrap();
            assert_eq!(
                normalized.world.schema_version,
                crate::CURRENT_SCHEMA_VERSION
            );
            assert!(normalized.world.gameplay.loot_tables.is_empty());
            assert!(normalized.world.gameplay.loot_sources.is_empty());
            assert!(normalized.world.gameplay.affixes.is_empty());
            assert!(normalized.world.gameplay.affix_pools.is_empty());

            if schema_version <= 5 {
                for field in ["lootTables", "lootSources"] {
                    let mut smuggled = legacy.clone();
                    smuggled["gameplay"][field] = serde_json::json!([]);
                    assert!(matches!(
                        WorldPackDocument::from_json(&serde_json::to_vec(&smuggled).unwrap()),
                        Err(DocumentToolError::LegacyItems(_))
                    ));
                }
            }

            let mut smuggled_affixes = legacy.clone();
            smuggled_affixes["gameplay"]["affixes"] = serde_json::json!([]);
            assert!(matches!(
                WorldPackDocument::from_json(&serde_json::to_vec(&smuggled_affixes).unwrap()),
                Err(DocumentToolError::LegacyItems(_))
            ));

            let mut smuggled_seal = legacy.clone();
            if schema_version <= 3 {
                smuggled_seal["items"] = serde_json::json!([{
                    "sealedLootTableId": "demo:rewards"
                }]);
            } else {
                smuggled_seal["itemPrototypes"][0]["sealedLootTableId"] =
                    serde_json::json!("demo:rewards");
            }
            assert!(matches!(
                WorldPackDocument::from_json(&serde_json::to_vec(&smuggled_seal).unwrap()),
                Err(DocumentToolError::LegacyItems(_))
            ));
        }

        let mut v6 = document();
        add_repeatable_loot(&mut v6);
        let mut v6 = serde_json::to_value(v6).unwrap();
        v6["schemaVersion"] = serde_json::json!(6);
        v6["gameplay"]["lootTables"][0]["pools"][0]["entries"][1]["affixPoolId"] =
            serde_json::json!("demo:affixes");
        assert!(matches!(
            WorldPackDocument::from_json(&serde_json::to_vec(&v6).unwrap()),
            Err(DocumentToolError::LegacyItems(_))
        ));
    }

    #[test]
    fn native_reward_dtos_reject_unknown_nested_fields() {
        let mut value = document();
        add_repeatable_loot(&mut value);
        let mut json = serde_json::to_value(value).unwrap();
        json["gameplay"]["lootTables"][0]["pools"][0]["entries"][0]["future"] =
            serde_json::json!(true);
        assert!(matches!(
            WorldPackDocument::from_json(&serde_json::to_vec(&json).unwrap()),
            Err(DocumentToolError::Json(_))
        ));

        let mut affix = serde_json::to_value(document()).unwrap();
        affix["gameplay"]["affixes"] = serde_json::json!([{
            "id":"demo:charged","label":"charged","position":"prefix",
            "attackBonusDelta":1,"future":true
        }]);
        assert!(matches!(
            WorldPackDocument::from_json(&serde_json::to_vec(&affix).unwrap()),
            Err(DocumentToolError::Json(_))
        ));

        let mut pool = serde_json::to_value(document()).unwrap();
        pool["gameplay"]["affixPools"] = serde_json::json!([{
            "id":"demo:affixes","entries":[{
                "kind":"nothing","id":"demo:plain","weight":1,"future":true
            }]
        }]);
        assert!(matches!(
            WorldPackDocument::from_json(&serde_json::to_vec(&pool).unwrap()),
            Err(DocumentToolError::Json(_))
        ));
    }

    #[test]
    fn legacy_item_normalization_rejects_oversized_and_control_text() {
        let legacy = serde_json::json!({
            "schemaVersion": 3,
            "id": "legacy",
            "name": "Legacy",
            "startRoomId": "legacy:start",
            "items": [{
                "id": "legacy:blade",
                "name": "blade",
                "description": "A legacy blade.",
                "aliases": ["sword"]
            }],
            "rooms": [{
                "id": "legacy:start",
                "title": "Start",
                "description": "A starting room.",
                "items": ["legacy:blade"]
            }]
        });

        for (field, value) in [
            (
                "name",
                serde_json::json!("n".repeat(crate::MAX_ITEM_NAME_BYTES + 1)),
            ),
            ("name", serde_json::json!("unsafe\u{1b}name")),
            (
                "description",
                serde_json::json!("d".repeat(crate::MAX_ITEM_DESCRIPTION_BYTES + 1)),
            ),
            ("description", serde_json::json!("unsafe\u{1b}description")),
        ] {
            let mut candidate = legacy.clone();
            candidate["items"][0][field] = value;
            assert!(matches!(
                WorldPackDocument::from_json(&serde_json::to_vec(&candidate).unwrap()),
                Err(DocumentToolError::LegacyItems(_))
            ));
        }

        for alias in [
            "a".repeat(crate::MAX_ITEM_ALIAS_BYTES + 1),
            "unsafe\u{1b}alias".to_owned(),
        ] {
            let mut candidate = legacy.clone();
            candidate["items"][0]["aliases"] = serde_json::json!([alias]);
            assert!(matches!(
                WorldPackDocument::from_json(&serde_json::to_vec(&candidate).unwrap()),
                Err(DocumentToolError::LegacyItems(_))
            ));
        }

        let mut v4 = serde_json::to_value(document()).unwrap();
        v4["schemaVersion"] = serde_json::json!(4);
        v4["itemPrototypes"][0]
            .as_object_mut()
            .unwrap()
            .remove("maxStackUnits");
        v4["itemPrototypes"][0]["name"] =
            serde_json::json!("n".repeat(crate::MAX_ITEM_NAME_BYTES + 1));
        let normalized = WorldPackDocument::from_json(&serde_json::to_vec(&v4).unwrap()).unwrap();
        assert!(normalized.validate().unwrap_err().0.iter().any(|error| {
            error
                .to_string()
                .contains("name must contain 1..=128 safe bytes")
        }));
    }

    #[test]
    fn legacy_item_normalization_rejects_ids_outside_the_runtime_contract() {
        let legacy = serde_json::json!({
            "schemaVersion": 3,
            "id": "legacy",
            "name": "Legacy",
            "startRoomId": "legacy:start",
            "items": [{
                "id": "Legacy:Blade",
                "name": "blade",
                "description": "A legacy blade."
            }],
            "rooms": [{
                "id": "legacy:start",
                "title": "Start",
                "description": "A starting room.",
                "items": ["Legacy:Blade"]
            }]
        });

        assert!(matches!(
            WorldPackDocument::from_json(&serde_json::to_vec(&legacy).unwrap()),
            Err(DocumentToolError::LegacyItems(error))
                if error.contains("lowercase namespaced identifier")
        ));
    }

    #[test]
    fn v6_rejects_legacy_item_authorities() {
        let mut native = serde_json::to_value(document()).unwrap();
        native["rooms"][0]["items"] = serde_json::json!(["demo:lantern"]);
        let error =
            WorldPackDocument::from_json(&serde_json::to_vec(&native).unwrap()).unwrap_err();
        assert!(matches!(error, DocumentToolError::LegacyItems(_)));
    }

    #[test]
    fn v4_normalizes_to_v6_but_rejects_schema_smuggling() {
        let mut legacy = serde_json::to_value(document()).unwrap();
        legacy["schemaVersion"] = serde_json::json!(4);
        legacy["itemPrototypes"][0]
            .as_object_mut()
            .unwrap()
            .remove("maxStackUnits");
        let normalized =
            WorldPackDocument::from_json(&serde_json::to_vec(&legacy).unwrap()).unwrap();
        normalized.validate().unwrap();
        assert_eq!(
            normalized.world.schema_version,
            crate::CURRENT_SCHEMA_VERSION
        );
        assert_eq!(normalized.world.item_prototypes[0].max_stack_units, 1);
        assert!(normalized.world.item_prototypes[0].container.is_none());

        for (field, value) in [
            ("maxStackUnits", serde_json::json!(2)),
            (
                "container",
                serde_json::json!({"maxSlots": 1, "maxWeightGrams": 0}),
            ),
        ] {
            let mut smuggled = legacy.clone();
            smuggled["itemPrototypes"][0][field] = value;
            assert!(matches!(
                WorldPackDocument::from_json(&serde_json::to_vec(&smuggled).unwrap()),
                Err(DocumentToolError::LegacyItems(_))
            ));
        }

        for transfer_policy in ["character_bound", "non_transferable"] {
            let mut smuggled = legacy.clone();
            smuggled["itemPrototypes"][0]["transferPolicy"] = serde_json::json!(transfer_policy);
            assert!(matches!(
                WorldPackDocument::from_json(&serde_json::to_vec(&smuggled).unwrap()),
                Err(DocumentToolError::LegacyItems(error))
                    if error.contains("must use transferPolicy transferable")
            ));
        }

        let mut smuggled_location = legacy;
        smuggled_location["initialItemInstances"][0]["location"] = serde_json::json!({
            "kind": "container",
            "containerInstanceId": "demo:lantern"
        });
        assert!(matches!(
            WorldPackDocument::from_json(&serde_json::to_vec(&smuggled_location).unwrap()),
            Err(DocumentToolError::LegacyItems(_))
        ));
    }

    #[test]
    fn scene_resolution_never_exposes_variant_ids_or_conditions() {
        let mut document = document();
        add_variants(&mut document);
        document.validate().unwrap();

        let resolved = document
            .resolve_scene("demo:square", &night_rain_context())
            .unwrap();
        assert_eq!(resolved.title, "Rain-dark Lantern Square");
        assert_eq!(resolved.items, ["blue lantern"]);
        let wire = serde_json::to_string(&resolved).unwrap();
        for secret in [
            "author-only-night-rain",
            "author-only-default",
            "world.time.period",
            "world.weather",
            "demo:square",
            "demo:lantern",
        ] {
            assert!(!wire.contains(secret), "resolved scene leaked {secret:?}");
        }

        let preview = document
            .render_preview("demo:square", &night_rain_context(), PreviewFormat::Text)
            .unwrap();
        assert!(preview.contains("Rain-dark Lantern Square"));
        assert!(!preview.contains("author-only"));
        assert!(!preview.contains("demo:"));
    }

    #[test]
    fn variant_validation_requires_default_unique_priority_and_valid_condition() {
        let mut document = document();
        document.scene_variants.push(RoomSceneVariants {
            room_id: "demo:square".into(),
            variants: vec![
                SceneVariant {
                    id: "one".into(),
                    priority: 7,
                    when: Some(Condition::Eq(
                        ConditionField::WorldTimeHour,
                        ConditionValue::String("late".into()),
                    )),
                    title: Some("One".into()),
                    description: None,
                    asset_manifest: None,
                    image_alt: LocalizedText::default(),
                    audio: None,
                },
                SceneVariant {
                    id: "two".into(),
                    priority: 7,
                    when: Some(Condition::Eq(
                        ConditionField::ActorIsAdmin,
                        ConditionValue::Boolean(true),
                    )),
                    title: Some("Two".into()),
                    description: None,
                    asset_manifest: None,
                    image_alt: LocalizedText::default(),
                    audio: None,
                },
            ],
        });
        let errors = document.validate().unwrap_err().0;
        assert!(
            errors.iter().any(|error| matches!(
                error,
                ValidationError::DuplicateSceneVariantPriority { .. }
            ))
        );
        assert!(
            errors
                .iter()
                .any(|error| matches!(error, ValidationError::MissingDefaultSceneVariant { .. }))
        );
        assert!(
            errors
                .iter()
                .any(|error| matches!(error, ValidationError::InvalidSceneVariantCondition { .. }))
        );
    }

    #[test]
    fn scene_variant_text_uses_the_same_utf8_byte_and_control_bounds_as_rooms() {
        let mut boundary = document();
        add_variants(&mut boundary);
        boundary.scene_variants[0].variants[0].title =
            Some("é".repeat(crate::MAX_ROOM_TITLE_BYTES / 2));
        boundary.scene_variants[0].variants[0].description =
            Some("é".repeat(crate::MAX_ROOM_DESCRIPTION_BYTES / 2));
        boundary.validate().unwrap();

        for (title, description, expected) in [
            (
                Some("é".repeat(crate::MAX_ROOM_TITLE_BYTES / 2 + 1)),
                None,
                "title must contain 1..=128 safe bytes",
            ),
            (
                Some("unsafe\nvariant".into()),
                None,
                "title must contain 1..=128 safe bytes",
            ),
            (
                None,
                Some("é".repeat(crate::MAX_ROOM_DESCRIPTION_BYTES / 2 + 1)),
                "description must contain 1..=8192 safe bytes",
            ),
            (
                None,
                Some("unsafe\u{7}variant".into()),
                "description must contain 1..=8192 safe bytes",
            ),
        ] {
            let mut invalid = boundary.clone();
            if let Some(title) = title {
                invalid.scene_variants[0].variants[0].title = Some(title);
            }
            if let Some(description) = description {
                invalid.scene_variants[0].variants[0].description = Some(description);
            }
            let messages = invalid
                .validate()
                .unwrap_err()
                .0
                .into_iter()
                .map(|error| error.to_string())
                .collect::<Vec<_>>()
                .join("\n");
            assert!(messages.contains(expected), "{messages}");
        }
    }

    #[test]
    fn script_validation_compiles_lua_and_checks_every_declared_hook() {
        let definition = |source: &str, hooks: Vec<ScriptHook>| ScriptDefinition {
            id: "demo:square-script".into(),
            room_id: "demo:square".into(),
            hooks,
            capabilities: vec![ScriptCapability::Narrate],
            limits: ScriptLimitsDefinition::default(),
            source: source.into(),
        };

        let mut valid = document();
        valid.scripts.push(definition(
            "function on_enter(ctx) return ctx end\nfunction on_leave(ctx) return ctx end",
            vec![ScriptHook::OnEnter, ScriptHook::OnLeave],
        ));
        valid.validate().unwrap();

        let mut malformed = document();
        malformed.scripts.push(definition(
            "function on_enter(ctx) this is not lua end",
            vec![ScriptHook::OnEnter],
        ));
        assert!(malformed.validate().unwrap_err().0.iter().any(|error| {
            matches!(
                error,
                ValidationError::InvalidScriptModule { script_id, .. }
                    if script_id == "demo:square-script"
            )
        }));

        let mut missing_hook = document();
        missing_hook.scripts.push(definition(
            "function on_enter(ctx) return ctx end",
            vec![ScriptHook::OnEnter, ScriptHook::OnLeave],
        ));
        assert!(missing_hook.validate().unwrap_err().0.iter().any(|error| {
            matches!(
                error,
                ValidationError::InvalidScriptModule { script_id, reason }
                    if script_id == "demo:square-script" && reason.contains("on_leave")
            )
        }));
    }

    #[test]
    fn authoring_rejects_a_room_hook_batch_larger_than_the_runtime_cap() {
        let script = |index: usize| ScriptDefinition {
            id: format!("demo:timer-{index:02}"),
            room_id: "demo:square".into(),
            hooks: vec![ScriptHook::OnTimer],
            capabilities: vec![ScriptCapability::Narrate],
            limits: ScriptLimitsDefinition::default(),
            source: "function on_timer() tiamud.emit('narrate', 'self', 'tick') end".into(),
        };
        let mut at_limit = document();
        at_limit.scripts = (0..MAX_MODULES_PER_ROOM_HOOK_BATCH).map(script).collect();
        at_limit.validate().unwrap();

        let mut over_limit = at_limit;
        over_limit
            .scripts
            .push(script(MAX_MODULES_PER_ROOM_HOOK_BATCH));
        let errors = over_limit.validate().unwrap_err().0;
        assert!(errors.iter().any(|error| matches!(
            error,
            ValidationError::TooManyScriptsForRoomHook {
                room_id,
                hook: ScriptHook::OnTimer,
                actual: 9,
                maximum: 8,
            } if room_id == "demo:square"
        )));
    }

    #[test]
    fn deterministic_artifact_ignores_semantically_irrelevant_order() {
        let mut left = document();
        add_variants(&mut left);
        add_repeatable_loot(&mut left);
        let mut right = left.clone();
        right.world.rooms.reverse();
        right.world.item_prototypes[0].aliases.reverse();
        right.world.gameplay.loot_tables[0].pools[0]
            .entries
            .reverse();
        right.scene_variants[0].variants.reverse();
        if let Some(Condition::All(conditions)) = &mut right.scene_variants[0].variants[0].when {
            conditions.reverse();
        } else if let Some(Condition::All(conditions)) =
            &mut right.scene_variants[0].variants[1].when
        {
            conditions.reverse();
        }

        assert_eq!(
            left.deterministic_artifact().unwrap(),
            right.deterministic_artifact().unwrap()
        );
    }

    #[test]
    fn embedded_content_tests_cover_selection_and_secret_leaks() {
        let mut document = document();
        add_variants(&mut document);
        document.content_tests.push(ContentTest {
            name: "night rain scene".into(),
            room_id: "demo:square".into(),
            context: night_rain_context(),
            expect: ContentTestExpectation {
                title: Some("Rain-dark Lantern Square".into()),
                description_contains: vec!["moonless".into()],
                output_excludes: vec!["author-only".into(), "world.time.period".into()],
                asset_manifest: Some("assets/night-rain/manifest.json".into()),
                exits: Some(vec![Direction::North]),
                items: Some(vec!["blue lantern".into()]),
            },
        });
        let report = document.run_content_tests().unwrap();
        assert!(report.is_success());
        assert_eq!((report.passed, report.total), (1, 1));

        document.content_tests[0].expect.title = Some("Wrong".into());
        let report = document.run_content_tests().unwrap();
        assert!(!report.is_success());
        assert!(report.failures[0].messages[0].contains("title was"));
    }

    #[test]
    fn text_and_ansi_previews_are_complete_and_sanitize_terminal_controls() {
        assert_eq!(
            sanitize_terminal_text("Lantern \u{1b}[31mSquare"),
            "Lantern [31mSquare"
        );
        let document = document();
        let text = document
            .render_preview(
                "demo:square",
                &ConditionContext::default(),
                PreviewFormat::Text,
            )
            .unwrap();
        assert!(!text.contains('\u{1b}'));
        assert!(text.contains("Exits: north"));
        assert!(text.contains("Items: blue lantern"));

        let ansi = document
            .render_preview(
                "demo:square",
                &ConditionContext::default(),
                PreviewFormat::Ansi,
            )
            .unwrap();
        assert!(ansi.starts_with("\u{1b}[1;36mLantern Square"));
    }

    #[test]
    fn semantic_diff_is_sorted_and_reports_authoring_changes() {
        let before = document();
        let mut after = before.clone();
        add_variants(&mut after);
        add_repeatable_loot(&mut after);
        after.world.item_prototypes[0].equipment_slot = Some(crate::EquipmentSlot::Trinket);
        after.world.gameplay.affixes.push(crate::AffixDefinition {
            id: "demo:charged".into(),
            label: "charged".into(),
            position: crate::AffixPosition::Prefix,
            attack_bonus_delta: 1,
            heal_amount_delta: 0,
        });
        after
            .world
            .gameplay
            .affix_pools
            .push(crate::AffixPoolDefinition {
                id: "demo:lantern-affixes".into(),
                entries: vec![crate::AffixPoolEntry {
                    kind: crate::AffixPoolEntryKind::Affix,
                    id: "demo:charged-roll".into(),
                    weight: 1,
                    affix_id: Some("demo:charged".into()),
                }],
            });
        let mut seal = after.world.item_prototypes[0].clone();
        seal.id = "demo:sealed-lantern-reward".into();
        seal.name = "sealed lantern reward".into();
        seal.aliases.clear();
        seal.equipment_slot = None;
        seal.attack_bonus = 0;
        seal.consumable = true;
        seal.transfer_policy = crate::ItemTransferPolicy::CharacterBound;
        seal.sealed_loot_table_id = Some("demo:lantern-rewards".into());
        after.world.item_prototypes.push(seal);
        after.world.gameplay.loot_tables[0]
            .guaranteed
            .push(crate::LootGuaranteedDefinition {
                id: "demo:earned-seal".into(),
                item_prototype_id: "demo:sealed-lantern-reward".into(),
                quantity: 1,
                affix_pool_id: None,
            });
        after
            .world
            .gameplay
            .loot_tables
            .push(crate::LootTableDefinition {
                id: "demo:lantern-rewards".into(),
                guaranteed: vec![crate::LootGuaranteedDefinition {
                    id: "demo:lantern-reward".into(),
                    item_prototype_id: "demo:lantern".into(),
                    quantity: 1,
                    affix_pool_id: Some("demo:lantern-affixes".into()),
                }],
                pools: Vec::new(),
            });
        after.world.rooms[0].description.push_str(" Changed.");
        after.content_tests.push(ContentTest {
            name: "smoke".into(),
            room_id: "demo:square".into(),
            context: ConditionContext::default(),
            expect: ContentTestExpectation {
                title: Some("Lantern Square".into()),
                ..ContentTestExpectation::default()
            },
        });
        let diff = before.diff(&after).unwrap();
        assert_eq!(diff.rooms.changed, ["demo:square"]);
        assert_eq!(diff.scene_variants.added, ["demo:square"]);
        assert_eq!(diff.item_prototypes.changed, ["demo:lantern"]);
        assert_eq!(diff.item_prototypes.added, ["demo:sealed-lantern-reward"]);
        assert_eq!(diff.content_tests.added, ["smoke"]);
        assert_eq!(
            diff.loot_tables.added,
            ["demo:lantern-rewards", "demo:wolf-drops"]
        );
        assert_eq!(diff.loot_sources.added, ["demo:wolf-source"]);
        assert_eq!(diff.affixes.added, ["demo:charged"]);
        assert_eq!(diff.affix_pools.added, ["demo:lantern-affixes"]);
        assert!(diff.to_string().contains("scene variants: + demo:square"));
    }

    fn sprite_source(pose: &str) -> EntitySpriteSource {
        let hash = "a".repeat(64);
        EntitySpriteSource {
            webp: Some(format!("assets/{hash}/sprite-{pose}-640.webp")),
            png: Some(format!("assets/{hash}/sprite-{pose}-640.png")),
        }
    }

    fn lantern_media() -> EntityMediaDefinition {
        EntityMediaDefinition {
            kind: EntityMediaKind::Item,
            subject_id: "demo:lantern".into(),
            alt: LocalizedText(BTreeMap::from([(
                "en".into(),
                "A cold blue lantern".into(),
            )])),
            decorative: false,
            footprint: NormalizedFootprint {
                width_milli: 180,
                height_milli: 420,
            },
            preferred_anchor: Some(NormalizedAnchor {
                x_milli: 250,
                y_milli: 900,
            }),
            priority: 5,
            layer: EntityMediaLayer::Foreground,
            poses: EntitySpritePoses {
                idle: sprite_source("idle"),
                attack: None,
                hit: Some(sprite_source("hit")),
                defeated: None,
            },
        }
    }

    #[test]
    fn entity_media_is_strict_validated_canonical_and_diffable() {
        let mut before = document();
        before.entity_media.push(lantern_media());
        before.validate().unwrap();

        let first = before.deterministic_artifact().unwrap();
        let reparsed = WorldPackDocument::from_json(&first).unwrap();
        assert_eq!(first, reparsed.deterministic_artifact().unwrap());

        let mut after = before.clone();
        after.entity_media[0].priority = 6;
        let diff = before.diff(&after).unwrap();
        assert_eq!(diff.entity_media.changed, ["item:demo:lantern"]);

        let mut invalid = before;
        invalid.entity_media[0].poses.idle.webp = Some("https://evil.invalid/sprite.webp".into());
        let error = format!("{:?}", invalid.validate().unwrap_err().0);
        assert!(error.contains("idle webp path must be assets/"));
    }

    #[test]
    fn item_media_targets_a_reusable_prototype_while_perception_targets_an_instance() {
        let mut document = document();
        document.world.initial_item_instances[0].id = "demo:lantern:one".into();
        document.entity_media.push(lantern_media());
        document.room_perception.push(RoomPerceptionDefinition {
            room_id: "demo:square".into(),
            light: LightLevel::Bright,
            hidden_items: BTreeMap::from([("demo:lantern:one".into(), 20)]),
            hidden_npcs: BTreeMap::new(),
            secret_exits: BTreeMap::new(),
        });

        document.validate().unwrap();

        document.entity_media[0].subject_id = "demo:lantern:one".into();
        let error = format!("{:?}", document.validate().unwrap_err().0);
        assert!(error.contains("item prototype or NPC spawn does not exist"));
    }

    #[test]
    fn entity_media_rejects_unknown_or_duplicate_subjects_and_alt_misuse() {
        let mut document = document();
        let mut media = lantern_media();
        media.subject_id = "demo:hidden-missing".into();
        media.decorative = true;
        document.entity_media.push(media.clone());
        document.entity_media.push(media);
        let error = format!("{:?}", document.validate().unwrap_err().0);
        assert!(error.contains("DuplicateEntityMedia"));
        assert!(error.contains("does not exist"));
        assert!(error.contains("decorative sprites must have empty alt text"));
    }
}
