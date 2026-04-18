# Backlog

## Tasks

### Decide pi agent scope: complete the port or mark it aspirational

**Category:** `meta`
**Status:** `done`
**Dependencies:** none

**Description:**

Multiple audit findings point to pi being a visibly less-polished
sibling to claude-code:

- Unresolved `{{MEMORY_DIR}}` in `memory-prompt.md`. Now that
  `substitute_tokens` hard-errors on unresolved tokens, pi invocation
  **fails immediately** rather than silently corrupting instructions —
  resolving this is no longer deferred cleanup, it is a hard blocker
  on using pi at all.
- stderr not captured on failure (no tail in error messages).
- Older default model (`claude-opus-4-6`) in
  `defaults/agents/pi/config.yaml` vs claude-code's more current
  `claude-sonnet-4-6` / haiku variants.
- No integration test exercises the pi agent path.

Pick a direction: either invest in parity (and cover it in tests +
docs) or mark pi explicitly aspirational in `README.md` /
`docs/architecture.md` so future readers don't assume drop-in
equivalence. If we commit to parity, extract the genuinely shared
spawn/stream/dispatch boilerplate from `claude_code.rs` and `pi.rs`
into `src/agent/common.rs` as part of that effort.

**Results:**

Decision: **full parity** (direction A). Pi remains a first-class peer
to claude-code; the existing README and architecture doc assertions
("selectable agent backends", "spawns `claude` or `pi`") stand as
written. No doc change required in this phase — the parity commitment
is made load-bearing by the follow-up tasks below, each of which adds
a concrete CI-verifiable guard or removes a known bug.

Rationale:

- ~80% of the plumbing is already in place (510 vs 492 LOC; stream
  parser, dispatch, subagent deployment all present). Walking away via
  direction B would discard sunk cost over a handful of gaps that are
  each small in isolation.
- The recurring failure mode is **absence of guardrails**, not
  depth-of-port: the `{{MEMORY_DIR}}` hard-error was introduced by an
  unrelated change to `substitute_tokens` and went undetected because
  nothing in CI exercises the pi code path. Adding that safety net
  (Tasks 5 and 6 below) is the highest-leverage piece of the parity
  investment.
- The task description explicitly calls out `src/agent/common.rs`
  extraction "as part of that effort" if parity is chosen — Task 7
  captures that, deferred until after the stderr capture bug (Task 3)
  lands so the helper has two real callsites to justify extraction.

Follow-up work queued as the four tasks below (model bump,
`embedded_defaults_are_valid` extension, integration test, shared
common.rs extraction). Tasks 2 and 3 are no longer blocked by this
decision — their `Dependencies:` lines have been updated.

Next steps suggest: pick Task 2 next (it is the hard boot-blocker and
unlocks every other pi path), then Task 3, then in any order 4–5 (both
are isolated), then Task 6 once the pi path boots cleanly, then Task 7
as a clean-up refactor.

---

### Add `*.local.yaml` overlay so user config survives `init --force`

**Category:** `enhancement`
**Status:** `done`
**Dependencies:** none

**Description:**

`init --force` rewrites every file in `EMBEDDED_FILES` whose content
has drifted from the embedded default, which stomps any local edits to
`config.yaml`, `agents/<name>/config.yaml`, or
`agents/<name>/tokens.yaml`. This bit the user when setting
`models.work: ""` in `agents/claude-code/config.yaml` to suppress the
`--model` flag (so Claude Code's interactive 1M-context default would
win) — the next `init --force` reset the field.

Add a sibling `*.local.yaml` overlay convention: if present, the
overlay is deep-merged into the base before deserialization. Overlays
are not in `EMBEDDED_FILES`, so `init --force` never touches them. This
generalises to every future config tweak (models, thinking, params,
provider, tokens) rather than being a single-field escape hatch.

**Results:**

Implemented in `src/config.rs`:

- `merge_yaml(base, overlay)` — recursive deep-merge. Scalar collisions
  go to overlay; map collisions recurse key-by-key so base-only keys
  survive (e.g. overriding `models.work` doesn't wipe `models.reflect`).
- `load_with_optional_overlay<T>(base_path, overlay_path)` — generic
  helper that reads base, optionally reads overlay, merges at the
  `serde_yaml::Value` layer, then deserializes into `T`.
- `load_shared_config`, `load_agent_config`, `load_tokens` rewired to
  use the helper. No API change; existing callers unaffected.

Typed vs raw-YAML decision: merging is done on `serde_yaml::Value`
trees (not typed structs) so the helper is reusable for any config
file without per-type merge logic. The `provider: Option<String>` case
works naturally — an overlay with no `provider` key leaves base alone;
an overlay with an explicit `provider: openai` overrides.

Tests (8 new in `src/config.rs`):

- `merge_yaml_overrides_scalar_at_root`, `_keeps_base_keys_absent_from_overlay`,
  `_recurses_into_nested_maps` — direct unit tests on the merge primitive.
- `load_agent_config_without_overlay_uses_base` — overlay file absent,
  behaviour identical to pre-change loader.
- `load_agent_config_overlay_merges_into_base` — the load-bearing use
  case: `models.work: ""` in overlay blanks just that field while
  other phases survive.
- `load_shared_config_overlay_overrides_agent_choice` — overlay can
  swap `agent: claude-code` → `agent: pi` without disturbing
  `headroom`.
- `load_tokens_overlay_augments_and_overrides` — overlay can add new
  token keys and redirect existing ones in one file.
- `load_agent_config_shape_mismatched_overlay_surfaces_path_in_error`
  — operator-UX guard: deserialization failures name the overlay
  file, not just the base.

Docs: `docs/architecture.md` gained a paragraph in the Configuration
section explaining the overlay semantics plus a diagram line showing
`config.local.yaml` / `agents/claude-code/config.local.yaml` in the
layout tree.

Verification: `cargo test` — 143 lib tests + 8 integration tests pass.

**Operator recipe** (for the original 1M-context request):

Create `agents/claude-code/config.local.yaml`:

```yaml
models:
  work: ""
```

Next `invoke_interactive` will skip `--model` (see
`claude_code.rs:213`), letting Claude Code's interactive default (the
1M-context variant you selected in the TUI) win. The file is invisible
to `init --force`.

