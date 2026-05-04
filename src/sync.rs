//! `ravel-lite sync <plan> --from <other-plan>`: pull commits from one
//! plan's per-target branches into another plan's worktrees on its own
//! per-target branches.
//!
//! Two plans in the same ravel context can target overlapping
//! components. Each plan owns its own `ravel-lite/<plan>/main` branch
//! per touched repo. Plan branches do not auto-rebase against `main`;
//! they also do not auto-share work between plans. Sync is the explicit
//! mechanism a solo dev uses to opt into cross-plan visibility — for
//! every shared target, run `git merge <other-plan-branch>` in the
//! destination plan's worktree on the destination plan's branch.
//!
//! Targets unique to `<other-plan>` are mounted into `<plan>` first,
//! reusing the worktree-mount path. Then every shared target is
//! attempted; conflicts are reported per-target and left in the
//! worktree for the user to resolve manually. A clean run reports
//! mounts and merges; a partially-conflicted run reports both clean
//! merges and conflicts.
//!
//! See `docs/architecture-next.md` §`ravel-lite sync` and
//! §"What this replaces or removes" (sync replaces the old "direct
//! mode" target types).

use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{Context, Result};

use crate::bail_with;
use crate::cli::error_context::ResultExt;
use crate::cli::{CodedError, ErrorCode};
use crate::state::targets::{mount_target, read_targets, Target};

/// Per-target outcome of a single `git merge` attempt during sync.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MergeOutcome {
    pub repo_slug: String,
    pub component_id: String,
    pub working_root: PathBuf,
    pub kind: MergeKind,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MergeKind {
    /// `git merge` exited zero. Either fast-forward or true merge — both
    /// captured the same way; the audit trail is in the destination
    /// branch's commit graph.
    Clean,
    /// `git merge` exited non-zero with conflict markers in the index.
    /// The worktree is left dirty for the user to resolve.
    Conflict { stderr: String },
    /// Source and destination already point at the same commit. No
    /// merge was attempted (would be a no-op).
    AlreadyUpToDate,
}

/// Per-target outcome of a mount that ran because `<other>` carried a
/// component `<plan>` did not.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MountOutcome {
    pub repo_slug: String,
    pub component_id: String,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SyncReport {
    pub mounted: Vec<MountOutcome>,
    pub merges: Vec<MergeOutcome>,
}

impl SyncReport {
    pub fn has_conflicts(&self) -> bool {
        self.merges.iter().any(|m| matches!(m.kind, MergeKind::Conflict { .. }))
    }
}

/// Top-level entry point. `plan_dir` is the destination plan (its
/// worktrees and branches receive the merge); `other_plan_dir` is the
/// source plan whose per-target branches are merged in.
pub fn run_sync(plan_dir: &Path, other_plan_dir: &Path) -> Result<SyncReport> {
    if plan_dir == other_plan_dir {
        bail_with!(
            ErrorCode::InvalidInput,
            "destination plan and --from plan are the same directory ({}); \
             sync is for cross-plan visibility, not self-merge",
            plan_dir.display()
        );
    }

    let plan_name = plan_basename(plan_dir)?;
    let other_plan_name = plan_basename(other_plan_dir)?;
    let context_root = derive_context_root(plan_dir)?;
    let other_context_root = derive_context_root(other_plan_dir)?;
    if context_root != other_context_root {
        bail_with!(
            ErrorCode::InvalidInput,
            "destination plan and --from plan live in different ravel contexts \
             ({} vs {}); cross-context sync is not supported",
            context_root.display(),
            other_context_root.display()
        );
    }

    let plan_targets = read_targets(plan_dir)?.targets;
    let other_targets = read_targets(other_plan_dir)?.targets;

    let mut report = SyncReport::default();

    for unique in unique_to_other(&plan_targets, &other_targets) {
        mount_target(
            plan_dir,
            &context_root,
            &unique.repo_slug,
            &unique.component_id,
        )
        .with_context(|| {
            // errorcode-exempt: mount_target's CodedError survives the with_context wrap (see error_context tests)
            format!(
                "failed to mount {}:{} (unique to --from plan) into destination plan",
                unique.repo_slug, unique.component_id
            )
        })?;
        report.mounted.push(MountOutcome {
            repo_slug: unique.repo_slug.clone(),
            component_id: unique.component_id.clone(),
        });
    }

    let merge_set = read_targets(plan_dir)?.targets;
    let other_branch = format!("ravel-lite/{other_plan_name}/main");
    let dest_branch = format!("ravel-lite/{plan_name}/main");

    for target in &merge_set {
        if !other_has_component(&other_targets, &target.repo_slug, &target.component_id) {
            continue;
        }
        let working_root = plan_dir.join(&target.working_root);
        let outcome = merge_one_target(&working_root, &dest_branch, &other_branch, target)?;
        report.merges.push(outcome);
    }

    Ok(report)
}

