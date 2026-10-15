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
(see [Menu scenes](#menu-scenes-format-v2)). Format v3 adds parser-based
interactive fiction — rooms with exits, objects, rules and a default library
driven by free text — proposed in
[ADR 0031](../decisions/0031-parser-interactive-fiction-storypack-v3.md)
(see [Parser worlds](#parser-worlds-format-v3)). ADR 0025, 0028 and 0031 are
still **Proposed**. The server's story sessions (S2, ADR 0029) play v1–v3
packs through `story.*` (parser worlds through `story.command`), with
reviewed background and portrait images, a per-session language and
declared save-compatible revisions (see
[Presentation, languages and save compatibility](#presentation-languages-and-save-compatibility)). Optional
[Lua scripts](#lua-scripts-format-v3-optional) (ADR 0034, proposed) run
only in the offline tools; the server rejects packs that declare them.

The Rust types and validator in `crates/story-core` are the exact authority.
The published [JSON Schema](../../crates/story-tools/schema/story-pack.v3.schema.json)
(draft 2020-12, also printed by `tiamud-story schema`) describes the canonical
document shape for editors; it does not replace `tiamud-story validate`. The
historical [v1](../../crates/story-tools/schema/story-pack.v1.schema.json) and
[v2](../../crates/story-tools/schema/story-pack.v2.schema.json) schemas are
kept unchanged.

The reviewed Spanish examples are the gamebook
[`cripta-del-nigromante.story.json`](../../crates/story-core/fixtures/cripta-del-nigromante.story.json)
(format v1) and the noir menu adventure
[`lluvia-de-neon.story.json`](../../crates/story-core/fixtures/lluvia-de-neon.story.json)
(format v2), and the lighthouse text adventure
[`parser/faro-de-punta-gris.story.json`](../../crates/story-core/fixtures/parser/faro-de-punta-gris.story.json)
(format v3). The v3 fixture lives in the `parser/` subdirectory; the server
catalog scans `TIAMUD_STORY_DIR` and its direct subdirectories, so the whole
`fixtures/` directory, with its reviewed `story-media.json`, is a playable
catalog.

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

`format_version` is `1`, `2` or `3`. Each format is a strict superset of the
previous one. Format v2 only adds optional fields (`verbs`, `characters`,
`dialogues` and per-node `background`, `cues`, `hotspots`, `present`, `menu`,
plus the `uses` condition); format v3 adds `world`, per-node `room` and the
object conditions and effects. A document may not use features newer than its
`format_version`; `tiamud-story fmt --upgrade` rewrites a v1 or v2 pack as v3
by changing only the version.

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
| `{ "carried": "lampara" }` | (v3) The object is held, directly or inside held containers |
| `{ "object_at": { "obj": "llave", "at": "nowhere" } }` | (v3) The object is exactly at a location (`"carried"`, `"nowhere"`, `{ "room": n }`, `{ "in": c }`) |
| `{ "object_is": { "obj": "puerta", "flag": "locked" } }` | (v3) An object flag (`open`, `locked`, `lit`, `worn`) is set |
| `{ "visible": "aceitera" }` | (v3) The object is in the parser's scope now |
| `{ "player_in": "linterna" }` | (v3) The current node is this one |
| `{ "turn_cmp": { "op": "ge", "value": 30 } }` | (v3) Compares the turn counter |

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
| `{ "move_object": { "obj": "llave", "to": "carried" } }` | (v3) Moves an object; a move into itself or its own contents is ignored, and leaving the player's hands clears `worn` |
| `{ "set_object": { "obj": "puerta", "flag": "open", "value": true } }` | (v3) Sets a flag the object has (`open`/`locked` need `openable`, `lit` needs `lightable`, `worn` needs `wearable` and is ignored when not held); opening unlocks and locking closes |

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

## Parser worlds (format v3)

Format v3 expresses classic Spanish *aventuras conversacionales* (PAWS/DAAD,
InformATE): the player types `coge la lámpara y enciéndela`, the
[story parser](story-parser.md) turns it into commands, and authored rules and
a default library answer them. It is proposed in
[ADR 0031](../decisions/0031-parser-interactive-fiction-storypack-v3.md).
The reviewed example is
[`parser/faro-de-punta-gris.story.json`](../../crates/story-core/fixtures/parser/faro-de-punta-gris.story.json),
with its walkthrough
[`faro-victoria.script.json`](../../crates/story-tools/tests/fixtures/faro-victoria.script.json).

### The world section

```json
"world": {
  "intro": { "es": "La tormenta te sorprendió…" },
  "max_score": 30,
  "lexicons": [
    { "id": "es",
      "verbs": [{ "action": "fill", "display": "llenar", "words": ["llenar", "llena"],
                  "patterns": [{ "object": true, "prep": "with", "second": true }, { "object": true }] }] }
  ],
  "messages": { "no_exit": { "es": "Por ahí solo hay roca y mar." } },
  "objects": [ … ],
  "rules": [ … ]
}
```

- `lexicons` (1–4) names shipped lexicons by id (`es`, `en`); the first is
  the default. Each may add up to 32 **pack verbs** in the lexicon's own JSON
  shape (see the [parser guide](story-parser.md#lexicon-format-version-1)): a
  synonym of a library action (`"action": "take"`) or a new action that only
  rules handle (`fill` above). A pack verb whose word is already used by the
  lexicon is a validation error.
- `messages` replaces default library messages by key (see
  [Library messages](#library-messages)).
- `intro` is shown once, before the first room description; `max_score`
  turns the score line into "N de M puntos".
- `prompt` (optional) is the parser prompt per locale, for example
  `{ "es": "¿Qué haces?", "en": "What now?" }`: one line of at most 64 bytes
  per locale, shown by the browser next to the command field and sent to
  GMCP clients. Without it the prompt is `>`.

### Rooms and exits

A node with `room` is a place the parser acts in; its `text` is the room
description shown when the player looks or arrives:

```json
{ "id": "escalera",
  "text": { "es": "Los peldaños de hierro giran y giran…" },
  "room": {
    "name": { "es": "Escalera de caracol" },
    "dark": true,
    "exits": [
      { "direction": "down", "to": "vestibulo" },
      { "direction": "up", "to": "linterna",
        "when": { "object_is": { "obj": "lampara", "flag": "lit" } },
        "blocked": { "es": "Sin luz no te atreves a seguir subiendo." } }
    ] } }
```

`direction` is a canonical id every lexicon knows (`north`, `south`, `east`,
`west`, `northeast`, `northwest`, `southeast`, `southwest`, `up`, `down`,
`in`, `out`). `when` gates the exit and `blocked` is shown when it fails
(otherwise "No puedes ir por ahí."). `door` names an openable object standing
in the room; it must be open to pass. An ending node cannot be a room. Nodes
without `room` still work as cut-scenes with choices.

### Objects

```json
{ "id": "lampara",
  "name": { "es": "lámpara de aceite" },
  "gender": "feminine",
  "nouns": { "es": ["lámpara", "farol", "candil"] },
  "adjectives": { "es": ["aceite", "pequeña"] },
  "description": { "es": "Una lámpara de aceite de las de antes…" },
  "location": { "in": "cofre" },
  "portable": true,
  "lightable": true }
```

- `name` has **no article**: messages add `el`/`la`/`un`/`una` (or
  `the`/`a`/`an`) from `gender`; `proper: true` (people, `Remedios`) never
  takes one.
- `nouns` (required) and `adjectives` give, for **every** lexicon language,
  up to 16 single words each; they are folded like player input, so accents
  are optional. Every word the player types must be a noun or adjective of
  the object.
- `location` is `{ "room": node }`, `"carried"`, `{ "in": container }` or
  `"nowhere"` (out of play until an effect moves it). `found_in` lists further
  rooms for fixed objects (doors, scenery) that stand in several rooms.
- Properties: `portable` (can be taken), `scenery` (never listed, never
  taken), `animate` (people: `talk`, `give`, `show`), `container`, `openable`
  with `open`, `locked` with `key` (a portable object), `lightable` with `lit`,
  `wearable` with `worn` (requires `carried`), and `text` for `read`. A
  container that is not openable is always open. Contradictions (portable
  scenery, locked and open, worn but not carried) are validation errors, and
  containment may not form a cycle.

### Rules

```json
{ "id": "llenar-deposito", "phase": "instead", "action": "fill",
  "obj": "deposito", "obj2": "aceitera",
  "when": { "carried": "aceitera" },
  "effects": [
    { "move_object": { "obj": "aceitera", "to": "nowhere" } },
    { "set": { "var": "deposito_lleno", "value": true } },
    { "score": 10 } ],
  "text": { "es": "Vacías la aceitera en el depósito…" } }
```

A rule is keyed by `action` (a canonical action id some lexicon produces),
optional `obj`, `obj2` and, for movement, `direction`; an absent key matches
anything. For each command the engine runs:

1. the first `before` rule that matches and whose `when` holds — its text
   and effects run, and the action continues unless it moved the player or
   ended the story;
2. the first matching `instead` rule, which replaces the action; otherwise
3. the **default library**, and, when it succeeds, the first matching `after`
   rule.

Rule text is shown before the effects run; a `goto` effect moves the player
and shows the new room; `end` ends the story. Order rules from specific to
general: the first match in each phase wins.

### Default library

| Action (Spanish words) | Default behaviour |
| --- | --- |
| `look` (`mira`, `m`, `l`) | Room name, description, visible things and open containers' contents; in the dark, "Está oscuro como boca de lobo." |
| `examine` (`examina`, `x`, `mira X`) | Description, plus contents of an open container |
| `search` (`mira en X`, `registra`) / `look_under` (`mira bajo X`) | Container contents, closed or empty containers; otherwise "No encuentras nada interesante." |
| `inventory` (`i`, `inventario`) | Carried things, then worn ones |
| `take` / `drop` (`coge`, `deja`, with `todo`/`excepto`) | Portable, non-scenery, non-animate objects; worn things must be taken off first |
| `open` / `close` | Openable objects; locked ones refuse |
| `unlock` / `lock` (`abre X con Y`, `cierra X con Y`) | Needs the declared key in hand |
| `put_in` (`mete X en Y`) | Into an open container, never into itself; `put_on` always refuses |
| `go` (`n`, `ve al norte`, `sube`, `sal`) / `enter` / `exit` | Exits of the room, their conditions and doors; `entra en la puerta` goes through that door |
| `wait` (`z`, `espera`) | "Pasa el tiempo." |
| `give` / `show` (`dale X a Y`) | People decline unless a rule says otherwise |
| `talk` (`habla con X`) | "X no tiene nada que decirte." unless a rule says otherwise |
| `read` | The object's `text` |
| `wear` / `remove` (`ponte`, `quítate`) | Wearable objects in hand |
| `switch_on` / `switch_off` (`enciende`, `apaga`) | Lightable objects; lighting a dark room describes it, darkening it says so |
| anything else | "No ocurre nada." |

A line runs command by command, each against a **fresh scope** (`abre el
cofre, coge la lámpara` sees the lamp the first command revealed), and stops
at the first failure, question or meta command. Each executed command is one
turn; parser errors, questions and meta commands take none. `todo`/`all`
acts on at most 32 objects.

### Scope, light and pronouns

The parser sees what is held (including the contents of open held
containers) and, when the room is lit, what stands in it, with every enclosing
container open. A dark room is lit by a lit object held or standing there.
Words of objects out of sight answer "No ves eso por aquí." rather than "No
conozco la palabra…". Pronouns (`cógela`, `examínalo`, `it`) refer to the
object of the last command while it is visible; they do not track gender.

### Questions

When a noun phrase is ambiguous or an object is missing, the parser asks
(`¿Cuál quieres decir: la lámpara de aceite o el fanal?`, `¿En qué quieres
poner la aceitera de latón?`). The question is kept in the state as the line,
its command index and up to four answers so far; a saved and restored state
answers it the same way. The reply may be a word (`el fanal`), an ordinal
(`segunda`), a number or a new command, which drops the question. The view's
`prompt.question` flag is set while a question waits.

### Meta commands

`deshacer`, `guardar`, `cargar`, `fin`/`salir`, `reiniciar`, `puntos` and
`ayuda` (and their English forms) are not world actions: `step_command`
returns them to the host, which handles undo, save slots, quitting and
restarting. `puntos` and `ayuda` also answer in the transcript (`help` can be
replaced through `messages`).

### Playing on the server

Put the pack in `TIAMUD_STORY_DIR` or in one of its direct subdirectories
(for example `parser/`); the server validates it at startup like any other
pack. Players enter with `historia <id>` and then type commands: the browser
shows a command field with the prompt (`world.prompt`, else `>`), Up/Down
history and the replies; Telnet treats every line that is not a reserved
story word as a command. By default the instance plays in Spanish when the
pack has an `es` lexicon, otherwise in its first lexicon; `historia <id> en`
picks another lexicon of the pack (see [Languages](#languages-and-the-session-locale)).
On the server the meta commands do this: `deshacer` undoes
the last input, `reiniciar` asks for confirmation and restarts when typed
again (and can itself be undone), `guardar`/`cargar` without a number point
to the save slots (`guardar 3`, `cargar 0` or the saved-games panel),
`fin`/`salir` leave the story with an autosave, and `puntos`/`ayuda` answer in
place. Lines are limited to 256 bytes and only commands that do something in
the world take a turn. In a shared story typed commands are never voted: the
host types in host and vote modes and the turn holder in turns mode. Keep
reserved story words (`ver`, `partidas`, `continuar`, `votar`, `asientos`,
`compartir`, `invitar`, `expulsar`, `modo`, `historias`) out of pack verbs:
on Telnet they never reach the parser.

### Library messages

Every default message has a key that `world.messages` can replace for any
language: `dark`, `dark_name`, `you_see`, `inside_you_see`, `empty`,
`inventory`, `inventory_empty`, `wearing`, `taken`, `already_carried`,
`not_portable`, `take_animate`, `nothing_to_take`, `dropped`, `not_carried`,
`nothing_to_drop`, `take_off_first`, `opened`, `not_openable`,
`already_open`, `locked`, `closed`, `not_closable`, `already_closed`,
`unlocked`, `no_lock`, `not_locked`, `wrong_key`, `locked_with`,
`already_locked`, `close_first`, `put_in`, `not_container`,
`container_closed`, `put_in_self`, `put_on`, `no_exit`, `door_closed`,
`wait`, `cant_enter`, `give_not_animate`, `give_declined`, `show_declined`,
`talk_not_animate`, `talk_nothing`, `nothing_written`, `worn`,
`not_wearable`, `already_worn`, `removed`, `not_worn`, `switched_on`,
`not_lightable`, `already_on`, `switched_off`, `already_off`, `goes_dark`,
`search_nothing`, `nothing_happens`, `score`, `score_of` and `help`.
Placeholders: `{obj}`/`{Obj}` (definite name, capitalized), `{aobj}`
(indefinite), `{obj2}`/`{Obj2}`, `{o}`/`{o2}` (Spanish gender ending `o`/`a`),
`{list}` (indefinite names joined with `y`/`and`), `{score}`, `{max}` and
`{turns}`. Spanish output contracts `de el`/`a el` to `del`/`al`.

### What the player sees

In a room the scene text is the room's **name** (or "Oscuridad"); the
transcript of the last input — intro, room descriptions, library and rule
replies, parser questions and errors — arrives as text blocks after any menu
or dialogue text, rendered in every lexicon language. `prompt` tells a client
that free text is accepted; it is absent after the end and while a v2
dialogue waits. Authored ids, object state and rules never appear.

## Presentation, languages and save compatibility

### Background and portrait images

`background` and portrait ids are reviewed asset ids, never URLs. The server
turns an id into an image only through the reviewed story media map,
`story-media.json` at the top of `TIAMUD_STORY_DIR` (format 2):

```json
{
  "formatVersion": 2,
  "assets": { "musica-lluvia": "assets/<sha256>/audio-lluvia.ogg" },
  "images": {
    "fondo-oficina": {
      "manifest": "assets/<sha256>/manifest.json",
      "alt": { "es": "Un despacho de detective de noche…", "en": "A detective's office at night…" }
    }
  }
}
```

- `assets` maps audio cue ids to content-addressed audio, as before (format
  1 files with only `assets` still load).
- `images` (at most 256) maps a background or portrait id to the
  **manifest** of an opaque image published by the asset pipeline
  (`tiamud-assets build`, AVIF/WebP/JPEG variants at 640 and 1280 px with
  dimensions, hashes, author, licence, provenance and review time, the same
  schema as world scenes) and its alternative text in 1–16 locales (1–512
  bytes each, one line). The manifest's own English `alt` is not used by
  stories; the map's alt is resolved in the session language.
- An id that is missing from the map simply has no image: the scene text
  and the character names already carry everything. Unreviewed paths,
  remote URLs, missing alt text or unknown fields fail the server start.

The browser downloads the image only when multimedia is allowed (not in
reader mode, with Save-Data or with images off); otherwise, and when the
manifest or the picture cannot be loaded, it shows the background's alt text
instead. Telnet prints the background's alt text as `[…]` under the scene
title; portraits stay as the "Presentes" names. The sample art of
`lluvia-de-neon` (office and club backgrounds, Lola's and Dámaso's neutral
portraits) is mapped in
[`crates/story-core/fixtures/story-media.json`](../../crates/story-core/fixtures/story-media.json);
the alley and dawn backgrounds and the other expressions are deliberately
unmapped. Its sources and provenance are in
[`apps/web/artwork/story-lluvia/`](../../apps/web/artwork/story-lluvia/PROVENANCE.md).

### Languages and the session locale

A story can be played in each of its languages: the lexicons of a parser
world (`world.lexicons`), or the locales of the pack title for packs without
a parser. Spanish is the default when offered, else the first one. Players
choose when they enter: `historia <id> en` (`story <id> en`, also
`historia compartir <id> en`); `en-GB` selects `en`. A language the story
does not offer is refused with `unknown_locale`. Every text of the view —
title, scene, labels, events, parser replies, image alt text — and the parser
lexicon follow the session language; untranslated text falls back as
described under *Localized text*. The language is fixed when the instance
starts: re-entering a live instance keeps it, and people joining a shared
story play in the host's language. Server interface lines (help, hints,
errors) stay Spanish. Translate the title of a v1/v2 pack into every
language you want to offer.

### Compatible revisions

A save slot is bound to the SHA-256 of the exact pack file. When a new
revision only changes text or presentation, list the older revisions whose
saved games should still load:

```json
"compatible_revisions": ["<sha256 of the previous .story.json file>"]
```

At most 16 lowercase hexadecimal digests, unique, format v3 only (`fmt
--upgrade` first). Get the digest with `sha256sum old.story.json` before
replacing the file. A slot written by a listed revision is shown as
compatible and is re-validated against the new pack when loaded: if its
state no longer fits (an unknown node, item or object, a changed variable
set or bound), loading fails with `slot_incompatible` and nothing changes.
Nothing is migrated, so do not list a revision whose state shape you changed.

## Lua scripts (format v3, optional)

Proposed in [ADR 0034](../decisions/0034-lua-scripts-in-storypacks.md).
`tiamud-story` validates, tests and plays packs with scripts. The server
plays them only when the operator starts it with `TIAMUD_STORY_SCRIPTS=1`
(off by default; without it a scripted pack in the catalog **stops the
server at startup**), compiling every script while the catalog loads. In a
session a failing script, an exhausted quota or the per-input time budget
(100 ms by default for all calls of one input) shows players only a generic
error and leaves the game unchanged; the author detail (`id:line`) goes to
the server log, and three failures in a row close the instance after
autosaving its last good state. See
[`single-host.md`](../operations/single-host.md#story-sessions). Prefer the
closed DSL; reach for a script only for a rule the DSL cannot express, such
as the custom combat round of the sample
[`duelo-en-el-paso.story.json`](../../crates/story-script/fixtures/duelo-en-el-paso.story.json).

A script is a **pure function**. It reads a copy of the state and either
answers a yes/no question or *emits intents*; it never changes the story
itself. Every intent is an ordinary effect, checked against what the script
declared before it is applied.

```json
"scripts": [
  {
    "id": "asalto",
    "kind": "effect",
    "source": "function run(story, args)\n  ...\nend\n",
    "writes": ["vigor", "vigor_bandido", "ronda"],
    "goto": ["victoria", "derrota"],
    "score": true
  },
  {
    "id": "puede_huir",
    "kind": "condition",
    "source": "function check(story, args)\n  return story.vars.vigor >= 4\nend\n"
  }
]
```

Use them like any condition or effect, with up to 8 constant integer or
boolean arguments:

```json
{ "script": { "id": "asalto", "args": { "arma": 1 } } }
```

- **Condition scripts** (`kind: "condition"`) define `check(story, args)` and
  must return a boolean. They take no grants and cannot emit anything.
- **Effect scripts** (`kind: "effect"`) define `run(story, args)` and emit
  intents with `fx`. **Grants** limit what they may touch: `writes`
  (variables for `fx.set`/`fx.add`), `items` (`fx.give`/`fx.take`), `goto`
  (`fx.go`), `objects` (`fx.move`/`fx.set_object`), `score: true`
  (`fx.score`) and `ends: true` (`fx.finish`). An intent outside the grants,
  of the wrong type or out of range fails the step.

The `story` argument is a read-only copy: `story.node`, `story.turn`,
`story.score`, `story.vars` (id → integer or boolean), `story.items` and
`story.visited` (id → `true`) and, in parser worlds, `story.objects` (id →
`at`, `open`, `locked`, `lit`, `worn`; `at` is `carried`, `nowhere`,
`room:<node>` or `in:<object>`). Writing to it changes nothing.

| Lua helper | Intent |
| --- | --- |
| `fx.set(var, n_or_bool)` / `fx.add(var, n)` | `set` / `add` (integers clamp to the declared bounds) |
| `fx.give(item)` / `fx.take(item)` | `give` / `take` |
| `fx.go(node)` | `goto` (`goto` is a Lua keyword) |
| `fx.score(n)` | `score` |
| `fx.finish(id, "victory" \| "defeat" \| "neutral")` | `end` |
| `fx.move(obj, "carried" \| "nowhere" \| "room:<node>" \| "in:<object>")` | `move_object` |
| `fx.set_object(obj, "open" \| "locked" \| "lit" \| "worn", bool)` | `set_object` |
| `dice(count, sides)`, `tiamud.random(n)` | seeded dice: `1..=n` |

Intents apply in order after `run` returns, exactly like authored effects
and against the same 64-effect budget; `roll`, `if`, `cue` and nested
`script` cannot be emitted. Read `story` for decisions: it does not reflect
your own earlier intents.

**Determinism and sandbox.** Every call runs in a fresh Lua 5.4 VM with no
files, network, OS, clock, `load`, `require`, `print`, `next`,
`string.format`, pattern functions or `math.random`. `pairs` iterates in
sorted key order and `tostring` of a table returns `"table"`. Randomness
comes only from `dice`/`tiamud.random`: effect scripts are seeded from the
story's RNG (so the same seed and inputs replay exactly, and `undo` restores
it), condition scripts from the state without advancing it. Quotas per call:
64 KiB of source, 2 MiB of memory, 100 000 instructions, 25 ms, 32 intents of
at most 1 KiB per field; at most 16 script calls per step or view.

**Failures.** A compile error, Lua error, exhausted quota, non-boolean
`check`, malformed or ungranted intent, or exceeded call budget fails the
whole step and leaves the state untouched. `tiamud-story validate` compiles
every script (without calling it) and reports errors at
`scripts[i].source` with the script id and line (`asalto:2: …`); `play` and
`test` print the same located message. A server would show players only a
generic error.

**Lints.** The lints cannot look inside Lua: they assume an effect script may
change everything it is granted, count its `goto` grants as exits and
`ends: true` as a possible ending, and report scripts nothing references.

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
| Lexicons per world / pack verbs per lexicon | 4 / 32 |
| Objects / rules | 128 / 512 |
| Exits per room / extra rooms of one fixed object | 16 / 16 |
| Nouns, adjectives per object and language | 16 / 16 (one word, 32 bytes folded) |
| Object name | 128 bytes per locale |
| Library message override | 512 bytes per locale |
| Transcript replies per input / stored parser question | 256 / 1024 bytes |
| Answers chained to one parser question | 4 |
| Objects one `todo` command acts on | 32 |
| Player line | 256 bytes, 16 words, 4 commands (parser limits) |
| Test script file / steps | 256 KiB / 4096 |
| Interactive input line | 256 bytes (longer lines are discarded whole) |
| Lua scripts / source per script / all sources | 32 / 64 KiB / 256 KiB |
| Parser prompt | 64 bytes per locale, one line |
| Compatible revisions | 16 SHA-256 digests |
| Story media map: audio assets / images / alt text | 256 / 256 / 1–16 locales of 512 bytes |
| Script grants per list / arguments per reference | 64 / 8 |
| Script calls per step or view / intents per call | 16 / 32 |

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

- For format v3 the core validator rejects unknown objects, rooms, keys,
  doors and directions, actions no lexicon produces, words the parser cannot
  use, contradictory properties and containment cycles, and warns about rules
  with neither text nor effects; exits and rule destinations count for
  reachability, so a room no exit reaches is reported as unreachable. The
  linter adds objects that share a room and whose every word also names
  another object there (the parser always has to ask), objects that start
  `nowhere` and no effect moves, locked objects whose key never enters play
  or that have no key, unless an effect unlocks them, and, in multi-lexicon
  worlds, rule actions some lexicon cannot produce.

The summary line states the format version and, for v2 packs, the number of
verbs, menu entries, characters and dialogues; for v3 packs, rooms, objects,
rules and lexicons.

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

Script `format_version: 3` drives parser worlds:

- `{ "text": "coge la lámpara y enciéndela" }` types a line exactly as a
  player would, in the script's optional top-level `language` (default: the
  pack's first lexicon). A typed `deshacer`/`undo` undoes like `"undo"`.
- `expect` also accepts `carried` and `not_carried` (objects held directly),
  `objects` (exact locations, e.g. `{ "medalla": "nowhere" }`), `transcript`
  (substrings the rendered replies of the last input must contain) and
  `question` (a parser question awaits).

See the reviewed
[`faro-victoria.script.json`](../../crates/story-tools/tests/fixtures/faro-victoria.script.json).

Unknown fields, unknown node/variable/item/verb/menu-entry names, numbers
equal to 0, v2 steps in a v1 script, v3 steps in an older script and v3
steps against a pack without a parser world are script errors. The first failed step stops the run with its step number and every
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

In parser worlds (format v3) every line other than `estado`/`state` and
`salir`/`quit`/`q` goes to the story's parser (`--locale` picks the lexicon
language), and its replies appear in the `»` window. The parser's meta
commands are honoured: `deshacer`/`undo` undoes the last line (a line of only
meta commands is not itself undoable), `reiniciar`/`restart` starts over, and
save/restore report that offline play has no slots. A bare number still takes
a gamebook choice when the scene offers one.

### `fmt <pack> [--check | --output <file> [--force]] [--upgrade]`

Prints the canonical form of a valid pack: object keys sorted, two-space
indentation, trailing newline, optional fields omitted instead of `null`.
`--check` fails when the file differs from its canonical form; `--output`
writes atomically and refuses to replace an existing file without `--force`.
`--upgrade` rewrites a format v1 or v2 pack as format v3; only
`format_version` changes. The reviewed example fixtures are hand-formatted and are not required
to be canonical.

### `schema`

Prints the published StoryPack v3 JSON Schema (v1 and v2 documents also
satisfy it).
