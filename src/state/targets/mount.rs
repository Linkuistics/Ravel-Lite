//! Worktree mounting: shell-out side of `targets.yaml`.
//!
//! `mount_target` projects a `(repo_slug, component_id)` reference into
//! a plan as a mounted git worktree:
//!
//! 1. Resolves `repo_slug` against `<context>/repos.yaml` to find the
//!    source repo's `local_path`.
//! 2. Creates `<plan>/.worktrees/<repo_slug>/` on the plan-namespaced
//!    branch `ravel-lite/<plan>/main` via `git worktree add`. The
//!    starting point is the source repo's default branch
//!    (`origin/HEAD` if a remote is set, else local `HEAD` — works for
//!    fresh test fixtures).
//! 3. Resolves the component's `path_segments` from the source repo's
//!    `.atlas/components.yaml`.
//! 4. Persists the resulting `Target` row via `run_add`.
//!
//! Idempotent: a second mount of the same `(repo, component)` pair is a
//! no-op. Multiple components in the same repo share a single worktree;
//! the second component just appends a `Target` row.
//!
//! Reference: `docs/architecture-next.md` §Targets and worktrees.

use std::path::Path;
use std::process::Command;

use anyhow::{Context, Result};

use crate::bail_with;
use crate::cli::error_context::ResultExt;
use crate::cli::{CodedError, ErrorCode};

fn coded(code: ErrorCode, message: String) -> anyhow::Error {
    anyhow::Error::new(CodedError { code, message })
}

use atlas_index::{load_or_default_components, ComponentsFile};

use super::schema::Target;
use super::verbs::find_target;
use super::yaml_io::{read_targets, write_targets};
use crate::repos::load_for_lookup;

const WORKTREES_DIRNAME: &str = ".worktrees";
const ATLAS_COMPONENTS_REL: &str = ".atlas/components.yaml";

/// Mount the named component as a git worktree under
/// `<plan>/.worktrees/<repo_slug>/`. Idempotent. See module docs.
pub fn mount_target(
    plan_dir: &Path,
    context_root: &Path,
    repo_slug: &str,
    component_id: &str,
) -> Result<Target> {
    let registry = load_for_lookup(context_root)?;
    let repo_entry = registry.get(repo_slug).ok_or_else(|| {
        coded(
            ErrorCode::NotFound,
            format!(
                "repo slug {repo_slug:?} is not registered in {}/repos.yaml. \
                 Add it with `ravel-lite repo add {repo_slug} --url <url> --local-path <path>`.",
                context_root.display()
            ),
        )
    })?;
    let local_path = repo_entry.local_path.as_deref().ok_or_else(|| {
        coded(
            ErrorCode::InvalidInput,
            format!(
                "repo slug {repo_slug:?} has no local_path in {}/repos.yaml. \
                 Re-add it with `--local-path <path>`; clone-on-demand is not yet implemented.",
                context_root.display()
            ),
        )
    })?;

    let plan_name = plan_name_from_dir(plan_dir)?;
    let working_root_rel = format!("{WORKTREES_DIRNAME}/{repo_slug}");
    let working_root_abs = plan_dir.join(&working_root_rel);
    let branch = format!("ravel-lite/{plan_name}/main");

    if working_root_abs.exists() {
        verify_worktree_branch(&working_root_abs, &branch)?;
    } else {
        let starting_point = detect_default_branch_starting_point(local_path)?;
        ensure_worktrees_parent_exists(plan_dir)?;
        run_git_worktree_add(local_path, &working_root_abs, &branch, &starting_point)?;
    }

    let path_segments = resolve_path_segments(local_path, repo_slug, component_id)?;

    let mut targets = read_targets(plan_dir)?;
    if let Ok(existing) = find_target(&targets, repo_slug, component_id) {
        return Ok(existing.clone());
    }
    let target = Target {
        repo_slug: repo_slug.to_string(),
        component_id: component_id.to_string(),
        working_root: working_root_rel,
        branch,
        path_segments,
    };
    targets.targets.push(target.clone());
    write_targets(plan_dir, &targets)?;
    Ok(target)
}

fn plan_name_from_dir(plan_dir: &Path) -> Result<String> {
    plan_dir
        .file_name()
        .and_then(|n| n.to_str())
        .map(|s| s.to_string())
        .ok_or_else(|| {
            coded(
                ErrorCode::InvalidInput,
                format!(
                    "plan directory {} has no usable file name; cannot derive plan-namespaced branch",
                    plan_dir.display()
                ),
            )
        })
}

