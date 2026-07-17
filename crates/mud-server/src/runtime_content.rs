use std::{
    collections::{BTreeMap, BTreeSet, HashMap},
    sync::Arc,
    time::{Duration, Instant},
};

use serde::Serialize;
use thiserror::Error;
use tiamud_core::{
    AccountRole, AuthorizedRuntimeContext, DomainEvent, PlayerAction, QuestAction, SceneAudio,
    SceneImage, SceneView, SessionId, WeatherKind, World,
};
use tiamud_scripting::{
    MAX_MODULES_PER_ROOM_HOOK_BATCH, ScriptCapability, ScriptContext, ScriptError, ScriptHook,
    ScriptLimits, execute_hook,
};
use world_pack::{
    ConditionContext, EntityMediaDefinition, EntityMediaKind, EntitySpriteSource,
    MAX_PROJECTED_ENTITY_MEDIA, MAX_PROJECTED_ENTITY_POSES, RoomSceneVariants,
    ScriptCapability as PackScriptCapability, ScriptDefinition, ScriptHook as PackScriptHook,
    TimePeriod, Weather,
};

use crate::operations::{RuntimeMetrics, ScriptMetricOutcome};

pub(crate) const SCRIPT_BATCH_WALL_BUDGET: Duration = Duration::from_millis(50);

#[derive(Clone, Debug)]
pub(crate) struct RuntimeContent {
    variants: HashMap<String, RoomSceneVariants>,
    entity_media: HashMap<(EntityMediaKind, String), EntityMediaDefinition>,
    scripts: Vec<RuntimeScript>,
    seed: u64,
    operational_metrics: Option<Arc<RuntimeMetrics>>,
}

impl Default for RuntimeContent {
    fn default() -> Self {
        Self::new(Vec::new(), Vec::new(), 0x5449_414d_5544)
    }
}

impl RuntimeContent {
    pub(crate) fn new(
        variants: Vec<RoomSceneVariants>,
        scripts: Vec<RuntimeScript>,
        seed: u64,
    ) -> Self {
        let mut scripts = scripts;
        scripts.sort_by(|left, right| left.id.cmp(&right.id));
        Self {
            variants: variants
                .into_iter()
                .map(|variants| (variants.room_id.clone(), variants))
                .collect(),
            entity_media: HashMap::new(),
            scripts,
            seed,
            operational_metrics: None,
        }
    }

    pub(crate) fn with_entity_media(mut self, media: Vec<EntityMediaDefinition>) -> Self {
        self.entity_media = media
            .into_iter()
            .map(|media| ((media.kind, media.subject_id.clone()), media))
            .collect();
        self
    }

    pub(crate) fn resolve_entity_presentation(
        &self,
        world: &World,
        session: SessionId,
    ) -> ScenePresentation {
        let Ok(visible) = world.visible_scene_entity_ids(session) else {
            return ScenePresentation::default();
        };
        let mut candidates = visible
            .items
            .into_iter()
            .filter_map(|entity| {
                self.entity_media
                    .get(&(EntityMediaKind::Item, entity.prototype_id))
                    .map(|media| ProjectedEntityMedia::from_definition(entity.opaque_id, media))
            })
            .chain(visible.npcs.into_iter().filter_map(|entity| {
                self.entity_media
                    .get(&(EntityMediaKind::Npc, entity.stable_id))
                    .map(|media| ProjectedEntityMedia::from_definition(entity.opaque_id, media))
            }))
            .collect::<Vec<_>>();
        candidates.sort_by(|left, right| {
            right
                .priority
                .cmp(&left.priority)
                .then_with(|| left.id.cmp(&right.id))
        });
        let mut poses = 0;
        let entities = candidates
            .into_iter()
            .filter(|candidate| {
                let next = poses + candidate.poses.count();
                if next > MAX_PROJECTED_ENTITY_POSES {
                    false
                } else {
                    poses = next;
                    true
                }
            })
            .take(MAX_PROJECTED_ENTITY_MEDIA)
            .collect();
        ScenePresentation { entities }
    }

    pub(crate) fn with_operational_metrics(mut self, metrics: Arc<RuntimeMetrics>) -> Self {
        self.operational_metrics = Some(metrics);
        self
    }

    pub(crate) fn operational_metrics(&self) -> Option<Arc<RuntimeMetrics>> {
        self.operational_metrics.as_ref().map(Arc::clone)
    }

    /// Installs the presentation and script slice prepared for one or more
    /// areas. Validation happens against a private candidate so callers can
    /// compose this with the core lifecycle checkpoint without ever exposing
    /// a partially installed area to a transport.
    pub(crate) fn install_delta(
        &mut self,
        delta: RuntimeContentDelta,
    ) -> Result<(), RuntimeContentInstallError> {
        let mut candidate = self.clone();
        for variants in delta.variants {
            let room_id = variants.room_id.clone();
            if candidate
                .variants
                .insert(room_id.clone(), variants)
                .is_some()
            {
                return Err(RuntimeContentInstallError::ExistingRoomVariants(room_id));
            }
        }
        for media in delta.entity_media {
            let kind = media.kind;
            let subject_id = media.subject_id.clone();
            let key = (kind, subject_id.clone());
            if candidate.entity_media.insert(key, media).is_some() {
                return Err(RuntimeContentInstallError::EntityMediaOccupied { kind, subject_id });
            }
        }
        let mut script_ids = candidate
            .scripts
            .iter()
            .map(|script| script.id.as_str())
            .collect::<BTreeSet<_>>();
        for script in &delta.scripts {
            if !script_ids.insert(script.id.as_str()) {
                return Err(RuntimeContentInstallError::ScriptIdCollision(
                    script.id.clone(),
                ));
            }
        }
        candidate.scripts.extend(delta.scripts);
        candidate
            .scripts
            .sort_by(|left, right| left.id.cmp(&right.id));
        *self = candidate;
        Ok(())
    }

