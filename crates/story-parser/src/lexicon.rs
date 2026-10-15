//! Versioned, data-driven lexicon and its compiled, validated form.

use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};

use crate::text::fold;
use crate::{
    LEXICON_FORMAT_VERSION, MAX_CANDIDATES, MAX_CONTRACTION_EXPANSION, MAX_CONTRACTIONS,
    MAX_DIRECTIONS, MAX_ENCLITICS, MAX_LEXICON_BYTES, MAX_MESSAGE_BYTES, MAX_PATTERNS_PER_VERB,
    MAX_PHRASE_WORDS, MAX_PHRASES, MAX_PREPOSITIONS, MAX_VERBS, MAX_WORD_BYTES,
    MAX_WORDS_PER_ENTRY, valid_id,
};

/// Errors returned while loading or validating a lexicon.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum LexiconError {
    #[error("lexicon document exceeds {limit} bytes")]
    TooLarge { limit: usize },
    #[error("malformed lexicon: {0}")]
    Malformed(String),
    #[error("unsupported lexicon format version {found}")]
    UnsupportedVersion { found: u32 },
    #[error("invalid lexicon: {0}")]
    Invalid(String),
}

/// A language's vocabulary and grammar. Every structure rejects unknown
/// fields. Words are written naturally (accents allowed) and folded when the
/// lexicon is compiled.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Lexicon {
    pub format_version: u32,
    /// Language tag, e.g. `es` or `en`.
    pub language: String,
    /// Action produced by a bare direction (`n`, `norte`).
    pub go_action: String,
    pub verbs: Vec<VerbDecl>,
    #[serde(default)]
    pub prepositions: Vec<PrepositionDecl>,
    #[serde(default)]
    pub directions: Vec<DirectionDecl>,
    /// Articles and determiners dropped inside noun phrases.
    #[serde(default)]
    pub articles: Vec<String>,
    /// Words ignored inside noun phrases (`de` in `llave de hierro`).
    #[serde(default)]
    pub fillers: Vec<String>,
    /// Stand-alone pronouns referring to the last referenced object.
    #[serde(default)]
    pub pronouns: Vec<String>,
    /// Pronoun suffixes attached to verbs (`cógelo`).
    #[serde(default)]
    pub enclitics: Vec<EncliticDecl>,
    /// One word standing for several (`del` → `de el`).
    #[serde(default)]
    pub contractions: Vec<ContractionDecl>,
    /// Several words standing for one (`dentro de` → `en`, `pick up` → `take`).
    #[serde(default)]
    pub phrases: Vec<PhraseDecl>,
    /// Words meaning "everything" (`todo`, `all`).
    #[serde(default)]
    pub all_words: Vec<String>,
    /// Words introducing exclusions (`excepto`, `menos`, `except`).
    #[serde(default)]
    pub except_words: Vec<String>,
    /// List conjunctions that also split commands when a verb follows.
    #[serde(default)]
    pub and_words: Vec<String>,
    /// Word sequences that always split commands (`y luego`, `then`).
    #[serde(default)]
    pub then_phrases: Vec<Vec<String>>,
    /// Ordinal words used to answer ambiguity questions; group *n* selects the
    /// *n*-th candidate.
    #[serde(default)]
    pub ordinals: Vec<Vec<String>>,
    pub messages: Messages,
}

/// One verb: its words (infinitive, imperative, abbreviations) and grammar.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct VerbDecl {
    /// Canonical action id produced unless a pattern overrides it.
    pub action: String,
    /// Word used in questions (`coger` in "¿Qué quieres coger?").
    pub display: String,
    pub words: Vec<String>,
    /// Out-of-world command (save, undo, quit…) that should not take a turn.
    #[serde(default)]
    pub meta: bool,
    /// The verb accepts a direction (`ve al norte`, `go north`).
    #[serde(default)]
    pub directions: bool,
    #[serde(default)]
    pub patterns: Vec<PatternDecl>,
}

