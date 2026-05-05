//! Helpers shared by the three apply_* modules: `invoke_phase` renders
//! a prompt and calls `agent.invoke_headless`.

use std::collections::HashMap;
use std::sync::Arc;

use anyhow::Result;
use tokio::sync::mpsc;

use crate::agent::Agent;
use crate::prompt::compose_prompt;
use crate::types::{LlmPhase, PlanContext};

use super::validate::Validated;

pub async fn invoke_phase(
    agent: Arc<dyn Agent>,
    v: &Validated,
    phase: LlmPhase,
) -> Result<()> {
    let new_plan_name = v
        .new_plan_dir
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("");
    let mut tokens = HashMap::new();
    tokens.insert("OLD_PLAN_PATH".into(), v.old_plan_path.display().to_string());
    tokens.insert("NEW_PLAN_DIR".into(), v.new_plan_dir.display().to_string());
    tokens.insert("NEW_PLAN_NAME".into(), new_plan_name.to_string());
    tokens.insert("SOURCE_REPO_SLUG".into(), v.source_repo_slug.clone());
    tokens.insert(
        "SOURCE_REPO_PATH".into(),
        v.source_repo_path.display().to_string(),
    );

    let ctx = PlanContext {
        plan_dir: v.new_plan_dir.display().to_string(),
        project_dir: v.config_dir.display().to_string(),
        dev_root: v.config_dir.display().to_string(),
        related_plans: String::new(),
        config_root: v.config_dir.display().to_string(),
    };

    let prompt = compose_prompt(phase, &ctx, &tokens, &[])?;
    // No TUI for the migrator's headless calls; drop the receiver — the
    // sender stays alive for the call's lifetime, then both ends are
    // dropped together.
    let (tx, _rx) = mpsc::unbounded_channel();
    agent
        .invoke_headless(&prompt, &ctx, phase, "migrate", tx)
        .await
}
