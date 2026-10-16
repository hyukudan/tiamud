//! StoryPack content model. Every struct rejects unknown fields so a typo in
//! authored JSON fails loudly instead of being ignored.
//!
//! Format v2 adds menu-driven scenes (verbs × targets → responses), speakers
//! with portraits, dialogue trees, backgrounds and per-node cues. Every v2
//! field is optional and omitted when empty, so a v1 document is a valid v2
//! document; the validator rejects v2 features in a document that still
//! declares `format_version: 1`.
//!
//! Format v3 adds the parser world (ADR 0031): a `world` section with
//! lexicons, objects and rules, `room` data on nodes, and object conditions
//! and effects. As before, every v3 field is optional and the validator
//! rejects v3 features in a document that declares an older version.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::script::{ScriptDecl, ScriptRef};
use crate::world::{Location, ObjectFlag, Room, World};
use crate::{
    LocalizedText, MAX_PACK_BYTES, STORY_PACK_FORMAT_VERSION, STORY_PACK_MIN_FORMAT_VERSION,
    StoryError, bounded_message,
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
    /// Menu verbs (format v2). Their order is the order menus offer them in
    /// only through each node's own `menu.verbs` list.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub verbs: Vec<VerbDecl>,
    /// Speakers with portraits (format v2).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub characters: Vec<CharacterDecl>,
    /// Dialogue trees (format v2), entered from a menu response.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub dialogues: Vec<Dialogue>,
    /// Parser world (format v3): lexicons, objects and rules.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub world: Option<World>,
    /// Optional Lua scripts (format v3, phase S6, ADR 0034). A pack that
    /// declares any only validates with [`StoryPack::validate_scripted`] and
    /// only runs through a [`crate::ScriptHost`].
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub scripts: Vec<ScriptDecl>,
    /// Earlier revisions of this pack whose saved games still load (format
    /// v3): lowercase hexadecimal SHA-256 digests of their exact file bytes,
    /// at most [`crate::MAX_COMPATIBLE_REVISIONS`]. A slot written by a listed
    /// revision is re-validated against this pack before it is restored;
    /// nothing is migrated.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub compatible_revisions: Vec<String>,
}

impl StoryPack {
    /// Parses a bounded JSON document. This only checks size, syntax, shape
    /// and format version (v1 to v3); call [`StoryPack::validate`] before use.
    pub fn from_json_slice(bytes: &[u8]) -> Result<Self, StoryError> {
        if bytes.len() > MAX_PACK_BYTES {
            return Err(StoryError::TooLarge {
                limit: MAX_PACK_BYTES,
            });
        }
        let pack: Self = serde_json::from_slice(bytes)
            .map_err(|error| StoryError::Malformed(bounded_message(error.to_string())))?;
        if !(STORY_PACK_MIN_FORMAT_VERSION..=STORY_PACK_FORMAT_VERSION)
            .contains(&pack.format_version)
        {
            return Err(StoryError::UnsupportedVersion {
                found: pack.format_version,
            });
        }
        Ok(pack)
    }

    /// Upgrades a v1 or v2 pack to the current format in place. Each format is
    /// a strict superset of the previous one, so the upgrade only rewrites
    /// `format_version`; no id, text or rule changes. Returns true when the
    /// version changed.
    pub fn upgrade(&mut self) -> bool {
        if self.format_version < STORY_PACK_FORMAT_VERSION {
            self.format_version = STORY_PACK_FORMAT_VERSION;
            true
        } else {
            false
        }
    }

