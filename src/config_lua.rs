//! Lua-backed configuration surface.
//!
//! Replaces the deep-merge YAML overlay (`*.local.yaml`) with a
//! turing-complete config layer. Shipped defaults remain authored as
//! YAML inside the embedded set — Lua does not re-encode them, it only
//! *mutates* the resulting Rust structs through a small imperative API
//! (`ravel.set_agent`, `ravel.set_model`, `ravel.append_prompt`, …)
//! and a writable `ravel.config` table for the SharedConfig top-level
//! fields.
//!
//! Layering:
//!   1. Start from the embedded YAML defaults, deserialised into
//!      `SharedConfig` / `AgentConfig` / token maps.
//!   2. Run `<global>/config.lua` (if present) in a fresh Lua state.
//!   3. Run `<plan>/config.lua` (if present) in the same Lua state,
//!      after global. Setters last-write-wins; `append_prompt`
//!      accumulates across both layers.
//!
//! The Lua state is *not* sandboxed — config is trusted code, same
//! threat model as `wezterm.lua` or `init.lua`. A `config.lua` that
//! crashes or throws surfaces as a normal Rust error with the layer
//! and message inlined.

use std::collections::HashMap;
use std::path::Path;
use std::sync::{Arc, Mutex};

use anyhow::{Context, Result};
use mlua::{Lua, Table};

use crate::cli::error_context::ResultExt;
use crate::cli::ErrorCode;
use crate::init::require_embedded;
use crate::migrate_v1_to_v2;
use crate::types::{AgentConfig, SharedConfig};

/// Names that the Rust side knows how to construct embedded defaults
/// for. Anything outside this set falls through with empty values
/// when first looked up via `ravel.set_model("…")` etc.
pub const KNOWN_AGENTS: &[&str] = &["claude-code", "pi"];

/// Result of resolving the global + plan Lua layers on top of the
/// embedded YAML defaults.
#[derive(Debug, Clone)]
pub struct ResolvedConfig {
    pub shared: SharedConfig,
    pub agents: HashMap<String, AgentConfig>,
    pub tokens: HashMap<String, HashMap<String, String>>,
    /// Phase prompt appends accumulated across all `ravel.append_prompt`
    /// calls in the order they were registered (global first, then
    /// plan). Multiple registrations for the same phase keep their
    /// insertion order so the rendered prompt is deterministic.
    pub prompt_appends: HashMap<String, Vec<String>>,
}

impl ResolvedConfig {
    /// Look up an agent's config, falling back to a freshly-constructed
    /// empty value (matching `Default`) so callers never have to
    /// branch on missing-agent.
    pub fn agent(&self, name: &str) -> AgentConfig {
        self.agents.get(name).cloned().unwrap_or_default()
    }

    /// Phase appends for a given phase name (no entry → empty slice).
    pub fn appends_for(&self, phase: &str) -> &[String] {
        self.prompt_appends
            .get(phase)
            .map(Vec::as_slice)
            .unwrap_or(&[])
    }
}

/// Mutable accumulator captured by Lua closures. Setters mutate this
/// table; the Rust side reads it back at the end.
#[derive(Debug, Default, Clone)]
struct Accumulator {
    shared: SharedConfig,
    agents: HashMap<String, AgentConfig>,
    tokens: HashMap<String, HashMap<String, String>>,
    prompt_appends: HashMap<String, Vec<String>>,
}

