//! Consume-and-remove drain for `<plan>/target-requests.yaml`.
//!
//! `drain_target_requests` is the runner-side half of the dynamic-mount
//! contract in `docs/architecture-next.md` §Phase boundaries. The runner
//! invokes this between phases. Semantics:
//!
//! 1. If `target-requests.yaml` is absent, return — empty queue is the
//!    common case and is not an error.
//! 2. Parse the file. A malformed file errors loudly; the runner does
//!    not silently swallow a corrupt request queue (a swallowed parse
//!    error would silently lose a mount the user requested).
//! 3. For each request, parse the `<repo>:<component>` reference and
//!    call `mount_target`, which updates `targets.yaml` as a side
//!    effect. Errors propagate; partial drains are left on disk so the
//!    user can see what was processed and what wasn't.
//! 4. After every request mounts successfully, delete
//!    `target-requests.yaml`.
//!
//! Idempotent: running drain on an empty/missing queue is a no-op.

use std::path::Path;

use anyhow::{Context, Result};

use super::yaml_io::{delete_target_requests, read_target_requests};
use crate::cli::error_context::ResultExt;
use crate::cli::ErrorCode;
use crate::state::targets::mount_target;

pub fn drain_target_requests(plan_dir: &Path, context_root: &Path) -> Result<usize> {
    let file = read_target_requests(plan_dir)?;
    if file.requests.is_empty() {
        // Nothing queued. Still delete a stray empty-but-present file so
        // the next drain has a clean precondition.
        delete_target_requests(plan_dir)?;
        return Ok(0);
    }

    let mut mounted = 0usize;
    for req in &file.requests {
        mount_target(
            plan_dir,
            context_root,
            &req.component.repo_slug,
            &req.component.component_id,
        )
        .with_context(|| {
            format!(
                "failed to mount {} (reason: {}) from {}/target-requests.yaml",
                req.component,
                req.reason,
                plan_dir.display()
            )
        })
        .with_code(ErrorCode::IoError)?;
        mounted += 1;
    }

    delete_target_requests(plan_dir)?;
    Ok(mounted)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::component_ref::ComponentRef;
    use crate::repos;
    use crate::state::target_requests::schema::{
        TargetRequest, TargetRequestsFile, TARGET_REQUESTS_SCHEMA_VERSION,
    };
    use crate::state::target_requests::yaml_io::{
        target_requests_path, write_target_requests,
    };
    use crate::state::targets::read_targets;
    use std::fs;
    use std::path::PathBuf;
    use std::process::Command;
    use tempfile::TempDir;

    /// Mirrors the `targets::mount` test fixture: a context root with
    /// `repos.yaml` pointing at a real source repo whose
    /// `.atlas/components.yaml` lists one component. We reuse the
    /// pattern rather than the helper because the helper is private to
    /// that module; a duplicate fixture is the lesser evil to avoid
    /// widening the targets test surface.
    fn fixture(component_ids: &[&str]) -> (TempDir, PathBuf, PathBuf) {
        let tmp = TempDir::new().unwrap();
        let source = tmp.path().join("source");
        fs::create_dir_all(&source).unwrap();
        run_git_or_panic(&source, &["init", "--initial-branch=main"]);
        run_git_or_panic(&source, &["config", "user.email", "test@example"]);
        run_git_or_panic(&source, &["config", "user.name", "test"]);
        fs::write(source.join("README.md"), "src\n").unwrap();
        run_git_or_panic(&source, &["add", "."]);
        run_git_or_panic(&source, &["commit", "-m", "init"]);

        let atlas_dir = source.join(".atlas");
        fs::create_dir_all(&atlas_dir).unwrap();
        let mut comps = String::new();
        for id in component_ids {
            comps.push_str(&format!("  - id: {id}\n"));
            comps.push_str("    kind: rust-library\n");
            comps.push_str("    evidence_grade: strong\n");
            comps.push_str("    rationale: fixture\n");
            comps.push_str("    path_segments:\n");
            comps.push_str(&format!("      - path: crates/{id}\n"));
            comps.push_str("        content_sha: 'fixture'\n");
        }
        let yaml = format!(
            "schema_version: 1\nroot: {root}\ngenerated_at: '2026-04-24T00:00:00Z'\n\
             cache_fingerprints:\n  ontology_sha: ''\n  prompt_shas: {{}}\n  \
             model_id: ''\n  backend_version: ''\ncomponents:\n{comps}",
            root = source.display()
        );
        fs::write(atlas_dir.join("components.yaml"), yaml).unwrap();

        let context = tmp.path().join("context");
        fs::create_dir_all(&context).unwrap();
        repos::run_add(&context, "atlas", "git@example/atlas.git", Some(&source)).unwrap();

        let plan = context.join("plans").join("test-plan");
        fs::create_dir_all(&plan).unwrap();

        (tmp, plan, context)
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
    fn drain_returns_zero_and_no_error_when_file_is_absent() {
        let tmp = TempDir::new().unwrap();
        let plan = tmp.path().join("plan");
        fs::create_dir_all(&plan).unwrap();
        let context = tmp.path().join("context");
        fs::create_dir_all(&context).unwrap();
        assert_eq!(drain_target_requests(&plan, &context).unwrap(), 0);
    }

    #[test]
    fn drain_mounts_each_request_and_removes_the_file() {
        let (_tmp, plan, context) = fixture(&["atlas-ontology", "atlas-discovery"]);

        let file = TargetRequestsFile {
            schema_version: TARGET_REQUESTS_SCHEMA_VERSION,
            requests: vec![
                TargetRequest {
                    component: ComponentRef::new("atlas", "atlas-ontology"),
                    reason: "core schema".into(),
                },
                TargetRequest {
                    component: ComponentRef::new("atlas", "atlas-discovery"),
                    reason: "discovery pipeline".into(),
                },
            ],
        };
        write_target_requests(&plan, &file).unwrap();

        let mounted = drain_target_requests(&plan, &context).unwrap();
        assert_eq!(mounted, 2);

        assert!(
            !target_requests_path(&plan).exists(),
            "drain must delete target-requests.yaml after success"
        );
        let on_disk = read_targets(&plan).unwrap();
        assert_eq!(on_disk.targets.len(), 2);
        assert_eq!(on_disk.targets[0].component_id, "atlas-ontology");
        assert_eq!(on_disk.targets[1].component_id, "atlas-discovery");
    }

    #[test]
    fn drain_is_idempotent_across_runs() {
        // First drain mounts and removes the file. Second drain on the
        // same plan finds nothing queued and returns zero — the runner
        // must be safe to call drain at every phase boundary, not just
        // when work is pending.
        let (_tmp, plan, context) = fixture(&["atlas-ontology"]);
        let file = TargetRequestsFile {
            schema_version: TARGET_REQUESTS_SCHEMA_VERSION,
            requests: vec![TargetRequest {
                component: ComponentRef::new("atlas", "atlas-ontology"),
                reason: "core".into(),
            }],
        };
        write_target_requests(&plan, &file).unwrap();

        assert_eq!(drain_target_requests(&plan, &context).unwrap(), 1);
        assert_eq!(drain_target_requests(&plan, &context).unwrap(), 0);
    }

    #[test]
    fn drain_deletes_an_empty_queue_so_next_call_has_a_clean_precondition() {
        // An empty-but-present file is unusual but not impossible — a
        // user could land here by hand-editing or by a bug elsewhere.
        // Treat it as "nothing to mount, but tidy up", not as "error".
        let (_tmp, plan, _context) = fixture(&["atlas-ontology"]);
        let context_root = plan.parent().unwrap().parent().unwrap();
        write_target_requests(
            &plan,
            &TargetRequestsFile {
                schema_version: TARGET_REQUESTS_SCHEMA_VERSION,
                requests: vec![],
            },
        )
        .unwrap();

        assert_eq!(drain_target_requests(&plan, context_root).unwrap(), 0);
        assert!(!target_requests_path(&plan).exists());
    }

    #[test]
    fn drain_errors_when_a_request_references_an_unknown_component() {
        // Bad request: the file lists a component that
        // .atlas/components.yaml does not know about. Drain must error
        // out loudly and leave the file in place — so the user (or
        // operator running the runner) can see what was queued and
        // correct it.
        let (_tmp, plan, context) = fixture(&["atlas-ontology"]);
        let file = TargetRequestsFile {
            schema_version: TARGET_REQUESTS_SCHEMA_VERSION,
            requests: vec![TargetRequest {
                component: ComponentRef::new("atlas", "not-a-real-component"),
                reason: "typo".into(),
            }],
        };
        write_target_requests(&plan, &file).unwrap();

        let err = drain_target_requests(&plan, &context).unwrap_err();
        let msg = format!("{err:#}");
        assert!(
            msg.contains("not-a-real-component"),
            "error must cite the bad component: {msg}"
        );
        assert!(
            target_requests_path(&plan).exists(),
            "file must remain on disk for inspection after a failed drain"
        );
    }

    #[test]
    fn drain_errors_on_malformed_component_reference() {
        // After lifting `component` to `ComponentRef`, the parse error
        // surfaces at deserialise time inside `read_target_requests`
        // rather than at drain time. Either way drain still errors out
        // loudly without mounting anything — that is the property the
        // runner relies on.
        let (_tmp, plan, context) = fixture(&["atlas-ontology"]);
        fs::write(
            target_requests_path(&plan),
            "schema_version: 1\nrequests:\n  - component: no-colon-here\n    reason: bad\n",
        )
        .unwrap();

        let err = drain_target_requests(&plan, &context).unwrap_err();
        let msg = format!("{err:#}");
        assert!(
            msg.contains("repo_slug") || msg.contains("missing ':'"),
            "error must explain expected shape: {msg}"
        );
    }
}
