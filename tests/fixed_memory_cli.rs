//! End-to-end CLI tests for the `fixed-memory` command tree.
//!
//! These exercise the binary against a temp config dir so the layered
//! overlay (embedded + user) is verified through the same code path the
//! LLM hits during a real phase invocation. Unit-level coverage of
//! `discover`, `compose`, and `extract_description` lives next to the
//! implementation in `src/fixed_memory.rs`.

use std::fs;
use std::process::Command;

use tempfile::TempDir;

/// Path within the config dir where user overrides are loaded from.
const USER_OVERLAY_DIR: &str = "fixed-memory";

fn binary() -> Command {
    Command::new(env!("CARGO_BIN_EXE_ravel-lite"))
}

fn config_dir() -> TempDir {
    let dir = TempDir::new().expect("temp dir creates");
    ravel_lite::init::run_init(dir.path(), false).expect("init scaffolds context");
    dir
}

fn write_user_entry(config: &TempDir, slug: &str, body: &str) {
    let dir = config.path().join(USER_OVERLAY_DIR);
    fs::create_dir_all(&dir).unwrap();
    fs::write(dir.join(format!("{slug}.md")), body).unwrap();
}

#[test]
fn list_yaml_default_format_emits_schema_version_and_embedded_entries() {
    let cfg = config_dir();

    let out = binary()
        .args(["fixed-memory", "list", "--config"])
        .arg(cfg.path())
        .output()
        .expect("binary must spawn");
    assert!(out.status.success(), "list must exit 0: {out:?}");
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("schema_version: 1"), "stdout: {stdout}");
    // Every shipped slug should surface as embedded-only when there is
    // no user overlay. `coding-style` and `memory-style` are the
    // canonical ones — both must round-trip through the binary.
    assert!(stdout.contains("slug: coding-style"), "stdout: {stdout}");
    assert!(stdout.contains("slug: memory-style"), "stdout: {stdout}");
}

#[test]
fn list_renders_user_only_slug_with_user_source_label() {
    let cfg = config_dir();
    write_user_entry(&cfg, "coding-style-haskell", "# Haskell coding style\n");

    let out = binary()
        .args(["fixed-memory", "list", "--config"])
        .arg(cfg.path())
        .arg("--format")
        .arg("yaml")
        .output()
        .expect("binary must spawn");
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        stdout.contains("slug: coding-style-haskell"),
        "user-only slug missing from list: {stdout}"
    );
    // Yaml-encoded `sources: [user]` is rendered as a `- user` line by
    // serde_yaml's default block-style output. Match on the line rather
    // than the inline form to avoid a serialiser-specific pin.
    let haskell_block = stdout
        .split("- slug: coding-style-haskell")
        .nth(1)
        .expect("haskell entry must follow its slug header");
    let user_only_window: String = haskell_block.lines().take(6).collect::<Vec<_>>().join("\n");
    assert!(
        user_only_window.contains("- user"),
        "expected `- user` source label near haskell entry; got window: {user_only_window}"
    );
    assert!(
        !user_only_window.contains("- embedded"),
        "user-only entry must not carry embedded source label: {user_only_window}"
    );
}

#[test]
fn list_records_both_sources_for_overlapping_slug() {
    let cfg = config_dir();
    write_user_entry(&cfg, "coding-style-rust", "# my override\n");

    let out = binary()
        .args(["fixed-memory", "list", "--config"])
        .arg(cfg.path())
        .arg("--format")
        .arg("json")
        .output()
        .expect("binary must spawn");
    assert!(out.status.success(), "list --format json must exit 0");
    let stdout = String::from_utf8_lossy(&out.stdout);
    let parsed: serde_json::Value =
        serde_json::from_str(&stdout).expect("json output must parse");
    let entries = parsed["entries"]
        .as_array()
        .expect("entries field must be a json array");
    let rust = entries
        .iter()
        .find(|e| e["slug"] == "coding-style-rust")
        .expect("coding-style-rust slug must be in list output");
    let sources: Vec<&str> = rust["sources"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_str().unwrap())
        .collect();
    assert_eq!(sources, vec!["embedded", "user"]);
}

