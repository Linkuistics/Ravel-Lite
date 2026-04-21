### Session 3 (2026-04-21T12:53:55Z) — survey-restructure sub-plan close-out: 5a–5d delivered

- Ran the survey-restructure sub-plan through four full work cycles, delivering tasks 5a, 5b, 5c, and 5d as distinct commits against main.
- **5a** (`5711fac`): Structured YAML output for `ravel-lite survey`; `survey-format` subcommand for human rendering; `input_hash` field seeded in Rust post-parse. `src/survey/schema.rs` gained `Serialize` derives and `schema_version` marker.
- **5b** (`5e295f4`): Incremental survey via `--prior` and `--force`. New `src/survey/delta.rs` owns hash-comparison and delta-merge logic. `src/survey/invoke.rs` refactored into `compute_survey_response` (in-memory) + `run_survey` (CLI wrapper). `defaults/survey-incremental.md` added as the delta-path prompt template.
- **5c** (`fdaeb02`): Multi-plan `run` mode with survey-driven routing. New `src/multi_plan.rs` (539 lines) implements `build_plan_dir_map`, `options_from_response`, `select_plan_interactive`, and `run_multi_plan`. `ravel-lite run` now accepts `1..N` plan dirs; `--survey-state` required for N > 1. Design rationale captured in `docs/survey-pivot-design.md`.
- **5d** (`06ce874`): Removed `src/pivot.rs`, `push-plan` CLI verb, `run_stack` logic, and `stack.yaml` infrastructure. `src/state.rs` trimmed from ~230 to ~80 lines. `src/phase_loop.rs` de-pivoted.
- Sub-plan close-out triage (`19ad808`, `7735d3f`): propagated results to core backlog and closed the survey-restructure plan. Deleted `LLM_STATE/survey-restructure/` directory in source commit `080c9e6`.
- `tests/integration.rs` overhauled throughout (1197 lines changed) to cover the new survey/incremental/multi-plan paths and remove obsolete pivot/stack tests.

What worked: linear dependency chain 5a → 5b → 5c held; sub-plan broke work into per-cycle chunks that each compiled and tested green before committing.

What this suggests trying next: the "Migrate Ravel orchestrator off removed push-plan verb" task is now unblocked and urgent — the orchestrator will break on next invocation. The structured-data research task (backlog CLI verbs) is next highest value but not urgent.
