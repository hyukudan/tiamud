//! Optional Lua script host for StoryPacks (phase S6, ADR 0034).
//!
//! `tiamud-story-core` stays pure and dependency-light: it only exposes the
//! [`ScriptHost`] hook points ([`Condition::Script`] and [`Effect::Script`],
//! evaluated by the `*_scripted` functions). This crate implements that host
//! on top of the existing `tiamud-scripting` sandbox:
//!
//! - every invocation runs in a **fresh** Lua 5.4 VM without filesystem,
//!   network, processes, module loading, `load`, debug, `print`, OS time or
//!   `math.random`, under the sandbox quotas (64 KiB source, 2 MiB memory,
//!   100 000 instructions, 25 ms, 32 events of at most 1 KiB per field);
//! - the script receives a **copy** of a bounded, read-only projection of the
//!   state ([`tiamud_story_core::ScriptSnapshot`]) and its constant
//!   arguments, so it cannot mutate anything;
//! - an effect script can only *emit* intents through the `fx` helpers; this
//!   crate turns them into ordinary [`Effect`] values and `story-core` checks
//!   each one against the script's declared grants before applying it, so Lua
//!   is never a second authority;
//! - randomness comes only from `tiamud.random`/`dice`, seeded by the story
//!   RNG; `pairs` iterates in sorted key order, `next` and `string.format`
//!   are removed and `tostring` never prints addresses, so the same state
//!   and seed always yield the same intents.
//!
//! A condition script defines `check(story, args)` and returns a boolean; an
//! effect script defines `run(story, args)`. See the StoryPack authoring
//! guide for the full Lua API.

#![forbid(unsafe_code)]

use std::collections::{BTreeMap, BTreeSet};

use tiamud_scripting::{ScriptLimits, ScriptValue, call_function, validate_functions};
use tiamud_story_core::{
    Condition, Effect, Ending, EndingKind, Location, MAX_SCRIPT_SOURCE_BYTES, ObjectFlag,
    ScriptCall, ScriptDecl, ScriptFailure, ScriptHost, ScriptKind, ScriptSnapshot, StoryPack,
    ValidationErrors, ValidationIssue, ValidationReport, Value,
};

/// Lua helpers prepended to every script, on the script's own first line so
/// that Lua line numbers in diagnostics match the authored source. It must
/// never contain a newline or a `--` comment.
const PRELUDE: &str = concat!(
    "local __emit, __rnd, __type, __mtype, __raw_tostring, __rawnext, __sort, __error = ",
    "tiamud.emit, tiamud.random, type, math.type, tostring, next, table.sort, error; ",
    "local function __int(v, what) if __mtype(v) ~= 'integer' then ",
    "__error(what .. ' must be an integer', 3) end return __raw_tostring(v) end; ",
    "local function __id(v, what) if __type(v) ~= 'string' then ",
    "__error(what .. ' must be a string id', 3) end return v end; ",
    "local function __bool(v, what) if __type(v) ~= 'boolean' then ",
    "__error(what .. ' must be a boolean', 3) end return v and 'true' or 'false' end; ",
    "fx = { ",
    "set = function(var, v) __id(var, 'fx.set variable') ",
    "if __type(v) == 'boolean' then __emit('set', var, v and 'true' or 'false') ",
    "else __emit('set', var, __int(v, 'fx.set value')) end end, ",
    "add = function(var, d) __emit('add', __id(var, 'fx.add variable'), __int(d, 'fx.add delta')) end, ",
    "give = function(i) __emit('give', __id(i, 'fx.give item'), nil) end, ",
    "take = function(i) __emit('take', __id(i, 'fx.take item'), nil) end, ",
    "go = function(n) __emit('goto', __id(n, 'fx.go node'), nil) end, ",
    "score = function(d) __emit('score', nil, __int(d, 'fx.score delta')) end, ",
    "finish = function(id, kind) __emit('end', __id(id, 'fx.finish ending'), ",
    "kind == nil and 'neutral' or __id(kind, 'fx.finish kind')) end, ",
    "move = function(o, to) __emit('move', __id(o, 'fx.move object'), __id(to, 'fx.move destination')) end, ",
    "set_object = function(o, flag, v) __emit('set_object', __id(o, 'fx.set_object object'), ",
    "__id(flag, 'fx.set_object flag') .. '=' .. __bool(v, 'fx.set_object value')) end, ",
    "}; ",
    "function dice(count, sides) __int(count, 'dice count') __int(sides, 'dice sides') ",
    "if count < 1 or count > 16 or sides < 2 or sides > 100 then ",
    "__error('dice needs 1..16 dice of 2..100 sides', 2) end ",
    "local total = 0 for _ = 1, count do total = total + __rnd(sides) end return total end; ",
    "pairs = function(t) if __type(t) ~= 'table' then __error('pairs expects a table', 2) end ",
    "local keys, k = {}, __rawnext(t) ",
    "while k ~= nil do local kt = __type(k) if kt ~= 'string' and kt ~= 'number' then ",
    "__error('pairs only iterates string and number keys deterministically', 2) end ",
    "keys[#keys + 1] = k k = __rawnext(t, k) end ",
    "__sort(keys, function(a, b) local ta, tb = __type(a), __type(b) ",
    "if ta ~= tb then return ta == 'number' end return a < b end) ",
    "local i = 0 return function() i = i + 1 local key = keys[i] ",
    "if key ~= nil then return key, t[key] end end, t, nil end; ",
    "next = nil; string.format = nil; ",
    "tostring = function(v) local t = __type(v) if t == 'table' or t == 'function' ",
    "or t == 'thread' or t == 'userdata' then return t end return __raw_tostring(v) end; ",
);

