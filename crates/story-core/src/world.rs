//! StoryPack v3 parser world: rooms with exits, objects with properties and a
//! location tree, the parser vocabulary, and `before`/`instead`/`after`
//! rules over the closed condition/effect DSL (ADR 0031).
//!
//! Everything here is optional: a pack without a `world` and without node
//! `room`s is a v1/v2 pack and behaves exactly as before.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::{Condition, Effect, LocalizedText};

/// Words per locale, e.g. `{"es": ["lámpara", "farol"], "en": ["lamp"]}`.
pub type LocalizedWords = BTreeMap<String, Vec<String>>;

/// The parser world of a v3 pack.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct World {
    /// Shipped lexicons the pack is played with, by id (`es`, `en`), each
    /// with optional pack-specific verbs. The first one is the default.
    pub lexicons: Vec<LexiconRef>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub objects: Vec<ObjectDecl>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub rules: Vec<Rule>,
    /// Overrides of the default library messages.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub messages: BTreeMap<LibraryMessage, LocalizedText>,
    /// Text shown once, before the first room description.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub intro: Option<LocalizedText>,
    /// Maximum score shown by the `score` meta command.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_score: Option<i64>,
}

impl World {
    pub(crate) fn object(&self, id: &str) -> Option<&ObjectDecl> {
        self.objects.iter().find(|object| object.id == id)
    }

    pub(crate) fn rule(&self, id: &str) -> Option<&Rule> {
        self.rules.iter().find(|rule| rule.id == id)
    }

    /// Language tags of the lexicons, in pack order.
    pub fn languages(&self) -> Vec<&str> {
        self.lexicons
            .iter()
            .map(|lexicon| lexicon.id.as_str())
            .collect()
    }
}

/// A shipped lexicon (`es`, `en`) plus pack verbs. Extension verbs use the
/// lexicon's own JSON shape and may reuse library action ids as synonyms or
/// declare new actions that only rules handle.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LexiconRef {
    pub id: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub verbs: Vec<tiamud_story_parser::VerbDecl>,
}

/// Room data of a node (format v3). A node with a room is a place the parser
/// can act in; its `text` is the room description.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Room {
    /// Short room name shown as the scene heading.
    pub name: LocalizedText,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub exits: Vec<Exit>,
    /// A dark room hides its contents unless a lit object is in reach.
    #[serde(default, skip_serializing_if = "is_false")]
    pub dark: bool,
}

/// A way out of a room, keyed by a canonical lexicon direction id.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Exit {
    pub direction: String,
    pub to: String,
    /// Passability gate; when it fails `blocked` (or the default "you can't
    /// go that way") is shown.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub when: Option<Condition>,
    /// Door object that must be open to pass.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub door: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub blocked: Option<LocalizedText>,
}

/// Where an object is.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub enum Location {
    /// On the floor of a room node.
    Room(String),
    /// Held by the player.
    Carried,
    /// Inside a container object.
    In(String),
    /// Out of play.
    Nowhere,
}

/// Grammatical gender used by Spanish default messages (`{o}` → `o`/`a`).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Gender {
    #[default]
    Masculine,
    Feminine,
}

/// A declared world object. Ids are durable once published.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ObjectDecl {
    pub id: String,
    /// Name without article (`lámpara de aceite`); messages add articles
    /// unless `proper` is set.
    pub name: LocalizedText,
    /// Parser nouns per lexicon language.
    pub nouns: LocalizedWords,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub adjectives: LocalizedWords,
    pub description: LocalizedText,
    pub location: Location,
    /// Further rooms a fixed object (scenery, door) is also present in.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub found_in: Vec<String>,
    #[serde(default, skip_serializing_if = "is_default_gender")]
    pub gender: Gender,
    /// A proper name takes no article (`Remedios`).
    #[serde(default, skip_serializing_if = "is_false")]
    pub proper: bool,
    #[serde(default, skip_serializing_if = "is_false")]
    pub portable: bool,
    /// Never listed in room descriptions and never takeable.
    #[serde(default, skip_serializing_if = "is_false")]
    pub scenery: bool,
    /// A person or creature: talk, give and show target it.
    #[serde(default, skip_serializing_if = "is_false")]
    pub animate: bool,
    #[serde(default, skip_serializing_if = "is_false")]
    pub container: bool,
    #[serde(default, skip_serializing_if = "is_false")]
    pub openable: bool,
    /// Initially open (always open for containers that are not openable).
    #[serde(default, skip_serializing_if = "is_false")]
    pub open: bool,
    #[serde(default, skip_serializing_if = "is_false")]
    pub locked: bool,
    /// Object that locks and unlocks this one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub key: Option<String>,
    #[serde(default, skip_serializing_if = "is_false")]
    pub lightable: bool,
    /// Initially lit; a lit object in reach lights a dark room.
    #[serde(default, skip_serializing_if = "is_false")]
    pub lit: bool,
    #[serde(default, skip_serializing_if = "is_false")]
    pub wearable: bool,
    /// Initially worn (requires `location: carried`).
    #[serde(default, skip_serializing_if = "is_false")]
    pub worn: bool,
    /// Text shown by `read`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub text: Option<LocalizedText>,
}

