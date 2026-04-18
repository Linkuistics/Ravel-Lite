# Backlog

## Tasks

### Resolve path tokens inside `{{RELATED_PLANS}}` expansion

**Category:** `bug`
**Status:** `done`
**Dependencies:** none

**Description:**

`substitute_tokens` (src/prompt.rs) replaced path tokens
(`{{DEV_ROOT}}`, `{{PROJECT}}`, `{{PLAN}}`, `{{ORCHESTRATOR}}`) before
expanding `{{RELATED_PLANS}}`. Any `{{DEV_ROOT}}` etc. authored inside a
plan's `related-plans.md` — which `defaults/create-plan.md:129` says is
REQUIRED usage — was inlined too late to be substituted, and the
unresolved-token guard regex then hard-errored at prompt-compose time.

This blocked any plan whose `related-plans.md` used the documented
placeholder convention. Observed failure: TRIAGE phase for
Modaliser-Racket / modaliser exited with `Fatal error: Prompt contains
unresolved token(s) after substitution: {{DEV_ROOT}}`. All three plans
on disk that have `related-plans.md` (APIAnyware-MacOS/targets/racket-oo,
Mnemosyne/sub-B-phase-cycle, Modaliser-Racket/modaliser) use
`{{DEV_ROOT}}` and were affected.

**Results:**

Reordered substitution in `src/prompt.rs::substitute_tokens` into two
tiers: content macros first (`{{RELATED_PLANS}}` and custom tokens),
then atomic path tokens. Inlined content from `related-plans.md` now
gets a chance to be path-substituted in the same pass as the surrounding
prompt. Added regression test
`substitutes_path_tokens_inside_related_plans` that asserts a
`{{DEV_ROOT}}` literal inside `ctx.related_plans` resolves to the
absolute dev-root path. Before the fix this test reproduced the exact
user-visible error string; after the fix it passes along with all 14
existing prompt tests, 144 unit tests, and 8 integration tests
(including `shipped_pi_prompts_have_no_dangling_tokens` and
`embedded_defaults_are_valid`). A short explanatory comment now
documents the two-tier ordering so a future "tidy up" refactor doesn't
silently reintroduce the bug.

Unaddressed: custom tokens remain in the content-macro tier defensively,
even though no agent today injects multi-line content via a custom
token. If that invariant changes, tests already cover the ordering.

---

### Add `version` subcommand (and enable `--version` flag) to the CLI

**Category:** `enhancement`
**Status:** `done`
**Dependencies:** none

**Description:**

The binary exposed `raveloop <COMMAND>` via clap-derive but no way to
print the installed version. After a `cargo install --path . --force`
rebuild (e.g. picking up the substitution-order fix), the user had no
quick check that the binary on `$PATH` matched the expected release.

**Results:**

In `src/main.rs`, added `version` to the top-level `#[command(...)]`
attribute — clap-derive wires `--version` / `-V` automatically, pulling
from `env!("CARGO_PKG_VERSION")`. Also added an explicit `Version`
variant to the `Commands` enum that prints `raveloop <version>`, so the
subcommand form (`raveloop version`) is symmetric with the rest of the
CLI (`raveloop run`, `raveloop init`, etc.). All three surfaces —
`raveloop version`, `raveloop --version`, `raveloop -V` — return
`raveloop 0.1.0`, and the help listing now includes a `version` entry.
Reinstalled via `cargo install --path . --force`; no tests added
(trivial wiring, clap-derive owns the parse path). Full test suite
(144 unit + 8 integration) remained green.

Deliberately out of scope: embedding a git commit SHA / build date
requires a `build.rs`, which is speculative machinery until there is a
real drift-detection need.

---

### Capture and surface pi subprocess stderr on non-zero exit

**Category:** `bug`
**Status:** `done`
**Dependencies:** none

**Description:**

`PiAgent::invoke_headless` (src/agent/pi.rs:~236) uses
`stderr(Stdio::inherit())`, which lets pi's error output bypass the TUI
log and bleed into the raw terminal during a headless phase — often
overwritten immediately by later TUI repaints. `ClaudeCodeAgent`
(src/agent/claude_code.rs:~199) already pipes stderr, accumulates a
tail buffer, and includes the tail in its `anyhow::bail!` on failure.

