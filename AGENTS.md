# Tiamud agent guide

This file applies to the entire repository. It is the working contract for
coding agents and contributors. Keep it aligned with the current code,
accepted decisions and retained evidence.

## Read this first

Before making a material change, read these sources in order:

1. [`docs/project/PROJECT_MEMORY.md`](docs/project/PROJECT_MEMORY.md) for the
   durable technical context and current continuation notes.
2. [`docs/project/COMPLETION_MATRIX.md`](docs/project/COMPLETION_MATRIX.md) for
   authoritative implementation and acceptance status.
3. The relevant accepted ADRs in [`docs/decisions/`](docs/decisions/).
4. [`docs/spec-v0.3.md`](docs/spec-v0.3.md), the relevant design document and
   the implementation itself.
5. Dated records in [`docs/project/log/`](docs/project/log/) only when exact
   historical commands or evidence matter.

Source precedence is deliberate:

- accepted ADRs define architectural decisions;
- code and focused tests prove behavior;
- the completion matrix defines whether a requirement is complete or partial;
- project memory is the durable summary and hand-off;
- dated logs describe a delivery at that time and may be superseded;
- `docs/spec-v0.2.md` is the historical Spanish specification, not the current
  character/media contract.

Do not infer that a feature is implemented merely because an ADR or workflow
describes it. Do not silently change an accepted decision; add or supersede an
ADR and update the matrix, memory and user-facing documentation together.

## Product and repository identity

- The definitive product name is **Tiamud**. Do not reintroduce a working title
  or use the former repository name as product identity.
- The canonical repository is `hyukudan/tiamud`; its default and only active
  development branch is `main`.
- `envy-import` is only the offline historical EnvyMUD/Merc-family compatibility
  tool. Tiamud is a clean implementation inspired by classic MUDs, not a fork
  of their source code.
- The project is pre-release `0.1.0`, currently private, with no public
  production deployment or stable protocol release.
- Software, tests, configuration, scripts and binaries are `MIT OR
  Apache-2.0`. Original prose, documentation, worlds and media are
  `CC-BY-4.0`. Preserve the mixed-work boundary and asset-level provenance.
- Never assume that historical or third-party input is redistributable.
  Compatibility is not a license.

## Current implemented baseline

### Runtime and persistence

- Rust 2024 workspace, pinned toolchain 1.93.1, MSRV 1.88.
- One Axum/Tokio `tiamud-server` process owns one authoritative single-writer
  world actor and embeds the reviewed browser build and Lantern Square pack.
- Browser and Telnet inputs become the same transport-neutral
  `tiamud_core::PlayerAction` and enter the same authorization/domain path.
- Mutations are serialized by the actor. A failed durable commit restores the
  exact in-memory checkpoint; do not weaken this rollback boundary.
- World and social updates use bounded 256-entry per-session mailboxes routed
  directly by `SessionId`. Preserve O(S) observer fan-out, lag recovery and
  cancellation-safe session cleanup.
- PostgreSQL schema v8 is recommended for a new public single-host deployment.
  SQLite schema v8 remains the zero-service local/default and legacy path.
  Exactly one backend is authoritative; there is no dual write.
- The offline `tiamud-migrate-postgres` command moves one current SQLite
  snapshot into an empty PostgreSQL target transactionally.
- Queryable state remains relational. Bounded gameplay/world snapshots are
  opaque JSON stored as `TEXT` in both adapters; do not migrate them to `jsonb`
  without a measured query requirement and an ADR.
- Each committed runtime snapshot records its covered event sequence and
  atomically retains only the covered 10,000-sequence diagnostic suffix.
- Authentication is registered-account based across browser and Telnet.
  Argon2id work, token/session lifetimes, connection caps and all queues are
  bounded. Stored bearer credentials are SHA-256 digests, never raw tokens.

### World, characters and gameplay

