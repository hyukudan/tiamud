use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};

use crate::rng::SplitMix64;
use crate::{
    Ending, MAX_MENU_USES, MAX_SCORE_ABS, MAX_STATE_BYTES, MAX_TURNS, STORY_STATE_FORMAT_VERSION,
    StoryError, StoryPack, Value, VariableKind, bounded_message,
};

/// Position inside the single active dialogue tree.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DialoguePosition {
    pub dialogue: String,
    pub node: String,
}

/// What the last menu action answered, so the view can show its text until
/// the next input. Any later input replaces or clears it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub enum LastResponse {
    /// An authored response of a menu entry.
    Entry {
        node: String,
        entry: String,
        response: String,
    },
    /// The entry's "nothing new" text: no response applied any more.
    Exhausted { node: String, entry: String },
    /// The scene's default for the verb.
    SceneDefault { node: String, verb: String },
    /// The verb's own default.
    VerbDefault { verb: String },
}

/// Complete, serializable state of one story playthrough.
///
/// Every collection is keyed by pack-declared ids, so its size is bounded by
/// the validated pack. [`StoryState::check_against`] re-establishes that bound
/// for any state read from outside, and [`crate::step`] calls it first.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StoryState {
    pub format_version: u32,
    pub pack_id: String,
    pub node: String,
    pub vars: BTreeMap<String, Value>,
    pub inventory: BTreeSet<String>,
    pub visited: BTreeSet<String>,
    /// SplitMix64 generator state.
    pub rng: u64,
    pub turn: u32,
    pub score: i64,
    pub ended: Option<Ending>,
    /// Use counters of menu entries, keyed `node/entry`, saturating at
    /// [`MAX_MENU_USES`] (format v2).
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub menu_uses: BTreeMap<String, u16>,
    /// Spent `once` responses, keyed `node/entry/response` (format v2).
    #[serde(default, skip_serializing_if = "BTreeSet::is_empty")]
    pub spent_responses: BTreeSet<String>,
    /// Taken `once` dialogue options, keyed `dialogue/node/option` (v2).
    #[serde(default, skip_serializing_if = "BTreeSet::is_empty")]
    pub spent_options: BTreeSet<String>,
    /// Active dialogue, if any (format v2).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub dialogue: Option<DialoguePosition>,
    /// Response of the last menu action (format v2).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last: Option<LastResponse>,
}

/// Joins authored ids into a state key. Ids never contain `/`.
pub(crate) fn key(parts: &[&str]) -> String {
    parts.join("/")
}

impl StoryState {
    /// Creates the initial state for `pack` with a session seed. The pack
    /// should already be validated; inconsistencies return an error.
    pub fn new(pack: &StoryPack, seed: u64) -> Result<Self, StoryError> {
        let start = pack
            .node(&pack.start)
            .ok_or_else(|| StoryError::InvalidPack("start node is missing".to_owned()))?;
        let mut vars = BTreeMap::new();
        for decl in &pack.variables {
            let value = match decl.kind {
                VariableKind::Int { min, max, initial } => {
                    if min > max {
                        return Err(StoryError::InvalidPack(
                            "variable bounds are inverted".to_owned(),
                        ));
                    }
                    Value::Int(initial.clamp(min, max))
                }
                VariableKind::Bool { initial } => Value::Bool(initial),
            };
            vars.insert(decl.id.clone(), value);
        }
        let mut inventory = BTreeSet::new();
        for item in &pack.initial_inventory {
            if pack.item(item).is_none() {
                return Err(StoryError::InvalidPack(
                    "initial inventory names an undeclared item".to_owned(),
                ));
            }
            inventory.insert(item.clone());
        }
        let mut visited = BTreeSet::new();
        visited.insert(start.id.clone());
        Ok(Self {
            format_version: STORY_STATE_FORMAT_VERSION,
            pack_id: pack.id.clone(),
            node: start.id.clone(),
            vars,
            inventory,
            visited,
            rng: SplitMix64::initial_state(seed),
            turn: 0,
            score: 0,
            ended: start.ending.clone(),
            menu_uses: BTreeMap::new(),
            spent_responses: BTreeSet::new(),
            spent_options: BTreeSet::new(),
            dialogue: None,
            last: None,
        })
    }

