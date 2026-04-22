//! CLI-facing plan-state commands used by phase prompts.
//!
//! Submodules:
//! - `phase`    — `set-phase` (existing)
//! - `backlog`  — typed backlog.yaml + CRUD verbs (R1)
//! - `memory`   — typed memory.yaml + per-entry CRUD verbs (R2)
//! - `migrate`  — one-shot per-plan .md → .yaml conversion (backlog + memory)

pub mod backlog;
pub mod memory;
pub mod migrate;
pub mod phase;

pub use phase::run_set_phase;