    /// The language a pack is played in when nobody asks for another one
    /// (lowercase): the first parser lexicon of a v3 world, in pack order;
    /// without a parser, English when the title offers it, else the title's
    /// first locale; `en` for a pack without any title locale.
    pub fn default_locale(&self) -> String {
        if let Some(lexicon) = self.world.as_ref().and_then(|world| world.lexicons.first()) {
            return lexicon.id.to_ascii_lowercase();
        }
        let locales = &self.title.0;
        if locales
            .keys()
            .any(|locale| locale.eq_ignore_ascii_case("en"))
        {
            return "en".to_owned();
        }
        locales
            .keys()
            .next()
            .map_or_else(|| "en".to_owned(), |locale| locale.to_ascii_lowercase())
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

    pub(crate) fn verb(&self, id: &str) -> Option<&VerbDecl> {
        self.verbs.iter().find(|verb| verb.id == id)
    }

    pub(crate) fn character(&self, id: &str) -> Option<&CharacterDecl> {
        self.characters.iter().find(|character| character.id == id)
    }

    pub(crate) fn dialogue(&self, id: &str) -> Option<&Dialogue> {
        self.dialogues.iter().find(|dialogue| dialogue.id == id)
    }

    /// The world object `id`, if the pack has a world and declares it.
    pub(crate) fn object(&self, id: &str) -> Option<&crate::world::ObjectDecl> {
        self.world.as_ref().and_then(|world| world.object(id))
    }

    /// The declared script `id`.
    pub(crate) fn script(&self, id: &str) -> Option<&ScriptDecl> {
        self.scripts.iter().find(|script| script.id == id)
    }

    /// Room data of node `id`, if the node exists and is a room.
    pub(crate) fn room(&self, id: &str) -> Option<&Room> {
        self.node(id).and_then(|node| node.room.as_ref())
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
    /// Reviewed background image asset id (format v2). Presentation only.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub background: Option<String>,
    /// Media cues emitted on every entry into the node (format v2).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub cues: Vec<Cue>,
    /// Things a hotspot verb can target in this scene (format v2).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub hotspots: Vec<Hotspot>,
    /// Characters present in this scene (format v2); character verbs target
    /// them and the view shows their portraits.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub present: Vec<Presence>,
    /// Verb menu of this scene (format v2).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub menu: Option<Menu>,
    /// Room of the parser world (format v3): name, exits and darkness.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub room: Option<Room>,
}

impl Node {
    pub(crate) fn hotspot(&self, id: &str) -> Option<&Hotspot> {
        self.hotspots.iter().find(|hotspot| hotspot.id == id)
    }

    pub(crate) fn presence(&self, character: &str) -> Option<&Presence> {
        self.present
            .iter()
            .find(|presence| presence.character == character)
    }

    /// True when the node uses any format v2 feature.
    pub(crate) fn uses_v2(&self) -> bool {
        self.background.is_some()
            || !self.cues.is_empty()
            || !self.hotspots.is_empty()
            || !self.present.is_empty()
            || self.menu.is_some()
    }
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
    /// Compares how many times a menu entry has been used (format v2). The
    /// counter saturates at [`crate::MAX_MENU_USES`].
    Uses {
        node: String,
        entry: String,
        op: CmpOp,
        value: i64,
    },
    /// True when the object is held, directly or inside held containers
    /// (format v3).
    Carried(String),
    /// True when the object is exactly at `at` (format v3).
    ObjectAt {
        obj: String,
        at: Location,
    },
    /// True when the object's flag is set (format v3).
    ObjectIs {
        obj: String,
        flag: ObjectFlag,
    },
    /// True when the object is in the parser's scope now: in the room (and
    /// lit) or held, with every enclosing container open (format v3).
    Visible(String),
    /// True when the current node is `node` (format v3).
    PlayerIn(String),
    /// Compares the turn counter with a constant (format v3).
    TurnCmp {
        op: CmpOp,
        value: i64,
    },
    /// Evaluates a declared condition script through the runtime's
    /// [`crate::ScriptHost`] (format v3, ADR 0034). Without a host it fails
    /// closed.
    Script(ScriptRef),
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
    /// Moves a world object (format v3). A move that would put a container
    /// inside itself is ignored; leaving the player's hands clears `worn`.
    MoveObject {
        obj: String,
        to: Location,
    },
    /// Sets or clears an object flag (format v3). Setting `worn` on an
    /// object that is not held is ignored.
    SetObject {
        obj: String,
        flag: ObjectFlag,
        value: bool,
    },
    /// Runs a declared effect script through the runtime's
    /// [`crate::ScriptHost`] (format v3, ADR 0034) and applies the closed
    /// intents it emits, each checked against the script's grants. Without a
    /// host it fails closed.
    Script(ScriptRef),
}

/// Media cue naming a reviewed asset id, never a URL.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub enum Cue {
    Music { asset: String },
    Sfx { asset: String },
    StopMusic,
}

/// What a menu verb is applied to.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TargetKind {
    /// The verb acts on the scene itself and takes no target.
    None,
    /// A hotspot declared on the current node.
    Hotspot,
    /// A character present on the current node.
    Character,
    /// An item currently held.
    Item,
}

/// A menu verb such as MIRAR or HABLAR. Ids are authored and durable; labels
/// are display text only and never parsed.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct VerbDecl {
    pub id: String,
    pub label: LocalizedText,
    pub targets: TargetKind,
    /// Response when neither an entry nor a scene default applies.
    pub default: Vec<TextBlock>,
}

