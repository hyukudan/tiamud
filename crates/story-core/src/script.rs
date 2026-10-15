//! Optional script hook points (phase S6, ADR 0034).
//!
//! `story-core` never runs a script itself and stays free of any scripting
//! dependency. A pack may declare `scripts` and reference them with
//! [`Condition::Script`](crate::Condition::Script) and
//! [`Effect::Script`](crate::Effect::Script); those references are evaluated
//! through a [`ScriptHost`] that the caller passes to the `*_scripted`
//! functions ([`crate::step_scripted`], [`crate::view_scripted`], …). The
//! sibling crate `tiamud-story-script` implements the host with the existing
//! quota-limited Lua sandbox.
//!
//! A script is a pure function of a bounded, read-only [`ScriptSnapshot`]:
//!
//! - a condition script returns a boolean and may emit nothing;
//! - an effect script returns a list of ordinary closed [`Effect`] intents,
//!   which this module checks against the script's declared grants before the
//!   engine applies them exactly like authored effects.
//!
//! Scripts therefore never mutate state and are never a second authority.
//! Without a host, every script reference fails closed with
//! [`StoryError::ScriptUnavailable`], and [`crate::StoryPack::validate`]
//! rejects packs that declare scripts at all.

use std::cell::Cell;
use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};

use crate::rng::SplitMix64;
use crate::world::{Location, ObjectFlag, ObjectState};
use crate::{
    Effect, MAX_SCORE_DELTA, MAX_SCRIPT_CALLS_PER_STEP, MAX_SCRIPT_EFFECTS, MAX_VAR_ABS,
    StoryError, StoryPack, StoryState, Value, VariableKind, bounded_message, valid_id,
};

/// What a declared script computes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ScriptKind {
    /// Lua function `check(story, args)` returning a boolean. Pure: it takes
    /// no grants and may emit nothing.
    Condition,
    /// Lua function `run(story, args)` that emits effect intents through the
    /// `fx` helpers; only what its grants allow is accepted.
    Effect,
}

/// A script declared by a pack (format v3, optional `scripts` section). The
/// grant lists are the only things its emitted intents may touch.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ScriptDecl {
    /// Durable id referenced by `{"script": {"id": …}}`.
    pub id: String,
    pub kind: ScriptKind,
    /// Lua 5.4 source text. Compiled by the host, never by `story-core`.
    pub source: String,
    /// Variables the script may `set` or `add`.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub writes: Vec<String>,
    /// Items the script may `give` or `take`.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub items: Vec<String>,
    /// Nodes the script may `goto`. They count as exits for reachability.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub goto: Vec<String>,
    /// World objects the script may move or flag (format v3 worlds).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub objects: Vec<String>,
    /// True when the script may change the score.
    #[serde(default, skip_serializing_if = "is_false")]
    pub score: bool,
    /// True when the script may end the story.
    #[serde(default, skip_serializing_if = "is_false")]
    pub ends: bool,
}

impl ScriptDecl {
    /// True when the declaration grants anything.
    pub(crate) fn has_grants(&self) -> bool {
        !self.writes.is_empty()
            || !self.items.is_empty()
            || !self.goto.is_empty()
            || !self.objects.is_empty()
            || self.score
            || self.ends
    }
}

/// A use of a declared script with bounded constant arguments.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ScriptRef {
    pub id: String,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub args: BTreeMap<String, Value>,
}

/// Bounded, read-only projection of a [`StoryState`] handed to a script. It
/// is a copy: nothing a script does to it reaches the state. The RNG state,
/// transcript, menu counters and undo history are never exposed.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ScriptSnapshot {
    pub node: String,
    pub turn: u32,
    pub score: i64,
    /// Every variable, integers and booleans ("flags") alike.
    pub vars: BTreeMap<String, Value>,
    pub items: BTreeSet<String>,
    pub visited: BTreeSet<String>,
    /// World objects with their location and flags (format v3).
    pub objects: BTreeMap<String, ObjectState>,
}

impl ScriptSnapshot {
    pub(crate) fn of(state: &StoryState) -> Self {
        Self {
            node: state.node.clone(),
            turn: state.turn,
            score: state.score,
            vars: state.vars.clone(),
            items: state.inventory.clone(),
            visited: state.visited.clone(),
            objects: state.objects.clone(),
        }
    }
}

