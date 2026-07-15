# Single-host operation

This runbook covers a public Tiamud installation on one Linux host. HTTPS/WSS
terminates at Caddy and the server keeps SQLite, WorldPacks, assets, and backups
on local storage. The public Telnet listener terminates TLS directly in the
server. Telnet credentials must not be exposed over the public Internet until
that listener has a valid certificate; bind explicit plaintext mode only to a
trusted network or firewall it off.

This is a reviewed reference procedure, not evidence that a public production
host has been installed. SQLite is the recommended/default profile. The
optional PostgreSQL profile is documented separately below because its backup
and recovery tools differ.

## Layout

```text
/usr/local/bin/tiamud-server
/usr/local/bin/tiamud-admin
/etc/tiamud/tiamud.env
/var/lib/tiamud/
  data/tiamud.db
  worlds/lantern-square.world.json
  backups/
```

Create a dedicated `tiamud` system user, copy the reviewed
[`deploy/tiamud.service`](../../deploy/tiamud.service) to
`/etc/systemd/system/`, copy
[`deploy/tiamud.env.example`](../../deploy/tiamud.env.example) to
`/etc/tiamud/tiamud.env`, and adjust addresses and paths. Install the example
[`deploy/Caddyfile`](../../deploy/Caddyfile) after replacing the hostname.
Install a separately renewed PEM certificate chain and private key at the paths
configured by `TIAMUD_TELNET_TLS_CERT` and `TIAMUD_TELNET_TLS_KEY`; restrict the
key to the `tiamud` service account and restart after atomic certificate
renewal. Startup fails closed when TLS mode has an invalid identity.

`TIAMUD_TELNET_MODE` defaults to `tls`; there is no implicit downgrade. The
only unencrypted setting is the exact value `plaintext`, which logs a prominent
warning. It exists for development or an explicitly trusted private network:

```sh
TIAMUD_TELNET_MODE=plaintext tiamud-server
```

Do not use that exception for public registration/login or route it through
Caddy: Caddy terminates HTTP(S), not the direct Telnet protocol.

```sh
sudo systemctl daemon-reload
sudo systemctl enable --now tiamud
curl --fail http://127.0.0.1:8080/health/live
curl --fail http://127.0.0.1:8080/health/ready
```

`/health/live` proves that the HTTP process is responding. `/health/ready`
performs bounded probes of both the world actor and SQLite worker and returns
`503` if either cannot answer. Operators must still run `tiamud-admin check`
and the smoke workflow after an update; readiness is not a replacement for
integrity or gameplay verification.

The service writes structured logs to stdout/stderr. `systemd-journald` owns
rotation and retention; configure its host-wide limits to match available disk.

The SQLite environment uses `TIAMUD_DB_PATH`. `TIAMUD_DATABASE_URL` is a
mutually exclusive alternative accepting `sqlite://...`, `postgresql://...` or
`postgres://...`. Startup selects exactly one store and readiness reports its
backend; Tiamud never dual-writes.

## Rate limits and metrics

The environment example configures bounded browser authentication and command
token buckets:

- `TIAMUD_AUTH_ATTEMPTS_PER_MINUTE` and `TIAMUD_AUTH_BURST` apply per source IP
  to browser and Telnet registration and login.
- `TIAMUD_COMMANDS_PER_SECOND` and `TIAMUD_COMMAND_BURST` apply per browser
  WebSocket or entered Telnet connection.
- `TIAMUD_MAX_SESSIONS_PER_ACCOUNT` and `TIAMUD_MAX_SESSIONS_PER_IP` cap
  concurrent entered sessions across browser and Telnet (defaults `4` and
  `16`). Permits are released on disconnect, including cancelled connections.

These application controls complement, but do not replace, host-level firewall
and reverse-proxy connection and request limits.

Prometheus-format metrics are available only from the server's loopback
listener in the reference deployment:

```sh
curl --fail http://127.0.0.1:8080/metrics
```

The reviewed Caddy configuration deliberately returns `404` for public
`/metrics` requests. The endpoint covers connections by WebSocket and Telnet
transport, authenticated sessions, action totals/errors/latency, world-queue
depth, authentication throttling, coalesced output and slow-client disconnects,
dropped output and per-transport output-queue depth, SQLite transactions/
errors/latency and queue depth, ticks/errors/latency, scripts/errors/timeouts/
latency, and resident process memory. Alert at minimum on readiness loss,
database or tick errors, script timeouts, slow-client disconnect or dropped-
output growth, a persistently full world/database/transport queue, and sustained
resident-memory growth.

## Graceful shutdown

`SIGTERM` and `SIGINT` stop new HTTP and Telnet acceptance, notify active
listeners, and place a shutdown barrier behind already accepted world actions.
The reference service allows five seconds for Telnet sessions, ten seconds for
the actor, and five seconds for persistence to drain and close before logging a
timeout. Keep systemd's stop timeout above those internal bounds. A timeout or
forced kill requires an integrity check before returning the instance to
service:

