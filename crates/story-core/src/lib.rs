//! Pure, deterministic narrative ruleset for Tiamud story sessions (phase S0).
//!
//! `story-core` is a sibling of `tiamud-core`, not an extension of it. It owns
//! the StoryPack content format, its validator, a closed condition/effect DSL,
//! seeded dice and the turn-based [`step`] transition. It performs no I/O, has
//! no async runtime, reads no clock and draws no entropy: the same pack, seed
//! and input sequence always produce the same states and events.
//!
//! Every quantity is bounded by the constants below. Validation rejects packs
//! that exceed them, and the runtime re-checks state against its pack before
//! every transition, so arbitrary (even unvalidated) inputs return an error
//! instead of panicking or exceeding a bound.
//!
//! Phase S0 is a library prototype only: no server actor, protocol, persistence
//! or client consumes it yet. See ADR 0025. Format v2 adds the core model of
//! menu-driven adventures (verb menus, speakers with portraits, dialogue trees
//! and paginated views) proposed in ADR 0028; it is still library-only.
//! Format v3 adds the parser world proposed in ADR 0031: rooms with exits,
//! objects with a location tree and properties, `before`/`instead`/`after`
//! rules and a default library driven by `tiamud-story-parser`
//! ([`StoryInput::Command`], [`step_command`]). The server does not wire it
//! yet.

#![forbid(unsafe_code)]

mod command;
mod engine;
mod messages;
mod pack;
mod rng;
mod runtime;
mod script;
mod state;
mod text;
mod undo;
mod validate;
mod view;
mod world;

pub use command::{CommandStep, MetaRequest, build_parser, step_command, step_command_scripted};
pub use engine::{
    MenuSelection, StoryEvent, StoryInput, resolve_choice, resolve_dialogue_option, resolve_menu,
    resolve_menu_scripted, step, step_scripted,
};
pub use pack::{
    CharacterDecl, Choice, CmpOp, Condition, Cue, Dialogue, DialogueNext, DialogueNode,
    DialogueOption, DiceSpec, Effect, Ending, EndingKind, Hotspot, ItemDecl, Menu, MenuDefault,
    MenuEntry, MenuVerb, Node, Presence, Response, RollTest, StoryPack, TargetKind, TextBlock,
    Value, VariableDecl, VariableKind, VerbDecl,
};
pub use script::{
    ScriptCall, ScriptDecl, ScriptFailure, ScriptHost, ScriptKind, ScriptRef, ScriptSnapshot,
};
pub use state::{DialoguePosition, LastResponse, PendingQuestion, Reply, StoryState};
pub use text::LocalizedText;
pub use undo::UndoRing;
pub use validate::{ValidationErrors, ValidationIssue, ValidationReport};
pub use view::{
    PageItem, StoryView, ViewBlock, ViewChoice, ViewDialogue, ViewMenu, ViewPage, ViewPortrait,
    ViewPrompt, ViewStat, ViewTarget, ViewVerb, view, view_pages, view_pages_scripted,
    view_scripted,
};
pub use world::{
    Exit, Gender, LexiconRef, LibraryMessage, LocalizedWords, Location, ObjectDecl, ObjectFlag,
    ObjectState, Room, Rule, RulePhase, World,
};

/// Current version of the StoryPack JSON document (v2 adds menus, speakers,
/// portraits and dialogue trees; v3 adds the parser world).
pub const STORY_PACK_FORMAT_VERSION: u32 = 3;
/// Oldest StoryPack version still loaded. v1 and v2 documents are strict
/// subsets of v3 and upgrade by rewriting `format_version` only.
pub const STORY_PACK_MIN_FORMAT_VERSION: u32 = 1;
/// Version of the serialized [`StoryState`] document. v1 states (without menu
/// counters, spent responses or dialogue position) and v2 states (without
/// objects, transcript, pronoun or pending question) still load.
pub const STORY_STATE_FORMAT_VERSION: u32 = 3;

