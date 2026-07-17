# Single-host operation

This runbook covers a public Tiamud installation on one Linux host. HTTPS/WSS
terminates at Caddy; the Rust server, PostgreSQL, WorldPacks, assets and backups
remain on that host. The public Telnet listener terminates TLS directly in the
server. Telnet credentials must not be exposed over the public Internet until
that listener has a valid certificate; bind explicit plaintext mode only to a
trusted network or firewall it off.

This is a reviewed reference procedure, not evidence that a public production
host has been installed. PostgreSQL is the recommended profile for a new public
host. SQLite remains the executable's zero-configuration fallback for local or
legacy installations and has a separate procedure below. Tiamud selects
exactly one; neither profile requires Kubernetes.

## Layout

```text
/usr/local/bin/tiamud-server
/usr/local/bin/tiamud-admin
/etc/tiamud/tiamud.env
/var/lib/tiamud/
  worlds/lantern-square.world.json
  backups/

PostgreSQL data and configuration use the distribution's protected paths.
```

Create a dedicated `tiamud` system user, copy the reviewed
[`deploy/tiamud.service`](../../deploy/tiamud.service) to
`/etc/systemd/system/`, copy
[`deploy/tiamud.env.example`](../../deploy/tiamud.env.example) to
`/etc/tiamud/tiamud.env`, provision the local role/database named by its active
PostgreSQL URL, replace credentials, and adjust addresses and paths. To choose
the SQLite fallback instead, comment that URL and enable exactly the documented
`TIAMUD_DB_PATH` line. Install the example
[`deploy/Caddyfile`](../../deploy/Caddyfile) after replacing the hostname.
Install a separately renewed PEM certificate chain and private key at the paths
configured by `TIAMUD_TELNET_TLS_CERT` and `TIAMUD_TELNET_TLS_KEY`; restrict the
key to the `tiamud` service account and restart after atomic certificate
renewal. Startup fails closed when TLS mode has an invalid identity.

The reference unit writes only below `/var/lib/tiamud`, limits address families
to local/IPv4/IPv6 sockets, removes ambient capabilities and isolates devices,
namespaces, kernel controls and other users' process details. Validate it on
the target distribution with `systemd-analyze security tiamud.service`; an
older systemd may require explicitly removing an unsupported directive.

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
performs bounded probes of both the world actor and selected storage worker and returns
`503` if either cannot answer. Operators must still run `tiamud-admin check`
and the smoke workflow after an update; readiness is not a replacement for
integrity or gameplay verification.

The service writes structured logs to stdout/stderr. `systemd-journald` owns
rotation and retention; configure its host-wide limits to match available disk.

The recommended environment uses `TIAMUD_DATABASE_URL=postgresql://...`.
`TIAMUD_DB_PATH` selects the mutually exclusive SQLite fallback; an explicit
`sqlite://...` URL is also accepted. With neither variable the executable uses
`data/tiamud.db`, which is useful locally but is not the new-public-host
recommendation. Startup selects exactly one store and readiness reports its
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
dropped output and per-transport output-queue depth, selected-database
transactions/errors/latency and queue depth, ticks/errors/latency, scripts/errors/timeouts/
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
service. For SQLite, use the repository CLI:

```sh
sudo systemctl stop tiamud
tiamud-admin check \
  --database /var/lib/tiamud/data/tiamud.db \
  --world-pack /var/lib/tiamud/worlds/lantern-square.world.json
```

For PostgreSQL, use the installation's native consistency/recovery procedure,
then check Tiamud readiness and smoke behavior. `tiamud-admin check` and its
backup subcommands are SQLite tools and are not PostgreSQL recovery evidence.

## SQLite fallback: backup and verification

This section applies to the SQLite local/legacy profile. Create backups on a
filesystem covered by the host's normal off-machine backup policy. Every
command refuses to overwrite an existing destination.

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

Restore character rows and the shared world snapshot from the same committed
database point. Ability cooldowns are absolute world-tick deadlines, so copying
characters into a different world clock or combining independently restored
fragments is unsupported; use the verified whole-backend recovery path.

## Event retention and SQLite file size

In current schema v11, both SQLite and PostgreSQL retain the v8
`last_event_sequence` snapshot watermark behavior. The same transaction retains the
covered 10,000-sequence suffix and removes only rows older than that watermark;
sequence gaps can make the exact row count smaller. A database without a
snapshot is never compacted. This keeps normal runtime storage bounded without
weakening snapshot/event atomicity.

SQLite normally places deleted rows on its internal free-page list and reuses
those pages for later writes. The database file therefore may not become
physically smaller after event compaction. Do not run `VACUUM` against the live
server. If reclaiming filesystem space is operationally necessary, treat it as
offline maintenance: stop Tiamud, create and verify a backup, perform the
operator-reviewed rebuild on a separate copy or during the maintenance window,
verify integrity and a restore, then restart the service.