/// One grammar line. `{}` is the bare verb. `object` takes a noun phrase;
/// `lead` requires a preposition before it (`mirar en X`); `prep` and `second`
/// add a second noun phrase after a preposition (`poner X en Y`). `multi`
/// allows `todo`/`excepto` in the first noun phrase.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PatternDecl {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub action: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub lead: Option<String>,
    #[serde(default)]
    pub object: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub prep: Option<String>,
    #[serde(default)]
    pub second: bool,
    #[serde(default)]
    pub multi: bool,
}

/// A canonical preposition and its surface words.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PrepositionDecl {
    pub id: String,
    pub display: String,
    pub words: Vec<String>,
}

/// A canonical direction and its words and abbreviations.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DirectionDecl {
    pub id: String,
    pub words: Vec<String>,
}

/// How an enclitic suffix refers to objects.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EncliticKind {
    /// Direct object pronoun (`lo`, `la`, `melo`): the first object is the
    /// last referenced object.
    Direct,
    /// Indirect object pronoun (`le`, `les`): fills the recipient or the
    /// object of a leading preposition when the line leaves it empty.
    Indirect,
    /// Reflexive or first/second person (`me`, `te`, `se`): ignored.
    None,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EncliticDecl {
    pub suffix: String,
    pub kind: EncliticKind,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ContractionDecl {
    pub word: String,
    pub expands: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PhraseDecl {
    pub words: Vec<String>,
    pub replace: String,
}

/// Message templates. Placeholders are `{word}`, `{verb}`, `{prep}`, `{obj}`
/// and `{list}`; they are substituted in a single pass.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Messages {
    pub empty: String,
    pub too_long: String,
    pub too_many_words: String,
    pub too_many_commands: String,
    pub unknown_word: String,
    pub no_verb: String,
    pub not_understood: String,
    pub not_here: String,
    pub no_referent: String,
    pub all_not_allowed: String,
    pub too_vague: String,
    pub not_an_answer: String,
    pub missing_object: String,
    pub missing_lead: String,
    pub missing_second: String,
    pub missing_direction: String,
    pub ambiguous: String,
    pub or_word: String,
}

impl Messages {
    fn all(&self) -> [(&'static str, &str); 18] {
        [
            ("empty", &self.empty),
            ("too_long", &self.too_long),
            ("too_many_words", &self.too_many_words),
            ("too_many_commands", &self.too_many_commands),
            ("unknown_word", &self.unknown_word),
            ("no_verb", &self.no_verb),
            ("not_understood", &self.not_understood),
            ("not_here", &self.not_here),
            ("no_referent", &self.no_referent),
            ("all_not_allowed", &self.all_not_allowed),
            ("too_vague", &self.too_vague),
            ("not_an_answer", &self.not_an_answer),
            ("missing_object", &self.missing_object),
            ("missing_lead", &self.missing_lead),
            ("missing_second", &self.missing_second),
            ("missing_direction", &self.missing_direction),
            ("ambiguous", &self.ambiguous),
            ("or_word", &self.or_word),
        ]
    }
}

impl Lexicon {
    /// Parses a lexicon from JSON bytes, enforcing [`MAX_LEXICON_BYTES`] and
    /// the format version. Validation happens in [`crate::Parser::new`].
    pub fn from_json_slice(bytes: &[u8]) -> Result<Self, LexiconError> {
        if bytes.len() > MAX_LEXICON_BYTES {
            return Err(LexiconError::TooLarge {
                limit: MAX_LEXICON_BYTES,
            });
        }
        let lexicon: Lexicon = serde_json::from_slice(bytes)
            .map_err(|error| LexiconError::Malformed(bounded(error.to_string())))?;
        if lexicon.format_version != LEXICON_FORMAT_VERSION {
            return Err(LexiconError::UnsupportedVersion {
                found: lexicon.format_version,
            });
        }
        Ok(lexicon)
    }
}

fn bounded(mut message: String) -> String {
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

/// Compiled verb.
#[derive(Debug, Clone)]
pub(crate) struct CVerb {
    pub action: String,
    pub display: String,
    pub meta: bool,
    pub directions: bool,
    pub patterns: Vec<CPattern>,
}

/// Compiled pattern with preposition indices.
#[derive(Debug, Clone)]
pub(crate) struct CPattern {
    pub action: Option<String>,
    pub lead: Option<usize>,
    pub object: bool,
    pub prep: Option<usize>,
    pub multi: bool,
}

#[derive(Debug, Clone)]
pub(crate) struct CPrep {
    pub id: String,
    pub display: String,
}

/// A validated lexicon with folded lookup tables.
#[derive(Debug, Clone)]
pub(crate) struct Compiled {
    pub language: String,
    pub go_action: String,
    pub verbs: Vec<CVerb>,
    pub verb_words: BTreeMap<String, usize>,
    pub preps: Vec<CPrep>,
    pub prep_words: BTreeMap<String, usize>,
    pub directions: Vec<String>,
    pub direction_words: BTreeMap<String, usize>,
    pub articles: BTreeSet<String>,
    pub fillers: BTreeSet<String>,
    pub pronouns: BTreeSet<String>,
    /// Shortest suffix first (longest verb stem first).
    pub enclitics: Vec<(String, EncliticKind)>,
    pub contractions: BTreeMap<String, Vec<String>>,
    /// Longest phrase first.
    pub phrases: Vec<(Vec<String>, String)>,
    pub all_words: BTreeSet<String>,
    pub all_display: String,
    pub except_words: BTreeSet<String>,
    pub and_words: BTreeSet<String>,
    /// Longest phrase first.
    pub then_phrases: Vec<Vec<String>>,
    pub ordinals: BTreeMap<String, usize>,
    /// Every word the lexicon knows in any role.
    pub known: BTreeSet<String>,
    pub messages: Messages,
}

fn invalid(message: impl Into<String>) -> LexiconError {
    LexiconError::Invalid(bounded(message.into()))
}

fn count(len: usize, max: usize, what: &str) -> Result<(), LexiconError> {
    if len > max {
        return Err(invalid(format!("{what}: at most {max} entries allowed")));
    }
    Ok(())
}

/// Folds and validates one lexicon word.
fn word(raw: &str, what: &str) -> Result<String, LexiconError> {
    let folded = fold(raw);
    if folded.is_empty()
        || folded.len() > MAX_WORD_BYTES
        || !folded.chars().all(char::is_alphanumeric)
    {
        return Err(invalid(format!(
            "{what}: word '{raw}' must be one alphanumeric word of 1..={MAX_WORD_BYTES} bytes"
        )));
    }
    Ok(folded)
}

fn word_set(
    raw: &[String],
    what: &str,
    known: &mut BTreeSet<String>,
) -> Result<BTreeSet<String>, LexiconError> {
    count(raw.len(), MAX_WORDS_PER_ENTRY, what)?;
    let mut set = BTreeSet::new();
    for entry in raw {
        let folded = word(entry, what)?;
        known.insert(folded.clone());
        set.insert(folded);
    }
    Ok(set)
}

fn id(raw: &str, what: &str) -> Result<String, LexiconError> {
    if !valid_id(raw) {
        return Err(invalid(format!("{what}: id '{raw}' is invalid")));
    }
    Ok(raw.to_owned())
}

fn display(raw: &str, what: &str) -> Result<String, LexiconError> {
    if raw.trim().is_empty() || raw.len() > MAX_WORD_BYTES * 2 || raw.chars().any(char::is_control)
    {
        return Err(invalid(format!("{what}: display text is invalid")));
    }
    Ok(raw.to_owned())
}

fn phrase_words(
    raw: &[String],
    what: &str,
    known: &mut BTreeSet<String>,
) -> Result<Vec<String>, LexiconError> {
    if raw.is_empty() || raw.len() > MAX_PHRASE_WORDS {
        return Err(invalid(format!(
            "{what}: phrases need 1..={MAX_PHRASE_WORDS} words"
        )));
    }
    raw.iter()
        .map(|entry| {
            let folded = word(entry, what)?;
            known.insert(folded.clone());
            Ok(folded)
        })
        .collect()
}

impl Compiled {
    pub fn new(lexicon: &Lexicon) -> Result<Self, LexiconError> {
        if lexicon.format_version != LEXICON_FORMAT_VERSION {
            return Err(LexiconError::UnsupportedVersion {
                found: lexicon.format_version,
            });
        }
        let language = &lexicon.language;
        if !(2..=8).contains(&language.len())
            || !language
                .bytes()
                .all(|b| b.is_ascii_lowercase() || b == b'-')
        {
            return Err(invalid("language tag is invalid"));
        }
        let mut known = BTreeSet::new();

        // Prepositions.
        count(lexicon.prepositions.len(), MAX_PREPOSITIONS, "prepositions")?;
        let mut preps = Vec::new();
        let mut prep_ids = BTreeMap::new();
        let mut prep_words = BTreeMap::new();
        for (index, decl) in lexicon.prepositions.iter().enumerate() {
            let prep_id = id(&decl.id, "preposition")?;
            if prep_ids.insert(prep_id.clone(), index).is_some() {
                return Err(invalid(format!("preposition '{prep_id}' is duplicated")));
            }
            for entry in word_set(&decl.words, "preposition", &mut known)? {
                if prep_words.insert(entry.clone(), index).is_some() {
                    return Err(invalid(format!(
                        "preposition word '{entry}' belongs to two prepositions"
                    )));
                }
            }
            if decl.words.is_empty() {
                return Err(invalid(format!("preposition '{prep_id}' has no words")));
            }
            preps.push(CPrep {
                id: prep_id,
                display: display(&decl.display, "preposition")?,
            });
        }

        // Directions.
        count(lexicon.directions.len(), MAX_DIRECTIONS, "directions")?;
        let mut directions = Vec::new();
        let mut direction_words = BTreeMap::new();
        for (index, decl) in lexicon.directions.iter().enumerate() {
            let direction_id = id(&decl.id, "direction")?;
            if directions.contains(&direction_id) {
                return Err(invalid(format!("direction '{direction_id}' is duplicated")));
            }
            if decl.words.is_empty() {
                return Err(invalid(format!("direction '{direction_id}' has no words")));
            }
            for entry in word_set(&decl.words, "direction", &mut known)? {
                if direction_words.insert(entry.clone(), index).is_some() {
                    return Err(invalid(format!(
                        "direction word '{entry}' belongs to two directions"
                    )));
                }
            }
            directions.push(direction_id);
        }

        // Verbs.
        count(lexicon.verbs.len(), MAX_VERBS, "verbs")?;
        let mut verbs = Vec::new();
        let mut verb_words = BTreeMap::new();
        for (index, decl) in lexicon.verbs.iter().enumerate() {
            let action = id(&decl.action, "verb action")?;
            if decl.words.is_empty() {
                return Err(invalid(format!("verb '{action}' has no words")));
            }
            for entry in word_set(&decl.words, "verb", &mut known)? {
                if direction_words.contains_key(&entry) {
                    return Err(invalid(format!(
                        "word '{entry}' is both a verb and a direction"
                    )));
                }
                if verb_words.insert(entry.clone(), index).is_some() {
                    return Err(invalid(format!("verb word '{entry}' belongs to two verbs")));
                }
            }
            count(decl.patterns.len(), MAX_PATTERNS_PER_VERB, "verb patterns")?;
            if decl.patterns.is_empty() && !decl.directions {
                return Err(invalid(format!(
                    "verb '{action}' needs a pattern or `directions`"
                )));
            }
            let mut patterns = Vec::new();
            for pattern in &decl.patterns {
                let prep_index = |name: &Option<String>| -> Result<Option<usize>, LexiconError> {
                    match name {
                        None => Ok(None),
                        Some(name) => prep_ids.get(name).copied().map(Some).ok_or_else(|| {
                            invalid(format!("verb '{action}': unknown preposition '{name}'"))
                        }),
                    }
                };
                let lead = prep_index(&pattern.lead)?;
                let prep = prep_index(&pattern.prep)?;
                let ok = pattern.second == prep.is_some()
                    && (pattern.object || (lead.is_none() && prep.is_none() && !pattern.multi))
                    && !(lead.is_some() && prep.is_some());
                if !ok {
                    return Err(invalid(format!(
                        "verb '{action}': pattern needs `second` exactly with `prep`, `object` with `lead`/`prep`/`multi`, and not both `lead` and `prep`"
                    )));
                }
                let pattern_action = match &pattern.action {
                    Some(name) => Some(id(name, "pattern action")?),
                    None => None,
                };
                patterns.push(CPattern {
                    action: pattern_action,
                    lead,
                    object: pattern.object,
                    prep,
                    multi: pattern.multi,
                });
            }
            verbs.push(CVerb {
                action,
                display: display(&decl.display, "verb")?,
                meta: decl.meta,
                directions: decl.directions,
                patterns,
            });
        }

        let go_action = id(&lexicon.go_action, "go_action")?;
        if !directions.is_empty()
            && !verbs
                .iter()
                .any(|verb| verb.directions && verb.action == go_action)
        {
            return Err(invalid(
                "go_action must be the action of a verb with `directions`",
            ));
        }

        let articles = word_set(&lexicon.articles, "articles", &mut known)?;
        let fillers = word_set(&lexicon.fillers, "fillers", &mut known)?;
        let pronouns = word_set(&lexicon.pronouns, "pronouns", &mut known)?;
        let all_words = word_set(&lexicon.all_words, "all_words", &mut known)?;
        let all_display = lexicon
            .all_words
            .first()
            .cloned()
            .unwrap_or_else(|| "all".to_owned());
        let except_words = word_set(&lexicon.except_words, "except_words", &mut known)?;
        let and_words = word_set(&lexicon.and_words, "and_words", &mut known)?;
        for entry in &all_words {
            if verb_words.contains_key(entry) || except_words.contains(entry) {
                return Err(invalid(format!("word '{entry}' has conflicting roles")));
            }
        }
        for entry in and_words.iter().chain(&pronouns) {
            if verb_words.contains_key(entry) {
                return Err(invalid(format!(
                    "word '{entry}' is both a verb and a conjunction or pronoun"
                )));
            }
        }

        count(lexicon.enclitics.len(), MAX_ENCLITICS, "enclitics")?;
        let mut enclitics = Vec::new();
        for decl in &lexicon.enclitics {
            let suffix = word(&decl.suffix, "enclitic")?;
            if enclitics.iter().any(|(existing, _)| existing == &suffix) {
                return Err(invalid(format!("enclitic '{suffix}' is duplicated")));
            }
            enclitics.push((suffix, decl.kind));
        }
        // Shortest suffix first, so the longest verb stem wins: `póntela` is
        // `ponte` + `la` (wear it), not `pon` + `tela`.
        enclitics.sort_by(|a, b| a.0.len().cmp(&b.0.len()).then_with(|| a.0.cmp(&b.0)));

        count(lexicon.contractions.len(), MAX_CONTRACTIONS, "contractions")?;
        let mut contractions = BTreeMap::new();
        for decl in &lexicon.contractions {
            let key = word(&decl.word, "contraction")?;
            if !(2..=MAX_CONTRACTION_EXPANSION).contains(&decl.expands.len()) {
                return Err(invalid(format!(
                    "contraction '{key}' must expand into 2..={MAX_CONTRACTION_EXPANSION} words"
                )));
            }
            let expands = decl
                .expands
                .iter()
                .map(|entry| word(entry, "contraction"))
                .collect::<Result<Vec<_>, _>>()?;
            known.insert(key.clone());
            known.extend(expands.iter().cloned());
            if contractions.insert(key.clone(), expands).is_some() {
                return Err(invalid(format!("contraction '{key}' is duplicated")));
            }
        }

        count(lexicon.phrases.len(), MAX_PHRASES, "phrases")?;
        let mut phrases = Vec::new();
        for decl in &lexicon.phrases {
            let words = phrase_words(&decl.words, "phrase", &mut known)?;
            if words.len() < 2 {
                return Err(invalid("phrases need at least two words"));
            }
            let replace = word(&decl.replace, "phrase replacement")?;
            known.insert(replace.clone());
            phrases.push((words, replace));
        }
        phrases.sort_by(|a, b| b.0.len().cmp(&a.0.len()).then_with(|| a.0.cmp(&b.0)));

        count(lexicon.then_phrases.len(), MAX_PHRASES, "then_phrases")?;
        let mut then_phrases = Vec::new();
        for raw in &lexicon.then_phrases {
            then_phrases.push(phrase_words(raw, "then_phrases", &mut known)?);
        }
        then_phrases.sort_by(|a, b| b.len().cmp(&a.len()).then_with(|| a.cmp(b)));

        count(lexicon.ordinals.len(), MAX_CANDIDATES, "ordinals")?;
        let mut ordinals = BTreeMap::new();
        for (index, group) in lexicon.ordinals.iter().enumerate() {
            for entry in word_set(group, "ordinals", &mut known)? {
                if ordinals.insert(entry.clone(), index).is_some() {
                    return Err(invalid(format!("ordinal '{entry}' is duplicated")));
                }
            }
        }

        for (name, template) in lexicon.messages.all() {
            if template.trim().is_empty()
                || template.len() > MAX_MESSAGE_BYTES
                || template.chars().any(char::is_control)
            {
                return Err(invalid(format!(
                    "message '{name}' must be 1..={MAX_MESSAGE_BYTES} bytes without control characters"
                )));
            }
        }

        Ok(Self {
            language: language.clone(),
            go_action,
            verbs,
            verb_words,
            preps,
            prep_words,
            directions,
            direction_words,
            articles,
            fillers,
            pronouns,
            enclitics,
            contractions,
            phrases,
            all_words,
            all_display,
            except_words,
            and_words,
            then_phrases,
            ordinals,
            known,
            messages: lexicon.messages.clone(),
        })
    }

    /// Resolves a word to a verb, stripping one enclitic suffix if needed.
    /// Enclitics attach only to verbs with an object-taking pattern, so a
    /// noun such as `vela` is never read as `ve` + `la`.
    pub fn verb_of(&self, folded: &str) -> Option<(usize, Option<(EncliticKind, usize)>)> {
        if let Some(&verb) = self.verb_words.get(folded) {
            return Some((verb, None));
        }
        for (suffix, kind) in &self.enclitics {
            if folded.len() > suffix.len()
                && let Some(stem) = folded.strip_suffix(suffix.as_str())
                && let Some(&verb) = self.verb_words.get(stem)
                && self
                    .verbs
                    .get(verb)
                    .is_some_and(|verb| verb.patterns.iter().any(|pattern| pattern.object))
            {
                return Some((verb, Some((*kind, stem.len()))));
            }
        }
        None
    }

    /// True when a word can start a command (verb, enclitic verb or direction).
    pub fn starts_command(&self, folded: &str) -> bool {
        self.direction_words.contains_key(folded) || self.verb_of(folded).is_some()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn minimal() -> serde_json::Value {
        serde_json::json!({
            "format_version": 1,
            "language": "es",
            "go_action": "go",
            "verbs": [
                { "action": "go", "display": "ir", "words": ["ir", "ve"], "directions": true },
                { "action": "take", "display": "coger", "words": ["coger", "coge"],
                  "patterns": [{ "object": true, "multi": true }] }
            ],
            "directions": [{ "id": "north", "words": ["norte", "n"] }],
            "messages": {
                "empty": "¿Perdón?", "too_long": "x", "too_many_words": "x",
                "too_many_commands": "x", "unknown_word": "x {word}", "no_verb": "x",
                "not_understood": "x", "not_here": "x", "no_referent": "x",
                "all_not_allowed": "x", "too_vague": "x", "not_an_answer": "x",
                "missing_object": "x", "missing_lead": "x", "missing_second": "x",
                "missing_direction": "x", "ambiguous": "x {list}", "or_word": "o"
            }
        })
    }

    fn compile(value: serde_json::Value) -> Result<Compiled, LexiconError> {
        let bytes = serde_json::to_vec(&value).unwrap();
        Compiled::new(&Lexicon::from_json_slice(&bytes)?)
    }

    #[test]
    fn minimal_lexicon_compiles() {
        let compiled = compile(minimal()).unwrap();
        assert_eq!(compiled.verb_words.get("coge"), Some(&1));
        assert!(compiled.starts_command("n"));
    }

    #[test]
    fn rejects_unknown_fields_and_versions() {
        let mut value = minimal();
        value["extra"] = serde_json::json!(1);
        assert!(matches!(compile(value), Err(LexiconError::Malformed(_))));
        let mut value = minimal();
        value["verbs"][1]["patterns"][0]["objet"] = serde_json::json!(true);
        assert!(matches!(compile(value), Err(LexiconError::Malformed(_))));
        let mut value = minimal();
        value["format_version"] = serde_json::json!(2);
        assert_eq!(
            compile(value).unwrap_err(),
            LexiconError::UnsupportedVersion { found: 2 }
        );
        assert!(matches!(
            Lexicon::from_json_slice(&vec![b' '; MAX_LEXICON_BYTES + 1]),
            Err(LexiconError::TooLarge { .. })
        ));
    }

    #[test]
    fn rejects_conflicts_and_bad_patterns() {
        let mut value = minimal();
        value["verbs"][1]["words"] = serde_json::json!(["ve"]);
        assert!(matches!(compile(value), Err(LexiconError::Invalid(_))));
        let mut value = minimal();
        value["verbs"][1]["words"] = serde_json::json!(["n"]);
        assert!(matches!(compile(value), Err(LexiconError::Invalid(_))));
        let mut value = minimal();
        value["verbs"][1]["patterns"] = serde_json::json!([{ "object": true, "prep": "in" }]);
        assert!(matches!(compile(value), Err(LexiconError::Invalid(_))));
        let mut value = minimal();
        value["verbs"][1]["patterns"] = serde_json::json!([{ "second": true, "object": true }]);
        assert!(matches!(compile(value), Err(LexiconError::Invalid(_))));
        let mut value = minimal();
        value["verbs"][1]["words"] = serde_json::json!(["dos palabras"]);
        assert!(matches!(compile(value), Err(LexiconError::Invalid(_))));
        let mut value = minimal();
        value["verbs"][1]["action"] = serde_json::json!("Take");
        assert!(matches!(compile(value), Err(LexiconError::Invalid(_))));
        let mut value = minimal();
        value["messages"]["empty"] = serde_json::json!("");
        assert!(matches!(compile(value), Err(LexiconError::Invalid(_))));
        let mut value = minimal();
        value["go_action"] = serde_json::json!("take");
        assert!(matches!(compile(value), Err(LexiconError::Invalid(_))));
    }

    #[test]
    fn strips_enclitics_preferring_the_longest_verb_stem() {
        let mut value = minimal();
        value["enclitics"] = serde_json::json!([
            { "suffix": "lo", "kind": "direct" },
            { "suffix": "melo", "kind": "direct" },
            { "suffix": "le", "kind": "indirect" }
        ]);
        value["verbs"][1]["words"] = serde_json::json!(["coge", "da"]);
        let compiled = compile(value).unwrap();
        assert_eq!(
            compiled.verb_of("cogelo"),
            Some((1, Some((EncliticKind::Direct, 4))))
        );
        assert_eq!(
            compiled.verb_of("damelo"),
            Some((1, Some((EncliticKind::Direct, 2))))
        );
        assert_eq!(compiled.verb_of("coge"), Some((1, None)));
        assert_eq!(compiled.verb_of("lo"), None);
        assert_eq!(compiled.verb_of("isla"), None);
    }
}
