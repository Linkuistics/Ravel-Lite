//! LLM-driven discovery of cross-project relationships.
//!
//! Two-stage pipeline keyed from the per-context repo registry:
//! * Stage 1 (per-repo, cached): subagent reads the repo's working tree
//!   and emits a structured interaction-surface record.
//! * Stage 2 (global, uncached): one LLM call over all N surface records
//!   proposes edges, written to `<config-dir>/discover-proposals.yaml`
//!   for review.
//!
//! Edge vocabulary: `docs/component-ontology.md`.

use std::path::{Path, PathBuf};
use std::time::Duration;

use anyhow::{Context, Result};

use crate::bail_with;
use crate::cli::error_context::ResultExt;
use crate::cli::ErrorCode;

pub mod apply;
pub mod cache;
pub mod schema;
pub mod stage1;
pub mod stage2;
pub mod tree_sha;

use crate::config::{load_agent_config, load_shared_config};
use crate::init::require_embedded;
use crate::repos::{self, ReposRegistry};

use self::schema::{ProposalsFile, Stage1Failure, SurfaceFile};
use self::stage1::{run_stage1, Stage1Config, Stage1Outcome};
use self::stage2::{run_stage2, Stage2Config};

/// One discovery target: a repo slug paired with its working tree on
/// disk. The working tree is `RepoEntry.local_path` when present;
/// otherwise the deterministic context-cache path
/// `<context>/.cache/repos/<slug>/` (per architecture-next §"The repo
/// registry"). Stage 1 reads from this path.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DiscoverTarget {
    pub slug: String,
    pub working_tree: PathBuf,
}

/// Fallback subdirectory inside the context root when a repo entry has
/// no `local_path`. Worktree mounting is owned by a separate
/// architecture-next task; for now the path is constructed but not
/// auto-populated. Stage 1 will fail if the directory is absent — that
/// is the intended signal that the user must either supply
/// `--local-path` or wait for the worktree machinery.
pub const REPO_CACHE_SUBDIR: &str = ".cache/repos";

/// Compose the working-tree path for a repo entry. Pure path math; no
/// disk access.
pub fn working_tree_for(context_root: &Path, slug: &str, entry: &repos::RepoEntry) -> PathBuf {
    entry
        .local_path
        .clone()
        .unwrap_or_else(|| context_root.join(REPO_CACHE_SUBDIR).join(slug))
}

/// Materialise every catalogued repo as a `DiscoverTarget`.
fn targets_from(registry: &ReposRegistry, context_root: &Path) -> Vec<DiscoverTarget> {
    registry
        .repos
        .iter()
        .map(|(slug, entry)| DiscoverTarget {
            slug: slug.clone(),
            working_tree: working_tree_for(context_root, slug, entry),
        })
        .collect()
}

pub const PROPOSALS_FILE: &str = "discover-proposals.yaml";
pub const DEFAULT_CONCURRENCY: usize = 4;
pub const DEFAULT_DISCOVER_MODEL: &str = "claude-sonnet-4-6";

pub struct RunDiscoverOptions {
    pub project_filter: Option<String>,
    pub concurrency: Option<usize>,
    pub apply: bool,
}

