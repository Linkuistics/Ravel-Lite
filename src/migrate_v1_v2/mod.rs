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
pub mod transform;
pub mod validate;

/// Top-level entry point. Drives the full migration in two halves:
/// mechanical (validate + copy + transform) followed by three
/// sequential headless LLM phases (intent → targets → memory).
/// Migration writes only into the new v2 plan dir under the
/// config-dir; the original v1 plan is left untouched.
///
/// Emits step-by-step progress to stderr (matches `discover/mod.rs`
/// pattern). The three LLM phases each take ~30–60s with no
/// intermediate output, so the prefix lines warn the user the verb
/// is alive and waiting.
pub async fn run_migrate_v1_v2(
    agent: Arc<dyn Agent>,
    old_plan_path: &Path,
    new_plan_name: &str,
    config_dir: &Path,
) -> Result<()> {
    eprintln!(
        "migrate-v1-v2: {} → plans/{}",
        old_plan_path.display(),
        new_plan_name
    );

    eprintln!("[1/6] Validating inputs");
    let validated = validate::validate_inputs(old_plan_path, new_plan_name, config_dir)?;

    eprintln!("[2/6] Copying state files");
    copy::copy_plan_state(&validated.old_plan_path, &validated.new_plan_dir)?;

    eprintln!("[3/6] Reshaping v1→v2 wire format");
    transform::run(&validated.new_plan_dir)?;

    eprintln!("[4/6] migrate-intent (LLM, ~30–60s) — extracting intents from phase.md");
    apply_intent::run(agent.clone(), &validated).await?;

    eprintln!("[5/6] migrate-targets (LLM, ~30–60s) — identifying target components");
    apply_targets::run(agent.clone(), &validated).await?;

    eprintln!("[6/6] migrate-memory-backfill (LLM, ~30–60s) — attributing memory entries");
    apply_memory::run(agent.clone(), &validated).await?;

    eprintln!(
        "✓ migrate-v1-v2 complete: {}",
        validated.new_plan_dir.display()
    );
    Ok(())
}
