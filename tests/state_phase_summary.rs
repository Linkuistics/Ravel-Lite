//! End-to-end CLI integration tests for `ravel-lite state phase-summary
//! render`. Shells out to the built binary via CARGO_BIN_EXE_ravel-lite
//! after committing a baseline backlog.yaml / memory.yaml in a temp git
//! repo and then mutating the tree — exercising the `git show
//! <sha>:<path>` resolution that the verb relies on.

use std::path::{Path, PathBuf};
use std::process::Command;

use tempfile::TempDir;

fn bin() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_ravel-lite"))
}

fn init_git_repo(plan: &Path) {
    Command::new("git").current_dir(plan).args(["init", "-q"]).output().unwrap();
    Command::new("git").current_dir(plan).args(["config", "user.email", "t@t"]).output().unwrap();
    Command::new("git").current_dir(plan).args(["config", "user.name", "t"]).output().unwrap();
}

fn commit_all(plan: &Path, message: &str) -> String {
    Command::new("git").current_dir(plan).args(["add", "-A"]).output().unwrap();
    Command::new("git").current_dir(plan).args(["commit", "-q", "-m", message]).output().unwrap();
    let out = Command::new("git")
        .current_dir(plan)
        .args(["rev-parse", "HEAD"])
        .output()
        .unwrap();
    String::from_utf8(out.stdout).unwrap().trim().to_string()
}

#[test]
fn phase_summary_triage_reports_done_new_and_obsolete_vs_baseline() {
    let tmp = TempDir::new().unwrap();
    let plan = tmp.path();
    init_git_repo(plan);

    // Baseline: foo=not_started, gone=not_started.
    std::fs::write(
        plan.join("backlog.yaml"),
        r#"tasks:
- id: foo
  title: Foo task
  category: core
  status: not_started
  description: |
    body
- id: gone
  title: Soon obsolete
  category: core
  status: not_started
  description: |
    body
"#,
    )
    .unwrap();
    let baseline_sha = commit_all(plan, "baseline");

    // Current: foo=done, new fresh task added, gone removed.
    std::fs::write(
        plan.join("backlog.yaml"),
        r#"tasks:
- id: foo
  title: Foo task
  category: core
  status: done
  description: |
    body
  results: |
    did it
- id: fresh
  title: Fresh task
  category: core
  status: not_started
  description: |
    body
"#,
    )
    .unwrap();

    let out = Command::new(bin())
        .args(["state", "phase-summary", "render"])
        .arg(plan)
        .args(["--phase", "triage", "--baseline", &baseline_sha])
        .output()
        .expect("failed to spawn ravel-lite");
    assert!(
        out.status.success(),
        "phase-summary failed: stderr={}",
        String::from_utf8_lossy(&out.stderr)
    );
    let stdout = String::from_utf8(out.stdout).unwrap();
    assert!(stdout.contains("[DONE] Foo task"), "missing DONE:\n{stdout}");
    assert!(stdout.contains("[NEW] Fresh task"), "missing NEW:\n{stdout}");
    assert!(
        stdout.contains("[OBSOLETE] Soon obsolete"),
        "missing OBSOLETE:\n{stdout}"
    );
}

#[test]
fn phase_summary_dream_always_emits_a_stats_line() {
    let tmp = TempDir::new().unwrap();
    let plan = tmp.path();
    init_git_repo(plan);

    std::fs::write(
        plan.join("memory.yaml"),
        r#"schema_version: 1
items:
- id: rule
  kind: memory-entry
  claim: A rule
  justifications:
  - kind: rationale
    text: |
      original wordy body with many words in it
  status: active
  authored_at: test
  authored_in: test
"#,
    )
    .unwrap();
    let baseline_sha = commit_all(plan, "baseline");

    std::fs::write(
        plan.join("memory.yaml"),
        r#"schema_version: 1
items:
- id: rule
  kind: memory-entry
  claim: A rule
  justifications:
  - kind: rationale
    text: |
      shorter
  status: active
  authored_at: test
  authored_in: test
"#,
    )
    .unwrap();

    let out = Command::new(bin())
        .args(["state", "phase-summary", "render"])
        .arg(plan)
        .args(["--phase", "dream", "--baseline", &baseline_sha])
        .output()
        .expect("failed to spawn ravel-lite");
    assert!(
        out.status.success(),
        "phase-summary failed: stderr={}",
        String::from_utf8_lossy(&out.stderr)
    );
    let stdout = String::from_utf8(out.stdout).unwrap();
    assert!(stdout.contains("[STALE] A rule"), "missing STALE:\n{stdout}");
    assert!(stdout.contains("[STATS]"), "missing STATS:\n{stdout}");
    assert!(stdout.contains(" → "), "STATS must have continuation line:\n{stdout}");
}

#[test]
fn phase_summary_empty_baseline_sha_treats_state_as_first_cycle() {
    let tmp = TempDir::new().unwrap();
    let plan = tmp.path();
    init_git_repo(plan);

    std::fs::write(
        plan.join("backlog.yaml"),
        r#"tasks:
- id: first
  title: First task
  category: core
  status: not_started
  description: |
    body
"#,
    )
    .unwrap();

    let out = Command::new(bin())
        .args(["state", "phase-summary", "render"])
        .arg(plan)
        .args(["--phase", "triage", "--baseline", ""])
        .output()
        .expect("failed to spawn ravel-lite");
    assert!(
        out.status.success(),
        "phase-summary failed: stderr={}",
        String::from_utf8_lossy(&out.stderr)
    );
    let stdout = String::from_utf8(out.stdout).unwrap();
    assert!(
        stdout.contains("[NEW] First task"),
        "first-cycle must render current task as NEW:\n{stdout}"
    );
}

#[test]
fn phase_summary_rejects_unknown_phase_with_clear_error() {
    let tmp = TempDir::new().unwrap();
    let plan = tmp.path();
    init_git_repo(plan);

    let out = Command::new(bin())
        .args(["state", "phase-summary", "render"])
        .arg(plan)
        .args(["--phase", "work", "--baseline", ""])
        .output()
        .unwrap();
    assert!(!out.status.success(), "invalid --phase must fail");
    let stderr = String::from_utf8(out.stderr).unwrap();
    assert!(
        stderr.contains("triage") && stderr.contains("reflect") && stderr.contains("dream"),
        "error must list valid phase names: {stderr}"
    );
}
