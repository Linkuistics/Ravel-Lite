use std::fs;
use std::path::Path;

use anyhow::{Context, Result};

use crate::config::CONFIG_ENV_VAR;

struct EmbeddedFile {
    path: &'static str,
    content: &'static str,
}

/// Lookup a tool-shipped default by its `defaults/`-relative path.
/// Returns `None` if the path is not registered.
///
/// This is the single runtime entry point for prompts and agent
/// definitions: every reader that used to `fs::read_to_string()`
/// against the config dir now goes through here, so the embedded set
/// is the only source of truth. The `every_*_path_resolves` drift
/// guard relies on this being the only accessor.
pub fn embedded_content(relative_path: &str) -> Option<&'static str> {
    EMBEDDED_FILES
        .iter()
        .find(|f| f.path == relative_path)
        .map(|f| f.content)
}

/// Like [`embedded_content`] but errors with a deterministic message
/// when the path is missing. Use this at every callsite where the
/// file is shipped under `defaults/` and a missing entry would be a
/// drift bug rather than a runtime miss.
pub fn require_embedded(relative_path: &str) -> Result<&'static str> {
    embedded_content(relative_path).with_context(|| {
        format!(
            "Embedded default not registered for path '{relative_path}'. \
             This is a drift bug: the path must be added to EMBEDDED_FILES \
             in src/init.rs."
        )
    })
}

/// Iterate `(path, content)` pairs whose path starts with `prefix`.
/// Used by readers that scan a directory of shipped defaults — e.g.
/// the pi subagent dispatcher previously walked
/// `<config>/agents/pi/subagents/`.
pub fn embedded_entries_with_prefix(
    prefix: &str,
) -> impl Iterator<Item = (&'static str, &'static str)> + '_ {
    EMBEDDED_FILES
        .iter()
        .filter(move |f| f.path.starts_with(prefix))
        .map(|f| (f.path, f.content))
}

const EMBEDDED_FILES: &[EmbeddedFile] = &[
    EmbeddedFile { path: "config.yaml", content: include_str!("../defaults/config.yaml") },
    EmbeddedFile { path: "agents/claude-code/config.yaml", content: include_str!("../defaults/agents/claude-code/config.yaml") },
    EmbeddedFile { path: "agents/claude-code/tokens.yaml", content: include_str!("../defaults/agents/claude-code/tokens.yaml") },
    EmbeddedFile { path: "agents/pi/config.yaml", content: include_str!("../defaults/agents/pi/config.yaml") },
    EmbeddedFile { path: "agents/pi/tokens.yaml", content: include_str!("../defaults/agents/pi/tokens.yaml") },
    EmbeddedFile { path: "agents/pi/prompts/system-prompt.md", content: include_str!("../defaults/agents/pi/prompts/system-prompt.md") },
    EmbeddedFile { path: "agents/pi/prompts/memory-prompt.md", content: include_str!("../defaults/agents/pi/prompts/memory-prompt.md") },
    EmbeddedFile { path: "phases/work.md", content: include_str!("../defaults/phases/work.md") },
    EmbeddedFile { path: "phases/analyse-work.md", content: include_str!("../defaults/phases/analyse-work.md") },
    EmbeddedFile { path: "phases/reflect.md", content: include_str!("../defaults/phases/reflect.md") },
    EmbeddedFile { path: "phases/dream.md", content: include_str!("../defaults/phases/dream.md") },
    EmbeddedFile { path: "phases/triage.md", content: include_str!("../defaults/phases/triage.md") },
    EmbeddedFile { path: "fixed-memory/coding-style.md", content: include_str!("../defaults/fixed-memory/coding-style.md") },
    EmbeddedFile { path: "fixed-memory/coding-style-rust.md", content: include_str!("../defaults/fixed-memory/coding-style-rust.md") },
    EmbeddedFile { path: "fixed-memory/coding-style-swift.md", content: include_str!("../defaults/fixed-memory/coding-style-swift.md") },
    EmbeddedFile { path: "fixed-memory/coding-style-typescript.md", content: include_str!("../defaults/fixed-memory/coding-style-typescript.md") },
    EmbeddedFile { path: "fixed-memory/coding-style-python.md", content: include_str!("../defaults/fixed-memory/coding-style-python.md") },
    EmbeddedFile { path: "fixed-memory/coding-style-bash.md", content: include_str!("../defaults/fixed-memory/coding-style-bash.md") },
    EmbeddedFile { path: "fixed-memory/coding-style-elixir.md", content: include_str!("../defaults/fixed-memory/coding-style-elixir.md") },
    EmbeddedFile { path: "fixed-memory/memory-style.md", content: include_str!("../defaults/fixed-memory/memory-style.md") },
    EmbeddedFile { path: "agents/pi/subagents/brainstorming.md", content: include_str!("../defaults/agents/pi/subagents/brainstorming.md") },
    EmbeddedFile { path: "agents/pi/subagents/tdd.md", content: include_str!("../defaults/agents/pi/subagents/tdd.md") },
    EmbeddedFile { path: "agents/pi/subagents/writing-plans.md", content: include_str!("../defaults/agents/pi/subagents/writing-plans.md") },
    EmbeddedFile { path: "survey.md", content: include_str!("../defaults/survey.md") },
    EmbeddedFile { path: "survey-incremental.md", content: include_str!("../defaults/survey-incremental.md") },
    EmbeddedFile { path: "create-plan.md", content: include_str!("../defaults/create-plan.md") },
    EmbeddedFile { path: "discover-stage1.md", content: include_str!("../defaults/discover-stage1.md") },
    EmbeddedFile { path: "discover-stage2.md", content: include_str!("../defaults/discover-stage2.md") },
    EmbeddedFile { path: "ontology.yaml", content: include_str!("../defaults/ontology.yaml") },
];