    pub(crate) fn resolve_scene(
        &self,
        world: &World,
        session: SessionId,
        role: AccountRole,
        scene: &mut SceneView,
    ) {
        if !world.can_receive_visual_scene(session).unwrap_or(false) {
            return;
        }
        let Ok(runtime) = world.authorized_runtime_context(session) else {
            return;
        };
        let Some(variants) = self.variants.get(&runtime.room_id) else {
            return;
        };
        let context = condition_context(&runtime, role);
        let selected = variants
            .variants
            .iter()
            .filter(|variant| {
                variant
                    .when
                    .as_ref()
                    .is_some_and(|condition| condition.evaluate(&context).unwrap_or(false))
            })
            .max_by_key(|variant| variant.priority)
            .or_else(|| {
                variants
                    .variants
                    .iter()
                    .find(|variant| variant.when.is_none())
            });
        let Some(selected) = selected else {
            return;
        };
        if let Some(title) = &selected.title {
            scene.title.clone_from(title);
        }
        if let Some(description) = &selected.description {
            scene.prose = split_prose(description);
        }
        if let Some(manifest) = &selected.asset_manifest {
            scene.image = Some(SceneImage {
                alt: selected
                    .image_alt
                    .resolve("en")
                    .expect("validated manifest overrides have localized alt text")
                    .to_owned(),
                localized_alt: selected.image_alt.0.clone(),
                manifest: Some(format!("/{manifest}")),
                avif: None,
                webp: None,
                jpeg: None,
                width: None,
                height: None,
            });
        } else if !selected.image_alt.is_empty()
            && let (Some(image), Some(alt)) = (&mut scene.image, selected.image_alt.resolve("en"))
        {
            image.alt = alt.to_owned();
            image.localized_alt = selected.image_alt.0.clone();
        }
        if let Some(audio) = &selected.audio {
            scene.audio = Some(SceneAudio {
                channel: audio.channel.as_str().to_owned(),
                url: format!("/{}", audio.path),
                looped: audio.looped,
            });
        }
    }

    /// Execute every room-local module for one hook. All Lua executions finish
    /// before any event is applied. If one module or event fails, the entire
    /// hook batch is inert and the world is restored to its pre-hook checkpoint.
    pub(crate) fn run_hook(
        &self,
        world: &mut World,
        session: SessionId,
        role: AccountRole,
        hook: ScriptHook,
    ) -> ScriptBatchOutcome {
        self.run_hook_with_budget(world, session, role, hook, SCRIPT_BATCH_WALL_BUDGET)
    }

    pub(crate) fn run_hook_with_budget(
        &self,
        world: &mut World,
        session: SessionId,
        role: AccountRole,
        hook: ScriptHook,
        maximum_wall_time: Duration,
    ) -> ScriptBatchOutcome {
        let Ok(runtime) = world.authorized_runtime_context(session) else {
            return ScriptBatchOutcome::rejected();
        };
        let scripts = self
            .scripts
            .iter()
            .filter(|script| script.room_id == runtime.room_id && script.hooks.contains(&hook))
            .take(MAX_MODULES_PER_ROOM_HOOK_BATCH + 1)
            .collect::<Vec<_>>();
        if scripts.is_empty() {
            return ScriptBatchOutcome::default();
        }
        if scripts.len() > MAX_MODULES_PER_ROOM_HOOK_BATCH {
            if let Some(metrics) = &self.operational_metrics {
                metrics.script_finished(Duration::ZERO, ScriptMetricOutcome::Error);
            }
            tracing::warn!(
                hook = hook.function_name(),
                category = "batch_module_limit",
                maximum = MAX_MODULES_PER_ROOM_HOOK_BATCH,
                "WorldPack script batch rejected before execution"
            );
            return ScriptBatchOutcome::rejected();
        }

        let batch_budget = maximum_wall_time.min(SCRIPT_BATCH_WALL_BUDGET);
        let batch_started = Instant::now();
        if batch_budget.is_zero() {
            return reject_script_budget(self, hook, batch_started, "aggregate_budget");
        }

        let context = script_context(&runtime, role);
        let mut pending = Vec::new();
        for script in scripts {
            let Some(remaining) = batch_budget.checked_sub(batch_started.elapsed()) else {
                return reject_script_budget(self, hook, batch_started, "aggregate_budget");
            };
            if remaining.is_zero() {
                return reject_script_budget(self, hook, batch_started, "aggregate_budget");
            }
            let mut context = context.clone();
            context.seed = deterministic_hook_seed(
                self.seed,
                &script.id,
                runtime.character_id.0,
                runtime.logical_tick,
                hook,
            );
            let mut module_limits = script.limits;
            module_limits.max_wall_time = module_limits.max_wall_time.min(remaining);
            match execute_hook(
                &script.source,
                hook,
                &context,
                &script.capabilities,
                module_limits,
            ) {
                Ok(outcome) => {
                    if let Some(metrics) = &self.operational_metrics {
                        metrics.script_finished(outcome.elapsed, ScriptMetricOutcome::Success);
                    }
                    pending.extend(outcome.events);
                    if batch_started.elapsed() > batch_budget {
                        return reject_script_budget(self, hook, batch_started, "aggregate_budget");
                    }
                }
                Err(error) => {
                    if matches!(error, ScriptError::ExecutionQuota)
                        && batch_started.elapsed() >= batch_budget
                    {
                        return reject_script_budget(self, hook, batch_started, "aggregate_budget");
                    }
                    if let Some(metrics) = &self.operational_metrics {
                        metrics.script_finished(
                            std::time::Duration::ZERO,
                            if matches!(error, ScriptError::ExecutionQuota) {
                                ScriptMetricOutcome::Timeout
                            } else {
                                ScriptMetricOutcome::Error
                            },
                        );
                    }
                    tracing::warn!(
                        hook = hook.function_name(),
                        category = script_error_category(&error),
                        "isolated WorldPack script failure"
                    );
                    return ScriptBatchOutcome::rejected();
                }
            }
        }

        if batch_started.elapsed() > batch_budget {
            return reject_script_budget(self, hook, batch_started, "aggregate_budget");
        }

        let Some(validated) = validate_events(&pending) else {
            if let Some(metrics) = &self.operational_metrics {
                metrics.script_finished(std::time::Duration::ZERO, ScriptMetricOutcome::Error);
            }
            tracing::warn!(
                hook = hook.function_name(),
                category = "invalid_event",
                "isolated WorldPack script failure"
            );
            return ScriptBatchOutcome::rejected();
        };
        let checkpoint = world.checkpoint();
        let mut result = ScriptBatchOutcome::default();
        for event in validated {
            match event {
                ValidatedEvent::Narrate(text) => result.narration.push(text),
                ValidatedEvent::SetFlag(flag) => {
                    match world.set_authorized_script_flag(session, &flag) {
                        Ok(changed) => result.changed |= changed,
                        Err(_) => {
                            world.restore_checkpoint(checkpoint);
                            if let Some(metrics) = &self.operational_metrics {
                                metrics.script_finished(
                                    std::time::Duration::ZERO,
                                    ScriptMetricOutcome::Error,
                                );
                            }
                            tracing::warn!(
                                hook = hook.function_name(),
                                category = "invalid_effect",
                                "isolated WorldPack script failure"
                            );
                            return ScriptBatchOutcome::rejected();
                        }
                    }
                }
                ValidatedEvent::Damage(amount) => {
                    match world.apply_authorized_script_damage(session, amount) {
                        Ok(changed) => result.changed |= changed,
                        Err(_) => {
                            return rollback_script_batch(world, checkpoint.clone(), self, hook);
                        }
                    }
                }
                ValidatedEvent::Heal(amount) => {
                    match world.apply_authorized_script_heal(session, amount) {
                        Ok(changed) => result.changed |= changed,
                        Err(_) => {
                            return rollback_script_batch(world, checkpoint.clone(), self, hook);
                        }
                    }
                }
                ValidatedEvent::GiveItem(item_id) => {
                    match world.give_authorized_script_item(session, &item_id) {
                        Ok(event) => {
                            result.changed |= event.is_some();
                            result.domain_events.extend(event);
                        }
                        Err(_) => {
                            return rollback_script_batch(world, checkpoint.clone(), self, hook);
                        }
                    }
                }
                ValidatedEvent::StartQuest(quest_id) => {
                    match world.apply(session, PlayerAction::Quest(QuestAction::Accept(quest_id))) {
                        Ok(outcome) => {
                            result.changed |= outcome.changed_character.is_some()
                                || !outcome.domain_events.is_empty();
                            result.domain_events.extend(outcome.domain_events);
                        }
                        Err(_) => {
                            return rollback_script_batch(world, checkpoint.clone(), self, hook);
                        }
                    }
                }
                ValidatedEvent::Schedule { delay_ticks, flag } => {
                    match world.schedule_authorized_script_flag(session, delay_ticks, &flag) {
                        Ok(changed) => result.changed |= changed,
                        Err(_) => {
                            return rollback_script_batch(world, checkpoint.clone(), self, hook);
                        }
                    }
                }
            }
        }
        if batch_started.elapsed() > batch_budget {
            world.restore_checkpoint(checkpoint);
            return reject_script_budget(self, hook, batch_started, "aggregate_budget");
        }
        result
    }
}

