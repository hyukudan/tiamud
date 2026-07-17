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
  area-registry/
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

WebSocket `command.submit` permanece limitado a 1.024 bytes; una línea Telnet
admite hasta 8 KiB para que `board post` pueda transportar el asunto y cuerpo
máximos y descarta entera cualquier línea mayor. El ingreso JSON WebSocket
rechaza claves duplicadas a cualquier profundidad. No eleves los límites del
proxy o listener para eludir esos controles de aplicación.

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

In current schema v15, both SQLite and PostgreSQL retain the
`last_event_sequence` snapshot watermark behavior. The same transaction retains the
covered 10,000-sequence suffix and removes only rows older than that watermark;
sequence gaps can make the exact row count smaller. A database without a
snapshot is never compacted. This keeps normal runtime storage bounded without
weakening snapshot/event atomicity.

Offline mail and community boards use separate physical-retention loops.
Startup must complete one purge batch for each surface before readiness;
failure blocks startup. A live actor then retries once per hour. Mail deletes
at most 256 expired messages/evidence rows per run; the board purge similarly
combines at most 256 expired posts/evidence rows. A periodic failure is warned
without stopping play because every list/read, capacity and evidence query
filters expiry independently; delayed maintenance never makes expired content
visible or count against a live limit. Repeated errors or a growing expired-row
backlog are operational alerts, not a reason to raise a batch without
measurement.

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

Schema v12 adds bounded offline mail without changing the runtime snapshot:
the closed `social_reports.report_kind` marker, `social_mail_messages`,
per-account `social_mail_usage` and the independently retained
`social_mail_report_evidence`. The marker keeps an open mail report in the
per-account cap after its 90-day evidence body is purged; it does not extend
body retention. The v11→v12 step is additive in both backends. Keep the
pre-migration backup until inbox, quota, report resolution, restart and the
selected backend's integrity checks pass.

Schema v13 adds the two closed public boards without changing the runtime
snapshot: `social_board_posts`, per-account `social_board_usage`, retained
`social_board_report_evidence` and `report_kind='board'`. SQLite rebuilds
`social_reports` plus both mail child tables transactionally to widen the
closed `CHECK`; it copies parents before children, checks foreign keys before
commit and preserves all v12 IDs, links and bytes. PostgreSQL replaces its
named constraint. Keep the v12 safety backup until catalog/list/read/post/
delete/report, staff resolution, expiry, restart and integrity checks pass.

El esquema v14 añade el singleton `world_area_generation`. Guarda el ID del
mundo, los digests activo y anterior del catálogo, la secuencia de activación y
el timestamp; no guarda rutas del host ni duplica estado runtime. La migración
v13→v14 es aditiva. En aquel baseline, un corte 6C actualizaba esta fila y
`WorldRuntimeSnapshot` v7 en la misma transacción mediante compare-and-swap;
v15 conserva la frontera con snapshot v8.
Conserva la copia de seguridad hasta que pasen arranque, activación, rollback
explícito y las comprobaciones de integridad del backend elegido.

El esquema v15 añade `world_area_checkpoints` sin cambiar la liga generacional.
Cada fila conserva generación, unidad, revisiones, epoch, versión, tick, JSON
acotado y SHA-256. La migración v14→v15 crea la tabla vacía. Conserva la copia
hasta validar que un snapshot v8 residente arranca, que un manifiesto retirado
falla cerrado sin su checkpoint exacto y que el mover copia o rechaza cada fila
de forma atómica. No habilites LRU ni retiros automáticos solo por migrar.

Para una instalación 6B existente, la fila v14 nace vacía y no existe forma de
reconstruir automáticamente qué catálogo v13 estaba ejecutando. Recovery sin
snapshot/ledger puede contener personajes; por eso el servidor consulta ambas
autoridades. Si existe recovery compartido o cualquier personaje durable, el
primer arranque v14 debe usar **exactamente el mismo G0** del último arranque
v13. Procedimiento obligatorio:

