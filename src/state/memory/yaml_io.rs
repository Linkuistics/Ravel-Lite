//! Atomic read/write of `<plan>/memory.yaml`.

use std::path::{Path, PathBuf};

use anyhow::{Context, Result};

use super::schema::{MemoryFile, MEMORY_SCHEMA_VERSION};
use crate::bail_with;
use crate::cli::error_context::ResultExt;
use crate::cli::ErrorCode;
use crate::state::filenames::MEMORY_FILENAME;

pub fn memory_path(plan_dir: &Path) -> PathBuf {
    plan_dir.join(MEMORY_FILENAME)
}

pub fn read_memory(plan_dir: &Path) -> Result<MemoryFile> {
    let path = memory_path(plan_dir);
    if !path.exists() {
        bail_with!(
            ErrorCode::NotFound,
            "{MEMORY_FILENAME} not found at {}. Run `ravel-lite state migrate` to convert an existing memory.md.",
            path.display()
        );
    }
    let text = std::fs::read_to_string(&path)
        .with_context(|| format!("Failed to read {}", path.display()))
        .with_code(ErrorCode::IoError)?;
    let parsed: MemoryFile = serde_yaml::from_str(&text)
        .with_context(|| format!("Failed to parse {} as {MEMORY_FILENAME} schema", path.display()))
        .with_code(ErrorCode::InvalidInput)?;
    if parsed.schema_version != MEMORY_SCHEMA_VERSION {
        bail_with!(
            ErrorCode::Conflict,
            "{} declares schema_version {}, expected {}.",
            path.display(),
            parsed.schema_version,
            MEMORY_SCHEMA_VERSION
        );
    }
    Ok(parsed)
}

pub fn write_memory(plan_dir: &Path, memory: &MemoryFile) -> Result<()> {
    let path = memory_path(plan_dir);
    let yaml = serde_yaml::to_string(memory)
        .with_context(|| format!("Failed to serialise {MEMORY_FILENAME}"))?;
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
    use crate::plan_kg::MemoryStatus;
    use crate::state::memory::schema::MemoryEntry;
    use knowledge_graph::{Item, Justification, KindMarker};
    use tempfile::TempDir;

    fn sample_entry() -> MemoryEntry {
        MemoryEntry {
            item: Item {
                id: "sample".into(),
                kind: KindMarker::new(),
                claim: "Sample claim".into(),
                justifications: vec![Justification::Rationale {
                    text: "Paragraph one.\n\nParagraph two, with `code`.\n".into(),
                }],
                status: MemoryStatus::Active,
                supersedes: vec![],
                superseded_by: None,
                defeated_by: None,
                authored_at: "2026-04-29T00:00:00Z".into(),
                authored_in: "test".into(),
            },
            attribution: None,
        }
    }

    #[test]
    fn write_then_read_round_trips_entry_fields() {
        let tmp = TempDir::new().unwrap();
        let memory = MemoryFile {
            schema_version: MEMORY_SCHEMA_VERSION,
            items: vec![sample_entry()],
        };
        write_memory(tmp.path(), &memory).unwrap();

        let round_tripped = read_memory(tmp.path()).unwrap();
        assert_eq!(round_tripped.items.len(), 1);
        assert_eq!(round_tripped.items[0].item.id, "sample");
        assert_eq!(round_tripped.items[0].item.claim, "Sample claim");
        assert_eq!(round_tripped.items[0].item.status, MemoryStatus::Active);
    }

    #[test]
    fn write_emits_block_scalar_for_multi_line_rationale() {
        let tmp = TempDir::new().unwrap();
        let memory = MemoryFile {
            schema_version: MEMORY_SCHEMA_VERSION,
            items: vec![sample_entry()],
        };
        write_memory(tmp.path(), &memory).unwrap();

        let raw = std::fs::read_to_string(memory_path(tmp.path())).unwrap();
        assert!(
            raw.contains("text: |") || raw.contains("text: |-"),
            "multi-line rationale text must emit as block scalar: {raw}"
        );
    }

    #[test]
    fn read_errors_when_memory_yaml_is_missing() {
        let tmp = TempDir::new().unwrap();
        let err = read_memory(tmp.path()).unwrap_err();
        let msg = format!("{err:#}");
        assert!(msg.contains(MEMORY_FILENAME), "error must name {MEMORY_FILENAME}: {msg}");
        assert!(msg.contains("state migrate"), "error must suggest migrate: {msg}");
    }

    #[test]
    fn read_errors_on_schema_version_mismatch() {
        let tmp = TempDir::new().unwrap();
        std::fs::write(memory_path(tmp.path()), "schema_version: 99\nitems: []\n").unwrap();
        let err = read_memory(tmp.path()).unwrap_err();
        let msg = format!("{err:#}");
        assert!(msg.contains("schema_version"), "error must cite schema_version: {msg}");
        assert!(msg.contains("99"), "error must show found version: {msg}");
    }
}
