# World authoring

This is the current entry point for creating Tiamud content. The runtime consumes
a canonical **WorldPack v10**. Authors can keep it as strict **AreaPack v9**
modules under a **WorldProjectManifest v2** and compose them offline. 6A publishes
a local catalog and immutable bundles; 6B incorporates them on demand; 6C can
replace only the whole generation during a quiescent cutover; 6D internally allows
retiring/rehydrating the same revision of an empty unit. There is no revision
replacement or operational LRU. Areas are not services and are not deployed
independently.

The Rust types, validators, and tests in `crates/world-pack` are the exact schema
authority. These pages explain the supported flow and link the detailed design
without duplicating every field.

## Where to start

1. Read the [single-host flow](workflow.md).
2. Add or modify an [area module](modular-worlds.md).
3. Define reusable prototypes and durable initial instances with the
   [schema guide](../world-authoring.md#item-prototypes-and-instances).
4. Define [progression, abilities, and combat](progression-and-combat.md).
5. Compile and reference [atmospheric media or sprites](media.md).
6. Run all the [validation and publishing gates](validation-and-publishing.md).

Branching gamebooks for the separate narrative ruleset (ADR 0025, accepted) use
**StoryPacks**, not WorldPacks. They are authored and tested offline with
`tiamud-story`; see [StoryPack authoring](story-packs.md). They are not yet
playable on a running server. A standalone Spanish/English interactive-fiction
command parser library exists but is not integrated with StoryPacks yet; see
[story parser](story-parser.md).

The reviewed example contains:

- project: [`worlds/lantern-square.project.json`](../../worlds/lantern-square.project.json);
- local release metadata:
  [`worlds/lantern-square.area-release.json`](../../worlds/lantern-square.area-release.json);
- modules: [`worlds/areas/`](../../worlds/areas/); and
- canonical artifact: [`worlds/lantern-square.world.json`](../../worlds/lantern-square.world.json).

## Fast path

Run the tools offline from the repository root. Output files are not silently
replaced.

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

Do not publish just because a document can be decoded. First review prose,
licenses, alternative text, hidden state, balance, and the semantic diff.

## Catalog, bundles, and local generations (phases 6A–6D)

6A does not replace `compose` or change WorldPack v9, AreaPack v8, or WorldProject
v2. It adds a second local, offline representation:

- a catalog of up to 1 MiB and 256 entries;
- a release manifest of up to 256 KiB; the three JSON envelopes are strict,
  reject recursive duplicate keys, and limit depth to 16;
- a canonical bundle of up to 16 MiB and a sum of entries also of 16 MiB;
- metadata that binds revision, ownership, dependencies, portals, and digests;
- `contentLicense=CC-BY-4.0` for the world, prose, and media, separate from
  `scriptLicense=MIT OR Apache-2.0` for Lua/software;
- explicit `unsigned_local`, which is not remote signing or a redistribution
  license; and
- content-immutable blobs with a local pointer atomically replaced.

`AreaPack.dependsOn` remains the exhaustive authoring declaration. The v1 catalog
derives `hardDependencies` only from cross-references that are not `RoomExit`;
cross-area exits are `portals` and do not fix residency. `softDependencies` is
empty and each portal keeps its cardinal `sourceExit`.

The Rust API calls the envelopes `AreaReleaseManifest`, `AreaBundle`, and
`AreaCatalog`. `build_area_release` builds a `VerifiedAreaRelease` and
`verify_area_release` checks the catalog, bundles, and monolithic fallback. Each
bundle includes a strict `CompiledArea` bound by `compiledAreaSha256`.

```sh
cargo run --locked -p world-pack --bin tiamud-world -- area-publish \
  worlds/lantern-square.project.json \
  --release worlds/lantern-square.area-release.json \
  --registry build/area-registry
```

`area-publish` prints the exact content-addressed path. The reviewed fixture
publishes three bundles and the catalog
`e954be5b7e1d6918a1b87d5405d651f0fc52ee81996ab0c100cfcb1909ed15a6`:

```sh
cargo run --locked -p world-pack --bin tiamud-world -- area-verify \
  --catalog build/area-registry/area-catalogs/71b3f0f3493d91254d4ce1ed60c147e9f0f480ca204f62f05b3fd05d92e6e3f1/e954be5b7e1d6918a1b87d5405d651f0fc52ee81996ab0c100cfcb1909ed15a6.catalog.json \
  --registry build/area-registry
```

The registry uses
`area-catalogs/<world-key>/<catalog-digest>.catalog.json`,
`area-bundles/<world-key>/<area-key>/<bundle-digest>.area.json`,
`area-worlds/<world-key>/<world-digest>.world.json`, and the atomic pointer
`area-active/<world-key>.json`. The keys are the SHA-256 of the exact UTF-8 IDs.
The pointer contains `schema`, `worldId`, `catalogSha256`, and the relative
`catalog` path. "Active" means the selected editorial publication, not the
residency of a bundle in the actor.

Cooperating publishers are serialized through a per-world advisory `fd-lock` and
re-read the active revision inside the lock. Verification prevalidates the
aggregate budget before I/O; an identical blob is reopened and resynced.
`area-verify` is read-only and only accepts the exact published path.

`unsigned_local` assumes a trusted registry and host. The lock does not protect
against a process that ignores it or can mutate the registry; a hostile writer
and its TOCTOU races are out of scope.

6B incorporates bundles on demand and keeps them resident. 6C opens `area-active`
on SIGHUP, verifies the full release, and privately builds the candidate
generation. It only replaces the whole generation with zero sessions and zero
leases; it evicts no one. Compatibility requires identical gameplay, initial
items, NPCs, loot, affixes, areas, ownership, and stable IDs and zero persistent
migration. It may change prose, exits, and validated synchronous presentation/Lua.

During the cutover, the server holds the same cooperative registry lock as
`area-publish`/`area-select` until the durable CAS is resolved.

The active generation lives in SQL v15; the v8 snapshot and pointer advance by CAS
in one transaction. For rollback, select a retained catalog and request the
cutover:

```sh
cargo run --locked -p world-pack --bin tiamud-world -- area-select \
  --catalog <ruta-exacta-del-catalogo-retenido> \
  --registry build/area-registry
sudo systemctl kill --signal=HUP tiamud
```

When upgrading from 6B/v13 with shared recovery or characters, boot v14 first with
the exact G0 catalog and
`TIAMUD_ACK_LEGACY_AREA_GENERATION=<sha256-exacto-G0>`. Verify its durable binding
and remove the variable. The new row is born empty: introducing G1 on that first
boot would accept it as the initial authority without comparing compatibility with
G0. Copy the backend and registry before the change, and publish or select G1 only
after binding G0.

Empty recovery can contain characters. Without a generational binding, shared
recovery, or any character forces the exact G0 ACK; accounts alone with no
characters/world recovery can bind without an ACK. Converting a monolithic
WorldPack with existing authority requires offline migration and is out of 6C
scope. If a commit is ambiguous, SQL is re-read and, without a determinable
G0/G1, readiness stays 503. The ACK is rejected in monolithic mode, with a
binding, without recovery/character, or on a different digest.

6D adds internal retirement/rehydration of the same revision with the v8 snapshot
and SQL v15, but remains partial. There is no revision replacement, state
migrations, LRU, GC, network, compression, or remote signing. The historical 6C
gate passes 774 Rust tests —329 from the server— and the re-audit 0 P1/0 P2.
Disposable PostgreSQL 18 passes 20/20 adapter tests and 7/7 focused server tests,
including the v14 CAS; the smoke binds G0, validates readiness/Web/Telnet, two
resident areas, `noop`, and a clean shutdown. See
[ADR 0005](../decisions/0005-hot-area-lifecycle.md),
[ADR 0020](../decisions/0020-additive-local-area-loading.md), and
[ADR 0021](../decisions/0021-quiescent-area-generation-cutover.md), and see
[ADR 0022](../decisions/0022-reversible-area-unit-retirement.md) for 6D.

The retained 6A gate passes 144/144 `world-pack` tests, including 10/10 CLI, and
707/707 Rust workspace tests, plus clippy, format/diff, MSRV Rust 1.88, E2E,
smoke, cargo-deny, and licenses/notices. It must not be attributed to 6C.

## Authoring principles

- Text and structured state contain all playable facts; media is optional
  presentation.
- Stable namespaced IDs are content identity. Rename them only as an explicit
  migration decision.
- Item content lives in `itemPrototypes`; each authored unit lives in
  `initialItemInstances` with a `room`, `npc_spawn`, `container`, or `staged`
  location. Fungible stacks are derived: do not publish or persist a stack ID or
  quantity, and do not restore the v3 fields `items`, `itemRules`, or
  `lootItemIds`.
- WorldPack v9 and AreaPack v8 accept `transferable`, `character_bound`, and
  `non_transferable`. The binding is runtime state created on first acquisition
  and persisted in the v8 snapshot, not an authoring field. WorldPack v4/AreaPack
  v3 and older entries only normalize to `transferable`.
- The transfer policy covers the whole subtree. A transferable bag with a
  bound/non-transferable descendant cannot be given or dropped. `canGive` is a UI
  aid; the server revalidates ownership, binding, proximity, social policy, and
  both inventories atomically.
- Use the `npc_spawn` location for one-time fixed loot and
  `lootTables`/`lootSources` for repeatable drops. Guaranteed entries precede the
  weighted pools; `nothing` is explicit and each spawn has a source. The worst
  case fixed+repeatable does not exceed 64 roots.
- Tables/sources originate in WorldPack v6/AreaPack v5. Earlier formats normalize
  to empty and reject injected fields. The canonical table digest becomes durable
  provenance; IDs, weights, quantities, and references are strict.
- Seals and affixes originate in WorldPack v7/AreaPack v6 and remain current. A
  seal is unique, consumable, `character_bound`, with no equipment/container/effect
  and references a table that guarantees 1..=16 units and at most 1,024 odds rows.
  It is not an initial instance or a direct NPC source; it comes from a normal
  table and does not produce another seal. `open` consumes an earned seal and
  `reward odds` shows authored results without revealing RNG, identity, or internal
  digests. A pool picks at most one affix without bias for an unsealed unique item.
  Opening, RNG, counters, creation, binding, and capacity commit or roll back
  together. These are not paid boxes, keys, or purchasable rolls; pity, need/greed,
  master-loot, and contribution are out of scope.
- WorldPack v9/AreaPack v8 preserve the v8/v7 communication abilities. Known
  telepathy has a positive cost, cooldown, one online ally, and up to 64 blocked
  rooms. It causes no damage/healing/conditions and does not belong to NPC sets.
  There are up to 32; earlier formats reject the field.
- WorldPack v9/AreaPack v8 add up to 32 closed emotes. Unique lowercase ASCII
  commands, targeting, and templates determine the exact form; only
  `{actor}`/`{target}` are allowed and the worst-case rendering fits in 512 bytes.
  Earlier formats and import normalize to an empty catalog.
- `WorldRuntimeSnapshot` v8 is the durable authority for locations, binding,
  reservations, provenance, rights, affixes, and opening counter. SQLite and
  PostgreSQL v15 store it as opaque `TEXT`; v1–v5 retain 1 MiB and v6–v8, 2 MiB.
  The v6 conversion adds empty affixes and a counter of one. There is no
  loot/escrow mirror table or double write.
- Base/variant titles allow 1..=128 safe UTF-8 bytes; descriptions, 1..=8,192.
  Bytes are counted, not characters.
- An item's `entityMedia` points to the prototype; `roomPerception.hiddenItems`,
  to the durable instance. Do not swap those identities.
- Visibility is resolved on the server. Never send clients a catalog of secret
  rooms, variants, entities, or assets.
- Characters use bounded grants/requirements, not arbitrary progression scripts.
- Lua has declared capabilities and hard quotas; it does not bypass the domain.
- Asset conversion and optional AI remain out of the game.
- An identified person reviews imported, generated, or uploaded material before
  publishing it.

## Detailed references

- [Progression design v3](../design/character-progression-v3.md)
- [Optional compositor design](../design/scene-compositor-v1.md)
- [WorldPack v3 character decision](../decisions/0007-worldpack-v3-character-model.md)
- [Prototype/instance foundation](../decisions/0009-item-prototypes-instances-and-capacity.md)
- [Derived stacks and containers](../decisions/0010-derived-stacks-and-bounded-containers.md)
- [Give and executable policies](../decisions/0011-atomic-give-and-item-transfer-policies.md)
- [Trade reservations](../decisions/0012-confirmed-trade-reservations.md)
- [Loot, rights, and provenance](../decisions/0013-repeatable-loot-rights-and-provenance.md)
- [Seals and affixes](../decisions/0014-earned-sealed-rewards-and-deterministic-affixes.md)
- [Authored telepathy](../decisions/0015-authored-online-telepathy.md)
- [Authored emotes](../decisions/0016-authored-room-emotes.md)
- [Scene compositor](../decisions/0008-optional-scene-compositor.md)
- [Area lifecycle](../decisions/0005-hot-area-lifecycle.md)
- [Additive loading 6B](../decisions/0020-additive-local-area-loading.md)
- [Quiescent cutover 6C](../decisions/0021-quiescent-area-generation-cutover.md)
- [Reversible retirement 6D](../decisions/0022-reversible-area-unit-retirement.md)
- [Asset compiler](../../crates/asset-pipeline/README.md)
- [Lua runtime](../../crates/scripting/README.md)
- [Optional editorial AI](../../crates/editorial-ai/README.md)
- [Historical Envy/Merc importer](../../tools/envy-import/README.md)

The [WorldPack v10/AreaPack v9 guide](../world-authoring.md) is the compact schema
and flow reference. Lantern Square contains three rooms, ten prototypes, nine
initial instances, two tables, one source, one affix, one pool, one direct
telepathy, one emote, two commodities, one quest, and one shop. The 55,264-byte
canonical artifact has SHA-256
`666dda61438a6a1e2b17d6ef0ba825b4d72c7e1b9d122e93284d71dab8ab4092`.
It keeps `tiamud:road-token` and `tiamud:wolf-clasp` as transferable and
demonstrates 3C with the non-transferable `tiamud:rain-writ` in `demo:north-gate`
and the bound fixed loot `tiamud:stormglass-talon` from `tiamud:gate-stalker-1`.
That spawn uses round-robin for a guaranteed token, an earned seal, and one roll:
60% clasp and 40% nothing. The normal clasp and the seal's talon also roll a
`storm-bitten` prefix at 60%, demonstrating that affixes are not exclusive to
seals. The Wanderer receives `tiamud:mind-whisper`; `demo:archive` demonstrates
its authored block. `tiamud:smile` demonstrates optional targeting and
per-perspective templates.
