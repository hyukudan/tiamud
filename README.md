# Project Envy

> Working title. This is a private, early-stage project and the final name has not been chosen.

Project Envy is an attempt to build a new multiplayer narrative game engine inspired by the spirit and play model of EnvyMUD. It is a clean implementation rather than a continuation of the historical codebase.

The project is **browser-first, but not browser-only**, and **text-complete, but not text-only**. The official web client should make the game approachable and atmospheric, while every rule and piece of actionable information remains available through text. A basic Telnet client must remain a viable way to play.

## Current status

The first runnable walking skeleton is in place. It serves the Svelte browser client, accepts versioned WebSocket commands, exposes the same three-room demo over basic Telnet, and persists character position in SQLite. It is a prototype rather than a stable API or complete game.

## Direction

The first useful release should run comfortably on a single ordinary computer without requiring containers, Kubernetes, Redis, object storage, or a cluster. The intended initial shape is one Rust server process with an embedded web application, local assets, and a database.

Key principles:

- A new Rust game engine, informed by EnvyMUD's strengths without copying its implementation.
- One authoritative world model shared by the browser and Telnet transports.
- A browser-first, responsive interface with semantic text, contextual actions, and accessible controls.
- Text-complete gameplay: atmospheric media must never contain exclusive game information.
- Basic Telnet compatibility, with richer MUD protocols added incrementally.
- Atmospheric illustrations that support the writing instead of attempting to compose every dynamic object or character into a scene.
- Bounded queues, explicit resource limits, and predictable degradation for slow clients.
- A single-host-first deployment that can grow only when measured needs justify it.

## Storage strategy

SQLite is the planned default because it keeps local development, personal worlds, tests, backups, and single-host deployments simple. PostgreSQL is a planned selectable backend behind the same persistence boundary for installations that need its concurrency, operational tooling, or query capabilities.

An installation will use one authoritative database backend. We do not plan to mirror writes between SQLite and PostgreSQL. Portable core data should remain relational; PostgreSQL-specific features such as `jsonb` may be used deliberately for flexible metadata when they provide a clear benefit, without making the game rules depend on arbitrary database queries.

The exact persistence contract will be validated with a vertical prototype before it is treated as stable.

## Multimedia and assets

Illustrations are intended as atmospheric accompaniment to the narrative. The initial offline pipeline targets 640 px and 1280 px derivatives, with AVIF preferred by the browser, WebP as the modern fallback, and JPEG for compatibility. Assets are validated, content-addressed, and served over ordinary HTTP(S). Live image generation, dynamic scene composition, video, terminal graphics, and a large asset service are outside the initial critical path.

## AI policy

Development is supported by AI tools for exploration, review, drafting, and repetitive implementation work. Their output is treated as untrusted until it has been reviewed, tested, and accepted by a human contributor. Design decisions and repository history should make that review visible where it matters.

AI is not a required runtime dependency. Optional lightweight local models may later assist authors with tasks such as draft alternative text, consistency checks, or content review. Such tools should run outside the authoritative game loop, remain opt-in, have strict resource limits, and always require human editorial approval.

## Run the prototype

Prerequisites are Rust 1.93.1, Node.js, and pnpm. Node is only needed to build the browser client.

```sh
pnpm --dir apps/web install --frozen-lockfile
pnpm --dir apps/web build
cargo run -p mud-server
```

Then open `http://127.0.0.1:8080`, or connect a Telnet client to `127.0.0.1:4000`. The server creates `data/mud.db` automatically. Available prototype commands are `look`, `north`, `south`, `up`, and `down`; short forms and `go <direction>` also work.

Audit an Envy area without executing or importing it:

```sh
cargo run -p envy-import -- audit path/to/area.are
```

Compile a reviewed atmospheric source into content-addressed AVIF/WebP/JPEG variants:

```sh
cargo run -p envy-asset-pipeline --bin envy-assets -- build \
  --source artwork/scene.png \
  --alt "A rain-darkened square surrounds a stone fountain." \
  --output data/assets
```

Configuration is intentionally small:

- `MUD_HTTP_ADDR`, default `127.0.0.1:8080`
- `MUD_TELNET_ADDR`, default `127.0.0.1:4000`
- `MUD_DB_PATH`, default `data/mud.db`

Run the current verification suite with:

```sh
pnpm --dir apps/web check
pnpm --dir apps/web test
pnpm --dir apps/web build
cargo fmt --all -- --check
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
node scripts/smoke.mjs
```

## Repository map

- `crates/mud-core`: transport-independent entity graph, actions, and semantic scene views.
- `crates/mud-server`: world actor, SQLite adapter, HTTP/WebSocket server, embedded web assets, and Telnet adapter.
- `crates/world-pack`: portable, versioned world schema and validation.
- `crates/asset-pipeline`: bounded offline AVIF/WebP/JPEG compiler and asset manifests.
- `tools/envy-import`: offline audit and conversion tool for stock Envy 2.2 area structure.
- `apps/web`: responsive Svelte client and the `mud.v1` browser protocol adapter.
- `docs/decisions`: accepted architecture decision records.
- `docs/project`: durable project memory and dated implementation logs.
- `docs/protocol`: versioned wire contracts shared by clients and the server.
- `docs/spec-v0.2.md`: local working specification; intentionally ignored by Git.

## Initial roadmap

1. Define entity identities, time and scheduling, link-dead behavior, persistence boundaries, and the semantic view-event model.
2. Replace temporary guest sessions with accounts and durable character selection.
3. Complete the WorldPack schema, atmospheric asset compiler, and the offline Envy audit/import tool.
4. Implement a small complete game loop: exploration, NPCs, combat, inventory, loot, death, recovery, and reconnection.
5. Add atmospheric scene assets and accessible browser interactions without weakening text completeness.
6. Introduce world-pack tooling and, independently, an importer that helps migrate legally usable Envy-style areas.
7. Validate the single-host alpha with backups, load tests, fuzzing, observability, and a minimal set of richer Telnet/GMCP capabilities.
8. Add optional PostgreSQL support and advanced media only in response to concrete use cases.

## Project boundaries

The initial release deliberately avoids microservices, mandatory containers, Kubernetes, runtime generative AI, dynamic raster composition, a custom public image format, and a bespoke terminal client. These may be reconsidered later, but they are not prerequisites for a good small MUD.

## Legal and content note

Compatibility tools and behavioral inspiration do not imply permission to redistribute historical code, areas, text, or media. Imported content must have a known and compatible license or be supplied by a user who has the necessary rights.
