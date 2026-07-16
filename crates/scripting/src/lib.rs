//! Quota-limited Lua 5.4 hooks for trusted WorldPack authoring.
//!
//! Every execution owns a fresh VM with no filesystem, network, process,
//! package loader, native-module loader, or debug library. Scripts can affect
//! the host only by emitting allow-listed events through explicit capabilities.

use std::collections::{BTreeMap, BTreeSet};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use mlua::{ChunkMode, Error as LuaError, HookTriggers, Lua, LuaOptions, StdLib, VmState};
use serde::{Deserialize, Serialize};
use thiserror::Error;

const HOOK_GRANULARITY: u32 = 100;

/// Shared authoring/runtime ceiling for modules attached to the same room and
/// hook. WorldPack validation and the server both enforce this value.
pub const MAX_MODULES_PER_ROOM_HOOK_BATCH: usize = 8;

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ScriptHook {
    OnEnter,
    OnLeave,
    OnLook,
    OnUse,
    OnTalk,
    OnCombatStart,
    OnDefeat,
    OnTimer,
    OnQuestEvent,
}

impl ScriptHook {
    pub const fn function_name(self) -> &'static str {
        match self {
            Self::OnEnter => "on_enter",
            Self::OnLeave => "on_leave",
            Self::OnLook => "on_look",
            Self::OnUse => "on_use",
            Self::OnTalk => "on_talk",
            Self::OnCombatStart => "on_combat_start",
            Self::OnDefeat => "on_defeat",
            Self::OnTimer => "on_timer",
            Self::OnQuestEvent => "on_quest_event",
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ScriptCapability {
    Narrate,
    SetFlag,
    Damage,
    Heal,
    GiveItem,
    StartQuest,
    Schedule,
}

impl ScriptCapability {
    fn event_kind(self) -> &'static str {
        match self {
            Self::Narrate => "narrate",
            Self::SetFlag => "set_flag",
            Self::Damage => "damage",
            Self::Heal => "heal",
            Self::GiveItem => "give_item",
            Self::StartQuest => "start_quest",
            Self::Schedule => "schedule",
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ScriptEvent {
    pub kind: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub target: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub value: Option<String>,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct ScriptContext {
    pub values: BTreeMap<String, String>,
    pub seed: u64,
}

#[derive(Clone, Copy, Debug)]
pub struct ScriptLimits {
    pub max_source_bytes: usize,
    pub max_memory_bytes: usize,
    pub max_instructions: u64,
    pub max_wall_time: Duration,
    pub max_events: usize,
    pub max_event_field_bytes: usize,
}

impl Default for ScriptLimits {
    fn default() -> Self {
        Self {
            max_source_bytes: 64 * 1024,
            max_memory_bytes: 2 * 1024 * 1024,
            max_instructions: 100_000,
            max_wall_time: Duration::from_millis(25),
            max_events: 32,
            max_event_field_bytes: 1024,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ScriptOutcome {
    pub events: Vec<ScriptEvent>,
    pub instructions: u64,
    pub memory_bytes: usize,
    pub elapsed: Duration,
}

#[derive(Debug, Error)]
pub enum ScriptError {
    #[error("script source is {actual} bytes, exceeding the {maximum}-byte limit")]
    SourceTooLarge { actual: usize, maximum: usize },
    #[error("script limits must all be non-zero")]
    InvalidLimits,
    #[error("Lua hook {0} is not defined")]
    MissingHook(&'static str),
    #[error("script exceeded its instruction or wall-clock quota")]
    ExecutionQuota,
    #[error("script exceeded its memory quota")]
    MemoryQuota,
    #[error("script attempted a capability it was not granted: {0}")]
    CapabilityDenied(String),
    #[error("script emitted too many events or an oversized event field")]
    EventQuota,
    #[error("script failed: {0}")]
    Lua(String),
}

/// Compile and initialize a module under the same sandbox and quotas used by
/// gameplay, without invoking any hook. This makes malformed WorldPacks fail
/// closed at load time while keeping hook effects inert.
pub fn validate_module(
    source: &str,
    hooks: &BTreeSet<ScriptHook>,
    limits: ScriptLimits,
) -> Result<(), ScriptError> {
    validate_limits(source, limits)?;
    let started = Instant::now();
    let lua = Lua::new_with(
        StdLib::TABLE | StdLib::STRING | StdLib::MATH,
        LuaOptions::default(),
    )
    .map_err(map_lua_error)?;
    lua.set_memory_limit(limits.max_memory_bytes)
        .map_err(map_lua_error)?;
    let instructions = Arc::new(Mutex::new(0_u64));
    let hook_instructions = Arc::clone(&instructions);
    lua.set_hook(
        HookTriggers::new().every_nth_instruction(HOOK_GRANULARITY),
        move |_lua, _debug| {
            let mut consumed = hook_instructions
                .lock()
                .map_err(|_| LuaError::RuntimeError("quota state poisoned".to_owned()))?;
            *consumed = consumed.saturating_add(u64::from(HOOK_GRANULARITY));
            if *consumed > limits.max_instructions || started.elapsed() > limits.max_wall_time {
                return Err(LuaError::RuntimeError("TIAMUD_EXECUTION_QUOTA".to_owned()));
            }
            Ok(VmState::Continue)
        },
    )
    .map_err(map_lua_error)?;
    install_capability_api(&lua, 0, &BTreeSet::new(), limits)?;
    lua.load(source)
        .set_name("@worldpack")
        .set_mode(ChunkMode::Text)
        .exec()
        .map_err(map_lua_error)?;
    for hook in hooks {
        if lua
            .globals()
            .get::<Option<mlua::Function>>(hook.function_name())
            .map_err(map_lua_error)?
            .is_none()
        {
            return Err(ScriptError::MissingHook(hook.function_name()));
        }
    }
    Ok(())
}

/// Execute one named hook in a fresh, isolated Lua VM.
pub fn execute_hook(
    source: &str,
    hook: ScriptHook,
    context: &ScriptContext,
    capabilities: &BTreeSet<ScriptCapability>,
    limits: ScriptLimits,
) -> Result<ScriptOutcome, ScriptError> {
    validate_limits(source, limits)?;
    let started = Instant::now();
    let lua = Lua::new_with(
        StdLib::TABLE | StdLib::STRING | StdLib::MATH,
        LuaOptions::default(),
    )
    .map_err(map_lua_error)?;
    lua.set_memory_limit(limits.max_memory_bytes)
        .map_err(map_lua_error)?;

    let instructions = Arc::new(Mutex::new(0_u64));
    let hook_instructions = Arc::clone(&instructions);
    lua.set_hook(
        HookTriggers::new().every_nth_instruction(HOOK_GRANULARITY),
        move |_lua, _debug| {
            let mut consumed = hook_instructions
                .lock()
                .map_err(|_| LuaError::RuntimeError("quota state poisoned".to_owned()))?;
            *consumed = consumed.saturating_add(u64::from(HOOK_GRANULARITY));
            if *consumed > limits.max_instructions || started.elapsed() > limits.max_wall_time {
                return Err(LuaError::RuntimeError("TIAMUD_EXECUTION_QUOTA".to_owned()));
            }
            Ok(VmState::Continue)
        },
    )
    .map_err(map_lua_error)?;

    install_capability_api(&lua, context.seed, capabilities, limits)?;
    lua.load(source)
        .set_name("@worldpack")
        .set_mode(ChunkMode::Text)
        .exec()
        .map_err(map_lua_error)?;

    let function = lua
        .globals()
        .get::<Option<mlua::Function>>(hook.function_name())
        .map_err(map_lua_error)?
        .ok_or(ScriptError::MissingHook(hook.function_name()))?;
    let context_table = lua.create_table().map_err(map_lua_error)?;
    for (key, value) in &context.values {
        validate_field(key, limits.max_event_field_bytes)?;
        validate_field(value, limits.max_event_field_bytes)?;
        context_table
            .set(key.as_str(), value.as_str())
            .map_err(map_lua_error)?;
    }
    function.call::<()>(context_table).map_err(map_lua_error)?;

    let events = lua
        .app_data_ref::<SharedEvents>()
        .ok_or_else(|| ScriptError::Lua("event buffer missing".to_owned()))?
        .0
        .lock()
        .map_err(|_| ScriptError::Lua("event buffer poisoned".to_owned()))?
        .clone();
    let consumed = *instructions
        .lock()
        .map_err(|_| ScriptError::Lua("quota state poisoned".to_owned()))?;
    Ok(ScriptOutcome {
        events,
        instructions: consumed,
        memory_bytes: lua.used_memory(),
        elapsed: started.elapsed(),
    })
}

#[derive(Clone)]
struct SharedEvents(Arc<Mutex<Vec<ScriptEvent>>>);

fn install_capability_api(
    lua: &Lua,
    seed: u64,
    capabilities: &BTreeSet<ScriptCapability>,
    limits: ScriptLimits,
) -> Result<(), ScriptError> {
    for forbidden in [
        "dofile",
        "load",
        "loadfile",
        "require",
        "print",
        "warn",
        "collectgarbage",
    ] {
        lua.globals()
            .set(forbidden, mlua::Value::Nil)
            .map_err(map_lua_error)?;
    }
    harden_standard_libraries(lua)?;
    let api = lua.create_table().map_err(map_lua_error)?;
    let allowed = capabilities
        .iter()
        .map(|capability| capability.event_kind().to_owned())
        .collect::<BTreeSet<_>>();
    let events = Arc::new(Mutex::new(Vec::new()));
    lua.set_app_data(SharedEvents(Arc::clone(&events)));
    api.set(
        "emit",
        lua.create_function(
            move |_lua, (kind, target, value): (String, Option<String>, Option<String>)| {
                if !allowed.contains(&kind) {
                    return Err(LuaError::RuntimeError(format!(
                        "TIAMUD_CAPABILITY_DENIED:{kind}"
                    )));
                }
                if kind.len() > limits.max_event_field_bytes
                    || target
                        .as_ref()
                        .is_some_and(|field| field.len() > limits.max_event_field_bytes)
                    || value
                        .as_ref()
                        .is_some_and(|field| field.len() > limits.max_event_field_bytes)
                {
                    return Err(LuaError::RuntimeError("TIAMUD_EVENT_QUOTA".to_owned()));
                }
                let mut events = events
                    .lock()
                    .map_err(|_| LuaError::RuntimeError("event buffer poisoned".to_owned()))?;
                if events.len() >= limits.max_events {
                    return Err(LuaError::RuntimeError("TIAMUD_EVENT_QUOTA".to_owned()));
                }
                events.push(ScriptEvent {
                    kind,
                    target,
                    value,
                });
                Ok(())
            },
        )
        .map_err(map_lua_error)?,
    )
    .map_err(map_lua_error)?;

    let random_state = Arc::new(Mutex::new(seed));
    api.set(
        "random",
        lua.create_function(move |_lua, maximum: u32| {
            if maximum == 0 {
                return Err(LuaError::RuntimeError(
                    "random maximum must be positive".to_owned(),
                ));
            }
            let mut state = random_state
                .lock()
                .map_err(|_| LuaError::RuntimeError("random state poisoned".to_owned()))?;
            *state = state
                .wrapping_mul(6_364_136_223_846_793_005)
                .wrapping_add(1_442_695_040_888_963_407);
            Ok(((*state >> 32) as u32 % maximum) + 1)
        })
        .map_err(map_lua_error)?,
    )
    .map_err(map_lua_error)?;
    lua.globals().set("tiamud", api).map_err(map_lua_error)
}

/// Remove standard-library entry points that bypass the sandbox's guarantees.
///
/// Lua's instruction hook cannot interrupt a long-running pattern operation
/// while the VM is executing its C implementation. Packs do not need pattern
/// matching for gameplay hooks, so fail closed instead of pretending that the
/// wall-clock quota can preempt those calls. Likewise, all authored randomness
/// must flow through the seeded `tiamud.random` capability.
fn harden_standard_libraries(lua: &Lua) -> Result<(), ScriptError> {
    let string: mlua::Table = lua.globals().get("string").map_err(map_lua_error)?;
    for function in ["find", "gmatch", "gsub", "match"] {
        string
            .set(function, mlua::Value::Nil)
            .map_err(map_lua_error)?;
    }

    let math: mlua::Table = lua.globals().get("math").map_err(map_lua_error)?;
    for function in ["random", "randomseed"] {
        math.set(function, mlua::Value::Nil)
            .map_err(map_lua_error)?;
    }
    Ok(())
}

fn validate_limits(source: &str, limits: ScriptLimits) -> Result<(), ScriptError> {
    if source.len() > limits.max_source_bytes {
        return Err(ScriptError::SourceTooLarge {
            actual: source.len(),
            maximum: limits.max_source_bytes,
        });
    }
    if limits.max_source_bytes == 0
        || limits.max_memory_bytes == 0
        || limits.max_instructions == 0
        || limits.max_wall_time.is_zero()
        || limits.max_events == 0
        || limits.max_event_field_bytes == 0
    {
        return Err(ScriptError::InvalidLimits);
    }
    Ok(())
}

fn validate_field(value: &str, maximum: usize) -> Result<(), ScriptError> {
    if value.len() > maximum || value.chars().any(|character| character == '\0') {
        return Err(ScriptError::EventQuota);
    }
    Ok(())
}

fn map_lua_error(error: LuaError) -> ScriptError {
    let message = error.to_string();
    if message.contains("TIAMUD_EXECUTION_QUOTA") {
        ScriptError::ExecutionQuota
    } else if message.contains("memory error") || message.contains("not enough memory") {
        ScriptError::MemoryQuota
    } else if let Some((_, capability)) = message.split_once("TIAMUD_CAPABILITY_DENIED:") {
        ScriptError::CapabilityDenied(
            capability
                .lines()
                .next()
                .unwrap_or("unknown")
                .trim()
                .to_owned(),
        )
    } else if message.contains("TIAMUD_EVENT_QUOTA") {
        ScriptError::EventQuota
    } else {
        ScriptError::Lua(message)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn capabilities(values: &[ScriptCapability]) -> BTreeSet<ScriptCapability> {
        values.iter().copied().collect()
    }

    #[test]
    fn emits_only_explicit_capabilities_and_has_no_os_or_package_api() {
        let outcome = execute_hook(
            r#"
                function on_enter(ctx)
                    assert(os == nil and io == nil and package == nil and require == nil)
                    assert(print == nil and warn == nil and collectgarbage == nil)
                    tiamud.emit("narrate", ctx.character, "Welcome")
                end
            "#,
            ScriptHook::OnEnter,
            &ScriptContext {
                values: BTreeMap::from([("character".to_owned(), "opaque-1".to_owned())]),
                seed: 7,
            },
            &capabilities(&[ScriptCapability::Narrate]),
            ScriptLimits::default(),
        )
        .unwrap();
        assert_eq!(outcome.events.len(), 1);
        assert_eq!(outcome.events[0].target.as_deref(), Some("opaque-1"));
    }

    #[test]
    fn rejects_denied_capabilities_and_binary_loading() {
        let denied = execute_hook(
            "function on_use() tiamud.emit('give_item', 'player', 'sword') end",
            ScriptHook::OnUse,
            &ScriptContext::default(),
            &capabilities(&[ScriptCapability::Narrate]),
            ScriptLimits::default(),
        );
        assert!(matches!(denied, Err(ScriptError::CapabilityDenied(_))));

        let binary = execute_hook(
            "\x1bLua",
            ScriptHook::OnEnter,
            &ScriptContext::default(),
            &BTreeSet::new(),
            ScriptLimits::default(),
        );
        assert!(matches!(binary, Err(ScriptError::Lua(_))));
    }

    #[test]
    fn module_validation_requires_every_declared_hook_without_running_it() {
        let hooks = BTreeSet::from([ScriptHook::OnEnter, ScriptHook::OnLeave]);
        let error = validate_module(
            "function on_enter() error('must not run') end",
            &hooks,
            ScriptLimits::default(),
        )
        .unwrap_err();
        assert!(matches!(error, ScriptError::MissingHook("on_leave")));

        validate_module(
            "function on_enter() error('must not run') end function on_leave() end",
            &hooks,
            ScriptLimits::default(),
        )
        .unwrap();
    }

    #[test]
    fn stops_infinite_loops_and_event_chains() {
        let limited = ScriptLimits {
            max_instructions: 1_000,
            max_wall_time: Duration::from_secs(1),
            max_events: 2,
            ..ScriptLimits::default()
        };
        assert!(matches!(
            execute_hook(
                "function on_timer() while true do end end",
                ScriptHook::OnTimer,
                &ScriptContext::default(),
                &BTreeSet::new(),
                limited,
            ),
            Err(ScriptError::ExecutionQuota)
        ));
        assert!(matches!(
            execute_hook(
                "function on_timer() for i=1,3 do tiamud.emit('narrate', nil, 'x') end end",
                ScriptHook::OnTimer,
                &ScriptContext::default(),
                &capabilities(&[ScriptCapability::Narrate]),
                limited,
            ),
            Err(ScriptError::EventQuota)
        ));
    }

    #[test]
    fn random_is_deterministic_for_a_controlled_seed() {
        let source = r#"
            function on_look()
                tiamud.emit("narrate", nil, tostring(tiamud.random(100)))
            end
        "#;
        let run = || {
            execute_hook(
                source,
                ScriptHook::OnLook,
                &ScriptContext {
                    values: BTreeMap::new(),
                    seed: 42,
                },
                &capabilities(&[ScriptCapability::Narrate]),
                ScriptLimits::default(),
            )
            .unwrap()
            .events
        };
        assert_eq!(run(), run());
    }

    #[test]
    fn removes_nondeterministic_randomness_and_uninterruptible_patterns() {
        let outcome = execute_hook(
            r#"
                function on_look()
                    assert(math.random == nil and math.randomseed == nil)
                    assert(string.find == nil and string.gmatch == nil)
                    assert(string.gsub == nil and string.match == nil)
                    tiamud.emit("narrate", nil, string.upper("safe"))
                end
            "#,
            ScriptHook::OnLook,
            &ScriptContext::default(),
            &capabilities(&[ScriptCapability::Narrate]),
            ScriptLimits::default(),
        )
        .unwrap();

        assert_eq!(outcome.events[0].value.as_deref(), Some("SAFE"));
    }

    #[test]
    fn enforces_source_and_memory_bounds() {
        let tiny_source = ScriptLimits {
            max_source_bytes: 4,
            ..ScriptLimits::default()
        };
        assert!(matches!(
            execute_hook(
                "12345",
                ScriptHook::OnEnter,
                &ScriptContext::default(),
                &BTreeSet::new(),
                tiny_source
            ),
            Err(ScriptError::SourceTooLarge { .. })
        ));

        let tiny_memory = ScriptLimits {
            max_memory_bytes: 64 * 1024,
            ..ScriptLimits::default()
        };
        let result = execute_hook(
            "function on_enter() local x = string.rep('x', 200000) end",
            ScriptHook::OnEnter,
            &ScriptContext::default(),
            &BTreeSet::new(),
            tiny_memory,
        );
        assert!(matches!(result, Err(ScriptError::MemoryQuota)));
    }
}