- WorldPack v3 is canonical. Native modular authoring uses strict AreaPack v2.
  WorldPack v2, AreaPack v1 and Envy/Merc data exist only at explicit
  normalization/import boundaries.
- The runtime validates and loads one complete WorldPack before opening
  listeners. `TIAMUD_WORLD_PACK` selects an external reviewed pack; otherwise
  the embedded pack is used.
- `tiamud-world compose` provides deterministic offline modular composition.
  Runtime hot loading, replacement and LRU eviction are **not implemented**.
  ADR 0005 is the accepted future design, not a shipped feature.
- The v3 character model is implemented end to end: lineage, optional
  background, base and advanced class, one global XP curve, one active class,
  class history, bounded point pools, ranked talent graphs and atomic
  promotion.
- `archetype` is not canonical runtime identity. Do not add a parallel legacy
  model; normalize it at the boundary.
- Permanent grants and requirements use closed bounded vocabularies. Capacity
  increases never heal/refill; a newly granted resource starts empty.
- Martial, spell and utility abilities share one definition model with bounded
  costs, cooldowns, targets, damage/healing, resistance and statuses. Player
  abilities also observe the one-logical-tick global cooldown.
- NPC ability choice is deterministic and rule based with a basic-attack
  fallback. Runtime generative AI is absent.
- Items/equipment, combat, defeat/recovery, loot, logical time, weather,
  quests, parties, economy, chat and moderation are persistent across both
  backends.
- General use-based training is not implemented. The remaining legacy
  post-defeat `combat` skill increment is sample-world policy debt, not a
  general training system.

### Perception, protocol and clients

- Domain actions produce persistent facts; perception then projects an
  observer-specific scene, player state and events.
- Stable world IDs, secret variants and hidden content stay server-side.
  Browser/Telnet clients receive authorized, session-salted opaque IDs.
- Text contains every playable fact. Images, audio, ANSI, terminal graphics
  and scene composition are optional presentation only.
- The experimental `tiamud.v1` protocol uses monotonic sequence numbers,
  bounded request-result deduplication, heartbeats and fresh authoritative
  recovery. It intentionally has no general event replay.
- A recovery request returns correlated, independently 16 KiB-budgeted
  `sync.scene` and `sync.player` frames. The browser must validate and apply
  the matching pair atomically.
- Every normal outbound WebSocket frame is also bounded to 16 KiB compact
  JSON. Do not bypass frame, depth or queue limits for a new feature.
- An oversized GMCP scene degrades to a bounded minimal structured fallback
  and an observable metric; it must never disappear silently.
- The Svelte 5 browser implements registered entry, the complete v3 character
  sheet, creation, abilities, talents/promotions, inventory/equipment, map,
  quests, party/economy/social functions and persistent accessibility/media
  preferences.
- Visible narrative scrollback is capped at 500 entries and command history at
  100 entries. Preserve reader position and the unseen-entry jump behavior.
- The Service Worker caches only a bounded same-origin shell; API, WebSocket,
  image and audio responses are excluded.
- Telnet is a first-class complete text client. The production byte parser is
  the internal `tiamud-telnet-codec` crate shared with fuzzing. Preserve RFC
  1143 negotiation, TTYPE/MTTS, NAWS, CHARSET, GMCP, MSSP, MCCP2 and opt-in
  rich output behavior.
- Direct Telnet TLS 1.2/1.3 is the fail-closed default. Plaintext is only the
  exact `TIAMUD_TELNET_MODE=plaintext` development/private exception and must
  never carry public credentials.

### Media, scripts and optional AI

- Atmospheric assets are offline, immutable and content addressed. Opaque
  scenes prefer AVIF, then WebP/JPEG. Transparent entity sprites use alpha
  WebP plus PNG; alpha AVIF is not yet claimed.
- Media manifests preserve dimensions, hashes, license, provenance,
  classification, review and localized alternative text.
