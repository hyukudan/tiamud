# Documentation image provenance

These README visuals were created specifically for Tiamud. No historical MUD
artwork, production account data or third-party game asset was used. They are
documentation content under the repository's CC-BY-4.0 terms, to the extent
contributors hold rights in the rendered material. The underlying software
and UI source keep their MIT OR Apache-2.0 terms. See
../../LICENSE-CONTENT.md, ../../LICENSE-CC-BY-4.0 and ../../LICENSE.

## tiamud-logo.svg

- Purpose: provide a reusable Tiamud wordmark for the repository header and
  future project surfaces.
- Dimensions: 1200 x 240 with a transparent background.
- Final SHA-256:
  `30960f18d077a3e4dedc48c669e076e40aca00bc4bedf6e29c8a69564166f76b`.
- Design: original deterministic vector artwork created for Tiamud on
  2026-07-16. Custom ivory letter paths avoid any font dependency; a restrained
  aged-gold rule and blue-glass lantern integrated into the letter I connect
  the mark to Lantern Square and the panoramic hero.
- Accessibility and safety: the SVG has an accessible title and description,
  uses only local paths, gradients and shapes, and contains no scripts,
  `foreignObject`, external resources, embedded raster data or font files.
- Review: parsed as XML, rendered in Chromium at 1200 x 240 and 480 x 96, and
  visually reviewed for light and dark GitHub themes and mobile legibility.

## tiamud-hero.webp

- Purpose: provide an inviting, decorative fantasy hero for the repository
  landing page while establishing Lantern Square as a crossroads into a larger
  shared world. It is concept art, not a gameplay capture.
- Dimensions: 1760 x 652 (2.70:1).
- Generated source: 2060 x 763 RGB PNG, SHA-256
  `c2bb74a4c32d93059158c7b9629c7446b484f909cc6ab853c5bc53fb6d8b4be2`.
- Final SHA-256:
  `c455a736dce2e9c129684cd5d13ea71a364731c6ed83802bb8c61f63d8b542c9`.
- Generation: OpenAI built-in image generation, directed and reviewed by a
  Tiamud project contributor on 2026-07-16. The built-in path did not expose a
  model identifier.
- Reference-image role: `apps/web/artwork/lantern-square-source.png` was
  supplied only as a reference for Tiamud's reviewed palette, atmosphere and
  world language. It was not an edit target, and the prompt explicitly required
  original composition, architecture and object placement rather than a copy.
- Human review: the generated source and encoded WebP were visually inspected
  for composition, responsive center safety, environmental detail, palette,
  traveler scale, unwanted text and prohibited UI, logo, watermark, border and
  horror elements. No corrective generation was needed; the final encoding was
  inspected again for visible compression damage.
- Processing: no crop was applied because the generated 2.70:1 source already
  matched the intended 2.5:1 to 3:1 banner. It was resized without stretching
  and encoded with cwebp 1.3.2 using
  `cwebp -quiet -q 82 -m 6 -pass 10 -resize 1760 0 -metadata none generated-source.png -o docs/images/tiamud-hero.webp`.
  `webpinfo` reports one lossy VP8 chunk and no alpha, animation or metadata
  chunks; the final file is 137,886 bytes.

Prompt summary: create original, extremely wide painterly narrative-game key
art for Tiamud, centered on a sapphire-blue glass lantern at a rain-polished
medieval route junction during blue hour. Show old stone arches and several
roads receding into warm-lit districts and distant horizons, with a few small
traveler silhouettes implying shared exploration. Use deep navy, slate,
charcoal, rain-blue and restrained amber; keep the essential scene and lantern
safe in the central horizontal band for responsive display. Make the mood
mysterious, elegant and inviting rather than horrific. Include no readable
text, title, logo, signage, runes, UI, fake screenshot, watermark, border,
clock tower, close-up characters, monsters, combat, gore, neon treatment or
glossy 3D-render appearance.

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
