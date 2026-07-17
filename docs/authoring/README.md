# World authoring

This is the current English entry point for creating Tiamud content. The
runtime consumes one canonical **WorldPack v9**. Authors can maintain that
world as strict **AreaPack v8** modules under a **WorldProjectManifest v2** and
compose them offline. Phase 6A is **implemented** and adds a separate local
catalog-and-bundle publication path for reviewed area revisions. Areas are not
services and are still not loaded independently at runtime.

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
- local area-release metadata:
  [`worlds/lantern-square.area-release.json`](../../worlds/lantern-square.area-release.json);
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

## Local area catalog and bundles (phase 6A, implemented)

Phase 6A does not replace `compose` or change WorldPack v9, AreaPack v8 or
WorldProject v2. It prepares a second, offline-only representation for a later
runtime design:

- one local catalog is bounded to 1 MiB and 256 entries;
- the release manifest is bounded to 256 KiB; all three JSON envelopes are
  strict, reject duplicate keys recursively and cap nesting at depth 16;
- each area bundle is canonical JSON bounded to 16 MiB, and the sum of entry
  sizes is also bounded to 16 MiB;
- release metadata binds one area revision to its ownership, dependencies,
  portals and exact digests;
- `contentLicense=CC-BY-4.0` covers world content, prose and media, while
  `scriptLicense=MIT OR Apache-2.0` separately covers Lua/software;
- `unsigned_local` must be selected explicitly and is not a remote signature
  or a redistribution claim; and
- content-addressed blobs are immutable, while the local current pointer is
  replaced atomically.

`AreaPack.dependsOn` remains the exhaustive authoring declaration that permits
cross-area references. It is not copied verbatim into `hardDependencies`:
catalog v1 derives hard dependencies only from non-`RoomExit` cross-area
references. Cross-area room exits become `portals` and do not pin residency;
`softDependencies` is empty in v1. Each portal retains its cardinal
`sourceExit`.

The Rust API names the strict envelopes `AreaReleaseManifest`,
`AreaBundle` and `AreaCatalog`; `build_area_release` constructs a
`VerifiedAreaRelease`, while `verify_area_release` checks the complete catalog,
bundle set and monolithic WorldPack fallback together. Each bundle embeds a
strict `CompiledArea` and binds it with `compiledAreaSha256`.

```sh
cargo run --locked -p world-pack --bin tiamud-world -- area-publish \
  worlds/lantern-square.project.json \
  --release worlds/lantern-square.area-release.json \
  --registry build/area-registry
```

`area-publish` prints the exact content-addressed catalog path. The reviewed
fixture publishes three bundles and catalog
`e954be5b7e1d6918a1b87d5405d651f0fc52ee81996ab0c100cfcb1909ed15a6`:

```sh
cargo run --locked -p world-pack --bin tiamud-world -- area-verify \
  --catalog build/area-registry/area-catalogs/71b3f0f3493d91254d4ce1ed60c147e9f0f480ca204f62f05b3fd05d92e6e3f1/e954be5b7e1d6918a1b87d5405d651f0fc52ee81996ab0c100cfcb1909ed15a6.catalog.json \
  --registry build/area-registry
```

The registry layout is
`area-catalogs/<world-key>/<catalog-digest>.catalog.json`,
`area-bundles/<world-key>/<area-key>/<bundle-digest>.area.json`,
`area-worlds/<world-key>/<world-digest>.world.json`, and the atomic pointer
`area-active/<world-key>.json`. World and area keys are SHA-256 of the exact
UTF-8 IDs. The pointer records `schema`, `worldId`, `catalogSha256` and the
relative `catalog` path. "Active" here means the locally selected editorial
publication, not a bundle loaded by the world actor.

Cooperating publishers for one world serialize through a per-world advisory
`fd-lock` and re-read the active revision while holding it. Verification
preflights the checked aggregate byte budget before bundle I/O. Reused
identical blobs are reopened and durably resynchronized. `area-verify` is
read-only, does not create a missing registry and accepts only the exact path
printed by publication.

`unsigned_local` assumes a local registry on a trusted host with cooperating
publishers. The advisory lock is not a security boundary against a process
that ignores it or can mutate the registry; hostile-writer TOCTOU races are
outside this trust mode.

Use the existing `compose` workflow above as the authoritative way to build
the one WorldPack loaded by the server. Phase 6A
does not fetch over a network, compress or remotely sign artifacts, and the
server does not load, activate, replace, unload or evict these bundles. Those
are distinct later phases of
[ADR 0005](../decisions/0005-hot-area-lifecycle.md).

