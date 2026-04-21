# Backlog

## Tasks

### 5a — Structured YAML output for `survey`

**Category:** `feature`
**Status:** `done`
**Dependencies:** none

**Description:**

Make `ravel-lite survey` emit canonical YAML that round-trips cleanly,
replace root-walked CLI args with positional plan-directory args, add a
`survey-format` subcommand that renders a saved YAML survey as markdown,
and embed an `input_hash` forward-compat field on `PlanRow` that 5b will
consume. See `docs/survey-pivot-design.md` §5a for full design and
rationale.

**Deliverables:**

1. Add `serde::Serialize` to `SurveyResponse`, `PlanRow`, `Blocker`,
   `ParallelStream`, `Recommendation` in `src/survey/schema.rs`.
2. Change `run_survey` in `src/survey/invoke.rs` to emit YAML via
   `serde_yaml::to_string` on the parsed struct — re-emission, not
   pass-through of the LLM's raw stdout. The markdown render path
   is removed from `run_survey`; consumers who want markdown invoke
   `survey-format`.
3. Collapse `discover_plans` in `src/survey/discover.rs` from a
   `read_dir` walk into a single-plan loader. Positional args in
   `src/main.rs` change from plan-root directories to plan
   directories themselves.
4. Add `input_hash: String` to `PlanRow`. Compute in Rust over each
   plan's `phase.md` + `backlog.md` + `memory.md` + `related-plans.md`
   contents (explicitly NOT `session-log.md`) using SHA-256 via the
   `sha2` crate. Inject the hash into each `PlanRow` **after** parsing
   the LLM's YAML response, matched by project+plan identifier. The
   LLM never sees or handles the hash — no `survey.md` prompt change
   for the hash field.
5. New `ravel-lite survey-format <file>` subcommand: reads a YAML
   survey from the given path, parses via existing
   `parse_survey_response`, renders via existing `render_survey_output`,
   prints to stdout.
6. Tests: serialize↔deserialize round-trip produces byte-identical
   YAML across two emissions; hash stability across equal inputs;
   `survey-format` golden output matches current `run_survey` markdown
   output for the same struct; positional-arg CLI parse.

**Results:**

All six deliverables implemented. 229 tests pass (183 unit + 46
integration); end-to-end `survey-format` smoke on a hand-crafted YAML
renders the expected markdown.

**What was done:**

- `sha2 = "0.10"` added to `Cargo.toml` for SHA-256.
- `SurveyResponse`, `PlanRow`, `Blocker`, `ParallelStream`,
  `Recommendation` gained `serde::Serialize`. `PlanRow` gained
  `input_hash: String` with `#[serde(default)]` — the LLM never sees or
  emits the field on input, but every on-disk YAML always carries it
  after post-parse injection.
- `src/survey/discover.rs`: the `fs::read_dir` walk is gone; replaced
  with `load_plan(plan_dir)` that reads one plan at a time. `PlanSnapshot`
  gained `input_hash`, computed at load time over phase + backlog +
  memory + related-plans with length-prefixed sections so "absent"
  and "empty" hash distinctly (verified by test).
- `src/survey/schema.rs`: added `emit_survey_yaml`, `inject_input_hashes`,
  `plan_key`. Injection is strict in both directions — a row for an
  undiscovered plan is a hard error (LLM drift), and a discovered plan
  missing from the response is a hard error (prompt-contract violation).
- `run_survey` in `src/survey/invoke.rs` rewritten to take plan dirs,
  sort discovered plans by (project, plan), inject hashes, and emit
  canonical YAML via `serde_yaml::to_string`. The markdown render path
  moved to the new `run_survey_format(path)`.
- `src/main.rs`: `Survey` subcommand's positional arg renamed from
  `roots: Vec<PathBuf>` to `plan_dirs: Vec<PathBuf>`; new subcommand
  `SurveyFormat { file: PathBuf }`. `--help` output verified.
- `src/survey.rs`: library re-exports updated — `discover_plans` out,
  `load_plan` in, plus `emit_survey_yaml`, `inject_input_hashes`,
  `plan_key`, `parse_survey_response`, `render_survey_output`,
  `PlanRow`, `SurveyResponse`, `run_survey_format`.