/// Render a `SyncReport` as a short markdown summary suitable for stdout.
pub fn render_report(report: &SyncReport) -> String {
    let mut out = String::new();
    out.push_str("# sync report\n\n");

    if report.mounted.is_empty() {
        out.push_str("## mounted\n\n_no new targets mounted_\n\n");
    } else {
        out.push_str("## mounted\n\n");
        for m in &report.mounted {
            out.push_str(&format!("- {}:{}\n", m.repo_slug, m.component_id));
        }
        out.push('\n');
    }

    if report.merges.is_empty() {
        out.push_str("## merges\n\n_no shared targets to merge_\n");
        return out;
    }

    out.push_str("## merges\n\n");
    for m in &report.merges {
        let label = match &m.kind {
            MergeKind::Clean => "merged".to_string(),
            MergeKind::AlreadyUpToDate => "up-to-date".to_string(),
            MergeKind::Conflict { .. } => "CONFLICT".to_string(),
        };
        out.push_str(&format!("- {}:{} — {label}\n", m.repo_slug, m.component_id));
    }

    if report.has_conflicts() {
        out.push_str("\n## conflict details\n\n");
        for m in &report.merges {
            if let MergeKind::Conflict { stderr } = &m.kind {
                out.push_str(&format!(
                    "### {}:{}\n\nworktree: `{}`\n\n```\n{}\n```\n\n",
                    m.repo_slug,
                    m.component_id,
                    m.working_root.display(),
                    stderr.trim_end(),
                ));
            }
        }
        out.push_str(
            "Resolve each CONFLICT worktree by hand: `git -C <worktree> status` \
             to see the unmerged paths, edit, `git add`, then `git commit`. \
             Re-run sync to retry the remaining clean merges.\n",
        );
    }

    out
}

fn merge_one_target(
    working_root: &Path,
    dest_branch: &str,
    other_branch: &str,
    target: &Target,
) -> Result<MergeOutcome> {
    verify_on_branch(working_root, dest_branch).with_context(|| {
        // errorcode-exempt: verify_on_branch's CodedError survives the with_context wrap (see error_context tests)
        format!(
            "destination worktree {} is not on the expected branch {dest_branch}; \
             refuse to merge into a manually-checked-out branch",
            working_root.display()
        )
    })?;

    if branches_share_tip(working_root, dest_branch, other_branch)? {
        return Ok(MergeOutcome {
            repo_slug: target.repo_slug.clone(),
            component_id: target.component_id.clone(),
            working_root: working_root.to_path_buf(),
            kind: MergeKind::AlreadyUpToDate,
        });
    }

    let output = run_git(working_root, &["merge", "--no-edit", other_branch])?;
    if output.status.success() {
        return Ok(MergeOutcome {
            repo_slug: target.repo_slug.clone(),
            component_id: target.component_id.clone(),
            working_root: working_root.to_path_buf(),
            kind: MergeKind::Clean,
        });
    }
    let stderr = String::from_utf8_lossy(&output.stderr).into_owned();
    let stdout = String::from_utf8_lossy(&output.stdout).into_owned();
    let combined = if stderr.is_empty() { stdout } else { stderr };
    Ok(MergeOutcome {
        repo_slug: target.repo_slug.clone(),
        component_id: target.component_id.clone(),
        working_root: working_root.to_path_buf(),
        kind: MergeKind::Conflict { stderr: combined },
    })
}

