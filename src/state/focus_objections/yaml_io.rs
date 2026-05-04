//! Atomic read/write/delete of `<plan>/focus-objections.yaml`.
//!
//! Mirrors `target_requests::yaml_io`: "missing = empty default" so
//! callers ask "what objections were raised?" without first checking
//! for the file. The drain semantic at the next-triage boundary is
//! "consume and remove", so `delete_focus_objections` is part of the
//! surface (idempotent).

use std::path::{Path, PathBuf};

use anyhow::{Context, Result};

use super::schema::{FocusObjectionsFile, FOCUS_OBJECTIONS_SCHEMA_VERSION};
use crate::bail_with;
use crate::cli::error_context::ResultExt;
use crate::cli::ErrorCode;
use crate::state::filenames::FOCUS_OBJECTIONS_FILENAME;

pub fn focus_objections_path(plan_dir: &Path) -> PathBuf {
    plan_dir.join(FOCUS_OBJECTIONS_FILENAME)
}

/// Read `<plan>/focus-objections.yaml`. Returns an empty default when
/// the file is absent.
///
/// Restart-not-resume contract (architecture-next §Recovery from
/// interrupted cycles): an absent file means "no objections raised
/// by the prior work phase" — the steady-state condition before the
/// LLM has had a chance to escalate, not an error. A
/// present-but-malformed file (or a `schema_version` mismatch) is
/// surfaced as a loud error: silently dropping objections would lose
/// LLM-authored escalation that the next triage needs to see.
/// Neither shape panics, so a fresh `ravel-lite run` after Ctrl-C
/// can always start, but malformed objections require user attention
/// before triage can drain them.
pub fn read_focus_objections(plan_dir: &Path) -> Result<FocusObjectionsFile> {
    let path = focus_objections_path(plan_dir);
    if !path.exists() {
        return Ok(FocusObjectionsFile::default());
    }
    let text = std::fs::read_to_string(&path)
        .with_context(|| format!("Failed to read {}", path.display()))
        .with_code(ErrorCode::IoError)?;
    let parsed: FocusObjectionsFile = serde_yaml::from_str(&text)
        .with_context(|| {
            format!(
                "Failed to parse {} as {FOCUS_OBJECTIONS_FILENAME} schema",
                path.display()
            )
        })
        .with_code(ErrorCode::InvalidInput)?;
    if parsed.schema_version != FOCUS_OBJECTIONS_SCHEMA_VERSION {
        bail_with!(
            ErrorCode::Conflict,
            "{} declares schema_version {}, expected {}.",
            path.display(),
            parsed.schema_version,
            FOCUS_OBJECTIONS_SCHEMA_VERSION
        );
    }
    Ok(parsed)
}

pub fn write_focus_objections(plan_dir: &Path, file: &FocusObjectionsFile) -> Result<()> {
    let path = focus_objections_path(plan_dir);
    let yaml = serde_yaml::to_string(file)
        .with_context(|| format!("Failed to serialise {FOCUS_OBJECTIONS_FILENAME}"))
        .with_code(ErrorCode::Internal)?;
    atomic_write(&path, yaml.as_bytes())
}

/// Remove the file from disk. Idempotent — the drain semantic is
/// "consume and remove" and a successful previous drain leaves nothing
/// behind.
pub fn delete_focus_objections(plan_dir: &Path) -> Result<()> {
    let path = focus_objections_path(plan_dir);
    match std::fs::remove_file(&path) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(e)
            .with_context(|| format!("Failed to remove {}", path.display()))
            .with_code(ErrorCode::IoError),
    }
}

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
    use crate::state::focus_objections::schema::Objection;
    use tempfile::TempDir;

    fn sample_file() -> FocusObjectionsFile {
        FocusObjectionsFile {
            schema_version: FOCUS_OBJECTIONS_SCHEMA_VERSION,
            objections: vec![Objection::Premature {
                reasoning: "Need to understand X first.\n".into(),
            }],
        }
    }

    #[test]
    fn read_returns_empty_default_when_file_is_absent() {
        let tmp = TempDir::new().unwrap();
        let parsed = read_focus_objections(tmp.path()).unwrap();
        assert_eq!(parsed.schema_version, FOCUS_OBJECTIONS_SCHEMA_VERSION);
        assert!(parsed.objections.is_empty());
    }

    #[test]
    fn write_then_read_round_trips_objections() {
        let tmp = TempDir::new().unwrap();
        write_focus_objections(tmp.path(), &sample_file()).unwrap();
        assert_eq!(read_focus_objections(tmp.path()).unwrap(), sample_file());
    }

    #[test]
    fn read_errors_on_schema_version_mismatch() {
        let tmp = TempDir::new().unwrap();
        std::fs::write(
            focus_objections_path(tmp.path()),
            "schema_version: 99\nobjections: []\n",
        )
        .unwrap();
        let err = read_focus_objections(tmp.path()).unwrap_err();
        let msg = format!("{err:#}");
        assert!(msg.contains("schema_version"), "error must cite schema_version: {msg}");
        assert!(msg.contains("99"), "error must show found version: {msg}");
    }

    #[test]
    fn read_bails_on_malformed_yaml() {
        // Restart-not-resume: a malformed file (e.g. an LLM authored
        // garbage, or Ctrl-C interrupted a non-atomic external writer)
        // surfaces as a loud error rather than panicking or silently
        // dropping objections. The error must cite the filename so the
        // user can locate the corrupt file.
        let tmp = TempDir::new().unwrap();
        std::fs::write(
            focus_objections_path(tmp.path()),
            "this: is not: a valid: shape\n",
        )
        .unwrap();
        let err = read_focus_objections(tmp.path()).unwrap_err();
        let msg = format!("{err:#}");
        assert!(
            msg.contains(FOCUS_OBJECTIONS_FILENAME),
            "error must cite the file: {msg}"
        );
    }

    #[test]
    fn delete_is_idempotent_when_file_missing() {
        let tmp = TempDir::new().unwrap();
        delete_focus_objections(tmp.path()).unwrap();
        delete_focus_objections(tmp.path()).unwrap();
    }

    #[test]
    fn delete_removes_existing_file() {
        let tmp = TempDir::new().unwrap();
        write_focus_objections(tmp.path(), &sample_file()).unwrap();
        assert!(focus_objections_path(tmp.path()).exists());
        delete_focus_objections(tmp.path()).unwrap();
        assert!(!focus_objections_path(tmp.path()).exists());
    }

    #[test]
    fn write_uses_atomic_rename_via_dot_tmp_path() {
        let tmp = TempDir::new().unwrap();
        write_focus_objections(tmp.path(), &sample_file()).unwrap();

        let final_path = focus_objections_path(tmp.path());
        assert!(final_path.exists(), "final file must be present after write");
        let tmp_path = tmp
            .path()
            .join(format!(".{FOCUS_OBJECTIONS_FILENAME}.tmp"));
        assert!(!tmp_path.exists(), "temp file must be renamed away after write");
    }
}
