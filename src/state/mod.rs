//! CLI-facing plan-state commands used by phase prompts.
//!
//! Submodules:
//! - `phase` — `set-phase` (existing)
//! - `backlog` — typed backlog.yaml + CRUD verbs (R1)
//! - `intents` — typed intents.yaml + minimal CRUD verbs; canonical
//!   intent source under the architecture-next plan KG
//! - `memory` — typed memory.yaml + per-entry CRUD verbs (R2)
//! - `session_log` — typed session-log.yaml + latest-session.yaml
//!   verbs, plus the programmatic append used by
//!   `phase_loop::GitCommitWork` (R3)
//! - `targets` — typed targets.yaml runtime mount records and CRUD
//!   verbs; the data layer for architecture-next's per-repo plan
//!   branches and dynamic worktree mounts
//! - `target_requests` — typed target-requests.yaml scratch queue,
//!   CRUD verbs, and the phase-boundary `drain_target_requests`
//!   function the runner calls between phases
//! - `discover_proposals` — typed-CLI façade for `discover-proposals.yaml`
//!   so Stage 2's LLM emits each proposal via `add-proposal` rather than
//!   writing YAML, letting clap reject a hallucinated `--kind` on the
//!   single bad call instead of the whole batch
//! - `migrate` — one-shot per-plan .md → .yaml conversion
//!   (backlog + memory + session-log/latest-session)

pub mod backlog;
pub mod discover_proposals;
pub mod filenames;
pub mod findings;
pub mod focus_objections;
pub mod intents;
pub mod memory;
pub mod migrate;
pub mod phase;
pub mod session_log;
pub mod target_requests;
pub mod targets;
pub mod this_cycle_focus;

pub use phase::run_set_phase;
