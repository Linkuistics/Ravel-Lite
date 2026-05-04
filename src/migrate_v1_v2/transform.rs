//! Half-A step 3: deterministic v1→v2 wire-shape transform for the
//! state YAML files copied in step 2.
//!
//! v1 backlog.yaml uses `tasks:` with `title`/`description`/`status:
//! not_started|in_progress|done|blocked` and no top-level
//! `schema_version`. v1 memory.yaml uses `entries:` with `title`/`body`.
//! Both must be reshaped to the v2 TMS-item form before
//! `apply_intent` reads them via `read_backlog`/`read_memory` (which
//! validate `schema_version: 1` and the `items:` key).
//!
//! Status collapse on backlog: `not_started` and `in_progress` both
//! map to `active` (the established v1→v2 status mapping per commit
//! 7a37e50, "four-state status collapses to BacklogStatus … dropping
//! in_progress").

use std::path::Path;

use anyhow::{Context, Result};
use knowledge_graph::{Item, Justification, KindMarker};
use serde::Deserialize;

use crate::bail_with;
use crate::cli::error_context::ResultExt;
use crate::cli::ErrorCode;
use crate::plan_kg::{BacklogStatus, MemoryStatus};
use crate::state::backlog::yaml_io::{backlog_path, write_backlog};
use crate::state::backlog::{BacklogEntry, BacklogFile, BACKLOG_SCHEMA_VERSION};
use crate::state::memory::yaml_io::{memory_path, write_memory};
use crate::state::memory::{MemoryEntry, MemoryFile, MEMORY_SCHEMA_VERSION};

#[derive(Deserialize)]
struct V1BacklogFile {
    #[serde(default)]
    tasks: Vec<V1BacklogTask>,
}

#[derive(Deserialize)]
struct V1BacklogTask {
    id: String,
    title: String,
    category: String,
    status: String,
    #[serde(default)]
    blocked_reason: Option<String>,
    #[serde(default)]
    dependencies: Vec<String>,
    #[serde(default)]
    description: String,
    #[serde(default)]
    results: Option<String>,
    #[serde(default)]
    handoff: Option<String>,
}

/// Reshape both state files in `plan_dir` from v1 to v2 wire shape.
/// `backlog.yaml` is required and will error if missing; `memory.yaml`
/// is optional (mirrors `copy::FILES_TO_COPY` semantics) and skipped
/// when absent.
pub fn run(plan_dir: &Path) -> Result<()> {
    let now = current_utc_rfc3339();
    transform_backlog_v1_to_v2(plan_dir, &now, "migrate-v1-v2")?;
    if memory_path(plan_dir).is_file() {
        transform_memory_v1_to_v2(plan_dir, &now, "migrate-v1-v2")?;
    }
    Ok(())
}

/// Reshape `<plan_dir>/backlog.yaml` from the v1 wire shape to the v2
/// TMS-item shape.
pub fn transform_backlog_v1_to_v2(
    plan_dir: &Path,
    authored_at: &str,
    authored_in: &str,
) -> Result<()> {
    let path = backlog_path(plan_dir);
    let text = std::fs::read_to_string(&path)
        .with_context(|| format!("read {}", path.display()))
        .with_code(ErrorCode::IoError)?;
    let v1: V1BacklogFile = serde_yaml::from_str(&text)
        .with_context(|| format!("parse {} as v1 backlog wire shape", path.display()))
        .with_code(ErrorCode::InvalidInput)?;

    let mut items = Vec::with_capacity(v1.tasks.len());
    for task in v1.tasks {
        let status = map_backlog_status(&task.status)
            .with_context(|| format!("task {:?}: unknown v1 status {:?}", task.id, task.status))
            .with_code(ErrorCode::InvalidInput)?;
        let justifications = if task.description.trim().is_empty() {
            Vec::new()
        } else {
            vec![Justification::Rationale {
                text: task.description,
            }]
        };
        items.push(BacklogEntry {
            item: Item {
                id: task.id,
                kind: KindMarker::new(),
                claim: task.title,
                justifications,
                status,
                supersedes: vec![],
                superseded_by: None,
                defeated_by: None,
                authored_at: authored_at.to_string(),
                authored_in: authored_in.to_string(),
            },
            category: task.category,
            blocked_reason: task.blocked_reason,
            dependencies: task.dependencies,
            results: task.results,
            handoff: task.handoff,
            legacy: false,
        });
    }

    write_backlog(
        plan_dir,
        &BacklogFile {
            schema_version: BACKLOG_SCHEMA_VERSION,
            items,
        },
    )
}

#[derive(Deserialize)]
struct V1MemoryFile {
    #[serde(default)]
    entries: Vec<V1MemoryEntry>,
}

#[derive(Deserialize)]
struct V1MemoryEntry {
    id: String,
    title: String,
    #[serde(default)]
    body: String,
}

