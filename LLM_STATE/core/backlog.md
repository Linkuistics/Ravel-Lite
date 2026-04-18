# Backlog

## Tasks

### Decide pi agent scope: complete the port or mark it aspirational

**Category:** `meta`
**Status:** `not_started`
**Dependencies:** none

**Description:**

Multiple audit findings point to pi being a visibly less-polished
sibling to claude-code:

- Unresolved `{{MEMORY_DIR}}` in `memory-prompt.md`. Now that
  `substitute_tokens` hard-errors on unresolved tokens, pi invocation
  **fails immediately** rather than silently corrupting instructions —
  resolving this is no longer deferred cleanup, it is a hard blocker
  on using pi at all.
- stderr not captured on failure (no tail in error messages).
- Older default model (`claude-opus-4-6`) in
  `defaults/agents/pi/config.yaml` vs claude-code's more current
  `claude-sonnet-4-6` / haiku variants.
- No integration test exercises the pi agent path.

Pick a direction: either invest in parity (and cover it in tests +
docs) or mark pi explicitly aspirational in `README.md` /
`docs/architecture.md` so future readers don't assume drop-in
equivalence. If we commit to parity, extract the genuinely shared
spawn/stream/dispatch boilerplate from `claude_code.rs` and `pi.rs`
into `src/agent/common.rs` as part of that effort.

**Results:** _pending_

---

### Resolve or remove `{{MEMORY_DIR}}` token in pi memory prompt

**Category:** `bug`
**Status:** `not_started`
**Dependencies:** Decide pi agent scope

**Description:**

`defaults/agents/pi/prompts/memory-prompt.md` references `{{MEMORY_DIR}}`
at three sites (lines ~3, 61, 74) but `PiAgent::load_prompt_file`
(src/agent/pi.rs:~142) only substitutes `{{PROJECT}}`, `{{DEV_ROOT}}`,
and `{{PLAN}}`. Previously the literal `{{MEMORY_DIR}}` passed through
to the LLM unchanged; now that `substitute_tokens` hard-errors on
unresolved tokens, pi invocation fails immediately on any phase that
loads this prompt.

Decide whether memory lives in a distinct directory from the plan (if
so, thread `MEMORY_DIR` through `PlanContext` and the pi token map) or
rewrite the prompt to use `{{PLAN}}` and drop the placeholder. Also
grep the prompt for any other dangling `{{...}}` while you're there.

**Results:** _pending_

---

### Capture and surface pi subprocess stderr on non-zero exit

**Category:** `bug`
**Status:** `not_started`
**Dependencies:** Decide pi agent scope

**Description:**

`PiAgent::invoke_headless` (src/agent/pi.rs:~236) uses
`stderr(Stdio::inherit())`, which lets pi's error output bypass the TUI
log and bleed into the raw terminal during a headless phase — often
overwritten immediately by later TUI repaints. `ClaudeCodeAgent`
(src/agent/claude_code.rs:~199) already pipes stderr, accumulates a
tail buffer, and includes the tail in its `anyhow::bail!` on failure.

Port the same pattern to `PiAgent`. When touching the code, consider
extracting `drain_stderr_into_buffer(..)` into a shared helper (e.g.
`src/agent/common.rs`) so the two copies can't drift.

**Results:** _pending_

---

### Reliably mark completed backlog items as `done`

**Category:** `bug`
**Status:** `done`
**Dependencies:** none

**Description:**

Completed tasks are unreliably transitioning from `Status:
not_started` (or `in_progress`) to `Status: done` in `backlog.md`.
The work-phase prompt currently says only "Record results on the
task in `{{PLAN}}/backlog.md`: what was done, what worked, what
didn't, what this suggests next." (`defaults/phases/work.md:76-78`).
It never explicitly instructs the agent to flip the `Status:` line.
When the model focuses on writing the `Results:` block, the status
field silently stays stale — which then misleads triage into
treating a finished task as still open, wasting a future cycle or
(worse) causing duplicate work.

Two complementary fixes to implement together:

1. **Tighten `defaults/phases/work.md`.** Change step 7's wording so
   it names the status transition as a required part of the
   recorded result: e.g. "Update the task's `Status:` line to
   `done` (or `blocked` with a reason) and write a `Results:`
   block beneath it covering what was done, what worked, what
   didn't, and what this suggests next." Make the status update
   the *first* sub-bullet so a hurried model sees it even if it
   skims.