The final gate passes 144/144 `world-pack` tests, including 10/10 CLI tests,
and 707/707 Rust workspace tests. Workspace strict clippy, formatting/diff,
Rust 1.88 MSRV check, the three-bundle fixture E2E, smoke, cargo-deny
advisories/sources and license/notices for 308 Rust and 76 web dependencies
also pass. Phase 6A does not change Web, protocol, runtime, snapshots or SQL;
it adds no new Web or PostgreSQL gate claim.

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
- Native WorldPack v9 and AreaPack v8 accept `transferable`,
  `character_bound` and `non_transferable`. Character binding is runtime state,
  not an authoring field: it is created on first acquisition and persisted in
  WorldRuntimeSnapshot v7. Legacy WorldPack v4/AreaPack v3 inputs must remain
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
- Loot tables and sources are native from WorldPack v6/AreaPack v5. Older
  packs normalize to empty collections and reject injected fields instead of
  inferring probabilities. The canonical table digest becomes durable drop
  provenance; IDs, weights, quantities and references are validated strictly.
- Sealed earned rewards and affixes were introduced by WorldPack v7/AreaPack
  v6 and remain valid in v9/v8. A sealed prototype is unique, consumable, `character_bound`, has no
  equipment/container/effect payload and names one table whose minimum and
  maximum guarantee 1..=16 units and expands to at most 1,024 visible odds
  rows. It cannot be an initial instance or a direct NPC `lootSource`, must
  itself be produced by an NPC-sourced normal table and its table cannot output
  another sealed prototype. `open` consumes one earned seal and `reward odds`
  reports exact authored outcomes without exposing RNG state, hidden instance
  identity or provenance digests.
  Affix pools make one unbiased weighted choice per referenced unique,
  non-sealed item; each affix is a reviewed prefix or suffix with bounded
  attack/heal deltas, and an item receives at most one affix. Normal and sealed
  loot use the same rules. Opening, RNG advancement, counters, creation,
  binding and capacity validation commit atomically; failure restores them all.
  These are encounter rewards, not paid boxes, keys or purchasable random
  rolls. Pity, need/greed, master-loot and contribution sharing are not part of
  this contract.
- WorldPack v9/AreaPack v8 preserve the closed communication abilities added
  in WorldPack v8/AreaPack v7. Telepathy is an
  ordinary known ability with a positive resource cost and cooldown, a single
  online ally target and up to 64 unique existing blocked rooms. It cannot
  damage, heal, apply statuses or belong to an NPC ability set. A world may
  define at most 32 such abilities; v7/v6 and older inputs reject rather than
  ignore this later field.
- WorldPack v9/AreaPack v8 add up to 32 closed room emotes. Commands are unique
  bounded lowercase ASCII, targeting determines the exact template set, and
  only the exact `{actor}`/`{target}` placeholders are accepted. Rendered text
  must remain within 512 bytes for worst-case 64-byte names. Older packs and
  `envy-import` normalize to an empty emote catalog rather than inventing one.
- WorldRuntimeSnapshot v7 is the single durable authority for item location,
  binding, trade reservations, loot provenance, ground rights, applied affixes
  and sealed-open counters. SQLite and PostgreSQL schema v13 store it as opaque
  `TEXT`; legacy raw v1-v5 snapshots retain a 1 MiB physical cap while v6 and
  v7 admission and commits use 2 MiB. Strict v6 upgrade adds empty affix state
  and starts the sealed-open counter at one. There is no mirror loot/escrow
  table or dual write.
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
- [Earned sealed rewards and deterministic affixes](../decisions/0014-earned-sealed-rewards-and-deterministic-affixes.md)
- [Authored direct-online telepathy](../decisions/0015-authored-online-telepathy.md)
- [Authored room emotes](../decisions/0016-authored-room-emotes.md)
- [Scene compositor decision](../decisions/0008-optional-scene-compositor.md)
- [Asset compiler](../../crates/asset-pipeline/README.md)
- [Lua runtime](../../crates/scripting/README.md)
- [Optional editorial AI](../../crates/editorial-ai/README.md)
- [Historical Envy/Merc importer](../../tools/envy-import/README.md)

The [Spanish WorldPack v9/AreaPack v8 guide](../world-authoring.md) is the
current compact schema and workflow reference. Its reviewed Lantern Square
example has three rooms, ten item prototypes, nine initial instances, two
loot tables, one loot source, one affix, one affix pool, one authored
direct-online telepathy ability and one authored room emote. The 53,567-byte
canonical artifact has SHA-256
`9a75894a9e2e0c48d20257633f323d52bbe78b9db106de8c497f4784392b5e75`.
It keeps the published `tiamud:road-token` and `tiamud:wolf-clasp` semantics
transferable, and demonstrates phase 3C with the non-transferable consumable
`tiamud:rain-writ` in `demo:north-gate` and the character-bound fixed loot
`tiamud:stormglass-talon` on `tiamud:gate-stalker-1`. That spawn also uses
party round-robin for one guaranteed road token, one earned sealed oath and
one weighted roll: 60% wolf clasp, 40% nothing. Both the normal clasp drop and
the talon yielded by breaking the bound oath make one 60% `storm-bitten`
prefix roll (40% unchanged), demonstrating that affixes are not limited to
sealed rewards. The default Wanderer also receives the resource-costed
`tiamud:mind-whisper`; `demo:archive` demonstrates an authored telepathy block.
`tiamud:smile` demonstrates optional character targeting and exact
observer-specific templates.
