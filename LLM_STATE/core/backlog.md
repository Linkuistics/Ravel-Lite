# Backlog

## Tasks

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

Note: work-baseline is now seeded atomically in the triage commit
(`git_save_work_baseline` in `GitCommitTriage`), so the baseline SHA
is reliably available when the dirty check runs.

**Results:** _pending_

---
