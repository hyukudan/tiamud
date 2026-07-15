# Envy atmospheric asset pipeline

This crate compiles a reviewed, opaque raster illustration into bounded browser
variants. It is an offline publishing tool: it is never called by the game loop
and it contains no AI or network access.

The default build emits two widths (640 and 1280 pixels), preserving aspect
ratio, in this order:

1. AVIF, encoded in pure Rust with `ravif`.
2. WebP, encoded with `libwebp` through the optional default
   `webp-encoding` feature.
3. JPEG as a broadly compatible fallback.

Every output has a SHA-256 digest. The bundle directory and `assetId` are
derived from the normalized metadata and encoded bytes, so identical inputs and
settings produce the same manifest. Published bundles are treated as immutable.

## Usage

```sh
cargo run --manifest-path crates/asset-pipeline/Cargo.toml -- \
  build \
  --source artwork/plaza.png \
  --alt "An empty rain-darkened plaza surrounds a stone fountain." \
  --output public/assets
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

Compile with `--no-default-features` on a platform where `libwebp` cannot be
built. AVIF and JPEG remain mandatory; the manifest then honestly omits WebP.
The tool never shells out to an encoder and never silently substitutes a
different format.

## Deliberate limits

- Sources must be at least 1280 pixels wide; the compiler never upscales.
- Sources must be opaque. This keeps AVIF, WebP, and JPEG visually equivalent.
- PNG, JPEG, and WebP sources are accepted and treated as already oriented
  sRGB imagery. Metadata and color profiles are not copied.
- Default maximums are 64 MiB encoded input, 8192 pixels on either axis,
  40 million decoded pixels, and 8 MiB per emitted variant.
- Alternative text is trimmed, required, single-line, control-character-free,
  and limited to 1024 Unicode scalar values.

These policies can be tightened through the library API, but target widths and
format quality are fixed in schema version 1 to keep builds reproducible.
