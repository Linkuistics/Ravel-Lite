//! Read-only graph-RAG queries over the union of registered repos'
//! `.atlas/components.yaml` files. Companion to the Atlas indexer
//! (sibling `atlas-contracts` workspace), which produces those files.
//!
//! See `docs/architecture-next.md` §"Catalog as graph (graph-RAG)" and
//! §"CLI surface" for the full verb list. This module currently
//! implements:
//!
//! - `list-repos` — registry overview (delegates to `repos::run_list`).
//! - `freshness` — per-repo `.atlas/components.yaml` presence + age,
//!   with `--require-fresh` exiting non-zero when any catalog is
//!   absent or unparseable.
//! - `list-components` — list every component in every fresh repo,
//!   optionally filtered by `--repo` and/or `--kind`.
//! - `summary` — per-repo component counts grouped by kind.
//!
//! The remaining graph-query verbs (`describe`, `edges`, `neighbors`,
//! `path`, `scc`, `roots`, `memory`) need additional edge-graph
//! machinery and are tracked as follow-up backlog tasks.
//!
//! The shared in-memory representation is [`Catalog`]: the union of
//! every fresh repo's `ComponentsFile`, keyed by repo slug. Subsequent
//! verbs build on top of this loader instead of re-parsing
//! `components.yaml` per invocation.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use anyhow::{bail, Context, Result};
use indexmap::IndexMap;
use serde::Serialize;

use atlas_index::{load_components, ComponentEntry, ComponentsFile};

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
pub struct RepoFreshness {
    pub repo: String,
    pub status: FreshnessStatus,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub components_yaml: Option<PathBuf>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub generated_at: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub mtime_age_seconds: Option<u64>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum FreshnessStatus {
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
    compute_freshness_full(registry, now)
        .into_iter()
        .map(|(report, _data)| report)
        .collect()
}

/// Single source of I/O: walks the registry once, parsing each repo's
/// `components.yaml` at most once and threading the parsed result back
/// alongside its freshness report. `compute_freshness` discards the
/// parsed half; `Catalog::load` keeps it.
fn compute_freshness_full(
    registry: &ReposRegistry,
    now: SystemTime,
) -> Vec<(RepoFreshness, Option<LoadedComponents>)> {
    registry
        .repos
        .iter()
        .map(|(slug, entry)| compute_one_full(slug, entry, now))
        .collect()
}

/// Per-repo parsed `components.yaml` plus the path it came from. Only
/// produced for repos whose freshness status is `Fresh`.
struct LoadedComponents {
    local_path: PathBuf,
    components_yaml: PathBuf,
    file: ComponentsFile,
}

fn compute_one_full(
    slug: &str,
    entry: &RepoEntry,
    now: SystemTime,
) -> (RepoFreshness, Option<LoadedComponents>) {
    let Some(local_path) = entry.local_path.as_deref() else {
        return (
            RepoFreshness {
                repo: slug.to_string(),
                status: FreshnessStatus::NoLocalPath,
                components_yaml: None,
                generated_at: None,
                mtime_age_seconds: None,
            },
            None,
        );
    };
    let components_path = local_path.join(ATLAS_DIR).join(COMPONENTS_FILENAME);
    if !components_path.exists() {
        return (
            RepoFreshness {
                repo: slug.to_string(),
                status: FreshnessStatus::Missing,
                components_yaml: Some(components_path),
                generated_at: None,
                mtime_age_seconds: None,
            },
            None,
        );
    }
    let mtime_age_seconds = std::fs::metadata(&components_path)
        .ok()
        .and_then(|m| m.modified().ok())
        .and_then(|mtime| now.duration_since(mtime).ok())
        .map(|d| d.as_secs());
    match load_components(&components_path) {
        Ok(file) => {
            let report = RepoFreshness {
                repo: slug.to_string(),
                status: FreshnessStatus::Fresh,
                components_yaml: Some(components_path.clone()),
                generated_at: Some(file.generated_at.clone()),
                mtime_age_seconds,
            };
            let loaded = LoadedComponents {
                local_path: local_path.to_path_buf(),
                components_yaml: components_path,
                file,
            };
            (report, Some(loaded))
        }
        Err(_) => (
            RepoFreshness {
                repo: slug.to_string(),
                status: FreshnessStatus::Unparseable,
                components_yaml: Some(components_path),
                generated_at: None,
                mtime_age_seconds,
            },
            None,
        ),
    }
}

// ---------- Catalog: in-memory union of every fresh repo ----------

/// Per-repo loaded catalog: the parsed `components.yaml` plus the
/// disk paths it was read from. Only built for repos whose freshness
/// status is `Fresh`; non-fresh repos surface only via
/// [`Catalog::freshness`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RepoCatalog {
    /// Absolute path to the repo's working tree (mirror of the
    /// `local_path` from the registry).
    pub local_path: PathBuf,
    /// Absolute path to the parsed `<local_path>/.atlas/components.yaml`.
    pub components_yaml: PathBuf,
    /// Parsed file contents.
    pub file: ComponentsFile,
}

