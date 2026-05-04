//! End-to-end CLI integration tests for `ravel-lite state backlog *`.
//! Shells out to the built binary via CARGO_BIN_EXE_ravel-lite,
//! matching the pattern in tests/integration.rs.

use std::path::PathBuf;
use std::process::Command;

use tempfile::TempDir;

fn bin() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_ravel-lite"))
}

/// Seed two TMS-shaped backlog items that the
/// `list_format_markdown_*` tests below assert against. Replaces the
/// pre-cutover .md → migrate path with a direct YAML write.
fn seed_two_task_backlog_yaml(plan_dir: &std::path::Path) {
    let yaml = "\
schema_version: 1
items:
- id: add-clippy-d-warnings-ci-gate
  kind: backlog-item
  claim: 'Add clippy `-D warnings` CI gate'
  justifications:
  - kind: rationale
    text: |
      Cargo clippy is clean today. Add a CI gate to keep it that way.
  status: active
  authored_at: test
  authored_in: test
  category: maintenance
- id: remove-claude-code-debug-file-workaround
  kind: backlog-item
  claim: 'Remove Claude Code `--debug-file` workaround'
  justifications:
  - kind: rationale
    text: |
      Blocked on upstream Claude Code release past 2.1.116.
  status: active
  authored_at: test
  authored_in: test
  category: maintenance
";
    std::fs::write(plan_dir.join("backlog.yaml"), yaml).unwrap();
}

fn add_seed_task(plan_dir: &std::path::Path) {
    // Start from an empty backlog.yaml and append one task via the CLI
    // so state_backlog tests share a compact, repeatable setup.
    std::fs::write(
        plan_dir.join("backlog.yaml"),
        "schema_version: 1\nitems: []\n",
    )
    .unwrap();
    let add = Command::new(bin())
        .args(["state", "backlog", "add"])
        .arg(plan_dir)
        .args(["--title", "Seed task", "--category", "maintenance"])
        .args(["--description", "Original body.\n"])
        .output()
        .unwrap();
    assert!(
        add.status.success(),
        "seed add failed: {}",
        String::from_utf8_lossy(&add.stderr)
    );
}

#[test]
fn set_description_via_body_file_round_trips_through_cli() {
    let tmp = TempDir::new().unwrap();
    add_seed_task(tmp.path());

    let body_file = tmp.path().join("new-body.md");
    std::fs::write(&body_file, "Fresh brief from disk.\n").unwrap();

    let out = Command::new(bin())
        .args(["state", "backlog", "set-description"])
        .arg(tmp.path())
        .args(["seed-task", "--body-file"])
        .arg(&body_file)
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "set-description failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );

    let show = Command::new(bin())
        .args(["state", "backlog", "show"])
        .arg(tmp.path())
        .arg("seed-task")
        .output()
        .unwrap();
    let stdout = String::from_utf8(show.stdout).unwrap();
    assert!(
        stdout.contains("Fresh brief from disk."),
        "show must reflect new body: {stdout}"
    );
    assert!(
        !stdout.contains("Original body."),
        "old body must be gone: {stdout}"
    );
}

