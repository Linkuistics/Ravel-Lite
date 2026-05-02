//! Atomic read/write/delete of `<plan>/commits.yaml`.
//!
//! Mirrors the `target_requests::yaml_io` "missing = empty default"
//! pattern: callers can ask "what is queued?" without first checking
//! whether the file exists. Adds `delete_commits` because
//! `apply_commits_spec` must consume-then-apply (the file deletes
//! BEFORE git operations so a `paths: ["."]` entry doesn't sweep the
//! spec into its own commit).
//!
//! Unlike the TMS file readers, this one does not bail on a
//! `schema_version` mismatch — see `schema.rs` for the rationale.

use std::path::{Path, PathBuf};

use anyhow::{Context, Result};

use super::schema::CommitsSpec;
use crate::state::filenames::COMMITS_FILENAME;

pub fn commits_path(plan_dir: &Path) -> PathBuf {
    plan_dir.join(COMMITS_FILENAME)
}

/// Read `<plan>/commits.yaml`. Returns `CommitsSpec::default()` when the
/// file is absent (the missing-file shape is a valid empty queue, not an
/// error). Bails on parse failure so CLI verbs can surface malformed
/// YAML to the user; callers that prefer to fall back silently —
/// notably `apply_commits_spec` — must do so at their own boundary.
///
/// Restart-not-resume contract (architecture-next §Recovery from
/// interrupted cycles): an absent file means "no in-flight commit
/// queue from a previous cycle" — it is the steady-state condition
/// between cycles, not an error. A present-but-malformed file means
/// the LLM or a bug authored content the runner can't trust; the
/// error returned here is the surfacing mechanism. Neither shape
/// panics, so a fresh `ravel-lite run` after Ctrl-C can always start
/// without manual cleanup of the scratch file.
pub fn read_commits(plan_dir: &Path) -> Result<CommitsSpec> {
    let path = commits_path(plan_dir);
    if !path.exists() {
        return Ok(CommitsSpec::default());
    }
    let text = std::fs::read_to_string(&path)
        .with_context(|| format!("Failed to read {}", path.display()))?;
    let parsed: CommitsSpec = serde_yaml::from_str(&text).with_context(|| {
        format!(
            "Failed to parse {} as {COMMITS_FILENAME} schema",
            path.display()
        )
    })?;
    Ok(parsed)
}

pub fn write_commits(plan_dir: &Path, spec: &CommitsSpec) -> Result<()> {
    let path = commits_path(plan_dir);
    let yaml = serde_yaml::to_string(spec)
        .with_context(|| format!("Failed to serialise {COMMITS_FILENAME}"))?;
    atomic_write(&path, yaml.as_bytes())
}

/// Remove the file from disk. Idempotent: missing file is not an error.
/// Used by `apply_commits_spec` to consume the spec before any git
/// operations, so a `paths: ["."]` catch-all entry can't sweep the
/// spec file into its own commit.
pub fn delete_commits(plan_dir: &Path) -> Result<()> {
    let path = commits_path(plan_dir);
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
    use crate::component_ref::ComponentRef;
    use crate::state::commits::schema::{CommitSpec, COMMITS_SCHEMA_VERSION};
    use tempfile::TempDir;

    fn sample_spec() -> CommitsSpec {
        CommitsSpec {
            schema_version: COMMITS_SCHEMA_VERSION,
            commits: vec![
                CommitSpec {
                    paths: vec!["src/**".into()],
                    message: "Wire greeting".into(),
                    target: Some(ComponentRef::new("ravel-lite", "phase-loop")),
                },
                CommitSpec {
                    paths: vec!["LLM_STATE/**".into()],
                    message: "Seed backlog entry".into(),
                    target: None,
                },
            ],
        }
    }

    #[test]
    fn read_returns_empty_default_when_commits_yaml_is_absent() {
        let tmp = TempDir::new().unwrap();
        let parsed = read_commits(tmp.path()).unwrap();
        assert_eq!(parsed.schema_version, COMMITS_SCHEMA_VERSION);
        assert!(parsed.commits.is_empty());
    }

    #[test]
    fn write_then_read_round_trips_spec() {
        let tmp = TempDir::new().unwrap();
        let spec = sample_spec();
        write_commits(tmp.path(), &spec).unwrap();
        assert_eq!(read_commits(tmp.path()).unwrap(), spec);
    }

    #[test]
    fn read_accepts_v1_wire_shape_without_schema_version_or_target() {
        // Verbatim shape an existing v1 analyse-work prompt would emit.
        let tmp = TempDir::new().unwrap();
        std::fs::write(
            commits_path(tmp.path()),
            "commits:\n  - paths: [\"src/**\"]\n    message: legacy\n",
        )
        .unwrap();

        let parsed = read_commits(tmp.path()).unwrap();
        assert_eq!(parsed.schema_version, COMMITS_SCHEMA_VERSION);
        assert_eq!(parsed.commits.len(), 1);
        assert_eq!(parsed.commits[0].message, "legacy");
        assert!(parsed.commits[0].target.is_none());
    }

    #[test]
    fn read_does_not_bail_on_unknown_schema_version() {
        // The reader is intentionally permissive about schema_version
        // (see schema.rs). A future version label must parse rather
        // than fail the work-commit phase loop.
        let tmp = TempDir::new().unwrap();
        std::fs::write(
            commits_path(tmp.path()),
            "schema_version: 99\ncommits: []\n",
        )
        .unwrap();
        let parsed = read_commits(tmp.path()).unwrap();
        assert_eq!(parsed.schema_version, 99);
    }

    #[test]
    fn read_bails_on_malformed_yaml() {
        // CLI verbs want loud failures; the silent-fallback behaviour
        // for `apply_commits_spec` is implemented at its own call site.
        let tmp = TempDir::new().unwrap();
        std::fs::write(
            commits_path(tmp.path()),
            "this: is not: a valid: spec\n",
        )
        .unwrap();
        let err = read_commits(tmp.path()).unwrap_err();
        let msg = format!("{err:#}");
        assert!(
            msg.contains(COMMITS_FILENAME),
            "error must cite the file: {msg}"
        );
    }

    #[test]
    fn delete_is_idempotent_when_file_missing() {
        let tmp = TempDir::new().unwrap();
        delete_commits(tmp.path()).unwrap();
        delete_commits(tmp.path()).unwrap();
    }

    #[test]
    fn delete_removes_existing_file() {
        let tmp = TempDir::new().unwrap();
        write_commits(tmp.path(), &sample_spec()).unwrap();
        assert!(commits_path(tmp.path()).exists());
        delete_commits(tmp.path()).unwrap();
        assert!(!commits_path(tmp.path()).exists());
    }

    #[test]
    fn write_uses_atomic_rename_via_dot_tmp_path() {
        let tmp = TempDir::new().unwrap();
        write_commits(tmp.path(), &sample_spec()).unwrap();

        let final_path = commits_path(tmp.path());
        assert!(final_path.exists(), "final file must be present after write");
        let tmp_path = tmp.path().join(format!(".{COMMITS_FILENAME}.tmp"));
        assert!(!tmp_path.exists(), "temp file must be renamed away after write");
    }
}
