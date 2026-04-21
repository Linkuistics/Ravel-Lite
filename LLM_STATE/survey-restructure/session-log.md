# Session Log

### Session 1 (2026-04-21T11:25:50Z) — Implement task 5a: structured YAML output, survey-format, input hashes

- Implemented all six deliverables of task 5a in full.
- Added `sha2 = "0.10"` to `Cargo.toml`; SHA-256 `input_hash` computed in Rust using length-prefixed sections over `phase.md + backlog.md + memory.md + related-plans.md` so absent vs. empty hash distinctly.
- `SurveyResponse`, `PlanRow`, `Blocker`, `ParallelStream`, `Recommendation` gained `serde::Serialize`; `PlanRow` gained `input_hash: String` with `#[serde(default)]` so LLM-emitted YAML without the field still parses.
- `discover_plans` (read_dir walk) replaced by `load_plan(plan_dir)` in `src/survey/discover.rs`; CLI positional args renamed from `roots` to `plan_dirs`; callers name plans individually rather than walking a root.
- `run_survey` rewritten in `src/survey/invoke.rs` to take plan dirs, sort plans by (project, plan), inject hashes strictly (undiscovered row = hard error; missing row = hard error), emit YAML via `serde_yaml::to_string`. Markdown render path removed from `run_survey`.
- `src/survey/schema.rs` gained `emit_survey_yaml`, `inject_input_hashes`, `plan_key`; `src/survey.rs` re-exports updated.
- New `ravel-lite survey-format <file>` subcommand added to `src/main.rs`; `run_survey_format(path)` in `src/survey/invoke.rs`.
- Integration tests rewritten/added: `survey_loads_plans_from_multiple_projects_individually_named`, `survey_yaml_emit_injects_input_hashes_and_round_trips`, `survey_format_renders_markdown_matching_direct_render`.
- Core backlog tasks #1 (coordinator plans) and #5 (incremental survey) marked `done` in `LLM_STATE/core/backlog.md` as superseded by survey-restructure plan. `docs/survey-pivot-design.md` and `LLM_STATE/core/related-plans.md` created.

**What worked:** Hard-error injection, length-prefixed hashing, clean split of YAML persistence from markdown presentation. All tests pass.

**Deliberately not committed:** None — all paths in the snapshot were staged.

**What this suggests for 5b:** `plan_key` + per-row `input_hash` are the keying primitives for delta classification. `parse_survey_response` is the single entry point for both LLM stdout and `--prior` file reads. `inject_input_hashes`' strict validation is the model for 5b's "delta refuses mutation outside the declared changed set" invariant. `schema_version: 1` is a one-line `SurveyResponse` addition deferred to 5b (adding it here would couple 5a to a 5b-only concern).

### Session 2 (2026-04-21T11:47:53Z) — Delete pivot/stack infrastructure (task 5d)

- Attempted and completed task 5d: removal of the LLM-authored coordinator-plan concept from the codebase.
- What worked: clean deletion across all five affected files with no regressions. `cargo build` and `cargo test` (178 unit + 17 integration + 0 doc tests) passed. `cargo clippy` showed 6 pre-existing doc-formatting errors in `src/survey/schema.rs`, confirmed out of scope.
- What was left out of scope per deliverable: `LLM_STATE/core/memory.md` entries about pivot/stack/run_stack remain (flagged for next core triage cycle); `docs/survey-pivot-design.md` is reference material and untouched; the out-of-repo Ravel orchestrator will break on next invocation as expected.
- What this suggests next: task 5b (incremental survey via `--prior`) is unblocked. Task 5c is unblocked modulo 5b. The clean `phase_loop`/`run_single_plan` seam in `src/phase_loop.rs` means 5c can branch on plan-count in `main::run_phase_loop` with minimal churn — single-plan path is the existing call; multi-plan path adds the survey-routed dispatch loop.
- Key learning: `run_single_plan` was retained as a one-line delegate (not removed outright) to preserve an obvious expansion seam for 5c without re-plumbing main.

Files changed (all source, no LLM_STATE):
- `src/pivot.rs` — deleted (267 lines)
- `src/lib.rs` — removed `pub mod pivot` declaration
- `src/main.rs` — removed `mod pivot`, `StateCommands::PushPlan` variant + dispatch arm, renamed `run_stack` call to `run_single_plan` (-15 lines net)
- `src/state.rs` — deleted `run_push_plan` and 5 unit tests, removed pivot import, updated module doc (-141 lines net)
- `src/phase_loop.rs` — deleted all pivot helpers (`raw_phase_label`, `set_title_for_context`, `format_breadcrumb`, `log_phase_header_with_breadcrumb`, `sync_stack_to_disk`, `on_disk_stack_len`, `on_disk_new_top`, `stack_snapshot`, `do_push`, `build_prompt`); collapsed `run_stack` to 9-line `run_single_plan`; removed `#[allow(dead_code)]` on `phase_loop` (-261 lines net)
- `tests/integration.rs` — deleted entire pivot test region (~760 lines); pruned stale `AtomicUsize`/`Ordering` imports

**Deliberately not committed:** None — all source paths in the snapshot were staged.

### Session 3 (2026-04-21T12:08:16Z) — Incremental survey via --prior (task 5b)

- Implemented all seven deliverables for task 5b: delta classifier, incremental render, cold/warm invoke split, --prior/--force CLI flags, schema_version guard, noop fast path, and 13 new tests.
- New module `src/survey/delta.rs` introduces `classify_delta`, `merge_delta`, and `DeltaClassification` which drives the cold/incremental decision in `invoke.rs`.
- `src/survey/compose.rs` gained `render_incremental_survey_input` — sends only changed+added plans to the LLM, carrying the full prior as context.
- `src/survey/invoke.rs` refactored: extracted `spawn_claude_and_read` so the cold and incremental paths share identical subprocess/timeout/error logic; added prior-load, classify, merge, and schema-version validation.
- `src/survey/schema.rs` gained `schema_version: u32` with `#[serde(default)]` for forward-compatible YAML parsing of 5a-emitted files without the field.
- `src/main.rs`: `--prior <file>` and `--force` flags added to the `survey` subcommand; forwarded through to `run_survey`.
- `defaults/survey-incremental.md` added as the warm-path prompt; registered in `src/init.rs::EMBEDDED_FILES`.
- Tests: 13 new tests in `src/survey/delta.rs` (unit classify/merge), `src/survey/compose.rs` (incremental render), `src/survey/invoke.rs` (schema-version guard), and `tests/integration.rs` (3 end-to-end tests). Total: 203 library + 20 integration, all green. Clippy clean on touched files; 6 pre-existing `doc_lazy_continuation` warnings on `PlanRow::input_hash` are out-of-scope.
- Noop fast path (`classification.is_noop()`) carries the prior forward with no LLM call — makes 5c's every-cycle survey invocation affordable.
- Gotchas: `Clone` propagation required across all nested `SurveyResponse` structs; `deny(warnings)` surfaced unused-public items when `delta.rs` had no consumer yet.
- Suggests next: task 5c can call `run_survey` with a `--survey-state` path that doubles as `--prior` input and output; `run_single_plan` in `phase_loop.rs` is the dispatch seam.

**Deliberately not committed:** None — all source paths in the snapshot were staged.

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
