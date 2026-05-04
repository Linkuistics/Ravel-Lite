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
//! - `edges` — direct edges touching a component, optionally filtered
//!   by direction (`--in` / `--out` / `--both`).
//! - `neighbors` — bounded-depth BFS from a component over the
//!   undirected edge graph, emitting one line per reached component
//!   with hop count.
//! - `roots` — components with no incoming directed edges; symmetric
//!   edges do not disqualify (peer relationships do not establish
//!   hierarchy).
//!
//! The remaining graph-algorithm verbs (`path`, `scc`) build on the
//! `EdgeGraph` loaded here and are tracked as a follow-up backlog
//! task.
//!
//! The shared in-memory representation is [`Catalog`]: the union of
//! every fresh repo's `ComponentsFile`, keyed by repo slug. Subsequent
//! verbs build on top of this loader instead of re-parsing
//! `components.yaml` per invocation.

use std::collections::{BTreeMap, BTreeSet, HashSet, VecDeque};
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use anyhow::{Context, Result};

use crate::bail_with;
use crate::cli::error_context::ResultExt;
use crate::cli::{CodedError, ErrorCode};
use component_ontology::{Edge, EdgeKind, LifecycleScope, RelatedComponentsFile};
use indexmap::IndexMap;
use serde::Serialize;

use atlas_index::{load_components, ComponentEntry, ComponentsFile};
use knowledge_graph::Justification;

use crate::cli::OutputFormat;
use crate::directed_graph::DirectedGraph;
use crate::repos::{self, RepoEntry, ReposRegistry};
use crate::state::filenames::RELATED_COMPONENTS_FILENAME;
use crate::state::memory::{MemoryEntry, MemoryFile, MEMORY_SCHEMA_VERSION};

const ATLAS_DIR: &str = ".atlas";
const COMPONENTS_FILENAME: &str = "components.yaml";
const MEMORY_FILENAME: &str = "memory.yaml";

