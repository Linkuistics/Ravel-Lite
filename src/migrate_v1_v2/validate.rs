//! Half-A step 1: validate the inputs and resolve the canonical paths
//! every later step depends on.

use std::path::{Path, PathBuf};

use anyhow::Result;

use crate::bail_with;
use crate::cli::error_context::ResultExt;
use crate::cli::ErrorCode;
use crate::repos::{load_for_lookup, REGISTRY_FILE};

/// Resolved inputs after validation.
#[derive(Debug, Clone)]
pub struct Validated {
    pub old_plan_path: PathBuf,
    pub new_plan_dir: PathBuf,
    pub source_repo_slug: String,
    pub source_repo_path: PathBuf,
    pub config_dir: PathBuf,
}

pub fn validate_inputs(
    old_plan_path: &Path,
    new_plan_name: &str,
    config_dir: &Path,
) -> Result<Validated> {
    let old_plan_path = old_plan_path
        .canonicalize()
        .map_err(|e| anyhow::anyhow!("cannot resolve {}: {e}", old_plan_path.display())) // errorcode-exempt: tagged via .with_code() below
        .with_code(ErrorCode::NotFound)?;

    if !old_plan_path.join("phase.md").is_file() {
        bail_with!(
            ErrorCode::InvalidInput,
            "{} is not a v1 plan dir (no phase.md).",
            old_plan_path.display()
        );
    }
    if !old_plan_path.join("backlog.yaml").is_file() {
        if old_plan_path.join("backlog.md").is_file() {
            bail_with!(
                ErrorCode::InvalidInput,
                "no backlog.yaml found at {} — this plan has only legacy markdown state. \
                 Re-export it as YAML before migrating.",
                old_plan_path.display()
            );
        }
        bail_with!(
            ErrorCode::InvalidInput,
            "no backlog.yaml found at {} — not a v1 plan dir.",
            old_plan_path.display()
        );
    }
    if old_plan_path.join("intents.yaml").is_file() {
        bail_with!(
            ErrorCode::InvalidInput,
            "{} already has intents.yaml — this plan looks v2-shaped already.",
            old_plan_path.display()
        );
    }

    if !config_dir.join(REGISTRY_FILE).is_file() {
        bail_with!(
            ErrorCode::NotFound,
            "config dir {} has no {REGISTRY_FILE} — run `ravel-lite init` first.",
            config_dir.display()
        );
    }

    crate::create::validate_plan_name(new_plan_name)?;
    let new_plan_dir = config_dir.join("plans").join(new_plan_name);
    if new_plan_dir.exists() {
        bail_with!(
            ErrorCode::Conflict,
            "{} already exists — pick a different --as.",
            new_plan_dir.display()
        );
    }

    // Source repo path = <old_plan_path>/../..
    let source_repo_path = old_plan_path
        .parent()
        .and_then(Path::parent)
        .ok_or_else(|| {
            anyhow::anyhow!( // errorcode-exempt: tagged via .with_code() below
                "cannot derive source repo from {}: not enough path components",
                old_plan_path.display()
            )
        })
        .with_code(ErrorCode::InvalidInput)?
        .to_path_buf();

    // Look up repo_slug by matching local_path in repos.yaml.
    let registry = load_for_lookup(config_dir)?;
    let mut found_slug: Option<String> = None;
    for (slug, entry) in registry.repos.iter() {
        if let Some(local) = &entry.local_path {
            if let Ok(canonical) = local.canonicalize() {
                if canonical == source_repo_path {
                    found_slug = Some(slug.clone());
                    break;
                }
            }
        }
    }
    let source_repo_slug = found_slug
        .ok_or_else(|| {
            anyhow::anyhow!( // errorcode-exempt: tagged via .with_code() below
                "source repo at {} is not registered in {}/{REGISTRY_FILE}. \
                 Add it with `ravel-lite repo add <slug> --url <url> --local-path {}`.",
                source_repo_path.display(),
                config_dir.display(),
                source_repo_path.display()
            )
        })
        .with_code(ErrorCode::NotFound)?;

    if !source_repo_path.join(".atlas/components.yaml").is_file() {
        bail_with!(
            ErrorCode::NotFound,
            "source repo {} has no .atlas/components.yaml — run Atlas first.",
            source_repo_path.display()
        );
    }

    Ok(Validated {
        old_plan_path,
        new_plan_dir,
        source_repo_slug,
        source_repo_path,
        config_dir: config_dir.to_path_buf(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    fn make_v1_plan(root: &Path, project: &str, plan: &str) -> PathBuf {
        let plan_dir = root.join(project).join("LLM_STATE").join(plan);
        fs::create_dir_all(&plan_dir).unwrap();
        fs::write(plan_dir.join("phase.md"), "triage\n").unwrap();
        fs::write(plan_dir.join("backlog.yaml"), "schema_version: 1\nitems: []\n").unwrap();
        fs::write(plan_dir.join("memory.yaml"), "schema_version: 1\nitems: []\n").unwrap();
        plan_dir
    }

    fn make_config_dir(root: &Path, repo_slug: &str, repo_path: &Path) -> PathBuf {
        let cfg = root.join("ravel-context");
        fs::create_dir_all(cfg.join("plans")).unwrap();
        fs::write(
            cfg.join("repos.yaml"),
            format!(
                "schema_version: 1\nrepos:\n  {repo_slug}:\n    url: git@example:foo.git\n    local_path: {}\n",
                repo_path.display()
            ),
        )
        .unwrap();
        cfg
    }

    fn make_atlas(repo: &Path) {
        let atlas = repo.join(".atlas");
        fs::create_dir_all(&atlas).unwrap();
        fs::write(
            atlas.join("components.yaml"),
            "schema_version: 1\nroot: foo\ncomponents: []\n",
        )
        .unwrap();
    }

    #[test]
    fn validates_well_formed_v1_plan() {
        let tmp = TempDir::new().unwrap();
        let plan = make_v1_plan(tmp.path(), "MyProj", "core");
        let repo = tmp.path().join("MyProj");
        make_atlas(&repo);
        let cfg = make_config_dir(tmp.path(), "myproj", &repo);

        let v = validate_inputs(&plan, "myproj-core", &cfg).unwrap();
        assert_eq!(v.source_repo_slug, "myproj");
        assert_eq!(v.new_plan_dir, cfg.join("plans/myproj-core"));
    }

    #[test]
    fn rejects_pure_md_plan() {
        let tmp = TempDir::new().unwrap();
        let plan = tmp.path().join("MyProj/LLM_STATE/core");
        fs::create_dir_all(&plan).unwrap();
        fs::write(plan.join("phase.md"), "triage\n").unwrap();
        fs::write(plan.join("backlog.md"), "# stale\n").unwrap();

        let cfg = make_config_dir(tmp.path(), "myproj", &tmp.path().join("MyProj"));
        make_atlas(&tmp.path().join("MyProj"));

        let err = validate_inputs(&plan, "x", &cfg).unwrap_err();
        let msg = format!("{err:#}");
        assert!(msg.contains("backlog.yaml"), "msg: {msg}");
        assert!(
            msg.contains("Re-export") || msg.contains("legacy markdown"),
            "msg: {msg}"
        );
    }

    #[test]
    fn rejects_already_v2_plan() {
        let tmp = TempDir::new().unwrap();
        let plan = make_v1_plan(tmp.path(), "MyProj", "core");
        fs::write(plan.join("intents.yaml"), "schema_version: 1\nitems: []\n").unwrap();
        let cfg = make_config_dir(tmp.path(), "myproj", &tmp.path().join("MyProj"));
        make_atlas(&tmp.path().join("MyProj"));

        let err = validate_inputs(&plan, "x", &cfg).unwrap_err();
        assert!(format!("{err:#}").contains("v2-shaped"), "{err:#}");
    }

    #[test]
    fn rejects_collision_with_existing_plan_name() {
        let tmp = TempDir::new().unwrap();
        let plan = make_v1_plan(tmp.path(), "MyProj", "core");
        let cfg = make_config_dir(tmp.path(), "myproj", &tmp.path().join("MyProj"));
        make_atlas(&tmp.path().join("MyProj"));
        fs::create_dir_all(cfg.join("plans/already-here")).unwrap();

        let err = validate_inputs(&plan, "already-here", &cfg).unwrap_err();
        let msg = format!("{err:#}");
        assert!(
            msg.contains("already exists") || msg.contains("collision"),
            "msg: {msg}"
        );
    }

    #[test]
    fn rejects_unregistered_source_repo() {
        let tmp = TempDir::new().unwrap();
        let plan = make_v1_plan(tmp.path(), "MyProj", "core");
        let cfg = make_config_dir(tmp.path(), "other-repo", &tmp.path().join("OtherProj"));
        fs::create_dir_all(tmp.path().join("OtherProj")).unwrap();
        make_atlas(&tmp.path().join("MyProj"));

        let err = validate_inputs(&plan, "x", &cfg).unwrap_err();
        assert!(format!("{err:#}").contains("repo add"), "{err:#}");
    }

    #[test]
    fn rejects_missing_atlas_index() {
        let tmp = TempDir::new().unwrap();
        let plan = make_v1_plan(tmp.path(), "MyProj", "core");
        let cfg = make_config_dir(tmp.path(), "myproj", &tmp.path().join("MyProj"));
        let err = validate_inputs(&plan, "x", &cfg).unwrap_err();
        assert!(format!("{err:#}").contains("Atlas"), "{err:#}");
    }

    #[test]
    fn rejects_missing_repos_yaml() {
        let tmp = TempDir::new().unwrap();
        let plan = make_v1_plan(tmp.path(), "MyProj", "core");
        let cfg = tmp.path().join("ravel-context");
        fs::create_dir_all(&cfg).unwrap();
        let err = validate_inputs(&plan, "x", &cfg).unwrap_err();
        assert!(format!("{err:#}").contains("ravel-lite init"), "{err:#}");
    }
}