Port the same pattern to `PiAgent`. When touching the code, consider
extracting `drain_stderr_into_buffer(..)` into a shared helper (e.g.
`src/agent/common.rs`) so the two copies can't drift. (Task 7 covers
the full shared-helper extraction; this task can either inline the
port and defer extraction, or co-land a narrow helper — either is
fine.)

**Results:**

Ported the claude-code stderr-capture pattern into
`PiAgent::invoke_headless`: `.stderr(Stdio::piped())`, a tokio::spawn
task that drains stderr line-by-line into a rolling buffer capped at
`STDERR_BUFFER_CAP` (4096 bytes), a one-shot `⚠` overflow warning
emitted via `UIMessage::Persist`, and a conditional `anyhow::bail!`
that now includes the `--- stderr ---\n<tail>` section when non-empty.
Pi's `invoke_interactive` is unchanged — it still uses
`Stdio::inherit()`, which is correct because the user sees errors live
in an interactive session.

Chose the **inline port** over co-landing a narrow helper. Reason: the
backlog entry for task 7 cites the universal coding-style rule
*"extracting a helper with a single caller is premature abstraction"*;
task 3 adds the second real caller, and task 7 now has two identical
copies of `STDERR_BUFFER_CAP`, `warning_line`, and the drain loop to
unify cleanly. The constant and `warning_line` are duplicated into
pi.rs with explicit `Duplicated from claude_code.rs` comments so the
next maintainer sees the extraction signal.

Full test suite still green (144 unit + 8 integration). No dedicated
test added for the stderr-tail behaviour — task 6 (the pi integration
test, which task 3 unblocks) will exercise this end-to-end with a
scripted failing mock-pi binary.

---

### Bump pi default models to match claude-code

**Category:** `enhancement`
**Status:** `done`
**Dependencies:** none

**Description:**

`defaults/agents/pi/config.yaml` sets `models.work: claude-opus-4-6`
while `defaults/agents/claude-code/config.yaml` uses
`claude-opus-4-7`. Bring pi's per-phase models in line with
claude-code's defaults so a user swapping `agent: claude-code` for
`agent: pi` gets equivalent capability, not a silent downgrade.

