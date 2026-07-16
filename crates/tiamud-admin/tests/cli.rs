use std::{fs, process::Command};

use rusqlite::{Connection, OpenFlags};
use tempfile::TempDir;

#[test]
fn cli_checks_and_backs_up_a_restorable_database() {
    let temporary = TempDir::new().unwrap();
    let database = temporary.path().join("tiamud.sqlite3");
    let world = temporary.path().join("world.json");
    let backup = temporary.path().join("backups/tiamud.sqlite3");
    let restored_database = temporary.path().join("restored/tiamud.sqlite3");
    let connection = Connection::open(&database).unwrap();
    connection
        .execute_batch(
            "CREATE TABLE state (value TEXT NOT NULL);
             INSERT INTO state (value) VALUES ('restorable');
             PRAGMA user_version = 3;",
        )
        .unwrap();
    drop(connection);
    fs::write(
        &world,
        r#"{
            "schemaVersion": 3,
            "id": "cli-test",
            "name": "CLI Test",
            "startRoomId": "room:start",
            "rooms": [{
                "id": "room:start",
                "title": "Start",
                "description": "The beginning."
            }]
        }"#,
    )
    .unwrap();

    let check = Command::new(env!("CARGO_BIN_EXE_tiamud-admin"))
        .args(["check", "--database"])
        .arg(&database)
        .arg("--world-pack")
        .arg(&world)
        .output()
        .unwrap();
    assert!(
        check.status.success(),
        "check stderr: {}",
        String::from_utf8_lossy(&check.stderr)
    );
    assert!(String::from_utf8_lossy(&check.stdout).contains("check passed"));

    let snapshot = Command::new(env!("CARGO_BIN_EXE_tiamud-admin"))
        .args(["backup", "create", "--database"])
        .arg(&database)
        .arg("--output")
        .arg(&backup)
        .output()
        .unwrap();
    assert!(
        snapshot.status.success(),
        "backup stderr: {}",
        String::from_utf8_lossy(&snapshot.stderr)
    );
    assert!(String::from_utf8_lossy(&snapshot.stdout).contains("backup created"));

    let verified = Command::new(env!("CARGO_BIN_EXE_tiamud-admin"))
        .args(["backup", "verify", "--backup"])
        .arg(&backup)
        .output()
        .unwrap();
    assert!(
        verified.status.success(),
        "verify stderr: {}",
        String::from_utf8_lossy(&verified.stderr)
    );

    let restore = Command::new(env!("CARGO_BIN_EXE_tiamud-admin"))
        .args(["backup", "restore", "--backup"])
        .arg(&backup)
        .arg("--database")
        .arg(&restored_database)
        .output()
        .unwrap();
    assert!(
        restore.status.success(),
        "restore stderr: {}",
        String::from_utf8_lossy(&restore.stderr)
    );

    let restored =
        Connection::open_with_flags(restored_database, OpenFlags::SQLITE_OPEN_READ_ONLY).unwrap();
    let value: String = restored
        .query_row("SELECT value FROM state", [], |row| row.get(0))
        .unwrap();
    assert_eq!(value, "restorable");
}
