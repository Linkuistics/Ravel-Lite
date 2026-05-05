//! Orchestrator: mount a set of `(repo_slug, component_id)` refs plus
//! their build-time transitive closure as worktrees on the plan branch.
//!
//! Two callers share this entry point:
//!
//! - `migrate-v1-v2 → apply-targets`, which feeds the LLM-proposed
//!   targets in as the initial set.
//! - `ravel-lite plan add-target <plan> <repo>:<component>`, which
//!   feeds in a single ref from a human or work-phase agent.
//!
//! Both produce identical on-disk state: every component the build
//! transitively requires gets a `Target` row, grouped by repo into
//! shared worktrees on `ravel-lite/<plan>/main`.
//!
//! See `docs/superpowers/specs/2026-05-05-multi-repo-plan-build-closure-design.md`.

use std::path::Path;
use std::time::SystemTime;

use anyhow::Result;

use crate::atlas::{Catalog, EdgeGraph};
use crate::bail_with;
use crate::cli::ErrorCode;
use crate::repos::load_for_lookup;

use super::closure::expand_build_closure;
use super::mount::mount_target;
use super::schema::Target;
use super::verbs::find_target;
use super::yaml_io::read_targets;

/// Mount `initial_refs` and every component reachable from them along
/// `depends-on`/`links-statically`/`links-dynamically`/
/// `has-optional-dependency` edges. Idempotent: re-running with a subset
/// of already-mounted refs is a no-op; re-running with overlapping refs
/// only mounts the new ones.
///
/// Returns every `Target` row in the closure (newly mounted *or*
/// pre-existing), in the order they appear in `targets.yaml` after the
/// call. Caller can diff this against the pre-call state to report
/// what actually landed.
pub fn mount_with_closure(
    plan_dir: &Path,
    context_root: &Path,
    initial_refs: &[(String, String)],
) -> Result<Vec<Target>> {
    let registry = load_for_lookup(context_root)?;
    let catalog = Catalog::load(&registry, SystemTime::now());

    validate_initial_refs(&catalog, initial_refs)?;

    let graph = EdgeGraph::from_catalog(&catalog)?;
    let closure = expand_build_closure(initial_refs, &graph, &catalog)?;

    let mut mounted: Vec<Target> = Vec::with_capacity(closure.len());
    for (repo_slug, component_id) in &closure {
        let existing = read_targets(plan_dir)?;
        if let Ok(target) = find_target(&existing, repo_slug, component_id) {
            mounted.push(target.clone());
            continue;
        }
        let target = mount_target(plan_dir, context_root, repo_slug, component_id)?;
        mounted.push(target);
    }
    Ok(mounted)
}