/// In-memory union of every fresh repo's `.atlas/components.yaml`,
/// keyed by repo slug. The freshness report is retained alongside so
/// callers can decide what to do with non-fresh repos (e.g. report,
/// surface a warning, or hard-fail in pre-flight).
///
/// Iteration order over `repos` matches the registry's insertion order
/// (via `IndexMap`), so downstream verbs render deterministically.
#[derive(Debug, Clone)]
pub struct Catalog {
    pub repos: IndexMap<String, RepoCatalog>,
    pub freshness: Vec<RepoFreshness>,
}

impl Catalog {
    /// Walk every repo in `registry`, retaining a parsed entry for each
    /// fresh one. Repos with status `NoLocalPath`, `Missing`, or
    /// `Unparseable` are skipped from `repos` but still recorded in
    /// `freshness`. `now` is parameterised so tests can pin mtime ages.
    pub fn load(registry: &ReposRegistry, now: SystemTime) -> Catalog {
        let entries = compute_freshness_full(registry, now);
        let mut repos = IndexMap::with_capacity(entries.len());
        let mut freshness = Vec::with_capacity(entries.len());
        for (report, loaded) in entries {
            if let Some(loaded) = loaded {
                repos.insert(
                    report.repo.clone(),
                    RepoCatalog {
                        local_path: loaded.local_path,
                        components_yaml: loaded.components_yaml,
                        file: loaded.file,
                    },
                );
            }
            freshness.push(report);
        }
        Catalog { repos, freshness }
    }

    /// Iterate `(repo_slug, component)` pairs across every fresh repo,
    /// in registry order then per-repo source order. `deleted`
    /// components are filtered out — they record a tombstone for
    /// rename-matching but should not appear in user-facing listings.
    pub fn iter_components(&self) -> impl Iterator<Item = (&str, &ComponentEntry)> {
        self.repos.iter().flat_map(|(slug, rc)| {
            rc.file
                .components
                .iter()
                .filter(|c| !c.deleted)
                .map(move |c| (slug.as_str(), c))
        })
    }
}

// ---------- list-components / summary verbs ----------

/// `atlas list-components [--repo R] [--kind K]` — print one line per
/// component as `<repo_slug>/<component_id>  <kind>`. Exits 0 with no
/// output when the catalog is empty (e.g. no registered repos, or every
/// repo non-fresh).
///
/// `--repo` errors when the slug is not a fresh repo so the user does
/// not silently get an empty listing for a typo'd slug.
pub fn run_list_components(
    context_root: &Path,
    repo_filter: Option<&str>,
    kind_filter: Option<&str>,
) -> Result<()> {
    let registry = repos::load_or_empty(context_root)?;
    let catalog = Catalog::load(&registry, SystemTime::now());
    enforce_repo_filter_known(&catalog, repo_filter)?;
    for (slug, comp) in catalog.iter_components() {
        if !matches_filters(slug, comp, repo_filter, kind_filter) {
            continue;
        }
        println!("{slug}/{id}  {kind}", id = comp.id, kind = comp.kind);
    }
    Ok(())
}