Next steps suggested:

- None required. The overlay is a general-purpose escape hatch; future
  tweaks (e.g. adding `--context-window` style flags via `params`)
  compose with the same overlay mechanism.

---

### Resolve or remove `{{MEMORY_DIR}}` token in pi memory prompt

**Category:** `bug`
**Status:** `done`
**Dependencies:** none

**Description:**

`defaults/agents/pi/prompts/memory-prompt.md` references `{{MEMORY_DIR}}`
at three sites (lines ~3, 61, 74) but `PiAgent::load_prompt_file`
(src/agent/pi.rs:~142) only substitutes `{{PROJECT}}`, `{{DEV_ROOT}}`,
and `{{PLAN}}`. Previously the literal `{{MEMORY_DIR}}` passed through
to the LLM unchanged; now that `substitute_tokens` hard-errors on
unresolved tokens, pi invocation fails immediately on any phase that
loads this prompt.

Decide whether memory lives in a distinct directory from the plan (if
so, thread `MEMORY_DIR` through `PlanContext` and the pi token map) or
rewrite the prompt to use `{{PLAN}}` and drop the placeholder. Also
grep the prompt for any other dangling `{{...}}` while you're there.

**Results:**

Decision: **rewrite to use `{{PLAN}}`** (option B). Auto-memory now
lives at `{{PLAN}}/auto-memory/`, colocated with other plan-tracked
state (`backlog.md`, `memory.md`, `phase.md`). The subdirectory avoids
the macOS case-insensitive collision between the auto-memory index
(`MEMORY.md`) and Raveloop's distilled memory file (`memory.md`).

Why this over threading `MEMORY_DIR` through `PlanContext`:

- Minimal surface area — no `PlanContext`, `main.rs`, or token-map
  plumbing changes.
- Consistent with plan-centric architecture: all tracked state already
  lives in the plan directory.
- Coding-style rule "don't introduce abstractions beyond what the task
  requires" applies — the distinct-dir option adds new path convention
  and cross-cutting wiring for no current benefit.

Secondary architectural fix: `PiAgent::load_prompt_file` now routes
through `crate::prompt::substitute_tokens` instead of doing its own
`str::replace` dance. The old path silently bypassed the hard-error
guard introduced in the recent prompt-compose hardening — which is
exactly how the `{{MEMORY_DIR}}` bug slipped past. Unifying the
substitution path closes that hole: any new dangling `{{X}}` in a pi
prompt now fails at load time with a descriptive error listing the
token name.

Files touched:

- `defaults/agents/pi/prompts/memory-prompt.md` — 3 occurrences of
  `{{MEMORY_DIR}}` replaced with `{{PLAN}}/auto-memory`. Pedagogical
  `{{memory name}}` / `{{memory content}}` placeholders inside the
  sample-YAML code block are intentionally preserved — they contain
  spaces, so `unresolved_token_regex` (which requires
  `[A-Za-z0-9_]+`) never matches them.
- `src/agent/pi.rs` — `load_prompt_file` rewritten to delegate to
  `substitute_tokens`. Three new tests: `load_prompt_substitutes_plan_token`
  (happy path), `load_prompt_fails_on_unresolved_token` (regression
  guard), `shipped_pi_prompts_have_no_dangling_tokens` (drift guard
  that iterates every on-disk pi prompt and asserts clean
  substitution, following the pattern from
  `every_default_coding_style_file_is_embedded`).

Verification: `cargo test` — 135 lib tests + 8 integration tests pass,
including the three new pi tests.

Next steps suggested:

- Task 3 (stderr capture) is now unblocked and is the next bug in the
  pi parity sequence — it is the last pi-path correctness bug.
- Task 6 (pi integration test) now has one of its two blockers cleared.
  That task can leverage the new `load_prompt_fails_on_unresolved_token`
  test scaffold as a model for a contract-level prompt-loading assertion.
- Worth noting for a future task: memory-prompt.md directs pi to write
  auto-memory files but no subsequent phase auto-loads them back into
  pi's context. This write-only asymmetry is out of scope for the
  current task (which was purely about resolving the dangling token),
  but a follow-up may want to either wire up reading or remove the
  memory-prompt.md entirely.

---

### Capture and surface pi subprocess stderr on non-zero exit

**Category:** `bug`
**Status:** `not_started`
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

**Results:** _pending_

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

Landing this task before Task 5 means the `embedded_defaults_are_valid`
extension can assert the specific canonical values; landing them in
the other order means Task 5 only asserts non-empty strings. Either
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
**Dependencies:** Resolve or remove `{{MEMORY_DIR}}` token in pi memory prompt; Capture and surface pi subprocess stderr on non-zero exit

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
on non-zero exit (so Task 3's fix stays fixed), and dispatch invokes
the right args for the target plan.

Depends on Tasks 2 and 3 being done so the pi path actually boots
end-to-end and `stderr` is piped — without those landing first, the
test either hangs on `substitute_tokens` or has nothing to assert
about stderr.

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

Must land AFTER Task 3 so the stderr-tail helper has two real
callsites to justify its existence — extracting a helper with a
single caller is premature abstraction per the universal coding-style
rules. The `ClaudeCodeAgent` test surface and the pi integration test
from Task 6 together form the regression net for this refactor.

**Results:** _pending_

---
