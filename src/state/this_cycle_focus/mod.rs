//! Typed `this-cycle-focus.yaml` surface and CRUD CLI verbs.
//!
//! `this-cycle-focus.yaml` is the one-shot focus record written by
//! triage and consumed by work — see `docs/architecture-next.md` §TRIAGE
//! step 6 and §WORK.

pub mod schema;
pub mod verbs;
pub mod yaml_io;

pub use schema::{ThisCycleFocus, THIS_CYCLE_FOCUS_SCHEMA_VERSION};
pub use verbs::{run_clear, run_set, run_show};
pub use yaml_io::{
    delete_this_cycle_focus, read_this_cycle_focus, this_cycle_focus_path, write_this_cycle_focus,
};
