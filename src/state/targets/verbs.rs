//! Handlers for every `state targets <verb>` CLI verb.
//!
//! Targets are runtime mount records, not TMS items, so the verb
//! surface differs from `state intents`/`memory`/`backlog`:
//!
//! - No `set-status`/`set-body`: a target is either mounted or not;
//!   there is no lifecycle status.
//! - `add` requires explicit mount metadata (`--working-root`,
//!   `--branch`, `--path-segment`); the worktree-mounting machinery
//!   that derives those values from a ComponentRef and a default
//!   branch is a separate task.
//! - Identity is the `(repo_slug, component_id)` pair, addressed via a
//!   single positional `<repo>:<component>` (matching the
//!   `target-requests.yaml` notation in
//!   `docs/architecture-next.md` §Dynamic mounting).

use std::path::Path;

use anyhow::Result;

use crate::bail_with;
use crate::cli::{CodedError, ErrorCode, OutputFormat};

use super::schema::{Target, TargetsFile, TARGETS_SCHEMA_VERSION};
use super::yaml_io::{read_targets, write_targets};

pub fn run_list(plan_dir: &Path, format: OutputFormat) -> Result<()> {
    let targets = read_targets(plan_dir)?;
    emit(&targets, format)
}

pub fn run_show(plan_dir: &Path, reference: &str, format: OutputFormat) -> Result<()> {
    let (repo_slug, component_id) = parse_reference(reference)?;
    let targets = read_targets(plan_dir)?;
    let entry = find_target(&targets, &repo_slug, &component_id)?;
    let wrapper = TargetsFile {
        schema_version: TARGETS_SCHEMA_VERSION,
        targets: vec![entry.clone()],
    };
    emit(&wrapper, format)
}

#[derive(Debug, Clone)]
pub struct AddRequest {
    pub repo_slug: String,
    pub component_id: String,
    pub working_root: String,
    pub branch: String,
    pub path_segments: Vec<String>,
}

pub fn run_add(plan_dir: &Path, req: &AddRequest) -> Result<()> {
    if req.repo_slug.is_empty() {
        bail_with!(ErrorCode::InvalidInput, "--repo must be non-empty");
    }
    if req.component_id.is_empty() {
        bail_with!(ErrorCode::InvalidInput, "--component must be non-empty");
    }
    if req.working_root.is_empty() {
        bail_with!(ErrorCode::InvalidInput, "--working-root must be non-empty");
    }
    if req.branch.is_empty() {
        bail_with!(ErrorCode::InvalidInput, "--branch must be non-empty");
    }
    let mut targets = read_targets(plan_dir)?;
    if find_target(&targets, &req.repo_slug, &req.component_id).is_ok() {
        bail_with!(
            ErrorCode::Conflict,
            "target {}:{} already mounted",
            req.repo_slug,
            req.component_id
        );
    }
    targets.targets.push(Target {
        repo_slug: req.repo_slug.clone(),
        component_id: req.component_id.clone(),
        working_root: req.working_root.clone(),
        branch: req.branch.clone(),
        path_segments: req.path_segments.clone(),
    });
    write_targets(plan_dir, &targets)
}

pub fn run_remove(plan_dir: &Path, reference: &str) -> Result<()> {
    let (repo_slug, component_id) = parse_reference(reference)?;
    let mut targets = read_targets(plan_dir)?;
    let before = targets.targets.len();
    targets
        .targets
        .retain(|t| !(t.repo_slug == repo_slug && t.component_id == component_id));
    if targets.targets.len() == before {
        bail_with!(
            ErrorCode::NotFound,
            "no target {repo_slug}:{component_id} to remove"
        );
    }
    write_targets(plan_dir, &targets)
}

pub(crate) fn parse_reference(reference: &str) -> Result<(String, String)> {
    match reference.split_once(':') {
        Some((repo, component)) if !repo.is_empty() && !component.is_empty() => {
            Ok((repo.to_string(), component.to_string()))
        }
        _ => bail_with!(
            ErrorCode::InvalidInput,
            "target reference {reference:?} must be `<repo_slug>:<component_id>` with both parts non-empty"
        ),
    }
}

pub(crate) fn find_target<'a>(
    targets: &'a TargetsFile,
    repo_slug: &str,
    component_id: &str,
) -> Result<&'a Target> {
    targets
        .targets
        .iter()
        .find(|t| t.repo_slug == repo_slug && t.component_id == component_id)
        .ok_or_else(|| anyhow::Error::new(CodedError {
            code: ErrorCode::NotFound,
            message: format!("no target {repo_slug}:{component_id}"),
        }))
}

