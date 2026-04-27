// src/config.rs
//
// Public config-loading entry points. The on-disk layer is now Lua —
// see `crate::config_lua` for the layered global+plan resolver. The
// loaders here are thin wrappers that select a slice of the resolved
// config; they intentionally have the same shape as the pre-Lua API
// so call sites that only need global config (main.rs, create.rs,
// discover/mod.rs, survey/invoke.rs) can stay unchanged.
//
// Plan-aware consumers (phase_loop) call `config_lua::resolve` directly
// with the plan dir so plan-level overrides and `append_prompt`
// accumulators are picked up.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use anyhow::Result;

use crate::config_lua;
use crate::types::{AgentConfig, SharedConfig};

/// Environment variable that overrides the default config-directory location.
pub const CONFIG_ENV_VAR: &str = "RAVEL_LITE_CONFIG";

/// Resolve the Ravel-Lite config directory using the precedence chain:
///   1. explicit `--config <path>` flag
///   2. `RAVEL_LITE_CONFIG` environment variable
///   3. XDG default at `<dirs::config_dir()>/ravel-lite/`
///   4. hard error (no walk-up, no magic, no registry)
///
/// The resolved path must be an existing directory; otherwise an
/// actionable error pointing at `ravel-lite init` is returned.
pub fn resolve_config_dir(explicit_flag: Option<PathBuf>) -> Result<PathBuf> {
    let env_var = std::env::var(CONFIG_ENV_VAR).ok().map(PathBuf::from);
    let xdg_default = dirs::config_dir().map(|p| p.join("ravel-lite"));
    select_config_dir(explicit_flag, env_var, xdg_default)
}

fn select_config_dir(
    explicit: Option<PathBuf>,
    env: Option<PathBuf>,
    default: Option<PathBuf>,
) -> Result<PathBuf> {
    let (source, candidate) = if let Some(path) = explicit {
        ("--config flag".to_string(), path)
    } else if let Some(path) = env {
        (format!("environment variable {CONFIG_ENV_VAR}"), path)
    } else if let Some(path) = default {
        ("default location (dirs::config_dir()/ravel-lite)".to_string(), path)
    } else {
        anyhow::bail!(
            "Could not resolve Ravel-Lite config directory.\n\
             No --config flag, no RAVEL_LITE_CONFIG environment variable, and no user config dir available on this platform.\n\
             Create one with `ravel-lite init <dir>` and either pass --config <dir> or set RAVEL_LITE_CONFIG=<dir>."
        );
    };

    if !candidate.is_dir() {
        anyhow::bail!(
            "Ravel-Lite config directory {} (from {}) does not exist or is not a directory.\n\
             Create it with `ravel-lite init {}`.",
            candidate.display(),
            source,
            candidate.display()
        );
    }

    Ok(candidate)
}

pub fn load_shared_config(config_root: &Path) -> Result<SharedConfig> {
    Ok(config_lua::resolve(config_root, None)?.shared)
}

pub fn load_agent_config(config_root: &Path, agent_name: &str) -> Result<AgentConfig> {
    Ok(config_lua::resolve(config_root, None)?.agent(agent_name))
}

