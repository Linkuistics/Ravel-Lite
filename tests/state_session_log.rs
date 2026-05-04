//! End-to-end CLI integration tests for `ravel-lite state session-log *`.
//! Shells out to the built binary via CARGO_BIN_EXE_ravel-lite.

use std::path::PathBuf;
use std::process::Command;

use tempfile::TempDir;

fn bin() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_ravel-lite"))
}

// The legacy session-log.md / latest-session.md → .yaml conversion
// (`state migrate`) was retired in the v1→v2 cutover; every plan ships
// YAML state directly. The replacement, `migrate-v1-v2`, is covered by
// `tests/migrate_v1_v2_e2e.rs`.

#[test]
fn append_set_latest_and_show_round_trip_through_cli() {
    let tmp = TempDir::new().unwrap();
    // Start from an empty plan; append creates session-log.yaml.
    let append = Command::new(bin())
        .args(["state", "session-log", "append"])
        .arg(tmp.path())
        .args(["--id", "2026-04-22-first"])
        .args(["--timestamp", "2026-04-22T14:00:00Z"])
        .args(["--phase", "work"])
        .args(["--body", "First body.\n"])
        .output()
        .unwrap();
    assert!(
        append.status.success(),
        "append failed: stderr={}",
        String::from_utf8_lossy(&append.stderr)
    );
    assert!(tmp.path().join("session-log.yaml").exists());

    // Idempotent re-append: same id → no-op.
    let append2 = Command::new(bin())
        .args(["state", "session-log", "append"])
        .arg(tmp.path())
        .args(["--id", "2026-04-22-first"])
        .args(["--timestamp", "2026-04-22T14:00:00Z"])
        .args(["--phase", "work"])
        .args(["--body", "First body.\n"])
        .output()
        .unwrap();
    assert!(append2.status.success());

    let list = Command::new(bin())
        .args(["state", "session-log", "list"])
        .arg(tmp.path())
        .output()
        .unwrap();
    let stdout = String::from_utf8(list.stdout).unwrap();
    let id_hits: usize = stdout.matches("id: 2026-04-22-first").count();
    assert_eq!(id_hits, 1, "idempotent append should not duplicate: {stdout}");

    // set-latest writes latest-session.yaml.
    let set_latest = Command::new(bin())
        .args(["state", "session-log", "set-latest"])
        .arg(tmp.path())
        .args(["--id", "2026-04-22-second"])
        .args(["--timestamp", "2026-04-22T15:00:00Z"])
        .args(["--body", "Second body.\n"])
        .output()
        .unwrap();
    assert!(set_latest.status.success());

    let show = Command::new(bin())
        .args(["state", "session-log", "show-latest"])
        .arg(tmp.path())
        .output()
        .unwrap();
    let stdout = String::from_utf8(show.stdout).unwrap();
    assert!(stdout.contains("2026-04-22-second"), "id expected: {stdout}");

    // show <id> on the log returns the first session.
    let show_id = Command::new(bin())
        .args(["state", "session-log", "show"])
        .arg(tmp.path())
        .arg("2026-04-22-first")
        .output()
        .unwrap();
    assert!(show_id.status.success());
    let stdout = String::from_utf8(show_id.stdout).unwrap();
    assert!(stdout.contains("First body."), "body expected: {stdout}");
}

