# Backlog

## Tasks

### Add integration test exercising the pi phase path

**Category:** `test`
**Status:** `done`
**Dependencies:** none (Capture and surface pi subprocess stderr on non-zero exit — done)

**Description:**

The existing `phase_contract_round_trip_writes_expected_files` test
runs `phase_loop` with `ContractMockAgent` — it verifies the phase
state machine writes the right files but does not exercise either the
`ClaudeCodeAgent` or `PiAgent` concrete implementations. As a result,
changes like `substitute_tokens` hard-erroring on unresolved tokens
broke pi end-to-end without a single test failing.

Add a contract-level test that runs a phase cycle with `PiAgent` as
the agent, using a mock `pi` binary on PATH (or a test double) that
emits a scripted stream of JSON events. At minimum the test should
cover: prompt loading resolves every `{{…}}` token (catches
`{{MEMORY_DIR}}`-class regressions), stream parsing maps events to
`UIMessage` variants correctly, stderr-tail appears in error messages
on non-zero exit (so the stderr capture fix stays fixed), and dispatch
invokes the right args for the target plan.

**Results:**

Added a `pi_integration` module to `tests/integration.rs` with three
tests. A shared `EnvOverride` helper serializes PATH/HOME mutation
across concurrent tests via a `OnceLock<Mutex<()>>`; field-drop order
keeps the mutex held until env restoration completes, so a panicked
test can't leak a fake-pi PATH into the next runner.

The three tests:

1. `pi_phase_cycle_substitutes_tokens_and_streams_events` — full
   `phase_loop` cycle with a real `PiAgent`. Fake pi (a shell script
   in a tempdir) dumps its `-p` arg for inspection, writes the
   analyse-work contract files so the cycle advances, and emits a
   `tool_execution_start` + `message_end` pair. Asserts: zero `{{…}}`
   in the captured prompt; the substituted prompt embeds the real PLAN
   path; the channel saw `Progress` + `Persist` + `AgentDone`
   variants; the audit commit landed using `commit-message.md`.

2. `pi_invoke_headless_surfaces_stderr_tail_on_failure` — fake pi
   exits 17 with stderr text. Asserts the returned error contains both
   the stderr tail and the exit code (regression guard for the
   `Stdio::inherit()` → buffered stderr fix).

3. `pi_dispatch_subagent_invokes_pi_with_target_plan_args` — fake pi
   dumps its argv to a file. Asserts `--no-session`,
   `--append-system-prompt`, `--provider anthropic`, `--mode json`,
   `-p`, and the prompt all appear in the dispatched argv.

`PiAgent::setup` runs first via `phase_loop`, which would normally
trigger `pi install` against the operator's real `~/.pi`. Test 1
redirects HOME to a tempdir pre-seeded with a settings.json declaring
`pi-subagent` already installed, so setup is a no-op. Tests 2 and 3
bypass setup by calling `invoke_headless` / `dispatch_subagent`
directly.

What this suggests next: the remaining backlog item — extracting
shared spawn/stream/dispatch boilerplate to `src/agent/common.rs` —
now has a regression net on the pi side. Both agent types are covered
by integration tests that exercise the real argv builder, real stream
parser, and real stderr surfacing path; the refactor can lift duplicated
code with confidence that drift between the two implementations would
fail one test or the other.

---

### Extract shared spawn/stream/dispatch boilerplate to `src/agent/common.rs`

**Category:** `refactor`
**Status:** `not_started`
**Dependencies:** none (Capture and surface pi subprocess stderr on non-zero exit — done)

**Description:**

`src/agent/claude_code.rs` (492 LOC) and `src/agent/pi.rs` (510 LOC)
duplicate significant boilerplate around process spawn, stdout line
reading, stderr-tail buffering, subagent dispatch scaffolding, and
UIMessage emission patterns. The duplication is a drift source — e.g.
one agent getting an `anyhow::bail!` improvement that the other
silently misses.

Identify genuinely shared logic (process spawn helper, stdout
line-read loop that forwards to the right formatter, stderr drain +
tail buffer with size cap, dispatch pattern scaffolding) and lift
into a new `src/agent/common.rs`. Leave agent-specific surface — CLI
flag construction, JSON event parsing (different schemas between the
two agents) — in the concrete `*.rs` files.

`STDERR_BUFFER_CAP` and `warning_line` are currently duplicated across
`pi.rs` and `claude_code.rs` with comments pointing here. The pi
integration test (task above) forms the regression net for this
refactor alongside the existing `ClaudeCodeAgent` test surface.

**Results:** _pending_

---

### Narrow `warn_if_project_tree_dirty` to work-agent-touched files only

**Category:** `enhancement`
**Status:** `not_started`
**Dependencies:** none

**Description:**

`warn_if_project_tree_dirty` at `phase_loop.rs:94` is pathspec-unscoped
— it fires on any dirty file in the project tree. In monorepos with
multiple plans the check can still produce false positives from sibling
plans' in-flight writes, even after the atomic phase-transition fix.

Narrow the check to: compute `git diff --name-only <work_baseline>`
(files changed since the work baseline) intersected with the current
dirty list, so the warning only fires on files the active work agent
could plausibly have touched. This is a defense-in-depth refinement;
no correctness regression possible since the current check is strictly
more noisy, never more accurate.

**Results:** _pending_

---
