# Tiamud operator tools

`tiamud-admin` provides offline integrity checks and consistent SQLite backups.
It never starts the game server and refuses to overwrite an existing backup.

Check both the authoritative database and its WorldPack:

```sh
cargo run -p tiamud-admin -- check \
  --database data/tiamud.db \
  --world-pack worlds/lantern-square.world.json
```

Create and verify a SQLite snapshot with `VACUUM INTO`:

```sh
cargo run -p tiamud-admin -- backup create \
  --database data/tiamud.db \
  --output backups/tiamud-$(date +%F).db

cargo run -p tiamud-admin -- backup verify \
  --backup backups/tiamud-2026-07-15.db
```

Use a new output path for every backup. The command stages and validates the
snapshot before publishing it atomically in the destination directory.

Restore a verified backup to a new path, or migrate an older database after a
mandatory safety backup:

```sh
cargo run -p tiamud-admin -- backup restore \
  --backup backups/tiamud-2026-07-15.db \
  --database restored/tiamud.db

cargo run -p tiamud-admin -- migrate \
  --database data/tiamud.db \
  --backup backups/pre-migrate.db
```

All backup and restore destinations must be new paths; the tools refuse to
overwrite an existing file. The complete one-host procedure, including the
reviewed Tiamud systemd and Caddy examples, lives in the
[single-host runbook](../../docs/operations/single-host.md). Remaining
operational gaps such as DB-aware readiness, metrics, Telnet TLS, and retained
load/soak evidence are recorded honestly in the
[completion matrix](../../docs/project/COMPLETION_MATRIX.md).