/// Paths that used to ship via `EMBEDDED_FILES` but have been removed
/// or renamed. `init --force` deletes these from the target dir so
/// existing configs catch up to the current layout without manual
/// cleanup. Keep the list narrow: only add an entry when we are sure
/// the path was once ours and a user could not legitimately be keeping
/// it for their own purposes.
///
/// The bulk cleanup of formerly-materialised defaults (`config.yaml`,
/// `phases/*.md`, etc.) is owned by the v1→v2 migration task — `init`
/// deliberately leaves those in place so an upgrade does not
/// disturb a user's tree without an explicit migration step.
const RETIRED_PATHS: &[&str] = &[
    // Former location of pi subagent prompts; moved to
    // `agents/pi/subagents/` as part of the drift-guard cleanup.
    "skills",
];

/// Filename of the optional starter Lua config stub written into a
/// fresh config dir. Contents are entirely commented out — the runtime
/// works correctly against an empty file or no file at all, so the
/// stub is purely a discoverability aid for users who want to start
/// customising.
const CONFIG_LUA_FILENAME: &str = "config.lua";

const CONFIG_LUA_STUB: &str = r#"-- ravel-lite config (Lua)
--
-- This file is optional. Out of the box, ravel-lite reads every
-- shipped prompt and agent definition from its embedded set, so an
-- empty config dir works out of the box.
--
-- Customise behaviour by uncommenting and editing the calls below.
-- See `ravel-lite reference config-and-prompts` for the full API.
--
-- ravel.set_agent("claude-code")
-- ravel.set_model("work", "claude-opus-4-7")
-- ravel.append_prompt("work", [[
--   Project-specific guidance appended after the embedded work prompt.
-- ]])
"#;

/// Initialise (or refresh) a config dir. Materialisation of shipped
/// defaults has been removed: prompts and agent definitions are read
/// from the embedded set at runtime, so the config dir holds only
/// user-owned files.
///
/// `force` no longer rewrites shipped files (there is nothing to
/// rewrite); it only prunes paths in `RETIRED_PATHS`. The
/// `config.lua` stub is preserved on `--force` so a user's edits are
/// not silently overwritten.
pub fn run_init(target_dir: &Path, force: bool) -> Result<()> {
    fs::create_dir_all(target_dir)
        .with_context(|| format!("Failed to create {}", target_dir.display()))?;

    let stub_path = target_dir.join(CONFIG_LUA_FILENAME);
    let stub_existed = stub_path.exists();
    if !stub_existed {
        fs::write(&stub_path, CONFIG_LUA_STUB)
            .with_context(|| format!("Failed to write {}", stub_path.display()))?;
    }

    let mut pruned = 0;
    if force {
        for retired in RETIRED_PATHS {
            let path = target_dir.join(retired);
            if !path.exists() {
                continue;
            }
            if path.is_dir() {
                fs::remove_dir_all(&path).with_context(|| {
                    format!("Failed to prune retired dir {}", path.display())
                })?;
            } else {
                fs::remove_file(&path).with_context(|| {
                    format!("Failed to prune retired file {}", path.display())
                })?;
            }
            pruned += 1;
            println!("  ✗ Pruned retired path: {retired}");
        }
    }

    if force {
        println!("  ✓ Init --force complete: {pruned} pruned (shipped defaults are embedded; nothing to refresh on disk)");
    } else if stub_existed {
        println!("  ✓ Config dir already initialised at {}", target_dir.display());
    } else {
        println!("  ✓ Wrote starter {CONFIG_LUA_FILENAME} (all settings commented out)");
    }

    print_discovery_guidance(target_dir);
    Ok(())
}

