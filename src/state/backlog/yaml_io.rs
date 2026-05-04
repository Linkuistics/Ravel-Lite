//! Atomic read/write of `<plan>/backlog.yaml`. Format preservation
//! note: serde_yaml 0.9 emits multi-line strings as `|` block scalars
//! automatically when they contain a newline, which renders results
//! and rationale bodies readably without escaping.

use std::path::{Path, PathBuf};

use anyhow::{Context, Result};

use super::schema::{BacklogFile, BACKLOG_SCHEMA_VERSION};
use crate::bail_with;
use crate::cli::error_context::ResultExt;
use crate::cli::ErrorCode;
use crate::state::filenames::BACKLOG_FILENAME;

pub fn backlog_path(plan_dir: &Path) -> PathBuf {
    plan_dir.join(BACKLOG_FILENAME)
}

pub fn read_backlog(plan_dir: &Path) -> Result<BacklogFile> {
    let path = backlog_path(plan_dir);
    if !path.exists() {
        bail_with!(
            ErrorCode::NotFound,
            "{BACKLOG_FILENAME} not found at {}. The plan must be a v2 layout — run `ravel-lite migrate-v1-v2 <old-plan-path> --as <name>` to convert a legacy plan.",
            path.display()
        );
    }
    let text = std::fs::read_to_string(&path)
        .with_context(|| format!("Failed to read {}", path.display()))
        .with_code(ErrorCode::IoError)?;
    let parsed: BacklogFile = serde_yaml::from_str(&text)
        .with_context(|| {
            format!("Failed to parse {} as {BACKLOG_FILENAME} schema", path.display())
        })
        .with_code(ErrorCode::InvalidInput)?;
    if parsed.schema_version != BACKLOG_SCHEMA_VERSION {
        bail_with!(
            ErrorCode::Conflict,
            "{} declares schema_version {}, expected {}.",
            path.display(),
            parsed.schema_version,
            BACKLOG_SCHEMA_VERSION
        );
    }
    Ok(parsed)
}

pub fn write_backlog(plan_dir: &Path, backlog: &BacklogFile) -> Result<()> {
    let path = backlog_path(plan_dir);
    let yaml = serde_yaml::to_string(backlog)
        .with_context(|| format!("Failed to serialise {BACKLOG_FILENAME}"))
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
    use crate::plan_kg::BacklogStatus;
    use crate::state::backlog::schema::BacklogEntry;
    use knowledge_graph::{Item, Justification, KindMarker};
    use tempfile::TempDir;

    fn sample_entry() -> BacklogEntry {
        BacklogEntry {
            item: Item {
                id: "sample".into(),
                kind: KindMarker::new(),
                claim: "Sample item".into(),
                justifications: vec![Justification::Rationale {
                    text: "Paragraph one.\n\nParagraph two, with `code`.\n".into(),
                }],
                status: BacklogStatus::Active,
                supersedes: vec![],
                superseded_by: None,
                defeated_by: None,
                authored_at: "2026-04-29T00:00:00Z".into(),
                authored_in: "test".into(),
            },
            category: "maintenance".into(),
            blocked_reason: None,
            dependencies: vec![],
            results: None,
            handoff: None,
            legacy: false,
        }
    }

    #[test]
    fn write_then_read_round_trips_entry_fields() {
        let tmp = TempDir::new().unwrap();
        let backlog = BacklogFile {
            schema_version: BACKLOG_SCHEMA_VERSION,
            items: vec![sample_entry()],
        };
        write_backlog(tmp.path(), &backlog).unwrap();

        let round_tripped = read_backlog(tmp.path()).unwrap();
        assert_eq!(round_tripped.items.len(), 1);
        assert_eq!(round_tripped.items[0].item.id, "sample");
        assert_eq!(round_tripped.items[0].item.claim, "Sample item");
        assert_eq!(round_tripped.items[0].item.status, BacklogStatus::Active);
    }

    #[test]
    fn write_emits_block_scalar_for_multi_line_rationale() {
        let tmp = TempDir::new().unwrap();
        let backlog = BacklogFile {
            schema_version: BACKLOG_SCHEMA_VERSION,
            items: vec![sample_entry()],
        };
        write_backlog(tmp.path(), &backlog).unwrap();

        let raw = std::fs::read_to_string(backlog_path(tmp.path())).unwrap();
        assert!(
            raw.contains("text: |") || raw.contains("text: |-"),
            "multi-line rationale text must emit as block scalar: {raw}"
        );
    }

    #[test]
    fn read_errors_when_backlog_yaml_is_missing() {
        let tmp = TempDir::new().unwrap();
        let err = read_backlog(tmp.path()).unwrap_err();
        let msg = format!("{err:#}");
        assert!(msg.contains(BACKLOG_FILENAME), "error must name {BACKLOG_FILENAME}: {msg}");
        assert!(msg.contains("migrate-v1-v2"), "error must suggest migrate: {msg}");
    }

    #[test]
    fn read_errors_on_schema_version_mismatch() {
        let tmp = TempDir::new().unwrap();
        std::fs::write(
            backlog_path(tmp.path()),
            "schema_version: 99\nitems: []\n",
        )
        .unwrap();
        let err = read_backlog(tmp.path()).unwrap_err();
        let msg = format!("{err:#}");
        assert!(msg.contains("schema_version"), "error must cite schema_version: {msg}");
        assert!(msg.contains("99"), "error must show found version: {msg}");
    }
}