fn unique_to_other<'a>(plan: &[Target], other: &'a [Target]) -> Vec<&'a Target> {
    other
        .iter()
        .filter(|o| {
            !plan
                .iter()
                .any(|p| p.repo_slug == o.repo_slug && p.component_id == o.component_id)
        })
        .collect()
}

fn other_has_component(other: &[Target], repo_slug: &str, component_id: &str) -> bool {
    other
        .iter()
        .any(|t| t.repo_slug == repo_slug && t.component_id == component_id)
}

fn plan_basename(plan_dir: &Path) -> Result<String> {
    plan_dir
        .file_name()
        .and_then(|n| n.to_str())
        .map(|s| s.to_string())
        .ok_or_else(|| {
            anyhow::Error::new(CodedError {
                code: ErrorCode::InvalidInput,
                message: format!(
                    "plan directory {} has no usable file name; cannot derive plan-namespaced branch",
                    plan_dir.display()
                ),
            })
        })
}

/// Per architecture-next.md §Layout, plans live at
/// `<context>/plans/<plan>/`. The context root is therefore two
/// parents up. Errors when the path lacks two ancestors.
fn derive_context_root(plan_dir: &Path) -> Result<PathBuf> {
    let parent = plan_dir.parent().ok_or_else(|| {
        anyhow::Error::new(CodedError {
            code: ErrorCode::InvalidInput,
            message: format!(
                "plan directory {} has no parent; cannot derive ravel context root",
                plan_dir.display()
            ),
        })
    })?;
    let grandparent = parent.parent().ok_or_else(|| {
        anyhow::Error::new(CodedError {
            code: ErrorCode::InvalidInput,
            message: format!(
                "plan directory {} has no grandparent; \
                 expected layout is <context>/plans/<plan>/",
                plan_dir.display()
            ),
        })
    })?;
    Ok(grandparent.to_path_buf())
}

fn verify_on_branch(working_root: &Path, expected: &str) -> Result<()> {
    let out = run_git(working_root, &["symbolic-ref", "--short", "HEAD"])?;
    if !out.status.success() {
        bail_with!(
            ErrorCode::Conflict,
            "{} is not a git worktree (or HEAD is detached)",
            working_root.display()
        );
    }
    let actual = String::from_utf8_lossy(&out.stdout).trim().to_string();
    if actual != expected {
        bail_with!(
            ErrorCode::Conflict,
            "{} is on branch {actual:?}, expected {expected:?}",
            working_root.display()
        );
    }
    Ok(())
}