pub fn load_tokens(
    config_root: &Path,
    agent_name: &str,
) -> Result<HashMap<String, String>> {
    Ok(config_lua::resolve(config_root, None)?
        .tokens
        .get(agent_name)
        .cloned()
        .unwrap_or_default())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    #[test]
    fn loads_shared_config_from_embedded() {
        // Bare config dir (no config.lua) returns the embedded default.
        let dir = TempDir::new().unwrap();
        let cfg = load_shared_config(dir.path()).unwrap();
        assert!(!cfg.agent.is_empty());
        assert!(cfg.headroom > 0);
    }

    #[test]
    fn loads_agent_config_from_embedded() {
        let dir = TempDir::new().unwrap();
        let cc = load_agent_config(dir.path(), "claude-code").unwrap();
        assert!(cc.models.contains_key("reflect"));
    }

    #[test]
    fn loads_tokens_from_embedded() {
        let dir = TempDir::new().unwrap();
        let tokens = load_tokens(dir.path(), "claude-code").unwrap();
        // The shipped agent has token mappings — exact contents are
        // verified by the embedded-defaults integration test.
        assert!(!tokens.is_empty());
    }

    #[test]
    fn lua_global_layer_overrides_embedded() {
        let dir = TempDir::new().unwrap();
        fs::write(
            dir.path().join("config.lua"),
            "ravel.set_agent('pi')\nravel.set_headroom(9000)\n",
        )
        .unwrap();
        let cfg = load_shared_config(dir.path()).unwrap();
        assert_eq!(cfg.agent, "pi");
        assert_eq!(cfg.headroom, 9000);
    }

    #[test]
    fn lua_set_model_for_overrides_per_agent_phase() {
        // Equivalent of the old `*.local.yaml` overlay that pinned a
        // single phase's model — this is the canonical migration
        // example for users moving from YAML to Lua.
        let dir = TempDir::new().unwrap();
        fs::write(
            dir.path().join("config.lua"),
            "ravel.set_model_for('claude-code', 'work', '')\n",
        )
        .unwrap();
        let cfg = load_agent_config(dir.path(), "claude-code").unwrap();
        assert_eq!(cfg.models.get("work").unwrap(), "");
        assert!(cfg.models.contains_key("reflect"), "sibling phases preserved");
    }

    #[test]
    fn lua_set_token_overrides_a_single_token() {
        let dir = TempDir::new().unwrap();
        fs::write(
            dir.path().join("config.lua"),
            "ravel.set_token('claude-code', 'TOOL_READ', 'CustomRead')\n",
        )
        .unwrap();
        let tokens = load_tokens(dir.path(), "claude-code").unwrap();
        assert_eq!(tokens.get("TOOL_READ").unwrap(), "CustomRead");
    }

    #[test]
    fn malformed_lua_surfaces_clear_error() {
        let dir = TempDir::new().unwrap();
        fs::write(dir.path().join("config.lua"), "this is not valid lua! {{").unwrap();
        let err = load_shared_config(dir.path()).unwrap_err();
        let msg = format!("{err:#}");
        assert!(msg.contains("config.lua"), "msg: {msg}");
    }

    // ---- select_config_dir ----

    #[test]
    fn explicit_flag_takes_precedence_over_env_and_default() {
        let explicit = TempDir::new().unwrap();
        let env = TempDir::new().unwrap();
        let default = TempDir::new().unwrap();

        let resolved = select_config_dir(
            Some(explicit.path().to_path_buf()),
            Some(env.path().to_path_buf()),
            Some(default.path().to_path_buf()),
        )
        .unwrap();

        assert_eq!(resolved, explicit.path());
    }

    #[test]
    fn env_takes_precedence_over_default_when_no_explicit() {
        let env = TempDir::new().unwrap();
        let default = TempDir::new().unwrap();

        let resolved = select_config_dir(
            None,
            Some(env.path().to_path_buf()),
            Some(default.path().to_path_buf()),
        )
        .unwrap();

        assert_eq!(resolved, env.path());
    }

    #[test]
    fn default_used_when_no_explicit_and_no_env() {
        let default = TempDir::new().unwrap();
        let resolved = select_config_dir(None, None, Some(default.path().to_path_buf())).unwrap();
        assert_eq!(resolved, default.path());
    }

    #[test]
    fn nonexistent_explicit_errors_with_candidate_path() {
        let missing = PathBuf::from("/definitely/not/a/real/path/for/ravel-lite/test");
        let err = select_config_dir(Some(missing.clone()), None, None).unwrap_err();
        let message = format!("{err:#}");
        assert!(message.contains(&missing.display().to_string()));
        assert!(message.contains("--config flag"));
        assert!(message.contains("ravel-lite init"));
    }

    #[test]
    fn nonexistent_env_errors_with_env_var_name() {
        let missing = PathBuf::from("/definitely/not/a/real/path/for/ravel-lite/test");
        let err = select_config_dir(None, Some(missing), None).unwrap_err();
        let message = format!("{err:#}");
        assert!(message.contains("RAVEL_LITE_CONFIG"));
    }

    #[test]
    fn all_sources_missing_errors_with_init_guidance() {
        let err = select_config_dir(None, None, None).unwrap_err();
        let message = format!("{err:#}");
        assert!(message.contains("ravel-lite init"));
        assert!(message.contains("RAVEL_LITE_CONFIG"));
    }

    #[test]
    fn candidate_that_is_a_file_errors() {
        let dir = TempDir::new().unwrap();
        let file_path = dir.path().join("not-a-dir");
        fs::write(&file_path, "").unwrap();

        let err = select_config_dir(Some(file_path.clone()), None, None).unwrap_err();
        let message = format!("{err:#}");
        assert!(message.contains("not a directory") || message.contains("does not exist"));
    }
}