/// Maximum serialized StoryPack size accepted by [`StoryPack::from_json_slice`].
pub const MAX_PACK_BYTES: usize = 2 * 1024 * 1024;
/// Maximum serialized StoryState size accepted by [`StoryState::from_json_slice`].
pub const MAX_STATE_BYTES: usize = 256 * 1024;
/// Maximum bytes of any authored identifier.
pub const MAX_ID_BYTES: usize = 64;
/// Maximum nodes per pack.
pub const MAX_NODES: usize = 4096;
/// Maximum choices authored on one node.
pub const MAX_CHOICES_PER_NODE: usize = 32;
/// Maximum choices across the whole pack.
pub const MAX_TOTAL_CHOICES: usize = 32_768;
/// Maximum effects of one choice, counting nested `if` branches.
pub const MAX_EFFECTS_PER_CHOICE: usize = 64;
/// Maximum nesting of `if` effects.
pub const MAX_EFFECT_DEPTH: usize = 4;
/// Maximum declared variables.
pub const MAX_VARIABLES: usize = 256;
/// Maximum declared items (and therefore inventory size).
pub const MAX_ITEMS: usize = 256;
/// Maximum condition nesting depth.
pub const MAX_CONDITION_DEPTH: usize = 8;
/// Maximum condition nodes in one condition tree.
pub const MAX_CONDITION_NODES: usize = 64;
/// Maximum locales on one localized text.
pub const MAX_LOCALES: usize = 16;
/// Maximum bytes of one locale's node text.
pub const MAX_TEXT_BYTES: usize = 4096;
/// Maximum bytes of one locale's label, title or name.
pub const MAX_LABEL_BYTES: usize = 256;
/// Absolute bound of any integer variable bound, value or delta.
pub const MAX_VAR_ABS: i64 = 1_000_000_000;
/// Absolute bound of the accumulated score.
pub const MAX_SCORE_ABS: i64 = 1_000_000_000;
/// Absolute bound of one score delta.
pub const MAX_SCORE_DELTA: i64 = 1_000_000;
/// Maximum dice in one roll.
pub const MAX_DICE_COUNT: u8 = 16;
/// Maximum faces of one die.
pub const MAX_DICE_SIDES: u8 = 100;
/// Absolute bound of a roll's fixed modifier.
pub const MAX_DICE_MODIFIER: i64 = 1_000_000;
/// Maximum turns a single state may advance.
pub const MAX_TURNS: u32 = 1_000_000;
/// Maximum bytes of a choice token accepted by [`step`].
pub const MAX_TOKEN_BYTES: usize = 32;
/// Maximum issues reported by one validation run.
pub const MAX_REPORTED_ISSUES: usize = 128;
/// Maximum capacity of an [`UndoRing`].
pub const MAX_UNDO_CAPACITY: usize = 64;
/// Maximum declared menu verbs per pack, and verbs offered by one menu.
pub const MAX_VERBS: usize = 16;
/// Maximum hotspots on one node.
pub const MAX_HOTSPOTS_PER_NODE: usize = 32;
/// Maximum characters present on one node.
pub const MAX_PRESENT_PER_NODE: usize = 8;
/// Maximum menu entries on one node.
pub const MAX_MENU_ENTRIES_PER_NODE: usize = 128;
/// Maximum menu entries across the whole pack.
pub const MAX_TOTAL_MENU_ENTRIES: usize = 4096;
/// Maximum responses of one menu entry.
pub const MAX_RESPONSES_PER_ENTRY: usize = 8;
/// Maximum text blocks of one response, default or dialogue node.
pub const MAX_TEXT_BLOCKS: usize = 8;
/// Maximum media cues on one node.
pub const MAX_CUES_PER_NODE: usize = 4;
/// Maximum declared characters.
pub const MAX_CHARACTERS: usize = 64;
/// Maximum portrait expressions of one character.
pub const MAX_EXPRESSIONS: usize = 16;
/// Maximum dialogue trees.
pub const MAX_DIALOGUES: usize = 256;
/// Maximum nodes of one dialogue tree.
pub const MAX_DIALOGUE_NODES: usize = 128;
/// Maximum dialogue nodes across the whole pack.
pub const MAX_TOTAL_DIALOGUE_NODES: usize = 4096;
/// Maximum options of one dialogue node.
pub const MAX_DIALOGUE_OPTIONS: usize = 16;
/// Saturation value of a menu entry use counter.
pub const MAX_MENU_USES: u16 = 9999;
/// Maximum serialized JSON bytes of one [`ViewPage`]. A later protocol frame
/// wraps a page in an envelope and must stay under the 16 KiB frame limit, so
/// 4 KiB are left for that envelope. Validated packs always fit: the largest
/// single page item (one 4096-byte text block, escaped at most 2×, plus a
/// 256-byte speaker name) is far below this bound.
pub const MAX_VIEW_BYTES: usize = 12 * 1024;
/// Maximum lexicons of a parser world.
pub const MAX_LEXICONS: usize = 4;
/// Maximum pack verbs added to one lexicon.
pub const MAX_EXTENSION_VERBS: usize = 32;
/// Maximum world objects (and therefore parser scope size, which
/// `tiamud-story-parser` bounds at 128).
pub const MAX_OBJECTS: usize = 128;
/// Maximum world rules.
pub const MAX_RULES: usize = 512;
/// Maximum exits of one room.
pub const MAX_EXITS: usize = 16;
/// Maximum extra rooms of one fixed object (`found_in`).
pub const MAX_FOUND_IN: usize = 16;
/// Maximum bytes of one locale of an object name. Articles are added on top
/// and the parser accepts names up to 256 bytes.
pub const MAX_OBJECT_NAME_BYTES: usize = 128;
/// Maximum bytes of one locale of a library message override.
pub const MAX_MESSAGE_BYTES: usize = 512;
/// Maximum transcript replies kept for one input.
pub const MAX_REPLIES: usize = 256;
/// Maximum bytes of one stored parser question or error.
pub const MAX_PARSER_REPLY_BYTES: usize = 1024;
/// Maximum replies chained to one pending parser question.
pub const MAX_PENDING_ANSWERS: usize = 4;
/// Maximum objects one `todo`/`all` command acts on.
pub const MAX_MULTI_OBJECTS: usize = 32;
/// Maximum declared scripts (ADR 0034).
pub const MAX_SCRIPTS: usize = 32;
/// Maximum bytes of one script source; equal to the Lua sandbox's own
/// 64 KiB source quota.
pub const MAX_SCRIPT_SOURCE_BYTES: usize = 64 * 1024;
/// Maximum bytes of all script sources of a pack together.
pub const MAX_TOTAL_SCRIPT_BYTES: usize = 256 * 1024;
/// Maximum entries of one script grant list (`writes`, `items`, `goto`,
/// `objects`).
pub const MAX_SCRIPT_GRANTS: usize = 64;
/// Maximum constant arguments of one script reference.
pub const MAX_SCRIPT_ARGS: usize = 8;
/// Maximum script invocations (conditions and effects together) during one
/// step or one view. Exceeding it fails the whole step closed.
pub const MAX_SCRIPT_CALLS_PER_STEP: usize = 16;
/// Maximum intents one effect script may emit.
pub const MAX_SCRIPT_EFFECTS: usize = 32;

