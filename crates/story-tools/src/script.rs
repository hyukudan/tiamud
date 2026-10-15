//! Scripted, deterministic story tests.
//!
//! A script is a bounded JSON document:
//!
//! ```json
//! {
//!   "format_version": 2,
//!   "seed": 7,
//!   "steps": [
//!     { "expect": { "node": "inicio", "choices": 1 } },
//!     { "choose": 1 },
//!     "undo",
//!     { "act": { "verb": "investigar", "target": "mesa" } },
//!     { "menu": [2, 1] },
//!     { "say": 1 },
//!     { "expect": { "vars": { "destreza": 9 }, "has": ["antorcha"], "ended": false } }
//!   ]
//! }
//! ```
//!
//! `choose` takes a 1-based index into the choices *available* in the current
//! view, exactly as numbered by `preview` and `play`. `menu` takes a 1-based
//! verb number and, when the verb needs one, a 1-based target number, as
//! numbered by `play`. `act` names the verb and target by authored id (a test
//! is an authoring artefact; players only ever see opaque tokens). `say`
//! takes a 1-based dialogue option. `undo` restores the state before the last
//! accepted input. `expect` asserts any subset of fields. Format v1 scripts
//! (only `choose`/`undo`/`expect`) still run.
//!
//! Format v3 scripts drive parser worlds: `{ "text": "coge la lámpara" }`
//! types a line exactly as a player would (in the script's `language`, by
//! default the pack's first lexicon), and a typed `deshacer`/`undo` undoes
//! like the `undo` step. Expectations gain `carried`, `not_carried`,
//! `objects` (exact locations such as `{ "llave": "carried" }` or
//! `{ "aceitera": { "room": "escalera" } }`), `transcript` (substrings the
//! rendered replies must contain) and `question` (a parser question awaits).

use std::collections::{BTreeMap, BTreeSet};

use serde::Deserialize;
use tiamud_story_core::{
    Location, MAX_ITEMS, MAX_OBJECTS, MAX_VARIABLES, StoryError, StoryEvent, StoryInput, StoryPack,
    StoryState, UndoRing, Value, resolve_menu, step, step_command, view,
};

