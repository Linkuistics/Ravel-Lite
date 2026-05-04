//! Helpers shared by the three apply_* modules:
//! - `invoke_phase` renders a prompt and calls `agent.invoke_headless`.
//! - `confirm` prints a yes/no prompt to stdin/stdout.

use std::collections::HashMap;
use std::io::{self, Write};
use std::sync::Arc;

use anyhow::Result;
use tokio::sync::mpsc;

use crate::agent::Agent;
use crate::bail_with;
use crate::cli::ErrorCode;
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

/// Print a Y/N prompt to stdout, read from stdin. Returns Ok(()) on
/// "y"/"yes" (case-insensitive); errors with `ErrorCode::Cancelled`
/// otherwise. Default is no — empty input or any other answer cancels.
pub fn confirm(message: &str) -> Result<()> {
    print!("{message} [y/N] ");
    io::stdout().flush().ok();
    let mut buf = String::new();
    io::stdin().read_line(&mut buf)?;
    let trimmed = buf.trim().to_ascii_lowercase();
    if trimmed == "y" || trimmed == "yes" {
        return Ok(());
    }
    bail_with!(ErrorCode::Cancelled, "user declined the apply step");
}