1. detén el servicio y crea/verifica copias del backend y del registro completo;
2. registra la ruta y SHA-256 exactos de `TIAMUD_AREA_CATALOG` G0;
3. instala/migra v14 sin publicar ni seleccionar otro catálogo;
4. configura temporalmente
   `TIAMUD_ACK_LEGACY_AREA_GENERATION=<sha256-exacto-G0>`, arranca todavía con
   G0 y verifica readiness, logs y la fila `world_area_generation`;
5. elimina esa variable de la configuración del servicio después de la liga; y
6. solo después publica o selecciona G1 y usa el SIGHUP de 6C.

No combines actualización de binario/esquema y cambio de catálogo. La primera
liga v14 acepta G0 como autoridad inicial y no atraviesa compatibilidad/CAS
contra una identidad anterior ausente; usar G1 en ese momento se saltaría la
puerta 6C y perdería el rollback durable a G0.

Una base vacía o con cuentas pero sin personajes ni recovery de mundo puede
ligar su primera generación sin ACK. No uses esta excepción para convertir una
autoridad monolítica existente a catálogo: 6C no define esa migración. La
variable falla cerrada en modo monolítico, con liga existente, sin
recovery/personaje que reconocer o si no coincide con el SHA-256 canónico.

The decoder applies a dual admission rule: raw legacy snapshots v1-v5 remain
limited to 1 MiB, while v6-v8 snapshots and new commits may use up
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

The command requires schema v15 and refuses to run while any operational
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
equivalent schema-v15 transaction:

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
    IF (SELECT version FROM tiamud_schema WHERE singleton = 1) IS DISTINCT FROM 15 THEN
        RAISE EXCEPTION 'Implementor recovery requires Tiamud schema 15';
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

La voz espacial 5E no cambia esquema ni retiene cuerpos y ha pasado su puerta
integrada automatizada. Un smoke operativo específico deberá situar
personajes en origen, un vecino, una sala a dos saltos y otra tras una salida
secreta; comprobar `yell` radio uno, `shout` radio tres, exclusión secreta y
supresión por `condition.deaf`, tanto en Web como en texto/GMCP. Ese escenario
manual no se ejecutó como parte de 5E y no debe inferirse del smoke general.

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

The mover requires the current SQLite schema v15, including durable social
channel subscriptions, staff profiles, capability assignments, report
resolutions, staff audit, offline mail usage/messages and retained mail-report
evidence, board posts, usage and retained evidence, and the durable area
generation row and every validated area checkpoint when present. It refuses a
non-empty target and commits schema
plus rows in one PostgreSQL transaction. Configure exactly one line in the
service environment and remove/comment the other:

```text
TIAMUD_DATABASE_URL=postgresql://tiamud@db.example/tiamud?sslmode=require
# TIAMUD_DB_PATH is absent
```

Exercise registration, login, movement, combat and social state before deleting
or archiving the SQLite rollback copy. For schema v15 explicitly send, list,
read, delete and report mail, restart, and confirm that the moved PostgreSQL
state retains the inbox, usage quota and report evidence. Also publish, list,
read, delete and report one board post; confirm post quota/evidence survive the
move, resolve the report and verify readiness again.

En la comprobación Telnet/GMCP, confirma por separado que una línea máxima de
publicación conserva el cuerpo textual completo y que todo paquete negociado
permanece bajo 8 KiB. Un `Tiamud.Board.Entry` cuyo escape JSON no quepa debe
seguir siendo el mismo paquete, con resumen, prefijo UTF-8 de `body`,
`truncated: true` y `overflow` con paquete, máximo y bytes originales. El
fallback equivalente de `Tiamud.Mail.Message` es defensivo: el cuerpo canónico
de correo de 2.048 bytes normalmente cabe completo.
The repository's retained [PostgreSQL 18 schema-v8 acceptance
run](../project/log/2026-07-16-postgresql-v8-acceptance.md) proves the earlier v8 adapter,
verified SQLite recovery, atomic mover, mixed runtime gameplay and durable
authenticated restart against a separate disposable server process. Separately,
an isolated checksummed PostgreSQL 18.4 instance passes the v13 adapter and
real mover suite 19/19 after correcting a v6 fixture defect, including a
representative current SQLite source. Focused real-server tests also cover the
board/runtime flow and concurrent advisory-lock quota path. These checks do not
establish monitoring,
TLS, PostgreSQL backup/restore or point-in-time recovery for a production
installation.

