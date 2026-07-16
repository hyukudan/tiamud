# Tiamud technical specification v0.3

Status: pre-release implementation specification, 2026-07-16.

This specification records the current product and architecture decisions for
Tiamud, the browser-first narrative multiplayer engine inspired by EnvyMUD.
It supersedes the character and multimedia planning portions of the historical
Spanish [v0.2 specification](spec-v0.2.md). Accepted decisions and implemented
code remain the final authority where this summary is less specific.

## Product contract

Tiamud is a persistent multiplayer text world with optional illustrated
presentation. One authoritative world state serves the official browser,
Telnet and compatible MUD clients.

The product must remain:

- fully playable as plain text;
- browser-first without making a browser authoritative;
- simple to run on one ordinary machine;
- deterministic and bounded at every untrusted boundary;
- data-driven for world, character and media content;
- accessible when images, colour, audio and motion are disabled;
- compatible with reviewed Envy/Merc area import without executing historical
  programs or assuming redistribution rights; and
- independent of mandatory AI, GPU, Kubernetes, Redis or cloud services.

AI may assist development or offline editorial work. It is optional, bounded,
reviewed and never participates in authoritative play.

## Deployment and persistence

One Rust server process owns one world actor and exposes HTTP/WebSocket plus a
Telnet listener. Static web/media resources may be served by that process or a
same-host reverse proxy. PostgreSQL is recommended for a new public single-host
installation; SQLite is the implemented zero-service local fallback. A server
selects exactly one backend and never dual-writes.

The operational baseline is documented in
[single-host.md](operations/single-host.md). Microservices and orchestration
are non-goals until measured load demonstrates a need.

## World data and authoring

WorldPack v3 is the canonical runtime/authoring target. AreaPack v2 supports
strict modular composition. WorldPack v2, AreaPack v1 and Envy/Merc data are
normalized at an import or composition boundary; the v3 runtime does not keep
legacy vocabulary as its public domain model.

Required authoring properties:

- stable lowercase namespaced IDs;
- strict bounded structures and cross-reference validation;
- deterministic canonical artifacts and digests;
- reviewed content tests and preview tools;
- no online image conversion or generative AI in the game loop;
- source, license, provenance and review metadata for media; and
- server-side resolution of every secret or perception-dependent variant.

The current authoring guide is [world-authoring.md](world-authoring.md); v3
character details are in
[character-progression-v3.md](design/character-progression-v3.md).

## Character model

The v3 composition is:

```text
neutral engine sheet baseline
  + lineage creation grants
  + optional background grants
  + base class creation grants
  + active-class level grants
  + acquired lineage/class talent ranks
  + one-time advanced-class promotion grants
  = permanent character sheet

equipment + active statuses are applied afterwards
```

The neutral sheet baseline contributes no hidden attributes, skills or
resource capacity. Base-class creation grants therefore define the real
starting sheet, with optional lineage/background additions. Character creation
fills health and resources once; later capacity grants preserve current values.

Canonical terms are lineage, background, base class, talent or
specialization, and advanced class. `Race` and `archetype` are import/UI
synonyms only.

The first version uses:

- one global cumulative XP and level curve;
- one active class;
- separate bounded point pools for class and lineage development;
- explicit talent acquisition distinct from class promotion;
- declarative promotion requirements and checked grants;
- immutable lineage/background identity after creation;
- no multiclassing, class XP, reversible promotion or arbitrary permanent
  grant scripts.

Lineage evolution means developing bounded lineage nodes; it does not replace
the character's lineage. Advanced class promotion is an atomic identity
transition that preserves global level and XP.

The initial demonstration catalog is:

| Dimension | Definitions |
| --- | --- |
| Lineages | Mortal, Stonekin, Embermarked |
| Backgrounds | Caravan Scout, Archive Apprentice, City Guard |
| Base classes | Wanderer, Vanguard, Arcanist |
| Advanced classes | Pathfinder, Bastion, Stormcaller, Spellblade |