## SQLite schema migration

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

Schema v11 changes the physical bound of the opaque `world_runtime` singleton,
not its authority or representation. SQLite rebuilds that table transactionally
during v10→v11 and preserves its singleton key, snapshot version, JSON,
`last_event_sequence` and update timestamp. PostgreSQL replaces the named size
constraint and advances `tiamud_schema` in the same migration chain. Neither
backend creates a mirror loot table, converts the snapshot to `jsonb` or writes
to both stores.

The decoder applies a dual admission rule: raw legacy snapshots v1-v5 remain
limited to 1 MiB, while v6 and current v7 snapshots and new commits may use up
to 2 MiB. This prevents an oversized old document from using migration as a
way around its original bound. A strict v6 document expands to the exact v7
form with empty applied-affix state and sealed-open counter one; v7 additionally
retains applied affixes plus sealed reward provenance and open counters. The
whole snapshot remains one authority: a successful `open` commits seal
consumption, RNG advancement, counters, created items, bindings and capacity
together, while any failure restores the checkpoint exactly. Always keep the
pre-migration backup until integrity, readiness, restart and gameplay checks
pass.

## Emergency Implementor recovery

Schema migration never recreates canonical staff authority during an ordinary
restart. If an installation has no `administrator` or `implementor` profile,
stop the server and use the explicit, audited break-glass command with an
existing registered account:

```sh
sudo systemctl stop tiamud
tiamud-admin backup create \
  --database /var/lib/tiamud/data/tiamud.db \
  --output /var/lib/tiamud/backups/pre-admin-recovery.sqlite3
tiamud-admin recover-admin \
  --database /var/lib/tiamud/data/tiamud.db \
  --account Keeper
sudo systemctl start tiamud
```

The command requires schema v11 and refuses to run while any operational
`administrator` or `implementor` profile exists. A legacy `admin` role alone
does not count as executable authority. Recovery replaces the target's profile
with `implementor`, clears its lore title and previous direct assignments,
installs exactly the twelve closed capabilities, and retains the legacy
`admin` role only for compatibility. Profile, bundle, role, canonical
append-only `staff_audit` entry and compatibility `moderation_audit` entry
commit in one transaction; any failure rolls all of them back.

`tiamud-admin` remains SQLite-only. For PostgreSQL, stop the server, take and
verify the installation's normal `pg_dump` or physical backup, then run
`psql "$TIAMUD_DATABASE_URL" --set=target_account=Keeper` and apply this
equivalent schema-v11 transaction:

```sql
BEGIN;
LOCK TABLE accounts, staff_profiles, staff_capability_assignments,
           moderation_audit, staff_audit IN EXCLUSIVE MODE;
SELECT set_config('tiamud.recovery_account', :'target_account', true);
DO $$
DECLARE
    target_id BIGINT;
    recovered_at BIGINT := floor(extract(epoch FROM now()))::BIGINT;
BEGIN
    IF (SELECT version FROM tiamud_schema WHERE singleton = 1) IS DISTINCT FROM 11 THEN
        RAISE EXCEPTION 'Implementor recovery requires Tiamud schema 11';
    END IF;
    IF EXISTS (
        SELECT 1 FROM staff_profiles
        WHERE profile_key IN ('administrator', 'implementor')
    ) THEN
        RAISE EXCEPTION 'an administrator or implementor profile already exists';
    END IF;
    SELECT id INTO target_id
    FROM accounts
    WHERE lower(name) = lower(current_setting('tiamud.recovery_account'))
      AND auth_state = 'registered';
    IF target_id IS NULL THEN
        RAISE EXCEPTION 'the target must be an existing registered account';
    END IF;

    UPDATE accounts SET role = 'admin' WHERE id = target_id;

    INSERT INTO staff_profiles
        (account_id, profile_key, lore_title, assigned_by_account_id,
         created_at, updated_at)
    VALUES
        (target_id, 'implementor', NULL, target_id, recovered_at, recovered_at)
    ON CONFLICT (account_id) DO UPDATE SET
        profile_key = EXCLUDED.profile_key,
        lore_title = NULL,
        assigned_by_account_id = EXCLUDED.assigned_by_account_id,
        updated_at = EXCLUDED.updated_at;

    DELETE FROM staff_capability_assignments WHERE account_id = target_id;
    INSERT INTO staff_capability_assignments
        (account_id, capability, assigned_by_account_id, created_at)
    SELECT target_id, recovered.capability, target_id, recovered_at
    FROM (VALUES
        ('reports.view'),
        ('reports.resolve'),
        ('moderation.mute'),
        ('moderation.ban'),
        ('staff.assign'),
        ('staff.inspect'),
        ('staff.where'),
        ('staff.goto'),
        ('staff.transfer'),
        ('staff.restore'),
        ('staff.announce'),
        ('staff.audit')
    ) AS recovered(capability);

    INSERT INTO moderation_audit
        (actor_account_id, target_account_id, action, details_json, created_at)
    VALUES
        (target_id, target_id, 'recover_admin',
         '{"profile":"implementor","source":"postgres_break_glass"}',
         recovered_at);
    INSERT INTO staff_audit
        (actor_account_id, target_account_id, capability, action,
         reason, details_json, created_at)
    VALUES
        (target_id, target_id, 'staff.assign', 'recover_admin',
         'Offline break-glass recovery',
         '{"profile":"implementor","source":"postgres_break_glass"}',
         recovered_at);
END $$;
COMMIT;
```

