//! CLI-facing plan-state commands used by phase prompts.
//!
//! Submodules:
//! - `phase`    — `set-phase` (existing)
//! - `backlog`  — typed backlog.yaml + CRUD verbs (R1)
//! - `migrate`  — one-shot per-plan .md → .yaml conversion (backlog only in R1)

pub mod backlog;
pub mod migrate;
pub mod phase;

pub use phase::run_set_phase;