    /// Verifies that this state belongs to `pack` and respects every bound:
    /// known node, exactly the declared variables with matching types inside
    /// their bounds, declared items, visited nodes that exist, bounded score
    /// and turn, and v2 counters, spent sets, dialogue position and last
    /// response that all resolve in the pack. A state that passes cannot make
    /// [`crate::step`] exceed a bound.
    pub fn check_against(&self, pack: &StoryPack) -> Result<(), StoryError> {
        let mismatch = |message: &str| Err(StoryError::StateMismatch(message.to_owned()));
        match self.format_version {
            STORY_STATE_FORMAT_VERSION => {}
            1 if self.menu_uses.is_empty()
                && self.spent_responses.is_empty()
                && self.spent_options.is_empty()
                && self.dialogue.is_none()
                && self.last.is_none() => {}
            1 => return mismatch("a v1 state carries v2 fields"),
            found => return Err(StoryError::UnsupportedVersion { found }),
        }
        if self.pack_id != pack.id {
            return mismatch("pack id differs");
        }
        if pack.node(&self.node).is_none() {
            return mismatch("current node is unknown");
        }
        if self.vars.len() != pack.variables.len() {
            return mismatch("variable set differs from the pack");
        }
        for decl in &pack.variables {
            match (self.vars.get(&decl.id), &decl.kind) {
                (Some(Value::Int(value)), VariableKind::Int { min, max, .. }) => {
                    if value < min || value > max {
                        return mismatch("variable is outside its bounds");
                    }
                }
                (Some(Value::Bool(_)), VariableKind::Bool { .. }) => {}
                _ => return mismatch("variable is missing or has the wrong type"),
            }
        }
        if self.inventory.iter().any(|item| pack.item(item).is_none()) {
            return mismatch("inventory names an undeclared item");
        }
        if self.visited.len() > pack.nodes.len()
            || self.visited.iter().any(|node| pack.node(node).is_none())
        {
            return mismatch("visited set names an unknown node");
        }
        if self.score.unsigned_abs() > MAX_SCORE_ABS.unsigned_abs() {
            return mismatch("score is outside its bounds");
        }
        if self.turn > MAX_TURNS {
            return mismatch("turn is outside its bounds");
        }
        for (entry_key, uses) in &self.menu_uses {
            if *uses > MAX_MENU_USES || !entry_exists(pack, entry_key) {
                return mismatch("menu use counter is unknown or out of bounds");
            }
        }
        if !self
            .spent_responses
            .iter()
            .all(|spent| response_exists(pack, spent))
        {
            return mismatch("spent response is unknown");
        }
        if !self
            .spent_options
            .iter()
            .all(|spent| option_exists(pack, spent))
        {
            return mismatch("spent dialogue option is unknown");
        }
        if let Some(position) = &self.dialogue
            && pack
                .dialogue(&position.dialogue)
                .and_then(|dialogue| dialogue.node(&position.node))
                .is_none()
        {
            return mismatch("dialogue position is unknown");
        }
        if let Some(last) = &self.last {
            let known = match last {
                LastResponse::Entry {
                    node,
                    entry,
                    response,
                } => response_exists(pack, &key(&[node, entry, response])),
                LastResponse::Exhausted { node, entry } => entry_exists(pack, &key(&[node, entry])),
                LastResponse::SceneDefault { node, verb } => pack
                    .node(node)
                    .and_then(|node| node.menu.as_ref())
                    .is_some_and(|menu| menu.defaults.iter().any(|d| d.verb == *verb)),
                LastResponse::VerbDefault { verb } => pack.verb(verb).is_some(),
            };
            if !known {
                return mismatch("last response is unknown");
            }
        }
        Ok(())
    }

    /// Parses a bounded state document, checks it against `pack` and upgrades
    /// a v1 state to the current version.
    pub fn from_json_slice(bytes: &[u8], pack: &StoryPack) -> Result<Self, StoryError> {
        if bytes.len() > MAX_STATE_BYTES {
            return Err(StoryError::TooLarge {
                limit: MAX_STATE_BYTES,
            });
        }
        let mut state: Self = serde_json::from_slice(bytes)
            .map_err(|error| StoryError::Malformed(bounded_message(error.to_string())))?;
        state.check_against(pack)?;
        state.format_version = STORY_STATE_FORMAT_VERSION;
        Ok(state)
    }

    /// Serializes the state. Output size is bounded by the pack's bounds.
    pub fn to_json(&self) -> Result<Vec<u8>, StoryError> {
        serde_json::to_vec(self)
            .map_err(|error| StoryError::Malformed(bounded_message(error.to_string())))
    }
}

fn entry_exists(pack: &StoryPack, entry_key: &str) -> bool {
    let Some((node, entry)) = entry_key.split_once('/') else {
        return false;
    };
    pack.node(node)
        .and_then(|node| node.menu.as_ref())
        .and_then(|menu| menu.entry(entry))
        .is_some()
}

fn response_exists(pack: &StoryPack, response_key: &str) -> bool {
    let mut parts = response_key.split('/');
    let (Some(node), Some(entry), Some(response), None) =
        (parts.next(), parts.next(), parts.next(), parts.next())
    else {
        return false;
    };
    pack.node(node)
        .and_then(|node| node.menu.as_ref())
        .and_then(|menu| menu.entry(entry))
        .and_then(|entry| entry.response(response))
        .is_some()
}

fn option_exists(pack: &StoryPack, option_key: &str) -> bool {
    let mut parts = option_key.split('/');
    let (Some(dialogue), Some(node), Some(option), None) =
        (parts.next(), parts.next(), parts.next(), parts.next())
    else {
        return false;
    };
    pack.dialogue(dialogue)
        .and_then(|dialogue| dialogue.node(node))
        .and_then(|node| node.option(option))
        .is_some()
}