- Optional entity presentation is wired from strict authoring through
  perception-safe server projection to deterministic browser layout. The
  server caps the sidecar at 12 annotations/32 poses; the browser composes at
  most 10 sprites and retains semantic overflow.
- Combat cues are implemented for the closed NPC attack/hit/defeat mapping and
  use projected opaque IDs. They are never inferred from prose. There is no
  reviewed bundled demonstration sprite set, separate player composition,
  server-emitted heal cue or post-removal defeat tombstone.
- Screen-reader, Save-Data and multimedia-off modes fetch no sprite media;
  reduced motion uses a static pose. Sprite layers are non-interactive.
- Lua 5.4 hooks run in fresh capability-only VMs without filesystem, network,
  process, native-module or debug access. Source, memory, instruction,
  wall-time and event quotas are mandatory. A room/hook batch is capped at
  eight modules and 50 ms aggregate wall time.
- Editorial AI is optional, author-only and outside play. Provider `none` is
  the default. Local endpoints must be loopback; explicitly configured remote
  compatible endpoints must be HTTPS. Drafts never publish themselves and
  require normal validation plus human review.

### Retained evidence snapshot

The dated project memory records the following 2026-07-16 baseline. It is
historical evidence for that tree, not permission to skip tests after a change:

- 409 locked Rust workspace tests and strict all-target clippy passed; the
  focused `tiamud-server` suite contained 158 passing tests.
- The final web invocation passed `svelte-check`, 96 unit tests in 16 files,
  a production build and 28 combined Chromium/Firefox Playwright scenarios.
- A disposable PostgreSQL 18.4 gate covered schema-v8 opt-in tests, verified
  SQLite recovery, the public mover, mixed real-server gameplay and durable
  authenticated restart.
- The clean `2b18cfe` routing candidate passed the official 30-second mixed
  WebSocket/real-Telnet profile with 66 rounds, 145 actions, 10/10 readiness,
  zero domain/storage/script/transport/router errors or drops and zero live
  mailboxes at shutdown.
- The production Telnet parser completed a pinned 121-second cargo-fuzz
  campaign: 3,081,428 executions with no crash, timeout, out-of-memory failure
  or failure artifact.
- A genuine authenticated Chromium capture against the real local server
  exposed and closed strict-auth metadata and scene-manifest cancellation
  defects. README hero art is explicitly decorative; the gameplay capture is
  the product-evidence image.

Remote GitHub Actions may still fail before running a step because of account
billing/spending state. Always inspect the actual job; do not replace retained
local evidence with a badge assumption.

## Non-negotiable design constraints

Preserve all of these unless an accepted superseding ADR says otherwise:

1. One ordinary host and one modular server process remain the reference
   deployment. Do not introduce Kubernetes, Redis, mandatory object storage,
   microservices or a second state authority as a shortcut.
2. Keep one authoritative world writer and one selected persistence backend.
   Never add dual writes or weaken checkpoint/rollback consistency.
3. Keep domain behavior transport neutral. Browser structured actions must not
   bypass the same rules used by Telnet text commands.
4. Apply authorization and perception before serialization. Presentation may
   annotate visible facts but must never introduce visibility or stable IDs.
5. Keep plain text complete and accessible. Missing media, disabled colour or
   a basic Telnet client must not lose a playable fact or action.
6. Bound every untrusted input and output: bytes, nesting, counts, queues,
   concurrency, decompression, scripts, assets, retries and time.
7. Prefer deterministic data and pure transformations. Runtime AI, arbitrary
   progression scripts, remote media and prose-derived gameplay are forbidden.
8. Treat published content IDs as durable identity. Renames after a public v3
   release require explicit aliases/tombstones/migrations.
9. Optimize only from evidence. The next known scaling path is dirty-character
   tracking and additive tick deltas/coalescing, while preserving the current
   atomic runtime snapshot boundary.