/// Reshape `<plan_dir>/memory.yaml` from the v1 wire shape
/// (`entries:` / `title` / `body`) to the v2 TMS-item shape.
/// All migrated entries land in `MemoryStatus::Active`; the later
/// `migrate-memory-backfill` LLM phase decides which need
/// `attribution` (and which collapse to `Legacy` for user curation).
pub fn transform_memory_v1_to_v2(
    plan_dir: &Path,
    authored_at: &str,
    authored_in: &str,
) -> Result<()> {
    let path = memory_path(plan_dir);
    let text = std::fs::read_to_string(&path)
        .with_context(|| format!("read {}", path.display()))
        .with_code(ErrorCode::IoError)?;
    let v1: V1MemoryFile = serde_yaml::from_str(&text)
        .with_context(|| format!("parse {} as v1 memory wire shape", path.display()))
        .with_code(ErrorCode::InvalidInput)?;

    let mut items = Vec::with_capacity(v1.entries.len());
    for entry in v1.entries {
        let justifications = if entry.body.trim().is_empty() {
            Vec::new()
        } else {
            vec![Justification::Rationale { text: entry.body }]
        };
        items.push(MemoryEntry {
            item: Item {
                id: entry.id,
                kind: KindMarker::new(),
                claim: entry.title,
                justifications,
                status: MemoryStatus::Active,
                supersedes: vec![],
                superseded_by: None,
                defeated_by: None,
                authored_at: authored_at.to_string(),
                authored_in: authored_in.to_string(),
            },
            attribution: None,
        });
    }

    write_memory(
        plan_dir,
        &MemoryFile {
            schema_version: MEMORY_SCHEMA_VERSION,
            items,
        },
    )
}

/// Same UTC RFC3339 formatter the four state-verb modules duplicate
/// (see `state/memory/verbs.rs::current_utc_rfc3339`). Inlined here to
/// avoid a cross-module dependency on the discover pipeline; promote
/// to a shared module when the next site lands.
fn current_utc_rfc3339() -> String {
    use std::time::{SystemTime, UNIX_EPOCH};
    let secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    format_unix_utc(secs)
}

fn format_unix_utc(mut secs: u64) -> String {
    let seconds = (secs % 60) as u32;
    secs /= 60;
    let minutes = (secs % 60) as u32;
    secs /= 60;
    let hours = (secs % 24) as u32;
    let mut days = secs / 24;
    let mut year: u32 = 1970;
    loop {
        let year_days = if is_leap(year) { 366 } else { 365 };
        if days < year_days as u64 {
            break;
        }
        days -= year_days as u64;
        year += 1;
    }
    let month_lens: [u32; 12] = [
        31,
        if is_leap(year) { 29 } else { 28 },
        31,
        30,
        31,
        30,
        31,
        31,
        30,
        31,
        30,
        31,
    ];
    let mut month: u32 = 0;
    while month < 12 && days >= month_lens[month as usize] as u64 {
        days -= month_lens[month as usize] as u64;
        month += 1;
    }
    let day = (days as u32) + 1;
    let month_1based = month + 1;
    format!(
        "{year:04}-{month_1based:02}-{day:02}T{hours:02}:{minutes:02}:{seconds:02}Z"
    )
}

fn is_leap(year: u32) -> bool {
    (year.is_multiple_of(4) && !year.is_multiple_of(100)) || year.is_multiple_of(400)
}