/// `atlas summary [--repo R]` — per-repo component counts grouped by
/// kind. Output:
///
/// ```text
/// <repo_slug>  (<total> total)
///   <count>  <kind>
///   ...
/// ```
///
/// Repos with no components (or filtered out) are omitted. `--repo`
/// errors when the slug is not a fresh repo.
pub fn run_summary(context_root: &Path, repo_filter: Option<&str>) -> Result<()> {
    let registry = repos::load_or_empty(context_root)?;
    let catalog = Catalog::load(&registry, SystemTime::now());
    enforce_repo_filter_known(&catalog, repo_filter)?;
    let by_repo = aggregate_summary(&catalog, repo_filter);
    for (slug, kinds) in &by_repo {
        let total: usize = kinds.values().sum();
        println!("{slug}  ({total} total)");
        for (kind, count) in kinds {
            println!("  {count:>4}  {kind}");
        }
    }
    Ok(())
}

/// Pure aggregation: group component counts by repo, then by kind.
/// `BTreeMap` for kinds gives deterministic alphabetical ordering;
/// `IndexMap` for repos preserves the registry's insertion order so
/// rendered output is stable.
fn aggregate_summary(
    catalog: &Catalog,
    repo_filter: Option<&str>,
) -> IndexMap<String, BTreeMap<String, usize>> {
    let mut by_repo: IndexMap<String, BTreeMap<String, usize>> = IndexMap::new();
    for (slug, comp) in catalog.iter_components() {
        if !matches_filters(slug, comp, repo_filter, None) {
            continue;
        }
        *by_repo
            .entry(slug.to_string())
            .or_default()
            .entry(comp.kind.clone())
            .or_insert(0) += 1;
    }
    by_repo
}

fn matches_filters(
    slug: &str,
    comp: &ComponentEntry,
    repo_filter: Option<&str>,
    kind_filter: Option<&str>,
) -> bool {
    if let Some(r) = repo_filter {
        if slug != r {
            return false;
        }
    }
    if let Some(k) = kind_filter {
        if comp.kind != k {
            return false;
        }
    }
    true
}