/// Event kinds an effect script may emit; a condition script may emit none.
const EFFECT_EVENT_KINDS: [&str; 9] = [
    "set",
    "add",
    "give",
    "take",
    "goto",
    "score",
    "end",
    "move",
    "set_object",
];

/// Sandbox quotas for one story script invocation: the shared defaults of
/// `tiamud-scripting`, with the source allowance widened by the prelude only.
pub fn story_script_limits() -> ScriptLimits {
    ScriptLimits {
        max_source_bytes: MAX_SCRIPT_SOURCE_BYTES + PRELUDE.len(),
        ..ScriptLimits::default()
    }
}

/// [`ScriptHost`] that runs StoryPack scripts in the quota-limited Lua sandbox.
#[derive(Debug, Clone, Copy)]
pub struct LuaStoryHost {
    limits: ScriptLimits,
}

impl Default for LuaStoryHost {
    fn default() -> Self {
        Self::new()
    }
}

impl LuaStoryHost {
    /// Host with the default story limits ([`story_script_limits`]).
    pub fn new() -> Self {
        Self {
            limits: story_script_limits(),
        }
    }

    /// Host with explicit quotas (tests, or a stricter runtime). The source
    /// quota must leave room for the prelude.
    pub fn with_limits(limits: ScriptLimits) -> Self {
        Self { limits }
    }

    fn call(
        &self,
        call: &ScriptCall<'_>,
        function: &str,
        kinds: &BTreeSet<String>,
    ) -> Result<tiamud_scripting::FunctionOutcome, ScriptFailure> {
        let source = script_source(call.script);
        let args = [snapshot_value(call.snapshot), args_value(call.args)];
        call_function(
            &source,
            &chunk_name(call.script),
            function,
            &args,
            kinds,
            call.seed,
            self.limits,
        )
        .map_err(|error| ScriptFailure::new(error.to_string()))
    }
}

impl ScriptHost for LuaStoryHost {
    fn check(&self, call: &ScriptCall<'_>) -> Result<bool, ScriptFailure> {
        let outcome = self.call(call, "check", &BTreeSet::new())?;
        match outcome.result {
            ScriptValue::Bool(answer) => Ok(answer),
            _ => Err(ScriptFailure::new("check() must return a boolean")),
        }
    }

    fn run(&self, call: &ScriptCall<'_>) -> Result<Vec<Effect>, ScriptFailure> {
        let kinds = EFFECT_EVENT_KINDS
            .iter()
            .map(|kind| (*kind).to_owned())
            .collect();
        let outcome = self.call(call, "run", &kinds)?;
        outcome
            .events
            .iter()
            .enumerate()
            .map(|(index, event)| {
                intent(&event.kind, event.target.as_deref(), event.value.as_deref()).map_err(
                    |message| ScriptFailure::new(format!("intent {}: {message}", index + 1)),
                )
            })
            .collect()
    }
}