/// A speaker with an expression → reviewed portrait asset id map.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CharacterDecl {
    pub id: String,
    pub name: LocalizedText,
    /// Expression id (for example `neutral`, `enfadado`) → portrait asset id.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub portraits: BTreeMap<String, String>,
    /// Expression used when a block or presence names none. Defaults to
    /// `neutral`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default_expression: Option<String>,
}

impl CharacterDecl {
    /// Portrait asset for `expression` (or the default one). A missing
    /// portrait only loses media, never play.
    pub(crate) fn portrait(&self, expression: Option<&str>) -> Option<&String> {
        let expression = expression
            .or(self.default_expression.as_deref())
            .unwrap_or("neutral");
        self.portraits.get(expression)
    }
}

/// One paragraph of displayed text, optionally spoken by a character with an
/// expression. Text is presentation only; no rule reads it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TextBlock {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub speaker: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expression: Option<String>,
    pub text: LocalizedText,
}

/// Something in a scene that hotspot verbs may target.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Hotspot {
    pub id: String,
    pub label: LocalizedText,
    /// Visibility gate; a hidden hotspot is not offered as a target.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub when: Option<Condition>,
}

/// A character standing in a scene.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Presence {
    pub character: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expression: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub when: Option<Condition>,
}

/// Verb menu of one scene.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Menu {
    /// Offered verbs, in display order.
    pub verbs: Vec<MenuVerb>,
    /// Authored responses per (verb, target).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub entries: Vec<MenuEntry>,
    /// Scene-specific fallbacks per verb, used before the verb's own default.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub defaults: Vec<MenuDefault>,
}

impl Menu {
    pub(crate) fn entry(&self, id: &str) -> Option<&MenuEntry> {
        self.entries.iter().find(|entry| entry.id == id)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MenuVerb {
    pub verb: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub when: Option<Condition>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MenuDefault {
    pub verb: String,
    pub blocks: Vec<TextBlock>,
}

/// Responses to one verb applied to one target in one scene.
///
/// Every use increments a bounded per-entry counter (see
/// [`Condition::Uses`]). The first response whose `when` holds and that is
/// not a spent `once` response is given; when none applies, `exhausted`
/// ("nothing new") is shown, then the scene default, then the verb default.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MenuEntry {
    /// Authored id, unique within its node. Never sent to players.
    pub id: String,
    pub verb: String,
    /// Hotspot, character or item id, as the verb's [`TargetKind`] requires;
    /// absent for verbs without targets.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub target: Option<String>,
    pub responses: Vec<Response>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub exhausted: Vec<TextBlock>,
}

impl MenuEntry {
    pub(crate) fn response(&self, id: &str) -> Option<&Response> {
        self.responses.iter().find(|response| response.id == id)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Response {
    /// Authored id, unique within its entry. Never sent to players.
    pub id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub when: Option<Condition>,
    /// A spent `once` response is never given again.
    #[serde(default, skip_serializing_if = "is_false")]
    pub once: bool,
    pub blocks: Vec<TextBlock>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub effects: Vec<Effect>,
    /// Scene to move to; a `goto` effect overrides it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub target: Option<String>,
    /// Dialogue to start when the response neither moves nor ends the story.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub dialogue: Option<String>,
}

/// A bounded dialogue tree. There is no call stack: at most one dialogue is
/// active and `return` only goes back to its `start` node.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Dialogue {
    pub id: String,
    pub start: String,
    pub nodes: Vec<DialogueNode>,
}

impl Dialogue {
    pub(crate) fn node(&self, id: &str) -> Option<&DialogueNode> {
        self.nodes.iter().find(|node| node.id == id)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DialogueNode {
    pub id: String,
    pub lines: Vec<TextBlock>,
    pub options: Vec<DialogueOption>,
}

impl DialogueNode {
    pub(crate) fn option(&self, id: &str) -> Option<&DialogueOption> {
        self.options.iter().find(|option| option.id == id)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DialogueOption {
    /// Authored id, unique within its dialogue node. Never sent to players.
    pub id: String,
    pub label: LocalizedText,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub when: Option<Condition>,
    /// A taken `once` option is never offered again.
    #[serde(default, skip_serializing_if = "is_false")]
    pub once: bool,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub effects: Vec<Effect>,
    /// Where the conversation continues. A `goto` or `end` effect always
    /// leaves the dialogue.
    #[serde(default)]
    pub next: DialogueNext,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub enum DialogueNext {
    /// Continue at another node of the same dialogue.
    Goto(String),
    /// Go back to the dialogue's start node.
    Return,
    /// Leave the dialogue and return to the scene menu.
    #[default]
    Exit,
}

fn is_false(value: &bool) -> bool {
    !*value
}
