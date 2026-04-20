//! Stack-based pivot mechanism for nested plan execution.
//!
//! When a plan's work phase appends a frame to `<root>/stack.yaml`,
//! the driver pushes the new top onto an in-memory `Vec<PlanContext>`
//! and runs a full cycle of the target plan before popping back.
//!
//! This module owns:
//! - `Frame` / `Stack` — the on-disk schema.
//! - `read_stack` / `write_stack` — file I/O.
//! - `validate_push` — depth cap + cycle detection + target validity.
//! - `decide_after_work` / `decide_after_cycle` — pure state transitions.
//!
//! No tokio, no async. The async driver in `phase_loop.rs` orchestrates.
//! See docs/superpowers/specs/2026-04-20-hierarchical-pivot-design.md.

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

/// Hard compile-time cap on nesting depth. Prevents runaway recursion
/// from a buggy coordinator prompt.
pub const MAX_STACK_DEPTH: usize = 5;

/// One entry in `stack.yaml`. Path is the only required field; `pushed_at`
/// and `reason` are informational and appear in the TUI / session log.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Frame {
    pub path: PathBuf,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pushed_at: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
}

/// The on-disk stack. Ordered; `frames.last()` is the currently-executing plan.
/// A stack with `len <= 1` means "just the root"; the file is normally
/// deleted in that state.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct Stack {
    #[serde(default)]
    pub frames: Vec<Frame>,
}
