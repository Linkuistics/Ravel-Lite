### Session 2 (2026-04-19T06:09:37Z) — Add pi_integration tests pinning PiAgent contract

- Attempted and completed: added `pi_integration` module to `tests/integration.rs` (407 lines) with three tests covering the real `PiAgent` spawn/stream/dispatch path — closing the gap that let the `{{MEMORY_DIR}}` regression escape undetected
- `pi_phase_cycle_substitutes_tokens_and_streams_events`: full `phase_loop` cycle with a real `PiAgent` and fake `pi` shell script; asserts zero unresolved `{{…}}` tokens in the captured prompt, correct `UIMessage` variant fan-out (`Progress`, `Persist`, `AgentDone`), and audit commit via `commit-message.md`
- `pi_invoke_headless_surfaces_stderr_tail_on_failure`: non-zero `pi` exit (code 17) must surface the stderr tail in the returned error — regression guard for the `Stdio::inherit` → buffered-stderr fix
- `pi_dispatch_subagent_invokes_pi_with_target_plan_args`: pins the argv contract for `dispatch_subagent` (`--no-session`, `--append-system-prompt`, `--provider anthropic`, `--mode json`, `-p`, prompt)
- `EnvOverride` helper serialises `PATH`/`HOME` mutation via a process-wide `OnceLock<Mutex<()>>`; struct-field drop order keeps the lock held until env restoration completes, preventing fake-pi PATH from leaking into concurrent test runners
- What this suggests next: `Extract shared spawn/stream/dispatch boilerplate to src/agent/common.rs` now has full regression coverage on both the pi and claude-code sides; the refactor can proceed with confidence
