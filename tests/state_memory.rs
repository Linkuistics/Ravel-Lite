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

fn seed_three_entry_memory(plan_dir: &std::path::Path) {
    std::fs::write(
        plan_dir.join("memory.yaml"),
        "schema_version: 1\nitems: []\n",
    )
    .unwrap();
    for (title, body) in [
        ("First insight", "Body one.\n"),
        ("Second insight", "Body two.\n"),
        ("Third insight", "Body three.\n"),
    ] {
        let out = Command::new(bin())
            .args(["state", "memory", "add"])
            .arg(plan_dir)
            .args(["--title", title, "--body", body])
            .output()
            .unwrap();
        assert!(
            out.status.success(),
            "seed add failed: {}",
            String::from_utf8_lossy(&out.stderr)
        );
    }
}

#[test]
fn list_limit_truncates_yaml_output_with_metadata() {
    let tmp = TempDir::new().unwrap();
    seed_three_entry_memory(tmp.path());

    let out = Command::new(bin())
        .args(["state", "memory", "list"])
        .arg(tmp.path())
        .args(["--limit", "1"])
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "list --limit 1 failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let stdout = String::from_utf8(out.stdout).unwrap();
    assert!(
        stdout.contains("first-insight"),
        "first entry must be present: {stdout}"
    );
    assert!(
        !stdout.contains("third-insight"),
        "third entry must be truncated out: {stdout}"
    );
    assert!(
        stdout.contains("truncated: true"),
        "truncation metadata must be present: {stdout}"
    );
    assert!(stdout.contains("total: 3"), "total must be present: {stdout}");
    assert!(
        stdout.contains("returned: 1"),
        "returned must be present: {stdout}"
    );
    let stderr = String::from_utf8(out.stderr).unwrap();
    assert!(
        stderr.contains("Showing 1 of 3"),
        "stderr must carry the human truncation summary: {stderr}"
    );
}

#[test]
fn list_all_overrides_default_and_emits_no_truncation_metadata() {
    let tmp = TempDir::new().unwrap();
    seed_three_entry_memory(tmp.path());

    let out = Command::new(bin())
        .args(["state", "memory", "list"])
        .arg(tmp.path())
        .arg("--all")
        .output()
        .unwrap();
    assert!(out.status.success());
    let stdout = String::from_utf8(out.stdout).unwrap();
    assert!(stdout.contains("first-insight"), "{stdout}");
    assert!(stdout.contains("second-insight"), "{stdout}");
    assert!(stdout.contains("third-insight"), "{stdout}");
    assert!(
        !stdout.contains("truncated"),
        "untruncated output must not carry truncation metadata: {stdout}"
    );
}

#[test]
fn list_limit_and_all_are_mutually_exclusive_at_the_clap_layer() {
    let tmp = TempDir::new().unwrap();
    seed_three_entry_memory(tmp.path());

    let out = Command::new(bin())
        .args(["state", "memory", "list"])
        .arg(tmp.path())
        .args(["--limit", "1"])
        .arg("--all")
        .output()
        .unwrap();
    assert!(
        !out.status.success(),
        "--limit and --all together must fail at clap parse"
    );
    let stderr = String::from_utf8(out.stderr).unwrap();
    assert!(
        stderr.contains("cannot be used with"),
        "stderr must explain the mutex: {stderr}"
    );
}

#[test]
fn list_without_limit_or_all_remains_unbounded_for_backwards_compat() {
    let tmp = TempDir::new().unwrap();
    seed_three_entry_memory(tmp.path());

    let out = Command::new(bin())
        .args(["state", "memory", "list"])
        .arg(tmp.path())
        .output()
        .unwrap();
    assert!(out.status.success());
    let stdout = String::from_utf8(out.stdout).unwrap();
    // Default behaviour is unchanged from the pre-pagination shape: no
    // truncation metadata, every entry present.
    assert!(stdout.contains("first-insight"), "{stdout}");
    assert!(stdout.contains("third-insight"), "{stdout}");
    assert!(
        !stdout.contains("truncated"),
        "default list must not emit truncation metadata: {stdout}"
    );
}
