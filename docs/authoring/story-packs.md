# StoryPack authoring

A **StoryPack** is a self-contained branching narrative (gamebook, "choose your
adventure" with stats and dice) for the separate narrative ruleset proposed in
[ADR 0025](../decisions/0025-narrative-ruleset-story-core.md). It is not a
WorldPack and shares no game state with the MUD.

Current status (see the [completion matrix](../project/COMPLETION_MATRIX.md)):
the pure ruleset (`crates/story-core`, phase S0) and the offline authoring tools
(`crates/story-tools`, binary `tiamud-story`, phase S1 tooling) exist. Format
v2 adds menu-driven adventures — verb menus, hotspots, speakers with portraits
and dialogue trees — as a library-only core model proposed in
[ADR 0028](../decisions/0028-story-menus-portraits-and-dialogue-trees.md)
(see [Menu scenes](#menu-scenes-format-v2)). ADR 0025 and ADR 0028 are still
**Proposed**. There is no server actor, `story.*` protocol, Telnet or browser
surface, and no persisted save slot: StoryPacks are not playable on a running
Tiamud server yet. Everything below runs offline on an author machine.

The Rust types and validator in `crates/story-core` are the exact authority.
The published [JSON Schema](../../crates/story-tools/schema/story-pack.v2.schema.json)
(draft 2020-12, also printed by `tiamud-story schema`) describes the canonical
document shape for editors; it does not replace `tiamud-story validate`. The
historical [v1 schema](../../crates/story-tools/schema/story-pack.v1.schema.json)
is kept unchanged.

The reviewed Spanish examples are the gamebook
[`cripta-del-nigromante.story.json`](../../crates/story-core/fixtures/cripta-del-nigromante.story.json)
(format v1) and the noir menu adventure
[`lluvia-de-neon.story.json`](../../crates/story-core/fixtures/lluvia-de-neon.story.json)
(format v2).

## Document

```json
{
  "format_version": 1,
  "id": "cripta-del-nigromante",
  "title": { "es": "La cripta del nigromante" },
  "start": "inicio",
  "variables": [ ... ],
  "items": [ ... ],
  "initial_inventory": [ ... ],
  "nodes": [ ... ]
}
```

Every object rejects unknown fields, so a typo fails loudly. `variables`,
`items` and `initial_inventory` may be omitted. Optional fields are omitted
rather than written as `null` in canonical form.

`format_version` is `1` or `2`. Format v2 is a strict superset: it only adds
optional fields (`verbs`, `characters`, `dialogues` and per-node `background`,
`cues`, `hotspots`, `present`, `menu`, plus the `uses` condition). A document
that still says `1` may not use them; `tiamud-story fmt --upgrade` rewrites a
v1 pack as v2 by changing only the version.

- **Identifiers** (`id`, node, choice, variable, item, ending and cue asset
  ids) are 1–64 bytes of lowercase ASCII letters, digits, `-`, `_` or `.`,
  starting with a letter or digit. Published ids are durable: renaming one later
  needs an explicit alias or migration.
- **Localized text** maps locale tags to plain text: `{ "es": "…", "en": "…" }`,
  1–16 locales. Resolution order is exact locale, language prefix, Spanish,
  English, then the first entry. Labels, titles and names allow at most 256
  bytes and no newlines; node text allows 4096 bytes and `\n`. Text is
  presentation only: no rule ever reads prose.

### Variables

```json
{ "id": "resistencia", "label": { "es": "Resistencia" }, "visible": true,
  "kind": { "int": { "min": 0, "max": 20, "initial": 12 } } }
{ "id": "esqueleto_vencido", "kind": { "bool": { "initial": false } } }
```

Integers are bounded by their declared `min..=max` (each within ±1 000 000 000)
and every change is clamped to them. Only `visible` variables appear on the
player's stat sheet, with their `label` (or the id if no label is given).

### Items and inventory

`items` declares `{ "id", "name" }`. The inventory is a set of declared ids;
`initial_inventory` lists the ones held at the start.

### Nodes, choices and endings

```json
{
  "id": "vestibulo",
  "text": { "es": "Una antorcha arde junto a una puerta de hierro cerrada." },
  "choices": [
    {
      "id": "abrir-puerta",
      "label": { "es": "Abrir la puerta de hierro con la llave" },
      "when": { "has_item": "llave-de-hierro" },
      "effects": [{ "take": "llave-de-hierro" }, { "score": 100 }],
      "target": "camara-del-tesoro"
    }
  ]
}
```

- A choice whose `when` condition is false is **not shown** at all; it never
  receives a token.
- After the effects run, the story moves to the last `goto` or roll-test
  destination, otherwise to `target`, otherwise it stays on the same node.
- A node with `"ending": { "id": "tesoro", "kind": "victory" }` ends the story
  when entered and cannot offer choices. Kinds are `victory`, `defeat` and
  `neutral`. The `end` effect ends the story mid-choice.

## Conditions

| Form | Meaning |
| --- | --- |
| `{ "all": [c, …] }` / `{ "any": [c, …] }` | Conjunction / disjunction of at least one condition |
| `{ "not": c }` | Negation |
| `{ "var_cmp": { "var": "x", "op": "ge", "value": 14 } }` | Integer comparison; `op` is `eq`, `ne`, `lt`, `le`, `gt` or `ge` |
| `{ "has_item": "llave" }` | Item is held |
| `{ "visited": "foso" }` | Node was entered at least once (the start node counts) |
| `{ "flag": "esqueleto_vencido" }` | Boolean variable is true |
| `{ "uses": { "node": "oficina", "entry": "investigar-ventana", "op": "eq", "value": 0 } }` | (v2) Compares how many times a menu entry has been used; the counter saturates at 9999 |

A condition tree nests at most 8 levels and holds at most 64 nodes.

## Effects

Effects run in order when a choice is taken.

| Effect | Meaning |
| --- | --- |
| `{ "set": { "var": "x", "value": 3 } }` | Assigns a value of the variable's type; integers must lie within bounds |
| `{ "add": { "var": "x", "delta": -2 } }` | Checked addition, then clamped to the bounds |
| `{ "give": "item" }` / `{ "take": "item" }` | Adds / removes an inventory item |
| `{ "goto": "node" }` | Sets the destination; the last one wins |
| `{ "roll": { … } }` | Seeded dice, see below |
| `{ "if": { "when": c, "then": [ … ], "otherwise": [ … ] } }` | Branch evaluated against the state at that point; nests at most 4 levels |
| `{ "cue": { "music": { "asset": "id" } } }`, `{ "cue": { "sfx": { "asset": "id" } } }`, `{ "cue": "stop_music" }` | Presentation-only media cue by reviewed asset id, never a URL; losing it never changes play |
| `{ "score": 50 }` | Adds to the score (each delta within ±1 000 000) |
| `{ "end": { "id": "huida", "kind": "neutral" } }` | Ends the story; remaining effects are skipped |

One choice may hold at most 64 effects, counting nested `if` branches.

### Dice

```json
{ "roll": {
    "dice": { "count": 2, "sides": 6, "modifier": 0 },
    "modifier_var": "destreza",
    "store": "tirada",
    "test": { "at_least": 14, "success": "golpe", "failure": "herida" }
} }
```

`count` is 1–16, `sides` 2–100 and `modifier` within ±1 000 000. The total is
the faces plus `modifier` plus the optional integer `modifier_var`. `store`
writes the total, clamped, into an integer variable. `test` compares the total
with `at_least` and, when `success`/`failure` is set, overrides the destination
like a `goto`.

Dice come from a local SplitMix64 generator whose state is part of the story
state. The same pack, seed and choices always produce the same rolls; author
tools take the seed as an argument and default to `0`.

## Menu scenes (format v2)

Format v2 expresses menu-driven investigation in the style of 1980s Spanish
adventures: a scene with a background, characters with portraits, a menu of
verbs applied to targets, answers that change when an action is repeated, and
dialogue trees. A node may combine a menu with ordinary choices.

### Verbs

```json
"verbs": [
  { "id": "investigar", "label": { "es": "INVESTIGAR", "en": "INVESTIGATE" },
    "targets": "hotspot",
    "default": [{ "text": { "es": "Buscas a conciencia, pero no hay nada." } }] }
]
```

`targets` is `none` (the verb acts on the scene), `hotspot` (a hotspot of the
current node), `character` (a character present on the node) or `item` (an
item currently held). `default` (1–8 text blocks) answers whenever nothing more
specific applies. Labels are display text only and are never parsed by rules.

### Characters and portraits

```json
"characters": [
  { "id": "lola", "name": { "es": "Lola" },
    "portraits": { "neutral": "retrato-lola-neutral", "nerviosa": "retrato-lola-nerviosa" } }
]
```

`portraits` maps expression ids to reviewed portrait asset ids (never URLs, at
most 16). A block or presence without an expression uses
`default_expression`, or `neutral`. A missing portrait only loses media: the
view carries no portrait and `validate` reports a lint.

### Text blocks

Responses, defaults and dialogue lines are lists of blocks:
`{ "speaker": "lola", "expression": "nerviosa", "text": { "es": "—…" } }`.
`speaker` must be a declared character; `expression` needs a speaker. Block
text follows node text rules (4096 bytes per locale, newlines allowed).

### Scene fields

```json
{
  "id": "club",
  "text": { "es": "El Faro Azul huele a ozono y a ginebra sintética." },
  "background": "fondo-club",
  "cues": [{ "music": { "asset": "musica-club" } }],
  "hotspots": [
    { "id": "reservado", "label": { "es": "Reservado" } },
    { "id": "callejon", "label": { "es": "Callejón trasero" }, "when": { "flag": "sabe_damaso" } }
  ],
  "present": [{ "character": "lola", "expression": "neutral" }],
  "menu": { … }
}
```

`background` is a reviewed asset id. `cues` (at most 4) are emitted every time
the node is entered and are repeated in the view so a late client can
resynchronize. A hotspot or presence whose `when` is false is not offered as a
target.

### Menus, entries and responses

```json
"menu": {
  "verbs": [{ "verb": "mirar" }, { "verb": "investigar" }, { "verb": "moverse" }],
  "entries": [
    {
      "id": "investigar-ventana",
      "verb": "investigar",
      "target": "ventana",
      "responses": [
        { "id": "primera",
          "when": { "uses": { "node": "oficina", "entry": "investigar-ventana", "op": "eq", "value": 0 } },
          "blocks": [{ "text": { "es": "Te parece ver un destello al otro lado de la calle." } }] },
        { "id": "dron", "once": true,
          "blocks": [{ "text": { "es": "Un dron de vigilancia lleva horas frente a tu ventana." } }],
          "effects": [{ "set": { "var": "dron_visto", "value": true } }] }
      ],
      "exhausted": [{ "text": { "es": "El dron sigue ahí. Nada nuevo." } }]
    }
  ],
  "defaults": [{ "verb": "mirar", "blocks": [{ "text": { "es": "La lluvia no deja ver nada." } }] }]
}
```

- `verbs` lists the declared verbs this scene offers, in display order (1–16,
  each with an optional `when`).
- An entry answers one `(verb, target)` pair; the target must match the verb's
  `targets` kind and exist (a hotspot of this node, a present character or a
  declared item), otherwise validation fails. At most one entry per pair.
- When the player applies a verb to a target, the **first** response whose
  `when` holds and that is not an already spent `once` response is given: its
  blocks are shown, its effects run (same DSL and budget as choices), then the
  story moves to its `target` (a `goto` effect overrides it) or, if it neither
  moves nor ends, opens its `dialogue`.
- If no response applies, `exhausted` ("nothing new") is shown; without it,
  the scene `defaults` entry for the verb, then the verb `default`. A verb
  applied to a target with no entry also gets those defaults.
- Every use of an entry increments a counter (saturating at 9999). Responses
  are selected with the count *before* the current use, so `uses == 0` means
  "the first time". Repeating an investigation therefore advances only through
  counters, flags and items that the author sets — never through prose.

The answer of the last menu action stays visible until the next input, even
when it moved the player to another scene.

### Dialogue trees

```json
"dialogues": [
  { "id": "charla-lola", "start": "saludo",
    "nodes": [
      { "id": "saludo",
        "lines": [{ "speaker": "lola", "text": { "es": "—¿Qué te pongo, detective?" } }],
        "options": [
          { "id": "preguntar-iker", "label": { "es": "Preguntar por Iker Soto" },
            "once": true, "next": { "goto": "iker" } },
          { "id": "pedir-cafe", "label": { "es": "Pedir un café solo" },
            "effects": [{ "set": { "var": "lola_confia", "value": true } }],
            "next": { "goto": "cafe" } },
          { "id": "despedirse", "label": { "es": "Despedirte" } }
        ] },
      { "id": "iker", "lines": [ … ],
        "options": [{ "id": "volver", "label": { "es": "Cambiar de tema" }, "next": "return" }] }
    ] }
]
```

A response with `"dialogue": "charla-lola"` (typically HABLAR on a character)
enters the tree at `start`. Each node shows 1–8 lines and offers 1–16 options
with `when`, `once` and effects. `next` is `{ "goto": node }` within the same
dialogue, `"return"` to the start node, or `"exit"` (the default) back to the
scene menu. A `goto` or `end` effect always leaves the dialogue. There is no
call stack: only one dialogue is active. While a dialogue waits for an answer
the scene menu and choices are hidden; if none of its options is available any
more, the scene menu comes back.

### What the player sees

The view adds the background id, scene cues, portraits of present characters,
a message window (the last response followed by the current dialogue lines,
each with speaker name and portrait id), the menu (verbs with their targets)
or the dialogue options. Verbs, targets and options carry opaque tokens bound
to the turn — and, for targets, to their verb — so a token from an earlier
view or for another verb is rejected. Authored ids, counters and spent
responses never appear.

`view_pages` projects that view for one locale into ordered pages whose JSON
is at most 12 KiB each (leaving room for a future protocol envelope within the
16 KiB frame); long scenes are paginated, never truncated.

## Limits

| Quantity | Limit |
| --- | --- |
| Pack file | 2 MiB |
| Nodes | 4096 |
| Choices per node / in total | 32 / 32 768 |
| Variables / items | 256 / 256 |
| Effects per choice, response or dialogue option (including nested) | 64 |
| `if` nesting / condition nesting / condition nodes | 4 / 8 / 64 |
| Verbs (declared, and per menu) | 16 |
| Hotspots / present characters / cues per node | 32 / 8 / 4 |
| Menu entries per node / in total | 128 / 4096 |
| Responses per entry | 8 |
| Text blocks per response, default, "nothing new" or dialogue node | 8 |
| Characters / portrait expressions per character | 64 / 16 |
| Dialogues / nodes per dialogue / dialogue nodes in total / options per node | 256 / 128 / 4096 / 16 |
| Menu entry use counter | saturates at 9999 |
| One view page (JSON) | 12 KiB |
| Turns per playthrough | 1 000 000 |
| Undo depth in the author tools | 64 |
| Test script file / steps | 256 KiB / 4096 |
| Interactive input line | 256 bytes (longer lines are discarded whole) |

## The `tiamud-story` CLI

Run it offline from the repository root:

```sh
cargo run --locked -p tiamud-story-tools --bin tiamud-story -- <command> …
```

### `validate <pack> [--deny-warnings]`

Parses, validates and lints the pack. Errors (malformed JSON, unknown fields,
dangling references, out-of-range values, exceeded limits) fail the command.
Findings that do not make the pack unsafe are printed and exit successfully,
unless `--deny-warnings` is given:

- `warning:` lines come from the core validator: nodes unreachable from the
  start node, nodes with neither choices nor ending, nodes whose every choice
  is conditional, roll tests without destinations and packs where no ending is
  reachable.
- `lint:` lines come from the authoring linter, which computes a sound
  over-approximation of what any playthrough can observe (integer ranges from
  declared bounds or, for variables that are never changed, their initial
  value; boolean values ever assigned; items ever given or taken; reachable
  nodes). It reports choice conditions that can never hold or always hold,
  dead `if` branches, roll tests that always or never succeed, nodes reachable
  only through impossible choices, nodes where no choice can ever be
  available, and **soft-locks**: reachable nodes from which no ending can be
  reached. A lint that says "never" is certain; anything uncertain is silent,
  so a clean report is not a proof that every ending is winnable.
- For format v2 the linter also reports menu verbs that are never offered,
  menu entries that can never be selected (hidden hotspot, absent character or
  unobtainable item), responses whose condition never holds or that are
  shadowed by an earlier always-applying repeatable response, "nothing new"
  texts that can never be shown, dialogues no response starts, dialogue nodes
  unreachable from their start, options that can never be offered, and
  portraits that name an expression the character does not declare. Menu
  responses and dialogue exits count for reachability, endings and
  soft-locks. Dangling references (a target that is not a hotspot of the node,
  an undeclared verb, speaker or dialogue) are validation errors, not lints.

The summary line states the format version and, for v2 packs, the number of
verbs, menu entries, characters and dialogues.

### `preview <pack> [--node id] [--dialogue id[/node]] [--locale es] [--seed N]`

Renders one node as a player would see it in plain text: title, text, turn,
score, visible stats, inventory and numbered available choices (or the ending
banner). Conditions are evaluated against the initial state, and the command
notes how many authored choices that state hides. Authored ids and hidden
variables are never printed. Menu scenes also show `[fondo: …]`, present
characters with portrait ids, and `Acciones:` with numbered verbs and
targets (`(nada)` when a verb has nothing to apply to); `--dialogue` renders a
dialogue node over the scene with its lines and numbered `Respuestas:`.

### `test <pack> --script <file>`

Replays a scripted playthrough with a fixed seed and checks expectations:

```json
{
  "format_version": 1,
  "seed": 3,
  "steps": [
    { "expect": { "node": "inicio", "turn": 0, "choices": 1, "ended": false } },
    { "choose": 1 },
    { "expect": { "node": "vestibulo", "vars": { "destreza": 12 } } },
    "undo",
    { "expect": { "has": [], "lacks": ["antorcha"], "score": 0 } }
  ]
}
```

- `{ "choose": n }` takes the *n*-th available choice, numbered from 1 exactly
  as `preview` and `play` show it.
- `"undo"` restores the state before the last accepted choice.
- `{ "expect": { … } }` checks any subset of `node`, `turn`, `score`, `vars`
  (exact values, hidden variables included), `has`, `lacks`, `choices` (number
  available), `ended` and `ending` (ending id).

Script `format_version: 2` adds steps for menu scenes:

- `{ "menu": [v, t] }` applies the *v*-th offered verb to its *t*-th target
  (`{ "menu": [v] }` for a verb without target), numbered as `play` shows them.
- `{ "act": { "verb": "investigar", "target": "mesa" } }` names verb and target
  by authored id; a test is an authoring artefact, players only see tokens.
- `{ "say": n }` takes the *n*-th available dialogue option.
- `expect` also accepts `dialogue` (a dialogue awaits an answer), `options`
  (available dialogue options), `verbs` (offered menu verbs) and `uses`
  (`{ "node/entry": count }`).

See the reviewed
[`lluvia-victoria.script.json`](../../crates/story-tools/tests/fixtures/lluvia-victoria.script.json).

Unknown fields, unknown node/variable/item/verb/menu-entry names, numbers
equal to 0 and v2 steps in a v1 script are script errors. The first failed step stops the run with its step number and every
mismatch; the command then exits with failure. See the reviewed scripts in
[`crates/story-tools/tests/fixtures/`](../../crates/story-tools/tests/fixtures/).

### `play <pack> [--seed N] [--locale es]`

An interactive player on standard input and output for authors. Type a choice
number, or `deshacer`/`undo`, `estado`/`state` (full author state including
hidden variables and the seed), `mirar`/`look`, `ayuda`/`help` and
`salir`/`quit`. End of input also quits. Dice rolls, items, score changes and
media cues are shown as bracketed notes. Interface words are Spanish unless the
locale is English. Nothing is saved.

In menu scenes type `N` (verb without target), `N M` (verb *N* on target *M*),
`cN` (gamebook choice *N*), or the displayed words, such as `mirar mesa` or
`investigar ventana`; words match the labels the view offers without regard
to case or Spanish accents, and a lone `mirar` still redraws the scene. While a
dialogue waits, a number answers it. Response and dialogue text appears in a
`»` message window with speaker and portrait id.

### `fmt <pack> [--check | --output <file> [--force]] [--upgrade]`

Prints the canonical form of a valid pack: object keys sorted, two-space
indentation, trailing newline, optional fields omitted instead of `null`.
`--check` fails when the file differs from its canonical form; `--output`
writes atomically and refuses to replace an existing file without `--force`.
`--upgrade` rewrites a format v1 pack as format v2; only `format_version`
changes. The reviewed example fixtures are hand-formatted and are not required
to be canonical.

### `schema`

Prints the published StoryPack v2 JSON Schema (a v1 document also satisfies
it).