Mirror the claude-code config: `work: claude-opus-4-7`, and verify the
other phases (`analyse-work`, `reflect`, `dream`, `triage`) use the
same models the claude-code defaults do (currently `claude-sonnet-4-6`
in both — already aligned, but re-verify at implementation time in
case claude-code's defaults have moved). Also review the `thinking:`
map: `work: medium` is pi-specific (claude-code does not use this
field) but the other `thinking` phases are blank, which may or may
not be intended — sanity-check against pi-coding-agent's own defaults.

Landing this task before the `embedded_defaults_are_valid` extension
means that test can assert the specific canonical values; landing them
in the other order means it only asserts non-empty strings. Either
sequencing is acceptable.

**Results:**

Changed `defaults/agents/pi/config.yaml` line 3 from `claude-opus-4-6`
to `claude-opus-4-7`. All other phase models (`analyse-work`, `reflect`,
`dream`, `triage`) already matched claude-code at `claude-sonnet-4-6`;
re-verified at edit time. Left the `thinking:` map alone —
`work: medium` is pi-specific and the other entries are intentionally
blank (pi's own default). No evidence either way on whether those
blanks should be populated, so YAGNI.

Did NOT extend the `embedded_defaults_are_valid` test to assert the
specific canonical model string `claude-opus-4-7`. Reason: every model
bump would then require a test change, turning a principled invariant
(non-empty model) into a brittle literal check. If pi and claude-code
must stay in lockstep on shared phases, a cleaner guard would assert
pi.models[phase] == cc.models[phase] per phase — deferred as a judgment
call worth discussing.

---

### Extend `embedded_defaults_are_valid` to cover pi config

**Category:** `test`
**Status:** `done`
**Dependencies:** none

**Description:**

Memory records that `embedded_defaults_are_valid` asserts every
(agent, phase) pair in `defaults/agents/claude-code/config.yaml` has a
non-empty model string — a cheap guard against silent model-omission
regressions. No equivalent guard exists for pi, which is how the
`claude-opus-4-6` staleness survived unnoticed until audit.

Extend the existing test (or add a sibling test) to load
`defaults/agents/pi/config.yaml` and assert the same invariant for
every (agent, phase) pair. Consider also asserting a non-empty
`provider` string, since `PiAgent::build_headless_args` defaults it to
`"anthropic"` if missing — making that an explicit config requirement
rather than an implicit fallback eliminates a drift source.

**Results:**

On inspection, the memory entry was slightly out of date: the existing
test already iterates `[("claude-code", &cc), ("pi", &pi)]` at
tests/integration.rs:84 and asserts the non-empty-model invariant for
both. The real gap was the `provider` field — required by pi at spawn
time, but defaulted to `"anthropic"` in `build_headless_args` when
absent, which would silently disagree with a future provider change in
the shipped config.

Added a new block in `embedded_defaults_are_valid` that `.expect(...)`s
`pi.provider` to be `Some(...)` and asserts the trimmed string is
non-empty. Specific error messages identify which invariant fired, as
with the surrounding block. Caught by running the test after task 4:
it continued to pass because the embedded default already sets
`provider: anthropic`; regression coverage kicks in only if someone
strips or blanks the line in a future edit.

Did NOT update `LLM_STATE/core/memory.md` to fix the stale claim about
"only claude-code covered" — memory curation is reflect-phase work and
the stale entry is harmless (it understates the guard, not overstates).

---



---

### Bump pi default models to match claude-code

**Category:** `enhancement`
**Status:** `not_started`
**Dependencies:** none

**Description:**

`defaults/agents/pi/config.yaml` sets `models.work: claude-opus-4-6`
while `defaults/agents/claude-code/config.yaml` uses
`claude-opus-4-7`. Bring pi's per-phase models in line with
claude-code's defaults so a user swapping `agent: claude-code` for
`agent: pi` gets equivalent capability, not a silent downgrade.

Mirror the claude-code config: `work: claude-opus-4-7`, and verify the
other phases (`analyse-work`, `reflect`, `dream`, `triage`) use the
same models the claude-code defaults do (currently `claude-sonnet-4-6`
in both — already aligned, but re-verify at implementation time in
case claude-code's defaults have moved). Also review the `thinking:`
map: `work: medium` is pi-specific (claude-code does not use this
field) but the other `thinking` phases are blank, which may or may
not be intended — sanity-check against pi-coding-agent's own defaults.

Landing this task before the `embedded_defaults_are_valid` extension
means that test can assert the specific canonical values; landing them
in the other order means it only asserts non-empty strings. Either
sequencing is acceptable.

**Results:** _pending_

---

### Extend `embedded_defaults_are_valid` to cover pi config

**Category:** `test`
**Status:** `not_started`
**Dependencies:** none

**Description:**

Memory records that `embedded_defaults_are_valid` asserts every
(agent, phase) pair in `defaults/agents/claude-code/config.yaml` has a
non-empty model string — a cheap guard against silent model-omission
regressions. No equivalent guard exists for pi, which is how the
`claude-opus-4-6` staleness survived unnoticed until audit.

Extend the existing test (or add a sibling test) to load
`defaults/agents/pi/config.yaml` and assert the same invariant for
every (agent, phase) pair. Consider also asserting a non-empty
`provider` string, since `PiAgent::build_headless_args` defaults it to
`"anthropic"` if missing — making that an explicit config requirement
rather than an implicit fallback eliminates a drift source.

**Results:** _pending_

---

### Add integration test exercising the pi phase path

**Category:** `test`
**Status:** `not_started`
**Dependencies:** Capture and surface pi subprocess stderr on non-zero exit

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

Depends on "Capture and surface pi subprocess stderr on non-zero exit"
being done so the pi path boots end-to-end and `stderr` is piped —
without that landing first, the test has nothing to assert about
stderr.

**Results:** _pending_

---

### Extract shared spawn/stream/dispatch boilerplate to `src/agent/common.rs`

**Category:** `refactor`
**Status:** `not_started`
**Dependencies:** Capture and surface pi subprocess stderr on non-zero exit

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

Must land AFTER "Capture and surface pi subprocess stderr on non-zero exit"
so the stderr-tail helper has two real callsites to justify its
existence — extracting a helper with a single caller is premature
abstraction per the universal coding-style rules. The `ClaudeCodeAgent`
test surface and the pi integration test together form the regression
net for this refactor.

**Results:** _pending_

---