/// Run the layered Lua resolution against the embedded base.
///
/// `global_dir` is the resolved config dir (always present);
/// `plan_dir` may be `None` for non-plan-scoped commands. Either or
/// both may lack a `config.lua`, which is fine — the embedded base
/// surfaces unchanged.
pub fn resolve(global_dir: &Path, plan_dir: Option<&Path>) -> Result<ResolvedConfig> {
    // First-touch v1 → v2 migration: rewrite legacy materialised
    // defaults to the Lua surface before reading any layer. Idempotent
    // and a no-op for fresh or already-v2 dirs.
    migrate_v1_to_v2::migrate_if_needed(global_dir)
        .context("v1\u{2192}v2 config-dir migration")?;

    let acc = Arc::new(Mutex::new(seed_from_embedded()?));

    let lua = Lua::new();
    install_ravel_table(&lua, acc.clone())
        .map_err(lua_anyhow("install ravel table"))
        .with_code(ErrorCode::Internal)?;

    if let Some(layer_path) = lua_path(global_dir) {
        run_layer(&lua, &layer_path, "global")?;
        commit_config_table(&lua)
            .map_err(lua_anyhow("commit after global layer"))
            .with_code(ErrorCode::InvalidInput)?;
    }
    if let Some(plan) = plan_dir {
        if let Some(layer_path) = lua_path(plan) {
            run_layer(&lua, &layer_path, "plan")?;
            commit_config_table(&lua)
                .map_err(lua_anyhow("commit after plan layer"))
                .with_code(ErrorCode::InvalidInput)?;
        }
    }

    drop(lua);
    let final_acc = Arc::try_unwrap(acc)
        .map_err(|_| anyhow::anyhow!("internal: Lua state held an extra reference to accumulator"))
        .with_code(ErrorCode::Internal)?
        .into_inner()
        .map_err(|e| anyhow::anyhow!("internal: poisoned config accumulator: {e}"))
        .with_code(ErrorCode::Internal)?;

    Ok(ResolvedConfig {
        shared: final_acc.shared,
        agents: final_acc.agents,
        tokens: final_acc.tokens,
        prompt_appends: final_acc.prompt_appends,
    })
}

fn seed_from_embedded() -> Result<Accumulator> {
    let shared: SharedConfig = serde_yaml::from_str(require_embedded("config.yaml")?)
        .context("parse embedded config.yaml")?;

    let mut agents = HashMap::new();
    let mut tokens = HashMap::new();
    for name in KNOWN_AGENTS {
        let cfg_key = format!("agents/{name}/config.yaml");
        let cfg: AgentConfig = serde_yaml::from_str(require_embedded(&cfg_key)?)
            .with_context(|| format!("parse embedded {cfg_key}"))?;
        agents.insert((*name).to_string(), cfg);

        let tok_key = format!("agents/{name}/tokens.yaml");
        let tok: HashMap<String, String> = serde_yaml::from_str(require_embedded(&tok_key)?)
            .with_context(|| format!("parse embedded {tok_key}"))?;
        tokens.insert((*name).to_string(), tok);
    }

    Ok(Accumulator {
        shared,
        agents,
        tokens,
        prompt_appends: HashMap::new(),
    })
}

fn lua_path(dir: &Path) -> Option<std::path::PathBuf> {
    let path = dir.join("config.lua");
    if path.exists() { Some(path) } else { None }
}

fn run_layer(lua: &Lua, path: &Path, label: &str) -> Result<()> {
    let body = std::fs::read_to_string(path)
        .with_context(|| format!("Failed to read {label} {}", path.display()))?;
    let chunk_name = path.to_string_lossy().to_string();
    lua.load(&body)
        .set_name(chunk_name)
        .exec()
        .map_err(|e| {
            anyhow::anyhow!(
                "Failed to execute {label} {}: {e}",
                path.display()
            )
        })
        .with_code(ErrorCode::InvalidInput)
}

/// Convert an `mlua::Error` plus a contextual label into an
/// `anyhow::Error`. mlua's `Error` is not `Send + Sync`, which blocks
/// the `?`-via-`anyhow::Context` path, so every Lua boundary call
/// passes through this helper.
fn lua_anyhow(context: &'static str) -> impl Fn(mlua::Error) -> anyhow::Error {
    move |e| anyhow::anyhow!("{context}: {e}")
}

