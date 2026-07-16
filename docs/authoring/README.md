# World authoring

This is the current English entry point for creating Tiamud content. The
runtime consumes one canonical **WorldPack v3**. Authors can maintain that
world as strict **AreaPack v2** modules and compose them offline; areas are not
services and are not loaded independently at runtime.

The Rust types, validators and tests in `crates/world-pack` are the exact schema
authority. These pages explain the supported workflow and link to detailed
design documents instead of duplicating every field.

## Start here

1. Read the [single-host workflow](workflow.md).
2. Add or edit an [area module](modular-worlds.md).
3. Define [character progression, abilities and combat data](progression-and-combat.md).
4. Compile and reference [atmospheric media or entity sprites](media.md).
5. Run every [validation and publication gate](validation-and-publishing.md).

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
- [WorldPack v3 decision](../decisions/0007-worldpack-v3-character-model.md)
- [Scene compositor decision](../decisions/0008-optional-scene-compositor.md)
- [Asset compiler](../../crates/asset-pipeline/README.md)
- [Lua runtime](../../crates/scripting/README.md)
- [Optional editorial AI](../../crates/editorial-ai/README.md)
- [Historical Envy/Merc importer](../../tools/envy-import/README.md)

The old [Spanish WorldPack v2 guide](../world-authoring.md) is retained only as
a historical implementation record. Do not use its v2 schema examples as the
v3 authoring contract.
