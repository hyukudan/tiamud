# Authoring workflow on a single host

Tiamud runs on an ordinary machine. Authoring needs no Kubernetes, no build
farm, and no live content service. The normal flow uses the repository, the Rust
CLI, and a reviewable asset directory; it then publishes immutable artifacts to
the server's local registry.

```text
AreaPack v8 + reviewed assets
              |
              v
      compose and validate
              |
              v
    test, preview, and compare
              |
              v
      6A immutable publication
              |
              v
 6B additive load, 6C cutover, or 6D internal retirement
```

## Recommended loop

1. Work on the canonical `main` branch and edit the minimal owning AreaPack.
2. Compile raster assets offline; record provenance, license, and review.
3. Compose a candidate at a new path.
4. Validate, run embedded tests, and preview the affected scenes.
5. Compare with the reviewed artifact using `diff`.
6. Run the repository tests and request human review.
7. Publish the immutable catalog and bundles with `area-publish`.
8. For a server in catalog mode, send SIGHUP only during a window when it can
   drop to zero sessions; keep the previous catalog for rollback.

The runtime does not compose areas, invoke image encoders, query AI, or download
third-party media. See the
[operations guide](../operations/single-host.md) for service, database, TLS,
backups, and rollback.

AreaPacks are a multipack editorial layout, not deployable services. 6B resolves
local bundles on demand within a generation. 6C opens and verifies the complete
candidate outside the actor and only activates it with zero sessions and zero
leases; a `busy` keeps the previous generation. The cutover does not migrate
data: rules, initial items, NPCs, loot, affixes, areas, ownership, and stable IDs
must be identical. 6D only retires/rehydrates the same revision of an empty unit
internally; there is no revision replacement or operational LRU.

For an explicit rollback, select a retained immutable catalog and request local
reconciliation:

```sh
cargo run --locked -p world-pack --bin tiamud-world -- area-select \
  --catalog <exact-path-of-the-retained-catalog> \
  --registry build/area-registry
sudo systemctl kill --signal=HUP tiamud
```

The server holds the shared cooperative lock with `area-publish`/`area-select`
during the cutover until the CAS resolves; do not run tools that ignore that
lock over the trusted registry.

## Historical content

EnvyMUD/Merc files are offline source material, not runtime data. Use the
[conservative importer](../../tools/envy-import/README.md), inspect every
diagnostic, and resolve provenance and license. Unsupported records remain inert
and imported programs never run.

## Optional AI

AI only assists authoring. The default provider is `none`; local Ollama,
llama.cpp, and explicit compatible HTTPS endpoints are optional. The output is an
untrusted draft, cannot be published, and requires a separate human decision. See
the [editorial AI guide](../../crates/editorial-ai/README.md).

## Runtime boundary

The server loads the embedded WorldPack, `TIAMUD_WORLD_PACK`, or the
`TIAMUD_AREA_REGISTRY` + `TIAMUD_AREA_CATALOG` pair. In catalog mode, SQL (v14+)
binds the durable generation and prevails on restart over `area-active`; SIGHUP
is the only live reconciliation. `WorldRuntimeSnapshot` uses v8 and an activation
confirms snapshot + pointer via CAS before the in-memory swap.

When upgrading a 6B/v13 installation with shared recovery or characters, keep the
exact G0 on the first v14 startup, set
`TIAMUD_ACK_LEGACY_AREA_GENERATION=<exact-G0-sha256>` and verify its binding
before removing the variable and publishing/selecting G1. V13 did not store the
identity; changing both at once would skip the 6C comparison.
Empty recovery may contain characters. Without a binding, shared recovery, or any
character forces the G0 ACK; standalone accounts without characters/world
recovery can bind without an ACK. A monolithic authority does not convert
implicitly. An ambiguous commit re-reads SQL and remains fail-stop with readiness
503 if it cannot determine G0/G1. The ACK fails closed in monolithic mode, with a
binding, without recovery/character, or against a different digest.

The artifact contract stays WorldPack v9/AreaPack v8/Project v2. Raw snapshots
v1–v5 allow 1 MiB; v6–v8, 2 MiB. There is no other loot, inventory, or trade
authority and no double writes. The 6C gate passes 774 Rust tests —329 of them
server— and the re-audit 0 P1/0 P2. Disposable PostgreSQL 18 passes 20/20
adapter tests and 7/7 focused server tests, including the v14 CAS; the smoke test
binds G0 and validates `noop`, readiness, Web, Telnet, and clean shutdown. See
the [PostgreSQL v14/PWA acceptance log](../project/log/2026-07-17-postgresql-v14-and-pwa-acceptance.md).
