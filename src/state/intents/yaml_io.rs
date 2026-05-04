//! Atomic read/write of `<plan>/intents.yaml`.

use std::path::{Path, PathBuf};

use anyhow::{Context, Result};

use super::schema::{IntentsFile, INTENTS_SCHEMA_VERSION};
use crate::bail_with;
use crate::cli::error_context::ResultExt;
use crate::cli::ErrorCode;
use crate::state::filenames::INTENTS_FILENAME;

pub fn intents_path(plan_dir: &Path) -> PathBuf {
    plan_dir.join(INTENTS_FILENAME)
}

pub fn read_intents(plan_dir: &Path) -> Result<IntentsFile> {
    let path = intents_path(plan_dir);
    if !path.exists() {
        bail_with!(
            ErrorCode::NotFound,
            "{INTENTS_FILENAME} not found at {}. The plan may need to be re-scaffolded or migrated.",
            path.display()
        );
    }
    let text = std::fs::read_to_string(&path)
        .with_context(|| format!("Failed to read {}", path.display()))
        .with_code(ErrorCode::IoError)?;
    let parsed: IntentsFile = serde_yaml::from_str(&text)
        .with_context(|| format!("Failed to parse {} as {INTENTS_FILENAME} schema", path.display()))
        .with_code(ErrorCode::InvalidInput)?;
    if parsed.schema_version != INTENTS_SCHEMA_VERSION {
        bail_with!(
            ErrorCode::Conflict,
            "{} declares schema_version {}, expected {}.",
            path.display(),
            parsed.schema_version,
            INTENTS_SCHEMA_VERSION
        );
    }
    Ok(parsed)
}

pub fn write_intents(plan_dir: &Path, intents: &IntentsFile) -> Result<()> {
    let path = intents_path(plan_dir);
    let yaml = serde_yaml::to_string(intents)
        .with_context(|| format!("Failed to serialise {INTENTS_FILENAME}"))
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
    use crate::plan_kg::IntentStatus;
    use crate::state::intents::schema::IntentEntry;
    use knowledge_graph::{Item, Justification, KindMarker};
    use tempfile::TempDir;

    fn sample_entry() -> IntentEntry {
        IntentEntry {
            item: Item {
                id: "sample".into(),
                kind: KindMarker::new(),
                claim: "Sample claim".into(),
                justifications: vec![Justification::Rationale {
                    text: "Paragraph one.\n\nParagraph two, with `code`.\n".into(),
                }],
                status: IntentStatus::Active,
                supersedes: vec![],
                superseded_by: None,
                defeated_by: None,
                authored_at: "2026-04-29T00:00:00Z".into(),
                authored_in: "test".into(),
            },
        }
    }

    #[test]
    fn write_then_read_round_trips_entry_fields() {
        let tmp = TempDir::new().unwrap();
        let intents = IntentsFile {
            schema_version: INTENTS_SCHEMA_VERSION,
            items: vec![sample_entry()],
        };
        write_intents(tmp.path(), &intents).unwrap();

        let round_tripped = read_intents(tmp.path()).unwrap();
        assert_eq!(round_tripped.items.len(), 1);
        assert_eq!(round_tripped.items[0].item.id, "sample");
        assert_eq!(round_tripped.items[0].item.claim, "Sample claim");
        assert_eq!(round_tripped.items[0].item.status, IntentStatus::Active);
    }

    #[test]
    fn write_emits_block_scalar_for_multi_line_rationale() {
        let tmp = TempDir::new().unwrap();
        let intents = IntentsFile {
            schema_version: INTENTS_SCHEMA_VERSION,
            items: vec![sample_entry()],
        };
        write_intents(tmp.path(), &intents).unwrap();

        let raw = std::fs::read_to_string(intents_path(tmp.path())).unwrap();
        assert!(
            raw.contains("text: |") || raw.contains("text: |-"),
            "multi-line rationale text must emit as block scalar: {raw}"
        );
    }

    #[test]
    fn read_errors_when_intents_yaml_is_missing() {
        let tmp = TempDir::new().unwrap();
        let err = read_intents(tmp.path()).unwrap_err();
        let msg = format!("{err:#}");
        assert!(msg.contains(INTENTS_FILENAME), "error must name {INTENTS_FILENAME}: {msg}");
    }

    #[test]
    fn read_errors_on_schema_version_mismatch() {
        let tmp = TempDir::new().unwrap();
        std::fs::write(intents_path(tmp.path()), "schema_version: 99\nitems: []\n").unwrap();
        let err = read_intents(tmp.path()).unwrap_err();
        let msg = format!("{err:#}");
        assert!(msg.contains("schema_version"), "error must cite schema_version: {msg}");
        assert!(msg.contains("99"), "error must show found version: {msg}");
    }
}
