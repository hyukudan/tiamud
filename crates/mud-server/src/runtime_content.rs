use std::{
    collections::{BTreeMap, BTreeSet, HashMap},
    sync::Arc,
};

use tiamud_core::{
    AccountRole, AuthorizedRuntimeContext, DomainEvent, PlayerAction, QuestAction, SceneAudio,
    SceneImage, SceneView, SessionId, WeatherKind, World,
};
use tiamud_scripting::{
    ScriptCapability, ScriptContext, ScriptError, ScriptHook, ScriptLimits, execute_hook,
};
use world_pack::{
    ConditionContext, RoomSceneVariants, ScriptCapability as PackScriptCapability,
    ScriptDefinition, ScriptHook as PackScriptHook, TimePeriod, Weather,
};

use crate::operations::{RuntimeMetrics, ScriptMetricOutcome};

#[derive(Debug)]
pub(crate) struct RuntimeContent {
    variants: HashMap<String, RoomSceneVariants>,
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
            scripts,
            seed,
            operational_metrics: None,
        }
    }

    pub(crate) fn with_operational_metrics(mut self, metrics: Arc<RuntimeMetrics>) -> Self {
        self.operational_metrics = Some(metrics);
        self
    }

    pub(crate) fn operational_metrics(&self) -> Option<Arc<RuntimeMetrics>> {
        self.operational_metrics.as_ref().map(Arc::clone)
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
        let Ok(runtime) = world.authorized_runtime_context(session) else {
            return ScriptBatchOutcome::default();
        };
        let scripts = self
            .scripts
            .iter()
            .filter(|script| script.room_id == runtime.room_id && script.hooks.contains(&hook))
            .collect::<Vec<_>>();
        if scripts.is_empty() {
            return ScriptBatchOutcome::default();
        }

        let context = script_context(&runtime, role);
        let mut pending = Vec::new();
        for script in scripts {
            let mut context = context.clone();
            context.seed = deterministic_hook_seed(
                self.seed,
                &script.id,
                runtime.character_id.0,
                runtime.logical_tick,
                hook,
            );
            match execute_hook(
                &script.source,
                hook,
                &context,
                &script.capabilities,
                script.limits,
            ) {
                Ok(outcome) => {
                    if let Some(metrics) = &self.operational_metrics {
                        metrics.script_finished(outcome.elapsed, ScriptMetricOutcome::Success);
                    }
                    pending.extend(outcome.events);
                }
                Err(error) => {
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
                    return ScriptBatchOutcome::default();
                }
            }
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
            return ScriptBatchOutcome::default();
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
                            return ScriptBatchOutcome::default();
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
        result
    }
}

#[derive(Debug)]
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
    ScriptBatchOutcome::default()
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
