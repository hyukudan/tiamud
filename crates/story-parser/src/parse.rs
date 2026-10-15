//! Segmentation, grammar matching, noun-phrase resolution and follow-up
//! answers.

use std::fmt;

use serde::{Deserialize, Serialize};

use crate::lexicon::{CPattern, CVerb, Compiled, EncliticKind, Lexicon, LexiconError};
use crate::scope::{Scope, ScopeIndex};
use crate::text::{RawTok, Tok, TokKind, TokenizeError, capitalize_first, tokenize};
use crate::{MAX_CANDIDATES, MAX_COMMANDS, MAX_EXCEPT, MAX_INPUT_BYTES};

const SPANISH: &str = include_str!("../lexicons/es.json");
const ENGLISH: &str = include_str!("../lexicons/en.json");

macro_rules! id_type {
    ($name:ident, $doc:literal) => {
        #[doc = $doc]
        #[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
        #[serde(transparent)]
        pub struct $name(pub String);

        impl $name {
            pub fn as_str(&self) -> &str {
                &self.0
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str(&self.0)
            }
        }

        impl From<&str> for $name {
            fn from(value: &str) -> Self {
                Self(value.to_owned())
            }
        }
    };
}

id_type!(ActionId, "Canonical action id declared by the lexicon.");
id_type!(ObjRef, "Object id supplied by the caller's [`Scope`].");
id_type!(PrepId, "Canonical preposition id declared by the lexicon.");
id_type!(
    DirectionId,
    "Canonical direction id declared by the lexicon."
);

/// One fully resolved command.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Command {
    pub action: ActionId,
    /// Out-of-world verb (save, undo, quit…) as declared by the lexicon.
    pub meta: bool,
    pub obj: Option<ObjRef>,
    pub prep: Option<PrepId>,
    pub obj2: Option<ObjRef>,
    /// Set for movement (`n`, `ve al norte`, `go north`).
    pub direction: Option<DirectionId>,
    /// `todo`/`all` was used; the caller decides what "all" covers.
    pub all: bool,
    /// Objects excluded with `excepto`/`except` (only with `all`).
    pub except: Vec<ObjRef>,
}

/// One object offered by an ambiguity question.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Candidate {
    pub id: ObjRef,
    pub name: String,
}

/// Why a line or answer was not understood.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum ParseErrorKind {
    Empty,
    InputTooLong,
    InvalidUtf8,
    TooManyWords,
    TooManyCommands,
    /// A word no lexicon role, scope object or scope vocabulary knows.
    UnknownWord {
        word: String,
    },
    /// The command does not start with a verb or direction.
    NoVerb,
    /// Words are known but match no grammar pattern of the verb.
    NotUnderstood,
    /// The words name no object in scope.
    NotHere,
    /// A pronoun was used with nothing to refer to.
    NoReferent {
        word: String,
    },
    /// `todo`/`all` with a verb or slot that does not accept it.
    AllNotAllowed,
    /// More than [`MAX_CANDIDATES`] objects match.
    TooVague,
    /// An answer selects none of the offered candidates.
    NotAnAnswer,
    /// The caller's scope breaks a documented bound.
    InvalidScope,
    /// The pending question belongs to another lexicon.
    StalePending,
}

/// A parse failure with its rendered, localized message.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ParseError {
    pub kind: ParseErrorKind,
    pub message: String,
}

/// The result of parsing one command (or answering one question).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ParseOutcome {
    Command(Command),
    /// Several objects match; ask `question` and pass the reply to
    /// [`Parser::answer`] with `pending`.
    Ambiguous {
        candidates: Vec<Candidate>,
        question: String,
        pending: Pending,
    },
    /// A required object or direction is missing; ask `question` and pass the
    /// reply to [`Parser::answer`].
    Incomplete {
        question: String,
        pending: Pending,
    },
    Error(ParseError),
}

impl ParseOutcome {
    /// The command, if this outcome is one.
    pub fn command(&self) -> Option<&Command> {
        match self {
            ParseOutcome::Command(command) => Some(command),
            _ => None,
        }
    }
}

/// A normalized command of a line, ready to parse against a scope.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Segment {
    tokens: Vec<Tok>,
}

