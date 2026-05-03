//! Detect and repair stale item statuses in a backlog.
//!
//! One drift mode is repaired automatically — it does not need
//! operator judgement, so every cycle spent re-asking the LLM to do it
//! is waste:
//!
//! 1. `blocked` + every structural `dependencies` entry now `done` →
//!    `active`. The blocker resolved; the item is unblocked.
//!
//! Other apparent drifts (notably `active` with a non-empty `results`
//! field) are intentionally NOT repaired. Without an `in_progress`
//! discriminator (gone with the TMS-shape migration), `active` covers
//! both "not started" and "work staged but not yet flipped to done".
//! A result on an active item could equally signal an
//! operator-staged change, an amended record, or a forgotten flip —
//! flipping silently would lose information in the first two cases.
//!
//! The scan is pure; `run_repair_stale_statuses` reads the backlog,
//! applies the repairs in memory (unless `--dry-run`), writes the file
//! back, and emits the report.

use std::collections::HashSet;
use std::path::Path;

use anyhow::Result;

use crate::bail_with;
use crate::cli::ErrorCode;
use serde::{Deserialize, Serialize};

use crate::cli::OutputFormat;
use crate::plan_kg::BacklogStatus;

use super::schema::{BacklogEntry, BacklogFile};
use super::yaml_io::{read_backlog, write_backlog};