/// After scaffolding, tell the user how to make `ravel-lite` find this
/// config. Silent when the target is already the XDG default, since the
/// binary will find it there with no setup.
fn print_discovery_guidance(target_dir: &Path) {
    let xdg_default = dirs::config_dir().map(|p| p.join("ravel-lite"));
    let is_xdg_default = xdg_default.as_deref() == Some(target_dir);

    println!();
    if is_xdg_default {
        println!(
            "  Config is at the default location; ravel-lite will discover it automatically."
        );
    } else {
        println!("  To use this config as the default for ravel-lite, set:");
        println!("    export {CONFIG_ENV_VAR}={}", target_dir.display());
        println!("  Or pass --config {} on each invocation.", target_dir.display());
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn init_creates_dir_and_lua_stub_only() {
        // Slimmed `init`: no shipped defaults are materialised — those
        // live in the embedded set. The config dir gets created and a
        // commented-out `config.lua` stub lands so users have an
        // obvious starting point for customisation.
        let dir = TempDir::new().unwrap();
        let target = dir.path().join("my-config");
        run_init(&target, false).unwrap();

        assert!(target.is_dir(), "config dir must exist after init");
        assert!(target.join(CONFIG_LUA_FILENAME).exists(), "init must write the config.lua stub");
        assert!(
            !target.join("config.yaml").exists(),
            "init must not materialise the shipped config.yaml — defaults are embedded"
        );
        assert!(
            !target.join("phases").exists(),
            "init must not materialise phase prompts — defaults are embedded"
        );
        assert!(
            !target.join("agents").exists(),
            "init must not materialise agent definitions — defaults are embedded"
        );
    }

    #[test]
    fn init_lua_stub_is_entirely_commented() {
        // The stub must parse as a no-op Lua file: every non-blank line
        // is a comment so the runtime never sees a stray setter that
        // would shadow the embedded defaults.
        let dir = TempDir::new().unwrap();
        let target = dir.path().join("cfg");
        run_init(&target, false).unwrap();
        let body = fs::read_to_string(target.join(CONFIG_LUA_FILENAME)).unwrap();
        for (idx, line) in body.lines().enumerate() {
            let trimmed = line.trim();
            if trimmed.is_empty() {
                continue;
            }
            assert!(
                trimmed.starts_with("--"),
                "config.lua stub line {idx} is not a comment: {line:?}"
            );
        }
    }

    #[test]
    fn init_does_not_write_a_trampoline() {
        let dir = TempDir::new().unwrap();
        let target = dir.path().join("my-config");
        run_init(&target, false).unwrap();

        assert!(
            !target.join("ravel-lite").exists(),
            "init must not scaffold a ravel-lite trampoline; discovery uses env var + default location"
        );
    }

    #[test]
    fn init_preserves_existing_lua_stub() {
        // A second `init` run must never overwrite a customised
        // `config.lua` — the stub is a starting point, not a template
        // refresh point. The v1→v2 migration is the only path that
        // mutates user-owned config.
        let dir = TempDir::new().unwrap();
        let target = dir.path().join("my-config");
        run_init(&target, false).unwrap();

        fs::write(
            target.join(CONFIG_LUA_FILENAME),
            "-- user customisation\nravel.set_agent('pi')\n",
        )
        .unwrap();
        run_init(&target, false).unwrap();

        let content = fs::read_to_string(target.join(CONFIG_LUA_FILENAME)).unwrap();
        assert!(content.contains("ravel.set_agent('pi')"));
    }

    #[test]
    fn init_force_does_not_overwrite_lua_stub() {
        // `--force` is for retired-path pruning; it must not stomp a
        // user-edited `config.lua`.
        let dir = TempDir::new().unwrap();
        let target = dir.path().join("my-config");
        run_init(&target, false).unwrap();
        fs::write(target.join(CONFIG_LUA_FILENAME), "-- mine\n").unwrap();
        run_init(&target, true).unwrap();
        let content = fs::read_to_string(target.join(CONFIG_LUA_FILENAME)).unwrap();
        assert_eq!(content, "-- mine\n");
    }

    #[test]
    fn every_file_under_defaults_is_registered_in_embedded_files() {
        // Drift guard: every file shipped under `defaults/` must have a
        // matching `EmbeddedFile` entry, otherwise `init` and
        // `init --force` silently fail to scaffold or refresh it — the
        // file ships in the source tree but never reaches the user's
        // config dir. This generalises an older coding-style-specific
        // guard so any addition anywhere in `defaults/` is covered. A
        // missing registration for `discover-stage2.md` is exactly the
        // bug this replaces.
        let defaults_root = Path::new(env!("CARGO_MANIFEST_DIR")).join("defaults");
        let mut on_disk: Vec<String> = Vec::new();
        collect_files_recursively(&defaults_root, &defaults_root, &mut on_disk);
        on_disk.sort();
        assert!(!on_disk.is_empty(), "expected at least one file under defaults/");

        let embedded: std::collections::HashSet<&str> =
            EMBEDDED_FILES.iter().map(|f| f.path).collect();
        let missing: Vec<&String> = on_disk
            .iter()
            .filter(|p| !embedded.contains(p.as_str()))
            .collect();
        assert!(
            missing.is_empty(),
            "defaults/ file(s) missing from EMBEDDED_FILES: {missing:?}"
        );
    }

    fn collect_files_recursively(root: &Path, current: &Path, out: &mut Vec<String>) {
        for entry in fs::read_dir(current).unwrap().flatten() {
            let path = entry.path();
            if path.is_dir() {
                collect_files_recursively(root, &path, out);
            } else if path.is_file() {
                let rel = path.strip_prefix(root).unwrap();
                out.push(rel.to_string_lossy().replace('\\', "/"));
            }
        }
    }

    #[test]
    fn init_force_prunes_retired_paths() {
        let dir = TempDir::new().unwrap();
        let target = dir.path().join("my-config");
        // Simulate the pre-rename layout: a stale skills/ directory
        // holding pi subagent prompts that have since moved into the
        // embedded set under `agents/pi/subagents/`.
        fs::create_dir_all(target.join("skills")).unwrap();
        fs::write(target.join("skills/brainstorming.md"), "stale\n").unwrap();

        run_init(&target, true).unwrap();

        assert!(
            !target.join("skills").exists(),
            "init --force should prune the retired skills/ directory"
        );
        // Replacement subagents now live in the embedded set, not on
        // disk; verify the registry knows them so the runtime has a
        // working subagent dispatch path post-prune.
        assert!(
            embedded_content("agents/pi/subagents/brainstorming.md").is_some(),
            "replacement subagent must be in the embedded set after prune"
        );
    }

    #[test]
    fn init_without_force_does_not_prune_retired_paths() {
        let dir = TempDir::new().unwrap();
        let target = dir.path().join("my-config");
        fs::create_dir_all(target.join("skills")).unwrap();
        fs::write(target.join("skills/brainstorming.md"), "stale\n").unwrap();

        run_init(&target, false).unwrap();

        assert!(
            target.join("skills").exists(),
            "non-force init must not prune — pruning is opt-in via --force"
        );
    }

    #[test]
    fn embedded_lookup_returns_known_path() {
        // The runtime accessor for shipped defaults must hit every
        // registered path. A mismatched key produces `None` so callers
        // can decide between fallback or hard error via
        // `require_embedded`.
        assert!(
            embedded_content("phases/work.md").is_some(),
            "phases/work.md must be embedded"
        );
        assert!(
            embedded_content("create-plan.md").is_some(),
            "create-plan.md must be embedded"
        );
        assert!(
            embedded_content("agents/pi/prompts/system-prompt.md").is_some(),
            "pi system prompt must be embedded"
        );
    }

    #[test]
    fn embedded_lookup_returns_none_for_unregistered_path() {
        assert!(embedded_content("not-a-real-default.md").is_none());
    }

    #[test]
    fn require_embedded_errors_with_path_in_message() {
        let err = require_embedded("not-a-real-default.md").unwrap_err();
        let msg = format!("{err:#}");
        assert!(msg.contains("not-a-real-default.md"), "msg: {msg}");
    }

    #[test]
    fn embedded_entries_with_prefix_walks_subagents() {
        let count = embedded_entries_with_prefix("agents/pi/subagents/").count();
        assert!(count > 0, "expected at least one pi subagent embedded");
        for (rel, _) in embedded_entries_with_prefix("agents/pi/subagents/") {
            assert!(rel.starts_with("agents/pi/subagents/"), "prefix mismatch: {rel}");
            assert!(rel.ends_with(".md"), "expected .md subagent: {rel}");
        }
    }
}
