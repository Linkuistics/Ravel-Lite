# Backlog

## Tasks

### Add `ravel-lite state` subcommand so prompts mutate phase/stack via CLI

**Category:** `enhancement`
**Status:** `not_started`
**Dependencies:** none

**Description:**

Prompts currently mutate plan state by writing files directly:
`Write "analyse-work" to {{PLAN}}/phase.md` (5 shipped phase prompts),
and for the Ravel coordinator, appending a frame to
`{{PLAN}}/stack.yaml` in `Ravel/LLM_STATE/ravel-orchestrator/prompt-work.md`.
Two problems with the file-write path:

1. **Tool-call overhead per transition.** Claude Code's `Write` tool
   requires a prior `Read` of any existing file. Every phase.md
   transition = `Read` + `Write` = 2 tool calls, plus a permission
   prompt on `Write` unless pre-approved. A `Bash(ravel-lite state *)`
   allowlist entry collapses this to 1 tool call and 0 prompts per
   transition — and there are 4–5 transitions per cycle (analyse-work,
   reflect, dream, triage each advance phase.md, plus work itself).
2. **Structural invariants restated in prose.** The coordinator prompt
   contains a case-analysis the LLM must get right per pivot:
   *"If stack.yaml does not exist, create with root+target. If it
   exists with only root, rewrite with both. If multi-frame, append."*
   That logic lives once in `src/pivot.rs` (`validate_push` +
   `read_stack`/`write_stack`); re-exposing it via a CLI verb lets
   every caller delegate rather than reimplement.

These justifications are orthogonal: `set-phase` cashes in #1,
`push-plan` cashes in #2. Both benefit from living under a shared
`state` namespace so operators add one glob to `.claude/settings.json`.

**Scope (proposed):**

Two subcommands now; `pop-plan` deferred (no LLM-driven caller — pops
are driver-internal, triggered by cycle end or user-declined confirm).

| Subcommand | argv | Callers |
|---|---|---|
| `set-phase` | `ravel-lite state set-phase <plan-dir> <phase>` | 5 shipped prompts: `work.md`, `analyse-work.md`, `reflect.md`, `dream.md`, `triage.md` |
| `push-plan` | `ravel-lite state push-plan <plan-dir> <target-plan-dir> [--reason <s>]` | `Ravel/LLM_STATE/ravel-orchestrator/prompt-work.md` |

**`set-phase` semantics:**
- Validate `<phase>` via `Phase::parse`; reject typos with an error
  listing allowed values.
- Require `<plan-dir>/phase.md` to already exist (prevents silently
  creating a new "plan dir").
- Write atomically (write to tmp, rename).

**`push-plan` semantics:** matches the coordinator prose it replaces:
- `<plan-dir>` is the coordinator's own plan (where `stack.yaml`
  lives); `<target-plan-dir>` is the child to push.
- If `stack.yaml` absent: create with
  `[{path: <plan-dir>}, {path: <target>, reason, pushed_at}]`.
- If present: append `{path: <target>, ...}` as a new last frame.
- Validation via `pivot::validate_push` (cycle detection, depth cap,
  target has `phase.md`).
- `--reason` optional; `pushed_at` set automatically (match the
  timestamp code at `phase_loop.rs:316-318`).

**Design decisions:**
- Positional `<plan-dir>` (matches `run` / `create` convention).
- Binary discoverability: prompts call `ravel-lite`; assume it's on
  `$PATH`. Matches how `claude`/`pi` are invoked elsewhere. No new
  token machinery.
- Allowlist hint: document `Bash(ravel-lite state *)` pattern in the
  migration notes so operators get the tool-call savings.
- No new dependencies — everything needed is in `src/pivot.rs` and
  `src/types.rs` already.

**Deliverables:**

1. `src/main.rs` — new `State` subcommand with `SetPhase` / `PushPlan`
   variants; thin dispatch layer.
2. `src/state.rs` (new) — the two handlers, TDD-style: unit tests for
   validation errors and happy paths; integration test that shells out
   to the built binary and asserts phase.md / stack.yaml contents.
3. Prompt updates (this repo): 5 files in `defaults/phases/*.md`
   replace `Write X to {{PLAN}}/phase.md` with
   `ravel-lite state set-phase {{PLAN}} X`.
4. Out-of-repo: the Ravel coordinator prompt at
   `Ravel/LLM_STATE/ravel-orchestrator/prompt-work.md` should be
   updated by Ravel's maintainer to call `ravel-lite state push-plan`
   — **not part of this task**. We ship the mechanism here.

**Followups / later:**
- `pop-plan` if a prompt ever needs LLM-driven popping.
- Optional `--plan $RAVEL_LITE_PLAN_DIR` env fallback if prompt
  substitution around `{{PLAN}}` turns out to be awkward.
- `{{RAVEL_LITE_BIN}}` token if PATH-based discoverability fails in
  real deployments.

**Results:** _pending_

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

Note: work-baseline is seeded atomically in the triage commit
(`git_save_work_baseline` in `GitCommitTriage`), so the baseline SHA
is reliably available when the dirty check runs.

**Results:** _pending_

---

### Remove Claude Code `--debug-file` workaround once version exceeds 2.1.116

**Category:** `maintenance`
**Status:** `not_started`
**Dependencies:** none

**Description:**

`invoke_interactive` in `src/agent/claude_code.rs` passes
`--debug-file /tmp/claude-debug.log` as a workaround for a TUI
rendering failure in Claude Code ≤2.1.116. The root cause was not
found; debug mode happens to mask it via an unknown upstream mechanism.

When the installed `claude` binary is updated past 2.1.116, remove both
`args.push` lines adding `--debug-file` and `/tmp/claude-debug.log`.
Verify that the Work phase TUI renders correctly without the flag
before closing.

**Results:** _pending_

---