fn map_backlog_status(s: &str) -> Result<BacklogStatus> {
    match s {
        "not_started" | "in_progress" | "active" => Ok(BacklogStatus::Active),
        "done" => Ok(BacklogStatus::Done),
        "blocked" => Ok(BacklogStatus::Blocked),
        "defeated" => Ok(BacklogStatus::Defeated),
        "superseded" => Ok(BacklogStatus::Superseded),
        _ => bail_with!(ErrorCode::InvalidInput, "no v2 mapping for status {:?}", s),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::backlog::yaml_io::read_backlog;
    use tempfile::TempDir;

    const V1_BACKLOG_REAL_SHAPE: &str = r#"tasks:
- id: first-task
  title: First task title
  category: architecture-next
  status: not_started
  dependencies: []
  description: |
    First task description.

    With multiple paragraphs.
- id: second-task
  title: Second task title
  category: infra
  status: blocked
  blocked_reason: 'Waiting on upstream'
  dependencies:
  - first-task
  description: |
    Second task description.
- id: third-task
  title: Third task in progress
  category: maintenance
  status: in_progress
  dependencies: []
  description: |
    Third task description.
- id: fourth-task
  title: Fourth task done
  category: maintenance
  status: done
  dependencies: []
  description: |
    Fourth task description.
"#;

    #[test]
    fn backlog_transform_produces_v2_shape_readable_by_read_backlog() {
        let tmp = TempDir::new().unwrap();
        std::fs::write(tmp.path().join("backlog.yaml"), V1_BACKLOG_REAL_SHAPE).unwrap();

        transform_backlog_v1_to_v2(tmp.path(), "2026-05-04T00:00:00Z", "migrate-v1-v2").unwrap();

        let backlog = read_backlog(tmp.path()).unwrap();
        assert_eq!(backlog.schema_version, BACKLOG_SCHEMA_VERSION);
        assert_eq!(backlog.items.len(), 4);

        let first = &backlog.items[0];
        assert_eq!(first.item.id, "first-task");
        assert_eq!(first.item.claim, "First task title");
        assert_eq!(first.item.status, BacklogStatus::Active, "not_started→active");
        assert_eq!(first.category, "architecture-next");
        assert_eq!(first.dependencies, Vec::<String>::new());
        assert_eq!(first.item.authored_at, "2026-05-04T00:00:00Z");
        assert_eq!(first.item.authored_in, "migrate-v1-v2");
        assert_eq!(first.item.justifications.len(), 1);
        match &first.item.justifications[0] {
            Justification::Rationale { text } => {
                assert!(
                    text.contains("First task description") && text.contains("multiple paragraphs"),
                    "rationale must carry full description body, got: {text:?}"
                );
            }
            other => panic!("expected Rationale, got {other:?}"),
        }
        assert!(!first.legacy);

        let second = &backlog.items[1];
        assert_eq!(second.item.status, BacklogStatus::Blocked);
        assert_eq!(second.blocked_reason.as_deref(), Some("Waiting on upstream"));
        assert_eq!(second.dependencies, vec!["first-task".to_string()]);

        let third = &backlog.items[2];
        assert_eq!(third.item.status, BacklogStatus::Active, "in_progress→active");

        let fourth = &backlog.items[3];
        assert_eq!(fourth.item.status, BacklogStatus::Done);
    }

    #[test]
    fn backlog_transform_errors_on_unknown_status() {
        let tmp = TempDir::new().unwrap();
        std::fs::write(
            tmp.path().join("backlog.yaml"),
            "tasks:\n- id: t\n  title: T\n  category: x\n  status: weird\n  description: ''\n",
        )
        .unwrap();
        let err =
            transform_backlog_v1_to_v2(tmp.path(), "2026-05-04T00:00:00Z", "migrate-v1-v2").unwrap_err();
        let msg = format!("{err:#}");
        assert!(msg.contains("weird"), "msg must cite bad status: {msg}");
    }

    const V1_MEMORY_REAL_SHAPE: &str = r#"entries:
- id: first-fact
  title: First fact title
  body: |
    First fact body.

    With multiple paragraphs.
- id: second-fact
  title: Second fact title
  body: |
    Second fact body.
"#;

    #[test]
    fn memory_transform_produces_v2_shape_readable_by_read_memory() {
        use crate::state::memory::yaml_io::read_memory;
        use crate::state::memory::MEMORY_SCHEMA_VERSION;
        let tmp = TempDir::new().unwrap();
        std::fs::write(tmp.path().join("memory.yaml"), V1_MEMORY_REAL_SHAPE).unwrap();

        transform_memory_v1_to_v2(tmp.path(), "2026-05-04T00:00:00Z", "migrate-v1-v2").unwrap();

        let memory = read_memory(tmp.path()).unwrap();
        assert_eq!(memory.schema_version, MEMORY_SCHEMA_VERSION);
        assert_eq!(memory.items.len(), 2);

        let first = &memory.items[0];
        assert_eq!(first.item.id, "first-fact");
        assert_eq!(first.item.claim, "First fact title");
        assert_eq!(first.item.status, MemoryStatus::Active);
        assert_eq!(first.item.authored_at, "2026-05-04T00:00:00Z");
        assert_eq!(first.item.authored_in, "migrate-v1-v2");
        assert!(first.attribution.is_none());
        match &first.item.justifications[0] {
            Justification::Rationale { text } => {
                assert!(
                    text.contains("First fact body") && text.contains("multiple paragraphs"),
                    "rationale must carry full body, got: {text:?}"
                );
            }
            other => panic!("expected Rationale, got {other:?}"),
        }
    }

    #[test]
    fn backlog_transform_handles_empty_description() {
        let tmp = TempDir::new().unwrap();
        std::fs::write(
            tmp.path().join("backlog.yaml"),
            "tasks:\n- id: t\n  title: T\n  category: x\n  status: not_started\n  description: ''\n",
        )
        .unwrap();
        transform_backlog_v1_to_v2(tmp.path(), "2026-05-04T00:00:00Z", "migrate-v1-v2").unwrap();
        let backlog = read_backlog(tmp.path()).unwrap();
        assert!(backlog.items[0].item.justifications.is_empty());
    }
}
