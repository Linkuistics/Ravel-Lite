//! Read-only graph-RAG queries over the union of registered repos'
//! `.atlas/components.yaml` files. Companion to the Atlas indexer
//! (sibling `atlas-contracts` workspace), which produces those files.
//!
//! See `docs/architecture-next.md` §"Catalog as graph (graph-RAG)" and
//! §"CLI surface" for the full verb list. This module currently
//! implements two of the simplest verbs:
//!
//! - `list-repos` — registry overview (delegates to `repos::run_list`).
//! - `freshness` — per-repo `.atlas/components.yaml` presence + age,
//!   with `--require-fresh` exiting non-zero when any catalog is
//!   absent or unparseable.
//!
//! The remaining graph-query verbs (`list-components`, `describe`,
//! `edges`, `neighbors`, `path`, `scc`, `roots`, `summary`, `memory`)
//! need the full catalog union + edge-graph machinery and are tracked
//! as follow-ups in the parent backlog task.

use std::path::{Path, PathBuf};
use std::time::SystemTime;

use anyhow::{bail, Context, Result};
use serde::Serialize;

use atlas_index::load_components;

use crate::repos::{self, RepoEntry, ReposRegistry};

const ATLAS_DIR: &str = ".atlas";
const COMPONENTS_FILENAME: &str = "components.yaml";

/// Per-repo overview, read-only. Same data as `ravel-lite repo list`;
/// surfaced here under `atlas` because the graph-RAG mental model
/// (docs/architecture-next.md §"Catalog as graph") treats the repo
/// registry as the entry point to the catalog graph, separate from
/// the `repo` registry-management surface.
pub fn run_list_repos(context_root: &Path) -> Result<()> {
    repos::run_list(context_root)
}

/// Per-repo `.atlas/components.yaml` presence + age check. Always
/// emits the YAML report; with `require_fresh`, additionally errors
/// non-zero if any repo's catalog status is not `fresh`.
pub fn run_freshness(context_root: &Path, require_fresh: bool) -> Result<()> {
    let registry = repos::load_or_empty(context_root)?;
    let report = compute_freshness(&registry, SystemTime::now());
    let yaml = serde_yaml::to_string(&FreshnessReport {
        freshness: report.clone(),
    })
    .context("Failed to serialise atlas freshness report to YAML")?;
    print!("{yaml}");
    if require_fresh {
        let stale: Vec<&str> = report
            .iter()
            .filter(|f| f.status != FreshnessStatus::Fresh)
            .map(|f| f.repo.as_str())
            .collect();
        if !stale.is_empty() {
            bail!(
                "atlas freshness check failed: {} of {} repo(s) lack a fresh catalog: [{}]",
                stale.len(),
                report.len(),
                stale.join(", "),
            );
        }
    }
    Ok(())
}

