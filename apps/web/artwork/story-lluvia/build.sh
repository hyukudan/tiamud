#!/usr/bin/env bash
# Publishes the sample art of `lluvia-de-neon` through the reviewed asset
# pipeline into apps/web/public/assets. Run from the repository root after
# `python3 -I apps/web/artwork/story-lluvia/generate.py apps/web/artwork/story-lluvia/sources`.
set -euo pipefail

sources=apps/web/artwork/story-lluvia/sources
output=apps/web/public/assets
reviewed_at=2026-10-15T00:00:00Z

build() {
  local name=$1 alt=$2 focal_y=$3
  cargo run --quiet --locked -p tiamud-asset-pipeline --bin tiamud-assets -- build \
    --source "$sources/$name.png" \
    --alt "$alt" \
    --output "$output" \
    --focal-y-per-mille "$focal_y" \
    --author "Tiamud project contributors" \
    --license CC-BY-4.0 \
    --provenance "Original placeholder drawn only from code (gradients and geometric shapes) by apps/web/artwork/story-lluvia/generate.py for the lluvia-de-neon StoryPack; no external image, font, photograph or model was used." \
    --generator "apps/web/artwork/story-lluvia/generate.py (deterministic Pillow drawing)" \
    --reviewed-at "$reviewed_at"
}

build fondo-oficina "A detective's office at night: a wide rain-streaked window over dark towers, a pink neon sign across the street and a desk lamp glowing on the desk." 500
build fondo-club "A dim nightclub bar: shelves of bottles in purple light, a cyan neon sign and empty stools along the counter." 500
build retrato-lola-neutral "Portrait silhouette of Lola, a woman with a bob haircut, backlit by pink neon against a teal night." 450
build retrato-damaso-neutral "Portrait silhouette of Dámaso, a broad man in a wide-brimmed hat, backlit by amber light." 450
