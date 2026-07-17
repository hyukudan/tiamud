# Projects, areas and composition

The current modular source format is a **WorldProject manifest v2** containing
strict **AreaPack v5** files. `tiamud-world compose` validates the complete
graph and produces one canonical **WorldPack v6** document.

Use [`worlds/lantern-square.project.json`](../../worlds/lantern-square.project.json)
and [`worlds/areas/square.area.json`](../../worlds/areas/square.area.json) as
reviewed examples.

## Ownership and dependencies

An area owns every room, item prototype, initial item instance, NPC, loot table,
loot source, ability, progression entry, media record, script and content test
it declares. Stable IDs are globally unique across the composed project. A legacy-normalized
prototype/instance pair may preserve the same historical string because the
two IDs occupy distinct typed namespaces.

An item `entityMedia` record depends on the area that owns its prototype.
`roomPerception.hiddenItems` instead depends on the area that owns the concrete
initial instance. This distinction lets all visible units reuse one sprite
definition while visibility remains instance-specific.

`dependsOn` must name every other area referenced by structured data. Cycles
are allowed: two areas may contain exits to each other. An undeclared edge,
missing target, duplicate owner or cross-kind ID collision fails composition.
Lua source is opaque to the graph checker and cannot establish a verified
dependency.

For loot, cross-area dependencies include a table entry's item prototype and a
source's NPC spawn and table. A table and source have separate ownership even
when they are declared in the same area. Each spawn may have at most one
source, every table must be used, and fixed `npc_spawn` instances plus that
table's repeatable worst case may not exceed 64 roots per defeat.

```json
{
  "schemaVersion": 5,
  "id": "north-gate",
  "dependsOn": ["square"],
  "itemPrototypes": [],
  "initialItemInstances": [],
  "rooms": [],
  "npcPrototypes": [],
  "npcSpawns": [],
  "lootTables": [],
  "lootSources": [],
  "abilities": [],
  "entityMedia": []
}
```

Unknown fields fail closed in modular authoring. Keep an AreaPack focused on a
coherent ownership boundary, not on a deployment boundary.

AreaPack v1-v4 normalize to v5 without inventing loot. Declaring
`lootTables` or `lootSources`, even empty ones, in an older schema is rejected
rather than silently reinterpreted. Native base/variant room titles are limited
to 128 safe UTF-8 bytes and base/variant descriptions to 8,192 safe UTF-8 bytes.

## Compose

```sh
cargo run --locked -p world-pack --bin tiamud-world -- compose \
  worlds/lantern-square.project.json \
  --output build/worlds/lantern-square.candidate.world.json
```

Composition checks bounded file sizes, safe regular paths, project-local asset
resolution, source digests, declared dependencies, global references and the
finished WorldPack. It writes atomically and refuses to replace an existing
output.

The `CompositionReport` records area source digests, ownership and audited
cross-area references; the flat runtime artifact does not retain area identity.
Semantically irrelevant source ordering is canonicalized, making diffs and
reproducibility checks useful.

This is a multipack authoring boundary, not runtime sharding. The complete area
graph is read from reviewed local files, composed and validated offline, then
the server loads one flat WorldPack at startup. It does not download missing
areas, resolve dependencies during play, hot-add modules or evict them through
LRU. Runtime hot-loading remains future architecture.

Lantern Square currently composes three areas into three rooms, nine item
prototypes, nine initial instances, one loot table and one loot source. The
canonical 50,498-byte artifact has SHA-256
`3d6c7f40e7a5e56d509b46aeb89439aeea8f5a3168d249dd46609df1fe54c816`.

## Direct WorldPack authoring

A monolithic WorldPack v6 remains valid, but AreaPacks are preferable once
multiple builders or regions are involved. Use `tiamud-world build` to validate
and canonicalize a direct document:

```sh
cargo run --locked -p world-pack --bin tiamud-world -- build \
  draft.world.json --output build/worlds/draft.canonical.world.json
```

Do not hand-merge generated artifacts. Change their owning sources, compose a
new candidate and review its semantic diff.