## Sustitución quiescente de generaciones de área

Esta operación solo existe cuando el servicio usa
`TIAMUD_AREA_REGISTRY` + `TIAMUD_AREA_CATALOG`. La primera ejecución liga el
catálogo verificado en SQL v15 antes de readiness. En reinicios posteriores la
fila durable prevalece: el servidor deriva la ruta exacta a partir del registro,
`worldId` y digest aunque `area-active` apunte a una publicación distinta. Una
base ligada a catálogo no puede arrancar en modo WorldPack monolítico. Si se
migra desde v13, completa primero la liga de G0 descrita en la sección anterior;
no uses este flujo para introducir G1 en el mismo primer arranque v14.

Una activación 6C no aplica un parche de área. Verifica y construye **la
generación completa** fuera del actor. Debe conservar mundo/sala inicial, RNG,
gameplay, objetos e instancias iniciales, NPC, botín, afijos, conjunto de áreas,
ownership, IDs de sala y VNUM; no admite migración persistente. Solo prosa,
salidas y capas de presentación/percepción/Lua síncrono validadas pueden cambiar.

Flujo de activación creciente:

1. crea y verifica la copia del backend conforme a este runbook;
2. publica con `tiamud-world area-publish` y conserva la ruta exacta del
   catálogo anterior;
3. espera una ventana sin usuarios y envía SIGHUP; y
4. inspecciona `journalctl -u tiamud` y ejecuta readiness/smoke.

```sh
sudo systemctl kill --signal=HUP tiamud
curl --fail http://127.0.0.1:8080/health/ready
```

SIGHUP solo reconcilia el puntero local; no es una señal de apagado y no existe
un comando Web, Telnet, GMCP o staff equivalente. El servidor relee el puntero
para detectar una publicación concurrente, verifica todos los blobs y hace
staging privado con gauges aislados; las métricas públicas siguen describiendo
la generación activa. Después cierra admisiones y exige **cero sesiones y cero
leases**. Si hay un login, acción o carga admitida, devuelve `busy`, reabre
admisiones y conserva la generación activa sin expulsar, mover o desconectar a
nadie. El operador debe esperar y reintentar otro SIGHUP. Tras el staging, el
runtime adquiere el lock cooperativo de lectura del registro usado por
`area-publish`/`area-select`, revalida entonces el puntero, aborta el intento si
el lock está ocupado y lo conserva hasta resolver el CAS. Un
publicador cooperante espera en vez de cambiar `area-active` durante el corte.
En Solaris, runtime y CLI usan el mismo protocolo de registro `fcntl` compatible
con `fd-lock`; no dependen de locks de familias distintas.

Con quiescencia, el actor exige todas las unidades residentes y restaura el
snapshot v8 actual sobre la candidata,
confirma snapshot + puntero durable mediante CAS y solo entonces intercambia
mundo, contenido runtime y coordinador. Verificación, incompatibilidad, restore
y fallos definitivos antes del commit dejan activa la generación anterior. Una
caída antes/después del commit recupera respectivamente la identidad
anterior/nueva.

Un error devuelto por el commit puede ser ambiguo y **no** demuestra rollback.
El servidor relee `world_area_generation`: si contiene G1 publica la candidata;
si todavía contiene G0 reabre la anterior. Cualquier otro valor o error de
lectura deja el router en `Committing`, detiene el actor y mantiene readiness
503 sin aceptar tráfico hasta reinicio y recuperación de la autoridad durable.

Para rollback, no edites a mano `area-active`. Selecciona un catálogo inmutable
retenido y solicita el mismo corte:

```sh
tiamud-world area-select \
  --catalog /var/lib/tiamud/area-registry/area-catalogs/<world-key>/<digest>.catalog.json \
  --registry /var/lib/tiamud/area-registry
sudo systemctl kill --signal=HUP tiamud
```

