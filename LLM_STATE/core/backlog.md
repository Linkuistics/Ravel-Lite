# Backlog

## Tasks

### Add terminal-title OSC on phase transitions and pivots

**Category:** `enhancement`
**Status:** `not_started`
**Dependencies:** none

**Description:**

Write `<project-root-basename> <plan-name> <phase>` to the terminal title via
OSC escape sequences at the start of each phase handler in `phase_loop.rs` and
at push/pop sites in `run_stack`.

- Escape sequence: `\033]0;…\007` to stdout; wrap as
  `\033Ptmux;…\033\\` when `$TMUX` is set.
- Do not restore on exit — the next shell prompt overwrites.
- New module `src/term_title.rs` with one public function:
  `set_title(project: &str, plan: &str, phase: &str)`.
- Project name: project root basename (same convention as phase-header render).

Call sites:
- Each `LlmPhase` variant arm in `phase_loop.rs` on entry.
- `run_stack` push and pop sites.

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
