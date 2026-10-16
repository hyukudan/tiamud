//! The caller-supplied world scope and its bounded, folded index.

use std::collections::BTreeSet;

use crate::text::fold;
use crate::{
    MAX_ID_BYTES, MAX_SCOPE_NAME_BYTES, MAX_SCOPE_OBJECTS, MAX_SCOPE_WORDS_PER_OBJECT,
    MAX_WORD_BYTES,
};

/// One object the player can currently refer to.
///
/// `id` is opaque to the parser and is returned in [`crate::Command`]s.
/// `name` is the display text used in questions (include the article you
/// want shown, e.g. `la llave dorada`). `nouns` and `adjectives` are single
/// words in the player's language; they are folded like player input, so
/// `lámpara` matches `lampara`. Every noun phrase word must match a noun or
/// adjective of the object; at least one noun match is preferred.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScopeObject {
    pub id: String,
    pub name: String,
    pub nouns: Vec<String>,
    pub adjectives: Vec<String>,
}

impl ScopeObject {
    /// Convenience constructor.
    pub fn new(id: &str, name: &str, nouns: &[&str], adjectives: &[&str]) -> Self {
        Self {
            id: id.to_owned(),
            name: name.to_owned(),
            nouns: nouns.iter().map(|word| (*word).to_owned()).collect(),
            adjectives: adjectives.iter().map(|word| (*word).to_owned()).collect(),
        }
    }
}

/// What the parser may see of the world. Implemented by the caller (for
/// example a story session wrapping `tiamud-story-core` state).
pub trait Scope {
    /// Objects currently in scope, in a deterministic order. At most
    /// [`MAX_SCOPE_OBJECTS`]; a larger or malformed scope is an error, never
    /// silently truncated. Duplicate ids after the first are ignored.
    fn objects(&self) -> &[ScopeObject];

    /// The object a pronoun (`lo`, `it`) refers to, if any.
    fn last_referenced(&self) -> Option<&str> {
        None
    }

    /// Whether a folded word belongs to the game's wider vocabulary (objects
    /// out of sight). Known-but-absent words produce "you can't see that"
    /// instead of "I don't know that word".
    fn knows_word(&self, _folded: &str) -> bool {
        false
    }
}

/// A plain owned scope for tests, tools and simple callers.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SimpleScope {
    pub objects: Vec<ScopeObject>,
    pub last: Option<String>,
    /// Folded extra vocabulary.
    pub vocabulary: BTreeSet<String>,
}

impl SimpleScope {
    pub fn new(objects: Vec<ScopeObject>) -> Self {
        Self {
            objects,
            ..Self::default()
        }
    }

    /// Sets the pronoun referent.
    pub fn with_last(mut self, id: &str) -> Self {
        self.last = Some(id.to_owned());
        self
    }

    /// Adds out-of-sight vocabulary (folded on insertion).
    pub fn with_vocabulary(mut self, words: &[&str]) -> Self {
        self.vocabulary.extend(words.iter().map(|word| fold(word)));
        self
    }
}

impl Scope for SimpleScope {
    fn objects(&self) -> &[ScopeObject] {
        &self.objects
    }

    fn last_referenced(&self) -> Option<&str> {
        self.last.as_deref()
    }

    fn knows_word(&self, folded: &str) -> bool {
        self.vocabulary.contains(folded)
    }
}

#[derive(Debug, Clone)]
pub(crate) struct IndexedObject {
    pub id: String,
    pub name: String,
    pub nouns: BTreeSet<String>,
    pub adjectives: BTreeSet<String>,
}

/// Folded, validated view of a scope for one parse.
#[derive(Debug, Clone, Default)]
pub(crate) struct ScopeIndex {
    pub objects: Vec<IndexedObject>,
    pub words: BTreeSet<String>,
}

fn scope_words(raw: &[String]) -> Option<BTreeSet<String>> {
    if raw.len() > MAX_SCOPE_WORDS_PER_OBJECT {
        return None;
    }
    let mut set = BTreeSet::new();
    for entry in raw {
        if entry.len() > MAX_WORD_BYTES * 4 {
            return None;
        }
        let folded = fold(entry);
        if folded.is_empty()
            || folded.len() > MAX_WORD_BYTES
            || !folded.chars().all(char::is_alphanumeric)
        {
            return None;
        }
        set.insert(folded);
    }
    Some(set)
}

impl ScopeIndex {
    /// Builds the index, or `None` when the scope breaks a documented bound.
    pub fn build(scope: &dyn Scope) -> Option<Self> {
        let objects = scope.objects();
        if objects.len() > MAX_SCOPE_OBJECTS {
            return None;
        }
        let mut index = Self::default();
        let mut seen = BTreeSet::new();
        for object in objects {
            if object.id.is_empty()
                || object.id.len() > MAX_ID_BYTES
                || object.id.chars().any(char::is_control)
                || object.name.len() > MAX_SCOPE_NAME_BYTES
                || object.name.chars().any(char::is_control)
            {
                return None;
            }
            let nouns = scope_words(&object.nouns)?;
            let adjectives = scope_words(&object.adjectives)?;
            if !seen.insert(object.id.as_str()) {
                continue;
            }
            index.words.extend(nouns.iter().cloned());
            index.words.extend(adjectives.iter().cloned());
            index.objects.push(IndexedObject {
                id: object.id.clone(),
                name: if object.name.trim().is_empty() {
                    object.id.clone()
                } else {
                    object.name.clone()
                },
                nouns,
                adjectives,
            });
        }
        Some(index)
    }

    pub fn position(&self, id: &str) -> Option<usize> {
        self.objects.iter().position(|object| object.id == id)
    }

    pub fn name_of(&self, id: &str) -> String {
        self.objects
            .iter()
            .find(|object| object.id == id)
            .map_or_else(|| id.to_owned(), |object| object.name.clone())
    }
}
