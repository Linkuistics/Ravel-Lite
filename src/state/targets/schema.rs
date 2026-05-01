//! Typed schema for `<plan>/targets.yaml`.
//!
//! A `Target` is a component projected into a plan as a mounted
//! worktree: a `(repo_slug, component_id)` ComponentRef plus the
//! cached mount metadata (worktree path, branch, component
//! path-segments). Unlike `intents.yaml`, `backlog.yaml`, and
//! `memory.yaml`, targets are NOT TMS-shaped knowledge items — they
//! are pure runtime state, born when the runner mounts a worktree and
//! drained when the plan finishes. See
//! `docs/architecture-next.md` §Targets and worktrees for the design
//! rationale, and §Layout for where `targets.yaml` sits among the
//! plan-state files.

use serde::{Deserialize, Serialize};

pub const TARGETS_SCHEMA_VERSION: u32 = 1;

/// One mounted target. Identity is the `(repo_slug, component_id)`
/// pair; (repo_slug, component_id) is unique per `targets.yaml`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Target {
    /// Slug from the ravel-context `repos.yaml` registry. Used as the
    /// branch-namespace component and the `.worktrees/<repo_slug>/`
    /// directory name.
    pub repo_slug: String,
    /// Atlas component id, unique within a repo.
    pub component_id: String,
    /// Worktree mount path, relative to the plan directory. Conventionally
    /// `.worktrees/<repo_slug>`. Multiple components in the same repo
    /// share one worktree, so multiple `Target` rows may have the same
    /// `working_root`.
    pub working_root: String,
    /// Plan-namespaced git branch, conventionally
    /// `ravel-lite/<plan>/main`. Created at mount time from
    /// HEAD-of-default-branch in the source repo.
    pub branch: String,
    /// Path segments locating the component within its worktree, as
    /// resolved from `.atlas/components.yaml` at mount time. Cached
    /// here so the runner does not re-parse Atlas data on every phase.
    #[serde(default)]
    pub path_segments: Vec<String>,
}

/// The full `targets.yaml` document.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct TargetsFile {
    pub schema_version: u32,
    #[serde(default)]
    pub targets: Vec<Target>,
}

impl Default for TargetsFile {
    fn default() -> Self {
        Self {
            schema_version: TARGETS_SCHEMA_VERSION,
            targets: Vec::new(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_target(repo: &str, component: &str) -> Target {
        Target {
            repo_slug: repo.into(),
            component_id: component.into(),
            working_root: format!(".worktrees/{repo}"),
            branch: "ravel-lite/test-plan/main".to_string(),
            path_segments: vec!["crates".into(), component.into()],
        }
    }

    #[test]
    fn target_round_trips_through_yaml() {
        let target = sample_target("atlas", "atlas-ontology");
        let yaml = serde_yaml::to_string(&target).unwrap();
        let decoded: Target = serde_yaml::from_str(&yaml).unwrap();
        assert_eq!(decoded, target);
    }

    #[test]
    fn target_yaml_omits_empty_path_segments_round_trip() {
        let target = Target {
            repo_slug: "atlas".into(),
            component_id: "root".into(),
            working_root: ".worktrees/atlas".into(),
            branch: "ravel-lite/p/main".into(),
            path_segments: vec![],
        };
        let yaml = serde_yaml::to_string(&target).unwrap();
        let decoded: Target = serde_yaml::from_str(&yaml).unwrap();
        assert_eq!(decoded, target);
    }

    #[test]
    fn targets_file_default_has_current_schema_version() {
        let file = TargetsFile::default();
        assert_eq!(file.schema_version, TARGETS_SCHEMA_VERSION);
        assert!(file.targets.is_empty());
    }

    #[test]
    fn targets_file_round_trips_through_yaml() {
        let file = TargetsFile {
            schema_version: TARGETS_SCHEMA_VERSION,
            targets: vec![
                sample_target("atlas", "atlas-ontology"),
                sample_target("sidekick", "router"),
            ],
        };
        let yaml = serde_yaml::to_string(&file).unwrap();
        let decoded: TargetsFile = serde_yaml::from_str(&yaml).unwrap();
        assert_eq!(decoded, file);
    }

    #[test]
    fn targets_file_rejects_yaml_without_schema_version() {
        let yaml = "targets: []\n";
        let result: Result<TargetsFile, _> = serde_yaml::from_str(yaml);
        assert!(
            result.is_err(),
            "schema_version is required; missing must fail"
        );
    }

    #[test]
    fn targets_file_accepts_yaml_without_targets_key() {
        // `targets:` field is `default`, so an empty document with just
        // `schema_version` decodes as an empty file — useful for the
        // "freshly initialised, nothing mounted yet" case.
        let yaml = "schema_version: 1\n";
        let parsed: TargetsFile = serde_yaml::from_str(yaml).unwrap();
        assert_eq!(parsed.schema_version, TARGETS_SCHEMA_VERSION);
        assert!(parsed.targets.is_empty());
    }
}
