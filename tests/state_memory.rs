//! End-to-end CLI integration tests for `ravel-lite state memory *`.
//! Shells out to the built binary via CARGO_BIN_EXE_ravel-lite.

use std::path::PathBuf;
use std::process::Command;

use tempfile::TempDir;

fn bin() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_ravel-lite"))
}

// The legacy memory.md → memory.yaml conversion (`state migrate`) was
// retired in the v1→v2 cutover; every plan ships YAML state directly.
// The replacement, `migrate-v1-v2`, is covered by
// `tests/migrate_v1_v2_e2e.rs`.

#[test]
fn add_set_body_set_title_delete_round_trip_through_cli() {
    let tmp = TempDir::new().unwrap();
    // Start from an empty memory.yaml.
    std::fs::write(
        tmp.path().join("memory.yaml"),
        "schema_version: 1\nitems: []\n",
    )
    .unwrap();

    let add = Command::new(bin())
        .args(["state", "memory", "add"])
        .arg(tmp.path())
        .args(["--title", "New insight"])
        .args(["--body", "Body of insight.\n"])
        .output()
        .unwrap();
    assert!(add.status.success(), "add failed: {}", String::from_utf8_lossy(&add.stderr));

    let set_body = Command::new(bin())
        .args(["state", "memory", "set-body"])
        .arg(tmp.path())
        .args(["new-insight", "--body", "Rewritten body.\n"])
        .output()
        .unwrap();
    assert!(set_body.status.success());

    let set_title = Command::new(bin())
        .args(["state", "memory", "set-title"])
        .arg(tmp.path())
        .args(["new-insight", "Renamed insight"])
        .output()
        .unwrap();
    assert!(set_title.status.success());

    let show = Command::new(bin())
        .args(["state", "memory", "show"])
        .arg(tmp.path())
        .arg("new-insight")
        .output()
        .unwrap();
    let stdout = String::from_utf8(show.stdout).unwrap();
    assert!(stdout.contains("Renamed insight"), "title must update: {stdout}");
    assert!(stdout.contains("Rewritten body."), "body must update: {stdout}");
    assert!(stdout.contains("id: new-insight"), "id must remain stable: {stdout}");

    let delete = Command::new(bin())
        .args(["state", "memory", "delete"])
        .arg(tmp.path())
        .arg("new-insight")
        .output()
        .unwrap();
    assert!(delete.status.success());

    let list = Command::new(bin())
        .args(["state", "memory", "list"])
        .arg(tmp.path())
        .output()
        .unwrap();
    let stdout = String::from_utf8(list.stdout).unwrap();
    assert!(
        !stdout.contains("new-insight"),
        "entry must be gone after delete: {stdout}"
    );
}

