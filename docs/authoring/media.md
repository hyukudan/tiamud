# Atmospheric media and entity sprites

Media accompanies prose; it never carries gameplay-exclusive information. The
server resolves perception first and only sends scene and entity media that the
character can already know. Telnet and image-free browser sessions remain
complete.

## Atmospheric scenes

Opaque scene illustration is compiled offline into browser-preferred AVIF
variants, with WebP and JPEG fallbacks. Transparent or explicitly lossless
atmospheric sources use PNG. Rooms reference a reviewed, content-addressed
manifest; `roomMedia` provides localized alternative text and optional audio.
`sceneVariants` can select another authored manifest under bounded conditions of
time, weather, quest, and perception.

The exact encoding policy and commands are in the
[asset pipeline guide](../../crates/asset-pipeline/README.md).

## Entity sprites

`entityMedia` optionally decorates an existing reusable item prototype or NPC
spawn; it does not create an entity. For `kind: item`, `subjectId` is the
**prototype** ID, not an instance ID: the server reuses the definition for every
instance of that prototype that is already visible. For `kind: npc`, `subjectId`
is the spawn ID. By contrast, `roomPerception.hiddenItems` uses **instance** IDs,
so one unit can stay hidden without hiding the other units of the same prototype.

The `idle` pose is mandatory; `attack`, `hit`, and `defeated` are optional. Each
pose may only point to strict alpha WebP and/or PNG files, from the same project,
content-addressed, and generated with the `entity-sprite` profile.

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

Replace `<sha256>` with the 64-character lowercase-hexadecimal bundle ID emitted
by `tiamud-assets`. A decorative sprite must omit `alt`; a non-decorative one
requires valid localized alternative text. Coordinates and the footprint use
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

Compile each pose separately and copy the exact emitted paths into the owning
AreaPack. Entity sprites currently use static alpha WebP with a PNG fallback;
AVIF with alpha remains deferred. SVG, remote URLs, inline data, and animation
scripts are not accepted.

## Browser behavior

The browser uses deterministic responsive placement and may skip decoration
according to density, data-saving, accessibility, or decoding budgets. Semantic
entity lists and native action controls remain. Cosmetic combat hints can only
pick an already-created pose and never affect combat timing or state.

See the [scene compositor design](../design/scene-compositor-v1.md) for layout,
overflow, motion, and accessibility rules.
