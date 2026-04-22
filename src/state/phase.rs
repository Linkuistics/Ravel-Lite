//! CLI-facing plan-state mutations used by phase prompts.
//!
//! One verb today: `set-phase` (rewrite `<plan>/phase.md`). It exists so
//! LLM prompts can mutate plan state via one `Bash(ravel-lite state *)`
//! allowlist entry instead of a `Read` + `Write` tool-call pair per
//! transition.

use std::path::Path;

use anyhow::{bail, Context, Result};

use crate::dream::seed_dream_baseline_if_missing;
use crate::types::Phase;

/// Enumerated for error messages so a typo'd phase string comes back
/// with an actionable list of accepted values.
const VALID_PHASES: &[&str] = &[
    "work",
    "analyse-work",
    "reflect",
    "dream",
    "triage",
    "git-commit-work",
    "git-commit-reflect",
    "git-commit-dream",
    "git-commit-triage",
];

pub fn run_set_phase(plan_dir: &Path, phase: &str) -> Result<()> {
    if Phase::parse(phase).is_none() {
        bail!(
            "Invalid phase '{phase}'. Accepted values: {}",
            VALID_PHASES.join(", ")
        );
    }
    let target = plan_dir.join("phase.md");
    if !target.exists() {
        bail!(
            "phase.md not found at {}. set-phase refuses to create a new plan dir.",
            target.display()
        );
    }
    atomic_write(&target, phase.as_bytes())?;
    // Every LLM phase transition funnels through this CLI verb, so
    // seeding here guarantees a baseline exists on any plan the driver
    // touches — including coordinator plans that never reach the
    // `GitCommitReflect` handler, and plans whose baseline was lost
    // between cycles. Idempotent no-op in steady state.
    seed_dream_baseline_if_missing(plan_dir);
    Ok(())
}

/// Write `bytes` to `path` via tmp-file + rename, so a concurrent reader
/// (e.g. the driver sampling phase.md between prompt turns) never sees
/// a truncated file. The tmp file sits next to the target so the rename
/// stays on-device.
fn atomic_write(path: &Path, bytes: &[u8]) -> Result<()> {
    let parent = path
        .parent()
        .with_context(|| format!("{} has no parent directory", path.display()))?;
    let file_name = path
        .file_name()
        .with_context(|| format!("{} has no file name", path.display()))?
        .to_string_lossy();
    let tmp = parent.join(format!(".{file_name}.tmp"));
    std::fs::write(&tmp, bytes)
        .with_context(|| format!("Failed to write temp file {}", tmp.display()))?;
    std::fs::rename(&tmp, path)
        .with_context(|| format!("Failed to rename {} to {}", tmp.display(), path.display()))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn set_phase_writes_valid_llm_phase_to_phase_md() {
        let tmp = TempDir::new().unwrap();
        let plan = tmp.path();
        std::fs::write(plan.join("phase.md"), "work").unwrap();

        run_set_phase(plan, "analyse-work").unwrap();

        let content = std::fs::read_to_string(plan.join("phase.md")).unwrap();
        assert_eq!(content.trim(), "analyse-work");
    }

    #[test]
    fn set_phase_rejects_missing_phase_md() {
        let tmp = TempDir::new().unwrap();
        let plan = tmp.path();
        // Deliberately no phase.md — simulates a typo'd plan-dir arg.
        let err = run_set_phase(plan, "reflect").unwrap_err();
        let msg = format!("{err:#}");
        assert!(msg.contains("phase.md"), "error must name phase.md: {msg}");
        assert!(!plan.join("phase.md").exists(), "must not silently create phase.md");
    }

    #[test]
    fn set_phase_seeds_dream_baseline_when_missing() {
        // Defense-in-depth: any LLM phase transition must leave the
        // plan with a baseline on disk. Coordinator plans never reach
        // `GitCommitReflect`, so this is their only seed path.
        let tmp = TempDir::new().unwrap();
        let plan = tmp.path();
        std::fs::write(plan.join("phase.md"), "work").unwrap();
        assert!(!plan.join("dream-baseline").exists());

        run_set_phase(plan, "analyse-work").unwrap();

        let baseline = std::fs::read_to_string(plan.join("dream-baseline")).unwrap();
        assert_eq!(baseline.trim(), "0");
    }

    #[test]
    fn set_phase_preserves_existing_dream_baseline() {
        // Idempotence: the seed must not clobber an already-written
        // baseline. Otherwise every phase transition would reset
        // progress toward the dream threshold.
        let tmp = TempDir::new().unwrap();
        let plan = tmp.path();
        std::fs::write(plan.join("phase.md"), "work").unwrap();
        std::fs::write(plan.join("dream-baseline"), "1234").unwrap();

        run_set_phase(plan, "analyse-work").unwrap();

        let baseline = std::fs::read_to_string(plan.join("dream-baseline")).unwrap();
        assert_eq!(baseline.trim(), "1234");
    }

    #[test]
    fn set_phase_rejects_typo_and_lists_valid_phases() {
        let tmp = TempDir::new().unwrap();
        let plan = tmp.path();
        std::fs::write(plan.join("phase.md"), "work").unwrap();

        let err = run_set_phase(plan, "analyze-work").unwrap_err();
        let msg = format!("{err:#}");
        assert!(msg.contains("analyze-work"), "error must include the bad input: {msg}");
        // Enumeration of valid phase names in the error lets the LLM
        // self-correct without a second round-trip.
        for valid in ["work", "analyse-work", "reflect", "dream", "triage",
                      "git-commit-work", "git-commit-reflect", "git-commit-dream",
                      "git-commit-triage"] {
            assert!(msg.contains(valid), "error must list '{valid}': {msg}");
        }

        let content = std::fs::read_to_string(plan.join("phase.md")).unwrap();
        assert_eq!(content.trim(), "work", "phase.md must be unchanged on error");
    }
}