fn branches_share_tip(working_root: &Path, a: &str, b: &str) -> Result<bool> {
    let resolve = |rev: &str| -> Result<Option<String>> {
        let out = run_git(working_root, &["rev-parse", "--verify", "--quiet", rev])?;
        if !out.status.success() {
            return Ok(None);
        }
        Ok(Some(String::from_utf8_lossy(&out.stdout).trim().to_string()))
    };
    let sha_a = resolve(a)?;
    let sha_b = resolve(b)?;
    Ok(matches!((sha_a, sha_b), (Some(x), Some(y)) if x == y))
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
    use crate::state::targets::mount_target;
    use std::fs;
    use tempfile::TempDir;

    /// Two plans in the same ravel context, one shared component, one
    /// shared source repo. `(tmp, plan_a, plan_b, context, source)`.
    fn two_plan_fixture(plan_a: &str, plan_b: &str) -> (TempDir, PathBuf, PathBuf, PathBuf, PathBuf) {
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
        repos::run_add(&context, "atlas", "git@example/atlas.git", Some(&source)).unwrap();

        let plan_a_dir = context.join("plans").join(plan_a);
        let plan_b_dir = context.join("plans").join(plan_b);
        fs::create_dir_all(&plan_a_dir).unwrap();
        fs::create_dir_all(&plan_b_dir).unwrap();

        (tmp, plan_a_dir, plan_b_dir, context, source)
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

    fn commit_file(worktree: &Path, name: &str, body: &str, msg: &str) {
        fs::write(worktree.join(name), body).unwrap();
        run_git_or_panic(worktree, &["add", name]);
        run_git_or_panic(worktree, &["-c", "user.email=test@example", "-c", "user.name=test", "commit", "-m", msg]);
    }

    #[test]
    fn derive_context_root_walks_up_two_levels_from_plan_dir() {
        let p = PathBuf::from("/tmp/ctx/plans/foo");
        assert_eq!(derive_context_root(&p).unwrap(), PathBuf::from("/tmp/ctx"));
    }

    #[test]
    fn derive_context_root_errors_when_plan_dir_has_no_grandparent() {
        let p = PathBuf::from("/foo");
        let err = derive_context_root(&p).unwrap_err();
        assert!(format!("{err:#}").contains("grandparent"));
    }

    #[test]
    fn unique_to_other_returns_targets_only_in_other() {
        let plan = vec![sample("atlas", "atlas-ontology")];
        let other = vec![
            sample("atlas", "atlas-ontology"),
            sample("atlas", "atlas-discovery"),
        ];
        let unique = unique_to_other(&plan, &other);
        assert_eq!(unique.len(), 1);
        assert_eq!(unique[0].component_id, "atlas-discovery");
    }

    #[test]
    fn unique_to_other_returns_empty_when_other_is_subset() {
        let plan = vec![
            sample("atlas", "atlas-ontology"),
            sample("atlas", "atlas-discovery"),
        ];
        let other = vec![sample("atlas", "atlas-ontology")];
        let unique = unique_to_other(&plan, &other);
        assert!(unique.is_empty());
    }

    fn sample(repo: &str, component: &str) -> Target {
        Target {
            repo_slug: repo.into(),
            component_id: component.into(),
            working_root: format!(".worktrees/{repo}"),
            branch: "ravel-lite/test/main".into(),
            path_segments: vec![],
        }
    }

    #[test]
    fn run_sync_errors_when_destination_and_from_are_the_same_dir() {
        let tmp = TempDir::new().unwrap();
        let plan = tmp.path().join("ctx/plans/p");
        fs::create_dir_all(&plan).unwrap();
        let err = run_sync(&plan, &plan).unwrap_err();
        assert!(format!("{err:#}").contains("same directory"));
    }

    #[test]
    fn run_sync_errors_when_plans_live_in_different_contexts() {
        let tmp = TempDir::new().unwrap();
        let a = tmp.path().join("ctx-a/plans/p");
        let b = tmp.path().join("ctx-b/plans/p");
        fs::create_dir_all(&a).unwrap();
        fs::create_dir_all(&b).unwrap();
        let err = run_sync(&a, &b).unwrap_err();
        assert!(format!("{err:#}").contains("different ravel contexts"));
    }

    #[test]
    fn run_sync_is_clean_noop_when_neither_plan_has_targets() {
        let (_tmp, a, b, _ctx, _src) = two_plan_fixture("plan-a", "plan-b");
        let report = run_sync(&a, &b).unwrap();
        assert!(report.mounted.is_empty());
        assert!(report.merges.is_empty());
        assert!(!report.has_conflicts());
    }

    #[test]
    fn run_sync_mounts_targets_unique_to_from_plan_into_destination() {
        // plan-b mounts atlas-ontology; plan-a has no targets. After sync
        // from b → a, plan-a has the same target mounted with its own
        // `ravel-lite/plan-a/main` branch.
        let (_tmp, a, b, ctx, _src) = two_plan_fixture("plan-a", "plan-b");
        mount_target(&b, &ctx, "atlas", "atlas-ontology").unwrap();

        let report = run_sync(&a, &b).unwrap();
        assert_eq!(report.mounted.len(), 1);
        assert_eq!(report.mounted[0].repo_slug, "atlas");
        assert_eq!(report.mounted[0].component_id, "atlas-ontology");

        // The destination plan now has a worktree on its own plan branch.
        let a_targets = read_targets(&a).unwrap();
        assert_eq!(a_targets.targets.len(), 1);
        assert_eq!(a_targets.targets[0].branch, "ravel-lite/plan-a/main");

        // Both plans pointed at the same source commit when mounted, so
        // their branches share a tip and the merge step is a no-op.
        assert_eq!(report.merges.len(), 1);
        assert!(matches!(report.merges[0].kind, MergeKind::AlreadyUpToDate));
    }

    #[test]
    fn run_sync_merges_a_diverged_shared_target_cleanly() {
        // Both plans mount the same target. Plan-b adds a commit on its
        // branch, plan-a does not. Sync merges plan-b's commit into
        // plan-a cleanly.
        let (_tmp, a, b, ctx, _src) = two_plan_fixture("plan-a", "plan-b");
        mount_target(&a, &ctx, "atlas", "atlas-ontology").unwrap();
        mount_target(&b, &ctx, "atlas", "atlas-ontology").unwrap();

        let b_worktree = b.join(".worktrees/atlas");
        commit_file(&b_worktree, "from-b.txt", "b\n", "feat: from b");

        let report = run_sync(&a, &b).unwrap();
        assert!(report.mounted.is_empty(), "no new mounts; both plans had this target");
        assert_eq!(report.merges.len(), 1);
        assert!(
            matches!(report.merges[0].kind, MergeKind::Clean),
            "expected clean merge, got {:?}",
            report.merges[0].kind
        );

        let a_worktree = a.join(".worktrees/atlas");
        assert!(a_worktree.join("from-b.txt").exists(), "merged file must materialise in plan-a");
    }

    #[test]
    fn run_sync_reports_a_conflict_without_aborting() {
        // Both plans mount the same target and edit the same file
        // differently. Sync attempts to merge plan-b into plan-a; the
        // merge conflicts and is reported as Conflict (not Err).
        let (_tmp, a, b, ctx, _src) = two_plan_fixture("plan-a", "plan-b");
        mount_target(&a, &ctx, "atlas", "atlas-ontology").unwrap();
        mount_target(&b, &ctx, "atlas", "atlas-ontology").unwrap();

        let a_worktree = a.join(".worktrees/atlas");
        let b_worktree = b.join(".worktrees/atlas");
        commit_file(&a_worktree, "shared.txt", "from a\n", "from a");
        commit_file(&b_worktree, "shared.txt", "from b\n", "from b");

        let report = run_sync(&a, &b).unwrap();
        assert_eq!(report.merges.len(), 1);
        match &report.merges[0].kind {
            MergeKind::Conflict { stderr } => {
                assert!(!stderr.is_empty(), "conflict outcome must carry git's stderr");
            }
            other => panic!("expected MergeKind::Conflict, got {other:?}"),
        }
        assert!(report.has_conflicts());
    }

    #[test]
    fn run_sync_skips_targets_unique_to_destination() {
        // Plan-a has a target plan-b doesn't. Sync must NOT touch it
        // (nothing to merge from b), and must NOT report it.
        let (_tmp, a, b, ctx, _src) = two_plan_fixture("plan-a", "plan-b");
        mount_target(&a, &ctx, "atlas", "atlas-ontology").unwrap();

        let report = run_sync(&a, &b).unwrap();
        assert!(report.mounted.is_empty());
        assert!(report.merges.is_empty(), "no shared targets means no merges");
    }

    #[test]
    fn render_report_summarises_mounts_and_merges() {
        let report = SyncReport {
            mounted: vec![MountOutcome {
                repo_slug: "atlas".into(),
                component_id: "atlas-discovery".into(),
            }],
            merges: vec![
                MergeOutcome {
                    repo_slug: "atlas".into(),
                    component_id: "atlas-ontology".into(),
                    working_root: PathBuf::from("/tmp/wt"),
                    kind: MergeKind::Clean,
                },
                MergeOutcome {
                    repo_slug: "atlas".into(),
                    component_id: "atlas-discovery".into(),
                    working_root: PathBuf::from("/tmp/wt2"),
                    kind: MergeKind::Conflict {
                        stderr: "CONFLICT (content): Merge conflict in shared.txt\n".into(),
                    },
                },
            ],
        };
        let rendered = render_report(&report);
        assert!(rendered.contains("# sync report"));
        assert!(rendered.contains("atlas:atlas-discovery"));
        assert!(rendered.contains("merged"));
        assert!(rendered.contains("CONFLICT"));
        assert!(rendered.contains("conflict details"));
        assert!(rendered.contains("Resolve each CONFLICT"));
    }

    #[test]
    fn render_report_mentions_no_mounts_or_merges_when_empty() {
        let report = SyncReport::default();
        let rendered = render_report(&report);
        assert!(rendered.contains("no new targets mounted"));
        assert!(rendered.contains("no shared targets to merge"));
    }
}
