# Documentation image provenance

These README visuals were created specifically for Tiamud. No historical MUD
artwork, production account data or third-party game asset was used. They are
documentation content under the repository's CC-BY-4.0 terms, to the extent
contributors hold rights in the rendered material. The underlying software
and UI source keep their MIT OR Apache-2.0 terms. See
../../LICENSE-CONTENT.md, ../../LICENSE-CC-BY-4.0 and ../../LICENSE.

## tiamud-client.webp

- Purpose: show the real browser entry surface instead of a conceptual mockup.
- Dimensions: 1440 x 900.
- Final SHA-256: d7d9445ea6bef57821b48f8696780fd28693c7bf6814355d7e63837be5b9c672.
- Capture: Playwright Chromium against the locally served client from commit
  a592eb7ae3df21912b271240c2d22fc4632bc161, dark color scheme, empty account
  fields and a 1440 x 900 viewport on 2026-07-15.
- Processing: encoded from the lossless screenshot with cwebp 1.3.2 at quality
  86; no metadata is retained.

The screen copy and layout are the shipped Svelte WelcomeGate, not generated
UI. The background is the reviewed Lantern Square asset already recorded in
its content-addressed manifest.

## tiamud-gameplay.webp

- Purpose: show a genuine authenticated browser session with the atmospheric
  scene, text-complete prose, character state, world panels, command history
  and command input visible together.
- Dimensions: 1600 x 1000.
- Final SHA-256: 137d985e8ef3a0f445f5b459217b296d0f4fdaffd3a069506ad45493d28c781e.
- Lossless capture SHA-256: 806e63666387b5ae7fe9c6f67e660f15925bd422ac49ffcab40c4e05a76e68f6.
- Capture: Playwright Chromium 1.61.1 against a disposable local Tiamud server
  on 2026-07-16, with a 1600 x 1000 CSS viewport, device scale factor 1, dark
  color scheme, reduced motion and `en-US` locale. The page was scrolled 72 CSS
  pixels to frame both the scene and command input. The source base was commit
  `ebaaeb38e43a8148f5d055df2f0430910d60d454`; the exact pending browser-fix
  patch used for the capture had SHA-256
  `46ac46b6f445c34039c2ea93357507a8efa4e01c8af429b7c8efca247e5a0d8e`.
- Session conditions: real HTTP account login, real character selection and a
  real authenticated `tiamud.v1` WebSocket session backed by temporary SQLite.
  The disposable character `Elara` entered the embedded reviewed Lantern
  Square WorldPack and submitted `look`. Its credentials, database and
  lossless working capture were removed after publication.
- Processing: encoded with cwebp 1.3.2 at quality 86 using
  `cwebp -quiet -q 86 -metadata none /tmp/tiamud-readme-capture-ebaaeb3/tiamud-gameplay.png -o docs/images/tiamud-gameplay.webp`.
  `webpinfo` reports one lossy VP8 chunk, with no alpha, animation or metadata
  chunks.

The interface, prose, state and atmosphere are all output from the shipped
client, server and reviewed world/media. No generated or mocked UI appears in
this capture.

## tiamud-worldpack.webp

- Purpose: explain the concrete three-room Lantern Square WorldPack and its
  media, perception, scene-variant, scripting and content-test extensions.
- Dimensions: 1600 x 901.
- Final SHA-256: c41aa22f2d353d5ec06c0c4d959d07ed5d365268adbfee9dbba2fade3f692da4.
- Generated PNG SHA-256: 93cf39fec7072d9216f9a82748760e8be5cd2eb9c6c3cf4ae57d5e8b2cc878f4.
- Generation: OpenAI image generation, directed and reviewed by Tiamud project
  contributors on 2026-07-15. The existing Lantern Square illustration was a
  palette and world-style reference only.
- Processing: resized from 1672 x 941 and encoded with cwebp 1.3.2 at quality
  84; no metadata is retained.

Prompt summary: depict exactly three connected isometric rooms from the
reviewed pack—North Gate with one gate-stalker, Lantern Square with its blue
lantern, and the underground Cartographer's Archive—surrounded by orderly
cards for JSON, content-addressed media, perception, conditional variants,
bounded Lua and content tests. Show one document and one topology, with no
database, multiprocess architecture, readable labels, logos or watermark.