/// Per-repo overview, read-only. Same data as `ravel-lite repo list`;
/// surfaced here under `atlas` because the graph-RAG mental model
/// (docs/architecture-next.md §"Catalog as graph") treats the repo
/// registry as the entry point to the catalog graph, separate from
/// the `repo` registry-management surface.
pub fn run_list_repos(context_root: &Path, format: OutputFormat) -> Result<()> {
    repos::run_list(context_root, format)
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
    .context("Failed to serialise atlas freshness report to YAML")
    .with_code(ErrorCode::Internal)?;
    print!("{yaml}");
    if require_fresh {
        let stale: Vec<&str> = report
            .iter()
            .filter(|f| f.status != FreshnessStatus::Fresh)
            .map(|f| f.repo.as_str())
            .collect();
        if !stale.is_empty() {
            bail_with!(
                ErrorCode::Conflict,
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

/// `atlas list-components [--repo R] [--kind K] [--format F]` — list
/// every component in every fresh repo. With no `--format`, emits the
/// human-readable text shape (one line per component as
/// `<repo_slug>/<component_id>  <kind>`); with `--format yaml|json`,
/// emits a versioned envelope with one record per component.
///
/// Exits 0 with no output (or an empty `components: []` list) when
/// the catalog is empty. `--repo` errors when the slug is not a fresh
/// repo so the user does not silently get an empty listing for a
/// typo'd slug.
pub fn run_list_components(
    context_root: &Path,
    repo_filter: Option<&str>,
    kind_filter: Option<&str>,
    format: Option<OutputFormat>,
) -> Result<()> {
    let registry = repos::load_or_empty(context_root)?;
    let catalog = Catalog::load(&registry, SystemTime::now());
    enforce_repo_filter_known(&catalog, repo_filter)?;
    let records: Vec<AtlasComponentRecord> = catalog
        .iter_components()
        .filter(|(slug, comp)| matches_filters(slug, comp, repo_filter, kind_filter))
        .map(|(slug, comp)| AtlasComponentRecord {
            repo: slug.to_string(),
            id: comp.id.clone(),
            kind: comp.kind.clone(),
        })
        .collect();
    match format {
        None => {
            for r in &records {
                println!("{repo}/{id}  {kind}", repo = r.repo, id = r.id, kind = r.kind);
            }
        }
        Some(fmt) => {
            let envelope = AtlasComponentsList {
                schema_version: ATLAS_COMPONENTS_LIST_SCHEMA_VERSION,
                components: records,
            };
            let serialised = match fmt {
                OutputFormat::Yaml => serde_yaml::to_string(&envelope)
                    .context("Failed to serialise atlas components list to YAML")
                    .with_code(ErrorCode::Internal)?,
                OutputFormat::Json => serde_json::to_string_pretty(&envelope)
                    .context("Failed to serialise atlas components list to JSON")
                    .with_code(ErrorCode::Internal)?
                    + "\n",
                OutputFormat::Markdown => bail_with!(
                    ErrorCode::InvalidInput,
                    "format `markdown` is not supported on `atlas list-components`; supported: yaml, json"
                ),
            };
            print!("{serialised}");
        }
    }
    Ok(())
}

/// Schema version for the `atlas list-components --format json|yaml`
/// envelope. Bump on incompatible field changes; new optional fields
/// are non-breaking.
pub const ATLAS_COMPONENTS_LIST_SCHEMA_VERSION: u32 = 1;

#[derive(Debug, Serialize)]
struct AtlasComponentsList {
    schema_version: u32,
    components: Vec<AtlasComponentRecord>,
}

#[derive(Debug, Serialize)]
struct AtlasComponentRecord {
    repo: String,
    id: String,
    kind: String,
}

/// `atlas summary [--repo R] [--format F]` — per-repo component
/// counts grouped by kind. Without `--format`, emits the human-
/// readable text shape:
///
/// ```text
/// <repo_slug>  (<total> total)
///   <count>  <kind>
///   ...
/// ```
///
/// With `--format yaml|json`, emits a versioned envelope with one
/// record per repo carrying `total` and `by_kind` (a map of kind name
/// → count). Repos with no components (or filtered out) are omitted
/// from both shapes. `--repo` errors when the slug is not a fresh
/// repo.
pub fn run_summary(
    context_root: &Path,
    repo_filter: Option<&str>,
    format: Option<OutputFormat>,
) -> Result<()> {
    let registry = repos::load_or_empty(context_root)?;
    let catalog = Catalog::load(&registry, SystemTime::now());
    enforce_repo_filter_known(&catalog, repo_filter)?;
    let by_repo = aggregate_summary(&catalog, repo_filter);
    match format {
        None => {
            for (slug, kinds) in &by_repo {
                let total: usize = kinds.values().sum();
                println!("{slug}  ({total} total)");
                for (kind, count) in kinds {
                    println!("  {count:>4}  {kind}");
                }
            }
        }
        Some(fmt) => {
            let repos: Vec<AtlasRepoSummary> = by_repo
                .iter()
                .map(|(slug, kinds)| AtlasRepoSummary {
                    repo: slug.clone(),
                    total: kinds.values().sum(),
                    by_kind: kinds.clone(),
                })
                .collect();
            let envelope = AtlasSummary {
                schema_version: ATLAS_SUMMARY_SCHEMA_VERSION,
                repos,
            };
            let serialised = match fmt {
                OutputFormat::Yaml => serde_yaml::to_string(&envelope)
                    .context("Failed to serialise atlas summary to YAML")
                    .with_code(ErrorCode::Internal)?,
                OutputFormat::Json => serde_json::to_string_pretty(&envelope)
                    .context("Failed to serialise atlas summary to JSON")
                    .with_code(ErrorCode::Internal)?
                    + "\n",
                OutputFormat::Markdown => bail_with!(
                    ErrorCode::InvalidInput,
                    "format `markdown` is not supported on `atlas summary`; supported: yaml, json"
                ),
            };
            print!("{serialised}");
        }
    }
    Ok(())
}

/// Schema version for the `atlas summary --format json|yaml`
/// envelope. Bump on incompatible field changes.
pub const ATLAS_SUMMARY_SCHEMA_VERSION: u32 = 1;

#[derive(Debug, Serialize)]
struct AtlasSummary {
    schema_version: u32,
    repos: Vec<AtlasRepoSummary>,
}

#[derive(Debug, Serialize)]
struct AtlasRepoSummary {
    repo: String,
    total: usize,
    by_kind: BTreeMap<String, usize>,
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
        bail_with!(
            ErrorCode::NotFound,
            "--repo {slug:?}: no fresh repos in catalog (registry empty or all repos lack a fresh `.atlas/components.yaml`)"
        );
    }
    bail_with!(
        ErrorCode::NotFound,
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
        let message = if available.is_empty() {
            format!(
                "ref {slug:?}/{id:?}: no fresh repos in catalog (registry empty or all repos lack a fresh `.atlas/components.yaml`)"
            )
        } else {
            format!(
                "ref {slug:?}/{id:?}: unknown repo slug; fresh repos: [{}]",
                available.join(", ")
            )
        };
        anyhow::Error::new(CodedError {
            code: ErrorCode::NotFound,
            message,
        })
    })?;
    let component = repo
        .file
        .components
        .iter()
        .find(|c| !c.deleted && c.id == id)
        .ok_or_else(|| anyhow::Error::new(CodedError {
            code: ErrorCode::NotFound,
            message: format!(
                "ref {slug:?}/{id:?}: no component with that id in repo {slug:?}"
            ),
        }))?;
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
        0 => bail_with!(
            ErrorCode::NotFound,
            "ref {id:?}: no component with that id in any fresh repo"
        ),
        1 => Ok(hits.into_iter().next().expect("len == 1")),
        _ => {
            let qualified: Vec<String> = hits
                .iter()
                .map(|h| format!("{}/{}", h.repo_slug, h.component.id))
                .collect();
            bail_with!(
                ErrorCode::Conflict,
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
        .context("Failed to serialise atlas describe report to YAML")
        .with_code(ErrorCode::Internal)?;
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
        .with_context(|| format!("Failed to read {}", path.display()))
        .with_code(ErrorCode::IoError)?;
    let parsed: MemoryFile = serde_yaml::from_str(&text)
        .with_context(|| {
            format!("Failed to parse {} as component memory.yaml schema", path.display())
        })
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
        .context("Failed to serialise component memory to YAML")
        .with_code(ErrorCode::Internal)?;
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

// ---------- Edge graph: cross-repo `related-components.yaml` union ----------

/// Direction filter for `atlas edges <ref>`. `Both` is the default
/// when no flag is supplied. Symmetric edge kinds (e.g.
/// `co-implements`, `communicates-with`) match every direction
/// because the relation has no inherent source/destination — filtering
/// them by `In` / `Out` would silently hide them, which is more
/// surprising than the alternative.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EdgeDirection {
    Both,
    In,
    Out,
}

/// Union of every fresh repo's `.atlas/related-components.yaml`,
/// deduplicated by `Edge::canonical_key`. Symmetric edges canonicalise
/// to sorted participants; directed edges keep the order recorded on
/// disk. The resulting `Vec<Edge>` is ordered by first-seen
/// (registry-then-file order) so downstream output is stable.
#[derive(Debug, Clone, Default)]
pub struct EdgeGraph {
    pub edges: Vec<Edge>,
}

impl EdgeGraph {
    /// Walk every fresh repo in `catalog`, loading
    /// `<local_path>/.atlas/related-components.yaml` from each one.
    /// Missing files are treated as empty (a repo that has not yet
    /// recorded any edges is valid). Schema-version mismatches and
    /// invalid edges still hard-error via the ontology loader.
    pub fn from_catalog(catalog: &Catalog) -> Result<EdgeGraph> {
        let mut seen_keys: HashSet<(EdgeKind, LifecycleScope, Vec<String>)> = HashSet::new();
        let mut edges: Vec<Edge> = Vec::new();
        for (slug, rc) in &catalog.repos {
            let path = rc.local_path.join(ATLAS_DIR).join(RELATED_COMPONENTS_FILENAME);
            let file: RelatedComponentsFile =
                component_ontology::load_or_default(&path)
                    .with_context(|| {
                        format!("repo {slug}: failed loading {}", path.display())
                    })
                    .with_code(ErrorCode::IoError)?;
            for edge in file.edges {
                let key = edge.canonical_key();
                if seen_keys.insert(key) {
                    edges.push(edge);
                }
            }
        }
        Ok(EdgeGraph { edges })
    }

    /// Edges touching `component`, filtered by `direction`. For
    /// directed edges: `Out` matches when `component` is the first
    /// participant, `In` when it is the second; for symmetric edges
    /// every direction matches.
    pub fn edges_touching(&self, component: &str, direction: EdgeDirection) -> Vec<&Edge> {
        self.edges
            .iter()
            .filter(|e| edge_matches_direction(e, component, direction))
            .collect()
    }

    /// Bounded-depth BFS from `start` over the undirected edge graph
    /// (every edge is traversable in both directions). Returns
    /// `(component, hops)` pairs in increasing hop order, BFS order
    /// within each hop. The starting node is included at hop 0.
    pub fn neighbors(&self, start: &str, depth: usize) -> Vec<(String, usize)> {
        let adjacency = self.adjacency();
        let mut visited: BTreeSet<String> = BTreeSet::new();
        let mut queue: VecDeque<(String, usize)> = VecDeque::new();
        let mut order: Vec<(String, usize)> = Vec::new();

        visited.insert(start.to_string());
        queue.push_back((start.to_string(), 0));
        order.push((start.to_string(), 0));

        while let Some((node, hops)) = queue.pop_front() {
            if hops == depth {
                continue;
            }
            if let Some(peers) = adjacency.get(&node) {
                for peer in peers {
                    if visited.insert(peer.clone()) {
                        queue.push_back((peer.clone(), hops + 1));
                        order.push((peer.clone(), hops + 1));
                    }
                }
            }
        }
        order
    }

    /// Components with no incoming directed edges. `component_universe`
    /// supplies the candidate set (typically the catalog's component
    /// IDs) so that isolated components — never mentioned in any edge —
    /// also surface as roots. Symmetric edges do not disqualify either
    /// endpoint: peer relationships do not establish hierarchy.
    pub fn roots<'a, I>(&self, component_universe: I) -> Vec<String>
    where
        I: IntoIterator<Item = &'a str>,
    {
        let mut has_incoming: HashSet<&str> = HashSet::new();
        for edge in &self.edges {
            if edge.kind.is_directed() {
                has_incoming.insert(edge.participants[1].as_str());
            }
        }
        let mut roots: Vec<String> = Vec::new();
        let mut seen: HashSet<String> = HashSet::new();
        for component in component_universe {
            if has_incoming.contains(component) {
                continue;
            }
            if seen.insert(component.to_string()) {
                roots.push(component.to_string());
            }
        }
        roots
    }

    /// Adjacency map keyed by component, listing each peer reachable
    /// in one hop. Both endpoints of every edge gain each other as
    /// peers — direction is irrelevant for neighborhood expansion.
    /// Peer order matches first-seen edge order; duplicates are
    /// suppressed.
    fn adjacency(&self) -> BTreeMap<String, Vec<String>> {
        let mut map: BTreeMap<String, Vec<String>> = BTreeMap::new();
        for edge in &self.edges {
            let a = &edge.participants[0];
            let b = &edge.participants[1];
            push_unique(map.entry(a.clone()).or_default(), b);
            push_unique(map.entry(b.clone()).or_default(), a);
        }
        map
    }
}

/// Project a `(Catalog, EdgeGraph)` pair into a directed adjacency
/// graph keyed on bare component IDs. The substrate for `atlas path`
/// (BFS) and `atlas scc` (Kosaraju/Tarjan), tracked as a follow-up.
///
/// **Symmetric edges (`co-implements`, `communicates-with`) are
/// excluded.** They have no inherent source/destination, and including
/// them as bidirectional pairs would surface every peer relationship
/// as a 2-node SCC — drowning real circular-dependency findings, which
/// is `atlas scc`'s motivating use case. Peer relationships remain
/// visible via `atlas neighbors` (undirected BFS over `EdgeGraph`).
///
/// Every fresh-repo component participates as a node, even if it
/// touches no directed edge — mirroring the universe pattern in
/// [`EdgeGraph::roots`] so isolated components don't silently vanish.
pub fn build_directed_component_graph(catalog: &Catalog) -> Result<DirectedGraph<String>> {
    let edges = EdgeGraph::from_catalog(catalog)?;
    let mut graph = DirectedGraph::new();
    for (_slug, component) in catalog.iter_components() {
        graph.add_node(component.id.clone());
    }
    for edge in &edges.edges {
        if edge.kind.is_directed() {
            graph.add_edge(edge.participants[0].clone(), edge.participants[1].clone());
        }
    }
    Ok(graph)
}

fn push_unique(target: &mut Vec<String>, value: &str) {
    if !target.iter().any(|existing| existing == value) {
        target.push(value.to_string());
    }
}

fn edge_matches_direction(edge: &Edge, component: &str, direction: EdgeDirection) -> bool {
    if !edge.involves(component) {
        return false;
    }
    if !edge.kind.is_directed() {
        // Symmetric edges have no source/destination; keeping them
        // visible under In/Out keeps `--in`/`--out` from silently
        // dropping legitimate peer relationships.
        return true;
    }
    match direction {
        EdgeDirection::Both => true,
        EdgeDirection::Out => edge.participants[0] == component,
        EdgeDirection::In => edge.participants[1] == component,
    }
}

// ---------- edges / neighbors / roots verbs ----------

/// `atlas edges <ref> [--in | --out | --both]` — list direct edges
/// touching `<ref>` as `<from>  --<kind>/<lifecycle>-->  <to>`. The
/// reference is resolved against the catalog (qualified `<repo>/<id>`
/// or unique bare `<id>`); only the bare component id participates in
/// edge matching because that is what `related-components.yaml`
/// participants record.
pub fn run_edges(context_root: &Path, ref_str: &str, direction: EdgeDirection) -> Result<()> {
    let registry = repos::load_or_empty(context_root)?;
    let catalog = Catalog::load(&registry, SystemTime::now());
    let resolved = resolve_ref(&catalog, ref_str)?;
    let graph = EdgeGraph::from_catalog(&catalog)?;
    for edge in graph.edges_touching(&resolved.component.id, direction) {
        println!("{}", format_edge_line(edge));
    }
    Ok(())
}

/// `atlas neighbors <ref> [--depth N]` — print every component
/// reachable from `<ref>` within `N` hops over the undirected edge
/// graph, one per line as `<hops>  <component>`. The starting
/// component is always emitted at hop 0; `--depth 0` reduces to that
/// single line.
pub fn run_neighbors(context_root: &Path, ref_str: &str, depth: usize) -> Result<()> {
    let registry = repos::load_or_empty(context_root)?;
    let catalog = Catalog::load(&registry, SystemTime::now());
    let resolved = resolve_ref(&catalog, ref_str)?;
    let graph = EdgeGraph::from_catalog(&catalog)?;
    for (component, hops) in graph.neighbors(&resolved.component.id, depth) {
        println!("{hops}  {component}");
    }
    Ok(())
}

/// `atlas path <from> <to> [--max-hops N]` — BFS shortest path over
/// the directed component graph (symmetric edges excluded; see
/// [`build_directed_component_graph`]). Endpoints are resolved against
/// the catalog (qualified `<repo>/<id>` or unique bare `<id>`); the
/// path is printed as one bare component id per line in traversal
/// order. If no path exists within `max_hops`, the function returns
/// an error so the caller exits non-zero with a clear "no path found"
/// diagnostic.
pub fn run_path(context_root: &Path, from: &str, to: &str, max_hops: usize) -> Result<()> {
    let registry = repos::load_or_empty(context_root)?;
    let catalog = Catalog::load(&registry, SystemTime::now());
    let from_resolved = resolve_ref(&catalog, from)?;
    let to_resolved = resolve_ref(&catalog, to)?;
    let graph = build_directed_component_graph(&catalog)?;
    match graph.shortest_path(
        &from_resolved.component.id,
        &to_resolved.component.id,
        max_hops,
    ) {
        Some(path) => {
            for node in path {
                println!("{node}");
            }
            Ok(())
        }
        None => bail_with!(
            ErrorCode::NotFound,
            "no path found from {} to {} within {} hops",
            from_resolved.component.id,
            to_resolved.component.id,
            max_hops,
        ),
    }
}

/// `atlas scc [--all]` — strongly connected components of the
/// directed component graph via Tarjan's algorithm. Each SCC is
/// printed on its own line as a comma-separated list of bare
/// component ids. By default only non-trivial SCCs (size > 1) appear,
/// because singleton SCCs are the common case and the motivating use
/// case is detecting circular dependencies; `--all` includes
/// singletons for full coverage.
pub fn run_scc(context_root: &Path, include_singletons: bool) -> Result<()> {
    let registry = repos::load_or_empty(context_root)?;
    let catalog = Catalog::load(&registry, SystemTime::now());
    let graph = build_directed_component_graph(&catalog)?;
    let sccs = graph.strongly_connected_components();
    for component in sccs {
        if !include_singletons && component.len() < 2 {
            continue;
        }
        println!("{}", component.join(", "));
    }
    Ok(())
}

/// `atlas roots` — every catalog component with no incoming directed
/// edge, one per line. Output is qualified `<repo_slug>/<component_id>`
/// because the same bare id may legally appear in multiple repos
/// (catalog hygiene is enforced only at lookup time, not on disk).
pub fn run_roots(context_root: &Path) -> Result<()> {
    let registry = repos::load_or_empty(context_root)?;
    let catalog = Catalog::load(&registry, SystemTime::now());
    let graph = EdgeGraph::from_catalog(&catalog)?;
    let mut has_incoming: HashSet<&str> = HashSet::new();
    for edge in &graph.edges {
        if edge.kind.is_directed() {
            has_incoming.insert(edge.participants[1].as_str());
        }
    }
    for (slug, component) in catalog.iter_components() {
        if has_incoming.contains(component.id.as_str()) {
            continue;
        }
        println!("{slug}/{id}", id = component.id);
    }
    Ok(())
}

fn format_edge_line(edge: &Edge) -> String {
    format!(
        "{from}  --{kind}/{lifecycle}-->  {to}",
        from = edge.participants[0],
        kind = edge.kind.as_str(),
        lifecycle = edge.lifecycle.as_str(),
        to = edge.participants[1],
    )
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
        run_list_components(tmp.path(), None, None, None).unwrap();
    }

    #[test]
    fn run_list_components_yaml_envelope_carries_schema_version() {
        let tmp = TempDir::new().unwrap();
        run_list_components(tmp.path(), None, None, Some(OutputFormat::Yaml)).unwrap();
        // Smoke: an empty catalog still produces a serialisable envelope.
        // Round-trip: an envelope containing one component renders both
        // schema_version and the component fields.
        let envelope = AtlasComponentsList {
            schema_version: ATLAS_COMPONENTS_LIST_SCHEMA_VERSION,
            components: vec![AtlasComponentRecord {
                repo: "atlas".into(),
                id: "core".into(),
                kind: "service".into(),
            }],
        };
        let yaml = serde_yaml::to_string(&envelope).unwrap();
        assert!(yaml.contains("schema_version: 1"), "yaml:\n{yaml}");
        assert!(yaml.contains("repo: atlas"), "yaml:\n{yaml}");
        let json = serde_json::to_string(&envelope).unwrap();
        assert!(json.contains("\"schema_version\":1"), "json: {json}");
        assert!(json.contains("\"repo\":\"atlas\""), "json: {json}");
    }

    #[test]
    fn run_summary_with_empty_registry_succeeds_silently() {
        let tmp = TempDir::new().unwrap();
        run_summary(tmp.path(), None, None).unwrap();
    }

    #[test]
    fn run_summary_envelope_carries_schema_version_and_by_kind_map() {
        let mut by_kind = BTreeMap::new();
        by_kind.insert("service".to_string(), 3);
        by_kind.insert("library".to_string(), 5);
        let envelope = AtlasSummary {
            schema_version: ATLAS_SUMMARY_SCHEMA_VERSION,
            repos: vec![AtlasRepoSummary {
                repo: "atlas".into(),
                total: 8,
                by_kind,
            }],
        };
        let json = serde_json::to_string(&envelope).unwrap();
        assert!(json.contains("\"schema_version\":1"), "json: {json}");
        assert!(json.contains("\"total\":8"), "json: {json}");
        assert!(json.contains("\"by_kind\""), "json: {json}");
        assert!(json.contains("\"library\":5"), "json: {json}");
        assert!(json.contains("\"service\":3"), "json: {json}");
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
        std::fs::create_dir_all(dead_dir.join(ATLAS_DIR)).unwrap();
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

    // ---------- EdgeGraph / edges / neighbors / roots tests ----------

    use component_ontology::EvidenceGrade;

    /// Build an `Edge` with the canonical participant order required
    /// by `Edge::validate` (sorted for symmetric kinds, caller-supplied
    /// for directed kinds). Strong evidence so `evidence_fields` need
    /// not be empty.
    fn edge(kind: EdgeKind, lifecycle: LifecycleScope, a: &str, b: &str) -> Edge {
        let participants = if kind.is_directed() {
            vec![a.to_string(), b.to_string()]
        } else {
            let mut p = vec![a.to_string(), b.to_string()];
            p.sort();
            p
        };
        Edge {
            kind,
            lifecycle,
            participants,
            evidence_grade: EvidenceGrade::Strong,
            evidence_fields: vec![format!("{a}.x")],
            rationale: "test".into(),
        }
    }

    /// Save `edges` to `<repo_dir>/.atlas/related-components.yaml`,
    /// creating the parent directory. The repo dir must already exist.
    fn write_related_components_yaml(repo_dir: &Path, edges: &[Edge]) {
        let dir = repo_dir.join(ATLAS_DIR);
        std::fs::create_dir_all(&dir).unwrap();
        let mut file = RelatedComponentsFile::default();
        for e in edges {
            file.add_edge(e.clone()).unwrap();
        }
        component_ontology::save_atomic(&dir.join(RELATED_COMPONENTS_FILENAME), &file).unwrap();
    }

    /// One repo's contribution to the test fixture: slug, the
    /// `components.yaml` rows it should contain, and the edge list for
    /// its `related-components.yaml`. Defined as an alias because the
    /// raw tuple trips clippy's `type_complexity` lint.
    type RepoSpec<'a> = (&'a str, &'a [(&'a str, &'a str)], &'a [Edge]);

    /// Stand up a registry with `repo_specs.len()` fresh repos. The
    /// repo's `components.yaml` and `related-components.yaml` are
    /// written under `<tmp>/<slug>/.atlas/`.
    fn registry_with_repos(tmp: &TempDir, repo_specs: &[RepoSpec<'_>]) -> ReposRegistry {
        let mut reg = empty_registry();
        for (slug, components, edges) in repo_specs {
            let repo_dir = tmp.path().join(slug);
            std::fs::create_dir_all(&repo_dir).unwrap();
            write_components_yaml(&repo_dir, components);
            write_related_components_yaml(&repo_dir, edges);
            repos::try_add(&mut reg, slug, "u", Some(&repo_dir)).unwrap();
        }
        reg
    }

    #[test]
    fn edge_graph_is_empty_when_no_repos_have_related_components_yaml() {
        let tmp = TempDir::new().unwrap();
        let reg = registry_with_one_fresh_repo(&tmp, "atlas", &[("a-1", "library")]);
        let catalog = Catalog::load(&reg, SystemTime::now());
        let graph = EdgeGraph::from_catalog(&catalog).unwrap();
        assert!(graph.edges.is_empty());
    }

    #[test]
    fn edge_graph_loads_edges_from_a_single_repo() {
        let tmp = TempDir::new().unwrap();
        let edges = [edge(EdgeKind::DependsOn, LifecycleScope::Build, "A", "B")];
        let reg = registry_with_repos(
            &tmp,
            &[("atlas", &[("A", "library"), ("B", "library")], &edges)],
        );
        let catalog = Catalog::load(&reg, SystemTime::now());
        let graph = EdgeGraph::from_catalog(&catalog).unwrap();
        assert_eq!(graph.edges.len(), 1);
        assert_eq!(graph.edges[0].participants, vec!["A".to_string(), "B".to_string()]);
    }

    #[test]
    fn edge_graph_unions_edges_across_repos() {
        let tmp = TempDir::new().unwrap();
        let alpha_edges = [edge(EdgeKind::DependsOn, LifecycleScope::Build, "Alpha-A", "Alpha-B")];
        let beta_edges = [edge(EdgeKind::DependsOn, LifecycleScope::Build, "Beta-A", "Beta-B")];
        let reg = registry_with_repos(
            &tmp,
            &[
                ("alpha", &[("Alpha-A", "library"), ("Alpha-B", "library")], &alpha_edges),
                ("beta", &[("Beta-A", "library"), ("Beta-B", "library")], &beta_edges),
            ],
        );
        let catalog = Catalog::load(&reg, SystemTime::now());
        let graph = EdgeGraph::from_catalog(&catalog).unwrap();
        assert_eq!(graph.edges.len(), 2, "union retains both repos' edges");
    }

    #[test]
    fn edge_graph_dedups_canonical_keys_across_repos() {
        // Same directed edge recorded in two repos → one survives.
        let tmp = TempDir::new().unwrap();
        let shared = [edge(EdgeKind::Generates, LifecycleScope::Codegen, "Gen", "Out")];
        let reg = registry_with_repos(
            &tmp,
            &[
                ("alpha", &[("Gen", "library"), ("Out", "library")], &shared),
                ("beta", &[("Gen", "library"), ("Out", "library")], &shared),
            ],
        );
        let catalog = Catalog::load(&reg, SystemTime::now());
        let graph = EdgeGraph::from_catalog(&catalog).unwrap();
        assert_eq!(graph.edges.len(), 1, "duplicate canonical key collapses to one edge");
    }

    #[test]
    fn edge_graph_keeps_directed_edges_with_swapped_participants_distinct() {
        // A→B and B→A are two distinct directed edges, not duplicates.
        let tmp = TempDir::new().unwrap();
        let edges = [
            edge(EdgeKind::Generates, LifecycleScope::Codegen, "A", "B"),
            edge(EdgeKind::Generates, LifecycleScope::Codegen, "B", "A"),
        ];
        let reg = registry_with_repos(
            &tmp,
            &[("alpha", &[("A", "library"), ("B", "library")], &edges)],
        );
        let catalog = Catalog::load(&reg, SystemTime::now());
        let graph = EdgeGraph::from_catalog(&catalog).unwrap();
        assert_eq!(graph.edges.len(), 2);
    }

    #[test]
    fn edge_graph_dedups_symmetric_edge_recorded_in_two_repos() {
        // Symmetric `co-implements` between Alpha and Beta, recorded in
        // both repos with the same (sorted) participant order.
        let tmp = TempDir::new().unwrap();
        let shared =
            [edge(EdgeKind::CoImplements, LifecycleScope::Design, "Alpha", "Beta")];
        let reg = registry_with_repos(
            &tmp,
            &[
                ("alpha", &[("Alpha", "library"), ("Beta", "library")], &shared),
                ("beta", &[("Alpha", "library"), ("Beta", "library")], &shared),
            ],
        );
        let catalog = Catalog::load(&reg, SystemTime::now());
        let graph = EdgeGraph::from_catalog(&catalog).unwrap();
        assert_eq!(graph.edges.len(), 1);
    }

    #[test]
    fn edges_touching_filters_directed_edges_by_in_out_both() {
        let tmp = TempDir::new().unwrap();
        // Directed: A → B (A is source, B is sink).
        let edges = [edge(EdgeKind::DependsOn, LifecycleScope::Build, "A", "B")];
        let reg = registry_with_repos(
            &tmp,
            &[("alpha", &[("A", "library"), ("B", "library")], &edges)],
        );
        let catalog = Catalog::load(&reg, SystemTime::now());
        let graph = EdgeGraph::from_catalog(&catalog).unwrap();

        // From A's perspective: --out matches, --in does not.
        assert_eq!(graph.edges_touching("A", EdgeDirection::Out).len(), 1);
        assert_eq!(graph.edges_touching("A", EdgeDirection::In).len(), 0);
        assert_eq!(graph.edges_touching("A", EdgeDirection::Both).len(), 1);

        // From B's perspective: --in matches, --out does not.
        assert_eq!(graph.edges_touching("B", EdgeDirection::Out).len(), 0);
        assert_eq!(graph.edges_touching("B", EdgeDirection::In).len(), 1);
        assert_eq!(graph.edges_touching("B", EdgeDirection::Both).len(), 1);
    }

    #[test]
    fn edges_touching_returns_symmetric_edges_in_every_direction() {
        let tmp = TempDir::new().unwrap();
        // Symmetric: stored sorted as Alpha, Beta.
        let edges =
            [edge(EdgeKind::CoImplements, LifecycleScope::Design, "Alpha", "Beta")];
        let reg = registry_with_repos(
            &tmp,
            &[("alpha", &[("Alpha", "library"), ("Beta", "library")], &edges)],
        );
        let catalog = Catalog::load(&reg, SystemTime::now());
        let graph = EdgeGraph::from_catalog(&catalog).unwrap();

        for direction in [EdgeDirection::Both, EdgeDirection::In, EdgeDirection::Out] {
            assert_eq!(
                graph.edges_touching("Alpha", direction).len(),
                1,
                "symmetric edges must surface in {direction:?}"
            );
            assert_eq!(graph.edges_touching("Beta", direction).len(), 1);
        }
    }

    #[test]
    fn edges_touching_excludes_edges_not_involving_the_component() {
        let tmp = TempDir::new().unwrap();
        let edges = [edge(EdgeKind::DependsOn, LifecycleScope::Build, "A", "B")];
        let reg = registry_with_repos(
            &tmp,
            &[("alpha", &[("A", "library"), ("B", "library"), ("C", "library")], &edges)],
        );
        let catalog = Catalog::load(&reg, SystemTime::now());
        let graph = EdgeGraph::from_catalog(&catalog).unwrap();
        assert!(graph.edges_touching("C", EdgeDirection::Both).is_empty());
    }

    #[test]
    fn neighbors_emits_only_self_at_depth_zero() {
        let tmp = TempDir::new().unwrap();
        let edges = [edge(EdgeKind::DependsOn, LifecycleScope::Build, "A", "B")];
        let reg = registry_with_repos(
            &tmp,
            &[("alpha", &[("A", "library"), ("B", "library")], &edges)],
        );
        let catalog = Catalog::load(&reg, SystemTime::now());
        let graph = EdgeGraph::from_catalog(&catalog).unwrap();
        let result = graph.neighbors("A", 0);
        assert_eq!(result, vec![("A".to_string(), 0)]);
    }

    #[test]
    fn neighbors_default_depth_one_visits_direct_peers_in_either_direction() {
        // Directed A → B; from B's perspective B is the sink, but
        // neighbors expansion is undirected so A is reachable in 1 hop.
        let tmp = TempDir::new().unwrap();
        let edges = [edge(EdgeKind::DependsOn, LifecycleScope::Build, "A", "B")];
        let reg = registry_with_repos(
            &tmp,
            &[("alpha", &[("A", "library"), ("B", "library")], &edges)],
        );
        let catalog = Catalog::load(&reg, SystemTime::now());
        let graph = EdgeGraph::from_catalog(&catalog).unwrap();

        let from_b = graph.neighbors("B", 1);
        let names: Vec<&str> = from_b.iter().map(|(n, _)| n.as_str()).collect();
        assert!(names.contains(&"A"), "undirected expansion reaches A from B");
        assert_eq!(from_b.len(), 2, "self + one peer");
    }

    #[test]
    fn neighbors_does_not_exceed_requested_depth() {
        // Chain A → B → C. depth=1 from A reaches B but not C.
        let tmp = TempDir::new().unwrap();
        let edges = [
            edge(EdgeKind::DependsOn, LifecycleScope::Build, "A", "B"),
            edge(EdgeKind::DependsOn, LifecycleScope::Build, "B", "C"),
        ];
        let reg = registry_with_repos(
            &tmp,
            &[(
                "alpha",
                &[("A", "library"), ("B", "library"), ("C", "library")],
                &edges,
            )],
        );
        let catalog = Catalog::load(&reg, SystemTime::now());
        let graph = EdgeGraph::from_catalog(&catalog).unwrap();

        let depth1 = graph.neighbors("A", 1);
        let names1: BTreeSet<&str> = depth1.iter().map(|(n, _)| n.as_str()).collect();
        assert_eq!(names1, BTreeSet::from(["A", "B"]));

        let depth2 = graph.neighbors("A", 2);
        let names2: BTreeSet<&str> = depth2.iter().map(|(n, _)| n.as_str()).collect();
        assert_eq!(names2, BTreeSet::from(["A", "B", "C"]));
    }

    #[test]
    fn neighbors_terminates_on_cycles_without_revisiting() {
        // Cycle A → B → C → A. Visit each node once.
        let tmp = TempDir::new().unwrap();
        let edges = [
            edge(EdgeKind::DependsOn, LifecycleScope::Build, "A", "B"),
            edge(EdgeKind::DependsOn, LifecycleScope::Build, "B", "C"),
            edge(EdgeKind::DependsOn, LifecycleScope::Build, "C", "A"),
        ];
        let reg = registry_with_repos(
            &tmp,
            &[(
                "alpha",
                &[("A", "library"), ("B", "library"), ("C", "library")],
                &edges,
            )],
        );
        let catalog = Catalog::load(&reg, SystemTime::now());
        let graph = EdgeGraph::from_catalog(&catalog).unwrap();
        let result = graph.neighbors("A", 10);
        assert_eq!(result.len(), 3, "each node visited exactly once");
        let names: BTreeSet<&str> = result.iter().map(|(n, _)| n.as_str()).collect();
        assert_eq!(names, BTreeSet::from(["A", "B", "C"]));
    }

    #[test]
    fn neighbors_assigns_minimum_hop_count_via_bfs() {
        // Diamond A→B, A→C, B→D, C→D. From A: B/C at hop 1, D at hop 2.
        let tmp = TempDir::new().unwrap();
        let edges = [
            edge(EdgeKind::DependsOn, LifecycleScope::Build, "A", "B"),
            edge(EdgeKind::DependsOn, LifecycleScope::Build, "A", "C"),
            edge(EdgeKind::DependsOn, LifecycleScope::Build, "B", "D"),
            edge(EdgeKind::DependsOn, LifecycleScope::Build, "C", "D"),
        ];
        let reg = registry_with_repos(
            &tmp,
            &[(
                "alpha",
                &[
                    ("A", "library"),
                    ("B", "library"),
                    ("C", "library"),
                    ("D", "library"),
                ],
                &edges,
            )],
        );
        let catalog = Catalog::load(&reg, SystemTime::now());
        let graph = EdgeGraph::from_catalog(&catalog).unwrap();
        let result = graph.neighbors("A", 5);
        let by_name: BTreeMap<&str, usize> =
            result.iter().map(|(n, h)| (n.as_str(), *h)).collect();
        assert_eq!(by_name.get("A"), Some(&0));
        assert_eq!(by_name.get("B"), Some(&1));
        assert_eq!(by_name.get("C"), Some(&1));
        assert_eq!(by_name.get("D"), Some(&2), "BFS picks the shortest path");
    }

    #[test]
    fn roots_includes_components_with_no_incoming_directed_edge() {
        // A → B. Only A has no incoming edge among {A, B}.
        let tmp = TempDir::new().unwrap();
        let edges = [edge(EdgeKind::DependsOn, LifecycleScope::Build, "A", "B")];
        let reg = registry_with_repos(
            &tmp,
            &[("alpha", &[("A", "library"), ("B", "library")], &edges)],
        );
        let catalog = Catalog::load(&reg, SystemTime::now());
        let graph = EdgeGraph::from_catalog(&catalog).unwrap();
        let universe: Vec<&str> = catalog.iter_components().map(|(_, c)| c.id.as_str()).collect();
        let roots = graph.roots(universe.iter().copied());
        assert_eq!(roots, vec!["A".to_string()]);
    }

    #[test]
    fn roots_treats_symmetric_edges_as_non_disqualifying() {
        // A and B share only a `co-implements` (symmetric). Both still
        // qualify as roots: peer relationships don't establish hierarchy.
        let tmp = TempDir::new().unwrap();
        let edges = [edge(EdgeKind::CoImplements, LifecycleScope::Design, "A", "B")];
        let reg = registry_with_repos(
            &tmp,
            &[("alpha", &[("A", "library"), ("B", "library")], &edges)],
        );
        let catalog = Catalog::load(&reg, SystemTime::now());
        let graph = EdgeGraph::from_catalog(&catalog).unwrap();
        let universe: Vec<&str> = catalog.iter_components().map(|(_, c)| c.id.as_str()).collect();
        let roots = graph.roots(universe.iter().copied());
        let set: BTreeSet<String> = roots.into_iter().collect();
        assert_eq!(set, BTreeSet::from(["A".to_string(), "B".to_string()]));
    }

    #[test]
    fn roots_includes_isolated_components_with_no_edges_at_all() {
        // C is in the catalog but appears in no edge.
        let tmp = TempDir::new().unwrap();
        let edges = [edge(EdgeKind::DependsOn, LifecycleScope::Build, "A", "B")];
        let reg = registry_with_repos(
            &tmp,
            &[(
                "alpha",
                &[("A", "library"), ("B", "library"), ("C", "library")],
                &edges,
            )],
        );
        let catalog = Catalog::load(&reg, SystemTime::now());
        let graph = EdgeGraph::from_catalog(&catalog).unwrap();
        let universe: Vec<&str> = catalog.iter_components().map(|(_, c)| c.id.as_str()).collect();
        let roots: BTreeSet<String> = graph.roots(universe.iter().copied()).into_iter().collect();
        assert!(roots.contains("A"));
        assert!(roots.contains("C"), "isolated component must surface as root");
        assert!(!roots.contains("B"));
    }

    #[test]
    fn roots_returns_empty_when_every_component_has_an_incoming_directed_edge() {
        // Cycle A → B → A: each has incoming, so no roots.
        let tmp = TempDir::new().unwrap();
        let edges = [
            edge(EdgeKind::DependsOn, LifecycleScope::Build, "A", "B"),
            edge(EdgeKind::DependsOn, LifecycleScope::Build, "B", "A"),
        ];
        let reg = registry_with_repos(
            &tmp,
            &[("alpha", &[("A", "library"), ("B", "library")], &edges)],
        );
        let catalog = Catalog::load(&reg, SystemTime::now());
        let graph = EdgeGraph::from_catalog(&catalog).unwrap();
        let universe: Vec<&str> = catalog.iter_components().map(|(_, c)| c.id.as_str()).collect();
        let roots = graph.roots(universe.iter().copied());
        assert!(roots.is_empty(), "cycle leaves no node without an incoming edge");
    }

    #[test]
    fn from_catalog_propagates_schema_version_mismatch_with_repo_context() {
        // Hand-write a v1 file; the loader must reject and the error
        // chain must mention the offending repo slug for diagnosis.
        let tmp = TempDir::new().unwrap();
        let reg = registry_with_one_fresh_repo(&tmp, "alpha", &[("A", "library")]);
        let path = tmp.path().join("alpha").join(ATLAS_DIR).join(RELATED_COMPONENTS_FILENAME);
        std::fs::write(&path, "schema_version: 1\nedges: []\n").unwrap();
        let catalog = Catalog::load(&reg, SystemTime::now());
        let err = EdgeGraph::from_catalog(&catalog).unwrap_err();
        let chain = format!("{err:#}");
        assert!(
            chain.contains("alpha"),
            "error chain must identify the offending repo: {chain}"
        );
        assert!(chain.contains("schema_version"));
    }

    #[test]
    fn format_edge_line_renders_directed_edges_with_arrow() {
        let e = edge(EdgeKind::DependsOn, LifecycleScope::Build, "A", "B");
        assert_eq!(format_edge_line(&e), "A  --depends-on/build-->  B");
    }

    #[test]
    fn format_edge_line_renders_symmetric_edges_with_sorted_participants() {
        let e = edge(EdgeKind::CoImplements, LifecycleScope::Design, "Beta", "Alpha");
        // Symmetric kinds canonicalise to sorted participants.
        assert_eq!(
            format_edge_line(&e),
            "Alpha  --co-implements/design-->  Beta"
        );
    }

    // ---------- build_directed_component_graph tests ----------

    #[test]
    fn directed_component_graph_is_empty_when_no_edges_present() {
        let tmp = TempDir::new().unwrap();
        let reg = registry_with_one_fresh_repo(&tmp, "alpha", &[("A", "library")]);
        let catalog = Catalog::load(&reg, SystemTime::now());
        let graph = build_directed_component_graph(&catalog).unwrap();
        // The single catalog component appears as an isolated node
        // (universe pattern), but no edges exist.
        assert_eq!(graph.node_count(), 1);
        assert_eq!(graph.edge_count(), 0);
        assert!(graph.contains_node(&"A".to_string()));
        assert!(graph.neighbors(&"A".to_string()).is_empty());
        assert!(graph.reverse_neighbors(&"A".to_string()).is_empty());
    }

    #[test]
    fn directed_component_graph_loads_directed_edge_with_correct_directionality() {
        let tmp = TempDir::new().unwrap();
        let edges = [edge(EdgeKind::DependsOn, LifecycleScope::Build, "A", "B")];
        let reg = registry_with_repos(
            &tmp,
            &[("alpha", &[("A", "library"), ("B", "library")], &edges)],
        );
        let catalog = Catalog::load(&reg, SystemTime::now());
        let graph = build_directed_component_graph(&catalog).unwrap();
        assert_eq!(graph.node_count(), 2);
        assert_eq!(graph.edge_count(), 1);
        assert!(graph.contains_edge(&"A".to_string(), &"B".to_string()));
        assert!(
            !graph.contains_edge(&"B".to_string(), &"A".to_string()),
            "directed edge A→B must NOT imply B→A"
        );
        assert_eq!(graph.neighbors(&"A".to_string()), ["B".to_string()]);
        assert_eq!(graph.reverse_neighbors(&"B".to_string()), ["A".to_string()]);
    }

    #[test]
    fn directed_component_graph_skips_symmetric_edges() {
        // Symmetric kinds (co-implements, communicates-with) carry no
        // direction; including them would inflate `atlas scc` with
        // peer-relationship 2-cycles.
        let tmp = TempDir::new().unwrap();
        let edges = [
            edge(EdgeKind::CoImplements, LifecycleScope::Design, "A", "B"),
            edge(EdgeKind::CommunicatesWith, LifecycleScope::Runtime, "A", "C"),
        ];
        let reg = registry_with_repos(
            &tmp,
            &[(
                "alpha",
                &[("A", "library"), ("B", "library"), ("C", "library")],
                &edges,
            )],
        );
        let catalog = Catalog::load(&reg, SystemTime::now());
        let graph = build_directed_component_graph(&catalog).unwrap();
        assert_eq!(graph.node_count(), 3, "all catalog components are nodes");
        assert_eq!(graph.edge_count(), 0, "symmetric edges contribute no directed edges");
    }

    #[test]
    fn directed_component_graph_includes_isolated_catalog_components_as_nodes() {
        // C has no edges; it must still surface in the node universe
        // so SCC / path enumeration can reason about it.
        let tmp = TempDir::new().unwrap();
        let edges = [edge(EdgeKind::DependsOn, LifecycleScope::Build, "A", "B")];
        let reg = registry_with_repos(
            &tmp,
            &[(
                "alpha",
                &[("A", "library"), ("B", "library"), ("C", "library")],
                &edges,
            )],
        );
        let catalog = Catalog::load(&reg, SystemTime::now());
        let graph = build_directed_component_graph(&catalog).unwrap();
        assert!(graph.contains_node(&"C".to_string()));
        assert!(graph.neighbors(&"C".to_string()).is_empty());
        assert!(graph.reverse_neighbors(&"C".to_string()).is_empty());
    }

    #[test]
    fn directed_component_graph_unions_directed_edges_across_repos() {
        let tmp = TempDir::new().unwrap();
        let alpha_edges = [edge(EdgeKind::DependsOn, LifecycleScope::Build, "Alpha-A", "Alpha-B")];
        let beta_edges = [edge(EdgeKind::DependsOn, LifecycleScope::Build, "Beta-A", "Beta-B")];
        let reg = registry_with_repos(
            &tmp,
            &[
                ("alpha", &[("Alpha-A", "library"), ("Alpha-B", "library")], &alpha_edges),
                ("beta", &[("Beta-A", "library"), ("Beta-B", "library")], &beta_edges),
            ],
        );
        let catalog = Catalog::load(&reg, SystemTime::now());
        let graph = build_directed_component_graph(&catalog).unwrap();
        assert_eq!(graph.edge_count(), 2);
        assert!(graph.contains_edge(&"Alpha-A".to_string(), &"Alpha-B".to_string()));
        assert!(graph.contains_edge(&"Beta-A".to_string(), &"Beta-B".to_string()));
    }

    #[test]
    fn directed_component_graph_preserves_cycle_through_reverse_neighbors() {
        // A → B → A: every node has both an out and an in. SCC will
        // care about exactly this kind of structure.
        let tmp = TempDir::new().unwrap();
        let edges = [
            edge(EdgeKind::DependsOn, LifecycleScope::Build, "A", "B"),
            edge(EdgeKind::DependsOn, LifecycleScope::Build, "B", "A"),
        ];
        let reg = registry_with_repos(
            &tmp,
            &[("alpha", &[("A", "library"), ("B", "library")], &edges)],
        );
        let catalog = Catalog::load(&reg, SystemTime::now());
        let graph = build_directed_component_graph(&catalog).unwrap();
        assert_eq!(graph.edge_count(), 2);
        assert_eq!(graph.neighbors(&"A".to_string()), ["B".to_string()]);
        assert_eq!(graph.neighbors(&"B".to_string()), ["A".to_string()]);
        assert_eq!(graph.reverse_neighbors(&"A".to_string()), ["B".to_string()]);
        assert_eq!(graph.reverse_neighbors(&"B".to_string()), ["A".to_string()]);
    }

    #[test]
    fn directed_component_graph_propagates_load_failure_with_repo_context() {
        // Schema-version mismatch in one repo's related-components.yaml
        // bubbles up through `EdgeGraph::from_catalog` with the offending
        // repo identified — same diagnosis story as the EdgeGraph layer.
        let tmp = TempDir::new().unwrap();
        let reg = registry_with_one_fresh_repo(&tmp, "alpha", &[("A", "library")]);
        let path = tmp.path().join("alpha").join(ATLAS_DIR).join(RELATED_COMPONENTS_FILENAME);
        std::fs::write(&path, "schema_version: 1\nedges: []\n").unwrap();
        let catalog = Catalog::load(&reg, SystemTime::now());
        let err = build_directed_component_graph(&catalog).unwrap_err();
        let chain = format!("{err:#}");
        assert!(chain.contains("alpha"));
        assert!(chain.contains("schema_version"));
    }
}
