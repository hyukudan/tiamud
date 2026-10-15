# Story parser (interactive-fiction command parser)

`crates/story-parser` (package `tiamud-story-parser`) is a pure, bounded,
data-driven parser for parser-based interactive fiction, in the Spanish
*aventuras conversacionales* tradition (PAWS/DAAD, the Spanish Inform 6
library) and in English. It turns a player line such as
`coge la llave dorada y ábrela` or `put the lamp in the box` into typed
commands whose objects are ids supplied by the caller.

Current status (see the [completion matrix](../project/COMPLETION_MATRIX.md)):
this is a **library only**. No StoryPack field, server actor, `story.command`
message, Telnet or browser surface uses it yet, and it does not depend on
`tiamud-story-core`. [ADR 0025](../decisions/0025-narrative-ruleset-story-core.md)
is still **Proposed**, and its phase table requires a dedicated ADR (parser
and language) before the parser is wired into story sessions.

The Rust types and validator in `crates/story-parser` are the exact
authority. The shipped lexicons are
[`lexicons/es.json`](../../crates/story-parser/lexicons/es.json) and
[`lexicons/en.json`](../../crates/story-parser/lexicons/en.json).

## What the parser does

- **Normalization.** Lines are at most 256 bytes and 16 words; longer lines are
  rejected whole, never truncated. Matching is case-insensitive and ignores the
  accents of vowels (`á é í ó ú ü` and grave/circumflex forms), so `CÓGELO`,
  `cogelo` and `cógelo` are the same word. **`ñ` stays distinct from `n`**,
  as in Spanish (`año`/`ano`); decomposed input (`n` + U+0303) is recomposed.
  Punctuation other than the separators below is treated as whitespace
  (`¿`, `¡`, quotes).
- **Several commands per line**, at most 4: `.`, `;`, `!` and `?` always
  separate; `then_phrases` (`y luego`, `luego`, `then`, `and then`) always
  separate; `,` and `and_words` (`y`, `and`) separate only when the next word
  starts a command (a verb, an enclitic verb or a direction). Otherwise they
  stay inside the command as list conjunctions (`todo menos la llave y la
  vela`). Empty commands (`n...`) are skipped; a fifth command rejects the line.
- **Verbs** in infinitive, imperative and abbreviated forms (`coger`, `coge`,
  `x`, `ex`, `i`, `z`), mapped to a canonical action id. Verbs flagged `meta`
  (save, restore, undo, score, help, quit, restart) are out-of-world; the
  in-world `inventario`, `mirar`, `examinar` and `esperar` are ordinary verbs.
- **Enclitic pronouns** (`cógelo`, `ábrela`, `examínalo`, `dáselo`, `dámelo`,
  `dale`, `háblale`, `cogerla`): the verb word is stripped of the longest
  matching suffix whose stem is a verb with an object pattern. `direct`
  suffixes (`lo`, `la`, `melo`, `selo`…) make the last referenced object the
  first object; `indirect` suffixes (`le`, `les`) fill the recipient
  (`dale la moneda`) or the object of a leading preposition (`háblale`) when
  the line leaves it empty; `none` suffixes (`me`, `te`, `se`, `nos`) are
  ignored. Pronouns do not track gender or number.
- **Stand-alone pronouns** (`lo`, `la`, `ella`, `it`, `them`): a noun phrase
  that is only a pronoun refers to the last referenced object. Within one
  line, the last referenced object becomes the first object of the previous
  command (`coge la moneda y ponla en la caja`).
- **Contractions** (`del` → `de el`, `al` → `a el`) and **phrases**
  (`dentro de` → `en`, `debajo de` → `bajo`, `pick up` → `take`,
  `turn on` → `light`) are expanded before parsing.
- **Directions** and abbreviations (`n s e o ne no se so`, `arriba abajo
  dentro fuera sube baja sal`; `n s e w ne nw se sw u d in out`) as a whole
  command become the lexicon's `go_action` with a `direction`. Verbs with
  `"directions": true` (`ve al norte`, `go to the north`) accept one direction
  after optional prepositions and articles.
- **Grammar** per verb, as patterns (see below): `verb`, `verb obj`,
  `verb obj prep obj2` and `verb prep obj`, each optionally overriding the
  action (`mirar en X` → `search`, `abrir X con Y` → `unlock`).
- **`todo` / `all`** with `excepto`/`menos`/`salvo`/`except`/`but` lists, only
  on patterns marked `multi`. An ambiguous exclusion excludes every matching
  object (`coge todo menos la llave` with two keys excludes both). At most 8
  exclusions. Plural noun filters (`todas las monedas`) and object lists
  outside `excepto` (`coge la llave y la vela`) are **not** supported and are
  reported as not understood.
