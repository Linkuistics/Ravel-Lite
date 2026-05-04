//! Bounded TMS check: validate `CodeAnchor` justifications against the
//! current working tree.
//!
//! For every memory entry that carries one or more `CodeAnchor`
//! justifications, this pass asks two mechanical questions per anchor:
//!
//! - Does the file at `path` (relative to `project_root`) exist?
//! - Is the blob SHA of its current contents equal to `sha_at_assertion`?
//!
//! The blob SHA is computed via `git hash-object`, which hashes file
//! contents using git's blob-object identity rule. This is independent
//! of commit state — an uncommitted edit is reflected immediately.
//!
//! See `docs/architecture-next.md` §Reflect's role on memory (bounded
//! TMS) for where this check sits in the reflect phase.

use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{Context, Result};

use crate::bail_with;
use crate::cli::error_context::ResultExt;
use crate::cli::ErrorCode;
use knowledge_graph::Justification;
use serde::{Deserialize, Serialize};

use super::schema::MemoryFile;

pub const REPORT_SCHEMA_VERSION: u32 = 1;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum SuspectReason {
    /// The file referenced by `path` does not exist under `project_root`.
    PathMissing,
    /// The file exists but its current blob SHA differs from
    /// `sha_at_assertion`.
    ShaMismatch,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Suspect {
    pub entry_id: String,
    pub component: String,
    pub path: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub lines: Option<String>,
    pub expected_sha: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub actual_sha: Option<String>,
    pub reason: SuspectReason,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SuspectReport {
    pub schema_version: u32,
    /// Number of `CodeAnchor` justifications visited (across all entries).
    pub checked: usize,
    pub suspects: Vec<Suspect>,
}

impl SuspectReport {
    pub fn empty() -> Self {
        Self {
            schema_version: REPORT_SCHEMA_VERSION,
            checked: 0,
            suspects: Vec::new(),
        }
    }
}

/// Walk active memory entries; visit each `CodeAnchor` justification;
/// produce a `SuspectReport`. Defeated and superseded entries are
/// skipped — the bounded check is for currently-asserted truth.
pub fn check_anchors(memory: &MemoryFile, project_root: &Path) -> Result<SuspectReport> {
    use crate::plan_kg::MemoryStatus;

    let mut report = SuspectReport::empty();
    for entry in &memory.items {
        if entry.item.status != MemoryStatus::Active {
            continue;
        }
        for justification in &entry.item.justifications {
            let Justification::CodeAnchor {
                component,
                path,
                lines,
                sha_at_assertion,
            } = justification
            else {
                continue;
            };
            report.checked += 1;
            let suspect = check_one_anchor(
                &entry.item.id,
                component,
                path,
                lines.as_deref(),
                sha_at_assertion,
                project_root,
            )?;
            if let Some(s) = suspect {
                report.suspects.push(s);
            }
        }
    }
    Ok(report)
}

fn check_one_anchor(
    entry_id: &str,
    component: &str,
    path: &str,
    lines: Option<&str>,
    expected_sha: &str,
    project_root: &Path,
) -> Result<Option<Suspect>> {
    let absolute = project_root.join(path);
    if !absolute.exists() {
        return Ok(Some(Suspect {
            entry_id: entry_id.to_string(),
            component: component.to_string(),
            path: path.to_string(),
            lines: lines.map(str::to_string),
            expected_sha: expected_sha.to_string(),
            actual_sha: None,
            reason: SuspectReason::PathMissing,
        }));
    }
    let actual_sha = git_hash_object(&absolute)?;
    if actual_sha == expected_sha {
        return Ok(None);
    }
    Ok(Some(Suspect {
        entry_id: entry_id.to_string(),
        component: component.to_string(),
        path: path.to_string(),
        lines: lines.map(str::to_string),
        expected_sha: expected_sha.to_string(),
        actual_sha: Some(actual_sha),
        reason: SuspectReason::ShaMismatch,
    }))
}

fn git_hash_object(path: &Path) -> Result<String> {
    let output = Command::new("git")
        .args(["hash-object", "--"])
        .arg(path)
        .output()
        .with_context(|| format!("failed to invoke `git hash-object -- {}`", path.display()))
        .with_code(ErrorCode::IoError)?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        bail_with!(
            ErrorCode::IoError,
            "`git hash-object -- {}` failed: {}",
            path.display(),
            stderr.trim()
        );
    }
    let stdout = String::from_utf8_lossy(&output.stdout);
    Ok(stdout.trim().to_string())
}

/// Convenience: read `<plan_dir>/memory.yaml`, run `check_anchors`, return.
pub fn check_anchors_from_disk(plan_dir: &Path, project_root: &Path) -> Result<SuspectReport> {
    let memory = super::yaml_io::read_memory(plan_dir)?;
    check_anchors(&memory, project_root)
}

/// Default project root for a plan dir, derived via the `<subtree>/<state-dir>/<plan>`
/// layout convention. Returned as a `PathBuf` so callers can join paths
/// against it.
pub fn default_project_root(plan_dir: &Path) -> Result<PathBuf> {
    let root_string = crate::git::project_root_for_plan(plan_dir)?;
    Ok(PathBuf::from(root_string))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::plan_kg::MemoryStatus;
    use crate::state::memory::schema::{MemoryEntry, MemoryFile, MEMORY_SCHEMA_VERSION};
    use knowledge_graph::{Item, KindMarker};
    use std::fs;
    use tempfile::TempDir;

    fn entry_with_anchor(
        id: &str,
        path: &str,
        sha_at_assertion: &str,
        status: MemoryStatus,
    ) -> MemoryEntry {
        MemoryEntry {
            item: Item {
                id: id.into(),
                kind: KindMarker::new(),
                claim: format!("claim for {id}"),
                justifications: vec![Justification::CodeAnchor {
                    component: "ravel-lite:cli".into(),
                    path: path.into(),
                    lines: Some("1-10".into()),
                    sha_at_assertion: sha_at_assertion.into(),
                }],
                status,
                supersedes: vec![],
                superseded_by: None,
                defeated_by: None,
                authored_at: "2026-04-29T00:00:00Z".into(),
                authored_in: "test".into(),
            },
            attribution: None,
        }
    }

    fn rationale_only_entry(id: &str) -> MemoryEntry {
        MemoryEntry {
            item: Item {
                id: id.into(),
                kind: KindMarker::new(),
                claim: format!("claim for {id}"),
                justifications: vec![Justification::Rationale {
                    text: "free prose only.\n".into(),
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

    fn write_file(root: &Path, rel: &str, contents: &str) -> String {
        let absolute = root.join(rel);
        if let Some(parent) = absolute.parent() {
            fs::create_dir_all(parent).unwrap();
        }
        fs::write(&absolute, contents).unwrap();
        // The blob SHA we'd expect is whatever `git hash-object` returns
        // for this content. Compute it once so the assertion matches the
        // exact algorithm under test rather than guessing.
        super::git_hash_object(&absolute).unwrap()
    }

    #[test]
    fn empty_memory_yields_empty_report() {
        let memory = MemoryFile::default();
        let project_root = TempDir::new().unwrap();
        let report = check_anchors(&memory, project_root.path()).unwrap();
        assert_eq!(report.checked, 0);
        assert!(report.suspects.is_empty());
    }

    #[test]
    fn rationale_only_entries_are_not_visited() {
        let memory = MemoryFile {
            schema_version: MEMORY_SCHEMA_VERSION,
            items: vec![rationale_only_entry("r-1"), rationale_only_entry("r-2")],
        };
        let project_root = TempDir::new().unwrap();
        let report = check_anchors(&memory, project_root.path()).unwrap();
        assert_eq!(report.checked, 0, "no code-anchor justifications to visit");
        assert!(report.suspects.is_empty());
    }

    #[test]
    fn matching_sha_produces_no_suspect() {
        let project_root = TempDir::new().unwrap();
        let sha = write_file(project_root.path(), "src/lib.rs", "pub fn hi() {}\n");

        let memory = MemoryFile {
            schema_version: MEMORY_SCHEMA_VERSION,
            items: vec![entry_with_anchor(
                "matches",
                "src/lib.rs",
                &sha,
                MemoryStatus::Active,
            )],
        };
        let report = check_anchors(&memory, project_root.path()).unwrap();
        assert_eq!(report.checked, 1);
        assert!(report.suspects.is_empty(), "matching anchor must not be suspect");
    }

    #[test]
    fn missing_file_yields_path_missing_suspect() {
        let project_root = TempDir::new().unwrap();
        let memory = MemoryFile {
            schema_version: MEMORY_SCHEMA_VERSION,
            items: vec![entry_with_anchor(
                "ghost",
                "src/never-existed.rs",
                "0000000000000000000000000000000000000000",
                MemoryStatus::Active,
            )],
        };
        let report = check_anchors(&memory, project_root.path()).unwrap();
        assert_eq!(report.checked, 1);
        assert_eq!(report.suspects.len(), 1);
        let suspect = &report.suspects[0];
        assert_eq!(suspect.entry_id, "ghost");
        assert_eq!(suspect.reason, SuspectReason::PathMissing);
        assert_eq!(suspect.actual_sha, None);
    }

    #[test]
    fn changed_content_yields_sha_mismatch_suspect() {
        let project_root = TempDir::new().unwrap();
        let original_sha = write_file(project_root.path(), "src/changed.rs", "pub fn before() {}\n");
        // Now overwrite the file's content; its blob SHA should differ.
        let new_sha = write_file(project_root.path(), "src/changed.rs", "pub fn after() {}\n");
        assert_ne!(original_sha, new_sha, "test harness sanity: writes must differ");

        let memory = MemoryFile {
            schema_version: MEMORY_SCHEMA_VERSION,
            items: vec![entry_with_anchor(
                "drifted",
                "src/changed.rs",
                &original_sha,
                MemoryStatus::Active,
            )],
        };
        let report = check_anchors(&memory, project_root.path()).unwrap();
        assert_eq!(report.checked, 1);
        assert_eq!(report.suspects.len(), 1);
        let suspect = &report.suspects[0];
        assert_eq!(suspect.entry_id, "drifted");
        assert_eq!(suspect.reason, SuspectReason::ShaMismatch);
        assert_eq!(suspect.expected_sha, original_sha);
        assert_eq!(suspect.actual_sha.as_deref(), Some(new_sha.as_str()));
    }

    #[test]
    fn defeated_entries_are_skipped() {
        let project_root = TempDir::new().unwrap();
        // No file written: an active entry would be a path-missing suspect,
        // but the entry below is defeated and must be ignored entirely.
        let memory = MemoryFile {
            schema_version: MEMORY_SCHEMA_VERSION,
            items: vec![entry_with_anchor(
                "old",
                "src/who-cares.rs",
                "abc",
                MemoryStatus::Defeated,
            )],
        };
        let report = check_anchors(&memory, project_root.path()).unwrap();
        assert_eq!(report.checked, 0);
        assert!(report.suspects.is_empty());
    }

    #[test]
    fn one_entry_with_multiple_anchors_each_checked_independently() {
        let project_root = TempDir::new().unwrap();
        let good_sha = write_file(project_root.path(), "src/ok.rs", "good\n");

        let mut entry = entry_with_anchor("multi", "src/ok.rs", &good_sha, MemoryStatus::Active);
        entry.item.justifications.push(Justification::CodeAnchor {
            component: "ravel-lite:cli".into(),
            path: "src/missing.rs".into(),
            lines: None,
            sha_at_assertion: "abc".into(),
        });

        let memory = MemoryFile {
            schema_version: MEMORY_SCHEMA_VERSION,
            items: vec![entry],
        };
        let report = check_anchors(&memory, project_root.path()).unwrap();
        assert_eq!(report.checked, 2, "both anchors visited");
        assert_eq!(report.suspects.len(), 1, "only the missing-path anchor is suspect");
        assert_eq!(report.suspects[0].path, "src/missing.rs");
        assert_eq!(report.suspects[0].reason, SuspectReason::PathMissing);
    }

    #[test]
    fn report_round_trips_through_yaml() {
        let report = SuspectReport {
            schema_version: REPORT_SCHEMA_VERSION,
            checked: 3,
            suspects: vec![
                Suspect {
                    entry_id: "e-1".into(),
                    component: "ravel-lite:cli".into(),
                    path: "src/a.rs".into(),
                    lines: Some("1-5".into()),
                    expected_sha: "deadbeef".into(),
                    actual_sha: Some("cafef00d".into()),
                    reason: SuspectReason::ShaMismatch,
                },
                Suspect {
                    entry_id: "e-2".into(),
                    component: "ravel-lite:cli".into(),
                    path: "src/b.rs".into(),
                    lines: None,
                    expected_sha: "abc".into(),
                    actual_sha: None,
                    reason: SuspectReason::PathMissing,
                },
            ],
        };
        let yaml = serde_yaml::to_string(&report).unwrap();
        let decoded: SuspectReport = serde_yaml::from_str(&yaml).unwrap();
        assert_eq!(decoded, report);
    }
}