#[derive(Debug, Clone, Copy, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RepairReason {
    DependenciesSatisfied,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Repair {
    pub task_id: String,
    pub old_status: BacklogStatus,
    pub new_status: BacklogStatus,
    pub reason: RepairReason,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct RepairReport {
    pub repairs: Vec<Repair>,
}

/// Pure analysis: returns the list of repairs that *would* apply to
/// `backlog`, without mutating it. Filesystem-independent so unit
/// tests can pin the semantics without tempdir setup.
pub fn analyse_repairs(backlog: &BacklogFile) -> RepairReport {
    let done_ids: HashSet<&str> = backlog
        .items
        .iter()
        .filter(|e| e.item.status == BacklogStatus::Done)
        .map(|e| e.item.id.as_str())
        .collect();

    let repairs = backlog
        .items
        .iter()
        .filter_map(|entry| detect_repair(entry, &done_ids))
        .collect();

    RepairReport { repairs }
}

fn detect_repair(entry: &BacklogEntry, done_ids: &HashSet<&str>) -> Option<Repair> {
    if entry.item.status == BacklogStatus::Blocked
        && dependencies_satisfied(&entry.dependencies, done_ids)
    {
        Some(Repair {
            task_id: entry.item.id.clone(),
            old_status: BacklogStatus::Blocked,
            new_status: BacklogStatus::Active,
            reason: RepairReason::DependenciesSatisfied,
        })
    } else {
        None
    }
}

/// A `blocked` item auto-unblocks only when it has at least one
/// structural dependency AND every one is `done`. With zero explicit
/// dependencies the blocker is external (operator decision, upstream
/// project, awaiting review) and must not silently resolve.
fn dependencies_satisfied(dependencies: &[String], done_ids: &HashSet<&str>) -> bool {
    if dependencies.is_empty() {
        return false;
    }
    dependencies.iter().all(|dep| done_ids.contains(dep.as_str()))
}

/// CLI entry point: load `<plan_dir>/backlog.yaml`, compute the
/// repairs, optionally apply them, write back, emit the report.
/// Returns the number of repairs applied (or that *would* apply, under
/// `--dry-run`) so the dispatcher can exit non-zero as a scripting
/// signal without the caller having to re-parse the output.
pub fn run_repair_stale_statuses(
    plan_dir: &Path,
    dry_run: bool,
    format: OutputFormat,
) -> Result<usize> {
    let mut backlog = read_backlog(plan_dir)?;
    let report = analyse_repairs(&backlog);

    if !dry_run && !report.repairs.is_empty() {
        apply_repairs(&mut backlog, &report);
        write_backlog(plan_dir, &backlog)?;
    }

    emit(&report, format)?;
    Ok(report.repairs.len())
}

fn apply_repairs(backlog: &mut BacklogFile, report: &RepairReport) {
    for repair in &report.repairs {
        if let Some(entry) = backlog.items.iter_mut().find(|e| e.item.id == repair.task_id) {
            entry.item.status = repair.new_status;
            // Unblocking an item must clear `blocked_reason` —
            // leaving the reason behind would fossilise a stale
            // blocker note on a now-actionable item.
            if repair.new_status != BacklogStatus::Blocked {
                entry.blocked_reason = None;
            }
        }
    }
}

fn emit(report: &RepairReport, format: OutputFormat) -> Result<()> {
    let serialised = match format {
        OutputFormat::Yaml => serde_yaml::to_string(report)?,
        OutputFormat::Json => serde_json::to_string_pretty(report)? + "\n",
        OutputFormat::Markdown => {
            bail_with!(
                ErrorCode::InvalidInput,
                "`backlog repair-stale-statuses` does not support --format markdown; use yaml or json"
            )
        }
    };
    print!("{serialised}");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::backlog::schema::BACKLOG_SCHEMA_VERSION;
    use knowledge_graph::{Item, Justification, KindMarker};
    use tempfile::TempDir;

    fn entry(
        id: &str,
        status: BacklogStatus,
        deps: &[&str],
        results: Option<&str>,
    ) -> BacklogEntry {
        BacklogEntry {
            item: Item {
                id: id.into(),
                kind: KindMarker::new(),
                claim: id.into(),
                justifications: vec![Justification::Rationale {
                    text: "body\n".into(),
                }],
                status,
                supersedes: vec![],
                superseded_by: None,
                defeated_by: None,
                authored_at: "test".into(),
                authored_in: "test".into(),
            },
            category: "maintenance".into(),
            blocked_reason: if status == BacklogStatus::Blocked {
                Some("upstream".into())
            } else {
                None
            },
            dependencies: deps.iter().map(|s| s.to_string()).collect(),
            results: results.map(String::from),
            handoff: None,
        }
    }

    fn backlog_with(items: Vec<BacklogEntry>) -> BacklogFile {
        BacklogFile {
            schema_version: BACKLOG_SCHEMA_VERSION,
            items,
        }
    }

    #[test]
    fn active_with_results_is_not_repaired() {
        // The merge from in_progress + not_started into Active loses
        // the discriminator that distinguished "forgot to flip" from
        // "operator-staged result". Repairing here would risk the
        // latter; the rule is dropped intentionally.
        let backlog = backlog_with(vec![entry(
            "foo",
            BacklogStatus::Active,
            &[],
            Some("did the thing\n"),
        )]);
        assert_eq!(analyse_repairs(&backlog), RepairReport::default());
    }

    #[test]
    fn blocked_with_all_deps_done_repairs_to_active() {
        let backlog = backlog_with(vec![
            entry("foo", BacklogStatus::Done, &[], Some("done\n")),
            entry("bar", BacklogStatus::Done, &[], Some("done\n")),
            entry("baz", BacklogStatus::Blocked, &["foo", "bar"], None),
        ]);
        let report = analyse_repairs(&backlog);
        assert_eq!(report.repairs.len(), 1);
        let r = &report.repairs[0];
        assert_eq!(r.task_id, "baz");
        assert_eq!(r.old_status, BacklogStatus::Blocked);
        assert_eq!(r.new_status, BacklogStatus::Active);
        assert_eq!(r.reason, RepairReason::DependenciesSatisfied);
    }

    #[test]
    fn blocked_with_some_deps_pending_is_not_repaired() {
        let backlog = backlog_with(vec![
            entry("foo", BacklogStatus::Done, &[], Some("done\n")),
            entry("bar", BacklogStatus::Active, &[], None),
            entry("baz", BacklogStatus::Blocked, &["foo", "bar"], None),
        ]);
        assert_eq!(analyse_repairs(&backlog), RepairReport::default());
    }

    #[test]
    fn blocked_with_no_dependencies_is_not_auto_unblocked() {
        let backlog = backlog_with(vec![entry("foo", BacklogStatus::Blocked, &[], None)]);
        assert_eq!(analyse_repairs(&backlog), RepairReport::default());
    }

    #[test]
    fn empty_backlog_yields_empty_report() {
        let backlog = backlog_with(vec![]);
        assert_eq!(analyse_repairs(&backlog), RepairReport::default());
    }

    #[test]
    fn run_applies_repairs_and_writes_back_when_not_dry_run() {
        let tmp = TempDir::new().unwrap();
        let backlog = backlog_with(vec![
            entry("foo", BacklogStatus::Done, &[], Some("done\n")),
            entry("baz", BacklogStatus::Blocked, &["foo"], None),
        ]);
        write_backlog(tmp.path(), &backlog).unwrap();

        let count = run_repair_stale_statuses(tmp.path(), false, OutputFormat::Yaml).unwrap();
        assert_eq!(count, 1, "one repair expected (baz → active)");

        let reloaded = read_backlog(tmp.path()).unwrap();
        let baz = reloaded.items.iter().find(|e| e.item.id == "baz").unwrap();
        assert_eq!(baz.item.status, BacklogStatus::Active);
        assert_eq!(
            baz.blocked_reason, None,
            "blocked_reason must clear when an item is unblocked"
        );
    }

    #[test]
    fn run_dry_run_reports_without_writing() {
        let tmp = TempDir::new().unwrap();
        let backlog = backlog_with(vec![
            entry("foo", BacklogStatus::Done, &[], Some("done\n")),
            entry("baz", BacklogStatus::Blocked, &["foo"], None),
        ]);
        write_backlog(tmp.path(), &backlog).unwrap();

        let count = run_repair_stale_statuses(tmp.path(), true, OutputFormat::Yaml).unwrap();
        assert_eq!(count, 1);

        let reloaded = read_backlog(tmp.path()).unwrap();
        let baz = reloaded.items.iter().find(|e| e.item.id == "baz").unwrap();
        assert_eq!(baz.item.status, BacklogStatus::Blocked);
    }

    #[test]
    fn run_on_empty_backlog_returns_zero_and_writes_nothing() {
        let tmp = TempDir::new().unwrap();
        let backlog = backlog_with(vec![]);
        write_backlog(tmp.path(), &backlog).unwrap();

        let count = run_repair_stale_statuses(tmp.path(), false, OutputFormat::Yaml).unwrap();
        assert_eq!(count, 0);
    }

    #[test]
    fn report_serialises_with_snake_case_reason_tag() {
        let report = RepairReport {
            repairs: vec![Repair {
                task_id: "bar".into(),
                old_status: BacklogStatus::Blocked,
                new_status: BacklogStatus::Active,
                reason: RepairReason::DependenciesSatisfied,
            }],
        };
        let yaml = serde_yaml::to_string(&report).unwrap();
        assert!(
            yaml.contains("dependencies_satisfied"),
            "yaml must use snake_case reason: {yaml}"
        );
    }
}