Check that the transaction reports `COMMIT`, verify the profile, all twelve
capabilities and both audit rows, then restart Tiamud. Any raised exception
aborts the transaction; use `ROLLBACK` before correcting the cause.

## PostgreSQL-first public profile

Provision a dedicated local database and least-privilege service role using the
host distribution's PostgreSQL procedure. Configure ordinary monitoring,
`pg_dump`/restore and, where required, point-in-time recovery before publishing
the service. Tiamud creates or verifies its own schema on connection, but its
`backup` subcommands are SQLite tools and must not be represented as PostgreSQL
backup coverage.

Configure exactly one storage line:

```text
TIAMUD_DATABASE_URL=postgresql://tiamud@127.0.0.1/tiamud?sslmode=require
# TIAMUD_DB_PATH is absent
```

The runtime PostgreSQL connector uses Rustls and the operating-system trust
store; URL `sslmode` remains authoritative, including an explicit local
plaintext choice. Exercise registration, login, character creation and
selection, an authored ability, movement, combat, social state, restart and
readiness before publishing the host.

### Move an existing SQLite installation

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

The mover requires the current SQLite schema v11, including durable social
channel subscriptions, staff profiles, capability assignments, report
resolutions and staff audit. It refuses a non-empty target and commits schema
plus rows in one PostgreSQL transaction. Configure exactly one line in the
service environment and remove/comment the other:

```text
TIAMUD_DATABASE_URL=postgresql://tiamud@db.example/tiamud?sslmode=require
# TIAMUD_DB_PATH is absent
```

Exercise registration, login, movement, combat, social state, restart and
readiness before deleting or archiving the SQLite rollback copy.
The repository's retained [PostgreSQL 18 schema-v8 acceptance
run](../project/log/2026-07-16-postgresql-v8-acceptance.md) proves the earlier v8 adapter,
verified SQLite recovery, atomic mover, mixed runtime gameplay and durable
authenticated restart against a separate disposable server process. It does
not prove or claim a schema-v11 acceptance run. Focused v11 migration, store
and transfer tests cover the runtime-snapshot cap, subscriptions and additive
staff tables, but they do not replace this exercise or establish monitoring,
TLS, PostgreSQL backup/restore or point-in-time recovery for a production
installation.

## Binary and world update

1. Verify the candidate archive checksum and OIDC-signed provenance as described
   in the [release runbook](releases.md), then extract it into staging.
2. Create and verify a native PostgreSQL backup, or the `tiamud-admin` backup
   for an SQLite installation.
3. Save the current binary and WorldPack under a versioned rollback directory.
4. Stop Tiamud, install the new binary and reviewed pack, validate the pack with
   `tiamud-world validate`, and run the integrity procedure for the selected
   database (`tiamud-admin check` applies only to SQLite).
5. Start the service and exercise registration, character entry, movement, and
   inventory against the installed listener, for example:

   ```sh
   node scripts/load.mjs --url http://127.0.0.1:8080 \
     --users 1 --rounds 1 --concurrency 1 --pace-ms 100
   ```

   `scripts/smoke.mjs` is useful in the candidate gate, but it starts its own
   temporary debug server and therefore is not proof of the installed service.

If validation or smoke testing fails, stop the service, restore the prior
binary and WorldPack, preserve the failed database for diagnosis, and follow
the selected backend's verified recovery procedure. SQLite restore deliberately
does not overwrite files, so that path cannot silently destroy forensic data;
PostgreSQL recovery must provide the equivalent operator-controlled safety.

This runbook covers the implemented HTTPS/WSS reverse-proxy, systemd, graceful
shutdown, migration, backup, restore, readiness, metrics, verified release, and
rollback paths. Load and memory evidence are described separately in the
[soak runbook](soak-testing.md). A configured 24-hour workflow is not itself a
successful 24-hour result; retain the completed candidate report before treating
acceptance criterion 19 as satisfied.