impl Segment {
    /// The folded words of this command; list conjunctions appear as `,`.
    pub fn words(&self) -> Vec<&str> {
        self.tokens
            .iter()
            .map(|token| match token.kind {
                TokKind::Conj => ",",
                _ => token.folded.as_str(),
            })
            .collect()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum Slot {
    Unused,
    Missing,
    Words(Vec<Tok>),
    Done(NpValue),
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum NpValue {
    One(String),
    All(Vec<String>),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Which {
    First,
    Second,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum Awaiting {
    Fill(Which),
    Choose(Which, Vec<String>),
    Direction,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct State {
    verb: usize,
    pattern: Option<usize>,
    first: Slot,
    second: Slot,
}

impl State {
    fn slot(&self, which: Which) -> &Slot {
        match which {
            Which::First => &self.first,
            Which::Second => &self.second,
        }
    }

    fn slot_mut(&mut self, which: Which) -> &mut Slot {
        match which {
            Which::First => &mut self.first,
            Which::Second => &mut self.second,
        }
    }
}

/// A question awaiting the player's reply. Valid only for the parser (same
/// lexicon) that produced it; bounded by the line that created it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Pending {
    language: String,
    action: String,
    state: State,
    awaiting: Awaiting,
}

impl Pending {
    /// The candidate ids of an ambiguity question (empty otherwise).
    pub fn candidates(&self) -> Vec<ObjRef> {
        match &self.awaiting {
            Awaiting::Choose(_, ids) => ids.iter().map(|id| ObjRef(id.clone())).collect(),
            _ => Vec::new(),
        }
    }
}

enum Np {
    Value(NpValue),
    Empty,
    Ambiguous(Vec<usize>),
    Fail(ParseErrorKind),
}

/// A compiled, validated lexicon ready to parse lines. Immutable and cheap to
/// share by reference.
#[derive(Debug, Clone)]
pub struct Parser {
    lex: Compiled,
}

impl Parser {
    /// Validates and compiles a lexicon.
    pub fn new(lexicon: &Lexicon) -> Result<Self, LexiconError> {
        Ok(Self {
            lex: Compiled::new(lexicon)?,
        })
    }

    /// Loads, validates and compiles a lexicon from JSON bytes.
    pub fn from_json_slice(bytes: &[u8]) -> Result<Self, LexiconError> {
        Self::new(&Lexicon::from_json_slice(bytes)?)
    }

    /// The reviewed Spanish lexicon shipped with the crate.
    pub fn spanish() -> Result<Self, LexiconError> {
        Self::from_json_slice(SPANISH.as_bytes())
    }

    /// The reviewed English lexicon shipped with the crate.
    pub fn english() -> Result<Self, LexiconError> {
        Self::from_json_slice(ENGLISH.as_bytes())
    }

    /// The JSON of a shipped lexicon by language tag (`es`, `en`).
    pub fn builtin_lexicon_json(language: &str) -> Option<&'static str> {
        match language {
            "es" => Some(SPANISH),
            "en" => Some(ENGLISH),
            _ => None,
        }
    }

    pub fn language(&self) -> &str {
        &self.lex.language
    }

    /// Normalizes a line and splits it into at most [`MAX_COMMANDS`] commands.
    pub fn split_line(&self, input: &str) -> Result<Vec<Segment>, ParseError> {
        let raw = tokenize(input).map_err(|error| {
            self.error(match error {
                TokenizeError::TooLong => ParseErrorKind::InputTooLong,
                TokenizeError::TooManyWords => ParseErrorKind::TooManyWords,
            })
        })?;

        let mut expanded = Vec::with_capacity(raw.len());
        for token in raw {
            match token {
                RawTok::Word(word) => match self.lex.contractions.get(&word.folded) {
                    Some(parts) => expanded.extend(
                        parts
                            .iter()
                            .map(|part| RawTok::Word(Tok::word(part.clone(), part.clone()))),
                    ),
                    None => expanded.push(RawTok::Word(word)),
                },
                other => expanded.push(other),
            }
        }

        let mut merged = Vec::with_capacity(expanded.len());
        let mut at = 0;
        while let Some(token) = expanded.get(at) {
            if matches!(token, RawTok::Word(_)) {
                if let Some(phrase) = self
                    .lex
                    .then_phrases
                    .iter()
                    .find(|phrase| words_at(&expanded, at, phrase))
                {
                    merged.push(RawTok::Stop);
                    at += phrase.len();
                    continue;
                }
                if let Some((phrase, replace)) = self
                    .lex
                    .phrases
                    .iter()
                    .find(|(phrase, _)| words_at(&expanded, at, phrase))
                {
                    let surface = expanded[at..at + phrase.len()]
                        .iter()
                        .filter_map(|token| match token {
                            RawTok::Word(word) => Some(word.surface.as_str()),
                            _ => None,
                        })
                        .collect::<Vec<_>>()
                        .join(" ");
                    merged.push(RawTok::Word(Tok::word(surface, replace.clone())));
                    at += phrase.len();
                    continue;
                }
            }
            merged.push(token.clone());
            at += 1;
        }

        let mut segments = Vec::new();
        let mut current = Vec::new();
        for (position, token) in merged.iter().enumerate() {
            let soft = match token {
                RawTok::Stop => {
                    close_segment(&mut current, &mut segments);
                    continue;
                }
                RawTok::Comma => true,
                RawTok::Word(word) => self.lex.and_words.contains(&word.folded),
            };
            if soft {
                let next_starts = matches!(
                    merged.get(position + 1),
                    Some(RawTok::Word(next)) if self.lex.starts_command(&next.folded)
                );
                if next_starts {
                    close_segment(&mut current, &mut segments);
                } else {
                    current.push(Tok {
                        surface: ",".to_owned(),
                        folded: ",".to_owned(),
                        kind: TokKind::Conj,
                    });
                }
            } else if let RawTok::Word(word) = token {
                current.push(word.clone());
            }
            if segments.len() > MAX_COMMANDS {
                return Err(self.error(ParseErrorKind::TooManyCommands));
            }
        }
        close_segment(&mut current, &mut segments);
        if segments.len() > MAX_COMMANDS {
            return Err(self.error(ParseErrorKind::TooManyCommands));
        }
        if segments.is_empty() {
            return Err(self.error(ParseErrorKind::Empty));
        }
        Ok(segments)
    }

    /// Parses a whole line against one scope. Commands are parsed in order
    /// and parsing stops at the first outcome that is not a command (the rest
    /// of the line is discarded, as in classic parsers). Pronouns in later
    /// commands refer to the first object of the previous command.
    ///
    /// The scope does not change between commands of one line; callers whose
    /// commands change visibility should use [`Parser::split_line`] and
    /// [`Parser::parse_segment`] and execute each command before parsing the
    /// next.
    pub fn parse_line(&self, input: &str, scope: &dyn Scope) -> Vec<ParseOutcome> {
        let segments = match self.split_line(input) {
            Ok(segments) => segments,
            Err(error) => return vec![ParseOutcome::Error(error)],
        };
        let Some(index) = ScopeIndex::build(scope) else {
            return vec![self.fail(ParseErrorKind::InvalidScope)];
        };
        let mut last = scope.last_referenced().map(str::to_owned);
        let mut outcomes = Vec::with_capacity(segments.len());
        for segment in &segments {
            let outcome = self.parse_tokens(&segment.tokens, &index, scope, last.as_deref());
            let command = outcome.command().cloned();
            outcomes.push(outcome);
            match command {
                Some(command) => {
                    if let Some(obj) = command.obj {
                        last = Some(obj.0);
                    }
                }
                None => break,
            }
        }
        outcomes
    }

    /// Like [`Parser::parse_line`] for raw bytes: rejects lines over
    /// [`MAX_INPUT_BYTES`] and invalid UTF-8.
    pub fn parse_line_bytes(&self, input: &[u8], scope: &dyn Scope) -> Vec<ParseOutcome> {
        if input.len() > MAX_INPUT_BYTES {
            return vec![self.fail(ParseErrorKind::InputTooLong)];
        }
        match std::str::from_utf8(input) {
            Ok(text) => self.parse_line(text, scope),
            Err(_) => vec![self.fail(ParseErrorKind::InvalidUtf8)],
        }
    }

    /// Parses one command produced by [`Parser::split_line`]. Pronouns refer
    /// to [`Scope::last_referenced`].
    pub fn parse_segment(&self, segment: &Segment, scope: &dyn Scope) -> ParseOutcome {
        let Some(index) = ScopeIndex::build(scope) else {
            return self.fail(ParseErrorKind::InvalidScope);
        };
        self.parse_tokens(&segment.tokens, &index, scope, scope.last_referenced())
    }

    /// Continues an [`ParseOutcome::Ambiguous`] or [`ParseOutcome::Incomplete`]
    /// question with the player's reply (`la dorada`, `the golden one`, `2`,
    /// `segunda`, `la llave`, `norte`). A reply that is itself a command, or a
    /// line of several commands, is parsed as a new line instead.
    pub fn answer(&self, pending: &Pending, input: &str, scope: &dyn Scope) -> Vec<ParseOutcome> {
        if pending.language != self.lex.language {
            return vec![self.fail(ParseErrorKind::StalePending)];
        }
        let Some(verb) = self.lex.verbs.get(pending.state.verb) else {
            return vec![self.fail(ParseErrorKind::StalePending)];
        };
        if verb.action != pending.action
            || pending
                .state
                .pattern
                .is_some_and(|pattern| verb.patterns.get(pattern).is_none())
        {
            return vec![self.fail(ParseErrorKind::StalePending)];
        }
        let segments = match self.split_line(input) {
            Ok(segments) => segments,
            Err(error) => return vec![ParseOutcome::Error(error)],
        };
        let [segment] = segments.as_slice() else {
            return self.parse_line(input, scope);
        };
        let tokens = &segment.tokens;
        let Some(index) = ScopeIndex::build(scope) else {
            return vec![self.fail(ParseErrorKind::InvalidScope)];
        };
        let last = scope.last_referenced();
        let starts_command = tokens
            .first()
            .is_some_and(|token| token.is_word() && self.lex.starts_command(&token.folded));

        match &pending.awaiting {
            Awaiting::Direction => {
                if let Some(direction) = self.direction_in(tokens) {
                    return vec![ParseOutcome::Command(self.go(verb, direction))];
                }
                self.parse_line(input, scope)
            }
            Awaiting::Fill(which) => {
                if starts_command {
                    return self.parse_line(input, scope);
                }
                if let Some(word) = self.first_unknown(tokens, &index, scope) {
                    return vec![self.fail(ParseErrorKind::UnknownWord { word })];
                }
                let mut words = tokens.as_slice();
                if words.len() > 1 && self.lex.prep_words.contains_key(&words[0].folded) {
                    words = &words[1..];
                }
                let mut state = pending.state.clone();
                *state.slot_mut(*which) = Slot::Words(words.to_vec());
                vec![self.advance(state, &index, last)]
            }
            Awaiting::Choose(which, ids) => {
                let choose = |id: &str| {
                    let mut state = pending.state.clone();
                    *state.slot_mut(*which) = Slot::Done(NpValue::One(id.to_owned()));
                    vec![self.advance(state, &index, last)]
                };
                if let [only] = tokens.as_slice()
                    && only.is_word()
                    && (1..=2).contains(&only.folded.len())
                    && only.folded.bytes().all(|b| b.is_ascii_digit())
                    && let Ok(number) = only.folded.parse::<usize>()
                    && let Some(id) = number.checked_sub(1).and_then(|n| ids.get(n))
                {
                    return choose(id);
                }
                let content: Vec<&Tok> = tokens
                    .iter()
                    .filter(|token| {
                        token.is_word()
                            && !self.lex.articles.contains(&token.folded)
                            && !self.lex.fillers.contains(&token.folded)
                    })
                    .collect();
                if let [only] = content.as_slice()
                    && let Some(&ordinal) = self.lex.ordinals.get(&only.folded)
                    && let Some(id) = ids.get(ordinal)
                {
                    return choose(id);
                }
                if !content.is_empty() && tokens.iter().all(Tok::is_word) {
                    let subset: Vec<usize> =
                        ids.iter().filter_map(|id| index.position(id)).collect();
                    let words: Vec<&str> =
                        content.iter().map(|token| token.folded.as_str()).collect();
                    let matches = match_words(&words, &index, &subset);
                    match matches.as_slice() {
                        [single] => {
                            if let Some(object) = index.objects.get(*single) {
                                return choose(&object.id);
                            }
                        }
                        [] => {}
                        many => {
                            return vec![self.ask_choice(
                                pending.state.clone(),
                                *which,
                                many.to_vec(),
                                &index,
                            )];
                        }
                    }
                }
                if starts_command {
                    return self.parse_line(input, scope);
                }
                if let Some(word) = self.first_unknown(tokens, &index, scope) {
                    return vec![self.fail(ParseErrorKind::UnknownWord { word })];
                }
                vec![self.fail(ParseErrorKind::NotAnAnswer)]
            }
        }
    }

    fn parse_tokens(
        &self,
        tokens: &[Tok],
        index: &ScopeIndex,
        scope: &dyn Scope,
        last: Option<&str>,
    ) -> ParseOutcome {
        if let Some(word) = self.first_unknown(tokens, index, scope) {
            return self.fail(ParseErrorKind::UnknownWord { word });
        }
        let Some(first) = tokens.first() else {
            return self.fail(ParseErrorKind::Empty);
        };
        if !first.is_word() {
            return self.fail(ParseErrorKind::NotUnderstood);
        }
        if let Some(&direction) = self.lex.direction_words.get(&first.folded) {
            if tokens.len() == 1
                && let Some(direction) = self.lex.directions.get(direction)
            {
                return ParseOutcome::Command(Command {
                    action: ActionId(self.lex.go_action.clone()),
                    meta: false,
                    obj: None,
                    prep: None,
                    obj2: None,
                    direction: Some(DirectionId(direction.clone())),
                    all: false,
                    except: Vec::new(),
                });
            }
            return self.fail(ParseErrorKind::NotUnderstood);
        }
        let Some((verb_index, enclitic)) = self.lex.verb_of(&first.folded) else {
            return self.fail(ParseErrorKind::NoVerb);
        };
        let Some(verb) = self.lex.verbs.get(verb_index) else {
            return self.fail(ParseErrorKind::NotUnderstood);
        };

        let mut rest: Vec<Tok> = tokens.get(1..).unwrap_or_default().to_vec();
        let mut indirect = None;
        if let Some((kind, stem)) = enclitic {
            let pronoun = Tok {
                surface: first.folded.get(stem..).unwrap_or_default().to_owned(),
                folded: first.folded.get(stem..).unwrap_or_default().to_owned(),
                kind: TokKind::Pronoun,
            };
            match kind {
                EncliticKind::Direct => rest.insert(0, pronoun),
                EncliticKind::Indirect => indirect = Some(pronoun),
                EncliticKind::None => {}
            }
        }

        if verb.directions {
            if let Some(direction) = self.direction_in(&rest) {
                return ParseOutcome::Command(self.go(verb, direction));
            }
            if rest.is_empty() && !verb.patterns.iter().any(|pattern| !pattern.object) {
                return self.ask_direction(verb_index, verb);
            }
        }

        match self.select_pattern(verb, &rest, indirect, index) {
            Some((pattern, first, second)) => self.advance(
                State {
                    verb: verb_index,
                    pattern: Some(pattern),
                    first,
                    second,
                },
                index,
                last,
            ),
            None => self.fail(ParseErrorKind::NotUnderstood),
        }
    }

    /// A single direction word, possibly after prepositions and articles
    /// (`al norte`, `to the north`).
    fn direction_in(&self, tokens: &[Tok]) -> Option<usize> {
        let mut rest = tokens;
        while rest.len() > 1
            && rest[0].is_word()
            && (self.lex.prep_words.contains_key(&rest[0].folded)
                || self.lex.articles.contains(&rest[0].folded))
        {
            rest = &rest[1..];
        }
        match rest {
            [only] if only.is_word() => self.lex.direction_words.get(&only.folded).copied(),
            _ => None,
        }
    }

    fn go(&self, verb: &CVerb, direction: usize) -> Command {
        Command {
            action: ActionId(verb.action.clone()),
            meta: verb.meta,
            obj: None,
            prep: None,
            obj2: None,
            direction: self
                .lex
                .directions
                .get(direction)
                .map(|id| DirectionId(id.clone())),
            all: false,
            except: Vec::new(),
        }
    }

    fn is_prep(&self, token: &Tok, prep: usize) -> bool {
        token.is_word() && self.lex.prep_words.get(&token.folded) == Some(&prep)
    }

    /// A preposition that is not an article, filler or scope word.
    fn has_stray_prep(&self, tokens: &[Tok], index: &ScopeIndex) -> bool {
        tokens.iter().any(|token| {
            token.is_word()
                && self.lex.prep_words.contains_key(&token.folded)
                && !self.lex.articles.contains(&token.folded)
                && !self.lex.fillers.contains(&token.folded)
                && !index.words.contains(&token.folded)
        })
    }

    /// Picks the first pattern that fully matches, otherwise the one missing
    /// the fewest slots (earliest on ties).
    fn select_pattern(
        &self,
        verb: &CVerb,
        rest: &[Tok],
        indirect: Option<Tok>,
        index: &ScopeIndex,
    ) -> Option<(usize, Slot, Slot)> {
        let stray = self.has_stray_prep(rest, index);
        let words = |tokens: &[Tok]| {
            if tokens.is_empty() {
                Slot::Missing
            } else {
                Slot::Words(tokens.to_vec())
            }
        };
        let mut best: Option<(u8, usize, Slot, Slot)> = None;
        for (position, pattern) in verb.patterns.iter().enumerate() {
            let candidate = if !pattern.object {
                rest.is_empty().then_some((0, Slot::Unused, Slot::Unused))
            } else if let Some(lead) = pattern.lead {
                if rest.first().is_some_and(|token| self.is_prep(token, lead)) {
                    let np = &rest[1..];
                    Some((u8::from(np.is_empty()), words(np), Slot::Unused))
                } else if rest.is_empty() {
                    match &indirect {
                        Some(pronoun) => {
                            Some((0, Slot::Words(vec![pronoun.clone()]), Slot::Unused))
                        }
                        None => Some((1, Slot::Missing, Slot::Unused)),
                    }
                } else {
                    None
                }
            } else if let Some(prep) = pattern.prep {
                if let Some(split) = rest.iter().position(|token| self.is_prep(token, prep)) {
                    let (np1, np2) = (&rest[..split], &rest[split + 1..]);
                    Some((
                        u8::from(np1.is_empty()) + u8::from(np2.is_empty()),
                        words(np1),
                        words(np2),
                    ))
                } else if rest.is_empty() {
                    Some((2, Slot::Missing, Slot::Missing))
                } else if stray {
                    None
                } else {
                    match &indirect {
                        Some(pronoun) => Some((
                            0,
                            Slot::Words(rest.to_vec()),
                            Slot::Words(vec![pronoun.clone()]),
                        )),
                        None => Some((1, Slot::Words(rest.to_vec()), Slot::Missing)),
                    }
                }
            } else if rest.is_empty() {
                Some((1, Slot::Missing, Slot::Unused))
            } else if stray {
                None
            } else {
                Some((0, Slot::Words(rest.to_vec()), Slot::Unused))
            };
            if let Some((missing, first, second)) = candidate {
                if missing == 0 {
                    return Some((position, first, second));
                }
                if best.as_ref().is_none_or(|(least, ..)| missing < *least) {
                    best = Some((missing, position, first, second));
                }
            }
        }
        best.map(|(_, position, first, second)| (position, first, second))
    }

    fn pattern_of(&self, state: &State) -> Option<(&CVerb, &CPattern)> {
        let verb = self.lex.verbs.get(state.verb)?;
        let pattern = verb.patterns.get(state.pattern?)?;
        Some((verb, pattern))
    }

    /// Resolves noun phrases, then asks for missing slots, then builds.
    fn advance(&self, mut state: State, index: &ScopeIndex, last: Option<&str>) -> ParseOutcome {
        let Some((_, pattern)) = self.pattern_of(&state) else {
            return self.fail(ParseErrorKind::StalePending);
        };
        let multi = pattern.multi;
        for which in [Which::First, Which::Second] {
            if let Slot::Words(words) = state.slot(which).clone() {
                let multi = multi && which == Which::First;
                match self.resolve_np(&words, multi, index, last) {
                    Np::Value(value) => *state.slot_mut(which) = Slot::Done(value),
                    Np::Empty => *state.slot_mut(which) = Slot::Missing,
                    Np::Ambiguous(candidates) => {
                        return self.ask_choice(state, which, candidates, index);
                    }
                    Np::Fail(kind) => return self.fail(kind),
                }
            }
        }
        for which in [Which::First, Which::Second] {
            if *state.slot(which) == Slot::Missing {
                return self.ask_fill(state, which, index);
            }
        }
        self.build(&state)
    }

    fn build(&self, state: &State) -> ParseOutcome {
        let Some((verb, pattern)) = self.pattern_of(state) else {
            return self.fail(ParseErrorKind::StalePending);
        };
        let (obj, all, except) = match &state.first {
            Slot::Done(NpValue::One(id)) => (Some(ObjRef(id.clone())), false, Vec::new()),
            Slot::Done(NpValue::All(except)) => (
                None,
                true,
                except.iter().map(|id| ObjRef(id.clone())).collect(),
            ),
            _ => (None, false, Vec::new()),
        };
        let obj2 = match &state.second {
            Slot::Done(NpValue::One(id)) => Some(ObjRef(id.clone())),
            Slot::Done(NpValue::All(_)) => return self.fail(ParseErrorKind::AllNotAllowed),
            _ => None,
        };
        let prep = pattern
            .prep
            .or(pattern.lead)
            .and_then(|prep| self.lex.preps.get(prep))
            .map(|prep| PrepId(prep.id.clone()));
        ParseOutcome::Command(Command {
            action: ActionId(
                pattern
                    .action
                    .clone()
                    .unwrap_or_else(|| verb.action.clone()),
            ),
            meta: verb.meta,
            obj,
            prep,
            obj2,
            direction: None,
            all,
            except,
        })
    }

    fn resolve_np(
        &self,
        tokens: &[Tok],
        multi: bool,
        index: &ScopeIndex,
        last: Option<&str>,
    ) -> Np {
        let mut tokens = tokens;
        while let Some((head, tail)) = tokens.split_first()
            && head.kind == TokKind::Conj
        {
            tokens = tail;
        }
        while tokens
            .last()
            .is_some_and(|token| token.kind == TokKind::Conj)
        {
            tokens = &tokens[..tokens.len() - 1];
        }
        if tokens.is_empty() {
            return Np::Empty;
        }
        if let [only] = tokens
            && (only.kind == TokKind::Pronoun
                || (only.is_word() && self.lex.pronouns.contains(&only.folded)))
        {
            return match last {
                Some(id) if index.position(id).is_some() => Np::Value(NpValue::One(id.to_owned())),
                Some(_) => Np::Fail(ParseErrorKind::NotHere),
                None => Np::Fail(ParseErrorKind::NoReferent {
                    word: only.surface.clone(),
                }),
            };
        }
        let start = tokens
            .iter()
            .position(|token| !(token.is_word() && self.lex.articles.contains(&token.folded)))
            .unwrap_or(tokens.len());
        let body = &tokens[start..];
        let Some(head) = body.first() else {
            return Np::Empty;
        };
        if head.is_word() && self.lex.all_words.contains(&head.folded) {
            if !multi {
                return Np::Fail(ParseErrorKind::AllNotAllowed);
            }
            let tail = &body[1..];
            let Some(marker) = tail.first() else {
                return Np::Value(NpValue::All(Vec::new()));
            };
            if !(marker.is_word() && self.lex.except_words.contains(&marker.folded)) {
                return Np::Fail(ParseErrorKind::NotUnderstood);
            }
            let mut except: Vec<String> = Vec::new();
            for part in tail[1..].split(|token| token.kind == TokKind::Conj) {
                let ids: Vec<String> = match self.resolve_np(part, false, index, last) {
                    Np::Value(NpValue::One(id)) => vec![id],
                    Np::Value(NpValue::All(_)) => {
                        return Np::Fail(ParseErrorKind::NotUnderstood);
                    }
                    Np::Ambiguous(candidates) => candidates
                        .iter()
                        .filter_map(|&candidate| index.objects.get(candidate))
                        .map(|object| object.id.clone())
                        .collect(),
                    Np::Empty => continue,
                    Np::Fail(kind) => return Np::Fail(kind),
                };
                for id in ids {
                    if !except.contains(&id) {
                        except.push(id);
                    }
                }
                if except.len() > MAX_EXCEPT {
                    return Np::Fail(ParseErrorKind::TooVague);
                }
            }
            if except.is_empty() {
                return Np::Fail(ParseErrorKind::NotUnderstood);
            }
            return Np::Value(NpValue::All(except));
        }
        if body.iter().any(|token| !token.is_word()) {
            return Np::Fail(ParseErrorKind::NotUnderstood);
        }
        let content: Vec<&str> = body
            .iter()
            .filter(|token| {
                !self.lex.articles.contains(&token.folded)
                    && !self.lex.fillers.contains(&token.folded)
            })
            .map(|token| token.folded.as_str())
            .collect();
        if content.is_empty() {
            return Np::Empty;
        }
        let everything: Vec<usize> = (0..index.objects.len()).collect();
        let matches = match_words(&content, index, &everything);
        match matches.len() {
            0 => Np::Fail(ParseErrorKind::NotHere),
            1 => match index.objects.get(matches[0]) {
                Some(object) => Np::Value(NpValue::One(object.id.clone())),
                None => Np::Fail(ParseErrorKind::NotHere),
            },
            n if n > MAX_CANDIDATES => Np::Fail(ParseErrorKind::TooVague),
            _ => Np::Ambiguous(matches),
        }
    }

    fn first_unknown(
        &self,
        tokens: &[Tok],
        index: &ScopeIndex,
        scope: &dyn Scope,
    ) -> Option<String> {
        tokens.iter().enumerate().find_map(|(position, token)| {
            if !token.is_word()
                || (position == 0 && self.lex.verb_of(&token.folded).is_some())
                || self.lex.known.contains(&token.folded)
                || index.words.contains(&token.folded)
                || token.folded.chars().all(|c| c.is_ascii_digit())
                || scope.knows_word(&token.folded)
            {
                None
            } else {
                Some(token.surface.clone())
            }
        })
    }

    fn ask_choice(
        &self,
        state: State,
        which: Which,
        candidates: Vec<usize>,
        index: &ScopeIndex,
    ) -> ParseOutcome {
        let candidates: Vec<Candidate> = candidates
            .iter()
            .take(MAX_CANDIDATES)
            .filter_map(|&candidate| index.objects.get(candidate))
            .map(|object| Candidate {
                id: ObjRef(object.id.clone()),
                name: object.name.clone(),
            })
            .collect();
        let names: Vec<&str> = candidates.iter().map(|c| c.name.as_str()).collect();
        let list = join_list(&names, &self.lex.messages.or_word);
        let question = render(&self.lex.messages.ambiguous, &[("list", &list)]);
        let action = self
            .lex
            .verbs
            .get(state.verb)
            .map(|verb| verb.action.clone())
            .unwrap_or_default();
        ParseOutcome::Ambiguous {
            pending: Pending {
                language: self.lex.language.clone(),
                action,
                awaiting: Awaiting::Choose(
                    which,
                    candidates.iter().map(|c| c.id.0.clone()).collect(),
                ),
                state,
            },
            candidates,
            question,
        }
    }

    fn ask_fill(&self, state: State, which: Which, index: &ScopeIndex) -> ParseOutcome {
        let Some((verb, pattern)) = self.pattern_of(&state) else {
            return self.fail(ParseErrorKind::StalePending);
        };
        let prep_display = |prep: Option<usize>| {
            prep.and_then(|prep| self.lex.preps.get(prep))
                .map(|prep| prep.display.clone())
                .unwrap_or_default()
        };
        let question = match which {
            Which::First => match pattern.lead {
                Some(lead) => render(
                    &self.lex.messages.missing_lead,
                    &[("verb", &verb.display), ("prep", &prep_display(Some(lead)))],
                ),
                None => render(
                    &self.lex.messages.missing_object,
                    &[("verb", &verb.display)],
                ),
            },
            Which::Second => {
                let obj = match &state.first {
                    Slot::Done(NpValue::One(id)) => index.name_of(id),
                    Slot::Done(NpValue::All(_)) => self.lex.all_display.clone(),
                    _ => String::new(),
                };
                render(
                    &self.lex.messages.missing_second,
                    &[
                        ("verb", &verb.display),
                        ("prep", &prep_display(pattern.prep)),
                        ("obj", &obj),
                    ],
                )
            }
        };
        ParseOutcome::Incomplete {
            question,
            pending: Pending {
                language: self.lex.language.clone(),
                action: verb.action.clone(),
                state,
                awaiting: Awaiting::Fill(which),
            },
        }
    }

    fn ask_direction(&self, verb_index: usize, verb: &CVerb) -> ParseOutcome {
        ParseOutcome::Incomplete {
            question: render(
                &self.lex.messages.missing_direction,
                &[("verb", &verb.display)],
            ),
            pending: Pending {
                language: self.lex.language.clone(),
                action: verb.action.clone(),
                state: State {
                    verb: verb_index,
                    pattern: None,
                    first: Slot::Unused,
                    second: Slot::Unused,
                },
                awaiting: Awaiting::Direction,
            },
        }
    }

    fn error(&self, kind: ParseErrorKind) -> ParseError {
        let messages = &self.lex.messages;
        let message = match &kind {
            ParseErrorKind::Empty => render(&messages.empty, &[]),
            ParseErrorKind::InputTooLong => render(&messages.too_long, &[]),
            ParseErrorKind::TooManyWords => render(&messages.too_many_words, &[]),
            ParseErrorKind::TooManyCommands => render(&messages.too_many_commands, &[]),
            ParseErrorKind::UnknownWord { word } => {
                render(&messages.unknown_word, &[("word", word)])
            }
            ParseErrorKind::NoVerb => render(&messages.no_verb, &[]),
            ParseErrorKind::NotHere => render(&messages.not_here, &[]),
            ParseErrorKind::NoReferent { word } => render(&messages.no_referent, &[("word", word)]),
            ParseErrorKind::AllNotAllowed => render(&messages.all_not_allowed, &[]),
            ParseErrorKind::TooVague => render(&messages.too_vague, &[]),
            ParseErrorKind::NotAnAnswer => render(&messages.not_an_answer, &[]),
            ParseErrorKind::InvalidUtf8
            | ParseErrorKind::NotUnderstood
            | ParseErrorKind::InvalidScope
            | ParseErrorKind::StalePending => render(&messages.not_understood, &[]),
        };
        ParseError { kind, message }
    }

    fn fail(&self, kind: ParseErrorKind) -> ParseOutcome {
        ParseOutcome::Error(self.error(kind))
    }
}

fn close_segment(current: &mut Vec<Tok>, segments: &mut Vec<Segment>) {
    while current
        .last()
        .is_some_and(|token| token.kind == TokKind::Conj)
    {
        current.pop();
    }
    let start = current
        .iter()
        .position(|token| token.kind != TokKind::Conj)
        .unwrap_or(current.len());
    let tokens: Vec<Tok> = current.drain(..).skip(start).collect();
    if !tokens.is_empty() {
        segments.push(Segment { tokens });
    }
}

fn words_at(tokens: &[RawTok], at: usize, phrase: &[String]) -> bool {
    phrase.iter().enumerate().all(|(offset, expected)| {
        matches!(
            at.checked_add(offset).and_then(|position| tokens.get(position)),
            Some(RawTok::Word(word)) if word.folded == *expected
        )
    })
}

/// Objects of `subset` whose nouns and adjectives cover every word; objects
/// matched by at least one noun are preferred over adjective-only matches.
fn match_words(words: &[&str], index: &ScopeIndex, subset: &[usize]) -> Vec<usize> {
    let mut hits = Vec::new();
    let mut noun_hits = Vec::new();
    for &candidate in subset {
        let Some(object) = index.objects.get(candidate) else {
            continue;
        };
        if words
            .iter()
            .all(|word| object.nouns.contains(*word) || object.adjectives.contains(*word))
        {
            hits.push(candidate);
            if words.iter().any(|word| object.nouns.contains(*word)) {
                noun_hits.push(candidate);
            }
        }
    }
    if noun_hits.is_empty() {
        hits
    } else {
        noun_hits
    }
}

fn join_list(names: &[&str], or_word: &str) -> String {
    let mut out = String::new();
    for (position, name) in names.iter().enumerate() {
        if position > 0 {
            if position + 1 == names.len() {
                out.push(' ');
                out.push_str(or_word);
                out.push(' ');
            } else {
                out.push_str(", ");
            }
        }
        out.push_str(name);
    }
    out
}

/// Single-pass placeholder substitution, whitespace tidying and
/// capitalization of the first letter.
fn render(template: &str, values: &[(&str, &str)]) -> String {
    let mut out = String::with_capacity(template.len() + 64);
    let mut rest = template;
    while let Some(start) = rest.find('{') {
        out.push_str(&rest[..start]);
        let after = &rest[start + 1..];
        if let Some(end) = after.find('}')
            && let Some((_, value)) = values.iter().find(|(key, _)| *key == &after[..end])
        {
            out.push_str(value);
            rest = &after[end + 1..];
            continue;
        }
        out.push('{');
        rest = after;
    }
    out.push_str(rest);
    let mut tidy = String::with_capacity(out.len());
    for c in out.chars() {
        if c.is_whitespace() {
            if !tidy.is_empty() && !tidy.ends_with(' ') {
                tidy.push(' ');
            }
        } else {
            if matches!(c, '?' | '.' | '!' | ',' | ';' | ':') && tidy.ends_with(' ') {
                tidy.pop();
            }
            tidy.push(c);
        }
    }
    while tidy.ends_with(' ') {
        tidy.pop();
    }
    capitalize_first(&tidy)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn renders_in_one_pass() {
        assert_eq!(
            render(
                "¿{prep} qué quieres {verb} {obj}?",
                &[("prep", "con"), ("verb", "abrir"), ("obj", "")]
            ),
            "¿Con qué quieres abrir?"
        );
        assert_eq!(render("x {list}", &[("list", "{list}")]), "X {list}");
        assert_eq!(render("{unknown} {", &[]), "{Unknown} {");
    }

    #[test]
    fn joins_lists() {
        assert_eq!(join_list(&["a"], "o"), "a");
        assert_eq!(join_list(&["a", "b"], "o"), "a o b");
        assert_eq!(join_list(&["a", "b", "c"], "or"), "a, b or c");
    }

    #[test]
    fn builtin_lexicons_compile() {
        assert_eq!(Parser::spanish().unwrap().language(), "es");
        assert_eq!(Parser::english().unwrap().language(), "en");
        assert!(Parser::builtin_lexicon_json("fr").is_none());
    }
}