The demo curve explicitly authors `100 × (level - 1)²` for levels 2–10.
Pathfinder, Bastion and Stormcaller promote from their corresponding base at
level 5; Spellblade permits a cross-discipline Vanguard or Arcanist entry at
level 6 with reachable additional requirements.

The requirement/grant grammar, graphs, invariants and examples are normative
in [character-progression-v3.md](design/character-progression-v3.md) and
[ADR 0007](decisions/0007-worldpack-v3-character-model.md).

## Combat, abilities and progression safety

Martial actions, spells and utility actions use one bounded ability
model with costs, cooldowns, targets, damage/healing, resistances and timed
statuses. NPCs use the same definitions and deterministic rule-based choice,
never runtime generative AI.

`martial`, `spell` and `utility` are the same category vocabulary in WorldPack
authoring, the Rust runtime and the browser wire. The pre-v3 `innate` category
name is not part of the clean model.

Authoring and runtime validate cumulative attribute, skill, resource and
resistance caps. Increasing maximum health is not a heal. Increasing resource
capacity preserves the current amount; a new pool starts empty. Permanent
progression actions apply atomically and exactly once.

The small hard-coded combat-skill and demonstration-quest rewards remain
sample-world policy debt. They do not bypass the bounded progression model,
but should move to authored signals/rewards before the first stable content
contract.

## Semantic scenes and media

A scene is a perception-safe semantic projection. Prose and structured
entities contain every playable fact. The current atmospheric image remains an
optional renderer.

The optional entity compositor adds only presentation annotations for already
visible entities:

- reviewed transparent item/NPC poses;
- normalized footprint, priority and semantic layer;
- client-side deterministic slots and safe-zone avoidance;
- a ten-sprite browser cap with bounded overflow roster;
- short, server-derived NPC attack/hit/defeat cues decorating the same
  projected semantic event (`heal` remains a reserved client value); and
- complete preference gating for screen readers, Save-Data, reduced motion,
  forced colours and multimedia-off play.

Presentation annotations are a web sidecar rather than fields on the core
`SceneView`, preventing accidental GMCP expansion or leakage. The server owns
visibility and selected sprite; asset manifests own geometry; the browser owns
responsive placement. Layout must be a pure function of the current snapshot,
not connection history.

Gameplay never waits for media. Cues decay to a snapshot-reconstructable idle
state and are never replayed after resync. A same-view snapshot retains a cue
only while its opaque NPC remains visible; removal, view change and resync
clear it. Player characters are not separately composited in v1, and defeated
NPCs have no lingering visual tombstone. Full rules and budgets are in
[scene-compositor-v1.md](design/scene-compositor-v1.md) and
[ADR 0008](decisions/0008-optional-scene-compositor.md).

## Media formats

- Opaque atmosphere: AVIF first, WebP and JPEG fallbacks.
- Transparent static entity sprite: WebP alpha plus PNG fallback.
- AVIF alpha: deferred until deterministic encode/decode tests demonstrate a
  useful supported path.
- Transparent/lossless atmosphere: PNG in the current pipeline.
- SVG, HTML, CSS, JavaScript, arbitrary remote hosts and inline `data:` media:
  rejected.

Assets are same-origin, immutable and content-addressed. Sprite poses are
static in v1. Animation is a bounded CSS presentation of a static pose; video,
animated WebP and runtime composition are deferred.

## Protocol

The current `tiamud.v1` WebSocket protocol remains experimental. Clients
receive salted opaque entity IDs, revisions and per-session sequence numbers.
Text and observer projection precede every protocol representation.

The implemented transport safeguards now include:

- a compact-serialized 16 KiB gate on every outbound WebSocket frame;
- correlated `sync.scene` and `sync.player` recovery frames, applied only as a
  matching authoritative pair; and
- oversized GMCP scenes degrading to a minimal structured scene plus an
  explicit bounded overflow notice instead of disappearing silently.

Before the first public compatibility milestone:

