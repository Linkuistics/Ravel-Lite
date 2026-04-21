# Backlog

## Tasks

### Add `ravel-lite state` subcommand so prompts mutate phase/stack via CLI

**Category:** `enhancement`
**Status:** `done`
**Dependencies:** none

**Description:**

Prompts currently mutate plan state by writing files directly:
`Write "analyse-work" to {{PLAN}}/phase.md` (5 shipped phase prompts),
and for the Ravel coordinator, appending a frame to
`{{PLAN}}/stack.yaml` in `Ravel/LLM_STATE/ravel-orchestrator/prompt-work.md`.
Two problems with the file-write path:

1. **Tool-call overhead per transition.** Claude Code's `Write` tool
   requires a prior `Read` of any existing file. Every phase.md
   transition = `Read` + `Write` = 2 tool calls, plus a permission
   prompt on `Write` unless pre-approved. A `Bash(ravel-lite state *)`
   allowlist entry collapses this to 1 tool call and 0 prompts per
   transition — and there are 4–5 transitions per cycle (analyse-work,
   reflect, dream, triage each advance phase.md, plus work itself).
2. **Structural invariants restated in prose.** The coordinator prompt
   contains a case-analysis the LLM must get right per pivot:
   *"If stack.yaml does not exist, create with root+target. If it
   exists with only root, rewrite with both. If multi-frame, append."*
   That logic lives once in `src/pivot.rs` (`validate_push` +
   `read_stack`/`write_stack`); re-exposing it via a CLI verb lets
   every caller delegate rather than reimplement.

These justifications are orthogonal: `set-phase` cashes in #1,
`push-plan` cashes in #2. Both benefit from living under a shared
`state` namespace so operators add one glob to `.claude/settings.json`.

**Scope (proposed):**

Two subcommands now; `pop-plan` deferred (no LLM-driven caller — pops
are driver-internal, triggered by cycle end or user-declined confirm).

| Subcommand | argv | Callers |
|---|---|---|
| `set-phase` | `ravel-lite state set-phase <plan-dir> <phase>` | 5 shipped prompts: `work.md`, `analyse-work.md`, `reflect.md`, `dream.md`, `triage.md` |
| `push-plan` | `ravel-lite state push-plan <plan-dir> <target-plan-dir> [--reason <s>]` | `Ravel/LLM_STATE/ravel-orchestrator/prompt-work.md` |

**`set-phase` semantics:**
- Validate `<phase>` via `Phase::parse`; reject typos with an error
  listing allowed values.
- Require `<plan-dir>/phase.md` to already exist (prevents silently
  creating a new "plan dir").
- Write atomically (write to tmp, rename).

**`push-plan` semantics:** matches the coordinator prose it replaces:
- `<plan-dir>` is the coordinator's own plan (where `stack.yaml`
  lives); `<target-plan-dir>` is the child to push.
- If `stack.yaml` absent: create with
  `[{path: <plan-dir>}, {path: <target>, reason, pushed_at}]`.
- If present: append `{path: <target>, ...}` as a new last frame.
- Validation via `pivot::validate_push` (cycle detection, depth cap,
  target has `phase.md`).
- `--reason` optional; `pushed_at` set automatically (match the
  timestamp code at `phase_loop.rs:316-318`).

**Design decisions:**
- Positional `<plan-dir>` (matches `run` / `create` convention).
- Binary discoverability: prompts call `ravel-lite`; assume it's on
  `$PATH`. Matches how `claude`/`pi` are invoked elsewhere. No new
  token machinery.
- Allowlist hint: document `Bash(ravel-lite state *)` pattern in the
  migration notes so operators get the tool-call savings.
- No new dependencies — everything needed is in `src/pivot.rs` and
  `src/types.rs` already.

**Deliverables:**

1. `src/main.rs` — new `State` subcommand with `SetPhase` / `PushPlan`
   variants; thin dispatch layer.
2. `src/state.rs` (new) — the two handlers, TDD-style: unit tests for
   validation errors and happy paths; integration test that shells out
   to the built binary and asserts phase.md / stack.yaml contents.
3. Prompt updates (this repo): 5 files in `defaults/phases/*.md`
   replace `Write X to {{PLAN}}/phase.md` with
   `ravel-lite state set-phase {{PLAN}} X`.
4. Out-of-repo: the Ravel coordinator prompt at
   `Ravel/LLM_STATE/ravel-orchestrator/prompt-work.md` should be
   updated by Ravel's maintainer to call `ravel-lite state push-plan`
   — **not part of this task**. We ship the mechanism here.

**Followups / later:**
- `pop-plan` if a prompt ever needs LLM-driven popping.
- Optional `--plan $RAVEL_LITE_PLAN_DIR` env fallback if prompt
  substitution around `{{PLAN}}` turns out to be awkward.
- `{{RAVEL_LITE_BIN}}` token if PATH-based discoverability fails in
  real deployments.
- Operator allowlist hint: add `Bash(ravel-lite state *)` to
  `.claude/settings.json` to realise the tool-call savings end-to-end.

**Results:**

Delivered as scoped.

- **`src/state.rs` (new)** — two handlers with full unit coverage (8
  tests, all TDD-driven red→green): `run_set_phase` validates the phase
  via `Phase::parse`, requires `phase.md` to already exist (refuses to
  create a plan dir silently), and writes atomically via a local
  `.phase.md.tmp` + rename. `run_push_plan` delegates to
  `pivot::validate_push` for cycle/depth/target checks, seeds the
  coordinator's own frame when `stack.yaml` is absent, and appends the
  target with a `pushed_at` timestamp from the new
  `pivot::push_timestamp()`.
