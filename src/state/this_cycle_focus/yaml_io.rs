//! Atomic read/write/delete of `<plan>/this-cycle-focus.yaml`.
//!
//! The file is single-document — `read` returns `None` when absent
//! rather than constructing a default, because "no current focus" is
//! distinct from "focus on nothing": there are points in the cycle (the
//! moment after a focus is consumed and before the next triage runs)
//! when the file is correctly absent. Callers either have a focus or
//! don't.

use std::path::{Path, PathBuf};

use anyhow::{bail, Context, Result};

use super::schema::{ThisCycleFocus, THIS_CYCLE_FOCUS_SCHEMA_VERSION};
use crate::state::filenames::THIS_CYCLE_FOCUS_FILENAME;

pub fn this_cycle_focus_path(plan_dir: &Path) -> PathBuf {
    plan_dir.join(THIS_CYCLE_FOCUS_FILENAME)
}

/// Read `<plan>/this-cycle-focus.yaml`. Returns `Ok(None)` when the
/// file is absent; an absent focus is a normal state between cycles.
pub fn read_this_cycle_focus(plan_dir: &Path) -> Result<Option<ThisCycleFocus>> {
    let path = this_cycle_focus_path(plan_dir);
    if !path.exists() {
        return Ok(None);
    }
    let text = std::fs::read_to_string(&path)
        .with_context(|| format!("Failed to read {}", path.display()))?;
    let parsed: ThisCycleFocus = serde_yaml::from_str(&text).with_context(|| {
        format!(
            "Failed to parse {} as {THIS_CYCLE_FOCUS_FILENAME} schema",
            path.display()
        )
    })?;
    if parsed.schema_version != THIS_CYCLE_FOCUS_SCHEMA_VERSION {
        bail!(
            "{} declares schema_version {}, expected {}.",
            path.display(),
            parsed.schema_version,
            THIS_CYCLE_FOCUS_SCHEMA_VERSION
        );
    }
    Ok(Some(parsed))
}

pub fn write_this_cycle_focus(plan_dir: &Path, focus: &ThisCycleFocus) -> Result<()> {
    let path = this_cycle_focus_path(plan_dir);
    let yaml = serde_yaml::to_string(focus)
        .with_context(|| format!("Failed to serialise {THIS_CYCLE_FOCUS_FILENAME}"))?;
    atomic_write(&path, yaml.as_bytes())
}

/// Remove the file from disk. Idempotent: missing file is not an error.
/// Used at the work→analyse-work boundary when the cycle has consumed
/// its focus.
pub fn delete_this_cycle_focus(plan_dir: &Path) -> Result<()> {
    let path = this_cycle_focus_path(plan_dir);
    match std::fs::remove_file(&path) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(e).with_context(|| format!("Failed to remove {}", path.display())),
    }
}

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

    fn sample_focus() -> ThisCycleFocus {
        ThisCycleFocus {
            schema_version: THIS_CYCLE_FOCUS_SCHEMA_VERSION,
            target: "atlas:atlas-core".into(),
            backlog_items: vec!["t-001".into(), "t-005".into()],
            notes: Some("Order matters: t-001 first.\n".into()),
        }
    }

    #[test]
    fn read_returns_none_when_file_is_absent() {
        let tmp = TempDir::new().unwrap();
        let parsed = read_this_cycle_focus(tmp.path()).unwrap();
        assert!(parsed.is_none());
    }

    #[test]
    fn write_then_read_round_trips_focus() {
        let tmp = TempDir::new().unwrap();
        write_this_cycle_focus(tmp.path(), &sample_focus()).unwrap();
        let round_tripped = read_this_cycle_focus(tmp.path()).unwrap().unwrap();
        assert_eq!(round_tripped, sample_focus());
    }

    #[test]
    fn read_errors_on_schema_version_mismatch() {
        let tmp = TempDir::new().unwrap();
        std::fs::write(
            this_cycle_focus_path(tmp.path()),
            "schema_version: 99\ntarget: atlas:core\n",
        )
        .unwrap();
        let err = read_this_cycle_focus(tmp.path()).unwrap_err();
        let msg = format!("{err:#}");
        assert!(msg.contains("schema_version"), "error must cite schema_version: {msg}");
        assert!(msg.contains("99"), "error must show found version: {msg}");
    }

    #[test]
    fn delete_is_idempotent_when_file_missing() {
        let tmp = TempDir::new().unwrap();
        delete_this_cycle_focus(tmp.path()).unwrap();
        delete_this_cycle_focus(tmp.path()).unwrap();
    }

    #[test]
    fn delete_removes_existing_file() {
        let tmp = TempDir::new().unwrap();
        write_this_cycle_focus(tmp.path(), &sample_focus()).unwrap();
        assert!(this_cycle_focus_path(tmp.path()).exists());
        delete_this_cycle_focus(tmp.path()).unwrap();
        assert!(!this_cycle_focus_path(tmp.path()).exists());
        // and read returns None again
        assert!(read_this_cycle_focus(tmp.path()).unwrap().is_none());
    }

    #[test]
    fn write_uses_atomic_rename_via_dot_tmp_path() {
        let tmp = TempDir::new().unwrap();
        write_this_cycle_focus(tmp.path(), &sample_focus()).unwrap();

        let final_path = this_cycle_focus_path(tmp.path());
        assert!(final_path.exists(), "final file must be present after write");
        let tmp_path = tmp
            .path()
            .join(format!(".{THIS_CYCLE_FOCUS_FILENAME}.tmp"));
        assert!(!tmp_path.exists(), "temp file must be renamed away after write");
    }
}
