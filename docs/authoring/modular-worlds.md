# Projects, areas and composition

The current modular source format is a **WorldProject manifest v2** containing
strict **AreaPack v2** files. `tiamud-world compose` validates the complete
graph and produces one canonical **WorldPack v3** document.

Use [`worlds/lantern-square.project.json`](../../worlds/lantern-square.project.json)
and [`worlds/areas/square.area.json`](../../worlds/areas/square.area.json) as
reviewed examples.

## Ownership and dependencies

An area owns every room, item, NPC, ability, progression entry, media record,
script and content test it declares. Stable IDs are globally unique across the
composed project.

`dependsOn` must name every other area referenced by structured data. Cycles
are allowed: two areas may contain exits to each other. An undeclared edge,
missing target, duplicate owner or cross-kind ID collision fails composition.
Lua source is opaque to the graph checker and cannot establish a verified
dependency.

```json
{
  "schemaVersion": 2,
  "id": "north-gate",
  "dependsOn": ["square"],
  "rooms": [],
  "npcPrototypes": [],
  "npcSpawns": [],
  "abilities": [],
  "entityMedia": []
}
```

Unknown fields fail closed in modular authoring. Keep an AreaPack focused on a
coherent ownership boundary, not on a deployment boundary.

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

The canonical artifact records area source digests, ownership and audited
cross-area references. Semantically irrelevant source ordering is
canonicalized, making diffs and reproducibility checks useful.

## Direct WorldPack authoring

A monolithic WorldPack v3 remains valid, but AreaPacks are preferable once
multiple builders or regions are involved. Use `tiamud-world build` to validate
and canonicalize a direct document:

```sh
cargo run --locked -p world-pack --bin tiamud-world -- build \
  draft.world.json --output build/worlds/draft.canonical.world.json
```

Do not hand-merge generated artifacts. Change their owning sources, compose a
new candidate and review its semantic diff.
