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
//! - `describe` — single-component detail by `<repo>/<id>` or bare
//!   `<id>` (unambiguous), including computed child list.
//! - `memory` — per-component `.atlas/memory.yaml`, optionally
//!   filtered by `--search` against the entry's claim, attribution,
//!   and any string-bearing justification field.
//!
//! The remaining edge-graph verbs (`edges`, `neighbors`, `path`,
//! `scc`, `roots`) need the cross-repo `related-components.yaml`
//! union loader and are tracked as follow-up backlog tasks.
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
use knowledge_graph::Justification;

use crate::repos::{self, RepoEntry, ReposRegistry};
use crate::state::memory::{MemoryEntry, MemoryFile, MEMORY_SCHEMA_VERSION};

const ATLAS_DIR: &str = ".atlas";
const COMPONENTS_FILENAME: &str = "components.yaml";
const MEMORY_FILENAME: &str = "memory.yaml";

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

// ---------- describe / memory verbs ----------

/// One component located inside the loaded catalog. Borrowed from the
/// catalog so callers can render its fields without further lookups.
#[derive(Debug)]
struct ResolvedComponent<'a> {
    repo_slug: &'a str,
    repo: &'a RepoCatalog,
    component: &'a ComponentEntry,
}

/// Resolve `<ref>` to a single component in the catalog. Accepts the
/// qualified form `<repo_slug>/<component_id>` or a bare
/// `<component_id>`; the bare form errors if the id is not unique
/// across fresh repos so the user is forced to disambiguate.
fn resolve_ref<'a>(catalog: &'a Catalog, ref_str: &str) -> Result<ResolvedComponent<'a>> {
    if let Some((slug, id)) = ref_str.split_once('/') {
        return resolve_qualified(catalog, slug, id);
    }
    resolve_bare(catalog, ref_str)
}

fn resolve_qualified<'a>(
    catalog: &'a Catalog,
    slug: &str,
    id: &str,
) -> Result<ResolvedComponent<'a>> {
    let (slug_owned, repo) = catalog.repos.get_key_value(slug).ok_or_else(|| {
        let available: Vec<&str> = catalog.repos.keys().map(String::as_str).collect();
        if available.is_empty() {
            anyhow::anyhow!(
                "ref {slug:?}/{id:?}: no fresh repos in catalog (registry empty or all repos lack a fresh `.atlas/components.yaml`)"
            )
        } else {
            anyhow::anyhow!(
                "ref {slug:?}/{id:?}: unknown repo slug; fresh repos: [{}]",
                available.join(", ")
            )
        }
    })?;
    let component = repo
        .file
        .components
        .iter()
        .find(|c| !c.deleted && c.id == id)
        .ok_or_else(|| {
            anyhow::anyhow!(
                "ref {slug:?}/{id:?}: no component with that id in repo {slug:?}"
            )
        })?;
    Ok(ResolvedComponent {
        repo_slug: slug_owned.as_str(),
        repo,
        component,
    })
}

fn resolve_bare<'a>(catalog: &'a Catalog, id: &str) -> Result<ResolvedComponent<'a>> {
    let mut hits: Vec<ResolvedComponent<'a>> = Vec::new();
    for (slug, repo) in &catalog.repos {
        for component in &repo.file.components {
            if !component.deleted && component.id == id {
                hits.push(ResolvedComponent {
                    repo_slug: slug.as_str(),
                    repo,
                    component,
                });
            }
        }
    }
    match hits.len() {
        0 => bail!("ref {id:?}: no component with that id in any fresh repo"),
        1 => Ok(hits.into_iter().next().expect("len == 1")),
        _ => {
            let qualified: Vec<String> = hits
                .iter()
                .map(|h| format!("{}/{}", h.repo_slug, h.component.id))
                .collect();
            bail!(
                "ref {id:?}: ambiguous; matches multiple repos: [{}]",
                qualified.join(", ")
            )
        }
    }
}

/// IDs (qualified as `<repo_slug>/<child_id>`) of every non-deleted
/// component in the same repo whose `parent` matches `parent_id`.
fn compute_children(repo_slug: &str, repo: &RepoCatalog, parent_id: &str) -> Vec<String> {
    repo.file
        .components
        .iter()
        .filter(|c| !c.deleted && c.parent.as_deref() == Some(parent_id))
        .map(|c| format!("{repo_slug}/{}", c.id))
        .collect()
}

