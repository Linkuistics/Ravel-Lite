### Session 4 (2026-04-21T12:29:28Z) — Multi-plan run mode with survey-driven routing (5c)

- Attempted and completed task 5c: multi-plan `run` mode with survey-driven routing.
- Implemented `src/multi_plan.rs` (new): `run_multi_plan` (the survey→select→dispatch→re-survey loop), `select_plan_from_response` (IO-parameterised via `BufRead`+`Write` generics for in-memory test driving), `options_from_response` (recommendation→option list with alphabetical fallback), `build_plan_dir_map` (project/plan-key → PathBuf validation with duplicate-key detection), and `dispatch_one_cycle` (per-cycle TUI setup + `phase_loop` + teardown).
- Refactored `src/survey/invoke.rs`: factored `run_survey` into a thin CLI wrapper over new `compute_survey_response(...)` returning `Result<SurveyResponse>`; the multi-plan runner needs the response in-memory (for `recommended_invocation_order`) and on disk (for next cycle's `--prior`). Re-exported from `src/survey.rs`.
- Modified `src/phase_loop.rs`: moved the inter-cycle `ui.confirm("Proceed to next work phase?")` prompt out of `handle_script_phase(GitCommitTriage)` into `run_single_plan`, which now wraps `phase_loop` in a loop. `phase_loop` itself now exits after one full cycle (returns `Ok(false)`). Single-plan behaviour preserved; multi-plan can use `phase_loop` directly without a spurious confirm.
- Extended `src/main.rs`: `Run` subcommand now accepts `Vec<PathBuf> plan_dirs` (1..N) and optional `--survey-state <path>`; CLI dispatch validates: `N==1` rejects `--survey-state`; `N>1` requires it.
- Added three integration tests in `tests/integration.rs`: `run_multi_plan_requires_survey_state_flag`, `run_single_plan_rejects_survey_state_flag` (also verifies state file is NOT written before validation fails), `multi_plan_round_trip_preserves_selection_mapping` (exercises `build_plan_dir_map`, `select_plan_from_response`, and YAML round-trip).
- All 237 tests pass (214 unit + 23 integration). Zero new clippy diagnostics.
- This was the capstone task of the survey-restructure plan; tasks 5a, 5b, 5c, 5d are all complete.
- What this suggests next: plan-level wrap-up — merge this branch, propagate outcome to `core/backlog.md`, decide whether to archive or retire this plan. That is reflect/triage work, not new feature work.

**Deliberately not committed:** None — all source paths in the snapshot were staged.
