//! CLI-facing plan-state mutations used by phase prompts.
//!
//! One verb today: `set-phase` (rewrite `<plan>/phase.md`). It exists so
//! LLM prompts can mutate plan state via one `Bash(ravel-lite state *)`
//! allowlist entry instead of a `Read` + `Write` tool-call pair per
//! transition.

use std::path::Path;

use anyhow::{Context, Result};

use crate::bail_with;
use crate::cli::error_context::ResultExt;
use crate::cli::ErrorCode;
use crate::state::filenames::PHASE_FILENAME;
use crate::types::Phase;

/// Enumerated for error messages so a typo'd phase string comes back
/// with an actionable list of accepted values.
const VALID_PHASES: &[&str] = &[
    "triage",
    "work",
    "analyse-work",
    "reflect",
    "git-commit-triage",
    "git-commit-work",
    "git-commit-analyse-work",
    "git-commit-reflect",
];

pub fn run_set_phase(plan_dir: &Path, phase: &str) -> Result<()> {
    if Phase::parse(phase).is_none() {
        bail_with!(
            ErrorCode::InvalidInput,
            "Invalid phase '{phase}'. Accepted values: {}",
            VALID_PHASES.join(", ")
        );
    }
    let target = plan_dir.join(PHASE_FILENAME);
    if !target.exists() {
        bail_with!(
            ErrorCode::NotFound,
            "{PHASE_FILENAME} not found at {}. set-phase refuses to create a new plan dir.",
            target.display()
        );
    }
    atomic_write(&target, phase.as_bytes())?;
    Ok(())
}

/// Write `bytes` to `path` via tmp-file + rename, so a concurrent reader
/// (e.g. the driver sampling phase.md between prompt turns) never sees
/// a truncated file. The tmp file sits next to the target so the rename
/// stays on-device.
fn atomic_write(path: &Path, bytes: &[u8]) -> Result<()> {
    let parent = path
        .parent()
        .with_context(|| format!("{} has no parent directory", path.display()))
        .with_code(ErrorCode::InvalidInput)?;
    let file_name = path
        .file_name()
        .with_context(|| format!("{} has no file name", path.display()))
        .with_code(ErrorCode::InvalidInput)?
        .to_string_lossy();
    let tmp = parent.join(format!(".{file_name}.tmp"));
    std::fs::write(&tmp, bytes)
        .with_context(|| format!("Failed to write temp file {}", tmp.display()))
        .with_code(ErrorCode::IoError)?;
    std::fs::rename(&tmp, path)
        .with_context(|| format!("Failed to rename {} to {}", tmp.display(), path.display()))
        .with_code(ErrorCode::IoError)?;
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
        std::fs::write(plan.join(PHASE_FILENAME), "work").unwrap();

        run_set_phase(plan, "analyse-work").unwrap();

        let content = std::fs::read_to_string(plan.join(PHASE_FILENAME)).unwrap();
        assert_eq!(content.trim(), "analyse-work");
    }

    #[test]
    fn set_phase_rejects_missing_phase_md() {
        let tmp = TempDir::new().unwrap();
        let plan = tmp.path();
        // Deliberately no phase.md — simulates a typo'd plan-dir arg.
        let err = run_set_phase(plan, "reflect").unwrap_err();
        let msg = format!("{err:#}");
        assert!(msg.contains(PHASE_FILENAME), "error must name {PHASE_FILENAME}: {msg}");
        assert!(!plan.join(PHASE_FILENAME).exists(), "must not silently create phase.md");
    }

    #[test]
    fn set_phase_rejects_typo_and_lists_valid_phases() {
        let tmp = TempDir::new().unwrap();
        let plan = tmp.path();
        std::fs::write(plan.join(PHASE_FILENAME), "work").unwrap();

        let err = run_set_phase(plan, "analyze-work").unwrap_err();
        let msg = format!("{err:#}");
        assert!(msg.contains("analyze-work"), "error must include the bad input: {msg}");
        // Enumeration of valid phase names in the error lets the LLM
        // self-correct without a second round-trip.
        for valid in ["triage", "work", "analyse-work", "reflect",
                      "git-commit-triage", "git-commit-work",
                      "git-commit-analyse-work", "git-commit-reflect"] {
            assert!(msg.contains(valid), "error must list '{valid}': {msg}");
        }

        let content = std::fs::read_to_string(plan.join(PHASE_FILENAME)).unwrap();
        assert_eq!(content.trim(), "work", "phase.md must be unchanged on error");
    }
}