/// Wrapper for the `describe` YAML output. Exposes the canonical
/// `<repo_slug>/<component_id>` ref alongside the unmodified
/// `ComponentEntry` so downstream tools see the same field names that
/// `components.yaml` already publishes.
#[derive(Debug, Serialize)]
struct DescribeReport<'a> {
    #[serde(rename = "ref")]
    reference: String,
    repo: &'a str,
    component: &'a ComponentEntry,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    children: Vec<String>,
}

/// `atlas describe <ref>` — emit one component's full record as YAML,
/// plus its children (computed by scanning siblings whose `parent`
/// matches). Errors when the ref does not resolve to exactly one
/// component.
pub fn run_describe(context_root: &Path, ref_str: &str) -> Result<()> {
    let registry = repos::load_or_empty(context_root)?;
    let catalog = Catalog::load(&registry, SystemTime::now());
    let resolved = resolve_ref(&catalog, ref_str)?;
    let children = compute_children(resolved.repo_slug, resolved.repo, &resolved.component.id);
    let report = DescribeReport {
        reference: format!("{}/{}", resolved.repo_slug, resolved.component.id),
        repo: resolved.repo_slug,
        component: resolved.component,
        children,
    };
    let yaml = serde_yaml::to_string(&report)
        .context("Failed to serialise atlas describe report to YAML")?;
    print!("{yaml}");
    Ok(())
}

/// On-disk path to a component's `.atlas/memory.yaml`. Returns `None`
/// when the component declares no path segments (no working directory
/// to anchor a memory file). When multiple segments exist, the first
/// is used as the canonical anchor.
fn component_memory_path(repo: &RepoCatalog, component: &ComponentEntry) -> Option<PathBuf> {
    let segment = component.path_segments.first()?;
    Some(
        repo.local_path
            .join(&segment.path)
            .join(ATLAS_DIR)
            .join(MEMORY_FILENAME),
    )
}

/// Read a component's `.atlas/memory.yaml`. A missing file is the
/// expected first-time state for a component, so it is reported as an
/// empty `MemoryFile` rather than an error. Parse failures and
/// schema-version mismatches still error so silent corruption cannot
/// hide.
fn read_component_memory(path: &Path) -> Result<MemoryFile> {
    if !path.exists() {
        return Ok(MemoryFile::default());
    }
    let text = std::fs::read_to_string(path)
        .with_context(|| format!("Failed to read {}", path.display()))?;
    let parsed: MemoryFile = serde_yaml::from_str(&text).with_context(|| {
        format!("Failed to parse {} as component memory.yaml schema", path.display())
    })?;
    if parsed.schema_version != MEMORY_SCHEMA_VERSION {
        bail!(
            "{} declares schema_version {}, expected {}.",
            path.display(),
            parsed.schema_version,
            MEMORY_SCHEMA_VERSION
        );
    }
    Ok(parsed)
}

/// `atlas memory <ref> [--search <term>]` — emit a component's memory
/// file as YAML. Missing files are reported as an empty `MemoryFile`
/// (graceful first-time state). `--search` filters entries whose
/// claim, attribution, or any justification's string fields contain
/// the term (case-insensitive substring).
pub fn run_memory(context_root: &Path, ref_str: &str, search: Option<&str>) -> Result<()> {
    let registry = repos::load_or_empty(context_root)?;
    let catalog = Catalog::load(&registry, SystemTime::now());
    let resolved = resolve_ref(&catalog, ref_str)?;
    let memory = match component_memory_path(resolved.repo, resolved.component) {
        Some(path) => read_component_memory(&path)?,
        None => MemoryFile::default(),
    };
    let output = match search {
        Some(needle) => {
            let needle_lc = needle.to_lowercase();
            MemoryFile {
                schema_version: memory.schema_version,
                items: memory
                    .items
                    .into_iter()
                    .filter(|e| entry_matches(e, &needle_lc))
                    .collect(),
            }
        }
        None => memory,
    };
    let yaml = serde_yaml::to_string(&output)
        .context("Failed to serialise component memory to YAML")?;
    print!("{yaml}");
    Ok(())
}