La secuencia durable aumenta también al revertir. `previous_catalog_sha256`
solo conserva la generación inmediatamente anterior; no es un historial ni
borra blobs. Retén explícitamente los catálogos y bundles necesarios.

6C no soporta jugadores durante drenaje, migraciones directas/inversas,
reemplazo de revisión por área, LRU, GC, registro remoto,
compresión o firma. La puerta pasa 774 pruebas Rust —329 del servidor— y la
reauditoría 0 P1/0 P2. Sobre PostgreSQL 18 desechable pasan 20/20 pruebas del
adaptador y 7/7 enfocadas del servidor, incluida la CAS de generación v14. El
humo real abre esquema 14, liga G0 `e954be…15a6` en secuencia 1, confirma
readiness PostgreSQL/mundo, Web 200, banner Telnet, dos áreas residentes,
SIGHUP `noop=1` y cierre actor/store limpio. El humo SQLite retenido cubre
además activación G1, rollback G0 y restart. Conserva el mismo procedimiento
sobre persistencia y registro desechables antes de una instalación propia y
consulta el [registro de aceptación PostgreSQL v14/PWA](../project/log/2026-07-17-postgresql-v14-and-pwa-acceptance.md).

## Retirada reversible 6D: estado operativo parcial

El esquema v15 y snapshot v8 permiten que el runtime interno retire y
rehidrate una unidad vacía de la misma revisión. No existe todavía un comando
de operador, señal, endpoint, acción staff ni política LRU para solicitarlo. No
edites `world_area_checkpoints` ni el manifiesto del snapshot a mano: deben
avanzar juntos mediante el CAS del servidor.

Antes de considerar esta función operativa en una instalación, aunque las
pruebas enfocadas PostgreSQL v15 pasan 22/22+8/8, falta probar el humo de
restart y un E2E específico de ACK perdido 6D, medir descenso
real de índices y RSS y definir presupuesto/histéresis. La evidencia PostgreSQL
v14 anterior no cubre esta tabla. Una inconsistencia de manifiesto/checkpoint
debe bloquear readiness; restaura el backend completo verificado en vez de
suprimir una fila aislada.

## Actualización del binario y del mundo

1. Verifica checksum y procedencia OIDC del candidato según la
   [guía de release](releases.md) y extráelo en staging.
2. Crea y verifica una copia nativa PostgreSQL o la copia `tiamud-admin` de una
   instalación SQLite.
3. Conserva binario, WorldPack y, en modo catálogo, el registro/catálogo exacto
   actuales bajo un directorio de rollback versionado.
4. Detén Tiamud, instala el binario y artefactos revisados, valida el WorldPack
   con `tiamud-world validate` y ejecuta la integridad del backend seleccionado
   (`tiamud-admin check` solo cubre SQLite).
5. Arranca el servicio y prueba registro, entrada, movimiento e inventario
   contra el listener instalado, por ejemplo:

   ```sh
   node scripts/load.mjs --url http://127.0.0.1:8080 \
     --users 1 --rounds 1 --concurrency 1 --pace-ms 100
   ```

   `scripts/smoke.mjs` ayuda en el gate candidato, pero inicia su propio servidor
   temporal de debug y no demuestra el servicio instalado.

Si falla validación o smoke, detén el servicio, restaura el binario y WorldPack
anteriores, conserva la base fallida para diagnóstico y sigue la recuperación
verificada del backend. SQLite no sobrescribe al restaurar, para no destruir
datos forenses; PostgreSQL debe aportar una seguridad equivalente controlada
por el operador. Un rollback de contenido 6C compatible usa `area-select` y
SIGHUP; recuperar la base completa es una operación distinta.

Esta guía cubre proxy HTTPS/WSS, systemd, apagado ordenado, migración, copia,
restore, readiness, métricas, release verificada y rollback. La evidencia de
carga y memoria se describe en la [guía de soak](soak-testing.md). Configurar
un workflow de 24 horas no equivale a ejecutarlo; conserva el informe completo
del candidato antes de dar por satisfecho el criterio 19.
