# Backlog

## Tasks

### Extract shared spawn/stream/dispatch boilerplate to `src/agent/common.rs`

**Category:** `refactor`
**Status:** `done`
**Dependencies:** none (Capture and surface pi subprocess stderr on non-zero exit — done)

**Description:**

`src/agent/claude_code.rs` (492 LOC) and `src/agent/pi.rs` (510 LOC)
duplicate significant boilerplate around process spawn, stdout line
reading, stderr-tail buffering, subagent dispatch scaffolding, and
UIMessage emission patterns. The duplication is a drift source — e.g.
one agent getting an `anyhow::bail!` improvement that the other
silently misses.

Identify genuinely shared logic (process spawn helper, stdout
line-read loop that forwards to the right formatter, stderr drain +
tail buffer with size cap, dispatch pattern scaffolding) and lift
into a new `src/agent/common.rs`. Leave agent-specific surface — CLI
flag construction, JSON event parsing (different schemas between the
two agents) — in the concrete `*.rs` files.

`STDERR_BUFFER_CAP` and `warning_line` are currently duplicated across
`pi.rs` and `claude_code.rs` with comments pointing here. Full
regression coverage exists on both sides via `pi_integration` tests
and existing `ClaudeCodeAgent` test surface; the refactor can proceed
safely.

**Results:**

Created `src/agent/common.rs` as the single home for post-spawn
plumbing shared by both streaming agents. Moved in: `STDERR_BUFFER_CAP`,
`STREAM_SNIPPET_BYTES`, `StreamLineOutcome`, `truncate_snippet`,
`warning_line`, `spawn_stderr_drain`, `pump_stdout_to_ui`,
`build_dispatch_plan_context`, and `run_streaming_child` — the last
being the atomic unit that makes drift impossible: both `invoke_headless`
implementations now delegate their entire post-spawn flow (pump, wait,
drain, AgentDone, exit-error shape) through one function.

Drew the shared/agent-specific boundary at `Command::spawn()`. Argv
construction stays per-agent (flags, prompt composition, model/thinking
knobs genuinely differ). Stream-JSON parsing stays per-agent (schemas
differ: claude emits `type:"assistant"` + nested `tool_use` blocks; pi
emits `tool_execution_start`). Both parsers now share a
`ParseLineFn = fn(&str, Option<LlmPhase>, &mut HashSet<String>) -> StreamLineOutcome`
type alias and are handed to the pump as function pointers.

Converted `parse_pi_stream_line` from `Option<FormattedOutput>` →
`StreamLineOutcome`, matching the memory directive "Apply this pattern
wherever an `Option` return collapses two semantically distinct
outcomes". Falls-out behavior improvement: pi now surfaces malformed
stream-JSON as a Persist warning instead of silently dropping the
line — the exact class of bug the enum was introduced to prevent on
the claude side. Added `parse_pi_malformed_json_surfaces_snippet` test
pinning the new behavior.

Verification: `cargo test` → 145 lib tests + 13 integration tests pass,
including all three `pi_integration` contract tests
(`pi_phase_cycle_substitutes_tokens_and_streams_events`,
`pi_invoke_headless_surfaces_stderr_tail_on_failure`,
`pi_dispatch_subagent_invokes_pi_with_target_plan_args`). `cargo clippy
--tests` flags zero issues in the three touched agent files (the
pre-existing warnings in `src/format.rs` and `src/survey/render.rs`
are unrelated).

Touched files: `src/agent/common.rs` (new, ~230 LOC), `src/agent/mod.rs`
(declare module), `src/agent/claude_code.rs` (~490 → ~310 LOC),
`src/agent/pi.rs` (~510 → ~450 LOC; `setup()` accounts for most of the
residual size — no analogue on the claude side to share with).

Suggests next: the `warn_if_project_tree_dirty` narrowing task is now
the only backlog item. Beyond that, if the common module grows further
(e.g. extracting shared `is_dangerous`-style config lookups), consider
a `CommandBuilder` helper — but not yet; speculative abstraction risk.

---

### Narrow `warn_if_project_tree_dirty` to work-agent-touched files only

**Category:** `enhancement`
**Status:** `not_started`
**Dependencies:** none

**Description:**

`warn_if_project_tree_dirty` at `phase_loop.rs:94` is pathspec-unscoped
— it fires on any dirty file in the project tree. In monorepos with
multiple plans the check can still produce false positives from sibling
plans' in-flight writes, even after the atomic phase-transition fix.

Narrow the check to: compute `git diff --name-only <work_baseline>`
(files changed since the work baseline) intersected with the current
dirty list, so the warning only fires on files the active work agent
could plausibly have touched. This is a defense-in-depth refinement;
no correctness regression possible since the current check is strictly
more noisy, never more accurate.

**Results:** _pending_

---
