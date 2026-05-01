//! Atomic read/write of `<plan>/targets.yaml`.
//!
//! `targets.yaml` is runtime state — born when the runner mounts the
//! first worktree, not at plan creation. Mirrors the
//! `findings::yaml_io::read_findings` "missing = empty" pattern rather
//! than the strict "missing is an error" pattern used by `intents`,
//! `backlog`, and `memory`, because callers should be able to ask
//! "what is mounted?" without first checking whether anything has been
//! mounted yet.

use std::path::{Path, PathBuf};

use anyhow::{bail, Context, Result};

use super::schema::{TargetsFile, TARGETS_SCHEMA_VERSION};
use crate::state::filenames::TARGETS_FILENAME;

pub fn targets_path(plan_dir: &Path) -> PathBuf {
    plan_dir.join(TARGETS_FILENAME)
}

/// Read `<plan>/targets.yaml`. Returns an empty (default) document when
/// the file does not yet exist — see module docs.
pub fn read_targets(plan_dir: &Path) -> Result<TargetsFile> {
    let path = targets_path(plan_dir);
    if !path.exists() {
        return Ok(TargetsFile::default());
    }
    let text = std::fs::read_to_string(&path)
        .with_context(|| format!("Failed to read {}", path.display()))?;
    let parsed: TargetsFile = serde_yaml::from_str(&text)
        .with_context(|| format!("Failed to parse {} as {TARGETS_FILENAME} schema", path.display()))?;
    if parsed.schema_version != TARGETS_SCHEMA_VERSION {
        bail!(
            "{} declares schema_version {}, expected {}.",
            path.display(),
            parsed.schema_version,
            TARGETS_SCHEMA_VERSION
        );
    }
    Ok(parsed)
}

pub fn write_targets(plan_dir: &Path, targets: &TargetsFile) -> Result<()> {
    let path = targets_path(plan_dir);
    let yaml = serde_yaml::to_string(targets)
        .with_context(|| format!("Failed to serialise {TARGETS_FILENAME}"))?;
    atomic_write(&path, yaml.as_bytes())
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
    use crate::state::targets::schema::Target;
    use tempfile::TempDir;

    fn sample_target() -> Target {
        Target {
            repo_slug: "atlas".into(),
            component_id: "atlas-ontology".into(),
            working_root: ".worktrees/atlas".into(),
            branch: "ravel-lite/sample-plan/main".into(),
            path_segments: vec!["crates".into(), "atlas-ontology".into()],
        }
    }

    #[test]
    fn read_returns_empty_default_when_targets_yaml_is_absent() {
        let tmp = TempDir::new().unwrap();
        let parsed = read_targets(tmp.path()).unwrap();
        assert_eq!(parsed.schema_version, TARGETS_SCHEMA_VERSION);
        assert!(parsed.targets.is_empty());
    }

    #[test]
    fn write_then_read_round_trips_target_fields() {
        let tmp = TempDir::new().unwrap();
        let targets = TargetsFile {
            schema_version: TARGETS_SCHEMA_VERSION,
            targets: vec![sample_target()],
        };
        write_targets(tmp.path(), &targets).unwrap();

        let round_tripped = read_targets(tmp.path()).unwrap();
        assert_eq!(round_tripped, targets);
    }

    #[test]
    fn read_errors_on_schema_version_mismatch() {
        let tmp = TempDir::new().unwrap();
        std::fs::write(targets_path(tmp.path()), "schema_version: 99\ntargets: []\n").unwrap();
        let err = read_targets(tmp.path()).unwrap_err();
        let msg = format!("{err:#}");
        assert!(msg.contains("schema_version"), "error must cite schema_version: {msg}");
        assert!(msg.contains("99"), "error must show found version: {msg}");
    }

    #[test]
    fn write_uses_atomic_rename_via_dot_tmp_path() {
        let tmp = TempDir::new().unwrap();
        let targets = TargetsFile {
            schema_version: TARGETS_SCHEMA_VERSION,
            targets: vec![sample_target()],
        };
        write_targets(tmp.path(), &targets).unwrap();

        let final_path = targets_path(tmp.path());
        assert!(final_path.exists(), "final file must be present after write");
        let tmp_path = tmp.path().join(format!(".{TARGETS_FILENAME}.tmp"));
        assert!(!tmp_path.exists(), "temp file must be renamed away after write");
    }
}
