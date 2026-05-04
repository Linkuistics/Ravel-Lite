//! migrate-targets phase application: parse `migrate-targets-proposal.yaml`,
//! mount each target via the existing `mount_target` machinery.

use std::fs;
use std::sync::Arc;

use anyhow::Result;

use crate::agent::Agent;
use crate::bail_with;
use crate::cli::ErrorCode;
use crate::state::targets::mount::mount_target;

use super::proposals::{TargetsProposal, TARGETS_PROPOSAL_FILENAME};
use super::validate::Validated;

pub async fn run(
    agent: Arc<dyn Agent>,
    v: &Validated,
    skip_confirm: bool,
) -> Result<()> {
    super::orchestrator::invoke_phase(agent, v, crate::types::LlmPhase::MigrateTargets).await?;
    apply_proposal(v, skip_confirm)
}

pub fn apply_proposal(v: &Validated, skip_confirm: bool) -> Result<()> {
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

    if !skip_confirm {
        super::orchestrator::confirm(&format!(
            "migrate-targets: mount {} targets in {}?",
            proposal.targets.len(),
            v.source_repo_slug
        ))?;
    }

    for tp in &proposal.targets {
        mount_target(
            &v.new_plan_dir,
            &v.config_dir,
            &v.source_repo_slug,
            &tp.component_id,
        )?;
    }

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

        // Atlas index with a single component "core".
        fs::create_dir_all(source.join(".atlas")).unwrap();
        fs::write(
            source.join(".atlas/components.yaml"),
            "schema_version: 1\n\
             root: source\n\
             generated_at: 2026-04-01T00:00:00Z\n\
             cache_fingerprints:\n  ontology_sha: ''\n  model_id: ''\n  backend_version: ''\n\
             components:\n  - id: core\n    kind: library\n    evidence_grade: strong\n    rationale: test fixture\n",
        )
        .unwrap();

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
    fn apply_mounts_one_target_and_writes_targets_yaml() {
        let tmp = TempDir::new().unwrap();
        let v = fake_validated(tmp.path());
        let proposal = TargetsProposal {
            targets: vec![super::super::proposals::TargetProposal {
                component_id: "core".into(),
            }],
        };
        fs::write(
            v.new_plan_dir.join(TARGETS_PROPOSAL_FILENAME),
            serde_yaml::to_string(&proposal).unwrap(),
        )
        .unwrap();

        apply_proposal(&v, true).unwrap();

        let targets =
            crate::state::targets::yaml_io::read_targets(&v.new_plan_dir).unwrap();
        assert_eq!(targets.targets.len(), 1);
        assert_eq!(targets.targets[0].repo_slug, "source");
        assert_eq!(targets.targets[0].component_id, "core");
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
        let err = apply_proposal(&v, true).unwrap_err();
        assert!(format!("{err:#}").contains(TARGETS_PROPOSAL_FILENAME));
    }
}
