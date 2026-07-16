# Claude guidance for Tiamud

Read [`AGENTS.md`](AGENTS.md) completely before changing this repository. It is
the canonical cross-agent working contract. This file is a compact Claude Code
entry point and must not be used to bypass the fuller invariants, verification
rules or honest-status boundary in `AGENTS.md`.

## Orientation order

1. `AGENTS.md`
2. `docs/project/PROJECT_MEMORY.md`
3. `docs/project/COMPLETION_MATRIX.md`
4. Relevant accepted ADR and design/specification
5. Current implementation and focused tests

For status conflicts, the completion matrix overrides old delivery logs and
stale implementation-status wording in an ADR. ADRs still govern accepted
architectural decisions. `docs/spec-v0.2.md` is historical Spanish context;
`docs/spec-v0.3.md` describes the current character/media baseline.

## Current state in one page

- Tiamud is the definitive name and `hyukudan/tiamud` is the canonical
  repository. Development is on `main`.
- It is a private pre-release `0.1.0`, not a public production service or
  stable protocol release.
- The pinned Rust toolchain is 1.93.1 (edition 2024, MSRV 1.88). CI uses Node
  24 and pnpm 11.11; web metadata permits pnpm 10+.
- One Rust server process owns one single-writer world actor. Browser and
  Telnet share the same domain actions, persistence and observer projection.
- PostgreSQL schema v8 is recommended for a new public single-host deployment;
  SQLite schema v8 is the no-service local/default and legacy path. One process
  opens exactly one backend and never dual-writes.
- Direct per-session bounded mailboxes route observer projections without the
  old broadcast wake-up amplification. Preserve actor ordering, lag recovery
  and checkpoint rollback.
- WorldPack v3 and strict AreaPack v2 are current. V2/v1/Envy data normalize at
  explicit offline boundaries. The server still loads one complete pack at
  startup; ADR 0005 hot loading/eviction is future architecture.
- The clean v3 character path is end to end: lineage, optional background,
  base/advanced class, global XP/level, class history, talent graphs, point
  pools, atomic promotion, abilities, resources, cooldowns, resistances and
  statuses across authoring, runtime, persistence, browser and Telnet.
- Social gameplay is deliberately narrower than the word `chat`: room-local
  `say`, online `tell`, basic six-member parties and durable moderation exist;
  party chat, general channels, telepathy, immortal ranks/commands, item rarity
  and random loot tables do not.
- Account roles (`Player`, `Moderator`, `Admin`) are assigned authority, not
  character progression. Future lore ranks must remain optional presentation
  over auditable capabilities and never be earned through XP.
- Optional sprite composition is technically wired end to end and
  perception-safe, including NPC attack/hit/defeat cues. The bundled world has
  no reviewed sample sprite set; player sprites, server heal cues and defeat
  tombstones are not implemented.
- Plain text is always complete. Media, ANSI, GMCP richness and terminal
  graphics are optional, bounded and consent-gated.
- Lua and editorial AI are offline/capability-limited boundaries. Runtime AI,
  arbitrary code execution and mandatory cloud services are prohibited.
- Software is `MIT OR Apache-2.0`; original content/docs/media are
  `CC-BY-4.0`. Preserve mixed-work notices and provenance.

The latest retained baseline in project memory reports 409 locked Rust tests,
158 focused server tests, 96 web unit tests, 28 combined Chromium/Firefox
scenarios, a disposable PostgreSQL 18.4 schema-v8/runtime/restart gate, a clean
mixed routing/load profile and 3,081,428 production Telnet-parser fuzz
executions without a failure. Those results bind the recorded 2026-07-16 tree;
rerun the relevant gates after changing it. Do not infer a remote CI pass from
the badge because account billing can stop a GitHub job before any step runs.

## Claude decision rules

- Start by locating the authoritative boundary. Fix domain defects in
  `tiamud-core`, not with transport-specific exceptions.
- Keep browser and Telnet behavior equivalent. A structured browser action is
  not a privileged path around domain validation.
- Apply perception before protocol and presentation. Never expose stable IDs,
  hidden variants or unperceived asset references.
- Preserve the single writer, one selected backend, transactional snapshots and
  exact rollback. Never add dual writes or a second source of truth.
- Bound all untrusted bytes, depth, counts, queues, concurrency, time, memory,
  scripts, assets and retries. Use checked/saturating arithmetic.
- Do not weaken the 16 KiB WebSocket gates or atomic paired recovery to make a
  feature fit. Shed optional presentation or redesign the message.
- Do not infer combat cues, state or authorization from prose.
- Do not add Kubernetes, Redis, microservices, runtime LLMs, remote arbitrary
  media or server-side pixel layout without a superseding ADR and measured
  need.
- Treat published content IDs as durable. Keep `archetype` at compatibility
  boundaries; do not recreate it as runtime identity alongside v3 classes.
- A capacity increase is not a heal/refill. Permanent progression mutations
  must remain atomic, bounded and exactly once.
- For persistence changes, update and test SQLite and PostgreSQL parity unless
  the code is explicitly backend-specific. Review snapshot/migration effects.
- For WorldPack changes, cover schema, validator, v2 normalization, AreaPack
  composition, server loader, reviewed sample data and CLI tests.
- For protocol changes, update Rust serialization, strict TypeScript parsing,
  socket/recovery tests and relevant Playwright/Telnet/GMCP coverage.
- For media changes, use the asset pipeline and content-addressed reviewed
  manifests. Missing or disabled media must leave gameplay unchanged.
- For repeatable/random loot, introduce durable item instances and explicit
  loot rights before tables or rarity. Do not add paid loot-box mechanics to
  the baseline.

## Minimum verification menu

Run focused tests while iterating. Before claiming a repository-wide result,
use the applicable final gates from `AGENTS.md`:

```sh
cargo fmt --all -- --check
cargo test --workspace --locked
cargo clippy --workspace --all-targets --locked -- -D warnings

pnpm --dir apps/web check
pnpm --dir apps/web test
pnpm --dir apps/web build
pnpm --dir apps/web e2e

bash scripts/verify-project-licenses.sh
node scripts/generate-third-party-notices.mjs --check
node --test scripts/verify-soak-report.test.mjs scripts/telnet-bot.test.mjs
```

PostgreSQL tests need an explicit disposable `TIAMUD_TEST_POSTGRES_URL`.
Transport smoke/load tests use Node 24. Never aim destructive tests or the
SQLite-to-PostgreSQL mover at production data.

The current host can run Chromium and Firefox but lacks the native WebKit
GTK/GStreamer/media libraries. Do not report WebKit as passed unless it really
ran. A short soak is diagnostic only; no successful 24-hour report is retained.

## Documentation and hand-off

When a material behavior or status changes:

1. update focused tests and any generated artifact;
2. update `docs/project/COMPLETION_MATRIX.md`;
3. update `docs/project/PROJECT_MEMORY.md`;
4. add or amend the relevant dated project log;
5. update README/spec/design/operations text that users would otherwise read
   incorrectly; and
6. add a new ADR before changing an accepted architecture decision.

Do not rewrite historical logs to make them look current. Do not mark a
configured workflow as a pass. Do not claim hot areas, a public deployment,
manual screen-reader acceptance, a tagged/provenance-verified release or a
24-hour soak until retained evidence proves each statement.

Before handing off, inspect `git diff`, run `git diff --check`, verify links and
commands you changed, and state exactly what was and was not validated.
