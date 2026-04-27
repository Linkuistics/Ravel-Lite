//! Integration tests for the Lua config surface.
//!
//! These tests exercise `config_lua::resolve` end-to-end (Lua eval +
//! Rust struct shape) and the `append_prompt` → `compose_prompt`
//! handoff that the runtime relies on. They sit at `tests/` rather
//! than in `src/config_lua.rs` because they cross module boundaries
//! into the prompt-composition path and use the public `lib.rs`
//! surface a real consumer would see.

use std::collections::HashMap;
use std::fs;

use tempfile::TempDir;

use ravel_lite::config_lua;
use ravel_lite::prompt::compose_prompt;
use ravel_lite::types::{LlmPhase, PlanContext};

fn write_lua(dir: &std::path::Path, body: &str) {
    fs::write(dir.join("config.lua"), body).unwrap();
}

#[test]
fn missing_config_lua_at_both_layers_returns_embedded_defaults() {
    let global = TempDir::new().unwrap();
    let plan = TempDir::new().unwrap();
    let resolved = config_lua::resolve(global.path(), Some(plan.path())).unwrap();

    // Embedded defaults declare claude-code as the agent and >0 headroom.
    assert!(!resolved.shared.agent.is_empty());
    assert!(resolved.shared.headroom > 0);
    assert!(resolved.appends_for("work").is_empty());
}

#[test]
fn global_layer_overrides_embedded_default_agent() {
    let global = TempDir::new().unwrap();
    write_lua(global.path(), "ravel.set_agent('pi')\n");
    let resolved = config_lua::resolve(global.path(), None).unwrap();
    assert_eq!(resolved.shared.agent, "pi");
}

#[test]
fn plan_layer_overrides_global_and_preserves_unrelated_global_keys() {
    // Layering: global pins headroom; plan overrides only agent and
    // a single model. Resolved state must reflect both — the same
    // semantic the YAML deep-merge used to provide for `*.local.yaml`
    // overlays.
    let global = TempDir::new().unwrap();
    let plan = TempDir::new().unwrap();
    write_lua(global.path(), "ravel.set_headroom(9000)\n");
    write_lua(
        plan.path(),
        "ravel.set_agent('claude-code')\n\
         ravel.set_model('work', 'claude-opus-4-7')\n",
    );

    let resolved = config_lua::resolve(global.path(), Some(plan.path())).unwrap();
    assert_eq!(resolved.shared.agent, "claude-code");
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
fn append_prompt_accumulates_global_then_plan_in_compose_prompt() {
    // End-to-end: a global `append_prompt` and a plan `append_prompt`
    // both reach the composed prompt for the matching phase, in
    // registration order, and tokens inside them substitute.
    let global = TempDir::new().unwrap();
    let plan = TempDir::new().unwrap();
    write_lua(
        global.path(),
        "ravel.append_prompt('triage', 'Global tip about {{PLAN}}.')\n",
    );
    write_lua(
        plan.path(),
        "ravel.append_prompt('triage', 'Plan-specific extra advice.')\n",
    );

    let resolved = config_lua::resolve(global.path(), Some(plan.path())).unwrap();
    let appends = resolved.appends_for("triage").to_vec();
    assert_eq!(appends.len(), 2);

    let plan_path_str = plan.path().to_string_lossy().to_string();
    let ctx = PlanContext {
        plan_dir: plan_path_str.clone(),
        project_dir: "/project".to_string(),
        dev_root: "/dev".to_string(),
        related_plans: String::new(),
        config_root: global.path().to_string_lossy().to_string(),
    };

    let composed = compose_prompt(LlmPhase::Triage, &ctx, &HashMap::new(), &appends).unwrap();

    let global_pos = composed
        .find("Global tip about")
        .expect("global append must reach composed prompt");
    let plan_pos = composed
        .find("Plan-specific extra advice")
        .expect("plan append must reach composed prompt");
    assert!(
        global_pos < plan_pos,
        "global append must precede plan append in composition order"
    );
    assert!(
        composed.contains(&plan_path_str),
        "{{PLAN}} inside an append must be substituted to the plan_dir; got: {composed}"
    );
}

#[test]
fn malformed_lua_surfaces_layer_label_and_filename_in_error() {
    let global = TempDir::new().unwrap();
    write_lua(global.path(), "this is not valid lua! {{\n");

    let err = config_lua::resolve(global.path(), None).unwrap_err();
    let msg = format!("{err:#}");
    assert!(msg.contains("global"), "msg should label the layer: {msg}");
    assert!(msg.contains("config.lua"), "msg should name the file: {msg}");
}

#[test]
fn lua_runtime_error_inside_plan_layer_names_plan_layer() {
    let global = TempDir::new().unwrap();
    let plan = TempDir::new().unwrap();
    write_lua(plan.path(), "error('plan-side problem')\n");

    let err = config_lua::resolve(global.path(), Some(plan.path())).unwrap_err();
    let msg = format!("{err:#}");
    assert!(msg.contains("plan"), "msg should label the layer: {msg}");
    assert!(msg.contains("plan-side problem"), "msg should propagate the Lua error: {msg}");
}

#[test]
fn set_token_overrides_only_the_named_token_for_named_agent() {
    let global = TempDir::new().unwrap();
    write_lua(
        global.path(),
        "ravel.set_token('claude-code', 'TOOL_READ', 'CustomRead')\n",
    );
    let resolved = config_lua::resolve(global.path(), None).unwrap();
    let cc_tokens = resolved.tokens.get("claude-code").unwrap();
    assert_eq!(cc_tokens.get("TOOL_READ").unwrap(), "CustomRead");
    // Unrelated tokens for the same agent are untouched (sibling key
    // preservation — the Lua-overlay equivalent of the deep-merge
    // contract that `*.local.yaml` used to provide).
    assert!(cc_tokens.contains_key("TOOL_WRITE"));
    // Tokens for another agent are untouched.
    let pi_tokens = resolved.tokens.get("pi").unwrap();
    assert!(pi_tokens.contains_key("TOOL_READ"));
}
