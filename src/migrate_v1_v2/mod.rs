//! `ravel-lite migrate-v1-v2 <old-plan-path> --as <new-name>` —
//! one-shot per-plan structural cutover from a v1 layout
//! (`<project>/LLM_STATE/<plan>/`) to a v2 layout
//! (`<config-dir>/plans/<plan>/`).
//!
//! See `docs/superpowers/specs/2026-05-04-migrate-v1-v2-design.md`.

use std::path::Path;
use std::sync::Arc;

use anyhow::Result;

use crate::agent::Agent;

pub mod apply_intent;
pub mod apply_memory;
pub mod apply_targets;
pub mod copy;
pub mod orchestrator;
pub mod proposals;
pub mod validate;

/// Top-level entry point. Drives the full migration in two halves:
/// mechanical (validate + copy) followed by three sequential headless
/// LLM phases (intent → targets → memory) with confirm-before-apply
/// between each agent output and runner application.
pub async fn run_migrate_v1_v2(
    agent: Arc<dyn Agent>,
    old_plan_path: &Path,
    new_plan_name: &str,
    config_dir: &Path,
    skip_confirm: bool,
) -> Result<()> {
    let validated = validate::validate_inputs(old_plan_path, new_plan_name, config_dir)?;
    copy::copy_plan_state(&validated.old_plan_path, &validated.new_plan_dir)?;
    apply_intent::run(agent.clone(), &validated, skip_confirm).await?;
    apply_targets::run(agent.clone(), &validated, skip_confirm).await?;
    apply_memory::run(agent.clone(), &validated, skip_confirm).await?;
    Ok(())
}