/// One script invocation as a host sees it.
#[derive(Debug, Clone, Copy)]
pub struct ScriptCall<'a> {
    pub script: &'a ScriptDecl,
    pub args: &'a BTreeMap<String, Value>,
    pub snapshot: &'a ScriptSnapshot,
    /// Deterministic seed for the script's own randomness. Effect scripts
    /// draw it from the story RNG (which therefore advances); condition
    /// scripts derive it from the state without advancing anything, so a
    /// view and the following step always agree.
    pub seed: u64,
}

/// An author-facing diagnostic from a host: compile errors, Lua runtime
/// errors with their line, quota exhaustion or malformed intents. Players
/// never see it; transports map every script failure to a generic error.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScriptFailure {
    pub message: String,
}

impl ScriptFailure {
    pub fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
        }
    }
}

/// Executes declared scripts for the engine. Implementations must be
/// deterministic functions of the call (no clock, entropy or I/O) and must
/// bound their own time and memory.
pub trait ScriptHost {
    /// Evaluates a condition script.
    fn check(&self, call: &ScriptCall<'_>) -> Result<bool, ScriptFailure>;
    /// Runs an effect script and returns the intents it emitted, in order.
    fn run(&self, call: &ScriptCall<'_>) -> Result<Vec<Effect>, ScriptFailure>;
}

/// Per-call script context: the optional host and the budget of script
/// invocations shared by one step or one view.
pub(crate) struct Scripts<'a> {
    host: Option<&'a dyn ScriptHost>,
    calls: Cell<usize>,
}

impl<'a> Scripts<'a> {
    pub(crate) const fn none() -> Self {
        Self {
            host: None,
            calls: Cell::new(0),
        }
    }

    pub(crate) fn with(host: &'a dyn ScriptHost) -> Self {
        Self {
            host: Some(host),
            calls: Cell::new(0),
        }
    }

    fn begin<'p>(
        &self,
        pack: &'p StoryPack,
        script: &ScriptRef,
        kind: ScriptKind,
    ) -> Result<(&'a dyn ScriptHost, &'p ScriptDecl), StoryError> {
        let decl = pack
            .script(&script.id)
            .filter(|decl| decl.kind == kind)
            .ok_or_else(|| {
                StoryError::InvalidPack("reference names an unknown script".to_owned())
            })?;
        let host = self.host.ok_or(StoryError::ScriptUnavailable)?;
        let used = self.calls.get();
        if used >= MAX_SCRIPT_CALLS_PER_STEP {
            return Err(StoryError::ScriptBudget {
                limit: MAX_SCRIPT_CALLS_PER_STEP,
            });
        }
        self.calls.set(used + 1);
        Ok((host, decl))
    }

    /// Evaluates a condition script against `state`.
    pub(crate) fn check(
        &self,
        pack: &StoryPack,
        state: &StoryState,
        script: &ScriptRef,
    ) -> Result<bool, StoryError> {
        let (host, decl) = self.begin(pack, script, ScriptKind::Condition)?;
        let snapshot = ScriptSnapshot::of(state);
        let call = ScriptCall {
            script: decl,
            args: &script.args,
            snapshot: &snapshot,
            seed: condition_seed(state, &decl.id),
        };
        host.check(&call).map_err(|failure| failed(decl, failure))
    }

    /// Runs an effect script and returns its intents after checking every
    /// one against the declaration's grants.
    pub(crate) fn run(
        &self,
        pack: &StoryPack,
        state: &StoryState,
        script: &ScriptRef,
        seed: u64,
    ) -> Result<Vec<Effect>, StoryError> {
        let (host, decl) = self.begin(pack, script, ScriptKind::Effect)?;
        let snapshot = ScriptSnapshot::of(state);
        let call = ScriptCall {
            script: decl,
            args: &script.args,
            snapshot: &snapshot,
            seed,
        };
        let effects = host.run(&call).map_err(|failure| failed(decl, failure))?;
        if effects.len() > MAX_SCRIPT_EFFECTS {
            return Err(rejected(
                decl,
                format!("emitted more than {MAX_SCRIPT_EFFECTS} effects"),
            ));
        }
        for (index, effect) in effects.iter().enumerate() {
            check_intent(pack, decl, effect)
                .map_err(|message| rejected(decl, format!("effect {}: {message}", index + 1)))?;
        }
        Ok(effects)
    }
}

fn failed(decl: &ScriptDecl, failure: ScriptFailure) -> StoryError {
    StoryError::Script {
        script: decl.id.clone(),
        message: bounded_message(failure.message),
    }
}