- Integration test `survey_plan_discovery_across_multiple_roots`
  rewritten as `survey_loads_plans_from_multiple_projects_individually_named`;
  added `survey_yaml_emit_injects_input_hashes_and_round_trips` and
  `survey_format_renders_markdown_matching_direct_render`.

**What worked / design choices made:**

- Hard-error over warn-and-skip for hash-injection mismatches. A silent
  empty `input_hash` would quietly defeat 5b's change detection; loud
  failure now surfaces both LLM drift and discovery bugs at the point
  they happen.
- Length-prefixed section hashing (`label\0present\0<u64-le-len><bytes>\0`
  per section) so a byte-swap between two files can't produce a hash
  collision with a different file layout.
- Related-plans.md is included in the hash but NOT in the LLM prompt
  input. The prompt template didn't mention it, adding it is scope
  creep, and hashing it means changes there still trigger re-survey
  under 5b.

**What didn't / deferred:**

- `schema_version: 1` deliberately NOT added — it's called out in the
  5b deliverables, not 5a. Doing it here would change the round-trip
  test fixture and couple 5a to a 5b-only concern.
- No changes to `defaults/survey.md`. The LLM's prompt contract is
  unchanged; `input_hash` is mechanical and lives in Rust only.

**What this suggests for 5b:**

- Adding `schema_version: u32` to `SurveyResponse` (with
  `#[serde(default = "default_schema_version")]` so 5a-emitted YAML
  without the marker still parses once 5b lands) is a one-line schema
  change that round-trips through the existing pipeline unchanged.
- Delta classification already has its keying primitive: `plan_key`
  and the per-row `input_hash` are enough to compare a freshly-loaded
  plan against a prior-survey row.
- `parse_survey_response` is the single entry point for both LLM
  stdout and `--prior` file reads — future schema changes touch one
  parser.
- `inject_input_hashes`' strict validation is the model for 5b's
  "validation refuses a delta that mutates a plan outside the declared
  changed set" — same shape of error, different predicate.

---

### 5b — Incremental survey via `--prior`

**Category:** `feature`
**Status:** `not_started`
**Dependencies:** 5a (canonical YAML round-trip + `input_hash` field)

**Description:**

Add a `--prior <file>` flag to `ravel-lite survey` so a prior YAML
survey can seed an incremental run: compare per-plan `input_hash`
values, send only changed+added plans to the LLM, and merge the delta
with unchanged rows from the prior. Per-cycle survey cost becomes
proportional to what actually changed — a precondition for 5c's
every-cycle survey to be affordable. See `docs/survey-pivot-design.md`
§5b.

**Deliverables:**

1. `--prior <file>` flag on `survey`. Parse the prior YAML; classify
   each plan as `unchanged` / `changed` / `removed` / `added` by
   comparing freshly-computed `input_hash` values against the prior.
2. Delta-aware `render_survey_input` in `src/survey/compose.rs`: only
   changed+added plans appear in the LLM payload. The prior survey
   is carried in full as context so the LLM can revisit cross-plan
   blockers and parallel streams when deltas affect them.
3. Merge logic in `run_survey`: LLM delta + prior-unchanged rows →
   final `SurveyResponse`. Validation refuses a delta that mutates a
   plan outside the declared changed set.
4. `--force` bypass flag: re-analyses everything regardless of hash
   match. For debugging and schema-bump paths.
5. Prompt strategy — settle during implementation; lean: two prompts
   (`defaults/survey.md` cold, `defaults/survey-incremental.md` warm)
   beats one with conditional branches. Embed via
   `src/init.rs::EMBEDDED_FILES`; preserve drift-guard coverage.
6. Include `schema_version: 1` at the top of emitted YAML.
   Mismatched-version `--prior` either fails fast with a remediation
   hint or auto-falls-back to `--force`-equivalent behaviour.
7. Tests: unchanged-plan reuse, changed-plan re-analysis,
   removed-plan pruning, added-plan detection, schema-bump
   invalidation, `--force` path, validation-rejects-delta-outside-
   changed-set.

