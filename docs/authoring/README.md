# World authoring

This is the current English entry point for creating Tiamud content. The
runtime consumes one canonical **WorldPack v5**. Authors can maintain that
world as strict **AreaPack v4** modules under a **WorldProjectManifest v2** and
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
- Native WorldPack v5 and AreaPack v4 accept `transferable`,
  `character_bound` and `non_transferable`. Character binding is runtime state,
  not an authoring field: it is created on first acquisition and persisted in
  WorldRuntimeSnapshot v4. Legacy WorldPack v4/AreaPack v3 inputs must remain
  exactly `transferable`; older packs and Envy/Merc imports also normalize only
  to transferable objects instead of inferring ownership from historical flags.
- Transfer policy applies to the entire contained subtree. A transferable bag
  that contains a bound or non-transferable item cannot be dropped or given.
  The projected `canGive` flag is UI guidance; the server always rechecks
  effective ownership, binding, proximity, social policy and both inventories
  atomically.
- WorldRuntimeSnapshot v4 stores location and mandatory per-instance binding
  as the single durable item authority. SQLite and PostgreSQL remain schema
  v10; neither authoring nor phase 3C adds a mirror table or dual writes.
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
- [Scene compositor decision](../decisions/0008-optional-scene-compositor.md)
- [Asset compiler](../../crates/asset-pipeline/README.md)
- [Lua runtime](../../crates/scripting/README.md)
- [Optional editorial AI](../../crates/editorial-ai/README.md)
- [Historical Envy/Merc importer](../../tools/envy-import/README.md)

The [Spanish WorldPack v5/AreaPack v4 guide](../world-authoring.md) is the
current compact schema and workflow reference. Its reviewed Lantern Square
example has nine item prototypes and ten initial instances. The 49,603-byte
canonical artifact has SHA-256
`f3ab27cb93a2b3c80fe0a9a6ecf8acdd856a45ec1edbda257dd33046a21772c8`.
It keeps the published `tiamud:road-token` and `tiamud:wolf-clasp` semantics
transferable, and demonstrates phase 3C with the non-transferable consumable
`tiamud:rain-writ` in `demo:north-gate` and the character-bound fixed loot
`tiamud:stormglass-talon` on `tiamud:gate-stalker-1`.
