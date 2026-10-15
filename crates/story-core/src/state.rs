use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};

use crate::rng::SplitMix64;
use crate::{
    Ending, MAX_SCORE_ABS, MAX_STATE_BYTES, MAX_TURNS, STORY_STATE_FORMAT_VERSION, StoryError,
    StoryPack, Value, VariableKind, bounded_message,
};

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
        })
    }

    /// Verifies that this state belongs to `pack` and respects every bound:
    /// known node, exactly the declared variables with matching types inside
    /// their bounds, declared items, visited nodes that exist, bounded score
    /// and turn. A state that passes cannot make [`crate::step`] exceed a bound.
    pub fn check_against(&self, pack: &StoryPack) -> Result<(), StoryError> {
        let mismatch = |message: &str| Err(StoryError::StateMismatch(message.to_owned()));
        if self.format_version != STORY_STATE_FORMAT_VERSION {
            return Err(StoryError::UnsupportedVersion {
                found: self.format_version,
            });
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
        Ok(())
    }

    /// Parses a bounded state document and checks it against `pack`.
    pub fn from_json_slice(bytes: &[u8], pack: &StoryPack) -> Result<Self, StoryError> {
        if bytes.len() > MAX_STATE_BYTES {
            return Err(StoryError::TooLarge {
                limit: MAX_STATE_BYTES,
            });
        }
        let state: Self = serde_json::from_slice(bytes)
            .map_err(|error| StoryError::Malformed(bounded_message(error.to_string())))?;
        state.check_against(pack)?;
        Ok(state)
    }

    /// Serializes the state. Output size is bounded by the pack's bounds.
    pub fn to_json(&self) -> Result<Vec<u8>, StoryError> {
        serde_json::to_vec(self)
            .map_err(|error| StoryError::Malformed(bounded_message(error.to_string())))
    }
}