- **Errors** name the problem in the lexicon's language, PAWS/Infocom style:
  `No conozco la palabra 'xyzzy'.`, `No ves eso por aquí.`,
  `Esa frase no empieza por un verbo que conozca.`

## Output

```rust
pub struct Command {
    pub action: ActionId,        // canonical action id from the lexicon
    pub meta: bool,              // out-of-world verb
    pub obj: Option<ObjRef>,     // object id from the scope
    pub prep: Option<PrepId>,    // canonical preposition (pattern `prep` or `lead`)
    pub obj2: Option<ObjRef>,
    pub direction: Option<DirectionId>,
    pub all: bool,               // `todo`/`all`; the caller decides what it covers
    pub except: Vec<ObjRef>,     // only with `all`
}

pub enum ParseOutcome {
    Command(Command),
    Ambiguous { candidates: Vec<Candidate>, question: String, pending: Pending },
    Incomplete { question: String, pending: Pending },
    Error(ParseError),           // `kind` plus a rendered localized `message`
}
```

`parse_line(input, &scope)` returns one outcome per command, in order, and
stops at the first outcome that is not a command; the rest of the line is
discarded, as in classic parsers. Commands before the stop are still valid and
should be executed first.

### Questions and answers

- **Ambiguity** (`coge la llave` with two keys) returns at most 8 candidates
  (`{id, name}`) and a rendered question:
  `¿Cuál quieres decir: la llave dorada o la llave de hierro?`. More than 8
  matches is the error `Tendrás que ser más concreto.` instead.
- **Missing objects** (`coge`, `pon la moneda`, `ve`) return `Incomplete` with
  `¿Qué quieres coger?`, `¿En qué quieres poner una moneda?` or
  `¿Hacia dónde quieres ir?`.
- Pass the player's reply to `parser.answer(&pending, reply, &scope)`. An
  ambiguity reply may be a number (`2`), an ordinal (`segunda`, `second`) or
  words that narrow the candidates (`la dorada`, `la de hierro`,
  `the golden one`); a reply that still matches several candidates asks again
  with the narrowed list. A missing-object reply is a noun phrase, optionally
  after a preposition (`en la caja`, `with the key`). A reply that starts with
  a verb or direction, or that contains several commands, is parsed as a new
  line instead.
- `Pending` is bound to the lexicon that produced it; answering with another
  lexicon returns `StalePending`. It is an in-memory value and is not
  serializable yet.

## The scope trait

```rust
pub trait Scope {
    fn objects(&self) -> &[ScopeObject];              // ≤ 128, deterministic order
    fn last_referenced(&self) -> Option<&str> { None } // pronoun referent
    fn knows_word(&self, folded: &str) -> bool { false } // out-of-sight vocabulary
}

pub struct ScopeObject {
    pub id: String,              // opaque, returned in commands
    pub name: String,            // display text for questions, e.g. "la llave dorada"
    pub nouns: Vec<String>,      // ≤ 16 single words
    pub adjectives: Vec<String>, // ≤ 16 single words
}
```

Nouns and adjectives are folded like player input, so `lámpara` matches
`lampara`. A noun phrase matches an object when every remaining word (after
articles and fillers are dropped) is one of its nouns or adjectives;
noun matches are preferred over adjective-only matches (`coge la dorada`
works). Words known to neither the lexicon nor the scope produce
"unknown word"; words known through `knows_word` but naming nothing in scope
produce "not here". A scope with more than 128 objects, more than 16 nouns or
adjectives per object, multi-word entries or oversized ids/names is rejected
with `InvalidScope`, never silently truncated. `SimpleScope` is a ready-made
implementation.

## Lexicon format (version 1)

Every object rejects unknown fields. Words are written naturally (accents
allowed) and folded at load time; each must be one alphanumeric word of at
most 32 bytes after folding.

```json
{
  "format_version": 1,
  "language": "es",
  "go_action": "go",
  "verbs": [
    { "action": "take", "display": "coger",
      "words": ["coger", "coge", "tomar", "toma"],
      "patterns": [{ "object": true, "multi": true }] },
    { "action": "look", "display": "mirar", "words": ["mirar", "mira", "m"],
      "patterns": [
        {},
        { "lead": "in", "object": true, "action": "search" },
        { "object": true, "action": "examine" }
      ] },
    { "action": "save", "display": "guardar", "meta": true,
      "words": ["guardar"], "patterns": [{}] },
    { "action": "go", "display": "ir", "directions": true, "words": ["ir", "ve"] }
  ],
  "prepositions": [{ "id": "in", "display": "en", "words": ["en"] }],
  "directions": [{ "id": "north", "words": ["norte", "n"] }],
  "articles": ["el", "la"],
  "fillers": ["de"],
  "pronouns": ["lo", "la"],
  "enclitics": [{ "suffix": "lo", "kind": "direct" }],
  "contractions": [{ "word": "del", "expands": ["de", "el"] }],
  "phrases": [{ "words": ["dentro", "de"], "replace": "en" }],
  "all_words": ["todo"],
  "except_words": ["excepto", "menos"],
  "and_words": ["y"],
  "then_phrases": [["y", "luego"], ["luego"]],
  "ordinals": [["primero", "primera"], ["segundo", "segunda"]],
  "messages": { "...": "see below" }
}
```

