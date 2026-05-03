//! Typed targets.yaml surface and CRUD CLI verbs.
//!
//! `targets.yaml` is the runtime mount record for a plan: which
//! components are projected into the plan as worktrees on which
//! plan-namespaced branches. Born when the runner mounts the first
//! worktree; drained at plan finish. See `docs/architecture-next.md`
//! §Targets and worktrees.

pub mod mount;
pub mod schema;
pub mod verbs;
pub mod yaml_io;

pub use mount::mount_target;
pub use schema::{Target, TargetsFile, TARGETS_SCHEMA_VERSION};
pub use verbs::{run_add, run_list, run_remove, run_show, AddRequest};
pub use yaml_io::{
    mounted_worktree_add_dirs, read_targets, resolve_target_worktree, targets_path, write_targets,
};
