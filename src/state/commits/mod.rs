//! Typed `commits.yaml` surface and read-only CLI verbs.
//!
//! `commits.yaml` is the one-shot work-commit spec authored by
//! analyse-work and consumed by `git-commit-work`. See
//! `docs/architecture-next.md` §Commits and `crate::git::apply_commits_spec`.

pub mod schema;
pub mod verbs;
pub mod yaml_io;

pub use schema::{CommitSpec, CommitsSpec, COMMITS_SCHEMA_VERSION};
pub use verbs::{run_list, run_show, OutputFormat};
pub use yaml_io::{commits_path, delete_commits, read_commits, write_commits};