fn rejected(decl: &ScriptDecl, message: String) -> StoryError {
    StoryError::Script {
        script: decl.id.clone(),
        message: bounded_message(message),
    }
}

/// Seed of a condition script: a pure function of the state's RNG, turn and
/// the script id, so re-evaluating the same state always agrees.
pub(crate) fn condition_seed(state: &StoryState, id: &str) -> u64 {
    // FNV-1a over the id; any fixed mixing works, this one is local and stable.
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for byte in id.bytes() {
        hash ^= u64::from(byte);
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    SplitMix64::from_state(state.rng ^ hash ^ u64::from(state.turn)).next_u64()
}

/// Checks one emitted intent: a closed subset of [`Effect`], inside the
/// declaration's grants and every DSL bound.
fn check_intent(pack: &StoryPack, decl: &ScriptDecl, effect: &Effect) -> Result<(), String> {
    let granted = |list: &[String], id: &str| list.iter().any(|entry| entry == id);
    match effect {
        Effect::Set { var, value } => {
            if !granted(&decl.writes, var) {
                return Err(format!("variable {var:?} is not in `writes`"));
            }
            match (pack.variable(var).map(|decl| &decl.kind), value) {
                (Some(VariableKind::Int { .. }), Value::Int(value)) => {
                    if value.unsigned_abs() > MAX_VAR_ABS.unsigned_abs() {
                        return Err("set value is out of range".to_owned());
                    }
                }
                (Some(VariableKind::Bool { .. }), Value::Bool(_)) => {}
                _ => return Err(format!("set value does not match the type of {var:?}")),
            }
        }
        Effect::Add { var, delta } => {
            if !granted(&decl.writes, var) {
                return Err(format!("variable {var:?} is not in `writes`"));
            }
            if !matches!(
                pack.variable(var).map(|decl| &decl.kind),
                Some(VariableKind::Int { .. })
            ) {
                return Err(format!("add needs an integer variable, not {var:?}"));
            }
            if delta.unsigned_abs() > MAX_VAR_ABS.unsigned_abs() {
                return Err("add delta is out of range".to_owned());
            }
        }
        Effect::Give(item) | Effect::Take(item) => {
            if !granted(&decl.items, item) {
                return Err(format!("item {item:?} is not in `items`"));
            }
        }
        Effect::Goto(node) => {
            if !granted(&decl.goto, node) {
                return Err(format!("node {node:?} is not in `goto`"));
            }
        }
        Effect::Score(delta) => {
            if !decl.score {
                return Err("the script may not change the score".to_owned());
            }
            if delta.unsigned_abs() > MAX_SCORE_DELTA.unsigned_abs() {
                return Err("score delta is out of range".to_owned());
            }
        }
        Effect::End(ending) => {
            if !decl.ends {
                return Err("the script may not end the story".to_owned());
            }
            if !valid_id(&ending.id) {
                return Err("ending id is not a valid identifier".to_owned());
            }
        }
        Effect::MoveObject { obj, to } => {
            if !granted(&decl.objects, obj) {
                return Err(format!("object {obj:?} is not in `objects`"));
            }
            match to {
                Location::Room(node) if pack.room(node).is_none() => {
                    return Err(format!("{node:?} is not a room"));
                }
                Location::In(container)
                    if !pack.object(container).is_some_and(|decl| decl.container) =>
                {
                    return Err(format!("{container:?} is not a container"));
                }
                _ => {}
            }
        }
        Effect::SetObject { obj, flag, .. } => {
            if !granted(&decl.objects, obj) {
                return Err(format!("object {obj:?} is not in `objects`"));
            }
            let Some(object) = pack.object(obj) else {
                return Err(format!("unknown object {obj:?}"));
            };
            let allowed = match flag {
                ObjectFlag::Open | ObjectFlag::Locked => object.openable,
                ObjectFlag::Lit => object.lightable || object.lit,
                ObjectFlag::Worn => object.wearable,
            };
            if !allowed {
                return Err(format!("object {obj:?} does not have this flag"));
            }
        }
        Effect::Roll { .. } | Effect::If { .. } | Effect::Cue(_) | Effect::Script(_) => {
            return Err("scripts may only emit set, add, give, take, goto, score, end, move_object and set_object".to_owned());
        }
    }
    Ok(())
}

fn is_false(value: &bool) -> bool {
    !*value
}
