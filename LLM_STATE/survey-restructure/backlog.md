# Backlog

## Tasks

### 5b — Incremental survey via `--prior`

**Category:** `feature`
**Status:** `not_started`
**Dependencies:** 5a ✓ (canonical YAML round-trip + `input_hash` field complete)

**Description:**

Add a `--prior <file>` flag to `ravel-lite survey` so a prior YAML
survey can seed an incremental run: compare per-plan `input_hash`
values, send only changed+added plans to the LLM, and merge the delta
with unchanged rows from the prior. Per-cycle survey cost becomes
proportional to what actually changed — a precondition for 5c's
every-cycle survey to be affordable. See `docs/survey-pivot-design.md`
§5b.

**Deliverables:**

1. `--prior <file>` flag on `survey`. Parse the prior YAML; classify
   each plan as `unchanged` / `changed` / `removed` / `added` by
   comparing freshly-computed `input_hash` values against the prior.
2. Delta-aware `render_survey_input` in `src/survey/compose.rs`: only
   changed+added plans appear in the LLM payload. The prior survey
   is carried in full as context so the LLM can revisit cross-plan
   blockers and parallel streams when deltas affect them.
3. Merge logic in `run_survey`: LLM delta + prior-unchanged rows →
   final `SurveyResponse`. Validation refuses a delta that mutates a
   plan outside the declared changed set (mirrors `inject_input_hashes`
   hard-error pattern).
4. `--force` bypass flag: re-analyses everything regardless of hash
   match. For debugging and schema-bump paths.
5. Prompt strategy — settle during implementation; lean: two prompts
   (`defaults/survey.md` cold, `defaults/survey-incremental.md` warm)
   beats one with conditional branches. Embed via
   `src/init.rs::EMBEDDED_FILES`; preserve drift-guard coverage.
6. Add `schema_version: u32` to `SurveyResponse` with
   `#[serde(default = "default_schema_version")]` so 5a-emitted YAML
   without the marker still parses once 5b lands. Mismatched-version
   `--prior` either fails fast with a remediation hint or
   auto-falls-back to `--force`-equivalent behaviour.
7. Tests: unchanged-plan reuse, changed-plan re-analysis,
   removed-plan pruning, added-plan detection, schema-bump
   invalidation, `--force` path, validation-rejects-delta-outside-
   changed-set.

**Results:** _pending_

---

### 5d — Remove `stack.yaml`, `push-plan` CLI, `pivot.rs`, and `run_stack`

**Category:** `enhancement`
**Status:** `done`
**Dependencies:** none. Recommended: run before 5c so 5c is built
against clean runner architecture. Can run in parallel with 5b.

**Description:**

Delete the infrastructure that supported the LLM-authored
coordinator-plan concept: the `push-plan` CLI verb, `pivot.rs` in
its entirety, `stack.yaml` I/O, and the `run_stack` wrapper (which
collapses back to a straightforward single-plan run loop). See
`docs/survey-pivot-design.md` §5d for scope and the external-impact
note about the out-of-repo Ravel orchestrator at
`{{DEV_ROOT}}/Ravel/LLM_STATE/ravel-orchestrator/`.

**Deliverables:**

1. Remove the `ravel-lite state push-plan` subcommand from
   `src/main.rs`; remove `run_push_plan` from `src/state.rs` along
   with its tests.
2. Delete `src/pivot.rs` in its entirety (`validate_push`,
   `push_timestamp`, `decide_after_work`, `decide_after_cycle`, and
   the `Frame`/`Stack` types). If `push_timestamp()`'s format is
   genuinely needed elsewhere, extract it to a small utility module;
   otherwise delete.
3. Collapse `run_stack` in `src/phase_loop.rs` back to a simple
   wrapper that loops `phase_loop` with the existing
   continue-or-exit user prompt. Rename appropriately (e.g.
   `run_single_plan`) — "stack" terminology is no longer meaningful.
4. Remove all `stack.yaml` I/O paths: reads, writes, validation,
   sync-to-disk logic, file-format parser.
5. Remove tests for pivot state machines, stack serialisation, and
   push-plan validation.
6. Grep `src/`, `defaults/`, `tests/` for remaining references
   (`stack.yaml`, `push-plan`, `pivot::`, `Frame`, `Stack`) and clean
   them up. Obsolete memory entries in `LLM_STATE/core/memory.md`
   are pruned by the next core triage cycle, not by this task.

**Results:**

Done. Net deletion: 1,441 lines across `src/` and `tests/`.

- `src/pivot.rs` — deleted in entirety (267 lines: `Frame`,
  `Stack`, `read_stack`, `write_stack`, `validate_push`,
  `decide_after_work`, `decide_after_cycle`, `frame_to_context`,
  `push_timestamp`, `MAX_STACK_DEPTH`, `find_project_root`).
  Removed from `src/lib.rs` module list.
