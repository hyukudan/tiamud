# Tiamud atmospheric asset pipeline

This crate compiles a reviewed raster illustration into bounded browser
variants. It is an offline publishing tool: it is never called by the game loop
and it contains no AI or network access.

For opaque imagery, the default build emits two widths (640 and 1280 pixels),
preserving aspect ratio, in this order:

1. AVIF, encoded in pure Rust with `ravif`.
2. WebP, encoded with `libwebp` through the optional default
   `webp-encoding` feature.
3. JPEG as a broadly compatible fallback.

Transparent sources, and opaque sources passed with `--lossless`, instead emit
PNG-only variants at both widths so alpha and lossless pixels are preserved.

Every output has a SHA-256 digest. The bundle directory and `assetId` are
derived from the normalized metadata and encoded bytes, so identical inputs and
settings produce the same manifest. AVIF encoding is pinned to one worker so
thread scheduling cannot affect its content address, and published manifests
must remain within the official client's 64 KiB limit. Published bundles are
treated as immutable.

## Usage

```sh
cargo run -p tiamud-asset-pipeline --bin tiamud-assets -- \
  build \
  --source artwork/plaza.png \
  --alt "An empty rain-darkened plaza surrounds a stone fountain." \
  --output public/assets \
  --author "Tiamud contributors" \
  --license "CC-BY-4.0" \
  --provenance "Original illustration commissioned for this scene." \
  --reviewed-at "2026-07-15T00:00:00Z"
```

The command prints the manifest path and identifiers as JSON. A bundle looks
like this:

```text
public/assets/<sha256>/
  manifest.json
  scene-640.avif
  scene-640.webp
  scene-640.jpg
  scene-1280.avif
  scene-1280.webp
  scene-1280.jpg
```

Untrusted uploads have an explicit offline review boundary:

```sh
cargo run -p tiamud-asset-pipeline --bin tiamud-assets -- \
  quarantine --source incoming/upload.png --output private/quarantine

cargo run -p tiamud-asset-pipeline --bin tiamud-assets -- \
  review \
  --record private/quarantine/<sha256>/quarantine.json \
  --reviewer "Tiamud editor" \
  --reviewed-at "2026-07-15T00:00:00Z"

cargo run -p tiamud-asset-pipeline --bin tiamud-assets -- \
  publish \
  --record private/quarantine/<sha256>/quarantine.json \
  --alt "A reviewed scene." \
  --output public/assets \
  --author "Tiamud contributors" \
  --license "CC-BY-4.0" \
  --provenance "Reviewed quarantined upload." \
  --reviewed-at "2026-07-15T00:00:00Z"
```

`quarantine` performs bounded decode and metadata-free PNG re-encoding. The
publisher verifies the content digest, dimensions, transparency flag and exact
human-review timestamp before it can read the sanitized bytes. The game server
never invokes this workflow.

Compile with `--no-default-features` on a platform where `libwebp` cannot be
built. AVIF and JPEG remain mandatory; the manifest then honestly omits WebP.
The tool never shells out to an encoder and never silently substitutes a
different format.

## Deliberate limits

- Sources must be at least 1280 pixels wide; the compiler never upscales.
- Transparent content is published only as PNG; it is never flattened into a
  misleading JPEG fallback.
- PNG, JPEG, and WebP sources are accepted and treated as already oriented
  sRGB imagery. Metadata and color profiles are not copied.
- Default maximums are 64 MiB encoded input, 8192 pixels on either axis,
  40 million decoded pixels, and 2 MiB per emitted variant.
- Alternative text is trimmed, required, single-line, control-character-free,
  and limited to 1024 Unicode scalar values.

These policies can be tightened through the library API, but target widths and
format quality are fixed in schema version 2 to keep builds reproducible. The
manifest records an editorial revision, focal point, computed dominant color,
author, license, provenance, optional generator, content classification, and a
human-review timestamp; those fields participate in the content address.

This is Tiamud's implemented codec-variant pipeline, not the semantic
scene-condition system tracked separately in the project
[completion matrix](../../docs/project/COMPLETION_MATRIX.md).
