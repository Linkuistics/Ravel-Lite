//! Typed `<context>/findings.yaml` surface and CRUD CLI verbs.
//!
//! `findings.yaml` is the context-level inbox of TMS findings.
//! Triage and reflect write to it when they observe something out of
//! scope for the current plan; the user processes the inbox out of
//! band (promote → new plan, file external bug, mark wontfix).
//! Nothing reads `findings.yaml` during plan execution — it is
//! advisory cross-plan, mediated by the user.
//!
//! See `docs/architecture-next.md` §Findings inbox.

pub mod schema;
pub mod verbs;
pub mod yaml_io;

pub use schema::{FindingEntry, FindingsFile, FINDINGS_SCHEMA_VERSION};
pub use verbs::{run_add, run_list, run_set_status, run_show, AddRequest, OutputFormat};
pub use yaml_io::{read_findings, write_findings};