- **Ids** (`action`, preposition and direction `id`, `go_action`) follow the
  StoryPack identifier rule: 1–64 bytes of lowercase ASCII letters, digits,
  `-`, `_` or `.`. Action ids are canonical across languages (the shipped
  lexicons share `take`, `put_in`, `unlock`, `go`, …).
- **Patterns.** `{}` is the bare verb. `object` takes a noun phrase. `lead`
  names a preposition required before it (`mirar en X`). `prep` with
  `second: true` adds a second noun phrase after that preposition
  (`poner X en Y`). `multi` allows `todo`/`excepto` in the first noun phrase.
  `action` overrides the verb's action. `lead` and `prep` cannot be combined.
  The first pattern that fully matches wins; if none does, the one missing the
  fewest objects is used to ask a question. A verb needs at least one pattern
  unless it has `"directions": true`.
- **Conflicts rejected at load:** a word in two verbs, a word that is both a
  verb and a direction, a preposition word in two prepositions, a verb word
  that is also a conjunction or pronoun, duplicate ids, enclitics or ordinal
  words, and a `go_action` that is not a `directions` verb. A word may be an
  article and a pronoun (`la`) or a direction and a preposition (`in`).
- **Messages** are required templates of at most 256 bytes: `empty`,
  `too_long`, `too_many_words`, `too_many_commands`, `unknown_word` (`{word}`),
  `no_verb`, `not_understood`, `not_here`, `no_referent` (`{word}`),
  `all_not_allowed`, `too_vague`, `not_an_answer`, `missing_object`
  (`{verb}`), `missing_lead` (`{verb}`, `{prep}`), `missing_second`
  (`{verb}`, `{prep}`, `{obj}`), `missing_direction` (`{verb}`), `ambiguous`
  (`{list}`) and `or_word`. Placeholders are substituted in one pass, spaces
  are tidied and the first letter is capitalized (after `¿`/`¡`).

## Limits

| Quantity | Limit |
| --- | --- |
| Input line | 256 bytes, 16 words (rejected whole) |
| Commands per line | 4 |
| Ambiguity candidates | 8 (more is "be more specific") |
| `excepto` exclusions | 8 |
| Scope objects / nouns / adjectives per object | 128 / 16 / 16 |
| Lexicon file | 256 KiB |
| Lexicon word | 32 bytes after folding |
| Verbs / prepositions / directions | 256 / 64 / 32 |
| Words per entry or list, patterns per verb | 64, 8 |
| Enclitics / contractions / phrases / then-phrases | 64 / 64 / 64 / 64 |
| Contraction expansion / phrase length | 3 / 4 words |
| Ordinal groups | 8 |
| Message template | 256 bytes |

The parser uses no regular expressions, performs no I/O, keeps no global
state, uses `BTreeMap`/`BTreeSet` lookups and is deterministic. Fixed-seed
fuzz tests feed random bytes, random lexicon word soup, random answers and
mutated lexicons and assert no panic, bounded outcomes and determinism.

## Planned integration with `story-core`

Integration is **not implemented**; it requires the parser/language ADR that
ADR 0025 reserves for the parser phase. The intended shape, which keeps
`tiamud-story-core` pure:

1. A StoryPack gains an optional, versioned parser section (or a sibling
   file) naming a lexicon and, per item, localized nouns and adjectives.
   Validation rejects unknown actions and words that break lexicon limits.
2. The story session implements `Scope` from `StoryState`: items visible in
   the current node plus the inventory, with `last_referenced` kept in the
   session (not in prose).
3. Authored rules map `(action, obj, prep, obj2, direction)` to choices or
   effects, so a parsed command becomes an ordinary `StoryInput` and `step`
   stays the only transition. Unmatched commands produce an authored default
   reply. Meta actions (`undo`, `save`, `restore`…) are handled by the session,
   never by the ruleset.
4. `Ambiguous`/`Incomplete` questions are session state: the next
   `story.command` is passed to `Parser::answer`. Questions never advance the
   turn.
5. Browser and Telnet share the same parser and the same rendered questions;
   ADR 0025's 16 KiB frame and Telnet parity rules apply unchanged.
