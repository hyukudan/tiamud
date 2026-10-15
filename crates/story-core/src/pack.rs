//! StoryPack content model. Every struct rejects unknown fields so a typo in
//! authored JSON fails loudly instead of being ignored.

use serde::{Deserialize, Serialize};

use crate::{
    LocalizedText, MAX_PACK_BYTES, STORY_PACK_FORMAT_VERSION, StoryError, bounded_message,
};

/// A complete, self-contained narrative: nodes, choices, typed variables and
/// declared items. Identifiers are durable once published.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StoryPack {
    pub format_version: u32,
    pub id: String,
    pub title: LocalizedText,
    /// Node where every new state begins.
    pub start: String,
    #[serde(default)]
    pub variables: Vec<VariableDecl>,
    #[serde(default)]
    pub items: Vec<ItemDecl>,
    #[serde(default)]
    pub initial_inventory: Vec<String>,
    pub nodes: Vec<Node>,
}

impl StoryPack {
    /// Parses a bounded JSON document. This only checks size, syntax, shape
    /// and format version; call [`StoryPack::validate`] before use.
    pub fn from_json_slice(bytes: &[u8]) -> Result<Self, StoryError> {
        if bytes.len() > MAX_PACK_BYTES {
            return Err(StoryError::TooLarge {
                limit: MAX_PACK_BYTES,
            });
        }
        let pack: Self = serde_json::from_slice(bytes)
            .map_err(|error| StoryError::Malformed(bounded_message(error.to_string())))?;
        if pack.format_version != STORY_PACK_FORMAT_VERSION {
            return Err(StoryError::UnsupportedVersion {
                found: pack.format_version,
            });
        }
        Ok(pack)
    }

    pub(crate) fn node(&self, id: &str) -> Option<&Node> {
        self.nodes.iter().find(|node| node.id == id)
    }

    pub(crate) fn variable(&self, id: &str) -> Option<&VariableDecl> {
        self.variables.iter().find(|decl| decl.id == id)
    }

    pub(crate) fn item(&self, id: &str) -> Option<&ItemDecl> {
        self.items.iter().find(|item| item.id == id)
    }
}

/// A typed, bounded story variable.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct VariableDecl {
    pub id: String,
    /// Shown on the stat sheet when `visible` is true.
    #[serde(default)]
    pub label: Option<LocalizedText>,
    #[serde(default)]
    pub visible: bool,
    pub kind: VariableKind,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub enum VariableKind {
    /// Integer clamped to `min..=max` after every change.
    Int {
        min: i64,
        max: i64,
        initial: i64,
    },
    Bool {
        initial: bool,
    },
}

/// A declared inventory item. Inventory is a set of these ids.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ItemDecl {
    pub id: String,
    pub name: LocalizedText,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Node {
    pub id: String,
    pub text: LocalizedText,
    #[serde(default)]
    pub choices: Vec<Choice>,
    /// Entering a node with an ending ends the story.
    #[serde(default)]
    pub ending: Option<Ending>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Choice {
    /// Authored id, unique within its node. Never sent to players.
    pub id: String,
    pub label: LocalizedText,
    /// Availability gate. Unavailable choices are never shown.
    #[serde(default)]
    pub when: Option<Condition>,
    #[serde(default)]
    pub effects: Vec<Effect>,
    /// Default destination; a `goto` or roll test effect overrides it. With
    /// neither, the story stays on the current node.
    #[serde(default)]
    pub target: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Ending {
    pub id: String,
    pub kind: EndingKind,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EndingKind {
    Victory,
    Defeat,
    Neutral,
}

/// A variable value. Serialized as a bare JSON number or boolean.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum Value {
    Bool(bool),
    Int(i64),
}

/// Closed condition AST. No prose, clock or randomness is ever consulted.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub enum Condition {
    All(Vec<Condition>),
    Any(Vec<Condition>),
    Not(Box<Condition>),
    /// Compares an integer variable with a constant.
    VarCmp {
        var: String,
        op: CmpOp,
        value: i64,
    },
    HasItem(String),
    Visited(String),
    /// True when a boolean variable is true.
    Flag(String),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CmpOp {
    Eq,
    Ne,
    Lt,
    Le,
    Gt,
    Ge,
}

impl CmpOp {
    pub(crate) fn apply(self, left: i64, right: i64) -> bool {
        match self {
            Self::Eq => left == right,
            Self::Ne => left != right,
            Self::Lt => left < right,
            Self::Le => left <= right,
            Self::Gt => left > right,
            Self::Ge => left >= right,
        }
    }
}

/// `count`d`sides` + `modifier`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DiceSpec {
    pub count: u8,
    pub sides: u8,
    #[serde(default)]
    pub modifier: i64,
}

/// Optional pass/fail test of a roll total. A destination set here overrides
/// the choice target like a `goto`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RollTest {
    pub at_least: i64,
    #[serde(default)]
    pub success: Option<String>,
    #[serde(default)]
    pub failure: Option<String>,
}

/// Closed effect DSL applied in order when a choice is taken.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub enum Effect {
    /// Assigns a value of the variable's declared type (clamped if integer).
    Set {
        var: String,
        value: Value,
    },
    /// Checked addition, saturated and then clamped to the declared bounds.
    Add {
        var: String,
        delta: i64,
    },
    Give(String),
    Take(String),
    /// Sets the destination; the last one wins.
    Goto(String),
    /// Rolls seeded dice, optionally adds an integer variable, stores the
    /// clamped total and tests it.
    Roll {
        dice: DiceSpec,
        #[serde(default)]
        modifier_var: Option<String>,
        #[serde(default)]
        store: Option<String>,
        #[serde(default)]
        test: Option<RollTest>,
    },
    /// Bounded branching evaluated against the state at that point.
    If {
        when: Condition,
        #[serde(default)]
        then: Vec<Effect>,
        #[serde(default)]
        otherwise: Vec<Effect>,
    },
    /// Presentation-only media cue; losing it never changes play.
    Cue(Cue),
    /// Adds to the score.
    Score(i64),
    /// Ends the story immediately; remaining effects are skipped.
    End(Ending),
}

/// Media cue naming a reviewed asset id, never a URL.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub enum Cue {
    Music { asset: String },
    Sfx { asset: String },
    StopMusic,
}
