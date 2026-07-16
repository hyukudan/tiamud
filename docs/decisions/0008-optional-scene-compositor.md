# ADR 0008: Optional semantic scene compositor

- Status: Accepted; browser foundation implemented, authoring/server projection pending
- Date: 2026-07-16

## Context

Tiamud currently projects complete prose and visible item/NPC lists plus one
optional atmospheric image. The user experience would benefit from optional
transparent entity art and small RPG-like reactions, but neither Telnet nor
the authoritative world should depend on media, screen size or animation.

Sending server-selected pixel positions would couple content to one viewport.
Allowing arbitrary client animation names, CSS, URLs or scripts would expand
the attack surface and make accessibility inconsistent. A promise that
arbitrary sprites never overlap is also impossible on small screens.

## Decision

- Attach optional visual presentation only to entities that already passed
  per-player perception. Use the same session-opaque entity IDs; never send a
  prototype catalog or hidden pose assets before an entity is visible.
- Keep the atmospheric background and every sprite redundant with prose and
  structured state. Text lists and native controls remain authoritative.
- Let authors provide normalized preferred anchors, footprints, priorities,
  semantic layers and reviewed pose assets. Let the responsive browser choose
  actual placement using a deterministic bounded algorithm.
- Sort by priority and opaque ID, avoid authored safe zones and existing
  footprints, try bounded smaller scales, then place lower-priority entities
  in an overflow roster. Assignment is a pure snapshot function with no sticky
  connection history. Never hide a semantic list entry.
- Use DOM raster images and CSS transforms for the MVP, capped initially at ten
  individually composed entities. Do not require Canvas, WebGL or a GPU.
- Define only bounded presentation cues: attack, hit, heal and defeat, with
  intensity 1–3, an optional predeclared pose and duration no greater than
  800 ms. A cue decorates the same perception-filtered `message.append`
  semantic event, so it cannot create an independent sequence gap. Gameplay
  applies first and never waits for a cue. Do not replay stale cues after
  reconnect or resync.
- Fetch no sprites in screen-reader, Save-Data or multimedia-off modes. Reduced
  motion uses the final static pose without movement. Forced-colours may hide
  all decorative imagery. Sprite layers do not receive pointer events or
  duplicate screen-reader announcements.
- Accept only bounded same-origin content-addressed raster resources. Reject
  SVG, HTML, CSS, JavaScript, `data:` URLs and third-party hosts.
- Keep opaque atmospheric art AVIF-first. Add a tested entity-sprite profile
  that preserves alpha; WebP alpha plus PNG is the minimum. Advertise AVIF
  alpha only after deterministic round-trip tests prove the pipeline.
- Evolve dense scene synchronization toward atomic snapshot/delta/resync
  batches. Cosmetic event IDs are independent of authoritative scene revision.
  Telnet remains complete; a future GMCP package is opt-in.

The detailed layout, cue and budget contract is in
[`scene-compositor-v1.md`](../design/scene-compositor-v1.md).

## Consequences

The browser may resemble a lightweight illustrated RPG without turning the
MUD into a frame-synchronized game server. Placement remains stable across
deltas and responsive across desktop/mobile sizes. Failure to fetch, decode or
place media removes decoration only.

The server still needs reviewed entity authoring, perception-safe projection
and combat-to-cue mapping before the feature is end-to-end. The asset pipeline
needs a transparent sprite profile, and dense scenes need an atomic protocol
revision before raising entity limits. Browser parser/layout readiness alone
must not be advertised as shipped server functionality.

Physics, continuous coordinates, arbitrary animation scripts, runtime image
generation, aggressive prefetch, remote asset hosts and required terminal
graphics are explicit non-goals.
