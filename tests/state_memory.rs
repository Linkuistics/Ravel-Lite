//! End-to-end CLI integration tests for `ravel-lite state memory *` and
//! the memory.md path of `ravel-lite state migrate`. Shells out to the
//! built binary via CARGO_BIN_EXE_ravel-lite.

use std::path::PathBuf;
use std::process::Command;

use tempfile::TempDir;

fn bin() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_ravel-lite"))
}

fn seed_two_entry_memory_md(plan_dir: &std::path::Path) {
    let content = "\
# Memory

## All prompt loading routes through `substitute_tokens`
Ad-hoc `str::replace` bypasses the hard-error guard regex. Drift guards require one canonical substitution path.

## Config overlays use deep-merge via `load_with_optional_overlay<T>()`
`src/config.rs` implements `*.local.yaml` overlays. Scalar collisions go to overlay; map collisions recurse.
";
    std::fs::write(plan_dir.join("memory.md"), content).unwrap();
}

#[test]
fn migrate_converts_memory_md_to_yaml_and_list_emits_entries() {
    let tmp = TempDir::new().unwrap();
    seed_two_entry_memory_md(tmp.path());

    let migrate = Command::new(bin())
        .args(["state", "migrate"])
        .arg(tmp.path())
        .output()
        .expect("failed to spawn ravel-lite");
    assert!(
        migrate.status.success(),
        "migrate failed: stderr={}",
        String::from_utf8_lossy(&migrate.stderr)
    );
    assert!(tmp.path().join("memory.yaml").exists());
    assert!(tmp.path().join("memory.md").exists(), "default is keep-originals");

    let list = Command::new(bin())
        .args(["state", "memory", "list"])
        .arg(tmp.path())
        .output()
        .expect("failed to spawn ravel-lite");
    assert!(
        list.status.success(),
        "list failed: stderr={}",
        String::from_utf8_lossy(&list.stderr)
    );
    let stdout = String::from_utf8(list.stdout).unwrap();
    assert!(
        stdout.contains("all-prompt-loading-routes-through-substitute-tokens"),
        "output must include first entry id: {stdout}"
    );
    assert!(
        stdout.contains("config-overlays-use-deep-merge-via-load-with-optional-overlay-t"),
        "output must include second entry id: {stdout}"
    );
}

#[test]
fn migrate_is_idempotent_across_repeated_runs_for_memory() {
    let tmp = TempDir::new().unwrap();
    seed_two_entry_memory_md(tmp.path());

    for _ in 0..2 {
        let out = Command::new(bin())
            .args(["state", "migrate"])
            .arg(tmp.path())
            .output()
            .unwrap();
        assert!(
            out.status.success(),
            "migrate failed: stderr={}",
            String::from_utf8_lossy(&out.stderr)
        );
    }

    let list = Command::new(bin())
        .args(["state", "memory", "list"])
        .arg(tmp.path())
        .output()
        .unwrap();
    let stdout = String::from_utf8(list.stdout).unwrap();
    let ids: usize = stdout.matches("id:").count();
    assert_eq!(ids, 2, "expected two entries after idempotent migrate, got stdout:\n{stdout}");
}

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

#[test]
fn migrate_handles_backlog_and_memory_together_in_one_run() {
    let tmp = TempDir::new().unwrap();
    seed_two_entry_memory_md(tmp.path());
    std::fs::write(
        tmp.path().join("backlog.md"),
        "\
### Solo task

**Category:** `maintenance`
**Status:** `not_started`
**Dependencies:** none

**Description:**

Body.

**Results:** _pending_

---
",
    )
    .unwrap();

    let out = Command::new(bin())
        .args(["state", "migrate"])
        .arg(tmp.path())
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "migrate failed: stderr={}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(tmp.path().join("backlog.yaml").exists());
    assert!(tmp.path().join("memory.yaml").exists());
}