/// Verify every initial ref names a fresh repo and a known component
/// before walking the graph. Surfaces a clearer error than the closure
/// walker would: the closure walker is concerned with *edge targets*,
/// while this check is concerned with the user-/LLM-supplied seed set.
fn validate_initial_refs(catalog: &Catalog, initial_refs: &[(String, String)]) -> Result<()> {
    for (repo_slug, component_id) in initial_refs {
        let Some(repo) = catalog.repos.get(repo_slug) else {
            let available: Vec<&str> = catalog.repos.keys().map(String::as_str).collect();
            bail_with!(
                ErrorCode::NotFound,
                "ref {repo_slug:?}:{component_id:?}: repo {repo_slug:?} is not a fresh \
                 entry in the catalog. Either it is missing from `repos.yaml`, has no \
                 `local_path`, or has no `.atlas/components.yaml`. Fresh repos: [{}]",
                available.join(", ")
            );
        };
        let known = repo
            .file
            .components
            .iter()
            .any(|c| !c.deleted && c.id == *component_id);
        if !known {
            bail_with!(
                ErrorCode::NotFound,
                "ref {repo_slug:?}:{component_id:?}: no component with id {component_id:?} \
                 in repo {repo_slug:?}. Check `ravel-lite atlas list-components --repo \
                 {repo_slug} --format yaml` for valid ids; re-run `atlas index` if the \
                 index is stale.",
            );
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::repos;
    use crate::state::targets::yaml_io::read_targets;
    use std::fs;
    use std::path::PathBuf;
    use std::process::Command;
    use tempfile::TempDir;

    /// Build a self-contained two-repo fixture: two real git repos under
    /// `tmp/`, each with `.atlas/components.yaml`. Cross-repo edges are
    /// written to one repo's `.atlas/related-components.yaml`. Returns
    /// `(tmp, plan_dir, context_root)` — keep `tmp` alive.
    fn two_repo_fixture(
        cross_repo_edges: &[(component_ontology::EdgeKind, &str, &str)],
    ) -> (TempDir, PathBuf, PathBuf) {
        let tmp = TempDir::new().unwrap();

        // Repo r1 with two components: app and shared.
        let r1 = tmp.path().join("r1");
        fs::create_dir_all(&r1).unwrap();
        init_git(&r1);
        write_components_yaml(&r1, &[("app", &["crates/app"]), ("shared", &["crates/shared"])]);

        // Repo r2 with one component: util.
        let r2 = tmp.path().join("r2");
        fs::create_dir_all(&r2).unwrap();
        init_git(&r2);
        write_components_yaml(&r2, &[("util", &["crates/util"])]);

        // Edges authored in r1's related-components.yaml.
        write_related_components_yaml(&r1, cross_repo_edges);

        let context = tmp.path().join("context");
        fs::create_dir_all(&context).unwrap();
        repos::run_add(&context, "r1", "git@example/r1.git", Some(&r1)).unwrap();
        repos::run_add(&context, "r2", "git@example/r2.git", Some(&r2)).unwrap();

        let plan = context.join("plans").join("test-plan");
        fs::create_dir_all(&plan).unwrap();

        (tmp, plan, context)
    }

    fn init_git(repo: &std::path::Path) {
        run_git(repo, &["init", "--initial-branch=main"]);
        run_git(repo, &["config", "user.email", "test@example"]);
        run_git(repo, &["config", "user.name", "test"]);
        fs::write(repo.join("README.md"), "x\n").unwrap();
        run_git(repo, &["add", "."]);
        run_git(repo, &["commit", "-m", "init"]);
    }

    fn run_git(cwd: &std::path::Path, args: &[&str]) {
        let out = Command::new("git").current_dir(cwd).args(args).output().unwrap();
        assert!(
            out.status.success(),
            "git {} failed in {}: {}",
            args.join(" "),
            cwd.display(),
            String::from_utf8_lossy(&out.stderr)
        );
    }

    fn write_components_yaml(repo: &std::path::Path, components: &[(&str, &[&str])]) {
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
            "schema_version: 1\nroot: {root}\ngenerated_at: '2026-05-05T00:00:00Z'\n\
             cache_fingerprints:\n  ontology_sha: ''\n  prompt_shas: {{}}\n  \
             model_id: ''\n  backend_version: ''\ncomponents:\n{comps}",
            root = repo.display()
        );
        fs::write(atlas_dir.join("components.yaml"), yaml).unwrap();
    }

    fn write_related_components_yaml(
        repo: &std::path::Path,
        edges: &[(component_ontology::EdgeKind, &str, &str)],
    ) {
        let atlas_dir = repo.join(".atlas");
        fs::create_dir_all(&atlas_dir).unwrap();
        let mut yaml = String::from("schema_version: 2\nedges:\n");
        if edges.is_empty() {
            yaml = "schema_version: 2\nedges: []\n".into();
        }
        for (kind, from, to) in edges {
            yaml.push_str(&format!("  - kind: {}\n", kind.as_str()));
            yaml.push_str("    lifecycle: build\n");
            yaml.push_str(&format!("    participants:\n      - {from}\n      - {to}\n"));
            yaml.push_str("    evidence_grade: strong\n");
            yaml.push_str("    evidence_fields: [Cargo.toml]\n");
            yaml.push_str("    rationale: test fixture\n");
        }
        fs::write(atlas_dir.join("related-components.yaml"), yaml).unwrap();
    }

    #[test]
    fn mounts_only_initial_when_no_edges() {
        // r1:app has no outgoing build edges; closure is just the
        // initial ref. Only r1's worktree is created.
        let (_tmp, plan, context) = two_repo_fixture(&[]);

        let mounted =
            mount_with_closure(&plan, &context, &[("r1".into(), "app".into())]).unwrap();

        assert_eq!(mounted.len(), 1);
        assert_eq!(mounted[0].repo_slug, "r1");
        assert_eq!(mounted[0].component_id, "app");
        assert!(plan.join(".worktrees/r1").is_dir());
        assert!(!plan.join(".worktrees/r2").exists(), "r2 must not be mounted");
    }

    #[test]
    fn mounts_cross_repo_dependency_eagerly() {
        // r1:app depends-on r2:util. Both worktrees must exist;
        // targets.yaml must list both rows.
        let (_tmp, plan, context) = two_repo_fixture(&[(
            component_ontology::EdgeKind::DependsOn,
            "app",
            "util",
        )]);

        let mounted =
            mount_with_closure(&plan, &context, &[("r1".into(), "app".into())]).unwrap();

        let slugs: Vec<&str> = mounted.iter().map(|t| t.repo_slug.as_str()).collect();
        assert!(slugs.contains(&"r1") && slugs.contains(&"r2"));
        assert!(plan.join(".worktrees/r1").is_dir());
        assert!(plan.join(".worktrees/r2").is_dir());

        let on_disk = read_targets(&plan).unwrap();
        assert_eq!(on_disk.targets.len(), 2);
    }

    #[test]
    fn second_call_with_same_input_is_a_noop() {
        // Idempotence: re-running with the same initial set adds no rows.
        let (_tmp, plan, context) = two_repo_fixture(&[(
            component_ontology::EdgeKind::DependsOn,
            "app",
            "util",
        )]);

        mount_with_closure(&plan, &context, &[("r1".into(), "app".into())]).unwrap();
        let after_first = read_targets(&plan).unwrap().targets.len();

        mount_with_closure(&plan, &context, &[("r1".into(), "app".into())]).unwrap();
        let after_second = read_targets(&plan).unwrap().targets.len();

        assert_eq!(after_first, after_second);
    }

    #[test]
    fn third_call_with_extra_ref_appends_only_new_rows() {
        // Initial mount: r1:app + closure (which is just app, no edges).
        // Then add-target r1:shared — must append exactly one row.
        let (_tmp, plan, context) = two_repo_fixture(&[]);
        mount_with_closure(&plan, &context, &[("r1".into(), "app".into())]).unwrap();
        assert_eq!(read_targets(&plan).unwrap().targets.len(), 1);

        mount_with_closure(&plan, &context, &[("r1".into(), "shared".into())]).unwrap();
        let after = read_targets(&plan).unwrap();
        assert_eq!(after.targets.len(), 2);
        assert!(after.targets.iter().any(|t| t.component_id == "shared"));
        // Both share the r1 worktree.
        assert!(after.targets.iter().all(|t| t.working_root == ".worktrees/r1"));
    }

    #[test]
    fn unknown_initial_repo_errors_with_fresh_repo_list() {
        let (_tmp, plan, context) = two_repo_fixture(&[]);
        let err = mount_with_closure(&plan, &context, &[("nope".into(), "app".into())]).unwrap_err();
        let msg = format!("{err:#}");
        assert!(msg.contains("\"nope\""), "must cite the bad slug: {msg}");
        assert!(msg.contains("Fresh repos"), "must list fresh repos: {msg}");
    }

    #[test]
    fn unknown_initial_component_errors_with_repo_qualified_message() {
        let (_tmp, plan, context) = two_repo_fixture(&[]);
        let err =
            mount_with_closure(&plan, &context, &[("r1".into(), "ghost".into())]).unwrap_err();
        let msg = format!("{err:#}");
        assert!(msg.contains("\"ghost\""), "must cite the missing component: {msg}");
        assert!(msg.contains("atlas list-components"), "must suggest the diagnostic command: {msg}");
    }
}