#[derive(Debug, Serialize)]
struct FreshnessReport {
    freshness: Vec<RepoFreshness>,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
struct RepoFreshness {
    repo: String,
    status: FreshnessStatus,
    #[serde(skip_serializing_if = "Option::is_none")]
    components_yaml: Option<PathBuf>,
    #[serde(skip_serializing_if = "Option::is_none")]
    generated_at: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    mtime_age_seconds: Option<u64>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
enum FreshnessStatus {
    /// `.atlas/components.yaml` present and parses cleanly.
    Fresh,
    /// repo's `local_path` resolves to an existing dir but the
    /// `.atlas/components.yaml` file under it is absent — Atlas
    /// has not run on this repo yet.
    Missing,
    /// repo has no `local_path` configured in `repos.yaml`; we
    /// have nothing to look at.
    NoLocalPath,
    /// `.atlas/components.yaml` exists on disk but does not
    /// deserialise — Atlas wrote a corrupt/version-mismatched
    /// file or someone hand-edited it.
    Unparseable,
}

/// Pure function: reports the freshness state for every repo in
/// `registry`. Takes `now` as a parameter rather than reading the
/// system clock so unit tests can pin the age computation.
fn compute_freshness(registry: &ReposRegistry, now: SystemTime) -> Vec<RepoFreshness> {
    registry
        .repos
        .iter()
        .map(|(slug, entry)| compute_one(slug, entry, now))
        .collect()
}

fn compute_one(slug: &str, entry: &RepoEntry, now: SystemTime) -> RepoFreshness {
    let Some(local_path) = entry.local_path.as_deref() else {
        return RepoFreshness {
            repo: slug.to_string(),
            status: FreshnessStatus::NoLocalPath,
            components_yaml: None,
            generated_at: None,
            mtime_age_seconds: None,
        };
    };
    let components_path = local_path.join(ATLAS_DIR).join(COMPONENTS_FILENAME);
    if !components_path.exists() {
        return RepoFreshness {
            repo: slug.to_string(),
            status: FreshnessStatus::Missing,
            components_yaml: Some(components_path),
            generated_at: None,
            mtime_age_seconds: None,
        };
    }
    let mtime_age_seconds = std::fs::metadata(&components_path)
        .ok()
        .and_then(|m| m.modified().ok())
        .and_then(|mtime| now.duration_since(mtime).ok())
        .map(|d| d.as_secs());
    match load_components(&components_path) {
        Ok(file) => RepoFreshness {
            repo: slug.to_string(),
            status: FreshnessStatus::Fresh,
            components_yaml: Some(components_path),
            generated_at: Some(file.generated_at),
            mtime_age_seconds,
        },
        Err(_) => RepoFreshness {
            repo: slug.to_string(),
            status: FreshnessStatus::Unparseable,
            components_yaml: Some(components_path),
            generated_at: None,
            mtime_age_seconds,
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;
    use tempfile::TempDir;

    fn write_minimal_components_yaml(path: &Path) {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        let yaml = "schema_version: 1\n\
                    root: /tmp\n\
                    generated_at: 2026-04-24T00:00:00Z\n\
                    cache_fingerprints:\n  \
                      ontology_sha: ''\n  \
                      model_id: ''\n  \
                      backend_version: ''\n\
                    components: []\n";
        std::fs::write(path, yaml).unwrap();
    }

    fn empty_registry() -> ReposRegistry {
        ReposRegistry::default()
    }

    #[test]
    fn empty_registry_yields_empty_report() {
        let report = compute_freshness(&empty_registry(), SystemTime::now());
        assert!(report.is_empty());
    }

    #[test]
    fn repo_without_local_path_is_no_local_path() {
        let mut reg = empty_registry();
        repos::try_add(&mut reg, "ravel-lite", "u", None).unwrap();
        let report = compute_freshness(&reg, SystemTime::now());
        assert_eq!(report.len(), 1);
        assert_eq!(report[0].status, FreshnessStatus::NoLocalPath);
        assert_eq!(report[0].repo, "ravel-lite");
        assert!(report[0].components_yaml.is_none());
    }

    #[test]
    fn repo_with_local_path_but_no_atlas_dir_is_missing() {
        let tmp = TempDir::new().unwrap();
        let repo_dir = tmp.path().join("repo");
        std::fs::create_dir_all(&repo_dir).unwrap();
        let mut reg = empty_registry();
        repos::try_add(&mut reg, "atlas", "u", Some(&repo_dir)).unwrap();
        let report = compute_freshness(&reg, SystemTime::now());
        assert_eq!(report[0].status, FreshnessStatus::Missing);
        assert!(report[0].components_yaml.is_some());
        assert!(report[0].generated_at.is_none());
    }

    #[test]
    fn repo_with_valid_components_yaml_is_fresh() {
        let tmp = TempDir::new().unwrap();
        let repo_dir = tmp.path().join("repo");
        std::fs::create_dir_all(&repo_dir).unwrap();
        write_minimal_components_yaml(&repo_dir.join(ATLAS_DIR).join(COMPONENTS_FILENAME));
        let mut reg = empty_registry();
        repos::try_add(&mut reg, "atlas", "u", Some(&repo_dir)).unwrap();
        let now = SystemTime::now() + Duration::from_secs(60);
        let report = compute_freshness(&reg, now);
        assert_eq!(report[0].status, FreshnessStatus::Fresh);
        assert_eq!(report[0].generated_at.as_deref(), Some("2026-04-24T00:00:00Z"));
        let age = report[0].mtime_age_seconds.expect("age must be reported for fresh");
        assert!(age >= 60, "age {age} should be at least 60s given +60s now offset");
    }

    #[test]
    fn unparseable_components_yaml_is_reported_unparseable() {
        let tmp = TempDir::new().unwrap();
        let repo_dir = tmp.path().join("repo");
        let atlas_dir = repo_dir.join(ATLAS_DIR);
        std::fs::create_dir_all(&atlas_dir).unwrap();
        std::fs::write(atlas_dir.join(COMPONENTS_FILENAME), "this: is: not: valid yaml ::").unwrap();
        let mut reg = empty_registry();
        repos::try_add(&mut reg, "atlas", "u", Some(&repo_dir)).unwrap();
        let report = compute_freshness(&reg, SystemTime::now());
        assert_eq!(report[0].status, FreshnessStatus::Unparseable);
        assert!(report[0].generated_at.is_none());
        assert!(report[0].mtime_age_seconds.is_some(), "mtime is still measurable on a parse failure");
    }

    #[test]
    fn report_preserves_registry_iteration_order() {
        let tmp = TempDir::new().unwrap();
        let mut reg = empty_registry();
        repos::try_add(&mut reg, "zeta", "u", None).unwrap();
        repos::try_add(&mut reg, "alpha", "u", None).unwrap();
        repos::try_add(&mut reg, "mu", "u", Some(&tmp.path().join("mu"))).unwrap();
        let report = compute_freshness(&reg, SystemTime::now());
        let slugs: Vec<&str> = report.iter().map(|r| r.repo.as_str()).collect();
        assert_eq!(slugs, vec!["zeta", "alpha", "mu"]);
    }

    #[test]
    fn yaml_output_is_stable_and_skips_none_fields() {
        let mut reg = empty_registry();
        repos::try_add(&mut reg, "ravel-lite", "u", None).unwrap();
        let report = compute_freshness(&reg, SystemTime::now());
        let yaml = serde_yaml::to_string(&FreshnessReport { freshness: report }).unwrap();
        assert!(yaml.contains("repo: ravel-lite"));
        assert!(yaml.contains("status: no_local_path"));
        assert!(!yaml.contains("components_yaml"), "absent fields must be omitted");
        assert!(!yaml.contains("generated_at"));
        assert!(!yaml.contains("mtime_age_seconds"));
    }
}
