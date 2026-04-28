use std::fs;
use std::process::Command;

use tempfile::TempDir;

/// End-to-end through the CLI binary: `repo add` persists to
/// `<context>/repos.yaml`, `list` round-trips, and `remove` deletes.
/// Guards the dispatch layer wiring (clap subcommand enum → repos
/// module handlers).
#[test]
fn repo_add_list_remove_via_binary() {
    let tmp = TempDir::new().unwrap();
    let cfg = tmp.path().join("cfg");
    fs::create_dir_all(&cfg).unwrap();
    let local = tmp.path().join("atlas-checkout");
    fs::create_dir_all(&local).unwrap();

    // add
    let out = Command::new(env!("CARGO_BIN_EXE_ravel-lite"))
        .args(["repo", "add", "--config"])
        .arg(&cfg)
        .args(["atlas", "--url", "git@github.com:antony/atlas.git", "--local-path"])
        .arg(&local)
        .output()
        .expect("binary must spawn");
    assert!(
        out.status.success(),
        "add failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(
        cfg.join("repos.yaml").exists(),
        "repos.yaml should exist after add"
    );

    // list (stdout is YAML)
    let out = Command::new(env!("CARGO_BIN_EXE_ravel-lite"))
        .args(["repo", "list", "--config"])
        .arg(&cfg)
        .output()
        .expect("binary must spawn");
    assert!(
        out.status.success(),
        "list failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("atlas"), "list should mention slug: {stdout}");
    assert!(
        stdout.contains("git@github.com:antony/atlas.git"),
        "list should mention url: {stdout}"
    );
    assert!(
        stdout.contains("schema_version"),
        "list should emit schema_version: {stdout}"
    );

    // remove
    let out = Command::new(env!("CARGO_BIN_EXE_ravel-lite"))
        .args(["repo", "remove", "--config"])
        .arg(&cfg)
        .arg("atlas")
        .output()
        .expect("binary must spawn");
    assert!(
        out.status.success(),
        "remove failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let after_remove: serde_yaml::Value =
        serde_yaml::from_str(&fs::read_to_string(cfg.join("repos.yaml")).unwrap()).unwrap();
    let remaining = after_remove["repos"].as_mapping().unwrap();
    assert!(
        remaining.is_empty(),
        "repos map should be empty after remove: {remaining:?}"
    );
}

/// Adding a slug twice is rejected. The user must remove and re-add
/// rather than mutate in place; this keeps the registry append-only at
/// the slug level.
#[test]
fn repo_add_rejects_duplicate_slug_via_binary() {
    let tmp = TempDir::new().unwrap();
    let cfg = tmp.path().join("cfg");
    fs::create_dir_all(&cfg).unwrap();

    let first = Command::new(env!("CARGO_BIN_EXE_ravel-lite"))
        .args(["repo", "add", "--config"])
        .arg(&cfg)
        .args(["atlas", "--url", "u1"])
        .output()
        .expect("binary must spawn");
    assert!(first.status.success());

    let second = Command::new(env!("CARGO_BIN_EXE_ravel-lite"))
        .args(["repo", "add", "--config"])
        .arg(&cfg)
        .args(["atlas", "--url", "u2"])
        .output()
        .expect("binary must spawn");
    assert!(!second.status.success(), "duplicate add should fail");
    let stderr = String::from_utf8_lossy(&second.stderr);
    assert!(
        stderr.contains("already registered"),
        "stderr should explain the duplicate: {stderr}"
    );
}

/// `repo add` without `--local-path` is valid: future operations clone
/// into the context cache on demand. The on-disk shape must omit the
/// `local_path` field rather than write `null` or an empty string.
#[test]
fn repo_add_without_local_path_omits_field_on_disk() {
    let tmp = TempDir::new().unwrap();
    let cfg = tmp.path().join("cfg");
    fs::create_dir_all(&cfg).unwrap();

    let out = Command::new(env!("CARGO_BIN_EXE_ravel-lite"))
        .args(["repo", "add", "--config"])
        .arg(&cfg)
        .args(["atlas", "--url", "git@example/atlas.git"])
        .output()
        .expect("binary must spawn");
    assert!(out.status.success());

    let yaml = fs::read_to_string(cfg.join("repos.yaml")).unwrap();
    assert!(
        !yaml.contains("local_path"),
        "absent local_path should not appear in YAML; got:\n{yaml}"
    );
}
