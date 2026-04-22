//! End-to-end CLI integration tests for `ravel-lite state
//! related-projects *` and `ravel-lite state migrate-related-projects`.
//! Shells out to the built binary via CARGO_BIN_EXE_ravel-lite, matching
//! the pattern in tests/integration.rs.

use std::path::{Path, PathBuf};
use std::process::Command;

use tempfile::TempDir;

fn bin() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_ravel-lite"))
}

/// Scaffold: `<tmp>/cfg/` for the config dir, `<tmp>/projects/<name>/`
/// for each project directory, and a plan dir at
/// `<tmp>/projects/<me>/LLM_STATE/core`. Seeds the projects catalog with
/// `me` plus the listed peers.
fn scaffold(tmp: &Path, me: &str, peers: &[&str]) -> (PathBuf, PathBuf) {
    let cfg = tmp.join("cfg");
    std::fs::create_dir_all(&cfg).unwrap();
    let projects_root = tmp.join("projects");

    for name in std::iter::once(me).chain(peers.iter().copied()) {
        let p = projects_root.join(name);
        std::fs::create_dir_all(&p).unwrap();
        let status = Command::new(bin())
            .args(["state", "projects", "add"])
            .args(["--config", cfg.to_str().unwrap()])
            .args(["--name", name])
            .args(["--path", p.to_str().unwrap()])
            .status()
            .unwrap();
        assert!(status.success(), "seed projects add failed for {name}");
    }

    let plan_dir = projects_root.join(me).join("LLM_STATE").join("core");
    std::fs::create_dir_all(&plan_dir).unwrap();
    (cfg, plan_dir)
}

#[test]
fn add_list_remove_round_trip_through_cli() {
    let tmp = TempDir::new().unwrap();
    let (cfg, _plan_dir) = scaffold(tmp.path(), "Me", &["Peer"]);

    let add = Command::new(bin())
        .args(["state", "related-projects", "add-edge"])
        .args(["--config", cfg.to_str().unwrap()])
        .args(["sibling", "Me", "Peer"])
        .output()
        .unwrap();
    assert!(
        add.status.success(),
        "add-edge failed: stderr={}",
        String::from_utf8_lossy(&add.stderr)
    );
    assert!(cfg.join("related-projects.yaml").exists());

    let list = Command::new(bin())
        .args(["state", "related-projects", "list"])
        .args(["--config", cfg.to_str().unwrap()])
        .output()
        .unwrap();
    assert!(list.status.success());
    let yaml = String::from_utf8(list.stdout).unwrap();
    assert!(yaml.contains("kind: sibling"), "list output: {yaml}");
    assert!(yaml.contains("Me"), "list output: {yaml}");
    assert!(yaml.contains("Peer"), "list output: {yaml}");

    let remove = Command::new(bin())
        .args(["state", "related-projects", "remove-edge"])
        .args(["--config", cfg.to_str().unwrap()])
        // Reverse participant order to exercise sibling canonicalisation.
        .args(["sibling", "Peer", "Me"])
        .output()
        .unwrap();
    assert!(
        remove.status.success(),
        "remove-edge failed: stderr={}",
        String::from_utf8_lossy(&remove.stderr)
    );

    let list2 = Command::new(bin())
        .args(["state", "related-projects", "list"])
        .args(["--config", cfg.to_str().unwrap()])
        .output()
        .unwrap();
    let yaml2 = String::from_utf8(list2.stdout).unwrap();
    assert!(!yaml2.contains("kind: sibling"), "edge must be gone: {yaml2}");
}

#[test]
fn add_edge_rejects_unknown_project_via_cli() {
    let tmp = TempDir::new().unwrap();
    let (cfg, _plan_dir) = scaffold(tmp.path(), "Me", &[]);

    let add = Command::new(bin())
        .args(["state", "related-projects", "add-edge"])
        .args(["--config", cfg.to_str().unwrap()])
        .args(["sibling", "Me", "Ghost"])
        .output()
        .unwrap();
    assert!(!add.status.success(), "unknown project must fail");
    let stderr = String::from_utf8(add.stderr).unwrap();
    assert!(stderr.contains("Ghost"), "stderr must name the missing project: {stderr}");
}

#[test]
fn migrate_related_projects_round_trips_from_md() {
    let tmp = TempDir::new().unwrap();
    let (cfg, plan_dir) = scaffold(tmp.path(), "Me", &["Peer", "Up", "Down"]);
    let projects_root = tmp.path().join("projects");

    let body = format!(
        "# Related Plans\n\n\
         ## Siblings\n- {peer} — peer\n\n\
         ## Parents\n- {up} — upstream\n\n\
         ## Children\n- {down} — downstream\n",
        peer = projects_root.join("Peer").display(),
        up = projects_root.join("Up").display(),
        down = projects_root.join("Down").display(),
    );
    std::fs::write(plan_dir.join("related-plans.md"), body).unwrap();

    let out = Command::new(bin())
        .args(["state", "migrate-related-projects"])
        .arg(&plan_dir)
        .args(["--config", cfg.to_str().unwrap()])
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "migrate failed: stderr={}",
        String::from_utf8_lossy(&out.stderr)
    );
    let stdout = String::from_utf8(out.stdout).unwrap();
    assert!(stdout.contains("added sibling edge"), "stdout: {stdout}");
    assert!(stdout.contains("added parent-of edge"), "stdout: {stdout}");

    // Re-running is a no-op — every edge becomes `already present`.
    let again = Command::new(bin())
        .args(["state", "migrate-related-projects"])
        .arg(&plan_dir)
        .args(["--config", cfg.to_str().unwrap()])
        .output()
        .unwrap();
    assert!(again.status.success());
    let stdout2 = String::from_utf8(again.stdout).unwrap();
    assert!(
        !stdout2.contains("added "),
        "second run must add nothing: {stdout2}"
    );
    assert!(stdout2.contains("already present"), "stdout: {stdout2}");
}

#[test]
fn list_with_plan_filter_restricts_to_plan_project_edges() {
    let tmp = TempDir::new().unwrap();
    let (cfg, plan_dir) = scaffold(tmp.path(), "Me", &["Peer", "Other", "Third"]);

    // Two edges: one involves Me, one doesn't.
    for (a, b) in [("Me", "Peer"), ("Other", "Third")] {
        let status = Command::new(bin())
            .args(["state", "related-projects", "add-edge"])
            .args(["--config", cfg.to_str().unwrap()])
            .args(["sibling", a, b])
            .status()
            .unwrap();
        assert!(status.success());
    }

    let list = Command::new(bin())
        .args(["state", "related-projects", "list"])
        .args(["--config", cfg.to_str().unwrap()])
        .args(["--plan", plan_dir.to_str().unwrap()])
        .output()
        .unwrap();
    assert!(list.status.success());
    let yaml = String::from_utf8(list.stdout).unwrap();
    assert!(yaml.contains("Peer"), "filtered output must contain Peer: {yaml}");
    assert!(!yaml.contains("Other"), "filtered output must exclude non-Me edges: {yaml}");
}
