//! Path-shape gate: ravel-lite 2.x cycle-shaped verbs refuse to operate
//! on v1 plan dirs (`<project>/LLM_STATE/<plan>/`).
//!
//! A v2 plan dir is `<X>/plans/<Y>/` where `<X>/repos.yaml` exists.
//! Detection is path-shape based — no marker file inside the plan dir.

use std::path::Path;

use anyhow::Result;

use crate::bail_with;
use crate::cli::ErrorCode;
use crate::repos::REGISTRY_FILE;

/// Verify `plan_dir` is a v2 plan dir. Returns `Ok(())` iff:
/// - `plan_dir.parent()` is named `plans`, AND
/// - `plan_dir.parent().parent()` (the context root) contains `repos.yaml`.
///
/// Otherwise returns an actionable error pointing the user at
/// `ravel-lite migrate-v1-v2`.
pub fn validate_v2_plan_dir(plan_dir: &Path) -> Result<()> {
    let parent = plan_dir.parent();
    let grandparent = parent.and_then(Path::parent);

    let parent_is_plans = parent
        .and_then(Path::file_name)
        .map(|n| n == "plans")
        .unwrap_or(false);
    let grandparent_has_registry = grandparent
        .map(|gp| gp.join(REGISTRY_FILE).is_file())
        .unwrap_or(false);

    if parent_is_plans && grandparent_has_registry {
        return Ok(());
    }

    bail_with!(
        ErrorCode::InvalidInput,
        "plan dir {} looks like a v1 layout (<project>/LLM_STATE/<plan>/). \
         ravel-lite 2.x does not run v1 plans directly — migrate it first:\n\n    \
         ravel-lite migrate-v1-v2 {} --as <new-name>\n\n\
         Then run against <config-dir>/plans/<new-name>/ instead.",
        plan_dir.display(),
        plan_dir.display()
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    #[test]
    fn accepts_v2_plan_dir() {
        let tmp = TempDir::new().unwrap();
        let context = tmp.path().join("context");
        fs::create_dir_all(context.join("plans/myplan")).unwrap();
        fs::write(context.join("repos.yaml"), "repos: {}\n").unwrap();
        validate_v2_plan_dir(&context.join("plans/myplan")).unwrap();
    }

    #[test]
    fn rejects_v1_plan_dir() {
        let tmp = TempDir::new().unwrap();
        let plan = tmp.path().join("project/LLM_STATE/core");
        fs::create_dir_all(&plan).unwrap();
        let err = validate_v2_plan_dir(&plan).unwrap_err();
        let msg = format!("{err:#}");
        assert!(msg.contains("v1 layout"), "msg: {msg}");
        assert!(msg.contains("migrate-v1-v2"), "msg: {msg}");
    }

    #[test]
    fn rejects_plans_dir_without_repos_yaml() {
        // Grandparent is missing repos.yaml — looks v2-shaped but isn't.
        let tmp = TempDir::new().unwrap();
        let plan = tmp.path().join("ctx/plans/p");
        fs::create_dir_all(&plan).unwrap();
        let err = validate_v2_plan_dir(&plan).unwrap_err();
        assert!(format!("{err:#}").contains("v1 layout"));
    }

    #[test]
    fn rejects_path_with_no_grandparent() {
        let err = validate_v2_plan_dir(Path::new("/")).unwrap_err();
        assert!(format!("{err:#}").contains("v1 layout"));
    }
}
