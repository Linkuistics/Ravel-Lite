//! Typed `focus-objections.yaml` surface and CRUD CLI verbs.
//!
//! The work phase writes objections here when triage's selected focus
//! turns out to be wrong; the next triage drains the file at the start
//! of its run. See `docs/architecture-next.md` §WORK and §TRIAGE for
//! the contract.

pub mod schema;
pub mod verbs;
pub mod yaml_io;

pub use schema::{
    FocusObjectionsFile, Objection, FOCUS_OBJECTIONS_SCHEMA_VERSION,
};
pub use verbs::{
    run_add_premature, run_add_skip_item, run_add_wrong_target, run_clear, run_list, OutputFormat,
};
pub use yaml_io::{
    delete_focus_objections, focus_objections_path, read_focus_objections, write_focus_objections,
};
