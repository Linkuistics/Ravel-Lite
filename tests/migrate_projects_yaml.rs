//! Integration tests for the projects.yaml → repos.yaml cutover error.
//!
//! Any consumer verb that previously read `projects.yaml` must, when
//! the legacy file is on disk and `repos.yaml` is empty, surface the
//! canonical migration message authored in
//! `repos::migrate_projects_yaml_error`. These tests pin both the user-
//! visible CLI behavior and the deprecation alias for `state projects`.

use std::path::PathBuf;
use std::process::Command;

use tempfile::TempDir;

fn bin() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_ravel-lite"))
}

/// Seed a config dir with only the legacy `projects.yaml` (empty list)
/// and no `repos.yaml`. Returns the config dir path.
fn scaffold_legacy_only() -> (TempDir, PathBuf) {
    let tmp = TempDir::new().unwrap();
    let cfg = tmp.path().join("cfg");
    std::fs::create_dir_all(&cfg).unwrap();
    std::fs::write(
        cfg.join("projects.yaml"),
        "schema_version: 1\nprojects: []\n",
    )
    .unwrap();
    (tmp, cfg)
}

#[test]
fn add_edge_emits_migration_error_when_only_projects_yaml_present() {
    let (_tmp, cfg) = scaffold_legacy_only();

    let output = Command::new(bin())
        .args(["state", "related-components", "add-edge"])
        .args(["--config", cfg.to_str().unwrap()])
        .args(["depends-on", "build", "Alpha", "Beta"])
        .args(["--evidence-grade", "weak"])
        .args(["--rationale", "test"])
        .output()
        .unwrap();

    assert!(!output.status.success(), "should fail with migration error");
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("projects.yaml is no longer supported"),
        "stderr must surface migration message; got:\n{stderr}"
    );
    assert!(
        stderr.contains("ravel-lite repo add"),
        "stderr must show the migration command; got:\n{stderr}"
    );
}

#[test]
fn add_proposal_emits_migration_error_when_only_projects_yaml_present() {
    let (_tmp, cfg) = scaffold_legacy_only();

    let output = Command::new(bin())
        .args(["state", "discover-proposals", "add-proposal"])
        .args(["--config", cfg.to_str().unwrap()])
        .args(["--kind", "depends-on"])
        .args(["--lifecycle", "build"])
        .args(["--participant", "Alpha", "--participant", "Beta"])
        .args(["--evidence-grade", "weak"])
        .args(["--rationale", "test"])
        .output()
        .unwrap();

    assert!(!output.status.success(), "should fail with migration error");
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("projects.yaml is no longer supported"),
        "stderr must surface migration message; got:\n{stderr}"
    );
}

#[test]
fn state_projects_alias_prints_migration_message() {
    // Any form of `state projects ...` must surface the migration error
    // — the deprecated subcommand is preserved only as an alias.
    let (_tmp, cfg) = scaffold_legacy_only();

    let output = Command::new(bin())
        .args(["state", "projects", "list"])
        .args(["--config", cfg.to_str().unwrap()])
        .output()
        .unwrap();

    assert!(!output.status.success(), "deprecation alias must exit non-zero");
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("projects.yaml is no longer supported"),
        "stderr must surface migration message; got:\n{stderr}"
    );
    assert!(
        stderr.contains("ravel-lite repo add"),
        "stderr must show the migration command; got:\n{stderr}"
    );
}

#[test]
fn add_edge_succeeds_when_legacy_file_present_but_repos_yaml_populated() {
    // User has begun migrating: legacy file still on disk but at least
    // one repo entry registered. The migration error must NOT fire —
    // catch only the "not yet started" case, not "in flight".
    let (_tmp, cfg) = scaffold_legacy_only();

    // Register both components so add-edge can validate participants.
    for slug in ["Alpha", "Beta"] {
        let status = Command::new(bin())
            .args(["repo", "add", "--config"])
            .arg(&cfg)
            .arg(slug)
            .args(["--url", "test-url"])
            .status()
            .unwrap();
        assert!(status.success());
    }

    let status = Command::new(bin())
        .args(["state", "related-components", "add-edge"])
        .args(["--config", cfg.to_str().unwrap()])
        .args(["depends-on", "build", "Alpha", "Beta"])
        .args(["--evidence-grade", "weak"])
        .args(["--rationale", "test"])
        .status()
        .unwrap();
    assert!(
        status.success(),
        "with repos.yaml populated, add-edge must succeed despite legacy projects.yaml on disk"
    );
}
