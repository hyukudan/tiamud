# Load and soak evidence

Status at the v0.2 documentation audit: the harness, CI short profile and
manual 24-hour workflow are implemented, but no successful 24-hour candidate
report is retained. This runbook therefore describes how to obtain acceptance;
it is not itself acceptance evidence.

`scripts/load.mjs` supplies the bounded multi-user workload. The retained
evidence runner, `scripts/soak.mjs`, starts an isolated server or targets an
existing process, delegates gameplay and reconnect checks to that load harness,
and periodically records:

- readiness results;
- Linux process RSS;
- initial and final Prometheus snapshots;
- action latency and protocol failures from the bots;
- initial, final, peak, and total RSS growth;
- post-warmup first/last window medians and linear RSS trend.

The schema-v2 JSON report records the exact duration, concurrency, thresholds,
runner, checked-out commit and tree, clean-tree state, SHA-256/size evidence for
the server binary and all four harness scripts (including the shared evidence
helper), timestamps, and every sample.
It also binds the default WorldPack and public asset tree to deterministic
digests; the asset digest hashes sorted `path\0bytes\0sha256\n` records without
embedding an absolute local path. A run fails for any gameplay/protocol error,
readiness loss, excessive total RSS growth, or a trend that exceeds both the
configured sustained-growth and slope limits.

In `--url` mode the report intentionally records `target.binary = null`: the
runner cannot prove which executable backs an arbitrary existing process. This
mode is useful for diagnostics, but it cannot satisfy candidate acceptance;
the retained 24-hour gate requires `--binary`, a clean Git tree and binary
SHA-256 evidence.

Each bot provisions a distinct account and character. Bootstrap verifies an
idempotent movement request, snapshot synchronization, reconnect persistence,
and fresh opaque view IDs. Long-running rounds move and look in both rooms,
send public speech, periodically attack the visible default NPC from one bot,
recover sequence gaps with `sync.request`, repeat idempotency checks, and force
additional reconnects. This keeps the workload deterministic while exercising
ticks, scripts, persistence, broadcast fan-out, coalescing, and recovery.

Two failure modes remain intentionally focused tests instead of mutations of
the default 24-hour world:

- missing/unreadable and failed-format image assets, plus cancellation of an
  in-flight optional manifest, are covered by `apps/web/src/lib/assets.test.ts`;
- instruction/event quotas and memory limits are covered by the
  `tiamud-scripting` tests `stops_infinite_loops_and_event_chains` and
  `enforces_source_and_memory_bounds`.

Those tests must pass in the same candidate gate. Injecting corrupt assets or
non-terminating content into the default shared world would make load failures
ambiguous and would not improve the retained memory trend.

## Short regression profile

CI runs and retains this 30-second profile. It proves that evidence collection
and failure gates work, but it is not the 24-hour acceptance result:

```sh
node scripts/soak.mjs \
  --binary target/debug/tiamud-server \
  --duration 30s \
  --warmup 5s \
  --sample-interval 5s \
  --users 2 \
  --concurrency 2 \
  --pace-ms 100 \
  --max-rss-growth-mb 96 \
  --report artifacts/soak-short.json
```

## 24-hour acceptance profile

The manual `24-hour soak evidence` workflow uses a dedicated Linux x86_64
self-hosted runner labelled `tiamud-soak`. That runner is intentional: hosted
jobs cannot provide one uninterrupted 24-hour execution window. The workflow
installs the frozen web graph, builds the complete browser
distribution, then compiles the locked workspace binaries with the same
path-remapping release flags used by the release workflow. This prevents the
server's minimal build-time fallback page from being mistaken for the release
candidate. The default dispatch then runs:

```sh
node scripts/soak.mjs \
  --binary target/release/tiamud-server \
  --duration 24h \
  --warmup 30m \
  --sample-interval 1m \
  --users 16 \
  --concurrency 4 \
  --pace-ms 150 \
  --max-rss-growth-mb 96 \
  --max-sustained-growth-mb 16 \
  --max-rss-slope-mb-per-hour 1 \
  --report artifacts/soak-24h.json
```

Before dispatch, ensure the runner has stable power, time synchronization,
sufficient disk, no unrelated workload, Node 24, and the pinned Rust toolchain.
Do not restart or update it during the run. The workflow retains the report for
90 days even on failure.

Acceptance criterion 19 is complete only when a report with
`schemaVersion = 2`, `profile.durationMs = 86400000`, `success = true`, no
readiness/load errors, enough post-warmup samples, and both RSS verdicts passing
has actually completed for the candidate revision. The recorded binary and
harness digests must match the retained candidate. The existence of the script,
workflow, a shorter report, or an in-progress dispatch must never be recorded
as a passed 24-hour soak.

The workflow runs the same deterministic acceptance verifier available for a
local retained report:

```sh
node scripts/verify-soak-report.mjs \
  --binary target/release/tiamud-server \
  artifacts/soak-24h.json
```

It requires the exact 24-hour profile, at least 16 users, concurrency of at
least 4, the documented pace/warmup/sample cadence and no-weaker RSS limits.
The temporary server
must report its canonical isolated SQLite/embedded-WorldPack configuration; the
runner strips inherited `TIAMUD_*` variables before launch. The verifier crosses
monotonic and wall-clock duration, load request/completion, samples, readiness,
RSS analysis, verdicts and zero-error metric deltas, then recomputes the clean
candidate, harness, WorldPack, asset-tree and supplied binary digests against
the current checkout. A short, internally inconsistent or detached report is
rejected. The verifier cannot authenticate an arbitrarily synthesized but
coherent JSON document; retain the original workflow artifact and its run
identity, and attach provenance when release infrastructure supports it.

For a public candidate, archive the successful JSON outside transient CI
storage alongside the release record. Repeat after changes to the actor,
persistence, transport queues, scripting limits, or long-lived session state.
