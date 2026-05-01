//! Typed targets.yaml surface and CRUD CLI verbs.
//!
//! `targets.yaml` is the runtime mount record for a plan: which
//! components are projected into the plan as worktrees on which
//! plan-namespaced branches. Born when the runner mounts the first
//! worktree; drained at plan finish. See `docs/architecture-next.md`
//! §Targets and worktrees.

pub mod schema;
pub mod verbs;
pub mod yaml_io;

pub use schema::{Target, TargetsFile, TARGETS_SCHEMA_VERSION};
pub use verbs::{run_add, run_list, run_remove, run_show, AddRequest, OutputFormat};
pub use yaml_io::{read_targets, targets_path, write_targets};
