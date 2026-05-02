//! Atomic read/write/delete of `<plan>/target-requests.yaml`.
//!
//! Mirrors the `targets::yaml_io` "missing = empty" pattern: callers ask
//! "what is queued?" without first checking whether the file exists.
//! Adds `delete_target_requests` because the drain semantic at phase
//! boundaries is "consume and remove" — see
//! `docs/architecture-next.md` §Phase boundaries.

use std::path::{Path, PathBuf};

use anyhow::{bail, Context, Result};

use super::schema::{TargetRequestsFile, TARGET_REQUESTS_SCHEMA_VERSION};
use crate::state::filenames::TARGET_REQUESTS_FILENAME;

pub fn target_requests_path(plan_dir: &Path) -> PathBuf {
    plan_dir.join(TARGET_REQUESTS_FILENAME)
}

/// Read `<plan>/target-requests.yaml`. Returns an empty default when
/// the file is absent — see module docs.
pub fn read_target_requests(plan_dir: &Path) -> Result<TargetRequestsFile> {
    let path = target_requests_path(plan_dir);
    if !path.exists() {
        return Ok(TargetRequestsFile::default());
    }
    let text = std::fs::read_to_string(&path)
        .with_context(|| format!("Failed to read {}", path.display()))?;
    let parsed: TargetRequestsFile = serde_yaml::from_str(&text).with_context(|| {
        format!(
            "Failed to parse {} as {TARGET_REQUESTS_FILENAME} schema",
            path.display()
        )
    })?;
    if parsed.schema_version != TARGET_REQUESTS_SCHEMA_VERSION {
        bail!(
            "{} declares schema_version {}, expected {}.",
            path.display(),
            parsed.schema_version,
            TARGET_REQUESTS_SCHEMA_VERSION
        );
    }
    Ok(parsed)
}

pub fn write_target_requests(plan_dir: &Path, requests: &TargetRequestsFile) -> Result<()> {
    let path = target_requests_path(plan_dir);
    let yaml = serde_yaml::to_string(requests)
        .with_context(|| format!("Failed to serialise {TARGET_REQUESTS_FILENAME}"))?;
    atomic_write(&path, yaml.as_bytes())
}

/// Remove the file from disk. Idempotent: missing file is not an error,
/// because the drain semantic is "consume and remove" and a successful
/// previous drain leaves nothing behind.
pub fn delete_target_requests(plan_dir: &Path) -> Result<()> {
    let path = target_requests_path(plan_dir);
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
    use crate::state::target_requests::schema::TargetRequest;
    use tempfile::TempDir;

    fn sample_request() -> TargetRequest {
        TargetRequest {
            component: ComponentRef::new("atlas", "atlas-ontology"),
            reason: "core schema needs work".to_string(),
        }
    }

    #[test]
    fn read_returns_empty_default_when_target_requests_yaml_is_absent() {
        let tmp = TempDir::new().unwrap();
        let parsed = read_target_requests(tmp.path()).unwrap();
        assert_eq!(parsed.schema_version, TARGET_REQUESTS_SCHEMA_VERSION);
        assert!(parsed.requests.is_empty());
    }

    #[test]
    fn write_then_read_round_trips_request_fields() {
        let tmp = TempDir::new().unwrap();
        let file = TargetRequestsFile {
            schema_version: TARGET_REQUESTS_SCHEMA_VERSION,
            requests: vec![sample_request()],
        };
        write_target_requests(tmp.path(), &file).unwrap();
        assert_eq!(read_target_requests(tmp.path()).unwrap(), file);
    }

    #[test]
    fn read_errors_on_schema_version_mismatch() {
        let tmp = TempDir::new().unwrap();
        std::fs::write(
            target_requests_path(tmp.path()),
            "schema_version: 99\nrequests: []\n",
        )
        .unwrap();
        let err = read_target_requests(tmp.path()).unwrap_err();
        let msg = format!("{err:#}");
        assert!(msg.contains("schema_version"), "error must cite schema_version: {msg}");
        assert!(msg.contains("99"), "error must show found version: {msg}");
    }

    #[test]
    fn delete_is_idempotent_when_file_missing() {
        let tmp = TempDir::new().unwrap();
        delete_target_requests(tmp.path()).unwrap();
        delete_target_requests(tmp.path()).unwrap();
    }

    #[test]
    fn delete_removes_existing_file() {
        let tmp = TempDir::new().unwrap();
        let file = TargetRequestsFile {
            schema_version: TARGET_REQUESTS_SCHEMA_VERSION,
            requests: vec![sample_request()],
        };
        write_target_requests(tmp.path(), &file).unwrap();
        assert!(target_requests_path(tmp.path()).exists());
        delete_target_requests(tmp.path()).unwrap();
        assert!(!target_requests_path(tmp.path()).exists());
    }

    #[test]
    fn write_uses_atomic_rename_via_dot_tmp_path() {
        let tmp = TempDir::new().unwrap();
        let file = TargetRequestsFile {
            schema_version: TARGET_REQUESTS_SCHEMA_VERSION,
            requests: vec![sample_request()],
        };
        write_target_requests(tmp.path(), &file).unwrap();

        let final_path = target_requests_path(tmp.path());
        assert!(final_path.exists(), "final file must be present after write");
        let tmp_path = tmp.path().join(format!(".{TARGET_REQUESTS_FILENAME}.tmp"));
        assert!(!tmp_path.exists(), "temp file must be renamed away after write");
    }
}