2. **Add a safety net in `defaults/phases/analyse-work.md`.**
   Analyse-work already reads `backlog.md` (step 4) and the diff
   (steps 2-3). Extend it: after determining the session produced
   a non-empty `Results:` block on a task whose `Status:` is still
   `not_started` or `in_progress`, flip the status to `done`
   before writing `latest-session.md`. Describe this as a
   post-condition check, not a judgement call — the diff is
   authoritative, and analyse-work runs against the diff.

Also add integration coverage: extend (or parallel-copy)
`phase_contract_round_trip_writes_expected_files` so the
`ContractMockAgent`'s analyse-work branch writes a `Results:`
block on a task that was `not_started` and leaves its `Status:`
line unchanged; the test then asserts that after analyse-work
runs, the status is `done`. That pins the safety-net behaviour.

**Suggests next:** If the tightened work-phase prompt alone closes
the gap in practice, the analyse-work safety net can be left as
belt-and-braces. If the gap persists, the third lever is a
`Status:`-flip check in `warn_if_project_tree_dirty`-style code
inside `phase_loop`, surfaced as a TUI warning — but that's a
bigger change and should only follow if prompt-level fixes fail.

**Results:**

All three sub-tasks implemented together:

1. **`defaults/phases/work.md` step 7 rewritten.** Now frames
   backlog updates as two required parts, with the `Status:` flip
   as the explicit first sub-bullet (plus an inline "required, not
   optional" reminder and a one-line rationale explaining why stale
   statuses mislead triage).
2. **`defaults/phases/analyse-work.md` extended with a new step 5
   safety-net.** Inserts a post-condition check between "read
   backlog.md" (step 4) and "determine session number" (now step
   6): any task with a non-empty `Results:` block but a stale
   `Status:` line is flipped to `done`. Framed as a diff-driven
   post-condition, not a judgement call — matches the task's
   wording. Subsequent steps renumbered 6–10.