pub async fn run_discover(config_root: &Path, options: RunDiscoverOptions) -> Result<()> {
    let registry = repos::load_for_lookup(config_root)?;
    if registry.repos.is_empty() {
        bail_with!(
            ErrorCode::NotFound,
            "repo registry is empty; nothing to discover"
        );
    }

    let all_targets = targets_from(&registry, config_root);
    let to_analyse: Vec<DiscoverTarget> = match &options.project_filter {
        Some(name) => {
            let target = all_targets
                .iter()
                .find(|t| t.slug == *name)
                .with_context(|| format!("repo '{name}' not in registry"))
                .with_code(ErrorCode::NotFound)?
                .clone();
            vec![target]
        }
        None => all_targets.clone(),
    };

    let shared = load_shared_config(config_root)?;
    let agent_config = load_agent_config(config_root, &shared.agent)?;
    let model = agent_config
        .models
        .get("discover")
        .cloned()
        .unwrap_or_else(|| DEFAULT_DISCOVER_MODEL.to_string());

    let stage1_prompt = require_embedded("discover-stage1.md")?.to_string();
    let stage2_prompt = require_embedded("discover-stage2.md")?.to_string();

    let concurrency = options.concurrency.unwrap_or(DEFAULT_CONCURRENCY).max(1);

    let stage1_cfg = Stage1Config {
        config_root: config_root.to_path_buf(),
        model: model.clone(),
        prompt_template: stage1_prompt,
        catalog_names: all_targets.iter().map(|t| t.slug.clone()).collect(),
        concurrency,
        timeout: Duration::from_secs(stage1::DEFAULT_STAGE1_TIMEOUT_SECS),
    };

    eprintln!(
        "Stage 1: analysing {} repo(s) with concurrency={}...",
        to_analyse.len(),
        concurrency
    );
    let outcomes = run_stage1(&to_analyse, &stage1_cfg).await?;

    // Collect surfaces for Stage 2. For a `--project` filter, fill in
    // the other catalogued projects from their cache so Stage 2 still
    // has the full set. Projects with no cache yet are skipped from
    // Stage 2 and recorded as "not yet analysed" failures.
    let mut surfaces: Vec<SurfaceFile> = Vec::new();
    let mut failures: Vec<Stage1Failure> = Vec::new();
    let mut any_fresh_surface = false;
    for outcome in outcomes {
        match outcome {
            Stage1Outcome::Fresh(s) => {
                any_fresh_surface = true;
                surfaces.push(s);
            }
            Stage1Outcome::Cached(s) => surfaces.push(s),
            Stage1Outcome::Failed(f) => {
                eprintln!("  Stage 1 FAILED  {}: {}", f.project, f.error);
                failures.push(f);
            }
        }
    }
    if options.project_filter.is_some() {
        for target in &all_targets {
            if surfaces.iter().any(|s| s.project == target.slug) {
                continue;
            }
            if failures.iter().any(|f| f.project == target.slug) {
                continue;
            }
            match cache::load(config_root, &target.slug)? {
                Some(cached) => surfaces.push(cached),
                None => failures.push(Stage1Failure {
                    project: target.slug.clone(),
                    error: "not yet analysed; run discover without --project to populate".to_string(),
                }),
            }
        }
    }

    // When every surface was cached and nothing failed this run, Stage 2's
    // input is byte-identical to last time — re-running it would regenerate
    // proposals that (modulo LLM noise) already exist in discover-proposals.yaml,
    // wastefully spending a claude call AND clobbering any manual edits the
    // user made to the proposals file. Preserve the existing file instead.
    let proposals_already_exist = proposals_path(config_root).exists();
    let skip_stage2_reuse_existing =
        !any_fresh_surface && failures.is_empty() && proposals_already_exist && !surfaces.is_empty();

    let proposals = if skip_stage2_reuse_existing {
        eprintln!(
            "Stage 2: skipped — all {} surface(s) served from cache; preserving existing {}",
            surfaces.len(),
            PROPOSALS_FILE
        );
        load_proposals(config_root)?
    } else if surfaces.is_empty() {
        // Skip Stage 2 entirely — asking the LLM to infer edges from
        // zero surfaces is meaningless and the spawned claude has no
        // useful work to do. Persist the failures so the user can act
        // on them, and let the caller bail at the end.
        eprintln!("Stage 2: skipped — no surfaces produced (all Stage 1 attempts failed)");
        let proposals = ProposalsFile {
            schema_version: schema::PROPOSALS_SCHEMA_VERSION,
            generated_at: stage1::current_utc_rfc3339(),
            source_project_states: Default::default(),
            proposals: Vec::new(),
            failures,
        };
        save_proposals_atomic(config_root, &proposals)?;
        proposals
    } else {
        eprintln!(
            "Stage 2: inferring edges over {} surface(s)...",
            surfaces.len()
        );
        let stage2_cfg = Stage2Config {
            config_root: config_root.to_path_buf(),
            model,
            prompt_template: stage2_prompt,
            timeout: Duration::from_secs(stage2::DEFAULT_STAGE2_TIMEOUT_SECS),
        };
        // `run_stage2` pre-initialises discover-proposals.yaml and
        // returns the file as it stands after the LLM's CLI-driven
        // appends; the file on disk is already the authority.
        run_stage2(&surfaces, failures, &stage2_cfg).await?
    };

    eprintln!(
        "{}: {} proposal(s), {} failure(s)",
        proposals_path(config_root).display(),
        proposals.proposals.len(),
        proposals.failures.len(),
    );

    if options.apply {
        apply::run_discover_apply(config_root)?;
    }

    // Bail on Stage 1 failures from THIS run — not on stale failures
    // preserved from a prior run in the skipped-Stage-2 path. The skip
    // branch only fires when `failures.is_empty()` for the current run,
    // so the loaded proposals' failure list (whatever it contains) is
    // from history and shouldn't block this run's exit status.
    if !skip_stage2_reuse_existing && !proposals.failures.is_empty() {
        bail_with!(
            ErrorCode::Conflict,
            "discover completed with Stage 1 failures — see the failures section of the proposals file"
        );
    }
    Ok(())
}

