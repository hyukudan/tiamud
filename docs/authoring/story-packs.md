# StoryPack authoring

A **StoryPack** is a self-contained branching narrative (gamebook, "choose your
adventure" with stats and dice) for the separate narrative ruleset proposed in
[ADR 0025](../decisions/0025-narrative-ruleset-story-core.md). It is not a
WorldPack and shares no game state with the MUD.

Current status (see the [completion matrix](../project/COMPLETION_MATRIX.md)):
the pure ruleset (`crates/story-core`, phase S0) and the offline authoring tools
(`crates/story-tools`, binary `tiamud-story`, phase S1 tooling) exist. ADR 0025
is still **Proposed**. There is no server actor, `story.*` protocol, Telnet or
browser surface, and no persisted save slot: StoryPacks are not playable on a
running Tiamud server yet. Everything below runs offline on an author machine.

The Rust types and validator in `crates/story-core` are the exact authority.
The published [JSON Schema](../../crates/story-tools/schema/story-pack.v1.schema.json)
(draft 2020-12, also printed by `tiamud-story schema`) describes the canonical
document shape for editors; it does not replace `tiamud-story validate`.

The reviewed Spanish example is
[`crates/story-core/fixtures/cripta-del-nigromante.story.json`](../../crates/story-core/fixtures/cripta-del-nigromante.story.json).

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

## Limits

| Quantity | Limit |
| --- | --- |
| Pack file | 2 MiB |
| Nodes | 4096 |
| Choices per node / in total | 32 / 32 768 |
| Variables / items | 256 / 256 |
| Effects per choice (including nested) | 64 |
| `if` nesting / condition nesting / condition nodes | 4 / 8 / 64 |
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

### `preview <pack> [--node id] [--locale es] [--seed N]`

Renders one node as a player would see it in plain text: title, text, turn,
score, visible stats, inventory and numbered available choices (or the ending
banner). Conditions are evaluated against the initial state, and the command
notes how many authored choices that state hides. Authored ids and hidden
variables are never printed.

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

Unknown fields, unknown node/variable/item names and `choose: 0` are script
errors. The first failed step stops the run with its step number and every
mismatch; the command then exits with failure. See the reviewed scripts in
[`crates/story-tools/tests/fixtures/`](../../crates/story-tools/tests/fixtures/).

### `play <pack> [--seed N] [--locale es]`

An interactive player on standard input and output for authors. Type a choice
number, or `deshacer`/`undo`, `estado`/`state` (full author state including
hidden variables and the seed), `mirar`/`look`, `ayuda`/`help` and
`salir`/`quit`. End of input also quits. Dice rolls, items, score changes and
media cues are shown as bracketed notes. Interface words are Spanish unless the
locale is English. Nothing is saved.

### `fmt <pack> [--check | --output <file> [--force]]`

Prints the canonical form of a valid pack: object keys sorted, two-space
indentation, trailing newline, optional fields omitted instead of `null`.
`--check` fails when the file differs from its canonical form; `--output`
writes atomically and refuses to replace an existing file without `--force`.
The reviewed example fixture is hand-formatted and is not required to be
canonical.

### `schema`

Prints the published StoryPack v1 JSON Schema.