/// Full source handed to the VM: prelude and authored source on one line.
fn script_source(script: &ScriptDecl) -> String {
    let mut source = String::with_capacity(PRELUDE.len() + script.source.len());
    source.push_str(PRELUDE);
    source.push_str(&script.source);
    source
}

/// Lua chunk name: `=` keeps it verbatim, so diagnostics read `id:line: …`.
fn chunk_name(script: &ScriptDecl) -> String {
    format!("={}", script.id)
}

const fn entry_function(kind: ScriptKind) -> &'static str {
    match kind {
        ScriptKind::Condition => "check",
        ScriptKind::Effect => "run",
    }
}

fn value(value: &Value) -> ScriptValue {
    match value {
        Value::Bool(flag) => ScriptValue::Bool(*flag),
        Value::Int(number) => ScriptValue::Int(*number),
    }
}

fn args_value(args: &BTreeMap<String, Value>) -> ScriptValue {
    ScriptValue::Map(
        args.iter()
            .map(|(name, arg)| (name.clone(), value(arg)))
            .collect(),
    )
}

fn set_value<'a>(ids: impl Iterator<Item = &'a String>) -> ScriptValue {
    ScriptValue::Map(
        ids.map(|id| (id.clone(), ScriptValue::Bool(true)))
            .collect(),
    )
}

/// Location as the string `fx.move` also accepts.
fn location_text(location: &Location) -> String {
    match location {
        Location::Carried => "carried".to_owned(),
        Location::Nowhere => "nowhere".to_owned(),
        Location::Room(node) => format!("room:{node}"),
        Location::In(container) => format!("in:{container}"),
    }
}

/// The Lua `story` table: `node`, `turn`, `score`, `vars` (id → integer or
/// boolean), `items`/`visited` (id → true) and `objects` (id → `at`, `open`,
/// `locked`, `lit`, `worn`).
fn snapshot_value(snapshot: &ScriptSnapshot) -> ScriptValue {
    let objects = snapshot
        .objects
        .iter()
        .map(|(id, object)| {
            let fields = BTreeMap::from([
                ("at".to_owned(), ScriptValue::Str(location_text(&object.at))),
                ("open".to_owned(), ScriptValue::Bool(object.open)),
                ("locked".to_owned(), ScriptValue::Bool(object.locked)),
                ("lit".to_owned(), ScriptValue::Bool(object.lit)),
                ("worn".to_owned(), ScriptValue::Bool(object.worn)),
            ]);
            (id.clone(), ScriptValue::Map(fields))
        })
        .collect();
    ScriptValue::Map(BTreeMap::from([
        ("node".to_owned(), ScriptValue::Str(snapshot.node.clone())),
        (
            "turn".to_owned(),
            ScriptValue::Int(i64::from(snapshot.turn)),
        ),
        ("score".to_owned(), ScriptValue::Int(snapshot.score)),
        (
            "vars".to_owned(),
            ScriptValue::Map(
                snapshot
                    .vars
                    .iter()
                    .map(|(id, var)| (id.clone(), value(var)))
                    .collect(),
            ),
        ),
        ("items".to_owned(), set_value(snapshot.items.iter())),
        ("visited".to_owned(), set_value(snapshot.visited.iter())),
        ("objects".to_owned(), ScriptValue::Map(objects)),
    ]))
}