10. Do not claim current hot areas, public production, current WebKit coverage,
    a manual screen-reader pass, a successful 24-hour soak, a public/tagged
    release or independently verified OIDC provenance.

## Repository map

| Path | Responsibility |
| --- | --- |
| `crates/mud-core/` | Transport-independent domain, actions, progression and perception |
| `crates/mud-server/` | Actor, auth, transports, routing, runtime loading and persistence facade |
| `crates/persistence-sqlite/` | SQLite schema v8 and local storage implementation |
| `crates/persistence-postgres/` | PostgreSQL schema v8, adapter and offline mover |
| `crates/world-pack/` | WorldPack/AreaPack model, validation, composition and authoring CLI |
| `crates/scripting/` | Quota-limited Lua capabilities |
| `crates/asset-pipeline/` | Reviewed deterministic media compiler |
| `crates/telnet-codec/` | Production Telnet byte parser shared with fuzzing |
| `crates/cell-frame/` | Bounded consent-gated terminal cell rendering |
| `crates/editorial-ai/` | Optional offline authoring adapters |
| `crates/tiamud-admin/` | SQLite integrity, migration and recovery CLI |
| `apps/web/` | Official Svelte browser client and Playwright coverage |
| `clients/` | TinTin++ and Mudlet integrations |
| `tools/envy-import/` | Offline conservative historical area importer |
| `worlds/` | Reviewed canonical pack plus modular source areas |
| `scripts/` | Smoke, load, soak, release and license tooling |
| `fuzz/` | Bounded corpora, dictionaries and fuzz targets |
| `docs/` | Specifications, ADRs, designs, operations, memory and evidence |
| `deploy/` | Reviewed Caddy, systemd and environment examples |

## Change workflow

### Before editing

- Inspect `git status` and preserve all existing work. Never discard or rewrite
  unrelated user changes.
- Read the relevant module, tests, ADR/design, project memory and completion
  matrix row before choosing a solution.
- Identify the authoritative boundary first: domain, persistence, perception,
  protocol, presentation or offline authoring.
- Prefer focused, reversible changes. Do not use a client-only patch to mask a
  server/domain defect.

### While editing

- Domain behavior belongs in `tiamud-core`; transports adapt it and must remain
  behaviorally equivalent.
- A persistence change normally requires SQLite and PostgreSQL parity,
  migration/restore coverage and snapshot compatibility review.
- A WorldPack change normally requires schema/validation, v2 normalization,
  AreaPack composition, runtime loading, reviewed sample data and CLI tests.
- A browser protocol change normally requires Rust serialization, strict
  TypeScript parsing, socket/recovery tests and Playwright coverage. Check
  Telnet/GMCP compatibility when the semantic projection changes.
- Media changes must flow through reviewed content-addressed manifests and the
  perception boundary. Never hand clients an arbitrary URL or hidden asset.
- Use saturating/checked arithmetic and explicit caps for authored or runtime
  values. Fail closed on ambiguous authority, schema or security state.
- Update documentation claims with the implementation. When material status
  changes, update the completion matrix, project memory and a dated project log.

### Generated and sensitive files

- Do not hand-edit `THIRD_PARTY_NOTICES.md`; regenerate/check it with
  `scripts/generate-third-party-notices.mjs` and its policy file.
- Preserve all complete license texts and `scripts/verify-project-licenses.sh`
  hashes. A license boundary change requires deliberate legal-document updates.
- Build world/media artifacts with repository tools; keep source and canonical
  output synchronized and verify deterministic bytes.
- Never commit `.env`, databases, operational logs, raw local reports,
  credentials, tokens, private keys or certificates. A retained acceptance
  report must be deliberately sanitized, verified and placed in a reviewed
  documentation/evidence path rather than force-added from `/artifacts`.
  `.gitignore` intentionally protects the raw classes.

## Verification

Use the smallest relevant gate while iterating, then broaden in proportion to
risk. Do not claim a pass for a command that was not run on the final tree.