- dense scene changes should move toward atomic baseRevision/revision batches.

The implemented cue bridge already decorates the existing projected
`message.append`; it does not create a standalone sequence or infer animation
from prose.

The implemented wire reference is [tiamud-v1.md](protocol/tiamud-v1.md).

## Accessibility and security invariants

- The image never contains exclusive gameplay information.
- Screen-reader mode disables decorative scene and cell art unless explicitly
  requested.
- Player-authored text cannot become ANSI, MXP, HTML, CSS, a URL or an alt
  attribute without context-appropriate escaping and validation.
- Native terminal graphics require explicit capability and consent.
- Heavy assets travel over HTTPS, not GMCP or Telnet byte streams.
- Parsers, subnegotiations, JSON depth, queues, scripts, assets and
  decompression all have explicit limits.
- Hidden entities, secret rooms and unresolved variants are never prefetched
  or announced.

## Implementation status

| Area | Status at v0.3 drafting |
| --- | --- |
| Combat and bounded martial/spell/utility abilities | Implemented and tested baseline |
| Cumulative progression caps and no-refill level-up | Implemented and tested |
| WorldPack v3 schema, validator, normalizer and demo catalog | Implemented and integrated into the server |
| Core lineage/class/talent/promotion runtime and persistence | Implemented and tested |
| Browser and Telnet v3 character creation | Implemented with default/none/explicit background selection |
| Browser progression sheet, talent and promotion actions | Implemented and tested |
| Split WebSocket recovery, outbound frame gate and GMCP overflow fallback | Implemented and tested |
| Browser compositor parser/layout/accessibility | Implemented and tested |
| Transparent sprite asset profile | Implemented with WebP alpha + PNG |
| Entity authoring and perception-safe server sidecar | Implemented and tested; no sample sprite set published yet |
| Combat cue server projection | Implemented and tested for the closed NPC attack/hit/defeat mapping; no player sprite, heal emission or defeat tombstone in v1 |
| Atomic dense-scene delta protocol | Pending |

Documentation and README claims must be refreshed when integration gates move;
an isolated parser, schema or client component is not an end-to-end shipped
feature.

## Acceptance gates

The v0.3 character integration is accepted only when:

1. A new character can select lineage, optional background and base class from
   both browser and text flows.
2. Creation derives the same bounded sheet on every persistence backend.
3. Global XP awards class/lineage points without healing or refilling.
4. Talent ranks and exclusions apply atomically and survive restart.
5. Every bundled advanced class has an actually reachable promotion path.
6. Cross-entry Spellblade is covered from Vanguard and Arcanist fixtures.
7. Invalid/cyclic/over-cap graphs fail publication.
8. WorldPack v2/Envy content normalizes deterministically or fails with an
   actionable diagnostic.

The scene compositor is accepted end-to-end only when:

1. The server projects assets solely for visible entities.
2. Missing/invalid assets leave semantic lists and actions unchanged.
3. Layout is deterministic, responsive and bounded; overflow never hides text.
4. Screen-reader/Save-Data modes fetch no sprite media.
5. Reduced motion shows a static result without movement.
6. Combat cues derive from projected semantic events and vanish on resync.
7. No WebSocket/GMCP budget is exceeded by the added presentation.
8. Plain Telnet remains complete and byte-clean.

## Deferred and rejected work

Deferred until demonstrated necessary: transformations, culture content,
talent respec, class ranks, graph editor, automated balance simulation,
animated entity media, native terminal raster graphics and hot area loading.
The compositor still needs reviewed demonstration sprites; a separately
composited player avatar, server-emitted heal cue and post-removal defeat
tombstone are optional future presentation work rather than v1 requirements.

Rejected as baseline: Kubernetes, microservices, Redis, mandatory object
storage, runtime image generation, runtime LLM decisions, server-side pixel
layout, continuous spatial physics, Canvas/WebGL for ten sprites, general
progression scripting and media blobs over Telnet.