fn install_ravel_table(lua: &Lua, acc: Arc<Mutex<Accumulator>>) -> mlua::Result<()> {
    let ravel: Table = lua.create_table()?;

    // ravel.config — read/write mirror of SharedConfig top-level
    // fields. Direct table access (`ravel.config.headroom = 9000`) is
    // the supported path for these scalars.
    let config_tbl: Table = lua.create_table()?;
    {
        let snap = acc.lock().unwrap();
        config_tbl.set("agent", snap.shared.agent.clone())?;
        config_tbl.set("headroom", snap.shared.headroom as i64)?;
    }
    ravel.set("config", config_tbl)?;

    // ravel.set_agent(name)
    let acc_set_agent = acc.clone();
    ravel.set(
        "set_agent",
        lua.create_function(move |lua, name: String| {
            acc_set_agent.lock().unwrap().shared.agent = name.clone();
            sync_config_field(lua, "agent", name)?;
            Ok(())
        })?,
    )?;

    // ravel.set_headroom(n)
    let acc_set_headroom = acc.clone();
    ravel.set(
        "set_headroom",
        lua.create_function(move |lua, headroom: i64| {
            if headroom < 0 {
                return Err(mlua::Error::external(format!(
                    "headroom must be non-negative, got {headroom}"
                )));
            }
            acc_set_headroom.lock().unwrap().shared.headroom = headroom as usize;
            sync_config_field(lua, "headroom", headroom)?;
            Ok(())
        })?,
    )?;

    // ravel.set_model(phase, name) — applies to the currently-active
    // agent (ravel.config.agent). Pass an empty string to defer the
    // model choice to the agent CLI's interactive default.
    let acc_set_model = acc.clone();
    ravel.set(
        "set_model",
        lua.create_function(move |_, (phase, model): (String, String)| {
            let mut g = acc_set_model.lock().unwrap();
            let agent = g.shared.agent.clone();
            g.agents
                .entry(agent)
                .or_default()
                .models
                .insert(phase, model);
            Ok(())
        })?,
    )?;

    // ravel.set_model_for(agent, phase, name)
    let acc_set_model_for = acc.clone();
    ravel.set(
        "set_model_for",
        lua.create_function(
            move |_, (agent, phase, model): (String, String, String)| {
                acc_set_model_for
                    .lock()
                    .unwrap()
                    .agents
                    .entry(agent)
                    .or_default()
                    .models
                    .insert(phase, model);
                Ok(())
            },
        )?,
    )?;

    // ravel.set_provider(provider) — applies to the active agent.
    let acc_set_provider = acc.clone();
    ravel.set(
        "set_provider",
        lua.create_function(move |_, provider: String| {
            let mut g = acc_set_provider.lock().unwrap();
            let agent = g.shared.agent.clone();
            g.agents.entry(agent).or_default().provider = Some(provider);
            Ok(())
        })?,
    )?;

    // ravel.set_provider_for(agent, provider)
    let acc_set_provider_for = acc.clone();
    ravel.set(
        "set_provider_for",
        lua.create_function(move |_, (agent, provider): (String, String)| {
            acc_set_provider_for
                .lock()
                .unwrap()
                .agents
                .entry(agent)
                .or_default()
                .provider = Some(provider);
            Ok(())
        })?,
    )?;

    // ravel.set_token(agent, name, value) — overrides a single
    // substitution token for a specific agent.
    let acc_set_token = acc.clone();
    ravel.set(
        "set_token",
        lua.create_function(
            move |_, (agent, name, value): (String, String, String)| {
                acc_set_token
                    .lock()
                    .unwrap()
                    .tokens
                    .entry(agent)
                    .or_default()
                    .insert(name, value);
                Ok(())
            },
        )?,
    )?;

    // ravel.append_prompt(phase, text) — registers append-only
    // prompt customisation. Multiple calls accumulate.
    let acc_append = acc.clone();
    ravel.set(
        "append_prompt",
        lua.create_function(move |_, (phase, text): (String, String)| {
            acc_append
                .lock()
                .unwrap()
                .prompt_appends
                .entry(phase)
                .or_default()
                .push(text);
            Ok(())
        })?,
    )?;

    // ravel._commit_config_table() — pulls direct edits on
    // `ravel.config` back into the accumulator. Called automatically
    // after each layer; users can also call it explicitly.
    let acc_commit = acc.clone();
    ravel.set(
        "_commit_config_table",
        lua.create_function(move |lua, ()| {
            let globals = lua.globals();
            let ravel_tbl: Table = globals.get("ravel")?;
            let cfg_tbl: Table = ravel_tbl.get("config")?;
            let agent: Option<String> = cfg_tbl.get("agent")?;
            let headroom: Option<i64> = cfg_tbl.get("headroom")?;
            let mut g = acc_commit.lock().unwrap();
            if let Some(a) = agent {
                g.shared.agent = a;
            }
            if let Some(h) = headroom {
                if h >= 0 {
                    g.shared.headroom = h as usize;
                }
            }
            Ok(())
        })?,
    )?;

    lua.globals().set("ravel", ravel)?;
    Ok(())
}

