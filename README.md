# Tiamud

[![CI](https://github.com/hyukudan/mud/actions/workflows/ci.yml/badge.svg)](https://github.com/hyukudan/mud/actions/workflows/ci.yml)
[![License: MIT OR Apache-2.0](https://img.shields.io/badge/license-MIT%20OR%20Apache--2.0-blue.svg)](#license)
[![Rust](https://img.shields.io/badge/Rust-1.88%2B-b7410e.svg)](https://www.rust-lang.org/)

![Lantern Square in the rain, the reviewed default Tiamud world](apps/web/public/assets/8ade88f0df1adb294333e72d3fa2d0863543dc0263ebe3038208f3c9e415924c/scene-1280.webp)

Tiamud is a clean Rust multiplayer narrative engine inspired by EnvyMUD, Merc
and other classic text MUDs. It is **browser-first, not browser-only**, and
**text-complete, not text-only**: the official web client adds atmosphere and
accessible controls while every playable fact remains available as text and a
basic Telnet client remains viable.

## Current status

The current `0.1.0` package line implements the repository's v0.2 development
specification and is playable end to end on one process and one ordinary host:

- registered accounts, Argon2id passwords, revocable sessions, character
  rosters and one-live-controller leases;
- shared authored rooms/items/NPCs, equipment, skills/effects, deterministic
  combat/loot/progression/recovery, logical time/weather, quests, parties,
  economy, chat and moderation;
- observer-specific perception and opaque IDs before WebSocket/Telnet
  serialization;
- a responsive Svelte client with typed recovery, persistent preferences and
  history, accessible panels, optional image/audio and a bounded Service
  Worker;
- fragment-safe Telnet with RFC 1143/MTTS, GMCP, MSSP, MCCP2, optional links,
  ANSI/CellFrame consumers and direct TLS;
- SQLite by default, optional PostgreSQL, WorldPack/asset/import tools,
  health/metrics, bounded queues and staged shutdown.

The [completion matrix](docs/project/COMPLETION_MATRIX.md) is the source of
truth. External evidence is still open for the 24-hour soak, a manual
screen-reader pass, local WebKit host dependencies, an external PostgreSQL
exercise, a third-party notice inventory, an independently verified tagged
release and Tiamud trademark/domain clearance. These are release acceptance
items, not hidden runtime dependencies.

## Run locally

Prerequisites: Rust 1.88 or newer (CI and release use 1.93.1), Node.js 20.19+
(or 22.12+) and pnpm 10+. Node is needed only to build/test the browser client.

```sh
pnpm --dir apps/web install --frozen-lockfile
pnpm --dir apps/web build
TIAMUD_TELNET_MODE=plaintext cargo run -p tiamud-server
```

Open `http://127.0.0.1:8080` or connect Telnet to `127.0.0.1:4000`.
`TIAMUD_TELNET_MODE=plaintext` is an explicit development/private-network
exception and logs a warning because credentials are unencrypted. Without it,
Telnet defaults to TLS and startup fails closed unless both certificate and key
are valid. Use the TLS configuration in the [single-host
runbook](docs/operations/single-host.md) for any public listener.

The default SQLite database is `data/tiamud.db`; the reviewed Lantern Square
WorldPack is embedded. Common commands include:

```text
look, north/east/south/west/up/down, inventory
take/drop/examine/use/equip/unequip <target>, attack <target>
weather, time, quest ..., party ..., balance/shop/buy/sell ...
say ..., tell ..., who, help, ignore/unignore, block/unblock, report ...
```

Administrative mute/ban/role commands are authorization checked and audited.

## Configuration

- `TIAMUD_HTTP_ADDR` (default `127.0.0.1:8080`)
- `TIAMUD_TELNET_ADDR` (default `127.0.0.1:4000`)
- `TIAMUD_TELNET_MODE`: exactly `tls` (default) or `plaintext`
- `TIAMUD_TELNET_TLS_CERT`, `TIAMUD_TELNET_TLS_KEY`
- `TIAMUD_TELNET_TLS_HANDSHAKE_TIMEOUT_SECONDS` (default `10`)
- `TIAMUD_TELNET_MAX_PENDING_HANDSHAKES` (default `128`)
- `TIAMUD_DB_PATH` (default `data/tiamud.db`) or mutually exclusive
  `TIAMUD_DATABASE_URL` (`sqlite://...`, `postgresql://...` or `postgres://...`)
- `TIAMUD_WORLD_PACK`: optional external authored document; the embedded pack
  is used when unset
- `TIAMUD_TICK_MILLIS` (default `1000`)
- `TIAMUD_AUTH_ATTEMPTS_PER_MINUTE`/`TIAMUD_AUTH_BURST` (defaults `10`/`5`)
- `TIAMUD_COMMANDS_PER_SECOND`/`TIAMUD_COMMAND_BURST` (defaults `12`/`20`)
- `TIAMUD_MAX_SESSIONS_PER_ACCOUNT`/`TIAMUD_MAX_SESSIONS_PER_IP` (defaults
  `4`/`16`, shared by browser and Telnet)
- `RUST_LOG`

`/health/live` checks the HTTP process and `/health/ready` probes the world and
selected storage worker. Prometheus metrics are available at loopback
`/metrics`; the reviewed Caddy configuration deliberately does not expose it.

## Operator and author tools

Check, back up, restore and safely migrate SQLite:

```sh
cargo run -p tiamud-admin -- check \
  --database data/tiamud.db \
  --world-pack worlds/lantern-square.world.json
cargo run -p tiamud-admin -- backup create \
  --database data/tiamud.db --output data/backups/tiamud.sqlite3
cargo run -p tiamud-admin -- backup verify \
  --backup data/backups/tiamud.sqlite3
cargo run -p tiamud-admin -- backup restore \
  --backup data/backups/tiamud.sqlite3 --database data/restored/tiamud.db
cargo run -p tiamud-admin -- migrate \
  --database data/tiamud.db --backup data/backups/pre-migrate.sqlite3
```

Validate/build/diff/preview/test and publish an immutable WorldPack:

```sh
mkdir -p data/worlds data/world-registry
cargo run -p world-pack --bin tiamud-world -- validate worlds/lantern-square.world.json
cargo run -p world-pack --bin tiamud-world -- build \
  worlds/lantern-square.world.json --output data/worlds/lantern-square.world.json
cargo run -p world-pack --bin tiamud-world -- preview \
  worlds/lantern-square.world.json --room demo:lantern-square --format ansi
cargo run -p world-pack --bin tiamud-world -- test worlds/lantern-square.world.json
cargo run -p world-pack --bin tiamud-world -- publish \
  worlds/lantern-square.world.json --registry data/world-registry
```

`tiamud-world rollback --registry ... --world ... --sha256 ...` atomically
reactivates a previously published digest. Feature-gated `ai-draft` and
`ai-review` commands are optional authoring aids; they never modify or publish
a WorldPack and are absent from the runtime server.

Move an offline current SQLite snapshot into an empty PostgreSQL schema (never
dual-write):

```sh
TIAMUD_POSTGRES_URL='postgresql://tiamud@localhost/tiamud?sslmode=require' \
  cargo run -p tiamud-persistence-postgres --bin tiamud-migrate-postgres -- \
  --sqlite data/transfer.sqlite3 --confirm-offline
```

Audit or conservatively convert a historical area without executing programs:

```sh
cargo run -p envy-import -- audit path/to/area.are
cargo run -p envy-import -- convert path/to/area.are --id example > example.world.json
```

Compile a reviewed image directly, or use the stricter
`quarantine` -> `review` -> `publish` workflow for an untrusted source:

```sh
cargo run -p tiamud-asset-pipeline --bin tiamud-assets -- build \
  --source artwork/scene.png \
  --alt "A rain-dark square surrounds a stone fountain." \
  --output data/assets --author "Example Studio" --license CC-BY-4.0 \
  --provenance "Original commissioned artwork" \
  --reviewed-at 2026-07-15T00:00:00Z
```

## Verification

```sh
pnpm --dir apps/web check
pnpm --dir apps/web test
pnpm --dir apps/web build
pnpm --dir apps/web e2e
cargo fmt --all -- --check
cargo test --workspace --locked
cargo clippy --workspace --all-targets --locked -- -D warnings
node scripts/smoke.mjs
node scripts/load.mjs --users 2 --rounds 2 --concurrency 2 --pace-ms 100
```

Playwright requires its browser engines and native dependencies. See
[soak-testing.md](docs/operations/soak-testing.md) before claiming 24-hour
acceptance.

## Repository map

- `crates/mud-core`: transport-independent world, gameplay and perception.
- `crates/mud-server`: actor, transports, storage adapters and embedded web.
- `crates/persistence-sqlite`, `crates/persistence-postgres`: schemas/tools.
- `crates/world-pack`, `crates/scripting`: authoring, variants and Lua quotas.
- `crates/asset-pipeline`, `crates/editorial-ai`, `crates/cell-frame`: optional
  offline/presentation capabilities.
- `crates/tiamud-admin`, `tools/envy-import`: operator/compatibility tools.
- `apps/web`, `clients/`: official browser, TinTin++ and Mudlet consumers.
- `docs/operations`, `deploy/`: single-host operation and release/soak flows.

## Legal and content boundary

Compatibility and inspiration do not grant redistribution rights. Historical
code, areas, help text or media require a known compatible license or rights
from the user supplying them. `envy-import` keeps unknown provenance and
licensing explicit and never turns historical input into project-owned
material. No historical MUD source or area is bundled with Tiamud.

## Contributing

Issues and pull requests are welcome. Keep domain behavior transport-neutral,
preserve text completeness and add focused coverage for new parser, state or
authorization paths. Before opening a pull request, run the commands in
[Verification](#verification); use GitHub private vulnerability reporting or a
security advisory instead of a public issue for sensitive security findings.

Contributions intentionally submitted for inclusion are accepted under the
same software or content terms that apply to the files being changed.

## License

Tiamud software source code, configuration and scripts are available under
your choice of the [MIT License](LICENSE-MIT) or the
[Apache License 2.0](LICENSE-APACHE), expressed as `MIT OR Apache-2.0` in
package metadata.

Original Tiamud narrative content, documentation, artwork and compiled media
are available under
[Creative Commons Attribution 4.0 International](LICENSE-CONTENT.md). Asset
manifests retain per-asset provenance and license metadata. Historical or
third-party material keeps its own license and is never relicensed merely by
being imported.
