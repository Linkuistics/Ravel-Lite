//! Atomic read/write of `<context>/findings.yaml`.

use std::path::{Path, PathBuf};

use anyhow::{Context, Result};

use super::schema::{FindingsFile, FINDINGS_SCHEMA_VERSION};
use crate::bail_with;
use crate::cli::error_context::ResultExt;
use crate::cli::ErrorCode;
use crate::state::filenames::FINDINGS_FILENAME;

pub fn findings_path(context_root: &Path) -> PathBuf {
    context_root.join(FINDINGS_FILENAME)
}

/// Read `<context>/findings.yaml`. Returns an empty (default) document
/// when the file does not yet exist — findings is an inbox, treating
/// "no inbox" the same as "empty inbox" lets callers avoid a separate
/// existence check before every read.
pub fn read_findings(context_root: &Path) -> Result<FindingsFile> {
    let path = findings_path(context_root);
    if !path.exists() {
        return Ok(FindingsFile::default());
    }
    let text = std::fs::read_to_string(&path)
        .with_context(|| format!("Failed to read {}", path.display()))
        .with_code(ErrorCode::IoError)?;
    let parsed: FindingsFile = serde_yaml::from_str(&text)
        .with_context(|| {
            format!("Failed to parse {} as {FINDINGS_FILENAME} schema", path.display())
        })
        .with_code(ErrorCode::InvalidInput)?;
    if parsed.schema_version != FINDINGS_SCHEMA_VERSION {
        bail_with!(
            ErrorCode::Conflict,
            "{} declares schema_version {}, expected {}.",
            path.display(),
            parsed.schema_version,
            FINDINGS_SCHEMA_VERSION
        );
    }
    Ok(parsed)
}

pub fn write_findings(context_root: &Path, findings: &FindingsFile) -> Result<()> {
    let path = findings_path(context_root);
    let yaml = serde_yaml::to_string(findings)
        .with_context(|| format!("Failed to serialise {FINDINGS_FILENAME}"))
        .with_code(ErrorCode::Internal)?;
    atomic_write(&path, yaml.as_bytes())
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
    use crate::plan_kg::FindingStatus;
    use crate::state::findings::schema::FindingEntry;
    use knowledge_graph::{Item, Justification, KindMarker};
    use tempfile::TempDir;

    fn sample_entry() -> FindingEntry {
        FindingEntry {
            item: Item {
                id: "sample".into(),
                kind: KindMarker::new(),
                claim: "Sample claim".into(),
                justifications: vec![Justification::Rationale {
                    text: "Why this is a finding.\n".into(),
                }],
                status: FindingStatus::New,
                supersedes: vec![],
                superseded_by: None,
                defeated_by: None,
                authored_at: "2026-04-30T00:00:00Z".into(),
                authored_in: "test".into(),
            },
            component: None,
            raised_in: None,
        }
    }

    #[test]
    fn read_returns_empty_when_findings_yaml_is_missing() {
        let tmp = TempDir::new().unwrap();
        let file = read_findings(tmp.path()).unwrap();
        assert_eq!(file.schema_version, FINDINGS_SCHEMA_VERSION);
        assert!(file.items.is_empty());
    }

    #[test]
    fn write_then_read_round_trips_entry_fields() {
        let tmp = TempDir::new().unwrap();
        let findings = FindingsFile {
            schema_version: FINDINGS_SCHEMA_VERSION,
            items: vec![sample_entry()],
        };
        write_findings(tmp.path(), &findings).unwrap();

        let round_tripped = read_findings(tmp.path()).unwrap();
        assert_eq!(round_tripped.items.len(), 1);
        assert_eq!(round_tripped.items[0].item.id, "sample");
        assert_eq!(round_tripped.items[0].item.claim, "Sample claim");
        assert_eq!(round_tripped.items[0].item.status, FindingStatus::New);
    }

    #[test]
    fn read_errors_on_schema_version_mismatch() {
        let tmp = TempDir::new().unwrap();
        std::fs::write(findings_path(tmp.path()), "schema_version: 99\nitems: []\n").unwrap();
        let err = read_findings(tmp.path()).unwrap_err();
        let msg = format!("{err:#}");
        assert!(msg.contains("schema_version"), "error must cite schema_version: {msg}");
        assert!(msg.contains("99"), "error must show found version: {msg}");
    }
}