/// Parses one emitted event into an intent. `story-core` then checks it
/// against the script's grants; this only rejects malformed shapes.
fn intent(kind: &str, target: Option<&str>, value: Option<&str>) -> Result<Effect, String> {
    let target = || {
        target
            .map(str::to_owned)
            .ok_or_else(|| format!("{kind} needs a target id"))
    };
    let integer = || -> Result<i64, String> {
        value
            .and_then(|text| text.parse::<i64>().ok())
            .ok_or_else(|| format!("{kind} needs an integer value"))
    };
    Ok(match kind {
        "set" => {
            let var = target()?;
            let value = match value {
                Some("true") => Value::Bool(true),
                Some("false") => Value::Bool(false),
                _ => Value::Int(integer()?),
            };
            Effect::Set { var, value }
        }
        "add" => Effect::Add {
            var: target()?,
            delta: integer()?,
        },
        "give" => Effect::Give(target()?),
        "take" => Effect::Take(target()?),
        "goto" => Effect::Goto(target()?),
        "score" => Effect::Score(integer()?),
        "end" => Effect::End(Ending {
            id: target()?,
            kind: match value {
                Some("victory") => EndingKind::Victory,
                Some("defeat") => EndingKind::Defeat,
                Some("neutral") | None => EndingKind::Neutral,
                Some(_) => return Err("ending kind must be victory, defeat or neutral".to_owned()),
            },
        }),
        "move" => Effect::MoveObject {
            obj: target()?,
            to: match value {
                Some("carried") => Location::Carried,
                Some("nowhere") => Location::Nowhere,
                Some(text) => match text.split_once(':') {
                    Some(("room", node)) if !node.is_empty() => Location::Room(node.to_owned()),
                    Some(("in", container)) if !container.is_empty() => {
                        Location::In(container.to_owned())
                    }
                    _ => {
                        return Err(
                            "destination must be carried, nowhere, room:<node> or in:<object>"
                                .to_owned(),
                        );
                    }
                },
                None => return Err("move needs a destination".to_owned()),
            },
        },
        "set_object" => {
            let obj = target()?;
            let (flag, setting) = value
                .and_then(|text| text.split_once('='))
                .ok_or_else(|| "set_object needs flag=value".to_owned())?;
            let flag = match flag {
                "open" => ObjectFlag::Open,
                "locked" => ObjectFlag::Locked,
                "lit" => ObjectFlag::Lit,
                "worn" => ObjectFlag::Worn,
                _ => return Err("flag must be open, locked, lit or worn".to_owned()),
            };
            let value = match setting {
                "true" => true,
                "false" => false,
                _ => return Err("set_object value must be a boolean".to_owned()),
            };
            Effect::SetObject { obj, flag, value }
        }
        _ => return Err(format!("unknown intent kind {kind:?}")),
    })
}

/// Validates a pack for a runtime with this host: the core scripted
/// validation, then every script source compiled and initialized in the
/// sandbox with its entry function (`check` or `run`) required. Nothing is
/// called, so no script effect can happen during validation.
pub fn validate_pack(pack: &StoryPack) -> Result<ValidationReport, ValidationErrors> {
    let report = pack.validate_scripted()?;
    let limits = story_script_limits();
    let errors: Vec<ValidationIssue> = pack
        .scripts
        .iter()
        .enumerate()
        .filter_map(|(index, script)| {
            validate_functions(
                &script_source(script),
                &chunk_name(script),
                &[entry_function(script.kind)],
                limits,
            )
            .err()
            .map(|error| ValidationIssue {
                path: format!("scripts[{index}].source"),
                message: format!("script {:?} does not compile: {error}", script.id),
            })
        })
        .collect();
    if errors.is_empty() {
        Ok(report)
    } else {
        Err(ValidationErrors {
            errors,
            truncated: false,
        })
    }
}