pub fn proposals_path(config_root: &Path) -> PathBuf {
    config_root.join(PROPOSALS_FILE)
}

pub fn save_proposals_atomic(config_root: &Path, file: &ProposalsFile) -> Result<()> {
    let path = proposals_path(config_root);
    let tmp = config_root.join(format!(".{PROPOSALS_FILE}.tmp"));
    let yaml = serde_yaml::to_string(file)
        .context("serialise ProposalsFile")
        .with_code(ErrorCode::Internal)?;
    std::fs::write(&tmp, yaml.as_bytes())?;
    std::fs::rename(&tmp, &path)?;
    Ok(())
}

pub fn load_proposals(config_root: &Path) -> Result<ProposalsFile> {
    let path = proposals_path(config_root);
    let content = std::fs::read_to_string(&path)
        .with_context(|| format!("failed to read {}", path.display()))
        .with_code(ErrorCode::IoError)?;
    let file: ProposalsFile = serde_yaml::from_str(&content)
        .with_context(|| format!("failed to parse {}", path.display()))
        .with_code(ErrorCode::InvalidInput)?;
    if file.schema_version != schema::PROPOSALS_SCHEMA_VERSION {
        bail_with!(
            ErrorCode::Conflict,
            "{} has schema_version {} but this ravel-lite expects {}",
            path.display(),
            file.schema_version,
            schema::PROPOSALS_SCHEMA_VERSION
        );
    }
    Ok(file)
}

#[cfg(test)]
mod tests {
    use super::*;
    use super::schema::*;
    use tempfile::TempDir;

    #[test]
    fn save_then_load_proposals_round_trips() {
        let tmp = TempDir::new().unwrap();
        let file = ProposalsFile {
            schema_version: PROPOSALS_SCHEMA_VERSION,
            generated_at: "2026-04-22T00:00:00Z".to_string(),
            source_project_states: [(
                "A".to_string(),
                super::tree_sha::ProjectState {
                    tree_sha: "abc".to_string(),
                    dirty_hash: "dirty-a".to_string(),
                },
            )]
            .into_iter()
            .collect(),
            proposals: vec![],
            failures: vec![Stage1Failure {
                project: "B".to_string(),
                error: "oops".to_string(),
            }],
        };
        save_proposals_atomic(tmp.path(), &file).unwrap();
        let loaded = load_proposals(tmp.path()).unwrap();
        assert_eq!(loaded, file);
    }
}
