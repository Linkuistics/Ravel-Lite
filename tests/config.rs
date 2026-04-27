use std::fs;

use tempfile::TempDir;

#[test]
fn config_loading_integration_with_lua_overrides() {
    // End-to-end check: a hand-authored `config.lua` flips agent,
    // pins headroom, overrides a model, and overrides a token. The
    // public loaders surface the resolved values as if they had been
    // set in YAML — the same shape, just sourced through the Lua layer.
    let dir = TempDir::new().unwrap();
    let config_root = dir.path();

    fs::write(
        config_root.join("config.lua"),
        "ravel.set_agent('claude-code')\n\
         ravel.set_headroom(1500)\n\
         ravel.set_model_for('claude-code', 'work', 'claude-sonnet-4-6')\n\
         ravel.set_token('claude-code', 'TOOL_READ', 'Read')\n",
    )
    .unwrap();

    let shared = ravel_lite::config::load_shared_config(config_root).unwrap();
    assert_eq!(shared.agent, "claude-code");
    assert_eq!(shared.headroom, 1500);

    let agent = ravel_lite::config::load_agent_config(config_root, "claude-code").unwrap();
    assert_eq!(agent.models.get("work").unwrap(), "claude-sonnet-4-6");

    let tokens = ravel_lite::config::load_tokens(config_root, "claude-code").unwrap();
    assert_eq!(tokens.get("TOOL_READ").unwrap(), "Read");
}

#[test]
fn embedded_defaults_are_valid() {
    // Validate every shipped default against the real loaders, but
    // straight from the embedded set — disk materialisation is gone, so
    // these must parse and resolve without `init` ever writing them.
    use ravel_lite::init::require_embedded;

    let cc: ravel_lite::types::AgentConfig =
        serde_yaml::from_str(require_embedded("agents/claude-code/config.yaml").unwrap()).unwrap();
    assert!(cc.models.contains_key("reflect"));

    let pi: ravel_lite::types::AgentConfig =
        serde_yaml::from_str(require_embedded("agents/pi/config.yaml").unwrap()).unwrap();
    assert!(pi.models.contains_key("reflect"));

    // Every LLM phase in every embedded agent config must declare a
    // non-empty model string. An empty string silently delegates to
    // whatever `claude` / `pi` pick at spawn time, which is neither
    // auditable nor stable across releases.
    for (agent_name, cfg) in [("claude-code", &cc), ("pi", &pi)] {
        for phase in ["work", "analyse-work", "reflect", "dream", "triage"] {
            let model = cfg
                .models
                .get(phase)
                .unwrap_or_else(|| panic!("{agent_name} defaults missing model for phase {phase}"));
            assert!(
                !model.trim().is_empty(),
                "{agent_name} defaults have empty model for phase {phase}; pick an explicit default"
            );
        }
    }

    // Pi's `build_headless_args` falls back to `"anthropic"` when the
    // config omits `provider`, which is an implicit drift source.
    // Require the embedded default to pin the value explicitly so the
    // fallback only fires for deliberately-minimal user configs.
    let pi_provider = pi
        .provider
        .as_ref()
        .expect("pi defaults must declare `provider` explicitly");
    assert!(
        !pi_provider.trim().is_empty(),
        "pi defaults have empty `provider`; pick an explicit default"
    );

    let shared: ravel_lite::types::SharedConfig =
        serde_yaml::from_str(require_embedded("config.yaml").unwrap()).unwrap();
    assert!(!shared.agent.is_empty());
    assert!(shared.headroom > 0);

    for phase in ["work", "analyse-work", "reflect", "dream", "triage"] {
        let body = require_embedded(&format!("phases/{phase}.md")).unwrap();
        assert!(!body.trim().is_empty(), "empty phase file: {phase}");
    }

    assert!(!ravel_lite::survey::load_survey_prompt().unwrap().trim().is_empty());
    assert!(
        !ravel_lite::survey::load_survey_incremental_prompt()
            .unwrap()
            .trim()
            .is_empty()
    );
    assert!(!require_embedded("create-plan.md").unwrap().trim().is_empty());
}