/// Newest script document version accepted by this module.
pub const STORY_SCRIPT_FORMAT_VERSION: u32 = 3;
/// Maximum transcript substrings one expectation checks.
pub const MAX_TRANSCRIPT_CHECKS: usize = 32;
/// Maximum script document size.
pub const MAX_SCRIPT_BYTES: usize = 256 * 1024;
/// Maximum steps in one script.
pub const MAX_SCRIPT_STEPS: usize = 4096;
/// Undo depth available to scripts and the interactive player.
pub const UNDO_CAPACITY: usize = 64;

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StoryScript {
    pub format_version: u32,
    #[serde(default)]
    pub seed: u64,
    /// Lexicon language of `text` steps and transcript checks (v3).
    #[serde(default)]
    pub language: Option<String>,
    pub steps: Vec<ScriptStep>,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub enum ScriptStep {
    /// 1-based index into the currently available choices.
    Choose(usize),
    /// 1-based verb number and optional 1-based target number (v2).
    Menu(Vec<usize>),
    /// Verb and target by authored id (v2).
    Act(Action),
    /// 1-based index into the active dialogue options (v2).
    Say(usize),
    /// A line of free text for the parser (v3).
    Text(String),
    Undo,
    Expect(Box<Expectation>),
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Action {
    pub verb: String,
    #[serde(default)]
    pub target: Option<String>,
}

/// Assertions on the current state. Absent fields are not checked.
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Expectation {
    #[serde(default)]
    pub node: Option<String>,
    #[serde(default)]
    pub turn: Option<u32>,
    #[serde(default)]
    pub score: Option<i64>,
    /// Exact values of the named variables (hidden ones included).
    #[serde(default)]
    pub vars: BTreeMap<String, Value>,
    /// Items that must be held.
    #[serde(default)]
    pub has: Vec<String>,
    /// Items that must not be held.
    #[serde(default)]
    pub lacks: Vec<String>,
    /// Number of available choices.
    #[serde(default)]
    pub choices: Option<usize>,
    #[serde(default)]
    pub ended: Option<bool>,
    /// Id of the ending reached.
    #[serde(default)]
    pub ending: Option<String>,
    /// Whether a dialogue awaits an answer (v2).
    #[serde(default)]
    pub dialogue: Option<bool>,
    /// Number of available dialogue options (v2).
    #[serde(default)]
    pub options: Option<usize>,
    /// Number of offered menu verbs (v2).
    #[serde(default)]
    pub verbs: Option<usize>,
    /// Use counters by `node/entry` (v2).
    #[serde(default)]
    pub uses: BTreeMap<String, u16>,
    /// World objects held directly by the player (v3).
    #[serde(default)]
    pub carried: Vec<String>,
    /// World objects not held directly by the player (v3).
    #[serde(default)]
    pub not_carried: Vec<String>,
    /// Exact object locations (v3).
    #[serde(default)]
    pub objects: BTreeMap<String, Location>,
    /// Substrings the rendered transcript of the last input must contain
    /// (v3).
    #[serde(default)]
    pub transcript: Vec<String>,
    /// Whether a parser question awaits an answer (v3).
    #[serde(default)]
    pub question: Option<bool>,
}

impl Expectation {
    fn uses_v2(&self) -> bool {
        self.dialogue.is_some()
            || self.options.is_some()
            || self.verbs.is_some()
            || !self.uses.is_empty()
    }

    fn uses_v3(&self) -> bool {
        !self.carried.is_empty()
            || !self.not_carried.is_empty()
            || !self.objects.is_empty()
            || !self.transcript.is_empty()
            || self.question.is_some()
    }
}

/// Why a script could not run or did not pass.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ScriptError {
    #[error("script exceeds {MAX_SCRIPT_BYTES} bytes")]
    TooLarge,
    #[error("malformed script: {0}")]
    Malformed(String),
    #[error("unsupported script format version {0}")]
    UnsupportedVersion(u32),
    #[error("script has {0} steps; the limit is {MAX_SCRIPT_STEPS}")]
    TooManySteps(usize),
    #[error("step {step}: {message}")]
    Invalid { step: usize, message: String },
    #[error("step {step}: {message}")]
    Failed { step: usize, message: String },
    #[error("story error: {0}")]
    Story(#[from] StoryError),
}

/// Successful script run.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScriptReport {
    pub steps: usize,
    /// Accepted inputs of any kind (choices, menu actions, dialogue answers).
    pub choices: usize,
    pub expectations: usize,
    pub final_state: StoryState,
}

impl StoryScript {
    /// Parses a bounded script document and checks its shape and bounds.
    pub fn from_json_slice(bytes: &[u8]) -> Result<Self, ScriptError> {
        if bytes.len() > MAX_SCRIPT_BYTES {
            return Err(ScriptError::TooLarge);
        }
        let script: Self = serde_json::from_slice(bytes)
            .map_err(|error| ScriptError::Malformed(truncate(error.to_string())))?;
        if !(1..=STORY_SCRIPT_FORMAT_VERSION).contains(&script.format_version) {
            return Err(ScriptError::UnsupportedVersion(script.format_version));
        }
        if script.steps.len() > MAX_SCRIPT_STEPS {
            return Err(ScriptError::TooManySteps(script.steps.len()));
        }
        Ok(script)
    }

    /// Checks that every name in the script exists in `pack`, so a typo is a
    /// script error instead of a confusing assertion failure.
    pub fn check_against(&self, pack: &StoryPack) -> Result<(), ScriptError> {
        let nodes: BTreeSet<&str> = pack.nodes.iter().map(|node| node.id.as_str()).collect();
        let vars: BTreeSet<&str> = pack.variables.iter().map(|var| var.id.as_str()).collect();
        let items: BTreeSet<&str> = pack.items.iter().map(|item| item.id.as_str()).collect();
        let verbs: BTreeSet<&str> = pack.verbs.iter().map(|verb| verb.id.as_str()).collect();
        let entries: BTreeSet<String> = pack
            .nodes
            .iter()
            .filter_map(|node| node.menu.as_ref().map(|menu| (node, menu)))
            .flat_map(|(node, menu)| {
                menu.entries
                    .iter()
                    .map(move |entry| format!("{}/{}", node.id, entry.id))
            })
            .collect();
        let objects: BTreeSet<&str> = pack
            .world
            .iter()
            .flat_map(|world| &world.objects)
            .map(|object| object.id.as_str())
            .collect();
        for (index, step) in self.steps.iter().enumerate() {
            let invalid = |message: String| ScriptError::Invalid {
                step: index + 1,
                message,
            };
            let v3_step = matches!(step, ScriptStep::Text(_))
                || matches!(step, ScriptStep::Expect(expect) if expect.uses_v3());
            if v3_step && self.format_version < 3 {
                return Err(invalid(
                    "text steps and v3 expectations need script format_version 3".to_owned(),
                ));
            }
            if v3_step && pack.world.is_none() {
                return Err(invalid(
                    "text steps and v3 expectations need a pack with a parser world".to_owned(),
                ));
            }
            let v2_step = matches!(
                step,
                ScriptStep::Menu(_) | ScriptStep::Act(_) | ScriptStep::Say(_)
            ) || matches!(step, ScriptStep::Expect(expect) if expect.uses_v2());
            if v2_step && self.format_version < 2 {
                return Err(invalid(
                    "menu, act, say and v2 expectations need script format_version 2".to_owned(),
                ));
            }
            match step {
                ScriptStep::Choose(0) | ScriptStep::Say(0) => {
                    return Err(invalid("numbers start at 1".to_owned()));
                }
                ScriptStep::Menu(numbers) => {
                    if numbers.is_empty() || numbers.len() > 2 || numbers.contains(&0) {
                        return Err(invalid(
                            "menu takes [verb] or [verb, target], numbered from 1".to_owned(),
                        ));
                    }
                }
                ScriptStep::Act(action) => {
                    if !verbs.contains(action.verb.as_str()) {
                        return Err(invalid(format!("unknown verb {:?}", action.verb)));
                    }
                }
                ScriptStep::Text(text) => {
                    if text.len() > tiamud_story_parser::MAX_INPUT_BYTES {
                        return Err(invalid(format!(
                            "text exceeds {} bytes",
                            tiamud_story_parser::MAX_INPUT_BYTES
                        )));
                    }
                }
                ScriptStep::Choose(_) | ScriptStep::Say(_) | ScriptStep::Undo => {}
                ScriptStep::Expect(expect) => {
                    if expect.carried.len() > MAX_OBJECTS
                        || expect.not_carried.len() > MAX_OBJECTS
                        || expect.objects.len() > MAX_OBJECTS
                        || expect.transcript.len() > MAX_TRANSCRIPT_CHECKS
                    {
                        return Err(invalid("expectation lists exceed pack limits".to_owned()));
                    }
                    if let Some(object) = expect
                        .carried
                        .iter()
                        .chain(&expect.not_carried)
                        .chain(expect.objects.keys())
                        .find(|object| !objects.contains(object.as_str()))
                    {
                        return Err(invalid(format!("unknown object {object:?}")));
                    }
                    if expect.vars.len() > MAX_VARIABLES
                        || expect.has.len() > MAX_ITEMS
                        || expect.lacks.len() > MAX_ITEMS
                        || expect.uses.len() > entries.len()
                    {
                        return Err(invalid("expectation lists exceed pack limits".to_owned()));
                    }
                    if let Some(node) = &expect.node
                        && !nodes.contains(node.as_str())
                    {
                        return Err(invalid(format!("unknown node {node:?}")));
                    }
                    if let Some(var) = expect.vars.keys().find(|var| !vars.contains(var.as_str())) {
                        return Err(invalid(format!("unknown variable {var:?}")));
                    }
                    if let Some(item) = expect
                        .has
                        .iter()
                        .chain(&expect.lacks)
                        .find(|item| !items.contains(item.as_str()))
                    {
                        return Err(invalid(format!("unknown item {item:?}")));
                    }
                    if let Some(entry) = expect.uses.keys().find(|key| !entries.contains(*key)) {
                        return Err(invalid(format!("unknown menu entry {entry:?}")));
                    }
                }
            }
        }
        Ok(())
    }
}

fn truncate(mut message: String) -> String {
    const LIMIT: usize = 256;
    if message.len() > LIMIT {
        let mut cut = LIMIT;
        while !message.is_char_boundary(cut) {
            cut -= 1;
        }
        message.truncate(cut);
    }
    message
}

/// Result of one accepted input.
pub type Stepped = (StoryState, Vec<StoryEvent>);

/// Chooses the 1-based `number` among the available choices of `state`.
pub fn choose(pack: &StoryPack, state: &StoryState, number: usize) -> Result<Stepped, ChooseError> {
    let current = view(pack, state)?;
    if current.ending.is_some() {
        return Err(ChooseError::Ended);
    }
    let token = number
        .checked_sub(1)
        .and_then(|index| current.choices.get(index))
        .map(|choice| choice.token.clone())
        .ok_or(ChooseError::OutOfRange {
            available: current.choices.len(),
        })?;
    Ok(step(pack, state, StoryInput::Choose { token })?)
}

/// Applies the 1-based `verb` (and `target`) of the current menu.
pub fn menu_by_number(
    pack: &StoryPack,
    state: &StoryState,
    verb: usize,
    target: Option<usize>,
) -> Result<Stepped, ChooseError> {
    let current = view(pack, state)?;
    if current.ending.is_some() {
        return Err(ChooseError::Ended);
    }
    if current.dialogue.is_some() {
        return Err(ChooseError::Story(StoryError::InDialogue));
    }
    let menu = current.menu.ok_or(ChooseError::NoMenu)?;
    let available = menu.verbs.len();
    let offered = verb
        .checked_sub(1)
        .and_then(|index| menu.verbs.into_iter().nth(index))
        .ok_or(ChooseError::OutOfRange { available })?;
    let target_token = match (offered.needs_target, target) {
        (false, None) => None,
        (false, Some(_)) => return Err(ChooseError::UnexpectedTarget),
        (true, None) => return Err(ChooseError::NeedsTarget),
        (true, Some(number)) => {
            let available = offered.targets.len();
            Some(
                number
                    .checked_sub(1)
                    .and_then(|index| offered.targets.get(index))
                    .map(|target| target.token.clone())
                    .ok_or(ChooseError::OutOfRange { available })?,
            )
        }
    };
    Ok(step(
        pack,
        state,
        StoryInput::MenuAction {
            verb: offered.token,
            target: target_token,
        },
    )?)
}

/// Applies a menu action named by authored verb and target ids.
pub fn menu_by_ids(
    pack: &StoryPack,
    state: &StoryState,
    verb: &str,
    target: Option<&str>,
) -> Result<Stepped, ChooseError> {
    let current = view(pack, state)?;
    if current.ending.is_some() {
        return Err(ChooseError::Ended);
    }
    if current.dialogue.is_some() {
        return Err(ChooseError::Story(StoryError::InDialogue));
    }
    let menu = current.menu.ok_or(ChooseError::NoMenu)?;
    for offered in menu.verbs {
        let candidates: Vec<Option<String>> = if offered.needs_target {
            offered
                .targets
                .into_iter()
                .map(|candidate| Some(candidate.token))
                .collect()
        } else {
            vec![None]
        };
        for candidate in candidates {
            let selection = resolve_menu(pack, state, &offered.token, candidate.as_deref())?;
            if selection.verb == verb && selection.target == target {
                return Ok(step(
                    pack,
                    state,
                    StoryInput::MenuAction {
                        verb: offered.token,
                        target: candidate,
                    },
                )?);
            }
        }
    }
    Err(ChooseError::NotOffered)
}

/// Answers the active dialogue with its 1-based `number` option.
pub fn say(pack: &StoryPack, state: &StoryState, number: usize) -> Result<Stepped, ChooseError> {
    let current = view(pack, state)?;
    if current.ending.is_some() {
        return Err(ChooseError::Ended);
    }
    let dialogue = current.dialogue.ok_or(ChooseError::NoDialogue)?;
    let available = dialogue.options.len();
    let token = number
        .checked_sub(1)
        .and_then(|index| dialogue.options.get(index))
        .map(|option| option.token.clone())
        .ok_or(ChooseError::OutOfRange { available })?;
    Ok(step(pack, state, StoryInput::DialogueChoice { token })?)
}

/// Failure to apply a numbered or named input.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ChooseError {
    #[error("the story has ended")]
    Ended,
    #[error("no such choice; {available} available")]
    OutOfRange { available: usize },
    #[error("this scene has no verb menu")]
    NoMenu,
    #[error("no dialogue is waiting for an answer")]
    NoDialogue,
    #[error("this verb needs a target")]
    NeedsTarget,
    #[error("this verb takes no target")]
    UnexpectedTarget,
    #[error("that verb and target are not offered now")]
    NotOffered,
    #[error("{0}")]
    Story(#[from] StoryError),
}

/// Runs `script` against a validated `pack` from a fresh state.
pub fn run_script(pack: &StoryPack, script: &StoryScript) -> Result<ScriptReport, ScriptError> {
    script.check_against(pack)?;
    let mut state = StoryState::new(pack, script.seed)?;
    let mut undo = UndoRing::new(UNDO_CAPACITY)?;
    let mut inputs = 0usize;
    let mut expectations = 0usize;
    for (index, step_spec) in script.steps.iter().enumerate() {
        let failed = |message: String| ScriptError::Failed {
            step: index + 1,
            message,
        };
        let stepped = match step_spec {
            ScriptStep::Choose(number) => Some(
                choose(pack, &state, *number)
                    .map_err(|error| failed(format!("choose {number}: {error}")))?,
            ),
            ScriptStep::Menu(numbers) => {
                let verb = numbers.first().copied().unwrap_or(0);
                Some(
                    menu_by_number(pack, &state, verb, numbers.get(1).copied())
                        .map_err(|error| failed(format!("menu {numbers:?}: {error}")))?,
                )
            }
            ScriptStep::Act(action) => Some(
                menu_by_ids(pack, &state, &action.verb, action.target.as_deref()).map_err(
                    |error| failed(format!("act {} {:?}: {error}", action.verb, action.target)),
                )?,
            ),
            ScriptStep::Say(number) => Some(
                say(pack, &state, *number)
                    .map_err(|error| failed(format!("say {number}: {error}")))?,
            ),
            ScriptStep::Text(text) => {
                let stepped = step_command(pack, &state, text, script.language.as_deref())
                    .map_err(|error| failed(format!("text {text:?}: {error}")))?;
                if stepped.meta.iter().any(|meta| meta.action == "undo") {
                    state = undo
                        .pop()
                        .ok_or_else(|| failed("undo: nothing to undo".to_owned()))?;
                    None
                } else {
                    Some((stepped.state, stepped.events))
                }
            }
            ScriptStep::Undo => {
                state = undo
                    .pop()
                    .ok_or_else(|| failed("undo: nothing to undo".to_owned()))?;
                None
            }
            ScriptStep::Expect(expect) => {
                check(pack, &state, expect, script.language.as_deref()).map_err(failed)?;
                expectations = expectations.saturating_add(1);
                None
            }
        };
        if let Some((next, _)) = stepped {
            undo.push(std::mem::replace(&mut state, next));
            inputs = inputs.saturating_add(1);
        }
    }
    Ok(ScriptReport {
        steps: script.steps.len(),
        choices: inputs,
        expectations,
        final_state: state,
    })
}

fn check(
    pack: &StoryPack,
    state: &StoryState,
    expect: &Expectation,
    language: Option<&str>,
) -> Result<(), String> {
    let mut problems = Vec::new();
    let location = |object: &str| state.objects.get(object).map(|object| &object.at);
    for object in &expect.carried {
        if location(object) != Some(&Location::Carried) {
            problems.push(format!("object {object:?} is not carried"));
        }
    }
    for object in &expect.not_carried {
        if location(object) == Some(&Location::Carried) {
            problems.push(format!("object {object:?} is carried"));
        }
    }
    for (object, wanted) in &expect.objects {
        if location(object) != Some(wanted) {
            problems.push(format!(
                "object {object:?} is at {:?}, expected {wanted:?}",
                location(object)
            ));
        }
    }
    if let Some(question) = expect.question
        && state.pending.is_some() != question
    {
        problems.push(if question {
            "no parser question awaits an answer".to_owned()
        } else {
            "a parser question awaits an answer".to_owned()
        });
    }
    if !expect.transcript.is_empty() {
        let locale = language
            .or_else(|| {
                pack.world
                    .as_ref()
                    .and_then(|world| world.lexicons.first())
                    .map(|lexicon| lexicon.id.as_str())
            })
            .unwrap_or("es");
        let current = view(pack, state).map_err(|error| error.to_string())?;
        let shown: Vec<&str> = current
            .blocks
            .iter()
            .filter_map(|block| block.text.resolve(locale))
            .collect();
        let shown = shown.join("\n");
        for wanted in &expect.transcript {
            if !shown.contains(wanted.as_str()) {
                problems.push(format!(
                    "transcript does not contain {wanted:?}; it reads {shown:?}"
                ));
            }
        }
    }
    if let Some(node) = &expect.node
        && state.node != *node
    {
        problems.push(format!("node is {:?}, expected {node:?}", state.node));
    }
    if let Some(turn) = expect.turn
        && state.turn != turn
    {
        problems.push(format!("turn is {}, expected {turn}", state.turn));
    }
    if let Some(score) = expect.score
        && state.score != score
    {
        problems.push(format!("score is {}, expected {score}", state.score));
    }
    for (var, wanted) in &expect.vars {
        let actual = state.vars.get(var);
        if actual != Some(wanted) {
            problems.push(format!(
                "variable {var:?} is {}, expected {}",
                actual.map_or_else(|| "missing".to_owned(), value_json),
                value_json(wanted)
            ));
        }
    }
    for item in &expect.has {
        if !state.inventory.contains(item) {
            problems.push(format!("item {item:?} is not held"));
        }
    }
    for item in &expect.lacks {
        if state.inventory.contains(item) {
            problems.push(format!("item {item:?} is held"));
        }
    }
    for (entry, wanted) in &expect.uses {
        let actual = state.menu_uses.get(entry).copied().unwrap_or(0);
        if actual != *wanted {
            problems.push(format!(
                "entry {entry:?} used {actual} time(s), expected {wanted}"
            ));
        }
    }
    if expect.choices.is_some()
        || expect.dialogue.is_some()
        || expect.options.is_some()
        || expect.verbs.is_some()
    {
        let current = view(pack, state).map_err(|error| error.to_string())?;
        if let Some(count) = expect.choices
            && current.choices.len() != count
        {
            problems.push(format!(
                "{} choice(s) available, expected {count}",
                current.choices.len()
            ));
        }
        if let Some(active) = expect.dialogue
            && current.dialogue.is_some() != active
        {
            problems.push(if active {
                "no dialogue is active".to_owned()
            } else {
                "a dialogue is active".to_owned()
            });
        }
        let options = current
            .dialogue
            .as_ref()
            .map_or(0, |dialogue| dialogue.options.len());
        if let Some(count) = expect.options
            && options != count
        {
            problems.push(format!("{options} dialogue option(s), expected {count}"));
        }
        let verbs = current.menu.as_ref().map_or(0, |menu| menu.verbs.len());
        if let Some(count) = expect.verbs
            && verbs != count
        {
            problems.push(format!("{verbs} menu verb(s), expected {count}"));
        }
    }
    if let Some(ended) = expect.ended
        && state.ended.is_some() != ended
    {
        problems.push(if ended {
            "story has not ended".to_owned()
        } else {
            "story has ended".to_owned()
        });
    }
    if let Some(ending) = &expect.ending {
        match &state.ended {
            Some(actual) if actual.id == *ending => {}
            Some(actual) => {
                problems.push(format!("ending is {:?}, expected {ending:?}", actual.id))
            }
            None => problems.push(format!("story has not ended, expected ending {ending:?}")),
        }
    }
    if problems.is_empty() {
        Ok(())
    } else {
        Err(format!("expectation failed: {}", problems.join("; ")))
    }
}

fn value_json(value: &Value) -> String {
    match value {
        Value::Int(value) => value.to_string(),
        Value::Bool(value) => value.to_string(),
    }
}