- `src/main.rs` — `mod pivot` line removed; `StateCommands::PushPlan`
  variant + dispatch arm removed; `run_stack` call renamed to
  `run_single_plan`. (-15 lines net.)
- `src/state.rs` — `run_push_plan` and its 5 unit tests deleted;
  `pivot` import removed; module doc updated to single-verb framing.
  (-141 lines.)
- `src/phase_loop.rs` — pivot helpers (`raw_phase_label`,
  `set_title_for_context`, `format_breadcrumb`,
  `log_phase_header_with_breadcrumb`, `sync_stack_to_disk`,
  `on_disk_stack_len`, `on_disk_new_top`, `stack_snapshot`,
  `do_push`, `build_prompt`) all deleted. `run_stack` collapsed to
  a 9-line `run_single_plan` that delegates to `phase_loop`.
  `#[allow(dead_code)]` on `phase_loop` removed (it's now the live
  entry point). `pivot` import dropped. (-261 lines.)
- `tests/integration.rs` — entire pivot test region (lines 1612-2369,
  ~760 lines) deleted: 6 type/serde tests, 5 validate_push tests, 4
  decide_after_work tests, 4 decide_after_cycle tests, 2
  frame_to_context tests, 3 breadcrumb tests, 2 run_stack tests
  (single-plan + short-circuit pivot), and the
  `state_set_phase_and_push_plan_via_binary` test. The
  `state_set_phase_rejects_invalid_phase_via_binary` test is retained
  as the binary-boundary coverage; `state.rs` unit tests still cover
  the in-process success path. Stale `AtomicUsize`/`Ordering` imports
  pruned from the file header.

`cargo build` clean. `cargo test`: 178 unit + 17 integration + 0 doc
tests pass; no regressions. `cargo clippy` shows 6 doc-formatting
errors in `src/survey/schema.rs` — confirmed pre-existing on the
unmodified tree, out of scope.

`run_single_plan` retained as a one-line delegate (not removed
outright) so the multi-plan branching for 5c has an obvious seam to
expand in `main::run_phase_loop` without re-plumbing.

Out of scope per the deliverable: `LLM_STATE/core/memory.md` entries
about pivot/stack/run_stack remain — flagged for the next core
triage cycle. `docs/survey-pivot-design.md` is reference material
and untouched. The out-of-repo Ravel orchestrator at
`{{DEV_ROOT}}/Ravel/LLM_STATE/ravel-orchestrator/` will break on
next invocation as expected; migration is separate.

What this suggests next: 5b is now unblocked, and 5c is unblocked
modulo 5b. The clean `phase_loop`/`run_single_plan` seam means 5c
can branch on plan-count in `main::run_phase_loop` with minimal
churn — single-plan path is the existing call; multi-plan path
introduces the survey-routed dispatch loop.

---

### 5c — Multi-plan `run` mode with survey-driven routing

**Category:** `feature`
**Status:** `not_started`
**Dependencies:** 5b (incremental survey for affordable per-cycle
invocation); 5d recommended first (clean runner architecture)

**Description:**

Turn `ravel-lite run` into a multi-plan orchestrator when given N
positional plan-dir args. At the top of every cycle, run an
incremental survey over all N plans, present the top-ranked plans to
the user via a minimal stdout prompt, and dispatch one phase cycle of
the user's choice before looping back. Replaces the LLM-driven
coordinator concept with a code-driven routing loop. See
`docs/survey-pivot-design.md` §5c.

**Deliverables:**

1. `run` accepts `N > 1` positional plan dirs. `N == 1` remains
   exactly as today (no survey, no state file, unchanged behaviour).
2. New required flag for `N > 1`: `--survey-state <path>`. Rejected
   when `N == 1`. The file is both output (written at cycle end) and
   input (read as `--prior` next cycle via 5b's incremental path).
3. Run-loop shape: **survey → select → dispatch one cycle → repeat**.
   Survey is the first operation of every iteration; no separate
   cold-start branch (cold vs incremental is internal to the survey
   call based on whether `--survey-state` already exists).
4. Minimal selection UI: plain stdout listing of top-ranked plans
   with ordinals, plan identifiers, and rationales; single stdin
   read for the user's numeric choice. No ratatui widget — a richer
   TUI selection experience is a separate future enhancement.
5. Dispatch: a single invocation of the existing `phase_loop` for
   the selected plan directory; return to the top of the run loop
   on completion.
6. Tests: integration test that exercises the full
   survey→select→dispatch→re-survey loop with fake plans;
   validation that `--survey-state` is required for `N > 1` and
   rejected for `N == 1`; state-file round-trip across invocations.

**Results:** _pending_