#[test]
fn list_markdown_renders_table_with_header_and_rows() {
    let cfg = config_dir();

    let out = binary()
        .args(["fixed-memory", "list", "--config"])
        .arg(cfg.path())
        .arg("--format")
        .arg("markdown")
        .output()
        .expect("binary must spawn");
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        stdout.starts_with("| slug | description | sources |\n|---|---|---|\n"),
        "markdown table header missing: {stdout}"
    );
    assert!(
        stdout.contains("| coding-style |"),
        "embedded slug must surface as a table row: {stdout}"
    );
}

#[test]
fn list_invalid_format_exits_nonzero_with_actionable_message() {
    let cfg = config_dir();

    let out = binary()
        .args(["fixed-memory", "list", "--config"])
        .arg(cfg.path())
        .arg("--format")
        .arg("toml")
        .output()
        .expect("binary must spawn");
    assert!(
        !out.status.success(),
        "unknown --format must exit non-zero"
    );
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("yaml") && stderr.contains("json") && stderr.contains("markdown"),
        "stderr must name the accepted formats: {stderr}"
    );
}

#[test]
fn show_embedded_only_emits_unchanged_body_without_delimiter() {
    let cfg = config_dir();

    let out = binary()
        .args(["fixed-memory", "show", "--config"])
        .arg(cfg.path())
        .arg("coding-style")
        .output()
        .expect("binary must spawn");
    assert!(out.status.success(), "show must exit 0");
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.starts_with("# Universal Coding Style"));
    assert!(
        !stdout.contains("User addendum"),
        "embedded-only show must not insert addendum delimiter"
    );
}

#[test]
fn show_user_only_emits_user_body_without_delimiter() {
    let cfg = config_dir();
    let body = "# Haskell coding style\nlines\n";
    write_user_entry(&cfg, "coding-style-haskell", body);

    let out = binary()
        .args(["fixed-memory", "show", "--config"])
        .arg(cfg.path())
        .arg("coding-style-haskell")
        .output()
        .expect("binary must spawn");
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert_eq!(stdout, body);
}

#[test]
fn show_both_layers_emits_embedded_then_delimiter_then_user() {
    let cfg = config_dir();
    let user_body = "# my override\nuser body\n";
    write_user_entry(&cfg, "coding-style-rust", user_body);

    let out = binary()
        .args(["fixed-memory", "show", "--config"])
        .arg(cfg.path())
        .arg("coding-style-rust")
        .output()
        .expect("binary must spawn");
    let stdout = String::from_utf8_lossy(&out.stdout);
    let split: Vec<&str> = stdout
        .split("\n---\n\n## User addendum (takes precedence over the above)\n\n")
        .collect();
    assert_eq!(
        split.len(),
        2,
        "delimiter must appear exactly once between layers; got: {stdout}"
    );
    assert!(
        split[0].contains("Rust Coding Style"),
        "embedded body must precede the delimiter; got: {}",
        split[0]
    );
    assert!(
        split[1].starts_with("# my override"),
        "user body must follow the delimiter; got: {}",
        split[1]
    );
}

#[test]
fn show_unknown_slug_exits_nonzero_and_names_available_slugs() {
    let cfg = config_dir();

    let out = binary()
        .args(["fixed-memory", "show", "--config"])
        .arg(cfg.path())
        .arg("does-not-exist-slug")
        .output()
        .expect("binary must spawn");
    assert!(
        !out.status.success(),
        "unknown slug must exit non-zero"
    );
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("does-not-exist-slug"),
        "stderr must name the failing slug: {stderr}"
    );
    assert!(
        stderr.contains("Available slugs:"),
        "stderr must surface remediation: {stderr}"
    );
    assert!(
        stderr.contains("coding-style"),
        "stderr must include at least one shipped slug as remediation: {stderr}"
    );
}
