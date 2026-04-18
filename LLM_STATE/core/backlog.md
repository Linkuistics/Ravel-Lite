# Backlog

## Tasks

### Commit plan-state transitions atomically with their script phases

**Category:** `bugfix`
**Status:** `done`
**Dependencies:** none

**Description:**

The four `GitCommit*` script-phase handlers in `src/phase_loop.rs`
(`handle_script_phase`, lines 128-163) all follow the pattern
`git_commit_plan → write_phase(next) → ask user`. Writing `phase.md`
*after* the commit leaves it dirty at the user-prompt point. In
monorepos where multiple plans coexist (e.g. APIAnyware-MacOS with
`LLM_STATE/core/` + `LLM_STATE/targets/racket-oo/`), a stale `phase.md`
from the interrupted plan surfaces as a false positive in
`warn_if_project_tree_dirty` when the other plan's work cycle runs.

Similarly, the `LlmPhase::Work` entry block (lines 199-202) writes
`work-baseline` and deletes `latest-session.md` with no commit — those
files stay dirty through the entire active work + analyse-work LLM
phases, committed only by the eventual `git-commit-work`. They add to
the same cross-plan false-positive surface.

Fix:

1. In every `GitCommit*` handler: call `write_phase(next)` **before**
   `git_commit_plan`. The commit then captures the phase advance
   atomically; the tree is clean at the user prompt.
2. In `GitCommitTriage` additionally perform `git_save_work_baseline`
   and `fs::remove_file("latest-session.md")` **before**
   `git_commit_plan`. These writes become part of the triage commit.
   Retain a first-run fallback at `LlmPhase::Work` entry that creates
   `work-baseline` only when missing, for fresh plans that start at
   `work` without a preceding triage.

Semantic note: after fix (2), `work-baseline` captures HEAD from
*before* the triage commit rather than after. The analyse-work
snapshot's `git diff --stat` therefore includes the triage commit's
plan-dir edits. That's informational only — analyse-work's
commit-untracked-source logic reads `git status --porcelain`, not the
stat.

TDD: pin the invariant with tests that assert `git status --porcelain`
is empty after `phase_loop` returns from both `git-commit-triage` and
`git-commit-work` user-declined exits.

**Results:**

Fix (1) applied in `src/phase_loop.rs:handle_script_phase`: all four
`ScriptPhase::GitCommit*` handlers now call `write_phase(next)` before
`git_commit_plan`. The `GitCommitReflect` branch needed a small
restructuring because it conditionally writes one of two next phases
(Dream vs Triage) and then renders a skip banner — hoisted the
`should_dream` check so the write happens first and the banner still
renders only in the skip path.

Fix (2) applied in the same place: `GitCommitTriage` now also calls
`git_save_work_baseline` before committing, capturing `work-baseline`
atomically with the triage transition. `LlmPhase::Work` entry was
reduced to a **first-run fallback**: it only seeds `work-baseline`
when missing (fresh plan starting at `work` with no preceding triage).

Deviation from the original plan: the `latest-session.md` deletion was
**not** moved into `GitCommitTriage` (and was dropped entirely from
`LlmPhase::Work` entry). Rationale: `phase_contract_round_trip_writes_expected_files`
surfaced that the prior `fs::remove_file(latest-session.md)` at Work
entry was decorative — analyse-work's prompt (step 8) already
overwrites the file unconditionally. Keeping it in place through
the triage commit preserves the prior session's log on disk and in
git history for operator inspection in the idle gap between cycles,
and eliminates the "D latest-session.md" transient dirt during work
phases. Net: less dirt, same correctness.

Semantic shift on `work-baseline`: now captures HEAD from **before**
the triage commit rather than after (since `git_save_work_baseline`
runs pre-commit in the handler). `work_tree_snapshot`'s `git diff
--stat` in analyse-work's prompt therefore includes the triage
commit's plan-dir edits (backlog tweaks) alongside work-phase source
changes. This is informational only — analyse-work's
commit-untracked-source logic reads `git status --porcelain`, which is
unaffected.

TDD artefacts:
- `tests/integration.rs::git_commit_triage_leaves_plan_tree_clean_at_user_prompt`
- `tests/integration.rs::git_commit_work_leaves_plan_tree_clean_at_user_prompt`

Both assert `git status --porcelain -- <plan_dir>` is empty after
`phase_loop` returns from a user-declined exit. RED verified (both
failed with `M plans/.../phase.md` before the fix); GREEN verified
after both fixes applied. Full suite: 10/10 integration, 144/144
unit, zero new clippy warnings (pre-existing warnings in
`src/format.rs` and `src/survey/render.rs` untouched).

Next-step suggestions: (a) the `warn_if_project_tree_dirty` check at
`phase_loop.rs:94` is now substantially less noisy, but still
pathspec-unscoped — a follow-up could narrow it to
`git diff --name-only <work_baseline>` intersected with the dirty
list, so the warning only fires on files the work agent could have
touched (defense-in-depth against the remaining sibling-plan edge
cases). (b) Consider whether `git_save_work_baseline` should be
idempotent across repeated `GitCommitTriage` runs (it is, but the
semantic capture-point could be documented in a module-level comment).

---

### Add integration test exercising the pi phase path

**Category:** `test`
**Status:** `not_started`
**Dependencies:** none (Capture and surface pi subprocess stderr on non-zero exit — done)

**Description:**

The existing `phase_contract_round_trip_writes_expected_files` test
runs `phase_loop` with `ContractMockAgent` — it verifies the phase
state machine writes the right files but does not exercise either the
`ClaudeCodeAgent` or `PiAgent` concrete implementations. As a result,
changes like `substitute_tokens` hard-erroring on unresolved tokens
broke pi end-to-end without a single test failing.

Add a contract-level test that runs a phase cycle with `PiAgent` as
the agent, using a mock `pi` binary on PATH (or a test double) that
emits a scripted stream of JSON events. At minimum the test should
cover: prompt loading resolves every `{{…}}` token (catches
`{{MEMORY_DIR}}`-class regressions), stream parsing maps events to
`UIMessage` variants correctly, stderr-tail appears in error messages
on non-zero exit (so the stderr capture fix stays fixed), and dispatch
invokes the right args for the target plan.

**Results:** _pending_

---

### Extract shared spawn/stream/dispatch boilerplate to `src/agent/common.rs`

**Category:** `refactor`
**Status:** `not_started`
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
`pi.rs` and `claude_code.rs` with comments pointing here. The pi
integration test (task above) forms the regression net for this
refactor alongside the existing `ClaudeCodeAgent` test surface.

**Results:** _pending_

---