**Results:** _pending_

---

### 5c — Multi-plan `run` mode with survey-driven routing

**Category:** `feature`
**Status:** `not_started`
**Dependencies:** 5b (incremental survey for affordable per-cycle
invocation)

**Description:**

Turn `ravel-lite run` into a multi-plan orchestrator when given N
positional plan-dir args. At the top of every cycle, run an
incremental survey over all N plans, present the top-ranked plans to
the user via a minimal stdout prompt, and dispatch one phase cycle of
the user's choice before looping back. Replaces the LLM-driven
coordinator concept with a code-driven routing loop. See
`docs/survey-pivot-design.md` §5c.

**Deliverables:**

1. `run` accepts `N > 1` positional plan dirs. `N == 1` remains
   exactly as today (no survey, no state file, unchanged behaviour).
2. New required flag for `N > 1`: `--survey-state <path>`. Rejected
   when `N == 1`. The file is both output (written at cycle end) and
   input (read as `--prior` next cycle via 5b's incremental path).
3. Run-loop shape: **survey → select → dispatch one cycle → repeat**.
   Survey is the first operation of every iteration; no separate
   cold-start branch (cold vs incremental is internal to the survey
   call based on whether `--survey-state` already exists).
4. Minimal selection UI: plain stdout listing of top-ranked plans
   with ordinals, plan identifiers, and rationales; single stdin
   read for the user's numeric choice. No ratatui widget — a richer
   TUI selection experience is a separate future enhancement.
5. Dispatch: a single invocation of the existing `phase_loop` for
   the selected plan directory; return to the top of the run loop
   on completion.
6. Tests: integration test that exercises the full
   survey→select→dispatch→re-survey loop with fake plans;
   validation that `--survey-state` is required for `N > 1` and
   rejected for `N == 1`; state-file round-trip across invocations.

**Results:** _pending_

---

### 5d — Remove `stack.yaml`, `push-plan` CLI, `pivot.rs`, and `run_stack`

**Category:** `enhancement`
**Status:** `not_started`
**Dependencies:** none. Code being deleted has no in-repo caller
(core task #1's supersession confirms this). Can run at any point
relative to 5a/5b/5c. Recommended ordering: **before 5c**, so 5c is
built against a clean runner architecture with no residual
stack/frame logic to coexist with.

**Description:**

Delete the infrastructure that supported the LLM-authored
coordinator-plan concept: the `push-plan` CLI verb, `pivot.rs` in
its entirety, `stack.yaml` I/O, and the `run_stack` wrapper (which
collapses back to a straightforward single-plan run loop). See
`docs/survey-pivot-design.md` §5d for scope and the external-impact
note about the out-of-repo Ravel orchestrator at
`{{DEV_ROOT}}/Ravel/LLM_STATE/ravel-orchestrator/`.

**Deliverables:**

1. Remove the `ravel-lite state push-plan` subcommand from
   `src/main.rs`; remove `run_push_plan` from `src/state.rs` along
   with its tests.
2. Delete `src/pivot.rs` in its entirety (`validate_push`,
   `push_timestamp`, `decide_after_work`, `decide_after_cycle`, and
   the `Frame`/`Stack` types). If `push_timestamp()`'s format is
   genuinely needed elsewhere, extract it to a small utility module;
   otherwise delete.
3. Collapse `run_stack` in `src/phase_loop.rs` back to a simple
   wrapper that loops `phase_loop` with the existing
   continue-or-exit user prompt. Rename appropriately (e.g.
   `run_single_plan`) — "stack" terminology is no longer meaningful.
4. Remove all `stack.yaml` I/O paths: reads, writes, validation,
   sync-to-disk logic, file-format parser.
5. Remove tests for pivot state machines, stack serialisation, and
   push-plan validation.
6. Grep `src/`, `defaults/`, `tests/` for remaining references
   (`stack.yaml`, `push-plan`, `pivot::`, `Frame`, `Stack`) and clean
   them up. Obsolete memory entries in `LLM_STATE/core/memory.md`
   are pruned by the next core triage cycle, not by this task.

**Results:** _pending_