#[test]
fn set_description_via_body_stdin_round_trips_through_cli() {
    use std::io::Write;
    use std::process::Stdio;

    let tmp = TempDir::new().unwrap();
    add_seed_task(tmp.path());

    let mut child = Command::new(bin())
        .args(["state", "backlog", "set-description"])
        .arg(tmp.path())
        .args(["seed-task", "--body", "-"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    child
        .stdin
        .as_mut()
        .unwrap()
        .write_all(b"Piped-in body.\n")
        .unwrap();
    let out = child.wait_with_output().unwrap();
    assert!(
        out.status.success(),
        "set-description failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );

    let show = Command::new(bin())
        .args(["state", "backlog", "show"])
        .arg(tmp.path())
        .arg("seed-task")
        .output()
        .unwrap();
    let stdout = String::from_utf8(show.stdout).unwrap();
    assert!(stdout.contains("Piped-in body."));
}

#[test]
fn set_description_errors_on_unknown_task() {
    let tmp = TempDir::new().unwrap();
    add_seed_task(tmp.path());

    let out = Command::new(bin())
        .args(["state", "backlog", "set-description"])
        .arg(tmp.path())
        .args(["nonexistent", "--body", "anything\n"])
        .output()
        .unwrap();
    assert!(!out.status.success(), "unknown id must exit non-zero");
    let stderr = String::from_utf8(out.stderr).unwrap();
    assert!(stderr.contains("nonexistent"), "stderr must cite the id: {stderr}");
}

#[test]
fn set_description_rejects_empty_body() {
    let tmp = TempDir::new().unwrap();
    add_seed_task(tmp.path());

    let out = Command::new(bin())
        .args(["state", "backlog", "set-description"])
        .arg(tmp.path())
        .args(["seed-task", "--body", ""])
        .output()
        .unwrap();
    assert!(!out.status.success(), "empty body must exit non-zero");
    let stderr = String::from_utf8(out.stderr).unwrap();
    assert!(stderr.contains("empty"), "stderr must mention empty: {stderr}");

    // Original body preserved.
    let show = Command::new(bin())
        .args(["state", "backlog", "show"])
        .arg(tmp.path())
        .arg("seed-task")
        .output()
        .unwrap();
    let stdout = String::from_utf8(show.stdout).unwrap();
    assert!(stdout.contains("Original body."));
}

#[test]
fn set_description_preserves_sibling_fields() {
    let tmp = TempDir::new().unwrap();
    add_seed_task(tmp.path());

    // Pre-load sibling fields; note the plan_dir goes immediately after
    // the subcommand, then positional task id + verb-specific args.
    let run = |args: &[&str]| {
        let out = Command::new(bin())
            .args(["state", "backlog"])
            .arg(args[0])
            .arg(tmp.path())
            .args(&args[1..])
            .output()
            .unwrap();
        assert!(
            out.status.success(),
            "cmd {args:?} failed: {}",
            String::from_utf8_lossy(&out.stderr)
        );
    };
    run(&["set-status", "seed-task", "blocked", "--reason", "upstream"]);
    run(&["set-title", "seed-task", "Renamed Seed"]);
    run(&["set-results", "seed-task", "--body", "Results body.\n"]);
    run(&["set-handoff", "seed-task", "--body", "Handoff body.\n"]);

    run(&["set-description", "seed-task", "--body", "Rewritten brief.\n"]);

    let show = Command::new(bin())
        .args(["state", "backlog", "show"])
        .arg(tmp.path())
        .arg("seed-task")
        .output()
        .unwrap();
    let stdout = String::from_utf8(show.stdout).unwrap();
    assert!(stdout.contains("Rewritten brief."), "desc updated: {stdout}");
    assert!(stdout.contains("blocked"), "status preserved: {stdout}");
    assert!(stdout.contains("Renamed Seed"), "title preserved: {stdout}");
    assert!(stdout.contains("Results body."), "results preserved: {stdout}");
    assert!(stdout.contains("Handoff body."), "handoff preserved: {stdout}");
    // The task id must remain stable across the rename + description rewrite.
    assert!(stdout.contains("id: seed-task"), "id stable: {stdout}");
}

#[test]
fn add_set_status_set_results_round_trip_through_cli() {
    let tmp = TempDir::new().unwrap();
    // Start from an empty backlog.yaml so add has nothing to collide with.
    std::fs::write(
        tmp.path().join("backlog.yaml"),
        "schema_version: 1\nitems: []\n",
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
        .args(["new-task", "done"])
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

    let show = Command::new(bin())
        .args(["state", "backlog", "show"])
        .arg(tmp.path())
        .arg("new-task")
        .output()
        .unwrap();
    let stdout = String::from_utf8(show.stdout).unwrap();
    assert!(stdout.contains("status: done"), "stdout: {stdout}");
    assert!(stdout.contains("Finished."));
}

#[test]
fn set_dependencies_round_trips_through_cli() {
    let tmp = TempDir::new().unwrap();
    std::fs::write(
        tmp.path().join("backlog.yaml"),
        "schema_version: 1\nitems: []\n",
    )
    .unwrap();

    let add = |title: &str| {
        let out = Command::new(bin())
            .args(["state", "backlog", "add"])
            .arg(tmp.path())
            .args(["--title", title, "--category", "maintenance"])
            .args(["--description", "body.\n"])
            .output()
            .unwrap();
        assert!(out.status.success(), "add failed: {}", String::from_utf8_lossy(&out.stderr));
    };
    add("First");
    add("Second");

    // Set Second to depend on First.
    let set = Command::new(bin())
        .args(["state", "backlog", "set-dependencies"])
        .arg(tmp.path())
        .args(["second", "--deps", "first"])
        .output()
        .unwrap();
    assert!(set.status.success(), "set-dependencies failed: {}", String::from_utf8_lossy(&set.stderr));

    let show = Command::new(bin())
        .args(["state", "backlog", "show"])
        .arg(tmp.path())
        .arg("second")
        .output()
        .unwrap();
    let stdout = String::from_utf8(show.stdout).unwrap();
    assert!(stdout.contains("dependencies:"), "dependencies field missing: {stdout}");
    assert!(stdout.contains("first"), "dep first not present: {stdout}");

    // `--deps ""` clears the list.
    let clear = Command::new(bin())
        .args(["state", "backlog", "set-dependencies"])
        .arg(tmp.path())
        .args(["second", "--deps", ""])
        .output()
        .unwrap();
    assert!(clear.status.success(), "clear failed: {}", String::from_utf8_lossy(&clear.stderr));

    let show = Command::new(bin())
        .args(["state", "backlog", "show"])
        .arg(tmp.path())
        .arg("second")
        .output()
        .unwrap();
    let stdout = String::from_utf8(show.stdout).unwrap();
    // An empty `dependencies` is omitted from the wire form (skip-empty);
    // the absence of `dependencies:` is the cleared signal.
    assert!(
        !stdout.contains("dependencies:"),
        "deps must be cleared (skip-empty omits the field): {stdout}"
    );
}

/// Seed a TMS-shaped backlog.yaml with a `blocked` item whose only
/// dep is `done` — the canonical drift mode that the kept repair rule
/// catches.
fn seed_unblockable_backlog(plan: &std::path::Path) {
    let yaml = "\
schema_version: 1
items:
- id: parent
  kind: backlog-item
  claim: Parent
  justifications:
  - kind: rationale
    text: |
      body
  status: done
  authored_at: test
  authored_in: test
  category: maintenance
- id: foo
  kind: backlog-item
  claim: Foo
  justifications:
  - kind: rationale
    text: |
      body
  status: blocked
  authored_at: test
  authored_in: test
  category: maintenance
  blocked_reason: upstream
  dependencies:
  - parent
";
    std::fs::write(plan.join("backlog.yaml"), yaml).unwrap();
}

/// Exit-code contract: `repair-stale-statuses` exits 1 when at least
/// one repair is applied so scripts can detect drift via `$?` without
/// re-parsing the YAML report. Also verifies the mutation actually
/// lands on disk (the unblocked item flips to `active`).
#[test]
fn repair_stale_statuses_exits_one_when_repairs_applied() {
    let tmp = TempDir::new().unwrap();
    seed_unblockable_backlog(tmp.path());

    let out = Command::new(bin())
        .args(["state", "backlog", "repair-stale-statuses"])
        .arg(tmp.path())
        .output()
        .unwrap();
    assert_eq!(
        out.status.code(),
        Some(1),
        "expected exit code 1 when repairs were applied, got status {:?}, stderr: {}",
        out.status,
        String::from_utf8_lossy(&out.stderr)
    );
    let stdout = String::from_utf8(out.stdout).unwrap();
    assert!(
        stdout.contains("task_id: foo") && stdout.contains("dependencies_satisfied"),
        "report must cite repaired item and reason: {stdout}"
    );

    let after = std::fs::read_to_string(tmp.path().join("backlog.yaml")).unwrap();
    assert!(
        after.contains("status: active"),
        "repaired backlog must show status: active, got:\n{after}"
    );
}

/// Exit-code contract: with no drift in the backlog, the verb exits 0
/// and leaves the file untouched.
#[test]
fn repair_stale_statuses_exits_zero_when_no_drift() {
    let tmp = TempDir::new().unwrap();
    let yaml = "\
schema_version: 1
items:
- id: foo
  kind: backlog-item
  claim: Foo
  justifications:
  - kind: rationale
    text: |
      body
  status: active
  authored_at: test
  authored_in: test
  category: maintenance
";
    std::fs::write(tmp.path().join("backlog.yaml"), yaml).unwrap();
    let before = std::fs::read_to_string(tmp.path().join("backlog.yaml")).unwrap();

    let out = Command::new(bin())
        .args(["state", "backlog", "repair-stale-statuses"])
        .arg(tmp.path())
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "expected exit 0 when no repairs were applied, got {:?}, stderr: {}",
        out.status,
        String::from_utf8_lossy(&out.stderr)
    );
    let after = std::fs::read_to_string(tmp.path().join("backlog.yaml")).unwrap();
    assert_eq!(before, after, "backlog must be byte-identical when no repairs applied");
}

/// `--dry-run` reports the repair but must not write.
#[test]
fn repair_stale_statuses_dry_run_does_not_mutate_disk() {
    let tmp = TempDir::new().unwrap();
    seed_unblockable_backlog(tmp.path());
    let before = std::fs::read_to_string(tmp.path().join("backlog.yaml")).unwrap();

    let out = Command::new(bin())
        .args(["state", "backlog", "repair-stale-statuses"])
        .arg(tmp.path())
        .arg("--dry-run")
        .output()
        .unwrap();
    assert_eq!(
        out.status.code(),
        Some(1),
        "dry-run still exits 1 when drift is detected (scripts need the signal)"
    );
    let after = std::fs::read_to_string(tmp.path().join("backlog.yaml")).unwrap();
    assert_eq!(before, after, "dry-run must not mutate disk");
}

#[test]
fn list_format_markdown_emits_deterministic_table_grouped_by_category() {
    let tmp = TempDir::new().unwrap();
    seed_two_task_backlog_yaml(tmp.path());

    let out = Command::new(bin())
        .args(["state", "backlog", "list"])
        .arg(tmp.path())
        .args(["--format", "markdown"])
        .output()
        .expect("failed to spawn ravel-lite");
    assert!(
        out.status.success(),
        "list --format markdown failed: stderr={}",
        String::from_utf8_lossy(&out.stderr)
    );
    let stdout = String::from_utf8(out.stdout).unwrap();
    assert!(stdout.starts_with("## maintenance"), "missing category heading:\n{stdout}");
    assert!(stdout.contains("| title | status | deps |"), "header row missing:\n{stdout}");
    assert!(
        stdout.contains("| Add clippy `-D warnings` CI gate |"),
        "task row missing:\n{stdout}"
    );
}

#[test]
fn list_format_markdown_with_invalid_group_by_rejects() {
    let tmp = TempDir::new().unwrap();
    seed_two_task_backlog_yaml(tmp.path());

    let out = Command::new(bin())
        .args(["state", "backlog", "list"])
        .arg(tmp.path())
        .args(["--format", "markdown", "--group-by", "priority"])
        .output()
        .unwrap();
    assert!(!out.status.success(), "invalid --group-by must fail");
    let stderr = String::from_utf8(out.stderr).unwrap();
    assert!(stderr.contains("priority"), "error must cite the bad value: {stderr}");
}

#[test]
fn list_yaml_truncates_with_metadata_when_limit_below_total() {
    let tmp = TempDir::new().unwrap();
    seed_two_task_backlog_yaml(tmp.path());

    let out = Command::new(bin())
        .args(["state", "backlog", "list"])
        .arg(tmp.path())
        .args(["--limit", "1"])
        .output()
        .unwrap();
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    let stdout = String::from_utf8(out.stdout).unwrap();
    assert!(
        stdout.contains("add-clippy-d-warnings-ci-gate"),
        "first task must be present: {stdout}"
    );
    assert!(
        !stdout.contains("remove-claude-code-debug-file-workaround"),
        "second task must be truncated: {stdout}"
    );
    assert!(stdout.contains("truncated: true"), "{stdout}");
    assert!(stdout.contains("total: 2"), "{stdout}");
    assert!(stdout.contains("returned: 1"), "{stdout}");
    let stderr = String::from_utf8(out.stderr).unwrap();
    assert!(stderr.contains("Showing 1 of 2"), "{stderr}");
}

#[test]
fn list_markdown_format_renders_full_table_regardless_of_limit() {
    // Markdown is for human consumption; truncation flags do not apply
    // because the rendered table is the complete view.
    let tmp = TempDir::new().unwrap();
    seed_two_task_backlog_yaml(tmp.path());

    let out = Command::new(bin())
        .args(["state", "backlog", "list"])
        .arg(tmp.path())
        .args(["--format", "markdown", "--limit", "1"])
        .output()
        .unwrap();
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    let stdout = String::from_utf8(out.stdout).unwrap();
    assert!(
        stdout.contains("Add clippy"),
        "markdown table must include first task: {stdout}"
    );
    assert!(
        stdout.contains("Remove Claude Code"),
        "markdown table must include second task: {stdout}"
    );
}
