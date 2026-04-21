# Session Log

### Session 1 (2026-04-21T08:03:01Z) — Runner-owned dream-baseline seeding and build metadata

- **What was attempted:** Three-layer self-healing for `dream-baseline`; build metadata in `--version`/`version`; `cargo-release` workflow; README Releasing section.
- **What worked:** All deliverables shipped. `seed_dream_baseline_if_missing` changed from "seed to current word count" to "seed to 0" — eliminating the bootstrap delay on pre-existing plans whose baseline had drifted above threshold. Seed now called from three layers: `run_create` (post-session scaffolding), `run_set_phase` (every LLM phase transition including coordinators), and `GitCommitReflect` (original layer). `build.rs` emits `BUILD_TIMESTAMP`, `GIT_DESCRIBE`, `GIT_SHA`; `main.rs` concatenates them into a `VERSION` constant for both `--version` and the `version` subcommand. `release.toml` configures `cargo-release` with `publish=false`, `push=false`. Removed dream-baseline authorship prose from `defaults/create-plan.md`. 44 tests pass, clippy clean. Also reset `LLM_STATE/core/dream-baseline` from 1019 → 0.
- **What to try next:** Ravel's coordinator plans (`ravel-orchestrator`, `sub-D-coordination`) will auto-heal missing `dream-baseline` on next `ravel-lite state set-phase` call after binary reinstall. Initial `v0.1.0` tag not yet created — `cargo release patch --execute` or manual `git tag -a v0.1.0` seeds it.
- **Key learnings:** Seeding to 0 ("never dreamed") rather than current word count is the correct sentinel — seeding to current count silently delays the first dream by `headroom` words on populated plans. Three-layer approach ensures no single unreachable code path can leave a plan without a baseline.

### Session 2 (2026-04-21T09:52:55Z) — pre-reflect gate removal, dirty-tree narrowing, retired-path pruning, hand-off convention

- Worked four backlog tasks to completion in a single session: (1) collapse the pre-reflect gate, (2) narrow the dirty-tree warning, (3) prune stale `skills/` paths, (4) preserve hand-offs across the analyse-work → triage boundary.
- All four tasks shipped with tests; the suite was green at end of session.
- The gate removal exposed two test failures: the `pi_phase_cycle` fake-pi script looped because it always wrote `git-commit-work` regardless of phase (fixed with a phase-aware case statement), and `pivot_run_stack_short_circuit_pivot` errored on missing `reflect.md` config (fixed by seeding all five phase configs).
- `ContractMockAgent::invoke_headless` for `Triage` was changed from overwrite to append so the safety-net test can observe analyse-work's status flips after the full cycle completes.
- The hand-off convention (analyse-work.md + triage.md) is now live in shipped prompts; the next real session that produces a hand-off is the first end-to-end exercise.
- No implementation work began on the two larger tasks still `not_started`: coordinator-plan creation and the structured-state research task.

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
