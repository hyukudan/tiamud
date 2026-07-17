# World authoring

This is the current English entry point for creating Tiamud content. The
runtime consumes one canonical **WorldPack v6**. Authors can maintain that
world as strict **AreaPack v5** modules under a **WorldProjectManifest v2** and
compose them offline; areas are not services and are not loaded independently
at runtime.

The Rust types, validators and tests in `crates/world-pack` are the exact schema
authority. These pages explain the supported workflow and link to detailed
design documents instead of duplicating every field.

## Start here

1. Read the [single-host workflow](workflow.md).
2. Add or edit an [area module](modular-worlds.md).
3. Define reusable item prototypes and durable initial instances using the
   current [Spanish schema guide](../world-authoring.md#prototipos-e-instancias-de-objeto).
4. Define [character progression, abilities and combat data](progression-and-combat.md).
5. Compile and reference [atmospheric media or entity sprites](media.md).
6. Run every [validation and publication gate](validation-and-publishing.md).

The reviewed example is:

- project: [`worlds/lantern-square.project.json`](../../worlds/lantern-square.project.json);
- area modules: [`worlds/areas/`](../../worlds/areas/);
- canonical artifact: [`worlds/lantern-square.world.json`](../../worlds/lantern-square.world.json).

## Fast path

Run authoring tools offline from the repository root. Output files are not
silently replaced.

```sh
mkdir -p build/worlds

cargo run --locked -p world-pack --bin tiamud-world -- compose \
  worlds/lantern-square.project.json \
  --output build/worlds/lantern-square.candidate.world.json

cargo run --locked -p world-pack --bin tiamud-world -- validate \
  build/worlds/lantern-square.candidate.world.json

cargo run --locked -p world-pack --bin tiamud-world -- test \
  build/worlds/lantern-square.candidate.world.json

cargo run --locked -p world-pack --bin tiamud-world -- preview \
  build/worlds/lantern-square.candidate.world.json \
  --room demo:lantern-square --format text

cargo run --locked -p world-pack --bin tiamud-world -- diff \
  worlds/lantern-square.world.json \
  build/worlds/lantern-square.candidate.world.json
```

Do not publish merely because a document parses. Review prose, licenses,
alternative text, hidden-state behavior, balance and the semantic diff first.

## Authoring principles

- Text and structured state contain all playable information; media is
  optional presentation.
- Stable namespaced IDs are content identity. Rename them only as an explicit
  migration decision.
- Item content belongs in `itemPrototypes`; every authored unit belongs in
  `initialItemInstances` with exactly one `room`, `npc_spawn`, `container` or
  `staged` location. Fungible stacks remain derived from individual instances;
  do not author or persist stack IDs/quantities. Do not restore v3 `items`,
  `itemRules` or `lootItemIds` fields.
- Native WorldPack v6 and AreaPack v5 accept `transferable`,
  `character_bound` and `non_transferable`. Character binding is runtime state,
  not an authoring field: it is created on first acquisition and persisted in
  WorldRuntimeSnapshot v6. Legacy WorldPack v4/AreaPack v3 inputs must remain
  exactly `transferable`; older packs and Envy/Merc imports also normalize only
  to transferable objects instead of inferring ownership from historical flags.
- Transfer policy applies to the entire contained subtree. A transferable bag
  that contains a bound or non-transferable item cannot be dropped or given.
  The projected `canGive` flag is UI guidance; the server always rechecks
  effective ownership, binding, proximity, social policy and both inventories
  atomically.
- Use a `npc_spawn` initial location for one-time fixed loot. Use
  `lootTables` plus `lootSources` for repeatable drops: guaranteed entries run
  first, weighted integer pools may explicitly select `nothing`, and each NPC
  spawn has at most one source. Fixed and repeatable roots share a hard
  worst-case limit of 64 per defeat.
- Loot tables and sources are native only to WorldPack v6/AreaPack v5. Older
  packs normalize to empty collections and reject injected fields instead of
  inferring probabilities. The canonical table digest becomes durable drop
  provenance; IDs, weights, quantities and references are validated strictly.
- WorldRuntimeSnapshot v6 is the single durable authority for item location,
  binding, trade reservations, loot provenance and ground rights. SQLite and
  PostgreSQL schema v11 store it as opaque `TEXT`; legacy raw v1-v5 snapshots
  retain a 1 MiB physical cap while v6 admission and commits use 2 MiB. There
  is no mirror loot/escrow table or dual write.
- Base and variant room titles contain 1..=128 safe UTF-8 bytes and base or
  variant descriptions 1..=8,192; limits count bytes rather than Unicode
  characters.
- Item `entityMedia` targets the reusable prototype, while
  `roomPerception.hiddenItems` targets one durable instance. Do not use an
  instance ID as the media subject or a prototype ID as the hidden-item key.
- Visibility is resolved by the server. Never send a catalog of secret rooms,
  variants, entities or assets to clients.
- Character rules use bounded grants and requirements, not arbitrary
  progression scripts.
- Lua hooks have declared capabilities and hard quotas; they are not a route
  around the domain model.
- Asset conversion and optional AI assistance stay outside the game loop.
- A named human reviews imported, generated and uploaded material before it
  becomes publishable.

## Detailed references

- [Character progression v3 design](../design/character-progression-v3.md)
- [Optional scene compositor design](../design/scene-compositor-v1.md)
- [WorldPack v3 character-model decision](../decisions/0007-worldpack-v3-character-model.md)
- [WorldPack v4 item prototype/instance foundation](../decisions/0009-item-prototypes-instances-and-capacity.md)
- [WorldPack v5 derived stacks and bounded containers](../decisions/0010-derived-stacks-and-bounded-containers.md)
- [Atomic give and executable transfer policies](../decisions/0011-atomic-give-and-item-transfer-policies.md)
- [Confirmed trade reservations](../decisions/0012-confirmed-trade-reservations.md)
- [Repeatable loot, rights and provenance](../decisions/0013-repeatable-loot-rights-and-provenance.md)
- [Scene compositor decision](../decisions/0008-optional-scene-compositor.md)
- [Asset compiler](../../crates/asset-pipeline/README.md)
- [Lua runtime](../../crates/scripting/README.md)
- [Optional editorial AI](../../crates/editorial-ai/README.md)
- [Historical Envy/Merc importer](../../tools/envy-import/README.md)

The [Spanish WorldPack v6/AreaPack v5 guide](../world-authoring.md) is the
current compact schema and workflow reference. Its reviewed Lantern Square
example has three rooms, nine item prototypes, nine initial instances, one
loot table and one loot source. The 50,498-byte
canonical artifact has SHA-256
`3d6c7f40e7a5e56d509b46aeb89439aeea8f5a3168d249dd46609df1fe54c816`.
It keeps the published `tiamud:road-token` and `tiamud:wolf-clasp` semantics
transferable, and demonstrates phase 3C with the non-transferable consumable
`tiamud:rain-writ` in `demo:north-gate` and the character-bound fixed loot
`tiamud:stormglass-talon` on `tiamud:gate-stalker-1`. That spawn also uses
party round-robin for one guaranteed road token and one weighted roll: 60%
wolf clasp, 40% nothing.
