# Backlog

## Tasks

### 5c — Multi-plan `run` mode with survey-driven routing

**Category:** `feature`
**Status:** `done`
**Dependencies:** 5b ✓ (incremental survey complete); 5d ✓ (clean runner architecture)

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

**Results:**

Implemented in this session. Multi-plan run mode now ships behind the
existing `ravel-lite run` command — invoking with two or more positional
plan-dir arguments switches the runner into the survey-routed dispatch
loop. Single-plan invocation is unchanged in behaviour (same
`run_phase_loop` path, same continuous "Proceed?" loop).

**Code shape:**

- `src/multi_plan.rs` (new): hosts `run_multi_plan` (the
  survey→select→dispatch→re-survey loop), `select_plan_from_response`
  (pure, `BufRead`+`Write`-parameterised so tests drive it with
  in-memory buffers), `options_from_response` (recommendation→option
  list with alphabetical fallback when `recommended_invocation_order`
  is empty), `build_plan_dir_map` (project/plan-key → PathBuf
  validation), and `dispatch_one_cycle` (per-cycle TUI setup +
  `phase_loop` + teardown).
- `src/survey/invoke.rs`: factored `run_survey` into a thin CLI wrapper
  over a new `compute_survey_response(...)` that returns
  `Result<SurveyResponse>`. Multi-plan needs the response in-memory
  (for `recommended_invocation_order` parsing) **and** on disk
  (for the next cycle's `--prior`); the existing CLI keeps its
  stdout-emitting semantics. Re-exported from `src/survey.rs`.
- `src/phase_loop.rs`: removed the `ui.confirm("Proceed to next work
  phase?")` call inside `handle_script_phase(GitCommitTriage)` — that
  handler now always returns `Ok(false)` after the commit, so
  `phase_loop` exits after exactly one full cycle. Moved the confirm
  prompt into `run_single_plan`, which now wraps `phase_loop` in a
  loop with the prompt between cycles. `run_single_plan`'s external
  behaviour for the single-plan case is preserved exactly. Multi-plan
  uses `phase_loop` directly so the inter-cycle "what next?" question
  is answered by the survey/select prompt rather than a y/n confirm.
- `src/main.rs`: `Run` subcommand now accepts `Vec<PathBuf> plan_dirs`
  (1..N) and an optional `--survey-state <path>`. Validation is in
  the dispatch arm: single-plan rejects `--survey-state`, multi-plan
  requires it. Single-plan path calls existing `run_phase_loop`
  unchanged; multi-plan path calls `multi_plan::run_multi_plan`.
- `src/lib.rs`: `pub mod multi_plan;` added.

**Selection UI:** plain stdout numbered list of plans from
`recommended_invocation_order`, each with its rationale; `0`, `q`, or
`quit` exits cleanly; EOF on stdin is also treated as exit so a
closed pipe in CI can't hang the loop. Up to three retries on
non-numeric or out-of-range input before the loop bails. No ratatui
widget per the design doc.

**Surfaced failure modes (per the "drift is user-visible" memory):**

- `options_from_response` hard-errors when a recommendation references
  a plan not in `plan_dir_by_key` — flagged as an LLM-drift signal
  with a remediation hint.
- `build_plan_dir_map` hard-errors when two plan dirs resolve to the
  same `project/plan` key (e.g. two same-named plans under the same
  git root nested at different paths) — would otherwise silently
  hide one of them from selection.

**Tests:** all 237 pass (214 unit + 23 integration). Integration
additions:

- `run_multi_plan_requires_survey_state_flag` — N>1 without `--survey-state`
  exits non-zero with a "required" diagnostic.
- `run_single_plan_rejects_survey_state_flag` — N==1 with `--survey-state`
  exits non-zero, and crucially the state file is NOT written
  (validation fires before any survey work).
- `multi_plan_round_trip_preserves_selection_mapping` — exercises the
  full Rust-side loop minus the claude spawn: parse YAML → emit (what
  `--survey-state` would persist) → re-parse (what next cycle's
  `--prior` would load) → `select_plan_from_response` with both
  ordinals (1 and 2) and verifies they resolve to the right
  `PathBuf`s. Covers the state-file round-trip integration the
  deliverable asks for; the "real" survey→dispatch loop with a live
  `claude` is necessarily an end-to-end manual test.

`src/multi_plan.rs` adds 12 unit tests covering selection happy path,
empty-recommendations alphabetical fallback, hallucinated-plan error,
input retry, retry exhaustion, `0`/`q`/EOF as exit, and
`build_plan_dir_map` duplicate-key + missing-phase.md errors.

`cargo clippy --all-targets`: only the 6 pre-existing
`doc_lazy_continuation` warnings in `src/survey/schema.rs` remain
(out-of-scope per memory). New code introduces zero new clippy
diagnostics.

**What this suggests next:**

This was the capstone task of the survey-restructure plan. With 5a
(structured YAML), 5b (incremental via `--prior`), 5c (multi-plan run
mode), and 5d (stack/pivot removal) all complete, the plan's design
goal — replacing the LLM-authored coordinator concept with code-driven
survey-routed dispatch — is delivered. Plan-level wrap-up
(merging this branch back, propagating outcome to `core/backlog.md`,
deciding whether to archive or retire this plan) is reflect/triage
work in this and the sibling plan, not new feature work here.

The "future follow-on" the design doc flagged — moving per-plan task
counts from LLM to Rust once `core/backlog.md` task #3 (structured
backlog parser) settles — remains as written, untouched by this work.
