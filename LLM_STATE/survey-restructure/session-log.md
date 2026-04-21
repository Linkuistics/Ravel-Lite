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