fn ensure_worktrees_parent_exists(plan_dir: &Path) -> Result<()> {
    let parent = plan_dir.join(WORKTREES_DIRNAME);
    if !parent.exists() {
        std::fs::create_dir_all(&parent)
            .with_context(|| format!("failed to create {}", parent.display()))
            .with_code(ErrorCode::IoError)?;
    }
    Ok(())
}

/// Pick a starting-point ref for `git worktree add`. Returns
/// `origin/<default>` when an `origin/HEAD` symbolic ref is configured,
/// otherwise falls back to local `HEAD`. The fallback is what makes
/// fresh test fixtures (no remote) work; real-world checkouts have
/// `origin/HEAD` from clone time.
fn detect_default_branch_starting_point(repo: &Path) -> Result<String> {
    let origin_head = run_git(
        repo,
        &["symbolic-ref", "--short", "refs/remotes/origin/HEAD"],
    )?;
    if origin_head.status.success() {
        let trimmed = String::from_utf8_lossy(&origin_head.stdout).trim().to_string();
        if !trimmed.is_empty() {
            return Ok(trimmed);
        }
    }
    Ok("HEAD".to_string())
}

fn run_git_worktree_add(
    repo: &Path,
    working_root: &Path,
    branch: &str,
    starting_point: &str,
) -> Result<()> {
    let working_root_str = working_root.to_str().ok_or_else(|| {
        coded(
            ErrorCode::InvalidInput,
            format!(
                "worktree path {} contains non-UTF-8 characters",
                working_root.display()
            ),
        )
    })?;
    let output = run_git(
        repo,
        &[
            "worktree",
            "add",
            "-b",
            branch,
            working_root_str,
            starting_point,
        ],
    )?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        bail_with!(
            ErrorCode::Conflict,
            "git worktree add failed in {}: {}",
            repo.display(),
            stderr.trim()
        );
    }
    Ok(())
}

/// Verify that an existing path is a worktree on the expected branch.
/// Errors when it is not — either the directory is not a worktree at
/// all, or it is on a different branch than the plan expects. Both
/// cases mean the user must intervene; idempotent re-mount only covers
/// the "same branch already there" case.
fn verify_worktree_branch(working_root: &Path, expected_branch: &str) -> Result<()> {
    let output = run_git(working_root, &["symbolic-ref", "--short", "HEAD"])?;
    if !output.status.success() {
        bail_with!(
            ErrorCode::Conflict,
            "{} exists but is not a git worktree (or HEAD is detached). \
             Remove it manually and retry the mount.",
            working_root.display()
        );
    }
    let actual = String::from_utf8_lossy(&output.stdout).trim().to_string();
    if actual != expected_branch {
        bail_with!(
            ErrorCode::Conflict,
            "{} is on branch {actual:?} but the mount expected {expected_branch:?}. \
             Remove or rename it manually before retrying.",
            working_root.display()
        );
    }
    Ok(())
}

fn resolve_path_segments(
    local_path: &Path,
    repo_slug: &str,
    component_id: &str,
) -> Result<Vec<String>> {
    let components_path = local_path.join(ATLAS_COMPONENTS_REL);
    if !components_path.exists() {
        bail_with!(
            ErrorCode::NotFound,
            "{} not found. Run `atlas index {}` and retry.",
            components_path.display(),
            local_path.display()
        );
    }
    let file: ComponentsFile = load_or_default_components(&components_path)
        .with_context(|| format!("failed to load {}", components_path.display()))
        .with_code(ErrorCode::IoError)?;
    let entry = file.components.iter().find(|c| c.id == component_id).ok_or_else(|| {
        coded(
            ErrorCode::NotFound,
            format!(
                "component {repo_slug}:{component_id} not found in {}. \
                 Either the id is wrong (check `ravel-lite atlas list-components --repo {repo_slug} --format yaml` for the bare `id:` values) \
                 or the index is stale (re-run `atlas index {}`).",
                components_path.display(),
                local_path.display()
            ),
        )
    })?;
    Ok(entry
        .path_segments
        .iter()
        .map(|seg| seg.path.to_string_lossy().to_string())
        .collect())
}