#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ScenePresentation {
    pub(crate) entities: Vec<ProjectedEntityMedia>,
}

impl ScenePresentation {
    pub(crate) fn is_empty(&self) -> bool {
        self.entities.is_empty()
    }

    pub(crate) fn contains_entity(&self, entity_id: &str) -> bool {
        self.entities.iter().any(|entity| entity.id == entity_id)
    }

    pub(crate) fn retain_entities(&mut self, mut keep: impl FnMut(&str) -> bool) {
        self.entities.retain(|entity| keep(&entity.id));
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ProjectedEntityMedia {
    id: String,
    alt: String,
    decorative: bool,
    footprint: ProjectedFootprint,
    #[serde(skip_serializing_if = "Option::is_none")]
    preferred_anchor: Option<ProjectedAnchor>,
    priority: i16,
    layer: &'static str,
    poses: ProjectedPoses,
}

impl ProjectedEntityMedia {
    fn from_definition(id: String, media: &EntityMediaDefinition) -> Self {
        Self {
            id,
            alt: if media.decorative {
                String::new()
            } else {
                media.alt.resolve("en").unwrap_or_default().to_owned()
            },
            decorative: media.decorative,
            footprint: ProjectedFootprint {
                width: f32::from(media.footprint.width_milli) / 1_000.0,
                height: f32::from(media.footprint.height_milli) / 1_000.0,
            },
            preferred_anchor: media.preferred_anchor.map(|anchor| ProjectedAnchor {
                x: f32::from(anchor.x_milli) / 1_000.0,
                y: f32::from(anchor.y_milli) / 1_000.0,
            }),
            priority: media.priority,
            layer: media.layer.as_str(),
            poses: ProjectedPoses::from_definition(&media.poses),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize)]
struct ProjectedFootprint {
    width: f32,
    height: f32,
}

impl Eq for ProjectedFootprint {}

#[derive(Clone, Copy, Debug, PartialEq, Serialize)]
struct ProjectedAnchor {
    x: f32,
    y: f32,
}

impl Eq for ProjectedAnchor {}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
struct ProjectedSource {
    #[serde(skip_serializing_if = "Option::is_none")]
    webp: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    png: Option<String>,
}

impl From<&EntitySpriteSource> for ProjectedSource {
    fn from(source: &EntitySpriteSource) -> Self {
        Self {
            webp: source.webp.as_ref().map(|path| format!("/{path}")),
            png: source.png.as_ref().map(|path| format!("/{path}")),
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
struct ProjectedPoses {
    idle: ProjectedSource,
    #[serde(skip_serializing_if = "Option::is_none")]
    attack: Option<ProjectedSource>,
    #[serde(skip_serializing_if = "Option::is_none")]
    hit: Option<ProjectedSource>,
    #[serde(skip_serializing_if = "Option::is_none")]
    defeated: Option<ProjectedSource>,
}

impl ProjectedPoses {
    fn from_definition(poses: &world_pack::EntitySpritePoses) -> Self {
        Self {
            idle: (&poses.idle).into(),
            attack: poses.attack.as_ref().map(Into::into),
            hit: poses.hit.as_ref().map(Into::into),
            defeated: poses.defeated.as_ref().map(Into::into),
        }
    }

    fn count(&self) -> usize {
        1 + usize::from(self.attack.is_some())
            + usize::from(self.hit.is_some())
            + usize::from(self.defeated.is_some())
    }
}

#[derive(Clone, Debug, Default)]
pub(crate) struct RuntimeContentDelta {
    pub(crate) variants: Vec<RoomSceneVariants>,
    pub(crate) entity_media: Vec<EntityMediaDefinition>,
    pub(crate) scripts: Vec<RuntimeScript>,
}

impl RuntimeContentDelta {
    pub(crate) fn new(
        variants: Vec<RoomSceneVariants>,
        entity_media: Vec<EntityMediaDefinition>,
        scripts: Vec<RuntimeScript>,
    ) -> Self {
        Self {
            variants,
            entity_media,
            scripts,
        }
    }
}

#[derive(Debug, Error, Eq, PartialEq)]
pub(crate) enum RuntimeContentInstallError {
    #[error("room `{0}` already has runtime variants")]
    ExistingRoomVariants(String),
    #[error("{kind:?} entity `{subject_id}` already has runtime media")]
    EntityMediaOccupied {
        kind: EntityMediaKind,
        subject_id: String,
    },
    #[error("script `{0}` is already installed")]
    ScriptIdCollision(String),
}

#[derive(Clone, Debug)]
pub(crate) struct RuntimeScript {
    pub(crate) id: String,
    pub(crate) room_id: String,
    pub(crate) hooks: BTreeSet<ScriptHook>,
    pub(crate) capabilities: BTreeSet<ScriptCapability>,
    pub(crate) limits: ScriptLimits,
    pub(crate) source: String,
}

impl RuntimeScript {
    pub(crate) fn from_definition(definition: ScriptDefinition) -> Self {
        Self {
            id: definition.id,
            room_id: definition.room_id,
            hooks: definition.hooks.into_iter().map(map_hook).collect(),
            capabilities: definition
                .capabilities
                .into_iter()
                .map(map_capability)
                .collect(),
            limits: ScriptLimits {
                max_source_bytes: definition.limits.max_source_bytes,
                max_memory_bytes: definition.limits.max_memory_bytes,
                max_instructions: definition.limits.max_instructions,
                max_wall_time: std::time::Duration::from_millis(
                    definition.limits.max_wall_time_millis,
                ),
                max_events: definition.limits.max_events,
                max_event_field_bytes: definition.limits.max_event_field_bytes,
            },
            source: definition.source,
        }
    }
}

#[derive(Debug, Default)]
pub(crate) struct ScriptBatchOutcome {
    pub(crate) narration: Vec<String>,
    pub(crate) changed: bool,
    pub(crate) domain_events: Vec<DomainEvent>,
    pub(crate) budget_exhausted: bool,
    /// Distinguishes an inert hook with no matching module from a rejected
    /// batch. Actions that make the hook part of their transaction can then
    /// fail closed without inferring rejection from an empty outcome.
    pub(crate) rejected: bool,
}

impl ScriptBatchOutcome {
    fn rejected() -> Self {
        Self {
            rejected: true,
            ..Self::default()
        }
    }
}

enum ValidatedEvent {
    Narrate(String),
    SetFlag(String),
    Damage(u32),
    Heal(u32),
    GiveItem(String),
    StartQuest(String),
    Schedule { delay_ticks: u64, flag: String },
}

fn validate_events(events: &[tiamud_scripting::ScriptEvent]) -> Option<Vec<ValidatedEvent>> {
    events
        .iter()
        .map(|event| {
            if event
                .target
                .as_deref()
                .is_some_and(|target| target != "self")
            {
                return None;
            }
            match event.kind.as_str() {
                "narrate" => {
                    let value = event.value.as_deref()?.trim();
                    if value.is_empty() || value.len() > 1024 {
                        return None;
                    }
                    let sanitized = value
                        .chars()
                        .filter(|character| !character.is_control() || *character == '\n')
                        .collect::<String>();
                    Some(ValidatedEvent::Narrate(sanitized))
                }
                "set_flag" => {
                    let flag = event.value.as_deref()?.trim();
                    if flag.is_empty()
                        || flag.len() > 64
                        || !flag.chars().all(|character| {
                            character.is_ascii_alphanumeric()
                                || matches!(character, '-' | '_' | '.')
                        })
                    {
                        return None;
                    }
                    Some(ValidatedEvent::SetFlag(flag.to_owned()))
                }
                "damage" => Some(ValidatedEvent::Damage(parse_script_amount(
                    event.value.as_deref()?,
                )?)),
                "heal" => Some(ValidatedEvent::Heal(parse_script_amount(
                    event.value.as_deref()?,
                )?)),
                "give_item" => Some(ValidatedEvent::GiveItem(parse_script_content_id(
                    event.value.as_deref()?,
                    128,
                )?)),
                "start_quest" => Some(ValidatedEvent::StartQuest(parse_script_content_id(
                    event.value.as_deref()?,
                    64,
                )?)),
                "schedule" => {
                    let (delay, flag) = event.value.as_deref()?.split_once(':')?;
                    let delay_ticks = delay.parse::<u64>().ok()?;
                    if delay_ticks == 0 || delay_ticks > 86_400 {
                        return None;
                    }
                    let flag = flag.trim();
                    if !valid_script_flag(flag) {
                        return None;
                    }
                    Some(ValidatedEvent::Schedule {
                        delay_ticks,
                        flag: flag.to_owned(),
                    })
                }
                _ => None,
            }
        })
        .collect()
}

fn parse_script_amount(value: &str) -> Option<u32> {
    let amount = value.trim().parse::<u32>().ok()?;
    (amount > 0 && amount <= 10_000).then_some(amount)
}

fn parse_script_content_id(value: &str, maximum: usize) -> Option<String> {
    let value = value.trim();
    (!value.is_empty()
        && value.len() <= maximum
        && value.chars().all(|character| {
            character.is_ascii_alphanumeric() || matches!(character, ':' | '/' | '-' | '_' | '.')
        }))
    .then(|| value.to_owned())
}

fn valid_script_flag(flag: &str) -> bool {
    !flag.is_empty()
        && flag.len() <= 64
        && flag.chars().all(|character| {
            character.is_ascii_alphanumeric() || matches!(character, '-' | '_' | '.')
        })
}

fn rollback_script_batch(
    world: &mut World,
    checkpoint: tiamud_core::WorldCheckpoint,
    content: &RuntimeContent,
    hook: ScriptHook,
) -> ScriptBatchOutcome {
    world.restore_checkpoint(checkpoint);
    if let Some(metrics) = &content.operational_metrics {
        metrics.script_finished(std::time::Duration::ZERO, ScriptMetricOutcome::Error);
    }
    tracing::warn!(
        hook = hook.function_name(),
        category = "invalid_effect",
        "isolated WorldPack script failure"
    );
    ScriptBatchOutcome::rejected()
}

fn reject_script_budget(
    content: &RuntimeContent,
    hook: ScriptHook,
    started: Instant,
    category: &'static str,
) -> ScriptBatchOutcome {
    if let Some(metrics) = &content.operational_metrics {
        metrics.script_finished(started.elapsed(), ScriptMetricOutcome::Timeout);
    }
    tracing::warn!(
        hook = hook.function_name(),
        category,
        "WorldPack script aggregate budget exhausted"
    );
    ScriptBatchOutcome {
        budget_exhausted: true,
        rejected: true,
        ..ScriptBatchOutcome::default()
    }
}

fn condition_context(runtime: &AuthorizedRuntimeContext, role: AccountRole) -> ConditionContext {
    let hour = (runtime.logical_tick % 24) as u8;
    ConditionContext {
        world_instance: runtime.world_instance.clone(),
        world_time_period: match hour {
            5..=7 => TimePeriod::Dawn,
            8..=17 => TimePeriod::Day,
            18..=20 => TimePeriod::Dusk,
            _ => TimePeriod::Night,
        },
        world_weather: match runtime.weather {
            WeatherKind::Clear => Weather::Clear,
            WeatherKind::Rain => Weather::Rain,
            WeatherKind::Wind => Weather::Cloudy,
            WeatherKind::Fog => Weather::Fog,
        },
        world_time_hour: hour,
        actor_level: runtime.actor_level,
        actor_is_admin: role == AccountRole::Admin,
        actor_flags: runtime.actor_flags.clone(),
        actor_quest_states: runtime.actor_quest_states.clone(),
        actor_content_preferences: runtime.actor_content_preferences.clone(),
        actor_senses: runtime.actor_senses.clone(),
    }
}

fn script_context(runtime: &AuthorizedRuntimeContext, role: AccountRole) -> ScriptContext {
    let condition = condition_context(runtime, role);
    ScriptContext {
        values: BTreeMap::from([
            ("actor_name".into(), runtime.character_name.clone()),
            ("actor_level".into(), runtime.actor_level.to_string()),
            (
                "actor_is_admin".into(),
                condition.actor_is_admin.to_string(),
            ),
            ("world_hour".into(), condition.world_time_hour.to_string()),
            (
                "world_weather".into(),
                match condition.world_weather {
                    Weather::Clear => "clear",
                    Weather::Cloudy => "cloudy",
                    Weather::Rain => "rain",
                    Weather::Storm => "storm",
                    Weather::Snow => "snow",
                    Weather::Fog => "fog",
                }
                .into(),
            ),
        ]),
        seed: 0,
    }
}

fn deterministic_hook_seed(
    base: u64,
    script_id: &str,
    character_id: i64,
    tick: u64,
    hook: ScriptHook,
) -> u64 {
    let mut hash = 0xcbf2_9ce4_8422_2325_u64;
    for byte in script_id.bytes().chain(hook.function_name().bytes()) {
        hash ^= u64::from(byte);
        hash = hash.wrapping_mul(0x100_0000_01b3);
    }
    base ^ hash ^ (character_id as u64).rotate_left(17) ^ tick.rotate_left(31)
}

pub(crate) const fn map_hook(hook: PackScriptHook) -> ScriptHook {
    match hook {
        PackScriptHook::OnEnter => ScriptHook::OnEnter,
        PackScriptHook::OnLeave => ScriptHook::OnLeave,
        PackScriptHook::OnLook => ScriptHook::OnLook,
        PackScriptHook::OnUse => ScriptHook::OnUse,
        PackScriptHook::OnTalk => ScriptHook::OnTalk,
        PackScriptHook::OnCombatStart => ScriptHook::OnCombatStart,
        PackScriptHook::OnDefeat => ScriptHook::OnDefeat,
        PackScriptHook::OnTimer => ScriptHook::OnTimer,
        PackScriptHook::OnQuestEvent => ScriptHook::OnQuestEvent,
    }
}

pub(crate) const fn map_capability(capability: PackScriptCapability) -> ScriptCapability {
    match capability {
        PackScriptCapability::Narrate => ScriptCapability::Narrate,
        PackScriptCapability::SetFlag => ScriptCapability::SetFlag,
        PackScriptCapability::Damage => ScriptCapability::Damage,
        PackScriptCapability::Heal => ScriptCapability::Heal,
        PackScriptCapability::GiveItem => ScriptCapability::GiveItem,
        PackScriptCapability::StartQuest => ScriptCapability::StartQuest,
        PackScriptCapability::Schedule => ScriptCapability::Schedule,
    }
}

fn split_prose(description: &str) -> Vec<String> {
    description
        .replace("\r\n", "\n")
        .replace('\r', "\n")
        .split("\n\n")
        .map(str::trim)
        .filter(|paragraph| !paragraph.is_empty())
        .map(str::to_owned)
        .collect()
}

fn script_error_category(error: &ScriptError) -> &'static str {
    match error {
        ScriptError::SourceTooLarge { .. }
        | ScriptError::ExecutionQuota
        | ScriptError::MemoryQuota
        | ScriptError::EventQuota => "quota",
        ScriptError::CapabilityDenied(_) => "capability",
        ScriptError::MissingHook(_) => "missing_hook",
        ScriptError::InvalidLimits => "invalid_limits",
        ScriptError::Lua(_) => "runtime",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_entity_media(subject_id: &str) -> EntityMediaDefinition {
        let hash = "a".repeat(64);
        EntityMediaDefinition {
            kind: EntityMediaKind::Item,
            subject_id: subject_id.into(),
            alt: world_pack::LocalizedText(BTreeMap::from([(
                "en".into(),
                "A reviewed lantern sprite".into(),
            )])),
            decorative: false,
            footprint: world_pack::NormalizedFootprint {
                width_milli: 180,
                height_milli: 420,
            },
            preferred_anchor: None,
            priority: 10,
            layer: world_pack::EntityMediaLayer::Foreground,
            poses: world_pack::EntitySpritePoses {
                idle: EntitySpriteSource {
                    webp: Some(format!("assets/{hash}/sprite-idle-640.webp")),
                    png: None,
                },
                attack: None,
                hit: None,
                defeated: None,
            },
        }
    }

    #[test]
    fn runtime_delta_is_atomic_when_it_contains_duplicate_media() {
        let mut content = RuntimeContent::default();
        let duplicate = test_entity_media("test:duplicate");
        let error = content
            .install_delta(RuntimeContentDelta::new(
                Vec::new(),
                vec![duplicate.clone(), duplicate],
                Vec::new(),
            ))
            .unwrap_err();

        assert_eq!(
            error,
            RuntimeContentInstallError::EntityMediaOccupied {
                kind: EntityMediaKind::Item,
                subject_id: "test:duplicate".into(),
            }
        );
        assert!(content.entity_media.is_empty());
    }

    #[test]
    fn runtime_delta_rejects_existing_scripts_without_mutating_order() {
        let mut content =
            RuntimeContent::new(Vec::new(), vec![flag_script(2, ScriptHook::OnLook)], 7);
        let before = content
            .scripts
            .iter()
            .map(|script| script.id.clone())
            .collect::<Vec<_>>();

        let error = content
            .install_delta(RuntimeContentDelta::new(
                Vec::new(),
                Vec::new(),
                vec![flag_script(2, ScriptHook::OnLook)],
            ))
            .unwrap_err();

        assert_eq!(
            error,
            RuntimeContentInstallError::ScriptIdCollision("script:bounded-02".into())
        );
        assert_eq!(
            content
                .scripts
                .iter()
                .map(|script| script.id.clone())
                .collect::<Vec<_>>(),
            before
        );
    }

    #[test]
    fn runtime_delta_commits_scripts_in_stable_order() {
        let mut content =
            RuntimeContent::new(Vec::new(), vec![flag_script(2, ScriptHook::OnLook)], 7);

        content
            .install_delta(RuntimeContentDelta::new(
                Vec::new(),
                Vec::new(),
                vec![
                    flag_script(3, ScriptHook::OnLook),
                    flag_script(1, ScriptHook::OnLook),
                ],
            ))
            .unwrap();

        assert_eq!(
            content
                .scripts
                .iter()
                .map(|script| script.id.as_str())
                .collect::<Vec<_>>(),
            vec![
                "script:bounded-01",
                "script:bounded-02",
                "script:bounded-03"
            ]
        );
    }

    #[test]
    fn entity_media_joins_only_visible_opaque_scene_entities() {
        let session = SessionId(88);
        let mut world = World::demo();
        world
            .connect(
                session,
                tiamud_core::CharacterSnapshot {
                    id: tiamud_core::CharacterId(91),
                    account_id: tiamud_core::AccountId(92),
                    name: "Media Observer".into(),
                    room_id: "demo:lantern-square".into(),
                    item_locations: BTreeMap::new(),
                },
            )
            .unwrap();
        let content = RuntimeContent::default()
            .with_entity_media(vec![test_entity_media("demo:blue-lantern")]);
        let scene = world.current_scene(session).unwrap();
        let visible_id = scene.items[0].id.clone();
        let projected = content.resolve_entity_presentation(&world, session);
        let json = serde_json::to_string(&projected).unwrap();
        assert_eq!(projected.entities.len(), 1);
        assert!(json.contains(&visible_id));
        assert!(!json.contains("demo:blue-lantern"));

        world
            .configure_hidden_item("demo:blue-lantern", 100)
            .unwrap();
        let hidden_scene = world.current_scene(session).unwrap();
        assert!(hidden_scene.items.is_empty());
        let hidden = content.resolve_entity_presentation(&world, session);
        assert!(hidden.entities.is_empty());
        assert!(
            !serde_json::to_string(&hidden)
                .unwrap()
                .contains("blue-lantern")
        );
    }

    #[test]
    fn repeated_instances_join_media_by_prototype_without_exposing_either_identity() {
        let prototype_id = "test:lantern";
        let first_instance = "test:lantern-a";
        let second_instance = "test:lantern-b";
        let mut world = World::from_definition(tiamud_core::WorldDefinition {
            start_room_id: "test:room".into(),
            item_prototypes: vec![tiamud_core::ItemPrototypeDefinition {
                prototype_id: prototype_id.into(),
                name: "reviewed lantern".into(),
                description: "A reusable reviewed prototype.".into(),
                aliases: vec!["lantern".into()],
                equipment_slot: None,
                attack_bonus: 0,
                heal_amount: 0,
                consumable: false,
                effect: None,
                weight_grams: 800,
                slot_cost: 1,
                rarity: tiamud_core::ItemRarity::Uncommon,
                item_level: 3,
                stack_policy: tiamud_core::ItemStackPolicy::Unique,
                max_stack_units: 1,
                container: None,
                transfer_policy: tiamud_core::ItemTransferPolicy::Transferable,
                sealed_loot_table_id: None,
            }],
            initial_item_instances: vec![first_instance, second_instance]
                .into_iter()
                .map(|instance_id| tiamud_core::InitialItemInstanceDefinition {
                    instance_id: tiamud_core::ItemInstanceId(instance_id.into()),
                    prototype_id: prototype_id.into(),
                    location: tiamud_core::InitialItemLocation::Room("test:room".into()),
                })
                .collect(),
            rooms: vec![tiamud_core::RoomDefinition {
                stable_id: "test:room".into(),
                title: "Media room".into(),
                prose: vec!["A bounded test room.".into()],
                image: None,
                audio: None,
                exits: Vec::new(),
                item_ids: Vec::new(),
            }],
        })
        .unwrap();
        let session = SessionId(89);
        world
            .connect(
                session,
                tiamud_core::CharacterSnapshot {
                    id: tiamud_core::CharacterId(93),
                    account_id: tiamud_core::AccountId(94),
                    name: "Media Observer".into(),
                    room_id: "test:room".into(),
                    item_locations: BTreeMap::new(),
                },
            )
            .unwrap();

        let scene_ids = world
            .current_scene(session)
            .unwrap()
            .items
            .into_iter()
            .map(|item| item.id)
            .collect::<BTreeSet<_>>();
        let projected = RuntimeContent::default()
            .with_entity_media(vec![test_entity_media(prototype_id)])
            .resolve_entity_presentation(&world, session);
        let projected_ids = projected
            .entities
            .iter()
            .map(|entity| entity.id.clone())
            .collect::<BTreeSet<_>>();
        let json = serde_json::to_string(&projected).unwrap();

        assert_eq!(projected.entities.len(), 2);
        assert_eq!(projected_ids, scene_ids);
        assert!(!json.contains(prototype_id));
        assert!(!json.contains(first_instance));
        assert!(!json.contains(second_instance));
    }

    #[test]
    fn selected_variant_projects_reviewed_alt_and_audio_without_author_metadata() {
        let session = SessionId(8);
        let mut world = World::demo();
        world
            .connect(
                session,
                tiamud_core::CharacterSnapshot {
                    id: tiamud_core::CharacterId(11),
                    account_id: tiamud_core::AccountId(4),
                    name: "Media Test".into(),
                    room_id: "demo:lantern-square".into(),
                    item_locations: BTreeMap::new(),
                },
            )
            .unwrap();
        let digest = "a".repeat(64);
        let content = RuntimeContent::new(
            vec![RoomSceneVariants {
                room_id: "demo:lantern-square".into(),
                variants: vec![world_pack::SceneVariant {
                    id: "author-secret-rain".into(),
                    priority: 0,
                    when: None,
                    title: None,
                    description: None,
                    asset_manifest: Some(format!("assets/{digest}/manifest.json")),
                    image_alt: world_pack::LocalizedText(BTreeMap::from([
                        ("en".into(), "Lantern Square under silver rain.".into()),
                        (
                            "es".into(),
                            "La Plaza del Farol bajo lluvia plateada.".into(),
                        ),
                    ])),
                    audio: Some(world_pack::AudioCue {
                        channel: world_pack::AudioChannel::Ambience,
                        path: format!("assets/{digest}/audio-rain.ogg"),
                        looped: true,
                    }),
                }],
            }],
            Vec::new(),
            1,
        );
        let mut scene = world.current_scene(session).unwrap();
        content.resolve_scene(&world, session, AccountRole::Player, &mut scene);

        assert_eq!(
            scene.image.as_ref().map(|image| image.alt.as_str()),
            Some("Lantern Square under silver rain.")
        );
        assert_eq!(
            scene.audio,
            Some(SceneAudio {
                channel: "ambience".into(),
                url: format!("/assets/{digest}/audio-rain.ogg"),
                looped: true,
            })
        );
        let wire = serde_json::to_string(&scene).unwrap();
        assert!(!wire.contains("author-secret-rain"));

        world
            .configure_room_light("demo:lantern-square", tiamud_core::LightLevel::Dark)
            .unwrap();
        let mut dark_scene = world.current_scene(session).unwrap();
        let protected_title = dark_scene.title.clone();
        let protected_prose = dark_scene.prose.clone();
        content.resolve_scene(&world, session, AccountRole::Player, &mut dark_scene);
        assert_eq!(dark_scene.title, protected_title);
        assert_eq!(dark_scene.prose, protected_prose);
        assert!(dark_scene.image.is_none());
        assert!(dark_scene.audio.is_none());
    }

    #[test]
    fn deterministic_seed_depends_only_on_durable_inputs() {
        let first = deterministic_hook_seed(7, "script:one", 12, 99, ScriptHook::OnEnter);
        assert_eq!(
            first,
            deterministic_hook_seed(7, "script:one", 12, 99, ScriptHook::OnEnter)
        );
        assert_ne!(
            first,
            deterministic_hook_seed(7, "script:one", 12, 100, ScriptHook::OnEnter)
        );
    }

    #[test]
    fn event_validation_accepts_bounded_capabilities_and_rejects_foreign_targets() {
        assert_eq!(
            validate_events(&[
                tiamud_scripting::ScriptEvent {
                    kind: "damage".into(),
                    target: Some("self".into()),
                    value: Some("1".into()),
                },
                tiamud_scripting::ScriptEvent {
                    kind: "schedule".into(),
                    target: Some("self".into()),
                    value: Some("2:later".into()),
                },
            ])
            .map(|events| events.len()),
            Some(2)
        );
        assert!(
            validate_events(&[tiamud_scripting::ScriptEvent {
                kind: "narrate".into(),
                target: Some("another-character".into()),
                value: Some("secret".into()),
            }])
            .is_none()
        );
        assert!(
            validate_events(&[tiamud_scripting::ScriptEvent {
                kind: "heal".into(),
                target: Some("self".into()),
                value: Some("0".into()),
            }])
            .is_none()
        );
    }

    fn script_test_world() -> (World, SessionId) {
        let session = SessionId(41);
        let mut world = World::demo();
        world
            .connect(
                session,
                tiamud_core::CharacterSnapshot {
                    id: tiamud_core::CharacterId(41),
                    account_id: tiamud_core::AccountId(4),
                    name: "Budget Test".into(),
                    room_id: "demo:lantern-square".into(),
                    item_locations: BTreeMap::new(),
                },
            )
            .unwrap();
        (world, session)
    }

    fn flag_script(index: usize, hook: ScriptHook) -> RuntimeScript {
        RuntimeScript {
            id: format!("script:bounded-{index:02}"),
            room_id: "demo:lantern-square".into(),
            hooks: BTreeSet::from([hook]),
            capabilities: BTreeSet::from([ScriptCapability::SetFlag]),
            limits: ScriptLimits::default(),
            source: format!(
                "function {}() tiamud.emit('set_flag', 'self', 'ran-{index}') end",
                hook.function_name()
            ),
        }
    }

    #[test]
    fn room_hook_batch_rejects_more_than_eight_modules_before_starting_any() {
        let (mut world, session) = script_test_world();
        let scripts = (0..=MAX_MODULES_PER_ROOM_HOOK_BATCH)
            .map(|index| flag_script(index, ScriptHook::OnLook))
            .collect();
        let content = RuntimeContent::new(Vec::new(), scripts, 7);

        let outcome =
            content.run_hook(&mut world, session, AccountRole::Player, ScriptHook::OnLook);

        assert!(!outcome.changed);
        assert!(outcome.narration.is_empty());
        assert!(
            world
                .character_gameplay_snapshot(session)
                .unwrap()
                .script_flags
                .is_empty()
        );
    }

    #[test]
    fn exhausted_aggregate_budget_is_inert_and_reported() {
        let (mut world, session) = script_test_world();
        let metrics = Arc::new(RuntimeMetrics::default());
        let content = RuntimeContent::new(Vec::new(), vec![flag_script(1, ScriptHook::OnTimer)], 7)
            .with_operational_metrics(metrics.clone());

        let outcome = content.run_hook_with_budget(
            &mut world,
            session,
            AccountRole::Player,
            ScriptHook::OnTimer,
            Duration::ZERO,
        );

        assert!(outcome.budget_exhausted);
        assert!(!outcome.changed);
        assert!(
            world
                .character_gameplay_snapshot(session)
                .unwrap()
                .script_flags
                .is_empty()
        );
        assert!(
            metrics
                .prometheus(0)
                .contains("tiamud_script_timeouts_total 1")
        );
    }

    #[test]
    fn runtime_applies_every_declared_capability_atomically() {
        let session = SessionId(9);
        let mut world = World::demo();
        world
            .connect(
                session,
                tiamud_core::CharacterSnapshot {
                    id: tiamud_core::CharacterId(12),
                    account_id: tiamud_core::AccountId(4),
                    name: "Builder Test".into(),
                    room_id: "demo:lantern-square".into(),
                    item_locations: BTreeMap::new(),
                },
            )
            .unwrap();
        let before = world.player_state_view(session).unwrap();
        let content = RuntimeContent::new(
            Vec::new(),
            vec![RuntimeScript {
                id: "script:all-capabilities".into(),
                room_id: "demo:lantern-square".into(),
                hooks: BTreeSet::from([ScriptHook::OnLook]),
                capabilities: BTreeSet::from([
                    ScriptCapability::Narrate,
                    ScriptCapability::SetFlag,
                    ScriptCapability::Damage,
                    ScriptCapability::Heal,
                    ScriptCapability::GiveItem,
                    ScriptCapability::StartQuest,
                    ScriptCapability::Schedule,
                ]),
                limits: ScriptLimits::default(),
                source: r#"
                    function on_look()
                        tiamud.emit('narrate', 'self', 'The archive answers.')
                        tiamud.emit('set_flag', 'self', 'archive-open')
                        tiamud.emit('damage', 'self', '3')
                        tiamud.emit('heal', 'self', '1')
                        tiamud.emit('give_item', 'self', 'demo:blue-lantern')
                        tiamud.emit('start_quest', 'self', 'first-hunt')
                        tiamud.emit('schedule', 'self', '2:archive-follow-up')
                    end
                "#
                .into(),
            }],
            7,
        );

        let outcome =
            content.run_hook(&mut world, session, AccountRole::Player, ScriptHook::OnLook);
        let after = world.player_state_view(session).unwrap();
        let durable = world.character_gameplay_snapshot(session).unwrap();
        assert!(outcome.changed);
        assert_eq!(outcome.narration, vec!["The archive answers."]);
        assert_eq!(outcome.domain_events.len(), 2);
        assert_eq!(after.character.health, before.character.health - 2);
        assert_eq!(after.inventory.len(), 1);
        assert_eq!(after.quests.len(), 1);
        assert!(durable.script_flags.contains("archive-open"));
        assert_eq!(durable.scheduled_script_flags.len(), 1);
        world.advance_tick().unwrap();
        world.advance_tick().unwrap();
        let due = world.character_gameplay_snapshot(session).unwrap();
        assert!(due.script_flags.contains("archive-follow-up"));
        assert!(due.scheduled_script_flags.is_empty());
    }
}