- **Timestamp consolidation** — `chrono_like_timestamp` moved from
  `phase_loop.rs` into `pivot.rs` as `push_timestamp()` so the driver's
  `sync_stack_to_disk` and the new `state push-plan` CLI write identical
  `pushed_at` formats. Single-source-of-truth for frame serialization.
- **CLI wiring** — `Commands::State` with `StateCommands::SetPhase` and
  `StateCommands::PushPlan` clap variants; argv shape:
  `ravel-lite state set-phase <plan-dir> <phase>` and
  `ravel-lite state push-plan <plan-dir> <target-plan-dir> [--reason <s>]`.
- **Integration tests** — two new tests in `tests/integration.rs`
  shell out via `CARGO_BIN_EXE_ravel-lite`, assert on-disk effects
  (phase.md contents, stack.yaml frames, reason text) and on-error exit
  codes + stderr diagnostics. First use of the `CARGO_BIN_EXE_*` pattern
  in this test file — pattern available for future CLI additions.
- **Prompt updates** — 5 shipped phase prompts in `defaults/phases/*.md`
  now instruct the LLM to run `ravel-lite state set-phase {{PLAN}} X`
  instead of `Write X to {{PLAN}}/phase.md`: `work.md`, `analyse-work.md`,
  `reflect.md`, `dream.md`, `triage.md`.
- **Clippy cleanup (requested mid-task)** — existing clippy debt cleared
  across `format.rs` (3× `map_or` → `is_some_and`, one `len() >= 1` →
  `!is_empty()`), `ui.rs` (`AppState` gets a `Default` impl;
  `AppState::new` delegates), `types.rs` (`LlmPhase::from_str` and
  `ScriptPhase::from_str` renamed to `parse`, aligning with
  `Phase::parse` and removing `should_implement_trait`; `ScriptPhase`
  enum carries a targeted `#[allow(clippy::enum_variant_names)]` with a
  load-bearing rationale comment), `survey/render.rs` (test helper gets
  `#[allow(clippy::too_many_arguments)]`), and `tests/integration.rs`
  (one `PathBuf` owned-for-compare → `Path` reference). Clippy is now
  fully green on `--all-targets`.
- **Installed** — `cargo install --path . --locked` placed the new
  binary at `~/.cargo/bin/ravel-lite`, so the updated phase prompts can
  invoke it directly the moment the next phase cycle kicks off.

**Verification:**
- `cargo test` — 155 lib + 44 integration tests all pass.
- `cargo clippy --all-targets` — clean (previously 8 pre-existing errors).
- `ravel-lite state --help` / `ravel-lite state set-phase --help` /
  `ravel-lite state push-plan --help` all render the expected argv
  surface.

**Design hand-offs for triage to promote (settled this session, not yet
backlog entries):**

1. **Terminal-title OSC on phase transitions and pivots.** Write
   `<project-root-basename> <plan-name> <phase>` via `\033]0;…\007` to
   stdout (tmux-wrap via `\033Ptmux;…\033\\` when `$TMUX` is set) at the
   start of each phase handler in `phase_loop.rs` + push/pop sites in
   `run_stack`. Don't restore on exit — the next shell prompt
   overwrites. Small new module `src/term_title.rs` with one
   `set_title(project, plan, phase)` function. Project name comes from
   project root basename (same convention as the phase-header render).
2. **Coordinator plan creation (lives in `ravel-lite-config`, not this
   repo).** Extend `ravel-lite-config/create-plan.md` so the LLM can
   decompose oversized requests into N child leaf plans + one
   coordinator parent that orchestrates them. New shared fragment
   `ravel-lite-config/coordinator-work-boilerplate.md` holds the
   invariant blocks (OVERRIDE NOTICE, "never leave both stack and phase
   unchanged", `ravel-lite state push-plan` usage — now a real CLI verb
   thanks to this task). Children authoritative-listed in each
   coordinator's `prompt-work.md` (not filesystem-derived, so coordinator
   can scope to a chosen subset). Create children first then coordinator
   (partial failure leaves usable leaf plans). No seeded backlog —
   coordinator's first work cycle picks the next child.
   Depends on: this task (✓), `ravel-lite-config` repo acceptance.

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

Note: work-baseline is seeded atomically in the triage commit
(`git_save_work_baseline` in `GitCommitTriage`), so the baseline SHA
is reliably available when the dirty check runs.

**Results:** _pending_

---

### Remove Claude Code `--debug-file` workaround once version exceeds 2.1.116

**Category:** `maintenance`
**Status:** `not_started`
**Dependencies:** none

**Description:**

`invoke_interactive` in `src/agent/claude_code.rs` passes
`--debug-file /tmp/claude-debug.log` as a workaround for a TUI
rendering failure in Claude Code ≤2.1.116. The root cause was not
found; debug mode happens to mask it via an unknown upstream mechanism.

When the installed `claude` binary is updated past 2.1.116, remove both
`args.push` lines adding `--debug-file` and `/tmp/claude-debug.log`.
Verify that the Work phase TUI renders correctly without the flag
before closing.

**Results:** _pending_

---