/// Errors returned by loading, validating against state, or stepping a story.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum StoryError {
    #[error("story document exceeds {limit} bytes")]
    TooLarge { limit: usize },
    #[error("malformed story document: {0}")]
    Malformed(String),
    #[error("unsupported story format version {found}")]
    UnsupportedVersion { found: u32 },
    #[error("story state does not match this pack: {0}")]
    StateMismatch(String),
    #[error("story pack is inconsistent: {0}")]
    InvalidPack(String),
    #[error("the story has ended")]
    Ended,
    #[error("choice token is malformed")]
    MalformedToken,
    #[error("choice token belongs to a previous view")]
    StaleChoice,
    #[error("choice token does not name an available choice")]
    UnknownChoice,
    #[error("effect budget exceeded")]
    EffectBudget,
    #[error("turn limit reached")]
    TurnLimit,
    #[error("undo capacity must be 1..={max}")]
    InvalidUndoCapacity { max: usize },
    #[error("a dialogue is active; answer it first")]
    InDialogue,
    #[error("no dialogue is active")]
    NotInDialogue,
    #[error("menu target is missing, unexpected or does not belong to the verb")]
    BadTarget,
    #[error("a single view item exceeds {limit} bytes")]
    ViewTooLarge { limit: usize },
    #[error("this story has no parser world")]
    NoParser,
    /// A script reference was reached but the runtime has no script host.
    #[error("this runtime has no script host; the pack's scripts cannot run")]
    ScriptUnavailable,
    /// More than [`MAX_SCRIPT_CALLS_PER_STEP`] script invocations.
    #[error("more than {limit} script calls in one step")]
    ScriptBudget { limit: usize },
    /// A script failed or emitted an intent it was not granted. The message
    /// is author-facing; transports must show players a generic error.
    #[error("script {script:?} failed: {message}")]
    Script { script: String, message: String },
}

/// Returns true for a bounded authored identifier: 1..=64 bytes, starting with
/// an ASCII lowercase letter or digit, then lowercase letters, digits, `-`,
/// `_` or `.`.
pub(crate) fn valid_id(id: &str) -> bool {
    let bytes = id.as_bytes();
    if bytes.is_empty() || bytes.len() > MAX_ID_BYTES {
        return false;
    }
    if !(bytes[0].is_ascii_lowercase() || bytes[0].is_ascii_digit()) {
        return false;
    }
    bytes
        .iter()
        .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || matches!(b, b'-' | b'_' | b'.'))
}

/// Truncates diagnostic text to a bounded size on a char boundary.
pub(crate) fn bounded_message(message: impl Into<String>) -> String {
    const LIMIT: usize = 256;
    let mut message = message.into();
    if message.len() > LIMIT {
        let mut cut = LIMIT;
        while !message.is_char_boundary(cut) {
            cut -= 1;
        }
        message.truncate(cut);
    }
    message
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ids_are_bounded_and_restricted() {
        assert!(valid_id("sala-1.b_c"));
        assert!(valid_id("9"));
        assert!(!valid_id(""));
        assert!(!valid_id("-x"));
        assert!(!valid_id("Sala"));
        assert!(!valid_id("sala con espacio"));
        assert!(!valid_id("ñ"));
        assert!(!valid_id(&"a".repeat(MAX_ID_BYTES + 1)));
        assert!(valid_id(&"a".repeat(MAX_ID_BYTES)));
    }

    #[test]
    fn messages_truncate_on_char_boundary() {
        let long = "é".repeat(300);
        let cut = bounded_message(long);
        assert!(cut.len() <= 256);
        assert!(cut.chars().all(|c| c == 'é'));
    }
}
