### Session 1 (2026-04-21T03:44:05Z) — pivot mechanism + dream-baseline bootstrap fix

- **Attempted:** Two major work streams: (1) implement the hierarchical plan-execution pivot mechanism (`src/pivot.rs`, `run_stack` in `phase_loop.rs`, stack-aware TUI breadcrumbs), and (2) fix the dream-phase bootstrap deadlock where `dream-baseline` was never seeded on first run.

- **What worked:**
  - Pivot mechanism fully implemented and committed in many incremental commits: `pivot.rs` with `Frame`/`Stack` serde types, `read_stack`/`write_stack`, `validate_push` (depth cap + cycle detection + target validity), `decide_after_work` and `decide_after_cycle` state machines, `frame_to_context` for reconstructing `PlanContext` from a frame. `run_stack` entry point in `phase_loop.rs` replaces `phase_loop` in `main.rs` and handles push/pop/continue across nested plan cycles. Comprehensive integration test coverage for all pivot types and state transitions added to `tests/integration.rs`.
  - Dream-baseline bootstrap fix implemented and committed last: `seed_dream_baseline_if_missing` in `src/dream.rs`, wired into `GitCommitReflect` handler in `phase_loop.rs`, with unit tests (seed-when-missing, no-op-when-present) and an integration test driving the full phase loop from `git-commit-reflect` with no baseline on disk.
  - TUI `suspend`/`resume` race was fixed: `Suspend` now carries an ack channel, `suspend()` is `async` and awaits confirmation that raw mode is disabled before returning; the event-poll loop was switched from `spawn_blocking(event::poll)` to `tokio::time::sleep` to prevent the blocking thread from racing the spawned child process for the tty.
  - Claude Code interactive TUI rendering bug worked around: removed `--output-format stream-json` from `invoke_interactive` (invalid without `-p`) and added `--debug-file /tmp/claude-debug.log` workaround (implicitly enables debug mode, which masks the rendering failure via an unknown mechanism). Comment documents the investigation and flags it for future removal when claude is updated past 2.1.116.

- **What didn't work / key debugging detour:** Substantial session time was spent diagnosing why the Work-phase TUI was silently failing to render when `ravel-lite` spawned `claude`. Ruled out: termios state, isatty, process-group, signal mask, args, env, version (2.1.113–2.1.116), cmux wrapper. The `--debug-file` workaround was found empirically; root cause is unknown and upstream.

- **What this suggests trying next:**
  - The `ravel-lite state set-phase` / `push-plan` subcommand (task 2 in backlog) — now motivated by the pivot mechanism being in place.
  - Narrow `warn_if_project_tree_dirty` to baseline-touched files (task 3).
  - Once claude is updated past 2.1.116, try removing the `--debug-file` workaround from `src/agent/claude_code.rs` (two `args.push` lines).

- **Key learnings:**
  - The pivot state machine (`decide_after_work`, `decide_after_cycle`) is purely functional — no async, no side effects — which makes it easy to unit-test the four-case matrix without a real agent.
  - `spawn_blocking` in a `tokio::select!` arm does not cancel cleanly when the select drops it; `tokio::time::sleep` is properly cancellable and avoids the tty race entirely.
  - The dream-baseline deadlock was masked for this plan because memory was below headroom; it would have become visible permanently once memory grew past 1500 + 723 = 2223 words.
