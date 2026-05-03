//! Typed `target-requests.yaml` surface, CRUD verbs, and phase-boundary
//! drain.
//!
//! `target-requests.yaml` is the request queue between plan creation /
//! work-phase intent and the runner's `mount_target` machinery: it
//! records "the LLM (or `ravel-lite create`) wants this component
//! mounted next; please drain it at the next phase boundary". See
//! `docs/architecture-next.md` §Dynamic mounting and §Phase boundaries.

pub mod drain;
pub mod schema;
pub mod verbs;
pub mod yaml_io;

pub use drain::drain_target_requests;
pub use schema::{TargetRequest, TargetRequestsFile, TARGET_REQUESTS_SCHEMA_VERSION};
pub use verbs::{run_add, run_list, run_remove, run_show};
pub use yaml_io::{
    delete_target_requests, read_target_requests, target_requests_path, write_target_requests,
};
