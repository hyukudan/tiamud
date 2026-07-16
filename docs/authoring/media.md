# Atmospheric media and entity sprites

Media accompanies the prose; it never carries exclusive gameplay information.
The server resolves perception first and sends only media for the scene and
entities the character can already know. Telnet and image-disabled browser
sessions remain complete.

## Atmospheric scenes

Opaque scene art is compiled offline to AVIF-first browser variants with WebP
and JPEG fallbacks. Transparent or explicitly lossless atmospheric sources use
PNG. Rooms reference a reviewed content-addressed manifest, while `roomMedia`
provides localized alternative text and optional audio. `sceneVariants` may
select another authorized manifest for bounded time, weather, quest and
perception conditions.

Read the exact encoder policy and commands in the
[asset pipeline guide](../../crates/asset-pipeline/README.md).

## Entity sprites

`entityMedia` optionally decorates an existing item instance or NPC spawn. It
does not create an entity. The `idle` pose is required; `attack`, `hit` and
`defeated` are optional. Each pose points only to strict same-project
content-addressed WebP-alpha and/or PNG files produced by the entity-sprite
asset profile.

```json
{
  "kind": "npc",
  "subjectId": "demo:gate-stalker-1",
  "decorative": true,
  "footprint": { "widthMilli": 220, "heightMilli": 420 },
  "preferredAnchor": { "xMilli": 700, "yMilli": 850 },
  "priority": 20,
  "layer": "foreground",
  "poses": {
    "idle": {
      "webp": "assets/<sha256>/sprite-idle-640.webp",
      "png": "assets/<sha256>/sprite-idle-640.png"
    },
    "hit": {
      "webp": "assets/<sha256>/sprite-hit-640.webp",
      "png": "assets/<sha256>/sprite-hit-640.png"
    }
  }
}
```

Replace `<sha256>` with the lowercase 64-character bundle ID emitted by
`tiamud-assets`. A decorative sprite must omit `alt`; a non-decorative sprite
requires valid localized alternative text. Coordinates and footprints use
thousandths of the scene, not CSS pixels.

```sh
cargo run --locked -p tiamud-asset-pipeline --bin tiamud-assets -- \
  build --kind entity-sprite --pose idle \
  --source artwork/gate-stalker-idle.png \
  --alt "A hunched gate-stalker watching the road." \
  --output apps/web/public/assets \
  --author "Tiamud contributors" \
  --license "CC-BY-4.0" \
  --provenance "Reviewed original transparent illustration." \
  --reviewed-at "2026-07-16T00:00:00Z"
```

Compile every pose separately and copy the exact emitted paths into the owning
AreaPack. Entity sprites currently use static WebP alpha plus PNG fallback;
alpha AVIF remains deferred. No SVG, remote URL, inline data or animation
script is accepted.

## Browser behavior

The browser uses deterministic responsive placement and may omit decoration
under density, data-saving, accessibility or decode budgets. Semantic entity
lists and native action controls remain present. Cosmetic combat cues may
select only an already-authored pose and never affect combat timing or state.

See the [scene compositor design](../design/scene-compositor-v1.md) for layout,
overflow, motion and accessibility rules.
