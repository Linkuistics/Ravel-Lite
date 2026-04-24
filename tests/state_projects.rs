use std::fs;
use std::process::Command;

use tempfile::TempDir;

/// End-to-end through the CLI binary: `state projects add` persists to
/// `<config>/projects.yaml`, `list` round-trips, `rename` mutates in
/// place, and `remove` deletes. Guards the CLI dispatch layer wiring
/// (clap subcommand enum → projects module handlers).
#[test]
fn state_projects_add_list_rename_remove_via_binary() {
    let tmp = TempDir::new().unwrap();
    let cfg = tmp.path().join("cfg");
    fs::create_dir_all(&cfg).unwrap();
    let project = tmp.path().join("some-project");
    fs::create_dir_all(&project).unwrap();

    // add
    let out = Command::new(env!("CARGO_BIN_EXE_ravel-lite"))
        .args(["state", "projects", "add", "--config"])
        .arg(&cfg)
        .args(["--name", "some-project", "--path"])
        .arg(&project)
        .output()
        .expect("binary must spawn");
    assert!(
        out.status.success(),
        "add failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(
        cfg.join("projects.yaml").exists(),
        "projects.yaml should exist after add"
    );

    // list (stdout is YAML)
    let out = Command::new(env!("CARGO_BIN_EXE_ravel-lite"))
        .args(["state", "projects", "list", "--config"])
        .arg(&cfg)
        .output()
        .expect("binary must spawn");
    assert!(
        out.status.success(),
        "list failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        stdout.contains("some-project"),
        "list should mention the project: {stdout}"
    );
    assert!(
        stdout.contains("schema_version"),
        "list should emit schema_version: {stdout}"
    );

    // rename
    let out = Command::new(env!("CARGO_BIN_EXE_ravel-lite"))
        .args(["state", "projects", "rename", "--config"])
        .arg(&cfg)
        .args(["some-project", "renamed-project"])
        .output()
        .expect("binary must spawn");
    assert!(
        out.status.success(),
        "rename failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    // Parse the YAML because the path still contains "some-project" as
    // its basename; only the `name:` field should have changed.
    let after_rename: serde_yaml::Value =
        serde_yaml::from_str(&fs::read_to_string(cfg.join("projects.yaml")).unwrap()).unwrap();
    let names: Vec<&str> = after_rename["projects"]
        .as_sequence()
        .unwrap()
        .iter()
        .map(|e| e["name"].as_str().unwrap())
        .collect();
    assert_eq!(
        names,
        vec!["renamed-project"],
        "only the name should have changed"
    );

    // remove
    let out = Command::new(env!("CARGO_BIN_EXE_ravel-lite"))
        .args(["state", "projects", "remove", "--config"])
        .arg(&cfg)
        .arg("renamed-project")
        .output()
        .expect("binary must spawn");
    assert!(
        out.status.success(),
        "remove failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let after_remove: serde_yaml::Value =
        serde_yaml::from_str(&fs::read_to_string(cfg.join("projects.yaml")).unwrap()).unwrap();
    let remaining = after_remove["projects"].as_sequence().unwrap();
    assert!(
        remaining.is_empty(),
        "projects list should be empty after remove: {remaining:?}"
    );
}

/// `state projects add` with a relative path resolves the input
/// against the child process's CWD (not `<config_root>`), then stores
/// the resulting location in `projects.yaml` in the portable form
/// (relative to the directory containing `projects.yaml`).
/// `Command::current_dir` scopes the CWD change to the child so this
/// test is safe under parallel execution.
#[test]
fn state_projects_add_relative_path_resolves_against_cwd_via_binary() {
    let tmp = TempDir::new().unwrap();
    let cfg = tmp.path().join("cfg");
    fs::create_dir_all(&cfg).unwrap();
    // Target project directory must exist relative to the spawn CWD.
    let workdir = tmp.path().join("workdir");
    fs::create_dir_all(workdir.join("rel-target")).unwrap();

    let out = Command::new(env!("CARGO_BIN_EXE_ravel-lite"))
        .current_dir(&workdir)
        .args(["state", "projects", "add", "--config"])
        .arg(&cfg)
        .args(["--name", "rel", "--path", "rel-target"])
        .output()
        .expect("binary must spawn");
    assert!(
        out.status.success(),
        "add failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );

    let catalog: serde_yaml::Value =
        serde_yaml::from_str(&fs::read_to_string(cfg.join("projects.yaml")).unwrap()).unwrap();
    let stored_path = catalog["projects"][0]["path"].as_str().unwrap();
    // On-disk form is relative to the directory containing projects.yaml.
    // Resolving it against `cfg` must yield the project dir under the
    // spawn CWD — that's the proof the CLI used CWD (not cfg) for input
    // resolution before relativising for storage.
    let resolved = cfg.join(stored_path);
    let cleaned = resolved
        .components()
        .fold(std::path::PathBuf::new(), |mut acc, c| match c {
            std::path::Component::ParentDir => {
                acc.pop();
                acc
            }
            std::path::Component::CurDir => acc,
            other => {
                acc.push(other);
                acc
            }
        });
    assert!(
        cleaned.ends_with("workdir/rel-target"),
        "resolved path must reflect CWD-based input resolution, got {}",
        cleaned.display()
    );
}

/// `state projects add` accepts `--path` with no `--name`, defaulting
/// the name to the resolved path's basename.
#[test]
fn state_projects_add_defaults_name_to_basename_via_binary() {
    let tmp = TempDir::new().unwrap();
    let cfg = tmp.path().join("cfg");
    fs::create_dir_all(&cfg).unwrap();
    let project = tmp.path().join("derived-name");
    fs::create_dir_all(&project).unwrap();

    let out = Command::new(env!("CARGO_BIN_EXE_ravel-lite"))
        .args(["state", "projects", "add", "--config"])
        .arg(&cfg)
        .arg("--path")
        .arg(&project)
        .output()
        .expect("binary must spawn");
    assert!(
        out.status.success(),
        "add failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );

    let catalog: serde_yaml::Value =
        serde_yaml::from_str(&fs::read_to_string(cfg.join("projects.yaml")).unwrap()).unwrap();
    assert_eq!(
        catalog["projects"][0]["name"].as_str().unwrap(),
        "derived-name"
    );
}
