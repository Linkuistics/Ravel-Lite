//! Typed schema for `<plan>/commits.yaml`.
//!
//! `commits.yaml` is a one-shot scratch file written by the analyse-work
//! phase and consumed by `git-commit-work`: it partitions the work-tree
//! diff into an ordered sequence of logical commits. Each entry names a
//! list of `git add` pathspecs and the commit message to use. The
//! optional `target` field names a `<repo_slug>:<component_id>`
//! ComponentRef so a future two-stream commit applier can route the
//! commit to the correct mounted worktree (architecture-next §Commits);
//! when absent the commit lands in the plan's git root, preserving v1
//! behaviour.
//!
//! Unlike the TMS state files (`intents.yaml`, `backlog.yaml`,
//! `memory.yaml`), commits are not knowledge-graph items — they are an
//! ordered work-list that the runner mechanically applies and removes.
//!
//! Backward compatibility note: `schema_version` is defaulted, not
//! required, and the reader does NOT reject mismatched values. Existing
//! v1 prompts emit no `schema_version` and no `target`; both fields
//! arrive via serde defaults. The only structural extension this version
//! makes is the optional `target` field — adding optional fields is
//! both forward- and backward-compatible, so no version bump is forced.

use serde::{Deserialize, Serialize};

use crate::component_ref::ComponentRef;

pub const COMMITS_SCHEMA_VERSION: u32 = 1;

fn default_schema_version() -> u32 {
    COMMITS_SCHEMA_VERSION
}

/// One entry in `commits.yaml`: a pathspec list and the commit message
/// that should be applied to those paths. Pathspec strings are passed
/// verbatim to `git add`, so standard git pathspec features (globs like
/// `src/**`, exclusions like `:!src/generated/`) all work.
///
/// `target` routes the commit to a specific mounted worktree when the
/// two-stream commit applier is wired in. `None` means "apply in the
/// plan's git root" — the v1 behaviour.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct CommitSpec {
    pub paths: Vec<String>,
    pub message: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub target: Option<ComponentRef>,
}

/// Shape of `commits.yaml` — an ordered list of `CommitSpec` entries
/// that together partition the work-tree diff into logical commits.
/// Analyse-work authors this file; `git-commit-work` applies it and
/// removes it.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct CommitsSpec {
    #[serde(default = "default_schema_version")]
    pub schema_version: u32,
    pub commits: Vec<CommitSpec>,
}

impl Default for CommitsSpec {
    fn default() -> Self {
        Self {
            schema_version: COMMITS_SCHEMA_VERSION,
            commits: Vec::new(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_spec_with_target() -> CommitSpec {
        CommitSpec {
            paths: vec!["src/**".to_string()],
            message: "Wire greeting".to_string(),
            target: Some(ComponentRef::new("ravel-lite", "phase-loop")),
        }
    }

    #[test]
    fn commit_spec_round_trips_with_target() {
        let spec = sample_spec_with_target();
        let yaml = serde_yaml::to_string(&spec).unwrap();
        let decoded: CommitSpec = serde_yaml::from_str(&yaml).unwrap();
        assert_eq!(decoded, spec);
    }

    #[test]
    fn commit_spec_round_trips_without_target() {
        let spec = CommitSpec {
            paths: vec!["src/**".to_string()],
            message: "Plain v1 entry".to_string(),
            target: None,
        };
        let yaml = serde_yaml::to_string(&spec).unwrap();
        // Wire form must omit the target key entirely when None — that's
        // the v1 wire shape the existing prompts emit.
        assert!(
            !yaml.contains("target"),
            "absent target must not appear in wire form: {yaml}"
        );
        let decoded: CommitSpec = serde_yaml::from_str(&yaml).unwrap();
        assert_eq!(decoded, spec);
    }

    #[test]
    fn commit_spec_parses_v1_yaml_without_target_field() {
        // Verbatim shape an existing v1 prompt emits.
        let yaml = "paths: [\"src/**\"]\nmessage: legacy entry\n";
        let parsed: CommitSpec = serde_yaml::from_str(yaml).unwrap();
        assert_eq!(parsed.paths, vec!["src/**".to_string()]);
        assert_eq!(parsed.message, "legacy entry");
        assert!(parsed.target.is_none());
    }

    #[test]
    fn commits_spec_round_trips_with_schema_version() {
        let spec = CommitsSpec {
            schema_version: COMMITS_SCHEMA_VERSION,
            commits: vec![sample_spec_with_target()],
        };
        let yaml = serde_yaml::to_string(&spec).unwrap();
        let decoded: CommitsSpec = serde_yaml::from_str(&yaml).unwrap();
        assert_eq!(decoded, spec);
    }

    #[test]
    fn commits_spec_parses_v1_yaml_without_schema_version() {
        // Existing v1 prompts write only `commits:` — no schema_version.
        // The default must fill in the current version so the reader
        // succeeds without a migration.
        let yaml = "commits:\n  - paths: [\".\"]\n    message: bump\n";
        let parsed: CommitsSpec = serde_yaml::from_str(yaml).unwrap();
        assert_eq!(parsed.schema_version, COMMITS_SCHEMA_VERSION);
        assert_eq!(parsed.commits.len(), 1);
        assert!(parsed.commits[0].target.is_none());
    }

    #[test]
    fn commits_spec_default_has_current_schema_version_and_empty_list() {
        let file = CommitsSpec::default();
        assert_eq!(file.schema_version, COMMITS_SCHEMA_VERSION);
        assert!(file.commits.is_empty());
    }

    #[test]
    fn commit_spec_target_uses_string_wire_form() {
        // ComponentRef's Serialize/Deserialize emits the
        // `<repo>:<component>` notation, matching the rest of the v2
        // surface (target_requests, this-cycle-focus). The on-disk
        // shape must not become a nested map.
        let spec = sample_spec_with_target();
        let yaml = serde_yaml::to_string(&spec).unwrap();
        assert!(
            yaml.contains("target: ravel-lite:phase-loop"),
            "expected stringified ComponentRef in wire form: {yaml}"
        );
    }
}