/// True when any of the entry's user-visible string fields contains
/// `needle_lc` (already lowercased by the caller). The search covers
/// the claim, attribution, and every justification's string-bearing
/// fields so users can grep on either the assertion text or its
/// supporting evidence.
fn entry_matches(entry: &MemoryEntry, needle_lc: &str) -> bool {
    if entry.item.claim.to_lowercase().contains(needle_lc) {
        return true;
    }
    if let Some(attr) = entry.attribution.as_deref() {
        if attr.to_lowercase().contains(needle_lc) {
            return true;
        }
    }
    entry
        .item
        .justifications
        .iter()
        .any(|j| justification_matches(j, needle_lc))
}

fn justification_matches(j: &Justification, needle_lc: &str) -> bool {
    let needle_in = |s: &str| s.to_lowercase().contains(needle_lc);
    match j {
        Justification::CodeAnchor {
            component,
            path,
            lines,
            sha_at_assertion,
        } => {
            needle_in(component)
                || needle_in(path)
                || needle_in(sha_at_assertion)
                || lines.as_deref().map(needle_in).unwrap_or(false)
        }
        Justification::Rationale { text } => needle_in(text),
        Justification::ServesIntent { intent_id } => needle_in(intent_id),
        Justification::Defeats { item_id } => needle_in(item_id),
        Justification::Supersedes { item_id } => needle_in(item_id),
        Justification::External { uri } => needle_in(uri),
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

    // ---------- describe / memory tests ----------

    /// Build a registry-and-disk layout for two fresh repos `atlas` and
    /// `beta`, returning the registry plus their working dirs so tests
    /// can write per-component memory files.
    fn registry_with_two_fresh_repos(
        tmp: &TempDir,
    ) -> (ReposRegistry, PathBuf, PathBuf) {
        let atlas_dir = tmp.path().join("atlas");
        let beta_dir = tmp.path().join("beta");
        std::fs::create_dir_all(&atlas_dir).unwrap();
        std::fs::create_dir_all(&beta_dir).unwrap();
        write_components_yaml(&atlas_dir, &[("a-1", "library"), ("a-2", "binary")]);
        write_components_yaml(&beta_dir, &[("b-1", "library")]);
        let mut reg = empty_registry();
        repos::try_add(&mut reg, "atlas", "u", Some(&atlas_dir)).unwrap();
        repos::try_add(&mut reg, "beta", "u", Some(&beta_dir)).unwrap();
        (reg, atlas_dir, beta_dir)
    }

    /// Write a `components.yaml` that includes parent + path_segments
    /// fields so describe/memory tests can exercise children-resolution
    /// and the on-disk memory path.
    fn write_components_yaml_rich(
        repo_dir: &Path,
        components: &[(&str, &str, Option<&str>, Option<&str>)],
    ) {
        let comp_path = repo_dir.join(ATLAS_DIR).join(COMPONENTS_FILENAME);
        std::fs::create_dir_all(comp_path.parent().unwrap()).unwrap();
        let mut yaml = String::from(
            "schema_version: 1\n\
             root: /tmp\n\
             generated_at: 2026-04-24T00:00:00Z\n\
             cache_fingerprints:\n  ontology_sha: ''\n  model_id: ''\n  backend_version: ''\n\
             components:\n",
        );
        for (id, kind, parent, segment_path) in components {
            yaml.push_str(&format!(
                "  - id: {id}\n    kind: {kind}\n    evidence_grade: strong\n    rationale: r\n"
            ));
            if let Some(p) = parent {
                yaml.push_str(&format!("    parent: {p}\n"));
            }
            if let Some(seg) = segment_path {
                yaml.push_str(&format!(
                    "    path_segments:\n      - path: {seg}\n        content_sha: deadbeef\n"
                ));
            }
        }
        std::fs::write(&comp_path, yaml).unwrap();
    }

    fn write_component_memory_yaml(repo_dir: &Path, segment: &str, body: &str) -> PathBuf {
        let dir = repo_dir.join(segment).join(ATLAS_DIR);
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join(MEMORY_FILENAME);
        std::fs::write(&path, body).unwrap();
        path
    }

    fn sample_memory_yaml(claim: &str, rationale: &str) -> String {
        format!(
            "schema_version: 1\n\
             items:\n\
             - id: m-1\n  \
               kind: memory-entry\n  \
               claim: {claim}\n  \
               justifications:\n    \
                 - kind: rationale\n      \
                   text: {rationale}\n  \
               status: active\n  \
               authored_at: 2026-04-29T00:00:00Z\n  \
               authored_in: test\n"
        )
    }

    #[test]
    fn resolve_qualified_ref_returns_the_named_component() {
        let tmp = TempDir::new().unwrap();
        let (reg, _, _) = registry_with_two_fresh_repos(&tmp);
        let catalog = Catalog::load(&reg, SystemTime::now());
        let resolved = resolve_ref(&catalog, "atlas/a-1").unwrap();
        assert_eq!(resolved.repo_slug, "atlas");
        assert_eq!(resolved.component.id, "a-1");
    }

    #[test]
    fn resolve_qualified_ref_unknown_repo_lists_available() {
        let tmp = TempDir::new().unwrap();
        let (reg, _, _) = registry_with_two_fresh_repos(&tmp);
        let catalog = Catalog::load(&reg, SystemTime::now());
        let err = resolve_ref(&catalog, "ghost/a-1").unwrap_err();
        let msg = format!("{err:#}");
        assert!(msg.contains("ghost"), "names the bad slug: {msg}");
        assert!(msg.contains("atlas"), "lists available: {msg}");
        assert!(msg.contains("beta"), "lists available: {msg}");
    }

    #[test]
    fn resolve_qualified_ref_unknown_id_names_repo() {
        let tmp = TempDir::new().unwrap();
        let (reg, _, _) = registry_with_two_fresh_repos(&tmp);
        let catalog = Catalog::load(&reg, SystemTime::now());
        let err = resolve_ref(&catalog, "atlas/no-such").unwrap_err();
        let msg = format!("{err:#}");
        assert!(msg.contains("no-such"), "names the bad id: {msg}");
        assert!(msg.contains("atlas"), "names the searched repo: {msg}");
    }

    #[test]
    fn resolve_qualified_ref_against_empty_catalog_explains_state() {
        let catalog = Catalog::load(&empty_registry(), SystemTime::now());
        let err = resolve_ref(&catalog, "atlas/a-1").unwrap_err();
        let msg = format!("{err:#}");
        assert!(
            msg.contains("no fresh repos in catalog"),
            "empty-catalog message must point at registry state: {msg}"
        );
    }

    #[test]
    fn resolve_bare_ref_unique_match_succeeds() {
        let tmp = TempDir::new().unwrap();
        let (reg, _, _) = registry_with_two_fresh_repos(&tmp);
        let catalog = Catalog::load(&reg, SystemTime::now());
        let resolved = resolve_ref(&catalog, "b-1").unwrap();
        assert_eq!(resolved.repo_slug, "beta");
        assert_eq!(resolved.component.id, "b-1");
    }

    #[test]
    fn resolve_bare_ref_not_found_errors() {
        let tmp = TempDir::new().unwrap();
        let (reg, _, _) = registry_with_two_fresh_repos(&tmp);
        let catalog = Catalog::load(&reg, SystemTime::now());
        let err = resolve_ref(&catalog, "ghost").unwrap_err();
        let msg = format!("{err:#}");
        assert!(msg.contains("ghost"), "names the missing id: {msg}");
    }

    #[test]
    fn resolve_bare_ref_ambiguous_lists_qualified_options() {
        let tmp = TempDir::new().unwrap();
        let atlas_dir = tmp.path().join("atlas");
        let beta_dir = tmp.path().join("beta");
        std::fs::create_dir_all(&atlas_dir).unwrap();
        std::fs::create_dir_all(&beta_dir).unwrap();
        write_components_yaml(&atlas_dir, &[("shared", "library")]);
        write_components_yaml(&beta_dir, &[("shared", "library")]);
        let mut reg = empty_registry();
        repos::try_add(&mut reg, "atlas", "u", Some(&atlas_dir)).unwrap();
        repos::try_add(&mut reg, "beta", "u", Some(&beta_dir)).unwrap();
        let catalog = Catalog::load(&reg, SystemTime::now());
        let err = resolve_ref(&catalog, "shared").unwrap_err();
        let msg = format!("{err:#}");
        assert!(msg.contains("ambiguous"), "msg must say ambiguous: {msg}");
        assert!(msg.contains("atlas/shared"), "lists qualified ref: {msg}");
        assert!(msg.contains("beta/shared"), "lists qualified ref: {msg}");
    }

    #[test]
    fn resolve_ref_skips_deleted_components() {
        // Two repos contain `dup`; one is tombstoned. The bare ref
        // resolves uniquely to the live one.
        let tmp = TempDir::new().unwrap();
        let live_dir = tmp.path().join("live");
        let dead_dir = tmp.path().join("dead");
        std::fs::create_dir_all(&live_dir).unwrap();
        std::fs::create_dir_all(&dead_dir.join(ATLAS_DIR)).unwrap();
        write_components_yaml(&live_dir, &[("dup", "library")]);
        let dead_yaml = "schema_version: 1\n\
                         root: /tmp\n\
                         generated_at: 2026-04-24T00:00:00Z\n\
                         cache_fingerprints:\n  ontology_sha: ''\n  model_id: ''\n  backend_version: ''\n\
                         components:\n  - id: dup\n    kind: library\n    evidence_grade: strong\n    rationale: r\n    deleted: true\n";
        std::fs::write(dead_dir.join(ATLAS_DIR).join(COMPONENTS_FILENAME), dead_yaml).unwrap();
        let mut reg = empty_registry();
        repos::try_add(&mut reg, "live", "u", Some(&live_dir)).unwrap();
        repos::try_add(&mut reg, "dead", "u", Some(&dead_dir)).unwrap();
        let catalog = Catalog::load(&reg, SystemTime::now());
        let resolved = resolve_ref(&catalog, "dup").unwrap();
        assert_eq!(resolved.repo_slug, "live");
    }

    #[test]
    fn compute_children_lists_only_direct_descendants() {
        let tmp = TempDir::new().unwrap();
        let repo_dir = tmp.path().join("atlas");
        std::fs::create_dir_all(&repo_dir).unwrap();
        write_components_yaml_rich(
            &repo_dir,
            &[
                ("root", "library", None, None),
                ("child-a", "library", Some("root"), None),
                ("child-b", "binary", Some("root"), None),
                ("grandchild", "library", Some("child-a"), None),
                ("orphan", "library", None, None),
            ],
        );
        let mut reg = empty_registry();
        repos::try_add(&mut reg, "atlas", "u", Some(&repo_dir)).unwrap();
        let catalog = Catalog::load(&reg, SystemTime::now());
        let repo = catalog.repos.get("atlas").unwrap();
        let children = compute_children("atlas", repo, "root");
        assert_eq!(children, vec!["atlas/child-a", "atlas/child-b"]);
    }

    #[test]
    fn run_describe_emits_yaml_with_ref_repo_component_and_children() {
        let tmp = TempDir::new().unwrap();
        let repo_dir = tmp.path().join("atlas");
        std::fs::create_dir_all(&repo_dir).unwrap();
        write_components_yaml_rich(
            &repo_dir,
            &[
                ("root", "library", None, Some("crates/root")),
                ("kid", "library", Some("root"), Some("crates/root/kid")),
            ],
        );
        let mut reg = empty_registry();
        repos::try_add(&mut reg, "atlas", "u", Some(&repo_dir)).unwrap();
        // Persist the registry so run_describe can reload it from disk.
        repos::save_atomic(tmp.path(), &reg).unwrap();

        let catalog = Catalog::load(&reg, SystemTime::now());
        let resolved = resolve_ref(&catalog, "atlas/root").unwrap();
        let report = DescribeReport {
            reference: format!("{}/{}", resolved.repo_slug, resolved.component.id),
            repo: resolved.repo_slug,
            component: resolved.component,
            children: compute_children(resolved.repo_slug, resolved.repo, &resolved.component.id),
        };
        let yaml = serde_yaml::to_string(&report).unwrap();
        assert!(yaml.contains("ref: atlas/root"), "yaml: {yaml}");
        assert!(yaml.contains("repo: atlas"), "yaml: {yaml}");
        assert!(yaml.contains("id: root"), "yaml: {yaml}");
        assert!(yaml.contains("kind: library"), "yaml: {yaml}");
        assert!(yaml.contains("- atlas/kid"), "children listed: {yaml}");
    }

    #[test]
    fn run_describe_top_level_call_succeeds_with_persisted_registry() {
        let tmp = TempDir::new().unwrap();
        let repo_dir = tmp.path().join("atlas");
        std::fs::create_dir_all(&repo_dir).unwrap();
        write_components_yaml(&repo_dir, &[("a-1", "library")]);
        let mut reg = empty_registry();
        repos::try_add(&mut reg, "atlas", "u", Some(&repo_dir)).unwrap();
        repos::save_atomic(tmp.path(), &reg).unwrap();
        // Smoke: I/O path completes without panicking.
        run_describe(tmp.path(), "atlas/a-1").unwrap();
    }

    #[test]
    fn component_memory_path_uses_first_segment_when_present() {
        let tmp = TempDir::new().unwrap();
        let repo_dir = tmp.path().join("atlas");
        std::fs::create_dir_all(&repo_dir).unwrap();
        write_components_yaml_rich(
            &repo_dir,
            &[("a-1", "library", None, Some("crates/foo"))],
        );
        let mut reg = empty_registry();
        repos::try_add(&mut reg, "atlas", "u", Some(&repo_dir)).unwrap();
        let catalog = Catalog::load(&reg, SystemTime::now());
        let resolved = resolve_ref(&catalog, "atlas/a-1").unwrap();
        let path = component_memory_path(resolved.repo, resolved.component).unwrap();
        let expected = repo_dir.join("crates/foo").join(ATLAS_DIR).join(MEMORY_FILENAME);
        assert_eq!(path, expected);
    }

    #[test]
    fn component_memory_path_is_none_when_no_segments() {
        let tmp = TempDir::new().unwrap();
        let repo_dir = tmp.path().join("atlas");
        std::fs::create_dir_all(&repo_dir).unwrap();
        write_components_yaml(&repo_dir, &[("a-1", "library")]);
        let mut reg = empty_registry();
        repos::try_add(&mut reg, "atlas", "u", Some(&repo_dir)).unwrap();
        let catalog = Catalog::load(&reg, SystemTime::now());
        let resolved = resolve_ref(&catalog, "atlas/a-1").unwrap();
        assert!(component_memory_path(resolved.repo, resolved.component).is_none());
    }

    #[test]
    fn read_component_memory_returns_empty_when_file_absent() {
        let tmp = TempDir::new().unwrap();
        let path = tmp.path().join("memory.yaml");
        let memory = read_component_memory(&path).unwrap();
        assert_eq!(memory.schema_version, MEMORY_SCHEMA_VERSION);
        assert!(memory.items.is_empty());
    }

    #[test]
    fn read_component_memory_parses_existing_file() {
        let tmp = TempDir::new().unwrap();
        let path = tmp.path().join("memory.yaml");
        std::fs::write(&path, sample_memory_yaml("hello", "supporting prose")).unwrap();
        let memory = read_component_memory(&path).unwrap();
        assert_eq!(memory.items.len(), 1);
        assert_eq!(memory.items[0].item.id, "m-1");
        assert_eq!(memory.items[0].item.claim, "hello");
    }

    #[test]
    fn read_component_memory_errors_on_schema_mismatch() {
        let tmp = TempDir::new().unwrap();
        let path = tmp.path().join("memory.yaml");
        std::fs::write(&path, "schema_version: 99\nitems: []\n").unwrap();
        let err = read_component_memory(&path).unwrap_err();
        let msg = format!("{err:#}");
        assert!(msg.contains("schema_version"), "msg: {msg}");
        assert!(msg.contains("99"), "msg: {msg}");
    }

    #[test]
    fn entry_matches_finds_substring_in_claim() {
        // entry_matches expects an already-lowercased needle (the
        // run_memory caller does the lowercasing). The case-folding
        // here is on the haystack side: an upper-case claim still
        // matches a lower-case needle.
        let yaml = sample_memory_yaml("Findable Claim Text", "rat");
        let memory: MemoryFile = serde_yaml::from_str(&yaml).unwrap();
        let entry = &memory.items[0];
        assert!(entry_matches(entry, "findable"));
        assert!(entry_matches(entry, "claim text"));
        assert!(!entry_matches(entry, "absent"));
    }

    #[test]
    fn entry_matches_finds_substring_in_rationale_text() {
        let yaml = sample_memory_yaml("c", "Detailed rationale prose here");
        let memory: MemoryFile = serde_yaml::from_str(&yaml).unwrap();
        let entry = &memory.items[0];
        assert!(entry_matches(entry, "rationale"));
    }

    #[test]
    fn entry_matches_finds_substring_in_attribution() {
        let mut memory: MemoryFile =
            serde_yaml::from_str(&sample_memory_yaml("c", "r")).unwrap();
        memory.items[0].attribution = Some("atlas/atlas-core".into());
        let entry = &memory.items[0];
        assert!(entry_matches(entry, "atlas-core"));
    }

    #[test]
    fn justification_matches_walks_each_kind() {
        let needle = "needle";
        let cases = vec![
            Justification::CodeAnchor {
                component: "atlas/needle-comp".into(),
                path: "src/x.rs".into(),
                lines: None,
                sha_at_assertion: "abc".into(),
            },
            Justification::CodeAnchor {
                component: "atlas/x".into(),
                path: "src/needle.rs".into(),
                lines: None,
                sha_at_assertion: "abc".into(),
            },
            Justification::CodeAnchor {
                component: "atlas/x".into(),
                path: "src/x.rs".into(),
                lines: Some("needle-10".into()),
                sha_at_assertion: "abc".into(),
            },
            Justification::Rationale {
                text: "the needle is here".into(),
            },
            Justification::ServesIntent {
                intent_id: "i-needle".into(),
            },
            Justification::Defeats {
                item_id: "m-needle".into(),
            },
            Justification::Supersedes {
                item_id: "m-needle".into(),
            },
            Justification::External {
                uri: "https://example.com/needle".into(),
            },
        ];
        for j in cases {
            assert!(justification_matches(&j, needle), "should match: {j:?}");
        }
    }

    #[test]
    fn justification_matches_misses_unrelated_text() {
        let j = Justification::Rationale {
            text: "haystack only".into(),
        };
        assert!(!justification_matches(&j, "needle"));
    }

    #[test]
    fn run_memory_returns_empty_when_segment_missing() {
        let tmp = TempDir::new().unwrap();
        let repo_dir = tmp.path().join("atlas");
        std::fs::create_dir_all(&repo_dir).unwrap();
        write_components_yaml(&repo_dir, &[("a-1", "library")]);
        let mut reg = empty_registry();
        repos::try_add(&mut reg, "atlas", "u", Some(&repo_dir)).unwrap();
        repos::save_atomic(tmp.path(), &reg).unwrap();
        // Smoke: empty memory path yields empty file output without
        // erroring (no path_segments → no on-disk anchor).
        run_memory(tmp.path(), "atlas/a-1", None).unwrap();
    }

    #[test]
    fn run_memory_returns_empty_when_file_absent() {
        let tmp = TempDir::new().unwrap();
        let repo_dir = tmp.path().join("atlas");
        std::fs::create_dir_all(&repo_dir).unwrap();
        write_components_yaml_rich(
            &repo_dir,
            &[("a-1", "library", None, Some("crates/a"))],
        );
        let mut reg = empty_registry();
        repos::try_add(&mut reg, "atlas", "u", Some(&repo_dir)).unwrap();
        repos::save_atomic(tmp.path(), &reg).unwrap();
        // path_segments[0] resolves but the .atlas/memory.yaml file
        // beneath it does not exist → graceful empty output.
        run_memory(tmp.path(), "atlas/a-1", None).unwrap();
    }

    #[test]
    fn run_memory_search_filters_to_matching_entries_only() {
        let tmp = TempDir::new().unwrap();
        let repo_dir = tmp.path().join("atlas");
        std::fs::create_dir_all(&repo_dir).unwrap();
        write_components_yaml_rich(
            &repo_dir,
            &[("a-1", "library", None, Some("crates/a"))],
        );
        let mut reg = empty_registry();
        repos::try_add(&mut reg, "atlas", "u", Some(&repo_dir)).unwrap();
        repos::save_atomic(tmp.path(), &reg).unwrap();
        // Two entries; only one mentions "needle".
        let body = "schema_version: 1\n\
                    items:\n\
                    - id: m-1\n  kind: memory-entry\n  claim: needle in the claim\n  status: active\n  authored_at: t\n  authored_in: test\n\
                    - id: m-2\n  kind: memory-entry\n  claim: unrelated\n  status: active\n  authored_at: t\n  authored_in: test\n";
        write_component_memory_yaml(&repo_dir, "crates/a", body);
        let context_root = tmp.path();
        let registry = repos::load_or_empty(context_root).unwrap();
        let catalog = Catalog::load(&registry, SystemTime::now());
        let resolved = resolve_ref(&catalog, "atlas/a-1").unwrap();
        let path = component_memory_path(resolved.repo, resolved.component).unwrap();
        let memory = read_component_memory(&path).unwrap();
        assert_eq!(memory.items.len(), 2);
        let needle_lc = "needle";
        let filtered: Vec<&MemoryEntry> = memory
            .items
            .iter()
            .filter(|e| entry_matches(e, needle_lc))
            .collect();
        assert_eq!(filtered.len(), 1);
        assert_eq!(filtered[0].item.id, "m-1");
    }
}
