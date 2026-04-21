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