fn run_git(cwd: &Path, args: &[&str]) -> Result<std::process::Output> {
    Command::new("git")
        .current_dir(cwd)
        .args(args)
        .output()
        .with_context(|| {
            format!(
                "failed to invoke `git {}` in {}",
                args.join(" "),
                cwd.display()
            )
        })
        .with_code(ErrorCode::IoError)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::repos;
    use std::fs;
    use std::path::PathBuf;
    use tempfile::TempDir;

    /// Build a self-contained fixture: a context root with `repos.yaml`
    /// pointing at a real (single-commit) source repo whose
    /// `.atlas/components.yaml` lists one component. Returns the
    /// `(tmp, plan_dir, context_root, source_repo)` quadruple — keep
    /// `tmp` alive for the duration of the test.
    fn fixture() -> (TempDir, PathBuf, PathBuf, PathBuf) {
        let tmp = TempDir::new().unwrap();
        let source = tmp.path().join("source");
        fs::create_dir_all(&source).unwrap();
        run_git_or_panic(&source, &["init", "--initial-branch=main"]);
        run_git_or_panic(&source, &["config", "user.email", "test@example"]);
        run_git_or_panic(&source, &["config", "user.name", "test"]);
        fs::write(source.join("README.md"), "src\n").unwrap();
        run_git_or_panic(&source, &["add", "."]);
        run_git_or_panic(&source, &["commit", "-m", "init"]);

        write_components_yaml(
            &source,
            &[("atlas-ontology", &["crates/atlas-ontology"])],
        );

        let context = tmp.path().join("context");
        fs::create_dir_all(&context).unwrap();
        repos::run_add(&context, "atlas", "git@example/atlas.git", Some(&source)).unwrap();

        let plan = context.join("plans").join("test-plan");
        fs::create_dir_all(&plan).unwrap();

        (tmp, plan, context, source)
    }

    fn write_components_yaml(repo: &Path, components: &[(&str, &[&str])]) {
        let atlas_dir = repo.join(".atlas");
        fs::create_dir_all(&atlas_dir).unwrap();
        let mut comps = String::new();
        for (id, segments) in components {
            comps.push_str(&format!("  - id: {id}\n"));
            comps.push_str("    kind: rust-library\n");
            comps.push_str("    evidence_grade: strong\n");
            comps.push_str("    rationale: fixture\n");
            comps.push_str("    path_segments:\n");
            for path in *segments {
                comps.push_str(&format!("      - path: {path}\n"));
                comps.push_str("        content_sha: 'fixture'\n");
            }
        }
        let yaml = format!(
            "schema_version: 1\nroot: {root}\ngenerated_at: '2026-04-24T00:00:00Z'\n\
             cache_fingerprints:\n  ontology_sha: ''\n  prompt_shas: {{}}\n  \
             model_id: ''\n  backend_version: ''\ncomponents:\n{comps}",
            root = repo.display()
        );
        fs::write(atlas_dir.join("components.yaml"), yaml).unwrap();
    }

    fn run_git_or_panic(cwd: &Path, args: &[&str]) {
        let out = Command::new("git").current_dir(cwd).args(args).output().unwrap();
        assert!(
            out.status.success(),
            "git {} failed in {}: {}",
            args.join(" "),
            cwd.display(),
            String::from_utf8_lossy(&out.stderr)
        );
    }

    #[test]
    fn mount_target_errors_when_repo_slug_unknown() {
        // `repos.yaml` does not list `atlas`; the error must name both
        // the missing slug and point at the registry path so the user
        // can act.
        let tmp = TempDir::new().unwrap();
        let context = tmp.path().join("context");
        fs::create_dir_all(&context).unwrap();
        let plan = context.join("plans").join("test-plan");
        fs::create_dir_all(&plan).unwrap();

        let err = mount_target(&plan, &context, "atlas", "atlas-ontology").unwrap_err();
        let msg = format!("{err:#}");
        assert!(msg.contains("atlas"), "error must cite the slug: {msg}");
        assert!(msg.contains("repo add"), "error must show migration command: {msg}");
    }

    #[test]
    fn mount_target_errors_when_repo_has_no_local_path() {
        // Repo registered without `--local-path`. Until clone-on-demand
        // lands, mounting must fail with an actionable message.
        let tmp = TempDir::new().unwrap();
        let context = tmp.path().join("context");
        fs::create_dir_all(&context).unwrap();
        repos::run_add(&context, "atlas", "git@example/atlas.git", None).unwrap();
        let plan = context.join("plans").join("test-plan");
        fs::create_dir_all(&plan).unwrap();

        let err = mount_target(&plan, &context, "atlas", "atlas-ontology").unwrap_err();
        let msg = format!("{err:#}");
        assert!(msg.contains("local_path"), "error must mention local_path: {msg}");
    }

    #[test]
    fn mount_target_attaches_second_component_to_existing_worktree() {
        // Two components in the same repo share a single worktree. The
        // second mount must not invoke `git worktree add` again (which
        // would fail with "already a working tree"), and both Target
        // rows must share the same `working_root`.
        let tmp = TempDir::new().unwrap();
        let source = tmp.path().join("source");
        fs::create_dir_all(&source).unwrap();
        run_git_or_panic(&source, &["init", "--initial-branch=main"]);
        run_git_or_panic(&source, &["config", "user.email", "test@example"]);
        run_git_or_panic(&source, &["config", "user.name", "test"]);
        fs::write(source.join("README.md"), "src\n").unwrap();
        run_git_or_panic(&source, &["add", "."]);
        run_git_or_panic(&source, &["commit", "-m", "init"]);
        write_components_yaml(
            &source,
            &[
                ("atlas-ontology", &["crates/atlas-ontology"]),
                ("atlas-discovery", &["crates/atlas-discovery"]),
            ],
        );
        let context = tmp.path().join("context");
        fs::create_dir_all(&context).unwrap();
        repos::run_add(&context, "atlas", "url", Some(&source)).unwrap();
        let plan = context.join("plans").join("test-plan");
        fs::create_dir_all(&plan).unwrap();

        let first = mount_target(&plan, &context, "atlas", "atlas-ontology").unwrap();
        let second = mount_target(&plan, &context, "atlas", "atlas-discovery").unwrap();

        assert_eq!(first.working_root, second.working_root);
        assert_eq!(first.branch, second.branch);

        let on_disk = read_targets(&plan).unwrap();
        assert_eq!(on_disk.targets.len(), 2);
        assert_eq!(
            on_disk.targets[1].path_segments,
            vec!["crates/atlas-discovery".to_string()]
        );
    }

    #[test]
    fn mount_target_is_idempotent_on_repeat_with_same_args() {
        // Two mounts of the same `(repo, component)` reference must not
        // error and must leave exactly one row in `targets.yaml`. The
        // second call returns the existing row unchanged.
        let (_tmp, plan, context, _source) = fixture();

        let first = mount_target(&plan, &context, "atlas", "atlas-ontology").unwrap();
        let second = mount_target(&plan, &context, "atlas", "atlas-ontology").unwrap();
        assert_eq!(first, second);

        let on_disk = read_targets(&plan).unwrap();
        assert_eq!(on_disk.targets.len(), 1, "second mount must not duplicate the row");
    }

    #[test]
    fn mount_target_creates_worktree_and_writes_targets_yaml_row() {
        let (_tmp, plan, context, _source) = fixture();

        let target = mount_target(&plan, &context, "atlas", "atlas-ontology").unwrap();

        assert_eq!(target.repo_slug, "atlas");
        assert_eq!(target.component_id, "atlas-ontology");
        assert_eq!(target.working_root, ".worktrees/atlas");
        assert_eq!(target.branch, "ravel-lite/test-plan/main");
        assert_eq!(target.path_segments, vec!["crates/atlas-ontology".to_string()]);

        // The git worktree exists at the expected path on the expected branch.
        let worktree = plan.join(".worktrees/atlas");
        assert!(worktree.is_dir(), "worktree directory must exist after mount");
        let head = Command::new("git")
            .current_dir(&worktree)
            .args(["symbolic-ref", "--short", "HEAD"])
            .output()
            .unwrap();
        assert!(head.status.success(), "worktree HEAD must be a symbolic ref");
        assert_eq!(
            String::from_utf8_lossy(&head.stdout).trim(),
            "ravel-lite/test-plan/main"
        );

        // The targets.yaml row was persisted.
        let on_disk = read_targets(&plan).unwrap();
        assert_eq!(on_disk.targets.len(), 1);
        assert_eq!(on_disk.targets[0].component_id, "atlas-ontology");
    }
}