/// Declared script ids that no condition or effect references, in pack
/// order. An authoring lint, not an error.
pub fn unused_scripts(pack: &StoryPack) -> Vec<&str> {
    let mut used = BTreeSet::new();
    let mut note_condition = |condition: &Condition| collect_condition(condition, &mut used, 0);
    for node in &pack.nodes {
        for choice in &node.choices {
            if let Some(when) = &choice.when {
                note_condition(when);
            }
        }
        if let Some(menu) = &node.menu {
            for verb in &menu.verbs {
                if let Some(when) = &verb.when {
                    note_condition(when);
                }
            }
            for response in menu.entries.iter().flat_map(|entry| &entry.responses) {
                if let Some(when) = &response.when {
                    note_condition(when);
                }
            }
        }
        for hotspot in &node.hotspots {
            if let Some(when) = &hotspot.when {
                note_condition(when);
            }
        }
        for presence in &node.present {
            if let Some(when) = &presence.when {
                note_condition(when);
            }
        }
        if let Some(room) = &node.room {
            for exit in &room.exits {
                if let Some(when) = &exit.when {
                    note_condition(when);
                }
            }
        }
    }
    for option in pack
        .dialogues
        .iter()
        .flat_map(|dialogue| &dialogue.nodes)
        .flat_map(|node| &node.options)
    {
        if let Some(when) = &option.when {
            note_condition(when);
        }
    }
    if let Some(world) = &pack.world {
        for rule in &world.rules {
            if let Some(when) = &rule.when {
                note_condition(when);
            }
        }
    }
    let mut effect_lists: Vec<&[Effect]> = Vec::new();
    for node in &pack.nodes {
        effect_lists.extend(node.choices.iter().map(|choice| choice.effects.as_slice()));
        if let Some(menu) = &node.menu {
            effect_lists.extend(
                menu.entries
                    .iter()
                    .flat_map(|entry| &entry.responses)
                    .map(|response| response.effects.as_slice()),
            );
        }
    }
    effect_lists.extend(
        pack.dialogues
            .iter()
            .flat_map(|dialogue| &dialogue.nodes)
            .flat_map(|node| &node.options)
            .map(|option| option.effects.as_slice()),
    );
    if let Some(world) = &pack.world {
        effect_lists.extend(world.rules.iter().map(|rule| rule.effects.as_slice()));
    }
    for effects in effect_lists {
        collect_effects(effects, &mut used, 0);
    }
    pack.scripts
        .iter()
        .map(|script| script.id.as_str())
        .filter(|id| !used.contains(*id))
        .collect()
}

/// Bounded like the core validator: walks stop past the documented depths.
fn collect_condition(condition: &Condition, used: &mut BTreeSet<String>, depth: usize) {
    if depth > tiamud_story_core::MAX_CONDITION_DEPTH {
        return;
    }
    match condition {
        Condition::Script(script) => {
            used.insert(script.id.clone());
        }
        Condition::All(children) | Condition::Any(children) => {
            for child in children {
                collect_condition(child, used, depth + 1);
            }
        }
        Condition::Not(child) => collect_condition(child, used, depth + 1),
        _ => {}
    }
}

fn collect_effects(effects: &[Effect], used: &mut BTreeSet<String>, depth: usize) {
    if depth > tiamud_story_core::MAX_EFFECT_DEPTH {
        return;
    }
    for effect in effects {
        match effect {
            Effect::Script(script) => {
                used.insert(script.id.clone());
            }
            Effect::If {
                when,
                then,
                otherwise,
            } => {
                collect_condition(when, used, 0);
                collect_effects(then, used, depth + 1);
                collect_effects(otherwise, used, depth + 1);
            }
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn prelude_is_one_line_without_comments() {
        assert!(!PRELUDE.contains('\n'));
        assert!(!PRELUDE.contains("--"));
    }

    #[test]
    fn intents_parse_only_closed_shapes() {
        assert_eq!(
            intent("add", Some("vigor"), Some("-2")),
            Ok(Effect::Add {
                var: "vigor".to_owned(),
                delta: -2
            })
        );
        assert_eq!(
            intent("move", Some("llave"), Some("in:cofre")),
            Ok(Effect::MoveObject {
                obj: "llave".to_owned(),
                to: Location::In("cofre".to_owned())
            })
        );
        for (kind, target, value) in [
            ("add", Some("vigor"), Some("dos")),
            ("add", None, Some("2")),
            ("score", None, Some("1.5")),
            ("end", Some("fin"), Some("epic")),
            ("move", Some("llave"), Some("bolsillo")),
            ("move", Some("llave"), Some("room:")),
            ("set_object", Some("cofre"), Some("open")),
            ("set_object", Some("cofre"), Some("open=yes")),
            ("set_object", Some("cofre"), Some("broken=true")),
            ("roll", None, None),
            ("cue", Some("musica"), None),
        ] {
            assert!(intent(kind, target, value).is_err(), "{kind} {value:?}");
        }
    }
}
