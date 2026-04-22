# Backlog

## Tasks

### R2 — Implement structured `state memory` verb surface + memory migration

**Category:** `enhancement`
**Status:** `done`
**Dependencies:** R1 (done — establishes the schema / yaml_io / migrate patterns the memory submodule reuses)

**Description:**

Mirrors the R1 structure for `memory.yaml`. Extends `state migrate` to cover
`memory.md` → `memory.yaml`. CLI: `state memory list / show / add / delete`.

**Results:**

Shipped the full `state memory` surface plus the memory branch of `state
migrate`, following R1's module pattern:

- **`src/state/memory/`** — `schema.rs` (`MemoryFile { entries: Vec<MemoryEntry { id, title, body }> }`
  with `#[serde(flatten)] extra` for forward-compat top-level keys),
  `yaml_io.rs` (atomic temp-file rename, same pattern as backlog), `parse_md.rs`
  (strict parser: splits on `^## ` headings, drops preamble before the first
  heading, errors on empty-body entries), `verbs.rs` (list / show / add /
  init / set-body / set-title / delete — no reorder, no status; dream
  mutates position implicitly through the Vec). `allocate_id` and
  `slug_from_title` are imported from `state::backlog::schema` rather than
  duplicated.
- **`src/state/migrate.rs`** — refactored from a single-file migrator into a
  two-phase planner: `plan_backlog_migration` and `plan_memory_migration`
  return `Option<PendingMigration>`, all source files parse before any target
  is written, validation round-trip runs per migrator. The empty-plan case
  now errors with "no migratable .md files found" rather than the prior
  backlog-specific message. All five previous R1 migrate tests still cover
  the corresponding behaviour (renamed / generalised).
- **CLI** — `MemoryCommands` enum in `main.rs`, wired through `dispatch_state
  → dispatch_memory`. Parser helper `parse_memory_format` mirrors
  `parse_output_format`.
- **Tests** — 22 lib unit tests in `state::memory::*` (incl.
  `parses_live_core_memory_without_error` for the real `LLM_STATE/core/memory.md`),
  9 lib unit tests in `state::migrate` (covers both files, idempotency, force,
  parse-failure atomicity, empty-plan error), 4 end-to-end CLI integration
  tests in `tests/state_memory.rs`. Total suite: 342 tests, 0 failures.
  `cargo run -- state migrate LLM_STATE/core --dry-run` reports
  `7 records` + `63 records` — a live migration would convert the core plan
  cleanly.

Suggests next: R3 (`state session-log`) can slot straight into the new
`PendingMigration` enum as a third variant; the parse-all-then-write-all
contract extends trivially. R1's pre-existing `iter_cloned_collect` clippy
lint in `backlog/parse_md.rs:227` is unrelated and untouched.

---

### R3 — Implement `state session-log` + `latest-session.yaml` + GitCommitWork rewire

**Category:** `enhancement`
**Status:** `not_started`
**Dependencies:** R1 (done)

**Description:**

Adds `state session-log` verbs (list, show, append, set-latest, show-latest),
makes `latest-session.yaml` a typed file (same record shape as session-log
entries), rewires `phase_loop::GitCommitWork` to parse the new YAML + append
to `session-log.yaml`'s `sessions:` list with session-id idempotency. Extends
`state migrate` to cover session-log + latest-session.

**Results:** _pending_

---

### R5 — Implement global `state related-projects` edge list + `migrate-related-projects`

**Category:** `enhancement`
**Status:** `not_started`
**Dependencies:** R4 (done — catalog exists; names ↔ paths resolution is now available)

**Description:**

Global `../ravel-lite-config/related-projects.yaml` edge list (sibling /
parent-of), name-indexed, shareable between users. CLI: `state related-projects
list [--plan <path>]`, `add-edge`, `remove-edge`. `state migrate-related-projects
<plan-dir>` one-shot merges a plan's legacy `related-plans.md` into the global
file, creating it on first call and deduping by (kind, participants).

**Results:** _pending_

---

### Move per-plan task-count extraction from LLM survey prompt into Rust

**Category:** `enhancement`
**Status:** `not_started`
**Dependencies:** none (R1 is done — the structured backlog parser now exists)

**Description:**

The survey LLM currently infers per-plan task counts from the raw markdown in
`backlog.md`. Now that the structured backlog parser from R1 exists, task counts
(total, not_started, in_progress, done) can be computed directly in Rust and
injected as pre-populated tokens into the survey prompt — removing an
unnecessary inference burden from the LLM.

**Deliverables:**

1. Extend the structured backlog parser to expose a `task_counts() -> TaskCounts`
   method.
2. In `src/survey/discover.rs`, compute task counts from the parsed backlog
   and inject them into `PlanRow` (replacing the LLM-inferred field).
3. Update `defaults/survey.md` to remove the instruction asking the LLM
   to count tasks; add a note that counts are pre-populated.
4. Test: assert counts are correct for a plan with tasks in each status.

**Results:** _pending_

---

### R6 — Migrate all phase prompts to use CLI verbs

**Category:** `enhancement`
**Status:** `not_started`
**Dependencies:** R1, R2, R3, R4, R5 (all verbs must exist before prompts can invoke them)

**Description:**

Replace direct `Read` / `Edit` of plan-state files with `ravel-lite state <verb>`
calls across `defaults/phases/work.md`, `analyse-work.md`, `reflect.md`,
`dream.md`, `triage.md`, `create-plan.md`, `defaults/survey.md`,
`defaults/survey-incremental.md`. ~5–15 instruction rewrites per file. Prompts
keep the `{{RELATED_PLANS}}` token (projection shape preserves plan paths).

**Results:** _pending_

---

### R7 — LLM-driven discovery for related-projects (subagent parallelism + SHA caching)

**Category:** `research`
**Status:** `not_started`
**Dependencies:** R5

**Description:**

Feature design + implementation. Given a set of projects, dispatch LLM
subagents in parallel to analyse each project's README / backlog / memory and
propose sibling / parent-of edges. SHA-based cache (keyed on per-project
content hash) avoids re-analysing unchanged projects. Output merges into the
global `related-projects.yaml`.

Large — probably needs its own design-ish pass (brainstorm → spec → plan)
before implementation.

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