/// Reject `--repo <slug>` early if the slug is not a fresh repo. A
/// silent empty listing for a typo'd slug is worse than an upfront
/// error pointing at the available set.
fn enforce_repo_filter_known(catalog: &Catalog, repo_filter: Option<&str>) -> Result<()> {
    let Some(slug) = repo_filter else {
        return Ok(());
    };
    if catalog.repos.contains_key(slug) {
        return Ok(());
    }
    let available: Vec<&str> = catalog.repos.keys().map(String::as_str).collect();
    if available.is_empty() {
        bail!("--repo {slug:?}: no fresh repos in catalog (registry empty or all repos lack a fresh `.atlas/components.yaml`)");
    }
    bail!(
        "--repo {slug:?}: not a fresh repo; fresh repos: [{}]",
        available.join(", ")
    );
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

    // ---------- Catalog / list-components / summary tests ----------

    /// Write a `components.yaml` containing the given `(id, kind)` pairs
    /// at `<repo_dir>/.atlas/components.yaml`. The repo dir must exist.
    fn write_components_yaml(repo_dir: &Path, components: &[(&str, &str)]) {
        let comp_path = repo_dir.join(ATLAS_DIR).join(COMPONENTS_FILENAME);
        std::fs::create_dir_all(comp_path.parent().unwrap()).unwrap();
        let mut yaml = String::from(
            "schema_version: 1\n\
             root: /tmp\n\
             generated_at: 2026-04-24T00:00:00Z\n\
             cache_fingerprints:\n  ontology_sha: ''\n  model_id: ''\n  backend_version: ''\n\
             components:\n",
        );
        for (id, kind) in components {
            yaml.push_str(&format!(
                "  - id: {id}\n    kind: {kind}\n    evidence_grade: strong\n    rationale: test\n"
            ));
        }
        std::fs::write(&comp_path, yaml).unwrap();
    }

    /// Build a registry + on-disk repo layout, returning the registry
    /// rooted at `tmp` for any tests that want to add more repos.
    fn registry_with_one_fresh_repo(
        tmp: &TempDir,
        slug: &str,
        components: &[(&str, &str)],
    ) -> ReposRegistry {
        let repo_dir = tmp.path().join(slug);
        std::fs::create_dir_all(&repo_dir).unwrap();
        write_components_yaml(&repo_dir, components);
        let mut reg = empty_registry();
        repos::try_add(&mut reg, slug, "u", Some(&repo_dir)).unwrap();
        reg
    }

    #[test]
    fn catalog_load_empty_registry_yields_empty_catalog() {
        let catalog = Catalog::load(&empty_registry(), SystemTime::now());
        assert!(catalog.repos.is_empty());
        assert!(catalog.freshness.is_empty());
    }

    #[test]
    fn catalog_load_skips_non_fresh_repos_in_repos_map() {
        // Three repos: one fresh, one missing (.atlas dir absent),
        // one with no local_path.
        let tmp = TempDir::new().unwrap();
        let mut reg = registry_with_one_fresh_repo(&tmp, "atlas", &[("a", "k")]);
        let missing_dir = tmp.path().join("missing-repo");
        std::fs::create_dir_all(&missing_dir).unwrap();
        repos::try_add(&mut reg, "missing", "u", Some(&missing_dir)).unwrap();
        repos::try_add(&mut reg, "no-path", "u", None).unwrap();

        let catalog = Catalog::load(&reg, SystemTime::now());
        assert_eq!(catalog.freshness.len(), 3, "every repo present in freshness");
        assert_eq!(catalog.repos.len(), 1, "only the fresh repo lands in repos map");
        assert!(catalog.repos.contains_key("atlas"));
        assert!(!catalog.repos.contains_key("missing"));
        assert!(!catalog.repos.contains_key("no-path"));
    }

    #[test]
    fn catalog_load_preserves_registry_order_in_freshness() {
        let tmp = TempDir::new().unwrap();
        let mut reg = empty_registry();
        repos::try_add(&mut reg, "zeta", "u", None).unwrap();
        repos::try_add(&mut reg, "alpha", "u", None).unwrap();
        let mu_dir = tmp.path().join("mu");
        std::fs::create_dir_all(&mu_dir).unwrap();
        write_components_yaml(&mu_dir, &[("c1", "library")]);
        repos::try_add(&mut reg, "mu", "u", Some(&mu_dir)).unwrap();

        let catalog = Catalog::load(&reg, SystemTime::now());
        let order: Vec<&str> = catalog.freshness.iter().map(|r| r.repo.as_str()).collect();
        assert_eq!(order, vec!["zeta", "alpha", "mu"]);
    }

    #[test]
    fn catalog_iter_components_yields_all_components_in_registry_order() {
        let tmp = TempDir::new().unwrap();
        let mut reg = registry_with_one_fresh_repo(
            &tmp,
            "atlas",
            &[("a-1", "library"), ("a-2", "binary")],
        );
        let beta_dir = tmp.path().join("beta-repo");
        std::fs::create_dir_all(&beta_dir).unwrap();
        write_components_yaml(&beta_dir, &[("b-1", "library")]);
        repos::try_add(&mut reg, "beta", "u", Some(&beta_dir)).unwrap();

        let catalog = Catalog::load(&reg, SystemTime::now());
        let pairs: Vec<(String, String)> = catalog
            .iter_components()
            .map(|(slug, c)| (slug.to_string(), c.id.clone()))
            .collect();
        assert_eq!(
            pairs,
            vec![
                ("atlas".to_string(), "a-1".to_string()),
                ("atlas".to_string(), "a-2".to_string()),
                ("beta".to_string(), "b-1".to_string()),
            ]
        );
    }

    #[test]
    fn catalog_iter_components_skips_deleted_components() {
        let tmp = TempDir::new().unwrap();
        let repo_dir = tmp.path().join("atlas");
        std::fs::create_dir_all(&repo_dir).unwrap();
        // Hand-write so we can include `deleted: true`.
        let yaml = "schema_version: 1\n\
                    root: /tmp\n\
                    generated_at: 2026-04-24T00:00:00Z\n\
                    cache_fingerprints:\n  ontology_sha: ''\n  model_id: ''\n  backend_version: ''\n\
                    components:\n\
                    \x20\x20- id: live\n    kind: library\n    evidence_grade: strong\n    rationale: r\n\
                    \x20\x20- id: tombstone\n    kind: library\n    evidence_grade: strong\n    rationale: r\n    deleted: true\n";
        std::fs::create_dir_all(repo_dir.join(ATLAS_DIR)).unwrap();
        std::fs::write(repo_dir.join(ATLAS_DIR).join(COMPONENTS_FILENAME), yaml).unwrap();
        let mut reg = empty_registry();
        repos::try_add(&mut reg, "atlas", "u", Some(&repo_dir)).unwrap();

        let catalog = Catalog::load(&reg, SystemTime::now());
        let ids: Vec<&str> = catalog.iter_components().map(|(_, c)| c.id.as_str()).collect();
        assert_eq!(ids, vec!["live"], "deleted components must not surface");
    }

    #[test]
    fn matches_filters_repo_only_returns_only_that_repo() {
        let tmp = TempDir::new().unwrap();
        let mut reg = registry_with_one_fresh_repo(&tmp, "atlas", &[("a-1", "library")]);
        let beta_dir = tmp.path().join("beta-repo");
        std::fs::create_dir_all(&beta_dir).unwrap();
        write_components_yaml(&beta_dir, &[("b-1", "library")]);
        repos::try_add(&mut reg, "beta", "u", Some(&beta_dir)).unwrap();

        let catalog = Catalog::load(&reg, SystemTime::now());
        let filtered: Vec<&str> = catalog
            .iter_components()
            .filter(|(slug, c)| matches_filters(slug, c, Some("beta"), None))
            .map(|(_, c)| c.id.as_str())
            .collect();
        assert_eq!(filtered, vec!["b-1"]);
    }

    #[test]
    fn matches_filters_kind_only_filters_across_repos() {
        let tmp = TempDir::new().unwrap();
        let reg = registry_with_one_fresh_repo(
            &tmp,
            "atlas",
            &[("a-1", "library"), ("a-2", "binary"), ("a-3", "library")],
        );
        let catalog = Catalog::load(&reg, SystemTime::now());
        let libs: Vec<&str> = catalog
            .iter_components()
            .filter(|(slug, c)| matches_filters(slug, c, None, Some("library")))
            .map(|(_, c)| c.id.as_str())
            .collect();
        assert_eq!(libs, vec!["a-1", "a-3"]);
    }

    #[test]
    fn matches_filters_repo_and_kind_combine() {
        let tmp = TempDir::new().unwrap();
        let mut reg = registry_with_one_fresh_repo(
            &tmp,
            "atlas",
            &[("a-1", "library"), ("a-2", "binary")],
        );
        let beta_dir = tmp.path().join("beta-repo");
        std::fs::create_dir_all(&beta_dir).unwrap();
        write_components_yaml(&beta_dir, &[("b-1", "binary")]);
        repos::try_add(&mut reg, "beta", "u", Some(&beta_dir)).unwrap();

        let catalog = Catalog::load(&reg, SystemTime::now());
        let hits: Vec<String> = catalog
            .iter_components()
            .filter(|(slug, c)| matches_filters(slug, c, Some("atlas"), Some("binary")))
            .map(|(slug, c)| format!("{slug}/{}", c.id))
            .collect();
        assert_eq!(hits, vec!["atlas/a-2"]);
    }

    #[test]
    fn enforce_repo_filter_known_accepts_fresh_repo() {
        let tmp = TempDir::new().unwrap();
        let reg = registry_with_one_fresh_repo(&tmp, "atlas", &[("a-1", "library")]);
        let catalog = Catalog::load(&reg, SystemTime::now());
        enforce_repo_filter_known(&catalog, Some("atlas")).unwrap();
        enforce_repo_filter_known(&catalog, None).unwrap();
    }

    #[test]
    fn enforce_repo_filter_known_rejects_unknown_repo_with_available_list() {
        let tmp = TempDir::new().unwrap();
        let reg = registry_with_one_fresh_repo(&tmp, "atlas", &[("a-1", "library")]);
        let catalog = Catalog::load(&reg, SystemTime::now());
        let err = enforce_repo_filter_known(&catalog, Some("ravel-lite")).unwrap_err();
        let msg = format!("{err:#}");
        assert!(msg.contains("ravel-lite"), "names the bad slug; got: {msg}");
        assert!(msg.contains("atlas"), "names the available slug; got: {msg}");
    }

    #[test]
    fn enforce_repo_filter_known_reports_empty_catalog_clearly() {
        let catalog = Catalog::load(&empty_registry(), SystemTime::now());
        let err = enforce_repo_filter_known(&catalog, Some("anything")).unwrap_err();
        let msg = format!("{err:#}");
        assert!(
            msg.contains("no fresh repos in catalog"),
            "empty-catalog message must point at registry state; got: {msg}"
        );
    }

    #[test]
    fn aggregate_summary_groups_by_repo_then_kind() {
        let tmp = TempDir::new().unwrap();
        let mut reg = registry_with_one_fresh_repo(
            &tmp,
            "atlas",
            &[("a-1", "library"), ("a-2", "library"), ("a-3", "binary")],
        );
        let beta_dir = tmp.path().join("beta-repo");
        std::fs::create_dir_all(&beta_dir).unwrap();
        write_components_yaml(&beta_dir, &[("b-1", "library")]);
        repos::try_add(&mut reg, "beta", "u", Some(&beta_dir)).unwrap();

        let catalog = Catalog::load(&reg, SystemTime::now());
        let agg = aggregate_summary(&catalog, None);
        let repo_order: Vec<&str> = agg.keys().map(String::as_str).collect();
        assert_eq!(repo_order, vec!["atlas", "beta"]);
        assert_eq!(agg["atlas"].get("library").copied(), Some(2));
        assert_eq!(agg["atlas"].get("binary").copied(), Some(1));
        assert_eq!(agg["beta"].get("library").copied(), Some(1));
    }

    #[test]
    fn aggregate_summary_repo_filter_drops_other_repos() {
        let tmp = TempDir::new().unwrap();
        let mut reg = registry_with_one_fresh_repo(&tmp, "atlas", &[("a-1", "library")]);
        let beta_dir = tmp.path().join("beta-repo");
        std::fs::create_dir_all(&beta_dir).unwrap();
        write_components_yaml(&beta_dir, &[("b-1", "library")]);
        repos::try_add(&mut reg, "beta", "u", Some(&beta_dir)).unwrap();

        let catalog = Catalog::load(&reg, SystemTime::now());
        let agg = aggregate_summary(&catalog, Some("beta"));
        assert_eq!(agg.len(), 1);
        assert!(agg.contains_key("beta"));
        assert!(!agg.contains_key("atlas"));
    }

    #[test]
    fn aggregate_summary_empty_when_no_components() {
        let tmp = TempDir::new().unwrap();
        let reg = registry_with_one_fresh_repo(&tmp, "atlas", &[]);
        let catalog = Catalog::load(&reg, SystemTime::now());
        let agg = aggregate_summary(&catalog, None);
        assert!(agg.is_empty());
    }

    #[test]
    fn run_list_components_with_empty_registry_succeeds_silently() {
        // Smoke: the verb walks the I/O path without panicking when
        // there's nothing to list. stdout output is tested through the
        // catalog/iter/aggregate helpers above.
        let tmp = TempDir::new().unwrap();
        run_list_components(tmp.path(), None, None).unwrap();
    }

    #[test]
    fn run_summary_with_empty_registry_succeeds_silently() {
        let tmp = TempDir::new().unwrap();
        run_summary(tmp.path(), None).unwrap();
    }
}
