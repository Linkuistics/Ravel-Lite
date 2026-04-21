# Memory

### Input hash uses length-prefixed concatenation
`PlanRow.input_hash` is SHA-256 over length-prefixed `phase.md + backlog.md + memory.md + related-plans.md`. Absent file hashes distinctly from empty file; this is intentional to detect missing vs. empty inputs.

### `PlanRow.input_hash` carries `#[serde(default)]`
The field is absent in LLM-emitted YAML and injected by the Rust harness post-parse. `#[serde(default)]` lets `parse_survey_response` accept both LLM output (no field) and harness-injected round-trips (field present).

### Hash injection hard-errors on unknown or missing rows
`inject_input_hashes` treats undiscovered rows and missing rows as hard errors. No silent pass-through. This invariant extends to 5b: delta classification must refuse mutation outside the declared changed set.

### YAML is persistence; markdown is presentation
`run_survey` emits YAML only (`serde_yaml::to_string`). Markdown rendering was removed from `run_survey` and lives exclusively in `survey-format`. Coupling them again would re-entangle two separate concerns.

### Survey CLI names plan dirs individually
`discover_plans` tree walk replaced by `load_plan(plan_dir)`. CLI positional args are `plan_dirs`. Callers enumerate plans explicitly; no implicit directory walk.

### `plan_key` and `input_hash` key 5b delta classification
These are the keying primitives for delta logic. `parse_survey_response` is the single entry point for both LLM stdout and `--prior` file reads.

### `schema_version` deferred to 5b
Adding it in 5a would couple 5a to a 5b-only concern. It is a one-line `SurveyResponse` addition deferred intentionally.

### `run_single_plan` is the seam for 5c multi-plan dispatch
`run_single_plan` in `src/phase_loop.rs` is a 9-line delegate retained intentionally. Task 5c branches on plan-count in `main::run_phase_loop`: single-plan path calls `run_single_plan` unchanged; multi-plan path adds a survey-routed dispatch loop around it.

### Six pre-existing clippy doc-formatting errors in `src/survey/schema.rs`
`cargo clippy` reports 6 doc-formatting warnings in `src/survey/schema.rs`. These predate the survey restructure and are out of scope for this plan.