```sh
sudo systemctl stop tiamud
tiamud-admin check \
  --database /var/lib/tiamud/data/tiamud.db \
  --world-pack /var/lib/tiamud/worlds/lantern-square.world.json
```

## Backup and verification

Create backups on a filesystem covered by the host's normal off-machine backup
policy. Every command refuses to overwrite an existing destination.

```sh
tiamud-admin backup create \
  --database /var/lib/tiamud/data/tiamud.db \
  --output /var/lib/tiamud/backups/tiamud-2026-07-15.sqlite3

tiamud-admin backup verify \
  --backup /var/lib/tiamud/backups/tiamud-2026-07-15.sqlite3
```

Exercise restore periodically into a new path, validate it, and remove the test
copy only after the check succeeds:

```sh
tiamud-admin backup restore \
  --backup /var/lib/tiamud/backups/tiamud-2026-07-15.sqlite3 \
  --database /var/lib/tiamud/restore-test/tiamud.db
```

Define retention by recovery requirements rather than available local space.
For a small public host, keep several daily and weekly verified snapshots,
copy them off-host, monitor free space before each backup, and periodically
restore a selected off-host copy into a new path. A backup that has not passed
`backup verify` and a restore exercise is not established recovery evidence.

## Schema migration

Stop the server so no process holds the authoritative database, then run the
shared migrator. A verified safety snapshot is mandatory and is created before
the first schema write.

```sh
sudo systemctl stop tiamud
tiamud-admin migrate \
  --database /var/lib/tiamud/data/tiamud.db \
  --backup /var/lib/tiamud/backups/pre-migrate-2026-07-15.sqlite3
sudo systemctl start tiamud
```

## Optional PostgreSQL profile

PostgreSQL is not needed for the reference single-host installation. Before
choosing it, run the opt-in integration test against a disposable service and
establish ordinary PostgreSQL monitoring, `pg_dump`/restore and point-in-time
recovery appropriate to the host. Tiamud's `backup` subcommands are SQLite
tools and must not be represented as PostgreSQL backup coverage.

To change an existing installation, stop Tiamud, create and verify a current
SQLite snapshot, provision an empty PostgreSQL database, then run the explicit
one-way offline mover. Prefer the environment variable so credentials do not
appear in the process list:

```sh
sudo systemctl stop tiamud
tiamud-admin backup create \
  --database /var/lib/tiamud/data/tiamud.db \
  --output /var/lib/tiamud/backups/pre-postgres.sqlite3
tiamud-admin backup verify \
  --backup /var/lib/tiamud/backups/pre-postgres.sqlite3

TIAMUD_POSTGRES_URL='postgresql://tiamud@db.example/tiamud?sslmode=require' \
  tiamud-migrate-postgres \
  --sqlite /var/lib/tiamud/backups/pre-postgres.sqlite3 \
  --confirm-offline
```

The mover requires the current SQLite schema, refuses a non-empty target and
commits schema plus rows in one PostgreSQL transaction. Configure exactly one
line in the service environment and remove/comment the other:

```text
TIAMUD_DATABASE_URL=postgresql://tiamud@db.example/tiamud?sslmode=require
# TIAMUD_DB_PATH is absent
```

The runtime PostgreSQL connector uses Rustls and the operating-system trust
store; URL `sslmode` remains authoritative, including an explicit local
plaintext choice. Exercise registration, login, movement, combat, social state,
restart and readiness before deleting or archiving the SQLite rollback copy.
The repository's [retained PostgreSQL 18 acceptance
run](../project/log/2026-07-15-postgresql-acceptance.md) proves the adapter,
atomic mover, runtime gameplay and restart against a separate disposable
server process. It does not replace this exercise or establish monitoring,
TLS, backup/restore or point-in-time recovery for a production installation.

## Binary and world update

1. Verify the candidate archive checksum and OIDC-signed provenance as described
   in the [release runbook](releases.md), then extract it into staging.
2. Create and verify a database backup.
3. Save the current binary and WorldPack under a versioned rollback directory.
4. Stop Tiamud, install the new binary and reviewed pack, and run
   `tiamud-admin check`.
5. Start the service and exercise registration, character entry, movement, and
   inventory against the installed listener, for example:

   ```sh
   node scripts/load.mjs --url http://127.0.0.1:8080 \
     --users 1 --rounds 1 --concurrency 1 --pace-ms 100
   ```

   `scripts/smoke.mjs` is useful in the candidate gate, but it starts its own
   temporary debug server and therefore is not proof of the installed service.

If validation or smoke testing fails, stop the service, restore the prior
binary and WorldPack, move the failed database aside, and restore the verified
pre-update snapshot to the authoritative path. The restore command deliberately
does not overwrite files, so rollback cannot silently destroy forensic data.

This runbook covers the implemented HTTPS/WSS reverse-proxy, systemd, graceful
shutdown, migration, backup, restore, readiness, metrics, verified release, and
rollback paths. Load and memory evidence are described separately in the
[soak runbook](soak-testing.md). A configured 24-hour workflow is not itself a
successful 24-hour result; retain the completed candidate report before treating
acceptance criterion 19 as satisfied.