3. **`phase_contract_round_trip_writes_expected_files` parallel-copied
   as `analyse_work_flips_stale_task_status_per_safety_net`** in
   `tests/integration.rs`. Pre-seeds backlog with a
   `Status: not_started` task whose `Results:` block is non-empty,
   runs `phase_loop` starting at analyse-work, declines every
   confirm so the loop exits right after `git-commit-work` (isolates
   the assertion from Triage's backlog rewrite), and asserts the
   Status line has been flipped to `done`. Extends
   `ContractMockAgent::AnalyseWork` with a `flip_stale_task_statuses`
   helper that mirrors the safety-net behaviour on `---`-delimited
   task blocks. The helper is a no-op on the original test's empty
   backlog, so `phase_contract_round_trip_writes_expected_files`
   continues to pass unchanged.

**What worked:** TDD — the new test failed with the expected
"still `not_started`" message on first run, then passed after the
mock helper landed. All 130 unit tests + 7 integration tests pass.
Clippy shows 10 pre-existing errors on `HEAD`; diff introduces no
additional clippy findings.

**What didn't:** Initial safety-net prompt was renumbered only
partially (two adjacent "7." entries after the insertion); fixed in
a second edit pass. Worth watching in future prompt insertions —
Markdown auto-number-renumbering is an LLM-reliability foot-gun.

**Suggests next:** The analyse-work safety-net relies on an LLM
correctly parsing `---`-delimited task blocks and `**Results:**`
markers. If field reports show the safety-net still missing stale
statuses, the third lever — an orchestrator-level
`warn_if_stale_task_status_after_work` check in Rust (mirroring
`warn_if_project_tree_dirty`) — becomes the follow-up. That
escalation path is pre-identified in the "Suggests next" block
above and intentionally deferred until we have evidence the prompt
lever isn't enough.

---

### Move source-commit authority from work phase to analyse-work

**Category:** `enhancement`
**Status:** `done`
**Dependencies:** none

**Description:**

Work-phase source commits were unreliable: the LLM sometimes
skipped the "commit your source edits" step, leaving dirty state
for later phases to stumble over (the `warn_if_project_tree_dirty`
check surfaced this but didn't prevent it). And even when the work
phase did commit, the commit-message narrative was split across
two authors (work-phase ad-hoc message + analyse-work's
`commit-message.md` for plan state), which is messy.

Move commit authority to analyse-work. Orchestrator captures a
fresh `git status --porcelain` + `git diff --stat <baseline>`
snapshot the moment analyse-work is invoked, injects it as a
`{{WORK_TREE_STATUS}}` token in the analyse-work prompt, and the
LLM stages + commits every path outside `{{PLAN}}/` (or explicitly
justifies leaving a path uncommitted, which flows into
`latest-session.md`). The plan-state commit via `git-commit-work`
continues to use `commit-message.md` — two commits per session,
each with a focused narrative.

**Results:**

Implemented as Option B from the design discussion (prompt-level
commit authority with orchestrator-assisted status injection, not
an orchestrator-driven deterministic commit). Changes:

1. **`src/git.rs`** — added `work_tree_snapshot(project_dir,
   baseline_sha)` which composes a human-readable snapshot with
   labelled `git diff --stat <baseline>` and `git status
   --porcelain` sections. Soft-fails (returns an explanatory
   string, not `Result::Err`) so `compose_prompt` can never wedge
   the loop on a transient git error. Two unit tests cover a
   dirty tree (tracked edit + untracked file) and a clean tree
   (explicit empty-state markers).
2. **`src/phase_loop.rs`** — when the loop enters
   `LlmPhase::AnalyseWork`, reads `work-baseline`, clones the
   agent's tokens map, and inserts `WORK_TREE_STATUS`. Snapshot
   captured at `compose_prompt` time (not at work-phase exit) so
   any hand-edits the user makes between work exit and
   analyse-work start are included.
3. **`defaults/phases/analyse-work.md`** — new "Work-tree
   snapshot" section near the top rendering the `{{WORK_TREE_STATUS}}`
   token verbatim, and a new step 6 instructing the LLM to stage +
   commit every path outside `{{PLAN}}/` with a descriptive
   message, or to justify each skipped path explicitly in
   `latest-session.md`. Subsequent steps renumbered 7–11, and the
   intro/role text updated to reflect the new responsibility.
4. **`defaults/phases/work.md`** — step 8 inverted from "commit
   your source-file changes yourself" to "do NOT commit source
   changes; analyse-work owns that". Preserves the option to run
   `git status` for orientation.
5. **`tests/integration.rs`** — `ContractMockAgent` extended with
   a `captured_prompts` map (for prompt-substitution assertions)
   and an opt-in `commit_project_dir` that simulates a well-behaved
   LLM staging + committing non-plan paths. New test
   `analyse_work_receives_snapshot_and_commits_uncommitted_source`
   pre-seeds an uncommitted source file, runs the phase loop, and
   asserts: the analyse-work prompt has no leftover `{{WORK_TREE_STATUS}}`
   placeholder; the snapshot surfaces the file path; the mock
   committed it; and the git log shows both a source commit and a
   distinct plan-state commit.

**What worked:** 140 tests pass (132 lib + 8 integration); no
clippy regressions (10 pre-existing errors, same count before and
after). The existing `phase_contract_round_trip_writes_expected_files`
test continued to pass unmodified — the prompt-capture addition
and the opt-in source-commit simulation are backward-compatible by
construction.

**What didn't:** Initial attempt had the source-commit step after
`latest-session.md` was written; that forced the LLM to "plan"
justifications before the commit. Swapped the order so the commit
happens FIRST and any justifications feed naturally into the
session log's "What didn't work" section. Also worth noting: the
`warn_if_project_tree_dirty` check already fires after
`git-commit-work` — it's the existing safety net for an
analyse-work that skips a commit despite the prompt, so no new
orchestrator-level enforcement was needed for this round.

**Suggests next:** Two observations worth capturing as follow-up
tasks if the cycle produces evidence they matter:

- If LLMs routinely justify away commits they should have made,
  escalate to Option C (script-phase `git-commit-source`
  that unconditionally commits every non-plan path). The
  `warn_if_project_tree_dirty` logs are the signal to watch.
- The source-commit message is freeform; consider whether a
  `source-commit-message.md` file (mirror of `commit-message.md`)
  would tighten the contract and let the commit be moved into a
  script phase later without prompt-rewriting the LLM step.

---
