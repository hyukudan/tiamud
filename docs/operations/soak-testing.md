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

The JSON report records the exact duration, concurrency, thresholds, runner,
commit when supplied by CI, timestamps, and every sample. A run fails for any
gameplay/protocol error, readiness loss, excessive total RSS growth, or a trend
that exceeds both the configured sustained-growth and slope limits.

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
jobs cannot provide one uninterrupted 24-hour execution window. The default
dispatch builds the locked release server and runs:

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
`profile.durationMs = 86400000`, `success = true`, no readiness/load errors,
enough post-warmup samples, and both RSS verdicts passing has actually completed
for the candidate revision. The existence of the script, workflow, a shorter
report, or an in-progress dispatch must never be recorded as a passed 24-hour
soak.

For a public candidate, archive the successful JSON outside transient CI
storage alongside the release record. Repeat after changes to the actor,
persistence, transport queues, scripting limits, or long-lived session state.
