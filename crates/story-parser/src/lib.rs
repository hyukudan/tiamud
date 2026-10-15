//! Pure, bounded, data-driven command parser for parser-based interactive
//! fiction (Spanish first, English too).
//!
//! The parser turns one player line such as `coge la llave dorada y ábrela`
//! or `put the lamp in the box` into typed [`Command`]s whose objects are ids
//! supplied by the caller through the [`Scope`] trait. It follows the
//! conventions of the Spanish *aventuras conversacionales* tradition
//! (PAWS/DAAD and the Spanish Inform 6 library): infinitive and imperative
//! verbs, enclitic pronouns (`cógelo`, `dáselo`, `háblale`), the contractions
//! `del`/`al`, `todo … excepto …`, single-letter directions and meta verbs.
//!
//! Everything about a language lives in a versioned JSON [`Lexicon`]; the
//! Spanish and English lexicons ship as reviewed fixtures. The crate performs
//! no I/O, holds no global state, reads no clock and draws no entropy: the
//! same lexicon, scope and input always produce the same outcome. Every
//! quantity is bounded by the constants below; no input panics, and no
//! regular expressions are used.
//!
//! `tiamud-story-core` uses it for StoryPack format v3 parser worlds (ADR
//! 0031); this crate does not depend on it. No server or protocol uses the
//! parser yet; see `docs/authoring/story-parser.md`.

#![forbid(unsafe_code)]

mod lexicon;
mod parse;
mod scope;
mod text;

pub use lexicon::{
    ContractionDecl, DirectionDecl, EncliticDecl, EncliticKind, Lexicon, LexiconError, Messages,
    PatternDecl, PhraseDecl, PrepositionDecl, VerbDecl,
};
pub use parse::{
    ActionId, Candidate, Command, DirectionId, ObjRef, ParseError, ParseErrorKind, ParseOutcome,
    Parser, Pending, PrepId, Segment,
};
pub use scope::{Scope, ScopeObject, SimpleScope};
pub use text::fold;

/// Version of the lexicon JSON document accepted by this crate.
pub const LEXICON_FORMAT_VERSION: u32 = 1;

/// Maximum bytes of one player line. Longer lines are rejected whole.
pub const MAX_INPUT_BYTES: usize = 256;
/// Maximum words in one player line, before contraction expansion.
pub const MAX_WORDS: usize = 16;
/// Maximum commands in one player line.
pub const MAX_COMMANDS: usize = 4;
/// Maximum candidates offered by one ambiguity question.
pub const MAX_CANDIDATES: usize = 8;
/// Maximum objects named after `excepto`/`except`.
pub const MAX_EXCEPT: usize = 8;

/// Maximum serialized lexicon size.
pub const MAX_LEXICON_BYTES: usize = 256 * 1024;
/// Maximum bytes of one lexicon or scope word after folding.
pub const MAX_WORD_BYTES: usize = 32;
/// Maximum bytes of an action, preposition or direction id.
pub const MAX_ID_BYTES: usize = 64;
/// Maximum verbs in one lexicon.
pub const MAX_VERBS: usize = 256;
/// Maximum words of one verb, preposition, direction or word list.
pub const MAX_WORDS_PER_ENTRY: usize = 64;
/// Maximum grammar patterns of one verb.
pub const MAX_PATTERNS_PER_VERB: usize = 8;
/// Maximum prepositions in one lexicon.
pub const MAX_PREPOSITIONS: usize = 64;
/// Maximum directions in one lexicon.
pub const MAX_DIRECTIONS: usize = 32;
/// Maximum enclitic suffixes in one lexicon.
pub const MAX_ENCLITICS: usize = 64;
/// Maximum contractions in one lexicon.
pub const MAX_CONTRACTIONS: usize = 64;
/// Maximum words one contraction expands into.
pub const MAX_CONTRACTION_EXPANSION: usize = 3;
/// Maximum multi-word phrases (and command-separator phrases) in one lexicon.
pub const MAX_PHRASES: usize = 64;
/// Maximum words in one phrase.
pub const MAX_PHRASE_WORDS: usize = 4;
/// Maximum bytes of one message template.
pub const MAX_MESSAGE_BYTES: usize = 256;

/// Maximum objects one scope may expose to a single parse.
pub const MAX_SCOPE_OBJECTS: usize = 128;
/// Maximum nouns, and separately adjectives, of one scope object.
pub const MAX_SCOPE_WORDS_PER_OBJECT: usize = 16;
/// Maximum bytes of a scope object's display name.
pub const MAX_SCOPE_NAME_BYTES: usize = 256;

/// Returns true for a bounded identifier: 1..=64 bytes, starting with an ASCII
/// lowercase letter or digit, then lowercase letters, digits, `-`, `_` or `.`.
/// This is the same rule StoryPack identifiers follow.
pub(crate) fn valid_id(id: &str) -> bool {
    let bytes = id.as_bytes();
    match bytes.first() {
        Some(first) if bytes.len() <= MAX_ID_BYTES => {
            (first.is_ascii_lowercase() || first.is_ascii_digit())
                && bytes.iter().all(|b| {
                    b.is_ascii_lowercase() || b.is_ascii_digit() || matches!(b, b'-' | b'_' | b'.')
                })
        }
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ids_are_bounded() {
        assert!(valid_id("put_in"));
        assert!(valid_id("go"));
        assert!(!valid_id(""));
        assert!(!valid_id("Go"));
        assert!(!valid_id("-x"));
        assert!(!valid_id(&"a".repeat(MAX_ID_BYTES + 1)));
    }
}