### Rust

```sh
cargo fmt --all -- --check
cargo test --workspace --locked
cargo clippy --workspace --all-targets --locked -- -D warnings
```

Useful focused packages include `tiamud-core`, `tiamud-server`, `world-pack`,
`tiamud-persistence-sqlite`, `tiamud-persistence-postgres`,
`tiamud-telnet-codec`, `tiamud-asset-pipeline` and `envy-import`.

PostgreSQL opt-in tests require a disposable database and explicit authority:

```sh
TIAMUD_TEST_POSTGRES_URL='postgresql://.../postgres?sslmode=disable' \
  cargo test --locked -p tiamud-persistence-postgres --all-targets
```

Never point destructive/migration tests at production data.

### Browser

```sh
pnpm --dir apps/web install --frozen-lockfile
pnpm --dir apps/web check
pnpm --dir apps/web test
pnpm --dir apps/web build
pnpm --dir apps/web e2e
```

Use Node 24 and pnpm 11.11 when reproducing CI. The package accepts pnpm 10+.
On the current local host, Chromium and Firefox run; WebKit lacks native
GTK/GStreamer/media libraries. That host limitation is not a WebKit pass.

### Project, transport and supply chain

```sh
bash scripts/verify-project-licenses.sh
node scripts/generate-third-party-notices.mjs --check
node --test scripts/verify-soak-report.test.mjs scripts/telnet-bot.test.mjs
node scripts/smoke.mjs
node scripts/load.mjs --users 2 --telnet-users 1 --rounds 2 \
  --concurrency 2 --pace-ms 100
```

The load/soak harness requires Node 24. A short profile is diagnostic evidence,
not proof of the 24-hour acceptance criterion. The interrupted local run is not
a successful retained report.

### World authoring

```sh
mkdir -p build/worlds
cargo run --locked -p world-pack --bin tiamud-world -- compose \
  worlds/lantern-square.project.json \
  --output build/worlds/lantern-square.composed.world.json
cargo run --locked -p world-pack --bin tiamud-world -- validate \
  worlds/lantern-square.world.json
cargo test --locked -p world-pack
```

Composition and publication must remain deterministic, strict,
non-clobbering and safe against path/symlink escape.

### Fuzzing

The production Telnet parser has a checked-in corpus/dictionary and a pinned
bounded workflow. Reproduce it only with the documented toolchain and limits:

```sh
cargo +nightly-2026-07-01 fuzz run telnet_parser -- \
  -dict=fuzz/dictionaries/telnet_parser.dict \
  -max_len=262144 -timeout=5 -rss_limit_mb=1024 -max_total_time=120
```

## Honest current limits

Keep these visible in reviews, hand-offs and user-facing claims:

- no successful retained 24-hour release-candidate soak;
- no current local WebKit run and no manual real screen-reader audit;
- no reviewed bundled sprite demonstration, although the technical bridge is
  implemented and tested;
- no runtime area hot-loading, replacement, checkpoint eviction or LRU;
- no public deployment, public repository decision, stable protocol milestone,
  tagged release or independently verified provenance bundle;
- no general use-based training, content-ID migration framework, multiclassing,
  player scene sprite, server heal cue or defeat tombstone;
- PostgreSQL production backup/TLS/monitoring/recovery remain operator-specific;
- trademark/domain clearance and third-party redistribution review remain
  external legal work.

GitHub workflows are configured, but a workflow definition is not evidence of
a successful run. Inspect the actual run and its steps; account billing may
prevent jobs from starting before repository code executes.

## Completion standard

A task is complete only when the requested behavior exists at every affected
boundary, focused tests cover it, broader gates pass in proportion to risk,
documentation is factual, generated artifacts are synchronized and `git diff`
contains only intentional changes. Report limitations explicitly instead of
turning planned architecture or configured CI into a completion claim.
