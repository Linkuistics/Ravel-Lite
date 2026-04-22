//! End-to-end CLI integration tests for `ravel-lite state backlog *`
//! and `ravel-lite state migrate`. Shells out to the built binary via
//! CARGO_BIN_EXE_ravel-lite, matching the pattern in tests/integration.rs.

use std::path::PathBuf;
use std::process::Command;

use tempfile::TempDir;

fn bin() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_ravel-lite"))
}

fn seed_two_task_backlog_md(plan_dir: &std::path::Path) {
    let content = "\
### Add clippy `-D warnings` CI gate

**Category:** `maintenance`
**Status:** `not_started`
**Dependencies:** none

**Description:**

Cargo clippy is clean today. Add a CI gate to keep it that way.

**Results:** _pending_

---

### Remove Claude Code `--debug-file` workaround

**Category:** `maintenance`
**Status:** `not_started`
**Dependencies:** none

**Description:**

Blocked on upstream Claude Code release past 2.1.116.

**Results:** _pending_

---
";
    std::fs::write(plan_dir.join("backlog.md"), content).unwrap();
}

#[test]
fn migrate_converts_backlog_md_to_yaml_and_list_emits_ready_tasks() {
    let tmp = TempDir::new().unwrap();
    seed_two_task_backlog_md(tmp.path());

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
    assert!(tmp.path().join("backlog.yaml").exists());
    assert!(tmp.path().join("backlog.md").exists(), "default is keep-originals");

    let list = Command::new(bin())
        .args(["state", "backlog", "list"])
        .arg(tmp.path())
        .args(["--status", "not_started", "--ready"])
        .output()
        .expect("failed to spawn ravel-lite");
    assert!(
        list.status.success(),
        "list failed: stderr={}",
        String::from_utf8_lossy(&list.stderr)
    );
    let stdout = String::from_utf8(list.stdout).unwrap();
    assert!(stdout.contains("add-clippy-d-warnings-ci-gate"), "output must include task id: {stdout}");
    assert!(
        stdout.contains("remove-claude-code-debug-file-workaround"),
        "output must include second task id: {stdout}"
    );
}

#[test]
fn migrate_dry_run_does_not_write_yaml() {
    let tmp = TempDir::new().unwrap();
    seed_two_task_backlog_md(tmp.path());

    let out = Command::new(bin())
        .args(["state", "migrate"])
        .arg(tmp.path())
        .arg("--dry-run")
        .output()
        .unwrap();
    assert!(out.status.success(), "dry-run must exit 0");
    assert!(!tmp.path().join("backlog.yaml").exists(), "dry-run wrote yaml");
}

#[test]
fn migrate_is_idempotent_across_repeated_runs() {
    let tmp = TempDir::new().unwrap();
    seed_two_task_backlog_md(tmp.path());

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

    // List must still yield two tasks (no duplication, no corruption).
    let list = Command::new(bin())
        .args(["state", "backlog", "list"])
        .arg(tmp.path())
        .output()
        .unwrap();
    let stdout = String::from_utf8(list.stdout).unwrap();
    let tasks: usize = stdout.matches("id:").count();
    assert_eq!(tasks, 2, "expected two tasks after idempotent migrate, got stdout:\n{stdout}");
}

#[test]
fn migrate_parse_failure_leaves_filesystem_untouched() {
    let tmp = TempDir::new().unwrap();
    std::fs::write(tmp.path().join("backlog.md"), "### Bad\n\nno fields\n").unwrap();

    let out = Command::new(bin())
        .args(["state", "migrate"])
        .arg(tmp.path())
        .output()
        .unwrap();
    assert!(!out.status.success(), "malformed input must exit non-zero");
    assert!(!tmp.path().join("backlog.yaml").exists(), "partial write on parse failure");
}

#[test]
fn add_set_status_set_results_round_trip_through_cli() {
    let tmp = TempDir::new().unwrap();
    // Start from an empty backlog.yaml so add has nothing to collide with.
    std::fs::write(
        tmp.path().join("backlog.yaml"),
        "tasks: []\n",
    )
    .unwrap();

    let add = Command::new(bin())
        .args(["state", "backlog", "add"])
        .arg(tmp.path())
        .args(["--title", "New task", "--category", "maintenance"])
        .args(["--description", "Task body.\n"])
        .output()
        .unwrap();
    assert!(add.status.success(), "add failed: {}", String::from_utf8_lossy(&add.stderr));

    let set_status = Command::new(bin())
        .args(["state", "backlog", "set-status"])
        .arg(tmp.path())
        .args(["new-task", "in_progress"])
        .output()
        .unwrap();
    assert!(set_status.status.success());

    let set_results = Command::new(bin())
        .args(["state", "backlog", "set-results"])
        .arg(tmp.path())
        .args(["new-task", "--body", "Finished.\n"])
        .output()
        .unwrap();
    assert!(set_results.status.success());

    // set-results is only meaningful on `done` tasks conceptually, but
    // the verb itself accepts any status; the conceptual invariant
    // (flip status to done first) is a prompt-side concern.
    let show = Command::new(bin())
        .args(["state", "backlog", "show"])
        .arg(tmp.path())
        .arg("new-task")
        .output()
        .unwrap();
    let stdout = String::from_utf8(show.stdout).unwrap();
    assert!(stdout.contains("in_progress"));
    assert!(stdout.contains("Finished."));
}