/// Mirror a setter's mutation back into `ravel.config` so subsequent
/// reads of the table inside the Lua layer see the new value.
fn sync_config_field<V: mlua::IntoLua>(
    lua: &Lua,
    field: &str,
    value: V,
) -> mlua::Result<()> {
    let globals = lua.globals();
    let ravel: Table = globals.get("ravel")?;
    let cfg: Table = ravel.get("config")?;
    cfg.set(field, value)?;
    Ok(())
}

fn commit_config_table(lua: &Lua) -> mlua::Result<()> {
    lua.load("ravel._commit_config_table()").exec()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    fn write_lua(dir: &Path, body: &str) -> std::path::PathBuf {
        let path = dir.join("config.lua");
        fs::write(&path, body).unwrap();
        path
    }

    #[test]
    fn resolves_to_embedded_defaults_when_no_layers_present() {
        let tmp = TempDir::new().unwrap();
        let resolved = resolve(tmp.path(), None).unwrap();
        assert!(!resolved.shared.agent.is_empty());
        assert!(resolved.shared.headroom > 0);
        assert!(resolved.agents.contains_key("claude-code"));
        assert!(resolved.agents.contains_key("pi"));
    }

    #[test]
    fn global_set_agent_overrides_embedded_default() {
        let tmp = TempDir::new().unwrap();
        write_lua(tmp.path(), "ravel.set_agent('pi')\n");
        let resolved = resolve(tmp.path(), None).unwrap();
        assert_eq!(resolved.shared.agent, "pi");
    }

    #[test]
    fn plan_layer_overrides_global() {
        let global = TempDir::new().unwrap();
        let plan = TempDir::new().unwrap();
        write_lua(global.path(), "ravel.set_agent('pi')\n");
        write_lua(plan.path(), "ravel.set_agent('claude-code')\n");

        let resolved = resolve(global.path(), Some(plan.path())).unwrap();
        assert_eq!(resolved.shared.agent, "claude-code");
    }

    #[test]
    fn plan_layer_does_not_lose_global_sibling_keys() {
        // Layering: global sets `headroom`; plan overrides only the
        // model. Resolved state must reflect both.
        let global = TempDir::new().unwrap();
        let plan = TempDir::new().unwrap();
        write_lua(global.path(), "ravel.set_headroom(9000)\n");
        write_lua(
            plan.path(),
            "ravel.set_agent('claude-code')\nravel.set_model('work', 'claude-opus-4-7')\n",
        );

        let resolved = resolve(global.path(), Some(plan.path())).unwrap();
        assert_eq!(resolved.shared.headroom, 9000);
        assert_eq!(
            resolved
                .agents
                .get("claude-code")
                .unwrap()
                .models
                .get("work")
                .unwrap(),
            "claude-opus-4-7"
        );
    }

    #[test]
    fn append_prompt_accumulates_across_layers() {
        let global = TempDir::new().unwrap();
        let plan = TempDir::new().unwrap();
        write_lua(global.path(), "ravel.append_prompt('work', 'global tip')\n");
        write_lua(plan.path(), "ravel.append_prompt('work', 'plan tip')\n");

        let resolved = resolve(global.path(), Some(plan.path())).unwrap();
        let appends = resolved.appends_for("work");
        assert_eq!(appends.len(), 2);
        assert_eq!(appends[0], "global tip");
        assert_eq!(appends[1], "plan tip");
        // Phases without registrations are empty.
        assert!(resolved.appends_for("triage").is_empty());
    }

    #[test]
    fn set_model_targets_active_agent() {
        let tmp = TempDir::new().unwrap();
        write_lua(
            tmp.path(),
            "ravel.set_agent('pi')\nravel.set_model('work', 'claude-opus-4-7')\n",
        );
        let resolved = resolve(tmp.path(), None).unwrap();
        assert_eq!(
            resolved.agents.get("pi").unwrap().models.get("work").unwrap(),
            "claude-opus-4-7"
        );
        // claude-code keeps its embedded default.
        let cc_work = resolved
            .agents
            .get("claude-code")
            .unwrap()
            .models
            .get("work")
            .cloned();
        assert!(cc_work.is_some());
    }

    #[test]
    fn set_model_for_targets_explicit_agent() {
        let tmp = TempDir::new().unwrap();
        write_lua(
            tmp.path(),
            "ravel.set_model_for('pi', 'work', 'claude-opus-4-7')\n",
        );
        let resolved = resolve(tmp.path(), None).unwrap();
        assert_eq!(
            resolved.agents.get("pi").unwrap().models.get("work").unwrap(),
            "claude-opus-4-7"
        );
    }

    #[test]
    fn set_token_overrides_embedded_token() {
        let tmp = TempDir::new().unwrap();
        write_lua(
            tmp.path(),
            "ravel.set_token('claude-code', 'TOOL_READ', 'CustomRead')\n",
        );
        let resolved = resolve(tmp.path(), None).unwrap();
        assert_eq!(
            resolved
                .tokens
                .get("claude-code")
                .unwrap()
                .get("TOOL_READ")
                .unwrap(),
            "CustomRead"
        );
    }

    #[test]
    fn lua_runtime_error_surfaces_layer_label_and_path() {
        let tmp = TempDir::new().unwrap();
        write_lua(tmp.path(), "error('boom')\n");
        let err = resolve(tmp.path(), None).unwrap_err();
        let msg = format!("{err:#}");
        assert!(msg.contains("global"), "msg: {msg}");
        assert!(msg.contains("config.lua"), "msg: {msg}");
        assert!(msg.contains("boom"), "msg: {msg}");
    }

    #[test]
    fn missing_files_at_both_layers_are_fine() {
        // The embedded defaults are sufficient on their own — running
        // with no Lua at either layer must produce a usable config.
        let tmp = TempDir::new().unwrap();
        let resolved = resolve(tmp.path(), Some(tmp.path())).unwrap();
        assert!(!resolved.shared.agent.is_empty());
    }

    #[test]
    fn config_table_direct_assignment_persists_after_commit() {
        // Direct table assignment on `ravel.config` must round-trip —
        // the auto-commit after each layer pulls it back into the
        // accumulator without the user having to remember anything.
        let tmp = TempDir::new().unwrap();
        write_lua(tmp.path(), "ravel.config.headroom = 9000\n");
        let resolved = resolve(tmp.path(), None).unwrap();
        assert_eq!(resolved.shared.headroom, 9000);
    }

    #[test]
    fn golden_lua_mirrors_old_yaml_overlay() {
        // Golden parity test: the canonical "blank work model + change
        // agent" override that used to live in `*.local.yaml` produces
        // the same final struct values when expressed as Lua. This is
        // the primary migration aid — a user porting their overlay
        // can verify byte-for-byte equivalence.
        let tmp = TempDir::new().unwrap();
        write_lua(
            tmp.path(),
            "ravel.set_agent('pi')\nravel.set_model_for('claude-code', 'work', '')\n",
        );
        let resolved = resolve(tmp.path(), None).unwrap();
        assert_eq!(resolved.shared.agent, "pi");
        assert_eq!(
            resolved
                .agents
                .get("claude-code")
                .unwrap()
                .models
                .get("work")
                .unwrap(),
            ""
        );
        // Sibling keys (e.g. reflect model) are untouched — same
        // semantic the YAML deep-merge guarantee provided.
        let reflect = resolved
            .agents
            .get("claude-code")
            .unwrap()
            .models
            .get("reflect");
        assert!(reflect.is_some());
        assert!(!reflect.unwrap().is_empty());
    }
}