fn emit(targets: &TargetsFile, format: OutputFormat) -> Result<()> {
    let serialised = match format {
        OutputFormat::Yaml => serde_yaml::to_string(targets)?,
        OutputFormat::Json => serde_json::to_string_pretty(targets)? + "\n",
        OutputFormat::Markdown => {
            bail_with!(
                ErrorCode::InvalidInput,
                "`state targets` does not support --format markdown; use yaml or json"
            )
        }
    };
    print!("{serialised}");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn sample_request(repo: &str, component: &str) -> AddRequest {
        AddRequest {
            repo_slug: repo.into(),
            component_id: component.into(),
            working_root: format!(".worktrees/{repo}"),
            branch: "ravel-lite/test-plan/main".into(),
            path_segments: vec!["crates".into(), component.into()],
        }
    }

    #[test]
    fn parse_reference_splits_on_first_colon() {
        let (repo, component) = parse_reference("atlas:atlas-ontology").unwrap();
        assert_eq!(repo, "atlas");
        assert_eq!(component, "atlas-ontology");
    }

    #[test]
    fn parse_reference_rejects_missing_colon() {
        let err = parse_reference("atlas-only").unwrap_err();
        let msg = format!("{err:#}");
        assert!(msg.contains("repo_slug"), "error must explain expected shape: {msg}");
    }

    #[test]
    fn parse_reference_rejects_empty_repo() {
        let err = parse_reference(":only-component").unwrap_err();
        let msg = format!("{err:#}");
        assert!(msg.contains("non-empty"), "error must mention non-empty: {msg}");
    }

    #[test]
    fn parse_reference_rejects_empty_component() {
        let err = parse_reference("only-repo:").unwrap_err();
        let msg = format!("{err:#}");
        assert!(msg.contains("non-empty"), "error must mention non-empty: {msg}");
    }

    #[test]
    fn run_add_appends_target_to_empty_file() {
        let tmp = TempDir::new().unwrap();
        run_add(tmp.path(), &sample_request("atlas", "atlas-ontology")).unwrap();

        let updated = read_targets(tmp.path()).unwrap();
        assert_eq!(updated.targets.len(), 1);
        assert_eq!(updated.targets[0].repo_slug, "atlas");
        assert_eq!(updated.targets[0].component_id, "atlas-ontology");
    }

    #[test]
    fn run_add_rejects_duplicate_repo_component_pair() {
        let tmp = TempDir::new().unwrap();
        run_add(tmp.path(), &sample_request("atlas", "atlas-ontology")).unwrap();

        let err = run_add(tmp.path(), &sample_request("atlas", "atlas-ontology")).unwrap_err();
        let msg = format!("{err:#}");
        assert!(msg.contains("already mounted"), "error must say already mounted: {msg}");
    }

    #[test]
    fn run_add_allows_two_components_in_the_same_repo() {
        // Multi-component-per-worktree per architecture-next: two
        // distinct components in the same repo share a worktree but
        // each gets its own `Target` row.
        let tmp = TempDir::new().unwrap();
        run_add(tmp.path(), &sample_request("atlas", "atlas-ontology")).unwrap();
        run_add(tmp.path(), &sample_request("atlas", "atlas-discovery")).unwrap();

        let updated = read_targets(tmp.path()).unwrap();
        assert_eq!(updated.targets.len(), 2);
        assert_eq!(updated.targets[0].working_root, updated.targets[1].working_root);
    }

    #[test]
    fn run_add_rejects_empty_required_fields() {
        let tmp = TempDir::new().unwrap();
        let mut req = sample_request("atlas", "ontology");
        req.repo_slug = String::new();
        let err = run_add(tmp.path(), &req).unwrap_err();
        assert!(format!("{err:#}").contains("--repo"));

        let mut req = sample_request("atlas", "ontology");
        req.component_id = String::new();
        let err = run_add(tmp.path(), &req).unwrap_err();
        assert!(format!("{err:#}").contains("--component"));

        let mut req = sample_request("atlas", "ontology");
        req.working_root = String::new();
        let err = run_add(tmp.path(), &req).unwrap_err();
        assert!(format!("{err:#}").contains("--working-root"));

        let mut req = sample_request("atlas", "ontology");
        req.branch = String::new();
        let err = run_add(tmp.path(), &req).unwrap_err();
        assert!(format!("{err:#}").contains("--branch"));
    }

    #[test]
    fn run_remove_drops_only_the_named_target() {
        let tmp = TempDir::new().unwrap();
        run_add(tmp.path(), &sample_request("atlas", "ontology")).unwrap();
        run_add(tmp.path(), &sample_request("sidekick", "router")).unwrap();

        run_remove(tmp.path(), "atlas:ontology").unwrap();

        let updated = read_targets(tmp.path()).unwrap();
        assert_eq!(updated.targets.len(), 1);
        assert_eq!(updated.targets[0].repo_slug, "sidekick");
    }

    #[test]
    fn run_remove_errors_when_target_not_present() {
        let tmp = TempDir::new().unwrap();
        run_add(tmp.path(), &sample_request("atlas", "ontology")).unwrap();

        let err = run_remove(tmp.path(), "missing:thing").unwrap_err();
        let msg = format!("{err:#}");
        assert!(msg.contains("missing:thing"), "error must cite the bad ref: {msg}");
    }

    #[test]
    fn find_target_returns_match() {
        let targets = TargetsFile {
            schema_version: TARGETS_SCHEMA_VERSION,
            targets: vec![Target {
                repo_slug: "atlas".into(),
                component_id: "ontology".into(),
                working_root: ".worktrees/atlas".into(),
                branch: "ravel-lite/p/main".into(),
                path_segments: vec![],
            }],
        };
        let found = find_target(&targets, "atlas", "ontology").unwrap();
        assert_eq!(found.repo_slug, "atlas");
    }
}