/// When a rule runs relative to the library action.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RulePhase {
    /// Runs first, then the action continues unless the rule moved the
    /// player or ended the story.
    Before,
    /// Replaces the library action entirely.
    Instead,
    /// Runs after a successful library action.
    After,
}

/// A rule keyed by `(action, obj?, obj2?)` (and `direction` for `go`).
/// Absent keys match anything. Within a phase the first matching rule whose
/// `when` holds runs; rules never read prose.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Rule {
    pub id: String,
    pub phase: RulePhase,
    pub action: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub obj: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub obj2: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub direction: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub when: Option<Condition>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub effects: Vec<Effect>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub text: Option<LocalizedText>,
}

/// Mutable flags of an object, for conditions and effects.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ObjectFlag {
    Open,
    Locked,
    Lit,
    Worn,
}

/// Dynamic state of one object.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ObjectState {
    pub at: Location,
    #[serde(default, skip_serializing_if = "is_false")]
    pub open: bool,
    #[serde(default, skip_serializing_if = "is_false")]
    pub locked: bool,
    #[serde(default, skip_serializing_if = "is_false")]
    pub lit: bool,
    #[serde(default, skip_serializing_if = "is_false")]
    pub worn: bool,
}

impl ObjectState {
    pub(crate) fn initial(decl: &ObjectDecl) -> Self {
        Self {
            at: decl.location.clone(),
            open: decl.open || (decl.container && !decl.openable),
            locked: decl.locked,
            lit: decl.lit,
            worn: decl.worn,
        }
    }

    pub(crate) fn flag(&self, flag: ObjectFlag) -> bool {
        match flag {
            ObjectFlag::Open => self.open,
            ObjectFlag::Locked => self.locked,
            ObjectFlag::Lit => self.lit,
            ObjectFlag::Worn => self.worn,
        }
    }
}

/// Default library messages. Packs may override any of them through
/// `world.messages`. Placeholders: `{obj}`/`{Obj}` (definite name, the
/// capitalized variant), `{aobj}` (indefinite name), `{obj2}`/`{Obj2}`,
/// `{o}`/`{o2}` (Spanish gender ending), `{list}`, `{score}`, `{max}`,
/// `{turns}`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LibraryMessage {
    Dark,
    DarkName,
    YouSee,
    InsideYouSee,
    Empty,
    Inventory,
    InventoryEmpty,
    Wearing,
    Taken,
    AlreadyCarried,
    NotPortable,
    TakeAnimate,
    NothingToTake,
    Dropped,
    NotCarried,
    NothingToDrop,
    TakeOffFirst,
    Opened,
    NotOpenable,
    AlreadyOpen,
    Locked,
    Closed,
    NotClosable,
    AlreadyClosed,
    Unlocked,
    NoLock,
    NotLocked,
    WrongKey,
    LockedWith,
    AlreadyLocked,
    CloseFirst,
    PutIn,
    NotContainer,
    ContainerClosed,
    PutInSelf,
    PutOn,
    NoExit,
    DoorClosed,
    Wait,
    CantEnter,
    GiveNotAnimate,
    GiveDeclined,
    ShowDeclined,
    TalkNotAnimate,
    TalkNothing,
    NothingWritten,
    Worn,
    NotWearable,
    AlreadyWorn,
    Removed,
    NotWorn,
    SwitchedOn,
    NotLightable,
    AlreadyOn,
    SwitchedOff,
    AlreadyOff,
    GoesDark,
    SearchNothing,
    NothingHappens,
    Score,
    ScoreOf,
    Help,
}

pub(crate) fn is_false(value: &bool) -> bool {
    !*value
}

fn is_default_gender(gender: &Gender) -> bool {
    *gender == Gender::Masculine
}
