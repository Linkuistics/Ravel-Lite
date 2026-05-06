//! migrate-targets phase application: parse `migrate-targets-proposal.yaml`,
//! expand the LLM's chosen refs into their build-time transitive closure,
//! and mount every component in the closure as a worktree on the plan
//! branch. See `state::targets::mount_with_closure` for the orchestration
//! and `state::targets::closure` for the closure semantics.

use std::fs;
use std::sync::Arc;

use anyhow::Result;
use component_ontology::ComponentId;

use crate::agent::Agent;
use crate::bail_with;
use crate::cli::ErrorCode;
use crate::state::targets::mount_with_closure::mount_with_closure;

use super::proposals::{TargetsProposal, TARGETS_PROPOSAL_FILENAME};
use super::validate::Validated;

pub async fn run(agent: Arc<dyn Agent>, v: &Validated) -> Result<()> {
    super::orchestrator::invoke_phase(agent, v, crate::types::LlmPhase::MigrateTargets).await?;
    apply_proposal(v)
}

pub fn apply_proposal(v: &Validated) -> Result<()> {
    let scratch = v.new_plan_dir.join(TARGETS_PROPOSAL_FILENAME);
    if !scratch.is_file() {
        bail_with!(
            ErrorCode::NotFound,
            "{} not written by migrate-targets phase",
            scratch.display()
        );
    }
    let body = fs::read_to_string(&scratch)?;
    let proposal: TargetsProposal = serde_yaml::from_str(&body)?;

    // `TargetProposal.component_id` is now typed via serde to a
    // `ComponentId`, so the LLM's emitted id is already the canonical
    // path-style form. Cross-repo *initial* targets are out of scope
    // for v1 (per the prompt); every initial ref names the source repo.
    let initial_refs: Vec<(String, ComponentId)> = proposal
        .targets
        .iter()
        .map(|tp| (v.source_repo_slug.clone(), tp.component_id.clone()))
        .collect();
    mount_with_closure(&v.new_plan_dir, &v.config_dir, &initial_refs)?;

    fs::remove_file(&scratch).ok();
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::path::{Path, PathBuf};
    use tempfile::TempDir;

    fn fake_validated(tmp: &Path) -> Validated {
        let context = tmp.join("ctx");
        fs::create_dir_all(context.join("plans")).unwrap();

        let source = tmp.join("source");
        fs::create_dir_all(&source).unwrap();
        // Real git repo so `git worktree add` works.
        run_git(&source, &["init", "--initial-branch=main"]);
        std::fs::write(source.join("README"), "x").unwrap();
        run_git(
            &source,
            &[
                "-c",
                "user.email=t@t",
                "-c",
                "user.name=t",
                "add",
                ".",
            ],
        );
        run_git(
            &source,
            &[
                "-c",
                "user.email=t@t",
                "-c",
                "user.name=t",
                "commit",
                "-m",
                "init",
            ],
        );

        // Atlas index with a workspace-root component `source` plus a
        // child `source/core`. Modelling the realistic path-style id
        // shape exercises the regression that motivated this refactor:
        // the LLM emits `source/core` verbatim, and the migrator must
        // mount it directly without stripping a `source/` prefix.
        fs::create_dir_all(source.join(".atlas")).unwrap();
        let components_yaml = "\
schema_version: 1
root: source
generated_at: 2026-04-01T00:00:00Z
cache_fingerprints:
  ontology_sha: ''
  model_id: ''
  backend_version: ''
components:
  - id: source
    kind: library
    evidence_grade: strong
    rationale: workspace root
  - id: source/core
    kind: library
    evidence_grade: strong
    rationale: test fixture
    parent: source
";
        fs::write(source.join(".atlas/components.yaml"), components_yaml).unwrap();

        fs::write(
            context.join("repos.yaml"),
            format!(
                "schema_version: 1\nrepos:\n  source:\n    url: git@example:source.git\n    local_path: {}\n",
                source.display()
            ),
        )
        .unwrap();

        let plan_dir = context.join("plans/myplan");
        fs::create_dir_all(&plan_dir).unwrap();
        fs::write(
            plan_dir.join("targets.yaml"),
            "schema_version: 1\ntargets: []\n",
        )
        .unwrap();

        Validated {
            old_plan_path: tmp.join("old"),
            new_plan_dir: plan_dir,
            source_repo_slug: "source".into(),
            source_repo_path: source,
            config_dir: context,
        }
    }

    fn run_git(cwd: &Path, args: &[&str]) {
        let out = std::process::Command::new("git")
            .args(args)
            .current_dir(cwd)
            .output()
            .expect("git failed to spawn");
        assert!(
            out.status.success(),
            "git {args:?} failed: {}",
            String::from_utf8_lossy(&out.stderr)
        );
    }

    #[test]
    fn apply_mounts_one_target_with_full_path_id_and_writes_targets_yaml() {
        // Regression: the LLM's `atlas list-components --format yaml`
        // output now publishes path-style ids verbatim (e.g.
        // `source/core`). The migrator must take them as-is and mount
        // the named component — without stripping the leading
        // `source/` segment as the previous prefix-strip hack did.
        let tmp = TempDir::new().unwrap();
        let v = fake_validated(tmp.path());
        let proposal = TargetsProposal {
            targets: vec![super::super::proposals::TargetProposal {
                component_id: ComponentId::parse("source/core").unwrap(),
            }],
        };
        fs::write(
            v.new_plan_dir.join(TARGETS_PROPOSAL_FILENAME),
            serde_yaml::to_string(&proposal).unwrap(),
        )
        .unwrap();

        apply_proposal(&v).unwrap();

        let targets =
            crate::state::targets::yaml_io::read_targets(&v.new_plan_dir).unwrap();
        assert_eq!(targets.targets.len(), 1);
        assert_eq!(targets.targets[0].repo_slug, "source");
        assert_eq!(targets.targets[0].component_id.as_str(), "source/core");
        assert!(v.new_plan_dir.join(".worktrees/source").is_dir());
    }

    #[test]
    fn apply_errors_when_scratch_missing() {
        let tmp = TempDir::new().unwrap();
        // Minimal Validated — fs paths exist but no proposal scratch file.
        let plan_dir = tmp.path().join("plans/p");
        fs::create_dir_all(&plan_dir).unwrap();
        let v = Validated {
            old_plan_path: PathBuf::from(tmp.path()),
            new_plan_dir: plan_dir,
            source_repo_slug: "x".into(),
            source_repo_path: PathBuf::from(tmp.path()),
            config_dir: PathBuf::from(tmp.path()),
        };
        let err = apply_proposal(&v).unwrap_err();
        assert!(format!("{err:#}").contains(TARGETS_PROPOSAL_FILENAME));
    }
}
