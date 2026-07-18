# Projects, areas, and composition

The current modular source is a **WorldProject v2** manifest with strict
**AreaPack v8** files. `tiamud-world compose` validates the full graph and
produces a canonical **WorldPack v9** document.

Use [`worlds/lantern-square.project.json`](../../worlds/lantern-square.project.json)
and [`worlds/areas/square.area.json`](../../worlds/areas/square.area.json) as
reviewed examples.

## Ownership and dependencies

An area owns every room, prototype, and initial instance of item, NPC, loot
table and source, affix/pool, ability, emote, progression entry, media, script,
and test that it declares. Stable IDs are globally unique in the composed
project. A prototype/instance pair normalized from legacy may keep the same
string because it occupies distinct typed namespaces.

An item `entityMedia` depends on the area that owns the prototype.
`roomPerception.hiddenItems` depends on the area that owns the concrete initial
instance. This way, every visible unit reuses one presentation without confusing
it with per-instance visibility.

`dependsOn` must name every area referenced by structured data. Cycles are
allowed, for example two reciprocal exits. An undeclared edge, a missing target,
double ownership, or a collision between ID classes prevents composition. Lua
code is opaque to the analyzer and does not create a verifiable dependency.

For loot, cross dependencies include the prototype of each entry, the NPC of a
source, and its table. Table and source have separate ownership. Each spawn
allows a single source, every table must be used, and fixed `npc_spawn`
instances plus the repeatable worst case do not exceed 64 roots per defeat. A
seal adds an edge to its table; `affixPoolId`, to the pool; and each pool row, to
the affix. A sealed table cannot be a direct NPC source. Rooms gated by a
communication ability also create audited edges.

```json
{
  "schemaVersion": 8,
  "id": "north-gate",
  "dependsOn": ["square"],
  "itemPrototypes": [],
  "initialItemInstances": [],
  "rooms": [],
  "npcPrototypes": [],
  "npcSpawns": [],
  "lootTables": [],
  "lootSources": [],
  "affixes": [],
  "affixPools": [],
  "abilities": [],
  "socialEmotes": [],
  "entityMedia": []
}
```

Unknown fields fail closed. Keep an AreaPack as a coherent ownership boundary,
not as a service or shard.

AreaPack v1–v7 normalizes to v8 without inventing emotes; v1–v6 also does not
invent communication, v1–v5 seals/affixes, and v1–v4 loot. Declaring later
fields in an earlier schema is rejected. Native titles and variants allow up to
128 UTF-8-safe bytes; descriptions, up to 8,192.

## Composition

```sh
cargo run --locked -p world-pack --bin tiamud-world -- compose \
  worlds/lantern-square.project.json \
  --output build/worlds/lantern-square.candidate.world.json
```

Composition checks sizes, safe regular paths, local assets, source digests,
declared dependencies, global references, and the final WorldPack. It writes
atomically and does not replace an existing output.

`CompositionReport` records digests, ownership, and cross-references. The flat
artifact does not preserve area identity. Order without semantics is canonicalized
so that diffing and reproducibility are useful.

This is multipack authoring, not sharding. The monolithic path composes
everything offline and loads one WorldPack. The 6A local path publishes bundles;
6B loads areas on demand within a generation; 6C can replace only the complete
generation via private staging, zero sessions/leases, and durable CAS. There is
no per-area deployment, download, LRU, or independent services.

Lantern Square composes three areas into three rooms, ten prototypes, nine
initial instances, two tables, one source, one affix, one pool, one direct
telepathy, and one emote. The 53,567-byte canonical artifact has SHA-256
`9a75894a9e2e0c48d20257633f323d52bbe78b9db106de8c497f4784392b5e75`.

## Direct WorldPack authoring

A monolithic WorldPack v9 is still valid, but AreaPack is preferable with
several builders or regions. `tiamud-world build` validates and canonicalizes a
direct document:

```sh
cargo run --locked -p world-pack --bin tiamud-world -- build \
  draft.world.json --output build/worlds/draft.canonical.world.json
```

Do not hand-mix generated artifacts. Change their owning sources, compose a
candidate, and review the semantic diff.
